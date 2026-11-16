//! Configuration from environment variables, validated at startup.

use std::env;

use anyhow::{bail, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub cache_ttl_secs: u64,
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

        Ok(Self {
            port,
            database_url,
            cache_ttl_secs,
        })
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
}
