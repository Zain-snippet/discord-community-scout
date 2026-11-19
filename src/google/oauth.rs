//! Google OAuth 2.0 Authorization Code flow with PKCE (S256).
//!
//! The client secret, when the OAuth client has one, stays on the server and is
//! only ever sent to Google's token endpoint. Nothing secret enters the browser
//! URL or the application log.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::google::tokens::TokenSet;

pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata";
const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const REVOKE_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";

/// How long a pending sign-in may take before its state value is discarded.
pub const AUTH_STATE_TTL_SECS: i64 = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum OAuthError {
    #[error("Google sign-in is not configured on this server.")]
    NotConfigured,
    #[error("Google sign-in expired. Start again.")]
    StateExpired,
    #[error("Google authorization expired or was revoked. Sign in again.")]
    AuthExpired,
    #[error("Google refused the sign-in request.")]
    Rejected,
    #[error("Google could not be reached.")]
    Unreachable,
}

/// URL-safe random text, 32 bytes encoded without padding.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// RFC 7636 verifier (43–128 characters) and its S256 challenge.
pub fn generate_pkce() -> Pkce {
    let verifier = random_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Pkce {
        verifier,
        challenge,
    }
}

/// RFC 3986 percent-encoding for query values.
pub fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
    }
    out
}

pub fn auth_url(client_id: &str, redirect_uri: &str, state: &str, challenge: &str) -> String {
    format!(
        "{AUTH_ENDPOINT}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&include_granted_scopes=true&state={}&code_challenge={}&code_challenge_method=S256",
        encode_query_value(client_id),
        encode_query_value(redirect_uri),
        encode_query_value(SCOPE),
        encode_query_value(state),
        encode_query_value(challenge),
    )
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
}

impl TokenResponse {
    pub fn into_token_set(self, previous_refresh: Option<String>) -> TokenSet {
        TokenSet {
            access_token: self.access_token,
            // Google only returns a refresh token on the first consent; keep the
            // previous one when it does not repeat it.
            refresh_token: self.refresh_token.or(previous_refresh),
            expires_at: chrono::Utc::now().timestamp() + self.expires_in.unwrap_or(3600),
            scope: self.scope,
            token_type: self.token_type,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: String,
}

fn map_failure(status: reqwest::StatusCode, body: &str) -> OAuthError {
    if status.as_u16() == 400
        && (body.contains("invalid_grant") || body.contains("invalid_request"))
    {
        return OAuthError::AuthExpired;
    }
    if status.is_server_error() {
        return OAuthError::Unreachable;
    }
    OAuthError::Rejected
}

async fn post_form(
    http: &reqwest::Client,
    url: &str,
    params: &[(&str, String)],
) -> Result<TokenResponse, OAuthError> {
    let response = http
        .post(url)
        .form(params)
        .send()
        .await
        .map_err(|_| OAuthError::Unreachable)?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        // Log the status and Google's error code only; never the request body.
        let code = serde_json::from_str::<ErrorBody>(&body)
            .map(|error| error.error)
            .unwrap_or_default();
        tracing::warn!(status = status.as_u16(), google_error = %code, "OAuth token request failed");
        return Err(map_failure(status, &body));
    }
    serde_json::from_str(&body).map_err(|_| OAuthError::Rejected)
}

/// Exchanges the authorization code for tokens.
#[allow(clippy::too_many_arguments)]
pub async fn exchange_code(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenSet, OAuthError> {
    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("client_id", client_id.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("code_verifier", verifier.to_string()),
    ];
    if let Some(secret) = client_secret {
        params.push(("client_secret", secret.to_string()));
    }
    let response = post_form(http, TOKEN_ENDPOINT, &params).await?;
    Ok(response.into_token_set(None))
}

/// Refreshes an expired access token.
pub async fn refresh(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<TokenSet, OAuthError> {
    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("client_id", client_id.to_string()),
    ];
    if let Some(secret) = client_secret {
        params.push(("client_secret", secret.to_string()));
    }
    let response = post_form(http, TOKEN_ENDPOINT, &params).await?;
    Ok(response.into_token_set(Some(refresh_token.to_string())))
}

/// Best-effort revocation. A failure here never blocks logout.
pub async fn revoke(http: &reqwest::Client, token: &str) {
    let result = http
        .post(REVOKE_ENDPOINT)
        .form(&[("token", token)])
        .send()
        .await;
    if let Err(error) = result {
        tracing::warn!(error = %error, "could not reach Google to revoke the token");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_the_verifier() {
        let pkce = generate_pkce();
        assert!((43..=128).contains(&pkce.verifier.len()));
        assert!(pkce
            .verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.verifier.as_bytes()));
        assert_eq!(pkce.challenge, expected);
        assert_ne!(generate_pkce().verifier, pkce.verifier);
    }

    #[test]
    fn auth_url_requests_only_app_data_scope() {
        let url = auth_url(
            "client.apps.googleusercontent.com",
            "http://127.0.0.1:8080/api/auth/google/callback",
            "state-value",
            "challenge-value",
        );
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.appdata"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("response_type=code"));
        assert!(!url.to_ascii_lowercase().contains("secret"));
        assert!(!url.contains("drive.file"));
        assert!(!url.contains("userinfo"));
    }

    #[test]
    fn query_encoding_escapes_reserved_characters() {
        assert_eq!(encode_query_value("a b"), "a%20b");
        assert_eq!(encode_query_value("a/b:c"), "a%2Fb%3Ac");
        assert_eq!(encode_query_value("safe-._~"), "safe-._~");
        assert_eq!(encode_query_value("é"), "%C3%A9");
    }

    #[test]
    fn token_response_keeps_the_previous_refresh_token() {
        let response = TokenResponse {
            access_token: "new-access".to_string(),
            refresh_token: None,
            expires_in: Some(3600),
            scope: None,
            token_type: Some("Bearer".to_string()),
        };
        let set = response.into_token_set(Some("old-refresh".to_string()));
        assert_eq!(set.access_token, "new-access");
        assert_eq!(set.refresh_token.as_deref(), Some("old-refresh"));
        assert!(set.expires_at > chrono::Utc::now().timestamp());
    }

    #[test]
    fn invalid_grant_is_reported_as_reauthorization() {
        let status = reqwest::StatusCode::BAD_REQUEST;
        assert!(matches!(
            map_failure(status, r#"{"error":"invalid_grant"}"#),
            OAuthError::AuthExpired
        ));
        assert!(matches!(
            map_failure(reqwest::StatusCode::TOO_MANY_REQUESTS, "{}"),
            OAuthError::Rejected
        ));
        assert!(matches!(
            map_failure(reqwest::StatusCode::BAD_GATEWAY, "{}"),
            OAuthError::Unreachable
        ));
    }
}
