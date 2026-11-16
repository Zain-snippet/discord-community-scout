//! CSV import parsing/validation and export rendering.
//!
//! Pure functions plus a thin database-facing step in the route layer. Parsing
//! never drops a row silently: every row reported as `invalid` carries the
//! reason, and rows that match an existing community are reported as `merge`
//! so their provenance is attached instead of the data being lost.

use serde::Serialize;

use crate::{
    models::{CommunityInput, PromotionPolicy, ReviewStatus},
    validate,
};

pub const MAX_ROWS: usize = 2000;
pub const MAX_BYTES: usize = 1024 * 1024;

pub const HEADERS: [&str; 12] = [
    "name",
    "description",
    "invite_url",
    "canonical_url",
    "tags",
    "language",
    "channel_name",
    "user_notes",
    "promotion_policy",
    "policy_evidence",
    "review_status",
    "member_count",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowStatus {
    /// New community, will be created.
    New,
    /// Invite already exists; provenance will be attached to that record.
    Merge,
    /// Row failed validation and will not be saved.
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
pub struct RowReport {
    pub line: usize,
    pub name: String,
    pub status: RowStatus,
    pub errors: Vec<String>,
    /// Populated for `Merge` rows: the existing community's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merges_into: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPreview {
    pub total_rows: usize,
    pub new_rows: usize,
    pub merge_rows: usize,
    pub invalid_rows: usize,
    pub rows: Vec<RowReport>,
}

#[derive(Debug, Clone)]
pub struct ParsedRow {
    pub line: usize,
    pub name: String,
    pub status: RowStatus,
    pub errors: Vec<String>,
    pub merges_into: Option<String>,
    /// Present when the row is savable (new or merge).
    pub input: Option<CommunityInput>,
    pub invite_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedCsv {
    pub rows: Vec<ParsedRow>,
    pub header_errors: Vec<String>,
}

fn parse_tags(raw: &str) -> Vec<String> {
    raw.split([';', '|'])
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

/// Parses a member count, ignoring anything that is not a positive integer.
fn parse_member_count(raw: &str) -> Option<i64> {
    match raw.trim().parse::<i64>() {
        Ok(value) if value > 0 => Some(value),
        _ => None,
    }
}

/// Validates one row into a `CommunityInput`, collecting every problem at once.
fn build_row(
    _line: usize,
    record: &csv::StringRecord,
    headers: &csv::StringRecord,
) -> (Option<CommunityInput>, Vec<String>) {
    let mut errors = Vec::new();

    let get = |field: &str| -> String {
        headers
            .iter()
            .position(|h| h.eq_ignore_ascii_case(field))
            .and_then(|idx| record.get(idx))
            .unwrap_or("")
            .trim()
            .to_string()
    };

    let name = match validate::required_text(&get("name"), "name", validate::MAX_NAME) {
        Ok(value) => value,
        Err(err) => {
            errors.push(err);
            String::new()
        }
    };

    let description = match validate::optional_text(
        Some(&get("description")),
        "description",
        validate::MAX_DESCRIPTION,
    ) {
        Ok(value) => value,
        Err(err) => {
            errors.push(err);
            None
        }
    };

    let tags = match validate::normalise_tags(&parse_tags(&get("tags"))) {
        Ok(value) => value,
        Err(err) => {
            errors.push(err);
            Vec::new()
        }
    };

    let invite_url = get("invite_url");
    let canonical_url = get("canonical_url");
    let (invite_url, _invite_code, canonical_url) = match validate::normalise_invite_fields(
        Some(invite_url.as_str()),
        Some(canonical_url.as_str()),
    ) {
        Ok(value) => value,
        Err(err) => {
            errors.push(format!("invite_url/canonical_url: {err}"));
            (None, None, None)
        }
    };

    let policy = {
        let raw = get("promotion_policy");
        if raw.is_empty() {
            PromotionPolicy::Unknown
        } else {
            match validate::parse_policy(&raw) {
                Ok(policy) => policy,
                Err(err) => {
                    errors.push(err);
                    PromotionPolicy::Unknown
                }
            }
        }
    };

    let review_status = {
        let raw = get("review_status").to_ascii_lowercase();
        if raw.is_empty() {
            ReviewStatus::NotReviewed
        } else {
            match ReviewStatus::parse(&raw) {
                Some(status) => status,
                None => {
                    errors.push("review_status is not a known status.".to_string());
                    ReviewStatus::NotReviewed
                }
            }
        }
    };

    if !errors.is_empty() {
        return (None, errors);
    }

    let input = CommunityInput {
        name,
        description,
        invite_url,
        canonical_url,
        tags: Some(tags),
        language: Some(get("language")).filter(|v| !v.is_empty()),
        channel_name: Some(get("channel_name")).filter(|v| !v.is_empty()),
        user_notes: Some(get("user_notes")).filter(|v| !v.is_empty()),
        promotion_policy: Some(policy.as_str().to_string()),
        policy_evidence: Some(get("policy_evidence")).filter(|v| !v.is_empty()),
        review_status: Some(review_status.as_str().to_string()),
    };

    let _ = parse_member_count(&get("member_count"));

    (Some(input), errors)
}

/// Parses and validates a whole CSV document.
pub fn parse_csv(text: &str) -> ParsedCsv {
    let mut header_errors = Vec::new();
    let mut rows = Vec::new();

    if text.len() > MAX_BYTES {
        header_errors.push(format!(
            "File is larger than the {} KiB limit.",
            MAX_BYTES / 1024
        ));
        return ParsedCsv {
            rows,
            header_errors,
        };
    }

    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());

    let headers = match reader.headers() {
        Ok(headers) => headers.clone(),
        Err(err) => {
            header_errors.push(format!("Could not read the header row: {err}"));
            return ParsedCsv {
                rows,
                header_errors,
            };
        }
    };

    if !headers.iter().any(|h| h.eq_ignore_ascii_case("name")) {
        header_errors.push(
            "The header row must contain a 'name' column. See docs/csv-format.md.".to_string(),
        );
        return ParsedCsv {
            rows,
            header_errors,
        };
    }

    for (index, record) in reader.records().enumerate() {
        let line = index + 2; // header is line 1

        if line.saturating_sub(1) > MAX_ROWS {
            header_errors.push(format!("At most {MAX_ROWS} data rows are accepted."));
            break;
        }

        let record = match record {
            Ok(record) => record,
            Err(err) => {
                rows.push(ParsedRow {
                    line,
                    name: String::new(),
                    status: RowStatus::Invalid,
                    errors: vec![format!("Malformed row: {err}")],
                    merges_into: None,
                    input: None,
                    invite_code: None,
                });
                continue;
            }
        };

        if record.iter().all(|field| field.trim().is_empty()) {
            continue; // blank line, not an error
        }

        let (input, mut errors) = build_row(line, &record, &headers);

        // Duplicate invite codes inside the same file.
        let invite_code = input
            .as_ref()
            .and_then(|i| i.invite_url.as_deref())
            .and_then(|url| validate::normalise_invite(url).1);

        if let Some(code) = &invite_code {
            if rows.iter().any(|row: &ParsedRow| {
                row.invite_code.as_deref() == Some(code.as_str())
                    && row.status != RowStatus::Invalid
            }) {
                errors.push("Duplicate invite link within this file.".to_string());
            }
        }

        let name = input.as_ref().map(|i| i.name.clone()).unwrap_or_default();

        rows.push(ParsedRow {
            line,
            name,
            status: if errors.is_empty() {
                RowStatus::New
            } else {
                RowStatus::Invalid
            },
            errors,
            merges_into: None,
            input,
            invite_code,
        });
    }

    ParsedCsv {
        rows,
        header_errors,
    }
}

/// Applies the result of the database duplicate lookup to the parsed rows.
pub fn mark_existing(rows: &mut [ParsedRow], existing: &[(String, String)]) {
    // existing: (invite_code, community_name)
    for row in rows.iter_mut() {
        if row.status == RowStatus::Invalid {
            continue;
        }
        if let Some(code) = &row.invite_code {
            if let Some((_, name)) = existing
                .iter()
                .find(|(existing_code, _)| existing_code.eq_ignore_ascii_case(code))
            {
                row.status = RowStatus::Merge;
                row.merges_into = Some(name.clone());
            }
        }
    }
}

/// Builds the preview payload from parsed rows.
pub fn preview_from(parsed: &ParsedCsv) -> ImportPreview {
    let rows: Vec<RowReport> = parsed
        .rows
        .iter()
        .map(|row| RowReport {
            line: row.line,
            name: if row.name.is_empty() {
                format!("(row {})", row.line)
            } else {
                row.name.clone()
            },
            status: row.status.clone(),
            errors: row.errors.clone(),
            merges_into: row.merges_into.clone(),
        })
        .collect();

    ImportPreview {
        total_rows: rows.len(),
        new_rows: rows.iter().filter(|r| r.status == RowStatus::New).count(),
        merge_rows: rows.iter().filter(|r| r.status == RowStatus::Merge).count(),
        invalid_rows: rows
            .iter()
            .filter(|r| r.status == RowStatus::Invalid)
            .count(),
        rows,
    }
}

/// Neutralises spreadsheet formula injection on export.
///
/// Values beginning with `=`, `+`, `-`, `@`, tab or carriage return are prefixed
/// with an apostrophe so spreadsheet software treats them as text.
pub fn escape_for_spreadsheet(value: &str) -> String {
    let needs_escape = value
        .chars()
        .next()
        .map(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r'))
        .unwrap_or(false);

    if needs_escape {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

/// Renders one database row into CSV fields in `HEADERS` order.
pub fn export_record(fields: &ExportRecord) -> Vec<String> {
    vec![
        escape_for_spreadsheet(&fields.name),
        escape_for_spreadsheet(&fields.description),
        fields.invite_url.clone().unwrap_or_default(),
        fields.canonical_url.clone().unwrap_or_default(),
        escape_for_spreadsheet(&fields.tags.join(";")),
        fields.language.clone().unwrap_or_default(),
        escape_for_spreadsheet(fields.channel_name.as_deref().unwrap_or("")),
        escape_for_spreadsheet(&fields.user_notes),
        fields.promotion_policy.clone(),
        escape_for_spreadsheet(fields.policy_evidence.as_deref().unwrap_or("")),
        fields.review_status.clone(),
        fields
            .member_count
            .map(|c| c.to_string())
            .unwrap_or_default(),
    ]
}

pub struct ExportRecord {
    pub name: String,
    pub description: String,
    pub invite_url: Option<String>,
    pub canonical_url: Option<String>,
    pub tags: Vec<String>,
    pub language: Option<String>,
    pub channel_name: Option<String>,
    pub user_notes: String,
    pub promotion_policy: String,
    pub policy_evidence: Option<String>,
    pub review_status: String,
    pub member_count: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str =
        "name,description,invite_url,canonical_url,tags,language,channel_name,user_notes,promotion_policy,policy_evidence,review_status,member_count";

    #[test]
    fn parses_a_valid_row() {
        let csv = format!(
            "{HEADER}\nStudents Hub,A place for students,https://discord.gg/abc,https://example.com,school;careers,English,#promos,note,allowed,quoted rule,suitable,1200"
        );
        let parsed = parse_csv(&csv);
        assert!(parsed.header_errors.is_empty());
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.rows[0].status, RowStatus::New);
        assert_eq!(parsed.rows[0].invite_code.as_deref(), Some("abc"));
        assert_eq!(
            parsed.rows[0].input.as_ref().unwrap().tags,
            Some(vec!["school".to_string(), "careers".to_string()])
        );
    }

    #[test]
    fn missing_name_column_is_a_header_error() {
        let parsed = parse_csv("description,invite_url\nhello,https://discord.gg/x");
        assert!(parsed.header_errors.iter().any(|e| e.contains("name")));
        assert!(parsed.rows.is_empty());
    }

    #[test]
    fn missing_name_value_is_a_row_error_not_a_silent_skip() {
        let csv = format!("{HEADER}\n,desc,https://discord.gg/abc,,,,,,unknown,,,");
        let parsed = parse_csv(&csv);
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.rows[0].status, RowStatus::Invalid);
        assert!(parsed.rows[0].errors.iter().any(|e| e.contains("name")));
    }

    #[test]
    fn unsafe_url_scheme_is_rejected() {
        let csv = format!("{HEADER}\nEvil,desc,javascript:alert(1),,,,,,unknown,,,");
        let parsed = parse_csv(&csv);
        assert_eq!(parsed.rows[0].status, RowStatus::Invalid);
    }

    #[test]
    fn duplicate_invites_inside_one_file_are_flagged() {
        let csv = format!(
            "{HEADER}\nA,one,https://discord.gg/dup,,,,,,unknown,,,\nB,two,discord.com/invite/DUP,,,,,,unknown,,,"
        );
        let parsed = parse_csv(&csv);
        assert_eq!(parsed.rows[0].status, RowStatus::New);
        assert_eq!(parsed.rows[1].status, RowStatus::Invalid);
        assert!(parsed.rows[1]
            .errors
            .iter()
            .any(|e| e.contains("Duplicate")));
    }

    #[test]
    fn malformed_policy_and_status_are_reported() {
        let csv = format!("{HEADER}\nA,d,https://discord.gg/a,,,,,,maybe,,nonsense,,");
        let parsed = parse_csv(&csv);
        assert_eq!(parsed.rows[0].status, RowStatus::Invalid);
        assert!(parsed.rows[0].errors.len() >= 2);
    }

    #[test]
    fn blank_lines_are_ignored() {
        let csv = format!("{HEADER}\n\nA,d,https://discord.gg/a,,,,,,unknown,,,\n\n");
        let parsed = parse_csv(&csv);
        assert_eq!(parsed.rows.len(), 1);
    }

    #[test]
    fn formula_prefixes_are_neutralised_on_export() {
        assert_eq!(escape_for_spreadsheet("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(escape_for_spreadsheet("+1"), "'+1");
        assert_eq!(escape_for_spreadsheet("-1"), "'-1");
        assert_eq!(escape_for_spreadsheet("@cmd"), "'@cmd");
        assert_eq!(escape_for_spreadsheet("normal"), "normal");
    }

    #[test]
    fn formula_like_names_are_imported_as_literal_text() {
        let csv = format!("{HEADER}\n=CMD|' /C calc'!A0,d,https://discord.gg/f,,,,,,unknown,,,");
        let parsed = parse_csv(&csv);
        // Stored verbatim; nothing executes it, and export escapes it.
        assert_eq!(parsed.rows[0].status, RowStatus::New);
        assert!(parsed.rows[0].name.starts_with('='));
        assert!(escape_for_spreadsheet(&parsed.rows[0].name).starts_with('\''));
    }

    #[test]
    fn existing_invites_are_marked_for_merge() {
        let csv = format!("{HEADER}\nA,d,https://discord.gg/known,,,,,,unknown,,,");
        let mut parsed = parse_csv(&csv);
        mark_existing(
            &mut parsed.rows,
            &[("known".to_string(), "Old Name".to_string())],
        );
        assert_eq!(parsed.rows[0].status, RowStatus::Merge);
        assert_eq!(parsed.rows[0].merges_into.as_deref(), Some("Old Name"));
    }

    #[test]
    fn preview_counts_by_status() {
        let csv = format!(
            "{HEADER}\nA,d,https://discord.gg/a,,,,,,unknown,,,\n,missing,,common,,,,,,unknown,,,\nC,d,https://discord.gg/c,,,,,,unknown,,,"
        );
        let parsed = parse_csv(&csv);
        let preview = preview_from(&parsed);
        assert_eq!(preview.total_rows, 3);
        assert_eq!(preview.new_rows, 2);
        assert_eq!(preview.invalid_rows, 1);
    }

    #[test]
    fn oversized_document_is_refused() {
        let big = "a".repeat(MAX_BYTES + 1);
        let parsed = parse_csv(&big);
        assert!(parsed.header_errors.iter().any(|e| e.contains("larger")));
    }
}
