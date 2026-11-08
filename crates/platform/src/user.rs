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

const AUDIT_INSERT: &str = "INSERT OR IGNORE INTO platform_audit_logs
     (id, actor_id, store_id, action, resource_type, resource_id, data, ip_address, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";

fn unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database_error) if database_error.is_unique_violation()
    )
}

/// Counts active owners in a store other than `membership_id` through `tx`.
async fn store_has_other_active_owner(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    store_id: &StoreId,
    membership_id: &str,
) -> PlatformResult<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT m.id) FROM store_memberships m
         JOIN membership_roles mr ON mr.membership_id = m.id
         JOIN roles r ON r.id = mr.role_id
         WHERE m.store_id = ? AND m.id != ? AND m.status = 'active' AND r.name = 'owner'",
    )
    .bind(store_id.as_str())
    .bind(membership_id)
    .fetch_one(&mut **transaction)
    .await?;
    Ok(count > 0)
}

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

    /// Returns the number of platform users.
    pub async fn count_users(&self) -> PlatformResult<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(self.pool())
            .await?)
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
        // Duplicate role names must not violate the membership_roles primary key
        // and must not produce duplicate audit entries.
        let roles: Vec<&str> = roles
            .iter()
            .copied()
            .collect::<BTreeSet<&str>>()
            .into_iter()
            .collect();
        let mut transaction = self.pool().begin_with("BEGIN IMMEDIATE").await?;
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
                let insert = sqlx::query(
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
                .await;
                if let Err(error) = insert {
                    return Err(if unique_violation(&error) {
                        PlatformError::Conflict(
                            "membership already exists for this user and store".to_owned(),
                        )
                    } else {
                        error.into()
                    });
                }
                id
            }
        };

        // Resolve every role before mutating so unknown roles abort the
        // transaction without partial role changes.
        let mut role_ids = Vec::with_capacity(roles.len());
        for role in &roles {
            let role_id: Option<String> = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
                .bind(*role)
                .fetch_optional(&mut *transaction)
                .await?;
            let Some(role_id) = role_id else {
                return Err(PlatformError::InvalidInput(format!("unknown role: {role}")));
            };
            role_ids.push(role_id);
        }

        // Removing the owner role from the last active owner would leave the
        // store administrable by nobody. The count runs in the same immediate
        // transaction as the replacement below, so it cannot race.
        if !roles.contains(&"owner") {
            let was_active_owner: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM store_memberships m
                    JOIN membership_roles mr ON mr.membership_id = m.id
                    JOIN roles r ON r.id = mr.role_id
                    WHERE m.id = ? AND m.status = 'active' AND r.name = 'owner'
                )",
            )
            .bind(&membership_id)
            .fetch_one(&mut *transaction)
            .await?;
            if was_active_owner
                && !store_has_other_active_owner(&mut transaction, store_id, &membership_id).await?
            {
                return Err(PlatformError::Conflict(
                    "cannot remove the last owner of a store".to_owned(),
                ));
            }
        }

        sqlx::query("UPDATE store_memberships SET status = 'active', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(&membership_id)
            .execute(&mut *transaction)
            .await?;

        sqlx::query("DELETE FROM membership_roles WHERE membership_id = ?")
            .bind(&membership_id)
            .execute(&mut *transaction)
            .await?;
        for role_id in &role_ids {
            sqlx::query("INSERT INTO membership_roles (membership_id, role_id) VALUES (?, ?)")
                .bind(&membership_id)
                .bind(role_id)
                .execute(&mut *transaction)
                .await?;
        }
        let audit_id = format!("audit_{}", uuid::Uuid::now_v7().simple());
        Self::append_audit_tx(
            &mut transaction,
            &audit_id,
            &NewAudit {
                actor_id: self.audit_actor.clone(),
                store_id: Some(store_id.to_string()),
                action: "membership.upserted".to_owned(),
                resource_type: Some("membership".to_owned()),
                resource_id: Some(membership_id.clone()),
                data: serde_json::json!({"roles": roles, "status": "active"}),
                ip_address: None,
            },
        )
        .await?;
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
        let mut transaction = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query(
            "SELECT id, status FROM store_memberships WHERE user_id = ? AND store_id = ?",
        )
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            return Err(PlatformError::NotFound("membership".to_owned()));
        };
        let membership_id: String = row.try_get("id")?;
        let status: String = row.try_get("status")?;

        // The owner check and the revocation share one immediate transaction,
        // so no concurrent revocation can remove the last two owners.
        if status == "active" {
            let is_owner: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM membership_roles mr
                    JOIN roles r ON r.id = mr.role_id
                    WHERE mr.membership_id = ? AND r.name = 'owner'
                )",
            )
            .bind(&membership_id)
            .fetch_one(&mut *transaction)
            .await?;
            if is_owner
                && !store_has_other_active_owner(&mut transaction, store_id, &membership_id).await?
            {
                return Err(PlatformError::Conflict(
                    "cannot remove the last owner of a store".to_owned(),
                ));
            }
        }

        sqlx::query("UPDATE store_memberships SET status = 'revoked', updated_at = ? WHERE id = ?")
            .bind(now_timestamp())
            .bind(&membership_id)
            .execute(&mut *transaction)
            .await?;
        let audit_id = format!("audit_{}", uuid::Uuid::now_v7().simple());
        Self::append_audit_tx(
            &mut transaction,
            &audit_id,
            &NewAudit {
                actor_id: self.audit_actor.clone(),
                store_id: Some(store_id.to_string()),
                action: "membership.removed".to_owned(),
                resource_type: Some("membership".to_owned()),
                resource_id: Some(membership_id),
                data: serde_json::json!({"status": "revoked"}),
                ip_address: None,
            },
        )
        .await?;
        transaction.commit().await?;
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

    /// Returns true when the user holds any of `roles` in an active store.
    ///
    /// Platform-level actions that are not tied to a single store use this to
    /// require existing store ownership instead of accepting any authenticated
    /// user.
    pub async fn has_any_store_role(
        &self,
        user_id: &UserId,
        roles: &[&str],
    ) -> PlatformResult<bool> {
        if roles.is_empty() {
            return Ok(false);
        }
        let placeholders = vec!["?"; roles.len()].join(", ");
        let sql = format!(
            "SELECT COUNT(*) FROM store_memberships m
             JOIN membership_roles mr ON mr.membership_id = m.id
             JOIN roles r ON r.id = mr.role_id
             WHERE m.user_id = ? AND m.status = 'active' AND r.name IN ({placeholders})"
        );
        let mut query = sqlx::query_scalar::<_, i64>(&sql).bind(user_id.as_str());
        for role in roles {
            query = query.bind(*role);
        }
        let count: i64 = query.fetch_one(self.pool()).await?;
        Ok(count > 0)
    }

    /// Returns the number of registered stores.
    pub async fn store_count(&self) -> PlatformResult<i64> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores")
            .fetch_one(self.pool())
            .await?;
        Ok(count)
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

    /// Appends an audit record with a generated identifier.
    pub async fn record_audit(&self, entry: NewAudit) -> PlatformResult<()> {
        let id = format!("audit_{}", uuid::Uuid::now_v7().simple());
        self.record_audit_with_id(&id, &entry).await
    }

    /// Appends an audit record under a caller-chosen identifier.
    ///
    /// Re-delivering the same identifier is ignored, which makes outbox
    /// retries idempotent when delivery succeeds but the outbox row is not
    /// marked processed.
    pub async fn record_audit_with_id(&self, id: &str, entry: &NewAudit) -> PlatformResult<()> {
        sqlx::query(AUDIT_INSERT)
            .bind(id)
            .bind(entry.actor_id.as_deref())
            .bind(entry.store_id.as_deref())
            .bind(&entry.action)
            .bind(entry.resource_type.as_deref())
            .bind(entry.resource_id.as_deref())
            .bind(entry.data.to_string())
            .bind(entry.ip_address.as_deref())
            .bind(now_timestamp())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Appends an audit record inside a mutation transaction.
    ///
    /// Uses the same insert-or-ignore semantics as [`Self::record_audit_with_id`]
    /// so a mutation and its audit row commit or roll back together.
    async fn append_audit_tx(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        id: &str,
        entry: &NewAudit,
    ) -> PlatformResult<()> {
        sqlx::query(AUDIT_INSERT)
            .bind(id)
            .bind(entry.actor_id.as_deref())
            .bind(entry.store_id.as_deref())
            .bind(&entry.action)
            .bind(entry.resource_type.as_deref())
            .bind(entry.resource_id.as_deref())
            .bind(entry.data.to_string())
            .bind(entry.ip_address.as_deref())
            .bind(now_timestamp())
            .execute(&mut **transaction)
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

    #[tokio::test]
    async fn audit_redelivery_with_the_same_id_is_ignored() {
        let (platform, _, store_id) = fixture().await;
        let entry = NewAudit {
            actor_id: Some("user_1".to_owned()),
            store_id: Some(store_id.to_string()),
            action: "page.updated".to_owned(),
            resource_type: Some("page".to_owned()),
            resource_id: Some("page_1".to_owned()),
            data: serde_json::json!({"revision": 2}),
            ip_address: None,
        };
        platform
            .record_audit_with_id("audit_delivery_1", &entry)
            .await
            .unwrap();
        platform
            .record_audit_with_id("audit_delivery_1", &entry)
            .await
            .unwrap();

        let logs = platform
            .list_audit_logs(Some(&store_id), Pagination::default())
            .await
            .unwrap();
        assert_eq!(logs.len(), 1, "outbox retries must not duplicate rows");
    }

    #[tokio::test]
    async fn duplicate_roles_are_deduplicated_and_audited_once() {
        let (platform, user_id, store_id) = fixture().await;
        let audited = platform
            .clone()
            .with_audit_actor(Some("admin_1".to_owned()));

        let membership = audited
            .upsert_membership(&user_id, &store_id, &["owner", "owner"])
            .await
            .unwrap();
        assert_eq!(membership.roles, vec!["owner"]);

        let role_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM membership_roles WHERE membership_id = ?")
                .bind(&membership.id)
                .fetch_one(audited.pool())
                .await
                .unwrap();
        assert_eq!(role_rows, 1);

        let logs = audited
            .list_audit_logs(Some(&store_id), Pagination::default())
            .await
            .unwrap();
        let upserts = logs
            .iter()
            .filter(|log| log.action == "membership.upserted")
            .count();
        assert_eq!(upserts, 1, "one mutation must produce one audit row");
    }

    #[tokio::test]
    async fn last_owner_cannot_be_demoted_or_removed() {
        let (platform, owner_id, store_id) = fixture().await;
        platform
            .upsert_membership(&owner_id, &store_id, &["owner"])
            .await
            .unwrap();

        let demote = platform
            .upsert_membership(&owner_id, &store_id, &["admin"])
            .await
            .unwrap_err();
        assert!(matches!(demote, crate::PlatformError::Conflict(_)));
        assert_eq!(
            platform
                .get_membership(&owner_id, &store_id)
                .await
                .unwrap()
                .unwrap()
                .roles,
            vec!["owner"]
        );

        let remove = platform
            .revoke_membership(&owner_id, &store_id)
            .await
            .unwrap_err();
        assert!(matches!(remove, crate::PlatformError::Conflict(_)));

        let second = platform
            .create_user_with_credential(CreateUserInput::new("second@example.com"), "hash")
            .await
            .unwrap();
        platform
            .upsert_membership(&second.id, &store_id, &["owner"])
            .await
            .unwrap();

        // With a second owner present the first can be demoted and removed.
        platform
            .upsert_membership(&owner_id, &store_id, &["admin"])
            .await
            .unwrap();
        platform
            .revoke_membership(&owner_id, &store_id)
            .await
            .unwrap();

        // The second owner is now the last one and is protected again.
        let remove_last = platform
            .revoke_membership(&second.id, &store_id)
            .await
            .unwrap_err();
        assert!(matches!(remove_last, crate::PlatformError::Conflict(_)));
    }

    #[tokio::test]
    async fn rbac_mutations_append_audits_without_secrets() {
        let (platform, _, store_id) = fixture().await;
        let audited = platform
            .clone()
            .with_audit_actor(Some("admin_1".to_owned()));
        let credential_hash = "credential-secret-value";

        let created = audited
            .create_user_with_credential(
                CreateUserInput::new("member@example.com"),
                credential_hash,
            )
            .await
            .unwrap();
        audited
            .upsert_membership(&created.id, &store_id, &["admin", "admin"])
            .await
            .unwrap();
        audited
            .revoke_membership(&created.id, &store_id)
            .await
            .unwrap();

        let logs = audited
            .list_audit_logs(None, Pagination::default())
            .await
            .unwrap();
        let actor_logs: Vec<_> = logs
            .iter()
            .filter(|log| log.actor_id.as_deref() == Some("admin_1"))
            .collect();
        assert_eq!(actor_logs.len(), 3);
        for action in ["user.created", "membership.upserted", "membership.removed"] {
            assert!(
                actor_logs.iter().any(|log| log.action == action),
                "missing audit action {action}"
            );
        }
        for log in actor_logs {
            let record = serde_json::to_string(log).unwrap();
            assert!(
                !record.contains(credential_hash),
                "audit rows must never contain credential material"
            );
        }
    }

    #[tokio::test]
    async fn verifying_an_unknown_user_is_not_found() {
        let platform = PlatformStore::in_memory().await.unwrap();
        let error = platform
            .verify_user_email(&fastcart_kernel::UserId::new())
            .await
            .unwrap_err();
        assert!(matches!(error, crate::PlatformError::NotFound(_)));
    }
}
