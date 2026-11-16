//! Discord Community Scout — a research assistant for public Discord communities.
//!
//! The application never automates a Discord account and never sends messages.
//! It collects publicly published community listings, scores them, records the
//! user's review decisions, and drafts outreach text for manual review.

mod config;
mod db;
mod error;
mod models;
mod repo;
mod routes;
mod scoring;
mod validate;

use std::net::SocketAddr;

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

    let state = routes::AppState {
        pool,
        config: std::sync::Arc::new(config.clone()),
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
