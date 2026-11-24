//! Customer, email and address attachment.

use super::*;

impl CartService {
    /// Attaches an authenticated customer and optionally updates the email.
    ///
    /// A cart that already belongs to a different customer is rejected with a
    /// conflict rather than silently transferred. Ownership and lifecycle are
    /// decided inside the immediate transaction and the update is conditional
    /// so a concurrent attach cannot be lost.
    pub async fn attach_customer(
        &self,
        cart_id: &CartId,
        customer_id: &CustomerId,
        email: Option<String>,
    ) -> Result<Cart, AppError> {
        let email = match email {
            Some(value) => {
                validate_email(&value, "email")?;
                Some(normalize_email(&value))
            }
            None => None,
        };

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let row = Self::cart_row_tx(&mut tx, cart_id).await?;
        Self::check_open(&row)?;
        if let Some(existing) = row.customer_id.as_deref()
            && existing != customer_id.as_str()
        {
            return Err(AppError::conflict(
                "cart already belongs to a different customer",
            ));
        }

        let result = sqlx::query(
            "UPDATE carts SET customer_id = ?, email = COALESCE(?, email), updated_at = ? \
             WHERE id = ? AND status = 'open' \
             AND (expires_at IS NULL OR expires_at > ?) \
             AND (customer_id IS NULL OR customer_id = ?)",
        )
        .bind(customer_id.as_str())
        .bind(email.as_deref())
        .bind(now)
        .bind(cart_id.as_str())
        .bind(now)
        .bind(customer_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        if result.rows_affected() == 0 {
            // The conditional update lost: classify the current state so a
            // missing cart is not reported as an ownership conflict.
            let current = Self::cart_row_tx(&mut tx, cart_id).await?;
            Self::check_open(&current)?;
            return Err(AppError::conflict(
                "cart already belongs to a different customer",
            ));
        }

        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Sets the contact email.
    pub async fn set_email(&self, cart_id: &CartId, email: &str) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        validate_email(email, "email")?;
        let email = normalize_email(email);
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
        sqlx::query("UPDATE carts SET email = ?, updated_at = ? WHERE id = ?")
            .bind(email)
            .bind(now)
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Sets the shipping address snapshot.
    pub async fn set_shipping_address(
        &self,
        cart_id: &CartId,
        input: AddressInput,
    ) -> Result<Cart, AppError> {
        self.set_address(cart_id, "shipping", input).await
    }
}

impl CartService {
    /// Sets the billing address snapshot.
    pub async fn set_billing_address(
        &self,
        cart_id: &CartId,
        input: AddressInput,
    ) -> Result<Cart, AppError> {
        self.set_address(cart_id, "billing", input).await
    }
}

impl CartService {
    pub(super) async fn set_address(
        &self,
        cart_id: &CartId,
        address_type: &str,
        input: AddressInput,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        validate_metadata(&input.metadata)?;
        let address = input.try_into_address()?;
        let now = now_timestamp();

        let column = match address_type {
            "shipping" => "shipping_address",
            _ => "billing_address",
        };

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
        sqlx::query(
            "INSERT INTO cart_addresses (id, cart_id, type, first_name, last_name, company, \
             address_1, address_2, city, province, postal_code, country_code, phone, metadata, \
             created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (cart_id, type) DO UPDATE SET first_name = excluded.first_name, \
             last_name = excluded.last_name, company = excluded.company, \
             address_1 = excluded.address_1, address_2 = excluded.address_2, city = excluded.city, \
             province = excluded.province, postal_code = excluded.postal_code, \
             country_code = excluded.country_code, phone = excluded.phone, \
             metadata = excluded.metadata, updated_at = excluded.updated_at",
        )
        .bind(format!("{cart_id}_{address_type}"))
        .bind(cart_id.as_str())
        .bind(address_type)
        .bind(address.first_name.as_deref())
        .bind(address.last_name.as_deref())
        .bind(address.company.as_deref())
        .bind(address.address_1.as_deref())
        .bind(address.address_2.as_deref())
        .bind(address.city.as_deref())
        .bind(address.province.as_deref())
        .bind(address.postal_code.as_deref())
        .bind(address.country_code.as_deref())
        .bind(address.phone.as_deref())
        .bind(address.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        let update = format!("UPDATE carts SET {column} = ?, updated_at = ? WHERE id = ?");
        sqlx::query(&update)
            .bind(json_string(&address)?)
            .bind(now)
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;

        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}
