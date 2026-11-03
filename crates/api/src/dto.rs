//! Public API request and response DTOs.
//!
//! Domain entities are never exposed directly: DTOs keep the HTTP contract
//! stable and prevent internal fields (database keys, credential hashes) from
//! leaking.

use fastcart_kernel::{Metadata, PageResult, Pagination};
use fastcart_module_order::{ClaimItemType, OrderEditAction};
use fastcart_module_theme::Theme;
use fastcart_platform::{Membership, Store, StoreDomain, User};
use fastcart_storefront::ThemeManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Pagination metadata in the public contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = User)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Store)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Domain)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Membership)]
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
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    /// Account email.
    pub email: String,
    /// Account password.
    pub password: String,
}

/// Successful login response body. Tokens are delivered as cookies.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    /// Authenticated user.
    pub user: UserDto,
    /// Double-submit CSRF token.
    pub csrf_token: String,
    /// Session expiry timestamp.
    pub expires_at: i64,
}

/// Current actor description.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct MeResponse {
    /// Authenticated user.
    pub user: UserDto,
    /// Selected store, when store-scoped.
    pub store_id: Option<String>,
    /// Effective permissions.
    pub permissions: Vec<String>,
}

/// First-run setup request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
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
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
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
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
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
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct DeleteStoreRequest {
    /// Whether to write a consistent backup before deletion.
    #[serde(default)]
    pub backup: bool,
    /// Whether to remove the database files. Requires `backup`.
    #[serde(default)]
    pub purge_data: bool,
}

/// Domain registration request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct DomainRequest {
    /// Hostname to register.
    pub hostname: String,
    /// Whether this hostname becomes primary.
    #[serde(default)]
    pub is_primary: bool,
}

/// Primary domain selection request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct PrimaryDomainRequest {
    /// Existing hostname to promote.
    pub hostname: String,
}

/// Membership upsert request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct MembershipRequest {
    /// User to add.
    pub user_id: String,
    /// Roles to assign.
    pub roles: Vec<String>,
}

/// User creation request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Role)]
pub struct RoleDto {
    /// Role name.
    pub name: String,
    /// Role description.
    pub description: String,
}

/// Result of a provisioning run.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProvisioningResponse {
    /// Provisioned store.
    pub store: StoreDto,
    /// Steps executed.
    pub steps: Vec<String>,
    /// Whether the store was already active.
    pub already_active: bool,
}

/// API token creation request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateApiTokenRequest {
    /// Human-readable name.
    pub name: String,
    /// Optional store scope.
    pub store_id: Option<String>,
    /// Granted permission scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Lifetime in seconds; defaults to the maximum.
    pub ttl_seconds: Option<i64>,
}

/// API token metadata. The secret is never included.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = ApiToken)]
pub struct ApiTokenDto {
    /// Token identifier.
    pub id: String,
    /// Owning user.
    pub user_id: String,
    /// Optional store scope.
    pub store_id: Option<String>,
    /// Human-readable name.
    pub name: String,
    /// Display prefix.
    pub prefix: String,
    /// Granted scopes.
    pub scopes: Vec<String>,
    /// Expiry timestamp.
    pub expires_at: Option<i64>,
    /// Revocation timestamp.
    pub revoked_at: Option<i64>,
    /// Last-use timestamp.
    pub last_used_at: Option<i64>,
    /// Creation timestamp.
    pub created_at: i64,
}

impl From<&fastcart_platform::ApiToken> for ApiTokenDto {
    fn from(token: &fastcart_platform::ApiToken) -> Self {
        Self {
            id: token.id.clone(),
            user_id: token.user_id.to_string(),
            store_id: token.store_id.as_ref().map(ToString::to_string),
            name: token.name.clone(),
            prefix: token.prefix.clone(),
            scopes: token.scopes.clone(),
            expires_at: token.expires_at,
            revoked_at: token.revoked_at,
            last_used_at: token.last_used_at,
            created_at: token.created_at,
        }
    }
}

/// Response for a newly created API token. The plaintext token is shown once.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = CreatedApiToken)]
pub struct CreatedApiTokenResponse {
    /// One-time plaintext token.
    pub token: String,
    /// Persisted metadata.
    pub record: ApiTokenDto,
}

/// A registered storefront theme as exposed to administrators.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeDto {
    /// Theme identifier.
    pub id: String,
    /// Parsed theme manifest.
    pub manifest: ThemeManifest,
    /// Bundle-relative asset root.
    pub bundle_path: String,
    /// JSON Schema used to validate settings.
    pub settings_schema: Option<Value>,
    /// Current theme settings.
    pub settings: Value,
    /// Whether this is the active theme.
    pub is_active: bool,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl From<Theme> for ThemeDto {
    fn from(theme: Theme) -> Self {
        Self {
            id: theme.id.to_string(),
            manifest: theme.manifest,
            bundle_path: theme.bundle_path,
            settings_schema: theme.settings_schema,
            settings: theme.settings,
            is_active: theme.is_active,
            created_at: theme.created_at,
            updated_at: theme.updated_at,
        }
    }
}

