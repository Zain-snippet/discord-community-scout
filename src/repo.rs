//! Database access for communities, provenance, dashboard metrics and settings.

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::{
    error::{AppError, AppResult},
    models::{
        Community, CommunityInput, Dashboard, LastActivity, ListQuery, Page, PromotionPolicy,
        Provenance, ReviewInput, ReviewStatus,
    },
    scoring::{self, Score, ScoreFactor},
    validate,
};

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Stored representation straight from the database row.
#[derive(Debug, sqlx::FromRow)]
struct CommunityRow {
    id: String,
    name: String,
    description: String,
    invite_url: Option<String>,
    invite_code: Option<String>,
    canonical_url: Option<String>,
    primary_source: String,
    tags: String,
    member_count: Option<i64>,
    language: Option<String>,
    relevance_score: i64,
    score_breakdown: String,
    review_status: String,
    promotion_policy: String,
    policy_evidence: Option<String>,
    policy_checked_at: Option<String>,
    channel_name: Option<String>,
    user_notes: String,
    contacted_at: Option<String>,
    last_checked_at: Option<String>,
    created_at: String,
    updated_at: String,
}

const SELECT_COLUMNS: &str = "id, name, description, invite_url, invite_code, canonical_url, \
     primary_source, tags, member_count, language, relevance_score, score_breakdown, \
     review_status, promotion_policy, policy_evidence, policy_checked_at, channel_name, \
     user_notes, contacted_at, last_checked_at, created_at, updated_at";

fn parse_json_vec<T: serde::de::DeserializeOwned>(raw: &str) -> Vec<T> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn policy_of(raw: &str) -> PromotionPolicy {
    PromotionPolicy::parse(raw).unwrap_or(PromotionPolicy::Unknown)
}

/// Scores a record and returns the stored score plus its breakdown.
pub fn score_record(
    query: Option<&str>,
    name: &str,
    description: &str,
    tags: &[String],
    policy: PromotionPolicy,
    evidence: Option<&str>,
    notes: &str,
) -> Score {
    scoring::score(&scoring::ScoreInput {
        query,
        name,
        description,
        tags,
        promotion_policy: policy,
        policy_evidence: evidence,
        user_notes: notes,
    })
}

/// Records without a discovery query are scored against their own tags, so the
/// stored number is still explainable rather than arbitrary.
fn self_query(tags: &[String]) -> Option<String> {
    if tags.is_empty() {
        None
    } else {
        Some(tags.join(" "))
    }
}

fn to_community(row: CommunityRow, sources: Vec<Provenance>) -> Community {
    let tags: Vec<String> = parse_json_vec(&row.tags);
    let breakdown: Vec<ScoreFactor> = parse_json_vec(&row.score_breakdown);
    let policy = policy_of(&row.promotion_policy);
    let (unsuitable_for_outreach, outreach_note) = match policy {
        PromotionPolicy::Prohibited => (
            true,
            "This community's rules prohibit promotion or recruitment. Do not post here."
                .to_string(),
        ),
        PromotionPolicy::Unknown => (
            false,
            "Rules are not verified. Check the community's posted rules before posting."
                .to_string(),
        ),
        PromotionPolicy::Allowed => (
            false,
            "Rules were recorded as allowing promotion. Confirm before posting.".to_string(),
        ),
    };

    Community {
        id: row.id,
        name: row.name,
        description: row.description,
        invite_url: row.invite_url,
        invite_code: row.invite_code,
        canonical_url: row.canonical_url,
        primary_source: row.primary_source,
        tags,
        member_count: row.member_count,
        language: row.language,
        relevance_score: row.relevance_score,
        score_breakdown: breakdown,
        review_status: row.review_status,
        promotion_policy: row.promotion_policy,
        policy_evidence: row.policy_evidence,
        policy_checked_at: row.policy_checked_at,
        channel_name: row.channel_name,
        user_notes: row.user_notes,
        contacted_at: row.contacted_at,
        last_checked_at: row.last_checked_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
        sources,
        unsuitable_for_outreach,
        outreach_note,
    }
}

