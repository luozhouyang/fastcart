//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use std::collections::BTreeMap;
use std::sync::Arc;

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CartId, CheckoutId, CurrencyCode, OrderId, PageResult, PaymentAttemptId, PaymentId,
    PaymentSessionId, RefundId, WebhookId, now_timestamp, validate_metadata,
    validate_non_negative_amount,
};
use serde_json::Value;
use sqlx::{QueryBuilder, Sqlite, Transaction};

use crate::domain::{
    CreatePaymentSessionInput, ListPaymentsFilter, Payment, PaymentAttempt, PaymentSession,
    PaymentStatus, PaymentWebhookRecord, Refund, RefundStatus, WebhookStatus,
};
use crate::provider::{
    CaptureRequest, CreateProviderSession, PaymentProvider, ProviderRegistry, RefundRequest,
    VerifiedWebhook,
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
        let provider_session = provider
            .create_session(CreateProviderSession {
                reference: session_id.as_str().to_owned(),
                amount: input.amount,
                currency_code: currency_code.clone(),
                metadata: input.metadata.clone(),
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
            data: input.metadata,
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
        Ok(session)
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

        let result = match provider
            .capture(CaptureRequest {
                provider_session_id,
                amount: session.amount,
                currency_code: session.currency_code.clone(),
                metadata: session.data.clone(),
            })
            .await
        {
            Ok(result) => result,
            Err(error) => {
                let message = error.to_string();
                self.record_session_failure(&session, &message).await?;
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
             WHERE id = ? AND status IN ('pending', 'requires_action', 'authorized')",
        )
        .bind(PaymentStatus::Captured.as_str())
        .bind(data.to_json_string())
        .bind(now)
        .bind(session.id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            return self.payment_for_session(&session).await;
        }
        insert_payment(&mut tx, &payment).await?;
        insert_attempt(&mut tx, &attempt).await?;
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
        let result = sqlx::query("UPDATE payments SET order_id = ?, updated_at = ? WHERE id = ?")
            .bind(order_id.as_str())
            .bind(now_timestamp())
            .bind(payment_id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("payment", payment_id));
        }
        self.get_payment(payment_id).await
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
    pub async fn refund(
        &self,
        payment_id: &str,
        amount: i64,
        reason: Option<String>,
    ) -> Result<Refund, AppError> {
        self.refund_with_idempotency(payment_id, amount, reason, None)
            .await
    }

    /// Refunds part or all of a captured payment with an optional provider
    /// idempotency key.
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
        if !matches!(
            payment.status,
            PaymentStatus::Captured | PaymentStatus::PartiallyRefunded
        ) {
            return Err(AppError::conflict(format!(
                "cannot refund a payment in status {}",
                payment.status
            )));
        }
        let provider_payment_id = payment
            .provider_payment_id
            .clone()
            .ok_or_else(|| AppError::conflict("payment is missing a provider payment id"))?;

        let refunded = self.refunded_total(&payment).await?;
        let requested_total = refunded
            .checked_add(amount)
            .ok_or_else(|| AppError::conflict("refund amount overflow"))?;
        if requested_total > payment.amount {
            return Err(AppError::conflict(
                "refund amount exceeds the remaining captured amount",
            ));
        }

        let provider = self
            .providers
            .get(&payment.provider)
            .ok_or_else(|| AppError::not_found("payment provider", &payment.provider))?;
        let result = provider
            .refund(RefundRequest {
                provider_payment_id,
                amount,
                currency_code: payment.currency_code.clone(),
                reason: reason.clone(),
                idempotency_key,
                metadata: payment.metadata.clone(),
            })
            .await?;

        let now = now_timestamp();
        let refund = Refund {
            id: RefundId::new(),
            payment_id: Some(payment.id.clone()),
            order_id: payment.order_id.clone(),
            amount,
            currency_code: payment.currency_code.clone(),
            status: result.status,
            provider_refund_id: Some(result.provider_refund_id),
            reason,
            created_at: now,
            updated_at: now,
        };

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let current_total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM refunds \
             WHERE payment_id = ? AND status != 'failed'",
        )
        .bind(payment.id.as_str())
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
        let status = if total_after >= payment.amount {
            PaymentStatus::Refunded
        } else {
            PaymentStatus::PartiallyRefunded
        };
        if !payment.status.can_transition_to(status) {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(format!(
                "cannot transition a payment from {} to {status}",
                payment.status
            )));
        }
        insert_refund(&mut tx, &refund).await?;
        sqlx::query("UPDATE payments SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(now)
            .bind(payment.id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(refund)
    }

    /// Verifies and persists a webhook event, applying known event types.
    ///
    /// Duplicate `(provider, provider_event_id)` deliveries return the existing
    /// record without reprocessing. Processing failures are stored on the
    /// record, and the webhook is still acknowledged.
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
            return Ok(existing);
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
            {
                if let Some(existing) = self.find_webhook(provider_id, &event.event_id).await? {
                    return Ok(existing);
                }
            }
            return Err(database_error(error));
        }

        let (status, last_error) = match self.apply_webhook_event(provider_id, &event).await {
            Ok(()) => (WebhookStatus::Processed, None),
            Err(error) => (WebhookStatus::Failed, Some(error.to_string())),
        };
        sqlx::query(
            "UPDATE payment_webhooks \
             SET status = ?, attempts = attempts + 1, processed_at = ?, last_error = ? \
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(now_timestamp())
        .bind(last_error)
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_webhook(id.as_str()).await
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
             provider_refund_id, reason, created_at, updated_at \
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

    async fn refunded_total(&self, payment: &Payment) -> Result<i64, AppError> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM refunds \
             WHERE payment_id = ? AND status != 'failed'",
        )
        .bind(payment.id.as_str())
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(total)
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
        sqlx::query(
            "UPDATE payment_sessions SET status = ?, data = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(data.to_json_string())
        .bind(now)
        .bind(session.id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        insert_attempt(&mut tx, &attempt).await?;
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
        sqlx::query(
            "UPDATE payment_sessions SET status = 'failed', data = ?, updated_at = ? \
             WHERE id = ? AND status IN ('pending', 'requires_action', 'authorized')",
        )
        .bind(data.to_json_string())
        .bind(now)
        .bind(session.id.as_str())
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
            "refund.succeeded" => self.apply_refund_event(provider, &event.data).await,
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
        if let Some(provider_payment_id) = provider_payment_id.as_deref() {
            if succeeded {
                sqlx::query(
                    "UPDATE payments SET status = 'captured', \
                     captured_at = COALESCE(captured_at, ?), updated_at = ? \
                     WHERE provider = ? AND provider_payment_id = ? \
                     AND status IN ('pending', 'requires_action', 'authorized', 'captured')",
                )
                .bind(now)
                .bind(now)
                .bind(provider)
                .bind(provider_payment_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            } else {
                sqlx::query(
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
            }
        }
        if let Some(provider_session_id) = provider_session_id.as_deref() {
            if succeeded {
                sqlx::query(
                    "UPDATE payment_sessions SET status = 'captured', updated_at = ? \
                     WHERE provider = ? AND provider_session_id = ? \
                     AND status IN ('pending', 'requires_action', 'authorized')",
                )
                .bind(now)
                .bind(provider)
                .bind(provider_session_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            } else {
                sqlx::query(
                    "UPDATE payment_sessions SET status = 'failed', updated_at = ? \
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
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    async fn apply_refund_event(&self, provider: &str, data: &Value) -> Result<(), AppError> {
        let provider_refund_id = string_field(data, &["provider_refund_id", "refund_id"]);
        let provider_payment_id = string_field(data, &["provider_payment_id", "payment_id"]);
        if provider_refund_id.is_none() && provider_payment_id.is_none() {
            return Ok(());
        }
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let mut payment_id: Option<String> = None;
        if let Some(provider_refund_id) = provider_refund_id.as_deref() {
            payment_id = sqlx::query_scalar::<_, Option<String>>(
                "SELECT payment_id FROM refunds WHERE provider_refund_id = ? LIMIT 1",
            )
            .bind(provider_refund_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .flatten();
            sqlx::query(
                "UPDATE refunds SET status = 'succeeded', updated_at = ? \
                 WHERE provider_refund_id = ?",
            )
            .bind(now)
            .bind(provider_refund_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }
        if payment_id.is_none() {
            if let Some(provider_payment_id) = provider_payment_id.as_deref() {
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
        }
        if let Some(payment_id) = payment_id {
            let amount: i64 = sqlx::query_scalar("SELECT amount FROM payments WHERE id = ?")
                .bind(&payment_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
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
            } else {
                PaymentStatus::PartiallyRefunded
            };
            sqlx::query(
                "UPDATE payments SET status = ?, updated_at = ? WHERE id = ? \
                 AND status IN ('captured', 'partially_refunded', 'refunded')",
            )
            .bind(status.as_str())
            .bind(now)
            .bind(&payment_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
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

#[derive(sqlx::FromRow)]
struct RefundRow {
    id: String,
    payment_id: Option<String>,
    order_id: Option<String>,
    amount: i64,
    currency_code: String,
    status: String,
    provider_refund_id: Option<String>,
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
          reason, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(refund.id.as_str())
    .bind(refund.payment_id.as_ref().map(PaymentId::as_str))
    .bind(refund.order_id.as_ref().map(OrderId::as_str))
    .bind(refund.amount)
    .bind(&refund.currency_code)
    .bind(refund.status.as_str())
    .bind(refund.provider_refund_id.as_deref())
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
        fail_refund: bool,
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
                fail_refund: false,
                create_calls: AtomicUsize::new(0),
                capture_calls: AtomicUsize::new(0),
                refund_calls: AtomicUsize::new(0),
            }
        }

        fn failing_capture(id: &str) -> Self {
            Self {
                fail_capture: true,
                ..Self::new(id)
            }
        }

        fn failing_refund(id: &str) -> Self {
            Self {
                fail_refund: true,
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
            if self.fail_capture {
                return Err(PaymentError::Provider("capture failed".to_owned()));
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
    async fn capture_failure_marks_session_and_attempt_failed() {
        let (service, _db) = setup().await;
        service
            .register_provider(Arc::new(TestProvider::failing_capture("broken")))
            .unwrap();
        let session = service.create_session(input("broken", 1000)).await.unwrap();

        let error = service.capture(session.id.as_str()).await.unwrap_err();
        assert!(matches!(error, AppError::ExternalService(_)));
        let stored = service.get_session(session.id.as_str()).await.unwrap();
        assert_eq!(stored.status, PaymentStatus::Failed);
        let attempts = service.list_attempts(session.id.as_str()).await.unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[1].status, PaymentStatus::Failed);
        assert!(attempts[1].error.is_some());
    }

    #[tokio::test]
    async fn refund_failure_returns_external_service_and_keeps_payment() {
        let (service, _db) = setup().await;
        service
            .register_provider(Arc::new(TestProvider::failing_refund("broken")))
            .unwrap();
        let session = service.create_session(input("broken", 1000)).await.unwrap();
        let payment = service.capture(session.id.as_str()).await.unwrap();

        let error = service
            .refund(payment.id.as_str(), 100, None)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::ExternalService(_)));
        assert!(
            service
                .list_refunds(payment.id.as_str())
                .await
                .unwrap()
                .is_empty()
        );
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
