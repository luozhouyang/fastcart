//! Platform users, store memberships, roles and audit logging.

use std::collections::BTreeSet;

use fastcart_kernel::{
    Metadata, PageResult, Pagination, StoreId, UserId, normalize_email, now_timestamp,
    validate_email,
};
use sqlx::{FromRow, Row};

use crate::error::{PlatformError, PlatformResult};
use crate::repository::PlatformStore;

/// Lifecycle state of a platform user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    /// The user can authenticate.
    Active,
    /// Authentication is disabled.
    Disabled,
    /// The user has been invited but has not accepted.
    Invited,
}

impl UserStatus {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Invited => "invited",
        }
    }

    /// Parses a stored value. Unknown values are treated as disabled.
    #[must_use]
    pub fn from_db(value: &str) -> Self {
        match value {
            "active" => Self::Active,
            "invited" => Self::Invited,
            _ => Self::Disabled,
        }
    }
}

/// A global platform user.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct User {
    /// User identifier.
    pub id: UserId,
    /// Normalized email address.
    pub email: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Account status.
    pub status: UserStatus,
    /// Email verification timestamp.
    pub email_verified_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

/// Input for creating a platform user.
#[derive(Clone, Debug)]
pub struct CreateUserInput {
    /// Email address.
    pub email: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
}

impl CreateUserInput {
    /// Creates user input.
    #[must_use]
    pub fn new(email: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            first_name: None,
            last_name: None,
        }
    }

    /// Sets the display name.
    #[must_use]
    pub fn with_name(
        mut self,
        first_name: impl Into<String>,
        last_name: impl Into<String>,
    ) -> Self {
        self.first_name = Some(first_name.into());
        self.last_name = Some(last_name.into());
        self
    }
}

/// Membership state for a user within a store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipStatus {
    /// The user can access the store.
    Active,
    /// Invitation sent but not accepted.
    Invited,
    /// Access revoked.
    Revoked,
}

impl MembershipStatus {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Invited => "invited",
            Self::Revoked => "revoked",
        }
    }

    /// Parses a stored value.
    #[must_use]
    pub fn from_db(value: &str) -> Self {
        match value {
            "active" => Self::Active,
            "invited" => Self::Invited,
            _ => Self::Revoked,
        }
    }
}

/// A user-to-store membership with assigned roles.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Membership {
    /// Membership identifier.
    pub id: String,
    /// User identifier.
    pub user_id: UserId,
    /// Store identifier.
    pub store_id: StoreId,
    /// Membership state.
    pub status: MembershipStatus,
    /// Assigned role names.
    pub roles: Vec<String>,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

