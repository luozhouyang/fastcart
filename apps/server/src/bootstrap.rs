//! Server bootstrap and lifecycle.

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::shutdown::shutdown_signal;
use crate::state::AppState;

/// Starts the HTTP server with the given configuration.
pub async fn run(config: Config) -> Result<()> {
    init_tracing(&config.server.log_level);
    config.validate().context("invalid configuration")?;

    let state = AppState::initialize(config.clone()).await?;
    let app = fastcart_api::build_router(state.context.clone());
    let address = config.listen_address();

    let listener = TcpListener::bind(&address)
        .await
        .with_context(|| format!("failed to bind {address}"))?;

    tracing::info!(
        address = %address,
        storefront_enabled = config.storefront.enabled,
        runtime = %config.storefront.runtime,
        "fastcart server listening"
    );

    let maintenance = tokio::spawn(crate::maintenance::maintenance_loop(state.context.clone()));

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server failed")?;

    maintenance.abort();
    state.databases.close_all().await;
    tracing::info!("fastcart server stopped");
    Ok(())
}

/// Loads configuration from the environment and starts the server.
pub async fn run_from_env() -> Result<()> {
    let config = Config::load()?;
    run(config).await
}

/// Installs the tracing subscriber. Safe to call more than once.
pub fn init_tracing(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}
