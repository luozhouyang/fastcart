//! Customer CRUD and guest lookup.

//! SQLite-backed customer application service.
//!
//! Every operation is implicitly scoped to the store owning the injected
//! [`StoreDatabase`]. Credentials are hashed with Argon2id and session tokens
//! are looked up exclusively by their SHA-256 digest.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CustomerId, Metadata, PageResult, Pagination, normalize_email, now_timestamp,
    validate_email, validate_metadata,
};

use crate::domain::{
    CreateCustomerInput, Customer, CustomerStatus, MAX_NAME_LENGTH, MAX_PHONE_LENGTH,
    UpdateCustomerInput,
};

use super::rows::*;
use super::*;
impl CustomerService {
    /// Creates a customer profile. Guests are created with `has_account = false`.
    ///
    /// The email is normalized to lowercase and must be unique within the store.
    pub async fn create_customer(&self, input: CreateCustomerInput) -> Result<Customer, AppError> {
        let email = normalize_email(&input.email);
        validate_email(&email, "email")?;
        let first_name = clean_optional(input.first_name, "first_name", MAX_NAME_LENGTH)?;
        let last_name = clean_optional(input.last_name, "last_name", MAX_NAME_LENGTH)?;
        let phone = clean_optional(input.phone, "phone", MAX_PHONE_LENGTH)?;
        validate_metadata(&input.metadata)?;
        self.ensure_email_available(&email, None).await?;

        let id = CustomerId::new();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO customers
             (id, email, first_name, last_name, phone, has_account, status, metadata,
              created_at, updated_at, deleted_at)
             VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?, ?, NULL)",
        )
        .bind(id.as_str())
        .bind(&email)
        .bind(&first_name)
        .bind(&last_name)
        .bind(&phone)
        .bind(CustomerStatus::Active.as_str())
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(|error| {
            conflict_or_database(error, "a customer with this email already exists")
        })?;

        self.get_customer(&id).await
    }

    /// Returns a non-deleted customer by identifier.
    pub async fn get_customer(&self, id: &CustomerId) -> Result<Customer, AppError> {
        let sql =
            format!("SELECT {CUSTOMER_COLUMNS} FROM customers WHERE id = ? AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, CustomerRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("customer", id))?;
        Ok(row.into_customer())
    }

    /// Returns a non-deleted customer by email, or `None` when absent.
    pub async fn get_customer_by_email(&self, email: &str) -> Result<Option<Customer>, AppError> {
        let email = normalize_email(email);
        let sql = format!(
            "SELECT {CUSTOMER_COLUMNS} FROM customers WHERE email = ? AND deleted_at IS NULL"
        );
        let row = sqlx::query_as::<_, CustomerRow>(&sql)
            .bind(&email)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(row.map(CustomerRow::into_customer))
    }

    /// Lists non-deleted customers with optional email/name search.
    pub async fn list_customers(
        &self,
        pagination: Pagination,
        query: Option<&str>,
    ) -> Result<PageResult<Customer>, AppError> {
        let pattern = query
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!("%{value}%"));

        let filter = if pattern.is_some() {
            "deleted_at IS NULL AND (email LIKE ? OR first_name LIKE ? OR last_name LIKE ? \
             OR (COALESCE(first_name, '') || ' ' || COALESCE(last_name, '')) LIKE ?)"
        } else {
            "deleted_at IS NULL"
        };

        let count_sql = format!("SELECT COUNT(*) FROM customers WHERE {filter}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(pattern) = &pattern {
            count_query = count_query
                .bind(pattern)
                .bind(pattern)
                .bind(pattern)
                .bind(pattern);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let page_sql = format!(
            "SELECT {CUSTOMER_COLUMNS} FROM customers WHERE {filter} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut page_query = sqlx::query_as::<_, CustomerRow>(&page_sql);
        if let Some(pattern) = &pattern {
            page_query = page_query
                .bind(pattern)
                .bind(pattern)
                .bind(pattern)
                .bind(pattern);
        }
        let rows = page_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        Ok(PageResult::new(
            rows.into_iter().map(CustomerRow::into_customer).collect(),
            count,
            pagination,
        ))
    }

    /// Applies a partial update to a non-deleted customer.
    ///
    /// The read, merge and write share a single write lock so concurrent
    /// updates cannot fabricate a result from a stale read. Renaming the email
    /// also updates the credential row used by login and clears verification.
    pub async fn update_customer(
        &self,
        id: &CustomerId,
        input: UpdateCustomerInput,
    ) -> Result<Customer, AppError> {
        let mut tx = self.db.begin().await?;
        let sql =
            format!("SELECT {CUSTOMER_COLUMNS} FROM customers WHERE id = ? AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, CustomerRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("customer", id))?;
        let mut customer = row.into_customer();

        let mut email_changed = false;
        if let Some(email) = input.email {
            let email = normalize_email(&email);
            validate_email(&email, "email")?;
            let existing: Option<String> =
                sqlx::query_scalar("SELECT id FROM customers WHERE email = ?")
                    .bind(&email)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(database_error)?;
            if let Some(existing) = existing
                && existing != customer.id.as_str()
            {
                return Err(AppError::conflict(
                    "a customer with this email already exists",
                ));
            }
            if email != customer.email {
                email_changed = true;
                customer.email_verified_at = None;
            }
            customer.email = email;
        }
        if let Some(first_name) = input.first_name {
            customer.first_name = clean_optional(Some(first_name), "first_name", MAX_NAME_LENGTH)?;
        }
        if let Some(last_name) = input.last_name {
            customer.last_name = clean_optional(Some(last_name), "last_name", MAX_NAME_LENGTH)?;
        }
        if let Some(phone) = input.phone {
            customer.phone = clean_optional(Some(phone), "phone", MAX_PHONE_LENGTH)?;
        }
        if let Some(status) = input.status {
            customer.status = status;
        }
        if let Some(metadata) = input.metadata {
            validate_metadata(&metadata)?;
            customer.metadata = metadata;
        }
        customer.updated_at = now_timestamp();

        let updated = sqlx::query(
            "UPDATE customers
             SET email = ?, first_name = ?, last_name = ?, phone = ?, status = ?,
                 email_verified_at = ?, metadata = ?, updated_at = ?
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(&customer.email)
        .bind(&customer.first_name)
        .bind(&customer.last_name)
        .bind(&customer.phone)
        .bind(customer.status.as_str())
        .bind(customer.email_verified_at)
        .bind(customer.metadata.to_json_string())
        .bind(customer.updated_at)
        .bind(customer.id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            conflict_or_database(error, "a customer with this email already exists")
        })?;
        if updated.rows_affected() != 1 {
            return Err(AppError::not_found("customer", id));
        }

        if email_changed {
            sqlx::query(
                "UPDATE customer_credentials SET email = ?, updated_at = ?
                 WHERE customer_id = ?",
            )
            .bind(&customer.email)
            .bind(customer.updated_at)
            .bind(customer.id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|error| {
                conflict_or_database(error, "a customer with this email already exists")
            })?;
        }

        tx.commit().await.map_err(database_error)?;
        Ok(customer)
    }

    /// Soft-deletes a customer and revokes every session it owns.
    ///
    /// Deleted customers can no longer be fetched or authenticated.
    pub async fn delete_customer(&self, id: &CustomerId) -> Result<(), AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let result = sqlx::query(
            "UPDATE customers SET deleted_at = ?, updated_at = ?
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(AppError::not_found("customer", id));
        }
        sqlx::query(
            "UPDATE customer_sessions SET revoked_at = ?
             WHERE customer_id = ? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Finds a customer by email or creates a guest profile for checkout.
    ///
    /// Existing profiles are returned unchanged when they are accounts. Guest
    /// profiles only receive names that were previously missing; existing data
    /// is never overwritten.
    ///
    /// Lookup, insertion and name backfill run inside one write transaction so
    /// concurrent checkouts for the same email resolve to a single profile.
    pub async fn find_or_create_guest(
        &self,
        email: &str,
        first_name: Option<&str>,
        last_name: Option<&str>,
    ) -> Result<Customer, AppError> {
        let email = normalize_email(email);
        validate_email(&email, "email")?;
        let first_name =
            clean_optional(first_name.map(str::to_owned), "first_name", MAX_NAME_LENGTH)?;
        let last_name = clean_optional(last_name.map(str::to_owned), "last_name", MAX_NAME_LENGTH)?;

        let mut tx = self.db.begin().await?;
        let sql = format!(
            "SELECT {CUSTOMER_COLUMNS} FROM customers WHERE email = ? AND deleted_at IS NULL"
        );
        let existing = sqlx::query_as::<_, CustomerRow>(&sql)
            .bind(&email)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;

        if let Some(row) = existing {
            let mut customer = row.into_customer();
            if customer.has_account {
                tx.commit().await.map_err(database_error)?;
                return Ok(customer);
            }
            let mut changed = false;
            if customer.first_name.is_none()
                && let Some(name) = first_name
            {
                customer.first_name = Some(name);
                changed = true;
            }
            if customer.last_name.is_none()
                && let Some(name) = last_name
            {
                customer.last_name = Some(name);
                changed = true;
            }
            if changed {
                customer.updated_at = now_timestamp();
                let updated = sqlx::query(
                    "UPDATE customers SET first_name = ?, last_name = ?, updated_at = ?
                     WHERE id = ? AND deleted_at IS NULL",
                )
                .bind(&customer.first_name)
                .bind(&customer.last_name)
                .bind(customer.updated_at)
                .bind(customer.id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                if updated.rows_affected() != 1 {
                    return Err(AppError::not_found("customer", &customer.id));
                }
            }
            tx.commit().await.map_err(database_error)?;
            return Ok(customer);
        }

        let id = CustomerId::new();
        let now = now_timestamp();
        let inserted = sqlx::query(
            "INSERT INTO customers
             (id, email, first_name, last_name, phone, has_account, status, metadata,
              created_at, updated_at, deleted_at)
             VALUES (?, ?, ?, ?, NULL, 0, ?, '{}', ?, ?, NULL)
             ON CONFLICT(email) DO NOTHING",
        )
        .bind(id.as_str())
        .bind(&email)
        .bind(&first_name)
        .bind(&last_name)
        .bind(CustomerStatus::Active.as_str())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        let row = if inserted.rows_affected() == 1 {
            Some(CustomerRow {
                id: id.as_str().to_owned(),
                email,
                first_name,
                last_name,
                phone: None,
                has_account: 0,
                status: CustomerStatus::Active.as_str().to_owned(),
                email_verified_at: None,
                metadata: Metadata::empty().to_json_string(),
                created_at: now,
                updated_at: now,
                deleted_at: None,
            })
        } else {
            // Another transaction won the race; return its profile unchanged.
            sqlx::query_as::<_, CustomerRow>(&sql)
                .bind(&email)
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?
        };

        tx.commit().await.map_err(database_error)?;
        row.map(CustomerRow::into_customer)
            .ok_or_else(|| AppError::conflict("a customer with this email already exists"))
    }

    pub(super) async fn ensure_customer_exists(
        &self,
        customer_id: &CustomerId,
    ) -> Result<(), AppError> {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM customers WHERE id = ? AND deleted_at IS NULL")
                .bind(customer_id.as_str())
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if exists.is_none() {
            return Err(AppError::not_found("customer", customer_id));
        }
        Ok(())
    }

    pub(super) async fn ensure_email_available(
        &self,
        email: &str,
        except: Option<&CustomerId>,
    ) -> Result<(), AppError> {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM customers WHERE email = ?")
                .bind(email)
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if let Some(existing) = existing {
            let is_other = except.is_none_or(|id| id.as_str() != existing);
            if is_other {
                return Err(AppError::conflict(
                    "a customer with this email already exists",
                ));
            }
        }
        Ok(())
    }
}
