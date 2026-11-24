//! Password authentication and admin sessions.

use super::*;

impl AuthService {
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
        let hash = Self::hash_password_offloaded(password).await?;
        self.platform
            .create_user_with_credential(input, &hash)
            .await
    }
}

impl AuthService {
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
        if !Self::verify_password_offloaded(&stored_hash, password).await? {
            return Err(PlatformError::InvalidCredentials);
        }
        self.create_session(user).await
    }
}

impl AuthService {
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
}

impl AuthService {
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
}

impl AuthService {
    /// Revokes a session by token.
    pub async fn logout(&self, token: &str) -> PlatformResult<()> {
        sqlx::query("UPDATE admin_sessions SET revoked_at = ? WHERE token_hash = ?")
            .bind(now_timestamp())
            .bind(hash_token(token))
            .execute(self.platform.pool())
            .await?;
        Ok(())
    }
}

impl AuthService {
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
}

impl AuthService {
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
        if !Self::verify_password_offloaded(&stored, current_password).await? {
            return Err(PlatformError::InvalidCredentials);
        }
        let hash = Self::hash_password_offloaded(new_password).await?;
        self.platform.update_password_hash(user_id, &hash).await?;
        self.revoke_all_sessions(user_id).await?;
        Ok(())
    }
}

impl AuthService {
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
}
