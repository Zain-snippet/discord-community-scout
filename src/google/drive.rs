//! Google Drive v3 client for the app-specific `appDataFolder`.
//!
//! Only one file is used (`scout-data.json`). The client never lists or touches
//! the user's normal Drive files.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::google::{oauth, GoogleService};
use crate::sync::document::CloudDocument;
use crate::sync::engine::{CloudDrive, DriveError, RemoteMeta};

const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";
const FILE_NAME: &str = "scout-data.json";

pub struct GoogleDrive {
    http: reqwest::Client,
    service: Arc<GoogleService>,
}

impl GoogleDrive {
    pub fn new(service: Arc<GoogleService>, http: reqwest::Client) -> Self {
        Self { http, service }
    }

    async fn token(&self) -> Result<String, DriveError> {
        self.service.access_token().await
    }

    async fn meta(&self, token: &str, file_id: &str) -> Result<RemoteMeta, DriveError> {
        let url = format!("{API}/files/{file_id}?fields=id,version,modifiedTime");
        let response = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| DriveError::Offline)?;
        let etag = etag_of(&response);
        let response = ensure(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|_| DriveError::Other("unreadable file metadata".to_string()))?;
        Ok(RemoteMeta {
            file_id: body
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(file_id)
                .to_string(),
            revision: body
                .get("version")
                .map(version_text)
                .unwrap_or_else(|| "0".to_string()),
            etag,
        })
    }
}

fn version_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => "0".to_string(),
    }
}

fn etag_of(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Maps a failed Drive response to a typed error. Response bodies are not
/// copied into errors, because they can echo request detail.
pub fn map_status(status: u16, body: &str) -> DriveError {
    match status {
        401 => DriveError::AuthExpired,
        404 => DriveError::NotFound,
        412 => DriveError::Conflict,
        429 => DriveError::RateLimited,
        403 if body.contains("insufficientPermissions") || body.contains("insufficient_scope") => {
            DriveError::AuthExpired
        }
        403 if body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded") => {
            DriveError::RateLimited
        }
        500..=599 => DriveError::Other("temporary Google server error".to_string()),
        _ => DriveError::Other(format!("Google Drive returned HTTP {status}")),
    }
}

async fn ensure(response: reqwest::Response) -> Result<reqwest::Response, DriveError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    tracing::warn!(
        status = status.as_u16(),
        google_error = %short_error(&body),
        "Google Drive request failed"
    );
    Err(map_status(status.as_u16(), &body))
}