async fn sources_for(pool: &SqlitePool, community_id: &str) -> AppResult<Vec<Provenance>> {
    let rows = sqlx::query(
        "SELECT source_name, source_external_id, listing_url, first_seen_at, last_seen_at
         FROM community_sources WHERE community_id = ? ORDER BY first_seen_at ASC, id ASC",
    )
    .bind(community_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Provenance {
            source_name: row.get("source_name"),
            source_external_id: row.get("source_external_id"),
            listing_url: row.get("listing_url"),
            first_seen_at: row.get("first_seen_at"),
            last_seen_at: row.get("last_seen_at"),
        })
        .collect())
}

async fn attach_sources(pool: &SqlitePool, mut community: Community) -> AppResult<Community> {
    community.sources = sources_for(pool, &community.id).await?;
    Ok(community)
}

/// Fetches one community with its provenance, or a 404.
pub async fn get(pool: &SqlitePool, id: &str) -> AppResult<Community> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM communities WHERE id = ?");
    let row = sqlx::query_as::<_, CommunityRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::not_found("Community not found."))?;

    attach_sources(pool, to_community(row, Vec::new())).await
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Lists communities with filtering, sorting and pagination.
pub async fn list(pool: &SqlitePool, query: &ListQuery) -> AppResult<Page<Community>> {
    let per_page = query.per_page.unwrap_or(25).clamp(1, 100);
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * per_page;

    let text = query
        .q
        .as_deref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| format!("%{}%", escape_like(v)));

    let status = match query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(raw) => Some(
            ReviewStatus::parse(raw)
                .ok_or_else(|| AppError::bad_request("Unknown review status filter."))?
                .as_str()
                .to_string(),
        ),
    };

    let source = query
        .source
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let tag = query
        .tag
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .map(|s| format!("%\"{}\"%", escape_like(&s)));
    let outreach_only = i64::from(query.outreach_only.unwrap_or(false));

    let order = match query.sort.as_deref().unwrap_or("relevance") {
        "relevance" => "relevance_score DESC, name COLLATE NOCASE ASC",
        "name" => "name COLLATE NOCASE ASC",
        "newest" => "created_at DESC",
        "oldest" => "created_at ASC",
        other => {
            return Err(AppError::bad_request(format!(
                "Unknown sort {other:?}. Use relevance, name, newest or oldest."
            )))
        }
    };

    let where_clause = "WHERE (?1 IS NULL OR (name LIKE ?1 ESCAPE '\\' OR description LIKE ?1 ESCAPE '\\' OR tags LIKE ?1 ESCAPE '\\'))
           AND (?2 IS NULL OR review_status = ?2)
           AND (?3 IS NULL OR primary_source = ?3)
           AND (?4 IS NULL OR tags LIKE ?4 ESCAPE '\\')
           AND (?5 = 0 OR promotion_policy <> 'prohibited')";

    let total: i64 =
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM communities {where_clause}"))
            .bind(text.clone())
            .bind(status.clone())
            .bind(source.clone())
            .bind(tag.clone())
            .bind(outreach_only)
            .fetch_one(pool)
            .await?;

    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM communities {where_clause} ORDER BY {order} LIMIT ?6 OFFSET ?7"
    );
    let rows = sqlx::query_as::<_, CommunityRow>(&sql)
        .bind(text)
        .bind(status)
        .bind(source)
        .bind(tag)
        .bind(outreach_only)
        .bind(per_page)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let community = to_community(row, Vec::new());
        items.push(attach_sources(pool, community).await?);
    }

    let total_pages = if total == 0 {
        1
    } else {
        (total + per_page - 1) / per_page
    };

    Ok(Page {
        items,
        page,
        per_page,
        total,
        total_pages,
    })
}

async fn invite_code_taken(
    pool: &SqlitePool,
    code: &str,
    ignore_id: Option<&str>,
) -> AppResult<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM communities WHERE invite_code = ? AND (?2 IS NULL OR id <> ?2)",
    )
    .bind(code)
    .bind(ignore_id)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

