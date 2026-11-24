//! Email verification and password reset.

//! SQLite-backed customer application service.
//!
//! Every operation is implicitly scoped to the store owning the injected
//! [`StoreDatabase`]. Credentials are hashed with Argon2id and session tokens
//! are looked up exclusively by their SHA-256 digest.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, CustomerId, normalize_email, now_timestamp, validate_email};
use sqlx::FromRow;

use crate::domain::{
    CreatedCustomerAuthToken, Customer, CustomerAuthTokenType, EMAIL_VERIFICATION_TTL_SECONDS,
    PASSWORD_RESET_TTL_SECONDS,
};

use super::*;

#[derive(Debug, FromRow)]
struct AuthTokenRow {
    id: String,
    customer_id: String,
    token_type: String,
    expires_at: i64,
    used_at: Option<i64>,
}

impl CustomerService {
    /// Creates an email-verification token, invalidating any earlier unused
    /// verification token for the same customer.
    ///
    /// The plaintext token is returned exactly once and must be delivered by
    /// email; only its SHA-256 digest is persisted.
    pub async fn request_email_verification(
        &self,
        customer_id: &CustomerId,
    ) -> Result<CreatedCustomerAuthToken, AppError> {
        let customer = self.get_customer(customer_id).await?;
        if customer.email_verified_at.is_some() {
            return Err(AppError::conflict("email is already verified"));
        }

        self.issue_auth_token(
            customer_id,
            CustomerAuthTokenType::EmailVerification,
            EMAIL_VERIFICATION_TTL_SECONDS,
        )
        .await
    }

    /// Consumes an email-verification token and marks the email as verified.
    ///
    /// Expired, already used and wrong-type tokens are rejected. The token is
    /// consumed with a conditional update so concurrent confirmations cannot
    /// both succeed.
    pub async fn verify_email(&self, token: &str) -> Result<Customer, AppError> {
        if token.is_empty() {
            return Err(invalid_auth_token());
        }
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let row = sqlx::query_as::<_, AuthTokenRow>(
            "SELECT id, customer_id, token_type, expires_at, used_at
             FROM customer_auth_tokens WHERE token_hash = ?",
        )
        .bind(hash_token(token))
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(invalid_auth_token)?;

        if row.token_type != CustomerAuthTokenType::EmailVerification.as_str()
            || row.used_at.is_some()
            || row.expires_at <= now
        {
            return Err(invalid_auth_token());
        }

        let consumed = sqlx::query(
            "UPDATE customer_auth_tokens SET used_at = ?
             WHERE id = ? AND used_at IS NULL AND expires_at > ?",
        )
        .bind(now)
        .bind(&row.id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if consumed.rows_affected() != 1 {
            return Err(invalid_auth_token());
        }

        let updated = sqlx::query(
            "UPDATE customers SET email_verified_at = ?, updated_at = ?
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(&row.customer_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() != 1 {
            return Err(invalid_auth_token());
        }
        tx.commit().await.map_err(database_error)?;

        self.get_customer(&CustomerId::from_string(row.customer_id))
            .await
    }

    /// Creates a password-reset token for an existing account.
    ///
    /// Unknown emails and guest profiles return `None` without creating a
    /// token, so callers can keep their response generic. Any earlier unused
    /// reset token for the account is invalidated.
    pub async fn request_password_reset(
        &self,
        email: &str,
    ) -> Result<Option<CreatedCustomerAuthToken>, AppError> {
        let email = normalize_email(email);
        if validate_email(&email, "email").is_err() {
            return Ok(None);
        }
        let Some(customer) = self.get_customer_by_email(&email).await? else {
            return Ok(None);
        };
        let credential: Option<String> =
            sqlx::query_scalar("SELECT id FROM customer_credentials WHERE customer_id = ?")
                .bind(customer.id.as_str())
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if credential.is_none() {
            return Ok(None);
        }

        self.issue_auth_token(
            &customer.id,
            CustomerAuthTokenType::PasswordReset,
            PASSWORD_RESET_TTL_SECONDS,
        )
        .await
        .map(Some)
    }

    /// Consumes a password-reset token, replaces the credential and
    /// revokes all active sessions in one transaction.
    ///
    /// The new password uses the same policy as registration.
    pub async fn reset_password(
        &self,
        token: &str,
        new_password: &str,
    ) -> Result<Customer, AppError> {
        validate_new_password(new_password)?;
        if token.is_empty() {
            return Err(invalid_auth_token());
        }

        let now = now_timestamp();
        let row = sqlx::query_as::<_, AuthTokenRow>(
            "SELECT id, customer_id, token_type, expires_at, used_at
             FROM customer_auth_tokens WHERE token_hash = ?",
        )
        .bind(hash_token(token))
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(invalid_auth_token)?;
        if row.token_type != CustomerAuthTokenType::PasswordReset.as_str()
            || row.used_at.is_some()
            || row.expires_at <= now
        {
            return Err(invalid_auth_token());
        }

        let password_hash = hash_password_offloaded(new_password).await?;
        let mut tx = self.db.begin().await?;

        let consumed = sqlx::query(
            "UPDATE customer_auth_tokens SET used_at = ?
             WHERE id = ? AND used_at IS NULL AND expires_at > ?",
        )
        .bind(now)
        .bind(&row.id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if consumed.rows_affected() != 1 {
            return Err(invalid_auth_token());
        }

        let customer =
            sqlx::query("UPDATE customers SET updated_at = ? WHERE id = ? AND deleted_at IS NULL")
                .bind(now)
                .bind(&row.customer_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        if customer.rows_affected() != 1 {
            return Err(invalid_auth_token());
        }

        let credential = sqlx::query(
            "UPDATE customer_credentials SET password_hash = ?, updated_at = ?
             WHERE customer_id = ?",
        )
        .bind(&password_hash)
        .bind(now)
        .bind(&row.customer_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if credential.rows_affected() != 1 {
            return Err(invalid_auth_token());
        }

        sqlx::query(
            "UPDATE customer_sessions SET revoked_at = ?
             WHERE customer_id = ? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(&row.customer_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;

        self.get_customer(&CustomerId::from_string(row.customer_id))
            .await
    }
}
