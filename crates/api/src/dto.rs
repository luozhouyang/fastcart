//! Public API request and response DTOs.
//!
//! Domain entities are never exposed directly: DTOs keep the HTTP contract
//! stable and prevent internal fields (database keys, credential hashes) from
//! leaking.

use fastcart_kernel::{Metadata, PageResult, Pagination};
use fastcart_module_cart::Cart;
use fastcart_module_catalog::{
    Category, Collection, Dimensions, Product, ProductStatus, ProductVariant,
};
use fastcart_module_checkout::{CheckoutSession, CheckoutStateHistory};
use fastcart_module_content::{NavigationTree, Page};
use fastcart_module_customer::{Address, Customer, CustomerGroup, CustomerStatus};
use fastcart_module_order::{ClaimItemType, Order, OrderEditAction};
use fastcart_module_shipping::ResolvedShippingOption;
use fastcart_module_store::Region;
use fastcart_module_theme::Theme;
use fastcart_platform::{Membership, Store, StoreDomain, User};
use fastcart_storefront::ThemeManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::routes::store_catalog::StoreProduct;
use crate::routes::store_inventory::VariantAvailability;

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

    /// Maps every record in the page while preserving pagination metadata.
    #[must_use]
    pub fn map<U>(self, map: impl FnMut(T) -> U) -> PageResponse<U> {
        PageResponse {
            data: self.data.into_iter().map(map).collect(),
            pagination: self.pagination,
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

/// A purchasable product variant.
///
/// This mirrors the catalog domain `ProductVariant` on the wire; the API DTO
/// exists so the documented contract is derived from a type owned by this
/// crate rather than the catalog module.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Variant)]
pub struct VariantDto {
    /// Variant identifier.
    pub id: String,
    /// Owning product identifier.
    pub product_id: String,
    /// Display title.
    pub title: String,
    /// Optional unique stock-keeping unit.
    pub sku: Option<String>,
    /// Optional barcode.
    pub barcode: Option<String>,
    /// Publication state.
    #[schema(required = false)]
    pub status: ProductStatus,
    /// Optional shipping weight.
    pub weight: Option<f64>,
    /// Optional physical dimensions.
    pub dimensions: Option<Dimensions>,
    /// Whether inventory is tracked for this variant.
    #[schema(required = false)]
    pub manage_inventory: bool,
    /// Whether orders may exceed available stock.
    #[schema(required = false)]
    pub allow_backorder: bool,
    /// Optional tax category reference.
    pub tax_category_id: Option<String>,
    /// Free-form metadata.
    #[schema(required = false)]
    pub metadata: Metadata,
    /// Option values selected for this variant.
    #[schema(required = false)]
    pub option_value_ids: Vec<String>,
    /// Creation timestamp in Unix seconds.
    #[schema(required = false)]
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    #[schema(required = false)]
    pub updated_at: i64,
    /// Soft-deletion timestamp, present when retained for history.
    pub deleted_at: Option<i64>,
}

impl From<ProductVariant> for VariantDto {
    fn from(variant: ProductVariant) -> Self {
        Self {
            id: variant.id,
            product_id: variant.product_id,
            title: variant.title,
            sku: variant.sku,
            barcode: variant.barcode,
            status: variant.status,
            weight: variant.weight,
            dimensions: Some(variant.dimensions),
            manage_inventory: variant.manage_inventory,
            allow_backorder: variant.allow_backorder,
            tax_category_id: variant.tax_category_id,
            metadata: variant.metadata,
            option_value_ids: variant.option_value_ids,
            created_at: variant.created_at,
            updated_at: variant.updated_at,
            deleted_at: variant.deleted_at,
        }
    }
}

/// A cart line item as described by the public contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = LineItem)]
pub struct LineItemDto {
    /// Line item identifier.
    pub id: String,
    /// Owning cart, when the line belongs to one.
    pub cart_id: Option<String>,
    /// Referenced variant.
    pub variant_id: String,
    /// Product title snapshot.
    #[schema(required = false)]
    pub title: String,
    /// Quantity of units.
    pub quantity: i64,
    /// Unit price in minor units.
    #[schema(required = false)]
    pub unit_price: i64,
    /// Total price in minor units.
    #[schema(required = false)]
    pub total_price: i64,
    /// Free-form metadata.
    #[schema(required = false)]
    pub metadata: Metadata,
}

/// Commercial totals summary reported by a checkout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Totals)]
pub struct TotalsDto {
    /// Currency of every amount.
    pub currency_code: String,
    /// Subtotal before discounts.
    pub subtotal: i64,
    /// Total discounts applied.
    #[schema(required = false)]
    pub discount_total: i64,
    /// Total shipping charges.
    #[schema(required = false)]
    pub shipping_total: i64,
    /// Total tax.
    #[schema(required = false)]
    pub tax_total: i64,
    /// Payable total.
    pub grand_total: i64,
}

/// Lifecycle state of the checkout summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutSummaryStatus {
    /// Created but not completed.
    Pending,
    /// Fully processed.
    Completed,
    /// Canceled before completion.
    Canceled,
    /// Expired before completion.
    Expired,
}

/// A checkout summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Checkout)]
pub struct CheckoutDto {
    /// Checkout identifier.
    pub id: String,
    /// Cart being checked out.
    pub cart_id: String,
    /// Current lifecycle state.
    pub status: CheckoutSummaryStatus,
    /// Optional contact email.
    pub email: Option<String>,
    /// Payment provider handling the checkout.
    #[schema(required = false)]
    pub payment_provider: String,
    /// Payment session attached to the checkout, if any.
    pub payment_session_id: Option<String>,
    /// Commercial totals snapshot.
    #[schema(required = false)]
    pub totals: TotalsDto,
    /// Creation timestamp in Unix seconds.
    #[schema(required = false)]
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    #[schema(required = false)]
    pub updated_at: i64,
    /// Expiry timestamp in Unix seconds.
    pub expires_at: Option<i64>,
}

/// Lifecycle status of a customer account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CustomerStatus)]
pub enum CustomerStatusDto {
    /// The customer may authenticate and place orders.
    Active,
    /// The customer is disabled and cannot authenticate.
    Disabled,
}

impl From<CustomerStatus> for CustomerStatusDto {
    fn from(status: CustomerStatus) -> Self {
        match status {
            CustomerStatus::Active => Self::Active,
            CustomerStatus::Disabled => Self::Disabled,
        }
    }
}

/// A store-scoped customer profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Customer)]
pub struct CustomerDto {
    /// Customer identifier.
    pub id: String,
    /// Normalized email address.
    pub email: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Contact phone number.
    pub phone: Option<String>,
    /// Whether the profile has registered credentials.
    #[schema(required = false)]
    pub has_account: bool,
    /// Lifecycle status.
    pub status: CustomerStatusDto,
    /// Email verification timestamp, absent until verified.
    pub email_verified_at: Option<i64>,
    /// Free-form extension metadata.
    #[schema(required = false)]
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    #[schema(required = false)]
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    #[schema(required = false)]
    pub updated_at: i64,
    /// Soft-deletion timestamp, present when retained for history.
    pub deleted_at: Option<i64>,
}

impl From<Customer> for CustomerDto {
    fn from(customer: Customer) -> Self {
        Self {
            id: customer.id.to_string(),
            email: customer.email,
            first_name: customer.first_name,
            last_name: customer.last_name,
            phone: customer.phone,
            has_account: customer.has_account,
            status: customer.status.into(),
            email_verified_at: customer.email_verified_at,
            metadata: customer.metadata,
            created_at: customer.created_at,
            updated_at: customer.updated_at,
            deleted_at: customer.deleted_at,
        }
    }
}

/// A customer segment used for pricing and promotions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = CustomerGroup)]
pub struct CustomerGroupDto {
    /// Group identifier.
    pub id: String,
    /// Unique human-readable name.
    pub name: String,
    /// Free-form extension metadata.
    #[schema(required = false)]
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    #[schema(required = false)]
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    #[schema(required = false)]
    pub updated_at: i64,
}

impl From<CustomerGroup> for CustomerGroupDto {
    fn from(group: CustomerGroup) -> Self {
        Self {
            id: group.id,
            name: group.name,
            metadata: group.metadata,
            created_at: group.created_at,
            updated_at: group.updated_at,
        }
    }
}

/// A validated theme manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = ThemeManifest)]
pub struct ThemeManifestDto {
    /// Stable theme identifier.
    pub id: String,
    /// Human-readable theme name.
    pub name: String,
    /// Theme version.
    pub version: String,
    /// Rendering runtime expected by the theme.
    pub runtime: String,
    /// Entry file inside the theme bundle.
    pub entry: String,
    /// Optional JSON Schema for theme settings.
    pub settings_schema: Option<String>,
    /// Optional asset manifest file.
    pub assets_manifest: Option<String>,
}

impl From<ThemeManifest> for ThemeManifestDto {
    fn from(manifest: ThemeManifest) -> Self {
        Self {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            runtime: manifest.runtime,
            entry: manifest.entry,
            settings_schema: manifest.settings_schema,
            assets_manifest: manifest.assets_manifest,
        }
    }
}

impl From<ThemeManifestDto> for ThemeManifest {
    fn from(manifest: ThemeManifestDto) -> Self {
        Self {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            runtime: manifest.runtime,
            entry: manifest.entry,
            settings_schema: manifest.settings_schema,
            assets_manifest: manifest.assets_manifest,
        }
    }
}

/// A signature attached to a webhook payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WebhookSignature {
    /// Signature value.
    pub signature: String,
    /// Signature timestamp in Unix seconds.
    pub timestamp: i64,
    /// Signature algorithm.
    pub algorithm: String,
}

