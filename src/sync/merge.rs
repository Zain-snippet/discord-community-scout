//! Three-way merge between the last synced document (base), this device's data
//! (local) and the current cloud document (remote).
//!
//! Pure functions, no I/O. The rules are the ones the product promises:
//!
//! 1. Different fields changed -> both changes kept.
//! 2. Same field changed to the same value -> accepted silently.
//! 3. Same field changed to different values -> conflict, local value kept
//!    pending a user decision, both values preserved.
//! 4. One side changed, the other still equals base -> the changed value wins.
//! 5. Deletion versus modification -> never silently deletes; conflict.

use std::collections::{BTreeMap, BTreeSet};

use crate::sync::document::{CloudCommunity, CloudDocument, CloudRun, CloudSource, MAX_RUNS};

/// Pseudo community id for workspace-level (settings) conflicts.
pub const SETTINGS_SCOPE: &str = "__settings__";
/// Field name used when one device deleted a record the other modified.
pub const DELETION_FIELD: &str = "__deleted__";
pub const DELETED_VALUE: &str = "deleted";
pub const PRESENT_VALUE: &str = "present";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldConflict {
    pub community_id: String,
    pub community_name: String,
    pub field: String,
    pub base_value: Option<String>,
    pub local_value: Option<String>,
    pub remote_value: Option<String>,
    /// Set only for deletion conflicts, so a later choice can restore the exact
    /// record that the other device still had.
    pub local_record: Option<String>,
    pub remote_record: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DocumentMerge {
    pub document: CloudDocument,
    pub conflicts: Vec<FieldConflict>,
}

fn conflict_id(community_id: &str, field: &str) -> String {
    format!("{community_id}::{field}")
}

impl FieldConflict {
    pub fn id(&self) -> String {
        conflict_id(&self.community_id, &self.field)
    }

    #[cfg(test)]
    pub fn is_deletion(&self) -> bool {
        self.field == DELETION_FIELD
    }

    #[cfg(test)]
    pub fn is_settings(&self) -> bool {
        self.community_id == SETTINGS_SCOPE
    }
}

/// Generic three-way merge for one field.
#[allow(clippy::too_many_arguments)]
fn merge_value<T, F>(
    field: &str,
    community_id: &str,
    community_name: &str,
    base: Option<&T>,
    local: Option<&T>,
    remote: Option<&T>,
    format: F,
    conflicts: &mut Vec<FieldConflict>,
) -> Option<T>
where
    T: Clone + PartialEq,
    F: Fn(&T) -> String,
{
    if local == remote {
        return local.cloned();
    }
    if local == base {
        return remote.cloned();
    }
    if remote == base {
        return local.cloned();
    }

    conflicts.push(FieldConflict {
        community_id: community_id.to_string(),
        community_name: community_name.to_string(),
        field: field.to_string(),
        base_value: base.map(&format),
        local_value: local.map(&format),
        remote_value: remote.map(&format),
        local_record: None,
        remote_record: None,
    });
    // Keep the local value on this device until the user chooses. The cloud is
    // not written while a conflict is open.
    local.cloned()
}

/// Provenance is append-only, so references are unioned instead of merged as a
/// single conflicting value. This never drops a source a device has seen.
fn merge_sources(local: &[CloudSource], remote: &[CloudSource]) -> Vec<CloudSource> {
    let mut merged: BTreeMap<(String, Option<String>), CloudSource> = BTreeMap::new();
    for source in remote.iter().chain(local.iter()) {
        let key = (
            source.source_name.clone(),
            source.source_external_id.clone(),
        );
        match merged.get_mut(&key) {
            None => {
                merged.insert(key, source.clone());
            }
            Some(existing) => {
                if source.first_seen_at < existing.first_seen_at {
                    existing.first_seen_at = source.first_seen_at.clone();
                }
                if source.last_seen_at > existing.last_seen_at {
                    existing.last_seen_at = source.last_seen_at.clone();
                }
                if existing.listing_url.is_none() {
                    existing.listing_url = source.listing_url.clone();
                }
            }
        }
    }
    merged.into_values().collect()
}

fn semantically_equal(a: &CloudCommunity, b: &CloudCommunity) -> bool {
    if a.id != b.id {
        return false;
    }
    // Derived scores are not user edits and are recomputed after every merge.
    let mut left = a.clone();
    let mut right = b.clone();
    left.relevance_score = 0;
    right.relevance_score = 0;
    left.score_breakdown.clear();
    right.score_breakdown.clear();
    left == right
}

fn merge_record(
    base: Option<&CloudCommunity>,
    local: &CloudCommunity,
    remote: &CloudCommunity,
) -> (CloudCommunity, Vec<FieldConflict>) {
    let id = local.id.as_str();
    let label = local.name.as_str();
    let mut out = local.clone();
    let mut conflicts = Vec::new();

    macro_rules! scalar {
        ($field:literal, $name:ident, $format:expr) => {
            out.$name = merge_value(
                $field,
                id,
                label,
                base.map(|c| &c.$name),
                Some(&local.$name),
                Some(&remote.$name),
                $format,
                &mut conflicts,
            )
            .unwrap_or_else(|| local.$name.clone());
        };
    }
    macro_rules! optional {
        ($field:literal, $name:ident) => {
            out.$name = merge_value(
                $field,
                id,
                label,
                base.and_then(|c| c.$name.as_ref()),
                local.$name.as_ref(),
                remote.$name.as_ref(),
                |value: &String| value.clone(),
                &mut conflicts,
            );
        };
    }

    scalar!("name", name, |value: &String| value.clone());
    scalar!("description", description, |value: &String| value.clone());
    optional!("invite_url", invite_url);
    optional!("invite_code", invite_code);
    optional!("canonical_url", canonical_url);
    scalar!("primary_source", primary_source, |value: &String| value
        .clone());
    out.tags = merge_value(
        "tags",
        id,
        label,
        base.map(|c| &c.tags),
        Some(&local.tags),
        Some(&remote.tags),
        |value: &Vec<String>| serde_json::to_string(value).unwrap_or_default(),
        &mut conflicts,
    )
    .unwrap_or_else(|| local.tags.clone());
    out.member_count = merge_value(
        "member_count",
        id,
        label,
        base.and_then(|c| c.member_count.as_ref()),
        local.member_count.as_ref(),
        remote.member_count.as_ref(),
        |value: &i64| value.to_string(),
        &mut conflicts,
    );
    optional!("language", language);
    // review_status and promotion_policy are the fields conflicts matter most for.
    scalar!("review_status", review_status, |value: &String| value
        .clone());
    scalar!("promotion_policy", promotion_policy, |value: &String| value
        .clone());
    optional!("policy_evidence", policy_evidence);
    optional!("policy_checked_at", policy_checked_at);
    optional!("channel_name", channel_name);
    scalar!("user_notes", user_notes, |value: &String| value.clone());
    optional!("contacted_at", contacted_at);
    optional!("last_checked_at", last_checked_at);

    // Meta fields are not user edits and must not generate conflicts.
    out.created_at = [local.created_at.as_str(), remote.created_at.as_str()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .min()
        .unwrap_or("")
        .to_string();
    out.updated_at = local.updated_at.clone().max(remote.updated_at.clone());

    out.sources = merge_sources(&local.sources, &remote.sources);
    // Scores are derived; the caller recomputes them from merged fields.
    out.relevance_score = 0;
    out.score_breakdown.clear();

    (out, conflicts)
}

fn deletion_conflict(
    local: Option<&CloudCommunity>,
    remote: Option<&CloudCommunity>,
    local_value: &str,
    remote_value: &str,
) -> FieldConflict {
    let reference = local.or(remote);
    let id = reference.map(|c| c.id.clone()).unwrap_or_default();
    let name = reference.map(|c| c.name.clone()).unwrap_or_default();
    FieldConflict {
        community_id: id.clone(),
        community_name: name,
        field: DELETION_FIELD.to_string(),
        base_value: Some(PRESENT_VALUE.to_string()),
        local_value: Some(local_value.to_string()),
        remote_value: Some(remote_value.to_string()),
        local_record: local.and_then(|c| serde_json::to_string(c).ok()),
        remote_record: remote.and_then(|c| serde_json::to_string(c).ok()),
    }
}

fn merge_settings(
    base: &BTreeMap<String, String>,
    local: &BTreeMap<String, String>,
    remote: &BTreeMap<String, String>,
    conflicts: &mut Vec<FieldConflict>,
) -> BTreeMap<String, String> {
    let keys: BTreeSet<&String> = base
        .keys()
        .chain(local.keys())
        .chain(remote.keys())
        .collect();
    let mut merged = BTreeMap::new();
    for key in keys {
        let field = format!("settings.{key}");
        let value = merge_value(
            &field,
            SETTINGS_SCOPE,
            "Workspace settings",
            base.get(key),
            local.get(key),
            remote.get(key),
            |value: &String| value.clone(),
            conflicts,
        );
        if let Some(value) = value {
            merged.insert(key.clone(), value);
        }
    }
    merged
}

fn merge_runs(local: &[CloudRun], remote: &[CloudRun]) -> Vec<CloudRun> {
    let mut by_id: BTreeMap<String, CloudRun> = BTreeMap::new();
    for run in remote.iter().chain(local.iter()) {
        match by_id.get(&run.id) {
            None => {
                by_id.insert(run.id.clone(), run.clone());
            }
            Some(existing) => {
                // Prefer the finished copy, then the more complete one. Runs are
                // immutable once finished, so this is not a user-visible choice.
                let existing_finished = existing.finished_at.is_some();
                let candidate_finished = run.finished_at.is_some();
                if (candidate_finished && !existing_finished)
                    || (candidate_finished == existing_finished
                        && run.result_count > existing.result_count)
                {
                    by_id.insert(run.id.clone(), run.clone());
                }
            }
        }
    }
    let mut runs: Vec<CloudRun> = by_id.into_values().collect();
    runs.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    runs.truncate(MAX_RUNS);
    runs
}

/// A user decision that overrides one conflict for the next merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub community_id: String,
    /// A field name, [`DELETION_FIELD`], or `settings.<key>`.
    pub field: String,
    pub keep_remote: bool,
}

/// Merges and then applies explicit user resolutions. Resolutions are applied
/// to the merged result (not to the base), so a "keep local" choice is uploaded
/// rather than being re-interpreted as a stale value.
pub fn merge_documents_resolved(
    base: &CloudDocument,
    local: &CloudDocument,
    remote: &CloudDocument,
    resolutions: &[Resolution],
) -> DocumentMerge {
    let mut merged = merge_documents(base, local, remote);

    for resolution in resolutions {
        if resolution.field == DELETION_FIELD {
            if !resolution.keep_remote {
                merged.document.remove_community(&resolution.community_id);
            }
        } else if let Some(key) = resolution.field.strip_prefix("settings.") {
            let chosen = if resolution.keep_remote {
                remote.settings.get(key)
            } else {
                local.settings.get(key)
            };
            match chosen {
                Some(value) => {
                    merged
                        .document
                        .settings
                        .insert(key.to_string(), value.clone());
                }
                None => {
                    merged.document.settings.remove(key);
                }
            }
        } else if let Some(record) = merged
            .document
            .communities
            .iter_mut()
            .find(|record| record.id == resolution.community_id)
        {
            let source = if resolution.keep_remote {
                remote.community(&resolution.community_id)
            } else {
                local.community(&resolution.community_id)
            };
            if let Some(source) = source {
                let value = source.field_text(&resolution.field);
                let _ = record.set_field(&resolution.field, value.as_deref());
            }
        }

        merged.conflicts.retain(|conflict| {
            !(conflict.community_id == resolution.community_id
                && conflict.field == resolution.field)
        });
    }

    merged
}

/// Merges the three documents. `base` is the last document this device synced;
/// passing an empty document is the first-sync case.
pub fn merge_documents(
    base: &CloudDocument,
    local: &CloudDocument,
    remote: &CloudDocument,
) -> DocumentMerge {
    let mut conflicts = Vec::new();
    let mut communities = Vec::new();

    let ids: BTreeSet<&String> = base
        .communities
        .iter()
        .chain(local.communities.iter())
        .chain(remote.communities.iter())
        .map(|record| &record.id)
        .collect();

    for id in ids {
        let base_record = base.community(id);
        let local_record = local.community(id);
        let remote_record = remote.community(id);

        match (base_record, local_record, remote_record) {
            (_, Some(local_record), Some(remote_record)) => {
                let (record, mut found) = merge_record(base_record, local_record, remote_record);
                communities.push(record);
                conflicts.append(&mut found);
            }
            (Some(base_record), Some(local_record), None) => {
                if !semantically_equal(local_record, base_record) {
                    // Remote deletion versus local modification: keep the record
                    // and let the user decide. Never silently delete.
                    conflicts.push(deletion_conflict(
                        Some(local_record),
                        None,
                        PRESENT_VALUE,
                        DELETED_VALUE,
                    ));
                    communities.push(local_record.clone());
                }
            }
            (Some(base_record), None, Some(remote_record)) => {
                if !semantically_equal(remote_record, base_record) {
                    // Local deletion versus remote modification: keep the remote
                    // record visible and flag it.
                    conflicts.push(deletion_conflict(
                        None,
                        Some(remote_record),
                        DELETED_VALUE,
                        PRESENT_VALUE,
                    ));
                    communities.push(remote_record.clone());
                }
            }
            (None, Some(local_record), None) => communities.push(local_record.clone()),
            (None, None, Some(remote_record)) => communities.push(remote_record.clone()),
            // Both sides deleted, or the record vanished everywhere.
            (Some(_), None, None) | (None, None, None) => {}
        }
    }

    communities.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_id = if !remote.workspace_id.is_empty() {
        remote.workspace_id.clone()
    } else if !local.workspace_id.is_empty() {
        local.workspace_id.clone()
    } else {
        base.workspace_id.clone()
    };

    let generation = base.generation.max(local.generation).max(remote.generation) + 1;

    let document = CloudDocument {
        schema_version: crate::sync::document::SCHEMA_VERSION,
        app: crate::sync::document::APP_ID.to_string(),
        workspace_id,
        generation,
        updated_at: local.updated_at.clone(),
        updated_by: local.updated_by.clone(),
        communities,
        settings: merge_settings(
            &base.settings,
            &local.settings,
            &remote.settings,
            &mut conflicts,
        ),
        discovery_runs: merge_runs(&local.discovery_runs, &remote.discovery_runs),
    };

    conflicts.sort_by(|a, b| {
        a.community_name
            .cmp(&b.community_name)
            .then_with(|| a.field.cmp(&b.field))
    });

    DocumentMerge {
        document,
        conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::document::{CloudCommunity, CloudDocument};

    fn record(id: &str, name: &str) -> CloudCommunity {
        CloudCommunity {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            invite_url: None,
            invite_code: None,
            canonical_url: None,
            primary_source: "Manual entry".to_string(),
            tags: vec![],
            member_count: None,
            language: None,
            relevance_score: 0,
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
            sources: vec![],
        }
    }

    fn doc(records: Vec<CloudCommunity>) -> CloudDocument {
        let mut doc = CloudDocument::empty("ws", "device-a", "2026-01-01T00:00:00Z");
        doc.communities = records;
        doc
    }

    #[test]
    fn different_fields_are_merged_from_both_devices() {
        let base = doc(vec![record("id-1", "Freelancers Hub")]);
        let mut local = base.clone();
        local.communities[0].review_status = "suitable".to_string();
        let mut remote = base.clone();
        remote.communities[0].user_notes = "Recruitment posts allowed.".to_string();

        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.conflicts.is_empty());
        let record = merged.document.community("id-1").unwrap();
        assert_eq!(record.review_status, "suitable");
        assert_eq!(record.user_notes, "Recruitment posts allowed.");
    }

    #[test]
    fn same_field_same_value_is_not_a_conflict() {
        let base = doc(vec![record("id-1", "A")]);
        let mut local = base.clone();
        local.communities[0].review_status = "suitable".to_string();
        let mut remote = base.clone();
        remote.communities[0].review_status = "suitable".to_string();

        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.conflicts.is_empty());
        assert_eq!(
            merged.document.community("id-1").unwrap().review_status,
            "suitable"
        );
    }

    #[test]
    fn same_field_different_values_keeps_both_and_conflicts() {
        let base = doc(vec![record("id-1", "Freelancers Hub")]);
        let mut local = base.clone();
        local.communities[0].review_status = "suitable".to_string();
        let mut remote = base.clone();
        remote.communities[0].review_status = "unsuitable".to_string();

        let merged = merge_documents(&base, &local, &remote);
        assert_eq!(merged.conflicts.len(), 1);
        let conflict = &merged.conflicts[0];
        assert_eq!(conflict.field, "review_status");
        assert_eq!(conflict.local_value.as_deref(), Some("suitable"));
        assert_eq!(conflict.remote_value.as_deref(), Some("unsuitable"));
        assert_eq!(conflict.base_value.as_deref(), Some("not_reviewed"));
        // The local value stays until the user chooses.
        assert_eq!(
            merged.document.community("id-1").unwrap().review_status,
            "suitable"
        );
    }

    #[test]
    fn stale_device_never_overwrites_a_newer_change() {
        let base = doc(vec![record("id-1", "A")]);
        // The phone changed the status.
        let mut remote = base.clone();
        remote.communities[0].review_status = "contacted".to_string();
        // The laptop still has the old value but did change notes.
        let mut local = base.clone();
        local.communities[0].user_notes = "Asked in #introductions.".to_string();

        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.conflicts.is_empty());
        let record = merged.document.community("id-1").unwrap();
        assert_eq!(record.review_status, "contacted");
        assert_eq!(record.user_notes, "Asked in #introductions.");
    }

    #[test]
    fn remote_deletion_versus_local_modification_is_preserved_and_flagged() {
        let base = doc(vec![record("id-1", "A")]);
        let mut local = base.clone();
        local.communities[0].user_notes = "still interested".to_string();
        let remote = doc(vec![]);

        let merged = merge_documents(&base, &local, &remote);
        assert_eq!(merged.document.communities.len(), 1);
        assert_eq!(merged.conflicts.len(), 1);
        let conflict = &merged.conflicts[0];
        assert!(conflict.is_deletion());
        assert_eq!(conflict.local_value.as_deref(), Some(PRESENT_VALUE));
        assert_eq!(conflict.remote_value.as_deref(), Some(DELETED_VALUE));
        assert!(conflict.local_record.is_some());
    }

    #[test]
    fn local_deletion_versus_remote_modification_is_preserved_and_flagged() {
        let base = doc(vec![record("id-1", "A")]);
        let local = doc(vec![]);
        let mut remote = base.clone();
        remote.communities[0].review_status = "suitable".to_string();

        let merged = merge_documents(&base, &local, &remote);
        assert_eq!(merged.document.communities.len(), 1);
        assert_eq!(merged.conflicts.len(), 1);
        let conflict = &merged.conflicts[0];
        assert_eq!(conflict.local_value.as_deref(), Some(DELETED_VALUE));
        assert_eq!(conflict.remote_value.as_deref(), Some(PRESENT_VALUE));
        assert!(conflict.remote_record.is_some());
    }

    #[test]
    fn a_deletion_propagates_when_the_other_side_was_unchanged() {
        let base = doc(vec![record("id-1", "A")]);
        let local = doc(vec![]);
        let remote = base.clone();
        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.document.communities.is_empty());
        assert!(merged.conflicts.is_empty());
    }

    #[test]
    fn both_sides_deleted_is_not_a_conflict() {
        let base = doc(vec![record("id-1", "A")]);
        let local = doc(vec![]);
        let remote = doc(vec![]);
        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.document.communities.is_empty());
        assert!(merged.conflicts.is_empty());
    }

    #[test]
    fn records_created_independently_on_both_devices_are_both_kept() {
        let base = doc(vec![]);
        let local = doc(vec![record("id-1", "From laptop")]);
        let remote = doc(vec![record("id-2", "From phone")]);
        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.conflicts.is_empty());
        assert_eq!(merged.document.communities.len(), 2);
        assert!(merged.document.community("id-1").is_some());
        assert!(merged.document.community("id-2").is_some());
    }

    #[test]
    fn the_same_new_record_from_both_devices_conflicts_per_field() {
        let base = doc(vec![]);
        let mut local = doc(vec![record("id-1", "Same")]);
        local.communities[0].review_status = "suitable".to_string();
        let mut remote = doc(vec![record("id-1", "Same")]);
        remote.communities[0].review_status = "unsuitable".to_string();

        let merged = merge_documents(&base, &local, &remote);
        assert_eq!(merged.conflicts.len(), 1);
        assert_eq!(merged.conflicts[0].field, "review_status");
        assert_eq!(merged.conflicts[0].base_value, None);
    }

    #[test]
    fn multiple_records_merge_independently() {
        let base = doc(vec![record("id-1", "A"), record("id-2", "B")]);
        let mut local = base.clone();
        local.communities[0].review_status = "suitable".to_string();
        local.communities[1].user_notes = "local note".to_string();
        let mut remote = base.clone();
        remote.communities[0].user_notes = "remote note".to_string();
        remote.communities[1].review_status = "contacted".to_string();

        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.conflicts.is_empty());
        let first = merged.document.community("id-1").unwrap();
        assert_eq!(first.review_status, "suitable");
        assert_eq!(first.user_notes, "remote note");
        let second = merged.document.community("id-2").unwrap();
        assert_eq!(second.review_status, "contacted");
        assert_eq!(second.user_notes, "local note");
    }

    #[test]
    fn derived_scores_are_not_merged_and_do_not_conflict() {
        let base = doc(vec![record("id-1", "A")]);
        let mut local = base.clone();
        local.communities[0].relevance_score = 10;
        let mut remote = base.clone();
        remote.communities[0].relevance_score = 90;
        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.conflicts.is_empty());
        assert_eq!(
            merged.document.community("id-1").unwrap().relevance_score,
            0
        );
    }

    #[test]
    fn updated_at_changing_on_both_sides_does_not_conflict() {
        let base = doc(vec![record("id-1", "A")]);
        let mut local = base.clone();
        local.communities[0].updated_at = "2026-02-01T00:00:00Z".to_string();
        let mut remote = base.clone();
        remote.communities[0].updated_at = "2026-03-01T00:00:00Z".to_string();
        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.conflicts.is_empty());
        assert_eq!(
            merged.document.community("id-1").unwrap().updated_at,
            "2026-03-01T00:00:00Z"
        );
    }

    #[test]
    fn sources_are_unioned_without_losing_references() {
        let base = doc(vec![record("id-1", "A")]);
        let mut local = base.clone();
        local.communities[0].sources.push(CloudSource {
            source_name: "manual".to_string(),
            source_external_id: None,
            listing_url: None,
            first_seen_at: "2026-01-01T00:00:00Z".to_string(),
            last_seen_at: "2026-01-01T00:00:00Z".to_string(),
        });
        let mut remote = base.clone();
        remote.communities[0].sources.push(CloudSource {
            source_name: "awesome-list".to_string(),
            source_external_id: Some("abc".to_string()),
            listing_url: Some("https://example.test".to_string()),
            first_seen_at: "2026-01-02T00:00:00Z".to_string(),
            last_seen_at: "2026-01-02T00:00:00Z".to_string(),
        });
        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.conflicts.is_empty());
        assert_eq!(merged.document.community("id-1").unwrap().sources.len(), 2);
    }

    #[test]
    fn settings_conflict_and_merge_by_key() {
        let mut base = doc(vec![]);
        base.settings
            .insert("sort".to_string(), "relevance".to_string());
        base.settings
            .insert("theme".to_string(), "light".to_string());
        let mut local = base.clone();
        local
            .settings
            .insert("sort".to_string(), "name".to_string());
        let mut remote = base.clone();
        remote
            .settings
            .insert("sort".to_string(), "newest".to_string());
        remote
            .settings
            .insert("theme".to_string(), "dark".to_string());

        let merged = merge_documents(&base, &local, &remote);
        assert_eq!(merged.conflicts.len(), 1);
        assert_eq!(merged.conflicts[0].field, "settings.sort");
        assert!(merged.conflicts[0].is_settings());
        assert_eq!(merged.document.settings.get("sort").unwrap(), "name");
        assert_eq!(merged.document.settings.get("theme").unwrap(), "dark");
    }

    #[test]
    fn a_setting_removed_on_one_side_stays_removed() {
        let mut base = doc(vec![]);
        base.settings.insert("old".to_string(), "value".to_string());
        let mut local = base.clone();
        local.settings.remove("old");
        let remote = base.clone();
        let merged = merge_documents(&base, &local, &remote);
        assert!(merged.conflicts.is_empty());
        assert!(!merged.document.settings.contains_key("old"));
    }

    #[test]
    fn runs_are_unioned_deduplicated_and_bounded() {
        let mut base = doc(vec![]);
        base.discovery_runs.clear();
        let mut local = base.clone();
        let mut remote = base.clone();
        for index in 0..40 {
            let id = format!("local-{index}");
            local.discovery_runs.push(CloudRun {
                id,
                query: "q".to_string(),
                sources: vec![],
                requested_limit: 10,
                status: "succeeded".to_string(),
                result_count: 1,
                saved_count: 1,
                partial: false,
                error: None,
                started_at: format!("2026-01-01T00:{index:02}:00Z"),
                finished_at: Some("2026-01-01T00:00:01Z".to_string()),
            });
            let id = format!("remote-{index}");
            remote.discovery_runs.push(CloudRun {
                id,
                query: "q".to_string(),
                sources: vec![],
                requested_limit: 10,
                status: "succeeded".to_string(),
                result_count: 1,
                saved_count: 1,
                partial: false,
                error: None,
                started_at: format!("2026-02-01T00:{index:02}:00Z"),
                finished_at: Some("2026-02-01T00:00:01Z".to_string()),
            });
        }
        // Same id on both sides.
        remote.discovery_runs.push(local.discovery_runs[0].clone());

        let merged = merge_documents(&base, &local, &remote);
        assert_eq!(merged.document.discovery_runs.len(), MAX_RUNS);
        let ids: BTreeSet<&String> = merged
            .document
            .discovery_runs
            .iter()
            .map(|run| &run.id)
            .collect();
        assert_eq!(ids.len(), MAX_RUNS);
    }

    #[test]
    fn keeping_the_local_value_survives_the_next_merge() {
        let base = doc(vec![record("id-1", "A")]);
        let mut local = base.clone();
        local.communities[0].review_status = "suitable".to_string();
        let mut remote = base.clone();
        remote.communities[0].review_status = "unsuitable".to_string();

        let resolved = merge_documents_resolved(
            &base,
            &local,
            &remote,
            &[Resolution {
                community_id: "id-1".to_string(),
                field: "review_status".to_string(),
                keep_remote: false,
            }],
        );
        assert!(resolved.conflicts.is_empty());
        assert_eq!(
            resolved.document.community("id-1").unwrap().review_status,
            "suitable"
        );
    }

    #[test]
    fn keeping_the_remote_value_resolves_the_conflict() {
        let base = doc(vec![record("id-1", "A")]);
        let mut local = base.clone();
        local.communities[0].review_status = "suitable".to_string();
        let mut remote = base.clone();
        remote.communities[0].review_status = "unsuitable".to_string();

        let resolved = merge_documents_resolved(
            &base,
            &local,
            &remote,
            &[Resolution {
                community_id: "id-1".to_string(),
                field: "review_status".to_string(),
                keep_remote: true,
            }],
        );
        assert!(resolved.conflicts.is_empty());
        assert_eq!(
            resolved.document.community("id-1").unwrap().review_status,
            "unsuitable"
        );
    }

    #[test]
    fn a_resolved_deletion_removes_the_record_from_the_merged_document() {
        let base = doc(vec![record("id-1", "A")]);
        let local = doc(vec![]);
        let mut remote = base.clone();
        remote.communities[0].user_notes = "changed".to_string();

        let resolved = merge_documents_resolved(
            &base,
            &local,
            &remote,
            &[Resolution {
                community_id: "id-1".to_string(),
                field: DELETION_FIELD.to_string(),
                keep_remote: false,
            }],
        );
        assert!(resolved.conflicts.is_empty());
        assert!(resolved.document.communities.is_empty());
    }

    #[test]
    fn a_resolved_settings_conflict_uses_the_chosen_value() {
        let mut base = doc(vec![]);
        base.settings
            .insert("sort".to_string(), "relevance".to_string());
        let mut local = base.clone();
        local
            .settings
            .insert("sort".to_string(), "name".to_string());
        let mut remote = base.clone();
        remote
            .settings
            .insert("sort".to_string(), "newest".to_string());

        let resolved = merge_documents_resolved(
            &base,
            &local,
            &remote,
            &[Resolution {
                community_id: SETTINGS_SCOPE.to_string(),
                field: "settings.sort".to_string(),
                keep_remote: false,
            }],
        );
        assert!(resolved.conflicts.is_empty());
        assert_eq!(resolved.document.settings.get("sort").unwrap(), "name");
    }

    #[test]
    fn a_record_that_never_existed_anywhere_is_ignored() {
        let merged = merge_documents(&doc(vec![]), &doc(vec![]), &doc(vec![]));
        assert!(merged.document.communities.is_empty());
        assert!(merged.conflicts.is_empty());
        assert_eq!(merged.document.generation, 1);
    }
}
