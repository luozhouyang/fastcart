//! Bulk recalculation from other modules.

use super::*;

impl CartService {
    /// Replaces every derived snapshot in one transaction.
    ///
    /// The caller computes prices, adjustments and tax from other modules and
    /// hands the whole result over. Validating and writing it atomically means
    /// a rejected recalculation leaves the previous snapshot intact instead of
    /// a cart whose discounts were cleared before the failing step ran.
    pub async fn apply_recalculation(
        &self,
        cart_id: &CartId,
        input: RecalculationInput,
    ) -> Result<Cart, AppError> {
        let cart = self.ensure_open(cart_id).await?;
        let cart_currency = CurrencyCode::new(cart.currency_code.as_str())?;
        let input_currency = CurrencyCode::new(input.currency_code.as_str())?;
        if !input.prices.is_empty() && input_currency != cart_currency {
            return Err(AppError::conflict(format!(
                "cart currency {cart_currency} does not match price currency {input_currency}"
            )));
        }
        for price in &input.prices {
            validate_non_negative_amount(price.unit_price, "unit_price")?;
        }
        for (_, adjustments) in &input.item_adjustments {
            for adjustment in adjustments {
                validate_adjustment_input(adjustment)?;
            }
        }
        for adjustment in &input.shipping_adjustments {
            validate_adjustment_input(adjustment)?;
        }
        for line in &input.tax_lines {
            validate_tax_line_input(line)?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;

        if let Some(inclusive) = input.tax_inclusive {
            sqlx::query(
                "UPDATE cart_items SET is_tax_inclusive = ?, updated_at = ? WHERE cart_id = ?",
            )
            .bind(inclusive)
            .bind(now)
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            sqlx::query("UPDATE cart_shipping_methods SET is_tax_inclusive = ? WHERE cart_id = ?")
                .bind(inclusive)
                .bind(cart_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        }

        for price in &input.prices {
            let result = sqlx::query(
                "UPDATE cart_items SET unit_price = ?, updated_at = ? WHERE id = ? AND cart_id = ?",
            )
            .bind(price.unit_price)
            .bind(now)
            .bind(price.item_id.as_str())
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if result.rows_affected() == 0 {
                return Err(AppError::not_found("cart item", &price.item_id));
            }
        }

        for (item_id, adjustments) in &input.item_adjustments {
            ensure_item_belongs_to_cart(&mut tx, cart_id, item_id).await?;
            sqlx::query("DELETE FROM cart_item_adjustments WHERE cart_item_id = ?")
                .bind(item_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            for adjustment in adjustments {
                insert_adjustment(
                    &mut tx,
                    "cart_item_adjustments",
                    "cart_item_id",
                    item_id.as_str(),
                    adjustment,
                    now,
                )
                .await?;
            }
        }

        if !input.shipping_adjustments.is_empty() {
            let method_id: Option<String> = sqlx::query_scalar(
                "SELECT id FROM cart_shipping_methods WHERE cart_id = ? \
                 ORDER BY created_at ASC, id ASC LIMIT 1",
            )
            .bind(cart_id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            let Some(method_id) = method_id else {
                return Err(AppError::not_found("shipping method", cart_id));
            };
            sqlx::query(
                "DELETE FROM cart_shipping_method_adjustments WHERE shipping_method_id = ?",
            )
            .bind(&method_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            for adjustment in &input.shipping_adjustments {
                insert_adjustment(
                    &mut tx,
                    "cart_shipping_method_adjustments",
                    "shipping_method_id",
                    &method_id,
                    adjustment,
                    now,
                )
                .await?;
            }
        } else {
            // A recalculation always replaces the shipping snapshot, so an
            // empty list clears whatever the previous pass wrote.
            sqlx::query(
                "DELETE FROM cart_shipping_method_adjustments WHERE shipping_method_id IN \
                 (SELECT id FROM cart_shipping_methods WHERE cart_id = ?)",
            )
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        // Reject tax lines whose scoped targets do not belong to this cart
        // before replacing the existing snapshot.
        for line in &input.tax_lines {
            if let Some(item_id) = &line.cart_item_id {
                let found: Option<String> =
                    sqlx::query_scalar("SELECT id FROM cart_items WHERE id = ? AND cart_id = ?")
                        .bind(item_id.as_str())
                        .bind(cart_id.as_str())
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(database_error)?;
                if found.is_none() {
                    return Err(AppError::validation(
                        "tax line cart_item_id does not belong to the cart",
                    ));
                }
            }
            if let Some(shipping_method_id) = &line.shipping_method_id {
                let found: Option<String> = sqlx::query_scalar(
                    "SELECT id FROM cart_shipping_methods \
                     WHERE cart_id = ? AND shipping_option_id = ?",
                )
                .bind(cart_id.as_str())
                .bind(shipping_method_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
                if found.is_none() {
                    return Err(AppError::validation(
                        "tax line shipping_method_id does not belong to the cart",
                    ));
                }
            }
        }
        sqlx::query("DELETE FROM cart_tax_lines WHERE cart_id = ?")
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        for line in &input.tax_lines {
            sqlx::query(
                "INSERT INTO cart_tax_lines (id, cart_id, cart_item_id, shipping_method_id, name, \
                 rate_bps, amount, is_tax_inclusive, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row_id("tax"))
            .bind(cart_id.as_str())
            .bind(line.cart_item_id.as_ref().map(LineItemId::as_str))
            .bind(line.shipping_method_id.as_deref())
            .bind(&line.name)
            .bind(line.rate_bps)
            .bind(line.amount)
            .bind(line.is_tax_inclusive)
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
    /// Recalculates and returns the current totals.
    pub async fn recalculate(&self, cart_id: &CartId) -> Result<CartTotals, AppError> {
        Ok(self.get_cart(cart_id).await?.totals)
    }
}
