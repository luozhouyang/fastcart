//! Cart, item and shipping discount and tax snapshots.

use super::*;

impl CartService {
    /// Adds a cart-level discount snapshot.
    pub async fn add_discount(
        &self,
        cart_id: &CartId,
        input: DiscountInput,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        validate_discount_input(&input)?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
        upsert_discount(&mut tx, cart_id, &input, now).await?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Removes every discount produced by a promotion.
    pub async fn remove_discount(
        &self,
        cart_id: &CartId,
        promotion_id: &PromotionId,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;

        let result =
            sqlx::query("DELETE FROM cart_discounts WHERE cart_id = ? AND promotion_id = ?")
                .bind(cart_id.as_str())
                .bind(promotion_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("cart discount", promotion_id));
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Replaces every cart-level discount, as promotion workflows do.
    pub async fn replace_discounts(
        &self,
        cart_id: &CartId,
        discounts: Vec<DiscountInput>,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        for discount in &discounts {
            validate_discount_input(discount)?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
        sqlx::query("DELETE FROM cart_discounts WHERE cart_id = ?")
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        for discount in &discounts {
            upsert_discount(&mut tx, cart_id, discount, now).await?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Appends one adjustment to a line item.
    pub async fn add_item_adjustment(
        &self,
        cart_id: &CartId,
        item_id: &LineItemId,
        input: AdjustmentInput,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        validate_adjustment_input(&input)?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
        ensure_item_belongs_to_cart(&mut tx, cart_id, item_id).await?;
        insert_adjustment(
            &mut tx,
            "cart_item_adjustments",
            "cart_item_id",
            item_id.as_str(),
            &input,
            now,
        )
        .await?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Replaces every adjustment attached to one line item.
    pub async fn replace_item_adjustments(
        &self,
        cart_id: &CartId,
        item_id: &LineItemId,
        adjustments: Vec<AdjustmentInput>,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        for adjustment in &adjustments {
            validate_adjustment_input(adjustment)?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
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
                &adjustment,
                now,
            )
            .await?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Replaces the shipping adjustments on the cart's single shipping method.
    pub async fn replace_shipping_adjustments(
        &self,
        cart_id: &CartId,
        adjustments: Vec<AdjustmentInput>,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        for adjustment in &adjustments {
            validate_adjustment_input(adjustment)?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
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

        sqlx::query("DELETE FROM cart_shipping_method_adjustments WHERE shipping_method_id = ?")
            .bind(&method_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        for adjustment in adjustments {
            insert_adjustment(
                &mut tx,
                "cart_shipping_method_adjustments",
                "shipping_method_id",
                &method_id,
                &adjustment,
                now,
            )
            .await?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Replaces every tax line on the cart.
    pub async fn replace_tax_lines(
        &self,
        cart_id: &CartId,
        lines: Vec<TaxLineInput>,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        for line in &lines {
            validate_tax_line_input(line)?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;

        // Reject tax lines whose scoped targets do not belong to this cart
        // before replacing the existing snapshot.
        for line in &lines {
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
        for line in lines {
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
