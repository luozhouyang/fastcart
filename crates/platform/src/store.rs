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

impl PlatformStore {
    /// Creates a store and its primary domain in one transaction.
    pub async fn create_store(&self, input: CreateStoreInput) -> PlatformResult<Store> {
        input.validate()?;
        let slug = input.slug.clone();
        let hostname = normalize_hostname(&input.primary_domain);
        let locale = Locale::new(input.locale.clone().unwrap_or_else(|| "en-US".to_owned()))?;
        let currency = CurrencyCode::new(
            input
                .currency_code
                .clone()
                .unwrap_or_else(|| "USD".to_owned()),
        )?;
        let timezone = input.timezone.clone().unwrap_or_else(|| "UTC".to_owned());

        let id = StoreId::new();
        let now = now_timestamp();
        let database_key = id.as_str().to_owned();
        let mut transaction = self.pool().begin().await?;

        let existing_slug: Option<String> =
            sqlx::query_scalar("SELECT id FROM stores WHERE slug = ?")
                .bind(&slug)
                .fetch_optional(&mut *transaction)
                .await?;
        if existing_slug.is_some() {
            return Err(PlatformError::SlugConflict(slug));
        }
        let existing_domain: Option<String> =
            sqlx::query_scalar("SELECT store_id FROM store_domains WHERE hostname = ?")
                .bind(&hostname)
                .fetch_optional(&mut *transaction)
                .await?;
        if existing_domain.is_some() {
            return Err(PlatformError::DomainConflict(hostname));
        }

        let insert = sqlx::query(
            "INSERT INTO stores
             (id, name, slug, status, database_key, primary_domain, locale, currency_code,
              timezone, metadata, created_at, updated_at)
             VALUES (?, ?, ?, 'provisioning', ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.name.trim())
        .bind(&slug)
        .bind(&database_key)
        .bind(&hostname)
        .bind(locale.as_str())
        .bind(currency.as_str())
        .bind(&timezone)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = insert {
            return Err(if unique_violation(&error) {
                PlatformError::SlugConflict(slug)
            } else {
                error.into()
            });
        }

        let domain_id = format!("dom_{}", uuid::Uuid::now_v7().simple());
        sqlx::query(
            "INSERT INTO store_domains (id, store_id, hostname, is_primary, verified_at, created_at)
             VALUES (?, ?, ?, 1, NULL, ?)",
        )
        .bind(domain_id)
        .bind(id.as_str())
        .bind(&hostname)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        self.get_store(&id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(id.to_string()))
    }

    /// Fetches a store by identifier.
    pub async fn get_store(&self, id: &StoreId) -> PlatformResult<Option<Store>> {
        let row = sqlx::query_as::<_, StoreRow>(&format!(
            "SELECT {STORE_COLUMNS} FROM stores WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(StoreRow::into_store))
    }

    /// Fetches a store by slug.
    pub async fn get_store_by_slug(&self, slug: &str) -> PlatformResult<Option<Store>> {
        let row = sqlx::query_as::<_, StoreRow>(&format!(
            "SELECT {STORE_COLUMNS} FROM stores WHERE slug = ?"
        ))
        .bind(slug)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(StoreRow::into_store))
    }

    /// Resolves a store by hostname.
    pub async fn get_store_by_domain(&self, hostname: &str) -> PlatformResult<Option<Store>> {
        let hostname = normalize_hostname(hostname);
        let row = sqlx::query_as::<_, StoreRow>(&format!(
            "SELECT {} FROM stores s
             JOIN store_domains d ON d.store_id = s.id
             WHERE d.hostname = ?",
            STORE_COLUMNS
                .split(',')
                .map(|column| format!("s.{}", column.trim()))
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .bind(hostname)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(StoreRow::into_store))
    }

    /// Lists stores with offset pagination.
    pub async fn list_stores(&self, pagination: Pagination) -> PlatformResult<PageResult<Store>> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores")
            .fetch_one(self.pool())
            .await?;
        let rows = sqlx::query_as::<_, StoreRow>(&format!(
            "SELECT {STORE_COLUMNS} FROM stores ORDER BY created_at ASC LIMIT ? OFFSET ?"
        ))
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.pool())
        .await?;
        Ok(PageResult::new(
            rows.into_iter().map(StoreRow::into_store).collect(),
            total,
            pagination,
        ))
    }

    /// Lists stores a user is an active member of.
    pub async fn list_stores_for_user(
        &self,
        user_id: &fastcart_kernel::UserId,
        pagination: Pagination,
    ) -> PlatformResult<PageResult<Store>> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM store_memberships WHERE user_id = ? AND status = 'active'",
        )
        .bind(user_id.as_str())
        .fetch_one(self.pool())
        .await?;
        let columns = STORE_COLUMNS
            .split(',')
            .map(|column| format!("s.{}", column.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = sqlx::query_as::<_, StoreRow>(&format!(
            "SELECT {columns} FROM stores s
             JOIN store_memberships m ON m.store_id = s.id
             WHERE m.user_id = ? AND m.status = 'active'
             ORDER BY s.created_at ASC LIMIT ? OFFSET ?"
        ))
        .bind(user_id.as_str())
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.pool())
        .await?;
        Ok(PageResult::new(
            rows.into_iter().map(StoreRow::into_store).collect(),
            total,
            pagination,
        ))
    }