/// Creates a community entered by hand.
pub async fn create_manual(pool: &SqlitePool, input: &CommunityInput) -> AppResult<Community> {
    create_from_source(pool, input, "Manual entry", None, None, None).await
}

/// Creates a community attributed to a given source (manual entry, CSV import or
/// a discovery connector).
///
/// `score_query` is the discovery query the record was found with. When it is
/// `None` the record is scored against its own tags, so the stored number is
/// still explainable rather than arbitrary.
pub async fn create_from_source(
    pool: &SqlitePool,
    input: &CommunityInput,
    source_name: &str,
    external_id: Option<&str>,
    listing_url: Option<&str>,
    score_query: Option<&str>,
) -> AppResult<Community> {
    let name = validate::required_text(&input.name, "Name", validate::MAX_NAME)
        .map_err(AppError::bad_request)?;
    let description = validate::optional_text(
        input.description.as_deref(),
        "Description",
        validate::MAX_DESCRIPTION,
    )
    .map_err(AppError::bad_request)?
    .unwrap_or_default();
    let tags = validate::normalise_tags(input.tags.as_deref().unwrap_or(&[]))
        .map_err(AppError::bad_request)?;
    let (invite_url, invite_code, canonical_url) = validate::normalise_invite_fields(
        input.invite_url.as_deref(),
        input.canonical_url.as_deref(),
    )
    .map_err(AppError::bad_request)?;
    let language = validate::optional_text(input.language.as_deref(), "Language", 60)
        .map_err(AppError::bad_request)?;
    let channel_name = validate::optional_text(
        input.channel_name.as_deref(),
        "Channel name",
        validate::MAX_CHANNEL,
    )
    .map_err(AppError::bad_request)?;
    let user_notes =
        validate::optional_text(input.user_notes.as_deref(), "Notes", validate::MAX_NOTES)
            .map_err(AppError::bad_request)?
            .unwrap_or_default();
    let policy = match input.promotion_policy.as_deref() {
        Some(raw) => validate::parse_policy(raw).map_err(AppError::bad_request)?,
        None => PromotionPolicy::Unknown,
    };
    let evidence = validate::optional_text(
        input.policy_evidence.as_deref(),
        "Policy evidence",
        validate::MAX_EVIDENCE,
    )
    .map_err(AppError::bad_request)?;
    let status = match input.review_status.as_deref() {
        Some(raw) => ReviewStatus::parse(&validate::clean_text(raw).to_ascii_lowercase())
            .ok_or_else(|| AppError::bad_request("Unknown review_status."))?,
        None => ReviewStatus::NotReviewed,
    };

    if let Some(code) = &invite_code {
        if invite_code_taken(pool, code, None).await? {
            return Err(AppError::conflict(
                "A community with this invite link already exists.",
            ));
        }
    }

    let tag_query = self_query(&tags);
    let score = score_record(
        score_query.or(tag_query.as_deref()),
        &name,
        &description,
        &tags,
        policy,
        evidence.as_deref(),
        &user_notes,
    );

    let id = uuid::Uuid::new_v4().to_string();
    let timestamp = now();
    let status_str = status.as_str();

    sqlx::query(
        "INSERT INTO communities (
            id, name, description, invite_url, invite_code, canonical_url, primary_source,
            tags, language, relevance_score, score_breakdown, review_status, promotion_policy,
            policy_evidence, policy_checked_at, channel_name, user_notes, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&description)
    .bind(&invite_url)
    .bind(&invite_code)
    .bind(&canonical_url)
    .bind(source_name)
    .bind(serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into()))
    .bind(&language)
    .bind(score.total)
    .bind(serde_json::to_string(&score.factors).unwrap_or_else(|_| "[]".into()))
    .bind(status_str)
    .bind(policy.as_str())
    .bind(&evidence)
    .bind(if policy == PromotionPolicy::Unknown {
        None
    } else {
        Some(timestamp.clone())
    })
    .bind(&channel_name)
    .bind(&user_notes)
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(pool)
    .await?;

    insert_provenance(
        pool,
        &id,
        source_name,
        external_id,
        listing_url.or(canonical_url.as_deref()),
        &timestamp,
    )
    .await?;

    get(pool, &id).await
}

