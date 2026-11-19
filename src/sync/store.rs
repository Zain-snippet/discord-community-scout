//! SQLite side of synchronization: build the local document, apply a merged
//! document, store conflicts, and keep bounded snapshots.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::models::{PromotionPolicy, ReviewStatus};
use crate::sync::document::{CloudCommunity, CloudDocument, CloudRun, CloudSource, MAX_RUNS};
use crate::sync::merge::FieldConflict;

pub const MAX_SNAPSHOTS: usize = 5;

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[derive(Debug, Clone)]
pub struct SyncState {
    pub workspace_id: Option<String>,
    pub device_id: String,
    pub base_revision: Option<String>,
    pub base_document: Option<String>,
    pub last_synced_at: Option<String>,
    /// Monotonic write counter. Triggers increment it, so a sync can tell
    /// whether new local edits arrived while it was uploading.
    pub dirty: i64,
    pub status: String,
    pub last_error: Option<String>,
}

impl SyncState {
    /// The last document this device confirmed with the cloud. A parse failure
    /// is treated as "no base", which forces an explicit migration decision
    /// instead of guessing.
    pub fn base(&self) -> Option<CloudDocument> {
        let raw = self.base_document.as_deref()?;
        CloudDocument::from_bytes(raw.as_bytes()).ok()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty != 0
    }
}

pub async fn load_state(pool: &SqlitePool) -> anyhow::Result<SyncState> {
    let row = sqlx::query(
        "SELECT workspace_id, device_id, base_revision, base_document,
                last_synced_at, dirty, status, last_error
         FROM sync_state WHERE id = 1",
    )
    .fetch_one(pool)
    .await?;

    Ok(SyncState {
        workspace_id: row.get("workspace_id"),
        device_id: row.get("device_id"),
        base_revision: row.get("base_revision"),
        base_document: row.get("base_document"),
        last_synced_at: row.get("last_synced_at"),
        dirty: row.get("dirty"),
        status: row.get("status"),
        last_error: row.get("last_error"),
    })
}

