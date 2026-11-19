//! The synchronization engine: one single-flight cycle against a cloud drive.
//!
//! The engine never trusts the cloud blindly. It validates before applying,
//! snapshots before destructive changes, refuses to overwrite a document it
//! cannot parse, and reports "synced" only after an upload is confirmed.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::sync::document::{CloudDocument, DocumentError};
use crate::sync::merge::{merge_documents_resolved, FieldConflict, Resolution};
use crate::sync::store::{self, SyncState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMeta {
    pub file_id: String,
    pub revision: String,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum DriveError {
    #[error("Google Drive is not configured on this server.")]
    NotConfigured,
    #[error("Google authorization has expired. Sign in again.")]
    AuthExpired,
    #[error("The Scout data file was not found in Google Drive.")]
    NotFound,
    #[error("The cloud document changed while it was being uploaded.")]
    Conflict,
    #[error("Google Drive could not be reached. Changes stay on this device.")]
    Offline,
    #[error("Google Drive asked the app to slow down.")]
    RateLimited,
    #[error("Google Drive refused the request: {0}")]
    Other(String),
}

#[async_trait]
pub trait CloudDrive: Send + Sync {
    /// Returns the metadata of the Scout document, or `None` when the workspace
    /// does not exist yet.
    async fn metadata(&self) -> Result<Option<RemoteMeta>, DriveError>;
    /// Downloads the raw bytes so the engine can validate and preserve them.
    async fn download(&self, file_id: &str) -> Result<(Vec<u8>, RemoteMeta), DriveError>;
    /// Creates the file when `file_id` is `None`, otherwise updates it and
    /// fails with [`DriveError::Conflict`] if `expected` no longer matches.
    async fn upload(
        &self,
        file_id: Option<&str>,
        document: &CloudDocument,
        expected: Option<&RemoteMeta>,
    ) -> Result<RemoteMeta, DriveError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Disconnected,
    Synced,
    Syncing,
    Offline,
    Pending,
    Conflict,
    AuthExpired,
    Failed,
    MigrationRequired,
}

impl SyncStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncStatus::Disconnected => "disconnected",
            SyncStatus::Synced => "synced",
            SyncStatus::Syncing => "syncing",
            SyncStatus::Offline => "offline",
            SyncStatus::Pending => "pending",
            SyncStatus::Conflict => "conflict",
            SyncStatus::AuthExpired => "auth_expired",
            SyncStatus::Failed => "failed",
            SyncStatus::MigrationRequired => "migration_required",
        }
    }

    pub fn from_db(raw: &str) -> Self {
        match raw {
            "synced" => SyncStatus::Synced,
            "syncing" => SyncStatus::Syncing,
            "offline" => SyncStatus::Offline,
            "pending" => SyncStatus::Pending,
            "conflict" => SyncStatus::Conflict,
            "auth_expired" => SyncStatus::AuthExpired,
            "failed" => SyncStatus::Failed,
            "migration_required" => SyncStatus::MigrationRequired,
            _ => SyncStatus::Disconnected,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncReport {
    pub status: SyncStatus,
    pub message: String,
    pub conflicts: usize,
    pub records: usize,
    pub uploaded: bool,
}

impl SyncReport {
    fn new(status: SyncStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            conflicts: 0,
            records: 0,
            uploaded: false,
        }
    }

    fn with_conflicts(mut self, conflicts: usize) -> Self {
        self.conflicts = conflicts;
        self
    }

    fn with_records(mut self, records: usize) -> Self {
        self.records = records;
        self
    }
}

/// What the user chose when local and cloud data both exist without a base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationChoice {
    /// Merge this device's records with the cloud document. Never overwrites.
    Merge,
    /// Adopt the cloud document. Local-only records are snapshotted, then
    /// removed; this is only performed on explicit request.
    UseCloud,
    /// Create the cloud workspace from this device (only valid when no cloud
    /// document exists yet).
    UploadLocal,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncSnapshot {
    pub status: SyncStatus,
    pub raw_status: String,
    pub last_synced_at: Option<String>,
    pub last_error: Option<String>,
    pub dirty: bool,
    pub conflicts: usize,
    pub records: i64,
    pub workspace_id: Option<String>,
    pub has_base: bool,
    pub generation: u64,
}

pub struct SyncEngine {
    pool: SqlitePool,
    drive: Arc<dyn CloudDrive>,
    data_dir: PathBuf,
    lock: tokio::sync::Mutex<()>,
}

impl SyncEngine {
    pub fn new(
        pool: SqlitePool,
        drive: Arc<dyn CloudDrive>,
        data_dir: impl Into<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            drive,
            data_dir: data_dir.into(),
            lock: tokio::sync::Mutex::new(()),
        })
    }

    pub async fn snapshot(&self) -> anyhow::Result<SyncSnapshot> {
        let state = store::load_state(&self.pool).await?;
        let conflicts = store::pending_conflicts(&self.pool).await?.len();
        let records = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM communities")
            .fetch_one(&self.pool)
            .await?;
        let generation = state.base().map(|base| base.generation).unwrap_or(0);
        let status = SyncStatus::from_db(&state.status);
        let dirty = state.is_dirty();
        let has_base = state.base_document.is_some();
        Ok(SyncSnapshot {
            status,
            raw_status: state.status,
            last_synced_at: state.last_synced_at,
            last_error: state.last_error,
            dirty,
            conflicts,
            records,
            workspace_id: state.workspace_id,
            has_base,
            generation,
        })
    }

    pub async fn conflicts(&self) -> anyhow::Result<Vec<store::ConflictRow>> {
        store::pending_conflicts(&self.pool).await
    }

    /// Runs a full cycle. Concurrent calls do not overlap.
    pub async fn sync_now(&self) -> SyncReport {
        self.run(None, &[]).await
    }

    pub async fn migrate(&self, choice: MigrationChoice) -> SyncReport {
        self.run(Some(choice), &[]).await
    }

    pub async fn resolve_conflict(
        &self,
        conflict_id: &str,
        keep_remote: bool,
    ) -> anyhow::Result<SyncReport> {
        let (community_id, field) = store::conflict_target(&self.pool, conflict_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Conflict not found."))?;
        let resolution = Resolution {
            community_id,
            field,
            keep_remote,
        };
        Ok(self.run(None, &[resolution]).await)
    }

    async fn run(
        &self,
        decision: Option<MigrationChoice>,
        resolutions: &[Resolution],
    ) -> SyncReport {
        let _guard = match self.lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                return SyncReport::new(
                    SyncStatus::Syncing,
                    "A synchronization is already in progress.",
                );
            }
        };

        match self.run_locked(decision, resolutions).await {
            Ok(report) => report,
            Err(err) => {
                // Never surface database or internal detail to the browser.
                tracing::error!(error = %err, "sync failed");
                let _ = store::set_status(&self.pool, "failed", Some("Sync failed unexpectedly."))
                    .await;
                SyncReport::new(
                    SyncStatus::Failed,
                    "Sync failed unexpectedly. Your local changes were kept.",
                )
            }
        }
    }

    fn snapshots_dir(&self) -> PathBuf {
        self.data_dir.join("snapshots")
    }

    fn recovery_dir(&self) -> PathBuf {
        self.data_dir.join("recovery")
    }

    async fn record_drive_failure(&self, error: &DriveError) -> anyhow::Result<SyncReport> {
        tracing::warn!(error = %error, "cloud drive request failed");
        let (status, message) = match error {
            DriveError::NotConfigured => (
                SyncStatus::Disconnected,
                "Google Drive is not configured on this server.",
            ),
            DriveError::AuthExpired => (
                SyncStatus::AuthExpired,
                "Google authorization expired or was revoked. Sign in again; no local data was lost.",
            ),
            DriveError::Offline => (
                SyncStatus::Offline,
                "Google Drive is unreachable. Changes stay on this device and will sync later.",
            ),
            DriveError::RateLimited => (
                SyncStatus::Failed,
                "Google Drive asked the app to slow down. A retry will happen automatically.",
            ),
            DriveError::Conflict => (
                SyncStatus::Failed,
                "The cloud document kept changing. Nothing was overwritten; try Sync now again.",
            ),
            DriveError::NotFound => (
                SyncStatus::Failed,
                "The cloud document disappeared. Local data is unchanged.",
            ),
            DriveError::Other(detail) => {
                tracing::warn!("drive error detail: {detail}");
                (
                    SyncStatus::Failed,
                    "Google Drive refused the request. Local data is unchanged.",
                )
            }
        };
        store::set_status(&self.pool, status.as_str(), Some(message)).await?;
        Ok(SyncReport::new(status, message))
    }

    /// Downloads and validates. A malformed body is copied to `data/recovery/`
    /// and a failure report is returned; it is never overwritten.
    async fn download_validated(
        &self,
        file_id: &str,
    ) -> Result<Result<(CloudDocument, RemoteMeta), SyncReport>, DriveError> {
        let (bytes, meta) = self.drive.download(file_id).await?;
        match CloudDocument::from_bytes(&bytes) {
            Ok(document) => Ok(Ok((document, meta))),
            Err(error) => {
                let message = match &error {
                    DocumentError::UnknownSchema(_) => format!(
                        "{error} No data was changed and the cloud file was left untouched."
                    ),
                    _ => format!(
                        "{error} The cloud file was preserved and left untouched; local data is unchanged."
                    ),
                };
                tracing::warn!(error = %error, "refusing to apply an invalid cloud document");
                if let Err(write_error) = store::write_recovery(&self.recovery_dir(), &bytes) {
                    tracing::warn!(error = %write_error, "could not preserve the cloud copy");
                }
                let _ = store::set_status(&self.pool, "failed", Some(&message)).await;
                Ok(Err(SyncReport::new(SyncStatus::Failed, message)))
            }
        }
    }

    fn prepare_document(&self, mut document: CloudDocument, state: &SyncState) -> CloudDocument {
        store::dedupe_invite_codes(&mut document);
        document.updated_at = store::now();
        document.updated_by = state.device_id.clone();
        document
    }

    async fn finish_upload(
        &self,
        document: &CloudDocument,
        meta: &RemoteMeta,
        dirty_at_start: i64,
        uploaded: bool,
    ) -> anyhow::Result<SyncReport> {
        store::save_sync_success(
            &self.pool,
            document,
            &meta.file_id,
            &meta.revision,
            meta.etag.as_deref(),
            dirty_at_start,
        )
        .await?;

        // Confirm with the local store rather than assuming: new local writes
        // may have landed while the upload was in flight.
        let state = store::load_state(&self.pool).await?;
        let local =
            store::local_document(&self.pool, &document.workspace_id, &state.device_id).await?;
        if same_content(&local, document) {
            store::set_dirty(&self.pool, 0).await?;
            store::set_status(&self.pool, "synced", None).await?;
            Ok(SyncReport {
                status: SyncStatus::Synced,
                message: if uploaded {
                    "Synced with Google Drive.".to_string()
                } else {
                    "Up to date with Google Drive.".to_string()
                },
                conflicts: 0,
                records: document.record_count(),
                uploaded,
            })
        } else {
            store::set_status(&self.pool, "pending", None).await?;
            Ok(SyncReport {
                status: SyncStatus::Pending,
                message: "Synced, but newer local changes are still pending.".to_string(),
                conflicts: 0,
                records: document.record_count(),
                uploaded,
            })
        }
    }

    async fn run_locked(
        &self,
        decision: Option<MigrationChoice>,
        resolutions: &[Resolution],
    ) -> anyhow::Result<SyncReport> {
        let state = store::load_state(&self.pool).await?;
        let workspace_id = state
            .workspace_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if state.workspace_id.as_deref() != Some(workspace_id.as_str()) {
            store::set_workspace(&self.pool, &workspace_id).await?;
        }

        let snapshot = store::snapshot_ids(&self.pool).await?;
        let local = store::local_document(&self.pool, &workspace_id, &state.device_id).await?;
        let dirty_at_start = state.dirty;

        let remote_meta = match self.drive.metadata().await {
            Ok(meta) => meta,
            Err(error) => return self.record_drive_failure(&error).await,
        };

        /* ------------------------------------------------- no cloud file yet */
        let Some(meta) = remote_meta else {
            let upload = match decision {
                Some(MigrationChoice::UseCloud) | Some(MigrationChoice::Merge) => {
                    return Ok(SyncReport::new(
                        SyncStatus::Failed,
                        "No cloud workspace exists yet, so there is nothing to merge or adopt.",
                    ));
                }
                Some(MigrationChoice::UploadLocal) => true,
                None => local.communities.is_empty() || state.base_document.is_some(),
            };

            if !upload {
                let message = format!(
                    "This device has {} saved record(s) and the cloud workspace does not exist yet. Confirm whether to create it from this device.",
                    local.record_count()
                );
                store::set_status(&self.pool, "migration_required", Some(&message)).await?;
                return Ok(SyncReport::new(SyncStatus::MigrationRequired, message)
                    .with_records(local.record_count()));
            }

            if state.base_document.is_none() && !local.communities.is_empty() {
                store::write_snapshot(&self.snapshots_dir(), "before-first-upload", &local)?;
            }

            let mut document = self.prepare_document(local, &state);
            document.generation = state.base().map(|base| base.generation).unwrap_or(0) + 1;
            let new_meta = match self.drive.upload(None, &document, None).await {
                Ok(meta) => meta,
                Err(error) => return self.record_drive_failure(&error).await,
            };
            return self
                .finish_upload(&document, &new_meta, dirty_at_start, true)
                .await;
        };

        /* ------------------------------------------- explicit risky choices */
        if decision == Some(MigrationChoice::UploadLocal) && state.base_document.is_none() {
            return Ok(SyncReport::new(
                SyncStatus::Failed,
                "The cloud workspace already has data. Choose merge or use cloud data instead; uploading would replace it.",
            ));
        }

        /* ------------------------------------------------------- first sync */
        if state.base_document.is_none() {
            match decision {
                None => {
                    let message = format!(
                        "This device has {} saved record(s) and Google Drive already has {} record(s). Choose whether to merge them or use the cloud copy.",
                        local.record_count(),
                        self.peek_remote_count(&meta).await
                    );
                    store::set_status(&self.pool, "migration_required", Some(&message)).await?;
                    return Ok(SyncReport::new(SyncStatus::MigrationRequired, message)
                        .with_records(local.record_count()));
                }
                Some(MigrationChoice::UseCloud) => {
                    let (remote, remote_meta) = match self.download_validated(&meta.file_id).await {
                        Ok(Ok(value)) => value,
                        Ok(Err(report)) => return Ok(report),
                        Err(error) => return self.record_drive_failure(&error).await,
                    };
                    store::write_snapshot(&self.snapshots_dir(), "before-use-cloud", &local)?;
                    store::apply_document(&self.pool, &remote, &snapshot).await?;
                    store::clear_conflicts(&self.pool).await?;
                    let report = self
                        .finish_upload(&remote, &remote_meta, dirty_at_start, false)
                        .await?;
                    return Ok(SyncReport {
                        status: report.status,
                        message: "Using the cloud workspace. A snapshot of this device's previous data was kept.".to_string(),
                        conflicts: 0,
                        records: remote.record_count(),
                        uploaded: false,
                    });
                }
                Some(MigrationChoice::Merge) => {}
                Some(MigrationChoice::UploadLocal) => unreachable!(),
            }
        }

        /* ------------------------------------------------------- no-op path */
        let base = state.base().unwrap_or_else(|| {
            CloudDocument::empty(&workspace_id, &state.device_id, &store::now())
        });
        if state.base_document.is_some()
            && !state.is_dirty()
            && state.base_revision.as_deref() == Some(meta.revision.as_str())
            && same_content(&local, &base)
        {
            store::set_status(&self.pool, "synced", None).await?;
            return Ok(SyncReport {
                status: SyncStatus::Synced,
                message: "Already up to date.".to_string(),
                conflicts: 0,
                records: local.record_count(),
                uploaded: false,
            });
        }

        let (remote, meta) = match self.download_validated(&meta.file_id).await {
            Ok(Ok(value)) => value,
            Ok(Err(report)) => return Ok(report),
            Err(error) => return self.record_drive_failure(&error).await,
        };

        if state.base_document.is_none() {
            // Explicit merge on a fresh device: keep both sides and surface
            // field conflicts instead of guessing.
            store::write_snapshot(&self.snapshots_dir(), "before-first-merge", &local)?;
            store::write_snapshot(&self.snapshots_dir(), "cloud-before-first-merge", &remote)?;
        }

        let merged = merge_documents_resolved(&base, &local, &remote, resolutions);
        if !merged.conflicts.is_empty() {
            self.record_conflicts(&merged.conflicts).await?;
            let document = self.prepare_document(merged.document, &state);
            store::apply_document(&self.pool, &document, &snapshot).await?;
            let count = merged.conflicts.len();
            store::set_status(
                &self.pool,
                "conflict",
                Some("Two devices changed the same field. Choose which value to keep."),
            )
            .await?;
            return Ok(SyncReport::new(
                SyncStatus::Conflict,
                "Changes conflict. Both values were preserved and nothing was uploaded.",
            )
            .with_conflicts(count));
        }

        store::clear_conflicts(&self.pool).await?;

        // Guard against an accidental empty workspace replacing real cloud data.
        if merged.document.communities.is_empty()
            && !remote.communities.is_empty()
            && local.communities.is_empty()
            && base.communities.is_empty()
        {
            let message =
                "Refusing to upload an empty workspace over a cloud document that has records.";
            store::set_status(&self.pool, "failed", Some(message)).await?;
            return Ok(SyncReport::new(SyncStatus::Failed, message));
        }

        let document = self.prepare_document(merged.document, &state);
        store::apply_document(&self.pool, &document, &snapshot).await?;

        // If the merged result already matches the cloud document, there is
        // nothing to upload: record the base and finish.
        if same_content(&document, &remote) {
            return self
                .finish_upload(&document, &meta, dirty_at_start, false)
                .await;
        }

        let mut meta = meta;
        self.upload_with_retries(
            &base,
            &mut meta,
            document,
            &state,
            dirty_at_start,
            resolutions,
        )
        .await
    }

    /// Uploads with a bounded number of re-merge attempts when another device
    /// changed the cloud document in the meantime.
    async fn upload_with_retries(
        &self,
        base: &CloudDocument,
        meta: &mut RemoteMeta,
        document: CloudDocument,
        state: &SyncState,
        dirty_at_start: i64,
        resolutions: &[Resolution],
    ) -> anyhow::Result<SyncReport> {
        let mut document = document;
        for attempt in 0..3 {
            match self
                .drive
                .upload(Some(&meta.file_id), &document, Some(meta))
                .await
            {
                Ok(new_meta) => {
                    return self
                        .finish_upload(&document, &new_meta, dirty_at_start, true)
                        .await;
                }
                Err(DriveError::Conflict) if attempt < 2 => {
                    let (remote, fresh) = match self.download_validated(&meta.file_id).await {
                        Ok(Ok(value)) => value,
                        Ok(Err(report)) => return Ok(report),
                        Err(error) => return self.record_drive_failure(&error).await,
                    };
                    let local =
                        store::local_document(&self.pool, &document.workspace_id, &state.device_id)
                            .await?;
                    let snapshot = store::snapshot_ids(&self.pool).await?;
                    let retry = merge_documents_resolved(base, &local, &remote, resolutions);
                    if !retry.conflicts.is_empty() {
                        self.record_conflicts(&retry.conflicts).await?;
                        let retry_document = self.prepare_document(retry.document, state);
                        store::apply_document(&self.pool, &retry_document, &snapshot).await?;
                        store::set_status(
                            &self.pool,
                            "conflict",
                            Some("Two devices changed the same field. Choose which value to keep."),
                        )
                        .await?;
                        return Ok(SyncReport::new(
                            SyncStatus::Conflict,
                            "Changes conflict. Both values were preserved and nothing was uploaded.",
                        )
                        .with_conflicts(retry.conflicts.len()));
                    }
                    document = self.prepare_document(retry.document, state);
                    store::apply_document(&self.pool, &document, &snapshot).await?;
                    *meta = fresh;
                }
                Err(DriveError::Conflict) => {
                    let message = "The cloud document kept changing. Nothing was overwritten; try Sync now again.";
                    store::set_status(&self.pool, "failed", Some(message)).await?;
                    return Ok(SyncReport::new(SyncStatus::Failed, message));
                }
                Err(error) => return self.record_drive_failure(&error).await,
            }
        }
        unreachable!("the loop always returns")
    }

    async fn record_conflicts(&self, conflicts: &[FieldConflict]) -> anyhow::Result<()> {
        store::replace_conflicts(&self.pool, conflicts).await
    }

    /// Reads the remote record count for the migration prompt without applying
    /// anything. A failure here is not fatal; the prompt still appears.
    async fn peek_remote_count(&self, meta: &RemoteMeta) -> usize {
        match self.download_validated(&meta.file_id).await {
            Ok(Ok((document, _))) => document.record_count(),
            _ => 0,
        }
    }

    /// The document this device would upload right now, for export.
    pub async fn export_document(&self) -> anyhow::Result<CloudDocument> {
        let state = store::load_state(&self.pool).await?;
        let workspace = state
            .workspace_id
            .clone()
            .unwrap_or_else(|| "local".to_string());
        store::local_document(&self.pool, &workspace, &state.device_id).await
    }

    pub fn list_snapshots(&self) -> std::io::Result<Vec<store::SnapshotInfo>> {
        store::list_snapshots(&self.snapshots_dir())
    }

    /// Replaces local records with a snapshot after preserving the current
    /// state. The next sync merges the restored data with the cloud copy.
    pub async fn restore_snapshot(&self, name: &str) -> anyhow::Result<SyncReport> {
        let dir = self.snapshots_dir();
        let document = store::read_snapshot(&dir, name)?;
        let state = store::load_state(&self.pool).await?;
        let workspace = state
            .workspace_id
            .clone()
            .unwrap_or_else(|| document.workspace_id.clone());
        let current = store::local_document(&self.pool, &workspace, &state.device_id).await?;
        store::write_snapshot(&dir, "before-restore", &current)?;
        let snapshot = store::snapshot_ids(&self.pool).await?;
        store::apply_document(&self.pool, &document, &snapshot).await?;
        store::set_dirty(&self.pool, state.dirty.max(1)).await?;
        Ok(self.sync_now().await)
    }
}