/// An outbound webhook envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WebhookEnvelope {
    /// Delivery identifier.
    pub id: String,
    /// Event type.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Store the event belongs to.
    pub store_id: String,
    /// Event payload.
    pub data: Value,
    /// Optional payload signature.
    pub signature: Option<WebhookSignature>,
}

/// An inbound payment-provider webhook.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PaymentWebhook {
    /// Payment provider name.
    pub provider: String,
    /// Provider-side event identifier.
    pub provider_event_id: String,
    /// Provider event type.
    pub event_type: String,
    /// Provider payload.
    pub payload: Value,
    /// Optional payload signature.
    pub signature: Option<WebhookSignature>,
}

/// A registered storefront theme as exposed to administrators.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Theme)]
pub struct ThemeDto {
    /// Theme identifier.
    pub id: String,
    /// Parsed theme manifest.
    pub manifest: ThemeManifestDto,
    /// Bundle-relative asset root.
    pub bundle_path: String,
    /// JSON Schema used to validate settings.
    pub settings_schema: Option<Value>,
    /// Current theme settings.
    pub settings: Value,
    /// Whether this is the active theme.
    pub is_active: bool,
    /// Creation timestamp.
    #[schema(required = false)]
    pub created_at: i64,
    /// Last update timestamp.
    #[schema(required = false)]
    pub updated_at: i64,
}

