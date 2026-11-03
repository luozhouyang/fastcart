//! Infrastructure implementations for persistence, async work and integrations.
//!
//! Infrastructure implements interfaces defined by domain and application
//! layers. It must not leak SQLx or provider types into public API DTOs.

use fastcart_kernel::AppError;

/// In-process caching with store-aware keys.
pub mod cache;
/// SQLite connections, transactions and health checks.
pub mod database;
/// In-process event bus and transactional outbox.
pub mod event_bus;
/// Persistent job queue.
pub mod jobs;
/// Local filesystem and future object storage adapters.
pub mod storage;
/// Per-store connection pools and migration coordination.
pub mod store_db_manager;

pub use cache::StoreCache;
pub use database::{DatabaseHealth, DatabaseOptions, connect_memory, connect_sqlite};
pub use event_bus::{
    EventBus, OutboxRecord, append_outbox_event, fetch_pending_outbox, mark_outbox_failed,
    mark_outbox_processed,
};
pub use jobs::{JobQueue, JobRecord, NewJob};
pub use storage::{LocalStorage, Storage};
pub use store_db_manager::{BackupManifest, StoreDatabase, StoreDatabaseManager};

/// Errors raised by infrastructure adapters.
#[derive(Debug, thiserror::Error)]
pub enum InfrastructureError {
    /// SQLite or SQLx failure.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// Filesystem failure.
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    /// Migration runner failure.
    #[error("migration error: {0}")]
    Migration(String),
    /// Invalid configuration or input.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// Data integrity failure.
    #[error("integrity error: {0}")]
    Integrity(String),
}

impl From<InfrastructureError> for AppError {
    fn from(error: InfrastructureError) -> Self {
        match error {
            InfrastructureError::Database(_) => Self::Database(error.to_string()),
            InfrastructureError::Io(_) => Self::Internal(error.to_string()),
            InfrastructureError::Migration(_) => Self::Internal(error.to_string()),
            InfrastructureError::InvalidInput(message) => Self::Validation(message),
            InfrastructureError::Integrity(message) => Self::Conflict(message),
        }
    }
}

/// Convenience result type for infrastructure adapters.
pub type Result<T> = std::result::Result<T, InfrastructureError>;

/// Maps a SQLx failure into the shared application error model.
///
/// Module repositories use this to keep database details out of API contracts.
#[must_use]
pub fn database_error(error: sqlx::Error) -> AppError {
    AppError::Database(error.to_string())
}
