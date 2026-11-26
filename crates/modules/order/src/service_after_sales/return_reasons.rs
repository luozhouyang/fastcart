//! Configurable return reasons: create, list, get, update and delete.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, Metadata, now_timestamp, validate_length, validate_metadata};

use crate::after_sales::{
    CreateReturnReasonInput, ReturnReason, ReturnReasonId, UpdateReturnReasonInput,
};
use crate::service::OrderService;

use super::normalize_text;

/// Stable machine-readable reason values are short slugs.
const MAX_VALUE_LENGTH: usize = 120;
/// Labels are human-readable and snapshotted onto after-sales records.
const MAX_LABEL_LENGTH: usize = 200;

const REASON_COLUMNS: &str = "id, value, label, description, metadata, created_at, updated_at";

#[derive(sqlx::FromRow)]
struct ReturnReasonRow {
    id: String,
    value: String,
    label: String,
    description: Option<String>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl ReturnReasonRow {
    fn into_domain(self) -> ReturnReason {
        ReturnReason {
            id: ReturnReasonId::from_string(self.id),
            value: self.value,
            label: self.label,
            description: self.description,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

fn normalize_value(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    validate_length(value, "return reason value", MAX_VALUE_LENGTH)?;
    Ok(value.to_owned())
}

fn normalize_label(label: &str) -> Result<String, AppError> {
    let label = label.trim();
    validate_length(label, "return reason label", MAX_LABEL_LENGTH)?;
    Ok(label.to_owned())
}

fn unique_or_database(error: sqlx::Error, conflict_message: &str) -> AppError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            AppError::conflict(conflict_message)
        }
        _ => database_error(error),
    }
}

/// Loads the label a new after-sales record snapshots.
///
/// The caller writes the label into the record's free-text `reason` column
/// instead of storing a reference, inside the same transaction that inserts
/// the record. Records therefore keep what was chosen at creation time even
/// after the reason is renamed or deleted.
pub(crate) async fn fetch_return_reason_label(
    executor: &mut sqlx::SqliteConnection,
    reason_id: &ReturnReasonId,
) -> Result<String, AppError> {
    sqlx::query_scalar::<_, String>("SELECT label FROM return_reasons WHERE id = ?")
        .bind(reason_id.as_str())
        .fetch_optional(executor)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("return reason", reason_id))
}

impl OrderService {
    /// Creates a return reason with a unique value.
    pub async fn create_return_reason(
        &self,
        input: CreateReturnReasonInput,
    ) -> Result<ReturnReason, AppError> {
        let value = normalize_value(&input.value)?;
        let label = normalize_label(&input.label)?;
        validate_metadata(&input.metadata)?;
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM return_reasons WHERE value = ?")
                .bind(&value)
                .fetch_optional(self.database().pool())
                .await
                .map_err(database_error)?;
        if existing.is_some() {
            return Err(AppError::conflict(format!(
                "return reason value already exists: {value}"
            )));
        }
        let id = ReturnReasonId::new();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO return_reasons
                (id, value, label, description, metadata, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(&value)
        .bind(&label)
        .bind(normalize_text(input.description))
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(self.database().pool())
        .await
        .map_err(|error| {
            unique_or_database(
                error,
                &format!("return reason value already exists: {value}"),
            )
        })?;
        self.load_return_reason(&id).await
    }

    /// Lists return reasons, oldest first.
    pub async fn list_return_reasons(&self) -> Result<Vec<ReturnReason>, AppError> {
        let rows = sqlx::query_as::<_, ReturnReasonRow>(&format!(
            "SELECT {REASON_COLUMNS} FROM return_reasons ORDER BY created_at ASC, rowid ASC"
        ))
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        Ok(rows.into_iter().map(ReturnReasonRow::into_domain).collect())
    }

    /// Loads one return reason.
    pub async fn get_return_reason(&self, id: &ReturnReasonId) -> Result<ReturnReason, AppError> {
        self.load_return_reason(id).await
    }

    /// Partially updates a return reason.
    ///
    /// Omitted fields keep their stored value; an explicit `null` description
    /// clears it. Editing a reason never rewrites the records that already
    /// snapshotted its label.
    pub async fn update_return_reason(
        &self,
        id: &ReturnReasonId,
        input: UpdateReturnReasonInput,
    ) -> Result<ReturnReason, AppError> {
        let existing = self.load_return_reason(id).await?;
        let value = match input.value {
            Some(value) => {
                let value = normalize_value(&value)?;
                if value != existing.value {
                    let duplicate: Option<String> = sqlx::query_scalar(
                        "SELECT id FROM return_reasons WHERE value = ? AND id <> ?",
                    )
                    .bind(&value)
                    .bind(id.as_str())
                    .fetch_optional(self.database().pool())
                    .await
                    .map_err(database_error)?;
                    if duplicate.is_some() {
                        return Err(AppError::conflict(format!(
                            "return reason value already exists: {value}"
                        )));
                    }
                }
                value
            }
            None => existing.value,
        };
        let label = match input.label {
            Some(label) => normalize_label(&label)?,
            None => existing.label,
        };
        let description = match input.description {
            Some(description) => normalize_text(description),
            None => existing.description,
        };
        let metadata = match input.metadata {
            Some(metadata) => {
                validate_metadata(&metadata)?;
                metadata
            }
            None => existing.metadata,
        };
        sqlx::query(
            "UPDATE return_reasons
             SET value = ?, label = ?, description = ?, metadata = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(value)
        .bind(label)
        .bind(description)
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.database().pool())
        .await
        .map_err(|error| unique_or_database(error, "return reason value already exists"))?;
        self.load_return_reason(id).await
    }

    /// Deletes a return reason.
    ///
    /// Returns, claims and exchanges snapshot the reason label they were
    /// created with and carry no foreign key to `return_reasons`, so deleting a
    /// reason leaves every historical record readable and unchanged.
    pub async fn delete_return_reason(&self, id: &ReturnReasonId) -> Result<(), AppError> {
        let affected = sqlx::query("DELETE FROM return_reasons WHERE id = ?")
            .bind(id.as_str())
            .execute(self.database().pool())
            .await
            .map_err(database_error)?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("return reason", id));
        }
        Ok(())
    }

    async fn load_return_reason(&self, id: &ReturnReasonId) -> Result<ReturnReason, AppError> {
        let row = sqlx::query_as::<_, ReturnReasonRow>(&format!(
            "SELECT {REASON_COLUMNS} FROM return_reasons WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.database().pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("return reason", id))?;
        Ok(row.into_domain())
    }
}
