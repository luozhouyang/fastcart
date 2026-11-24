//! Order refunds, payment-status transitions and provider reconciliation.

use fastcart_infrastructure::{append_outbox_event, database_error};
use fastcart_kernel::{
    AppError, CurrencyCode, DomainEventEnvelope, OrderId, RefundId, now_timestamp,
    validate_non_negative_amount,
};

use crate::domain::{Order, PaymentStatus, RefundInput, RefundStatus};

use super::{OrderService, REFUND_IDEMPOTENCY_SCOPE, insert_history, is_unique_violation};

impl OrderService {
    /// Updates the payment status of an order.
    ///
    /// Used by checkout workflows after a provider capture or refund. Order
    /// status and payment status remain independent dimensions.
    pub async fn set_payment_status(
        &self,
        id: &OrderId,
        status: PaymentStatus,
    ) -> Result<Order, AppError> {
        let order = self.load_order(id).await?;
        if order.payment_status == status {
            return Ok(order);
        }
        let allowed = match order.payment_status {
            PaymentStatus::Awaiting | PaymentStatus::Authorized => {
                matches!(status, PaymentStatus::Captured | PaymentStatus::Failed)
            }
            PaymentStatus::Captured => matches!(
                status,
                PaymentStatus::PartiallyRefunded | PaymentStatus::Refunded
            ),
            PaymentStatus::PartiallyRefunded => matches!(status, PaymentStatus::Refunded),
            PaymentStatus::Refunded | PaymentStatus::Failed => false,
        };
        if !allowed {
            return Err(AppError::conflict(format!(
                "cannot transition payment status from {} to {}",
                order.payment_status.as_str(),
                status.as_str()
            )));
        }
        let updated = sqlx::query(
            "UPDATE orders SET payment_status = ?, updated_at = ?
             WHERE id = ? AND payment_status = ?",
        )
        .bind(status.as_str())
        .bind(now_timestamp())
        .bind(id.as_str())
        .bind(order.payment_status.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "order payment status changed concurrently",
            ));
        }
        self.get(id).await
    }

    /// Updates the order-level refund status from the payment source of truth.
    pub async fn set_refund_status(
        &self,
        id: &OrderId,
        status: RefundStatus,
    ) -> Result<Order, AppError> {
        let order = self.load_order(id).await?;
        if order.refund_status == status {
            return Ok(order);
        }
        let allowed = match order.refund_status {
            RefundStatus::NotRefunded => {
                matches!(
                    status,
                    RefundStatus::PartiallyRefunded | RefundStatus::Refunded
                )
            }
            RefundStatus::PartiallyRefunded => matches!(status, RefundStatus::Refunded),
            RefundStatus::Refunded => false,
        };
        if !allowed {
            return Err(AppError::conflict(format!(
                "cannot transition refund status from {} to {}",
                order.refund_status.as_str(),
                status.as_str()
            )));
        }
        let updated = sqlx::query(
            "UPDATE orders SET refund_status = ?, updated_at = ? WHERE id = ? AND refund_status = ?",
        )
        .bind(status.as_str())
        .bind(now_timestamp())
        .bind(id.as_str())
        .bind(order.refund_status.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "order refund status changed concurrently",
            ));
        }
        self.get(id).await
    }

    /// Reverts the order-side refund a provider later reported as failed.
    ///
    /// The payment service emits `payment.refund_failed` after a refund it
    /// previously recorded is rejected. This finds the newest local refund row
    /// matching the order, payment and amount that is not already failed, marks
    /// it failed and recomputes the order payment status from the remaining
    /// successful refunds.
    ///
    /// Returns `true` when a row was reverted and `false` when nothing matched,
    /// which makes a replayed event an idempotent no-op.
    pub async fn mark_payment_refund_failed(
        &self,
        order_id: &OrderId,
        payment_id: &str,
        amount: i64,
    ) -> Result<bool, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let refund_id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM order_refunds
             WHERE order_id = ? AND payment_id = ? AND amount = ? AND status != 'failed'
             ORDER BY created_at DESC, rowid DESC
             LIMIT 1",
        )
        .bind(order_id.as_str())
        .bind(payment_id)
        .bind(amount)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        let Some(refund_id) = refund_id else {
            tx.commit().await.map_err(database_error)?;
            return Ok(false);
        };

        sqlx::query("UPDATE order_refunds SET status = 'failed', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(&refund_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;

        let total: i64 = sqlx::query_scalar("SELECT total FROM orders WHERE id = ?")
            .bind(order_id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("order", order_id))?;
        let refunded: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM order_refunds
             WHERE order_id = ? AND status != 'failed'",
        )
        .bind(order_id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        let payment_status = if refunded >= total {
            PaymentStatus::Refunded
        } else if refunded > 0 {
            PaymentStatus::PartiallyRefunded
        } else {
            PaymentStatus::Captured
        };
        sqlx::query("UPDATE orders SET payment_status = ?, updated_at = ? WHERE id = ?")
            .bind(payment_status.as_str())
            .bind(now)
            .bind(order_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(true)
    }

    /// Records a refund and updates refund and payment status.
    ///
    /// Refunds are cumulative: the sum of all refunds may never exceed the
    /// order total. When the cumulative refund reaches the order total the
    /// order is marked refunded, otherwise partially refunded.
    pub async fn add_refund(&self, id: &OrderId, input: RefundInput) -> Result<Order, AppError> {
        let (order, _) = self.add_refund_inner(id, input, None, true, None).await?;
        Ok(order)
    }

    /// Records an order refund exactly once for a caller-supplied key.
    ///
    /// A replay with the same key returns the previously recorded refund and
    /// the current order without writing a second refund row. The key is
    /// scoped to order refunds so after-sales workflows can safely retry.
    pub async fn add_refund_idempotent(
        &self,
        id: &OrderId,
        input: RefundInput,
        idempotency_key: &str,
    ) -> Result<(Order, RefundId), AppError> {
        self.add_refund_idempotent_capped(id, input, idempotency_key, None)
            .await
    }

    /// Records an order refund exactly once, capped by `refundable_ceiling`
    /// instead of the order's current total.
    ///
    /// An order edit can lower `orders.total` below what the customer actually
    /// paid, so the total is no longer a usable ceiling for the money side:
    /// the caller that knows the captured amount supplies it. `None` keeps the
    /// order-total rule [`Self::add_refund`] uses.
    pub async fn add_refund_idempotent_capped(
        &self,
        id: &OrderId,
        input: RefundInput,
        idempotency_key: &str,
        refundable_ceiling: Option<i64>,
    ) -> Result<(Order, RefundId), AppError> {
        let key = idempotency_key.trim();
        if key.is_empty() {
            return Err(AppError::validation(
                "refund idempotency_key must not be empty",
            ));
        }
        if let Some(ceiling) = refundable_ceiling {
            validate_non_negative_amount(ceiling, "refundable total")?;
        }
        self.add_refund_inner(id, input, Some(key), false, refundable_ceiling)
            .await
    }

    async fn add_refund_inner(
        &self,
        id: &OrderId,
        input: RefundInput,
        idempotency_key: Option<&str>,
        audit: bool,
        refundable_ceiling: Option<i64>,
    ) -> Result<(Order, RefundId), AppError> {
        if input.amount <= 0 {
            return Err(AppError::validation(
                "refund amount must be greater than zero".to_owned(),
            ));
        }
        let currency = CurrencyCode::new(&input.currency)?.as_str().to_owned();
        let order = self.load_order(id).await?;
        if currency != order.currency_code {
            return Err(AppError::validation(format!(
                "refund currency {currency} does not match order currency {}",
                order.currency_code
            )));
        }

        let mut tx = self.db.begin().await.map_err(AppError::from)?;

        if let Some(key) = idempotency_key {
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT resource_id FROM idempotency_keys WHERE key = ? AND scope = ?",
            )
            .bind(key)
            .bind(REFUND_IDEMPOTENCY_SCOPE)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            if let Some(existing) = existing {
                let refund_id = RefundId::from_string(existing);
                tx.commit().await.map_err(database_error)?;
                let order = self.load_order(id).await?;
                return Ok((order, refund_id));
            }
        }

        // Failed rows hold no money, so they must not count against the
        // ceiling; `mark_payment_refund_failed` keeps them for the audit trail.
        let refunded: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM order_refunds
             WHERE order_id = ? AND status != 'failed'",
        )
        .bind(id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        let total_refunded = refunded
            .checked_add(input.amount)
            .ok_or_else(|| AppError::validation("refund amount overflow"))?;
        // The order total is the normal ceiling. A caller that refunds money
        // the order edit lowered the total below passes what was captured
        // instead, because the customer still paid that amount.
        let refundable = refundable_ceiling.unwrap_or(order.total);
        if total_refunded > refundable {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(format!(
                "refund of {} exceeds the refundable order total of {refundable}",
                input.amount
            )));
        }
        let refund_status = if total_refunded >= refundable {
            RefundStatus::Refunded
        } else {
            RefundStatus::PartiallyRefunded
        };
        let payment_status = if total_refunded >= refundable {
            PaymentStatus::Refunded
        } else {
            PaymentStatus::PartiallyRefunded
        };

        let refund_id = RefundId::new();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO order_refunds
                (id, order_id, payment_id, amount, currency_code, status, reason,
                 provider_refund_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'pending', ?, NULL, ?, ?)",
        )
        .bind(refund_id.as_str())
        .bind(id.as_str())
        .bind(input.payment_id.as_str())
        .bind(input.amount)
        .bind(&currency)
        .bind(input.reason.as_deref())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        let updated = sqlx::query(
            "UPDATE orders SET refund_status = ?, payment_status = ?, updated_at = ?
             WHERE id = ? AND refund_status = ? AND payment_status = ?",
        )
        .bind(refund_status.as_str())
        .bind(payment_status.as_str())
        .bind(now)
        .bind(id.as_str())
        .bind(order.refund_status.as_str())
        .bind(order.payment_status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(
                "order refund status changed concurrently",
            ));
        }

        insert_history(
            &mut tx,
            id,
            order.status,
            Some(order.status),
            Some("refund recorded"),
            now,
        )
        .await?;

        if let Some(key) = idempotency_key {
            let result = sqlx::query(
                "INSERT INTO idempotency_keys (key, scope, resource_id, response, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(key)
            .bind(REFUND_IDEMPOTENCY_SCOPE)
            .bind(refund_id.as_str())
            .bind(serde_json::json!({"refund_id": refund_id.as_str()}).to_string())
            .bind(now)
            .execute(&mut *tx)
            .await;
            if let Err(error) = result {
                if is_unique_violation(&error) {
                    tx.rollback().await.map_err(database_error)?;
                    if let Some(existing) = self.find_refund_id_by_key(key).await? {
                        let order = self.load_order(id).await?;
                        return Ok((order, existing));
                    }
                    return Err(AppError::conflict(
                        "refund idempotency key was used concurrently".to_owned(),
                    ));
                }
                return Err(database_error(error));
            }
        }

        let event = DomainEventEnvelope::new(
            "order.refunded",
            Some(self.db.store_id().clone()),
            serde_json::json!({
                "order_id": id.as_str(),
                "payment_id": input.payment_id.as_str(),
                "refund_id": refund_id.as_str(),
                "amount": input.amount,
                "currency_code": currency,
            }),
        );
        append_outbox_event(&mut *tx, &event)
            .await
            .map_err(database_error)?;
        if audit {
            self.append_audit(
                &mut tx,
                "order.refunded",
                "order",
                id.as_str(),
                serde_json::json!({
                    "payment_id": input.payment_id,
                    "amount": input.amount,
                    "currency_code": order.currency_code,
                }),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;

        let order = self.load_order(id).await?;
        Ok((order, refund_id))
    }

    /// Returns the refund previously recorded for an order-refund key.
    ///
    /// After-sales workflows use this after a crash between recording the
    /// refund and linking it to a return or claim, so the retry can complete
    /// the link without attempting another refund.
    /// Backfills order-refund ledger rows for provider refunds that succeeded
    /// without a corresponding local row.
    ///
    /// The admin refund path moves provider money first and records the local
    /// row second. A crash in between leaves the payment ledger ahead of the
    /// order ledger; this reconciliation repairs that gap idempotently.
    pub async fn reconcile_provider_refunds(&self) -> Result<u64, AppError> {
        let rows = sqlx::query_as::<_, (String, String, i64, String, Option<String>)>(
            "SELECT r.id, r.order_id, r.amount, r.currency_code, r.provider_refund_id \
             FROM refunds r \
             WHERE r.order_id IS NOT NULL AND r.status = 'succeeded' \
               AND NOT EXISTS (\
                   SELECT 1 FROM order_refunds o \
                   WHERE o.order_id = r.order_id AND o.payment_id IS r.payment_id \
                     AND o.amount = r.amount AND o.status != 'failed'\
               )",
        )
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        if rows.is_empty() {
            return Ok(0);
        }

        let now = now_timestamp();
        let mut reconciled = 0_u64;
        for (provider_refund_id, order_id, amount, currency_code, provider_id) in rows {
            let Some(total): Option<i64> =
                sqlx::query_scalar("SELECT total FROM orders WHERE id = ?")
                    .bind(&order_id)
                    .fetch_optional(self.db.pool())
                    .await
                    .map_err(database_error)?
            else {
                continue;
            };
            let mut tx = self.db.begin().await.map_err(AppError::from)?;
            let inserted = sqlx::query(
                "INSERT INTO order_refunds \
                 (id, order_id, payment_id, amount, currency_code, status, reason, \
                  provider_refund_id, created_at, updated_at) \
                 VALUES (?, ?, NULL, ?, ?, 'succeeded', 'reconciled provider refund', ?, ?, ?)",
            )
            .bind(RefundId::new().as_str())
            .bind(&order_id)
            .bind(amount)
            .bind(&currency_code)
            .bind(
                provider_id
                    .as_deref()
                    .unwrap_or(provider_refund_id.as_str()),
            )
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if inserted.rows_affected() == 0 {
                tx.rollback().await.map_err(database_error)?;
                continue;
            }
            let refunded: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount), 0) FROM order_refunds \
                 WHERE order_id = ? AND status != 'failed'",
            )
            .bind(&order_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
            let (refund_status, payment_status) = if refunded >= total {
                (RefundStatus::Refunded, PaymentStatus::Refunded)
            } else {
                (
                    RefundStatus::PartiallyRefunded,
                    PaymentStatus::PartiallyRefunded,
                )
            };
            sqlx::query(
                "UPDATE orders SET refund_status = ?, payment_status = ?, updated_at = ? \
                 WHERE id = ?",
            )
            .bind(refund_status.as_str())
            .bind(payment_status.as_str())
            .bind(now)
            .bind(&order_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            tx.commit().await.map_err(database_error)?;
            reconciled += 1;
        }
        Ok(reconciled)
    }

    pub async fn refund_id_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<RefundId>, AppError> {
        self.find_refund_id_by_key(idempotency_key.trim()).await
    }

    async fn find_refund_id_by_key(&self, key: &str) -> Result<Option<RefundId>, AppError> {
        let resource = sqlx::query_scalar::<_, String>(
            "SELECT resource_id FROM idempotency_keys WHERE key = ? AND scope = ?",
        )
        .bind(key)
        .bind(REFUND_IDEMPOTENCY_SCOPE)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(resource.map(RefundId::from_string))
    }
}
