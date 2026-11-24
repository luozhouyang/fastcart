//! API token issue, authentication and revocation.

use super::*;

impl AuthService {
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
        let Some(store_id) = store_id else {
            return Err(PlatformError::InvalidInput(
                "API tokens must be scoped to a store".to_owned(),
            ));
        };
        if !self.platform.is_member(user_id, store_id).await? {
            return Err(PlatformError::Forbidden);
        }
        let allowed = self.platform.permissions_for(user_id, store_id).await?;
        if !scopes.iter().all(|scope| allowed.contains(scope)) {
            return Err(PlatformError::Forbidden);
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
        .bind(store_id.as_str())
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
                store_id: Some(store_id.clone()),
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
}

impl AuthService {
    /// Validates an API token.
    pub async fn authenticate_api_token(&self, token: &str) -> PlatformResult<AuthenticatedToken> {
        if !token.starts_with("fc_") || token.len() < 40 {
            return Err(PlatformError::Unauthorized);
        }
        let row = sqlx::query(
            "SELECT t.id, t.user_id, t.store_id, t.scopes, t.expires_at, t.revoked_at
             FROM api_tokens t
             JOIN users u ON u.id = t.user_id
             WHERE t.token_hash = ? AND u.status = 'active'",
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
        let Some(store_id_ref) = store_id.as_ref() else {
            return Err(PlatformError::Forbidden);
        };
        if !self.platform.is_member(&user_id, store_id_ref).await? {
            return Err(PlatformError::Forbidden);
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
}

impl AuthService {
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
}

impl AuthService {
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
