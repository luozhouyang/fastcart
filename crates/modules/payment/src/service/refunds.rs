//! Refunds and refund idempotency.

//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use fastcart_infrastructure::{append_outbox_event, database_error};
use fastcart_kernel::{AppError, DomainEventEnvelope, OrderId, PaymentId, RefundId, now_timestamp};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, Transaction};

use crate::domain::{PaymentStatus, Refund, RefundStatus};
use crate::provider::{PaymentError, RefundRequest};

use super::store::*;
use super::*;
/// Derives the deterministic provider idempotency key used by keyless refunds.
///
/// The reason is trimmed and blank reasons normalize to `None`, so a retry that
/// only changes surrounding whitespace still maps to the same refund. The
/// length-prefixed fields keep the digest unambiguous for arbitrary reasons.
fn keyless_refund_idempotency_key(payment_id: &str, amount: i64, reason: Option<&str>) -> String {
    let reason = reason.map(str::trim).filter(|reason| !reason.is_empty());
    let mut hasher = Sha256::new();
    hasher.update(b"fastcart.refund.v1\0");
    hasher.update((payment_id.len() as u64).to_be_bytes());
    hasher.update(payment_id.as_bytes());
    hasher.update(amount.to_be_bytes());
    match reason {
        Some(reason) => {
            hasher.update([1]);
            hasher.update((reason.len() as u64).to_be_bytes());
            hasher.update(reason.as_bytes());
        }
        None => hasher.update([0]),
    }
    format!("refund:auto:{}", hex::encode(hasher.finalize()))
}

