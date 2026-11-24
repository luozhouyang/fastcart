//! User records and credentials.

use super::*;

impl PlatformStore {
    /// Creates a user and credential atomically.
    pub async fn create_user_with_credential(
        &self,
        input: CreateUserInput,
        password_hash: &str,
    ) -> PlatformResult<User> {
        let email = normalize_email(&input.email);
        validate_email(&email, "email")
            .map_err(|error| PlatformError::InvalidInput(error.to_string()))?;
        let id = UserId::new();
        let now = now_timestamp();
        let mut transaction = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
            .bind(&email)
            .fetch_optional(&mut *transaction)
            .await?;
        if existing.is_some() {
            return Err(PlatformError::EmailConflict(email));
        }
        let insert = sqlx::query(
            "INSERT INTO users
             (id, email, first_name, last_name, status, email_verified_at, metadata,
              created_at, updated_at)
             VALUES (?, ?, ?, ?, 'active', NULL, '{}', ?, ?)",
        )
        .bind(id.as_str())
        .bind(&email)
        .bind(input.first_name.as_deref())
        .bind(input.last_name.as_deref())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = insert {
            return Err(if unique_violation(&error) {
                PlatformError::EmailConflict(email)
            } else {
                error.into()
            });
        }
        sqlx::query(
            "INSERT INTO user_credentials (id, user_id, provider_id, password_hash,
             created_at, updated_at) VALUES (?, ?, 'emailpass', ?, ?, ?)",
        )
        .bind(format!("cred_{}", uuid::Uuid::now_v7().simple()))
        .bind(id.as_str())
        .bind(password_hash)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let audit_id = format!("audit_{}", uuid::Uuid::now_v7().simple());
        Self::append_audit_tx(
            &mut transaction,
            &audit_id,
            &NewAudit {
                actor_id: self.audit_actor.clone(),
                store_id: None,
                action: "user.created".to_owned(),
                resource_type: Some("user".to_owned()),
                resource_id: Some(id.to_string()),
                data: serde_json::json!({"email": &email, "status": "active"}),
                ip_address: None,
            },
        )
        .await?;
        transaction.commit().await?;
        self.get_user(&id)
            .await?
            .ok_or_else(|| PlatformError::NotFound("user".to_owned()))
    }
}

impl PlatformStore {
    /// Fetches a user by identifier.
    pub async fn get_user(&self, id: &UserId) -> PlatformResult<Option<User>> {
        let row =
            sqlx::query_as::<_, UserRow>(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?"))
                .bind(id.as_str())
                .fetch_optional(self.pool())
                .await?;
        Ok(row.map(UserRow::into_user))
    }
}

impl PlatformStore {
    /// Fetches a user by email.
    pub async fn get_user_by_email(&self, email: &str) -> PlatformResult<Option<User>> {
        let email = normalize_email(email);
        let row = sqlx::query_as::<_, UserRow>(&format!(
            "SELECT {USER_COLUMNS} FROM users WHERE email = ?"
        ))
        .bind(email)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(UserRow::into_user))
    }
}

impl PlatformStore {
    /// Returns the stored password hash for a user.
    pub async fn get_password_hash(&self, user_id: &UserId) -> PlatformResult<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT password_hash FROM user_credentials
             WHERE user_id = ? AND provider_id = 'emailpass'",
        )
        .bind(user_id.as_str())
        .fetch_optional(self.pool())
        .await?)
    }
}

impl PlatformStore {
    /// Updates the stored password hash.
    pub async fn update_password_hash(
        &self,
        user_id: &UserId,
        password_hash: &str,
    ) -> PlatformResult<()> {
        let result = sqlx::query(
            "UPDATE user_credentials SET password_hash = ?, updated_at = ?
             WHERE user_id = ? AND provider_id = 'emailpass'",
        )
        .bind(password_hash)
        .bind(now_timestamp())
        .bind(user_id.as_str())
        .execute(self.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Err(PlatformError::NotFound(format!("user {user_id}")));
        }
        Ok(())
    }
}

impl PlatformStore {
    /// Updates a user's lifecycle status.
    pub async fn set_user_status(
        &self,
        user_id: &UserId,
        status: UserStatus,
    ) -> PlatformResult<User> {
        sqlx::query("UPDATE users SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(now_timestamp())
            .bind(user_id.as_str())
            .execute(self.pool())
            .await?;
        self.get_user(user_id)
            .await?
            .ok_or_else(|| PlatformError::NotFound(format!("user {user_id}")))
    }
}

impl PlatformStore {
    /// Marks a user's email as verified.
    pub async fn verify_user_email(&self, user_id: &UserId) -> PlatformResult<()> {
        let result =
            sqlx::query("UPDATE users SET email_verified_at = ?, updated_at = ? WHERE id = ?")
                .bind(now_timestamp())
                .bind(now_timestamp())
                .bind(user_id.as_str())
                .execute(self.pool())
                .await?;
        if result.rows_affected() == 0 {
            return Err(PlatformError::NotFound(format!("user {user_id}")));
        }
        Ok(())
    }
}

impl PlatformStore {
    /// Returns the number of platform users.
    pub async fn count_users(&self) -> PlatformResult<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(self.pool())
            .await?)
    }
}

impl PlatformStore {
    /// Lists users with pagination.
    pub async fn list_users(&self, pagination: Pagination) -> PlatformResult<PageResult<User>> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(self.pool())
            .await?;
        let rows = sqlx::query_as::<_, UserRow>(&format!(
            "SELECT {USER_COLUMNS} FROM users ORDER BY created_at ASC LIMIT ? OFFSET ?"
        ))
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.pool())
        .await?;
        Ok(PageResult::new(
            rows.into_iter().map(UserRow::into_user).collect(),
            total,
            pagination,
        ))
    }
}
