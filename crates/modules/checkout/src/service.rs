//! SQLite-backed checkout application service.

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CartId, CheckoutId, CurrencyCode, EventId, normalize_email, now_timestamp,
    validate_email,
};
use serde_json::Value;

use crate::domain::{
    CheckoutSession, CheckoutStateHistory, CheckoutStatus, CreateCheckoutInput,
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
    status: String,
    shipping_address: String,
    billing_address: String,
    shipping_method: String,
    totals: String,
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
            status: CheckoutStatus::from_stored(&self.status)?,
            shipping_address: json_value(&self.shipping_address),
            billing_address: json_value(&self.billing_address),
            shipping_method: json_value(&self.shipping_method),
            totals: json_value(&self.totals),
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

impl CheckoutService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

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

        if let Some(key) = idempotency_key.as_deref() {
            let existing = sqlx::query_scalar::<_, String>(
                "SELECT id FROM checkout_sessions WHERE idempotency_key = ?",
            )
            .bind(key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            if let Some(existing) = existing {
                tx.commit().await.map_err(database_error)?;
                return self.get(&CheckoutId::from_string(existing)).await;
            }
        }

        let cart_id = input.cart_id.as_str().to_owned();
        let existing_cart =
            sqlx::query_scalar::<_, String>("SELECT id FROM checkout_sessions WHERE cart_id = ?")
                .bind(&cart_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
        if existing_cart.is_some() {
            return Err(AppError::conflict(
                "a checkout session already exists for this cart".to_owned(),
            ));
        }

        let id = CheckoutId::new();
        sqlx::query(
            "INSERT INTO checkout_sessions (
                id, cart_id, customer_id, email, currency_code, status,
                shipping_address, billing_address, shipping_method, totals,
                promotion_codes, payment_session_id, order_id, idempotency_key,
                expires_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(&cart_id)
        .bind(input.customer_id.as_ref().map(|value| value.as_str()))
        .bind(email.as_deref())
        .bind(&currency)
        .bind(CheckoutStatus::Created.as_str())
        .bind(json_text(&input.shipping_address)?)
        .bind(json_text(&input.billing_address)?)
        .bind(json_text(&input.shipping_method)?)
        .bind(json_text(&input.totals)?)
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

    /// Loads a checkout session by identifier.
    pub async fn get(&self, id: &CheckoutId) -> Result<CheckoutSession, AppError> {
        let row = sqlx::query_as::<_, CheckoutRow>(
            "SELECT id, cart_id, customer_id, email, currency_code, status,
                    shipping_address, billing_address, shipping_method, totals,
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

    /// Loads the checkout session for a cart, if one exists.
    pub async fn get_by_cart(&self, cart_id: &CartId) -> Result<Option<CheckoutSession>, AppError> {
        let row = sqlx::query_as::<_, CheckoutRow>(
            "SELECT id, cart_id, customer_id, email, currency_code, status,
                    shipping_address, billing_address, shipping_method, totals,
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

    /// Loads the checkout session created with an idempotency key, if any.
    pub async fn get_by_idempotency_key(
        &self,
        key: &str,
    ) -> Result<Option<CheckoutSession>, AppError> {
        let row = sqlx::query_as::<_, CheckoutRow>(
            "SELECT id, cart_id, customer_id, email, currency_code, status,
                    shipping_address, billing_address, shipping_method, totals,
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

    /// Lists checkout sessions in a given state, newest first.
    pub async fn list_by_status(
        &self,
        status: CheckoutStatus,
    ) -> Result<Vec<CheckoutSession>, AppError> {
        let rows = sqlx::query_as::<_, CheckoutRow>(
            "SELECT id, cart_id, customer_id, email, currency_code, status,
                    shipping_address, billing_address, shipping_method, totals,
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

    /// Updates mutable checkout details while the session is still editable.
    pub async fn update_details(
        &self,
        id: &CheckoutId,
        input: UpdateCheckoutDetailsInput,
    ) -> Result<CheckoutSession, AppError> {
        let session = self.get(id).await?;
        if !session.status.can_update_details() {
            return Err(AppError::conflict(format!(
                "checkout in state {} cannot be updated",
                session.status.as_str()
            )));
        }

        let email = match input.email {
            Some(value) => {
                validate_email(&value, "email")?;
                Some(normalize_email(&value))
            }
            None => session.email.clone(),
        };
        let shipping_address = input
            .shipping_address
            .unwrap_or_else(|| session.shipping_address.clone());
        let billing_address = input
            .billing_address
            .unwrap_or_else(|| session.billing_address.clone());
        let shipping_method = input
            .shipping_method
            .unwrap_or_else(|| session.shipping_method.clone());
        let totals = input.totals.unwrap_or_else(|| session.totals.clone());
        let promotion_codes = input
            .promotion_codes
            .unwrap_or_else(|| session.promotion_codes.clone());

        let now = now_timestamp();
        sqlx::query(
            "UPDATE checkout_sessions
             SET email = ?, shipping_address = ?, billing_address = ?,
                 shipping_method = ?, totals = ?, promotion_codes = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(email.as_deref())
        .bind(json_text(&shipping_address)?)
        .bind(json_text(&billing_address)?)
        .bind(json_text(&shipping_method)?)
        .bind(json_text(&totals)?)
        .bind(json_text(&promotion_codes)?)
        .bind(now)
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;

        self.get(id).await
    }

    /// Applies a state transition, enforcing the pure state machine.
    ///
    /// Every accepted transition appends a history row. `reason` is recorded
    /// for `Failed` and `Canceled` transitions and stored as a note otherwise.
    pub async fn transition(
        &self,
        id: &CheckoutId,
        to: CheckoutStatus,
        reason: Option<&str>,
    ) -> Result<CheckoutSession, AppError> {
        let session = self.get(id).await?;
        if !session.status.can_transition_to(to) {
            return Err(AppError::conflict(format!(
                "cannot transition checkout from {} to {}",
                session.status.as_str(),
                to.as_str()
            )));
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query("UPDATE checkout_sessions SET status = ?, updated_at = ? WHERE id = ?")
            .bind(to.as_str())
            .bind(now)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        insert_history(&mut tx, id, Some(session.status), to, reason, now).await?;
        tx.commit().await.map_err(database_error)?;

        self.get(id).await
    }

    /// Attaches a payment session and moves `ReadyForPayment` to `PaymentPending`.
    pub async fn attach_payment_session(
        &self,
        id: &CheckoutId,
        payment_session_id: &fastcart_kernel::PaymentSessionId,
    ) -> Result<CheckoutSession, AppError> {
        let session = self.get(id).await?;
        if session.status != CheckoutStatus::ReadyForPayment {
            return Err(AppError::conflict(format!(
                "payment session can only be attached in ready_for_payment, found {}",
                session.status.as_str()
            )));
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "UPDATE checkout_sessions
             SET status = ?, payment_session_id = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(CheckoutStatus::PaymentPending.as_str())
        .bind(payment_session_id.as_str())
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        insert_history(
            &mut tx,
            id,
            Some(CheckoutStatus::ReadyForPayment),
            CheckoutStatus::PaymentPending,
            Some("payment session attached"),
            now,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get(id).await
    }

    /// Records the placed order and moves `PaymentConfirmed` to `OrderPlaced`.
    pub async fn mark_order_placed(
        &self,
        id: &CheckoutId,
        order_id: &fastcart_kernel::OrderId,
    ) -> Result<CheckoutSession, AppError> {
        let session = self.get(id).await?;
        if session.status != CheckoutStatus::PaymentConfirmed {
            return Err(AppError::conflict(format!(
                "order can only be placed in payment_confirmed, found {}",
                session.status.as_str()
            )));
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "UPDATE checkout_sessions SET status = ?, order_id = ?, updated_at = ? WHERE id = ?",
        )
        .bind(CheckoutStatus::OrderPlaced.as_str())
        .bind(order_id.as_str())
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        insert_history(
            &mut tx,
            id,
            Some(CheckoutStatus::PaymentConfirmed),
            CheckoutStatus::OrderPlaced,
            Some("order placed"),
            now,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get(id).await
    }

    /// Marks every expirable session whose expiry is at or before `now` as failed.
    ///
    /// Returns the number of sessions expired.
    pub async fn expire_due(&self, now: i64) -> Result<u64, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let due = sqlx::query_as::<_, (String, String)>(
            "SELECT id, status FROM checkout_sessions
             WHERE expires_at IS NOT NULL
               AND expires_at <= ?
               AND status IN ('created', 'validating', 'ready_for_payment', 'payment_pending')",
        )
        .bind(now)
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;

        for (id, status) in &due {
            let from = CheckoutStatus::from_stored(status)?;
            sqlx::query("UPDATE checkout_sessions SET status = ?, updated_at = ? WHERE id = ?")
                .bind(CheckoutStatus::Failed.as_str())
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
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
mod tests {
    use fastcart_infrastructure::StoreDatabase;
    use fastcart_kernel::{CartId, CheckoutId, OrderId, PaymentSessionId, StoreId, now_timestamp};
    use serde_json::json;

    use super::CheckoutService;
    use crate::domain::{CheckoutStatus, CreateCheckoutInput, UpdateCheckoutDetailsInput};

    async fn service() -> CheckoutService {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        CheckoutService::new(db)
    }

    fn input(cart: CartId) -> CreateCheckoutInput {
        CreateCheckoutInput {
            cart_id: cart,
            customer_id: None,
            email: Some("buyer@example.com".to_owned()),
            currency_code: "USD".to_owned(),
            shipping_address: json!({"city": "Berlin"}),
            billing_address: json!({"city": "Berlin"}),
            shipping_method: json!({"name": "Standard"}),
            totals: json!({"total": 2500}),
            promotion_codes: vec!["SAVE10".to_owned()],
            idempotency_key: None,
            expires_in_seconds: 3_600,
        }
    }

    #[tokio::test]
    async fn creates_session_and_history() {
        let service = service().await;
        let cart = CartId::new();
        let session = service.create(input(cart.clone())).await.unwrap();

        assert_eq!(session.status, CheckoutStatus::Created);
        assert_eq!(session.cart_id, cart);
        assert_eq!(session.currency_code, "USD");
        assert_eq!(session.promotion_codes, vec!["SAVE10".to_owned()]);
        assert!(session.expires_at.is_some());

        let history = service.list_history(&session.id).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].to_state, CheckoutStatus::Created);
        assert_eq!(history[0].from_state, None);
    }

    #[tokio::test]
    async fn create_is_idempotent_by_key() {
        let service = service().await;
        let cart = CartId::new();
        let mut request = input(cart);
        request.idempotency_key = Some("checkout-key".to_owned());

        let first = service.create(request.clone()).await.unwrap();
        let second = service.create(request).await.unwrap();

        assert_eq!(first.id, second.id);
        let found = service
            .get_by_idempotency_key("checkout-key")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, first.id);
    }

    #[tokio::test]
    async fn rejects_a_second_session_for_the_same_cart() {
        let service = service().await;
        let cart = CartId::new();
        service.create(input(cart.clone())).await.unwrap();

        let error = service.create(input(cart)).await.unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn accepts_the_documented_transition_chain() {
        let service = service().await;
        let session = service.create(input(CartId::new())).await.unwrap();

        service
            .transition(&session.id, CheckoutStatus::Validating, Some("validating"))
            .await
            .unwrap();
        let ready = service
            .transition(&session.id, CheckoutStatus::ReadyForPayment, Some("ready"))
            .await
            .unwrap();
        assert_eq!(ready.status, CheckoutStatus::ReadyForPayment);

        let payment_session = PaymentSessionId::new();
        let pending = service
            .attach_payment_session(&session.id, &payment_session)
            .await
            .unwrap();
        assert_eq!(pending.status, CheckoutStatus::PaymentPending);
        assert_eq!(pending.payment_session_id, Some(payment_session));

        service
            .transition(&session.id, CheckoutStatus::PaymentConfirmed, None)
            .await
            .unwrap();
        let order = OrderId::new();
        let placed = service
            .mark_order_placed(&session.id, &order)
            .await
            .unwrap();
        assert_eq!(placed.status, CheckoutStatus::OrderPlaced);
        assert_eq!(placed.order_id, Some(order));

        let history = service.list_history(&session.id).await.unwrap();
        assert_eq!(history.len(), 6);
        assert_eq!(
            history.last().unwrap().to_state,
            CheckoutStatus::OrderPlaced
        );
    }

    #[tokio::test]
    async fn rejects_illegal_transitions() {
        let service = service().await;
        let session = service.create(input(CartId::new())).await.unwrap();

        let error = service
            .transition(&session.id, CheckoutStatus::PaymentConfirmed, None)
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

        service
            .transition(&session.id, CheckoutStatus::Failed, Some("bad data"))
            .await
            .unwrap();
        let terminal = service
            .transition(&session.id, CheckoutStatus::Validating, None)
            .await
            .unwrap_err();
        assert!(matches!(terminal, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn payment_session_requires_ready_state() {
        let service = service().await;
        let session = service.create(input(CartId::new())).await.unwrap();

        let error = service
            .attach_payment_session(&session.id, &PaymentSessionId::new())
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn order_placement_requires_confirmed_payment() {
        let service = service().await;
        let session = service.create(input(CartId::new())).await.unwrap();

        let error = service
            .mark_order_placed(&session.id, &OrderId::new())
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn updates_details_only_in_editable_states() {
        let service = service().await;
        let session = service.create(input(CartId::new())).await.unwrap();

        let updated = service
            .update_details(
                &session.id,
                UpdateCheckoutDetailsInput {
                    email: Some("new@example.com".to_owned()),
                    totals: Some(json!({"total": 9000})),
                    promotion_codes: Some(vec!["NEW".to_owned()]),
                    ..UpdateCheckoutDetailsInput::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.email.as_deref(), Some("new@example.com"));
        assert_eq!(updated.totals, json!({"total": 9000}));
        assert_eq!(updated.promotion_codes, vec!["NEW".to_owned()]);

        service
            .transition(&session.id, CheckoutStatus::Validating, None)
            .await
            .unwrap();
        service
            .transition(&session.id, CheckoutStatus::ReadyForPayment, None)
            .await
            .unwrap();
        service
            .transition(&session.id, CheckoutStatus::PaymentPending, None)
            .await
            .unwrap();

        let error = service
            .update_details(
                &session.id,
                UpdateCheckoutDetailsInput {
                    email: Some("other@example.com".to_owned()),
                    ..UpdateCheckoutDetailsInput::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn expires_due_sessions() {
        let service = service().await;
        let expiring = service.create(input(CartId::new())).await.unwrap();
        let mut long_lived_input = input(CartId::new());
        long_lived_input.expires_in_seconds = 86_400;
        let long_lived = service.create(long_lived_input).await.unwrap();

        let count = service.expire_due(now_timestamp() + 7_200).await.unwrap();
        assert_eq!(count, 1);

        let expired = service.get(&expiring.id).await.unwrap();
        assert_eq!(expired.status, CheckoutStatus::Failed);
        let history = service.list_history(&expired.id).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].reason.as_deref(), Some("expired"));

        let still_open = service.get(&long_lived.id).await.unwrap();
        assert_eq!(still_open.status, CheckoutStatus::Created);
    }

    #[tokio::test]
    async fn validates_currency_email_and_expiry() {
        let service = service().await;

        let mut request = input(CartId::new());
        request.currency_code = "US".to_owned();
        assert!(service.create(request).await.is_err());

        let mut request = input(CartId::new());
        request.email = Some("not-an-email".to_owned());
        assert!(service.create(request).await.is_err());

        let mut request = input(CartId::new());
        request.expires_in_seconds = 0;
        assert!(service.create(request).await.is_err());
    }

    #[tokio::test]
    async fn missing_session_is_not_found() {
        let service = service().await;
        let error = service.get(&CheckoutId::new()).await.unwrap_err();
        assert!(error.is_not_found());
        assert!(service.get_by_cart(&CartId::new()).await.unwrap().is_none());
    }
}
