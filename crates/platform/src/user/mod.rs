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

mod audit;
mod memberships;
mod users;

pub use audit::{AuditLog, NewAudit};

#[cfg(test)]
mod tests;