/// Looks up existing communities by invite code.
///
/// Returns `(invite_code, community_id, name)` for each code that already exists.
pub async fn find_by_invite_codes(
    pool: &SqlitePool,
    codes: &[String],
) -> AppResult<Vec<(String, String, String)>> {
    let mut found = Vec::new();
    for code in codes {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT id, name FROM communities WHERE invite_code = ?")
                .bind(code)
                .fetch_optional(pool)
                .await?;
        if let Some((id, name)) = row {
            found.push((code.clone(), id, name));
        }
    }
    Ok(found)
}

/// Inserts or refreshes one provenance reference.
pub async fn insert_provenance(
    pool: &SqlitePool,
    community_id: &str,
    source_name: &str,
    external_id: Option<&str>,
    listing_url: Option<&str>,
    timestamp: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO community_sources
            (community_id, source_name, source_external_id, listing_url, first_seen_at, last_seen_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (community_id, source_name, ifnull(source_external_id, ''))
         DO UPDATE SET last_seen_at = excluded.last_seen_at,
                       listing_url = COALESCE(excluded.listing_url, community_sources.listing_url)",
    )
    .bind(community_id)
    .bind(source_name)
    .bind(external_id)
    .bind(listing_url)
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await?;
    Ok(())
}

/// Updates the editable fields of a community.
pub async fn update(pool: &SqlitePool, id: &str, input: &CommunityInput) -> AppResult<Community> {
    let existing = get(pool, id).await?;

    let name = validate::required_text(&input.name, "Name", validate::MAX_NAME)
        .map_err(AppError::bad_request)?;
    let description = validate::optional_text(
        input.description.as_deref(),
        "Description",
        validate::MAX_DESCRIPTION,
    )
    .map_err(AppError::bad_request)?
    .unwrap_or_default();
    let tags = validate::normalise_tags(input.tags.as_deref().unwrap_or(&[]))
        .map_err(AppError::bad_request)?;
    let (invite_url, invite_code, canonical_url) = validate::normalise_invite_fields(
        input.invite_url.as_deref(),
        input.canonical_url.as_deref(),
    )
    .map_err(AppError::bad_request)?;
    let language = validate::optional_text(input.language.as_deref(), "Language", 60)
        .map_err(AppError::bad_request)?;
    let channel_name = validate::optional_text(
        input.channel_name.as_deref(),
        "Channel name",
        validate::MAX_CHANNEL,
    )
    .map_err(AppError::bad_request)?;
    let user_notes =
        validate::optional_text(input.user_notes.as_deref(), "Notes", validate::MAX_NOTES)
            .map_err(AppError::bad_request)?
            .unwrap_or_default();
    let policy = match input.promotion_policy.as_deref() {
        Some(raw) => validate::parse_policy(raw).map_err(AppError::bad_request)?,
        None => policy_of(&existing.promotion_policy),
    };
    let evidence = match input.policy_evidence.as_deref() {
        Some(_) => validate::optional_text(
            input.policy_evidence.as_deref(),
            "Policy evidence",
            validate::MAX_EVIDENCE,
        )
        .map_err(AppError::bad_request)?,
        None => existing.policy_evidence.clone(),
    };

    if let Some(code) = &invite_code {
        if invite_code_taken(pool, code, Some(id)).await? {
            return Err(AppError::conflict(
                "Another community already uses this invite link.",
            ));
        }
    }

    let score = score_record(
        self_query(&tags).as_deref(),
        &name,
        &description,
        &tags,
        policy,
        evidence.as_deref(),
        &user_notes,
    );
    let timestamp = now();

    sqlx::query(
        "UPDATE communities SET
            name = ?, description = ?, invite_url = ?, invite_code = ?, canonical_url = ?,
            tags = ?, language = ?, relevance_score = ?, score_breakdown = ?,
            promotion_policy = ?, policy_evidence = ?, channel_name = ?, user_notes = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&name)
    .bind(&description)
    .bind(&invite_url)
    .bind(&invite_code)
    .bind(&canonical_url)
    .bind(serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into()))
    .bind(&language)
    .bind(score.total)
    .bind(serde_json::to_string(&score.factors).unwrap_or_else(|_| "[]".into()))
    .bind(policy.as_str())
    .bind(&evidence)
    .bind(&channel_name)
    .bind(&user_notes)
    .bind(&timestamp)
    .bind(id)
    .execute(pool)
    .await?;

    get(pool, id).await
}

