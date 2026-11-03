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

    /// Overrides the session lifetime.
    #[must_use]
    pub fn with_session_ttl(mut self, seconds: i64) -> Self {
        self.session_ttl_seconds = seconds.max(60);
        self
    }

    /// Returns the platform store.
    #[must_use]
    pub fn platform(&self) -> &PlatformStore {
        &self.platform
    }

    /// Hashes a password with Argon2id.
    pub fn hash_password(password: &str) -> PlatformResult<String> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|error| PlatformError::PasswordHash(error.to_string()))
    }

    /// Verifies a password against a stored Argon2 hash.
    #[must_use]
    pub fn verify_password(stored_hash: &str, password: &str) -> bool {
        PasswordHash::new(stored_hash).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
    }

    /// Registers a platform user.
    pub async fn register(&self, input: CreateUserInput, password: &str) -> PlatformResult<User> {
        let normalized = normalize_email(&input.email);
        validate_email(&normalized, "email")
            .map_err(|error| PlatformError::InvalidInput(error.to_string()))?;
        if password.len() < 8 {
            return Err(PlatformError::InvalidInput(
                "password must contain at least 8 characters".to_owned(),
            ));
        }
        let hash = Self::hash_password(password)?;
        self.platform
            .create_user_with_credential(input, &hash)
            .await
    }

    /// Authenticates a user and creates a server-side session.
    pub async fn login(&self, email: &str, password: &str) -> PlatformResult<CreatedSession> {
        let user = self
            .platform
            .get_user_by_email(email)
            .await?
            .ok_or(PlatformError::InvalidCredentials)?;
        if user.status != UserStatus::Active {
            return Err(PlatformError::Forbidden);
        }
        let stored_hash = self
            .platform
            .get_password_hash(&user.id)
            .await?
            .ok_or(PlatformError::InvalidCredentials)?;
        if !Self::verify_password(&stored_hash, password) {
            return Err(PlatformError::InvalidCredentials);
        }
        self.create_session(user).await
    }

    async fn create_session(&self, user: User) -> PlatformResult<CreatedSession> {
        let token = random_token(32);
        let csrf_token = random_token(24);
        let session_id = SessionId::new();
        let now = now_timestamp();
        let expires_at = now.saturating_add(self.session_ttl_seconds);
        sqlx::query(
            "INSERT INTO admin_sessions
             (id, user_id, token_hash, expires_at, created_at, last_used_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id.as_str())
        .bind(user.id.as_str())
        .bind(hash_token(&token))
        .bind(expires_at)
        .bind(now)
        .bind(now)
        .execute(self.platform.pool())
        .await?;
        Ok(CreatedSession {
            token,
            csrf_token,
            session_id,
            expires_at,
            user,
        })
    }

    /// Validates a session token and refreshes its last-used timestamp.
    pub async fn authenticate_session(&self, token: &str) -> PlatformResult<AuthenticatedSession> {
        if token.len() < 32 {
            return Err(PlatformError::Unauthorized);
        }
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.id AS session_id, s.expires_at, u.id AS user_id, u.email,
                    u.first_name, u.last_name, u.status, u.email_verified_at,
                    u.metadata, u.created_at, u.updated_at
             FROM admin_sessions s
             JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ? AND s.revoked_at IS NULL",
        )
        .bind(hash_token(token))
        .fetch_optional(self.platform.pool())
        .await?;
        let Some(row) = row else {
            return Err(PlatformError::Unauthorized);
        };
        if row.expires_at <= now_timestamp() {
            return Err(PlatformError::SessionExpired);
        }
        let session = row.into_session();
        if session.user.status != UserStatus::Active {
            return Err(PlatformError::Forbidden);
        }
        sqlx::query("UPDATE admin_sessions SET last_used_at = ? WHERE id = ?")
            .bind(now_timestamp())
            .bind(session.session_id.as_str())
            .execute(self.platform.pool())
            .await?;
        Ok(session)
    }

    /// Revokes a session by token.
    pub async fn logout(&self, token: &str) -> PlatformResult<()> {
        sqlx::query("UPDATE admin_sessions SET revoked_at = ? WHERE token_hash = ?")
            .bind(now_timestamp())
            .bind(hash_token(token))
            .execute(self.platform.pool())
            .await?;
        Ok(())
    }

    /// Revokes every session for a user.
    pub async fn revoke_all_sessions(&self, user_id: &UserId) -> PlatformResult<()> {
        sqlx::query(
            "UPDATE admin_sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(user_id.as_str())
        .execute(self.platform.pool())
        .await?;
        Ok(())
    }

    /// Changes a user's password and revokes existing sessions.
    pub async fn change_password(
        &self,
        user_id: &UserId,
        current_password: &str,
        new_password: &str,
    ) -> PlatformResult<()> {
        if new_password.len() < 8 {
            return Err(PlatformError::InvalidInput(
                "password must contain at least 8 characters".to_owned(),
            ));
        }
        let stored = self
            .platform
            .get_password_hash(user_id)
            .await?
            .ok_or(PlatformError::Unauthorized)?;
        if !Self::verify_password(&stored, current_password) {
            return Err(PlatformError::InvalidCredentials);
        }
        let hash = Self::hash_password(new_password)?;
        self.platform.update_password_hash(user_id, &hash).await?;
        self.revoke_all_sessions(user_id).await?;
        Ok(())
    }

    /// Authenticates an admin session and resolves permissions for a store.
    pub async fn authorize_for_store(
        &self,
        token: &str,
        store_id: &StoreId,
    ) -> PlatformResult<(AuthenticatedSession, BTreeSet<String>)> {
        let session = self.authenticate_session(token).await?;
        let permissions = self
            .platform
            .permissions_for(&session.user.id, store_id)
            .await?;
        if permissions.is_empty() && !self.platform.is_member(&session.user.id, store_id).await? {
            return Err(PlatformError::Forbidden);
        }
        Ok((session, permissions))
    }

    /// Creates an API token. The full token is returned exactly once.
    pub async fn create_api_token(
        &self,
        user_id: &UserId,
        store_id: Option<&StoreId>,
        name: &str,
        scopes: &[String],
        ttl_seconds: Option<i64>,
    ) -> PlatformResult<CreatedApiToken> {
        if name.trim().is_empty() || name.chars().count() > 120 {
            return Err(PlatformError::InvalidInput(
                "token name must be between 1 and 120 characters".to_owned(),
            ));
        }
        if let Some(store_id) = store_id {
            if !self.platform.is_member(user_id, store_id).await? {
                return Err(PlatformError::Forbidden);
            }
        }
        let ttl = ttl_seconds.unwrap_or(MAX_API_TOKEN_TTL_SECONDS);
        if ttl <= 0 || ttl > MAX_API_TOKEN_TTL_SECONDS {
            return Err(PlatformError::InvalidInput(
                "invalid API token lifetime".to_owned(),
            ));
        }
        let token = format!("fc_{}", random_token(32));
        let id = format!("token_{}", uuid::Uuid::now_v7().simple());
        let prefix: String = token.chars().take(12).collect();
        let now = now_timestamp();
        let expires_at = now.saturating_add(ttl);
        let scopes_json = serde_json::to_string(scopes)
            .map_err(|error| PlatformError::InvalidInput(error.to_string()))?;
        sqlx::query(
            "INSERT INTO api_tokens
             (id, store_id, user_id, name, token_prefix, token_hash, scopes,
              expires_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(store_id.map(StoreId::as_str))
        .bind(user_id.as_str())
        .bind(name.trim())
        .bind(&prefix)
        .bind(hash_token(&token))
        .bind(scopes_json)
        .bind(expires_at)
        .bind(now)
        .execute(self.platform.pool())
        .await?;
        Ok(CreatedApiToken {
            token,
            record: ApiToken {
                id,
                user_id: user_id.clone(),
                store_id: store_id.cloned(),
                name: name.trim().to_owned(),
                prefix,
                scopes: scopes.to_vec(),
                expires_at: Some(expires_at),
                revoked_at: None,
                last_used_at: None,
                created_at: now,
            },
        })
    }

    /// Validates an API token.
    pub async fn authenticate_api_token(&self, token: &str) -> PlatformResult<AuthenticatedToken> {
        if !token.starts_with("fc_") || token.len() < 40 {
            return Err(PlatformError::Unauthorized);
        }
        let row = sqlx::query(
            "SELECT id, user_id, store_id, scopes, expires_at, revoked_at
             FROM api_tokens WHERE token_hash = ?",
        )
        .bind(hash_token(token))
        .fetch_optional(self.platform.pool())
        .await?;
        let Some(row) = row else {
            return Err(PlatformError::Unauthorized);
        };
        let revoked_at: Option<i64> = row.try_get("revoked_at")?;
        if revoked_at.is_some() {
            return Err(PlatformError::SessionExpired);
        }
        let expires_at: Option<i64> = row.try_get("expires_at")?;
        if expires_at.is_some_and(|expires| expires <= now_timestamp()) {
            return Err(PlatformError::SessionExpired);
        }
        let store_id: Option<String> = row.try_get("store_id")?;
        let user_id = UserId::from_string(row.try_get::<String, _>("user_id")?);
        let store_id = store_id.map(StoreId::from_string);
        if let Some(store_id) = store_id.as_ref() {
            if !self.platform.is_member(&user_id, store_id).await? {
                return Err(PlatformError::Forbidden);
            }
        }
        let scopes: Vec<String> =
            serde_json::from_str(row.try_get::<String, _>("scopes")?.as_str()).unwrap_or_default();
        let token_id: String = row.try_get("id")?;
        sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE id = ?")
            .bind(now_timestamp())
            .bind(&token_id)
            .execute(self.platform.pool())
            .await?;
        Ok(AuthenticatedToken {
            token_id,
            user_id,
            store_id,
            scopes,
        })
    }

    /// Revokes an API token owned by a user.
    pub async fn revoke_api_token(&self, user_id: &UserId, token_id: &str) -> PlatformResult<()> {
        let result = sqlx::query(
            "UPDATE api_tokens SET revoked_at = ? WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(token_id)
        .bind(user_id.as_str())
        .execute(self.platform.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Err(PlatformError::NotFound(format!("api token {token_id}")));
        }
        Ok(())
    }

    /// Lists a user's API tokens without secrets.
    pub async fn list_api_tokens(&self, user_id: &UserId) -> PlatformResult<Vec<ApiToken>> {
        let rows = sqlx::query(
            "SELECT id, user_id, store_id, name, token_prefix, scopes, expires_at,
                    revoked_at, last_used_at, created_at
             FROM api_tokens WHERE user_id = ? ORDER BY created_at DESC",
        )
        .bind(user_id.as_str())
        .fetch_all(self.platform.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                let store_id: Option<String> = row.try_get("store_id")?;
                Ok(ApiToken {
                    id: row.try_get("id")?,
                    user_id: UserId::from_string(row.try_get::<String, _>("user_id")?),
                    store_id: store_id.map(StoreId::from_string),
                    name: row.try_get("name")?,
                    prefix: row.try_get("token_prefix")?,
                    scopes: serde_json::from_str(row.try_get::<String, _>("scopes")?.as_str())
                        .unwrap_or_default(),
                    expires_at: row.try_get("expires_at")?,
                    revoked_at: row.try_get("revoked_at")?,
                    last_used_at: row.try_get("last_used_at")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(Into::into)
    }
}

fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buffer);
    hex::encode(buffer)
}

/// SHA-256 token digest used for session and API token lookup.
#[must_use]
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::Pagination;

    use super::{AuthService, hash_token};
    use crate::repository::PlatformStore;
    use crate::store::CreateStoreInput;
    use crate::user::CreateUserInput;

    async fn service() -> AuthService {
        AuthService::new(PlatformStore::in_memory().await.unwrap())
    }

    #[tokio::test]
    async fn passwords_are_hashed_and_verified() {
        let hash = AuthService::hash_password("correct horse battery staple").unwrap();
        assert_ne!(hash, "correct horse battery staple");
        assert!(AuthService::verify_password(
            &hash,
            "correct horse battery staple"
        ));
        assert!(!AuthService::verify_password(&hash, "wrong"));
    }

    #[tokio::test]
    async fn registration_login_and_session_authentication() {
        let auth = service().await;
        let user = auth
            .register(
                CreateUserInput::new("owner@example.com").with_name("Ada", "Lovelace"),
                "hunter2hunter2",
            )
            .await
            .unwrap();

        let session = auth
            .login("OWNER@example.com", "hunter2hunter2")
            .await
            .unwrap();
        assert_eq!(session.user.id, user.id);
        assert!(!session.token.is_empty());
        assert!(!session.csrf_token.is_empty());

        let authenticated = auth.authenticate_session(&session.token).await.unwrap();
        assert_eq!(authenticated.user.id, user.id);
        assert_eq!(authenticated.session_id, session.session_id);
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_email_fail() {
        let auth = service().await;
        auth.register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
            .await
            .unwrap();
        assert!(
            auth.login("owner@example.com", "wrong-password")
                .await
                .is_err()
        );
        assert!(
            auth.login("missing@example.com", "hunter2hunter2")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn logout_and_password_change_revoke_sessions() {
        let auth = service().await;
        let user = auth
            .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
            .await
            .unwrap();
        let first = auth
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();

        auth.logout(&first.token).await.unwrap();
        assert!(auth.authenticate_session(&first.token).await.is_err());

        let second = auth
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();
        auth.change_password(&user.id, "hunter2hunter2", "new-password-1")
            .await
            .unwrap();
        assert!(auth.authenticate_session(&second.token).await.is_err());
        assert!(
            auth.login("owner@example.com", "hunter2hunter2")
                .await
                .is_err()
        );
        assert!(
            auth.login("owner@example.com", "new-password-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn expired_sessions_are_rejected() {
        let auth = service().await;
        auth.register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
            .await
            .unwrap();
        let session = auth
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();
        sqlx::query("UPDATE admin_sessions SET expires_at = 0 WHERE id = ?")
            .bind(session.session_id.as_str())
            .execute(auth.platform().pool())
            .await
            .unwrap();
        assert!(auth.authenticate_session(&session.token).await.is_err());
    }

    #[tokio::test]
    async fn store_authorization_requires_membership_and_permissions() {
        let auth = service().await;
        let user = auth
            .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
            .await
            .unwrap();
        let store = auth
            .platform()
            .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
            .await
            .unwrap();
        let session = auth
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();

        assert!(
            auth.authorize_for_store(&session.token, &store.id)
                .await
                .is_err()
        );

        auth.platform()
            .upsert_membership(&user.id, &store.id, &["owner"])
            .await
            .unwrap();
        let (_, permissions) = auth
            .authorize_for_store(&session.token, &store.id)
            .await
            .unwrap();
        assert!(permissions.contains("products.write"));
        assert!(permissions.contains("users.write"));
    }

    #[tokio::test]
    async fn api_tokens_are_hashed_and_revocable() {
        let auth = service().await;
        let user = auth
            .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
            .await
            .unwrap();

        let created = auth
            .create_api_token(
                &user.id,
                None,
                "automation",
                &["products.read".into()],
                None,
            )
            .await
            .unwrap();
        assert!(created.token.starts_with("fc_"));

        let stored_hash: String =
            sqlx::query_scalar("SELECT token_hash FROM api_tokens WHERE id = ?")
                .bind(&created.record.id)
                .fetch_one(auth.platform().pool())
                .await
                .unwrap();
        assert_eq!(stored_hash, hash_token(&created.token));
        assert_ne!(stored_hash, created.token);

        let authenticated = auth.authenticate_api_token(&created.token).await.unwrap();
        assert_eq!(authenticated.user_id, user.id);
        assert_eq!(authenticated.scopes, vec!["products.read"]);

        auth.revoke_api_token(&user.id, &created.record.id)
            .await
            .unwrap();
        assert!(auth.authenticate_api_token(&created.token).await.is_err());

        let listed = auth.list_api_tokens(&user.id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].revoked_at.is_some());
        assert!(!format!("{:?}", listed[0]).contains(&created.token));
    }

    #[tokio::test]
    async fn api_tokens_for_stores_require_membership() {
        let auth = service().await;
        let user = auth
            .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
            .await
            .unwrap();
        let store = auth
            .platform()
            .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
            .await
            .unwrap();
        assert!(
            auth.create_api_token(&user.id, Some(&store.id), "t", &[], None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn user_lists_are_paginated() {
        let auth = service().await;
        for index in 0..3 {
            auth.register(
                CreateUserInput::new(format!("user{index}@example.com")),
                "hunter2hunter2",
            )
            .await
            .unwrap();
        }
        let page = auth
            .platform()
            .list_users(Pagination::new(Some(2), Some(0)).unwrap())
            .await
            .unwrap();
        assert_eq!(page.data.len(), 2);
        assert_eq!(page.count, 3);
    }
}
