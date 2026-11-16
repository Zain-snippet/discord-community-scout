//! Error type that renders as a stable JSON envelope.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    Conflict(String),

    /// An upstream source could not be reached or refused the request. The
    /// message is written for the user and never includes internal detail.
    #[error("{0}")]
    Upstream(String),

    #[error("database error")]
    Database(#[from] sqlx::Error),

    #[error("internal error")]
    Internal(anyhow::Error),
}

#[allow(dead_code)]
impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.clone()),
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m.clone()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone()),
            AppError::Upstream(m) => (StatusCode::BAD_GATEWAY, "upstream_error", m.clone()),
            AppError::Database(err) => {
                // Log the detail, never return it to the client.
                tracing::error!(error = %err, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    "A database error occurred.".to_string(),
                )
            }
            AppError::Internal(err) => {
                tracing::error!(error = %err, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "An internal error occurred.".to_string(),
                )
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.parts();
        let body = json!({ "error": { "code": code, "message": message } });
        (status, Json(body)).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        AppError::Internal(value)
    }
}

#[allow(dead_code)]
pub type AppResult<T> = Result<T, AppError>;
