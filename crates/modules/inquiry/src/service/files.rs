//! Pending attachment lifecycle.

use super::*;

impl InquiryFileService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    /// Records a pending upload after validating it against the published form.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_pending(
        &self,
        form_handle: &str,
        field_key: &str,
        storage_key: String,
        original_name: Option<String>,
        content_type: String,
        size_bytes: i64,
        checksum_sha256: String,
        expires_at: i64,
    ) -> Result<InquiryFile, AppError> {
        let published = InquiryFormService::new(self.db.clone())
            .published_by_handle(form_handle)
            .await?;
        let form = published.form;
        let revision = published.revision;

        if !revision.settings.allow_attachments {
            return Err(AppError::validation(
                "this inquiry form does not accept attachments",
            ));
        }
        let field = revision
            .fields
            .iter()
            .find(|field| field.key == field_key)
            .ok_or_else(|| {
                AppError::validation(format!("unknown attachment field: {field_key}"))
            })?;
        if field.kind != FieldKind::File {
            return Err(AppError::validation(format!(
                "field {field_key} is not a file field"
            )));
        }
        let config = field.file.as_ref().ok_or_else(|| {
            AppError::internal(format!("file field {field_key} has no configuration"))
        })?;

        let content_type = content_type.trim().to_owned();
        if !ALLOWED_FILE_TYPES.contains(&content_type.as_str()) {
            return Err(AppError::validation(format!(
                "unsupported attachment content type: {content_type}"
            )));
        }
        if !config
            .accept
            .iter()
            .any(|accepted| accepted == &content_type)
        {
            return Err(AppError::validation(format!(
                "field {field_key} does not accept {content_type}"
            )));
        }

        if size_bytes <= 0 {
            return Err(AppError::validation(
                "attachment size must be greater than zero",
            ));
        }
        let size = u64::try_from(size_bytes)
            .map_err(|_| AppError::validation("invalid attachment size"))?;
        let max_bytes = config.max_bytes.min(MAX_FILE_BYTES);
        if size > max_bytes {
            return Err(AppError::validation(format!(
                "attachment must not exceed {max_bytes} bytes"
            )));
        }

        let now = now_timestamp();
        if expires_at <= now {
            return Err(AppError::validation(
                "attachment expiry must be in the future",
            ));
        }

        let storage_key = storage_key.trim().to_owned();
        if storage_key.is_empty() {
            return Err(AppError::validation("storage_key must not be empty"));
        }
        let checksum_sha256 = checksum_sha256.trim().to_owned();
        if checksum_sha256.is_empty() {
            return Err(AppError::validation("checksum_sha256 must not be empty"));
        }
        let original_name = original_name
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let id = InquiryFileId::new();
        sqlx::query(
            "INSERT INTO inquiry_files \
             (id, form_id, inquiry_id, field_key, storage_key, original_name, content_type, \
              size_bytes, checksum_sha256, status, expires_at, created_at, claimed_at) \
             VALUES (?, ?, NULL, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, NULL)",
        )
        .bind(id.as_str())
        .bind(form.id.as_str())
        .bind(field_key)
        .bind(&storage_key)
        .bind(&original_name)
        .bind(&content_type)
        .bind(size_bytes)
        .bind(&checksum_sha256)
        .bind(expires_at)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_conflict(error, "attachment storage key already exists"))?;

        self.get(&id).await
    }

    /// Loads one attachment by identifier.
    pub async fn get(&self, id: &InquiryFileId) -> Result<InquiryFile, AppError> {
        load_file_row(&self.db, id).await?.into_file()
    }

    /// Lists pending uploads whose expiry has passed.
    pub async fn expired_pending(&self, limit: i64) -> Result<Vec<InquiryFile>, AppError> {
        let now = now_timestamp();
        let limit = limit.clamp(1, 1_000);
        let sql = format!(
            "SELECT {FILE_COLUMNS} FROM inquiry_files \
             WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at <= ? \
             ORDER BY expires_at ASC, id ASC LIMIT ?"
        );
        let rows = sqlx::query_as::<_, InquiryFileRow>(&sql)
            .bind(now)
            .bind(limit)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        rows.into_iter().map(InquiryFileRow::into_file).collect()
    }

    /// Marks a pending upload as expired. Already claimed or expired rows are
    /// left untouched.
    pub async fn mark_expired(&self, id: &InquiryFileId) -> Result<(), AppError> {
        let result = sqlx::query(
            "UPDATE inquiry_files SET status = 'expired' WHERE id = ? AND status = 'pending'",
        )
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            let exists: Option<i64> =
                sqlx::query_scalar("SELECT 1 FROM inquiry_files WHERE id = ?")
                    .bind(id.as_str())
                    .fetch_optional(self.db.pool())
                    .await
                    .map_err(database_error)?;
            if exists.is_none() {
                return Err(AppError::not_found("inquiry file", id));
            }
        }
        Ok(())
    }
}
