//! SQLite-backed store settings and sales-channel service.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, Metadata, PageResult, Pagination, now_timestamp, validate_length, validate_metadata,
};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::domain::{CreateSalesChannelInput, SalesChannel, UpdateSalesChannelInput};

/// Maximum accepted setting key length, in characters.
pub const MAX_SETTING_KEY_LENGTH: usize = 120;
/// Maximum accepted setting value size, in bytes.
pub const MAX_SETTING_VALUE_BYTES: usize = 8192;
/// Maximum accepted sales-channel name length, in characters.
pub const MAX_SALES_CHANNEL_NAME_LENGTH: usize = 200;

const SALES_CHANNEL_COLUMNS: &str =
    "id, name, description, is_disabled, metadata, created_at, updated_at";

/// Store-level settings and sales-channel operations.
#[derive(Clone, Debug)]
pub struct StoreService {
    db: StoreDatabase,
}

impl StoreService {
    /// Creates a service over an already-migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    /// Returns a setting value, or `None` when the key is absent.
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, AppError> {
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
            .bind(key.trim())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)
    }

    /// Creates or replaces a setting after validating the key and value.
    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), AppError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(AppError::validation("setting key must not be empty"));
        }
        if key.chars().count() > MAX_SETTING_KEY_LENGTH {
            return Err(AppError::validation(format!(
                "setting key must not exceed {MAX_SETTING_KEY_LENGTH} characters"
            )));
        }
        if value.len() > MAX_SETTING_VALUE_BYTES {
            return Err(AppError::validation(format!(
                "setting value must not exceed {MAX_SETTING_VALUE_BYTES} bytes"
            )));
        }

        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(value)
        .bind(now_timestamp())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }

    /// Returns every setting ordered by key.
    pub async fn get_all_settings(&self) -> Result<BTreeMap<String, String>, AppError> {
        let rows = sqlx::query("SELECT key, value FROM settings ORDER BY key")
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let mut settings = BTreeMap::new();
        for row in rows {
            let key: String = row.try_get("key").map_err(database_error)?;
            let value: String = row.try_get("value").map_err(database_error)?;
            settings.insert(key, value);
        }
        Ok(settings)
    }

    /// Deletes a setting. Returns `true` when a row was removed.
    pub async fn delete_setting(&self, key: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM settings WHERE key = ?")
            .bind(key.trim())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(result.rows_affected() > 0)
    }

    /// Creates a sales channel.
    pub async fn create_sales_channel(
        &self,
        input: CreateSalesChannelInput,
    ) -> Result<SalesChannel, AppError> {
        let name = validate_channel_name(&input.name)?;
        validate_metadata(&input.metadata)?;
        self.ensure_channel_name_available(&name, None).await?;

        let id = new_sales_channel_id();
        let now = now_timestamp();
        let description = normalize_optional(input.description);
        sqlx::query(
            "INSERT INTO sales_channels \
             (id, name, description, is_disabled, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, 0, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&name)
        .bind(&description)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_or_database(error, "sales channel name already exists"))?;

        self.get_sales_channel(&id).await
    }

    /// Returns a sales channel by identifier.
    pub async fn get_sales_channel(&self, id: &str) -> Result<SalesChannel, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {SALES_CHANNEL_COLUMNS} FROM sales_channels WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("sales channel", id))?;
        sales_channel_from_row(&row)
    }

    /// Lists sales channels with pagination.
    pub async fn list_sales_channels(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<SalesChannel>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sales_channels")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query(&format!(
            "SELECT {SALES_CHANNEL_COLUMNS} FROM sales_channels \
             ORDER BY created_at ASC, id ASC LIMIT ? OFFSET ?"
        ))
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;

        let mut channels = Vec::with_capacity(rows.len());
        for row in &rows {
            channels.push(sales_channel_from_row(row)?);
        }
        Ok(PageResult::new(channels, count, pagination))
    }

    /// Partially updates a sales channel.
    pub async fn update_sales_channel(
        &self,
        id: &str,
        input: UpdateSalesChannelInput,
    ) -> Result<SalesChannel, AppError> {
        let existing = self.get_sales_channel(id).await?;

        let name = match input.name {
            Some(value) => validate_channel_name(&value)?,
            None => existing.name.clone(),
        };
        if name != existing.name {
            self.ensure_channel_name_available(&name, Some(id)).await?;
        }
        let description = match input.description {
            Some(value) => normalize_optional(Some(value)),
            None => existing.description.clone(),
        };
        let metadata = match input.metadata {
            Some(metadata) => {
                validate_metadata(&metadata)?;
                metadata
            }
            None => existing.metadata.clone(),
        };

        sqlx::query(
            "UPDATE sales_channels SET name = ?, description = ?, metadata = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&description)
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_or_database(error, "sales channel name already exists"))?;

        self.get_sales_channel(id).await
    }

    /// Disables a sales channel.
    pub async fn disable_sales_channel(&self, id: &str) -> Result<SalesChannel, AppError> {
        self.set_channel_disabled(id, true).await
    }

    /// Enables a sales channel.
    pub async fn enable_sales_channel(&self, id: &str) -> Result<SalesChannel, AppError> {
        self.set_channel_disabled(id, false).await
    }

    async fn set_channel_disabled(
        &self,
        id: &str,
        disabled: bool,
    ) -> Result<SalesChannel, AppError> {
        let result =
            sqlx::query("UPDATE sales_channels SET is_disabled = ?, updated_at = ? WHERE id = ?")
                .bind(if disabled { 1_i64 } else { 0_i64 })
                .bind(now_timestamp())
                .bind(id)
                .execute(self.db.pool())
                .await
                .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("sales channel", id));
        }
        self.get_sales_channel(id).await
    }

    async fn ensure_channel_name_available(
        &self,
        name: &str,
        exclude_id: Option<&str>,
    ) -> Result<(), AppError> {
        let count: i64 = match exclude_id {
            Some(exclude_id) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM sales_channels WHERE name = ? AND id <> ?")
                    .bind(name)
                    .bind(exclude_id)
                    .fetch_one(self.db.pool())
                    .await
                    .map_err(database_error)?
            }
            None => sqlx::query_scalar("SELECT COUNT(*) FROM sales_channels WHERE name = ?")
                .bind(name)
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?,
        };
        if count > 0 {
            return Err(AppError::conflict(format!(
                "sales channel name already exists: {name}"
            )));
        }
        Ok(())
    }
}

fn validate_channel_name(value: &str) -> Result<String, AppError> {
    validate_length(value, "sales channel name", MAX_SALES_CHANNEL_NAME_LENGTH)?;
    Ok(value.trim().to_owned())
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn checked_pagination(pagination: Pagination) -> Result<Pagination, AppError> {
    Pagination::new(Some(pagination.limit), Some(pagination.offset))
}

fn sales_channel_from_row(row: &SqliteRow) -> Result<SalesChannel, AppError> {
    let is_disabled: i64 = row.try_get("is_disabled").map_err(database_error)?;
    let metadata: String = row.try_get("metadata").map_err(database_error)?;
    Ok(SalesChannel {
        id: row.try_get("id").map_err(database_error)?,
        name: row.try_get("name").map_err(database_error)?,
        description: row.try_get("description").map_err(database_error)?,
        is_disabled: is_disabled != 0,
        metadata: Metadata::from_json_str(&metadata),
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn unique_or_database(error: sqlx::Error, conflict_message: &str) -> AppError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            AppError::conflict(conflict_message)
        }
        _ => database_error(error),
    }
}

fn new_sales_channel_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("sc_{nanos:x}{counter:04x}")
}
