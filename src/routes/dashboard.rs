//! Dashboard and metadata endpoints.

use axum::{extract::State, Json};
use serde::Serialize;

use super::AppState;
use crate::{
    error::AppResult,
    models::{Dashboard, PromotionPolicy, ReviewStatus},
    repo,
};

pub async fn dashboard(State(state): State<AppState>) -> AppResult<Json<Dashboard>> {
    Ok(Json(repo::dashboard(&state.pool).await?))
}

#[derive(Serialize)]
pub struct Meta {
    review_statuses: Vec<&'static str>,
    promotion_policies: Vec<&'static str>,
    sources: Vec<SourceCount>,
    tags: Vec<TagCount>,
}

#[derive(Serialize)]
pub struct SourceCount {
    name: String,
    count: i64,
}

#[derive(Serialize)]
pub struct TagCount {
    name: String,
    count: i64,
}

/// Vocabulary and current filter values, so the UI never hard-codes them.
pub async fn meta(State(state): State<AppState>) -> AppResult<Json<Meta>> {
    let sources = repo::source_counts(&state.pool)
        .await?
        .into_iter()
        .map(|(name, count)| SourceCount { name, count })
        .collect();

    let tags = repo::tag_counts(&state.pool)
        .await?
        .into_iter()
        .map(|(name, count)| TagCount { name, count })
        .collect();

    Ok(Json(Meta {
        review_statuses: ReviewStatus::ALL.iter().map(|s| s.as_str()).collect(),
        promotion_policies: vec![
            PromotionPolicy::Allowed.as_str(),
            PromotionPolicy::Prohibited.as_str(),
            PromotionPolicy::Unknown.as_str(),
        ],
        sources,
        tags,
    }))
}