/// Records a review decision.
pub async fn review(pool: &SqlitePool, id: &str, input: &ReviewInput) -> AppResult<Community> {
    let existing = get(pool, id).await?;

    let status = ReviewStatus::parse(&validate::clean_text(&input.review_status).to_ascii_lowercase())
        .ok_or_else(|| {
            AppError::bad_request(
                "review_status must be one of: not_reviewed, suitable, unsuitable, needs_manual_check, contacted.",
            )
        })?;

    let policy = match input.promotion_policy.as_deref() {
        Some(raw) => validate::parse_policy(raw).map_err(AppError::bad_request)?,
        None => policy_of(&existing.promotion_policy),
    };
    let evidence = match input.policy_evidence.as_deref() {
        Some(_) => validate::optional_text(
            input.policy_evidence.as_deref(),
            "Policy evidence",
            validate::MAX_EVIDENCE,
        )
        .map_err(AppError::bad_request)?,
        None => existing.policy_evidence.clone(),
    };
    let channel_name = match input.channel_name.as_deref() {
        Some(_) => validate::optional_text(
            input.channel_name.as_deref(),
            "Channel name",
            validate::MAX_CHANNEL,
        )
        .map_err(AppError::bad_request)?,
        None => existing.channel_name.clone(),
    };
    let user_notes = match input.user_notes.as_deref() {
        Some(_) => {
            validate::optional_text(input.user_notes.as_deref(), "Notes", validate::MAX_NOTES)
                .map_err(AppError::bad_request)?
                .unwrap_or_default()
        }
        None => existing.user_notes.clone(),
    };

    let score = score_record(
        self_query(&existing.tags).as_deref(),
        &existing.name,
        &existing.description,
        &existing.tags,
        policy,
        evidence.as_deref(),
        &user_notes,
    );

    let timestamp = now();
    let contacted_at = if input.contacted || status == ReviewStatus::Contacted {
        Some(
            existing
                .contacted_at
                .clone()
                .unwrap_or_else(|| timestamp.clone()),
        )
    } else {
        None
    };

    sqlx::query(
        "UPDATE communities SET
            review_status = ?, promotion_policy = ?, policy_evidence = ?,
            policy_checked_at = CASE WHEN ? = 'unknown' THEN NULL ELSE ? END,
            channel_name = ?, user_notes = ?, contacted_at = ?,
            last_checked_at = ?, relevance_score = ?, score_breakdown = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(status.as_str())
    .bind(policy.as_str())
    .bind(&evidence)
    .bind(policy.as_str())
    .bind(&timestamp)
    .bind(&channel_name)
    .bind(&user_notes)
    .bind(&contacted_at)
    .bind(&timestamp)
    .bind(score.total)
    .bind(serde_json::to_string(&score.factors).unwrap_or_else(|_| "[]".into()))
    .bind(&timestamp)
    .bind(id)
    .execute(pool)
    .await?;

    get(pool, id).await
}

pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let affected = sqlx::query("DELETE FROM communities WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::not_found("Community not found."));
    }
    Ok(())
}

