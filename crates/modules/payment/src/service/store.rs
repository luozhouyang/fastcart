//! Persistence rows, converters and loaders.

//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use fastcart_infrastructure::{append_outbox_event, database_error};
use fastcart_kernel::{
    AppError, CartId, CheckoutId, DomainEventEnvelope, OrderId, PaymentAttemptId, PaymentId,
    PaymentSessionId, RefundId, WebhookId,
};
use sqlx::{Sqlite, Transaction};

use crate::domain::{
    GiftCard, GiftCardStatus, Payment, PaymentAttempt, PaymentCollection, PaymentCollectionId,
    PaymentCollectionStatus, PaymentSession, PaymentStatus, PaymentWebhookRecord, Refund,
    RefundStatus, WebhookStatus,
};

/// Loads a payment session inside an open transaction.
pub(super) async fn load_session(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<PaymentSession, AppError> {
    let row = sqlx::query_as::<_, PaymentSessionRow>(
        "SELECT id, cart_id, checkout_id, order_id, provider, provider_session_id, \
         amount, currency_code, status, data, created_at, updated_at \
         FROM payment_sessions WHERE id = ?",
    )
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    row.map(session_from_row)
        .ok_or_else(|| AppError::not_found("payment session", session_id))
}

/// Loads a payment collection inside an open transaction.
pub(super) async fn load_collection(
    tx: &mut Transaction<'_, Sqlite>,
    collection_id: &str,
) -> Result<PaymentCollection, AppError> {
    let row = sqlx::query_as::<_, PaymentCollectionRow>(
        "SELECT id, checkout_id, order_id, amount, currency_code, status, created_at, \
         updated_at FROM payment_collections WHERE id = ?",
    )
    .bind(collection_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    row.map(collection_from_row)
        .ok_or_else(|| AppError::not_found("payment collection", collection_id))
}

/// Appends one payment lifecycle event inside the caller's transaction.
#[allow(clippy::too_many_arguments)]
pub(super) async fn append_payment_event(
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

#[derive(sqlx::FromRow)]
pub(super) struct PaymentSessionRow {
    pub(super) id: String,
    pub(super) cart_id: Option<String>,
    pub(super) checkout_id: Option<String>,
    pub(super) order_id: Option<String>,
    pub(super) provider: String,
    pub(super) provider_session_id: Option<String>,
    pub(super) amount: i64,
    pub(super) currency_code: String,
    pub(super) status: String,
    pub(super) data: String,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

#[derive(sqlx::FromRow)]
pub(super) struct PaymentCollectionRow {
    pub(super) id: String,
    pub(super) checkout_id: Option<String>,
    pub(super) order_id: Option<String>,
    pub(super) amount: i64,
    pub(super) currency_code: String,
    pub(super) status: String,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

#[derive(sqlx::FromRow)]
pub(super) struct PaymentAttemptRow {
    pub(super) id: String,
    pub(super) payment_session_id: String,
    pub(super) amount: i64,
    pub(super) currency_code: String,
    pub(super) status: String,
    pub(super) provider_reference: Option<String>,
    pub(super) error: Option<String>,
    pub(super) created_at: i64,
}

#[derive(sqlx::FromRow)]
pub(super) struct PaymentRow {
    pub(super) id: String,
    pub(super) order_id: Option<String>,
    pub(super) provider: String,
    pub(super) provider_payment_id: Option<String>,
    pub(super) amount: i64,
    pub(super) currency_code: String,
    pub(super) status: String,
    pub(super) captured_at: Option<i64>,
    pub(super) metadata: String,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

#[derive(Clone, sqlx::FromRow)]
pub(super) struct RefundRow {
    pub(super) id: String,
    pub(super) payment_id: Option<String>,
    pub(super) order_id: Option<String>,
    pub(super) amount: i64,
    pub(super) currency_code: String,
    pub(super) status: String,
    pub(super) provider_refund_id: Option<String>,
    pub(super) idempotency_key: Option<String>,
    pub(super) reason: Option<String>,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

#[derive(sqlx::FromRow)]
pub(super) struct GiftCardRow {
    pub(super) id: String,
    pub(super) code: String,
    pub(super) currency_code: String,
    pub(super) initial_amount_minor: i64,
    pub(super) balance_minor: i64,
    pub(super) status: String,
    pub(super) metadata: String,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

#[derive(sqlx::FromRow)]
pub(super) struct PaymentWebhookRow {
    pub(super) id: String,
    pub(super) provider: String,
    pub(super) provider_event_id: String,
    pub(super) event_type: String,
    pub(super) payload: String,
    pub(super) status: String,
    pub(super) attempts: i64,
    pub(super) processed_at: Option<i64>,
    pub(super) last_error: Option<String>,
    pub(super) created_at: i64,
}

pub(super) fn session_from_row(row: PaymentSessionRow) -> PaymentSession {
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

pub(super) fn collection_from_row(row: PaymentCollectionRow) -> PaymentCollection {
    PaymentCollection {
        id: PaymentCollectionId::from_string(row.id),
        checkout_id: row.checkout_id.map(CheckoutId::from_string),
        order_id: row.order_id.map(OrderId::from_string),
        amount: row.amount,
        currency_code: row.currency_code,
        status: PaymentCollectionStatus::parse(&row.status)
            .unwrap_or(PaymentCollectionStatus::Open),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub(super) fn attempt_from_row(row: PaymentAttemptRow) -> PaymentAttempt {
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

pub(super) fn payment_from_row(row: PaymentRow) -> Payment {
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

pub(super) fn refund_from_row(row: RefundRow) -> Refund {
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

pub(super) fn gift_card_from_row(row: GiftCardRow) -> GiftCard {
    GiftCard {
        id: row.id,
        code: row.code,
        currency_code: row.currency_code,
        initial_amount_minor: row.initial_amount_minor,
        balance_minor: row.balance_minor,
        status: GiftCardStatus::parse(&row.status).unwrap_or(GiftCardStatus::Disabled),
        metadata: fastcart_kernel::Metadata::from_json_str(&row.metadata),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub(super) fn webhook_from_row(row: PaymentWebhookRow) -> PaymentWebhookRecord {
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

pub(super) async fn insert_session(
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

pub(super) async fn insert_collection(
    tx: &mut Transaction<'_, Sqlite>,
    collection: &PaymentCollection,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO payment_collections \
         (id, checkout_id, order_id, amount, currency_code, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(collection.id.as_str())
    .bind(collection.checkout_id.as_ref().map(CheckoutId::as_str))
    .bind(collection.order_id.as_ref().map(OrderId::as_str))
    .bind(collection.amount)
    .bind(&collection.currency_code)
    .bind(collection.status.as_str())
    .bind(collection.created_at)
    .bind(collection.updated_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

pub(super) async fn insert_attempt(
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

pub(super) async fn insert_payment(
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

pub(super) async fn insert_refund(
    tx: &mut Transaction<'_, Sqlite>,
    refund: &Refund,
) -> Result<(), AppError> {
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
