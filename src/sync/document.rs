//! The cloud document: the single JSON file stored in Google Drive.
//!
//! This module is pure data plus validation. It performs no I/O so it can be
//! tested exhaustively, and it deliberately contains no secrets: tokens and
//! caches never enter the document.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::scoring::ScoreFactor;

/// Bumped only when an older document can no longer be read safely.
pub const SCHEMA_VERSION: u32 = 1;
/// Identifies the document so a user's other app data is never mistaken for ours.
pub const APP_ID: &str = "discord-community-scout";
/// Hard cap on an accepted document. Real data is far smaller; this is a guard
/// against a malformed or hostile file exhausting memory.
pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
/// Bounded history: only the most recent discovery runs are synchronized.
pub const MAX_RUNS: usize = 50;
/// Defensive upper bound on record count.
pub const MAX_COMMUNITIES: usize = 50_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSource {
    pub source_name: String,
    #[serde(default)]
    pub source_external_id: Option<String>,
    #[serde(default)]
    pub listing_url: Option<String>,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudCommunity {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub invite_url: Option<String>,
    #[serde(default)]
    pub invite_code: Option<String>,
    #[serde(default)]
    pub canonical_url: Option<String>,
    #[serde(default = "default_source")]
    pub primary_source: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub member_count: Option<i64>,
    #[serde(default)]
    pub language: Option<String>,
    /// Derived. Recomputed locally after every merge instead of being merged,
    /// so two devices never fight over a number they both derive.
    #[serde(default)]
    pub relevance_score: i64,
    #[serde(default)]
    pub score_breakdown: Vec<ScoreFactor>,
    #[serde(default = "default_review_status")]
    pub review_status: String,
    #[serde(default = "default_policy")]
    pub promotion_policy: String,
    #[serde(default)]
    pub policy_evidence: Option<String>,
    #[serde(default)]
    pub policy_checked_at: Option<String>,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub user_notes: String,
    #[serde(default)]
    pub contacted_at: Option<String>,
    #[serde(default)]
    pub last_checked_at: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    /// Append-only provenance references.
    #[serde(default)]
    pub sources: Vec<CloudSource>,
}

fn default_source() -> String {
    "Manual entry".to_string()
}

fn default_review_status() -> String {
    "not_reviewed".to_string()
}

fn default_policy() -> String {
    "unknown".to_string()
}

/// A discovery run, trimmed to the fields that are useful on another device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRun {
    pub id: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub requested_limit: i64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub result_count: i64,
    #[serde(default)]
    pub saved_count: i64,
    #[serde(default)]
    pub partial: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudDocument {
    pub schema_version: u32,
    pub app: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub updated_by: String,
    #[serde(default)]
    pub communities: Vec<CloudCommunity>,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    #[serde(default)]
    pub discovery_runs: Vec<CloudRun>,
}

/// Why a document was refused. The variants are displayed to the user, so they
/// must not contain file paths or other internal detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentError {
    TooLarge(usize),
    NotJson(String),
    WrongApp(String),
    UnknownSchema(u32),
    EmptyField(String),
    DuplicateId(String),
    InvalidField(String),
    TooManyCommunities(usize),
}

impl std::fmt::Display for DocumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocumentError::TooLarge(size) => write!(
                f,
                "The cloud document is {size} bytes, above the {} byte limit.",
                MAX_DOCUMENT_BYTES
            ),
            DocumentError::NotJson(detail) => write!(f, "The cloud document is not valid JSON: {detail}"),
            DocumentError::WrongApp(app) => write!(
                f,
                "The cloud file belongs to {app:?}, not to Discord Community Scout."
            ),
            DocumentError::UnknownSchema(version) => write!(
                f,
                "The cloud document uses schema version {version}; this build supports version {SCHEMA_VERSION}."
            ),
            DocumentError::EmptyField(field) => write!(f, "{field} must not be empty."),
            DocumentError::DuplicateId(id) => {
                write!(f, "The cloud document contains the id {id} more than once.")
            }
            DocumentError::InvalidField(detail) => write!(f, "The cloud document is invalid: {detail}"),
            DocumentError::TooManyCommunities(count) => write!(
                f,
                "The cloud document contains {count} communities, above the {MAX_COMMUNITIES} limit."
            ),
        }
    }
}

impl std::error::Error for DocumentError {}

