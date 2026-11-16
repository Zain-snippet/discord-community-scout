//! CSV import (preview + commit) and export endpoints.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use super::{ApiJson, AppState};
use crate::{
    csv_io::{self, RowStatus},
    error::{AppError, AppResult},
    models::ListQuery,
    repo,
};

#[derive(Debug, Deserialize)]
pub struct CsvPayload {
    pub csv: String,
}

#[derive(Debug, Serialize)]
pub struct CommitResult {
    pub imported: usize,
    pub merged: usize,
    pub skipped_invalid: usize,
    /// Row-level notes for anything that was not saved, so nothing is lost silently.
    pub problems: Vec<String>,
}

fn parse_or_reject(text: &str) -> AppResult<csv_io::ParsedCsv> {
    let parsed = csv_io::parse_csv(text);
    if !parsed.header_errors.is_empty() {
        return Err(AppError::bad_request(parsed.header_errors.join(" ")));
    }
    if parsed.rows.is_empty() {
        return Err(AppError::bad_request(
            "The file contains no data rows. Add at least one row below the header.",
        ));
    }
    Ok(parsed)
}

/// Validates a CSV document and reports what committing it would do.
pub async fn preview(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<CsvPayload>,
) -> AppResult<Json<csv_io::ImportPreview>> {
    let mut parsed = parse_or_reject(&payload.csv)?;

    let codes: Vec<String> = parsed
        .rows
        .iter()
        .filter_map(|row| row.invite_code.clone())
        .collect();
    let existing = repo::find_by_invite_codes(&state.pool, &codes).await?;
    let pairs: Vec<(String, String)> = existing
        .into_iter()
        .map(|(code, _id, name)| (code, name))
        .collect();

    csv_io::mark_existing(&mut parsed.rows, &pairs);

    Ok(Json(csv_io::preview_from(&parsed)))
}

/// Saves every valid row. Rows that match an existing invite attach provenance
/// instead of overwriting the existing record.
pub async fn commit(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<CsvPayload>,
) -> AppResult<(StatusCode, Json<CommitResult>)> {
    let mut parsed = parse_or_reject(&payload.csv)?;

    let codes: Vec<String> = parsed
        .rows
        .iter()
        .filter_map(|row| row.invite_code.clone())
        .collect();
    let existing = repo::find_by_invite_codes(&state.pool, &codes).await?;
    let pairs: Vec<(String, String)> = existing
        .iter()
        .map(|(code, _id, name)| (code.clone(), name.clone()))
        .collect();
    csv_io::mark_existing(&mut parsed.rows, &pairs);

    let mut by_code: std::collections::HashMap<String, String> = existing
        .into_iter()
        .map(|(code, id, _name)| (code, id))
        .collect();

    let mut imported = 0usize;
    let mut merged = 0usize;
    let mut skipped_invalid = 0usize;
    let mut problems = Vec::new();

    for row in &parsed.rows {
        match row.status {
            RowStatus::Invalid => {
                skipped_invalid += 1;
                problems.push(format!(
                    "Line {}: not saved — {}",
                    row.line,
                    row.errors.join(" ")
                ));
            }
            RowStatus::Merge => {
                let Some(input) = row.input.as_ref() else {
                    skipped_invalid += 1;
                    continue;
                };
                let Some(code) = row.invite_code.as_ref() else {
                    skipped_invalid += 1;
                    continue;
                };
                let Some(id) = by_code.get(code).cloned() else {
                    skipped_invalid += 1;
                    problems.push(format!(
                        "Line {}: matched a duplicate but the record could not be located.",
                        row.line
                    ));
                    continue;
                };
                repo::insert_provenance(
                    &state.pool,
                    &id,
                    "CSV import",
                    None,
                    input.canonical_url.as_deref(),
                    &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                )
                .await?;
                merged += 1;
            }
            RowStatus::New => {
                let Some(input) = row.input.as_ref() else {
                    skipped_invalid += 1;
                    continue;
                };
                match repo::create_from_source(
                    &state.pool,
                    input,
                    "CSV import",
                    None,
                    input.canonical_url.as_deref(),
                )
                .await
                {
                    Ok(community) => {
                        if let Some(code) = row.invite_code.clone() {
                            by_code.insert(code, community.id);
                        }
                        imported += 1;
                    }
                    Err(AppError::Conflict(message)) => {
                        // Another row created the same invite in this batch.
                        merged += 1;
                        problems.push(format!("Line {}: {message}", row.line));
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }

    Ok((
        StatusCode::OK,
        Json(CommitResult {
            imported,
            merged,
            skipped_invalid,
            problems,
        }),
    ))
}

/// Streams every saved community as CSV, including review fields.
pub async fn export(State(state): State<AppState>) -> AppResult<Response> {
    let mut items = Vec::new();
    let mut page = 1i64;
    loop {
        let query = ListQuery {
            page: Some(page),
            per_page: Some(100),
            sort: Some("name".to_string()),
            ..Default::default()
        };
        let result = repo::list(&state.pool, &query).await?;
        items.extend(result.items);
        if page >= result.total_pages {
            break;
        }
        page += 1;
    }

    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .write_record(csv_io::HEADERS)
        .map_err(|err| AppError::Internal(anyhow::anyhow!("failed to start CSV export: {err}")))?;

    for community in items {
        let record = csv_io::ExportRecord {
            name: community.name,
            description: community.description,
            invite_url: community.invite_url,
            canonical_url: community.canonical_url,
            tags: community.tags,
            language: community.language,
            channel_name: community.channel_name,
            user_notes: community.user_notes,
            promotion_policy: community.promotion_policy,
            policy_evidence: community.policy_evidence,
            review_status: community.review_status,
            member_count: community.member_count,
        };
        writer
            .write_record(csv_io::export_record(&record))
            .map_err(|err| AppError::Internal(anyhow::anyhow!("failed to write CSV row: {err}")))?;
    }

    let bytes = writer
        .into_inner()
        .map_err(|err| AppError::Internal(anyhow::anyhow!("failed to finish CSV export: {err}")))?;

    let disposition = header::HeaderValue::from_static("attachment; filename=\"communities.csv\"");
    let content_type = header::HeaderValue::from_static("text/csv; charset=utf-8");

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        bytes,
    )
        .into_response())
}
