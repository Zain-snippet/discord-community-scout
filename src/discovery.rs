//! Discovery orchestration: fetch, parse, filter, score, persist, record the run.

use std::{collections::HashMap, sync::Arc, time::Instant};

use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::Mutex;

use crate::{
    error::{AppError, AppResult},
    models::{Community, CommunityInput, PromotionPolicy},
    repo, scoring,
    sources::{awesome, Discovered},
    validate,
};

pub const MIN_LIMIT: i64 = 1;
pub const MAX_LIMIT: i64 = 50;

#[derive(Debug, Clone)]
pub struct CachedFetch {
    pub at: Instant,
    pub body: String,
}

pub type SourceCache = Arc<Mutex<HashMap<String, CachedFetch>>>;

#[derive(Debug, serde::Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub community: Community,
    /// `new` when saved by this run, `existing` when merged into an existing record.
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub run_id: String,
    pub query: String,
    pub requested_limit: i64,
    pub status: String,
    pub total_matched: usize,
    pub returned: usize,
    pub saved_new: usize,
    pub merged_existing: usize,
    pub partial: bool,
    pub duration_ms: u128,
    pub attribution: String,
    pub results: Vec<SearchHit>,
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Fetches the source document, using the cache when fresh.
async fn fetch_document(
    http: &reqwest::Client,
    cache: &SourceCache,
    ttl_secs: u64,
) -> Result<String, String> {
    {
        let guard = cache.lock().await;
        if let Some(cached) = guard.get(awesome::SOURCE_ID) {
            if cached.at.elapsed().as_secs() < ttl_secs {
                tracing::debug!("serving discovery source from cache");
                return Ok(cached.body.clone());
            }
        }
    }

    let mut last_error = String::new();
    for attempt in 1..=2 {
        let result = http
            .get(awesome::README_URL)
            .header(reqwest::header::ACCEPT, "text/plain")
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => match response.text().await {
                Ok(body) if !body.trim().is_empty() => {
                    let mut guard = cache.lock().await;
                    guard.insert(
                        awesome::SOURCE_ID.to_string(),
                        CachedFetch {
                            at: Instant::now(),
                            body: body.clone(),
                        },
                    );
                    return Ok(body);
                }
                Ok(_) => last_error = "the source returned an empty document".to_string(),
                Err(err) => last_error = format!("could not read the response: {err}"),
            },
            Ok(response) => {
                last_error = format!("the source returned HTTP {}", response.status().as_u16());
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    break; // do not hammer a rate limit
                }
            }
            Err(err) => last_error = format!("request failed: {err}"),
        }

        if attempt == 1 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    Err(last_error)
}

fn matches_query(entry: &Discovered, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let haystack = format!(
        "{} {} {} {}",
        entry.name.to_lowercase(),
        entry.category.to_lowercase(),
        entry.subcategory.to_lowercase(),
        entry.tags.join(" ").to_lowercase()
    );
    terms.iter().any(|term| haystack.contains(term))
}

pub struct SearchContext<'a> {
    pub pool: &'a SqlitePool,
    pub http: &'a reqwest::Client,
    pub cache: &'a SourceCache,
    pub ttl_secs: u64,
}

