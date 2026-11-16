//! Router assembly.

pub mod health;

use std::sync::Arc;

use axum::{routing::get, Router};
use sqlx::SqlitePool;
use tower_http::{
    compression::CompressionLayer,
    limit::RequestBodyLimitLayer,
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    // Read by discovery once the connector lands; the cache TTL lives here.
    #[allow(dead_code)]
    pub config: Arc<Config>,
}

pub fn router(state: AppState) -> Router {
    let static_files = ServeDir::new("static").fallback(ServeFile::new("static/index.html"));

    Router::new()
        .route("/api/health", get(health::health))
        .nest("/api", api_router())
        .fallback_service(static_files)
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            axum::http::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::X_FRAME_OPTIONS,
            axum::http::HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::REFERRER_POLICY,
            axum::http::HeaderValue::from_static("no-referrer"),
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn api_router() -> Router<AppState> {
    Router::new()
}
