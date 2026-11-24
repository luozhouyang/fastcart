//! Details updates and state transitions.

use super::*;

impl CheckoutService {
    /// Updates mutable checkout details while the session is still editable.
    ///
    /// Only supplied fields are written, guarded by the status observed inside
    /// the same immediate transaction, so concurrent partial updates cannot
    /// overwrite each other with stale snapshots.
    pub async fn update_details(
        &self,
        id: &CheckoutId,
        input: UpdateCheckoutDetailsInput,
    ) -> Result<CheckoutSession, AppError> {
        let email = match input.email {
            Some(value) => {
                validate_email(&value, "email")?;
                Some(normalize_email(&value))
            }
            None => None,
        };
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let status =
            sqlx::query_scalar::<_, String>("SELECT status FROM checkout_sessions WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
        let Some(status) = status else {
            return Err(AppError::not_found("checkout", id.as_str()));
        };
        let status = CheckoutStatus::from_stored(&status)?;
        if !status.can_update_details() {
            return Err(AppError::conflict(format!(
                "checkout in state {} cannot be updated",
                status.as_str()
            )));
        }

        let result = sqlx::query(
            "UPDATE checkout_sessions
             SET email = COALESCE(?, email),
                 shipping_address = COALESCE(?, shipping_address),
                 billing_address = COALESCE(?, billing_address),
                 shipping_method = COALESCE(?, shipping_method),
                 totals = COALESCE(?, totals),
                 promotion_codes = COALESCE(?, promotion_codes),
                 updated_at = ?
             WHERE id = ? AND status = ?",
        )
        .bind(email.as_deref())
        .bind(input.shipping_address.as_ref().map(json_text).transpose()?)
        .bind(input.billing_address.as_ref().map(json_text).transpose()?)
        .bind(input.shipping_method.as_ref().map(json_text).transpose()?)
        .bind(input.totals.as_ref().map(json_text).transpose()?)
        .bind(input.promotion_codes.as_ref().map(json_text).transpose()?)
        .bind(now)
        .bind(id.as_str())
        .bind(status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict(
                "checkout changed while updating details".to_owned(),
            ));
        }
        tx.commit().await.map_err(database_error)?;
        self.get(id).await
    }
}

impl CheckoutService {
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
        // A terminal session must never stay fenced by a completion claim:
        // the next attempt may be a fresh checkout for the same cart.
        let sql = if matches!(to, CheckoutStatus::Failed | CheckoutStatus::Canceled) {
            "UPDATE checkout_sessions
             SET status = ?, updated_at = ?,
                 placement_token = NULL, placement_claimed_at = NULL
             WHERE id = ? AND status = ?"
        } else {
            "UPDATE checkout_sessions SET status = ?, updated_at = ?
             WHERE id = ? AND status = ?"
        };
        let updated = sqlx::query(sql)
            .bind(to.as_str())
            .bind(now)
            .bind(id.as_str())
            .bind(session.status.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict("checkout status changed concurrently"));
        }
        insert_history(&mut tx, id, Some(session.status), to, reason, now).await?;
        tx.commit().await.map_err(database_error)?;

        self.get(id).await
    }
}
