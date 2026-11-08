//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use std::collections::BTreeMap;
use std::sync::Arc;

use fastcart_infrastructure::{StoreDatabase, append_outbox_event, database_error};
use fastcart_kernel::{
    AppError, CartId, CheckoutId, CurrencyCode, DomainEventEnvelope, Metadata, OrderId, PageResult,
    PaymentAttemptId, PaymentId, PaymentSessionId, RefundId, WebhookId, now_timestamp,
    validate_metadata, validate_non_negative_amount,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{QueryBuilder, Sqlite, Transaction};

use crate::domain::{
    CreatePaymentSessionInput, ListPaymentsFilter, Payment, PaymentAttempt, PaymentSession,
    PaymentStatus, PaymentWebhookRecord, Refund, RefundStatus, WebhookStatus,
};
use crate::provider::{
    CaptureRequest, CreateProviderSession, PaymentError, PaymentProvider, ProviderRegistry,
    RefundRequest, VerifiedWebhook,
};

/// Payment application service bound to one store database.
pub struct PaymentService {
    db: StoreDatabase,
    providers: ProviderRegistry,
}

impl PaymentService {
    /// Creates a service with an empty provider registry.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self {
            db,
            providers: ProviderRegistry::new(),
        }
    }

    /// Creates a service with a pre-populated provider registry.
    #[must_use]
    pub fn with_registry(db: StoreDatabase, providers: ProviderRegistry) -> Self {
        Self { db, providers }
    }

    /// Returns the provider registry.
    #[must_use]
    pub fn providers(&self) -> &ProviderRegistry {
        &self.providers
    }

    /// Registers a provider, failing when the identifier is already in use.
    pub fn register_provider(&self, provider: Arc<dyn PaymentProvider>) -> Result<(), AppError> {
        self.providers.register(provider)
    }

    /// Registers or replaces a provider.
    pub fn replace_provider(
        &self,
        provider: Arc<dyn PaymentProvider>,
    ) -> Option<Arc<dyn PaymentProvider>> {
        self.providers.replace(provider)
    }

    /// Creates a provider session and persists the local session and attempt.
    pub async fn create_session(
        &self,
        input: CreatePaymentSessionInput,
    ) -> Result<PaymentSession, AppError> {
        let provider = self
            .providers
            .get(&input.provider)
            .ok_or_else(|| AppError::not_found("payment provider", &input.provider))?;
        validate_non_negative_amount(input.amount, "amount")?;
        validate_metadata(&input.metadata)?;
        let currency = CurrencyCode::new(input.currency_code)?;
        let currency_code = currency.as_str().to_owned();

        let session_id = PaymentSessionId::new();
        // Providers receive the identifiers they need to correlate callbacks
        // with local carts, checkouts and orders.
        let mut provider_metadata = input.metadata.clone();
        provider_metadata.insert("session_id", Value::String(session_id.as_str().to_owned()));
        if let Some(cart_id) = input.cart_id.as_ref() {
            provider_metadata.insert("cart_id", Value::String(cart_id.as_str().to_owned()));
        }
        if let Some(checkout_id) = input.checkout_id.as_ref() {
            provider_metadata.insert(
                "checkout_id",
                Value::String(checkout_id.as_str().to_owned()),
            );
        }
        if let Some(order_id) = input.order_id.as_ref() {
            provider_metadata.insert("order_id", Value::String(order_id.as_str().to_owned()));
        }
        let provider_session = provider
            .create_session(CreateProviderSession {
                reference: session_id.as_str().to_owned(),
                amount: input.amount,
                currency_code: currency_code.clone(),
                metadata: provider_metadata,
            })
            .await?;
        if !matches!(
            provider_session.status,
            PaymentStatus::Pending | PaymentStatus::RequiresAction
        ) {
            return Err(AppError::internal(
                "payment provider returned an invalid initial session status",
            ));
        }

        // Provider data may contain a client secret needed by the storefront.
        // Return it to the caller but keep it out of the persisted record.
        let (stored_data, returned_data) =
            merge_provider_session_data(&input.metadata, &provider_session.data);
        let now = now_timestamp();
        let session = PaymentSession {
            id: session_id.clone(),
            cart_id: input.cart_id,
            checkout_id: input.checkout_id,
            order_id: input.order_id,
            provider: input.provider,
            provider_session_id: Some(provider_session.provider_session_id.clone()),
            amount: input.amount,
            currency_code: currency_code.clone(),
            status: provider_session.status,
            data: stored_data,
            created_at: now,
            updated_at: now,
        };
        let attempt = PaymentAttempt {
            id: PaymentAttemptId::new(),
            payment_session_id: session_id,
            amount: input.amount,
            currency_code,
            status: PaymentStatus::Pending,
            provider_reference: Some(provider_session.provider_session_id),
            error: None,
            created_at: now,
        };

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        insert_session(&mut tx, &session).await?;
        insert_attempt(&mut tx, &attempt).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(PaymentSession {
            data: returned_data,
            ..session
        })
    }

    /// Captures a payment session, returning the existing payment when the
    /// session was already captured.
    pub async fn capture(&self, session_id: &str) -> Result<Payment, AppError> {
        let session = self.get_session(session_id).await?;
        match session.status {
            PaymentStatus::Captured
            | PaymentStatus::PartiallyRefunded
            | PaymentStatus::Refunded => {
                return self.payment_for_session(&session).await;
            }
            PaymentStatus::Failed | PaymentStatus::Canceled => {
                return Err(AppError::conflict(format!(
                    "cannot capture a payment session in status {}",
                    session.status
                )));
            }
            PaymentStatus::Pending | PaymentStatus::RequiresAction | PaymentStatus::Authorized => {}
        }

        let provider = self
            .providers
            .get(&session.provider)
            .ok_or_else(|| AppError::not_found("payment provider", &session.provider))?;
        let provider_session_id = session.provider_session_id.clone().ok_or_else(|| {
            AppError::conflict("payment session is missing a provider session id")
        })?;

        // Claim the capture before calling the provider. The deterministic key
        // lets a retry reuse the provider's idempotency protection, while the
        // timestamp lets a crashed worker's stale claim expire.
        let capture_key = format!("capture:{}", session.id.as_str());
        let claimed_at = now_timestamp();
        let stale_cutoff = claimed_at.saturating_sub(120);
        let claimed = sqlx::query(
            "UPDATE payment_sessions
             SET data = json_set(data, '$.capture_claim', ?, '$.capture_claim_at', ?),
                 updated_at = ?
             WHERE id = ?
               AND status IN ('pending', 'requires_action', 'authorized')
               AND (json_extract(data, '$.capture_claim') IS NULL
                    OR json_extract(data, '$.capture_claim_at') <= ?)",
        )
        .bind(&capture_key)
        .bind(claimed_at)
        .bind(claimed_at)
        .bind(session.id.as_str())
        .bind(stale_cutoff)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if claimed.rows_affected() == 0 {
            let latest = self.get_session(session.id.as_str()).await?;
            if matches!(
                latest.status,
                PaymentStatus::Captured
                    | PaymentStatus::PartiallyRefunded
                    | PaymentStatus::Refunded
            ) {
                return self.payment_for_session(&latest).await;
            }
            return Err(AppError::conflict("payment capture is already in progress"));
        }

        let result = match provider
            .capture(CaptureRequest {
                provider_session_id,
                amount: session.amount,
                currency_code: session.currency_code.clone(),
                idempotency_key: Some(capture_key.clone()),
                metadata: session.data.clone(),
            })
            .await
        {
            Ok(result) => result,
            Err(error) => {
                let message = error.to_string();
                // A provider/transport error is ambiguous: the capture may have
                // reached the provider even though the response was lost. Keep
                // the session retryable and release the claim so a retry can
                // reuse the same deterministic idempotency key. Only a
                // structurally invalid request is a definitive failure.
                if matches!(error, PaymentError::InvalidRequest(_)) {
                    self.record_session_failure(&session, &message).await?;
                } else {
                    self.record_ambiguous_capture_failure(&session, &capture_key, &message)
                        .await?;
                }
                return Err(error.into());
            }
        };

        let now = now_timestamp();
        let payment = Payment {
            id: PaymentId::new(),
            order_id: session.order_id.clone(),
            provider: session.provider.clone(),
            provider_payment_id: Some(result.provider_payment_id.clone()),
            amount: session.amount,
            currency_code: session.currency_code.clone(),
            status: PaymentStatus::Captured,
            captured_at: Some(now),
            metadata: session.data.clone(),
            created_at: now,
            updated_at: now,
        };
        let mut data = session.data.clone();
        data.insert("payment_id", Value::String(payment.id.as_str().to_owned()));
        data.remove("capture_claim");
        data.remove("capture_claim_at");
        let attempt = PaymentAttempt {
            id: PaymentAttemptId::new(),
            payment_session_id: session.id.clone(),
            amount: session.amount,
            currency_code: session.currency_code.clone(),
            status: PaymentStatus::Captured,
            provider_reference: Some(result.provider_payment_id),
            error: None,
            created_at: now,
        };

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE payment_sessions SET status = ?, data = ?, updated_at = ? \
             WHERE id = ? AND status IN ('pending', 'requires_action', 'authorized') \
               AND json_extract(data, '$.capture_claim') = ?",
        )
        .bind(PaymentStatus::Captured.as_str())
        .bind(data.to_json_string())
        .bind(now)
        .bind(session.id.as_str())
        .bind(&capture_key)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            let latest = self.get_session(session.id.as_str()).await?;
            if matches!(
                latest.status,
                PaymentStatus::Captured
                    | PaymentStatus::PartiallyRefunded
                    | PaymentStatus::Refunded
            ) {
                return self.payment_for_session(&latest).await;
            }
            return Err(AppError::conflict("payment capture claim was lost"));
        }
        insert_payment(&mut tx, &payment).await?;
        insert_attempt(&mut tx, &attempt).await?;
        // Checkout captures before the order exists. In that case a later
        // `assign_order` emits the captured event once the order identifier is
        // known, so consumers never receive a payment event without an order.
        if let Some(order_id) = payment.order_id.as_ref() {
            append_payment_event(
                &mut tx,
                self.db.store_id(),
                "payment.captured",
                order_id,
                payment.amount,
                &payment.currency_code,
                &payment.provider,
                Some(payment.id.as_str()),
                None,
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(payment)
    }

    /// Marks a session failed and records a failed attempt.
    /// Associates a captured payment with the order it paid.
    ///
    /// The payment is captured before the order is created (so an order is
    /// never recorded without a successful capture), then linked here.
    pub async fn assign_order(
        &self,
        payment_id: &str,
        order_id: &OrderId,
    ) -> Result<Payment, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query(
            "UPDATE payments SET order_id = ?, updated_at = ?
             WHERE id = ? AND order_id IS NULL",
        )
        .bind(order_id.as_str())
        .bind(now)
        .bind(payment_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        let row = sqlx::query_as::<_, PaymentRow>(
            "SELECT id, order_id, provider, provider_payment_id, amount, currency_code, \
             status, captured_at, metadata, created_at, updated_at \
             FROM payments WHERE id = ?",
        )
        .bind(payment_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        let payment = row
            .map(payment_from_row)
            .ok_or_else(|| AppError::not_found("payment", payment_id))?;
        if result.rows_affected() == 0 {
            tx.commit().await.map_err(database_error)?;
            if payment.order_id.as_ref() == Some(order_id) {
                return Ok(payment);
            }
            return Err(AppError::conflict(
                "payment is already assigned to another order",
            ));
        }
        // The payment was captured before the order existed; this is the first
        // point at which the captured event can identify its order.
        if matches!(payment.status, PaymentStatus::Captured) {
            append_payment_event(
                &mut tx,
                self.db.store_id(),
                "payment.captured",
                order_id,
                payment.amount,
                &payment.currency_code,
                &payment.provider,
                Some(payment.id.as_str()),
                None,
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(payment)
    }

    pub async fn fail_session(
        &self,
        session_id: &str,
        reason: impl Into<String>,
    ) -> Result<PaymentSession, AppError> {
        let reason = reason.into();
        let session = self.get_session(session_id).await?;
        if !session.status.can_transition_to(PaymentStatus::Failed) {
            return Err(AppError::conflict(format!(
                "cannot fail a payment session in status {}",
                session.status
            )));
        }
        self.transition_session(&session, PaymentStatus::Failed, Some(&reason))
            .await
    }

    /// Cancels a session that has not been captured.
    pub async fn cancel_session(&self, session_id: &str) -> Result<PaymentSession, AppError> {
        let session = self.get_session(session_id).await?;
        if !session.status.can_transition_to(PaymentStatus::Canceled) {
            return Err(AppError::conflict(format!(
                "cannot cancel a payment session in status {}",
                session.status
            )));
        }
        self.transition_session(&session, PaymentStatus::Canceled, None)
            .await
    }

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

        // The existing-key lookup, the claim and the amount cap share one
        // BEGIN IMMEDIATE transaction. Concurrent callers with the same key
        // therefore observe the winning row instead of a unique violation.
        // The lookup happens before state validation so a duplicate request
        // still returns the winning refund after a full refund.
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
        if !matches!(
            payment.status,
            PaymentStatus::Captured | PaymentStatus::PartiallyRefunded
        ) {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(format!(
                "cannot refund a payment in status {}",
                payment.status
            )));
        }
        let provider = self
            .providers
            .get(&payment.provider)
            .ok_or_else(|| AppError::not_found("payment provider", &payment.provider))?;
        let provider_payment_id = payment
            .provider_payment_id
            .clone()
            .ok_or_else(|| AppError::conflict("payment is missing a provider payment id"))?;

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
        tx.commit().await.map_err(database_error)?;

        // Provider calls always happen outside the transaction. The provider
        // key is deterministic, so re-invoking after a crash reconciles the
        // same provider-side refund.
        let result = provider
            .refund(RefundRequest {
                provider_payment_id,
                amount,
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
                // been rejected without side effects. An unknown outcome stays
                // `pending`, which keeps it counted by the over-refund cap
                // above until a webhook or retry reconciles it.
                if matches!(error, PaymentError::InvalidRequest(_)) {
                    sqlx::query(
                        "UPDATE refunds SET status = 'failed', updated_at = ?
                         WHERE id = ? AND status IN ('pending', 'failed')",
                    )
                    .bind(now_timestamp())
                    .bind(refund_id.as_str())
                    .execute(self.db.pool())
                    .await
                    .map_err(database_error)?;
                }
                return Err(error.into());
            }
        };

        let now = now_timestamp();
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
            updated_at: now,
        };
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE refunds SET status = ?, provider_refund_id = ?, updated_at = ?
             WHERE id = ? AND status IN ('pending', 'failed')",
        )
        .bind(refund.status.as_str())
        .bind(refund.provider_refund_id.as_deref())
        .bind(now)
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
        // Recompute the payment status from the persisted refunds so a
        // provider-reported failure cannot leave the payment over-refunded.
        let refunded: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM refunds
             WHERE payment_id = ? AND status != 'failed'",
        )
        .bind(payment.id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        let reconciled_payment_status = if refunded >= payment.amount {
            PaymentStatus::Refunded
        } else if refunded > 0 {
            PaymentStatus::PartiallyRefunded
        } else {
            PaymentStatus::Captured
        };
        sqlx::query(
            "UPDATE payments SET status = ?, updated_at = ? WHERE id = ?
             AND status IN ('captured', 'partially_refunded', 'refunded')",
        )
        .bind(reconciled_payment_status.as_str())
        .bind(now)
        .bind(payment.id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        let event = DomainEventEnvelope::new(
            if refund.status == RefundStatus::Failed {
                "payment.refund_failed"
            } else {
                "payment.refunded"
            },
            Some(self.db.store_id().clone()),
            serde_json::json!({
                "payment_id": payment.id.as_str(),
                "order_id": payment.order_id.as_ref().map(OrderId::as_str),
                "refund_id": refund.id.as_str(),
                "amount": refund.amount,
                "currency_code": refund.currency_code,
                "status": refund.status.as_str(),
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

    /// Loads a payment session.
    pub async fn get_session(&self, session_id: &str) -> Result<PaymentSession, AppError> {
        let row = sqlx::query_as::<_, PaymentSessionRow>(
            "SELECT id, cart_id, checkout_id, order_id, provider, provider_session_id, \
             amount, currency_code, status, data, created_at, updated_at \
             FROM payment_sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        row.map(session_from_row)
            .ok_or_else(|| AppError::not_found("payment session", session_id))
    }

    /// Loads a payment.
    pub async fn get_payment(&self, payment_id: &str) -> Result<Payment, AppError> {
        let row = sqlx::query_as::<_, PaymentRow>(
            "SELECT id, order_id, provider, provider_payment_id, amount, currency_code, \
             status, captured_at, metadata, created_at, updated_at \
             FROM payments WHERE id = ?",
        )
        .bind(payment_id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        row.map(payment_from_row)
            .ok_or_else(|| AppError::not_found("payment", payment_id))
    }

    /// Lists attempts for a payment session.
    pub async fn list_attempts(&self, session_id: &str) -> Result<Vec<PaymentAttempt>, AppError> {
        let rows = sqlx::query_as::<_, PaymentAttemptRow>(
            "SELECT id, payment_session_id, amount, currency_code, status, \
             provider_reference, error, created_at \
             FROM payment_attempts WHERE payment_session_id = ? \
             ORDER BY created_at ASC, id ASC",
        )
        .bind(session_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows.into_iter().map(attempt_from_row).collect())
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

    /// Lists payments with optional order/status filters and pagination.
    pub async fn list_payments(
        &self,
        filter: ListPaymentsFilter,
    ) -> Result<PageResult<Payment>, AppError> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, order_id, provider, provider_payment_id, amount, currency_code, \
             status, captured_at, metadata, created_at, updated_at FROM payments WHERE 1 = 1",
        );
        let mut count_query =
            QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM payments WHERE 1 = 1");
        if let Some(order_id) = filter.order_id.as_ref() {
            query.push(" AND order_id = ").push_bind(order_id.as_str());
            count_query
                .push(" AND order_id = ")
                .push_bind(order_id.as_str());
        }
        if let Some(status) = filter.status {
            query.push(" AND status = ").push_bind(status.as_str());
            count_query
                .push(" AND status = ")
                .push_bind(status.as_str());
        }
        query
            .push(" ORDER BY created_at DESC, id DESC LIMIT ")
            .push_bind(filter.pagination.limit)
            .push(" OFFSET ")
            .push_bind(filter.pagination.offset);
        let rows: Vec<PaymentRow> = query
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
            rows.into_iter().map(payment_from_row).collect(),
            count,
            filter.pagination,
        ))
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

    /// Returns the captured payment for a session, when one exists.
    pub async fn payment_for_session_id(&self, session_id: &str) -> Result<Payment, AppError> {
        let session = self.get_session(session_id).await?;
        self.payment_for_session(&session).await
    }

    async fn payment_for_session(&self, session: &PaymentSession) -> Result<Payment, AppError> {
        if let Some(payment_id) = session.data.get("payment_id").and_then(Value::as_str) {
            let row = sqlx::query_as::<_, PaymentRow>(
                "SELECT id, order_id, provider, provider_payment_id, amount, currency_code, \
                 status, captured_at, metadata, created_at, updated_at \
                 FROM payments WHERE id = ?",
            )
            .bind(payment_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
            if let Some(row) = row {
                return Ok(payment_from_row(row));
            }
        }
        Err(AppError::not_found(
            "payment for session",
            session.id.as_str(),
        ))
    }

    async fn transition_session(
        &self,
        session: &PaymentSession,
        status: PaymentStatus,
        error: Option<&str>,
    ) -> Result<PaymentSession, AppError> {
        let now = now_timestamp();
        let stale_cutoff = now.saturating_sub(120);
        let mut data = session.data.clone();
        if let Some(error) = error {
            data.insert("last_error", Value::String(error.to_owned()));
        }
        let attempt = PaymentAttempt {
            id: PaymentAttemptId::new(),
            payment_session_id: session.id.clone(),
            amount: session.amount,
            currency_code: session.currency_code.clone(),
            status,
            provider_reference: session.provider_session_id.clone(),
            error: error.map(str::to_owned),
            created_at: now,
        };
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        // Compare-and-set against the status the caller observed. A capture or
        // webhook may have advanced the session after that read; never
        // overwrite the newer state with this stale transition. A live capture
        // claim also blocks the transition: the capture sets the claim while
        // the status is still pending, so ignoring it would erase the claim
        // and let cancel/fail win a race against a provider-side capture.
        let updated = sqlx::query(
            "UPDATE payment_sessions SET status = ?, data = ?, updated_at = ? \
             WHERE id = ? AND status = ? \
               AND (json_extract(data, '$.capture_claim') IS NULL \
                    OR json_extract(data, '$.capture_claim_at') <= ?)",
        )
        .bind(status.as_str())
        .bind(data.to_json_string())
        .bind(now)
        .bind(session.id.as_str())
        .bind(session.status.as_str())
        .bind(stale_cutoff)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            let latest = self.get_session(session.id.as_str()).await?;
            if latest.status != session.status {
                return Err(AppError::conflict(format!(
                    "payment session status changed from {} to {}",
                    session.status, latest.status
                )));
            }
            return Err(AppError::conflict(
                "payment capture is already in progress for this session",
            ));
        }
        insert_attempt(&mut tx, &attempt).await?;
        if status == PaymentStatus::Failed
            && let Some(order_id) = session.order_id.as_ref()
        {
            append_payment_event(
                &mut tx,
                self.db.store_id(),
                "payment.failed",
                order_id,
                session.amount,
                &session.currency_code,
                &session.provider,
                None,
                error,
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;

        Ok(PaymentSession {
            status,
            data,
            updated_at: now,
            ..session.clone()
        })
    }

    async fn record_session_failure(
        &self,
        session: &PaymentSession,
        message: &str,
    ) -> Result<(), AppError> {
        let now = now_timestamp();
        let mut data = session.data.clone();
        data.insert("last_error", Value::String(message.to_owned()));
        let attempt = PaymentAttempt {
            id: PaymentAttemptId::new(),
            payment_session_id: session.id.clone(),
            amount: session.amount,
            currency_code: session.currency_code.clone(),
            status: PaymentStatus::Failed,
            provider_reference: session.provider_session_id.clone(),
            error: Some(message.to_owned()),
            created_at: now,
        };
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE payment_sessions SET status = 'failed', data = ?, updated_at = ? \
             WHERE id = ? AND status IN ('pending', 'requires_action', 'authorized')",
        )
        .bind(data.to_json_string())
        .bind(now)
        .bind(session.id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            let latest = self.get_session(session.id.as_str()).await?;
            return Err(AppError::conflict(format!(
                "cannot fail a payment session in status {}",
                latest.status
            )));
        }
        insert_attempt(&mut tx, &attempt).await?;
        if let Some(order_id) = session.order_id.as_ref() {
            append_payment_event(
                &mut tx,
                self.db.store_id(),
                "payment.failed",
                order_id,
                session.amount,
                &session.currency_code,
                &session.provider,
                None,
                Some(message),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Records an ambiguous capture failure without making the session
    /// terminal.
    ///
    /// The provider may have captured the payment even though the response was
    /// lost, so the session stays retryable and the claim is released. A retry
    /// reuses the deterministic capture key, and a verified webhook can also
    /// reconcile the capture.
    async fn record_ambiguous_capture_failure(
        &self,
        session: &PaymentSession,
        capture_key: &str,
        message: &str,
    ) -> Result<(), AppError> {
        let now = now_timestamp();
        let mut data = session.data.clone();
        data.insert("last_error", Value::String(message.to_owned()));
        data.remove("capture_claim");
        data.remove("capture_claim_at");
        let attempt = PaymentAttempt {
            id: PaymentAttemptId::new(),
            payment_session_id: session.id.clone(),
            amount: session.amount,
            currency_code: session.currency_code.clone(),
            status: PaymentStatus::Failed,
            provider_reference: session.provider_session_id.clone(),
            error: Some(message.to_owned()),
            created_at: now,
        };
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "UPDATE payment_sessions SET data = ?, updated_at = ? \
             WHERE id = ? AND status IN ('pending', 'requires_action', 'authorized') \
               AND json_extract(data, '$.capture_claim') = ?",
        )
        .bind(data.to_json_string())
        .bind(now)
        .bind(session.id.as_str())
        .bind(capture_key)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        insert_attempt(&mut tx, &attempt).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
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
                                PaymentStatus::Captured
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
            let payment = sqlx::query_as::<_, (Option<String>, i64, String)>(
                "SELECT order_id, amount, currency_code FROM payments WHERE id = ?",
            )
            .bind(&payment_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            if let Some((order_id, amount, currency_code)) = payment {
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
                    PaymentStatus::Captured
                };
                let updated = sqlx::query(
                    "UPDATE payments SET status = ?, updated_at = ? WHERE id = ? \
                     AND status IN ('captured', 'partially_refunded', 'refunded')",
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

fn string_field(data: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        data.get(*key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

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

/// Wraps a webhook apply failure so the provider redelivers the event.
fn retryable_webhook_error(error: &AppError) -> AppError {
    AppError::ExternalService(format!("payment webhook processing failed: {error}"))
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

/// Appends one payment lifecycle event inside the caller's transaction.
#[allow(clippy::too_many_arguments)]
async fn append_payment_event(
    tx: &mut Transaction<'_, Sqlite>,
    store_id: &fastcart_kernel::StoreId,
    event_type: &str,
    order_id: &OrderId,
    amount: i64,
    currency_code: &str,
    provider: &str,
    payment_id: Option<&str>,
    reason: Option<&str>,
) -> Result<(), AppError> {
    let event = DomainEventEnvelope::new(
        event_type,
        Some(store_id.clone()),
        serde_json::json!({
            "payment_id": payment_id,
            "order_id": order_id.as_str(),
            "amount": amount,
            "currency_code": currency_code,
            "provider": provider,
            "reason": reason,
        }),
    );
    append_outbox_event(&mut **tx, &event)
        .await
        .map_err(database_error)
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

/// Merges provider session data into the session record.
///
/// The first value is persisted; the second is returned to the caller and may
/// contain transient secrets such as a PaymentIntent client secret.
fn merge_provider_session_data(base: &Metadata, provider_data: &Value) -> (Metadata, Metadata) {
    let mut stored = base.clone();
    let mut returned = base.clone();
    if let Value::Object(values) = provider_data {
        for (key, value) in values {
            if !is_sensitive_provider_data_key(key) {
                stored.insert(key.clone(), value.clone());
            }
            returned.insert(key.clone(), value.clone());
        }
    }
    (stored, returned)
}

fn is_sensitive_provider_data_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("client_secret") || key.eq_ignore_ascii_case("secret")
}

#[derive(sqlx::FromRow)]
struct PaymentSessionRow {
    id: String,
    cart_id: Option<String>,
    checkout_id: Option<String>,
    order_id: Option<String>,
    provider: String,
    provider_session_id: Option<String>,
    amount: i64,
    currency_code: String,
    status: String,
    data: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct PaymentAttemptRow {
    id: String,
    payment_session_id: String,
    amount: i64,
    currency_code: String,
    status: String,
    provider_reference: Option<String>,
    error: Option<String>,
    created_at: i64,
}

#[derive(sqlx::FromRow)]
struct PaymentRow {
    id: String,
    order_id: Option<String>,
    provider: String,
    provider_payment_id: Option<String>,
    amount: i64,
    currency_code: String,
    status: String,
    captured_at: Option<i64>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Clone, sqlx::FromRow)]
struct RefundRow {
    id: String,
    payment_id: Option<String>,
    order_id: Option<String>,
    amount: i64,
    currency_code: String,
    status: String,
    provider_refund_id: Option<String>,
    idempotency_key: Option<String>,
    reason: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct PaymentWebhookRow {
    id: String,
    provider: String,
    provider_event_id: String,
    event_type: String,
    payload: String,
    status: String,
    attempts: i64,
    processed_at: Option<i64>,
    last_error: Option<String>,
    created_at: i64,
}

fn session_from_row(row: PaymentSessionRow) -> PaymentSession {
    PaymentSession {
        id: PaymentSessionId::from_string(row.id),
        cart_id: row.cart_id.map(CartId::from_string),
        checkout_id: row.checkout_id.map(CheckoutId::from_string),
        order_id: row.order_id.map(OrderId::from_string),
        provider: row.provider,
        provider_session_id: row.provider_session_id,
        amount: row.amount,
        currency_code: row.currency_code,
        status: PaymentStatus::parse(&row.status).unwrap_or(PaymentStatus::Pending),
        data: fastcart_kernel::Metadata::from_json_str(&row.data),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn attempt_from_row(row: PaymentAttemptRow) -> PaymentAttempt {
    PaymentAttempt {
        id: PaymentAttemptId::from_string(row.id),
        payment_session_id: PaymentSessionId::from_string(row.payment_session_id),
        amount: row.amount,
        currency_code: row.currency_code,
        status: PaymentStatus::parse(&row.status).unwrap_or(PaymentStatus::Pending),
        provider_reference: row.provider_reference,
        error: row.error,
        created_at: row.created_at,
    }
}

fn payment_from_row(row: PaymentRow) -> Payment {
    Payment {
        id: PaymentId::from_string(row.id),
        order_id: row.order_id.map(OrderId::from_string),
        provider: row.provider,
        provider_payment_id: row.provider_payment_id,
        amount: row.amount,
        currency_code: row.currency_code,
        status: PaymentStatus::parse(&row.status).unwrap_or(PaymentStatus::Pending),
        captured_at: row.captured_at,
        metadata: fastcart_kernel::Metadata::from_json_str(&row.metadata),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn refund_from_row(row: RefundRow) -> Refund {
    Refund {
        id: RefundId::from_string(row.id),
        payment_id: row.payment_id.map(PaymentId::from_string),
        order_id: row.order_id.map(OrderId::from_string),
        amount: row.amount,
        currency_code: row.currency_code,
        status: RefundStatus::parse(&row.status).unwrap_or(RefundStatus::Failed),
        provider_refund_id: row.provider_refund_id,
        idempotency_key: row.idempotency_key,
        reason: row.reason,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn webhook_from_row(row: PaymentWebhookRow) -> PaymentWebhookRecord {
    PaymentWebhookRecord {
        id: WebhookId::from_string(row.id),
        provider: row.provider,
        provider_event_id: row.provider_event_id,
        event_type: row.event_type,
        payload: row.payload,
        status: WebhookStatus::parse(&row.status),
        attempts: row.attempts,
        processed_at: row.processed_at,
        last_error: row.last_error,
        created_at: row.created_at,
    }
}

async fn insert_session(
    tx: &mut Transaction<'_, Sqlite>,
    session: &PaymentSession,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO payment_sessions \
         (id, cart_id, checkout_id, order_id, provider, provider_session_id, amount, \
          currency_code, status, data, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(session.id.as_str())
    .bind(session.cart_id.as_ref().map(CartId::as_str))
    .bind(session.checkout_id.as_ref().map(CheckoutId::as_str))
    .bind(session.order_id.as_ref().map(OrderId::as_str))
    .bind(&session.provider)
    .bind(session.provider_session_id.as_deref())
    .bind(session.amount)
    .bind(&session.currency_code)
    .bind(session.status.as_str())
    .bind(session.data.to_json_string())
    .bind(session.created_at)
    .bind(session.updated_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn insert_attempt(
    tx: &mut Transaction<'_, Sqlite>,
    attempt: &PaymentAttempt,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO payment_attempts \
         (id, payment_session_id, amount, currency_code, status, provider_reference, error, \
          created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(attempt.id.as_str())
    .bind(attempt.payment_session_id.as_str())
    .bind(attempt.amount)
    .bind(&attempt.currency_code)
    .bind(attempt.status.as_str())
    .bind(attempt.provider_reference.as_deref())
    .bind(attempt.error.as_deref())
    .bind(attempt.created_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn insert_payment(
    tx: &mut Transaction<'_, Sqlite>,
    payment: &Payment,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO payments \
         (id, order_id, provider, provider_payment_id, amount, currency_code, status, \
          captured_at, metadata, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(payment.id.as_str())
    .bind(payment.order_id.as_ref().map(OrderId::as_str))
    .bind(&payment.provider)
    .bind(payment.provider_payment_id.as_deref())
    .bind(payment.amount)
    .bind(&payment.currency_code)
    .bind(payment.status.as_str())
    .bind(payment.captured_at)
    .bind(payment.metadata.to_json_string())
    .bind(payment.created_at)
    .bind(payment.updated_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn insert_refund(tx: &mut Transaction<'_, Sqlite>, refund: &Refund) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO refunds \
         (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
          idempotency_key, reason, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(refund.id.as_str())
    .bind(refund.payment_id.as_ref().map(PaymentId::as_str))
    .bind(refund.order_id.as_ref().map(OrderId::as_str))
    .bind(refund.amount)
    .bind(&refund.currency_code)
    .bind(refund.status.as_str())
    .bind(refund.provider_refund_id.as_deref())
    .bind(refund.idempotency_key.as_deref())
    .bind(refund.reason.as_deref())
    .bind(refund.created_at)
    .bind(refund.updated_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use fastcart_infrastructure::StoreDatabase;
    use fastcart_kernel::{
        AppError, Metadata, OrderId, PageResult, Pagination, StoreId, now_timestamp,
    };
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    use super::PaymentService;
    use crate::domain::{
        CreatePaymentSessionInput, ListPaymentsFilter, PaymentStatus, RefundStatus, WebhookStatus,
    };
    use crate::provider::{
        CaptureRequest, CaptureResult, CreateProviderSession, MANUAL_SIGNATURE_HEADER,
        ManualProvider, PaymentError, PaymentProvider, ProviderSession, RefundRequest,
        RefundResult, VerifiedWebhook,
    };

    async fn setup() -> (PaymentService, StoreDatabase) {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        let service = PaymentService::new(db.clone());
        (service, db)
    }

    fn manual(secret: Option<&str>) -> Arc<dyn PaymentProvider> {
        Arc::new(ManualProvider::new("manual", secret.map(str::to_owned)))
    }

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    fn headers(secret: &str, body: &[u8]) -> BTreeMap<String, String> {
        BTreeMap::from([(MANUAL_SIGNATURE_HEADER.to_owned(), sign(secret, body))])
    }

    fn input(provider: &str, amount: i64) -> CreatePaymentSessionInput {
        CreatePaymentSessionInput::new(provider, amount, "usd")
    }

    async fn create_and_capture(
        service: &PaymentService,
        amount: i64,
    ) -> (String, crate::domain::Payment) {
        let session = service
            .create_session(input("manual", amount))
            .await
            .unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();
        (session.id.to_string(), payment)
    }

    struct TestProvider {
        id: String,
        fail_create: bool,
        fail_capture: bool,
        invalid_capture: bool,
        capture_failures_remaining: AtomicUsize,
        fail_refund: bool,
        invalid_refund: bool,
        create_calls: AtomicUsize,
        capture_calls: AtomicUsize,
        refund_calls: AtomicUsize,
    }

    impl TestProvider {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_owned(),
                fail_create: false,
                fail_capture: false,
                invalid_capture: false,
                capture_failures_remaining: AtomicUsize::new(0),
                fail_refund: false,
                invalid_refund: false,
                create_calls: AtomicUsize::new(0),
                capture_calls: AtomicUsize::new(0),
                refund_calls: AtomicUsize::new(0),
            }
        }

        fn transient_capture(id: &str) -> Self {
            Self {
                capture_failures_remaining: AtomicUsize::new(1),
                ..Self::new(id)
            }
        }

        fn invalid_capture(id: &str) -> Self {
            Self {
                invalid_capture: true,
                ..Self::new(id)
            }
        }

        fn failing_refund(id: &str) -> Self {
            Self {
                fail_refund: true,
                ..Self::new(id)
            }
        }

        fn invalid_refund(id: &str) -> Self {
            Self {
                invalid_refund: true,
                ..Self::new(id)
            }
        }

        fn failing_create(id: &str) -> Self {
            Self {
                fail_create: true,
                ..Self::new(id)
            }
        }

        fn capture_count(&self) -> usize {
            self.capture_calls.load(Ordering::SeqCst)
        }

        fn refund_count(&self) -> usize {
            self.refund_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl PaymentProvider for TestProvider {
        fn id(&self) -> &str {
            &self.id
        }

        async fn create_session(
            &self,
            _request: CreateProviderSession,
        ) -> Result<ProviderSession, PaymentError> {
            self.create_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_create {
                return Err(PaymentError::Provider("create failed".to_owned()));
            }
            Ok(ProviderSession {
                provider_session_id: format!("{}_session", self.id),
                status: PaymentStatus::Pending,
                data: serde_json::json!({}),
            })
        }

        async fn capture(&self, request: CaptureRequest) -> Result<CaptureResult, PaymentError> {
            self.capture_calls.fetch_add(1, Ordering::SeqCst);
            if self.invalid_capture {
                return Err(PaymentError::InvalidRequest(
                    "capture request is invalid".to_owned(),
                ));
            }
            if self.fail_capture {
                return Err(PaymentError::Provider("capture failed".to_owned()));
            }
            let remaining = self.capture_failures_remaining.load(Ordering::SeqCst);
            if remaining > 0 {
                self.capture_failures_remaining
                    .store(remaining - 1, Ordering::SeqCst);
                return Err(PaymentError::Provider("capture transport error".to_owned()));
            }
            Ok(CaptureResult {
                provider_payment_id: format!("{}_payment", self.id),
                amount: request.amount,
                currency_code: request.currency_code,
                status: PaymentStatus::Captured,
            })
        }

        async fn refund(&self, request: RefundRequest) -> Result<RefundResult, PaymentError> {
            self.refund_calls.fetch_add(1, Ordering::SeqCst);
            if self.invalid_refund {
                return Err(PaymentError::InvalidRequest(
                    "refund request is invalid".to_owned(),
                ));
            }
            if self.fail_refund {
                return Err(PaymentError::Provider("refund failed".to_owned()));
            }
            Ok(RefundResult {
                provider_refund_id: format!("{}_refund", self.id),
                amount: request.amount,
                currency_code: request.currency_code,
                status: RefundStatus::Succeeded,
            })
        }

        fn verify_webhook(
            &self,
            _headers: &BTreeMap<String, String>,
            _body: &[u8],
        ) -> Result<VerifiedWebhook, PaymentError> {
            Err(PaymentError::InvalidSignature)
        }
    }

    #[tokio::test]
    async fn creates_session_and_attempt() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let mut input = input("manual", 2500);
        input.metadata.insert("checkout", serde_json::json!("v1"));
        input.cart_id = Some(fastcart_kernel::CartId::new());

        let session = service.create_session(input).await.unwrap();
        assert_eq!(session.amount, 2500);
        assert_eq!(session.currency_code, "USD");
        assert_eq!(session.status, PaymentStatus::Pending);
        assert!(session.provider_session_id.is_some());
        assert_eq!(session.data.get("checkout"), Some(&serde_json::json!("v1")));

        let stored = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(stored, session);
        let attempts = service.list_attempts(session.id.as_str()).await.unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].status, PaymentStatus::Pending);
        assert_eq!(attempts[0].amount, 2500);
    }

    #[tokio::test]
    async fn rejects_unknown_provider_negative_amount_and_bad_currency() {
        let (service, _db) = setup().await;

        let missing = service
            .create_session(input("missing", 100))
            .await
            .unwrap_err();
        assert!(matches!(missing, AppError::NotFound(_)));

        service.register_provider(manual(Some("secret"))).unwrap();
        let negative = service
            .create_session(input("manual", -1))
            .await
            .unwrap_err();
        assert!(matches!(negative, AppError::Validation(_)));

        let bad_currency = service
            .create_session(CreatePaymentSessionInput::new("manual", 100, "us"))
            .await
            .unwrap_err();
        assert!(matches!(bad_currency, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn full_flow_captures_and_refunds() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();

        let (session_id, payment) = create_and_capture(&service, 1000).await;
        assert_eq!(payment.status, PaymentStatus::Captured);
        assert_eq!(payment.amount, 1000);
        assert!(payment.provider_payment_id.is_some());
        assert!(payment.captured_at.is_some());

        let partial = service
            .refund(payment.id.as_str(), 400, Some("damaged".to_owned()))
            .await
            .unwrap();
        assert_eq!(partial.status, RefundStatus::Succeeded);
        assert_eq!(partial.amount, 400);
        assert_eq!(
            service
                .get_payment(payment.id.as_str())
                .await
                .unwrap()
                .status,
            PaymentStatus::PartiallyRefunded
        );

        let remainder = service
            .refund(payment.id.as_str(), 600, None)
            .await
            .unwrap();
        assert_eq!(remainder.amount, 600);
        let refunded = service.get_payment(payment.id.as_str()).await.unwrap();
        assert_eq!(refunded.status, PaymentStatus::Refunded);
        assert_eq!(
            service
                .list_refunds(payment.id.as_str())
                .await
                .unwrap()
                .len(),
            2
        );

        let captured_again = service.capture(&session_id).await.unwrap();
        assert_eq!(captured_again.id, payment.id);
        assert_eq!(captured_again.status, PaymentStatus::Refunded);
    }

    #[tokio::test]
    async fn capture_is_idempotent_and_does_not_repeat_provider_call() {
        let (service, _db) = setup().await;
        let provider = Arc::new(TestProvider::new("counter"));
        service.register_provider(provider.clone()).unwrap();

        let session = service
            .create_session(input("counter", 1500))
            .await
            .unwrap();
        let first = service.capture(session.id.as_str()).await.unwrap();
        let second = service.capture(session.id.as_str()).await.unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(provider.capture_count(), 1);
    }

    #[tokio::test]
    async fn rejects_over_refund_before_calling_provider() {
        let (service, _db) = setup().await;
        let provider = Arc::new(TestProvider::new("limited"));
        service.register_provider(provider.clone()).unwrap();
        let session = service
            .create_session(input("limited", 1000))
            .await
            .unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        service
            .refund(payment.id.as_str(), 700, None)
            .await
            .unwrap();
        let over = service
            .refund(payment.id.as_str(), 400, None)
            .await
            .unwrap_err();
        assert!(matches!(over, AppError::Conflict(_)));
        assert_eq!(
            service
                .get_payment(payment.id.as_str())
                .await
                .unwrap()
                .status,
            PaymentStatus::PartiallyRefunded
        );

        let zero = service
            .refund(payment.id.as_str(), 0, None)
            .await
            .unwrap_err();
        assert!(matches!(zero, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn rejects_refund_before_capture() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();

        let error = service
            .refund(session.id.as_str(), 100, None)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn provider_failures_surface_as_external_service_errors() {
        let (service, db) = setup().await;
        service
            .register_provider(Arc::new(TestProvider::failing_create("broken")))
            .unwrap();
        let error = service
            .create_session(input("broken", 100))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::ExternalService(_)));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payment_sessions")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn concurrent_captures_never_call_the_provider_twice() {
        let (service, _db) = setup().await;
        let provider = Arc::new(TestProvider::new("manual"));
        service.register_provider(provider.clone()).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let service = Arc::new(service);
        let first = service.clone();
        let second = service.clone();
        let id = session.id.to_string();
        let (left, right) = tokio::join!(first.capture(&id), second.capture(&id));
        assert!(left.is_ok() || right.is_ok());
        assert_eq!(provider.capture_count(), 1);
        let payments = service
            .list_payments(ListPaymentsFilter {
                order_id: None,
                status: None,
                pagination: Pagination::default(),
            })
            .await
            .unwrap();
        assert_eq!(payments.count, 1);
    }

    #[tokio::test]
    async fn ambiguous_capture_failure_stays_retryable_and_retries_provider() {
        let (service, _db) = setup().await;
        let provider = Arc::new(TestProvider::transient_capture("flaky"));
        service.register_provider(provider.clone()).unwrap();
        let session = service.create_session(input("flaky", 1000)).await.unwrap();

        let error = service.capture(session.id.as_str()).await.unwrap_err();
        assert!(matches!(error, AppError::ExternalService(_)));
        let stored = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(stored.status, PaymentStatus::Pending);
        assert!(stored.data.get("capture_claim").is_none());
        let attempts = service.list_attempts(session.id.as_str()).await.unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[1].status, PaymentStatus::Failed);
        assert!(attempts[1].error.is_some());

        let payment = service.capture(session.id.as_str()).await.unwrap();
        assert_eq!(payment.status, PaymentStatus::Captured);
        assert_eq!(provider.capture_count(), 2);
        let payments = service
            .list_payments(ListPaymentsFilter {
                order_id: None,
                status: None,
                pagination: Pagination::default(),
            })
            .await
            .unwrap();
        assert_eq!(payments.count, 1);
    }

    #[tokio::test]
    async fn invalid_capture_request_marks_session_and_attempt_failed() {
        let (service, _db) = setup().await;
        service
            .register_provider(Arc::new(TestProvider::invalid_capture("broken")))
            .unwrap();
        let session = service.create_session(input("broken", 1000)).await.unwrap();

        let error = service.capture(session.id.as_str()).await.unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
        let stored = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(stored.status, PaymentStatus::Failed);
        let attempts = service.list_attempts(session.id.as_str()).await.unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[1].status, PaymentStatus::Failed);
        assert!(attempts[1].error.is_some());
    }

    #[tokio::test]
    async fn definitive_refund_rejection_marks_the_refund_failed() {
        let (service, _db) = setup().await;
        service
            .register_provider(Arc::new(TestProvider::invalid_refund("broken")))
            .unwrap();
        let session = service.create_session(input("broken", 1000)).await.unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        let error = service
            .refund(payment.id.as_str(), 100, None)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
        let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
        assert_eq!(refunds.len(), 1);
        assert_eq!(refunds[0].status, RefundStatus::Failed);
        assert_eq!(
            service
                .get_payment(payment.id.as_str())
                .await
                .unwrap()
                .status,
            PaymentStatus::Captured
        );
    }

    #[tokio::test]
    async fn ambiguous_refund_failure_stays_pending_and_consumes_the_cap() {
        let (service, _db) = setup().await;
        let provider = Arc::new(TestProvider::failing_refund("unknown"));
        service.register_provider(provider.clone()).unwrap();
        let session = service
            .create_session(input("unknown", 1000))
            .await
            .unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        let error = service
            .refund(payment.id.as_str(), 700, Some("first".to_owned()))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::ExternalService(_)));
        let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
        assert_eq!(refunds.len(), 1);
        assert_eq!(refunds[0].status, RefundStatus::Pending);
        assert_eq!(
            service
                .get_payment(payment.id.as_str())
                .await
                .unwrap()
                .status,
            PaymentStatus::Captured
        );

        // 400 more would fit only if the unknown 700 were excluded from the
        // cap. Pending refunds count, so this must be rejected.
        let over = service
            .refund(payment.id.as_str(), 400, Some("second".to_owned()))
            .await
            .unwrap_err();
        assert!(matches!(over, AppError::Conflict(_)));
        assert_eq!(provider.refund_count(), 1);
        assert_eq!(
            service
                .list_refunds(payment.id.as_str())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn duplicate_refund_key_returns_the_existing_refund() {
        let (service, _db) = setup().await;
        let provider = Arc::new(TestProvider::new("dedupe"));
        service.register_provider(provider.clone()).unwrap();
        let session = service.create_session(input("dedupe", 1000)).await.unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        let first = service
            .refund_with_idempotency(
                payment.id.as_str(),
                400,
                None,
                Some("return-refund:dedupe".to_owned()),
            )
            .await
            .unwrap();
        let second = service
            .refund_with_idempotency(
                payment.id.as_str(),
                400,
                None,
                Some("return-refund:dedupe".to_owned()),
            )
            .await
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.status, RefundStatus::Succeeded);
        assert_eq!(provider.refund_count(), 1);
        assert_eq!(
            service
                .list_refunds(payment.id.as_str())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn keyless_refund_retry_reuses_the_same_refund() {
        let (service, _db) = setup().await;
        let provider = Arc::new(TestProvider::new("keyless"));
        service.register_provider(provider.clone()).unwrap();
        let session = service
            .create_session(input("keyless", 1000))
            .await
            .unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        let first = service
            .refund(payment.id.as_str(), 400, Some("damaged".to_owned()))
            .await
            .unwrap();
        // Only surrounding whitespace differs: the normalized reason must map
        // to the same deterministic idempotency key.
        let second = service
            .refund(payment.id.as_str(), 400, Some(" damaged ".to_owned()))
            .await
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.status, RefundStatus::Succeeded);
        assert_eq!(provider.refund_count(), 1);
        let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
        assert_eq!(refunds.len(), 1);
        assert_eq!(refunds[0].id, first.id);
    }

    #[tokio::test]
    async fn omitted_refund_key_is_derived_and_idempotent() {
        let (service, _db) = setup().await;
        let provider = Arc::new(TestProvider::new("derived"));
        service.register_provider(provider.clone()).unwrap();
        let session = service
            .create_session(input("derived", 1000))
            .await
            .unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        let first = service
            .refund_with_idempotency(payment.id.as_str(), 300, Some("goodwill".to_owned()), None)
            .await
            .unwrap();
        let second = service
            .refund_with_idempotency(
                payment.id.as_str(),
                300,
                Some(" goodwill ".to_owned()),
                None,
            )
            .await
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(provider.refund_count(), 1);
        let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
        assert_eq!(refunds.len(), 1);
    }

    #[tokio::test]
    async fn duplicate_full_refund_key_returns_existing_after_payment_is_refunded() {
        let (service, _db) = setup().await;
        let provider = Arc::new(TestProvider::new("dedupe-full"));
        service.register_provider(provider.clone()).unwrap();
        let session = service
            .create_session(input("dedupe-full", 1000))
            .await
            .unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        let first = service
            .refund_with_idempotency(
                payment.id.as_str(),
                1000,
                None,
                Some("return-refund:full".to_owned()),
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .get_payment(payment.id.as_str())
                .await
                .unwrap()
                .status,
            PaymentStatus::Refunded
        );
        let second = service
            .refund_with_idempotency(
                payment.id.as_str(),
                1000,
                None,
                Some("return-refund:full".to_owned()),
            )
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(provider.refund_count(), 1);
    }

    #[tokio::test]
    async fn concurrent_duplicate_refund_keys_return_the_winner() {
        let (service, _db) = setup().await;
        let provider = Arc::new(TestProvider::new("dedupe-concurrent"));
        service.register_provider(provider.clone()).unwrap();
        let session = service
            .create_session(input("dedupe-concurrent", 1000))
            .await
            .unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();
        let service = Arc::new(service);

        let first = service.clone();
        let second = service.clone();
        let payment_id = payment.id.to_string();
        let (left, right) = tokio::join!(
            first.refund_with_idempotency(
                &payment_id,
                400,
                None,
                Some("return-refund:race".to_owned())
            ),
            second.refund_with_idempotency(
                &payment_id,
                400,
                None,
                Some("return-refund:race".to_owned())
            )
        );
        let left = left.unwrap();
        let right = right.unwrap();
        assert_eq!(left.id, right.id);
        assert_eq!(
            service
                .list_refunds(payment_id.as_str())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn pending_refund_crash_retry_calls_the_provider_and_reconciles() {
        let (service, db) = setup().await;
        let provider = Arc::new(TestProvider::new("crash"));
        service.register_provider(provider.clone()).unwrap();
        let session = service.create_session(input("crash", 1000)).await.unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        // Simulate a crash after the pending refund row was committed but
        // before the provider call returned.
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO refunds \
             (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
              idempotency_key, reason, created_at, updated_at) \
             VALUES ('refund_crash', ?, NULL, 400, 'USD', 'pending', NULL, \
                     'return-refund:crash', NULL, ?, ?)",
        )
        .bind(payment.id.as_str())
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();

        let refund = service
            .refund_with_idempotency(
                payment.id.as_str(),
                400,
                None,
                Some("return-refund:crash".to_owned()),
            )
            .await
            .unwrap();

        assert_eq!(refund.id.as_str(), "refund_crash");
        assert_eq!(refund.status, RefundStatus::Succeeded);
        assert_eq!(provider.refund_count(), 1);
        assert_eq!(
            service
                .list_refunds(payment.id.as_str())
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            service
                .get_payment(payment.id.as_str())
                .await
                .unwrap()
                .status,
            PaymentStatus::PartiallyRefunded
        );
    }

    #[tokio::test]
    async fn refund_failed_webhook_reverts_the_payment_status() {
        let (service, db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO refunds \
             (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
              idempotency_key, reason, created_at, updated_at) \
             VALUES ('refund_revert', ?, NULL, 400, 'USD', 'succeeded', \
                     'manual_refund_revert', NULL, NULL, ?, ?)",
        )
        .bind(payment.id.as_str())
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query("UPDATE payments SET status = 'partially_refunded' WHERE id = ?")
            .bind(payment.id.as_str())
            .execute(db.pool())
            .await
            .unwrap();

        let body = r#"{"id":"evt_refund_failed","type":"refund.failed","data":{"provider_refund_id":"manual_refund_revert","provider_payment_id":"unused"}}"#;
        let record = service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(record.status, WebhookStatus::Processed);

        let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
        assert_eq!(refunds.len(), 1);
        assert_eq!(refunds[0].status, RefundStatus::Failed);
        assert_eq!(
            service
                .get_payment(payment.id.as_str())
                .await
                .unwrap()
                .status,
            PaymentStatus::Captured
        );
    }

    #[tokio::test]
    async fn webhook_with_valid_signature_updates_session_state() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let provider_session_id = session.provider_session_id.clone().unwrap();
        let body = format!(
            r#"{{"id":"evt_captured_1","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
        );
        let record = service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();

        assert_eq!(record.status, WebhookStatus::Processed);
        assert_eq!(record.event_type, "payment.captured");
        assert_eq!(record.attempts, 1);
        assert!(record.processed_at.is_some());
        let updated = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(updated.status, PaymentStatus::Captured);
    }

    #[tokio::test]
    async fn failed_webhooks_are_retried_instead_of_being_dropped() {
        let (service, db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let provider_session_id = session.provider_session_id.clone().unwrap();
        let body = format!(
            r#"{{"id":"evt_retry","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
        );
        sqlx::query(
            "INSERT INTO payment_webhooks
             (id, provider, provider_event_id, event_type, payload, status, attempts,
              processed_at, last_error, created_at)
             VALUES ('wh_retry', 'manual', 'evt_retry', 'payment.captured', '{}',
                     'failed', 1, NULL, 'transient', 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let record = service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(record.status, WebhookStatus::Processed);
        assert_eq!(record.attempts, 2);
        let updated = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(updated.status, PaymentStatus::Captured);
    }

    #[tokio::test]
    async fn received_webhook_is_applied_by_a_later_delivery() {
        let (service, db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let provider_session_id = session.provider_session_id.clone().unwrap();
        let body = format!(
            r#"{{"id":"evt_received","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
        );
        // Simulate a first delivery that inserted the row and then crashed
        // before applying the event.
        sqlx::query(
            "INSERT INTO payment_webhooks
             (id, provider, provider_event_id, event_type, payload, status, attempts,
              processed_at, last_error, created_at)
             VALUES ('wh_received', 'manual', 'evt_received', 'payment.captured', '{}',
                     'received', 0, NULL, NULL, 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let record = service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();

        assert_eq!(record.id.as_str(), "wh_received");
        assert_eq!(record.status, WebhookStatus::Processed);
        assert_eq!(record.attempts, 1);
        assert_eq!(
            service
                .get_session(session.id.as_str())
                .await
                .unwrap()
                .status,
            PaymentStatus::Captured
        );
    }

    #[tokio::test]
    async fn webhook_with_invalid_signature_is_rejected() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let body = br#"{"id":"evt_1","type":"payment.captured","data":{}}"#;

        let error = service
            .process_webhook("manual", headers("wrong-secret", body), body)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Unauthorized));
        let listed = service
            .list_webhooks(None, Pagination::default())
            .await
            .unwrap();
        assert_eq!(listed.count, 0);
    }

    #[tokio::test]
    async fn duplicate_webhook_is_idempotent() {
        let (service, db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let provider_session_id = session.provider_session_id.clone().unwrap();
        let body = format!(
            r#"{{"id":"evt_dup","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
        );
        let first = service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();
        let second = service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.attempts, 1);
        assert_eq!(second.status, WebhookStatus::Processed);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payment_webhooks")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn webhook_capture_creates_one_payment_and_is_idempotent_on_replay() {
        let (service, db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let provider_session_id = session.provider_session_id.clone().unwrap();
        let body = format!(
            r#"{{"id":"evt_async","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_1","amount":1000,"currency":"USD"}}}}"#
        );

        let record = service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(record.status, WebhookStatus::Processed);

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE provider = 'manual'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count, 1);

        let payment = service
            .payment_for_session_id(session.id.as_str())
            .await
            .unwrap();
        assert_eq!(
            payment.provider_payment_id.as_deref(),
            Some("manual_payment_1")
        );
        assert_eq!(payment.amount, 1000);
        assert_eq!(payment.currency_code, "USD");
        assert_eq!(payment.status, PaymentStatus::Captured);
        let updated = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(updated.status, PaymentStatus::Captured);
        assert_eq!(
            updated.data.get("payment_id"),
            Some(&serde_json::json!(payment.id.as_str()))
        );

        // Replaying the exact same provider event must not create another row.
        service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();
        // A different event with the same provider identifiers is also
        // idempotent: the unique provider payment id wins.
        let replay = format!(
            r#"{{"id":"evt_async_2","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_1","amount":1000,"currency":"USD"}}}}"#
        );
        service
            .process_webhook(
                "manual",
                headers("secret", replay.as_bytes()),
                replay.as_bytes(),
            )
            .await
            .unwrap();
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE provider = 'manual'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count, 1);
        let again = service
            .payment_for_session_id(session.id.as_str())
            .await
            .unwrap();
        assert_eq!(again.id, payment.id);
    }

    #[tokio::test]
    async fn webhook_capture_emits_the_captured_event_once() {
        let (service, db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let mut session_input = input("manual", 1000);
        let order_id = OrderId::new();
        session_input.order_id = Some(order_id.clone());
        let session = service.create_session(session_input).await.unwrap();
        let provider_session_id = session.provider_session_id.clone().unwrap();
        let body = format!(
            r#"{{"id":"evt_event_once","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_event"}}}}"#
        );
        service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();

        // A redelivery under a different provider event id is still the same
        // capture and must not emit a second lifecycle event.
        let replay = format!(
            r#"{{"id":"evt_event_once_2","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_event"}}}}"#
        );
        service
            .process_webhook(
                "manual",
                headers("secret", replay.as_bytes()),
                replay.as_bytes(),
            )
            .await
            .unwrap();

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM outbox WHERE event_type = 'payment.captured'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count, 1);
        let payload: String =
            sqlx::query_scalar("SELECT payload FROM outbox WHERE event_type = 'payment.captured'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["order_id"], order_id.as_str());
        assert!(payload["payment_id"].as_str().is_some());
    }

    #[tokio::test]
    async fn webhook_capture_reconciles_a_failed_session() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        service
            .fail_session(session.id.as_str(), "previous ambiguity")
            .await
            .unwrap();
        let provider_session_id = session.provider_session_id.clone().unwrap();
        let body = format!(
            r#"{{"id":"evt_recover","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_recover","amount":1000}}}}"#
        );

        service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();

        let updated = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(updated.status, PaymentStatus::Captured);
        let payment = service
            .payment_for_session_id(session.id.as_str())
            .await
            .unwrap();
        assert_eq!(payment.status, PaymentStatus::Captured);
        assert_eq!(
            payment.provider_payment_id.as_deref(),
            Some("manual_payment_recover")
        );
    }

    #[tokio::test]
    async fn webhook_apply_failure_is_retryable_and_reapplied_on_redelivery() {
        let (service, db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let provider_session_id = session.provider_session_id.clone().unwrap();
        let body = format!(
            r#"{{"id":"evt_apply_retry","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
        );
        sqlx::query(
            "CREATE TRIGGER reject_capture BEFORE UPDATE ON payment_sessions \
             WHEN NEW.status = 'captured' BEGIN SELECT RAISE(ABORT, 'transient'); END",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let error = service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::ExternalService(_)));
        let status: String = sqlx::query_scalar(
            "SELECT status FROM payment_webhooks WHERE provider_event_id = 'evt_apply_retry'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(status, "failed");

        sqlx::query("DROP TRIGGER reject_capture")
            .execute(db.pool())
            .await
            .unwrap();
        let record = service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(record.status, WebhookStatus::Processed);
        assert_eq!(record.attempts, 2);
        assert_eq!(
            service
                .get_session(session.id.as_str())
                .await
                .unwrap()
                .status,
            PaymentStatus::Captured
        );
    }

    #[tokio::test]
    async fn unknown_webhook_event_is_stored_without_state_changes() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let body =
            br#"{"id":"evt_unknown","type":"customer.created","data":{"customer_id":"cust_1"}}"#;
        let record = service
            .process_webhook("manual", headers("secret", body), body)
            .await
            .unwrap();

        assert_eq!(record.status, WebhookStatus::Processed);
        assert_eq!(record.event_type, "customer.created");
        let stored = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(stored.status, PaymentStatus::Pending);
    }

    #[tokio::test]
    async fn payment_failed_webhook_marks_session_failed() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let provider_session_id = session.provider_session_id.clone().unwrap();
        let body = format!(
            r#"{{"id":"evt_failed","type":"payment.failed","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
        );
        service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();

        let stored = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(stored.status, PaymentStatus::Failed);
    }

    #[tokio::test]
    async fn refund_succeeded_webhook_updates_refund_and_payment() {
        let (service, db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        // Persist a pending refund directly so the webhook has a row to update.
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO refunds \
             (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
              reason, created_at, updated_at) \
             VALUES ('refund_pending', ?, NULL, 400, 'USD', 'pending', 'manual_refund_1', \
              NULL, ?, ?)",
        )
        .bind(payment.id.as_str())
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();

        let body = r#"{"id":"evt_refund","type":"refund.succeeded","data":{"provider_refund_id":"manual_refund_1"}}"#;
        let record = service
            .process_webhook(
                "manual",
                headers("secret", body.as_bytes()),
                body.as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(record.status, WebhookStatus::Processed);

        let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
        assert_eq!(refunds.len(), 1);
        assert_eq!(refunds[0].status, RefundStatus::Succeeded);
        assert_eq!(
            service
                .get_payment(payment.id.as_str())
                .await
                .unwrap()
                .status,
            PaymentStatus::PartiallyRefunded
        );
    }

    #[tokio::test]
    async fn cancel_cannot_overwrite_a_session_captured_after_the_read() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();

        // Simulate cancel observing `pending`, then a capture completing
        // before the stale cancel write is issued.
        let stale = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(stale.status, PaymentStatus::Pending);
        let payment = service.capture(session.id.as_str()).await.unwrap();
        assert_eq!(payment.status, PaymentStatus::Captured);

        let error = service
            .transition_session(&stale, PaymentStatus::Canceled, None)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
        let stored = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(stored.status, PaymentStatus::Captured);
    }

    #[tokio::test]
    async fn cancel_conflicts_while_a_capture_claim_is_live() {
        let (service, db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let session = service.create_session(input("manual", 1000)).await.unwrap();
        let now = now_timestamp();
        // Simulate capture() having claimed the session but not yet completed
        // its provider call. The status is still `pending`.
        sqlx::query(
            "UPDATE payment_sessions
             SET data = json_set(data, '$.capture_claim', 'capture:live',
                                 '$.capture_claim_at', ?),
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(session.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

        let error = service
            .cancel_session(session.id.as_str())
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
        let stored = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(stored.status, PaymentStatus::Pending);
        assert!(stored.data.get("capture_claim").is_some());
    }

    #[tokio::test]
    async fn fail_and_cancel_respect_the_state_machine() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();

        let canceled_session = service.create_session(input("manual", 100)).await.unwrap();
        let canceled = service
            .cancel_session(canceled_session.id.as_str())
            .await
            .unwrap();
        assert_eq!(canceled.status, PaymentStatus::Canceled);
        let capture_error = service
            .capture(canceled_session.id.as_str())
            .await
            .unwrap_err();
        assert!(matches!(capture_error, AppError::Conflict(_)));

        let failed_session = service.create_session(input("manual", 100)).await.unwrap();
        let failed = service
            .fail_session(failed_session.id.as_str(), "declined")
            .await
            .unwrap();
        assert_eq!(failed.status, PaymentStatus::Failed);
        assert_eq!(
            failed.data.get("last_error"),
            Some(&serde_json::json!("declined"))
        );

        let captured_session = service.create_session(input("manual", 100)).await.unwrap();
        service.capture(captured_session.id.as_str()).await.unwrap();
        let cancel_error = service
            .cancel_session(captured_session.id.as_str())
            .await
            .unwrap_err();
        assert!(matches!(cancel_error, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn lists_payments_with_order_and_status_filters() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let mut first_input = input("manual", 500);
        first_input.order_id = Some(OrderId::new());
        let first = service.create_session(first_input).await.unwrap();
        service.capture(first.id.as_str()).await.unwrap();

        let mut second_input = input("manual", 700);
        second_input.order_id = Some(OrderId::new());
        let second = service.create_session(second_input).await.unwrap();
        service.capture(second.id.as_str()).await.unwrap();

        let by_order = service
            .list_payments(ListPaymentsFilter::new().with_order(first.order_id.clone().unwrap()))
            .await
            .unwrap();
        assert_eq!(by_order.count, 1);
        assert_eq!(by_order.data[0].amount, 500);

        let captured = service
            .list_payments(ListPaymentsFilter::new().with_status(PaymentStatus::Captured))
            .await
            .unwrap();
        assert_eq!(captured.count, 2);

        let page = service
            .list_payments(ListPaymentsFilter {
                pagination: Pagination::new(Some(1), Some(1)).unwrap(),
                ..ListPaymentsFilter::new()
            })
            .await
            .unwrap();
        assert_eq!(page.data.len(), 1);
        assert_eq!(page.count, 2);
        assert_eq!(page.pagination.limit, 1);

        let empty: PageResult<crate::domain::Payment> = service
            .list_payments(ListPaymentsFilter::new().with_status(PaymentStatus::Refunded))
            .await
            .unwrap();
        assert!(empty.data.is_empty());
    }

    #[tokio::test]
    async fn missing_resources_return_not_found() {
        let (service, _db) = setup().await;
        assert!(matches!(
            service.get_session("missing").await.unwrap_err(),
            AppError::NotFound(_)
        ));
        assert!(matches!(
            service.get_payment("missing").await.unwrap_err(),
            AppError::NotFound(_)
        ));
        assert!(matches!(
            service.get_webhook("missing").await.unwrap_err(),
            AppError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn webhook_unknown_provider_is_rejected() {
        let (service, _db) = setup().await;
        let error = service
            .process_webhook("missing", BTreeMap::new(), b"{}")
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn payment_metadata_is_copied_from_the_session() {
        let (service, _db) = setup().await;
        service.register_provider(manual(Some("secret"))).unwrap();
        let mut metadata = Metadata::empty();
        metadata.insert("source", serde_json::json!("storefront"));
        let mut session_input = input("manual", 990);
        session_input.metadata = metadata;
        let session = service.create_session(session_input).await.unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        assert_eq!(
            payment.metadata.get("source"),
            Some(&serde_json::json!("storefront"))
        );
    }
}