/// Content comparison used for the "already up to date" decision. Metadata and
/// derived scores are ignored; everything the user can edit is compared.
pub fn same_content(left: &CloudDocument, right: &CloudDocument) -> bool {
    if left.settings != right.settings {
        return false;
    }
    if left.discovery_runs.len() != right.discovery_runs.len() {
        return false;
    }
    if left.communities.len() != right.communities.len() {
        return false;
    }
    let mut left_runs: Vec<&str> = left
        .discovery_runs
        .iter()
        .map(|run| run.id.as_str())
        .collect();
    let mut right_runs: Vec<&str> = right
        .discovery_runs
        .iter()
        .map(|run| run.id.as_str())
        .collect();
    left_runs.sort_unstable();
    right_runs.sort_unstable();
    if left_runs != right_runs {
        return false;
    }
    for community in &left.communities {
        let Some(other) = right.community(&community.id) else {
            return false;
        };
        let mut a = community.clone();
        let mut b = other.clone();
        a.relevance_score = 0;
        b.relevance_score = 0;
        a.score_breakdown.clear();
        b.score_breakdown.clear();
        if a != b {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::document::CloudCommunity;
    use std::sync::Mutex;

    const TS: &str = "2026-01-01T00:00:00Z";

    fn record(id: &str, name: &str) -> CloudCommunity {
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
            created_at: TS.to_string(),
            updated_at: TS.to_string(),
            sources: vec![],
        }
    }

    fn remote_doc(records: Vec<CloudCommunity>) -> CloudDocument {
        let mut doc = CloudDocument::empty("remote-workspace", "remote-device", TS);
        doc.communities = records;
        doc
    }

    #[derive(Default)]
    struct FakeState {
        bytes: Option<Vec<u8>>,
        revision: u64,
        uploads: usize,
        auth_expired: bool,
        offline: bool,
        conflict_with: Option<Vec<u8>>,
    }

    struct FakeDrive {
        state: Mutex<FakeState>,
    }

    impl FakeDrive {
        fn new() -> Self {
            Self {
                state: Mutex::new(FakeState::default()),
            }
        }

        fn set_document(&self, document: &CloudDocument) {
            let mut state = self.state.lock().unwrap();
            state.bytes = Some(document.to_bytes().unwrap());
            state.revision += 1;
        }

        fn set_bytes(&self, bytes: &[u8]) {
            let mut state = self.state.lock().unwrap();
            state.bytes = Some(bytes.to_vec());
            state.revision += 1;
        }

        fn document(&self) -> Option<CloudDocument> {
            let state = self.state.lock().unwrap();
            state
                .bytes
                .as_deref()
                .and_then(|bytes| CloudDocument::from_bytes(bytes).ok())
        }

        fn bytes(&self) -> Option<Vec<u8>> {
            self.state.lock().unwrap().bytes.clone()
        }

        fn uploads(&self) -> usize {
            self.state.lock().unwrap().uploads
        }

        fn set_auth_expired(&self, value: bool) {
            self.state.lock().unwrap().auth_expired = value;
        }

        fn set_offline(&self, value: bool) {
            self.state.lock().unwrap().offline = value;
        }

        fn set_conflict_with(&self, document: &CloudDocument) {
            self.state.lock().unwrap().conflict_with = Some(document.to_bytes().unwrap());
        }

        fn meta(&self) -> Option<RemoteMeta> {
            let state = self.state.lock().unwrap();
            state.bytes.as_ref().map(|_| RemoteMeta {
                file_id: "file-1".to_string(),
                revision: state.revision.to_string(),
                etag: Some(format!("etag-{}", state.revision)),
            })
        }
    }

    #[async_trait]
    impl CloudDrive for FakeDrive {
        async fn metadata(&self) -> Result<Option<RemoteMeta>, DriveError> {
            {
                let state = self.state.lock().unwrap();
                if state.auth_expired {
                    return Err(DriveError::AuthExpired);
                }
                if state.offline {
                    return Err(DriveError::Offline);
                }
            }
            Ok(self.meta())
        }

        async fn download(&self, _file_id: &str) -> Result<(Vec<u8>, RemoteMeta), DriveError> {
            let state = self.state.lock().unwrap();
            if state.auth_expired {
                return Err(DriveError::AuthExpired);
            }
            if state.offline {
                return Err(DriveError::Offline);
            }
            let Some(bytes) = state.bytes.clone() else {
                return Err(DriveError::NotFound);
            };
            Ok((
                bytes,
                RemoteMeta {
                    file_id: "file-1".to_string(),
                    revision: state.revision.to_string(),
                    etag: Some(format!("etag-{}", state.revision)),
                },
            ))
        }

        async fn upload(
            &self,
            _file_id: Option<&str>,
            document: &CloudDocument,
            expected: Option<&RemoteMeta>,
        ) -> Result<RemoteMeta, DriveError> {
            let mut state = self.state.lock().unwrap();
            if state.auth_expired {
                return Err(DriveError::AuthExpired);
            }
            if state.offline {
                return Err(DriveError::Offline);
            }
            if let Some(surprise) = state.conflict_with.take() {
                state.bytes = Some(surprise);
                state.revision += 1;
                return Err(DriveError::Conflict);
            }
            if let Some(expected) = expected {
                if state.revision.to_string() != expected.revision {
                    return Err(DriveError::Conflict);
                }
            }
            state.bytes = Some(document.to_bytes().unwrap());
            state.revision += 1;
            state.uploads += 1;
            Ok(RemoteMeta {
                file_id: "file-1".to_string(),
                revision: state.revision.to_string(),
                etag: Some(format!("etag-{}", state.revision)),
            })
        }
    }

    async fn new_pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("scout-sync-{}.db", uuid::Uuid::new_v4()));
        let url = format!(
            "sqlite://{}?mode=rwc",
            path.to_string_lossy().replace('\\', "/")
        );
        crate::db::connect(&url).await.expect("test database")
    }

    fn data_dir() -> PathBuf {
        std::env::temp_dir().join(format!("scout-sync-data-{}", uuid::Uuid::new_v4()))
    }

    async fn harness() -> (Arc<SyncEngine>, Arc<FakeDrive>, SqlitePool) {
        let pool = new_pool().await;
        let fake = Arc::new(FakeDrive::new());
        let drive: Arc<dyn CloudDrive> = fake.clone();
        let engine = SyncEngine::new(pool.clone(), drive, data_dir());
        (engine, fake, pool)
    }

    async fn insert_community(pool: &SqlitePool, id: &str, name: &str) {
        sqlx::query(
            "INSERT INTO communities (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(TS)
        .bind(TS)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn set_field(pool: &SqlitePool, id: &str, sql: &str) {
        sqlx::query(&format!("UPDATE communities SET {sql} WHERE id = ?"))
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn record_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM communities")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn state_status(pool: &SqlitePool) -> String {
        sqlx::query_scalar("SELECT status FROM sync_state WHERE id = 1")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /* ------------------------------------------------ basic sync behaviour */

    #[tokio::test]
    async fn empty_local_database_creates_the_cloud_workspace() {
        let (engine, drive, pool) = harness().await;
        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::Synced);
        assert!(report.uploaded);
        assert!(drive.document().is_some());
        assert_eq!(state_status(&pool).await, "synced");
    }

    #[tokio::test]
    async fn local_records_require_explicit_migration_before_upload() {
        let (engine, drive, pool) = harness().await;
        insert_community(&pool, "id-local", "Local community").await;
        drive.set_document(&remote_doc(vec![record("id-remote", "Remote community")]));

        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::MigrationRequired);
        // The cloud copy is untouched.
        assert_eq!(drive.document().unwrap().communities.len(), 1);

        let report = engine.migrate(MigrationChoice::Merge).await;
        assert_eq!(report.status, SyncStatus::Synced);
        assert_eq!(drive.document().unwrap().communities.len(), 2);
        assert_eq!(record_count(&pool).await, 2);
        let names: Vec<String> = drive
            .document()
            .unwrap()
            .communities
            .into_iter()
            .map(|community| community.name)
            .collect();
        assert!(names.contains(&"Local community".to_string()));
        assert!(names.contains(&"Remote community".to_string()));
    }

    #[tokio::test]
    async fn using_the_cloud_copy_is_an_explicit_choice_and_snapshots_local_data() {
        let (engine, drive, pool) = harness().await;
        insert_community(&pool, "id-local", "Local only").await;
        drive.set_document(&remote_doc(vec![record("id-remote", "From the cloud")]));

        let report = engine.migrate(MigrationChoice::UseCloud).await;
        assert_eq!(report.status, SyncStatus::Synced);
        assert_eq!(record_count(&pool).await, 1);
        let remaining: String = sqlx::query_scalar("SELECT id FROM communities")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, "id-remote");
    }

    #[tokio::test]
    async fn a_local_change_is_uploaded_and_confirmed() {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        insert_community(&pool, "id-1", "Freelancers Hub").await;

        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::Synced);
        assert!(report.uploaded);
        assert_eq!(drive.document().unwrap().communities.len(), 1);
        assert_eq!(record_count(&pool).await, 1);
        assert_eq!(state_status(&pool).await, "synced");
    }

    #[tokio::test]
    async fn a_pull_that_matches_the_cloud_does_not_upload_again() {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        insert_community(&pool, "id-1", "A").await;
        engine.sync_now().await;
        let uploads = drive.uploads();
        drive.set_document(&remote_doc(vec![record("id-1", "A")]));

        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::Synced);
        assert!(!report.uploaded);
        assert_eq!(drive.uploads(), uploads);
    }

    #[tokio::test]
    async fn different_fields_from_two_devices_are_both_kept() {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        insert_community(&pool, "id-1", "Freelancers Hub").await;
        engine.sync_now().await;

        set_field(
            &pool,
            "id-1",
            "review_status = 'suitable', updated_at = '2026-02-01T00:00:00Z'",
        )
        .await;
        let mut remote = drive.document().unwrap();
        let record = remote
            .communities
            .iter_mut()
            .find(|community| community.id == "id-1")
            .unwrap();
        record.user_notes = "Recruitment posts allowed.".to_string();
        record.updated_at = "2026-02-01T00:00:01Z".to_string();
        drive.set_document(&remote);

        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::Synced);
        let remote = drive.document().unwrap();
        let record = remote.community("id-1").unwrap();
        assert_eq!(record.review_status, "suitable");
        assert_eq!(record.user_notes, "Recruitment posts allowed.");
        let local: (String, String) =
            sqlx::query_as("SELECT review_status, user_notes FROM communities WHERE id = 'id-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(local.0, "suitable");
        assert_eq!(local.1, "Recruitment posts allowed.");
    }

    /* --------------------------------------------------------- conflicts */

    async fn conflicting_harness() -> (Arc<SyncEngine>, Arc<FakeDrive>, SqlitePool) {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        insert_community(&pool, "id-1", "Freelancers Hub").await;
        engine.sync_now().await;

        set_field(
            &pool,
            "id-1",
            "review_status = 'suitable', updated_at = '2026-02-01T00:00:00Z'",
        )
        .await;
        let mut remote = drive.document().unwrap();
        remote
            .communities
            .iter_mut()
            .find(|community| community.id == "id-1")
            .unwrap()
            .review_status = "unsuitable".to_string();
        drive.set_document(&remote);
        (engine, drive, pool)
    }

    #[tokio::test]
    async fn conflicting_values_are_preserved_and_nothing_is_uploaded() {
        let (engine, drive, pool) = conflicting_harness().await;
        let uploads = drive.uploads();
        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::Conflict);
        assert_eq!(report.conflicts, 1);
        assert_eq!(drive.uploads(), uploads);
        assert_eq!(state_status(&pool).await, "conflict");

        let conflicts = engine.conflicts().await.unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "review_status");
        assert_eq!(conflicts[0].local_value.as_deref(), Some("suitable"));
        assert_eq!(conflicts[0].remote_value.as_deref(), Some("unsuitable"));

        // Local value is still visible while the conflict is open.
        let local: String =
            sqlx::query_scalar("SELECT review_status FROM communities WHERE id = 'id-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(local, "suitable");
    }

    #[tokio::test]
    async fn resolving_with_the_remote_value_uploads_the_choice() {
        let (engine, drive, pool) = conflicting_harness().await;
        engine.sync_now().await;
        let conflict = engine.conflicts().await.unwrap().remove(0);

        let report = engine.resolve_conflict(&conflict.id, true).await.unwrap();
        assert_eq!(report.status, SyncStatus::Synced);
        assert!(engine.conflicts().await.unwrap().is_empty());
        let remote = drive.document().unwrap();
        assert_eq!(
            remote.community("id-1").unwrap().review_status,
            "unsuitable"
        );
        let local: String =
            sqlx::query_scalar("SELECT review_status FROM communities WHERE id = 'id-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(local, "unsuitable");
    }

    #[tokio::test]
    async fn resolving_with_the_local_value_uploads_the_choice() {
        let (engine, drive, pool) = conflicting_harness().await;
        engine.sync_now().await;
        let conflict = engine.conflicts().await.unwrap().remove(0);

        let report = engine.resolve_conflict(&conflict.id, false).await.unwrap();
        assert_eq!(report.status, SyncStatus::Synced);
        let remote = drive.document().unwrap();
        assert_eq!(remote.community("id-1").unwrap().review_status, "suitable");
        let local: String =
            sqlx::query_scalar("SELECT review_status FROM communities WHERE id = 'id-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(local, "suitable");
    }

    #[tokio::test]
    async fn concurrent_deletion_and_modification_is_never_silent() {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        insert_community(&pool, "id-1", "A").await;
        engine.sync_now().await;

        // Local device deletes.
        sqlx::query("DELETE FROM communities WHERE id = 'id-1'")
            .execute(&pool)
            .await
            .unwrap();
        // Other device modifies.
        let mut remote = drive.document().unwrap();
        remote
            .communities
            .iter_mut()
            .find(|community| community.id == "id-1")
            .unwrap()
            .user_notes = "still relevant".to_string();
        drive.set_document(&remote);

        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::Conflict);
        let conflicts = engine.conflicts().await.unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "__deleted__");
        assert_eq!(record_count(&pool).await, 1);

        // Keep the record.
        let report = engine
            .resolve_conflict(&conflicts[0].id, true)
            .await
            .unwrap();
        assert_eq!(report.status, SyncStatus::Synced);
        assert_eq!(record_count(&pool).await, 1);
        let notes: String =
            sqlx::query_scalar("SELECT user_notes FROM communities WHERE id = 'id-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(notes, "still relevant");
    }

    #[tokio::test]
    async fn choosing_deletion_on_a_deletion_conflict_is_applied() {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        insert_community(&pool, "id-1", "A").await;
        engine.sync_now().await;
        sqlx::query("DELETE FROM communities WHERE id = 'id-1'")
            .execute(&pool)
            .await
            .unwrap();
        let mut remote = drive.document().unwrap();
        remote
            .communities
            .iter_mut()
            .find(|community| community.id == "id-1")
            .unwrap()
            .user_notes = "changed".to_string();
        drive.set_document(&remote);

        engine.sync_now().await;
        let conflicts = engine.conflicts().await.unwrap();
        let report = engine
            .resolve_conflict(&conflicts[0].id, false)
            .await
            .unwrap();
        assert_eq!(report.status, SyncStatus::Synced);
        assert_eq!(record_count(&pool).await, 0);
        assert!(drive.document().unwrap().community("id-1").is_none());
    }

    /* ------------------------------------------- failures, retries, integrity */

    #[tokio::test]
    async fn a_malformed_cloud_document_is_never_overwritten() {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        insert_community(&pool, "id-1", "A").await;
        engine.sync_now().await;
        drive.set_bytes(b"{ this is not json");

        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::Failed);
        assert_eq!(drive.bytes().unwrap(), b"{ this is not json");
        assert_eq!(record_count(&pool).await, 1);
        let recovery = engine.recovery_dir();
        assert!(std::fs::read_dir(&recovery).unwrap().next().is_some());
    }

    #[tokio::test]
    async fn an_unknown_schema_version_is_refused() {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        let mut value = serde_json::to_value(drive.document().unwrap()).unwrap();
        value["schema_version"] = serde_json::json!(999);
        drive.set_bytes(&serde_json::to_vec(&value).unwrap());

        let report = engine.sync_now().await;
        // No base exists yet on this workspace? It does, from the initial sync.
        assert_eq!(report.status, SyncStatus::Failed);
        let bytes = drive.bytes().unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("999"));
        assert_eq!(record_count(&pool).await, 0);
    }

    #[tokio::test]
    async fn auth_expiry_keeps_local_changes() {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        insert_community(&pool, "id-1", "A").await;
        drive.set_auth_expired(true);

        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::AuthExpired);
        assert_eq!(record_count(&pool).await, 1);
        assert_eq!(state_status(&pool).await, "auth_expired");
    }

    #[tokio::test]
    async fn offline_mode_keeps_local_changes_pending() {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        insert_community(&pool, "id-1", "A").await;
        drive.set_offline(true);

        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::Offline);
        let state = store::load_state(&pool).await.unwrap();
        assert!(state.is_dirty());
        assert_eq!(record_count(&pool).await, 1);
    }

    #[tokio::test]
    async fn a_upload_conflict_re_downloads_merges_and_retries() {
        let (engine, drive, pool) = harness().await;
        engine.sync_now().await;
        insert_community(&pool, "id-1", "A").await;
        engine.sync_now().await;

        set_field(
            &pool,
            "id-1",
            "user_notes = 'local note', updated_at = '2026-02-01T00:00:00Z'",
        )
        .await;

        // The other device writes between our download and our upload.
        let mut remote = drive.document().unwrap();
        let record = remote
            .communities
            .iter_mut()
            .find(|community| community.id == "id-1")
            .unwrap();
        record.review_status = "suitable".to_string();
        record.updated_at = "2026-02-01T00:00:01Z".to_string();
        drive.set_conflict_with(&remote);

        let uploads_before = drive.uploads();
        let report = engine.sync_now().await;
        assert_eq!(report.status, SyncStatus::Synced);
        let remote = drive.document().unwrap();
        let record = remote.community("id-1").unwrap();
        assert_eq!(record.user_notes, "local note");
        assert_eq!(record.review_status, "suitable");
        assert_eq!(drive.uploads(), uploads_before + 1);
    }

    #[tokio::test]
    async fn an_invalid_local_change_is_not_applied_when_resolving() {
        let (engine, drive, pool) = conflicting_harness().await;
        engine.sync_now().await;
        let conflict = engine.conflicts().await.unwrap().remove(0);
        assert!(engine.resolve_conflict("missing-id", false).await.is_err());
        // The real conflict is still resolvable afterwards.
        assert!(
            engine
                .resolve_conflict(&conflict.id, false)
                .await
                .unwrap()
                .status
                == SyncStatus::Synced
        );
        assert!(record_count(&pool).await >= 1);
        assert!(drive.document().is_some());
    }

    #[test]
    fn same_content_ignores_metadata_and_scores() {
        let mut left = remote_doc(vec![record("id-1", "A")]);
        let mut right = left.clone();
        right.updated_at = "2026-05-05T00:00:00Z".to_string();
        right.updated_by = "other".to_string();
        right.communities[0].relevance_score = 99;
        assert!(same_content(&left, &right));
        right.communities[0].user_notes = "different".to_string();
        assert!(!same_content(&left, &right));
        left.settings.insert("a".to_string(), "b".to_string());
        assert!(!same_content(&left, &right));
    }
}
