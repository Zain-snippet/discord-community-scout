//! Configuration from environment variables, validated at startup.

use std::{env, path::PathBuf};

use anyhow::{bail, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub cache_ttl_secs: u64,
    /// Directory for tokens, snapshots and recovery copies.
    pub data_dir: PathBuf,
    /// Google OAuth client id. Sync is disabled when this is absent.
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    /// Base URL this server is reached at, used to build the OAuth redirect.
    pub public_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let port = match env::var("PORT") {
            Ok(raw) => match raw.trim().parse::<u16>() {
                Ok(port) => port,
                Err(_) => bail!("PORT must be a number between 1 and 65535, got {raw:?}"),
            },
            Err(_) => 8080,
        };

        if port == 0 {
            bail!("PORT must not be 0");
        }

        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://data/scout.db?mode=rwc".to_string());

        if !database_url.starts_with("sqlite:") {
            bail!("DATABASE_URL must be a sqlite: URL, got {database_url:?}");
        }

        let cache_ttl_secs = match env::var("SCOUT_CACHE_TTL_SECS") {
            Ok(raw) => match raw.trim().parse::<u64>() {
                Ok(ttl) => ttl,
                Err(_) => bail!("SCOUT_CACHE_TTL_SECS must be a number, got {raw:?}"),
            },
            Err(_) => 3600,
        };

        let data_dir = env::var("SCOUT_DATA_DIR")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"));

        let google_client_id = env::var("GOOGLE_CLIENT_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let google_client_secret = env::var("GOOGLE_CLIENT_SECRET")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let public_url = env::var("SCOUT_PUBLIC_URL")
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));

        if !(public_url.starts_with("http://") || public_url.starts_with("https://")) {
            bail!("SCOUT_PUBLIC_URL must start with http:// or https://, got {public_url:?}");
        }

        Ok(Self {
            port,
            database_url,
            cache_ttl_secs,
            data_dir,
            google_client_id,
            google_client_secret,
            public_url,
        })
    }

    /// The exact redirect URI that must be registered in Google Cloud.
    pub fn redirect_uri(&self) -> String {
        format!("{}/api/auth/google/callback", self.public_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(database_url: &str) -> Result<Config> {
        Ok(Config {
            port: 8080,
            database_url: database_url.to_string(),
            cache_ttl_secs: 3600,
            data_dir: PathBuf::from("data"),
            google_client_id: None,
            google_client_secret: None,
            public_url: "http://127.0.0.1:8080".to_string(),
        })
    }

    #[test]
    fn accepts_sqlite_urls() {
        assert!(config_with("sqlite://data/scout.db?mode=rwc").is_ok());
        assert!(config_with("sqlite::memory:").is_ok());
        assert!(config_with("sqlite://data/test.db").is_ok());
    }

    #[test]
    fn validates_database_url_scheme() {
        // Mirrors the check in from_env: anything not starting with sqlite: is refused.
        for url in ["postgres://localhost/db", "data/scout.db", ""] {
            assert!(!url.starts_with("sqlite:"), "{url} should be rejected");
        }
    }

    #[test]
    fn redirect_uri_matches_the_registered_callback() {
        let mut config = config_with("sqlite::memory:").unwrap();
        config.public_url = "https://scout.example.com".to_string();
        assert_eq!(
            config.redirect_uri(),
            "https://scout.example.com/api/auth/google/callback"
        );
        config.public_url = "http://127.0.0.1:8080".to_string();
        assert_eq!(
            config.redirect_uri(),
            "http://127.0.0.1:8080/api/auth/google/callback"
        );
    }
}
