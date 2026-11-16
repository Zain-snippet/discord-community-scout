//! Health endpoint: reports database reachability, not just liveness.

use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;

use super::AppState;

#[derive(Serialize)]
pub struct Health {
    status: &'static str,
    version: &'static str,
    database: &'static str,
}

pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<Health>) {
    let database_ok = sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .is_ok();

    let (status_code, status) = if database_ok {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "degraded")
    };

    (
        status_code,
        Json(Health {
            status,
            version: env!("CARGO_PKG_VERSION"),
            database: if database_ok { "ok" } else { "unavailable" },
        }),
    )
}
