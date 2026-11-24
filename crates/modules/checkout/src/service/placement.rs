//! Placement claims, payment attachment and order completion.

use super::*;

impl CheckoutService {
    /// Attempts to fence a checkout completion with a short-lived lease.
    ///
    /// The claim succeeds only when the session is awaiting completion and has
    /// no live claim. An expired lease can be taken over, which is what lets a
    /// crashed runner's retry resume the work. The returned token must be
    /// passed to [`Self::release_placement`] if the attempt aborts.
    pub async fn claim_placement(
        &self,
        id: &CheckoutId,
        now: i64,
        lease_seconds: i64,
    ) -> Result<Option<String>, AppError> {
        let token = format!("plc_{}", uuid::Uuid::now_v7().simple());
        let expired_before = now.saturating_sub(lease_seconds.max(0));
        let result = sqlx::query(
            "UPDATE checkout_sessions
             SET placement_token = ?, placement_claimed_at = ?, updated_at = ?
             WHERE id = ?
               AND status IN ('payment_pending', 'payment_confirmed')
               AND (
                    placement_token IS NULL
                    OR placement_claimed_at IS NULL
                    OR placement_claimed_at <= ?
               )",
        )
        .bind(&token)
        .bind(now)
        .bind(now)
        .bind(id.as_str())
        .bind(expired_before)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok((result.rows_affected() == 1).then_some(token))
    }
}

impl CheckoutService {
    /// Returns true while `token` still holds the completion claim.
    ///
    /// Compensation uses this fence to avoid undoing work that a runner which
    /// took over an expired lease is still performing.
    pub async fn holds_placement(&self, id: &CheckoutId, token: &str) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM checkout_sessions
             WHERE id = ? AND placement_token = ?",
        )
        .bind(id.as_str())
        .bind(token)
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(count == 1)
    }
}

impl CheckoutService {
    /// Releases a completion claim held by `token`.
    ///
    /// The update is conditional on the token so a runner whose lease expired
    /// and was taken over cannot clear the new owner's claim.
    pub async fn release_placement(&self, id: &CheckoutId, token: &str) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE checkout_sessions
             SET placement_token = NULL, placement_claimed_at = NULL, updated_at = ?
             WHERE id = ? AND placement_token = ?",
        )
        .bind(now_timestamp())
        .bind(id.as_str())
        .bind(token)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }
}

impl CheckoutService {
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
        let updated = sqlx::query(
            "UPDATE checkout_sessions
             SET status = ?, payment_session_id = ?, updated_at = ?
             WHERE id = ? AND status = 'ready_for_payment' AND payment_session_id IS NULL",
        )
        .bind(CheckoutStatus::PaymentPending.as_str())
        .bind(payment_session_id.as_str())
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(
                "payment session was attached concurrently",
            ));
        }
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
}

impl CheckoutService {
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
        let updated = sqlx::query(
            "UPDATE checkout_sessions
             SET status = ?, order_id = ?, updated_at = ?,
                 placement_token = NULL, placement_claimed_at = NULL
             WHERE id = ? AND status = 'payment_confirmed' AND order_id IS NULL",
        )
        .bind(CheckoutStatus::OrderPlaced.as_str())
        .bind(order_id.as_str())
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(
                "order was placed concurrently for this checkout",
            ));
        }
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
}

impl CheckoutService {
    /// Marks an order placed only while `token` still owns the placement claim.
    ///
    /// Unlike [`Self::mark_order_placed`], this fences the final write: a runner
    /// whose lease expired and was taken over cannot clear the new owner's
    /// claim or place a competing order.
    pub async fn mark_order_placed_with_claim(
        &self,
        id: &CheckoutId,
        order_id: &fastcart_kernel::OrderId,
        token: &str,
    ) -> Result<CheckoutSession, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE checkout_sessions
             SET status = ?, order_id = ?, updated_at = ?,
                 placement_token = NULL, placement_claimed_at = NULL
             WHERE id = ? AND status = 'payment_confirmed' AND order_id IS NULL
               AND placement_token = ?",
        )
        .bind(CheckoutStatus::OrderPlaced.as_str())
        .bind(order_id.as_str())
        .bind(now)
        .bind(id.as_str())
        .bind(token)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(
                "checkout placement claim was lost while placing the order",
            ));
        }
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
}