/// Dashboard counters. Every value comes from the database.
pub async fn dashboard(pool: &SqlitePool) -> AppResult<Dashboard> {
    let row = sqlx::query(
        "SELECT
            COUNT(*) AS total,
            COALESCE(SUM(CASE WHEN review_status = 'not_reviewed' THEN 1 ELSE 0 END), 0) AS awaiting,
            COALESCE(SUM(CASE WHEN review_status <> 'not_reviewed' THEN 1 ELSE 0 END), 0) AS reviewed,
            COALESCE(SUM(CASE WHEN review_status = 'suitable' THEN 1 ELSE 0 END), 0) AS suitable,
            COALESCE(SUM(CASE WHEN review_status = 'unsuitable' THEN 1 ELSE 0 END), 0) AS unsuitable,
            COALESCE(SUM(CASE WHEN review_status = 'contacted' OR contacted_at IS NOT NULL THEN 1 ELSE 0 END), 0) AS contacted,
            COALESCE(SUM(CASE WHEN review_status = 'needs_manual_check' THEN 1 ELSE 0 END), 0) AS needs_check,
            COALESCE(SUM(CASE WHEN promotion_policy = 'prohibited' THEN 1 ELSE 0 END), 0) AS prohibited,
            COALESCE(SUM(CASE WHEN promotion_policy = 'allowed' THEN 1 ELSE 0 END), 0) AS allowed,
            COALESCE(SUM(CASE WHEN promotion_policy = 'unknown' THEN 1 ELSE 0 END), 0) AS unknown_policy
         FROM communities",
    )
    .fetch_one(pool)
    .await?;

    let last_community: Option<(String, String)> =
        sqlx::query_as("SELECT updated_at, name FROM communities ORDER BY updated_at DESC LIMIT 1")
            .fetch_optional(pool)
            .await?;

    let last_run: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT started_at, status, query, finished_at FROM discovery_runs ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    let community_activity = last_community.as_ref().map(|(at, name)| LastActivity {
        kind: "Community updated".to_string(),
        detail: name.clone(),
        at: at.clone(),
    });

    let run_activity = last_run
        .as_ref()
        .map(|(started, status, query, _)| LastActivity {
            kind: "Discovery run".to_string(),
            detail: format!("{query} ({status})"),
            at: started.clone(),
        });

    let last_activity = match (community_activity, run_activity) {
        (Some(a), Some(b)) => Some(if a.at >= b.at { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };

    Ok(Dashboard {
        total_communities: row.get("total"),
        awaiting_review: row.get("awaiting"),
        reviewed: row.get("reviewed"),
        suitable: row.get("suitable"),
        unsuitable: row.get("unsuitable"),
        contacted: row.get("contacted"),
        needs_manual_check: row.get("needs_check"),
        promotion_prohibited: row.get("prohibited"),
        promotion_allowed: row.get("allowed"),
        policy_unknown: row.get("unknown_policy"),
        last_activity,
    })
}

/// Distinct primary sources with counts, for filter controls.
pub async fn source_counts(pool: &SqlitePool) -> AppResult<Vec<(String, i64)>> {
    let rows = sqlx::query(
        "SELECT source_name, COUNT(*) AS total FROM community_sources
         GROUP BY source_name ORDER BY source_name COLLATE NOCASE ASC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.get::<String, _>("source_name"),
                row.get::<i64, _>("total"),
            )
        })
        .collect())
}

/// Distinct tags with counts, for filter controls.
pub async fn tag_counts(pool: &SqlitePool) -> AppResult<Vec<(String, i64)>> {
    let rows = sqlx::query_scalar::<_, String>("SELECT tags FROM communities")
        .fetch_all(pool)
        .await?;

    let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for raw in rows {
        for tag in parse_json_vec::<String>(&raw) {
            *counts.entry(tag).or_insert(0) += 1;
        }
    }
    Ok(counts.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_escaping_neutralises_wildcards() {
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
        assert_eq!(escape_like("c:\\x"), "c:\\\\x");
    }

    #[test]
    fn self_query_is_none_without_tags() {
        assert_eq!(self_query(&[]), None);
        assert_eq!(
            self_query(&["students".to_string(), "money".to_string()]),
            Some("students money".to_string())
        );
    }

    #[test]
    fn unknown_policy_falls_back_to_unknown() {
        assert_eq!(policy_of("garbage"), PromotionPolicy::Unknown);
    }
}
