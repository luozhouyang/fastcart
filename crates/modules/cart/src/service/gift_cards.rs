//! Gift cards applied to a cart as a payment method.

use super::*;

impl CartService {
    /// Applies a gift card snapshot to an open cart.
    ///
    /// The card itself belongs to the payment module, so the caller validates
    /// it — existence, active status, currency and balance — and passes the
    /// snapshot here. The cart only enforces what its own rows can prove: the
    /// currency matches the cart and the card is not already applied. The
    /// stored amount is the customer's request capped by the balance handed in,
    /// so asking for more than the card holds records the balance rather than
    /// failing.
    ///
    /// Runs inside one `BEGIN IMMEDIATE` transaction; the unique index on
    /// `(cart_id, gift_card_id)` backs the duplicate check against a concurrent
    /// application.
    pub async fn apply_gift_card(
        &self,
        cart_id: &CartId,
        input: GiftCardApplication,
    ) -> Result<Cart, AppError> {
        if input.gift_card_id.trim().is_empty() {
            return Err(AppError::validation("gift_card_id must not be empty"));
        }
        if input.masked_code.trim().is_empty() {
            return Err(AppError::validation("gift card code must not be empty"));
        }
        if let Some(amount) = input.amount_minor {
            validate_non_negative_amount(amount, "gift card amount")?;
        }
        let _ = CurrencyCode::new(input.currency_code.as_str())?;

        let balance = input.balance_minor.max(0);
        let amount = input
            .amount_minor
            .map_or(balance, |amount| amount.min(balance));

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let cart = Self::ensure_open_tx(&mut tx, cart_id).await?;
        if cart.currency_code != input.currency_code {
            return Err(AppError::validation(format!(
                "gift card currency {} does not match the cart currency {}",
                input.currency_code, cart.currency_code
            )));
        }
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM cart_gift_cards WHERE cart_id = ? AND gift_card_id = ?",
        )
        .bind(cart_id.as_str())
        .bind(&input.gift_card_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        if existing.is_some() {
            return Err(AppError::conflict(
                "gift card is already applied to this cart",
            ));
        }

        sqlx::query(
            "INSERT INTO cart_gift_cards (id, cart_id, gift_card_id, masked_code, amount_minor, \
             balance_minor, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row_id("cgc"))
        .bind(cart_id.as_str())
        .bind(&input.gift_card_id)
        .bind(&input.masked_code)
        .bind(amount)
        .bind(balance)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }

    /// Removes an applied gift card from an open cart.
    ///
    /// Nothing was spent yet: the stored row is a quote, and the redemption
    /// happens at checkout.
    pub async fn remove_gift_card(
        &self,
        cart_id: &CartId,
        gift_card_id: &str,
    ) -> Result<Cart, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
        let deleted =
            sqlx::query("DELETE FROM cart_gift_cards WHERE cart_id = ? AND gift_card_id = ?")
                .bind(cart_id.as_str())
                .bind(gift_card_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        if deleted.rows_affected() == 0 {
            return Err(AppError::not_found("cart gift card", gift_card_id));
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }

    /// Refreshes the stored balance snapshot of every applied gift card.
    ///
    /// The cart module never reads the payment-owned card, so the caller that
    /// does (the workflow) hands the balances in. A card it no longer sees is
    /// left untouched: the checkout is what refuses an unusable card, with a
    /// conflict naming it, instead of a pricing pass silently dropping the
    /// customer's payment method.
    pub async fn refresh_gift_card_balances(
        &self,
        cart_id: &CartId,
        balances: &BTreeMap<String, i64>,
    ) -> Result<Cart, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        Self::ensure_open_tx(&mut tx, cart_id).await?;
        for (gift_card_id, balance) in balances {
            let balance = (*balance).max(0);
            sqlx::query(
                "UPDATE cart_gift_cards SET balance_minor = ? \
                 WHERE cart_id = ? AND gift_card_id = ? AND balance_minor != ?",
            )
            .bind(balance)
            .bind(cart_id.as_str())
            .bind(gift_card_id)
            .bind(balance)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}
