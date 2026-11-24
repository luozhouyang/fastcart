//! SQLite-backed checkout application service.

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CartId, CheckoutId, CurrencyCode, EventId, normalize_email, now_timestamp,
    validate_email,
};
use serde_json::Value;

use crate::domain::{
    CheckoutCartItem, CheckoutSession, CheckoutStateHistory, CheckoutStatus, CreateCheckoutInput,
    UpdateCheckoutDetailsInput,
};

/// Persistence row for `checkout_sessions`.
#[derive(sqlx::FromRow)]
struct CheckoutRow {
    id: String,
    cart_id: String,
    customer_id: Option<String>,
    email: Option<String>,
    currency_code: String,
    region_id: Option<String>,
    status: String,
    shipping_address: String,
    billing_address: String,
    shipping_method: String,
    totals: String,
    cart_items: String,
    promotion_codes: String,
    payment_session_id: Option<String>,
    order_id: Option<String>,
    idempotency_key: Option<String>,
    expires_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

impl CheckoutRow {
    fn into_domain(self) -> Result<CheckoutSession, AppError> {
        Ok(CheckoutSession {
            id: CheckoutId::from_string(self.id),
            cart_id: CartId::from_string(self.cart_id),
            customer_id: self
                .customer_id
                .map(fastcart_kernel::CustomerId::from_string),
            email: self.email,
            currency_code: self.currency_code,
            region_id: self.region_id,
            status: CheckoutStatus::from_stored(&self.status)?,
            shipping_address: json_value(&self.shipping_address),
            billing_address: json_value(&self.billing_address),
            shipping_method: json_value(&self.shipping_method),
            totals: json_value(&self.totals),
            cart_items: serde_json::from_str(&self.cart_items).map_err(|error| {
                AppError::database(format!("invalid checkout cart item snapshot: {error}"))
            })?,
            promotion_codes: json_string_list(&self.promotion_codes),
            payment_session_id: self
                .payment_session_id
                .map(fastcart_kernel::PaymentSessionId::from_string),
            order_id: self.order_id.map(fastcart_kernel::OrderId::from_string),
            idempotency_key: self.idempotency_key,
            expires_at: self.expires_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// Persistence row for `checkout_state_history`.
#[derive(sqlx::FromRow)]
struct HistoryRow {
    id: String,
    checkout_id: String,
    from_state: Option<String>,
    to_state: String,
    reason: Option<String>,
    created_at: i64,
}

impl HistoryRow {
    fn into_domain(self) -> Result<CheckoutStateHistory, AppError> {
        let from_state = match self.from_state {
            Some(value) => Some(CheckoutStatus::from_stored(&value)?),
            None => None,
        };
        Ok(CheckoutStateHistory {
            id: self.id,
            checkout_id: CheckoutId::from_string(self.checkout_id),
            from_state,
            to_state: CheckoutStatus::from_stored(&self.to_state)?,
            reason: self.reason,
            created_at: self.created_at,
        })
    }
}

/// Checkout session application service.
#[derive(Clone, Debug)]
pub struct CheckoutService {
    db: StoreDatabase,
}

mod lifecycle;
mod placement;
mod transitions;

impl CheckoutService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }
}

async fn insert_history(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    checkout_id: &CheckoutId,
    from_state: Option<CheckoutStatus>,
    to_state: CheckoutStatus,
    reason: Option<&str>,
    created_at: i64,
) -> Result<(), AppError> {
    let history_id = EventId::new();
    sqlx::query(
        "INSERT INTO checkout_state_history
            (id, checkout_id, from_state, to_state, reason, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(history_id.as_str())
    .bind(checkout_id.as_str())
    .bind(from_state.map(CheckoutStatus::as_str))
    .bind(to_state.as_str())
    .bind(reason)
    .bind(created_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

/// Releases a stale checkout session so a retry can create a new one.
///
/// Failed and canceled sessions keep their terminal state; expired active
/// sessions are transitioned to `canceled` and recorded in history. In both
/// cases the idempotency key is cleared so the retry is a new attempt.
async fn cancel_stale_session(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    checkout_id: &str,
    status: &str,
    now: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE checkout_sessions
         SET status = CASE WHEN status IN ('failed', 'canceled') THEN status ELSE 'canceled' END,
             idempotency_key = NULL,
             placement_token = NULL,
             placement_claimed_at = NULL,
             updated_at = ?
         WHERE id = ?",
    )
    .bind(now)
    .bind(checkout_id)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    if status != "failed" && status != "canceled" {
        insert_history(
            tx,
            &CheckoutId::from_string(checkout_id.to_owned()),
            Some(CheckoutStatus::from_stored(status)?),
            CheckoutStatus::Canceled,
            Some("checkout expired"),
            now,
        )
        .await?;
    }
    Ok(())
}

fn map_create_error(error: sqlx::Error) -> AppError {
    if error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
    {
        return AppError::conflict(
            "a checkout session already exists for this cart or idempotency key".to_owned(),
        );
    }
    database_error(error)
}

fn json_text(value: &impl serde::Serialize) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::validation(format!("invalid JSON value: {error}")))
}

fn json_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn json_string_list(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
}

#[cfg(test)]
mod tests;
