//! Customer addresses.

//! SQLite-backed customer application service.
//!
//! Every operation is implicitly scoped to the store owning the injected
//! [`StoreDatabase`]. Credentials are hashed with Argon2id and session tokens
//! are looked up exclusively by their SHA-256 digest.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CountryCode, CustomerId, Metadata, now_timestamp, validate_metadata,
};
use sqlx::{FromRow, Sqlite, Transaction};

use crate::domain::{Address, AddressInput, MAX_NAME_LENGTH, MAX_PHONE_LENGTH};

use super::*;

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

fn required_optional(value: Option<String>, field: &str) -> Result<String, AppError> {
    let value = value
        .map(|value| value.trim().to_owned())
        .unwrap_or_default();
    if value.is_empty() {
        return Err(AppError::validation(format!("{field} is required")));
    }
    Ok(value)
}

impl CustomerService {
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
    ///
    /// The row is read, verified and updated under one write lock; defaults are
    /// only cleared once the target row has been confirmed.
    pub async fn update_address(
        &self,
        customer_id: &CustomerId,
        address_id: &str,
        input: AddressInput,
    ) -> Result<Address, AppError> {
        let normalized = normalize_address(input)?;
        let now = now_timestamp();

        let mut tx = self.db.begin().await?;
        let sql = format!(
            "SELECT {ADDRESS_COLUMNS} FROM customer_addresses
             WHERE id = ? AND customer_id = ?"
        );
        let existing = sqlx::query_as::<_, AddressRow>(&sql)
            .bind(address_id)
            .bind(customer_id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("address", address_id))?;

        let address = Address {
            id: existing.id,
            customer_id: CustomerId::from_string(existing.customer_id),
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

        let updated = sqlx::query(
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
        if updated.rows_affected() != 1 {
            return Err(AppError::not_found("address", address_id));
        }

        if address.is_default_shipping {
            clear_default_shipping(&mut tx, customer_id, Some(address_id)).await?;
        }
        if address.is_default_billing {
            clear_default_billing(&mut tx, customer_id, Some(address_id)).await?;
        }

        let row = sqlx::query_as::<_, AddressRow>(&sql)
            .bind(address_id)
            .bind(customer_id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("address", address_id))?;
        tx.commit().await.map_err(database_error)?;

        Ok(row.into_address())
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
}
