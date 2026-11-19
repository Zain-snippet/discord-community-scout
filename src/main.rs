//! Discord Community Scout — a research assistant for public Discord communities.
//!
//! The application never automates a Discord account and never sends messages.
//! It collects publicly published community listings, scores them, records the
//! user's review decisions, and drafts outreach text for manual review.

mod config;
mod csv_io;
mod db;
mod discovery;
mod error;
mod google;
mod models;
mod repo;
mod routes;
mod scoring;
mod sources;
mod sync;
mod templates;
mod validate;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = config::Config::from_env().context("invalid configuration")?;

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    let pool = db::connect(&config.database_url)
        .await
        .context("failed to open the database")?;

    // One shared client for outbound discovery fetches: bounded timeouts so a
    // slow source cannot hang a request, and a normal user agent.
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!(
            "discord-community-scout/",
            env!("CARGO_PKG_VERSION"),
            " (public community research tool)"
        ))
        .build()
        .context("failed to build the HTTP client")?;

    let data_dir = config.data_dir.clone();
    std::fs::create_dir_all(&data_dir).context("cannot create the data directory")?;

    let google_config = config
        .google_client_id
        .as_ref()
        .map(|client_id| google::GoogleConfig {
            client_id: client_id.clone(),
            client_secret: config.google_client_secret.clone(),
            redirect_uri: config.redirect_uri(),
        });
    let google = google::GoogleService::new(google_config, &data_dir, http.clone());
    let drive: Arc<dyn sync::engine::CloudDrive> = Arc::new(google::drive::GoogleDrive::new(
        google.clone(),
        http.clone(),
    ));
    let sync = sync::engine::SyncEngine::new(pool.clone(), drive, data_dir);

    if google.configured() {
        spawn_sync_worker(sync.clone(), google.clone());
        tracing::info!(
            redirect_uri = %config.redirect_uri(),
            "Google Drive sync enabled"
        );
    } else {
        tracing::info!("Google Drive sync disabled (GOOGLE_CLIENT_ID is not set)");
    }

    let state = routes::AppState {
        pool,
        config: std::sync::Arc::new(config.clone()),
        http,
        cache: discovery::SourceCache::default(),
        google,
        sync,
    };

    let app = routes::router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind {addr}"))?;

    tracing::info!("Discord Community Scout listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    Ok(())
}

/// Background synchronization.
///
/// The loop checks the cheap local dirty counter every five seconds while signed
/// in, which also debounces bursts of edits, and performs a metadata check every
/// two minutes. Conflicts, expired authorization and a pending migration pause
/// automatic syncing until the user acts, so the app does not hammer Google.
fn spawn_sync_worker(engine: Arc<sync::engine::SyncEngine>, google: Arc<google::GoogleService>) {
    tokio::spawn(async move {
        use sync::engine::SyncStatus;

        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_full = tokio::time::Instant::now() - Duration::from_secs(120);

        loop {
            ticker.tick().await;
            if !google.status().await.signed_in {
                continue;
            }
            let snapshot = match engine.snapshot().await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    tracing::warn!(error = %error, "could not read the sync state");
                    continue;
                }
            };
            if matches!(
                snapshot.status,
                SyncStatus::Conflict
                    | SyncStatus::MigrationRequired
                    | SyncStatus::AuthExpired
                    | SyncStatus::Disconnected
            ) {
                continue;
            }
            if !snapshot.dirty && last_full.elapsed() < Duration::from_secs(120) {
                continue;
            }
            let report = engine.sync_now().await;
            if matches!(report.status, SyncStatus::Synced | SyncStatus::Pending) {
                last_full = tokio::time::Instant::now();
            }
        }
    });
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutting down");
}
