//! Router assembly and shared extractors.

pub mod communities;
pub mod csv;
pub mod dashboard;
pub mod discovery;
pub mod health;
pub mod messages;

use std::sync::Arc;

use axum::{
    extract::{FromRequest, Request},
    routing::{get, patch, post},
    Router,
};
use serde::de::DeserializeOwned;
use sqlx::SqlitePool;
use tower_http::{
    compression::CompressionLayer,
    limit::RequestBodyLimitLayer,
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};

use crate::{config::Config, discovery::SourceCache, error::AppError};

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    /// Shared HTTP client for the discovery connector, with timeouts applied.
    pub http: reqwest::Client,
    /// In-memory cache of fetched source documents.
    pub cache: SourceCache,
}

/// JSON extractor that reports failures through the same error envelope as the
/// rest of the API, instead of axum's plain-text rejection.
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(request, state).await {
            Ok(axum::Json(value)) => Ok(ApiJson(value)),
            Err(rejection) => Err(AppError::bad_request(rejection.body_text())),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let static_files = ServeDir::new("static").fallback(ServeFile::new("static/index.html"));

    Router::new()
        .nest("/api", api_router())
        // Served explicitly so crawlers get a real robots file instead of the SPA
        // fallback. The UI is public; the JSON API is not meant for indexing.
        .route(
            "/robots.txt",
            get(|| async {
                (
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "text/plain; charset=utf-8",
                    )],
                    "User-agent: *\nDisallow: /api/\n",
                )
            }),
        )
        .fallback_service(static_files)
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
        .layer(CompressionLayer::new())
        // The UI has no build step, so a stale cached app.js would silently run
        // old code. Revalidation keeps clients current at negligible cost.
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        ))
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
        .route("/health", get(health::health))
        .route("/dashboard", get(dashboard::dashboard))
        .route("/meta", get(dashboard::meta))
        .route(
            "/communities",
            get(communities::list).post(communities::create),
        )
        .route(
            "/communities/{id}",
            get(communities::get)
                .patch(communities::update)
                .delete(communities::remove),
        )
        .route("/communities/{id}/review", patch(communities::review))
        .route("/csv/preview", axum::routing::post(csv::preview))
        .route("/csv/commit", axum::routing::post(csv::commit))
        .route("/csv/export", get(csv::export))
        .route("/message/draft", post(messages::create_draft))
        .route("/sources", get(discovery::sources))
        .route("/discovery/runs", get(discovery::runs))
        .route("/discovery/search", post(discovery::search))
}
