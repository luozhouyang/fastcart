//! SQLite-backed customer application service.
//!
//! Every operation is implicitly scoped to the store owning the injected
//! [`StoreDatabase`]. Credentials are hashed with Argon2id and session tokens
//! are looked up exclusively by their SHA-256 digest.

use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};
use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CountryCode, CustomerId, CustomerSessionId, Metadata, PageResult, Pagination,
    normalize_email, now_timestamp, validate_email, validate_metadata,
};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, Sqlite, Transaction};

use crate::domain::{
    Address, AddressInput, AuthenticatedCustomerSession, CUSTOMER_SESSION_TTL_SECONDS,
    CreateCustomerInput, CreatedCustomerSession, Customer, CustomerGroup, CustomerSession,
    CustomerStatus, LoginInput, MAX_NAME_LENGTH, MAX_PHONE_LENGTH, MIN_PASSWORD_LENGTH,
    RegisterCustomerInput, UpdateCustomerInput,
};

const CUSTOMER_COLUMNS: &str = "id, email, first_name, last_name, phone, has_account, status, \
     metadata, created_at, updated_at, deleted_at";

const ADDRESS_COLUMNS: &str = "id, customer_id, is_default_shipping, is_default_billing, \
     first_name, last_name, company, address_1, address_2, city, province, postal_code, \
     country_code, phone, metadata, created_at, updated_at";

const GROUP_COLUMNS: &str = "id, name, metadata, created_at, updated_at";

/// Application service for customers, addresses, groups and authentication.
#[derive(Clone, Debug)]
pub struct CustomerService {
    db: StoreDatabase,
}

impl CustomerService {
    /// Creates a service bound to a migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    /// Returns the underlying store database handle.
    #[must_use]
    pub fn database(&self) -> &StoreDatabase {
        &self.db
    }

