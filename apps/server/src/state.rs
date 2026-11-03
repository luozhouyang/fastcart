//! Process-level state shared by bootstrap and the HTTP layer.

use std::time::Instant;

use fastcart_api::ApiState;

use crate::config::Config;

/// State owned by the server process.
#[derive(Debug)]
pub struct AppState {
    /// Loaded configuration.
    pub config: Config,
    /// Process start time, used for uptime reporting.
    pub started_at: Instant,
}

impl AppState {
    /// Creates application state.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            started_at: Instant::now(),
        }
    }

    /// Builds the HTTP layer state.
    #[must_use]
    pub fn api_state(&self) -> ApiState {
        ApiState {
            service_name: "fastcart",
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}