impl From<Theme> for ThemeDto {
    fn from(theme: Theme) -> Self {
        Self {
            id: theme.id.to_string(),
            manifest: theme.manifest.into(),
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = StoreTheme)]
pub struct StoreThemeDto {
    /// Theme identifier.
    pub id: String,
    /// Parsed theme manifest.
    pub manifest: ThemeManifestDto,
    /// Bundle-relative asset root.
    pub bundle_path: String,
    /// Current theme settings.
    pub settings: Value,
}

impl From<Theme> for StoreThemeDto {
    fn from(theme: Theme) -> Self {
        Self {
            id: theme.id.to_string(),
            manifest: theme.manifest.into(),
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

// -------------------------------------------------------------------------
// Storefront response envelopes
//
// Store-facing handlers speak the Medusa v2 shape: list endpoints return a
// named collection with `count`/`offset`/`limit`, while single-resource
// endpoints wrap the resource in a named object. Admin responses keep using
// [`PageResponse`] and are unaffected.
// -------------------------------------------------------------------------

/// A single storefront product.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ProductResponse {
    /// The requested product.
    pub product: StoreProduct,
}

/// A page of storefront products.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ProductListResponse {
    /// Products in this page.
    pub products: Vec<StoreProduct>,
    /// Total number of matching products.
    pub count: i64,
    /// Number of products skipped.
    pub offset: i64,
    /// Maximum number of products requested.
    pub limit: i64,
}

impl ProductListResponse {
    /// Builds a product envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<StoreProduct>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            products: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// A page of storefront search results.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ProductSearchResponse {
    /// Matching products.
    pub products: Vec<Product>,
    /// Total number of matching products.
    pub count: i64,
    /// Number of products skipped.
    pub offset: i64,
    /// Maximum number of products requested.
    pub limit: i64,
}

impl ProductSearchResponse {
    /// Builds a search envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<Product>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            products: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// A page of storefront product categories.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CategoryListResponse {
    /// Categories in this page.
    pub product_categories: Vec<Category>,
    /// Total number of matching categories.
    pub count: i64,
    /// Number of categories skipped.
    pub offset: i64,
    /// Maximum number of categories requested.
    pub limit: i64,
}

impl CategoryListResponse {
    /// Builds a category envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<Category>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            product_categories: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// A page of storefront product collections.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CollectionListResponse {
    /// Collections in this page.
    pub collections: Vec<Collection>,
    /// Total number of matching collections.
    pub count: i64,
    /// Number of collections skipped.
    pub offset: i64,
    /// Maximum number of collections requested.
    pub limit: i64,
}

impl CollectionListResponse {
    /// Builds a collection envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<Collection>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            collections: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// The current storefront cart.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CartResponse {
    /// Cart contents and totals.
    pub cart: Cart,
}

/// A checkout session.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CheckoutResponse {
    /// Checkout contents and state.
    pub checkout: CheckoutSession,
}

/// Result of completing a checkout, as a Medusa v2 discriminated union.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CheckoutCompleteResponse {
    /// Discriminator identifying an order result.
    #[serde(rename = "type")]
    pub event_type: String,
    /// The placed order.
    pub order: Order,
}

