//! SQLite-backed application service for the media asset registry.

use fastcart_infrastructure::{AuditActor, StoreDatabase, database_error};
use fastcart_kernel::{AppError, PageResult, Pagination, now_timestamp};
use serde_json::Value;
use sqlx::FromRow;

use crate::domain::{Asset, AssetFilter, CreateAssetInput};
use crate::purge::insert_asset_blob_purge;

const ASSET_COLUMNS: &str = "id, filename, content_type, size_bytes, storage_key, alt, width, \
     height, created_by, created_at, deleted_at";

/// Application service for store-scoped media asset metadata.
#[derive(Clone, Debug)]
pub struct AssetService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

impl AssetService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db, audit: None }
    }

    /// Records asset mutations in the store audit outbox.
    ///
    /// When an actor is set, every mutating method appends its audit entry in
    /// the same transaction as the change, so a crash cannot separate them.
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
        data: Value,
    ) -> Result<(), AppError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, resource_type, resource_id, data)
                .await
                .map_err(database_error)?;
        }
        Ok(())
    }

    /// Registers a new asset and returns the stored row.
    pub async fn create_asset(&self, input: CreateAssetInput) -> Result<Asset, AppError> {
        input.validate()?;

        let filename = input.filename.trim().to_owned();
        let content_type = input.content_type.trim().to_owned();
        let storage_key = input.storage_key.trim().to_owned();
        let alt = input
            .alt
            .map(|alt| alt.trim().to_owned())
            .filter(|alt| !alt.is_empty());

        let id = format!("asset_{}", uuid::Uuid::now_v7().simple());
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "INSERT INTO assets (id, filename, content_type, size_bytes, storage_key, alt, width, \
             height, created_by, created_at, deleted_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(&id)
        .bind(&filename)
        .bind(&content_type)
        .bind(input.size_bytes)
        .bind(&storage_key)
        .bind(alt.as_deref())
        .bind(input.width)
        .bind(input.height)
        .bind(input.created_by.as_deref())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_conflict(error, "asset storage key already exists"))?;
        self.append_audit(
            &mut tx,
            "asset.created",
            "asset",
            &id,
            serde_json::json!({
                "filename": &filename,
                "content_type": &content_type,
                "size_bytes": input.size_bytes,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_asset(&id).await
    }

    /// Loads a live asset by identifier. Soft-deleted assets are not returned.
    pub async fn get_asset(&self, id: &str) -> Result<Asset, AppError> {
        let sql = format!("SELECT {ASSET_COLUMNS} FROM assets WHERE id = ? AND deleted_at IS NULL");
        sqlx::query_as::<_, AssetRow>(&sql)
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("asset", id))
            .map(AssetRow::into_asset)
    }

    /// Lists live assets, newest first, with an optional search filter.
    ///
    /// Search is case-insensitive across filename and alternative text, and
    /// LIKE wildcards in user input are escaped. An absent or blank search
    /// matches every live asset.
    pub async fn list_assets(
        &self,
        filter: AssetFilter,
        pagination: Pagination,
    ) -> Result<PageResult<Asset>, AppError> {
        let pattern = like_pattern(filter.search.as_deref());
        let conditions = if pattern.is_some() {
            " WHERE deleted_at IS NULL AND (LOWER(filename) LIKE LOWER(?) ESCAPE '\\' \
             OR LOWER(COALESCE(alt, '')) LIKE LOWER(?) ESCAPE '\\')"
        } else {
            " WHERE deleted_at IS NULL"
        };

        let count_sql = format!("SELECT COUNT(*) FROM assets{conditions}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(pattern) = pattern.as_deref() {
            count_query = count_query.bind(pattern).bind(pattern);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT {ASSET_COLUMNS} FROM assets{conditions} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query_as::<_, AssetRow>(&list_sql);
        if let Some(pattern) = pattern.as_deref() {
            list_query = list_query.bind(pattern).bind(pattern);
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let data = rows.into_iter().map(AssetRow::into_asset).collect();
        Ok(PageResult::new(data, count, pagination))
    }

    /// Soft-deletes an asset and returns the deleted row.
    ///
    /// The metadata update and a durable blob-purge obligation commit together.
    /// Callers attempt the storage delete immediately and may retry it from the
    /// pending purge row; a failed delete therefore cannot leak the object.
    /// Deleting an already deleted or unknown asset returns a not-found error.
    pub async fn delete_asset(&self, id: &str) -> Result<Asset, AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {ASSET_COLUMNS} FROM assets WHERE id = ? AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, AssetRow>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("asset", id))?;

        let now = now_timestamp();
        let result =
            sqlx::query("UPDATE assets SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("asset", id));
        }
        insert_asset_blob_purge(&mut *tx, &row.storage_key)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "asset.deleted",
            "asset",
            id,
            serde_json::json!({ "filename": &row.filename }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        let mut asset = row.into_asset();
        asset.deleted_at = Some(now);
        Ok(asset)
    }
}

/// Builds a case-insensitive `LIKE` pattern, escaping wildcards.
fn like_pattern(query: Option<&str>) -> Option<String> {
    let query = query?.trim();
    if query.is_empty() {
        return None;
    }
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for character in query.chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    Some(pattern)
}

fn unique_conflict(error: sqlx::Error, message: &str) -> AppError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            AppError::conflict(message)
        }
        _ => database_error(error),
    }
}

#[derive(Debug, FromRow)]
struct AssetRow {
    id: String,
    filename: String,
    content_type: String,
    size_bytes: i64,
    storage_key: String,
    alt: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    created_by: Option<String>,
    created_at: i64,
    deleted_at: Option<i64>,
}

impl AssetRow {
    fn into_asset(self) -> Asset {
        Asset {
            id: self.id,
            filename: self.filename,
            content_type: self.content_type,
            size_bytes: self.size_bytes,
            storage_key: self.storage_key,
            alt: self.alt,
            width: self.width,
            height: self.height,
            created_by: self.created_by,
            created_at: self.created_at,
            deleted_at: self.deleted_at,
        }
    }
}
