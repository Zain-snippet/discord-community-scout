//! Discovery source metadata, run history and search.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use super::{ApiJson, AppState};
use crate::{
    discovery::{self, SearchRequest, SearchResponse},
    error::AppResult,
    sources,
};

#[derive(Serialize)]
pub struct SourceView {
    id: &'static str,
    name: &'static str,
    homepage: &'static str,
    license: &'static str,
    enabled: bool,
    notes: &'static str,
}

#[derive(Serialize)]
pub struct SourcesResponse {
    /// True when at least one source can actually be queried.
    discovery_available: bool,
    /// Sources whose current terms and access permit automated use.
    sources: Vec<SourceView>,
}

/// Lists every source considered in the audit with its current availability, so
/// the UI can show what is integrated and what is not.
pub async fn sources() -> Json<SourcesResponse> {
    let all = sources::registry();
    let discovery_available = all.iter().any(|s| s.enabled);
    Json(SourcesResponse {
        discovery_available,
        sources: all
            .into_iter()
            .map(|s| SourceView {
                id: s.id,
                name: s.name,
                homepage: s.homepage,
                license: s.license,
                enabled: s.enabled,
                notes: s.notes,
            })
            .collect(),
    })
}

#[derive(Deserialize)]
pub struct RunsQuery {
    #[serde(default)]
    limit: Option<i64>,
}

/// Recent discovery runs, newest first.
pub async fn runs(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<RunsQuery>,
) -> AppResult<Json<Vec<discovery::RunSummary>>> {
    let runs = discovery::recent_runs(&state.pool, query.limit.unwrap_or(20)).await?;
    Ok(Json(runs))
}

/// Runs a permitted discovery search and stores what it finds.
pub async fn search(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<SearchRequest>,
) -> AppResult<Json<SearchResponse>> {
    let ctx = discovery::SearchContext {
        pool: &state.pool,
        http: &state.http,
        cache: &state.cache,
        ttl_secs: state.config.cache_ttl_secs,
    };
    Ok(Json(discovery::search(ctx, request).await?))
}
