//! Platform-level domain: stores, domains, lifecycle, users, memberships,
//! authentication and authorization.
//!
//! The platform database is global to the installation. Store commerce data is
//! isolated in per-store SQLite databases managed by the infrastructure crate.

pub mod auth;
pub mod domain;
pub mod error;
pub mod migrations;
pub mod provisioning;
pub mod repository;
pub mod resolver;
pub mod store;
pub mod user;

pub use auth::{
    ApiToken, AuthService, AuthenticatedSession, AuthenticatedToken, CreatedApiToken,
    CreatedSession, DEFAULT_SESSION_TTL_SECONDS, MAX_API_TOKEN_TTL_SECONDS, hash_token,
};
pub use domain::{StoreDomain, normalize_hostname, validate_hostname};
pub use error::{PlatformError, PlatformResult};
pub use migrations::{PLATFORM_MIGRATOR, open_memory_database, open_platform_database};
pub use provisioning::{
    DeleteStoreOptions, ProvisioningJob, ProvisioningReport, StoreProvisioningService,
};
pub use repository::PlatformStore;
pub use resolver::StoreResolver;
pub use store::{CreateStoreInput, Store, StoreRegistry, StoreStatus, UpdateStoreInput};
pub use user::{
    AuditLog, CreateUserInput, Membership, MembershipStatus, NewAudit, User, UserStatus,
};
