//! Registration, login and session lifecycle.

//! SQLite-backed customer application service.
//!
//! Every operation is implicitly scoped to the store owning the injected
//! [`StoreDatabase`]. Credentials are hashed with Argon2id and session tokens
//! are looked up exclusively by their SHA-256 digest.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CustomerId, CustomerSessionId, normalize_email, now_timestamp, validate_email,
};
use sqlx::FromRow;

use crate::domain::{
    AuthenticatedCustomerSession, CUSTOMER_SESSION_TTL_SECONDS, CreatedCustomerAuthToken,
    CreatedCustomerSession, Customer, CustomerAuthTokenType, CustomerSession, CustomerStatus,
    LoginInput, MAX_NAME_LENGTH, MAX_PHONE_LENGTH, RegisterCustomerInput,
};

use super::rows::*;
use super::*;
#[derive(Debug, FromRow)]
struct CustomerSessionRow {
    id: String,
    customer_id: String,
    expires_at: i64,
    revoked_at: Option<i64>,
    last_used_at: i64,
    created_at: i64,
}

impl CustomerSessionRow {
    fn into_session(self) -> CustomerSession {
        CustomerSession {
            id: CustomerSessionId::from_string(self.id),
            customer_id: CustomerId::from_string(self.customer_id),
            expires_at: self.expires_at,
            revoked_at: self.revoked_at,
            last_used_at: self.last_used_at,
            created_at: self.created_at,
        }
    }
}

impl CustomerService {
    /// Registers a customer account and stores an Argon2id password hash.
    ///
    /// A guest profile with the same email is upgraded to an account instead of
    /// being duplicated. An existing credential always yields a conflict.
    pub async fn register(&self, input: RegisterCustomerInput) -> Result<Customer, AppError> {
        let (customer, _) = self.register_internal(input, false).await?;
        Ok(customer)
    }

    /// Registers a customer and creates its first session in one transaction.
    ///
    /// The returned token is shown exactly once; only its SHA-256 digest is
    /// persisted. Registration and automatic login therefore succeed or fail
    /// together.
    pub async fn register_with_session(
        &self,
        input: RegisterCustomerInput,
    ) -> Result<CreatedCustomerSession, AppError> {
        let (_, session) = self.register_internal(input, true).await?;
        session.ok_or_else(|| AppError::internal("customer session was not created"))
    }

    async fn register_internal(
        &self,
        input: RegisterCustomerInput,
        create_session: bool,
    ) -> Result<(Customer, Option<CreatedCustomerSession>), AppError> {
        let email = normalize_email(&input.email);
        validate_email(&email, "email")?;
        validate_new_password(&input.password)?;
        let first_name = clean_optional(input.first_name, "first_name", MAX_NAME_LENGTH)?;
        let last_name = clean_optional(input.last_name, "last_name", MAX_NAME_LENGTH)?;
        let phone = clean_optional(input.phone, "phone", MAX_PHONE_LENGTH)?;

        let credential_exists: Option<String> =
            sqlx::query_scalar("SELECT id FROM customer_credentials WHERE email = ?")
                .bind(&email)
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if credential_exists.is_some() {
            return Err(AppError::conflict(
                "an account with this email already exists",
            ));
        }

        let existing = sqlx::query_as::<_, ExistingCustomerRow>(
            "SELECT id, has_account, deleted_at FROM customers WHERE email = ?",
        )
        .bind(&email)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        if let Some(existing) = &existing
            && (existing.deleted_at.is_some() || existing.has_account != 0)
        {
            return Err(AppError::conflict(
                "an account with this email already exists",
            ));
        }

        let password_hash = hash_password_offloaded(&input.password).await?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;

        let customer_id = if let Some(existing) = existing {
            let customer_id = CustomerId::from_string(existing.id);
            sqlx::query(
                "UPDATE customers
                 SET has_account = 1, first_name = COALESCE(?, first_name),
                     last_name = COALESCE(?, last_name), phone = COALESCE(?, phone),
                     updated_at = ?
                 WHERE id = ?",
            )
            .bind(first_name.as_deref())
            .bind(last_name.as_deref())
            .bind(phone.as_deref())
            .bind(now)
            .bind(customer_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            customer_id
        } else {
            let customer_id = CustomerId::new();
            sqlx::query(
                "INSERT INTO customers
                 (id, email, first_name, last_name, phone, has_account, status, metadata,
                  created_at, updated_at, deleted_at)
                 VALUES (?, ?, ?, ?, ?, 1, ?, '{}', ?, ?, NULL)",
            )
            .bind(customer_id.as_str())
            .bind(&email)
            .bind(&first_name)
            .bind(&last_name)
            .bind(&phone)
            .bind(CustomerStatus::Active.as_str())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|error| {
                conflict_or_database(error, "a customer with this email already exists")
            })?;
            customer_id
        };

        sqlx::query(
            "INSERT INTO customer_credentials
             (id, customer_id, email, password_hash, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("cred_{}", random_hex(16)))
        .bind(customer_id.as_str())
        .bind(&email)
        .bind(&password_hash)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            conflict_or_database(error, "an account with this email already exists")
        })?;
        let sql =
            format!("SELECT {CUSTOMER_COLUMNS} FROM customers WHERE id = ? AND deleted_at IS NULL");
        let customer = sqlx::query_as::<_, CustomerRow>(&sql)
            .bind(customer_id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("customer", &customer_id))?
            .into_customer();

