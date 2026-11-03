//! Store registry models and service interface.

use fastcart_kernel::StoreId;

use crate::error::PlatformError;

/// Lifecycle state of a store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreStatus {
    /// The store is being provisioned and must not serve traffic.
    Provisioning,
    /// The store is live.
    Active,
    /// Storefront and checkout traffic are rejected, data is preserved.
    Suspended,
    /// Deletion is in progress.
    Deleting,
    /// The store has been deleted.
    Deleted,
}

/// A store registry entry.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Store {
    /// Store identifier.
    pub id: StoreId,
    /// Display name.
    pub name: String,
    /// Unique URL-safe slug.
    pub slug: String,
    /// Current lifecycle state.
    pub status: StoreStatus,
    /// Internal database key. Never derived from untrusted input.
    pub database_key: String,
    /// Primary hostname, when registered.
    pub primary_domain: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// Input for creating a store registry entry.
#[derive(Clone, Debug)]
pub struct CreateStoreInput {
    /// Display name.
    pub name: String,
    /// Unique URL-safe slug.
    pub slug: String,
    /// Initial primary hostname.
    pub primary_domain: String,
}

/// Store registry service.
#[async_trait::async_trait]
pub trait StoreRegistry: Send + Sync {
    /// Creates a store registry entry.
    async fn create_store(&self, input: CreateStoreInput) -> Result<Store, PlatformError>;

    /// Looks up a store by identifier.
    async fn get_store(&self, id: &StoreId) -> Result<Option<Store>, PlatformError>;

    /// Looks up a store by normalized hostname.
    async fn get_store_by_domain(&self, hostname: &str) -> Result<Option<Store>, PlatformError>;

    /// Updates the lifecycle state of a store.
    async fn update_status(
        &self,
        id: &StoreId,
        status: StoreStatus,
    ) -> Result<Store, PlatformError>;
}
