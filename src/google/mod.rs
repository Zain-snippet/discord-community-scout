//! Google integration: OAuth, secure token storage and the Drive API client.
//!
//! Sign-in happens on the backend host. Browsers that reach that backend share
//! the connection; tokens never reach a browser.

pub mod drive;
pub mod oauth;
pub mod tokens;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use serde::Serialize;
use tokio::sync::Mutex;

use crate::sync::engine::DriveError;

use self::oauth::OAuthError;
use self::tokens::{TokenSet, TokenStore};

#[derive(Clone)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
}

struct PendingAuth {
    verifier: String,
    created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct GoogleStatus {
    pub configured: bool,
    pub signed_in: bool,
    /// Honest label for where the token lives on this host.
    pub token_storage: &'static str,
}

pub struct GoogleService {
    config: Option<GoogleConfig>,
    http: reqwest::Client,
    store: TokenStore,
    tokens: Mutex<Option<TokenSet>>,
    pending: std::sync::Mutex<HashMap<String, PendingAuth>>,
}

impl GoogleService {
    pub fn new(config: Option<GoogleConfig>, data_dir: &Path, http: reqwest::Client) -> Arc<Self> {
        let store = TokenStore::new(data_dir);
        let tokens = match store.load() {
            Ok(tokens) => tokens,
            Err(error) => {
                // A corrupt or undecryptable file must not crash startup. The
                // user simply signs in again.
                tracing::warn!(error = %error, "could not read stored Google tokens");
                None
            }
        };
        Arc::new(Self {
            config,
            http,
            store,
            tokens: Mutex::new(tokens),
            pending: std::sync::Mutex::new(HashMap::new()),
        })
    }

    pub fn configured(&self) -> bool {
        self.config.is_some()
    }

    pub async fn status(&self) -> GoogleStatus {
        GoogleStatus {
            configured: self.configured(),
            signed_in: self.tokens.lock().await.is_some(),
            token_storage: if cfg!(windows) {
                "Windows DPAPI"
            } else {
                "file with mode 0600 (not an OS keychain)"
            },
        }
    }

    /// Creates a single-use state value and returns Google's authorization URL.
    pub fn sign_in_url(&self) -> Result<String, OAuthError> {
        let config = self.config.as_ref().ok_or(OAuthError::NotConfigured)?;
        let pkce = oauth::generate_pkce();
        let state = oauth::random_token();

        {
            let mut pending = self.pending.lock().expect("pending auth lock");
            let cutoff = Utc::now().timestamp() - oauth::AUTH_STATE_TTL_SECS;
            pending.retain(|_, entry| entry.created_at >= cutoff);
            pending.insert(
                state.clone(),
                PendingAuth {
                    verifier: pkce.verifier,
                    created_at: Utc::now().timestamp(),
                },
            );
        }

        Ok(oauth::auth_url(
            &config.client_id,
            &config.redirect_uri,
            &state,
            &pkce.challenge,
        ))
    }

    /// Validates the single-use state and exchanges the code for tokens.
    pub async fn complete_sign_in(&self, code: &str, state: &str) -> Result<(), OAuthError> {
        let config = self.config.as_ref().ok_or(OAuthError::NotConfigured)?;
        let pending = {
            let mut pending = self.pending.lock().expect("pending auth lock");
            pending.remove(state)
        }
        .ok_or(OAuthError::StateExpired)?;

        if Utc::now().timestamp() - pending.created_at > oauth::AUTH_STATE_TTL_SECS {
            return Err(OAuthError::StateExpired);
        }

        let tokens = oauth::exchange_code(
            &self.http,
            &config.client_id,
            config.client_secret.as_deref(),
            code,
            &pending.verifier,
            &config.redirect_uri,
        )
        .await?;

        self.store.save(&tokens).map_err(|error| {
            tracing::warn!(error = %error, "could not persist Google tokens");
            OAuthError::Rejected
        })?;
        *self.tokens.lock().await = Some(tokens);
        tracing::info!("Google Drive connected");
        Ok(())
    }