/// Records a durable refund claim (timestamp and attempt) separately from the
/// provider result.
async fn record_refund_claim(
    tx: &mut Transaction<'_, Sqlite>,
    payment_id: &str,
    provider_key: &str,
    refund_id: &str,
    claimed_at: i64,
) -> Result<(), AppError> {
    let key = format!("payment-refund-claim:{payment_id}:{provider_key}");
    let previous: Option<String> =
        sqlx::query_scalar("SELECT response FROM idempotency_keys WHERE key = ?")
            .bind(&key)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?;
    let attempt = previous
        .as_deref()
        .and_then(|response| serde_json::from_str::<Value>(response).ok())
        .and_then(|response| response.get("attempt").and_then(Value::as_i64))
        .unwrap_or(0)
        + 1;
    let response = serde_json::json!({
        "refund_id": refund_id,
        "claimed_at": claimed_at,
        "attempt": attempt,
    })
    .to_string();
    sqlx::query(
        "INSERT INTO idempotency_keys (key, scope, resource_id, response, created_at) \
         VALUES (?, 'payment.refund_claim', ?, ?, ?) \
         ON CONFLICT(key) DO UPDATE SET resource_id = excluded.resource_id, \
             response = excluded.response",
    )
    .bind(&key)
    .bind(refund_id)
    .bind(response)
    .bind(claimed_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

impl PaymentService {
    /// Refunds part or all of a captured payment.
    ///
    /// Keyless calls derive a deterministic idempotency key from the payment,
    /// amount and trimmed reason, so an identical retry returns the existing
    /// refund instead of refunding the provider twice.
    pub async fn refund(
        &self,
        payment_id: &str,
        amount: i64,
        reason: Option<String>,
    ) -> Result<Refund, AppError> {
        let idempotency_key = keyless_refund_idempotency_key(payment_id, amount, reason.as_deref());
        self.refund_with_idempotency(payment_id, amount, reason, Some(idempotency_key))
            .await
    }

    /// Refunds part or all of a captured payment with an optional provider
    /// idempotency key.
    ///
    /// When no key is supplied, one is derived deterministically from the
    /// payment, amount and trimmed reason. Callers that need two identical
    /// refunds must pass distinct explicit keys.
    ///
    /// A refund on a payment linked to an order first credits the order's gift
    /// card redemptions, oldest first, up to the refund amount; only the
    /// remainder is sent to the payment provider. The credits are written in
    /// the claim transaction, before the provider call, so the ledger and the
    /// refund row can never disagree about which part of a redemption is still
    /// refundable: `BEGIN IMMEDIATE` serializes concurrent claims and each one
    /// sums the credits earlier refunds already wrote. A retry of a pending or
    /// failed refund takes its own earlier credits back and re-applies them, so
    /// replaying a key never credits anything twice.
    pub async fn refund_with_idempotency(
        &self,
        payment_id: &str,
        amount: i64,
        reason: Option<String>,
        idempotency_key: Option<String>,
    ) -> Result<Refund, AppError> {
        if amount <= 0 {
            return Err(AppError::validation("refund amount must be positive"));
        }
        let payment = self.get_payment(payment_id).await?;
        let idempotency_key = idempotency_key
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
            .unwrap_or_else(|| {
                keyless_refund_idempotency_key(payment_id, amount, reason.as_deref())
            });

        // The existing-key lookup, the claim, the amount cap and the gift card
        // allocation share one BEGIN IMMEDIATE transaction. Concurrent callers
        // with the same key therefore observe the winning row instead of a
        // unique violation. The lookup happens before state validation so a
        // duplicate request still returns the winning refund after a full
        // refund.
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let existing = sqlx::query_as::<_, RefundRow>(
            "SELECT id, payment_id, order_id, amount, currency_code, status,
                    provider_refund_id, idempotency_key, reason, created_at, updated_at
             FROM refunds WHERE payment_id = ? AND idempotency_key = ?",
        )
        .bind(payment.id.as_str())
        .bind(&idempotency_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        if let Some(existing) = existing.as_ref()
            && RefundStatus::parse(&existing.status) == Some(RefundStatus::Succeeded)
        {
            let refund = refund_from_row(existing.clone());
            tx.commit().await.map_err(database_error)?;
            return Ok(refund);
        }
        // A partially captured payment holds captured money too, so it is
        // refundable up to the amount actually captured.
        if !matches!(
            payment.status,
            PaymentStatus::Captured
                | PaymentStatus::PartiallyCaptured
                | PaymentStatus::PartiallyRefunded
        ) {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(format!(
                "cannot refund a payment in status {}",
                payment.status
            )));
        }

        let refund_id = existing
            .as_ref()
            .map_or_else(RefundId::new, |row| RefundId::from_string(row.id.clone()));
        let provider_key = idempotency_key.clone();
        let now = now_timestamp();

        let current_total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM refunds
             WHERE payment_id = ? AND status != 'failed' AND id != ?",
        )
        .bind(payment.id.as_str())
        .bind(refund_id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        let total_after = current_total
            .checked_add(amount)
            .ok_or_else(|| AppError::conflict("refund amount overflow"))?;
        if total_after > payment.amount {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(
                "refund amount exceeds the remaining captured amount",
            ));
        }
        let next_payment_status = if total_after >= payment.amount {
            PaymentStatus::Refunded
        } else {
            PaymentStatus::PartiallyRefunded
        };
        if !payment.status.can_transition_to(next_payment_status) {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(format!(
                "cannot transition a payment from {} to {next_payment_status}",
                payment.status
            )));
        }
        let pending = Refund {
            id: refund_id.clone(),
            payment_id: Some(payment.id.clone()),
            order_id: payment.order_id.clone(),
            amount,
            currency_code: payment.currency_code.clone(),
            status: RefundStatus::Pending,
            provider_refund_id: None,
            idempotency_key: Some(provider_key.clone()),
            reason: reason.clone(),
            created_at: existing.as_ref().map_or(now, |row| row.created_at),
            updated_at: now,
        };
        if existing.is_some() {
            // A retry of a pending or failed row replaces the claim while
            // keeping the caller's idempotency key and provider reference
            // space free for the new attempt.
            sqlx::query(
                "UPDATE refunds SET amount = ?, status = 'pending', provider_refund_id = NULL,
                 reason = ?, updated_at = ? WHERE id = ?",
            )
            .bind(amount)
            .bind(reason.as_deref())
            .bind(now)
            .bind(refund_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        } else {
            insert_refund(&mut tx, &pending).await?;
        }
        // The claim is durable and separate from the provider result so a
        // crashed caller can be observed and a retry re-invoked.
        record_refund_claim(
            &mut tx,
            payment.id.as_str(),
            &provider_key,
            refund_id.as_str(),
            now,
        )
        .await?;

        // Gift card money is restored inside the claim transaction, before the
        // provider is asked for the uncredited remainder. This ordering means
        // a definitive provider rejection has to take the credits back (see
        // `fail_refund`), while an ambiguous outcome keeps them: the refund
        // stays pending and its retry finds the coverage already recorded.
        // A retry removes its own earlier credits first so the allocation is
        // recomputed against every other refund's ledger, netting to the same
        // cards for an unchanged amount.
        if existing.is_some() {
            self.reverse_refund_gift_card_credits(&mut tx, refund_id.as_str())
                .await?;
        }
        let gift_card_refunded = self
            .allocate_refund_to_gift_cards(
                &mut tx,
                refund_id.as_str(),
                payment.order_id.as_ref(),
                amount,
            )
            .await?;
        let provider_refunded = amount - gift_card_refunded;
        // The provider is needed only for the remainder. Resolving it before
        // the commit keeps a missing provider or payment reference from
        // leaving credits behind for a refund that cannot be attempted.
        let provider_call = if provider_refunded > 0 {
            let provider = self
                .providers
                .get(&payment.provider)
                .ok_or_else(|| AppError::not_found("payment provider", &payment.provider))?;
            let provider_payment_id = payment
                .provider_payment_id
                .clone()
                .ok_or_else(|| AppError::conflict("payment is missing a provider payment id"))?;
            Some((provider, provider_payment_id))
        } else {
            None
        };
        tx.commit().await.map_err(database_error)?;

        let Some((provider, provider_payment_id)) = provider_call else {
            // The card credits covered the whole refund: no provider call is
            // made and the refund succeeds with no provider reference. Its
            // amount stays the full refunded amount.
            let refund = Refund {
                id: refund_id,
                payment_id: Some(payment.id.clone()),
                order_id: payment.order_id.clone(),
                amount,
                currency_code: payment.currency_code.clone(),
                status: RefundStatus::Succeeded,
                provider_refund_id: None,
                idempotency_key: pending.idempotency_key,
                reason,
                created_at: pending.created_at,
                updated_at: now_timestamp(),
            };
            return self.settle_refund(refund, gift_card_refunded).await;
        };

        // Provider calls always happen outside the transaction. Only the
        // remainder reached the provider, and the provider key is
        // deterministic, so re-invoking after a crash reconciles the same
        // provider-side refund.
        let result = provider
            .refund(RefundRequest {
                provider_payment_id,
                amount: provider_refunded,
                currency_code: payment.currency_code.clone(),
                reason: reason.clone(),
                idempotency_key: Some(provider_key),
                metadata: payment.metadata.clone(),
            })
            .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                // `PaymentError::Provider` covers both a provider rejection
                // and a transport failure where the provider may already have
                // accepted the refund. Only `InvalidRequest` is known to have
                // been rejected without side effects, so only then are the
                // gift card credits taken back. An unknown outcome stays
                // `pending`, which keeps it counted by the over-refund cap
                // above until a webhook or retry reconciles it.
                if matches!(error, PaymentError::InvalidRequest(_)) {
                    self.fail_refund(refund_id.as_str()).await?;
                }
                return Err(error.into());
            }
        };

        let refund = Refund {
            id: refund_id.clone(),
            payment_id: Some(payment.id.clone()),
            order_id: payment.order_id.clone(),
            amount,
            currency_code: payment.currency_code.clone(),
            status: result.status,
            provider_refund_id: Some(result.provider_refund_id),
            idempotency_key: pending.idempotency_key,
            reason,
            created_at: pending.created_at,
            updated_at: now_timestamp(),
        };
        self.settle_refund(refund, gift_card_refunded).await
    }

    /// Marks a definitively rejected refund failed and takes its card credits back.
    ///
    /// The row update and the reversal share one transaction, and the update
    /// runs first: a caller whose refund was already reconciled to a success
    /// by a concurrent request must not reverse credits that now belong to it.
    async fn fail_refund(&self, refund_id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE refunds SET status = 'failed', updated_at = ?
             WHERE id = ? AND status IN ('pending', 'failed')",
        )
        .bind(now_timestamp())
        .bind(refund_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() > 0 {
            self.reverse_refund_gift_card_credits(&mut tx, refund_id)
                .await?;
        }
        tx.commit().await.map_err(database_error)
    }

    /// Persists the final state of a refund, its payment and its event.
    ///
    /// `gift_card_refunded` is the part already credited to cards by the claim
    /// transaction; the event records it so consumers can tell where the money
    /// went. A failed outcome reverses those credits in the same transaction,
    /// before the payment status is recomputed, so a refund the provider
    /// rejected leaves no money on a card. When a concurrent caller or a
    /// verified webhook already reconciled the row, the persisted refund is
    /// returned without a second event or reversal.
    async fn settle_refund(
        &self,
        refund: Refund,
        gift_card_refunded: i64,
    ) -> Result<Refund, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE refunds SET status = ?, provider_refund_id = ?, updated_at = ?
             WHERE id = ? AND status IN ('pending', 'failed')",
        )
        .bind(refund.status.as_str())
        .bind(refund.provider_refund_id.as_deref())
        .bind(refund.updated_at)
        .bind(refund.id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            // A concurrent caller or a verified webhook already reconciled
            // this refund; return the persisted result.
            let row = sqlx::query_as::<_, RefundRow>(
                "SELECT id, payment_id, order_id, amount, currency_code, status,
                        provider_refund_id, idempotency_key, reason, created_at, updated_at
                 FROM refunds WHERE id = ?",
            )
            .bind(refund.id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            tx.commit().await.map_err(database_error)?;
            return row
                .map(refund_from_row)
                .ok_or_else(|| AppError::not_found("refund", refund.id.as_str()));
        }
        if refund.status == RefundStatus::Failed {
            // The provider rejected the refund: the cards must not keep money
            // the refund never returned.
            self.reverse_refund_gift_card_credits(&mut tx, refund.id.as_str())
                .await?;
        }
        // Recompute the payment status from the persisted refunds so a
        // provider-reported failure cannot leave the payment over-refunded.
        let refunded: i64 = if let Some(payment_id) = refund.payment_id.as_ref() {
            sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount), 0) FROM refunds
                 WHERE payment_id = ? AND status != 'failed'",
            )
            .bind(payment_id.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?
        } else {
            0
        };
        if let Some(payment_id) = refund.payment_id.as_ref() {
            let payment_amount: Option<i64> =
                sqlx::query_scalar("SELECT amount FROM payments WHERE id = ?")
                    .bind(payment_id.as_str())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(database_error)?;
            let reconciled_payment_status = match payment_amount {
                Some(amount) if refunded >= amount => PaymentStatus::Refunded,
                Some(_) if refunded > 0 => PaymentStatus::PartiallyRefunded,
                _ => PaymentStatus::Captured,
            };
            sqlx::query(
                "UPDATE payments SET status = ?, updated_at = ? WHERE id = ?
                 AND status IN ('partially_captured', 'captured', 'partially_refunded', 'refunded')",
            )
            .bind(reconciled_payment_status.as_str())
            .bind(refund.updated_at)
            .bind(payment_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }
        let event = DomainEventEnvelope::new(
            if refund.status == RefundStatus::Failed {
                "payment.refund_failed"
            } else {
                "payment.refunded"
            },
            Some(self.db.store_id().clone()),
            serde_json::json!({
                "payment_id": refund.payment_id.as_ref().map(PaymentId::as_str),
                "order_id": refund.order_id.as_ref().map(OrderId::as_str),
                "refund_id": refund.id.as_str(),
                "amount": refund.amount,
                "currency_code": refund.currency_code,
                "status": refund.status.as_str(),
                "gift_card_refunded": gift_card_refunded,
                "provider_refunded": if refund.status == RefundStatus::Failed {
                    0
                } else {
                    refund.amount - gift_card_refunded
                },
            }),
        );
        append_outbox_event(&mut *tx, &event)
            .await
            .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        if refund.status == RefundStatus::Failed {
            return Err(AppError::ExternalService(
                "payment provider rejected the refund".to_owned(),
            ));
        }
        Ok(refund)
    }

    /// Lists refunds for a payment.
    pub async fn list_refunds(&self, payment_id: &str) -> Result<Vec<Refund>, AppError> {
        let rows = sqlx::query_as::<_, RefundRow>(
            "SELECT id, payment_id, order_id, amount, currency_code, status, \
             provider_refund_id, idempotency_key, reason, created_at, updated_at \
             FROM refunds WHERE payment_id = ? ORDER BY created_at ASC, id ASC",
        )
        .bind(payment_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows.into_iter().map(refund_from_row).collect())
    }
}