pub async fn set_status(
    pool: &SqlitePool,
    status: &str,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE sync_state SET status = ?, last_error = ? WHERE id = 1")
        .bind(status)
        .bind(error)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_dirty(pool: &SqlitePool, dirty: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE sync_state SET dirty = ? WHERE id = 1")
        .bind(dirty)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_workspace(pool: &SqlitePool, workspace_id: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE sync_state SET workspace_id = ? WHERE id = 1")
        .bind(workspace_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Records a successful confirmed cloud write. `expected_dirty` is the counter
/// observed before the local document was built; if new edits arrived while the
/// upload was in flight the counter no longer matches and the workspace stays
/// dirty instead of being falsely marked clean.
pub async fn save_sync_success(
    pool: &SqlitePool,
    document: &CloudDocument,
    file_id: &str,
    revision: &str,
    etag: Option<&str>,
    expected_dirty: i64,
) -> anyhow::Result<()> {
    let bytes = document
        .to_bytes()
        .map_err(|err| anyhow::anyhow!("cannot serialise the synced document: {err}"))?;
    let text = String::from_utf8(bytes)?;
    sqlx::query(
        "UPDATE sync_state SET workspace_id = ?, drive_file_id = ?, base_revision = ?,
            base_etag = ?, base_document = ?, last_synced_at = ?,
            dirty = CASE WHEN dirty = ? THEN 0 ELSE dirty END,
            status = 'synced', last_error = NULL
         WHERE id = 1",
    )
    .bind(&document.workspace_id)
    .bind(file_id)
    .bind(revision)
    .bind(etag)
    .bind(text)
    .bind(now())
    .bind(expected_dirty)
    .execute(pool)
    .await?;
    Ok(())
}

/// Identifiers that existed locally when the merge started. A merged document
/// may delete only these; rows written after the merge began are never touched.
#[derive(Debug, Default, Clone)]
pub struct LocalSnapshot {
    pub community_ids: HashSet<String>,
    pub settings_keys: HashSet<String>,
}

pub async fn snapshot_ids(pool: &SqlitePool) -> anyhow::Result<LocalSnapshot> {
    let ids = sqlx::query_scalar::<_, String>("SELECT id FROM communities")
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
    let keys = sqlx::query_scalar::<_, String>("SELECT key FROM settings")
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
    Ok(LocalSnapshot {
        community_ids: ids,
        settings_keys: keys,
    })
}

pub async fn local_document(
    pool: &SqlitePool,
    workspace_id: &str,
    device_id: &str,
) -> anyhow::Result<CloudDocument> {
    let mut document = CloudDocument::empty(workspace_id, device_id, &now());

    let rows = sqlx::query(
        "SELECT id, name, description, invite_url, invite_code, canonical_url, primary_source,
                tags, member_count, language, relevance_score, score_breakdown, review_status,
                promotion_policy, policy_evidence, policy_checked_at, channel_name, user_notes,
                contacted_at, last_checked_at, created_at, updated_at
         FROM communities ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await?;

    let source_rows = sqlx::query(
        "SELECT community_id, source_name, source_external_id, listing_url, first_seen_at, last_seen_at
         FROM community_sources ORDER BY community_id ASC, first_seen_at ASC, id ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut sources: BTreeMap<String, Vec<CloudSource>> = BTreeMap::new();
    for row in source_rows {
        sources
            .entry(row.get("community_id"))
            .or_default()
            .push(CloudSource {
                source_name: row.get("source_name"),
                source_external_id: row.get("source_external_id"),
                listing_url: row.get("listing_url"),
                first_seen_at: row.get("first_seen_at"),
                last_seen_at: row.get("last_seen_at"),
            });
    }

    for row in rows {
        let id: String = row.get("id");
        let community = CloudCommunity {
            id: id.clone(),
            name: row.get("name"),
            description: row.get("description"),
            invite_url: row.get("invite_url"),
            invite_code: row.get("invite_code"),
            canonical_url: row.get("canonical_url"),
            primary_source: row.get("primary_source"),
            tags: parse_json::<Vec<String>>(row.get::<String, _>("tags")),
            member_count: row.get("member_count"),
            language: row.get("language"),
            relevance_score: row.get("relevance_score"),
            score_breakdown: parse_json::<Vec<crate::scoring::ScoreFactor>>(
                row.get::<String, _>("score_breakdown"),
            ),
            review_status: row.get("review_status"),
            promotion_policy: row.get("promotion_policy"),
            policy_evidence: row.get("policy_evidence"),
            policy_checked_at: row.get("policy_checked_at"),
            channel_name: row.get("channel_name"),
            user_notes: row.get("user_notes"),
            contacted_at: row.get("contacted_at"),
            last_checked_at: row.get("last_checked_at"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            sources: sources.remove(&id).unwrap_or_default(),
        };
        document.communities.push(community);
    }

    let settings_rows = sqlx::query("SELECT key, value FROM settings ORDER BY key ASC")
        .fetch_all(pool)
        .await?;
    for row in settings_rows {
        document.settings.insert(row.get("key"), row.get("value"));
    }

    let run_rows = sqlx::query(
        "SELECT id, query, sources, requested_limit, status, result_count, saved_count,
                partial, error, started_at, finished_at
         FROM discovery_runs ORDER BY started_at DESC LIMIT ?",
    )
    .bind(MAX_RUNS as i64)
    .fetch_all(pool)
    .await?;
    for row in run_rows {
        document.discovery_runs.push(CloudRun {
            id: row.get("id"),
            query: row.get("query"),
            sources: parse_json::<Vec<String>>(row.get::<String, _>("sources")),
            requested_limit: row.get("requested_limit"),
            status: row.get("status"),
            result_count: row.get("result_count"),
            saved_count: row.get("saved_count"),
            partial: row.get::<i64, _>("partial") != 0,
            error: row.get("error"),
            started_at: row.get("started_at"),
            finished_at: row.get("finished_at"),
        });
    }

    Ok(document)
}

fn parse_json<T: serde::de::DeserializeOwned + Default>(raw: String) -> T {
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Keeps the database's unique invite-code index satisfiable when two devices
/// independently added the same link. Later duplicates lose the code but keep
/// the record. Deterministic, so both devices converge on the same choice.
pub fn dedupe_invite_codes(document: &mut CloudDocument) -> usize {
    let mut seen: HashSet<String> = HashSet::new();
    let mut cleared = 0;
    document.communities.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    for community in &mut document.communities {
        if let Some(code) = community.invite_code.clone() {
            if !seen.insert(code.to_ascii_lowercase()) {
                community.invite_code = None;
                cleared += 1;
            }
        }
    }
    document.communities.sort_by(|a, b| a.id.cmp(&b.id));
    cleared
}

/// Applies a merged document to SQLite. Deletions are limited to identifiers
/// present in `snapshot`, so concurrent local writes survive.
pub async fn apply_document(
    pool: &SqlitePool,
    document: &CloudDocument,
    snapshot: &LocalSnapshot,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    for community in &document.communities {
        upsert_community(&mut tx, community).await?;
        for source in &community.sources {
            sqlx::query(
                "INSERT INTO community_sources
                    (community_id, source_name, source_external_id, listing_url, first_seen_at, last_seen_at)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT (community_id, source_name, ifnull(source_external_id, ''))
                 DO UPDATE SET last_seen_at = excluded.last_seen_at,
                               listing_url = COALESCE(excluded.listing_url, community_sources.listing_url)",
            )
            .bind(&community.id)
            .bind(&source.source_name)
            .bind(&source.source_external_id)
            .bind(&source.listing_url)
            .bind(&source.first_seen_at)
            .bind(&source.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    let retained: HashSet<&str> = document
        .communities
        .iter()
        .map(|community| community.id.as_str())
        .collect();
    for id in &snapshot.community_ids {
        if !retained.contains(id.as_str()) {
            sqlx::query("DELETE FROM communities WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }

    for key in &snapshot.settings_keys {
        if !document.settings.contains_key(key) {
            sqlx::query("DELETE FROM settings WHERE key = ?")
                .bind(key)
                .execute(&mut *tx)
                .await?;
        }
    }
    for (key, value) in &document.settings {
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(value)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    }

    for run in &document.discovery_runs {
        sqlx::query(
            "INSERT INTO discovery_runs
                (id, query, sources, requested_limit, status, result_count, saved_count,
                 partial, error, started_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET
                query = excluded.query, sources = excluded.sources,
                requested_limit = excluded.requested_limit, status = excluded.status,
                result_count = excluded.result_count, saved_count = excluded.saved_count,
                partial = excluded.partial, error = excluded.error,
                started_at = excluded.started_at, finished_at = excluded.finished_at",
        )
        .bind(&run.id)
        .bind(&run.query)
        .bind(serde_json::to_string(&run.sources).unwrap_or_else(|_| "[]".into()))
        .bind(run.requested_limit)
        .bind(&run.status)
        .bind(run.result_count)
        .bind(run.saved_count)
        .bind(i64::from(run.partial))
        .bind(&run.error)
        .bind(&run.started_at)
        .bind(&run.finished_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub(crate) async fn upsert_community(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    community: &CloudCommunity,
) -> anyhow::Result<()> {
    // Validation happened before merge, so these are only defensive fallbacks.
    let status = ReviewStatus::parse(&community.review_status)
        .unwrap_or(ReviewStatus::NotReviewed)
        .as_str()
        .to_string();
    let policy =
        PromotionPolicy::parse(&community.promotion_policy).unwrap_or(PromotionPolicy::Unknown);

    // Scores are derived from the merged fields; never taken from the document.
    let score = crate::repo::score_record(
        None,
        &community.name,
        &community.description,
        &community.tags,
        policy,
        community.policy_evidence.as_deref(),
        &community.user_notes,
    );

    sqlx::query(
        "INSERT INTO communities (
            id, name, description, invite_url, invite_code, canonical_url, primary_source,
            tags, member_count, language, relevance_score, score_breakdown, review_status,
            promotion_policy, policy_evidence, policy_checked_at, channel_name, user_notes,
            contacted_at, last_checked_at, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET
            name = excluded.name, description = excluded.description,
            invite_url = excluded.invite_url, invite_code = excluded.invite_code,
            canonical_url = excluded.canonical_url, primary_source = excluded.primary_source,
            tags = excluded.tags, member_count = excluded.member_count,
            language = excluded.language, relevance_score = excluded.relevance_score,
            score_breakdown = excluded.score_breakdown, review_status = excluded.review_status,
            promotion_policy = excluded.promotion_policy, policy_evidence = excluded.policy_evidence,
            policy_checked_at = excluded.policy_checked_at, channel_name = excluded.channel_name,
            user_notes = excluded.user_notes, contacted_at = excluded.contacted_at,
            last_checked_at = excluded.last_checked_at, created_at = excluded.created_at,
            updated_at = excluded.updated_at",
    )
    .bind(&community.id)
    .bind(&community.name)
    .bind(&community.description)
    .bind(&community.invite_url)
    .bind(&community.invite_code)
    .bind(&community.canonical_url)
    .bind(&community.primary_source)
    .bind(serde_json::to_string(&community.tags).unwrap_or_else(|_| "[]".into()))
    .bind(community.member_count)
    .bind(&community.language)
    .bind(score.total)
    .bind(serde_json::to_string(&score.factors).unwrap_or_else(|_| "[]".into()))
    .bind(status)
    .bind(policy.as_str())
    .bind(&community.policy_evidence)
    .bind(&community.policy_checked_at)
    .bind(&community.channel_name)
    .bind(&community.user_notes)
    .bind(&community.contacted_at)
    .bind(&community.last_checked_at)
    .bind(&community.created_at)
    .bind(&community.updated_at)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConflictRow {
    pub id: String,
    pub community_id: String,
    pub community_name: String,
    pub field: String,
    pub base_value: Option<String>,
    pub local_value: Option<String>,
    pub remote_value: Option<String>,
    pub created_at: String,
}

pub async fn pending_conflicts(pool: &SqlitePool) -> anyhow::Result<Vec<ConflictRow>> {
    let rows = sqlx::query(
        "SELECT id, community_id, community_name, field, base_value, local_value,
                remote_value, created_at
         FROM sync_conflicts ORDER BY community_name ASC, field ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ConflictRow {
            id: row.get("id"),
            community_id: row.get("community_id"),
            community_name: row.get("community_name"),
            field: row.get("field"),
            base_value: row.get("base_value"),
            local_value: row.get("local_value"),
            remote_value: row.get("remote_value"),
            created_at: row.get("created_at"),
        })
        .collect())
}

pub async fn replace_conflicts(
    pool: &SqlitePool,
    conflicts: &[FieldConflict],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM sync_conflicts")
        .execute(&mut *tx)
        .await?;
    for conflict in conflicts {
        sqlx::query(
            "INSERT OR REPLACE INTO sync_conflicts
                (id, community_id, community_name, field, base_value, local_value,
                 remote_value, local_record, remote_record, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(conflict.id())
        .bind(&conflict.community_id)
        .bind(&conflict.community_name)
        .bind(&conflict.field)
        .bind(&conflict.base_value)
        .bind(&conflict.local_value)
        .bind(&conflict.remote_value)
        .bind(&conflict.local_record)
        .bind(&conflict.remote_record)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn clear_conflicts(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sync_conflicts")
        .execute(pool)
        .await?;
    Ok(())
}

/// Looks up the community and field a conflict refers to.
pub async fn conflict_target(
    pool: &SqlitePool,
    conflict_id: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT community_id, field FROM sync_conflicts WHERE id = ?")
            .bind(conflict_id)
            .fetch_optional(pool)
            .await?;
    Ok(row)
}
/* --------------------------------------------------------------- snapshots */

/// Writes a bounded snapshot of a document before a destructive change.
pub fn write_snapshot(
    dir: &Path,
    label: &str,
    document: &CloudDocument,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let safe_label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let path = dir.join(format!("{stamp}-{safe_label}.json"));
    let bytes = document
        .to_bytes()
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    std::fs::write(&path, bytes)?;
    prune_snapshots(dir, MAX_SNAPSHOTS)?;
    Ok(path)
}

/// Preserves an unparseable cloud body so the user can recover it later.
pub fn write_recovery(dir: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string();
    let path = dir.join(format!("cloud-{stamp}.json"));
    std::fs::write(&path, bytes)?;
    prune_snapshots(dir, MAX_SNAPSHOTS)?;
    Ok(path)
}

fn prune_snapshots(dir: &Path, keep: usize) -> std::io::Result<()> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    if files.len() > keep {
        for path in &files[..files.len() - keep] {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SnapshotInfo {
    pub name: String,
    pub bytes: u64,
}

pub fn list_snapshots(dir: &Path) -> std::io::Result<Vec<SnapshotInfo>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        found.push(SnapshotInfo {
            name: name.to_string(),
            bytes: entry.metadata().map(|meta| meta.len()).unwrap_or(0),
        });
    }
    found.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(found)
}

/// Reads a snapshot by its bare file name; path separators are refused.
pub fn read_snapshot(dir: &Path, name: &str) -> std::io::Result<CloudDocument> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !name.ends_with(".json")
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid snapshot name",
        ));
    }
    let bytes = std::fs::read(dir.join(name))?;
    CloudDocument::from_bytes(&bytes)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_invite_codes_are_cleared_deterministically() {
        let mut document = CloudDocument::empty("ws", "device", "2026-01-01T00:00:00Z");
        let mut first = sample_record("id-b", "Later");
        first.invite_code = Some("same".to_string());
        let mut second = sample_record("id-a", "Earlier");
        second.invite_code = Some("SAME".to_string());
        second.created_at = "2025-01-01T00:00:00Z".to_string();
        document.communities = vec![first, second];

        assert_eq!(dedupe_invite_codes(&mut document), 1);
        assert_eq!(document.communities[0].id, "id-a");
        assert_eq!(document.communities[0].invite_code.as_deref(), Some("SAME"));
        assert_eq!(document.communities[1].invite_code, None);
    }

    #[test]
    fn snapshot_names_cannot_escape_the_directory() {
        let dir = std::env::temp_dir();
        assert!(read_snapshot(&dir, "../secrets.json").is_err());
        assert!(read_snapshot(&dir, "a/b.json").is_err());
        assert!(read_snapshot(&dir, "notes.txt").is_err());
    }

    fn sample_record(id: &str, name: &str) -> CloudCommunity {
        CloudCommunity {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            invite_url: None,
            invite_code: None,
            canonical_url: None,
            primary_source: "Manual entry".to_string(),
            tags: vec![],
            member_count: None,
            language: None,
            relevance_score: 0,
            score_breakdown: vec![],
            review_status: "not_reviewed".to_string(),
            promotion_policy: "unknown".to_string(),
            policy_evidence: None,
            policy_checked_at: None,
            channel_name: None,
            user_notes: String::new(),
            contacted_at: None,
            last_checked_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            sources: vec![],
        }
    }
}