impl CheckoutCompleteResponse {
    /// Wraps a placed order in the completion envelope.
    #[must_use]
    pub fn order(order: Order) -> Self {
        Self {
            event_type: "order".to_owned(),
            order,
        }
    }
}

/// The state transition history of a checkout.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CheckoutHistoryResponse {
    /// History entries in chronological order.
    pub history: Vec<CheckoutStateHistory>,
}

/// The authenticated storefront customer.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CustomerResponse {
    /// Customer profile.
    pub customer: CustomerDto,
}

/// A customer postal address.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = CustomerAddress)]
pub struct CustomerAddressDto {
    /// Address identifier.
    pub id: String,
    /// Owning customer.
    pub customer_id: String,
    /// Whether this is the default shipping address.
    pub is_default_shipping: bool,
    /// Whether this is the default billing address.
    pub is_default_billing: bool,
    /// Recipient given name.
    pub first_name: Option<String>,
    /// Recipient family name.
    pub last_name: Option<String>,
    /// Company name.
    pub company: Option<String>,
    /// First address line.
    pub address_1: Option<String>,
    /// Second address line.
    pub address_2: Option<String>,
    /// City or locality.
    pub city: Option<String>,
    /// Province, region or state.
    pub province: Option<String>,
    /// Postal or ZIP code.
    pub postal_code: Option<String>,
    /// Two-letter country code.
    pub country_code: Option<String>,
    /// Contact phone number.
    pub phone: Option<String>,
    /// Free-form extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

impl From<Address> for CustomerAddressDto {
    fn from(address: Address) -> Self {
        Self {
            id: address.id,
            customer_id: address.customer_id.to_string(),
            is_default_shipping: address.is_default_shipping,
            is_default_billing: address.is_default_billing,
            first_name: address.first_name,
            last_name: address.last_name,
            company: address.company,
            address_1: address.address_1,
            address_2: address.address_2,
            city: address.city,
            province: address.province,
            postal_code: address.postal_code,
            country_code: address.country_code.map(|code| code.as_str().to_owned()),
            phone: address.phone,
            metadata: address.metadata,
            created_at: address.created_at,
            updated_at: address.updated_at,
        }
    }
}

/// A single customer address.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CustomerAddressResponse {
    /// The address.
    pub address: CustomerAddressDto,
}

/// All addresses of the authenticated customer.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CustomerAddressListResponse {
    /// Customer addresses.
    pub addresses: Vec<CustomerAddressDto>,
}

/// The groups the authenticated customer belongs to.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CustomerGroupsResponse {
    /// Customer groups.
    pub customer_groups: Vec<CustomerGroupDto>,
}

/// A page of the authenticated customer's orders.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct OrderListResponse {
    /// Orders in this page.
    pub orders: Vec<Order>,
    /// Total number of matching orders.
    pub count: i64,
    /// Number of orders skipped.
    pub offset: i64,
    /// Maximum number of orders requested.
    pub limit: i64,
}

impl OrderListResponse {
    /// Builds an order envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<Order>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            orders: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// A single order owned by the authenticated customer.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct OrderResponse {
    /// The requested order.
    pub order: Order,
}

/// A published storefront content page.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct StorePageResponse {
    /// The requested page.
    pub page: Page,
}

/// A storefront navigation tree.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct NavigationResponse {
    /// The requested navigation tree.
    pub navigation: NavigationTree,
}

/// A page of storefront regions.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct RegionListResponse {
    /// Regions in this page.
    pub regions: Vec<Region>,
    /// Total number of matching regions.
    pub count: i64,
    /// Number of regions skipped.
    pub offset: i64,
    /// Maximum number of regions requested.
    pub limit: i64,
}

impl RegionListResponse {
    /// Builds a region envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<Region>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            regions: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// A single storefront region.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct RegionResponse {
    /// The requested region.
    pub region: Region,
}

/// The active storefront theme.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct StoreThemeResponse {
    /// Active theme manifest and settings.
    pub theme: StoreThemeDto,
}

/// Per-variant availability for a storefront product.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct VariantAvailabilityResponse {
    /// Availability records.
    pub variants: Vec<VariantAvailability>,
}

/// Shipping options available for a destination.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ShippingOptionsResponse {
    /// Matching shipping options.
    pub shipping_options: Vec<ResolvedShippingOption>,
}