#[derive(Debug, FromRow)]
struct UserRow {
    id: String,
    email: String,
    first_name: Option<String>,
    last_name: Option<String>,
    status: String,
    email_verified_at: Option<i64>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl UserRow {
    fn into_user(self) -> User {
        User {
            id: UserId::from_string(self.id),
            email: self.email,
            first_name: self.first_name,
            last_name: self.last_name,
            status: UserStatus::from_db(&self.status),
            email_verified_at: self.email_verified_at,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

const USER_COLUMNS: &str =
    "id, email, first_name, last_name, status, email_verified_at, metadata, created_at, updated_at";

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
        let mut transaction = self.pool().begin().await?;
        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
            .bind(&email)
            .fetch_optional(&mut *transaction)
            .await?;
        if existing.is_some() {
            return Err(PlatformError::EmailConflict(email));
        }
        sqlx::query(
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
        .await?;
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
        transaction.commit().await?;
        self.get_user(&id)
            .await?
            .ok_or_else(|| PlatformError::NotFound("user".to_owned()))
    }

    /// Fetches a user by identifier.
    pub async fn get_user(&self, id: &UserId) -> PlatformResult<Option<User>> {
        let row =
            sqlx::query_as::<_, UserRow>(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?"))
                .bind(id.as_str())
                .fetch_optional(self.pool())
                .await?;
        Ok(row.map(UserRow::into_user))
    }

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

    /// Marks a user's email as verified.
    pub async fn verify_user_email(&self, user_id: &UserId) -> PlatformResult<()> {
        sqlx::query("UPDATE users SET email_verified_at = ?, updated_at = ? WHERE id = ?")
            .bind(now_timestamp())
            .bind(now_timestamp())
            .bind(user_id.as_str())
            .execute(self.pool())
            .await?;
        Ok(())
    }

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

    /// Creates or updates a user's membership and replaces assigned roles.
    pub async fn upsert_membership(
        &self,
        user_id: &UserId,
        store_id: &StoreId,
        roles: &[&str],
    ) -> PlatformResult<Membership> {
        let now = now_timestamp();
        let mut transaction = self.pool().begin().await?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM store_memberships WHERE user_id = ? AND store_id = ?",
        )
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;

        let membership_id = match existing {
            Some(id) => id,
            None => {
                let id = format!("mem_{}", uuid::Uuid::now_v7().simple());
                sqlx::query(
                    "INSERT INTO store_memberships
                     (id, user_id, store_id, status, created_at, updated_at)
                     VALUES (?, ?, ?, 'active', ?, ?)",
                )
                .bind(&id)
                .bind(user_id.as_str())
                .bind(store_id.as_str())
                .bind(now)
                .bind(now)
                .execute(&mut *transaction)
                .await?;
                id
            }
        };
        sqlx::query("UPDATE store_memberships SET status = 'active', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(&membership_id)
            .execute(&mut *transaction)
            .await?;

        sqlx::query("DELETE FROM membership_roles WHERE membership_id = ?")
            .bind(&membership_id)
            .execute(&mut *transaction)
            .await?;
        for role in roles {
            let role_id: Option<String> = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
                .bind(role)
                .fetch_optional(&mut *transaction)
                .await?;
            let Some(role_id) = role_id else {
                return Err(PlatformError::InvalidInput(format!("unknown role: {role}")));
            };
            sqlx::query("INSERT INTO membership_roles (membership_id, role_id) VALUES (?, ?)")
                .bind(&membership_id)
                .bind(role_id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        self.get_membership(user_id, store_id)
            .await?
            .ok_or_else(|| PlatformError::NotFound("membership".to_owned()))
    }

    /// Returns a membership with its roles.
    pub async fn get_membership(
        &self,
        user_id: &UserId,
        store_id: &StoreId,
    ) -> PlatformResult<Option<Membership>> {
        let row = sqlx::query(
            "SELECT id, status, created_at, updated_at FROM store_memberships
             WHERE user_id = ? AND store_id = ?",
        )
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .fetch_optional(self.pool())
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let membership_id: String = row.try_get("id")?;
        let roles: Vec<String> = sqlx::query_scalar(
            "SELECT r.name FROM roles r
             JOIN membership_roles mr ON mr.role_id = r.id
             WHERE mr.membership_id = ? ORDER BY r.name ASC",
        )
        .bind(&membership_id)
        .fetch_all(self.pool())
        .await?;
        Ok(Some(Membership {
            id: membership_id,
            user_id: user_id.clone(),
            store_id: store_id.clone(),
            status: MembershipStatus::from_db(&row.try_get::<String, _>("status")?),
            roles,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        }))
    }

    /// Lists memberships for a store.
    pub async fn list_store_memberships(
        &self,
        store_id: &StoreId,
    ) -> PlatformResult<Vec<Membership>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT user_id FROM store_memberships WHERE store_id = ? ORDER BY created_at ASC",
        )
        .bind(store_id.as_str())
        .fetch_all(self.pool())
        .await?;
        let mut memberships = Vec::with_capacity(ids.len());
        for user_id in ids {
            if let Some(membership) = self
                .get_membership(&UserId::from_string(user_id), store_id)
                .await?
            {
                memberships.push(membership);
            }
        }
        Ok(memberships)
    }

    /// Revokes a membership.
    pub async fn revoke_membership(
        &self,
        user_id: &UserId,
        store_id: &StoreId,
    ) -> PlatformResult<()> {
        let result = sqlx::query(
            "UPDATE store_memberships SET status = 'revoked', updated_at = ?
             WHERE user_id = ? AND store_id = ?",
        )
        .bind(now_timestamp())
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .execute(self.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Err(PlatformError::NotFound("membership".to_owned()));
        }
        Ok(())
    }

    /// Resolves the effective permission set for a user within a store.
    pub async fn permissions_for(
        &self,
        user_id: &UserId,
        store_id: &StoreId,
    ) -> PlatformResult<BTreeSet<String>> {
        let permissions: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT p.name FROM permissions p
             JOIN role_permissions rp ON rp.permission_id = p.id
             JOIN membership_roles mr ON mr.role_id = rp.role_id
             JOIN store_memberships m ON m.id = mr.membership_id
             WHERE m.user_id = ? AND m.store_id = ? AND m.status = 'active'",
        )
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .fetch_all(self.pool())
        .await?;
        Ok(permissions.into_iter().collect())
    }

    /// Returns true when an active membership exists.
    pub async fn is_member(&self, user_id: &UserId, store_id: &StoreId) -> PlatformResult<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM store_memberships
             WHERE user_id = ? AND store_id = ? AND status = 'active'",
        )
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .fetch_one(self.pool())
        .await?;
        Ok(count > 0)
    }

    /// Lists available system roles.
    pub async fn list_roles(&self) -> PlatformResult<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT name, COALESCE(description, '') AS description FROM roles ORDER BY name",
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| Ok((row.try_get("name")?, row.try_get("description")?)))
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// Lists permission names.
    pub async fn list_permissions(&self) -> PlatformResult<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT name FROM permissions ORDER BY name ASC")
                .fetch_all(self.pool())
                .await?,
        )
    }

    /// Appends an audit record.
    pub async fn record_audit(&self, entry: NewAudit) -> PlatformResult<()> {
        sqlx::query(
            "INSERT INTO platform_audit_logs
             (id, actor_id, store_id, action, resource_type, resource_id, data, ip_address, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("audit_{}", uuid::Uuid::now_v7().simple()))
        .bind(entry.actor_id)
        .bind(entry.store_id)
        .bind(entry.action)
        .bind(entry.resource_type)
        .bind(entry.resource_id)
        .bind(entry.data.to_string())
        .bind(entry.ip_address)
        .bind(now_timestamp())
        .execute(self.pool())
        .await?;
        Ok(())
    }
}