    /// Updates mutable store attributes.
    pub async fn update_store(
        &self,
        id: &StoreId,
        input: UpdateStoreInput,
    ) -> PlatformResult<Store> {
        let current = self
            .get_store(id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(id.to_string()))?;
        if let Some(name) = input.name.as_deref()
            && (name.trim().is_empty() || name.chars().count() > 120)
        {
            return Err(PlatformError::InvalidInput(
                "store name must be between 1 and 120 characters".to_owned(),
            ));
        }
        if let Some(locale) = input.locale.as_deref() {
            Locale::new(locale)?;
        }
        if let Some(currency) = input.currency_code.as_deref() {
            CurrencyCode::new(currency)?;
        }
        let name = input.name.unwrap_or(current.name);
        let locale = input.locale.unwrap_or(current.locale);
        let currency_code = input.currency_code.unwrap_or(current.currency_code);
        let timezone = input.timezone.unwrap_or(current.timezone);
        let metadata = input.metadata.unwrap_or(current.metadata);
        sqlx::query(
            "UPDATE stores SET name = ?, locale = ?, currency_code = ?, timezone = ?,
             metadata = ?, updated_at = ? WHERE id = ?",
        )
        .bind(name)
        .bind(locale)
        .bind(currency_code)
        .bind(timezone)
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.pool())
        .await?;
        self.get_store(id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(id.to_string()))
    }

    /// Applies a validated lifecycle transition.
    pub async fn update_status(&self, id: &StoreId, status: StoreStatus) -> PlatformResult<Store> {
        let current = self
            .get_store(id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(id.to_string()))?;
        if !current.status.can_transition_to(status) {
            return Err(PlatformError::Conflict(format!(
                "cannot transition store {} from {} to {}",
                id, current.status, status
            )));
        }
        sqlx::query("UPDATE stores SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(now_timestamp())
            .bind(id.as_str())
            .execute(self.pool())
            .await?;
        self.get_store(id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(id.to_string()))
    }

    /// Registers a hostname for a store.
    pub async fn add_domain(
        &self,
        store_id: &StoreId,
        hostname: &str,
        is_primary: bool,
    ) -> PlatformResult<StoreDomain> {
        let hostname = normalize_hostname(hostname);
        validate_hostname(&hostname).map_err(PlatformError::InvalidInput)?;
        if self.get_store(store_id).await?.is_none() {
            return Err(PlatformError::StoreNotFound(store_id.to_string()));
        }
        let now = now_timestamp();
        let id = format!("dom_{}", uuid::Uuid::now_v7().simple());
        let mut transaction = self.pool().begin().await?;
        if is_primary {
            sqlx::query("UPDATE store_domains SET is_primary = 0 WHERE store_id = ?")
                .bind(store_id.as_str())
                .execute(&mut *transaction)
                .await?;
        }
        let insert = sqlx::query(
            "INSERT INTO store_domains (id, store_id, hostname, is_primary, verified_at, created_at)
             VALUES (?, ?, ?, ?, NULL, ?)",
        )
        .bind(&id)
        .bind(store_id.as_str())
        .bind(&hostname)
        .bind(i64::from(is_primary))
        .bind(now)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = insert {
            return Err(if unique_violation(&error) {
                PlatformError::DomainConflict(hostname)
            } else {
                error.into()
            });
        }
        if is_primary {
            sqlx::query("UPDATE stores SET primary_domain = ?, updated_at = ? WHERE id = ?")
                .bind(&hostname)
                .bind(now)
                .bind(store_id.as_str())
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(StoreDomain {
            id,
            store_id: store_id.clone(),
            hostname,
            is_primary,
            verified_at: None,
            created_at: now,
        })
    }

    /// Removes a domain. The primary domain cannot be removed directly.
    pub async fn remove_domain(&self, store_id: &StoreId, hostname: &str) -> PlatformResult<()> {
        let hostname = normalize_hostname(hostname);
        let is_primary: Option<i64> = sqlx::query_scalar(
            "SELECT is_primary FROM store_domains WHERE store_id = ? AND hostname = ?",
        )
        .bind(store_id.as_str())
        .bind(&hostname)
        .fetch_optional(self.pool())
        .await?;
        match is_primary {
            None => Err(PlatformError::NotFound(format!(
                "domain not found: {hostname}"
            ))),
            Some(1) => Err(PlatformError::Conflict(
                "cannot remove the primary domain; assign another primary domain first".to_owned(),
            )),
            Some(_) => {
                sqlx::query("DELETE FROM store_domains WHERE store_id = ? AND hostname = ?")
                    .bind(store_id.as_str())
                    .bind(&hostname)
                    .execute(self.pool())
                    .await?;
                Ok(())
            }
        }
    }

    /// Makes an existing domain primary.
    pub async fn set_primary_domain(
        &self,
        store_id: &StoreId,
        hostname: &str,
    ) -> PlatformResult<StoreDomain> {
        let hostname = normalize_hostname(hostname);
        let mut transaction = self.pool().begin().await?;
        let exists: Option<String> =
            sqlx::query_scalar("SELECT id FROM store_domains WHERE store_id = ? AND hostname = ?")
                .bind(store_id.as_str())
                .bind(&hostname)
                .fetch_optional(&mut *transaction)
                .await?;
        let Some(id) = exists else {
            return Err(PlatformError::NotFound(format!(
                "domain not found: {hostname}"
            )));
        };
        sqlx::query("UPDATE store_domains SET is_primary = 0 WHERE store_id = ?")
            .bind(store_id.as_str())
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE store_domains SET is_primary = 1 WHERE id = ?")
            .bind(&id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE stores SET primary_domain = ?, updated_at = ? WHERE id = ?")
            .bind(&hostname)
            .bind(now_timestamp())
            .bind(store_id.as_str())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        let domain = sqlx::query("SELECT created_at, verified_at FROM store_domains WHERE id = ?")
            .bind(&id)
            .fetch_one(self.pool())
            .await?;
        Ok(StoreDomain {
            id,
            store_id: store_id.clone(),
            hostname,
            is_primary: true,
            verified_at: domain.try_get("verified_at")?,
            created_at: domain.try_get("created_at")?,
        })
    }

    /// Lists domains registered to a store.
    pub async fn list_domains(&self, store_id: &StoreId) -> PlatformResult<Vec<StoreDomain>> {
        let rows = sqlx::query(
            "SELECT id, store_id, hostname, is_primary, verified_at, created_at
             FROM store_domains WHERE store_id = ? ORDER BY is_primary DESC, created_at ASC",
        )
        .bind(store_id.as_str())
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(StoreDomain {
                    id: row.try_get("id")?,
                    store_id: StoreId::from_string(row.try_get::<String, _>("store_id")?),
                    hostname: row.try_get("hostname")?,
                    is_primary: row.try_get::<i64, _>("is_primary")? != 0,
                    verified_at: row.try_get("verified_at")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// Marks a domain verified.
    pub async fn verify_domain(&self, store_id: &StoreId, hostname: &str) -> PlatformResult<()> {
        let hostname = normalize_hostname(hostname);
        let result = sqlx::query(
            "UPDATE store_domains SET verified_at = ? WHERE store_id = ? AND hostname = ?",
        )
        .bind(now_timestamp())
        .bind(store_id.as_str())
        .bind(&hostname)
        .execute(self.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Err(PlatformError::NotFound(format!(
                "domain not found: {hostname}"
            )));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl StoreRegistry for PlatformStore {
    async fn create_store(&self, input: CreateStoreInput) -> PlatformResult<Store> {
        PlatformStore::create_store(self, input).await
    }

    async fn get_store(&self, id: &StoreId) -> PlatformResult<Option<Store>> {
        PlatformStore::get_store(self, id).await
    }

    async fn get_store_by_domain(&self, hostname: &str) -> PlatformResult<Option<Store>> {
        PlatformStore::get_store_by_domain(self, hostname).await
    }

    async fn update_store(&self, id: &StoreId, input: UpdateStoreInput) -> PlatformResult<Store> {
        PlatformStore::update_store(self, id, input).await
    }

    async fn update_status(&self, id: &StoreId, status: StoreStatus) -> PlatformResult<Store> {
        PlatformStore::update_status(self, id, status).await
    }
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::{Pagination, StoreId};

    use super::{CreateStoreInput, StoreStatus, UpdateStoreInput};
    use crate::repository::PlatformStore;

    async fn store() -> PlatformStore {
        PlatformStore::in_memory().await.unwrap()
    }

    fn input(slug: &str, domain: &str) -> CreateStoreInput {
        CreateStoreInput::new("Demo Store", slug, domain)
    }

    #[tokio::test]
    async fn creates_store_and_primary_domain() {
        let platform = store().await;
        let store = platform
            .create_store(input("demo", "Shop.Example.com"))
            .await
            .unwrap();

        assert_eq!(store.slug, "demo");
        assert_eq!(store.status, StoreStatus::Provisioning);
        assert_eq!(store.primary_domain.as_deref(), Some("shop.example.com"));

        let resolved = platform
            .get_store_by_domain("shop.example.com:443")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.id, store.id);
    }

    #[tokio::test]
    async fn duplicate_slugs_and_domains_are_rejected() {
        let platform = store().await;
        platform
            .create_store(input("demo", "demo.example.com"))
            .await
            .unwrap();

        let slug_error = platform
            .create_store(input("demo", "other.example.com"))
            .await
            .unwrap_err();
        assert!(matches!(slug_error, crate::PlatformError::SlugConflict(_)));

        let domain_error = platform
            .create_store(input("other", "demo.example.com"))
            .await
            .unwrap_err();
        assert!(matches!(
            domain_error,
            crate::PlatformError::DomainConflict(_)
        ));
    }

    #[tokio::test]
    async fn lifecycle_transitions_are_enforced() {
        let platform = store().await;
        let store = platform
            .create_store(input("demo", "demo.example.com"))
            .await
            .unwrap();

        assert!(
            platform
                .update_status(&store.id, StoreStatus::Deleted)
                .await
                .is_err()
        );
        let active = platform
            .update_status(&store.id, StoreStatus::Active)
            .await
            .unwrap();
        assert!(active.status.can_serve_traffic());

        let suspended = platform
            .update_status(&store.id, StoreStatus::Suspended)
            .await
            .unwrap();
        assert!(!suspended.status.can_serve_traffic());
        platform
            .update_status(&store.id, StoreStatus::Active)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn domains_can_be_added_and_made_primary() {
        let platform = store().await;
        let store = platform
            .create_store(input("demo", "demo.example.com"))
            .await
            .unwrap();

        let added = platform
            .add_domain(&store.id, "www.example.com", false)
            .await
            .unwrap();
        assert!(!added.is_primary);

        let primary = platform
            .set_primary_domain(&store.id, "www.example.com")
            .await
            .unwrap();
        assert!(primary.is_primary);

        let domains = platform.list_domains(&store.id).await.unwrap();
        assert_eq!(domains.len(), 2);
        assert_eq!(domains.iter().filter(|domain| domain.is_primary).count(), 1);

        assert!(
            platform
                .remove_domain(&store.id, "www.example.com")
                .await
                .is_err()
        );
        platform
            .remove_domain(&store.id, "demo.example.com")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn lists_stores_with_pagination() {
        let platform = store().await;
        for index in 0..3 {
            platform
                .create_store(input(
                    &format!("store-{index}"),
                    &format!("store-{index}.example.com"),
                ))
                .await
                .unwrap();
        }
        let page = platform
            .list_stores(Pagination::new(Some(2), Some(0)).unwrap())
            .await
            .unwrap();
        assert_eq!(page.data.len(), 2);
        assert_eq!(page.count, 3);
    }

    #[tokio::test]
    async fn updates_store_attributes() {
        let platform = store().await;
        let store = platform
            .create_store(input("demo", "demo.example.com"))
            .await
            .unwrap();
        let updated = platform
            .update_store(
                &store.id,
                UpdateStoreInput {
                    name: Some("Renamed".into()),
                    locale: Some("fr-FR".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.locale, "fr-FR");
    }

    #[tokio::test]
    async fn rejects_invalid_input() {
        let platform = store().await;
        let mut invalid = input("Bad Slug", "demo.example.com");
        invalid.slug = "Bad Slug".into();
        assert!(platform.create_store(invalid).await.is_err());

        let invalid = input("demo", "http://demo.example.com");
        assert!(platform.create_store(invalid).await.is_err());
    }

    #[tokio::test]
    async fn missing_store_is_not_found() {
        let platform = store().await;
        assert!(platform.get_store(&StoreId::new()).await.unwrap().is_none());
    }
}