/// Runs one discovery search and persists what it finds.
pub async fn search(ctx: SearchContext<'_>, request: SearchRequest) -> AppResult<SearchResponse> {
    let query = validate::clean_text(&request.query);
    if query.is_empty() {
        return Err(AppError::bad_request("Enter at least one search term."));
    }
    if query.chars().count() > 200 {
        return Err(AppError::bad_request(
            "Search terms must be at most 200 characters.",
        ));
    }

    let limit = request.limit.unwrap_or(10).clamp(MIN_LIMIT, MAX_LIMIT);

    let run_id = uuid::Uuid::new_v4().to_string();
    let started_at = now();
    let started = Instant::now();

    sqlx::query(
        "INSERT INTO discovery_runs (id, query, sources, requested_limit, status, started_at)
         VALUES (?, ?, ?, ?, 'running', ?)",
    )
    .bind(&run_id)
    .bind(&query)
    .bind(serde_json::json!([awesome::SOURCE_ID]).to_string())
    .bind(limit)
    .bind(&started_at)
    .execute(ctx.pool)
    .await?;

    let document = match fetch_document(ctx.http, ctx.cache, ctx.ttl_secs).await {
        Ok(document) => document,
        Err(message) => {
            let finished = now();
            sqlx::query(
                "UPDATE discovery_runs SET status = 'failed', error = ?, finished_at = ? WHERE id = ?",
            )
            .bind(&message)
            .bind(&finished)
            .bind(&run_id)
            .execute(ctx.pool)
            .await?;

            return Err(AppError::Upstream(format!(
                "The discovery source could not be reached: {message}. CSV import and manual entry still work."
            )));
        }
    };

    let entries = awesome::parse_readme(&document);
    let terms: Vec<String> = scoring::tokenize(&query);

    let mut matched: Vec<(i64, Discovered)> = entries
        .into_iter()
        .filter(|entry| matches_query(entry, &terms))
        .map(|entry| {
            let description = entry.description();
            let score = scoring::score(&scoring::ScoreInput {
                query: Some(&query),
                name: &entry.name,
                description: &description,
                tags: &entry.tags,
                promotion_policy: PromotionPolicy::Unknown,
                policy_evidence: None,
                user_notes: "",
            });
            (score.total, entry)
        })
        .collect();

    matched.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    let total_matched = matched.len();
    matched.truncate(limit as usize);

    let mut results = Vec::with_capacity(matched.len());
    let mut saved_new = 0usize;
    let mut merged_existing = 0usize;

    for (_score, entry) in matched {
        let input = CommunityInput {
            name: entry.name.clone(),
            description: Some(entry.description()),
            invite_url: Some(entry.invite_url.clone()),
            canonical_url: Some(entry.listing_url.clone()),
            tags: Some(entry.tags.clone()),
            language: None,
            channel_name: None,
            user_notes: None,
            promotion_policy: Some(PromotionPolicy::Unknown.as_str().to_string()),
            policy_evidence: None,
            review_status: None,
        };

        let existing =
            repo::find_by_invite_codes(ctx.pool, std::slice::from_ref(&entry.invite_code)).await?;

        if let Some((_code, id, _name)) = existing.first() {
            repo::insert_provenance(
                ctx.pool,
                id,
                awesome::SOURCE_NAME,
                Some(&entry.invite_code),
                Some(&entry.listing_url),
                &now(),
            )
            .await?;
            let community = repo::get(ctx.pool, id).await?;
            merged_existing += 1;
            results.push(SearchHit {
                community,
                status: "existing".to_string(),
            });
        } else {
            let community = repo::create_from_source(
                ctx.pool,
                &input,
                awesome::SOURCE_NAME,
                Some(&entry.invite_code),
                Some(&entry.listing_url),
                Some(&query),
            )
            .await?;
            saved_new += 1;
            results.push(SearchHit {
                community,
                status: "new".to_string(),
            });
        }
    }

    let duration_ms = started.elapsed().as_millis();
    let status = "succeeded";

    sqlx::query(
        "UPDATE discovery_runs
         SET status = ?, result_count = ?, saved_count = ?, partial = 0, finished_at = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(total_matched as i64)
    .bind((saved_new + merged_existing) as i64)
    .bind(now())
    .bind(&run_id)
    .execute(ctx.pool)
    .await?;

    Ok(SearchResponse {
        run_id,
        query,
        requested_limit: limit,
        status: status.to_string(),
        total_matched,
        returned: results.len(),
        saved_new,
        merged_existing,
        partial: false,
        duration_ms,
        attribution: format!(
            "Source: {SOURCE_NAME} ({REPO_URL}), licensed CC-BY-4.0.",
            SOURCE_NAME = awesome::SOURCE_NAME,
            REPO_URL = awesome::REPO_URL
        ),
        results,
    })
}

#[derive(Debug, Serialize)]
pub struct RunSummary {
    pub id: String,
    pub query: String,
    pub requests_limit: i64,
    pub status: String,
    pub result_count: i64,
    pub saved_count: i64,
    pub error: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

/// Most recent discovery runs, newest first.
pub async fn recent_runs(pool: &SqlitePool, limit: i64) -> AppResult<Vec<RunSummary>> {
    let rows = sqlx::query(
        "SELECT id, query, requested_limit, status, result_count, saved_count, error,
                started_at, finished_at
         FROM discovery_runs ORDER BY started_at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 50))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| RunSummary {
            id: sqlx::Row::get(&row, "id"),
            query: sqlx::Row::get(&row, "query"),
            requests_limit: sqlx::Row::get(&row, "requested_limit"),
            status: sqlx::Row::get(&row, "status"),
            result_count: sqlx::Row::get(&row, "result_count"),
            saved_count: sqlx::Row::get(&row, "saved_count"),
            error: sqlx::Row::get(&row, "error"),
            started_at: sqlx::Row::get(&row, "started_at"),
            finished_at: sqlx::Row::get(&row, "finished_at"),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, category: &str) -> Discovered {
        Discovered {
            name: name.to_string(),
            invite_url: "https://discord.gg/x".to_string(),
            invite_code: "x".to_string(),
            category: category.to_string(),
            subcategory: String::new(),
            listing_url: "https://example.com".to_string(),
            tags: vec![category.to_lowercase()],
        }
    }

    #[test]
    fn query_matches_name_or_category() {
        let terms = vec!["rust".to_string()];
        assert!(matches_query(&entry("Rust Lang", "Programming"), &terms));
        assert!(matches_query(&entry("Foo", "Rust"), &terms));
        assert!(!matches_query(&entry("Foo", "Gaming"), &terms));
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(matches_query(&entry("Anything", "Whatever"), &[]));
    }

    #[test]
    fn limit_bounds_are_sane() {
        assert_eq!(5i64.clamp(MIN_LIMIT, MAX_LIMIT), 5);
        assert_eq!(0i64.clamp(MIN_LIMIT, MAX_LIMIT), 1);
        assert_eq!(1000i64.clamp(MIN_LIMIT, MAX_LIMIT), 50);
    }
}