        let session = if create_session {
            let token = random_hex(32);
            let csrf_token = random_hex(24);
            let session_id = CustomerSessionId::new();
            let expires_at = now.saturating_add(CUSTOMER_SESSION_TTL_SECONDS);
            sqlx::query(
                "INSERT INTO customer_sessions
                 (id, customer_id, token_hash, expires_at, revoked_at, last_used_at, created_at)
                 VALUES (?, ?, ?, ?, NULL, ?, ?)",
            )
            .bind(session_id.as_str())
            .bind(customer.id.as_str())
            .bind(hash_token(&token))
            .bind(expires_at)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            Some(CreatedCustomerSession {
                token,
                csrf_token,
                session_id,
                customer: customer.clone(),
                expires_at,
            })
        } else {
            None
        };

        tx.commit().await.map_err(database_error)?;
        Ok((customer, session))
    }

    /// Authenticates a customer and creates a store-scoped session.
    ///
    /// The returned token is shown exactly once; only its SHA-256 digest is
    /// persisted.
    pub async fn login(
        &self,
        email: &str,
        password: &str,
    ) -> Result<CreatedCustomerSession, AppError> {
        let email = normalize_email(email);
        let row = sqlx::query_as::<_, CredentialCustomerRow>(
            "SELECT c.id, c.email, c.first_name, c.last_name, c.phone, c.has_account,
                    c.status, c.email_verified_at, c.metadata, c.created_at, c.updated_at,
                    cc.password_hash
             FROM customers c
             JOIN customer_credentials cc ON cc.customer_id = c.id
             WHERE cc.email = ? AND c.deleted_at IS NULL",
        )
        .bind(&email)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            return Err(AppError::Unauthorized);
        };
        if !verify_password_offloaded(&row.password_hash, password).await? {
            return Err(AppError::Unauthorized);
        }
        let customer = row.into_customer();
        if customer.status != CustomerStatus::Active {
            return Err(AppError::Forbidden);
        }
        self.create_session(customer).await
    }

    /// Convenience wrapper around [`CustomerService::login`].
    pub async fn login_with_input(
        &self,
        input: LoginInput,
    ) -> Result<CreatedCustomerSession, AppError> {
        self.login(&input.email, &input.password).await
    }

    /// Validates a session token, refreshes its last-used timestamp and returns
    /// the authenticated customer.
    ///
    /// Revoked, expired and soft-deleted sessions are rejected.
    pub async fn authenticate_session(
        &self,
        token: &str,
    ) -> Result<AuthenticatedCustomerSession, AppError> {
        if token.len() < 32 {
            return Err(AppError::Unauthorized);
        }
        let row = sqlx::query_as::<_, SessionCustomerRow>(
            "SELECT s.id AS session_id, s.expires_at,
                    c.id, c.email, c.first_name, c.last_name, c.phone, c.has_account,
                    c.status, c.email_verified_at, c.metadata, c.created_at, c.updated_at
             FROM customer_sessions s
             JOIN customers c ON c.id = s.customer_id
             WHERE s.token_hash = ? AND s.revoked_at IS NULL AND c.deleted_at IS NULL",
        )
        .bind(hash_token(token))
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            return Err(AppError::Unauthorized);
        };
        let now = now_timestamp();
        if row.expires_at <= now {
            return Err(AppError::Unauthorized);
        }

        let session_id = CustomerSessionId::from_string(row.session_id.clone());
        let customer = row.into_customer();
        if customer.status != CustomerStatus::Active {
            return Err(AppError::Forbidden);
        }

        // Fail when the session was revoked or expired between the read and
        // this refresh instead of resurrecting it.
        let refreshed = sqlx::query(
            "UPDATE customer_sessions SET last_used_at = ?
             WHERE id = ? AND revoked_at IS NULL AND expires_at > ?",
        )
        .bind(now)
        .bind(session_id.as_str())
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if refreshed.rows_affected() != 1 {
            return Err(AppError::Unauthorized);
        }

        Ok(AuthenticatedCustomerSession {
            customer,
            session_id,
        })
    }

    /// Validates a session token without refreshing its last-used timestamp.
    ///
    /// This is the read-only counterpart of [`Self::authenticate_session`],
    /// intended for rendering contexts (for example SSR) that need to
    /// personalize a page without producing a side effect. Unknown, revoked or
    /// expired sessions and sessions owned by soft-deleted or disabled
    /// customers resolve to `None`. Database failures are still surfaced as
    /// [`AppError`].
    pub async fn session_customer(&self, token: &str) -> Result<Option<Customer>, AppError> {
        if token.len() < 32 {
            return Ok(None);
        }
        let row = sqlx::query_as::<_, SessionCustomerRow>(
            "SELECT s.id AS session_id, s.expires_at,
                    c.id, c.email, c.first_name, c.last_name, c.phone, c.has_account,
                    c.status, c.email_verified_at, c.metadata, c.created_at, c.updated_at
             FROM customer_sessions s
             JOIN customers c ON c.id = s.customer_id
             WHERE s.token_hash = ? AND s.revoked_at IS NULL AND c.deleted_at IS NULL",
        )
        .bind(hash_token(token))
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        if row.expires_at <= now_timestamp() {
            return Ok(None);
        }
        let customer = row.into_customer();
        if customer.status != CustomerStatus::Active {
            return Ok(None);
        }
        Ok(Some(customer))
    }

    /// Revokes the session identified by a token. Unknown tokens are ignored.
    pub async fn logout(&self, token: &str) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE customer_sessions SET revoked_at = ?
             WHERE token_hash = ? AND revoked_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(hash_token(token))
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }

    /// Revokes every active session owned by a customer.
    pub async fn revoke_all_sessions(&self, customer_id: &CustomerId) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE customer_sessions SET revoked_at = ?
             WHERE customer_id = ? AND revoked_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(customer_id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }

    /// Changes a customer's password and revokes all active sessions.
    ///
    /// The write is conditioned on the hash that was verified so a concurrent
    /// reset cannot be silently overwritten.
    pub async fn change_password(
        &self,
        customer_id: &CustomerId,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), AppError> {
        validate_new_password(new_password)?;

        let stored: Option<String> = sqlx::query_scalar(
            "SELECT password_hash FROM customer_credentials WHERE customer_id = ?",
        )
        .bind(customer_id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        let stored =
            stored.ok_or_else(|| AppError::not_found("customer credentials", customer_id))?;
        if !verify_password_offloaded(&stored, current_password).await? {
            return Err(AppError::Unauthorized);
        }

        let password_hash = hash_password_offloaded(new_password).await?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let updated = sqlx::query(
            "UPDATE customer_credentials SET password_hash = ?, updated_at = ?
             WHERE customer_id = ? AND password_hash = ?",
        )
        .bind(&password_hash)
        .bind(now)
        .bind(customer_id.as_str())
        .bind(&stored)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() != 1 {
            return Err(AppError::Unauthorized);
        }
        sqlx::query(
            "UPDATE customer_sessions SET revoked_at = ?
             WHERE customer_id = ? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(customer_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Inserts a single-use authentication token and invalidates older unused
    /// tokens of the same type for the customer.
    pub(super) async fn issue_auth_token(
        &self,
        customer_id: &CustomerId,
        token_type: CustomerAuthTokenType,
        ttl_seconds: i64,
    ) -> Result<CreatedCustomerAuthToken, AppError> {
        let token = random_hex(32);
        let now = now_timestamp();
        let expires_at = now.saturating_add(ttl_seconds);

        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE customer_auth_tokens SET used_at = ?
             WHERE customer_id = ? AND token_type = ? AND used_at IS NULL",
        )
        .bind(now)
        .bind(customer_id.as_str())
        .bind(token_type.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        sqlx::query(
            "INSERT INTO customer_auth_tokens
             (id, customer_id, token_hash, token_type, expires_at, used_at, created_at)
             VALUES (?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(format!("cat_{}", random_hex(16)))
        .bind(customer_id.as_str())
        .bind(hash_token(&token))
        .bind(token_type.as_str())
        .bind(expires_at)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;

        Ok(CreatedCustomerAuthToken { token, expires_at })
    }

    /// Returns the persisted sessions of a customer, excluding token digests.
    pub async fn list_sessions(
        &self,
        customer_id: &CustomerId,
    ) -> Result<Vec<CustomerSession>, AppError> {
        let rows = sqlx::query_as::<_, CustomerSessionRow>(
            "SELECT id, customer_id, expires_at, revoked_at, last_used_at, created_at
             FROM customer_sessions WHERE customer_id = ? ORDER BY created_at DESC",
        )
        .bind(customer_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows
            .into_iter()
            .map(CustomerSessionRow::into_session)
            .collect())
    }

    async fn create_session(&self, customer: Customer) -> Result<CreatedCustomerSession, AppError> {
        let token = random_hex(32);
        let csrf_token = random_hex(24);
        let session_id = CustomerSessionId::new();
        let now = now_timestamp();
        let expires_at = now.saturating_add(CUSTOMER_SESSION_TTL_SECONDS);

        sqlx::query(
            "INSERT INTO customer_sessions
             (id, customer_id, token_hash, expires_at, revoked_at, last_used_at, created_at)
             VALUES (?, ?, ?, ?, NULL, ?, ?)",
        )
        .bind(session_id.as_str())
        .bind(customer.id.as_str())
        .bind(hash_token(&token))
        .bind(expires_at)
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;

        Ok(CreatedCustomerSession {
            token,
            csrf_token,
            session_id,
            customer,
            expires_at,
        })
    }
}
