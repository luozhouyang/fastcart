//! Customer groups and membership.

//! SQLite-backed customer application service.
//!
//! Every operation is implicitly scoped to the store owning the injected
//! [`StoreDatabase`]. Credentials are hashed with Argon2id and session tokens
//! are looked up exclusively by their SHA-256 digest.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, CustomerId, Metadata, now_timestamp, validate_metadata};
use sqlx::FromRow;

use crate::domain::{CustomerGroup, MAX_NAME_LENGTH};

use super::*;

#[derive(Debug, FromRow)]
struct GroupRow {
    id: String,
    name: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl GroupRow {
    fn into_group(self) -> CustomerGroup {
        CustomerGroup {
            id: self.id,
            name: self.name,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

fn validate_non_empty(value: &str, field: &str, maximum: usize) -> Result<(), AppError> {
    if value.is_empty() {
        return Err(AppError::validation(format!("{field} must not be empty")));
    }
    if value.chars().count() > maximum {
        return Err(AppError::validation(format!(
            "{field} must not exceed {maximum} characters"
        )));
    }
    Ok(())
}

impl CustomerService {
    /// Creates a customer group. Names are unique within the store.
    pub async fn create_group(
        &self,
        name: &str,
        metadata: Metadata,
    ) -> Result<CustomerGroup, AppError> {
        let name = name.trim();
        validate_non_empty(name, "name", MAX_NAME_LENGTH)?;
        validate_metadata(&metadata)?;

        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM customer_groups WHERE name = ?")
                .bind(name)
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if existing.is_some() {
            return Err(AppError::conflict(
                "a customer group with this name already exists",
            ));
        }

        let now = now_timestamp();
        let group = CustomerGroup {
            id: format!("cgroup_{}", random_hex(16)),
            name: name.to_owned(),
            metadata,
            created_at: now,
            updated_at: now,
        };
        sqlx::query(
            "INSERT INTO customer_groups (id, name, metadata, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&group.id)
        .bind(&group.name)
        .bind(group.metadata.to_json_string())
        .bind(group.created_at)
        .bind(group.updated_at)
        .execute(self.db.pool())
        .await
        .map_err(|error| {
            conflict_or_database(error, "a customer group with this name already exists")
        })?;
        Ok(group)
    }

    /// Returns one customer group.
    pub async fn get_group(&self, id: &str) -> Result<CustomerGroup, AppError> {
        let sql = format!("SELECT {GROUP_COLUMNS} FROM customer_groups WHERE id = ?");
        let row = sqlx::query_as::<_, GroupRow>(&sql)
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("customer group", id))?;
        Ok(row.into_group())
    }

    /// Lists customer groups ordered by name.
    pub async fn list_groups(&self) -> Result<Vec<CustomerGroup>, AppError> {
        let sql = format!("SELECT {GROUP_COLUMNS} FROM customer_groups ORDER BY name ASC");
        let rows = sqlx::query_as::<_, GroupRow>(&sql)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(rows.into_iter().map(GroupRow::into_group).collect())
    }

    /// Adds a customer to a group. Assigning twice is a no-op.
    pub async fn assign_group(
        &self,
        customer_id: &CustomerId,
        group_id: &str,
    ) -> Result<(), AppError> {
        self.ensure_customer_exists(customer_id).await?;
        self.get_group(group_id).await?;
        sqlx::query(
            "INSERT OR IGNORE INTO customer_group_links (customer_id, group_id) VALUES (?, ?)",
        )
        .bind(customer_id.as_str())
        .bind(group_id)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }

    /// Removes a customer from a group.
    pub async fn remove_group(
        &self,
        customer_id: &CustomerId,
        group_id: &str,
    ) -> Result<(), AppError> {
        sqlx::query("DELETE FROM customer_group_links WHERE customer_id = ? AND group_id = ?")
            .bind(customer_id.as_str())
            .bind(group_id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(())
    }

    /// Lists the groups a customer belongs to.
    pub async fn list_customer_groups(
        &self,
        customer_id: &CustomerId,
    ) -> Result<Vec<CustomerGroup>, AppError> {
        let sql = "SELECT g.id, g.name, g.metadata, g.created_at, g.updated_at
             FROM customer_groups g
             JOIN customer_group_links l ON l.group_id = g.id
             WHERE l.customer_id = ?
             ORDER BY g.name ASC";
        let rows = sqlx::query_as::<_, GroupRow>(sql)
            .bind(customer_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(rows.into_iter().map(GroupRow::into_group).collect())
    }
}