impl CloudDocument {
    pub fn empty(workspace_id: &str, device_id: &str, now: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            app: APP_ID.to_string(),
            workspace_id: workspace_id.to_string(),
            generation: 0,
            updated_at: now.to_string(),
            updated_by: device_id.to_string(),
            communities: Vec::new(),
            settings: BTreeMap::new(),
            discovery_runs: Vec::new(),
        }
    }

    /// Parses and validates a downloaded document.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DocumentError> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::TooLarge(bytes.len()));
        }
        // Reject a BOM and empty bodies explicitly: serde would otherwise report
        // a generic parse error that is hard to act on.
        let text = std::str::from_utf8(bytes)
            .map_err(|_| DocumentError::NotJson("the file is not UTF-8".to_string()))?;
        let trimmed = text.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() {
            return Err(DocumentError::NotJson("the file is empty".to_string()));
        }
        let document: CloudDocument =
            serde_json::from_str(trimmed).map_err(|err| DocumentError::NotJson(err.to_string()))?;
        document.validate()?;
        Ok(document)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, DocumentError> {
        serde_json::to_vec(self).map_err(|err| DocumentError::NotJson(err.to_string()))
    }

    /// Validates everything that would make the document unsafe to apply.
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.app != APP_ID {
            return Err(DocumentError::WrongApp(self.app.clone()));
        }
        if self.schema_version != SCHEMA_VERSION {
            return Err(DocumentError::UnknownSchema(self.schema_version));
        }
        if self.workspace_id.trim().is_empty() {
            return Err(DocumentError::EmptyField("workspace_id".to_string()));
        }
        if self.communities.len() > MAX_COMMUNITIES {
            return Err(DocumentError::TooManyCommunities(self.communities.len()));
        }

        let mut seen = std::collections::HashSet::with_capacity(self.communities.len());
        for community in &self.communities {
            if community.id.trim().is_empty() {
                return Err(DocumentError::EmptyField("community id".to_string()));
            }
            if !seen.insert(community.id.as_str()) {
                return Err(DocumentError::DuplicateId(community.id.clone()));
            }
            if community.name.trim().is_empty() {
                return Err(DocumentError::InvalidField(format!(
                    "community {} has an empty name",
                    community.id
                )));
            }
            if crate::models::ReviewStatus::parse(&community.review_status).is_none() {
                return Err(DocumentError::InvalidField(format!(
                    "community {} has an unknown review_status",
                    community.id
                )));
            }
            if crate::models::PromotionPolicy::parse(&community.promotion_policy).is_none() {
                return Err(DocumentError::InvalidField(format!(
                    "community {} has an unknown promotion_policy",
                    community.id
                )));
            }
            for source in &community.sources {
                if source.source_name.trim().is_empty() {
                    return Err(DocumentError::InvalidField(format!(
                        "community {} has a source without a name",
                        community.id
                    )));
                }
            }
        }

        for run in &self.discovery_runs {
            if run.id.trim().is_empty() {
                return Err(DocumentError::EmptyField("discovery run id".to_string()));
            }
        }

        Ok(())
    }

    /// Looks up a record by id.
    pub fn community(&self, id: &str) -> Option<&CloudCommunity> {
        self.communities.iter().find(|c| c.id == id)
    }

    /// Replaces or inserts one record, keeping the list sorted by id so uploads
    /// from two devices are byte-stable when the content is equal.
    #[cfg(test)]
    pub fn put_community(&mut self, record: CloudCommunity) {
        match self.communities.iter_mut().find(|c| c.id == record.id) {
            Some(existing) => *existing = record,
            None => self.communities.push(record),
        }
        self.communities.sort_by(|a, b| a.id.cmp(&b.id));
    }

    pub fn remove_community(&mut self, id: &str) -> Option<CloudCommunity> {
        let index = self.communities.iter().position(|c| c.id == id)?;
        Some(self.communities.remove(index))
    }

    /// Number of records a user would call "saved data".
    pub fn record_count(&self) -> usize {
        self.communities.len()
    }
}

/// Fields that can appear in a conflict, used to apply a chosen value.
#[cfg(test)]
pub const MERGE_FIELDS: [&str; 20] = [
    "name",
    "description",
    "invite_url",
    "invite_code",
    "canonical_url",
    "primary_source",
    "tags",
    "member_count",
    "language",
    "review_status",
    "promotion_policy",
    "policy_evidence",
    "policy_checked_at",
    "channel_name",
    "user_notes",
    "contacted_at",
    "last_checked_at",
    "created_at",
    "updated_at",
    "sources",
];

