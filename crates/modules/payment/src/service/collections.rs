//! Payment collections and combination payment.

//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CheckoutId, CurrencyCode, OrderId, now_timestamp, validate_non_negative_amount,
};
use sqlx::{Sqlite, Transaction};

use crate::domain::{
    CreatePaymentSessionInput, Payment, PaymentCollection, PaymentCollectionId,
    PaymentCollectionStatus, PaymentSession, PaymentStatus,
};

use super::store::*;
use super::*;
/// Conflict describing how far a total is from the collection amount.
///
/// `source` names the numbers being compared, either the sessions that cover
/// the collection or the captures that settled it.
fn collection_totals_error(amount: i64, total: i64, source: &str) -> AppError {
    if total < amount {
        AppError::conflict(format!(
            "payment collection is under-captured by {}: {source} total {total} of {amount}",
            amount - total
        ))
    } else {
        AppError::conflict(format!(
            "payment collection is over-captured by {}: {source} total {total} of {amount}",
            total - amount
        ))
    }
}

/// Conflict describing a session that is not part of a collection.
fn not_in_collection(session_id: &str, collection_id: &str) -> AppError {
    AppError::conflict(format!(
        "payment session {session_id} is not attached to payment collection {collection_id}"
    ))
}

/// Validates that a split may come out of `primary`.
///
/// A split only spends money the session holds, and it leaves the session with
/// a positive amount: the session keeps the rest of the money, and an emptied
/// session could not pay the checkout it was created for.
fn ensure_split_amount(primary: &PaymentSession, amount: i64) -> Result<(), AppError> {
    if amount >= primary.amount {
        return Err(AppError::conflict(format!(
            "cannot split {amount} from payment session {} holding only {}",
            primary.id, primary.amount
        )));
    }
    if !matches!(
        primary.status,
        PaymentStatus::Pending | PaymentStatus::RequiresAction | PaymentStatus::Authorized
    ) {
        return Err(AppError::conflict(format!(
            "cannot split a payment session in status {}",
            primary.status
        )));
    }
    Ok(())
}

