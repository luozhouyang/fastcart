//! Payment sessions and attempts.

//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CurrencyCode, Metadata, OrderId, PaymentAttemptId, PaymentSessionId, now_timestamp,
    validate_metadata, validate_non_negative_amount,
};
use serde_json::Value;

use crate::domain::{
    CreatePaymentSessionInput, Payment, PaymentAttempt, PaymentSession, PaymentStatus,
};
use crate::provider::CreateProviderSession;

use super::store::*;
use super::*;
/// A session whose provider-side counterpart exists, awaiting its local rows.
///
/// The provider data a session was created with is split in two: the stored
/// record keeps everything that may be read back later, while the caller gets
/// the values — a client secret, for example — that only the storefront needs.
pub(super) struct PreparedSession {
    pub(super) session: PaymentSession,
    pub(super) attempt: PaymentAttempt,
    pub(super) returned_data: Metadata,
}

impl PreparedSession {
    /// The session as its creator sees it.
    pub(super) fn into_response(self) -> PaymentSession {
        PaymentSession {
            data: self.returned_data,
            ..self.session
        }
    }
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

impl PaymentService {
    /// Creates a provider session and persists the local session and attempt.
    pub async fn create_session(
        &self,
        input: CreatePaymentSessionInput,
    ) -> Result<PaymentSession, AppError> {
        let prepared = self.prepare_session(input).await?;
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        insert_session(&mut tx, &prepared.session).await?;
        insert_attempt(&mut tx, &prepared.attempt).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(prepared.into_response())
    }

    /// Creates the provider-side session and the local records for it.
    ///
    /// Provider calls stay outside database transactions, so a caller that has
    /// to decide something in a transaction first prepares the session here and
    /// then writes the returned records in its own transaction.
    pub(super) async fn prepare_session(
        &self,
        input: CreatePaymentSessionInput,
    ) -> Result<PreparedSession, AppError> {
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
        Ok(PreparedSession {
            session,
            attempt,
            returned_data,
        })
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

    pub(super) async fn transition_session(
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

    /// Returns the captured payment for a session, when one exists.
    pub async fn payment_for_session_id(&self, session_id: &str) -> Result<Payment, AppError> {
        let session = self.get_session(session_id).await?;
        self.payment_for_session(&session).await
    }

    pub(super) async fn payment_for_session(
        &self,
        session: &PaymentSession,
    ) -> Result<Payment, AppError> {
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
}
