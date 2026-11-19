//! Synchronization endpoints used by the UI.

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use super::{ApiJson, AppState};
use crate::error::{AppError, AppResult};
use crate::google::GoogleStatus;
use crate::sync::engine::{MigrationChoice, SyncReport, SyncSnapshot};
use crate::sync::store::{ConflictRow, SnapshotInfo};

#[derive(Debug, Serialize)]
pub struct SyncStatus {
    pub google: GoogleStatus,
    pub sync: SyncSnapshot,
}

pub async fn status(State(state): State<AppState>) -> AppResult<Json<SyncStatus>> {
    Ok(Json(SyncStatus {
        google: state.google.status().await,
        sync: state.sync.snapshot().await?,
    }))
}

pub async fn run(State(state): State<AppState>) -> Json<SyncReport> {
    Json(state.sync.sync_now().await)
}

pub async fn conflicts(State(state): State<AppState>) -> AppResult<Json<Vec<ConflictRow>>> {
    Ok(Json(state.sync.conflicts().await?))
}

#[derive(Debug, Deserialize)]
pub struct ResolveInput {
    /// `local` keeps this device's value, `remote` keeps the other value.
    pub choice: String,
}

pub async fn resolve(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<ResolveInput>,
) -> AppResult<Json<SyncReport>> {
    let keep_remote = match input.choice.as_str() {
        "local" => false,
        "remote" => true,
        other => {
            return Err(AppError::bad_request(format!(
                "choice must be local or remote, got {other:?}"
            )))
        }
    };
    let report = state.sync.resolve_conflict(&id, keep_remote).await?;
    Ok(Json(report))
}

#[derive(Debug, Deserialize)]
pub struct MigrationInput {
    /// `merge`, `use_cloud` or `upload_local`.
    pub choice: String,
    /// Explicit confirmation for the two lossy choices.
    #[serde(default)]
    pub confirm: bool,
}

pub async fn migrate(
    State(state): State<AppState>,
    ApiJson(input): ApiJson<MigrationInput>,
) -> AppResult<Json<SyncReport>> {
    let choice = match input.choice.as_str() {
        "merge" => MigrationChoice::Merge,
        "use_cloud" => MigrationChoice::UseCloud,
        "upload_local" => MigrationChoice::UploadLocal,
        other => {
            return Err(AppError::bad_request(format!(
                "choice must be merge, use_cloud or upload_local, got {other:?}"
            )))
        }
    };

    // Merging preserves both sides. The other two replace one side, so they
    // require an explicit confirmation flag from the dialog.
    if !matches!(choice, MigrationChoice::Merge) && !input.confirm {
        return Err(AppError::bad_request(
            "This choice replaces data on one side. Resend with confirm=true after the snapshot notice.",
        ));
    }

    Ok(Json(state.sync.migrate(choice).await))
}

/// Downloads the local copy of the cloud document (no tokens or secrets).
pub async fn export(State(state): State<AppState>) -> AppResult<Response> {
    let document = state.sync.export_document().await?;
    let bytes = document.to_bytes().map_err(|error| {
        AppError::Internal(anyhow::anyhow!("cannot serialise the export: {error}"))
    })?;
    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "application/json; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"scout-data.json\"".to_string(),
            ),
        ],
        bytes,
    )
        .into_response())
}

pub async fn snapshots(State(state): State<AppState>) -> AppResult<Json<Vec<SnapshotInfo>>> {
    let snapshots = state
        .sync
        .list_snapshots()
        .map_err(|error| AppError::Internal(anyhow::anyhow!("cannot list snapshots: {error}")))?;
    Ok(Json(snapshots))
}
#[derive(Debug, Deserialize)]
pub struct RestoreInput {
    #[serde(default)]
    pub confirm: bool,
}

pub async fn restore(
    State(state): State<AppState>,
    Path(name): Path<String>,
    ApiJson(input): ApiJson<RestoreInput>,
) -> AppResult<Json<SyncReport>> {
    if !input.confirm {
        return Err(AppError::bad_request(
            "Restoring a snapshot replaces current records. Resend with confirm=true.",
        ));
    }
    let report = state.sync.restore_snapshot(&name).await?;
    Ok(Json(report))
}
