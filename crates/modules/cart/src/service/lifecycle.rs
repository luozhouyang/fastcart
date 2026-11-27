//! Cart lifecycle: creation, lookup, completion, expiry and listing.

use super::*;

impl CartService {
    /// Creates an anonymous or customer cart.
    pub async fn create_cart(&self, input: CreateCartInput) -> Result<Cart, AppError> {
        let currency = CurrencyCode::new(input.currency_code)?;
        validate_metadata(&input.metadata)?;

        let email = match input.email {
            Some(value) => {
                validate_email(&value, "email")?;
                Some(normalize_email(&value))
            }
            None => None,
        };

        let ttl = input.expires_in_seconds.unwrap_or(DEFAULT_CART_TTL_SECONDS);
        if ttl < 0 {
            return Err(AppError::validation(
                "expires_in_seconds must not be negative",
            ));
        }

        let now = now_timestamp();
        let cart_id = CartId::new();
        let expires_at = now.saturating_add(ttl);

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "INSERT INTO carts (id, currency_code, customer_id, email, sales_channel_id, region_id, \
             status, shipping_address, billing_address, metadata, created_at, updated_at, \
             completed_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, 'open', '{}', '{}', ?, ?, ?, NULL, ?)",
        )
        .bind(cart_id.as_str())
        .bind(currency.as_str())
        .bind(input.customer_id.as_ref().map(CustomerId::as_str))
        .bind(email.as_deref())
        .bind(input.sales_channel_id.as_deref())
        .bind(input.region_id.as_deref())
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        let cart = Self::load_aggregate(&mut tx, &cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Loads the full aggregate and recalculates its totals.
    ///
    /// Every child table is read through one deferred transaction so a
    /// concurrent writer cannot produce a torn snapshot with totals derived
    /// from rows belonging to different revisions.
    pub async fn get_cart(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        let mut tx = self.db.pool().begin().await.map_err(database_error)?;
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Builds the aggregate from one transaction/connection.
    pub(super) async fn load_aggregate(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
    ) -> Result<Cart, AppError> {
        let row = Self::cart_row_tx(tx, cart_id).await?;
        let items = Self::load_items(tx, cart_id).await?;
        let discounts = Self::load_discounts(tx, cart_id).await?;
        let gift_cards = Self::load_gift_cards(tx, cart_id).await?;
        let shipping_methods = Self::load_shipping_methods(tx, cart_id).await?;
        let shipping_adjustments = Self::load_shipping_adjustments(tx, cart_id).await?;
        let tax_lines = Self::load_tax_lines(tx, cart_id).await?;
        let addresses = Self::load_addresses(tx, cart_id).await?;

        let item_adjustments = items
            .iter()
            .flat_map(|item| item.adjustments.iter().cloned())
            .collect::<Vec<_>>();

        let currency_code = CurrencyCode::new(row.currency_code.as_str())?;
        let shipping_address = match addresses.shipping {
            Some(address) => address,
            None => parse_address(&row.shipping_address)?,
        };
        let billing_address = match addresses.billing {
            Some(address) => address,
            None => parse_address(&row.billing_address)?,
        };
        let mut cart = Cart {
            id: CartId::from_string(row.id),
            currency_code: currency_code.clone(),
            customer_id: row.customer_id.map(CustomerId::from_string),
            email: row.email,
            sales_channel_id: row.sales_channel_id,
            region_id: row.region_id,
            status: CartStatus::from_db(&row.status),
            shipping_address,
            billing_address,
            items,
            discounts,
            gift_cards,
            shipping_methods,
            item_adjustments,
            shipping_adjustments,
            tax_lines,
            totals: CartTotals::zero(currency_code),
            metadata: Metadata::from_json_str(&row.metadata),
            created_at: row.created_at,
            updated_at: row.updated_at,
            completed_at: row.completed_at,
            expires_at: row.expires_at,
        };
        recalculate_totals(&mut cart);
        Ok(cart)
    }
}

impl CartService {
    /// Returns the newest open cart owned by the customer.
    ///
    /// Carts in any other lifecycle state (completed or expired) are ignored,
    /// as are carts belonging to other customers. Returns `None` when the
    /// customer has no open cart; database failures are surfaced as
    /// [`AppError`].
    pub async fn active_cart_for_customer(
        &self,
        customer_id: &CustomerId,
    ) -> Result<Option<Cart>, AppError> {
        let now = now_timestamp();
        let cart_id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM carts WHERE customer_id = ? AND status = ? \
             AND (expires_at IS NULL OR expires_at > ?) \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(customer_id.as_str())
        .bind(CartStatus::Open.as_str())
        .bind(now)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;

        match cart_id {
            Some(cart_id) => self.get_cart(&CartId::from_string(cart_id)).await.map(Some),
            None => Ok(None),
        }
    }
}

impl CartService {
    /// Marks an open cart as completed.
    pub async fn complete_cart(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query(
            "UPDATE carts SET status = 'completed', completed_at = ?, updated_at = ? \
             WHERE id = ? AND status = 'open'",
        )
        .bind(now)
        .bind(now)
        .bind(cart_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        if result.rows_affected() == 0 {
            let _ = Self::cart_row_tx(&mut tx, cart_id).await?;
            return Err(AppError::conflict("only open carts can be completed"));
        }
        let cart = Self::load_aggregate(&mut tx, cart_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(cart)
    }
}

impl CartService {
    /// Expires every open cart whose expiration timestamp passed.
    ///
    /// Returns the number of carts transitioned to `expired`.
    pub async fn expire_due(&self, now: i64) -> Result<u64, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query(
            "UPDATE carts SET status = 'expired', updated_at = ? \
             WHERE status = 'open' AND expires_at IS NOT NULL AND expires_at <= ?",
        )
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        let affected = result.rows_affected();
        tx.commit().await.map_err(database_error)?;
        Ok(affected)
    }
}

impl CartService {
    /// Lists carts, optionally filtered by customer.
    pub async fn list_carts(
        &self,
        customer_id: Option<&CustomerId>,
        pagination: Pagination,
    ) -> Result<PageResult<Cart>, AppError> {
        let (count, ids): (i64, Vec<String>) = match customer_id {
            Some(customer) => {
                let count: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM carts WHERE customer_id = ?")
                        .bind(customer.as_str())
                        .fetch_one(self.db.pool())
                        .await
                        .map_err(database_error)?;
                let ids: Vec<String> = sqlx::query_scalar(
                    "SELECT id FROM carts WHERE customer_id = ? ORDER BY created_at DESC, id DESC \
                     LIMIT ? OFFSET ?",
                )
                .bind(customer.as_str())
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?;
                (count, ids)
            }
            None => {
                let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM carts")
                    .fetch_one(self.db.pool())
                    .await
                    .map_err(database_error)?;
                let ids: Vec<String> = sqlx::query_scalar(
                    "SELECT id FROM carts ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?",
                )
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?;
                (count, ids)
            }
        };

        let mut carts = Vec::with_capacity(ids.len());
        for id in ids {
            carts.push(self.get_cart(&CartId::from_string(id)).await?);
        }
        Ok(PageResult::new(carts, count, pagination))
    }
}
