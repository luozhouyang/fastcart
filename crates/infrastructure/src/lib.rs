//! Infrastructure implementations for persistence, async work and integrations.
//!
//! Infrastructure implements interfaces defined by domain and application
//! layers. It must not leak SQLx or provider types into public API DTOs.
//!
//! Modules are declared during Phase 0 and implemented in later phases.

/// SQLite connections, transactions and health checks.
pub mod database {}

/// Per-store connection pools and migration coordination.
pub mod store_db_manager {}

/// In-process caching with store-aware keys.
pub mod cache {}

/// SQLite-backed job queue and workers.
pub mod jobs {}

/// In-process event bus and transactional outbox.
pub mod event_bus {}

/// Local filesystem and future object storage adapters.
pub mod storage {}
