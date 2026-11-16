//! Shared data-transfer objects.

use serde::{Deserialize, Serialize};

/// Review lifecycle of a community.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    NotReviewed,
    Suitable,
    Unsuitable,
    NeedsManualCheck,
    Contacted,
}

impl ReviewStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewStatus::NotReviewed => "not_reviewed",
            ReviewStatus::Suitable => "suitable",
            ReviewStatus::Unsuitable => "unsuitable",
            ReviewStatus::NeedsManualCheck => "needs_manual_check",
            ReviewStatus::Contacted => "contacted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "not_reviewed" => ReviewStatus::NotReviewed,
            "suitable" => ReviewStatus::Suitable,
            "unsuitable" => ReviewStatus::Unsuitable,
            "needs_manual_check" => ReviewStatus::NeedsManualCheck,
            "contacted" => ReviewStatus::Contacted,
            _ => return None,
        })
    }

    pub const ALL: [ReviewStatus; 5] = [
        ReviewStatus::NotReviewed,
        ReviewStatus::Suitable,
        ReviewStatus::Unsuitable,
        ReviewStatus::NeedsManualCheck,
        ReviewStatus::Contacted,
    ];
}

/// Whether the community's own published rules permit promotion or recruitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionPolicy {
    Allowed,
    Prohibited,
    Unknown,
}

impl PromotionPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            PromotionPolicy::Allowed => "allowed",
            PromotionPolicy::Prohibited => "prohibited",
            PromotionPolicy::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "allowed" => PromotionPolicy::Allowed,
            "prohibited" => PromotionPolicy::Prohibited,
            "unknown" => PromotionPolicy::Unknown,
            _ => return None,
        })
    }
}

/// A community as returned by the API.
#[derive(Debug, Clone, Serialize)]
pub struct Community {
    pub id: String,
    pub name: String,
    pub description: String,
    pub invite_url: Option<String>,
    pub invite_code: Option<String>,
    pub canonical_url: Option<String>,
    pub primary_source: String,
    pub tags: Vec<String>,
    pub member_count: Option<i64>,
    pub language: Option<String>,
    pub relevance_score: i64,
    pub score_breakdown: Vec<crate::scoring::ScoreFactor>,
    pub review_status: String,
    pub promotion_policy: String,
    pub policy_evidence: Option<String>,
    pub policy_checked_at: Option<String>,
    pub channel_name: Option<String>,
    pub user_notes: String,
    pub contacted_at: Option<String>,
    pub last_checked_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub sources: Vec<Provenance>,
    /// True when the record must not be used for outreach.
    pub unsuitable_for_outreach: bool,
    /// Human-readable reason for the outreach verdict.
    pub outreach_note: String,
}

/// One provenance reference for a community.
#[derive(Debug, Clone, Serialize)]
pub struct Provenance {
    pub source_name: String,
    pub source_external_id: Option<String>,
    pub listing_url: Option<String>,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

/// Payload for creating or updating a community by hand.
#[derive(Debug, Deserialize)]
pub struct CommunityInput {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub invite_url: Option<String>,
    #[serde(default)]
    pub canonical_url: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub user_notes: Option<String>,
    #[serde(default)]
    pub promotion_policy: Option<String>,
    #[serde(default)]
    pub policy_evidence: Option<String>,
    #[serde(default)]
    pub review_status: Option<String>,
}

/// Review decision recorded by the user.
#[derive(Debug, Deserialize)]
pub struct ReviewInput {
    pub review_status: String,
    #[serde(default)]
    pub promotion_policy: Option<String>,
    #[serde(default)]
    pub policy_evidence: Option<String>,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub user_notes: Option<String>,
    /// Set when marking a community as contacted/posted.
    #[serde(default)]
    pub contacted: bool,
}

/// Query parameters for the community list.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub per_page: Option<i64>,
    /// Exclude communities that must not be used for outreach.
    #[serde(default)]
    pub outreach_only: Option<bool>,
}

/// Paginated response envelope.
#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
    pub total_pages: i64,
}

/// Dashboard counters, all read from the database.
#[derive(Debug, Serialize)]
pub struct Dashboard {
    pub total_communities: i64,
    pub awaiting_review: i64,
    pub reviewed: i64,
    pub suitable: i64,
    pub unsuitable: i64,
    pub contacted: i64,
    pub needs_manual_check: i64,
    pub promotion_prohibited: i64,
    pub promotion_allowed: i64,
    pub policy_unknown: i64,
    pub last_activity: Option<LastActivity>,
}

#[derive(Debug, Serialize)]
pub struct LastActivity {
    pub kind: String,
    pub detail: String,
    pub at: String,
}
