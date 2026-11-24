//! Shipping method selection.

use super::*;

impl CartService {
    /// Replaces the cart's shipping method and clears its shipping adjustments.
    pub async fn set_shipping_method(
        &self,
        cart_id: &CartId,
        input: SetShippingMethodInput,
    ) -> Result<Cart, AppError> {
        let cart = self.ensure_open(cart_id).await?;
        let cart_currency = CurrencyCode::new(cart.currency_code.as_str())?;
        let shipping_currency = CurrencyCode::new(input.currency_code.as_str())?;
        if shipping_currency != cart_currency {
            return Err(AppError::conflict(format!(
                "cart currency {cart_currency} does not match shipping currency {shipping_currency}"
            )));
        }
        validate_non_negative_amount(input.amount, "shipping amount")?;
        validate_length(&input.name, "name", 255)?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;

        sqlx::query("DELETE FROM cart_shipping_methods WHERE cart_id = ?")
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        // Shipping-scoped tax lines describe the replaced method; keeping them
        // would leave stale tax on the aggregate.
        sqlx::query(
            "DELETE FROM cart_tax_lines WHERE cart_id = ? AND shipping_method_id IS NOT NULL",
        )
        .bind(cart_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        sqlx::query(
            "INSERT INTO cart_shipping_methods (id, cart_id, shipping_option_id, name, amount, \
             currency_code, is_tax_inclusive, data, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row_id("shm"))
        .bind(cart_id.as_str())
        .bind(input.shipping_option_id.as_str())
        .bind(&input.name)
        .bind(input.amount)
        .bind(shipping_currency.as_str())
        .bind(input.is_tax_inclusive)
        .bind(json_string(&input.data)?)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}