    // ---------------------------------------------------------------------
    // Customers
    // ---------------------------------------------------------------------

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
    pub async fn update_customer(
        &self,
        id: &CustomerId,
        input: UpdateCustomerInput,
    ) -> Result<Customer, AppError> {
        let mut customer = self.get_customer(id).await?;

        if let Some(email) = input.email {
            let email = normalize_email(&email);
            validate_email(&email, "email")?;
            self.ensure_email_available(&email, Some(id)).await?;
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

        sqlx::query(
            "UPDATE customers
             SET email = ?, first_name = ?, last_name = ?, phone = ?, status = ?,
                 metadata = ?, updated_at = ?
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(&customer.email)
        .bind(&customer.first_name)
        .bind(&customer.last_name)
        .bind(&customer.phone)
        .bind(customer.status.as_str())
        .bind(customer.metadata.to_json_string())
        .bind(customer.updated_at)
        .bind(customer.id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(|error| {
            conflict_or_database(error, "a customer with this email already exists")
        })?;

        Ok(customer)
    }

    /// Soft-deletes a customer and revokes every session it owns.
    ///
    /// Deleted customers can no longer be fetched or authenticated.
    pub async fn delete_customer(&self, id: &CustomerId) -> Result<(), AppError> {
        let now = now_timestamp();
        let result = sqlx::query(
            "UPDATE customers SET deleted_at = ?, updated_at = ?
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("customer", id));
        }
        self.revoke_all_sessions(id).await?;
        Ok(())
    }

    /// Finds a customer by email or creates a guest profile for checkout.
    ///
    /// Existing profiles are returned unchanged when they are accounts. Guest
    /// profiles only receive names that were previously missing; existing data
    /// is never overwritten.
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

        if let Some(mut customer) = self.get_customer_by_email(&email).await? {
            if customer.has_account {
                return Ok(customer);
            }
            let mut changed = false;
            if customer.first_name.is_none() {
                if let Some(name) = first_name {
                    customer.first_name = Some(name);
                    changed = true;
                }
            }
            if customer.last_name.is_none() {
                if let Some(name) = last_name {
                    customer.last_name = Some(name);
                    changed = true;
                }
            }
            if !changed {
                return Ok(customer);
            }
            customer.updated_at = now_timestamp();
            sqlx::query(
                "UPDATE customers SET first_name = ?, last_name = ?, updated_at = ?
                 WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(&customer.first_name)
            .bind(&customer.last_name)
            .bind(customer.updated_at)
            .bind(customer.id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
            return Ok(customer);
        }

        self.create_customer(CreateCustomerInput {
            email,
            first_name,
            last_name,
            phone: None,
            metadata: Metadata::empty(),
        })
        .await
    }

    // ---------------------------------------------------------------------
    // Addresses
    // ---------------------------------------------------------------------

    /// Adds an address, clearing the previous default in the same transaction
    /// when the new address is marked as a default.
    pub async fn add_address(
        &self,
        customer_id: &CustomerId,
        input: AddressInput,
    ) -> Result<Address, AppError> {
        self.ensure_customer_exists(customer_id).await?;
        let normalized = normalize_address(input)?;
        let now = now_timestamp();
        let address = Address {
            id: format!("addr_{}", random_hex(16)),
            customer_id: customer_id.clone(),
            is_default_shipping: normalized.is_default_shipping,
            is_default_billing: normalized.is_default_billing,
            first_name: normalized.first_name,
            last_name: normalized.last_name,
            company: normalized.company,
            address_1: Some(normalized.address_1),
            address_2: normalized.address_2,
            city: Some(normalized.city),
            province: normalized.province,
            postal_code: Some(normalized.postal_code),
            country_code: Some(normalized.country_code),
            phone: normalized.phone,
            metadata: normalized.metadata,
            created_at: now,
            updated_at: now,
        };

        let mut tx = self.db.begin().await?;
        if address.is_default_shipping {
            clear_default_shipping(&mut tx, customer_id, None).await?;
        }
        if address.is_default_billing {
            clear_default_billing(&mut tx, customer_id, None).await?;
        }
        sqlx::query(
            "INSERT INTO customer_addresses
             (id, customer_id, is_default_shipping, is_default_billing, first_name, last_name,
              company, address_1, address_2, city, province, postal_code, country_code, phone,
              metadata, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&address.id)
        .bind(address.customer_id.as_str())
        .bind(i64::from(address.is_default_shipping))
        .bind(i64::from(address.is_default_billing))
        .bind(&address.first_name)
        .bind(&address.last_name)
        .bind(&address.company)
        .bind(&address.address_1)
        .bind(&address.address_2)
        .bind(&address.city)
        .bind(&address.province)
        .bind(&address.postal_code)
        .bind(address.country_code.as_ref().map(CountryCode::as_str))
        .bind(&address.phone)
        .bind(address.metadata.to_json_string())
        .bind(address.created_at)
        .bind(address.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;

        Ok(address)
    }

    /// Replaces an existing address, moving defaults atomically.
    pub async fn update_address(
        &self,
        customer_id: &CustomerId,
        address_id: &str,
        input: AddressInput,
    ) -> Result<Address, AppError> {
        let existing = self.get_address(customer_id, address_id).await?;
        let normalized = normalize_address(input)?;
        let now = now_timestamp();
        let address = Address {
            id: existing.id,
            customer_id: existing.customer_id,
            is_default_shipping: normalized.is_default_shipping,
            is_default_billing: normalized.is_default_billing,
            first_name: normalized.first_name,
            last_name: normalized.last_name,
            company: normalized.company,
            address_1: Some(normalized.address_1),
            address_2: normalized.address_2,
            city: Some(normalized.city),
            province: normalized.province,
            postal_code: Some(normalized.postal_code),
            country_code: Some(normalized.country_code),
            phone: normalized.phone,
            metadata: normalized.metadata,
            created_at: existing.created_at,
            updated_at: now,
        };

        let mut tx = self.db.begin().await?;
        if address.is_default_shipping {
            clear_default_shipping(&mut tx, customer_id, Some(address_id)).await?;
        }
        if address.is_default_billing {
            clear_default_billing(&mut tx, customer_id, Some(address_id)).await?;
        }
        sqlx::query(
            "UPDATE customer_addresses
             SET is_default_shipping = ?, is_default_billing = ?, first_name = ?, last_name = ?,
                 company = ?, address_1 = ?, address_2 = ?, city = ?, province = ?,
                 postal_code = ?, country_code = ?, phone = ?, metadata = ?, updated_at = ?
             WHERE id = ? AND customer_id = ?",
        )
        .bind(i64::from(address.is_default_shipping))
        .bind(i64::from(address.is_default_billing))
        .bind(&address.first_name)
        .bind(&address.last_name)
        .bind(&address.company)
        .bind(&address.address_1)
        .bind(&address.address_2)
        .bind(&address.city)
        .bind(&address.province)
        .bind(&address.postal_code)
        .bind(address.country_code.as_ref().map(CountryCode::as_str))
        .bind(&address.phone)
        .bind(address.metadata.to_json_string())
        .bind(address.updated_at)
        .bind(&address.id)
        .bind(address.customer_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;

        Ok(address)
    }

    /// Returns one address owned by the customer.
    pub async fn get_address(
        &self,
        customer_id: &CustomerId,
        address_id: &str,
    ) -> Result<Address, AppError> {
        let sql = format!(
            "SELECT {ADDRESS_COLUMNS} FROM customer_addresses
             WHERE id = ? AND customer_id = ?"
        );
        let row = sqlx::query_as::<_, AddressRow>(&sql)
            .bind(address_id)
            .bind(customer_id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("address", address_id))?;
        Ok(row.into_address())
    }

    /// Lists a customer's addresses with defaults first.
    pub async fn list_addresses(&self, customer_id: &CustomerId) -> Result<Vec<Address>, AppError> {
        self.ensure_customer_exists(customer_id).await?;
        let sql = format!(
            "SELECT {ADDRESS_COLUMNS} FROM customer_addresses
             WHERE customer_id = ?
             ORDER BY is_default_shipping DESC, is_default_billing DESC, created_at ASC"
        );
        let rows = sqlx::query_as::<_, AddressRow>(&sql)
            .bind(customer_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(rows.into_iter().map(AddressRow::into_address).collect())
    }

    /// Removes a customer address.
    pub async fn remove_address(
        &self,
        customer_id: &CustomerId,
        address_id: &str,
    ) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM customer_addresses WHERE id = ? AND customer_id = ?")
            .bind(address_id)
            .bind(customer_id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("address", address_id));
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Groups
    // ---------------------------------------------------------------------

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

    // ---------------------------------------------------------------------
    // Authentication
    // ---------------------------------------------------------------------

    /// Registers a customer account and stores an Argon2id password hash.
    ///
    /// A guest profile with the same email is upgraded to an account instead of
    /// being duplicated. An existing credential always yields a conflict.
    pub async fn register(&self, input: RegisterCustomerInput) -> Result<Customer, AppError> {
        let email = normalize_email(&input.email);
        validate_email(&email, "email")?;
        validate_new_password(&input.password)?;
        let first_name = clean_optional(input.first_name, "first_name", MAX_NAME_LENGTH)?;
        let last_name = clean_optional(input.last_name, "last_name", MAX_NAME_LENGTH)?;
        let phone = clean_optional(input.phone, "phone", MAX_PHONE_LENGTH)?;

        let credential_exists: Option<String> =
            sqlx::query_scalar("SELECT id FROM customer_credentials WHERE email = ?")
                .bind(&email)
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if credential_exists.is_some() {
            return Err(AppError::conflict(
                "an account with this email already exists",
            ));
        }

        let existing = sqlx::query_as::<_, ExistingCustomerRow>(
            "SELECT id, has_account, deleted_at FROM customers WHERE email = ?",
        )
        .bind(&email)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        if let Some(existing) = &existing {
            if existing.deleted_at.is_some() || existing.has_account != 0 {
                return Err(AppError::conflict(
                    "an account with this email already exists",
                ));
            }
        }

        let password_hash = hash_password(&input.password)?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;

        let customer_id = if let Some(existing) = existing {
            let customer_id = CustomerId::from_string(existing.id);
            sqlx::query(
                "UPDATE customers
                 SET has_account = 1, first_name = COALESCE(?, first_name),
                     last_name = COALESCE(?, last_name), phone = COALESCE(?, phone),
                     updated_at = ?
                 WHERE id = ?",
            )
            .bind(first_name.as_deref())
            .bind(last_name.as_deref())
            .bind(phone.as_deref())
            .bind(now)
            .bind(customer_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            customer_id
        } else {
            let customer_id = CustomerId::new();
            sqlx::query(
                "INSERT INTO customers
                 (id, email, first_name, last_name, phone, has_account, status, metadata,
                  created_at, updated_at, deleted_at)
                 VALUES (?, ?, ?, ?, ?, 1, ?, '{}', ?, ?, NULL)",
            )
            .bind(customer_id.as_str())
            .bind(&email)
            .bind(&first_name)
            .bind(&last_name)
            .bind(&phone)
            .bind(CustomerStatus::Active.as_str())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|error| {
                conflict_or_database(error, "a customer with this email already exists")
            })?;
            customer_id
        };

        sqlx::query(
            "INSERT INTO customer_credentials
             (id, customer_id, email, password_hash, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("cred_{}", random_hex(16)))
        .bind(customer_id.as_str())
        .bind(&email)
        .bind(&password_hash)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            conflict_or_database(error, "an account with this email already exists")
        })?;
        tx.commit().await.map_err(database_error)?;

        self.get_customer(&customer_id).await
    }

    /// Authenticates a customer and creates a store-scoped session.
    ///
    /// The returned token is shown exactly once; only its SHA-256 digest is
    /// persisted.
    pub async fn login(
        &self,
        email: &str,
        password: &str,
    ) -> Result<CreatedCustomerSession, AppError> {
        let email = normalize_email(email);
        let row = sqlx::query_as::<_, CredentialCustomerRow>(
            "SELECT c.id, c.email, c.first_name, c.last_name, c.phone, c.has_account,
                    c.status, c.metadata, c.created_at, c.updated_at, cc.password_hash
             FROM customers c
             JOIN customer_credentials cc ON cc.customer_id = c.id
             WHERE cc.email = ? AND c.deleted_at IS NULL",
        )
        .bind(&email)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            return Err(AppError::Unauthorized);
        };
        if !verify_password(&row.password_hash, password) {
            return Err(AppError::Unauthorized);
        }
        let customer = row.into_customer();
        if customer.status != CustomerStatus::Active {
            return Err(AppError::Forbidden);
        }
        self.create_session(customer).await
    }

    /// Convenience wrapper around [`CustomerService::login`].
    pub async fn login_with_input(
        &self,
        input: LoginInput,
    ) -> Result<CreatedCustomerSession, AppError> {
        self.login(&input.email, &input.password).await
    }

    /// Validates a session token, refreshes its last-used timestamp and returns
    /// the authenticated customer.
    ///
    /// Revoked, expired and soft-deleted sessions are rejected.
    pub async fn authenticate_session(
        &self,
        token: &str,
    ) -> Result<AuthenticatedCustomerSession, AppError> {
        if token.len() < 32 {
            return Err(AppError::Unauthorized);
        }
        let row = sqlx::query_as::<_, SessionCustomerRow>(
            "SELECT s.id AS session_id, s.expires_at,
                    c.id, c.email, c.first_name, c.last_name, c.phone, c.has_account,
                    c.status, c.metadata, c.created_at, c.updated_at
             FROM customer_sessions s
             JOIN customers c ON c.id = s.customer_id
             WHERE s.token_hash = ? AND s.revoked_at IS NULL AND c.deleted_at IS NULL",
        )
        .bind(hash_token(token))
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            return Err(AppError::Unauthorized);
        };
        if row.expires_at <= now_timestamp() {
            return Err(AppError::Unauthorized);
        }

        let session_id = CustomerSessionId::from_string(row.session_id.clone());
        let customer = row.into_customer();
        if customer.status != CustomerStatus::Active {
            return Err(AppError::Forbidden);
        }

        sqlx::query("UPDATE customer_sessions SET last_used_at = ? WHERE id = ?")
            .bind(now_timestamp())
            .bind(session_id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;

        Ok(AuthenticatedCustomerSession {
            customer,
            session_id,
        })
    }

    /// Revokes the session identified by a token. Unknown tokens are ignored.
    pub async fn logout(&self, token: &str) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE customer_sessions SET revoked_at = ?
             WHERE token_hash = ? AND revoked_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(hash_token(token))
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }

    /// Revokes every active session owned by a customer.
    pub async fn revoke_all_sessions(&self, customer_id: &CustomerId) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE customer_sessions SET revoked_at = ?
             WHERE customer_id = ? AND revoked_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(customer_id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }

    /// Changes a customer's password and revokes all active sessions.
    pub async fn change_password(
        &self,
        customer_id: &CustomerId,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), AppError> {
        validate_new_password(new_password)?;

        let stored: Option<String> = sqlx::query_scalar(
            "SELECT password_hash FROM customer_credentials WHERE customer_id = ?",
        )
        .bind(customer_id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        let stored =
            stored.ok_or_else(|| AppError::not_found("customer credentials", customer_id))?;
        if !verify_password(&stored, current_password) {
            return Err(AppError::Unauthorized);
        }

        let password_hash = hash_password(new_password)?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE customer_credentials SET password_hash = ?, updated_at = ?
             WHERE customer_id = ?",
        )
        .bind(&password_hash)
        .bind(now)
        .bind(customer_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        sqlx::query(
            "UPDATE customer_sessions SET revoked_at = ?
             WHERE customer_id = ? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(customer_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Returns the persisted sessions of a customer, excluding token digests.
    pub async fn list_sessions(
        &self,
        customer_id: &CustomerId,
    ) -> Result<Vec<CustomerSession>, AppError> {
        let rows = sqlx::query_as::<_, CustomerSessionRow>(
            "SELECT id, customer_id, expires_at, revoked_at, last_used_at, created_at
             FROM customer_sessions WHERE customer_id = ? ORDER BY created_at DESC",
        )
        .bind(customer_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows
            .into_iter()
            .map(CustomerSessionRow::into_session)
            .collect())
    }

    async fn create_session(&self, customer: Customer) -> Result<CreatedCustomerSession, AppError> {
        let token = random_hex(32);
        let csrf_token = random_hex(24);
        let session_id = CustomerSessionId::new();
        let now = now_timestamp();
        let expires_at = now.saturating_add(CUSTOMER_SESSION_TTL_SECONDS);

        sqlx::query(
            "INSERT INTO customer_sessions
             (id, customer_id, token_hash, expires_at, revoked_at, last_used_at, created_at)
             VALUES (?, ?, ?, ?, NULL, ?, ?)",
        )
        .bind(session_id.as_str())
        .bind(customer.id.as_str())
        .bind(hash_token(&token))
        .bind(expires_at)
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;

        Ok(CreatedCustomerSession {
            token,
            csrf_token,
            session_id,
            customer,
            expires_at,
        })
    }

    async fn ensure_customer_exists(&self, customer_id: &CustomerId) -> Result<(), AppError> {
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

    async fn ensure_email_available(
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

// -------------------------------------------------------------------------
// Persistence rows
// -------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct CustomerRow {
    id: String,
    email: String,
    first_name: Option<String>,
    last_name: Option<String>,
    phone: Option<String>,
    has_account: i64,
    status: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
    deleted_at: Option<i64>,
}

impl CustomerRow {
    fn into_customer(self) -> Customer {
        Customer {
            id: CustomerId::from_string(self.id),
            email: self.email,
            first_name: self.first_name,
            last_name: self.last_name,
            phone: self.phone,
            has_account: self.has_account != 0,
            status: CustomerStatus::from_db(&self.status),
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct ExistingCustomerRow {
    id: String,
    has_account: i64,
    deleted_at: Option<i64>,
}

#[derive(Debug, FromRow)]
struct CredentialCustomerRow {
    id: String,
    email: String,
    first_name: Option<String>,
    last_name: Option<String>,
    phone: Option<String>,
    has_account: i64,
    status: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
    password_hash: String,
}

impl CredentialCustomerRow {
    fn into_customer(self) -> Customer {
        Customer {
            id: CustomerId::from_string(self.id),
            email: self.email,
            first_name: self.first_name,
            last_name: self.last_name,
            phone: self.phone,
            has_account: self.has_account != 0,
            status: CustomerStatus::from_db(&self.status),
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: None,
        }
    }
}

#[derive(Debug, FromRow)]
struct SessionCustomerRow {
    session_id: String,
    expires_at: i64,
    id: String,
    email: String,
    first_name: Option<String>,
    last_name: Option<String>,
    phone: Option<String>,
    has_account: i64,
    status: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl SessionCustomerRow {
    fn into_customer(self) -> Customer {
        Customer {
            id: CustomerId::from_string(self.id),
            email: self.email,
            first_name: self.first_name,
            last_name: self.last_name,
            phone: self.phone,
            has_account: self.has_account != 0,
            status: CustomerStatus::from_db(&self.status),
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: None,
        }
    }
}

#[derive(Debug, FromRow)]
struct AddressRow {
    id: String,
    customer_id: String,
    is_default_shipping: i64,
    is_default_billing: i64,
    first_name: Option<String>,
    last_name: Option<String>,
    company: Option<String>,
    address_1: Option<String>,
    address_2: Option<String>,
    city: Option<String>,
    province: Option<String>,
    postal_code: Option<String>,
    country_code: Option<String>,
    phone: Option<String>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl AddressRow {
    fn into_address(self) -> Address {
        Address {
            id: self.id,
            customer_id: CustomerId::from_string(self.customer_id),
            is_default_shipping: self.is_default_shipping != 0,
            is_default_billing: self.is_default_billing != 0,
            first_name: self.first_name,
            last_name: self.last_name,
            company: self.company,
            address_1: self.address_1,
            address_2: self.address_2,
            city: self.city,
            province: self.province,
            postal_code: self.postal_code,
            country_code: self
                .country_code
                .and_then(|code| CountryCode::new(code).ok()),
            phone: self.phone,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

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

#[derive(Debug, FromRow)]
struct CustomerSessionRow {
    id: String,
    customer_id: String,
    expires_at: i64,
    revoked_at: Option<i64>,
    last_used_at: i64,
    created_at: i64,
}

impl CustomerSessionRow {
    fn into_session(self) -> CustomerSession {
        CustomerSession {
            id: CustomerSessionId::from_string(self.id),
            customer_id: CustomerId::from_string(self.customer_id),
            expires_at: self.expires_at,
            revoked_at: self.revoked_at,
            last_used_at: self.last_used_at,
            created_at: self.created_at,
        }
    }
}

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

struct NormalizedAddress {
    is_default_shipping: bool,
    is_default_billing: bool,
    first_name: Option<String>,
    last_name: Option<String>,
    company: Option<String>,
    address_1: String,
    address_2: Option<String>,
    city: String,
    province: Option<String>,
    postal_code: String,
    country_code: CountryCode,
    phone: Option<String>,
    metadata: Metadata,
}

fn normalize_address(input: AddressInput) -> Result<NormalizedAddress, AppError> {
    let address_1 = required_optional(input.address_1, "address_1")?;
    let city = required_optional(input.city, "city")?;
    let postal_code = required_optional(input.postal_code, "postal_code")?;
    let country_code = required_optional(input.country_code, "country_code")?;
    let country_code = CountryCode::new(country_code)?;
    validate_metadata(&input.metadata)?;

    Ok(NormalizedAddress {
        is_default_shipping: input.is_default_shipping,
        is_default_billing: input.is_default_billing,
        first_name: clean_optional(input.first_name, "first_name", MAX_NAME_LENGTH)?,
        last_name: clean_optional(input.last_name, "last_name", MAX_NAME_LENGTH)?,
        company: clean_optional(input.company, "company", 255)?,
        address_1,
        address_2: clean_optional(input.address_2, "address_2", 255)?,
        city,
        province: clean_optional(input.province, "province", 255)?,
        postal_code,
        country_code,
        phone: clean_optional(input.phone, "phone", MAX_PHONE_LENGTH)?,
        metadata: input.metadata,
    })
}

async fn clear_default_shipping(
    tx: &mut Transaction<'_, Sqlite>,
    customer_id: &CustomerId,
    exclude: Option<&str>,
) -> Result<(), AppError> {
    let sql = if exclude.is_some() {
        "UPDATE customer_addresses SET is_default_shipping = 0
         WHERE customer_id = ? AND id <> ?"
    } else {
        "UPDATE customer_addresses SET is_default_shipping = 0 WHERE customer_id = ?"
    };
    let mut query = sqlx::query(sql).bind(customer_id.as_str());
    if let Some(exclude) = exclude {
        query = query.bind(exclude);
    }
    query.execute(&mut **tx).await.map_err(database_error)?;
    Ok(())
}

async fn clear_default_billing(
    tx: &mut Transaction<'_, Sqlite>,
    customer_id: &CustomerId,
    exclude: Option<&str>,
) -> Result<(), AppError> {
    let sql = if exclude.is_some() {
        "UPDATE customer_addresses SET is_default_billing = 0
         WHERE customer_id = ? AND id <> ?"
    } else {
        "UPDATE customer_addresses SET is_default_billing = 0 WHERE customer_id = ?"
    };
    let mut query = sqlx::query(sql).bind(customer_id.as_str());
    if let Some(exclude) = exclude {
        query = query.bind(exclude);
    }
    query.execute(&mut **tx).await.map_err(database_error)?;
    Ok(())
}

fn clean_optional(
    value: Option<String>,
    field: &str,
    maximum: usize,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > maximum {
        return Err(AppError::validation(format!(
            "{field} must not exceed {maximum} characters"
        )));
    }
    Ok(Some(trimmed.to_owned()))
}

fn required_optional(value: Option<String>, field: &str) -> Result<String, AppError> {
    let value = value
        .map(|value| value.trim().to_owned())
        .unwrap_or_default();
    if value.is_empty() {
        return Err(AppError::validation(format!("{field} is required")));
    }
    Ok(value)
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

fn validate_new_password(password: &str) -> Result<(), AppError> {
    if password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(AppError::validation(format!(
            "password must contain at least {MIN_PASSWORD_LENGTH} characters"
        )));
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| AppError::internal(error.to_string()))
}

fn verify_password(stored_hash: &str, password: &str) -> bool {
    PasswordHash::new(stored_hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buffer);
    hex::encode(buffer)
}

/// SHA-256 digest used for session token lookup.
fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn conflict_or_database(error: sqlx::Error, message: &'static str) -> AppError {
    if let sqlx::Error::Database(database) = &error {
        if database.is_unique_violation() {
            return AppError::conflict(message);
        }
    }
    database_error(error)
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::StoreId;

    use super::*;

    async fn service() -> (CustomerService, StoreDatabase) {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        (CustomerService::new(db.clone()), db)
    }

    fn address(address_1: &str) -> AddressInput {
        AddressInput::new(address_1, "Springfield", "12345", "us")
    }

    #[tokio::test]
    async fn creates_and_fetches_a_customer() {
        let (service, _) = service().await;
        let customer = service
            .create_customer(
                CreateCustomerInput::new("  Owner@Example.COM ").with_name("Ada", "Lovelace"),
            )
            .await
            .unwrap();

        assert_eq!(customer.email, "owner@example.com");
        assert_eq!(customer.first_name.as_deref(), Some("Ada"));
        assert_eq!(customer.last_name.as_deref(), Some("Lovelace"));
        assert!(!customer.has_account);
        assert_eq!(customer.status, CustomerStatus::Active);
        assert!(customer.deleted_at.is_none());

        let fetched = service.get_customer(&customer.id).await.unwrap();
        assert_eq!(fetched, customer);

        let by_email = service
            .get_customer_by_email("OWNER@example.com")
            .await
            .unwrap();
        assert_eq!(by_email, Some(customer));
    }

    #[tokio::test]
    async fn missing_customer_is_not_found() {
        let (service, _) = service().await;
        let error = service.get_customer(&CustomerId::new()).await.unwrap_err();
        assert!(matches!(error, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn updates_customer_fields_partially() {
        let (service, _) = service().await;
        let customer = service
            .create_customer(
                CreateCustomerInput::new("owner@example.com").with_name("Ada", "Lovelace"),
            )
            .await
            .unwrap();

        let updated = service
            .update_customer(
                &customer.id,
                UpdateCustomerInput {
                    first_name: Some("Grace".to_owned()),
                    status: Some(CustomerStatus::Disabled),
                    ..UpdateCustomerInput::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.first_name.as_deref(), Some("Grace"));
        assert_eq!(updated.last_name.as_deref(), Some("Lovelace"));
        assert_eq!(updated.status, CustomerStatus::Disabled);
        assert_eq!(updated.email, customer.email);
    }

    #[tokio::test]
    async fn lists_and_searches_customers() {
        let (service, _) = service().await;
        for (email, first) in [
            ("alice@example.com", "Alice"),
            ("bob@example.com", "Bob"),
            ("carol@example.com", "Carol"),
        ] {
            service
                .create_customer(CreateCustomerInput::new(email).with_name(first, "Example"))
                .await
                .unwrap();
        }

        let page = service
            .list_customers(Pagination::new(Some(2), Some(0)).unwrap(), None)
            .await
            .unwrap();
        assert_eq!(page.data.len(), 2);
        assert_eq!(page.count, 3);

        let searched = service
            .list_customers(Pagination::default(), Some("ALI"))
            .await
            .unwrap();
        assert_eq!(searched.count, 1);
        assert_eq!(searched.data[0].email, "alice@example.com");

        let by_name = service
            .list_customers(Pagination::default(), Some("bob"))
            .await
            .unwrap();
        assert_eq!(by_name.count, 1);
        assert_eq!(by_name.data[0].first_name.as_deref(), Some("Bob"));
    }

    #[tokio::test]
    async fn duplicate_email_conflicts() {
        let (service, _) = service().await;
        service
            .create_customer(CreateCustomerInput::new("owner@example.com"))
            .await
            .unwrap();
        let error = service
            .create_customer(CreateCustomerInput::new("OWNER@example.com"))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn invalid_email_is_rejected() {
        let (service, _) = service().await;
        let error = service
            .create_customer(CreateCustomerInput::new("not-an-email"))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn soft_deleted_customers_disappear() {
        let (service, _) = service().await;
        let customer = service
            .create_customer(CreateCustomerInput::new("gone@example.com"))
            .await
            .unwrap();
        let second = service
            .create_customer(CreateCustomerInput::new("kept@example.com"))
            .await
            .unwrap();

        service.delete_customer(&customer.id).await.unwrap();

        assert!(service.get_customer(&customer.id).await.is_err());
        assert!(
            service
                .get_customer_by_email("gone@example.com")
                .await
                .unwrap()
                .is_none()
        );
        let page = service
            .list_customers(Pagination::default(), None)
            .await
            .unwrap();
        assert_eq!(page.count, 1);
        assert_eq!(page.data[0].id, second.id);
        assert!(service.delete_customer(&customer.id).await.is_err());
    }

    #[tokio::test]
    async fn guest_upsert_fills_missing_names_only() {
        let (service, _) = service().await;
        let guest = service
            .create_customer(CreateCustomerInput::new("guest@example.com"))
            .await
            .unwrap();

        let filled = service
            .find_or_create_guest("GUEST@example.com", Some("Ada"), Some("Lovelace"))
            .await
            .unwrap();
        assert_eq!(filled.id, guest.id);
        assert_eq!(filled.first_name.as_deref(), Some("Ada"));
        assert!(!filled.has_account);

        let unchanged = service
            .find_or_create_guest("guest@example.com", Some("Grace"), Some("Hopper"))
            .await
            .unwrap();
        assert_eq!(unchanged.first_name.as_deref(), Some("Ada"));
        assert_eq!(unchanged.last_name.as_deref(), Some("Lovelace"));

        let account = service
            .register(
                RegisterCustomerInput::new("account@example.com", "hunter2hunter2")
                    .with_name("Account", "Owner"),
            )
            .await
            .unwrap();
        let untouched = service
            .find_or_create_guest("account@example.com", Some("Other"), Some("Name"))
            .await
            .unwrap();
        assert_eq!(untouched.id, account.id);
        assert_eq!(untouched.first_name.as_deref(), Some("Account"));
        assert!(untouched.has_account);
    }

    #[tokio::test]
    async fn address_crud_and_default_switching() {
        let (service, _) = service().await;
        let customer = service
            .create_customer(CreateCustomerInput::new("owner@example.com"))
            .await
            .unwrap();

        let mut first_input = address("1 Main St");
        first_input.is_default_shipping = true;
        let first = service
            .add_address(&customer.id, first_input)
            .await
            .unwrap();
        assert!(first.is_default_shipping);
        assert!(!first.is_default_billing);
        assert_eq!(
            first.country_code.as_ref().map(CountryCode::as_str),
            Some("US")
        );

        let mut second_input = address("2 Main St");
        second_input.is_default_shipping = true;
        second_input.is_default_billing = true;
        let second = service
            .add_address(&customer.id, second_input)
            .await
            .unwrap();

        let first_after = service.get_address(&customer.id, &first.id).await.unwrap();
        assert!(!first_after.is_default_shipping);

        let listed = service.list_addresses(&customer.id).await.unwrap();
        assert_eq!(listed.len(), 2);

        let mut update_input = address("3 Main St");
        update_input.is_default_billing = true;
        let updated = service
            .update_address(&customer.id, &first.id, update_input)
            .await
            .unwrap();
        assert_eq!(updated.address_1.as_deref(), Some("3 Main St"));
        assert!(updated.is_default_billing);
        assert!(!updated.is_default_shipping);

        let second_after = service.get_address(&customer.id, &second.id).await.unwrap();
        assert!(!second_after.is_default_billing);

        service
            .remove_address(&customer.id, &first.id)
            .await
            .unwrap();
        let remaining = service.list_addresses(&customer.id).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, second.id);
        assert!(
            service
                .remove_address(&customer.id, &first.id)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn address_validation_rejects_incomplete_input() {
        let (service, _) = service().await;
        let customer = service
            .create_customer(CreateCustomerInput::new("owner@example.com"))
            .await
            .unwrap();

        let missing_city = AddressInput::new("1 Main St", "  ", "12345", "US");
        let error = service
            .add_address(&customer.id, missing_city)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));

        let invalid_country = AddressInput::new("1 Main St", "Springfield", "12345", "USA");
        let error = service
            .add_address(&customer.id, invalid_country)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));

        assert!(
            service
                .add_address(&CustomerId::new(), address("1 Main St"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn groups_are_created_assigned_and_removed() {
        let (service, _) = service().await;
        let customer = service
            .create_customer(CreateCustomerInput::new("owner@example.com"))
            .await
            .unwrap();
        let group = service
            .create_group("VIP", Metadata::empty())
            .await
            .unwrap();
        assert_eq!(group.name, "VIP");

        let duplicate = service
            .create_group("VIP", Metadata::empty())
            .await
            .unwrap_err();
        assert!(matches!(duplicate, AppError::Conflict(_)));

        service.assign_group(&customer.id, &group.id).await.unwrap();
        service.assign_group(&customer.id, &group.id).await.unwrap();
        let assigned = service.list_customer_groups(&customer.id).await.unwrap();
        assert_eq!(assigned.len(), 1);
        assert_eq!(assigned[0].id, group.id);

        assert_eq!(service.get_group(&group.id).await.unwrap(), group);
        assert_eq!(service.list_groups().await.unwrap().len(), 1);

        service.remove_group(&customer.id, &group.id).await.unwrap();
        assert!(
            service
                .list_customer_groups(&customer.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn registration_login_and_session_authentication() {
        let (service, db) = service().await;
        let customer = service
            .register(
                RegisterCustomerInput::new("Owner@Example.com", "hunter2hunter2")
                    .with_name("Ada", "Lovelace"),
            )
            .await
            .unwrap();
        assert!(customer.has_account);
        assert_eq!(customer.email, "owner@example.com");

        let stored_hash: String = sqlx::query_scalar(
            "SELECT password_hash FROM customer_credentials WHERE customer_id = ?",
        )
        .bind(customer.id.as_str())
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert!(stored_hash.starts_with("$argon2"));
        assert_ne!(stored_hash, "hunter2hunter2");

        let session = service
            .login("OWNER@example.com", "hunter2hunter2")
            .await
            .unwrap();
        assert_eq!(session.customer.id, customer.id);
        assert_eq!(session.token.len(), 64);
        assert!(!session.csrf_token.is_empty());

        let stored_token_hash: String =
            sqlx::query_scalar("SELECT token_hash FROM customer_sessions WHERE id = ?")
                .bind(session.session_id.as_str())
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(stored_token_hash, hash_token(&session.token));
        assert_ne!(stored_token_hash, session.token);

        let debug = format!("{session:?}");
        assert!(!debug.contains(&session.token));
        assert!(!debug.contains("hunter2"));

        let authenticated = service.authenticate_session(&session.token).await.unwrap();
        assert_eq!(authenticated.customer.id, customer.id);
        assert_eq!(authenticated.session_id, session.session_id);
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_email_are_unauthorized() {
        let (service, _) = service().await;
        service
            .register(RegisterCustomerInput::new(
                "owner@example.com",
                "hunter2hunter2",
            ))
            .await
            .unwrap();

        let error = service
            .login("owner@example.com", "wrong-password")
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Unauthorized));

        let error = service
            .login("missing@example.com", "hunter2hunter2")
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Unauthorized));
    }

    #[tokio::test]
    async fn logout_revokes_the_session() {
        let (service, _) = service().await;
        service
            .register(RegisterCustomerInput::new(
                "owner@example.com",
                "hunter2hunter2",
            ))
            .await
            .unwrap();
        let session = service
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();

        service.logout(&session.token).await.unwrap();
        assert!(service.authenticate_session(&session.token).await.is_err());
    }

    #[tokio::test]
    async fn expired_sessions_are_rejected() {
        let (service, db) = service().await;
        service
            .register(RegisterCustomerInput::new(
                "owner@example.com",
                "hunter2hunter2",
            ))
            .await
            .unwrap();
        let session = service
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();

        sqlx::query("UPDATE customer_sessions SET expires_at = 0 WHERE id = ?")
            .bind(session.session_id.as_str())
            .execute(db.pool())
            .await
            .unwrap();

        assert!(service.authenticate_session(&session.token).await.is_err());
    }

    #[tokio::test]
    async fn changing_password_revokes_sessions() {
        let (service, _) = service().await;
        let customer = service
            .register(RegisterCustomerInput::new(
                "owner@example.com",
                "hunter2hunter2",
            ))
            .await
            .unwrap();
        let session = service
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();

        service
            .change_password(&customer.id, "hunter2hunter2", "new-password-1")
            .await
            .unwrap();

        assert!(service.authenticate_session(&session.token).await.is_err());
        assert!(
            service
                .login("owner@example.com", "hunter2hunter2")
                .await
                .is_err()
        );
        assert!(
            service
                .login("owner@example.com", "new-password-1")
                .await
                .is_ok()
        );

        let wrong_current = service
            .change_password(&customer.id, "nope-nope", "another-password")
            .await
            .unwrap_err();
        assert!(matches!(wrong_current, AppError::Unauthorized));

        let short = service
            .change_password(&customer.id, "new-password-1", "short")
            .await
            .unwrap_err();
        assert!(matches!(short, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn duplicate_registration_conflicts() {
        let (service, _) = service().await;
        service
            .register(RegisterCustomerInput::new(
                "owner@example.com",
                "hunter2hunter2",
            ))
            .await
            .unwrap();
        let error = service
            .register(RegisterCustomerInput::new(
                "OWNER@example.com",
                "other-password",
            ))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn deleted_customers_cannot_authenticate() {
        let (service, _) = service().await;
        let customer = service
            .register(RegisterCustomerInput::new(
                "owner@example.com",
                "hunter2hunter2",
            ))
            .await
            .unwrap();
        let session = service
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();

        service.delete_customer(&customer.id).await.unwrap();

        assert!(service.authenticate_session(&session.token).await.is_err());
        assert!(
            service
                .login("owner@example.com", "hunter2hunter2")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn customer_validation_bounds_input() {
        let (service, _) = service().await;

        let long_name = "a".repeat(MAX_NAME_LENGTH + 1);
        let error = service
            .create_customer(
                CreateCustomerInput::new("owner@example.com").with_name(long_name, "B"),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));

        let long_phone = "1".repeat(MAX_PHONE_LENGTH + 1);
        let error = service
            .create_customer(CreateCustomerInput::new("owner@example.com").with_phone(long_phone))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));

        let error = service
            .register(RegisterCustomerInput::new("owner@example.com", "short"))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn sessions_are_listed_without_token_digests() {
        let (service, _) = service().await;
        let customer = service
            .register(RegisterCustomerInput::new(
                "owner@example.com",
                "hunter2hunter2",
            ))
            .await
            .unwrap();
        let first = service
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();
        let second = service
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();

        let sessions = service.list_sessions(&customer.id).await.unwrap();
        assert_eq!(sessions.len(), 2);

        service.revoke_all_sessions(&customer.id).await.unwrap();
        assert!(service.authenticate_session(&first.token).await.is_err());
        assert!(service.authenticate_session(&second.token).await.is_err());

        let sessions = service.list_sessions(&customer.id).await.unwrap();
        assert!(sessions.iter().all(|session| session.revoked_at.is_some()));
    }
}
