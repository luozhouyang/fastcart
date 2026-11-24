//! Store registry models, lifecycle rules and platform persistence.

use fastcart_kernel::{
    CurrencyCode, Locale, Metadata, PageResult, Pagination, StoreId, now_timestamp, validate_handle,
};
use sqlx::{FromRow, Row};

use crate::domain::{StoreDomain, normalize_hostname, validate_hostname};
use crate::error::{PlatformError, PlatformResult};
use crate::repository::PlatformStore;

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

impl StoreStatus {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Deleting => "deleting",
            Self::Deleted => "deleted",
        }
    }

    /// Parses a database value. Unknown values are treated as suspended.
    #[must_use]
    pub fn from_db(value: &str) -> Self {
        match value {
            "provisioning" => Self::Provisioning,
            "active" => Self::Active,
            "suspended" => Self::Suspended,
            "deleting" => Self::Deleting,
            "deleted" => Self::Deleted,
            _ => Self::Suspended,
        }
    }

    /// Whether storefront traffic may be served.
    #[must_use]
    pub fn can_serve_traffic(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Whether administrative mutations are allowed.
    #[must_use]
    pub fn allows_administrative_writes(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Validates a lifecycle transition.
    #[must_use]
    pub fn can_transition_to(&self, next: Self) -> bool {
        if *self == next {
            return true;
        }
        matches!(
            (self, next),
            (Self::Provisioning, Self::Active)
                | (Self::Provisioning, Self::Deleting)
                | (Self::Active, Self::Suspended)
                | (Self::Active, Self::Deleting)
                | (Self::Suspended, Self::Active)
                | (Self::Suspended, Self::Deleting)
                | (Self::Deleting, Self::Deleted)
        )
    }
}

impl std::fmt::Display for StoreStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A store registry entry.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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
    /// Store locale.
    pub locale: String,
    /// Store display currency.
    pub currency_code: String,
    /// IANA time zone identifier.
    pub timezone: String,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

impl Store {
    /// Validates that the store may serve storefront traffic.
    pub fn ensure_active(&self) -> PlatformResult<()> {
        if self.status.can_serve_traffic() {
            Ok(())
        } else {
            Err(PlatformError::StoreUnavailable(format!(
                "store {} is {}",
                self.id, self.status
            )))
        }
    }
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
    /// Store locale.
    pub locale: Option<String>,
    /// Display currency.
    pub currency_code: Option<String>,
    /// IANA time zone.
    pub timezone: Option<String>,
    /// Extension metadata.
    pub metadata: Metadata,
}

impl CreateStoreInput {
    /// Creates input with the platform defaults for locale, currency and zone.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        slug: impl Into<String>,
        primary_domain: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            slug: slug.into(),
            primary_domain: primary_domain.into(),
            locale: None,
            currency_code: None,
            timezone: None,
            metadata: Metadata::empty(),
        }
    }

    /// Sets the locale, currency and timezone.
    #[must_use]
    pub fn with_region(
        mut self,
        locale: impl Into<String>,
        currency_code: impl Into<String>,
        timezone: impl Into<String>,
    ) -> Self {
        self.locale = Some(locale.into());
        self.currency_code = Some(currency_code.into());
        self.timezone = Some(timezone.into());
        self
    }

    fn validate(&self) -> PlatformResult<()> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err(PlatformError::InvalidInput(
                "store name must be between 1 and 120 characters".to_owned(),
            ));
        }
        validate_handle(&self.slug, "store slug")
            .map_err(|error| PlatformError::InvalidInput(error.to_string()))?;
        let hostname = normalize_hostname(&self.primary_domain);
        validate_hostname(&hostname).map_err(PlatformError::InvalidInput)?;
        Locale::new(self.locale.clone().unwrap_or_else(|| "en-US".to_owned()))?;
        CurrencyCode::new(
            self.currency_code
                .clone()
                .unwrap_or_else(|| "USD".to_owned()),
        )?;
        if self
            .timezone
            .clone()
            .unwrap_or_else(|| "UTC".to_owned())
            .trim()
            .is_empty()
        {
            return Err(PlatformError::InvalidInput(
                "timezone must not be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Input for updating a store registry entry.
#[derive(Clone, Debug, Default)]
pub struct UpdateStoreInput {
    /// New display name.
    pub name: Option<String>,
    /// New locale.
    pub locale: Option<String>,
    /// New display currency.
    pub currency_code: Option<String>,
    /// New IANA time zone.
    pub timezone: Option<String>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Store registry service.
#[async_trait::async_trait]
pub trait StoreRegistry: Send + Sync {
    /// Creates a store registry entry.
    async fn create_store(&self, input: CreateStoreInput) -> PlatformResult<Store>;

    /// Looks up a store by identifier.
    async fn get_store(&self, id: &StoreId) -> PlatformResult<Option<Store>>;

    /// Looks up a store by normalized hostname.
    async fn get_store_by_domain(&self, hostname: &str) -> PlatformResult<Option<Store>>;

    /// Updates store attributes.
    async fn update_store(&self, id: &StoreId, input: UpdateStoreInput) -> PlatformResult<Store>;

    /// Updates the lifecycle state of a store.
    async fn update_status(&self, id: &StoreId, status: StoreStatus) -> PlatformResult<Store>;
}

#[derive(Debug, FromRow)]
struct StoreRow {
    id: String,
    name: String,
    slug: String,
    status: String,
    database_key: String,
    primary_domain: Option<String>,
    locale: String,
    currency_code: String,
    timezone: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl StoreRow {
    fn into_store(self) -> Store {
        Store {
            id: StoreId::from_string(self.id),
            name: self.name,
            slug: self.slug,
            status: StoreStatus::from_db(&self.status),
            database_key: self.database_key,
            primary_domain: self.primary_domain,
            locale: self.locale,
            currency_code: self.currency_code,
            timezone: self.timezone,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

const STORE_COLUMNS: &str = "id, name, slug, status, database_key, primary_domain, locale,
    currency_code, timezone, metadata, created_at, updated_at";

fn unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database_error) if database_error.is_unique_violation()
    )
}

mod domains;
mod registry;
mod stores;

#[cfg(test)]
mod tests;