/// A new audit record.
#[derive(Clone, Debug)]
pub struct NewAudit {
    /// Acting user.
    pub actor_id: Option<String>,
    /// Store scope.
    pub store_id: Option<String>,
    /// Action name.
    pub action: String,
    /// Resource type.
    pub resource_type: Option<String>,
    /// Resource identifier.
    pub resource_id: Option<String>,
    /// JSON payload.
    pub data: serde_json::Value,
    /// Request IP address.
    pub ip_address: Option<String>,
}

impl NewAudit {
    /// Creates an audit entry for an action.
    #[must_use]
    pub fn new(action: impl Into<String>) -> Self {
        Self {
            actor_id: None,
            store_id: None,
            action: action.into(),
            resource_type: None,
            resource_id: None,
            data: serde_json::Value::Null,
            ip_address: None,
        }
    }
}

/// An audit record.
#[derive(Clone, Debug, FromRow, serde::Serialize, serde::Deserialize)]
pub struct AuditLog {
    /// Record identifier.
    pub id: String,
    /// Acting user.
    pub actor_id: Option<String>,
    /// Store scope.
    pub store_id: Option<String>,
    /// Action name.
    pub action: String,
    /// Resource type.
    pub resource_type: Option<String>,
    /// Resource identifier.
    pub resource_id: Option<String>,
    /// JSON payload.
    pub data: String,
    /// Request IP address.
    pub ip_address: Option<String>,
    /// Timestamp.
    pub created_at: i64,
}