fn short_error(body: &str) -> String {
    let value: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return "unparseable".to_string(),
    };
    value
        .pointer("/error/errors/0/reason")
        .or_else(|| value.pointer("/error/status"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

#[async_trait]
impl CloudDrive for GoogleDrive {
    async fn metadata(&self) -> Result<Option<RemoteMeta>, DriveError> {
        let token = self.token().await?;
        let query = oauth::encode_query_value(&format!("name = '{FILE_NAME}' and trashed = false"));
        let url = format!(
            "{API}/files?spaces=appDataFolder&q={query}&fields=files(id,name,version,modifiedTime)&pageSize=10"
        );
        let response = self
            .http
            .get(url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|_| DriveError::Offline)?;
        let response = ensure(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|_| DriveError::Other("unreadable file list".to_string()))?;
        let Some(files) = body.get("files").and_then(Value::as_array) else {
            return Ok(None);
        };
        // If duplicates somehow exist, the most recently modified wins.
        let newest = files.iter().max_by_key(|file| {
            file.get("modifiedTime")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        });
        Ok(newest.map(|file| RemoteMeta {
            file_id: file
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            revision: file
                .get("version")
                .map(version_text)
                .unwrap_or_else(|| "0".to_string()),
            etag: None,
        }))
    }

    async fn download(&self, file_id: &str) -> Result<(Vec<u8>, RemoteMeta), DriveError> {
        let token = self.token().await?;
        let url = format!("{API}/files/{file_id}?alt=media");
        let response = self
            .http
            .get(url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|_| DriveError::Offline)?;
        let media_etag = etag_of(&response);
        let response = ensure(response).await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|_| DriveError::Offline)?
            .to_vec();
        // Version must come from the resource, not the media body, so it matches
        // the value returned by `metadata`.
        let mut meta = self.meta(&token, file_id).await?;
        if meta.etag.is_none() {
            meta.etag = media_etag;
        }
        Ok((bytes, meta))
    }

    async fn upload(
        &self,
        file_id: Option<&str>,
        document: &CloudDocument,
        expected: Option<&RemoteMeta>,
    ) -> Result<RemoteMeta, DriveError> {
        let token = self.token().await?;
        let body = document
            .to_bytes()
            .map_err(|error| DriveError::Other(error.to_string()))?;

        let target = match file_id {
            Some(id) => id.to_string(),
            None => {
                let response = self
                    .http
                    .post(format!("{API}/files?fields=id,version"))
                    .bearer_auth(&token)
                    .json(&json!({ "name": FILE_NAME, "parents": ["appDataFolder"] }))
                    .send()
                    .await
                    .map_err(|_| DriveError::Offline)?;
                let response = ensure(response).await?;
                let value: Value = response
                    .json()
                    .await
                    .map_err(|_| DriveError::Other("unreadable created file".to_string()))?;
                value
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| DriveError::Other("the created file has no id".to_string()))?
            }
        };

        // Conditional update: ask Google to reject the write if the file changed.
        let mut request = self
            .http
            .patch(format!(
                "{UPLOAD_API}/files/{target}?uploadType=media&fields=id,version"
            ))
            .bearer_auth(&token)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=utf-8",
            );

        if let Some(expected) = expected {
            match &expected.etag {
                Some(etag) => {
                    request = request.header(reqwest::header::IF_MATCH, etag);
                }
                None => {
                    // No ETag available: compare versions immediately before the
                    // write. There is a small race window, but never a blind
                    // overwrite of a differently versioned document.
                    let current = self.meta(&token, &target).await?;
                    if current.revision != expected.revision {
                        return Err(DriveError::Conflict);
                    }
                }
            }
        }

        let response = request
            .body(body)
            .send()
            .await
            .map_err(|_| DriveError::Offline)?;
        let etag = etag_of(&response);
        let response = ensure(response).await?;
        let value: Value = response
            .json()
            .await
            .map_err(|_| DriveError::Other("unreadable upload confirmation".to_string()))?;
        let revision = value
            .get("version")
            .map(version_text)
            .unwrap_or_else(|| "0".to_string());

        if etag.is_some() {
            Ok(RemoteMeta {
                file_id: target,
                revision,
                etag,
            })
        } else {
            self.meta(&token, &target).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_statuses_map_to_typed_errors() {
        assert!(matches!(map_status(401, ""), DriveError::AuthExpired));
        assert!(matches!(map_status(404, ""), DriveError::NotFound));
        assert!(matches!(map_status(412, ""), DriveError::Conflict));
        assert!(matches!(map_status(429, ""), DriveError::RateLimited));
        assert!(matches!(
            map_status(
                403,
                r#"{"error":{"errors":[{"reason":"rateLimitExceeded"}]}}"#
            ),
            DriveError::RateLimited
        ));
        assert!(matches!(
            map_status(
                403,
                r#"{"error":{"errors":[{"reason":"insufficientPermissions"}]}}"#
            ),
            DriveError::AuthExpired
        ));
        assert!(matches!(map_status(500, ""), DriveError::Other(_)));
        assert!(matches!(
            map_status(400, "bad request"),
            DriveError::Other(_)
        ));
    }

    #[test]
    fn versions_are_read_from_strings_or_numbers() {
        assert_eq!(version_text(&json!("42")), "42");
        assert_eq!(version_text(&json!(42)), "42");
        assert_eq!(version_text(&Value::Null), "0");
    }

    #[test]
    fn only_a_reason_code_is_extracted_from_an_error_body() {
        let body = r#"{"error":{"code":403,"message":"sensitive text","errors":[{"reason":"quotaExceeded"}]}}"#;
        assert_eq!(short_error(body), "quotaExceeded");
        assert_eq!(short_error("not json"), "unparseable");
    }

    #[test]
    fn the_file_name_is_scoped_and_stable() {
        assert_eq!(FILE_NAME, "scout-data.json");
    }
}
