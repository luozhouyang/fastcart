//! Cart line item mutation.

use super::*;

impl CartService {
    /// Adds a line snapshot, merging into an existing variant line.
    ///
    /// When the variant is already present its quantity is incremented and the
    /// stored price snapshot is kept; workflows reprice explicitly with
    /// [`Self::set_item_unit_price`].
    pub async fn add_item(&self, cart_id: &CartId, input: AddItemInput) -> Result<Cart, AppError> {
        let cart = self.ensure_open(cart_id).await?;
        let cart_currency = CurrencyCode::new(cart.currency_code.as_str())?;
        let item_currency = CurrencyCode::new(input.currency_code.as_str())?;
        if item_currency != cart_currency {
            return Err(AppError::conflict(format!(
                "cart currency {cart_currency} does not match item currency {item_currency}"
            )));
        }
        validate_quantity(input.quantity, false)?;
        validate_non_negative_amount(input.unit_price, "unit_price")?;
        validate_length(&input.title, "title", 255)?;
        validate_metadata(&input.metadata)?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;

        let existing: Option<(String, i64)> = sqlx::query_as(
            "SELECT id, quantity FROM cart_items WHERE cart_id = ? AND variant_id = ?",
        )
        .bind(cart_id.as_str())
        .bind(input.variant_id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;

        if let Some((item_id, quantity)) = existing {
            let new_quantity = quantity
                .checked_add(input.quantity)
                .ok_or_else(|| AppError::validation("quantity overflow"))?;
            validate_quantity(new_quantity, false)?;
            sqlx::query("UPDATE cart_items SET quantity = ?, updated_at = ? WHERE id = ?")
                .bind(new_quantity)
                .bind(now)
                .bind(item_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        } else {
            sqlx::query(
                "INSERT INTO cart_items (id, cart_id, product_id, variant_id, quantity, title, \
                 variant_title, sku, thumbnail, unit_price, currency_code, is_tax_inclusive, \
                 requires_shipping, metadata, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(LineItemId::new().as_str())
            .bind(cart_id.as_str())
            .bind(input.product_id.as_str())
            .bind(input.variant_id.as_str())
            .bind(input.quantity)
            .bind(&input.title)
            .bind(input.variant_title.as_deref())
            .bind(input.sku.as_deref())
            .bind(input.thumbnail.as_deref())
            .bind(input.unit_price)
            .bind(item_currency.as_str())
            .bind(input.is_tax_inclusive)
            .bind(input.requires_shipping)
            .bind(input.metadata.to_json_string())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Sets a line quantity. A quantity of zero removes the line.
    pub async fn update_item_quantity(
        &self,
        cart_id: &CartId,
        item_id: &LineItemId,
        input: UpdateItemInput,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        validate_quantity(input.quantity, true)?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
        let found: Option<String> =
            sqlx::query_scalar("SELECT id FROM cart_items WHERE id = ? AND cart_id = ?")
                .bind(item_id.as_str())
                .bind(cart_id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
        if found.is_none() {
            return Err(AppError::not_found("cart item", item_id));
        }

        if input.quantity == 0 {
            sqlx::query("DELETE FROM cart_items WHERE id = ? AND cart_id = ?")
                .bind(item_id.as_str())
                .bind(cart_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            sqlx::query("DELETE FROM cart_tax_lines WHERE cart_id = ? AND cart_item_id = ?")
                .bind(cart_id.as_str())
                .bind(item_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        } else {
            sqlx::query(
                "UPDATE cart_items SET quantity = ?, updated_at = ? WHERE id = ? AND cart_id = ?",
            )
            .bind(input.quantity)
            .bind(now)
            .bind(item_id.as_str())
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Removes one line item.
    pub async fn remove_item(
        &self,
        cart_id: &CartId,
        item_id: &LineItemId,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;

        let result = sqlx::query("DELETE FROM cart_items WHERE id = ? AND cart_id = ?")
            .bind(item_id.as_str())
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("cart item", item_id));
        }
        sqlx::query("DELETE FROM cart_tax_lines WHERE cart_id = ? AND cart_item_id = ?")
            .bind(cart_id.as_str())
            .bind(item_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Removes every line item from the cart.
    pub async fn clear_items(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;

        sqlx::query("DELETE FROM cart_items WHERE cart_id = ?")
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("DELETE FROM cart_tax_lines WHERE cart_id = ? AND cart_item_id IS NOT NULL")
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Updates a line's unit price snapshot.
    ///
    /// Workflows call this after obtaining an authoritative price from the
    /// pricing module; the supplied currency must match the cart currency.
    pub async fn set_item_unit_price(
        &self,
        cart_id: &CartId,
        item_id: &LineItemId,
        unit_price: i64,
        currency_code: &str,
    ) -> Result<Cart, AppError> {
        let cart = self.ensure_open(cart_id).await?;
        validate_non_negative_amount(unit_price, "unit_price")?;
        let cart_currency = CurrencyCode::new(cart.currency_code.as_str())?;
        let price_currency = CurrencyCode::new(currency_code)?;
        if price_currency != cart_currency {
            return Err(AppError::conflict(format!(
                "cart currency {cart_currency} does not match price currency {price_currency}"
            )));
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
        let result = sqlx::query(
            "UPDATE cart_items SET unit_price = ?, updated_at = ? WHERE id = ? AND cart_id = ?",
        )
        .bind(unit_price)
        .bind(now)
        .bind(item_id.as_str())
        .bind(cart_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("cart item", item_id));
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}
