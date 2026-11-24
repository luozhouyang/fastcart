//! Platform authentication: password credentials, admin sessions, API tokens and RBAC.

use std::collections::BTreeSet;

use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};
use fastcart_kernel::{SessionId, StoreId, UserId, normalize_email, now_timestamp, validate_email};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, Row};

use crate::error::{PlatformError, PlatformResult};
use crate::repository::PlatformStore;
use crate::user::{CreateUserInput, User, UserStatus};

/// Default admin session lifetime in seconds (7 days).
pub const DEFAULT_SESSION_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;
/// Maximum API token lifetime in seconds (365 days).
pub const MAX_API_TOKEN_TTL_SECONDS: i64 = 365 * 24 * 60 * 60;

mod sessions;
mod tokens;

/// A newly created admin session. The plaintext token is returned once.
#[derive(Clone, Debug)]
pub struct CreatedSession {
    /// Opaque session token for the cookie.
    pub token: String,
    /// Double-submit CSRF token.
    pub csrf_token: String,
    /// Session identifier.
    pub session_id: SessionId,
    /// Expiry timestamp.
    pub expires_at: i64,
    /// Authenticated user.
    pub user: User,
}

/// A validated admin session.
#[derive(Clone, Debug)]
pub struct AuthenticatedSession {
    /// Authenticated user.
    pub user: User,
    /// Session identifier.
    pub session_id: SessionId,
    /// Expiry timestamp.
    pub expires_at: i64,
}

/// A persisted API token record. The secret is never stored or returned again.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiToken {
    /// Token identifier.
    pub id: String,
    /// Owning user.
    pub user_id: UserId,
    /// Optional store scope.
    pub store_id: Option<StoreId>,
    /// Human-readable name.
    pub name: String,
    /// Display prefix of the token.
    pub prefix: String,
    /// Granted scopes.
    pub scopes: Vec<String>,
    /// Expiry timestamp.
    pub expires_at: Option<i64>,
    /// Revocation timestamp.
    pub revoked_at: Option<i64>,
    /// Last use timestamp.
    pub last_used_at: Option<i64>,
    /// Creation timestamp.
    pub created_at: i64,
}

/// A newly created API token including the one-time plaintext value.
#[derive(Clone, Debug)]
pub struct CreatedApiToken {
    /// Full token, shown exactly once.
    pub token: String,
    /// Persisted record.
    pub record: ApiToken,
}

/// An authenticated API token.
#[derive(Clone, Debug)]
pub struct AuthenticatedToken {
    /// Token identifier.
    pub token_id: String,
    /// Owning user.
    pub user_id: UserId,
    /// Optional store scope.
    pub store_id: Option<StoreId>,
    /// Granted scopes.
    pub scopes: Vec<String>,
}

#[derive(Debug, FromRow)]
struct SessionRow {
    session_id: String,
    expires_at: i64,
    user_id: String,
    email: String,
    first_name: Option<String>,
    last_name: Option<String>,
    status: String,
    email_verified_at: Option<i64>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl SessionRow {
    fn into_session(self) -> AuthenticatedSession {
        AuthenticatedSession {
            user: User {
                id: UserId::from_string(self.user_id),
                email: self.email,
                first_name: self.first_name,
                last_name: self.last_name,
                status: UserStatus::from_db(&self.status),
                email_verified_at: self.email_verified_at,
                metadata: fastcart_kernel::Metadata::from_json_str(&self.metadata),
                created_at: self.created_at,
                updated_at: self.updated_at,
            },
            session_id: SessionId::from_string(self.session_id),
            expires_at: self.expires_at,
        }
    }
}

/// Authentication service backed by the platform database.
#[derive(Clone, Debug)]
pub struct AuthService {
    platform: PlatformStore,
    session_ttl_seconds: i64,
}

impl AuthService {
    /// Creates an authentication service.
    #[must_use]
    pub fn new(platform: PlatformStore) -> Self {
        Self {
            platform,
            session_ttl_seconds: DEFAULT_SESSION_TTL_SECONDS,
        }
    }
}

impl AuthService {
    /// Overrides the session lifetime.
    #[must_use]
    pub fn with_session_ttl(mut self, seconds: i64) -> Self {
        self.session_ttl_seconds = seconds.max(60);
        self
    }
}

impl AuthService {
    /// Sets the acting user recorded on platform mutations performed here.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.platform = self.platform.with_audit_actor(actor_id);
        self
    }
}

impl AuthService {
    /// Returns the platform store.
    #[must_use]
    pub fn platform(&self) -> &PlatformStore {
        &self.platform
    }
}

impl AuthService {
    /// Hashes a password with Argon2id.
    ///
    /// This is a CPU- and memory-bound call. Async callers must use
    /// [`AuthService::hash_password_offloaded`] so the work runs on the
    /// blocking pool instead of stalling a runtime worker.
    pub fn hash_password(password: &str) -> PlatformResult<String> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|error| PlatformError::PasswordHash(error.to_string()))
    }
}

impl AuthService {
    /// Verifies a password against a stored Argon2 hash.
    ///
    /// This is a CPU- and memory-bound call. Async callers must use
    /// [`AuthService::verify_password_offloaded`] so the work runs on the
    /// blocking pool instead of stalling a runtime worker.
    #[must_use]
    pub fn verify_password(stored_hash: &str, password: &str) -> bool {
        PasswordHash::new(stored_hash).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
    }
}

impl AuthService {
    /// Hashes a password on the blocking pool.
    ///
    /// Argon2 deliberately costs tens of milliseconds; running it inline on a
    /// runtime worker (the login endpoint is unauthenticated and can be hit
    /// concurrently) starves every other task on that worker.
    async fn hash_password_offloaded(password: &str) -> PlatformResult<String> {
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || Self::hash_password(&password))
            .await
            .map_err(password_task_error)?
    }
}

impl AuthService {
    /// Verifies a password on the blocking pool.
    ///
    /// Only the wrong-password case is `Ok(false)`; a failed or panicked task
    /// surfaces as an error so callers never report it as invalid credentials.
    async fn verify_password_offloaded(stored_hash: &str, password: &str) -> PlatformResult<bool> {
        let stored_hash = stored_hash.to_owned();
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || Self::verify_password(&stored_hash, &password))
            .await
            .map_err(password_task_error)
    }
}

fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buffer);
    hex::encode(buffer)
}

/// Maps a hashing task that failed to join into an internal error.
///
/// The task panicked or the runtime shut down, so the result says nothing
/// about the password and must not be reported as invalid credentials.
fn password_task_error(error: tokio::task::JoinError) -> PlatformError {
    PlatformError::PasswordHash(format!("password task failed: {error}"))
}

/// SHA-256 token digest used for session and API token lookup.
#[must_use]
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests;
