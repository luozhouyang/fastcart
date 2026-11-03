//! Public API request and response DTOs.
//!
//! Domain entities are never exposed directly: DTOs keep the HTTP contract
//! stable and prevent internal fields (database keys, credential hashes) from
//! leaking.

use fastcart_kernel::{PageResult, Pagination};
use fastcart_platform::{Membership, Store, StoreDomain, User};
use serde::{Deserialize, Serialize};

/// Pagination metadata in the public contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageInfo {
    /// Page size.
    pub limit: i64,
    /// Records skipped.
    pub offset: i64,
    /// Total matching records.
    pub total: i64,
}

/// Paginated API response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageResponse<T> {
    /// Page records.
    pub data: Vec<T>,
    /// Pagination metadata.
    pub pagination: PageInfo,
}

impl<T> PageResponse<T> {
    /// Maps a kernel page result into the public envelope.
    #[must_use]
    pub fn from_page(page: PageResult<T>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            data: page.data,
            pagination: PageInfo {
                limit,
                offset,
                total: page.count,
            },
        }
    }
}

/// A platform user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDto {
    /// User identifier.
    pub id: String,
    /// Email address.
    pub email: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Account status.
    pub status: String,
    /// Email verification timestamp.
    pub email_verified_at: Option<i64>,
}

impl From<&User> for UserDto {
    fn from(user: &User) -> Self {
        Self {
            id: user.id.to_string(),
            email: user.email.clone(),
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            status: format!("{:?}", user.status).to_ascii_lowercase(),
            email_verified_at: user.email_verified_at,
        }
    }
}

/// A registered store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreDto {
    /// Store identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// URL-safe slug.
    pub slug: String,
    /// Lifecycle status.
    pub status: String,
    /// Primary hostname.
    pub primary_domain: Option<String>,
    /// Locale.
    pub locale: String,
    /// Display currency.
    pub currency_code: String,
    /// IANA time zone.
    pub timezone: String,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl From<&Store> for StoreDto {
    fn from(store: &Store) -> Self {
        Self {
            id: store.id.to_string(),
            name: store.name.clone(),
            slug: store.slug.clone(),
            status: store.status.as_str().to_owned(),
            primary_domain: store.primary_domain.clone(),
            locale: store.locale.clone(),
            currency_code: store.currency_code.clone(),
            timezone: store.timezone.clone(),
            created_at: store.created_at,
            updated_at: store.updated_at,
        }
    }
}

/// A registered hostname.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainDto {
    /// Domain identifier.
    pub id: String,
    /// Store identifier.
    pub store_id: String,
    /// Normalized hostname.
    pub hostname: String,
    /// Whether this is the primary hostname.
    pub is_primary: bool,
    /// Verification timestamp.
    pub verified_at: Option<i64>,
}

impl From<&StoreDomain> for DomainDto {
    fn from(domain: &StoreDomain) -> Self {
        Self {
            id: domain.id.clone(),
            store_id: domain.store_id.to_string(),
            hostname: domain.hostname.clone(),
            is_primary: domain.is_primary,
            verified_at: domain.verified_at,
        }
    }
}

/// A user's membership in a store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipDto {
    /// Membership identifier.
    pub id: String,
    /// User identifier.
    pub user_id: String,
    /// Store identifier.
    pub store_id: String,
    /// Membership status.
    pub status: String,
    /// Assigned roles.
    pub roles: Vec<String>,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl From<&Membership> for MembershipDto {
    fn from(membership: &Membership) -> Self {
        Self {
            id: membership.id.clone(),
            user_id: membership.user_id.to_string(),
            store_id: membership.store_id.to_string(),
            status: format!("{:?}", membership.status).to_ascii_lowercase(),
            roles: membership.roles.clone(),
            created_at: membership.created_at,
            updated_at: membership.updated_at,
        }
    }
}

/// Login request body.
#[derive(Clone, Debug, Deserialize)]
pub struct LoginRequest {
    /// Account email.
    pub email: String,
    /// Account password.
    pub password: String,
}

/// Successful login response body. Tokens are delivered as cookies.
#[derive(Clone, Debug, Serialize)]
pub struct LoginResponse {
    /// Authenticated user.
    pub user: UserDto,
    /// Double-submit CSRF token.
    pub csrf_token: String,
    /// Session expiry timestamp.
    pub expires_at: i64,
}

/// Current actor description.
#[derive(Clone, Debug, Serialize)]
pub struct MeResponse {
    /// Authenticated user.
    pub user: UserDto,
    /// Selected store, when store-scoped.
    pub store_id: Option<String>,
    /// Effective permissions.
    pub permissions: Vec<String>,
}

/// First-run setup request.
#[derive(Clone, Debug, Deserialize)]
pub struct SetupRequest {
    /// Email address.
    pub email: String,
    /// Password.
    pub password: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
}

/// Store creation request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateStoreRequest {
    /// Display name.
    pub name: String,
    /// URL-safe slug.
    pub slug: String,
    /// Initial primary hostname.
    pub primary_domain: String,
    /// Locale, defaults to `en-US`.
    pub locale: Option<String>,
    /// Currency, defaults to `USD`.
    pub currency_code: Option<String>,
    /// Time zone, defaults to `UTC`.
    pub timezone: Option<String>,
}

/// Partial store update request.
#[derive(Clone, Debug, Deserialize)]
pub struct UpdateStoreRequest {
    /// Display name.
    pub name: Option<String>,
    /// Locale.
    pub locale: Option<String>,
    /// Currency.
    pub currency_code: Option<String>,
    /// Time zone.
    pub timezone: Option<String>,
}

/// Store deletion request.
#[derive(Clone, Debug, Deserialize)]
pub struct DeleteStoreRequest {
    /// Whether to write a consistent backup before deletion.
    #[serde(default)]
    pub backup: bool,
    /// Whether to remove the database files. Requires `backup`.
    #[serde(default)]
    pub purge_data: bool,
}

/// Domain registration request.
#[derive(Clone, Debug, Deserialize)]
pub struct DomainRequest {
    /// Hostname to register.
    pub hostname: String,
    /// Whether this hostname becomes primary.
    #[serde(default)]
    pub is_primary: bool,
}

/// Primary domain selection request.
#[derive(Clone, Debug, Deserialize)]
pub struct PrimaryDomainRequest {
    /// Existing hostname to promote.
    pub hostname: String,
}

/// Membership upsert request.
#[derive(Clone, Debug, Deserialize)]
pub struct MembershipRequest {
    /// User to add.
    pub user_id: String,
    /// Roles to assign.
    pub roles: Vec<String>,
}

/// User creation request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateUserRequest {
    /// Email address.
    pub email: String,
    /// Initial password.
    pub password: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Optional store to grant membership in.
    pub store_id: Option<String>,
    /// Roles for the optional membership.
    #[serde(default)]
    pub roles: Vec<String>,
}

/// Permission check response (used by the admin UI).
#[derive(Clone, Debug, Serialize)]
pub struct PermissionCheckResponse {
    /// Whether the permission is granted.
    pub allowed: bool,
}

/// A store role definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleDto {
    /// Role name.
    pub name: String,
    /// Role description.
    pub description: String,
}

/// Result of a provisioning run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProvisioningResponse {
    /// Provisioned store.
    pub store: StoreDto,
    /// Steps executed.
    pub steps: Vec<String>,
    /// Whether the store was already active.
    pub already_active: bool,
}
