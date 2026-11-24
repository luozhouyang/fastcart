//! Provider webhook ingestion and application.

//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use std::collections::BTreeMap;

use fastcart_infrastructure::{append_outbox_event, database_error};
use fastcart_kernel::{
    AppError, DomainEventEnvelope, OrderId, PageResult, PaymentAttemptId, PaymentId, WebhookId,
    now_timestamp,
};
use serde_json::Value;
use sqlx::{QueryBuilder, Sqlite, Transaction};

use crate::domain::{Payment, PaymentAttempt, PaymentStatus, PaymentWebhookRecord, WebhookStatus};
use crate::provider::VerifiedWebhook;

use super::store::*;
use super::*;
fn string_field(data: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        data.get(*key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

/// Wraps a webhook apply failure so the provider redelivers the event.
fn retryable_webhook_error(error: &AppError) -> AppError {
    AppError::ExternalService(format!("payment webhook processing failed: {error}"))
}

/// Resolves the order, amount and currency a webhook event applies to.
///
/// Provider events identify the payment or session, not the order, so the
/// association is read back after applying the state transition.
async fn payment_order_target(
    tx: &mut Transaction<'_, Sqlite>,
    provider: &str,
    provider_payment_id: Option<&str>,
    provider_session_id: Option<&str>,
) -> Result<Option<(OrderId, i64, String)>, AppError> {
    if let Some(provider_payment_id) = provider_payment_id {
        let row = sqlx::query_as::<_, (Option<String>, i64, String)>(
            "SELECT order_id, amount, currency_code FROM payments \
             WHERE provider = ? AND provider_payment_id = ? LIMIT 1",
        )
        .bind(provider)
        .bind(provider_payment_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;
        if let Some((Some(order_id), amount, currency_code)) = row {
            return Ok(Some((
                OrderId::from_string(order_id),
                amount,
                currency_code,
            )));
        }
    }
    if let Some(provider_session_id) = provider_session_id {
        let row = sqlx::query_as::<_, (Option<String>, i64, String)>(
            "SELECT order_id, amount, currency_code FROM payment_sessions \
             WHERE provider = ? AND provider_session_id = ? LIMIT 1",
        )
        .bind(provider)
        .bind(provider_session_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;
        if let Some((Some(order_id), amount, currency_code)) = row {
            return Ok(Some((
                OrderId::from_string(order_id),
                amount,
                currency_code,
            )));
        }
    }
    Ok(None)
}

impl PaymentService {
    /// Verifies and persists a webhook event, applying known event types.
    ///
    /// Duplicate `(provider, provider_event_id)` deliveries return the existing
    /// processed record without reprocessing. A duplicate whose row is still
    /// `received` or `failed` is re-applied, because the first delivery may
    /// have crashed after inserting the row. Processing failures are stored on
    /// the record and returned as a retryable error so the provider redelivers
    /// the event instead of it being silently dropped.
    pub async fn process_webhook(
        &self,
        provider_id: &str,
        headers: BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<PaymentWebhookRecord, AppError> {
        let provider = self
            .providers
            .get(provider_id)
            .ok_or_else(|| AppError::not_found("payment provider", provider_id))?;
        let event = provider.verify_webhook(&headers, body)?;

        if let Some(existing) = self.find_webhook(provider_id, &event.event_id).await? {
            return self.apply_existing_webhook(existing, &event).await;
        }

        let id = WebhookId::new();
        let now = now_timestamp();
        let payload = serde_json::to_string(&event.data).unwrap_or_else(|_| "{}".to_owned());
        let inserted = sqlx::query(
            "INSERT INTO payment_webhooks \
             (id, provider, provider_event_id, event_type, payload, status, attempts, \
              processed_at, last_error, created_at) \
             VALUES (?, ?, ?, ?, ?, 'received', 0, NULL, NULL, ?)",
        )
        .bind(id.as_str())
        .bind(provider_id)
        .bind(&event.event_id)
        .bind(&event.event_type)
        .bind(payload)
        .bind(now)
        .execute(self.db.pool())
        .await;
        if let Err(error) = inserted {
            if error
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
                && let Some(existing) = self.find_webhook(provider_id, &event.event_id).await?
            {
                // Another delivery inserted the event after our lookup but
                // before this insert. It may not have applied the event yet,
                // or may have crashed after the insert, so recover instead of
                // acknowledging an unapplied event.
                return self.apply_existing_webhook(existing, &event).await;
            }
            return Err(database_error(error));
        }

        let record = self.get_webhook(id.as_str()).await?;
        self.apply_existing_webhook(record, &event).await
    }

    /// Applies a verified event whose webhook row already exists.
    ///
    /// A unique-conflict loser can observe the row a concurrent winner
    /// inserted before the winner applies it (or after the winner crashed
    /// following the insert). Returning the unprocessed row would ACK an event
    /// that was never applied, so every delivery re-applies non-processed
    /// rows. `apply_webhook_event` is idempotent because each state write is
    /// guarded by the expected source status, making concurrent recovery safe.
    async fn apply_existing_webhook(
        &self,
        existing: PaymentWebhookRecord,
        event: &VerifiedWebhook,
    ) -> Result<PaymentWebhookRecord, AppError> {
        if existing.status == WebhookStatus::Processed {
            return Ok(existing);
        }
        let applied = self
            .apply_webhook_event(existing.provider.as_str(), event)
            .await;
        let (status, last_error) = match &applied {
            Ok(()) => (WebhookStatus::Processed, None),
            Err(error) => (WebhookStatus::Failed, Some(error.to_string())),
        };
        // Do not downgrade a row another delivery already marked processed.
        sqlx::query(
            "UPDATE payment_webhooks
             SET status = ?, attempts = attempts + 1, processed_at = ?, last_error = ?
             WHERE id = ? AND status != 'processed'",
        )
        .bind(status.as_str())
        .bind(if status == WebhookStatus::Processed {
            Some(now_timestamp())
        } else {
            None
        })
        .bind(last_error)
        .bind(existing.id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if let Err(error) = applied {
            return Err(retryable_webhook_error(&error));
        }
        self.get_webhook(existing.id.as_str()).await
    }

    /// Lists webhook records, optionally filtered by provider.
    pub async fn list_webhooks(
        &self,
        provider: Option<&str>,
        pagination: fastcart_kernel::Pagination,
    ) -> Result<PageResult<PaymentWebhookRecord>, AppError> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, provider, provider_event_id, event_type, payload, status, \
             attempts, processed_at, last_error, created_at \
             FROM payment_webhooks WHERE 1 = 1",
        );
        let mut count_query =
            QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM payment_webhooks WHERE 1 = 1");
        if let Some(provider) = provider {
            query.push(" AND provider = ").push_bind(provider);
            count_query.push(" AND provider = ").push_bind(provider);
        }
        query
            .push(" ORDER BY created_at DESC, id DESC LIMIT ")
            .push_bind(pagination.limit)
            .push(" OFFSET ")
            .push_bind(pagination.offset);
        let rows: Vec<PaymentWebhookRow> = query
            .build_query_as()
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let count: i64 = count_query
            .build_query_scalar()
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(PageResult::new(
            rows.into_iter().map(webhook_from_row).collect(),
            count,
            pagination,
        ))
    }

    /// Loads a webhook record.
    pub async fn get_webhook(&self, id: &str) -> Result<PaymentWebhookRecord, AppError> {
        let row = sqlx::query_as::<_, PaymentWebhookRow>(
            "SELECT id, provider, provider_event_id, event_type, payload, status, \
             attempts, processed_at, last_error, created_at \
             FROM payment_webhooks WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        row.map(webhook_from_row)
            .ok_or_else(|| AppError::not_found("payment webhook", id))
    }

    async fn find_webhook(
        &self,
        provider: &str,
        event_id: &str,
    ) -> Result<Option<PaymentWebhookRecord>, AppError> {
        let row = sqlx::query_as::<_, PaymentWebhookRow>(
            "SELECT id, provider, provider_event_id, event_type, payload, status, \
             attempts, processed_at, last_error, created_at \
             FROM payment_webhooks WHERE provider = ? AND provider_event_id = ?",
        )
        .bind(provider)
        .bind(event_id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(row.map(webhook_from_row))
    }

    async fn apply_webhook_event(
        &self,
        provider: &str,
        event: &VerifiedWebhook,
    ) -> Result<(), AppError> {
        match event.event_type.as_str() {
            "payment.succeeded" | "payment.captured" => {
                self.apply_payment_event(provider, &event.data, true).await
            }
            "payment.failed" => self.apply_payment_event(provider, &event.data, false).await,
            "payment.canceled" => {
                self.apply_payment_canceled_event(provider, &event.data)
                    .await
            }
            "refund.succeeded" => self.apply_refund_event(provider, &event.data, true).await,
            "refund.failed" => self.apply_refund_event(provider, &event.data, false).await,
            _ => Ok(()),
        }
    }

    async fn apply_payment_event(
        &self,
        provider: &str,
        data: &Value,
        succeeded: bool,
    ) -> Result<(), AppError> {
        let provider_payment_id = string_field(data, &["provider_payment_id", "payment_id"]);
        let provider_session_id = string_field(data, &["provider_session_id", "session_id"]);
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let mut transitioned = false;

        // Provider webhooks are the only capture path for asynchronous payment
        // methods. Read the local session first so a successful event can
        // create the payment row that a synchronous capture would have written.
        let session = if let Some(provider_session_id) = provider_session_id.as_deref() {
            sqlx::query_as::<_, PaymentSessionRow>(
                "SELECT id, cart_id, checkout_id, order_id, provider, provider_session_id, \
                 amount, currency_code, status, data, created_at, updated_at \
                 FROM payment_sessions WHERE provider = ? AND provider_session_id = ? LIMIT 1",
            )
            .bind(provider)
            .bind(provider_session_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .map(session_from_row)
        } else {
            None
        };
        let mut payment_id = session
            .as_ref()
            .and_then(|session| session.data.get("payment_id").and_then(Value::as_str))
            .map(str::to_owned);

        if let Some(provider_payment_id) = provider_payment_id.as_deref() {
            let existing = sqlx::query_as::<_, PaymentRow>(
                "SELECT id, order_id, provider, provider_payment_id, amount, currency_code, \
                 status, captured_at, metadata, created_at, updated_at \
                 FROM payments WHERE provider = ? AND provider_payment_id = ? LIMIT 1",
            )
            .bind(provider)
            .bind(provider_payment_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            if succeeded {
                if let Some(existing) = existing {
                    let was_captured =
                        PaymentStatus::parse(&existing.status).is_some_and(|status| {
                            matches!(
                                status,
                                PaymentStatus::PartiallyCaptured
                                    | PaymentStatus::Captured
                                    | PaymentStatus::PartiallyRefunded
                                    | PaymentStatus::Refunded
                            )
                        });
                    if !was_captured {
                        transitioned = true;
                    }
                    payment_id = Some(existing.id.clone());
                    sqlx::query(
                        "UPDATE payments SET status = 'captured', \
                         captured_at = COALESCE(captured_at, ?), updated_at = ? \
                         WHERE id = ? \
                         AND status IN ('pending', 'requires_action', 'authorized', 'failed')",
                    )
                    .bind(now)
                    .bind(now)
                    .bind(&existing.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(database_error)?;
                } else if let Some(session) = session.as_ref() {
                    // Insert once and let the unique provider identifier make
                    // replays a no-op.
                    let payment = Payment {
                        id: PaymentId::new(),
                        order_id: session.order_id.clone(),
                        provider: provider.to_owned(),
                        provider_payment_id: Some(provider_payment_id.to_owned()),
                        amount: session.amount,
                        currency_code: session.currency_code.clone(),
                        status: PaymentStatus::Captured,
                        captured_at: Some(now),
                        metadata: session.data.clone(),
                        created_at: now,
                        updated_at: now,
                    };
                    insert_payment(&mut tx, &payment).await?;
                    insert_attempt(
                        &mut tx,
                        &PaymentAttempt {
                            id: PaymentAttemptId::new(),
                            payment_session_id: session.id.clone(),
                            amount: payment.amount,
                            currency_code: payment.currency_code.clone(),
                            status: PaymentStatus::Captured,
                            provider_reference: Some(provider_payment_id.to_owned()),
                            error: None,
                            created_at: now,
                        },
                    )
                    .await?;
                    payment_id = Some(payment.id.as_str().to_owned());
                    transitioned = true;
                }
            } else {
                let updated = sqlx::query(
                    "UPDATE payments SET status = 'failed', updated_at = ? \
                     WHERE provider = ? AND provider_payment_id = ? \
                     AND status IN ('pending', 'requires_action', 'authorized')",
                )
                .bind(now)
                .bind(provider)
                .bind(provider_payment_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                transitioned |= updated.rows_affected() > 0;
            }
        }
        if let Some(session) = session.as_ref() {
            if succeeded {
                let mut new_data = session.data.clone();
                if let Some(payment_id) = payment_id.as_deref() {
                    new_data.insert("payment_id", Value::String(payment_id.to_owned()));
                }
                new_data.remove("capture_claim");
                new_data.remove("capture_claim_at");
                let updated = sqlx::query(
                    "UPDATE payment_sessions SET status = 'captured', data = ?, updated_at = ? \
                     WHERE id = ? \
                     AND status IN ('pending', 'requires_action', 'authorized', 'failed')",
                )
                .bind(new_data.to_json_string())
                .bind(now)
                .bind(session.id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                transitioned |= updated.rows_affected() > 0;
                if updated.rows_affected() == 0 && new_data != session.data {
                    // The session was captured already but never learned about
                    // the asynchronous payment; reconcile the reference.
                    sqlx::query(
                        "UPDATE payment_sessions SET data = ?, updated_at = ? WHERE id = ? \
                         AND status IN ('captured', 'partially_refunded', 'refunded')",
                    )
                    .bind(new_data.to_json_string())
                    .bind(now)
                    .bind(session.id.as_str())
                    .execute(&mut *tx)
                    .await
                    .map_err(database_error)?;
                }
            } else {
                let updated = sqlx::query(
                    "UPDATE payment_sessions SET status = 'failed', updated_at = ? \
                     WHERE id = ? \
                     AND status IN ('pending', 'requires_action', 'authorized')",
                )
                .bind(now)
                .bind(session.id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                transitioned |= updated.rows_affected() > 0;
            }
        }
        // Emit the same domain event the synchronous capture emits so
        // notifications and outbox consumers observe exactly one transition.
        // When the order is not known yet, `assign_order` emits it later.
        if transitioned
            && let Some((order_id, amount, currency)) = payment_order_target(
                &mut tx,
                provider,
                provider_payment_id.as_deref(),
                provider_session_id.as_deref(),
            )
            .await?
        {
            append_payment_event(
                &mut tx,
                self.db.store_id(),
                if succeeded {
                    "payment.captured"
                } else {
                    "payment.failed"
                },
                &order_id,
                amount,
                &currency,
                provider,
                if succeeded {
                    payment_id.as_deref()
                } else {
                    None
                },
                None,
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    async fn apply_payment_canceled_event(
        &self,
        provider: &str,
        data: &Value,
    ) -> Result<(), AppError> {
        let provider_payment_id = string_field(data, &["provider_payment_id", "payment_id"]);
        let provider_session_id = string_field(data, &["provider_session_id", "session_id"]);
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        if let Some(provider_payment_id) = provider_payment_id.as_deref() {
            sqlx::query(
                "UPDATE payments SET status = 'canceled', updated_at = ? \
                 WHERE provider = ? AND provider_payment_id = ? \
                 AND status IN ('pending', 'requires_action', 'authorized')",
            )
            .bind(now)
            .bind(provider)
            .bind(provider_payment_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }
        if let Some(provider_session_id) = provider_session_id.as_deref() {
            sqlx::query(
                "UPDATE payment_sessions SET status = 'canceled', updated_at = ? \
                 WHERE provider = ? AND provider_session_id = ? \
                 AND status IN ('pending', 'requires_action', 'authorized')",
            )
            .bind(now)
            .bind(provider)
            .bind(provider_session_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    async fn apply_refund_event(
        &self,
        provider: &str,
        data: &Value,
        succeeded: bool,
    ) -> Result<(), AppError> {
        let provider_refund_id = string_field(data, &["provider_refund_id", "refund_id"]);
        let provider_payment_id = string_field(data, &["provider_payment_id", "payment_id"]);
        if provider_refund_id.is_none() && provider_payment_id.is_none() {
            return Ok(());
        }
        let amount_hint = data
            .get("amount")
            .and_then(Value::as_i64)
            .filter(|amount| *amount > 0);
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let mut payment_id: Option<String> = None;
        let mut refund_id: Option<String> = None;
        let mut refund_amount = amount_hint;
        let mut refund_currency: Option<String> = None;
        let mut refund_transitioned = false;

        if let Some(provider_refund_id) = provider_refund_id.as_deref() {
            let row = sqlx::query_as::<_, (String, Option<String>, i64, String)>(
                "SELECT id, payment_id, amount, currency_code FROM refunds \
                 WHERE provider_refund_id = ? LIMIT 1",
            )
            .bind(provider_refund_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            if let Some((id, payment, amount, currency)) = row {
                refund_id = Some(id.clone());
                payment_id = payment;
                refund_amount = Some(amount);
                refund_currency = Some(currency);
                let next = if succeeded { "succeeded" } else { "failed" };
                let updated = sqlx::query(
                    "UPDATE refunds SET status = ?, updated_at = ? \
                     WHERE id = ? AND status != ?",
                )
                .bind(next)
                .bind(now)
                .bind(&id)
                .bind(next)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                refund_transitioned = updated.rows_affected() > 0;
            }
        }

        if payment_id.is_none()
            && let Some(provider_payment_id) = provider_payment_id.as_deref()
        {
            payment_id = sqlx::query_scalar::<_, String>(
                "SELECT id FROM payments WHERE provider = ? AND provider_payment_id = ? \
                     LIMIT 1",
            )
            .bind(provider)
            .bind(provider_payment_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        // A crash can leave a pending local refund without a provider
        // reference. A failure notification can still be matched by payment
        // and amount so the optimistic local refund is reverted.
        if !succeeded
            && refund_id.is_none()
            && let (Some(payment_id), Some(amount)) = (payment_id.as_deref(), amount_hint)
        {
            let row = sqlx::query_as::<_, (String, i64, String)>(
                "SELECT id, amount, currency_code FROM refunds \
                 WHERE payment_id = ? AND amount = ? AND status = 'pending' \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(payment_id)
            .bind(amount)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            if let Some((id, amount, currency)) = row {
                let updated = sqlx::query(
                    "UPDATE refunds SET status = 'failed', updated_at = ? \
                     WHERE id = ? AND status = 'pending'",
                )
                .bind(now)
                .bind(&id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                refund_id = Some(id);
                refund_amount = Some(amount);
                refund_currency = Some(currency);
                refund_transitioned = updated.rows_affected() > 0;
            }
        }

        if let Some(payment_id) = payment_id {
            let payment = sqlx::query_as::<_, (Option<String>, i64, String, String)>(
                "SELECT order_id, amount, currency_code, status FROM payments WHERE id = ?",
            )
            .bind(&payment_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            if let Some((order_id, amount, currency_code, current_status)) = payment {
                let refunded: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(SUM(amount), 0) FROM refunds \
                     WHERE payment_id = ? AND status != 'failed'",
                )
                .bind(&payment_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
                let status = if refunded >= amount {
                    PaymentStatus::Refunded
                } else if refunded > 0 {
                    PaymentStatus::PartiallyRefunded
                } else {
                    // Reverting the last refund restores the captured state;
                    // a partially captured payment stays partially captured.
                    match PaymentStatus::parse(&current_status) {
                        Some(PaymentStatus::PartiallyCaptured) => PaymentStatus::PartiallyCaptured,
                        _ => PaymentStatus::Captured,
                    }
                };
                let updated = sqlx::query(
                    "UPDATE payments SET status = ?, updated_at = ? WHERE id = ? \
                     AND status IN ('partially_captured', 'captured', 'partially_refunded', \
                                    'refunded')",
                )
                .bind(status.as_str())
                .bind(now)
                .bind(&payment_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                if !succeeded
                    && refund_transitioned
                    && updated.rows_affected() > 0
                    && let Some(order_id) = order_id.as_deref().map(OrderId::from_string)
                {
                    // Order-level consumers can revert the refund they
                    // recorded when the provider reported success first.
                    let event = DomainEventEnvelope::new(
                        "payment.refund_failed",
                        Some(self.db.store_id().clone()),
                        serde_json::json!({
                            "payment_id": payment_id,
                            "order_id": order_id.as_str(),
                            "refund_id": refund_id,
                            "amount": refund_amount.unwrap_or(0),
                            "currency_code": refund_currency.unwrap_or(currency_code),
                            "provider": provider,
                        }),
                    );
                    append_outbox_event(&mut *tx, &event)
                        .await
                        .map_err(database_error)?;
                }
            }
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}
