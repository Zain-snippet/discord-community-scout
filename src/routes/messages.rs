//! Message draft generation endpoint.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use super::{ApiJson, AppState};
use crate::{
    error::{AppError, AppResult},
    models::PromotionPolicy,
    repo,
    templates::{self, Draft, Tone, Variant},
};

#[derive(Debug, Deserialize)]
pub struct DraftRequest {
    pub community_id: String,
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    /// Extra context the user wants included, kept as a clearly marked note.
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DraftResponse {
    pub community_id: String,
    pub community_name: String,
    pub draft: Draft,
    /// The other length, offered as an alternative.
    pub alternative: Draft,
}

pub async fn create_draft(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<DraftRequest>,
) -> AppResult<Json<DraftResponse>> {
    let community = repo::get(&state.pool, &request.community_id).await?;

    let tone = match request.tone.as_deref() {
        Some(raw) => Tone::parse(raw)
            .ok_or_else(|| AppError::bad_request("tone must be conversational or direct."))?,
        None => Tone::Conversational,
    };

    let variant = match request.variant.as_deref() {
        Some(raw) => Variant::parse(raw)
            .ok_or_else(|| AppError::bad_request("variant must be concise or long."))?,
        None => Variant::Concise,
    };

    let policy =
        PromotionPolicy::parse(&community.promotion_policy).unwrap_or(PromotionPolicy::Unknown);

    let build = |variant: Variant, notes: Option<&str>| {
        templates::draft(&templates::DraftInput {
            community_name: &community.name,
            tags: &community.tags,
            policy,
            policy_evidence: community.policy_evidence.as_deref(),
            channel_name: community.channel_name.as_deref(),
            tone,
            variant,
            user_notes: notes,
        })
    };

    let notes = request
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let draft = build(variant, notes);

    let alternative_variant = match variant {
        Variant::Concise => Variant::Long,
        Variant::Long => Variant::Concise,
    };
    let alternative = build(alternative_variant, notes);

    // Safety net: the templates are tested not to emit unsupported claims, but
    // re-check at runtime so a future template edit cannot silently ship one.
    let claims: Vec<&'static str> = templates::forbidden_claims(&draft.text)
        .into_iter()
        .chain(templates::forbidden_claims(&alternative.text))
        .collect();
    if !claims.is_empty() {
        tracing::error!(claims = ?claims, "refusing to return a draft with unsupported claims");
        return Err(AppError::Internal(anyhow::anyhow!(
            "draft generation produced an unsupported claim"
        )));
    }

    Ok(Json(DraftResponse {
        community_id: community.id,
        community_name: community.name,
        draft,
        alternative,
    }))
}
