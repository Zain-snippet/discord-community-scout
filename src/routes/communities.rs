//! Community CRUD, listing and review endpoints.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use super::{ApiJson, AppState};
use crate::{
    error::{AppError, AppResult},
    models::{Community, CommunityInput, ListQuery, Page, ReviewInput},
    repo,
};

#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    /// Destructive actions require an explicit confirmation flag.
    #[serde(default)]
    confirm: bool,
}

pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Page<Community>>> {
    Ok(Json(repo::list(&state.pool, &query).await?))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Community>> {
    Ok(Json(repo::get(&state.pool, &id).await?))
}

pub async fn create(
    State(state): State<AppState>,
    ApiJson(input): ApiJson<CommunityInput>,
) -> AppResult<(StatusCode, Json<Community>)> {
    let community = repo::create_manual(&state.pool, &input).await?;
    Ok((StatusCode::CREATED, Json(community)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<CommunityInput>,
) -> AppResult<Json<Community>> {
    Ok(Json(repo::update(&state.pool, &id, &input).await?))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeleteQuery>,
) -> AppResult<StatusCode> {
    if !query.confirm {
        return Err(AppError::bad_request(
            "Deleting a community requires confirm=true.",
        ));
    }
    repo::delete(&state.pool, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<ReviewInput>,
) -> AppResult<Json<Community>> {
    Ok(Json(repo::review(&state.pool, &id, &input).await?))
}