/// Loads the collection of a checkout inside an open transaction.
async fn find_collection_for_checkout(
    tx: &mut Transaction<'_, Sqlite>,
    checkout_id: &str,
) -> Result<Option<PaymentCollection>, AppError> {
    let row = sqlx::query_as::<_, PaymentCollectionRow>(
        "SELECT id, checkout_id, order_id, amount, currency_code, status, created_at, \
         updated_at FROM payment_collections WHERE checkout_id = ?",
    )
    .bind(checkout_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(row.map(collection_from_row))
}

/// Loads the collection of an order inside an open transaction.
async fn find_collection_for_order(
    tx: &mut Transaction<'_, Sqlite>,
    order_id: &OrderId,
) -> Result<Option<PaymentCollection>, AppError> {
    let row = sqlx::query_as::<_, PaymentCollectionRow>(
        "SELECT id, checkout_id, order_id, amount, currency_code, status, created_at, \
         updated_at FROM payment_collections WHERE order_id = ?",
    )
    .bind(order_id.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(row.map(collection_from_row))
}

/// Id of the collection a session is attached to, inside an open transaction.
async fn collection_of_session(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<Option<String>, AppError> {
    sqlx::query_scalar("SELECT collection_id FROM payment_collection_sessions WHERE session_id = ?")
        .bind(session_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)
}

impl PaymentService {
    /// Creates the payment collection of a checkout, returning the existing
    /// collection when the checkout already has one.
    ///
    /// The collection is the idempotent handle on the money a checkout owes:
    /// replaying the call for the same checkout returns the stored row, while a
    /// different amount or currency for that checkout is a conflict. The lookup
    /// and the insert share one `BEGIN IMMEDIATE` transaction, so concurrent
    /// creators cannot open two collections for one checkout.
    pub async fn create_collection(
        &self,
        checkout_id: &str,
        amount: i64,
        currency_code: &str,
    ) -> Result<PaymentCollection, AppError> {
        validate_non_negative_amount(amount, "amount")?;
        let currency = CurrencyCode::new(currency_code)?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        if let Some(existing) = find_collection_for_checkout(&mut tx, checkout_id).await? {
            tx.rollback().await.map_err(database_error)?;
            if existing.amount != amount || existing.currency_code != currency.as_str() {
                return Err(AppError::conflict(format!(
                    "payment collection for checkout {checkout_id} already exists with {} {}",
                    existing.amount, existing.currency_code
                )));
            }
            return Ok(existing);
        }
        let collection = PaymentCollection {
            id: PaymentCollectionId::new(),
            checkout_id: Some(CheckoutId::from_string(checkout_id.to_owned())),
            order_id: None,
            amount,
            currency_code: currency.as_str().to_owned(),
            status: PaymentCollectionStatus::Open,
            created_at: now,
            updated_at: now,
        };
        insert_collection(&mut tx, &collection).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(collection)
    }

    /// Creates the payment collection of an order, returning the existing
    /// collection when the order already has one.
    ///
    /// Orders placed by a checkout already hold the collection the checkout
    /// opened, so this is the entry point for money an order owes outside its
    /// checkout: an additional charge raised after placement. The call is
    /// idempotent by the order — replaying it with the same amount and currency
    /// returns the stored row, while a different amount or currency for that
    /// order is a conflict. The lookup and the insert share one
    /// `BEGIN IMMEDIATE` transaction, so concurrent creators cannot open two
    /// collections for one order.
    pub async fn create_collection_for_order(
        &self,
        order_id: &OrderId,
        amount: i64,
        currency_code: &str,
    ) -> Result<PaymentCollection, AppError> {
        validate_non_negative_amount(amount, "amount")?;
        let currency = CurrencyCode::new(currency_code)?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        if let Some(existing) = find_collection_for_order(&mut tx, order_id).await? {
            tx.rollback().await.map_err(database_error)?;
            if existing.amount != amount || existing.currency_code != currency.as_str() {
                return Err(AppError::conflict(format!(
                    "payment collection for order {order_id} already exists with {} {}",
                    existing.amount, existing.currency_code
                )));
            }
            return Ok(existing);
        }
        let collection = PaymentCollection {
            id: PaymentCollectionId::new(),
            checkout_id: None,
            order_id: Some(order_id.clone()),
            amount,
            currency_code: currency.as_str().to_owned(),
            status: PaymentCollectionStatus::Open,
            created_at: now,
            updated_at: now,
        };
        insert_collection(&mut tx, &collection).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(collection)
    }

    /// Associates a payment collection with the order it is the money of.
    ///
    /// Only collections that no order has claimed can be assigned, and an order
    /// holds one collection, so the call is idempotent for the same order and a
    /// conflict for a different one. The lookup, both conflict checks and the
    /// write share one `BEGIN IMMEDIATE` transaction, so two orders cannot
    /// claim one collection and one order cannot claim two.
    pub async fn assign_collection_to_order(
        &self,
        collection_id: &str,
        order_id: &OrderId,
    ) -> Result<PaymentCollection, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let collection = load_collection(&mut tx, collection_id).await?;
        if let Some(current) = collection.order_id.as_ref() {
            tx.rollback().await.map_err(database_error)?;
            if current == order_id {
                return Ok(collection);
            }
            return Err(AppError::conflict(format!(
                "payment collection {collection_id} already belongs to order {current}"
            )));
        }
        // The unique index would reject the write as well, but a caller that
        // picked an order which already holds a collection deserves a conflict
        // instead of a database error.
        if let Some(existing) = find_collection_for_order(&mut tx, order_id).await? {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(format!(
                "order {order_id} already has payment collection {}",
                existing.id
            )));
        }
        sqlx::query("UPDATE payment_collections SET order_id = ?, updated_at = ? WHERE id = ?")
            .bind(order_id.as_str())
            .bind(now_timestamp())
            .bind(collection.id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        let assigned = load_collection(&mut tx, collection_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(assigned)
    }

    /// Returns the collection an order holds, when it has one.
    ///
    /// An order placed by a checkout holds the collection the checkout opened;
    /// money raised later is collected through that same collection.
    pub async fn collection_for_order(
        &self,
        order_id: &OrderId,
    ) -> Result<Option<PaymentCollection>, AppError> {
        let row = sqlx::query_as::<_, PaymentCollectionRow>(
            "SELECT id, checkout_id, order_id, amount, currency_code, status, created_at, \
             updated_at FROM payment_collections WHERE order_id = ?",
        )
        .bind(order_id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(row.map(collection_from_row))
    }

    /// Raises what a collection expects to collect.
    ///
    /// Money raised after the collection was opened — the price difference of a
    /// dearer replacement, say — belongs to the same collection: the order owes
    /// one pot, and capture settles it against one amount. Only an increase is
    /// accepted; lowering the amount would let a collection look settled
    /// without the money ever arriving.
    pub async fn raise_collection_amount(
        &self,
        collection_id: &str,
        additional: i64,
    ) -> Result<PaymentCollection, AppError> {
        if additional <= 0 {
            return Err(AppError::validation(
                "a payment collection top-up must be positive",
            ));
        }
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let collection = load_collection(&mut tx, collection_id).await?;
        let amount = collection
            .amount
            .checked_add(additional)
            .ok_or_else(|| AppError::validation("payment collection amount overflow"))?;
        sqlx::query("UPDATE payment_collections SET amount = ?, updated_at = ? WHERE id = ?")
            .bind(amount)
            .bind(now_timestamp())
            .bind(collection.id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        let raised = load_collection(&mut tx, collection_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(raised)
    }

    /// Attaches a payment session to a payment collection.
    ///
    /// Attaching the same session again is a no-op. A session that is priced in
    /// another currency, or that already pays into another collection, is
    /// rejected: the money of one session may only count once, and only in the
    /// currency the collection is priced in.
    pub async fn attach_session_to_collection(
        &self,
        collection_id: &str,
        session_id: &str,
    ) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let collection = load_collection(&mut tx, collection_id).await?;
        let session = load_session(&mut tx, session_id).await?;
        if session.currency_code != collection.currency_code {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::validation(format!(
                "cannot attach a {} payment session to a {} payment collection",
                session.currency_code, collection.currency_code
            )));
        }
        match collection_of_session(&mut tx, session.id.as_str()).await? {
            Some(owner) if owner == collection.id.as_str() => {
                tx.rollback().await.map_err(database_error)?;
                return Ok(());
            }
            Some(owner) => {
                tx.rollback().await.map_err(database_error)?;
                return Err(AppError::conflict(format!(
                    "payment session {session_id} is already attached to payment collection {owner}"
                )));
            }
            None => {}
        }
        sqlx::query(
            "INSERT INTO payment_collection_sessions (collection_id, session_id) VALUES (?, ?)",
        )
        .bind(collection.id.as_str())
        .bind(session.id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Splits part of a collection's payment onto a new session.
    ///
    /// A combination payment needs the sessions of a collection to cover it
    /// exactly: [`PaymentService::capture_collection`] only settles a collection
    /// whose sessions add up to its amount, so a collection that gained a
    /// session without giving the money up elsewhere could never be captured.
    /// The new session therefore takes `input.amount` out of the session the
    /// caller names — for a checkout, the session it was created with — and the
    /// covered total is unchanged.
    ///
    /// The coverage check, the reduction and the inserts share one
    /// `BEGIN IMMEDIATE` transaction, so concurrent splits cannot spend the same
    /// money twice. The provider session is created before the transaction
    /// (provider calls stay outside database transactions), so a split the
    /// transaction then rejects — for example because a concurrent caller took
    /// the remainder first — leaves a provider session that was never recorded
    /// locally.
    ///
    /// The primary session must hold more than `input.amount`: it keeps the
    /// rest of the money, and a session without an amount left is not a session
    /// the checkout can still be completed with.
    pub async fn split_collection_session(
        &self,
        collection_id: &str,
        primary_session_id: &str,
        input: CreatePaymentSessionInput,
    ) -> Result<PaymentSession, AppError> {
        if input.amount <= 0 {
            return Err(AppError::validation("split amount must be positive"));
        }
        let currency = CurrencyCode::new(input.currency_code.as_str())?;
        let collection = self.get_collection(collection_id).await?;
        if currency.as_str() != collection.currency_code {
            return Err(AppError::validation(format!(
                "cannot split payment collection {collection_id}: it is priced in {}, not {}",
                collection.currency_code,
                currency.as_str()
            )));
        }
        // These checks are repeated inside the transaction below; doing them
        // here keeps an obviously invalid split from opening a provider session
        // that would only be abandoned.
        let primary = self.get_session(primary_session_id).await?;
        if !self
            .list_collection_sessions(collection_id)
            .await?
            .iter()
            .any(|session| session.id.as_str() == primary_session_id)
        {
            return Err(not_in_collection(primary_session_id, collection_id));
        }
        ensure_split_amount(&primary, input.amount)?;

        let prepared = self.prepare_session(input).await?;
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let collection = load_collection(&mut tx, collection_id).await?;
        let primary = load_session(&mut tx, primary_session_id).await?;
        match collection_of_session(&mut tx, primary_session_id).await? {
            Some(owner) if owner == collection.id.as_str() => {}
            _ => {
                tx.rollback().await.map_err(database_error)?;
                return Err(not_in_collection(primary_session_id, collection_id));
            }
        }
        if prepared.session.currency_code != collection.currency_code {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::validation(format!(
                "cannot split payment collection {collection_id}: it is priced in {}, not {}",
                collection.currency_code, prepared.session.currency_code
            )));
        }
        if let Err(error) = ensure_split_amount(&primary, prepared.session.amount) {
            tx.rollback().await.map_err(database_error)?;
            return Err(error);
        }
        sqlx::query("UPDATE payment_sessions SET amount = ?, updated_at = ? WHERE id = ?")
            .bind(primary.amount - prepared.session.amount)
            .bind(now_timestamp())
            .bind(primary.id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        insert_session(&mut tx, &prepared.session).await?;
        insert_attempt(&mut tx, &prepared.attempt).await?;
        sqlx::query(
            "INSERT INTO payment_collection_sessions (collection_id, session_id) VALUES (?, ?)",
        )
        .bind(collection.id.as_str())
        .bind(prepared.session.id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(prepared.into_response())
    }

    /// Captures every session of a collection and verifies the total.
    ///
    /// Each session goes through the normal single-session capture path, so its
    /// idempotency key and concurrency fence still apply; the session captures
    /// are what makes a repeated call return the recorded payments instead of
    /// charging again. The collection only reports success when the captured
    /// amounts add up to its amount. When the sessions do not cover the amount
    /// exactly, the call is rejected before any capture happens, so a
    /// combination payment never charges a part of the money the collection
    /// cannot settle.
    pub async fn capture_collection(&self, collection_id: &str) -> Result<Vec<Payment>, AppError> {
        let collection = self.get_collection(collection_id).await?;
        let sessions = self.list_collection_sessions(collection_id).await?;
        if sessions.is_empty() {
            return Err(AppError::conflict(format!(
                "payment collection {collection_id} has no payment sessions to capture"
            )));
        }
        let covered = sessions
            .iter()
            .try_fold(0_i64, |total, session| total.checked_add(session.amount))
            .ok_or_else(|| AppError::conflict("payment collection amount overflow"))?;
        if covered != collection.amount {
            return Err(collection_totals_error(
                collection.amount,
                covered,
                "sessions",
            ));
        }

        let mut payments = Vec::with_capacity(sessions.len());
        let mut failure = None;
        for session in &sessions {
            match self.capture(session.id.as_str()).await {
                Ok(payment) => payments.push(payment),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        // The stored status is only a cache of the captured amounts: refresh it
        // even when a capture failed, so it cannot keep claiming the collection
        // is untouched while money has already moved.
        self.refresh_collection_status(collection_id).await;
        if let Some(error) = failure {
            return Err(error);
        }

        let captured = payments
            .iter()
            .try_fold(0_i64, |total, payment| total.checked_add(payment.amount))
            .ok_or_else(|| AppError::conflict("payment collection amount overflow"))?;
        if captured != collection.amount {
            return Err(collection_totals_error(
                collection.amount,
                captured,
                "captures",
            ));
        }
        Ok(payments)
    }

    /// Derives the collection status from the amounts actually captured.
    pub async fn collection_status(
        &self,
        collection_id: &str,
    ) -> Result<PaymentCollectionStatus, AppError> {
        let collection = self.get_collection(collection_id).await?;
        let captured = self.collection_captured_total(&collection).await?;
        Ok(PaymentCollectionStatus::from_captured(
            collection.amount,
            captured,
        ))
    }

    /// Loads a payment collection.
    pub async fn get_collection(&self, collection_id: &str) -> Result<PaymentCollection, AppError> {
        let row = sqlx::query_as::<_, PaymentCollectionRow>(
            "SELECT id, checkout_id, order_id, amount, currency_code, status, created_at, \
             updated_at FROM payment_collections WHERE id = ?",
        )
        .bind(collection_id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        row.map(collection_from_row)
            .ok_or_else(|| AppError::not_found("payment collection", collection_id))
    }

    /// Loads the collection of a checkout, when the checkout has one.
    pub async fn collection_for_checkout(
        &self,
        checkout_id: &str,
    ) -> Result<Option<PaymentCollection>, AppError> {
        let row = sqlx::query_as::<_, PaymentCollectionRow>(
            "SELECT id, checkout_id, order_id, amount, currency_code, status, created_at, \
             updated_at FROM payment_collections WHERE checkout_id = ?",
        )
        .bind(checkout_id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(row.map(collection_from_row))
    }

    /// Lists the payment sessions attached to a collection, oldest first.
    pub async fn list_collection_sessions(
        &self,
        collection_id: &str,
    ) -> Result<Vec<PaymentSession>, AppError> {
        let rows = sqlx::query_as::<_, PaymentSessionRow>(
            "SELECT s.id, s.cart_id, s.checkout_id, s.order_id, s.provider, \
             s.provider_session_id, s.amount, s.currency_code, s.status, s.data, s.created_at, \
             s.updated_at FROM payment_sessions s \
             JOIN payment_collection_sessions pcs ON pcs.session_id = s.id \
             WHERE pcs.collection_id = ? \
             ORDER BY s.created_at ASC, s.id ASC",
        )
        .bind(collection_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows.into_iter().map(session_from_row).collect())
    }

    /// Amount captured against the sessions of a collection.
    async fn collection_captured_total(
        &self,
        collection: &PaymentCollection,
    ) -> Result<i64, AppError> {
        // The payment a session recorded is what was captured for it; a session
        // that was never captured has no payment identifier in its data.
        let captured: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(p.amount), 0) FROM payment_collection_sessions pcs \
             JOIN payment_sessions s ON s.id = pcs.session_id \
             JOIN payments p ON p.id = json_extract(s.data, '$.payment_id') \
             WHERE pcs.collection_id = ?",
        )
        .bind(collection.id.as_str())
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(captured)
    }

    /// Recomputes and stores the cached collection status.
    ///
    /// Best effort: `collection_status` derives the live value, so a failed
    /// cache write is logged instead of failing a capture that already moved
    /// money.
    async fn refresh_collection_status(&self, collection_id: &str) {
        let refreshed = async {
            let collection = self.get_collection(collection_id).await?;
            let status = PaymentCollectionStatus::from_captured(
                collection.amount,
                self.collection_captured_total(&collection).await?,
            );
            if status != collection.status {
                sqlx::query(
                    "UPDATE payment_collections SET status = ?, updated_at = ? WHERE id = ?",
                )
                .bind(status.as_str())
                .bind(now_timestamp())
                .bind(collection_id)
                .execute(self.db.pool())
                .await
                .map_err(database_error)?;
            }
            Ok::<(), AppError>(())
        }
        .await;
        if let Err(error) = refreshed {
            tracing::warn!(
                collection_id,
                error = %error,
                "failed to refresh the payment collection status"
            );
        }
    }
}