impl CloudCommunity {
    /// Renders a field as the text stored in a conflict row.
    pub fn field_text(&self, field: &str) -> Option<String> {
        Some(match field {
            "name" => self.name.clone(),
            "description" => self.description.clone(),
            "invite_url" => self.invite_url.clone()?,
            "invite_code" => self.invite_code.clone()?,
            "canonical_url" => self.canonical_url.clone()?,
            "primary_source" => self.primary_source.clone(),
            "tags" => serde_json::to_string(&self.tags).ok()?,
            "member_count" => self.member_count?.to_string(),
            "language" => self.language.clone()?,
            "review_status" => self.review_status.clone(),
            "promotion_policy" => self.promotion_policy.clone(),
            "policy_evidence" => self.policy_evidence.clone()?,
            "policy_checked_at" => self.policy_checked_at.clone()?,
            "channel_name" => self.channel_name.clone()?,
            "user_notes" => self.user_notes.clone(),
            "contacted_at" => self.contacted_at.clone()?,
            "last_checked_at" => self.last_checked_at.clone()?,
            "created_at" => self.created_at.clone(),
            "updated_at" => self.updated_at.clone(),
            "sources" => serde_json::to_string(&self.sources).ok()?,
            _ => return None,
        })
    }

    /// Applies a conflict resolution value. `None` clears an optional field.
    pub fn set_field(&mut self, field: &str, value: Option<&str>) -> Result<(), String> {
        let text = value.unwrap_or("").to_string();
        match field {
            "name" => {
                if text.trim().is_empty() {
                    return Err("name cannot be empty".to_string());
                }
                self.name = text;
            }
            "description" => self.description = text,
            "invite_url" => self.invite_url = value.map(str::to_string),
            "invite_code" => self.invite_code = value.map(str::to_string),
            "canonical_url" => self.canonical_url = value.map(str::to_string),
            "primary_source" => {
                if text.trim().is_empty() {
                    return Err("primary_source cannot be empty".to_string());
                }
                self.primary_source = text;
            }
            "tags" => {
                self.tags = serde_json::from_str(value.unwrap_or("[]"))
                    .map_err(|_| "tags must be a JSON array".to_string())?;
            }
            "member_count" => {
                self.member_count = match value {
                    None | Some("") => None,
                    Some(raw) => Some(
                        raw.parse::<i64>()
                            .map_err(|_| "member_count must be a number".to_string())?,
                    ),
                };
            }
            "language" => self.language = value.map(str::to_string),
            "review_status" => {
                if crate::models::ReviewStatus::parse(&text).is_none() {
                    return Err("unknown review_status".to_string());
                }
                self.review_status = text;
            }
            "promotion_policy" => {
                if crate::models::PromotionPolicy::parse(&text).is_none() {
                    return Err("unknown promotion_policy".to_string());
                }
                self.promotion_policy = text;
            }
            "policy_evidence" => self.policy_evidence = value.map(str::to_string),
            "policy_checked_at" => self.policy_checked_at = value.map(str::to_string),
            "channel_name" => self.channel_name = value.map(str::to_string),
            "user_notes" => self.user_notes = text,
            "contacted_at" => self.contacted_at = value.map(str::to_string),
            "last_checked_at" => self.last_checked_at = value.map(str::to_string),
            "created_at" => self.created_at = text,
            "updated_at" => self.updated_at = text,
            "sources" => {
                self.sources = serde_json::from_str(value.unwrap_or("[]"))
                    .map_err(|_| "sources must be a JSON array".to_string())?;
            }
            other => return Err(format!("unknown field {other}")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CloudDocument {
        let mut doc = CloudDocument::empty("ws-1", "device-1", "2026-01-01T00:00:00Z");
        doc.communities.push(CloudCommunity {
            id: "11111111-1111-4111-8111-111111111111".to_string(),
            name: "Freelancers Hub".to_string(),
            description: "A place for freelancers".to_string(),
            invite_url: Some("https://discord.gg/abc".to_string()),
            invite_code: Some("abc".to_string()),
            canonical_url: None,
            primary_source: "Manual entry".to_string(),
            tags: vec!["freelance".to_string()],
            member_count: Some(1200),
            language: Some("en".to_string()),
            relevance_score: 42,
            score_breakdown: vec![],
            review_status: "not_reviewed".to_string(),
            promotion_policy: "unknown".to_string(),
            policy_evidence: None,
            policy_checked_at: None,
            channel_name: None,
            user_notes: String::new(),
            contacted_at: None,
            last_checked_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            sources: vec![CloudSource {
                source_name: "Manual entry".to_string(),
                source_external_id: None,
                listing_url: None,
                first_seen_at: "2026-01-01T00:00:00Z".to_string(),
                last_seen_at: "2026-01-01T00:00:00Z".to_string(),
            }],
        });
        doc
    }

    #[test]
    fn round_trips_through_json() {
        let doc = sample();
        let bytes = doc.to_bytes().unwrap();
        let parsed = CloudDocument::from_bytes(&bytes).unwrap();
        assert_eq!(doc, parsed);
    }

    #[test]
    fn rejects_unknown_schema_version() {
        let doc = sample();
        let mut value = serde_json::to_value(&doc).unwrap();
        value["schema_version"] = serde_json::json!(99);
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            CloudDocument::from_bytes(&bytes),
            Err(DocumentError::UnknownSchema(99))
        );
    }

    #[test]
    fn rejects_a_different_application() {
        let doc = sample();
        let mut value = serde_json::to_value(&doc).unwrap();
        value["app"] = serde_json::json!("some-other-app");
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(matches!(
            CloudDocument::from_bytes(&bytes),
            Err(DocumentError::WrongApp(_))
        ));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let mut doc = sample();
        let duplicate = doc.communities[0].clone();
        doc.communities.push(duplicate);
        assert!(matches!(doc.validate(), Err(DocumentError::DuplicateId(_))));
    }

    #[test]
    fn rejects_empty_name_and_bad_status() {
        let mut doc = sample();
        doc.communities[0].name = "  ".to_string();
        assert!(doc.validate().is_err());

        let mut doc = sample();
        doc.communities[0].review_status = "maybe".to_string();
        assert!(doc.validate().is_err());

        let mut doc = sample();
        doc.communities[0].promotion_policy = "probably".to_string();
        assert!(doc.validate().is_err());
    }

    #[test]
    fn rejects_empty_and_malformed_bodies() {
        assert!(matches!(
            CloudDocument::from_bytes(b""),
            Err(DocumentError::NotJson(_))
        ));
        assert!(matches!(
            CloudDocument::from_bytes(b"{not json"),
            Err(DocumentError::NotJson(_))
        ));
        assert!(matches!(
            CloudDocument::from_bytes(&[0xff, 0xfe, 0x00]),
            Err(DocumentError::NotJson(_))
        ));
    }

    #[test]
    fn rejects_oversized_documents() {
        let bytes = vec![b'{'; MAX_DOCUMENT_BYTES + 1];
        assert_eq!(
            CloudDocument::from_bytes(&bytes),
            Err(DocumentError::TooLarge(MAX_DOCUMENT_BYTES + 1))
        );
    }

    #[test]
    fn tolerates_a_utf8_bom_and_missing_optional_fields() {
        let mut value = serde_json::to_value(sample()).unwrap();
        // Simulate an older writer that omitted optional fields.
        value["communities"][0]
            .as_object_mut()
            .unwrap()
            .remove("user_notes");
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend(serde_json::to_vec(&value).unwrap());
        let parsed = CloudDocument::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.communities[0].user_notes, "");
    }

    #[test]
    fn put_and_remove_keep_a_stable_order() {
        let mut doc = sample();
        let mut second = doc.communities[0].clone();
        second.id = "00000000-0000-4000-8000-000000000000".to_string();
        second.name = "Another".to_string();
        doc.put_community(second);
        assert_eq!(doc.communities[0].name, "Another");
        let removed = doc.remove_community("00000000-0000-4000-8000-000000000000");
        assert!(removed.is_some());
        assert_eq!(doc.record_count(), 1);
    }

    #[test]
    fn every_merge_field_round_trips_through_field_text() {
        let doc = sample();
        let record = &doc.communities[0];
        for field in MERGE_FIELDS {
            let mut copy = record.clone();
            let text = copy.field_text(field);
            // Optional fields that are None have no text to apply; the setter
            // must accept None for them.
            assert!(copy.set_field(field, text.as_deref()).is_ok(), "{field}");
            if let Some(text) = text {
                assert_eq!(
                    copy.field_text(field).as_deref(),
                    Some(text.as_str()),
                    "{field}"
                );
            }
        }
    }

    #[test]
    fn set_field_rejects_invalid_values() {
        let mut record = sample().communities.remove(0);
        assert!(record.set_field("review_status", Some("maybe")).is_err());
        assert!(record.set_field("member_count", Some("lots")).is_err());
        assert!(record.set_field("name", Some("  ")).is_err());
        assert!(record.set_field("tags", Some("nope")).is_err());
        assert!(record.set_field("unknown", Some("x")).is_err());
        assert!(record.set_field("review_status", Some("suitable")).is_ok());
        assert_eq!(record.review_status, "suitable");
    }
}