    /// Returns a valid access token, refreshing it when needed.
    pub async fn access_token(&self) -> Result<String, DriveError> {
        let config = self.config.as_ref().ok_or(DriveError::NotConfigured)?;
        let mut guard = self.tokens.lock().await;
        let Some(current) = guard.clone() else {
            return Err(DriveError::AuthExpired);
        };
        if !current.is_expired(60) {
            return Ok(current.access_token);
        }

        let Some(refresh_token) = current.refresh_token.clone() else {
            *guard = None;
            let _ = self.store.clear();
            return Err(DriveError::AuthExpired);
        };

        match oauth::refresh(
            &self.http,
            &config.client_id,
            config.client_secret.as_deref(),
            &refresh_token,
        )
        .await
        {
            Ok(tokens) => {
                if let Err(error) = self.store.save(&tokens) {
                    tracing::warn!(error = %error, "could not persist refreshed Google tokens");
                }
                let access = tokens.access_token.clone();
                *guard = Some(tokens);
                Ok(access)
            }
            Err(OAuthError::Unreachable) => Err(DriveError::Offline),
            Err(error) => {
                tracing::warn!(error = %error, "Google token refresh failed");
                *guard = None;
                let _ = self.store.clear();
                Err(DriveError::AuthExpired)
            }
        }
    }

    /// Revokes the refresh token at Google (best effort) and removes local
    /// credentials. Application data is never deleted.
    pub async fn logout(&self) {
        let token = {
            let mut guard = self.tokens.lock().await;
            guard
                .take()
                .map(|tokens| tokens.refresh_token.unwrap_or(tokens.access_token))
        };
        if let Some(token) = token {
            oauth::revoke(&self.http, &token).await;
        }
        if let Err(error) = self.store.clear() {
            tracing::warn!(error = %error, "could not delete the local token file");
        }
        tracing::info!("Google Drive disconnected");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(config: Option<GoogleConfig>) -> Arc<GoogleService> {
        let dir = std::env::temp_dir().join(format!("scout-google-{}", uuid::Uuid::new_v4()));
        GoogleService::new(config, &dir, reqwest::Client::new())
    }

    fn config() -> GoogleConfig {
        GoogleConfig {
            client_id: "client-id".to_string(),
            client_secret: Some("server-secret".to_string()),
            redirect_uri: "http://127.0.0.1:8080/api/auth/google/callback".to_string(),
        }
    }

    #[tokio::test]
    async fn an_unconfigured_service_reports_itself_honestly() {
        let service = service(None);
        let status = service.status().await;
        assert!(!status.configured);
        assert!(!status.signed_in);
        assert!(matches!(
            service.sign_in_url(),
            Err(OAuthError::NotConfigured)
        ));
        assert!(matches!(
            service.access_token().await,
            Err(DriveError::NotConfigured)
        ));
    }

    #[tokio::test]
    async fn the_sign_in_url_does_not_contain_the_client_secret() {
        let service = service(Some(config()));
        let url = service.sign_in_url().unwrap();
        assert!(url.contains("client-id"));
        assert!(!url.contains("server-secret"));
        assert!(url.contains("drive.appdata"));
    }

    #[tokio::test]
    async fn an_unknown_state_is_rejected() {
        let service = service(Some(config()));
        let _ = service.sign_in_url().unwrap();
        assert!(matches!(
            service.complete_sign_in("code", "not-the-state").await,
            Err(OAuthError::StateExpired)
        ));
        // Replaying the same unknown value stays rejected.
        assert!(matches!(
            service.complete_sign_in("code", "not-the-state").await,
            Err(OAuthError::StateExpired)
        ));
    }

    #[tokio::test]
    async fn logout_clears_local_credentials_without_touching_data() {
        let service = service(Some(config()));
        // There is nothing stored; logout must still succeed.
        service.logout().await;
        assert!(!service.status().await.signed_in);
    }
}