/// The active storefront theme exposed to public storefronts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoreThemeDto {
    /// Theme identifier.
    pub id: String,
    /// Parsed theme manifest.
    pub manifest: ThemeManifest,
    /// Bundle-relative asset root.
    pub bundle_path: String,
    /// Current theme settings.
    pub settings: Value,
}

impl From<Theme> for StoreThemeDto {
    fn from(theme: Theme) -> Self {
        Self {
            id: theme.id.to_string(),
            manifest: theme.manifest,
            bundle_path: theme.bundle_path,
            settings: theme.settings,
        }
    }
}

// -------------------------------------------------------------------------
// Post-purchase requests: returns, claims, exchanges and order edits
// -------------------------------------------------------------------------

/// One returned line in a create-return request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReturnItemRequest {
    /// Historical order line being returned.
    pub order_item_id: String,
    /// Requested return quantity.
    pub quantity: i64,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Request body for creating a return.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateReturnRequest {
    /// Optional reason for the return.
    pub reason: Option<String>,
    /// Refund created for this return, when linked.
    pub refund_id: Option<String>,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
    /// Returned lines.
    pub items: Vec<ReturnItemRequest>,
}

/// One received line in a receive-return request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReceiveReturnItemRequest {
    /// Return line's historical order line.
    pub order_item_id: String,
    /// Incremental quantity received.
    pub quantity: i64,
}

/// Request body for receiving a return.
///
/// An empty `items` list receives every outstanding quantity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReceiveReturnRequest {
    /// Lines to receive.
    #[serde(default)]
    pub items: Vec<ReceiveReturnItemRequest>,
}

/// Request body for writing off received return quantities.
///
/// An empty `items` list writes off every outstanding received quantity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WriteOffReturnRequest {
    /// Lines to write off.
    #[serde(default)]
    pub items: Vec<ReceiveReturnItemRequest>,
}

/// Request body for canceling a post-purchase operation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CancelAfterSaleRequest {
    /// Optional cancellation reason.
    pub reason: Option<String>,
}

/// One claimed line in a create-claim request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ClaimItemRequest {
    /// Historical order line affected by the claim.
    pub order_item_id: String,
    /// Claimed quantity.
    pub quantity: i64,
    /// Refund or replacement resolution.
    pub item_type: ClaimItemType,
    /// Replacement variant for `replace` claims.
    pub replacement_variant_id: Option<String>,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Request body for creating a claim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateClaimRequest {
    /// Optional reason for the claim.
    pub reason: Option<String>,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
    /// Claimed lines.
    pub items: Vec<ClaimItemRequest>,
}

/// One exchange line in a create-exchange request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExchangeItemRequest {
    /// Historical order line this exchange line refers to.
    pub order_item_id: String,
    /// Exchanged quantity.
    pub quantity: i64,
    /// Replacement variant for outbound lines.
    pub replacement_variant_id: Option<String>,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Request body for creating an exchange.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateExchangeRequest {
    /// Optional reason for the exchange.
    pub reason: Option<String>,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
    /// Items returned by the customer.
    pub inbound_items: Vec<ExchangeItemRequest>,
    /// Replacement items shipped to the customer.
    pub outbound_items: Vec<ExchangeItemRequest>,
}

/// One edit line in a create-order-edit request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OrderEditItemRequest {
    /// Historical order line for remove/update lines.
    pub order_item_id: Option<String>,
    /// Kind of change.
    pub action: OrderEditAction,
    /// Product identifier snapshot for added lines.
    pub product_id: Option<String>,
    /// Variant identifier snapshot for added lines.
    pub variant_id: Option<String>,
    /// Product title snapshot for added lines.
    pub product_title: Option<String>,
    /// Variant title snapshot for added lines.
    pub variant_title: Option<String>,
    /// SKU snapshot for added lines.
    pub sku: Option<String>,
    /// Unit price for added lines; ignored for existing lines.
    pub unit_price: Option<i64>,
    /// Expected current quantity for remove/update lines.
    pub before_quantity: Option<i64>,
    /// Quantity after the edit.
    pub after_quantity: i64,
}

/// Request body for creating an order edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateOrderEditRequest {
    /// Optional reason for the edit.
    pub reason: Option<String>,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
    /// Edit lines.
    pub items: Vec<OrderEditItemRequest>,
}
