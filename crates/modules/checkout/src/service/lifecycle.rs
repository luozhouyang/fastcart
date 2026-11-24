//! Session creation, lookup and expiry.

use super::*;

impl CheckoutService {
    /// Creates a checkout session and records its initial history row.
    ///
    /// Idempotency: when `idempotency_key` was already used, the existing
    /// session is returned without creating another row. A cart may only have
    /// one checkout session (enforced by a unique index); creating a second one
    /// with a different idempotency key returns a conflict.
    pub async fn create(&self, input: CreateCheckoutInput) -> Result<CheckoutSession, AppError> {
        let currency = CurrencyCode::new(&input.currency_code)?.as_str().to_owned();
        let email = match input.email.as_deref() {
            Some(value) => {
                validate_email(value, "email")?;
                Some(normalize_email(value))
            }
            None => None,
        };
        if input.expires_in_seconds <= 0 {
            return Err(AppError::validation(
                "expires_in_seconds must be positive".to_owned(),
            ));
        }
        let idempotency_key = input
            .idempotency_key
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let now = now_timestamp();
        let expires_at = now
            .checked_add(input.expires_in_seconds)
            .ok_or_else(|| AppError::validation("checkout expiry is out of range"))?;

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let cart_id = input.cart_id.as_str().to_owned();
        let requested_customer = input.customer_id.as_ref().map(|value| value.as_str());

        if let Some(key) = idempotency_key.as_deref() {
            let existing =
                sqlx::query_as::<_, (String, String, Option<i64>, String, Option<String>)>(
                    "SELECT id, status, expires_at, cart_id, customer_id \
                 FROM checkout_sessions WHERE idempotency_key = ?",
                )
                .bind(key)
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
            if let Some((existing, status, expires_at, existing_cart, existing_customer)) = existing
            {
                let stale = matches!(status.as_str(), "failed" | "canceled")
                    || expires_at.is_some_and(|expires| expires <= now);
                if stale {
                    // A failed or expired attempt must not poison retries with
                    // the same key; release the key and start a fresh session.
                    cancel_stale_session(&mut tx, &existing, status.as_str(), now).await?;
                } else if existing_cart != cart_id
                    || existing_customer.as_deref() != requested_customer
                {
                    // Never replay another cart's or customer's checkout for a
                    // reused key; that would expose a different session's data.
                    return Err(AppError::conflict(
                        "idempotency key was used for a different checkout".to_owned(),
                    ));
                } else {
                    tx.commit().await.map_err(database_error)?;
                    return self.get(&CheckoutId::from_string(existing)).await;
                }
            }
        }

        let cart = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT status, currency_code, customer_id FROM carts WHERE id = ?",
        )
        .bind(&cart_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        let Some((cart_status, cart_currency, cart_customer)) = cart else {
            return Err(AppError::not_found("cart", &cart_id));
        };
        if cart_status != "open" {
            return Err(AppError::conflict(
                "only open carts can be checked out".to_owned(),
            ));
        }
        if cart_currency != currency {
            return Err(AppError::validation(
                "checkout currency must match the cart currency".to_owned(),
            ));
        }
        if requested_customer != cart_customer.as_deref() {
            return Err(AppError::conflict(
                "checkout customer does not match the cart owner".to_owned(),
            ));
        }

        let existing_cart = sqlx::query_as::<_, (String, String, Option<i64>)>(
            "SELECT id, status, expires_at FROM checkout_sessions \
             WHERE cart_id = ? AND status NOT IN ('failed', 'canceled')",
        )
        .bind(&cart_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        if let Some((existing, status, expires_at)) = existing_cart {
            if expires_at.is_some_and(|expires| expires <= now) {
                cancel_stale_session(&mut tx, &existing, &status, now).await?;
            } else {
                return Err(AppError::conflict(
                    "a checkout session already exists for this cart".to_owned(),
                ));
            }
        }

        let cart_items = sqlx::query_as::<_, (String, i64)>(
            "SELECT variant_id, quantity FROM cart_items WHERE cart_id = ? ORDER BY variant_id",
        )
        .bind(&cart_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?
        .into_iter()
        .map(|(variant_id, quantity)| CheckoutCartItem {
            variant_id,
            quantity,
        })
        .collect::<Vec<_>>();
        if cart_items.is_empty() {
            return Err(AppError::validation("cart is empty"));
        }

        let id = CheckoutId::new();
        sqlx::query(
            "INSERT INTO checkout_sessions (
                id, cart_id, customer_id, email, currency_code, region_id, status,
                shipping_address, billing_address, shipping_method, totals, cart_items,
                promotion_codes, payment_session_id, order_id, idempotency_key,
                expires_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(&cart_id)
        .bind(input.customer_id.as_ref().map(|value| value.as_str()))
        .bind(email.as_deref())
        .bind(&currency)
        .bind(input.region_id.as_deref())
        .bind(CheckoutStatus::Created.as_str())
        .bind(json_text(&input.shipping_address)?)
        .bind(json_text(&input.billing_address)?)
        .bind(json_text(&input.shipping_method)?)
        .bind(json_text(&input.totals)?)
        .bind(
            serde_json::to_string(&cart_items)
                .map_err(|error| AppError::validation(error.to_string()))?,
        )
        .bind(json_text(&input.promotion_codes)?)
        .bind(idempotency_key.as_deref())
        .bind(expires_at)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(map_create_error)?;

        insert_history(
            &mut tx,
            &id,
            None,
            CheckoutStatus::Created,
            Some("checkout created"),
            now,
        )
        .await?;

        tx.commit().await.map_err(database_error)?;
        self.get(&id).await
    }
}

impl CheckoutService {
    /// Loads a checkout session by identifier.
    pub async fn get(&self, id: &CheckoutId) -> Result<CheckoutSession, AppError> {
        let row = sqlx::query_as::<_, CheckoutRow>(
            "SELECT id, cart_id, customer_id, email, currency_code, region_id, status,
                    shipping_address, billing_address, shipping_method, totals, cart_items,
                    promotion_codes, payment_session_id, order_id, idempotency_key,
                    expires_at, created_at, updated_at
             FROM checkout_sessions WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row.into_domain(),
            None => Err(AppError::not_found("checkout session", id)),
        }
    }
}

impl CheckoutService {
    /// Loads the checkout session for a cart, if one exists.
    pub async fn get_by_cart(&self, cart_id: &CartId) -> Result<Option<CheckoutSession>, AppError> {
        let row = sqlx::query_as::<_, CheckoutRow>(
            "SELECT id, cart_id, customer_id, email, currency_code, region_id, status,
                    shipping_address, billing_address, shipping_method, totals, cart_items,
                    promotion_codes, payment_session_id, order_id, idempotency_key,
                    expires_at, created_at, updated_at
             FROM checkout_sessions WHERE cart_id = ?",
        )
        .bind(cart_id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        row.map(CheckoutRow::into_domain).transpose()
    }
}

impl CheckoutService {
    /// Loads the checkout session created with an idempotency key, if any.
    pub async fn get_by_idempotency_key(
        &self,
        key: &str,
    ) -> Result<Option<CheckoutSession>, AppError> {
        let row = sqlx::query_as::<_, CheckoutRow>(
            "SELECT id, cart_id, customer_id, email, currency_code, region_id, status,
                    shipping_address, billing_address, shipping_method, totals, cart_items,
                    promotion_codes, payment_session_id, order_id, idempotency_key,
                    expires_at, created_at, updated_at
             FROM checkout_sessions WHERE idempotency_key = ?",
        )
        .bind(key)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        row.map(CheckoutRow::into_domain).transpose()
    }
}

impl CheckoutService {
    /// Lists checkout sessions in a given state, newest first.
    pub async fn list_by_status(
        &self,
        status: CheckoutStatus,
    ) -> Result<Vec<CheckoutSession>, AppError> {
        let rows = sqlx::query_as::<_, CheckoutRow>(
            "SELECT id, cart_id, customer_id, email, currency_code, region_id, status,
                    shipping_address, billing_address, shipping_method, totals, cart_items,
                    promotion_codes, payment_session_id, order_id, idempotency_key,
                    expires_at, created_at, updated_at
             FROM checkout_sessions WHERE status = ?
             ORDER BY created_at DESC, id DESC",
        )
        .bind(status.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(CheckoutRow::into_domain).collect()
    }
}

impl CheckoutService {
    /// Returns the state history of a checkout session, oldest first.
    pub async fn list_history(
        &self,
        id: &CheckoutId,
    ) -> Result<Vec<CheckoutStateHistory>, AppError> {
        // Ensure a missing checkout returns a not-found rather than an empty list.
        self.get(id).await?;
        let rows = sqlx::query_as::<_, HistoryRow>(
            "SELECT id, checkout_id, from_state, to_state, reason, created_at
             FROM checkout_state_history WHERE checkout_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(HistoryRow::into_domain).collect()
    }
}

impl CheckoutService {
    /// Narrows a set of checkout ids to those that can never complete.
    ///
    /// Used by maintenance to decide which promotion claims to give back: a
    /// checkout in a terminal state will never place its order, so the
    /// redemption it took before the capture would otherwise stay consumed.
    ///
    /// Only the ids passed in are considered, so the caller keeps the set
    /// bounded by the outstanding claims rather than by checkout history.
    pub async fn abandoned_ids(&self, candidates: &[String]) -> Result<Vec<String>, AppError> {
        let mut abandoned = Vec::new();
        for chunk in candidates.chunks(900) {
            let sql = format!(
                "SELECT id FROM checkout_sessions WHERE id IN ({}) \
                 AND status IN ('failed', 'canceled', 'expired')",
                vec!["?"; chunk.len()].join(", ")
            );
            let mut query = sqlx::query_scalar::<_, String>(&sql);
            for id in chunk {
                query = query.bind(id);
            }
            abandoned.extend(
                query
                    .fetch_all(self.db.pool())
                    .await
                    .map_err(database_error)?,
            );
        }
        Ok(abandoned)
    }
}

impl CheckoutService {
    /// Marks every expirable session whose expiry is at or before `now` as failed.
    ///
    /// Returns the number of sessions expired.
    pub async fn expire_due(&self, now: i64) -> Result<u64, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let due = sqlx::query_as::<_, (String, String)>(
            "SELECT cs.id, cs.status FROM checkout_sessions cs
             WHERE cs.expires_at IS NOT NULL
               AND cs.expires_at <= ?
               AND cs.status IN ('created', 'validating', 'ready_for_payment', 'payment_pending')
               AND NOT EXISTS (
                    SELECT 1 FROM payment_sessions ps
                    WHERE ps.id = cs.payment_session_id
                      AND ps.status IN ('captured', 'partially_refunded', 'refunded')
               )",
        )
        .bind(now)
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;

        for (id, status) in &due {
            let from = CheckoutStatus::from_stored(status)?;
            let updated = sqlx::query(
                "UPDATE checkout_sessions
                 SET status = ?, updated_at = ?,
                     placement_token = NULL, placement_claimed_at = NULL
                 WHERE id = ? AND status = ?",
            )
            .bind(CheckoutStatus::Failed.as_str())
            .bind(now)
            .bind(id)
            .bind(status)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if updated.rows_affected() == 0 {
                continue;
            }
            insert_history(
                &mut tx,
                &CheckoutId::from_string(id.clone()),
                Some(from),
                CheckoutStatus::Failed,
                Some("expired"),
                now,
            )
            .await?;
        }

        tx.commit().await.map_err(database_error)?;
        Ok(u64::try_from(due.len()).unwrap_or(u64::MAX))
    }
}
