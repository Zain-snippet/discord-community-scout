//! Google sign-in endpoints.

use axum::{
    extract::{Query, State},
    response::Redirect,
    Json,
};
use serde::Deserialize;

use super::sync::SyncStatus;
use super::AppState;
use crate::error::{AppError, AppResult};
use crate::sync::store;

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub async fn status(State(state): State<AppState>) -> AppResult<Json<SyncStatus>> {
    Ok(Json(SyncStatus {
        google: state.google.status().await,
        sync: state.sync.snapshot().await?,
    }))
}

pub async fn start(State(state): State<AppState>) -> AppResult<Redirect> {
    match state.google.sign_in_url() {
        Ok(url) => Ok(Redirect::to(&url)),
        Err(crate::google::oauth::OAuthError::NotConfigured) => Err(AppError::bad_request(
            "Google sign-in is not configured on this server. Set GOOGLE_CLIENT_ID.",
        )),
        Err(_) => Err(AppError::Internal(anyhow::anyhow!(
            "could not start the Google sign-in flow"
        ))),
    }
}

/// Google redirects here. The response is always a redirect back to the app:
/// the browser never receives tokens, and failures do not echo request detail.
pub async fn callback(
    State(state): State<AppState>,
    Query(query): Query<CallbackQuery>,
) -> Redirect {
    if query.error.is_some() {
        return Redirect::to("/?google=denied");
    }
    let Some(code) = query.code.as_deref() else {
        return Redirect::to("/?google=error");
    };
    let Some(state_value) = query.state.as_deref() else {
        return Redirect::to("/?google=error");
    };

    match state.google.complete_sign_in(code, state_value).await {
        Ok(()) => {
            // Pull the existing workspace immediately; a failure is reported by
            // the sync status endpoint, not by breaking the redirect.
            let report = state.sync.sync_now().await;
            if report.status == crate::sync::engine::SyncStatus::AuthExpired {
                return Redirect::to("/?google=error");
            }
            Redirect::to("/?google=connected")
        }
        Err(error) => {
            tracing::warn!(error = %error, "Google sign-in failed");
            Redirect::to("/?google=error")
        }
    }
}

pub async fn logout(State(state): State<AppState>) -> AppResult<Json<SyncStatus>> {
    state.google.logout().await;
    // Local and cloud data are untouched; only the credential is gone.
    store::set_status(&state.pool, "disconnected", None).await?;
    Ok(Json(SyncStatus {
        google: state.google.status().await,
        sync: state.sync.snapshot().await?,
    }))
}
