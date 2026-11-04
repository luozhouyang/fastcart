//! Regions, supported currencies and storefront publishable API keys.

use fastcart_infrastructure::{AuditActor, StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CountryCode, CurrencyCode, Metadata, PageResult, Pagination, now_timestamp,
    validate_metadata,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

/// A country/currency/fulfillment configuration for a storefront.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Region {
    /// Region identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// ISO 4217 currency used by prices in this region.
    pub currency_code: String,
    /// ISO 3166-1 alpha-2 countries served by this region.
    pub countries: Vec<String>,
    /// Tax region used for calculations.
    pub tax_region_id: Option<String>,
    /// Payment provider identifiers enabled in this region.
    pub payment_providers: Vec<String>,
    /// Whether this is the storefront default region.
    pub is_default: bool,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

/// Input for creating a region.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateRegionInput {
    /// Display name.
    pub name: String,
    /// ISO currency code.
    pub currency_code: String,
    /// ISO country codes.
    pub countries: Vec<String>,
    /// Optional tax region.
    pub tax_region_id: Option<String>,
    /// Enabled payment providers.
    pub payment_providers: Vec<String>,
    /// Whether this is the default region.
    pub is_default: bool,
    /// Extension metadata.
    pub metadata: Metadata,
}

/// Input for partially updating a region.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateRegionInput {
    /// New name.
    pub name: Option<String>,
    /// New currency.
    pub currency_code: Option<String>,
    /// Replacement countries.
    pub countries: Option<Vec<String>>,
    /// Replacement tax region; `Some(None)` clears it.
    pub tax_region_id: Option<Option<String>>,
    /// Replacement payment providers.
    pub payment_providers: Option<Vec<String>>,
    /// Whether this is the default region.
    pub is_default: Option<bool>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// A storefront publishable API key scoped to sales channels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PublishableApiKey {
    /// Key identifier.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Display prefix.
    pub token_prefix: String,
    /// Sales channel identifiers this key may access.
    pub sales_channel_ids: Vec<String>,
    /// Revocation timestamp.
    pub revoked_at: Option<i64>,
    /// Last use timestamp.
    pub last_used_at: Option<i64>,
    /// Creation timestamp.
    pub created_at: i64,
}

/// A newly created publishable key; the secret is shown once.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreatedPublishableApiKey {
    /// Full `pk_...` token.
    pub token: String,
    /// Persisted metadata.
    pub key: PublishableApiKey,
}

/// Input for creating a publishable key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreatePublishableApiKeyInput {
    /// Human-readable title.
    pub title: String,
    /// Sales channels the key may use.
    pub sales_channel_ids: Vec<String>,
}

/// Region and publishable-key operations.
#[derive(Clone, Debug)]
pub struct RegionService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