impl PlatformStore {
    /// Lists recent audit records for a store.
    pub async fn list_audit_logs(
        &self,
        store_id: Option<&StoreId>,
        pagination: Pagination,
    ) -> PlatformResult<Vec<AuditLog>> {
        let rows =
            match store_id {
                Some(store_id) => {
                    sqlx::query_as::<_, AuditLog>(
                        "SELECT * FROM platform_audit_logs WHERE store_id = ?
                     ORDER BY created_at DESC LIMIT ? OFFSET ?",
                    )
                    .bind(store_id.as_str())
                    .bind(pagination.limit)
                    .bind(pagination.offset)
                    .fetch_all(self.pool())
                    .await?
                }
                None => sqlx::query_as::<_, AuditLog>(
                    "SELECT * FROM platform_audit_logs ORDER BY created_at DESC LIMIT ? OFFSET ?",
                )
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(self.pool())
                .await?,
            };
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::Pagination;

    use crate::repository::PlatformStore;
    use crate::store::CreateStoreInput;
    use crate::user::{CreateUserInput, MembershipStatus, NewAudit, UserStatus};

    async fn fixture() -> (
        PlatformStore,
        fastcart_kernel::UserId,
        fastcart_kernel::StoreId,
    ) {
        let platform = PlatformStore::in_memory().await.unwrap();
        let user = platform
            .create_user_with_credential(
                CreateUserInput::new("Owner@Example.com").with_name("Ada", "Lovelace"),
                "hash",
            )
            .await
            .unwrap();
        let store = platform
            .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
            .await
            .unwrap();
        (platform, user.id, store.id)
    }

    #[tokio::test]
    async fn users_are_created_with_normalized_email() {
        let (platform, user_id, _) = fixture().await;
        let user = platform.get_user(&user_id).await.unwrap().unwrap();
        assert_eq!(user.email, "owner@example.com");
        assert_eq!(user.status, UserStatus::Active);
        assert_eq!(
            platform
                .get_password_hash(&user_id)
                .await
                .unwrap()
                .as_deref(),
            Some("hash")
        );
    }

    #[tokio::test]
    async fn duplicate_emails_are_rejected() {
        let (platform, _, _) = fixture().await;
        let error = platform
            .create_user_with_credential(CreateUserInput::new("OWNER@example.com"), "hash")
            .await
            .unwrap_err();
        assert!(matches!(error, crate::PlatformError::EmailConflict(_)));
    }

    #[tokio::test]
    async fn memberships_grant_role_permissions() {
        let (platform, user_id, store_id) = fixture().await;
        assert!(!platform.is_member(&user_id, &store_id).await.unwrap());

        let membership = platform
            .upsert_membership(&user_id, &store_id, &["product-manager"])
            .await
            .unwrap();
        assert_eq!(membership.status, MembershipStatus::Active);
        assert_eq!(membership.roles, vec!["product-manager"]);

        let permissions = platform.permissions_for(&user_id, &store_id).await.unwrap();
        assert!(permissions.contains("products.write"));
        assert!(permissions.contains("inventory.write"));
        assert!(!permissions.contains("users.write"));

        platform
            .revoke_membership(&user_id, &store_id)
            .await
            .unwrap();
        assert!(
            platform
                .permissions_for(&user_id, &store_id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn unknown_roles_are_rejected() {
        let (platform, user_id, store_id) = fixture().await;
        assert!(
            platform
                .upsert_membership(&user_id, &store_id, &["wizard"])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn audit_logs_are_recorded() {
        let (platform, _, store_id) = fixture().await;
        platform
            .record_audit(NewAudit {
                actor_id: Some("user_1".to_owned()),
                store_id: Some(store_id.to_string()),
                action: "store.created".to_owned(),
                resource_type: Some("store".to_owned()),
                resource_id: Some(store_id.to_string()),
                data: serde_json::json!({"ok": true}),
                ip_address: None,
            })
            .await
            .unwrap();
        let logs = platform
            .list_audit_logs(Some(&store_id), Pagination::default())
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].action, "store.created");
    }
}