impl RegionService {
    /// Creates a service over an already-migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db, audit: None }
    }

    /// Records region and publishable-key mutations in the store audit outbox.
    ///
    /// When an actor is set, audited mutations append their outbox row in the
    /// same transaction as the change, so a rollback leaves no audit behind.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

    /// Appends one audit entry to the outbox inside a mutation transaction.
    async fn append_audit(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        data: serde_json::Value,
    ) -> Result<(), AppError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, resource_type, resource_id, data)
                .await
                .map_err(database_error)?;
        }
        Ok(())
    }

    /// Returns the default region.
    pub async fn default_region(&self) -> Result<Option<Region>, AppError> {
        let row = sqlx::query(
            "SELECT id, name, currency_code, countries, tax_region_id, payment_providers,
                    is_default, metadata, created_at, updated_at
             FROM regions WHERE is_default = 1",
        )
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        row.map(region_from_row).transpose()
    }

    /// Creates a region, optionally making it the default.
    pub async fn create_region(&self, input: CreateRegionInput) -> Result<Region, AppError> {
        let name = validate_name(&input.name, "region name")?;
        let currency = CurrencyCode::new(&input.currency_code)?.as_str().to_owned();
        let countries = normalize_countries(&input.countries)?;
        validate_metadata(&input.metadata)?;
        let id = format!("reg_{}", uuid::Uuid::now_v7().simple());
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        if input.is_default {
            sqlx::query("UPDATE regions SET is_default = 0, updated_at = ? WHERE is_default = 1")
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        }
        sqlx::query(
            "INSERT INTO regions
             (id, name, currency_code, countries, tax_region_id, payment_providers,
              is_default, metadata, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&name)
        .bind(&currency)
        .bind(serde_json::to_string(&countries).unwrap_or_else(|_| "[]".to_owned()))
        .bind(input.tax_region_id.as_deref())
        .bind(serde_json::to_string(&input.payment_providers).unwrap_or_else(|_| "[]".to_owned()))
        .bind(input.is_default)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "region.created",
            "region",
            &id,
            serde_json::json!({"name": name, "is_default": input.is_default}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.get_region(&id).await
    }

    /// Loads a region by identifier.
    pub async fn get_region(&self, id: &str) -> Result<Region, AppError> {
        let row = sqlx::query(
            "SELECT id, name, currency_code, countries, tax_region_id, payment_providers,
                    is_default, metadata, created_at, updated_at
             FROM regions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("region", id))?;
        region_from_row(row)
    }

    /// Lists regions newest first.
    pub async fn list_regions(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<Region>, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM regions")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query(
            "SELECT id, name, currency_code, countries, tax_region_id, payment_providers,
                    is_default, metadata, created_at, updated_at
             FROM regions ORDER BY created_at ASC, id ASC LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let mut regions = Vec::with_capacity(rows.len());
        for row in rows {
            regions.push(region_from_row(row)?);
        }
        Ok(PageResult::new(regions, count, pagination))
    }

    /// Partially updates a region.
    pub async fn update_region(
        &self,
        id: &str,
        input: UpdateRegionInput,
    ) -> Result<Region, AppError> {
        let now = now_timestamp();
        // `begin` takes the write lock up front, so the read below and the
        // write it feeds see the same snapshot.
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let row = sqlx::query(
            "SELECT id, name, currency_code, countries, tax_region_id, payment_providers,
                    is_default, metadata, created_at, updated_at
             FROM regions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("region", id))?;
        let existing = region_from_row(row)?;

        let name = input.name.as_deref().map_or_else(
            || Ok(existing.name.clone()),
            |name| validate_name(name, "region name"),
        )?;
        let currency = match input.currency_code.as_deref() {
            Some(currency) => CurrencyCode::new(currency)?.as_str().to_owned(),
            None => existing.currency_code.clone(),
        };
        let countries = match input.countries.as_deref() {
            Some(countries) => normalize_countries(countries)?,
            None => existing.countries.clone(),
        };
        let tax_region_id = input
            .tax_region_id
            .unwrap_or(existing.tax_region_id.clone());
        let payment_providers = input
            .payment_providers
            .unwrap_or_else(|| existing.payment_providers.clone());
        let metadata = input.metadata.unwrap_or_else(|| existing.metadata.clone());
        validate_metadata(&metadata)?;
        let is_default = input.is_default.unwrap_or(existing.is_default);
        let changed = name != existing.name
            || currency != existing.currency_code
            || countries != existing.countries
            || tax_region_id != existing.tax_region_id
            || payment_providers != existing.payment_providers
            || metadata != existing.metadata
            || is_default != existing.is_default;
        if is_default {
            // Always clear any other default before setting this one. The
            // partial unique index only allows a single default row.
            sqlx::query(
                "UPDATE regions SET is_default = 0, updated_at = ? \
                 WHERE is_default = 1 AND id <> ?",
            )
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }
        let updated = sqlx::query(
            "UPDATE regions SET name = ?, currency_code = ?, countries = ?, tax_region_id = ?,
             payment_providers = ?, is_default = ?, metadata = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&currency)
        .bind(serde_json::to_string(&countries).unwrap_or_else(|_| "[]".to_owned()))
        .bind(tax_region_id.as_deref())
        .bind(serde_json::to_string(&payment_providers).unwrap_or_else(|_| "[]".to_owned()))
        .bind(is_default)
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_conflict(error, "another region is already the default"))?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::not_found("region", id));
        }
        if changed {
            self.append_audit(
                &mut tx,
                "region.updated",
                "region",
                id,
                serde_json::json!({"name": name, "is_default": is_default}),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        self.get_region(id).await
    }

    /// Deletes a non-default region.
    pub async fn delete_region(&self, id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let row = sqlx::query(
            "SELECT id, name, currency_code, countries, tax_region_id, payment_providers,
                    is_default, metadata, created_at, updated_at
             FROM regions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("region", id))?;
        let region = region_from_row(row)?;
        if region.is_default {
            return Err(AppError::conflict("the default region cannot be deleted"));
        }
        sqlx::query("DELETE FROM regions WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "region.deleted",
            "region",
            id,
            serde_json::Value::Null,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Creates a publishable key scoped to sales channels.
    pub async fn create_publishable_key(
        &self,
        input: CreatePublishableApiKeyInput,
    ) -> Result<CreatedPublishableApiKey, AppError> {
        let title = validate_name(&input.title, "publishable key title")?;
        if input.sales_channel_ids.is_empty() {
            return Err(AppError::validation(
                "a publishable key must be scoped to at least one sales channel",
            ));
        }
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        for channel_id in &input.sales_channel_ids {
            let channel: Option<(String, i64)> =
                sqlx::query_as("SELECT id, is_disabled FROM sales_channels WHERE id = ?")
                    .bind(channel_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(database_error)?;
            match channel {
                Some((_, 0)) => {}
                Some(_) => {
                    return Err(AppError::validation(format!(
                        "sales channel {channel_id} is disabled"
                    )));
                }
                None => {
                    return Err(AppError::not_found("sales channel", channel_id));
                }
            }
        }
        let token = format!("pk_{}", random_token(32));
        let id = format!("pk_{}", uuid::Uuid::now_v7().simple());
        let prefix: String = token.chars().take(12).collect();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO publishable_api_keys
             (id, title, token_prefix, token_hash, sales_channel_ids, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&title)
        .bind(&prefix)
        .bind(hash_token(&token))
        .bind(serde_json::to_string(&input.sales_channel_ids).unwrap_or_else(|_| "[]".to_owned()))
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "publishable_key.created",
            "publishable_api_key",
            &id,
            serde_json::json!({"sales_channel_ids": input.sales_channel_ids.clone()}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(CreatedPublishableApiKey {
            token,
            key: PublishableApiKey {
                id,
                title,
                token_prefix: prefix,
                sales_channel_ids: input.sales_channel_ids,
                revoked_at: None,
                last_used_at: None,
                created_at: now,
            },
        })
    }

    /// Lists publishable keys without secrets.
    pub async fn list_publishable_keys(&self) -> Result<Vec<PublishableApiKey>, AppError> {
        let rows = sqlx::query(
            "SELECT id, title, token_prefix, sales_channel_ids, revoked_at, last_used_at,
                    created_at FROM publishable_api_keys ORDER BY created_at DESC",
        )
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(publishable_key_from_row).collect()
    }

    /// Revokes a publishable key.
    pub async fn revoke_publishable_key(&self, id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query(
            "UPDATE publishable_api_keys SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("publishable API key", id));
        }
        self.append_audit(
            &mut tx,
            "publishable_key.revoked",
            "publishable_api_key",
            id,
            serde_json::Value::Null,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Validates a storefront publishable key and returns its allowed channels.
    pub async fn authenticate_publishable_key(
        &self,
        token: &str,
    ) -> Result<PublishableApiKey, AppError> {
        if !token.starts_with("pk_") || token.len() < 40 {
            return Err(AppError::Unauthorized);
        }
        let row = sqlx::query(
            "SELECT id, title, token_prefix, sales_channel_ids, revoked_at, last_used_at,
                    created_at FROM publishable_api_keys WHERE token_hash = ?",
        )
        .bind(hash_token(token))
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or(AppError::Unauthorized)?;
        let key = publishable_key_from_row(row)?;
        if key.revoked_at.is_some() {
            return Err(AppError::Unauthorized);
        }
        sqlx::query("UPDATE publishable_api_keys SET last_used_at = ? WHERE id = ?")
            .bind(now_timestamp())
            .bind(&key.id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(key)
    }
}

fn region_from_row(row: SqliteRow) -> Result<Region, AppError> {
    let countries: Vec<String> = serde_json::from_str(
        row.try_get::<String, _>("countries")
            .map_err(database_error)?
            .as_str(),
    )
    .unwrap_or_default();
    let payment_providers: Vec<String> = serde_json::from_str(
        row.try_get::<String, _>("payment_providers")
            .map_err(database_error)?
            .as_str(),
    )
    .unwrap_or_default();
    Ok(Region {
        id: row.try_get("id").map_err(database_error)?,
        name: row.try_get("name").map_err(database_error)?,
        currency_code: row.try_get("currency_code").map_err(database_error)?,
        countries,
        tax_region_id: row.try_get("tax_region_id").map_err(database_error)?,
        payment_providers,
        is_default: row
            .try_get::<i64, _>("is_default")
            .map_err(database_error)?
            != 0,
        metadata: Metadata::from_json_str(
            &row.try_get::<String, _>("metadata")
                .map_err(database_error)?,
        ),
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn publishable_key_from_row(row: SqliteRow) -> Result<PublishableApiKey, AppError> {
    let sales_channel_ids: Vec<String> = serde_json::from_str(
        row.try_get::<String, _>("sales_channel_ids")
            .map_err(database_error)?
            .as_str(),
    )
    .unwrap_or_default();
    Ok(PublishableApiKey {
        id: row.try_get("id").map_err(database_error)?,
        title: row.try_get("title").map_err(database_error)?,
        token_prefix: row.try_get("token_prefix").map_err(database_error)?,
        sales_channel_ids,
        revoked_at: row.try_get("revoked_at").map_err(database_error)?,
        last_used_at: row.try_get("last_used_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
    })
}

fn validate_name(value: &str, field: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 200 {
        return Err(AppError::validation(format!(
            "{field} must be between 1 and 200 characters"
        )));
    }
    Ok(value.to_owned())
}

fn normalize_countries(countries: &[String]) -> Result<Vec<String>, AppError> {
    if countries.is_empty() {
        return Err(AppError::validation(
            "a region must include at least one country",
        ));
    }
    let mut normalized = std::collections::BTreeSet::new();
    for country in countries {
        normalized.insert(CountryCode::new(country.clone())?.as_str().to_owned());
    }
    Ok(normalized.into_iter().collect())
}

fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buffer);
    hex::encode(buffer)
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn unique_conflict(error: sqlx::Error, message: &str) -> AppError {
    if let sqlx::Error::Database(database) = &error
        && (database.is_unique_violation()
            || database
                .message()
                .to_ascii_lowercase()
                .contains("unique constraint"))
    {
        return AppError::conflict(message);
    }
    database_error(error)
}
