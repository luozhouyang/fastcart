//! Maintained OpenAPI 3.1 description of the Fastcart HTTP API.
//!
//! The document is generated from a compact route table so that coverage is
//! explicit and testable. Every route registered by
//! [`crate::routes`] is represented, including the health and metrics
//! operations, the public and internal storefront namespaces, and provider
//! webhooks. Route-local request bodies that are private to their handler are
//! documented with the generic [`JsonObject`] schema.
//!
//! [`JsonObject`]: #/components/schemas/JsonObject

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use axum::Router;
use serde_json::{Map, Value, json};

use crate::state::AppContext;

/// OpenAPI components generated directly from Rust response types.
///
/// Paths remain registered by the Axum routers; utoipa owns every schema in
/// this document so field names, nullability and enums cannot drift from the
/// types that are actually serialized.
#[derive(utoipa::OpenApi)]
#[openapi(components(schemas(
    fastcart_kernel::CurrencyCode,
    fastcart_kernel::CountryCode,
    fastcart_kernel::Locale,
    fastcart_kernel::Metadata,
    fastcart_kernel::Money,
    fastcart_kernel::Pagination,
    fastcart_module_catalog::Category,
    fastcart_module_catalog::Collection,
    fastcart_module_catalog::Dimensions,
    fastcart_module_catalog::Product,
    fastcart_module_catalog::ProductImage,
    fastcart_module_catalog::ProductOption,
    fastcart_module_catalog::ProductOptionValue,
    fastcart_module_catalog::ProductStatus,
    fastcart_module_catalog::ProductVariant,
    fastcart_module_catalog::Tag,
    fastcart_module_store::PublishableApiKey,
    fastcart_module_store::Region,
    crate::routes::admin_campaigns::CampaignDto,
    crate::routes::admin_campaigns::CreateCampaignRequest,
    crate::routes::admin_campaigns::LinkPromotionRequest,
    crate::routes::admin_campaigns::UpdateCampaignRequest,
    crate::routes::admin_regions::CreatePublishableKeyRequest,
    crate::routes::admin_regions::CreateRegionRequest,
    crate::routes::admin_regions::CreatedPublishableKeyDto,
    crate::routes::admin_regions::PublishableKeyDto,
    crate::routes::admin_regions::RegionDto,
    crate::routes::admin_regions::UpdateRegionRequest,
    fastcart_module_content::Navigation,
    fastcart_module_content::NavigationItem,
    fastcart_module_content::NavigationTree,
    fastcart_module_content::NavigationTreeItem,
    fastcart_module_content::Page,
    fastcart_module_content::PageStatus,
    fastcart_module_content::PageVersion,
    fastcart_module_content::Redirect,
    fastcart_module_shipping::FulfillmentSet,
    fastcart_module_shipping::ResolvedShippingOption,
    fastcart_module_shipping::SelectedShippingRate,
    fastcart_module_shipping::ShippingOption,
    fastcart_module_shipping::ShippingProfile,
    fastcart_module_shipping::ShippingRate,
    fastcart_module_store::SalesChannel,
    fastcart_module_tax::TaxBreakdown,
    fastcart_module_tax::TaxCategory,
    fastcart_module_tax::TaxLine,
    fastcart_module_tax::TaxRate,
    fastcart_module_tax::TaxRegion,
    fastcart_module_webhook::DeliveryStatus,
    fastcart_module_webhook::WebhookDelivery,
    fastcart_module_webhook::WebhookEndpoint,
    fastcart_module_cart::Address,
    fastcart_module_cart::Cart,
    fastcart_module_cart::CartAdjustment,
    fastcart_module_cart::CartDiscount,
    fastcart_module_cart::CartItem,
    fastcart_module_cart::CartShippingMethod,
    fastcart_module_cart::CartStatus,
    fastcart_module_cart::CartTaxLine,
    fastcart_module_cart::CartTotals,
    fastcart_module_checkout::CheckoutSession,
    fastcart_module_checkout::CheckoutStateHistory,
    fastcart_module_checkout::CheckoutStatus,
    fastcart_module_inventory::InventoryItem,
    fastcart_module_inventory::InventoryLevel,
    fastcart_module_inventory::InventoryLocation,
    fastcart_module_inventory::InventoryMovement,
    fastcart_module_inventory::InventoryReservation,
    fastcart_module_inventory::ReservationStatus,
    fastcart_module_order::FulfillmentStatus,
    fastcart_module_order::Order,
    fastcart_module_order::OrderAdjustment,
    fastcart_module_order::OrderClaim,
    fastcart_module_order::OrderClaimItem,
    fastcart_module_order::OrderEdit,
    fastcart_module_order::OrderEditAction,
    fastcart_module_order::OrderEditItem,
    fastcart_module_order::OrderEditItemStatus,
    fastcart_module_order::OrderEditStatus,
    fastcart_module_order::OrderExchange,
    fastcart_module_order::OrderExchangeItem,
    fastcart_module_order::OrderFulfillment,
    fastcart_module_order::OrderItem,
    fastcart_module_order::OrderItemAdjustment,
    fastcart_module_order::OrderRefund,
    fastcart_module_order::OrderReturn,
    fastcart_module_order::OrderReturnItem,
    fastcart_module_order::OrderStatus,
    fastcart_module_order::OrderStatusHistory,
    fastcart_module_order::ClaimItemStatus,
    fastcart_module_order::ClaimItemType,
    fastcart_module_order::ClaimStatus,
    fastcart_module_order::ExchangeItemDirection,
    fastcart_module_order::ExchangeItemStatus,
    fastcart_module_order::ExchangeStatus,
    fastcart_module_order::PaymentStatus,
    fastcart_module_order::RefundStatus,
    fastcart_module_order::ReturnItemStatus,
    fastcart_module_order::ReturnStatus,
    crate::dto::CancelAfterSaleRequest,
    crate::dto::ClaimItemRequest,
    crate::dto::CreateClaimRequest,
    crate::dto::CreateExchangeRequest,
    crate::dto::CreateOrderEditRequest,
    crate::dto::CreateReturnRequest,
    crate::dto::ExchangeItemRequest,
    crate::dto::OrderEditItemRequest,
    crate::dto::ReceiveReturnItemRequest,
    crate::dto::ReceiveReturnRequest,
    crate::dto::ReturnItemRequest,
    crate::dto::WriteOffReturnRequest,
    fastcart_module_payment::Payment,
    fastcart_module_payment::PaymentAttempt,
    fastcart_module_payment::PaymentSession,
    fastcart_module_payment::PaymentWebhookRecord,
    fastcart_module_payment::Refund,
    fastcart_module_payment::WebhookStatus,
    fastcart_module_pricing::Price,
    fastcart_module_pricing::PriceList,
    fastcart_module_pricing::PriceListRule,
    fastcart_module_pricing::PriceListStatus,
    fastcart_module_pricing::PriceListType,
    fastcart_module_pricing::PriceSet,
    fastcart_module_pricing::SelectedPrice,
    crate::routes::store_catalog::CalculatedPrice,
    fastcart_module_promotion::Campaign,
    fastcart_module_promotion::Coupon,
    fastcart_module_promotion::Promotion,
    fastcart_module_promotion::PromotionApplication,
    fastcart_module_promotion::PromotionResult,
    fastcart_module_promotion::PromotionRule,
    fastcart_module_promotion::PromotionRuleAttribute,
    fastcart_module_promotion::PromotionRuleOperator,
    fastcart_module_promotion::PromotionStatus,
    fastcart_module_promotion::PromotionType,
    fastcart_module_promotion::PromotionUsage,
)))]
struct GeneratedSchemas;

/// Builds the OpenAPI 3.1 document.
#[must_use]
pub fn document() -> Value {
    static DOCUMENT: OnceLock<Value> = OnceLock::new();
    DOCUMENT.get_or_init(build_document).clone()
}

fn build_document() -> Value {
    let mut paths: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    for route in routes() {
        paths
            .entry(route.path.clone())
            .or_default()
            .insert(route.method.as_str().to_owned(), operation(&route));
    }
    let paths = Value::Object(
        paths
            .into_iter()
            .map(|(path, item)| (path, Value::Object(item)))
            .collect(),
    );

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Fastcart Commerce API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Versioned Fastcart commerce API. Administrative endpoints are \
                store-scoped through the x-store-id header; storefront endpoints resolve the \
                store from the trusted Host header. Unsafe cookie-authenticated requests must \
                echo the double-submit CSRF token in the x-csrf-token header. Paths are listed \
                in their absolute form; `/api/v1` is the default server prefix. The internal \
                storefront namespace is additionally gated by the \
                x-fastcart-internal-secret header.",
        },
        "servers": [{"url": "/"}],
        "tags": tags(),
        "paths": paths,
        "components": {
            "securitySchemes": security_schemes(),
            "schemas": schemas(),
        },
    })
}

fn generated_schemas() -> Map<String, Value> {
    let document = <GeneratedSchemas as utoipa::OpenApi>::openapi();
    let value = serde_json::to_value(document).unwrap_or(Value::Null);
    value
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// Axum handler serving the document at `GET /openapi.json`.
pub async fn handler() -> axum::Json<Value> {
    axum::Json(document())
}

/// Router exposing the document.
///
/// The caller merges this router under `/api/v1`, so the handler is reachable
/// at `/api/v1/openapi.json`.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route("/openapi.json", axum::routing::get(handler))
}

// -------------------------------------------------------------------------
// Route model
// -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Get,
    Post,
    Patch,
    Put,
    Delete,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Patch => "patch",
            Self::Put => "put",
            Self::Delete => "delete",
        }
    }

    fn is_mutating(self) -> bool {
        !matches!(self, Self::Get)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Auth {
    /// No authentication is required.
    Public,
    /// Administrator session cookie or bearer API token.
    Admin,
    /// Storefront customer session cookie.
    Customer,
    /// Optional customer session; anonymous access is allowed.
    CustomerOptional,
}

impl Auth {
    fn uses_cookie(self) -> bool {
        matches!(self, Self::Admin | Self::Customer | Self::CustomerOptional)
    }
}

#[derive(Clone, Copy, Debug)]
enum Schema {
    None,
    Ref(&'static str),
    Page(&'static str),
    Array(&'static str),
    MapOf(&'static str),
    NullableRef(&'static str),
    Text,
}

#[derive(Clone, Copy, Debug)]
struct Success {
    status: u16,
    schema: Schema,
}

const fn ok(schema: Schema) -> Success {
    Success {
        status: 200,
        schema,
    }
}

const fn created(schema: Schema) -> Success {
    Success {
        status: 201,
        schema,
    }
}

const fn no_content() -> Success {
    Success {
        status: 204,
        schema: Schema::None,
    }
}

struct Route {
    method: Method,
    path: String,
    summary: &'static str,
    auth: Auth,
    query: &'static [&'static str],
    body: Option<&'static str>,
    success: Success,
}

impl Route {
    fn tag(&self) -> &'static str {
        tag_for(&self.path)
    }
}

const PAGED: &[&str] = &["limit", "offset"];

fn route(
    method: Method,
    path: &str,
    summary: &'static str,
    auth: Auth,
    query: &'static [&'static str],
    body: Option<&'static str>,
    success: Success,
) -> Route {
    Route {
        method,
        path: path.to_owned(),
        summary,
        auth,
        query,
        body,
        success,
    }
}

fn get(
    path: &str,
    summary: &'static str,
    auth: Auth,
    query: &'static [&'static str],
    success: Success,
) -> Route {
    route(Method::Get, path, summary, auth, query, None, success)
}

fn post(
    path: &str,
    summary: &'static str,
    auth: Auth,
    body: Option<&'static str>,
    success: Success,
) -> Route {
    route(Method::Post, path, summary, auth, &[], body, success)
}

fn patch(
    path: &str,
    summary: &'static str,
    auth: Auth,
    body: &'static str,
    success: Success,
) -> Route {
    route(Method::Patch, path, summary, auth, &[], Some(body), success)
}

fn put(
    path: &str,
    summary: &'static str,
    auth: Auth,
    body: &'static str,
    success: Success,
) -> Route {
    route(Method::Put, path, summary, auth, &[], Some(body), success)
}

fn del(path: &str, summary: &'static str, auth: Auth, success: Success) -> Route {
    route(Method::Delete, path, summary, auth, &[], None, success)
}

fn delq(
    path: &str,
    summary: &'static str,
    auth: Auth,
    query: &'static [&'static str],
    success: Success,
) -> Route {
    route(Method::Delete, path, summary, auth, query, None, success)
}

fn routes() -> Vec<Route> {
    let mut routes = platform_routes();
    routes.extend(admin_routes());
    routes.extend(store_routes());
    routes.extend(internal_routes());
    routes.extend(webhook_routes());
    routes.extend(operations_routes());
    routes
}

fn tag_for(path: &str) -> &'static str {
    if path.starts_with("/api/v1/platform") {
        "Platform"
    } else if path.starts_with("/api/v1/admin") {
        "Admin"
    } else if path.starts_with("/api/v1/store") {
        "Store"
    } else if path.starts_with("/api/v1/webhooks") {
        "Webhooks"
    } else if path.starts_with("/internal/storefront") {
        "Internal"
    } else {
        "Operations"
    }
}

// -------------------------------------------------------------------------
// Platform routes
// -------------------------------------------------------------------------

fn platform_routes() -> Vec<Route> {
    vec![
        post(
            "/api/v1/platform/auth/login",
            "Log in a platform administrator",
            Auth::Public,
            Some("LoginRequest"),
            ok(Schema::Ref("LoginResponse")),
        ),
        post(
            "/api/v1/platform/auth/setup",
            "Create the first platform administrator",
            Auth::Public,
            Some("SetupRequest"),
            ok(Schema::Ref("LoginResponse")),
        ),
        post(
            "/api/v1/platform/auth/logout",
            "Log out the current platform session",
            Auth::Admin,
            None,
            no_content(),
        ),
        get(
            "/api/v1/platform/auth/me",
            "Describe the authenticated platform actor",
            Auth::Admin,
            &[],
            ok(Schema::Ref("MeResponse")),
        ),
        get(
            "/api/v1/platform/roles",
            "List store roles",
            Auth::Admin,
            &[],
            ok(Schema::Array("Role")),
        ),
        post(
            "/api/v1/platform/users",
            "Create a platform user",
            Auth::Admin,
            Some("CreateUserRequest"),
            created(Schema::Ref("User")),
        ),
        get(
            "/api/v1/platform/api-tokens",
            "List API tokens",
            Auth::Admin,
            &[],
            ok(Schema::Array("ApiToken")),
        ),
        post(
            "/api/v1/platform/api-tokens",
            "Create an API token",
            Auth::Admin,
            Some("CreateApiTokenRequest"),
            created(Schema::Ref("CreatedApiToken")),
        ),
        del(
            "/api/v1/platform/api-tokens/{token_id}",
            "Revoke an API token",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/platform/stores",
            "List stores visible to the actor",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("Store")),
        ),
        post(
            "/api/v1/platform/stores",
            "Create and provision a store",
            Auth::Admin,
            Some("CreateStoreRequest"),
            created(Schema::Ref("Store")),
        ),
        get(
            "/api/v1/platform/stores/{store_id}",
            "Get a store",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Store")),
        ),
        patch(
            "/api/v1/platform/stores/{store_id}",
            "Update a store",
            Auth::Admin,
            "UpdateStoreRequest",
            ok(Schema::Ref("Store")),
        ),
        post(
            "/api/v1/platform/stores/{store_id}/activate",
            "Activate a store",
            Auth::Admin,
            None,
            ok(Schema::Ref("Store")),
        ),
        post(
            "/api/v1/platform/stores/{store_id}/suspend",
            "Suspend a store",
            Auth::Admin,
            None,
            ok(Schema::Ref("Store")),
        ),
        post(
            "/api/v1/platform/stores/{store_id}/resume",
            "Resume a suspended store",
            Auth::Admin,
            None,
            ok(Schema::Ref("Store")),
        ),
        post(
            "/api/v1/platform/stores/{store_id}/provision",
            "Provision store infrastructure",
            Auth::Admin,
            None,
            ok(Schema::Ref("ProvisioningResponse")),
        ),
        post(
            "/api/v1/platform/stores/{store_id}/delete",
            "Delete a store",
            Auth::Admin,
            Some("DeleteStoreRequest"),
            ok(Schema::Ref("Store")),
        ),
        get(
            "/api/v1/platform/stores/{store_id}/domains",
            "List store domains",
            Auth::Admin,
            &[],
            ok(Schema::Array("Domain")),
        ),
        post(
            "/api/v1/platform/stores/{store_id}/domains",
            "Register a store domain",
            Auth::Admin,
            Some("DomainRequest"),
            created(Schema::Ref("Domain")),
        ),
        post(
            "/api/v1/platform/stores/{store_id}/domains/primary",
            "Set the primary store domain",
            Auth::Admin,
            Some("PrimaryDomainRequest"),
            ok(Schema::Ref("Domain")),
        ),
        del(
            "/api/v1/platform/stores/{store_id}/domains/{hostname}",
            "Remove a store domain",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/platform/stores/{store_id}/memberships",
            "List store memberships",
            Auth::Admin,
            &[],
            ok(Schema::Array("Membership")),
        ),
        post(
            "/api/v1/platform/stores/{store_id}/memberships",
            "Create or update a store membership",
            Auth::Admin,
            Some("MembershipRequest"),
            created(Schema::Ref("Membership")),
        ),
        del(
            "/api/v1/platform/stores/{store_id}/memberships/{user_id}",
            "Remove a store membership",
            Auth::Admin,
            no_content(),
        ),
    ]
}

// -------------------------------------------------------------------------
// Admin routes
// -------------------------------------------------------------------------

fn admin_routes() -> Vec<Route> {
    let mut routes = vec![
        post(
            "/api/v1/admin/auth/login",
            "Log in a store administrator",
            Auth::Public,
            Some("LoginRequest"),
            ok(Schema::Ref("LoginResponse")),
        ),
        post(
            "/api/v1/admin/auth/logout",
            "Log out the current administrator session",
            Auth::Admin,
            None,
            no_content(),
        ),
        get(
            "/api/v1/admin/auth/me",
            "Describe the authenticated administrator actor",
            Auth::Admin,
            &[],
            ok(Schema::Ref("MeResponse")),
        ),
        get(
            "/api/v1/admin/stores",
            "List stores available to the administrator",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("Store")),
        ),
    ];
    routes.extend(admin_catalog_routes());
    routes.extend(admin_content_routes());
    routes.extend(admin_customer_routes());
    routes.extend(admin_inventory_routes());
    routes.extend(admin_order_routes());
    routes.extend(admin_payment_routes());
    routes.extend(admin_pricing_routes());
    routes.extend(admin_promotion_routes());
    routes.extend(admin_campaign_routes());
    routes.extend(admin_region_routes());
    routes.extend(admin_shipping_routes());
    routes.extend(admin_tax_routes());
    routes.extend(admin_theme_routes());
    routes.extend(admin_webhook_routes());
    routes
}

fn admin_catalog_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/catalog/products",
            "List products",
            Auth::Admin,
            &[
                "limit",
                "offset",
                "status",
                "category_id",
                "collection_id",
                "tag",
                "query",
                "include_deleted",
            ],
            ok(Schema::Page("Product")),
        ),
        post(
            "/api/v1/admin/catalog/products",
            "Create a product",
            Auth::Admin,
            Some("CreateProductRequest"),
            created(Schema::Ref("Product")),
        ),
        get(
            "/api/v1/admin/catalog/products/{id}",
            "Get a product",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Product")),
        ),
        patch(
            "/api/v1/admin/catalog/products/{id}",
            "Update a product",
            Auth::Admin,
            "UpdateProductRequest",
            ok(Schema::Ref("Product")),
        ),
        del(
            "/api/v1/admin/catalog/products/{id}",
            "Delete a product",
            Auth::Admin,
            no_content(),
        ),
        post(
            "/api/v1/admin/catalog/products/{id}/publish",
            "Publish a product",
            Auth::Admin,
            None,
            ok(Schema::Ref("Product")),
        ),
        post(
            "/api/v1/admin/catalog/products/{id}/archive",
            "Archive a product",
            Auth::Admin,
            None,
            ok(Schema::Ref("Product")),
        ),
        post(
            "/api/v1/admin/catalog/products/{id}/variants",
            "Create a product variant",
            Auth::Admin,
            Some("CreateVariantRequest"),
            created(Schema::Ref("Variant")),
        ),
        post(
            "/api/v1/admin/products/{id}/sales-channels",
            "Replace the sales channels a product is published to",
            Auth::Admin,
            Some("SetProductSalesChannelsRequest"),
            ok(Schema::Ref("Product")),
        ),
        get(
            "/api/v1/admin/catalog/variants/{id}",
            "Get a variant",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Variant")),
        ),
        patch(
            "/api/v1/admin/catalog/variants/{id}",
            "Update a variant",
            Auth::Admin,
            "UpdateVariantRequest",
            ok(Schema::Ref("Variant")),
        ),
        del(
            "/api/v1/admin/catalog/variants/{id}",
            "Delete a variant",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/catalog/products/{id}/images",
            "List product images",
            Auth::Admin,
            &[],
            ok(Schema::Array("ProductImage")),
        ),
        post(
            "/api/v1/admin/catalog/products/{id}/images",
            "Add a product image",
            Auth::Admin,
            Some("CreateImageRequest"),
            created(Schema::Ref("ProductImage")),
        ),
        del(
            "/api/v1/admin/catalog/images/{id}",
            "Remove a product image",
            Auth::Admin,
            no_content(),
        ),
        post(
            "/api/v1/admin/catalog/products/{id}/images/reorder",
            "Reorder product images",
            Auth::Admin,
            Some("ImageReorderRequest"),
            ok(Schema::Array("ProductImage")),
        ),
        get(
            "/api/v1/admin/catalog/categories",
            "List categories",
            Auth::Admin,
            &["limit", "offset", "parent_id"],
            ok(Schema::Page("Category")),
        ),
        post(
            "/api/v1/admin/catalog/categories",
            "Create a category",
            Auth::Admin,
            Some("CreateCategoryRequest"),
            created(Schema::Ref("Category")),
        ),
        get(
            "/api/v1/admin/catalog/categories/{id}",
            "Get a category",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Category")),
        ),
        patch(
            "/api/v1/admin/catalog/categories/{id}",
            "Update a category",
            Auth::Admin,
            "UpdateCategoryRequest",
            ok(Schema::Ref("Category")),
        ),
        del(
            "/api/v1/admin/catalog/categories/{id}",
            "Delete a category",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/catalog/collections",
            "List collections",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("Collection")),
        ),
        post(
            "/api/v1/admin/catalog/collections",
            "Create a collection",
            Auth::Admin,
            Some("CreateCollectionRequest"),
            created(Schema::Ref("Collection")),
        ),
        get(
            "/api/v1/admin/catalog/collections/{id}",
            "Get a collection",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Collection")),
        ),
        patch(
            "/api/v1/admin/catalog/collections/{id}",
            "Update a collection",
            Auth::Admin,
            "UpdateCollectionRequest",
            ok(Schema::Ref("Collection")),
        ),
        del(
            "/api/v1/admin/catalog/collections/{id}",
            "Delete a collection",
            Auth::Admin,
            no_content(),
        ),
        post(
            "/api/v1/admin/catalog/collections/{id}/publish",
            "Publish a collection",
            Auth::Admin,
            None,
            ok(Schema::Ref("Collection")),
        ),
        post(
            "/api/v1/admin/catalog/collections/{id}/unpublish",
            "Unpublish a collection",
            Auth::Admin,
            None,
            ok(Schema::Ref("Collection")),
        ),
        post(
            "/api/v1/admin/catalog/collections/{id}/products",
            "Add products to a collection",
            Auth::Admin,
            Some("CollectionProductsRequest"),
            no_content(),
        ),
        del(
            "/api/v1/admin/catalog/collections/{id}/products/{product_id}",
            "Remove a product from a collection",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/catalog/tags",
            "List tags",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("Tag")),
        ),
        post(
            "/api/v1/admin/catalog/tags",
            "Create a tag",
            Auth::Admin,
            Some("TagRequest"),
            created(Schema::Ref("Tag")),
        ),
        post(
            "/api/v1/admin/catalog/products/{id}/tags",
            "Attach a tag to a product",
            Auth::Admin,
            Some("ProductTagRequest"),
            ok(Schema::Ref("Tag")),
        ),
        del(
            "/api/v1/admin/catalog/products/{id}/tags/{tag_id}",
            "Detach a tag from a product",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/catalog/search",
            "Search products as an administrator",
            Auth::Admin,
            &["limit", "offset", "q"],
            ok(Schema::Page("Product")),
        ),
        post(
            "/api/v1/admin/catalog/reindex",
            "Reindex the product search catalog",
            Auth::Admin,
            None,
            ok(Schema::Ref("ReindexResponse")),
        ),
        get(
            "/api/v1/admin/settings",
            "Get all store settings",
            Auth::Admin,
            &[],
            ok(Schema::Ref("SettingsMap")),
        ),
        put(
            "/api/v1/admin/settings",
            "Update multiple store settings",
            Auth::Admin,
            "SettingsRequest",
            ok(Schema::Ref("SettingsMap")),
        ),
        get(
            "/api/v1/admin/settings/{key}",
            "Get a store setting",
            Auth::Admin,
            &[],
            ok(Schema::Ref("SettingValue")),
        ),
        put(
            "/api/v1/admin/settings/{key}",
            "Set a store setting",
            Auth::Admin,
            "SettingRequest",
            ok(Schema::Ref("SettingValue")),
        ),
        del(
            "/api/v1/admin/settings/{key}",
            "Delete a store setting",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/sales-channels",
            "List sales channels",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("SalesChannel")),
        ),
        post(
            "/api/v1/admin/sales-channels",
            "Create a sales channel",
            Auth::Admin,
            Some("CreateSalesChannelRequest"),
            created(Schema::Ref("SalesChannel")),
        ),
        get(
            "/api/v1/admin/sales-channels/{id}",
            "Get a sales channel",
            Auth::Admin,
            &[],
            ok(Schema::Ref("SalesChannel")),
        ),
        patch(
            "/api/v1/admin/sales-channels/{id}",
            "Update a sales channel",
            Auth::Admin,
            "UpdateSalesChannelRequest",
            ok(Schema::Ref("SalesChannel")),
        ),
        post(
            "/api/v1/admin/sales-channels/{id}/disable",
            "Disable a sales channel",
            Auth::Admin,
            None,
            ok(Schema::Ref("SalesChannel")),
        ),
        post(
            "/api/v1/admin/sales-channels/{id}/enable",
            "Enable a sales channel",
            Auth::Admin,
            None,
            ok(Schema::Ref("SalesChannel")),
        ),
    ]
}

fn admin_content_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/content/pages",
            "List content pages",
            Auth::Admin,
            &["limit", "offset", "status", "include_deleted"],
            ok(Schema::Page("Page")),
        ),
        post(
            "/api/v1/admin/content/pages",
            "Create a content page",
            Auth::Admin,
            Some("CreatePageRequest"),
            created(Schema::Ref("Page")),
        ),
        get(
            "/api/v1/admin/content/pages/{id}",
            "Get a content page",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Page")),
        ),
        patch(
            "/api/v1/admin/content/pages/{id}",
            "Update a content page",
            Auth::Admin,
            "UpdatePageRequest",
            ok(Schema::Ref("Page")),
        ),
        del(
            "/api/v1/admin/content/pages/{id}",
            "Delete a content page",
            Auth::Admin,
            no_content(),
        ),
        post(
            "/api/v1/admin/content/pages/{id}/publish",
            "Publish a content page",
            Auth::Admin,
            None,
            ok(Schema::Ref("Page")),
        ),
        post(
            "/api/v1/admin/content/pages/{id}/unpublish",
            "Unpublish a content page",
            Auth::Admin,
            None,
            ok(Schema::Ref("Page")),
        ),
        post(
            "/api/v1/admin/content/pages/{id}/archive",
            "Archive a content page",
            Auth::Admin,
            None,
            ok(Schema::Ref("Page")),
        ),
        get(
            "/api/v1/admin/content/pages/{id}/versions",
            "List page versions",
            Auth::Admin,
            &[],
            ok(Schema::Array("PageVersion")),
        ),
        post(
            "/api/v1/admin/content/pages/{id}/versions/{version}/restore",
            "Restore a page version",
            Auth::Admin,
            None,
            ok(Schema::Ref("Page")),
        ),
        get(
            "/api/v1/admin/content/navigations",
            "List navigations",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("Navigation")),
        ),
        post(
            "/api/v1/admin/content/navigations",
            "Create a navigation",
            Auth::Admin,
            Some("CreateNavigationRequest"),
            created(Schema::Ref("Navigation")),
        ),
        get(
            "/api/v1/admin/content/navigations/{id}",
            "Get a navigation",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Navigation")),
        ),
        patch(
            "/api/v1/admin/content/navigations/{id}",
            "Update a navigation",
            Auth::Admin,
            "UpdateNavigationRequest",
            ok(Schema::Ref("Navigation")),
        ),
        del(
            "/api/v1/admin/content/navigations/{id}",
            "Delete a navigation",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/content/navigations/{id}/tree",
            "Get a navigation tree",
            Auth::Admin,
            &[],
            ok(Schema::Ref("NavigationTree")),
        ),
        post(
            "/api/v1/admin/content/navigations/{id}/items",
            "Add a navigation item",
            Auth::Admin,
            Some("NavigationItemRequest"),
            created(Schema::Ref("NavigationItem")),
        ),
        patch(
            "/api/v1/admin/content/navigation-items/{item_id}",
            "Update a navigation item",
            Auth::Admin,
            "NavigationItemRequest",
            ok(Schema::Ref("NavigationItem")),
        ),
        del(
            "/api/v1/admin/content/navigation-items/{item_id}",
            "Delete a navigation item",
            Auth::Admin,
            no_content(),
        ),
        post(
            "/api/v1/admin/content/navigations/{id}/items/reorder",
            "Reorder navigation items",
            Auth::Admin,
            Some("NavigationItemReorderRequest"),
            ok(Schema::Array("NavigationItem")),
        ),
        get(
            "/api/v1/admin/content/redirects",
            "List redirects",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("Redirect")),
        ),
        post(
            "/api/v1/admin/content/redirects",
            "Create a redirect",
            Auth::Admin,
            Some("CreateRedirectRequest"),
            created(Schema::Ref("Redirect")),
        ),
        get(
            "/api/v1/admin/content/redirects/{id}",
            "Get a redirect",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Redirect")),
        ),
        del(
            "/api/v1/admin/content/redirects/{id}",
            "Delete a redirect",
            Auth::Admin,
            no_content(),
        ),
    ]
}

fn admin_customer_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/customers",
            "List customers",
            Auth::Admin,
            &["limit", "offset", "query"],
            ok(Schema::Page("Customer")),
        ),
        get(
            "/api/v1/admin/customers/{id}",
            "Get a customer",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Customer")),
        ),
        patch(
            "/api/v1/admin/customers/{id}",
            "Update a customer",
            Auth::Admin,
            "UpdateCustomerRequest",
            ok(Schema::Ref("Customer")),
        ),
        del(
            "/api/v1/admin/customers/{id}",
            "Delete a customer",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/customers/{id}/addresses",
            "List customer addresses",
            Auth::Admin,
            &[],
            ok(Schema::Array("Address")),
        ),
        post(
            "/api/v1/admin/customers/{id}/addresses",
            "Add a customer address",
            Auth::Admin,
            Some("AddressRequest"),
            created(Schema::Ref("Address")),
        ),
        patch(
            "/api/v1/admin/customers/{id}/addresses/{address_id}",
            "Update a customer address",
            Auth::Admin,
            "AddressRequest",
            ok(Schema::Ref("Address")),
        ),
        del(
            "/api/v1/admin/customers/{id}/addresses/{address_id}",
            "Remove a customer address",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/customer-groups",
            "List customer groups",
            Auth::Admin,
            &[],
            ok(Schema::Array("CustomerGroup")),
        ),
        post(
            "/api/v1/admin/customer-groups",
            "Create a customer group",
            Auth::Admin,
            Some("CreateGroupRequest"),
            created(Schema::Ref("CustomerGroup")),
        ),
        post(
            "/api/v1/admin/customer-groups/{group_id}/customers",
            "Assign a customer to a group",
            Auth::Admin,
            Some("GroupCustomerRequest"),
            no_content(),
        ),
        del(
            "/api/v1/admin/customer-groups/{group_id}/customers/{customer_id}",
            "Remove a customer from a group",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/customers/{id}/groups",
            "List a customer's groups",
            Auth::Admin,
            &[],
            ok(Schema::Array("CustomerGroup")),
        ),
    ]
}

fn admin_inventory_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/inventory/locations",
            "List inventory locations",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("InventoryLocation")),
        ),
        post(
            "/api/v1/admin/inventory/locations",
            "Create an inventory location",
            Auth::Admin,
            Some("CreateInventoryLocationRequest"),
            created(Schema::Ref("InventoryLocation")),
        ),
        get(
            "/api/v1/admin/inventory/locations/{id}",
            "Get an inventory location",
            Auth::Admin,
            &[],
            ok(Schema::Ref("InventoryLocation")),
        ),
        patch(
            "/api/v1/admin/inventory/locations/{id}",
            "Update an inventory location",
            Auth::Admin,
            "UpdateInventoryLocationRequest",
            ok(Schema::Ref("InventoryLocation")),
        ),
        get(
            "/api/v1/admin/inventory/items",
            "List inventory items",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("InventoryItem")),
        ),
        post(
            "/api/v1/admin/inventory/items",
            "Create an inventory item",
            Auth::Admin,
            Some("CreateInventoryItemRequest"),
            created(Schema::Ref("InventoryItem")),
        ),
        get(
            "/api/v1/admin/inventory/items/{id}",
            "Get an inventory item",
            Auth::Admin,
            &[],
            ok(Schema::Ref("InventoryItem")),
        ),
        get(
            "/api/v1/admin/inventory/items/by-variant/{variant_id}",
            "Get an inventory item by variant",
            Auth::Admin,
            &[],
            ok(Schema::Ref("InventoryItem")),
        ),
        get(
            "/api/v1/admin/inventory/levels",
            "List inventory levels",
            Auth::Admin,
            &["item_id", "location_id"],
            ok(Schema::Array("InventoryLevel")),
        ),
        get(
            "/api/v1/admin/inventory/availability/{variant_id}",
            "Get variant availability",
            Auth::Admin,
            &[],
            ok(Schema::Ref("InventoryAvailability")),
        ),
        post(
            "/api/v1/admin/inventory/adjustments",
            "Adjust stock",
            Auth::Admin,
            Some("InventoryAdjustmentRequest"),
            ok(Schema::Ref("InventoryLevel")),
        ),
        post(
            "/api/v1/admin/inventory/transfers",
            "Transfer stock between locations",
            Auth::Admin,
            Some("InventoryTransferRequest"),
            ok(Schema::Ref("TransferResponse")),
        ),
        post(
            "/api/v1/admin/inventory/reservations",
            "Reserve stock for a checkout",
            Auth::Admin,
            Some("ReservationRequest"),
            created(Schema::Array("InventoryReservation")),
        ),
        post(
            "/api/v1/admin/inventory/reservations/release",
            "Release stock reservations",
            Auth::Admin,
            Some("ReleaseReservationsRequest"),
            ok(Schema::Ref("ReleasedResponse")),
        ),
        post(
            "/api/v1/admin/inventory/reservations/commit",
            "Commit stock reservations",
            Auth::Admin,
            Some("CommitReservationsRequest"),
            ok(Schema::Ref("CommittedResponse")),
        ),
        post(
            "/api/v1/admin/inventory/reservations/expire",
            "Expire due stock reservations",
            Auth::Admin,
            None,
            ok(Schema::Ref("ReleasedResponse")),
        ),
        get(
            "/api/v1/admin/inventory/movements",
            "List inventory movements",
            Auth::Admin,
            &["item_id", "limit"],
            ok(Schema::Array("InventoryMovement")),
        ),
    ]
}

fn admin_order_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/orders",
            "List orders",
            Auth::Admin,
            &[
                "limit",
                "offset",
                "status",
                "payment_status",
                "customer_id",
                "email",
            ],
            ok(Schema::Page("Order")),
        ),
        get(
            "/api/v1/admin/orders/{id}",
            "Get an order",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Order")),
        ),
        get(
            "/api/v1/admin/orders/by-number/{display_number}",
            "Get an order by display number",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Order")),
        ),
        post(
            "/api/v1/admin/orders/{id}/confirm",
            "Confirm an order",
            Auth::Admin,
            None,
            ok(Schema::Ref("Order")),
        ),
        post(
            "/api/v1/admin/orders/{id}/process",
            "Start processing an order",
            Auth::Admin,
            None,
            ok(Schema::Ref("Order")),
        ),
        post(
            "/api/v1/admin/orders/{id}/cancel",
            "Cancel an order and restock inventory",
            Auth::Admin,
            Some("CancelOrderRequest"),
            ok(Schema::Ref("Order")),
        ),
        post(
            "/api/v1/admin/orders/{id}/fulfill",
            "Fulfill an order",
            Auth::Admin,
            Some("FulfillOrderRequest"),
            ok(Schema::Ref("Order")),
        ),
        get(
            "/api/v1/admin/orders/{id}/history",
            "List order status history",
            Auth::Admin,
            &[],
            ok(Schema::Array("OrderStatusHistory")),
        ),
        get(
            "/api/v1/admin/orders/{id}/refunds",
            "List order refunds",
            Auth::Admin,
            &[],
            ok(Schema::Array("OrderRefund")),
        ),
        post(
            "/api/v1/admin/orders/{id}/refunds",
            "Refund an order payment",
            Auth::Admin,
            Some("RefundOrderRequest"),
            ok(Schema::Ref("Order")),
        ),
        get(
            "/api/v1/admin/orders/{id}/returns",
            "List order returns",
            Auth::Admin,
            &[],
            ok(Schema::Array("OrderReturn")),
        ),
        post(
            "/api/v1/admin/orders/{id}/returns",
            "Create an order return",
            Auth::Admin,
            Some("CreateReturnRequest"),
            created(Schema::Ref("OrderReturn")),
        ),
        get(
            "/api/v1/admin/orders/{id}/returns/{return_id}",
            "Get an order return",
            Auth::Admin,
            &[],
            ok(Schema::Ref("OrderReturn")),
        ),
        post(
            "/api/v1/admin/orders/{id}/returns/{return_id}/request",
            "Request an order return",
            Auth::Admin,
            None,
            ok(Schema::Ref("OrderReturn")),
        ),
        post(
            "/api/v1/admin/orders/{id}/returns/{return_id}/receive",
            "Receive an order return",
            Auth::Admin,
            Some("ReceiveReturnRequest"),
            ok(Schema::Ref("OrderReturn")),
        ),
        post(
            "/api/v1/admin/orders/{id}/returns/{return_id}/write-off",
            "Write off an order return",
            Auth::Admin,
            Some("WriteOffReturnRequest"),
            ok(Schema::Ref("OrderReturn")),
        ),
        post(
            "/api/v1/admin/orders/{id}/returns/{return_id}/cancel",
            "Cancel an order return",
            Auth::Admin,
            Some("CancelAfterSaleRequest"),
            ok(Schema::Ref("OrderReturn")),
        ),
        get(
            "/api/v1/admin/orders/{id}/claims",
            "List order claims",
            Auth::Admin,
            &[],
            ok(Schema::Array("OrderClaim")),
        ),
        post(
            "/api/v1/admin/orders/{id}/claims",
            "Create an order claim",
            Auth::Admin,
            Some("CreateClaimRequest"),
            created(Schema::Ref("OrderClaim")),
        ),
        get(
            "/api/v1/admin/orders/{id}/claims/{claim_id}",
            "Get an order claim",
            Auth::Admin,
            &[],
            ok(Schema::Ref("OrderClaim")),
        ),
        post(
            "/api/v1/admin/orders/{id}/claims/{claim_id}/accept",
            "Accept an order claim",
            Auth::Admin,
            None,
            ok(Schema::Ref("OrderClaim")),
        ),
        post(
            "/api/v1/admin/orders/{id}/claims/{claim_id}/cancel",
            "Cancel an order claim",
            Auth::Admin,
            Some("CancelAfterSaleRequest"),
            ok(Schema::Ref("OrderClaim")),
        ),
        get(
            "/api/v1/admin/orders/{id}/exchanges",
            "List order exchanges",
            Auth::Admin,
            &[],
            ok(Schema::Array("OrderExchange")),
        ),
        post(
            "/api/v1/admin/orders/{id}/exchanges",
            "Create an order exchange",
            Auth::Admin,
            Some("CreateExchangeRequest"),
            created(Schema::Ref("OrderExchange")),
        ),
        get(
            "/api/v1/admin/orders/{id}/exchanges/{exchange_id}",
            "Get an order exchange",
            Auth::Admin,
            &[],
            ok(Schema::Ref("OrderExchange")),
        ),
        post(
            "/api/v1/admin/orders/{id}/exchanges/{exchange_id}/accept",
            "Accept an order exchange",
            Auth::Admin,
            None,
            ok(Schema::Ref("OrderExchange")),
        ),
        post(
            "/api/v1/admin/orders/{id}/exchanges/{exchange_id}/cancel",
            "Cancel an order exchange",
            Auth::Admin,
            Some("CancelAfterSaleRequest"),
            ok(Schema::Ref("OrderExchange")),
        ),
        get(
            "/api/v1/admin/orders/{id}/edits",
            "List order edits",
            Auth::Admin,
            &[],
            ok(Schema::Array("OrderEdit")),
        ),
        post(
            "/api/v1/admin/orders/{id}/edits",
            "Create an order edit",
            Auth::Admin,
            Some("CreateOrderEditRequest"),
            created(Schema::Ref("OrderEdit")),
        ),
        get(
            "/api/v1/admin/orders/{id}/edits/{edit_id}",
            "Get an order edit",
            Auth::Admin,
            &[],
            ok(Schema::Ref("OrderEdit")),
        ),
        post(
            "/api/v1/admin/orders/{id}/edits/{edit_id}/confirm",
            "Confirm an order edit",
            Auth::Admin,
            None,
            ok(Schema::Ref("OrderEdit")),
        ),
        post(
            "/api/v1/admin/orders/{id}/edits/{edit_id}/cancel",
            "Cancel an order edit",
            Auth::Admin,
            Some("CancelAfterSaleRequest"),
            ok(Schema::Ref("OrderEdit")),
        ),
    ]
}

fn admin_payment_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/payments",
            "List payments",
            Auth::Admin,
            &["limit", "offset", "order_id", "status"],
            ok(Schema::Page("Payment")),
        ),
        get(
            "/api/v1/admin/payments/{id}",
            "Get a payment",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Payment")),
        ),
        get(
            "/api/v1/admin/payments/{id}/refunds",
            "List payment refunds",
            Auth::Admin,
            &[],
            ok(Schema::Array("PaymentRefund")),
        ),
        post(
            "/api/v1/admin/payments/{id}/refunds",
            "Create a payment refund",
            Auth::Admin,
            Some("CreatePaymentRefundRequest"),
            created(Schema::Ref("PaymentRefund")),
        ),
        get(
            "/api/v1/admin/payment-sessions/{id}",
            "Get a payment session",
            Auth::Admin,
            &[],
            ok(Schema::Ref("PaymentSession")),
        ),
        post(
            "/api/v1/admin/payment-sessions/{id}/capture",
            "Capture a payment session",
            Auth::Admin,
            None,
            ok(Schema::Ref("Payment")),
        ),
        post(
            "/api/v1/admin/payment-sessions/{id}/cancel",
            "Cancel a payment session",
            Auth::Admin,
            None,
            ok(Schema::Ref("PaymentSession")),
        ),
        post(
            "/api/v1/admin/payment-sessions/{id}/fail",
            "Fail a payment session",
            Auth::Admin,
            Some("FailSessionRequest"),
            ok(Schema::Ref("PaymentSession")),
        ),
        get(
            "/api/v1/admin/payment-sessions/{id}/attempts",
            "List payment attempts",
            Auth::Admin,
            &[],
            ok(Schema::Array("PaymentAttempt")),
        ),
        get(
            "/api/v1/admin/payment-webhooks",
            "List received payment webhooks",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("PaymentWebhookRecord")),
        ),
        get(
            "/api/v1/admin/payment-webhooks/{id}",
            "Get a received payment webhook",
            Auth::Admin,
            &[],
            ok(Schema::Ref("PaymentWebhookRecord")),
        ),
    ]
}

fn admin_pricing_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/price-lists",
            "List price lists",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("PriceList")),
        ),
        post(
            "/api/v1/admin/price-lists",
            "Create a price list",
            Auth::Admin,
            Some("CreatePriceListRequest"),
            created(Schema::Ref("PriceList")),
        ),
        get(
            "/api/v1/admin/price-lists/{id}",
            "Get a price list",
            Auth::Admin,
            &[],
            ok(Schema::Ref("PriceList")),
        ),
        patch(
            "/api/v1/admin/price-lists/{id}",
            "Update a price list",
            Auth::Admin,
            "UpdatePriceListRequest",
            ok(Schema::Ref("PriceList")),
        ),
        del(
            "/api/v1/admin/price-lists/{id}",
            "Delete a price list",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/prices",
            "List prices",
            Auth::Admin,
            &["limit", "offset", "price_list_id", "variant_id", "currency"],
            ok(Schema::Page("Price")),
        ),
        post(
            "/api/v1/admin/prices",
            "Create a price",
            Auth::Admin,
            Some("PriceRequest"),
            created(Schema::Ref("Price")),
        ),
        post(
            "/api/v1/admin/prices/bulk",
            "Bulk upsert prices",
            Auth::Admin,
            Some("BulkPricesRequest"),
            ok(Schema::Array("Price")),
        ),
        get(
            "/api/v1/admin/prices/calculate",
            "Calculate a single variant price",
            Auth::Admin,
            &[
                "variant_id",
                "currency",
                "quantity",
                "customer_group_id",
                "sales_channel_id",
            ],
            ok(Schema::NullableRef("SelectedPrice")),
        ),
        post(
            "/api/v1/admin/prices/calculate",
            "Calculate prices for several variants",
            Auth::Admin,
            Some("CalculatePricesRequest"),
            ok(Schema::MapOf("SelectedPrice")),
        ),
        get(
            "/api/v1/admin/prices/{id}",
            "Get a price",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Price")),
        ),
        patch(
            "/api/v1/admin/prices/{id}",
            "Update a price",
            Auth::Admin,
            "UpdatePriceRequest",
            ok(Schema::Ref("Price")),
        ),
        del(
            "/api/v1/admin/prices/{id}",
            "Delete a price",
            Auth::Admin,
            no_content(),
        ),
    ]
}

fn admin_promotion_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/promotions",
            "List promotions",
            Auth::Admin,
            &[
                "limit",
                "offset",
                "status",
                "is_automatic",
                "promotion_type",
            ],
            ok(Schema::Page("Promotion")),
        ),
        post(
            "/api/v1/admin/promotions",
            "Create a promotion",
            Auth::Admin,
            Some("CreatePromotionRequest"),
            created(Schema::Ref("Promotion")),
        ),
        get(
            "/api/v1/admin/promotions/{id}",
            "Get a promotion",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Promotion")),
        ),
        patch(
            "/api/v1/admin/promotions/{id}",
            "Update a promotion",
            Auth::Admin,
            "UpdatePromotionRequest",
            ok(Schema::Ref("Promotion")),
        ),
        del(
            "/api/v1/admin/promotions/{id}",
            "Delete a promotion",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/promotions/{id}/usage",
            "Get promotion usage",
            Auth::Admin,
            &[],
            ok(Schema::Ref("PromotionUsage")),
        ),
        get(
            "/api/v1/admin/coupons",
            "List coupons",
            Auth::Admin,
            &["limit", "offset", "promotion_id"],
            ok(Schema::Page("Coupon")),
        ),
        post(
            "/api/v1/admin/coupons",
            "Create a coupon",
            Auth::Admin,
            Some("CreateCouponRequest"),
            created(Schema::Ref("Coupon")),
        ),
        post(
            "/api/v1/admin/coupons/validate",
            "Validate a coupon",
            Auth::Admin,
            Some("ValidateCouponRequest"),
            ok(Schema::Ref("CouponValidation")),
        ),
        get(
            "/api/v1/admin/coupons/{id}",
            "Get a coupon",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Coupon")),
        ),
        patch(
            "/api/v1/admin/coupons/{id}",
            "Update a coupon",
            Auth::Admin,
            "UpdateCouponRequest",
            ok(Schema::Ref("Coupon")),
        ),
        del(
            "/api/v1/admin/coupons/{id}",
            "Delete a coupon",
            Auth::Admin,
            no_content(),
        ),
    ]
}

fn admin_campaign_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/campaigns",
            "List campaigns",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("Campaign")),
        ),
        post(
            "/api/v1/admin/campaigns",
            "Create a campaign",
            Auth::Admin,
            Some("CreateCampaignRequest"),
            created(Schema::Ref("Campaign")),
        ),
        get(
            "/api/v1/admin/campaigns/{id}",
            "Get a campaign",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Campaign")),
        ),
        patch(
            "/api/v1/admin/campaigns/{id}",
            "Update a campaign",
            Auth::Admin,
            "UpdateCampaignRequest",
            ok(Schema::Ref("Campaign")),
        ),
        del(
            "/api/v1/admin/campaigns/{id}",
            "Delete a campaign",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/campaigns/{id}/promotions",
            "List promotions linked to a campaign",
            Auth::Admin,
            &[],
            ok(Schema::Array("String")),
        ),
        post(
            "/api/v1/admin/campaigns/{id}/promotions",
            "Link a promotion to a campaign",
            Auth::Admin,
            Some("LinkPromotionRequest"),
            no_content(),
        ),
        del(
            "/api/v1/admin/campaigns/{id}/promotions/{promotion_id}",
            "Unlink a promotion from a campaign",
            Auth::Admin,
            no_content(),
        ),
    ]
}

fn admin_region_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/regions",
            "List regions",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("RegionDto")),
        ),
        post(
            "/api/v1/admin/regions",
            "Create a region",
            Auth::Admin,
            Some("CreateRegionRequest"),
            created(Schema::Ref("RegionDto")),
        ),
        get(
            "/api/v1/admin/regions/{id}",
            "Get a region",
            Auth::Admin,
            &[],
            ok(Schema::Ref("RegionDto")),
        ),
        patch(
            "/api/v1/admin/regions/{id}",
            "Update a region",
            Auth::Admin,
            "UpdateRegionRequest",
            ok(Schema::Ref("RegionDto")),
        ),
        del(
            "/api/v1/admin/regions/{id}",
            "Delete a region",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/publishable-api-keys",
            "List publishable API keys",
            Auth::Admin,
            &[],
            ok(Schema::Array("PublishableKeyDto")),
        ),
        post(
            "/api/v1/admin/publishable-api-keys",
            "Create a publishable API key",
            Auth::Admin,
            Some("CreatePublishableKeyRequest"),
            created(Schema::Ref("CreatedPublishableKeyDto")),
        ),
        del(
            "/api/v1/admin/publishable-api-keys/{id}",
            "Revoke a publishable API key",
            Auth::Admin,
            no_content(),
        ),
    ]
}

fn admin_shipping_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/shipping/profiles",
            "List shipping profiles",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("ShippingProfile")),
        ),
        post(
            "/api/v1/admin/shipping/profiles",
            "Create a shipping profile",
            Auth::Admin,
            Some("CreateShippingProfileRequest"),
            created(Schema::Ref("ShippingProfile")),
        ),
        get(
            "/api/v1/admin/shipping/profiles/{id}",
            "Get a shipping profile",
            Auth::Admin,
            &[],
            ok(Schema::Ref("ShippingProfile")),
        ),
        patch(
            "/api/v1/admin/shipping/profiles/{id}",
            "Update a shipping profile",
            Auth::Admin,
            "UpdateShippingProfileRequest",
            ok(Schema::Ref("ShippingProfile")),
        ),
        del(
            "/api/v1/admin/shipping/profiles/{id}",
            "Delete a shipping profile",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/shipping/fulfillment-sets",
            "List fulfillment sets",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("FulfillmentSet")),
        ),
        post(
            "/api/v1/admin/shipping/fulfillment-sets",
            "Create a fulfillment set",
            Auth::Admin,
            Some("CreateFulfillmentSetRequest"),
            created(Schema::Ref("FulfillmentSet")),
        ),
        get(
            "/api/v1/admin/shipping/fulfillment-sets/{id}",
            "Get a fulfillment set",
            Auth::Admin,
            &[],
            ok(Schema::Ref("FulfillmentSet")),
        ),
        patch(
            "/api/v1/admin/shipping/fulfillment-sets/{id}",
            "Update a fulfillment set",
            Auth::Admin,
            "UpdateFulfillmentSetRequest",
            ok(Schema::Ref("FulfillmentSet")),
        ),
        del(
            "/api/v1/admin/shipping/fulfillment-sets/{id}",
            "Delete a fulfillment set",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/shipping/providers",
            "List registered fulfillment providers",
            Auth::Admin,
            &[],
            ok(Schema::Array("String")),
        ),
        get(
            "/api/v1/admin/shipping/options",
            "List shipping options",
            Auth::Admin,
            &["limit", "offset", "profile_id"],
            ok(Schema::Page("ShippingOption")),
        ),
        post(
            "/api/v1/admin/shipping/options",
            "Create a shipping option",
            Auth::Admin,
            Some("CreateShippingOptionRequest"),
            created(Schema::Ref("ShippingOption")),
        ),
        get(
            "/api/v1/admin/shipping/options-list",
            "List shipping options for a destination",
            Auth::Admin,
            &["country", "subtotal"],
            ok(Schema::Array("ResolvedShippingOption")),
        ),
        get(
            "/api/v1/admin/shipping/options/{id}",
            "Get a shipping option",
            Auth::Admin,
            &[],
            ok(Schema::Ref("ShippingOption")),
        ),
        patch(
            "/api/v1/admin/shipping/options/{id}",
            "Update a shipping option",
            Auth::Admin,
            "UpdateShippingOptionRequest",
            ok(Schema::Ref("ShippingOption")),
        ),
        del(
            "/api/v1/admin/shipping/options/{id}",
            "Delete a shipping option",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/shipping/options/{id}/rates",
            "List shipping rates",
            Auth::Admin,
            &[],
            ok(Schema::Array("ShippingRate")),
        ),
        post(
            "/api/v1/admin/shipping/options/{id}/rates",
            "Create a shipping rate",
            Auth::Admin,
            Some("CreateShippingRateRequest"),
            created(Schema::Ref("ShippingRate")),
        ),
        get(
            "/api/v1/admin/shipping/options/{id}/rate",
            "Select a shipping rate for a destination",
            Auth::Admin,
            &["country", "province", "subtotal"],
            ok(Schema::NullableRef("SelectedShippingRate")),
        ),
        post(
            "/api/v1/admin/shipping/options/{id}/locations",
            "Assign a location to a shipping option",
            Auth::Admin,
            Some("AddShippingLocationRequest"),
            no_content(),
        ),
        del(
            "/api/v1/admin/shipping/options/{id}/locations/{location_id}",
            "Remove a location from a shipping option",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/shipping/rates/{id}",
            "Get a shipping rate",
            Auth::Admin,
            &[],
            ok(Schema::Ref("ShippingRate")),
        ),
        patch(
            "/api/v1/admin/shipping/rates/{id}",
            "Update a shipping rate",
            Auth::Admin,
            "UpdateShippingRateRequest",
            ok(Schema::Ref("ShippingRate")),
        ),
        del(
            "/api/v1/admin/shipping/rates/{id}",
            "Delete a shipping rate",
            Auth::Admin,
            no_content(),
        ),
    ]
}

fn admin_tax_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/tax/categories",
            "List tax categories",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("TaxCategory")),
        ),
        post(
            "/api/v1/admin/tax/categories",
            "Create a tax category",
            Auth::Admin,
            Some("CreateTaxCategoryRequest"),
            created(Schema::Ref("TaxCategory")),
        ),
        get(
            "/api/v1/admin/tax/categories/{id}",
            "Get a tax category",
            Auth::Admin,
            &[],
            ok(Schema::Ref("TaxCategory")),
        ),
        patch(
            "/api/v1/admin/tax/categories/{id}",
            "Update a tax category",
            Auth::Admin,
            "UpdateTaxCategoryRequest",
            ok(Schema::Ref("TaxCategory")),
        ),
        del(
            "/api/v1/admin/tax/categories/{id}",
            "Delete a tax category",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/tax/regions",
            "List tax regions",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("TaxRegion")),
        ),
        post(
            "/api/v1/admin/tax/regions",
            "Create a tax region",
            Auth::Admin,
            Some("CreateTaxRegionRequest"),
            created(Schema::Ref("TaxRegion")),
        ),
        get(
            "/api/v1/admin/tax/regions/{id}",
            "Get a tax region",
            Auth::Admin,
            &[],
            ok(Schema::Ref("TaxRegion")),
        ),
        patch(
            "/api/v1/admin/tax/regions/{id}",
            "Update a tax region",
            Auth::Admin,
            "UpdateTaxRegionRequest",
            ok(Schema::Ref("TaxRegion")),
        ),
        del(
            "/api/v1/admin/tax/regions/{id}",
            "Delete a tax region",
            Auth::Admin,
            no_content(),
        ),
        get(
            "/api/v1/admin/tax/rates",
            "List tax rates for a region",
            Auth::Admin,
            &["region_id"],
            ok(Schema::Array("TaxRate")),
        ),
        post(
            "/api/v1/admin/tax/rates",
            "Create a tax rate",
            Auth::Admin,
            Some("CreateTaxRateRequest"),
            created(Schema::Ref("TaxRate")),
        ),
        get(
            "/api/v1/admin/tax/rates/{id}",
            "Get a tax rate",
            Auth::Admin,
            &[],
            ok(Schema::Ref("TaxRate")),
        ),
        patch(
            "/api/v1/admin/tax/rates/{id}",
            "Update a tax rate",
            Auth::Admin,
            "UpdateTaxRateRequest",
            ok(Schema::Ref("TaxRate")),
        ),
        del(
            "/api/v1/admin/tax/rates/{id}",
            "Delete a tax rate",
            Auth::Admin,
            no_content(),
        ),
        post(
            "/api/v1/admin/tax/calculate",
            "Calculate tax for a destination",
            Auth::Admin,
            Some("TaxCalculateRequest"),
            ok(Schema::Ref("TaxCalculationResponse")),
        ),
    ]
}

fn admin_theme_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/themes",
            "List registered themes",
            Auth::Admin,
            PAGED,
            ok(Schema::Page("Theme")),
        ),
        post(
            "/api/v1/admin/themes",
            "Register a theme",
            Auth::Admin,
            Some("RegisterThemeRequest"),
            created(Schema::Ref("Theme")),
        ),
        get(
            "/api/v1/admin/themes/active",
            "Get the active theme",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Theme")),
        ),
        post(
            "/api/v1/admin/themes/rollback",
            "Roll back to the previous theme",
            Auth::Admin,
            None,
            ok(Schema::Ref("Theme")),
        ),
        get(
            "/api/v1/admin/themes/{id}",
            "Get a theme",
            Auth::Admin,
            &[],
            ok(Schema::Ref("Theme")),
        ),
        del(
            "/api/v1/admin/themes/{id}",
            "Delete a theme",
            Auth::Admin,
            no_content(),
        ),
        patch(
            "/api/v1/admin/themes/{id}/settings",
            "Replace theme settings",
            Auth::Admin,
            "UpdateThemeSettingsRequest",
            ok(Schema::Ref("Theme")),
        ),
        post(
            "/api/v1/admin/themes/{id}/activate",
            "Activate a theme",
            Auth::Admin,
            None,
            ok(Schema::Ref("Theme")),
        ),
    ]
}

fn admin_webhook_routes() -> Vec<Route> {
    vec![
        get(
            "/api/v1/admin/webhooks",
            "List webhook endpoints",
            Auth::Admin,
            &[],
            ok(Schema::Array("WebhookEndpoint")),
        ),
        post(
            "/api/v1/admin/webhooks",
            "Create a webhook endpoint",
            Auth::Admin,
            Some("CreateWebhookRequest"),
            created(Schema::Ref("CreatedWebhookEndpoint")),
        ),
        get(
            "/api/v1/admin/webhooks/{id}",
            "Get a webhook endpoint",
            Auth::Admin,
            &[],
            ok(Schema::Ref("WebhookEndpoint")),
        ),
        patch(
            "/api/v1/admin/webhooks/{id}",
            "Update a webhook endpoint",
            Auth::Admin,
            "UpdateWebhookRequest",
            ok(Schema::Ref("WebhookEndpoint")),
        ),
        del(
            "/api/v1/admin/webhooks/{id}",
            "Delete a webhook endpoint",
            Auth::Admin,
            no_content(),
        ),
        post(
            "/api/v1/admin/webhooks/{id}/rotate-secret",
            "Rotate a webhook signing secret",
            Auth::Admin,
            None,
            ok(Schema::Ref("RotatedSecret")),
        ),
        get(
            "/api/v1/admin/webhooks/{id}/deliveries",
            "List webhook deliveries",
            Auth::Admin,
            &["limit"],
            ok(Schema::Array("WebhookDelivery")),
        ),
        post(
            "/api/v1/admin/webhook-deliveries/{id}/replay",
            "Replay a webhook delivery",
            Auth::Admin,
            None,
            ok(Schema::Ref("WebhookDelivery")),
        ),
    ]
}

// -------------------------------------------------------------------------
// Storefront and internal routes
// -------------------------------------------------------------------------

fn store_routes() -> Vec<Route> {
    vec![
        post(
            "/api/v1/store/carts",
            "Create a cart",
            Auth::CustomerOptional,
            Some("CreateCartRequest"),
            created(Schema::Ref("Cart")),
        ),
        get(
            "/api/v1/store/carts/{cart_id}",
            "Get a cart",
            Auth::CustomerOptional,
            &[],
            ok(Schema::Ref("Cart")),
        ),
        post(
            "/api/v1/store/carts/{cart_id}/items",
            "Add an item to a cart",
            Auth::CustomerOptional,
            Some("AddItemRequest"),
            ok(Schema::Ref("Cart")),
        ),
        delq(
            "/api/v1/store/carts/{cart_id}/items",
            "Clear cart items",
            Auth::CustomerOptional,
            &["country_code", "province_code"],
            ok(Schema::Ref("Cart")),
        ),
        patch(
            "/api/v1/store/carts/{cart_id}/items/{item_id}",
            "Update a cart item quantity",
            Auth::CustomerOptional,
            "UpdateItemRequest",
            ok(Schema::Ref("Cart")),
        ),
        delq(
            "/api/v1/store/carts/{cart_id}/items/{item_id}",
            "Remove a cart item",
            Auth::CustomerOptional,
            &["country_code", "province_code"],
            ok(Schema::Ref("Cart")),
        ),
        post(
            "/api/v1/store/carts/{cart_id}/email",
            "Set the cart contact email",
            Auth::CustomerOptional,
            Some("SetEmailRequest"),
            ok(Schema::Ref("Cart")),
        ),
        post(
            "/api/v1/store/carts/{cart_id}/customer",
            "Attach the authenticated customer to a cart",
            Auth::Customer,
            None,
            ok(Schema::Ref("Cart")),
        ),
        post(
            "/api/v1/store/carts/{cart_id}/addresses",
            "Set a cart shipping or billing address",
            Auth::CustomerOptional,
            Some("SetCartAddressRequest"),
            ok(Schema::Ref("Cart")),
        ),
        post(
            "/api/v1/store/carts/{cart_id}/shipping-method",
            "Set the cart shipping method",
            Auth::CustomerOptional,
            Some("SetShippingMethodRequest"),
            ok(Schema::Ref("Cart")),
        ),
        post(
            "/api/v1/store/carts/{cart_id}/discounts",
            "Apply a discount code to a cart",
            Auth::CustomerOptional,
            Some("DiscountRequest"),
            ok(Schema::Ref("Cart")),
        ),
        delq(
            "/api/v1/store/carts/{cart_id}/discounts/{code}",
            "Remove a discount code from a cart",
            Auth::CustomerOptional,
            &["country_code", "province_code"],
            ok(Schema::Ref("Cart")),
        ),
        post(
            "/api/v1/store/checkouts",
            "Create a checkout from a cart",
            Auth::CustomerOptional,
            Some("CreateCheckoutRequest"),
            created(Schema::Ref("CheckoutSession")),
        ),
        get(
            "/api/v1/store/checkouts/{checkout_id}",
            "Get a checkout",
            Auth::CustomerOptional,
            &[],
            ok(Schema::Ref("CheckoutSession")),
        ),
        post(
            "/api/v1/store/checkouts/{checkout_id}/complete",
            "Complete a checkout",
            Auth::CustomerOptional,
            Some("CompleteCheckoutRequest"),
            ok(Schema::Ref("CompleteCheckoutResponse")),
        ),
        post(
            "/api/v1/store/checkouts/{checkout_id}/cancel",
            "Cancel a checkout",
            Auth::CustomerOptional,
            None,
            ok(Schema::Ref("CheckoutSession")),
        ),
        get(
            "/api/v1/store/checkouts/{checkout_id}/history",
            "List checkout state history",
            Auth::CustomerOptional,
            &[],
            ok(Schema::Array("CheckoutStateHistory")),
        ),
        get(
            "/api/v1/store/products",
            "List published products",
            Auth::Public,
            &[
                "limit",
                "offset",
                "category_id",
                "collection_id",
                "tag",
                "query",
                "currency_code",
                "region_id",
                "customer_group_id",
                "quantity",
            ],
            ok(Schema::Page("StoreProduct")),
        ),
        get(
            "/api/v1/store/products/{handle}",
            "Get a published product by handle",
            Auth::Public,
            &[
                "currency_code",
                "region_id",
                "customer_group_id",
                "quantity",
            ],
            ok(Schema::Ref("StoreProduct")),
        ),
        get(
            "/api/v1/store/categories",
            "List published categories",
            Auth::Public,
            PAGED,
            ok(Schema::Page("Category")),
        ),
        get(
            "/api/v1/store/collections",
            "List published collections",
            Auth::Public,
            PAGED,
            ok(Schema::Page("Collection")),
        ),
        get(
            "/api/v1/store/search",
            "Search published products",
            Auth::Public,
            &["q", "limit", "offset"],
            ok(Schema::Page("Product")),
        ),
        get(
            "/api/v1/store/settings/public",
            "Get public store settings",
            Auth::Public,
            &[],
            ok(Schema::Ref("SettingsMap")),
        ),
        get(
            "/api/v1/store/pages/{handle}",
            "Get a published content page",
            Auth::Public,
            &[],
            ok(Schema::Ref("Page")),
        ),
        get(
            "/api/v1/store/navigation/{handle}",
            "Get a navigation tree",
            Auth::Public,
            &[],
            ok(Schema::Ref("NavigationTree")),
        ),
        post(
            "/api/v1/store/customers",
            "Register a storefront customer",
            Auth::Public,
            Some("RegisterCustomerRequest"),
            ok(Schema::Ref("CustomerSessionResponse")),
        ),
        post(
            "/api/v1/store/customers/login",
            "Log in a storefront customer",
            Auth::Public,
            Some("CustomerLoginRequest"),
            ok(Schema::Ref("CustomerSessionResponse")),
        ),
        post(
            "/api/v1/store/customers/logout",
            "Log out the current storefront customer",
            Auth::Customer,
            None,
            no_content(),
        ),
        get(
            "/api/v1/store/customers/me",
            "Get the current customer",
            Auth::Customer,
            &[],
            ok(Schema::Ref("Customer")),
        ),
        patch(
            "/api/v1/store/customers/me",
            "Update the current customer",
            Auth::Customer,
            "UpdateProfileRequest",
            ok(Schema::Ref("Customer")),
        ),
        get(
            "/api/v1/store/customers/me/addresses",
            "List the current customer's addresses",
            Auth::Customer,
            &[],
            ok(Schema::Array("Address")),
        ),
        post(
            "/api/v1/store/customers/me/addresses",
            "Add an address for the current customer",
            Auth::Customer,
            Some("AddressRequest"),
            created(Schema::Ref("Address")),
        ),
        patch(
            "/api/v1/store/customers/me/addresses/{address_id}",
            "Update the current customer's address",
            Auth::Customer,
            "AddressRequest",
            ok(Schema::Ref("Address")),
        ),
        del(
            "/api/v1/store/customers/me/addresses/{address_id}",
            "Remove the current customer's address",
            Auth::Customer,
            no_content(),
        ),
        get(
            "/api/v1/store/customers/me/orders",
            "List the current customer's orders",
            Auth::Customer,
            PAGED,
            ok(Schema::Page("Order")),
        ),
        get(
            "/api/v1/store/customers/me/orders/{order_id}",
            "Get one of the current customer's orders",
            Auth::Customer,
            &[],
            ok(Schema::Ref("Order")),
        ),
        get(
            "/api/v1/store/customers/me/groups",
            "List the current customer's groups",
            Auth::Customer,
            &[],
            ok(Schema::Array("CustomerGroup")),
        ),
        post(
            "/api/v1/store/customers/me/password",
            "Change the current customer's password",
            Auth::Customer,
            Some("ChangePasswordRequest"),
            no_content(),
        ),
        get(
            "/api/v1/store/products/{handle}/availability",
            "Get variant availability for a product",
            Auth::Public,
            &[],
            ok(Schema::Array("VariantAvailability")),
        ),
        get(
            "/api/v1/store/shipping/options",
            "List shipping options for a destination",
            Auth::Public,
            &["country_code", "subtotal"],
            ok(Schema::Array("ResolvedShippingOption")),
        ),
        get(
            "/api/v1/store/theme",
            "Get the active storefront theme",
            Auth::Public,
            &[],
            ok(Schema::Ref("StoreTheme")),
        ),
        get(
            "/api/v1/store/regions",
            "List storefront regions",
            Auth::Public,
            PAGED,
            ok(Schema::Page("RegionDto")),
        ),
        get(
            "/api/v1/store/regions/{id}",
            "Get a storefront region",
            Auth::Public,
            &[],
            ok(Schema::Ref("RegionDto")),
        ),
    ]
}

fn internal_routes() -> Vec<Route> {
    store_routes()
        .into_iter()
        .map(|mut route| {
            route.path = route
                .path
                .replacen("/api/v1/store", "/internal/storefront", 1);
            route
        })
        .collect()
}

fn webhook_routes() -> Vec<Route> {
    vec![post(
        "/api/v1/webhooks/payments/{provider}",
        "Receive a payment provider webhook",
        Auth::Public,
        Some("JsonObject"),
        ok(Schema::Ref("PaymentWebhookRecord")),
    )]
}

fn operations_routes() -> Vec<Route> {
    vec![
        get(
            "/health",
            "Service health summary",
            Auth::Public,
            &[],
            ok(Schema::Ref("HealthStatus")),
        ),
        get(
            "/health/live",
            "Liveness probe",
            Auth::Public,
            &[],
            ok(Schema::Ref("LivenessStatus")),
        ),
        get(
            "/health/ready",
            "Readiness probe",
            Auth::Public,
            &[],
            ok(Schema::Ref("ReadinessStatus")),
        ),
        get(
            "/metrics",
            "Prometheus metrics",
            Auth::Public,
            &[],
            ok(Schema::Text),
        ),
    ]
}

// -------------------------------------------------------------------------
// Operation building
// -------------------------------------------------------------------------

fn operation(route: &Route) -> Value {
    let mut parameters = path_parameters(&route.path);
    parameters.extend(route.query.iter().map(|name| query_parameter(name)));
    if route.auth == Auth::Admin && route.path.starts_with("/api/v1/admin") {
        parameters.push(store_header_parameter());
    }
    if route.method.is_mutating() && route.auth.uses_cookie() {
        parameters.push(csrf_parameter());
    }
    if route.tag() == "Webhooks" {
        parameters.push(webhook_signature_parameter());
    }
    if route.tag() == "Internal" {
        parameters.push(internal_secret_parameter());
    }

    let mut operation = json!({
        "operationId": operation_id(route.method, &route.path),
        "tags": [route.tag()],
        "summary": route.summary,
        "security": security_for(route.auth),
        "parameters": parameters,
        "responses": responses_for(route),
    });
    if let Some(body) = route.body {
        operation["requestBody"] = request_body(body);
    }
    operation
}

fn operation_id(method: Method, path: &str) -> String {
    let mut id = String::from(method.as_str());
    for segment in path.trim_matches('/').split('/') {
        if let Some(name) = segment
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            id.push_str("_by_");
            id.push_str(&sanitize(name));
        } else if !segment.is_empty() {
            id.push('_');
            id.push_str(&sanitize(segment));
        }
    }
    id
}

fn sanitize(segment: &str) -> String {
    segment
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn security_for(auth: Auth) -> Value {
    match auth {
        Auth::Public => json!([]),
        Auth::Admin => json!([{"adminSession": []}, {"apiToken": []}]),
        Auth::Customer => json!([{"customerSession": []}]),
        Auth::CustomerOptional => json!([{"customerSession": []}, {}]),
    }
}

fn responses_for(route: &Route) -> Value {
    let mut responses = Map::new();
    responses.insert(
        route.success.status.to_string(),
        success_response(route.success.schema),
    );
    if route.tag() == "Operations" {
        if route.path == "/health/ready" {
            responses.insert(
                "503".to_owned(),
                json!({
                    "description": "Service is not ready",
                    "content": {"application/json": {"schema": schema_ref("ReadinessStatus")}},
                }),
            );
        }
    } else {
        for (code, description) in ERROR_RESPONSES {
            responses.insert((*code).to_owned(), error_response(description));
        }
    }
    Value::Object(responses)
}

const ERROR_RESPONSES: &[(&str, &str)] = &[
    ("400", "Validation error"),
    ("401", "Authentication required"),
    ("403", "Permission denied"),
    ("404", "Resource not found"),
    ("409", "State conflict"),
    ("429", "Rate limited"),
    ("500", "Internal error"),
];

fn error_response(description: &str) -> Value {
    json!({
        "description": description,
        "content": {
            "application/json": {
                "schema": {"$ref": "#/components/schemas/ErrorEnvelope"},
            },
        },
    })
}

fn success_response(schema: Schema) -> Value {
    match schema {
        Schema::None => json!({"description": "No content"}),
        Schema::Text => json!({
            "description": "Successful response",
            "content": {"text/plain": {"schema": {"type": "string"}}},
        }),
        _ => json!({
            "description": "Successful response",
            "content": {"application/json": {"schema": schema_value(schema)}},
        }),
    }
}

fn schema_value(schema: Schema) -> Value {
    match schema {
        Schema::None => json!({"type": "null"}),
        Schema::Ref(name) => schema_ref(name),
        Schema::Page(name) => page_schema(name),
        Schema::Array(name) => array_of(schema_ref(name)),
        Schema::MapOf(name) => map_of(schema_ref(name)),
        Schema::NullableRef(name) => nullable(schema_ref(name)),
        Schema::Text => json!({"type": "string"}),
    }
}

fn page_schema(name: &str) -> Value {
    json!({
        "type": "object",
        "required": ["data", "pagination"],
        "properties": {
            "data": {"type": "array", "items": schema_ref(name)},
            "pagination": schema_ref("PageInfo"),
        },
    })
}

fn request_body(schema_name: &str) -> Value {
    json!({
        "required": true,
        "content": {
            "application/json": {
                "schema": {"$ref": format!("#/components/schemas/{schema_name}")},
            },
        },
    })
}

fn path_parameters(path: &str) -> Vec<Value> {
    path.split('/')
        .filter_map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
                .map(path_parameter)
        })
        .collect()
}

fn path_parameter(name: &str) -> Value {
    let schema = match name {
        "display_number" | "version" => json!({"type": "integer", "format": "int64"}),
        _ => json!({"type": "string"}),
    };
    json!({
        "name": name,
        "in": "path",
        "required": true,
        "schema": schema,
    })
}

fn query_parameter(name: &str) -> Value {
    match name {
        "limit" => json!({
            "name": "limit",
            "in": "query",
            "description": "Maximum number of records to return (1-250).",
            "schema": {
                "type": "integer",
                "format": "int64",
                "minimum": 1,
                "maximum": 250,
                "default": 50,
            },
        }),
        "offset" => json!({
            "name": "offset",
            "in": "query",
            "description": "Number of records to skip.",
            "schema": {"type": "integer", "format": "int64", "minimum": 0, "default": 0},
        }),
        "include_deleted" | "is_automatic" => {
            json!({"name": name, "in": "query", "schema": {"type": "boolean"}})
        }
        "quantity" | "subtotal" | "amount_minor" | "ttl_seconds" => json!({
            "name": name,
            "in": "query",
            "schema": {"type": "integer", "format": "int64"},
        }),
        _ => json!({"name": name, "in": "query", "schema": {"type": "string"}}),
    }
}

fn store_header_parameter() -> Value {
    json!({
        "name": "x-store-id",
        "in": "header",
        "required": false,
        "description": "Store selected by the authenticated administrator.",
        "schema": {"type": "string"},
    })
}

fn csrf_parameter() -> Value {
    json!({
        "name": "x-csrf-token",
        "in": "header",
        "required": false,
        "description": "Double-submit CSRF token; must match the session CSRF cookie.",
        "schema": {"type": "string"},
    })
}

fn internal_secret_parameter() -> Value {
    json!({
        "name": "x-fastcart-internal-secret",
        "in": "header",
        "required": true,
        "description": "Shared secret that gates the internal SSR-to-commerce boundary.",
        "schema": {"type": "string"},
    })
}

fn webhook_signature_parameter() -> Value {
    json!({
        "name": "x-webhook-signature",
        "in": "header",
        "required": false,
        "description": "Provider-specific signature header verified by the payment adapter.",
        "schema": {"type": "string"},
    })
}

// -------------------------------------------------------------------------
// Components
// -------------------------------------------------------------------------

fn tags() -> Value {
    json!([
        {"name": "Platform", "description": "Installation, stores, domains and members."},
        {"name": "Admin", "description": "Store-scoped administration."},
        {"name": "Store", "description": "Public storefront operations."},
        {"name": "Webhooks", "description": "Provider callbacks."},
        {"name": "Internal", "description": "Trusted SSR-to-commerce boundary."},
        {"name": "Operations", "description": "Health and metrics."},
    ])
}

fn security_schemes() -> Value {
    json!({
        "adminSession": {
            "type": "apiKey",
            "in": "cookie",
            "name": "fc_admin_session",
            "description": "Administrator session cookie. Unsafe requests also require the \
                x-csrf-token header matching the fc_csrf cookie.",
        },
        "apiToken": {
            "type": "http",
            "scheme": "bearer",
            "bearerFormat": "opaque",
            "description": "Bearer API token created through the platform API.",
        },
        "customerSession": {
            "type": "apiKey",
            "in": "cookie",
            "name": "fc_customer_session",
            "description": "Storefront customer session cookie. Unsafe requests also require \
                the x-csrf-token header matching the fc_customer_csrf cookie.",
        },
    })
}

fn schemas() -> Value {
    let mut schemas = Map::new();
    // Insert schemas one by one so the macro expansion depth stays bounded.
    macro_rules! schema {
        ($($name:literal : $schema:expr),* $(,)?) => {
            $(schemas.insert($name.to_owned(), $schema);)*
        };
    }
    schema! {
        // Errors and pagination.
        "String": string(),
        "ErrorEnvelope": object(&[
            ("error", object(&[
                ("code", string()),
                ("message", string()),
                ("request_id", nullable(string())),
            ], &["code", "message"])),
        ], &["error"]),


        "PageInfo": object(&[
            ("limit", integer()),
            ("offset", integer()),
            ("total", integer()),
        ], &["limit", "offset", "total"]),


        "PageMeta": object(&[
            ("limit", integer()),
            ("offset", integer()),
            ("total", integer()),
            ("has_more", boolean()),
        ], &["limit", "offset", "total"]),


        "JsonObject": json!({"type": "object", "additionalProperties": true}),


        "Totals": object(&[
            ("currency_code", string()),
            ("subtotal", integer()),
            ("discount_total", integer()),
            ("shipping_total", integer()),
            ("tax_total", integer()),
            ("grand_total", integer()),
        ], &["currency_code", "subtotal", "grand_total"]),



        // Identity and platform resources.
        "User": object(&[
            ("id", string()),
            ("email", string()),
            ("first_name", optional_string()),
            ("last_name", optional_string()),
            ("status", string_enum(&["active", "disabled"])),
            ("email_verified_at", nullable(integer())),
        ], &["id", "email", "status"]),


        "Store": object(&[
            ("id", string()),
            ("name", string()),
            ("slug", string()),
            ("status", string()),
            ("primary_domain", optional_string()),
            ("locale", string()),
            ("currency_code", string()),
            ("timezone", string()),
            ("created_at", integer()),
            ("updated_at", integer()),
        ], &["id", "name", "slug", "status"]),


        "Domain": object(&[
            ("id", string()),
            ("store_id", string()),
            ("hostname", string()),
            ("is_primary", boolean()),
            ("verified_at", nullable(integer())),
        ], &["id", "store_id", "hostname", "is_primary"]),


        "Membership": object(&[
            ("id", string()),
            ("user_id", string()),
            ("store_id", string()),
            ("status", string()),
            ("roles", array_of(string())),
            ("created_at", integer()),
            ("updated_at", integer()),
        ], &["id", "user_id", "store_id", "roles"]),


        "Role": object(&[
            ("name", string()),
            ("description", string()),
        ], &["name", "description"]),


        "ApiToken": object(&[
            ("id", string()),
            ("user_id", string()),
            ("store_id", optional_string()),
            ("name", string()),
            ("prefix", string()),
            ("scopes", array_of(string())),
            ("expires_at", nullable(integer())),
            ("revoked_at", nullable(integer())),
            ("last_used_at", nullable(integer())),
            ("created_at", integer()),
        ], &["id", "user_id", "name", "prefix", "scopes"]),


        "CreatedApiToken": object(&[
            ("token", string()),
            ("record", schema_ref("ApiToken")),
        ], &["token", "record"]),


        "ProvisioningResponse": object(&[
            ("store", schema_ref("Store")),
            ("steps", array_of(string())),
            ("already_active", boolean()),
        ], &["store", "steps", "already_active"]),


        "LoginResponse": object(&[
            ("user", schema_ref("User")),
            ("csrf_token", string()),
            ("expires_at", integer()),
        ], &["user", "csrf_token", "expires_at"]),


        "MeResponse": object(&[
            ("user", schema_ref("User")),
            ("store_id", optional_string()),
            ("permissions", array_of(string())),
        ], &["user", "permissions"]),


        "Variant": object(&[
            ("id", string()),
            ("product_id", string()),
            ("title", string()),
            ("sku", optional_string()),
            ("barcode", optional_string()),
            ("status", string_enum(&["draft", "active", "archived"])),
            ("weight", nullable(number())),
            ("dimensions", nullable(schema_ref("Dimensions"))),
            ("manage_inventory", boolean()),
            ("allow_backorder", boolean()),
            ("metadata", schema_ref("Metadata")),
            ("created_at", integer()),
            ("updated_at", integer()),
        ], &["id", "product_id", "title"]),


        "LineItem": object(&[
            ("id", string()),
            ("cart_id", optional_string()),
            ("variant_id", string()),
            ("title", string()),
            ("quantity", integer()),
            ("unit_price", integer()),
            ("total_price", integer()),
            ("metadata", schema_ref("Metadata")),
        ], &["id", "variant_id", "quantity"]),


        "Checkout": object(&[
            ("id", string()),
            ("cart_id", string()),
            ("status", string_enum(&["pending", "completed", "canceled", "expired"])),
            ("email", optional_string()),
            ("payment_provider", string()),
            ("payment_session_id", optional_string()),
            ("totals", schema_ref("Totals")),
            ("created_at", integer()),
            ("updated_at", integer()),
            ("expires_at", nullable(integer())),
        ], &["id", "cart_id", "status"]),


        "Customer": object(&[
            ("id", string()),
            ("email", string()),
            ("first_name", optional_string()),
            ("last_name", optional_string()),
            ("phone", optional_string()),
            ("status", string_enum(&["active", "disabled"])),
            ("metadata", schema_ref("Metadata")),
            ("created_at", integer()),
            ("updated_at", integer()),
        ], &["id", "email", "status"]),


        "CustomerGroup": object(&[
            ("id", string()),
            ("name", string()),
            ("metadata", schema_ref("Metadata")),
            ("created_at", integer()),
            ("updated_at", integer()),
        ], &["id", "name"]),


        "CustomerSessionResponse": object(&[
            ("customer", schema_ref("Customer")),
            ("csrf_token", string()),
            ("expires_at", integer()),
        ], &["customer", "csrf_token", "expires_at"]),


        "CompleteCheckoutResponse": object(&[
            ("checkout", schema_ref("Checkout")),
            ("order", schema_ref("Order")),
            ("payment", schema_ref("Payment")),
        ], &["checkout", "order", "payment"]),


        "VariantAvailability": object(&[
            ("variant_id", string()),
            ("available", integer()),
            ("manage_inventory", boolean()),
            ("allow_backorder", boolean()),
        ], &["variant_id", "available"]),


        "SettingsMap": map_of(string()),


        "SettingValue": object(&[
            ("key", string()),
            ("value", string()),
        ], &["key", "value"]),


        "ReindexResponse": object(&[
            ("indexed", boolean()),
        ], &["indexed"]),


        "InventoryAvailability": object(&[
            ("variant_id", string()),
            ("available", integer()),
        ], &["variant_id", "available"]),


        "TransferResponse": object(&[
            ("from", schema_ref("InventoryLevel")),
            ("to", schema_ref("InventoryLevel")),
        ], &["from", "to"]),


        "ReleasedResponse": object(&[("released", integer())], &["released"]),


        "CommittedResponse": object(&[("committed", integer())], &["committed"]),


        "PaymentRefund": object(&[
            ("id", string()),
            ("payment_id", optional_string()),
            ("order_id", optional_string()),
            ("amount", integer()),
            ("currency_code", string()),
            ("status", string()),
            ("provider_refund_id", optional_string()),
            ("reason", optional_string()),
            ("created_at", integer()),
            ("updated_at", integer()),
        ], &["id", "amount", "currency_code", "status"]),


        "CouponValidation": object(&[
            ("valid", boolean()),
            ("coupon", nullable(schema_ref("Coupon"))),
        ], &["valid"]),


        "TaxCalculationResponse": object(&[
            ("lines", array_of(schema_ref("TaxLine"))),
            ("totals", schema_ref("TaxBreakdown")),
        ], &["lines", "totals"]),


        "CreatedWebhookEndpoint": object(&[
            ("id", string()),
            ("url", string()),
            ("events", array_of(string())),
            ("is_active", boolean()),
            ("metadata", schema_ref("Metadata")),
            ("created_at", integer()),
            ("updated_at", integer()),
            ("secret", string()),
        ], &["id", "url", "events", "secret"]),


        "RotatedSecret": object(&[("secret", string())], &["secret"]),


        "WebhookSignature": object(&[
            ("signature", string()),
            ("timestamp", integer()),
            ("algorithm", string()),
        ], &["signature", "timestamp"]),


        "WebhookEnvelope": object(&[
            ("id", string()),
            ("type", string()),
            ("created_at", integer()),
            ("store_id", string()),
            ("data", json!({"type": "object", "additionalProperties": true})),
            ("signature", schema_ref("WebhookSignature")),
        ], &["id", "type", "created_at", "store_id", "data"]),


        "PaymentWebhook": object(&[
            ("provider", string()),
            ("provider_event_id", string()),
            ("event_type", string()),
            ("payload", json!({"type": "object", "additionalProperties": true})),
            ("signature", schema_ref("WebhookSignature")),
        ], &["provider", "provider_event_id", "event_type", "payload"]),



        // Themes and operations.
        "ThemeManifest": object(&[
            ("id", string()),
            ("name", string()),
            ("version", string()),
            ("runtime", string()),
            ("entry", string()),
            ("settings_schema", optional_string()),
            ("assets_manifest", optional_string()),
        ], &["id", "name", "version", "runtime", "entry"]),


        "Theme": object(&[
            ("id", string()),
            ("manifest", schema_ref("ThemeManifest")),
            ("bundle_path", string()),
            ("settings_schema", nullable(json!({"type": "object", "additionalProperties": true}))),
            ("settings", json!({"type": "object", "additionalProperties": true})),
            ("is_active", boolean()),
            ("created_at", integer()),
            ("updated_at", integer()),
        ], &["id", "manifest", "bundle_path", "settings", "is_active"]),


        "StoreTheme": object(&[
            ("id", string()),
            ("manifest", schema_ref("ThemeManifest")),
            ("bundle_path", string()),
            ("settings", json!({"type": "object", "additionalProperties": true})),
        ], &["id", "manifest", "bundle_path", "settings"]),


        "RegisterThemeRequest": object(&[
            ("manifest", schema_ref("ThemeManifest")),
            ("bundle_path", string()),
            ("settings_schema", nullable(json!({"type": "object", "additionalProperties": true}))),
            ("settings", nullable(json!({"type": "object", "additionalProperties": true}))),
        ], &["manifest", "bundle_path"]),


        "UpdateThemeSettingsRequest": object(&[
            ("settings", json!({"type": "object", "additionalProperties": true})),
        ], &["settings"]),


        "HealthStatus": object(&[
            ("status", string()),
            ("service", string()),
            ("version", string()),
        ], &["status"]),


        "LivenessStatus": object(&[("status", string())], &["status"]),


        "ReadinessStatus": object(&[
            ("status", string()),
            ("service", optional_string()),
            ("version", optional_string()),
        ], &["status"]),



        // Request bodies: authentication and platform.
        "LoginRequest": object(&[
            ("email", string()),
            ("password", string()),
        ], &["email", "password"]),


        "SetupRequest": object(&[
            ("email", string()),
            ("password", string()),
            ("first_name", optional_string()),
            ("last_name", optional_string()),
        ], &["email", "password"]),


        "CreateUserRequest": object(&[
            ("email", string()),
            ("password", string()),
            ("first_name", optional_string()),
            ("last_name", optional_string()),
            ("store_id", optional_string()),
            ("roles", array_of(string())),
        ], &["email", "password"]),


        "CreateApiTokenRequest": object(&[
            ("name", string()),
            ("store_id", optional_string()),
            ("scopes", array_of(string())),
            ("ttl_seconds", nullable(integer())),
        ], &["name"]),


        "CreateStoreRequest": object(&[
            ("name", string()),
            ("slug", string()),
            ("primary_domain", string()),
            ("locale", optional_string()),
            ("currency_code", optional_string()),
            ("timezone", optional_string()),
        ], &["name", "slug", "primary_domain"]),


        "UpdateStoreRequest": object(&[
            ("name", optional_string()),
            ("locale", optional_string()),
            ("currency_code", optional_string()),
            ("timezone", optional_string()),
        ], &[]),


        "DeleteStoreRequest": object(&[
            ("backup", boolean()),
            ("purge_data", boolean()),
        ], &[]),


        "DomainRequest": object(&[
            ("hostname", string()),
            ("is_primary", boolean()),
        ], &["hostname"]),


        "PrimaryDomainRequest": object(&[("hostname", string())], &["hostname"]),


        "MembershipRequest": object(&[
            ("user_id", string()),
            ("roles", array_of(string())),
        ], &["user_id", "roles"]),



        // Request bodies: catalog and settings.
        "CreateProductRequest": object(&[
            ("title", string()),
            ("handle", optional_string()),
            ("subtitle", optional_string()),
            ("description", optional_string()),
            ("thumbnail", optional_string()),
            ("weight", nullable(number())),
            ("dimensions", nullable(schema_ref("Dimensions"))),
            ("metadata", schema_ref("Metadata")),
            ("variants", array_of(json!({"type": "object", "additionalProperties": true}))),
            ("options", array_of(json!({"type": "object", "additionalProperties": true}))),
            ("category_ids", array_of(string())),
            ("collection_ids", array_of(string())),
            ("tag_values", array_of(string())),
            ("images", array_of(string())),
        ], &["title"]),


        "UpdateProductRequest": object(&[
            ("title", optional_string()),
            ("handle", optional_string()),
            ("subtitle", optional_string()),
            ("description", optional_string()),
            ("thumbnail", optional_string()),
            ("status", optional_string()),
            ("weight", nullable(number())),
            ("dimensions", nullable(schema_ref("Dimensions"))),
            ("metadata", schema_ref("Metadata")),
        ], &[]),


        "SetProductSalesChannelsRequest": object(&[
            ("sales_channel_ids", array_of(string())),
        ], &[]),


        "StoreProduct": json!({
            "allOf": [
                schema_ref("Product"),
                {
                    "type": "object",
                    "properties": {
                        "calculated_price": nullable(schema_ref("CalculatedPrice")),
                    },
                },
            ],
        }),


        "CreateVariantRequest": object(&[
            ("title", string()),
            ("sku", optional_string()),
            ("barcode", optional_string()),
            ("status", optional_string()),
            ("weight", nullable(number())),
            ("dimensions", nullable(schema_ref("Dimensions"))),
            ("manage_inventory", nullable(boolean())),
            ("allow_backorder", nullable(boolean())),
            ("tax_category_id", optional_string()),
            ("metadata", schema_ref("Metadata")),
            ("option_value_ids", array_of(string())),
            ("option_values", array_of(json!({"type": "object", "additionalProperties": true}))),
        ], &["title"]),


        "UpdateVariantRequest": object(&[
            ("title", optional_string()),
            ("sku", optional_string()),
            ("barcode", optional_string()),
            ("status", optional_string()),
            ("weight", nullable(number())),
            ("dimensions", nullable(schema_ref("Dimensions"))),
            ("manage_inventory", nullable(boolean())),
            ("allow_backorder", nullable(boolean())),
            ("tax_category_id", optional_string()),
            ("metadata", schema_ref("Metadata")),
        ], &[]),


        "CreateImageRequest": object(&[
            ("url", string()),
            ("alt", optional_string()),
            ("variant_id", optional_string()),
            ("rank", nullable(integer())),
            ("metadata", schema_ref("Metadata")),
        ], &["url"]),


        "ImageReorderRequest": object(&[("image_ids", array_of(string()))], &["image_ids"]),


        "CreateCategoryRequest": object(&[
            ("title", string()),
            ("handle", optional_string()),
            ("parent_id", optional_string()),
            ("description", optional_string()),
            ("is_active", nullable(boolean())),
            ("is_internal", nullable(boolean())),
            ("rank", nullable(integer())),
            ("metadata", schema_ref("Metadata")),
        ], &["title"]),


        "UpdateCategoryRequest": object(&[
            ("title", optional_string()),
            ("handle", optional_string()),
            ("parent_id", optional_string()),
            ("description", optional_string()),
            ("is_active", nullable(boolean())),
            ("is_internal", nullable(boolean())),
            ("rank", nullable(integer())),
            ("metadata", schema_ref("Metadata")),
        ], &[]),


        "CreateCollectionRequest": object(&[
            ("title", string()),
            ("handle", optional_string()),
            ("description", optional_string()),
            ("is_published", nullable(boolean())),
            ("rank", nullable(integer())),
            ("metadata", schema_ref("Metadata")),
        ], &["title"]),


        "UpdateCollectionRequest": object(&[
            ("title", optional_string()),
            ("handle", optional_string()),
            ("description", optional_string()),
            ("is_published", nullable(boolean())),
            ("rank", nullable(integer())),
            ("metadata", schema_ref("Metadata")),
        ], &[]),


        "CollectionProductsRequest": object(&[("product_ids", array_of(string()))], &["product_ids"]),


        "TagRequest": object(&[("value", string())], &["value"]),


        "ProductTagRequest": object(&[("tag_id", string())], &["tag_id"]),


        "SettingsRequest": object(&[("values", map_of(string()))], &["values"]),


        "SettingRequest": object(&[("value", string())], &["value"]),


        "CreateSalesChannelRequest": object(&[
            ("name", string()),
            ("description", optional_string()),
            ("metadata", schema_ref("Metadata")),
        ], &["name"]),


        "UpdateSalesChannelRequest": object(&[
            ("name", optional_string()),
            ("description", optional_string()),
            ("is_enabled", nullable(boolean())),
            ("metadata", schema_ref("Metadata")),
        ], &[]),



        // Request bodies: content.
        "CreatePageRequest": object(&[
            ("title", string()),
            ("handle", string()),
            ("body", optional_string()),
            ("status", optional_string()),
            ("metadata", schema_ref("Metadata")),
        ], &["title", "handle"]),


        "UpdatePageRequest": object(&[
            ("title", optional_string()),
            ("handle", optional_string()),
            ("body", optional_string()),
            ("status", optional_string()),
            ("metadata", schema_ref("Metadata")),
        ], &[]),


        "CreateNavigationRequest": object(&[
            ("title", string()),
            ("handle", string()),
            ("metadata", schema_ref("Metadata")),
        ], &["title", "handle"]),


        "UpdateNavigationRequest": object(&[
            ("title", optional_string()),
            ("handle", optional_string()),
            ("is_active", nullable(boolean())),
            ("metadata", schema_ref("Metadata")),
        ], &[]),


        "NavigationItemRequest": object(&[
            ("title", string()),
            ("url", optional_string()),
            ("parent_id", optional_string()),
            ("item_type", optional_string()),
            ("rank", nullable(integer())),
            ("metadata", schema_ref("Metadata")),
        ], &["title"]),


        "NavigationItemReorderRequest": object(&[("item_ids", array_of(string()))], &["item_ids"]),


        "CreateRedirectRequest": object(&[
            ("from_path", string()),
            ("to_path", string()),
            ("status_code", nullable(integer())),
            ("metadata", schema_ref("Metadata")),
        ], &["from_path", "to_path"]),



        // Request bodies: customers.
        "UpdateCustomerRequest": object(&[
            ("email", optional_string()),
            ("first_name", optional_string()),
            ("last_name", optional_string()),
            ("phone", optional_string()),
            ("status", optional_string()),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "AddressRequest": object(&[
            ("is_default_shipping", nullable(boolean())),
            ("is_default_billing", nullable(boolean())),
            ("first_name", optional_string()),
            ("last_name", optional_string()),
            ("company", optional_string()),
            ("address_1", optional_string()),
            ("address_2", optional_string()),
            ("city", optional_string()),
            ("province", optional_string()),
            ("postal_code", optional_string()),
            ("country_code", optional_string()),
            ("phone", optional_string()),
            ("metadata", schema_ref("Metadata")),
        ], &[]),


        "CreateGroupRequest": object(&[
            ("name", string()),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["name"]),


        "GroupCustomerRequest": object(&[("customer_id", string())], &["customer_id"]),



        // Request bodies: inventory.
        "CreateInventoryLocationRequest": object(&[
            ("name", string()),
            ("address", nullable(schema_ref("Metadata"))),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["name"]),


        "UpdateInventoryLocationRequest": object(&[
            ("name", optional_string()),
            ("address", nullable(schema_ref("Metadata"))),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "CreateInventoryItemRequest": object(&[
            ("sku", optional_string()),
            ("variant_id", optional_string()),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "InventoryAdjustmentRequest": object(&[
            ("inventory_item_id", string()),
            ("location_id", string()),
            ("quantity_delta", integer()),
            ("reason", string()),
            ("reference", optional_string()),
        ], &["inventory_item_id", "location_id", "quantity_delta", "reason"]),


        "InventoryTransferRequest": object(&[
            ("inventory_item_id", string()),
            ("from_location_id", string()),
            ("to_location_id", string()),
            ("quantity", integer()),
            ("reason", optional_string()),
        ], &["inventory_item_id", "from_location_id", "to_location_id", "quantity"]),


        "ReservationRequest": object(&[
            ("lines", array_of(json!({"type": "object", "additionalProperties": true}))),
            ("checkout_id", string()),
            ("ttl_seconds", integer()),
        ], &["lines", "checkout_id", "ttl_seconds"]),


        "ReleaseReservationsRequest": object(&[
            ("reservation_ids", array_of(string())),
        ], &[]),


        "CommitReservationsRequest": object(&[
            ("checkout_id", string()),
            ("order_id", string()),
        ], &["checkout_id", "order_id"]),



        // Request bodies: orders and payments.
        "CancelOrderRequest": object(&[("reason", optional_string())], &[]),


        "FulfillOrderRequest": object(&[
            ("provider_id", optional_string()),
            ("provider_data", nullable(schema_ref("Metadata"))),
            ("items", array_of(schema_ref("FulfillmentItemRequest"))),
            ("tracking_numbers", array_of(string())),
            ("shipped_at", nullable(integer())),
            ("partial", boolean()),
        ], &[]),


        "FulfillmentItemRequest": object(&[
            ("order_item_id", string()),
            ("quantity", integer()),
        ], &["order_item_id", "quantity"]),


        "RefundOrderRequest": object(&[
            ("payment_id", string()),
            ("amount", integer()),
            ("currency", string()),
            ("reason", optional_string()),
        ], &["payment_id", "amount", "currency"]),


        "CreatePaymentRefundRequest": object(&[
            ("amount", integer()),
            ("reason", optional_string()),
            ("idempotency_key", optional_string()),
        ], &["amount"]),


        "FailSessionRequest": object(&[("reason", optional_string())], &[]),



        // Request bodies: pricing and promotions.
        "CreatePriceListRequest": object(&[
            ("title", string()),
            ("description", optional_string()),
            ("price_type", optional_string()),
            ("status", optional_string()),
            ("starts_at", nullable(integer())),
            ("ends_at", nullable(integer())),
            ("metadata", nullable(schema_ref("Metadata"))),
            ("rules", array_of(json!({"type": "object", "additionalProperties": true}))),
        ], &["title"]),


        "UpdatePriceListRequest": object(&[
            ("title", optional_string()),
            ("description", optional_string()),
            ("price_type", optional_string()),
            ("status", optional_string()),
            ("starts_at", nullable(integer())),
            ("ends_at", nullable(integer())),
            ("metadata", nullable(schema_ref("Metadata"))),
            ("rules", array_of(json!({"type": "object", "additionalProperties": true}))),
        ], &[]),


        "PriceRequest": object(&[
            ("price_list_id", optional_string()),
            ("variant_id", string()),
            ("currency_code", string()),
            ("amount", integer()),
            ("min_quantity", nullable(integer())),
            ("max_quantity", nullable(integer())),
            ("customer_group_id", optional_string()),
            ("sales_channel_id", optional_string()),
            ("starts_at", nullable(integer())),
            ("ends_at", nullable(integer())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["variant_id", "currency_code", "amount"]),


        "UpdatePriceRequest": object(&[
            ("price_list_id", nullable(string())),
            ("variant_id", optional_string()),
            ("currency_code", optional_string()),
            ("amount", nullable(integer())),
            ("min_quantity", nullable(integer())),
            ("max_quantity", nullable(integer())),
            ("customer_group_id", nullable(string())),
            ("sales_channel_id", nullable(string())),
            ("starts_at", nullable(integer())),
            ("ends_at", nullable(integer())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "BulkPricesRequest": object(&[
            ("prices", array_of(schema_ref("PriceRequest"))),
        ], &["prices"]),


        "CalculatePricesRequest": object(&[
            ("variant_ids", array_of(string())),
            ("currency", string()),
            ("quantity", nullable(integer())),
            ("customer_group_id", optional_string()),
            ("sales_channel_id", optional_string()),
        ], &["variant_ids", "currency"]),


        "CreatePromotionRequest": object(&[
            ("code", optional_string()),
            ("title", string()),
            ("promotion_type", string()),
            ("value", integer()),
            ("status", optional_string()),
            ("is_automatic", nullable(boolean())),
            ("starts_at", nullable(integer())),
            ("ends_at", nullable(integer())),
            ("usage_limit", nullable(integer())),
            ("metadata", nullable(schema_ref("Metadata"))),
            ("rules", array_of(json!({"type": "object", "additionalProperties": true}))),
        ], &["title", "promotion_type", "value"]),


        "UpdatePromotionRequest": object(&[
            ("code", nullable(string())),
            ("title", optional_string()),
            ("promotion_type", optional_string()),
            ("value", nullable(integer())),
            ("status", optional_string()),
            ("is_automatic", nullable(boolean())),
            ("starts_at", nullable(integer())),
            ("ends_at", nullable(integer())),
            ("usage_limit", nullable(integer())),
            ("metadata", nullable(schema_ref("Metadata"))),
            ("rules", array_of(json!({"type": "object", "additionalProperties": true}))),
        ], &[]),


        "CreateCouponRequest": object(&[
            ("code", string()),
            ("promotion_id", string()),
            ("usage_limit", nullable(integer())),
            ("starts_at", nullable(integer())),
            ("ends_at", nullable(integer())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["code", "promotion_id"]),


        "UpdateCouponRequest": object(&[
            ("code", optional_string()),
            ("promotion_id", optional_string()),
            ("usage_limit", nullable(integer())),
            ("starts_at", nullable(integer())),
            ("ends_at", nullable(integer())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "ValidateCouponRequest": object(&[("code", string())], &["code"]),



        // Request bodies: shipping and tax.
        "CreateShippingProfileRequest": object(&[
            ("name", string()),
            ("profile_type", optional_string()),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["name"]),


        "UpdateShippingProfileRequest": object(&[
            ("name", optional_string()),
            ("profile_type", optional_string()),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "CreateShippingOptionRequest": object(&[
            ("profile_id", string()),
            ("name", string()),
            ("amount_minor", integer()),
            ("currency", string()),
            ("provider_id", optional_string()),
            ("provider_data", nullable(schema_ref("Metadata"))),
            ("fulfillment_set_id", optional_string()),
            ("is_enabled", nullable(boolean())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["profile_id", "name", "amount_minor", "currency"]),


        "UpdateShippingOptionRequest": object(&[
            ("name", optional_string()),
            ("amount_minor", nullable(integer())),
            ("currency", optional_string()),
            ("provider_id", optional_string()),
            ("provider_data", nullable(schema_ref("Metadata"))),
            ("fulfillment_set_id", nullable(string())),
            ("is_enabled", nullable(boolean())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "CreateFulfillmentSetRequest": object(&[
            ("name", string()),
            ("type", optional_string()),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["name"]),


        "UpdateFulfillmentSetRequest": object(&[
            ("name", optional_string()),
            ("type", optional_string()),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "CreateShippingRateRequest": object(&[
            ("country_code", optional_string()),
            ("province_code", optional_string()),
            ("amount_minor", integer()),
            ("currency", string()),
            ("min_subtotal", nullable(integer())),
            ("max_subtotal", nullable(integer())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["amount_minor", "currency"]),


        "UpdateShippingRateRequest": object(&[
            ("country_code", nullable(string())),
            ("province_code", nullable(string())),
            ("amount_minor", nullable(integer())),
            ("currency", optional_string()),
            ("min_subtotal", nullable(integer())),
            ("max_subtotal", nullable(integer())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "AddShippingLocationRequest": object(&[("location_id", string())], &["location_id"]),


        "CreateTaxCategoryRequest": object(&[
            ("name", string()),
            ("code", string()),
            ("description", optional_string()),
        ], &["name", "code"]),


        "UpdateTaxCategoryRequest": object(&[
            ("name", optional_string()),
            ("code", optional_string()),
            ("description", nullable(string())),
        ], &[]),


        "CreateTaxRegionRequest": object(&[
            ("name", string()),
            ("country_code", string()),
            ("province_code", optional_string()),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["name", "country_code"]),


        "UpdateTaxRegionRequest": object(&[
            ("name", optional_string()),
            ("country_code", optional_string()),
            ("province_code", nullable(string())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "CreateTaxRateRequest": object(&[
            ("region_id", string()),
            ("category_id", optional_string()),
            ("name", string()),
            ("rate_bps", integer()),
            ("is_inclusive", nullable(boolean())),
            ("is_default", nullable(boolean())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["region_id", "name", "rate_bps"]),


        "UpdateTaxRateRequest": object(&[
            ("category_id", nullable(string())),
            ("name", optional_string()),
            ("rate_bps", nullable(integer())),
            ("is_inclusive", nullable(boolean())),
            ("is_default", nullable(boolean())),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &[]),


        "TaxCalculateRequest": object(&[
            ("country", string()),
            ("province", optional_string()),
            ("category_id", optional_string()),
            ("amount_minor", nullable(integer())),
            ("lines", array_of(json!({"type": "object", "additionalProperties": true}))),
        ], &["country"]),



        // Request bodies: webhooks.
        "CreateWebhookRequest": object(&[
            ("url", string()),
            ("events", array_of(string())),
            ("secret", optional_string()),
            ("metadata", nullable(schema_ref("Metadata"))),
        ], &["url", "events"]),


        "UpdateWebhookRequest": object(&[
            ("url", optional_string()),
            ("events", array_of(string())),
            ("is_active", nullable(boolean())),
        ], &[]),



        // Request bodies: carts, checkouts and storefront customers.
        "CreateCartRequest": object(&[
            ("currency_code", string()),
            ("email", optional_string()),
            ("sales_channel_id", optional_string()),
            ("expires_in_seconds", nullable(integer())),
        ], &["currency_code"]),


        "AddItemRequest": object(&[
            ("variant_id", string()),
            ("quantity", integer()),
            ("country_code", optional_string()),
            ("province_code", optional_string()),
        ], &["variant_id", "quantity"]),


        "UpdateItemRequest": object(&[("quantity", integer())], &["quantity"]),


        "SetEmailRequest": object(&[("email", string())], &["email"]),


        "SetCartAddressRequest": object(&[
            ("type", string_enum(&["shipping", "billing"])),
            ("first_name", optional_string()),
            ("last_name", optional_string()),
            ("company", optional_string()),
            ("address_1", optional_string()),
            ("address_2", optional_string()),
            ("city", optional_string()),
            ("province", optional_string()),
            ("postal_code", optional_string()),
            ("country_code", optional_string()),
            ("phone", optional_string()),
        ], &["type"]),


        "SetShippingMethodRequest": object(&[
            ("shipping_option_id", string()),
            ("country_code", optional_string()),
            ("province_code", optional_string()),
        ], &["shipping_option_id"]),


        "DiscountRequest": object(&[("code", string())], &["code"]),


        "CreateCheckoutRequest": object(&[
            ("cart_id", string()),
            ("email", optional_string()),
            ("payment_provider", optional_string()),
            ("idempotency_key", optional_string()),
            ("expires_in_seconds", nullable(integer())),
            ("country_code", optional_string()),
            ("province_code", optional_string()),
        ], &["cart_id"]),


        "CompleteCheckoutRequest": object(&[
            ("country_code", optional_string()),
            ("province_code", optional_string()),
        ], &[]),


        "RegisterCustomerRequest": object(&[
            ("email", string()),
            ("password", string()),
            ("first_name", optional_string()),
            ("last_name", optional_string()),
        ], &["email", "password"]),


        "CustomerLoginRequest": object(&[
            ("email", string()),
            ("password", string()),
        ], &["email", "password"]),


        "UpdateProfileRequest": object(&[
            ("first_name", optional_string()),
            ("last_name", optional_string()),
            ("phone", optional_string()),
        ], &[]),


        "ChangePasswordRequest": object(&[
            ("current_password", string()),
            ("new_password", string()),
        ], &["current_password", "new_password"]),
    }
    for (name, schema) in generated_schemas() {
        schemas.insert(name, schema);
    }
    Value::Object(schemas)
}

// -------------------------------------------------------------------------
// Schema helpers
// -------------------------------------------------------------------------

fn object(properties: &[(&str, Value)], required: &[&str]) -> Value {
    let properties: Map<String, Value> = properties
        .iter()
        .map(|(name, schema)| ((*name).to_owned(), schema.clone()))
        .collect();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": true,
    })
}

fn schema_ref(name: &str) -> Value {
    json!({"$ref": format!("#/components/schemas/{name}")})
}

fn string() -> Value {
    json!({"type": "string"})
}

fn integer() -> Value {
    json!({"type": "integer", "format": "int64"})
}

fn number() -> Value {
    json!({"type": "number", "format": "double"})
}

fn boolean() -> Value {
    json!({"type": "boolean"})
}

fn string_enum(values: &[&str]) -> Value {
    json!({"type": "string", "enum": values})
}

fn optional_string() -> Value {
    nullable(string())
}

fn array_of(items: Value) -> Value {
    json!({"type": "array", "items": items})
}

fn map_of(values: Value) -> Value {
    json!({"type": "object", "additionalProperties": values})
}

fn nullable(schema: Value) -> Value {
    json!({"anyOf": [schema, {"type": "null"}]})
}

// -------------------------------------------------------------------------
// Coverage tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn document_has_required_metadata() {
        let document = document();
        assert_eq!(document["openapi"], "3.1.0");
        assert_eq!(document["info"]["title"], "Fastcart Commerce API");
        assert!(document["info"]["version"].is_string());
        assert!(document["info"]["description"].is_string());
        assert_eq!(document["servers"][0]["url"], "/");

        let tags: BTreeSet<&str> = document["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .filter_map(|tag| tag["name"].as_str())
            .collect();
        for expected in [
            "Platform",
            "Admin",
            "Store",
            "Webhooks",
            "Internal",
            "Operations",
        ] {
            assert!(tags.contains(expected), "missing tag {expected}");
        }

        let schemes = document["components"]["securitySchemes"]
            .as_object()
            .expect("security schemes");
        for expected in ["adminSession", "apiToken", "customerSession"] {
            assert!(
                schemes.contains_key(expected),
                "missing security scheme {expected}"
            );
        }
        assert_eq!(schemes["adminSession"]["name"], "fc_admin_session");
        assert_eq!(schemes["customerSession"]["name"], "fc_customer_session");
        assert_eq!(schemes["apiToken"]["scheme"], "bearer");
    }

    #[test]
    fn paths_cover_every_namespace() {
        let document = document();
        let paths = document["paths"].as_object().expect("paths");
        let count = |prefix: &str| paths.keys().filter(|path| path.starts_with(prefix)).count();

        assert_eq!(count("/api/v1/platform"), 20);
        assert_eq!(count("/api/v1/admin"), 158);
        assert_eq!(count("/api/v1/store"), 38);
        assert_eq!(count("/api/v1/webhooks"), 1);
        assert_eq!(count("/internal/storefront"), 38);
        assert_eq!(count("/health") + count("/metrics"), 4);
        assert_eq!(paths.len(), 259);

        for key in [
            "/api/v1/platform/stores",
            "/api/v1/platform/stores/{store_id}/domains",
            "/api/v1/admin/catalog/products",
            "/api/v1/admin/orders/{id}/refunds",
            "/api/v1/admin/themes",
            "/api/v1/store/checkouts/{checkout_id}/complete",
            "/api/v1/store/theme",
            "/api/v1/webhooks/payments/{provider}",
            "/internal/storefront/carts/{cart_id}",
            "/internal/storefront/theme",
            "/health/ready",
            "/metrics",
        ] {
            assert!(paths.contains_key(key), "missing path {key}");
        }
    }

    #[test]
    fn utoipa_schemas_match_rust_field_names() {
        let document = document();
        let schemas = document["components"]["schemas"]
            .as_object()
            .expect("schemas");
        let properties = |name: &str| {
            schemas[name]["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{name} has no properties"))
        };
        assert!(properties("Cart").contains_key("items"));
        assert!(!properties("Cart").contains_key("line_items"));
        assert!(properties("Order").contains_key("items"));
        assert!(!properties("Order").contains_key("line_items"));
        assert!(properties("CartTotals").contains_key("total"));
        assert!(!properties("CartTotals").contains_key("grand_total"));
        assert!(properties("InventoryLevel").contains_key("stocked_quantity"));
        assert!(properties("Coupon").contains_key("usage_count"));
        assert!(properties("SelectedPrice").contains_key("price_id"));
        let status = schemas["CheckoutStatus"]["enum"]
            .as_array()
            .expect("checkout status enum");
        assert!(status.iter().any(|value| value == "order_placed"));
    }

    #[test]
    fn operations_are_well_formed() {
        let document = document();
        let mut operation_count = 0;
        let mut operation_ids = BTreeSet::new();

        for (path, item) in document["paths"].as_object().expect("paths") {
            assert!(
                path.starts_with('/'),
                "path must start with a slash: {path}"
            );
            assert!(!path.contains(':'), "unsupported colon parameter in {path}");
            let expected_parameters: Vec<String> = path_parameters(path)
                .iter()
                .filter_map(|parameter| parameter["name"].as_str().map(str::to_owned))
                .collect();

            for (method, operation) in item.as_object().expect("path item") {
                assert!(
                    matches!(method.as_str(), "get" | "post" | "patch" | "put" | "delete"),
                    "unsupported method {method} on {path}"
                );
                operation_count += 1;

                let tags = operation["tags"].as_array().expect("tags");
                assert!(!tags.is_empty(), "{method} {path} has no tag");
                let summary = operation["summary"].as_str().expect("summary");
                assert!(!summary.is_empty(), "{method} {path} has no summary");

                let operation_id = operation["operationId"].as_str().expect("operationId");
                assert!(
                    operation_ids.insert(operation_id.to_owned()),
                    "duplicate operationId {operation_id}"
                );

                let parameters = operation["parameters"].as_array().expect("parameters");
                for name in &expected_parameters {
                    assert!(
                        parameters.iter().any(|parameter| {
                            parameter["name"].as_str() == Some(name.as_str())
                                && parameter["in"] == "path"
                                && parameter["required"].as_bool() == Some(true)
                        }),
                        "{method} {path} is missing path parameter {name}"
                    );
                }

                let responses = operation["responses"].as_object().expect("responses");
                assert!(
                    responses.keys().any(|status| status.starts_with('2')),
                    "{method} {path} has no 2xx response"
                );
                if tag_for(path) != "Operations" {
                    assert_eq!(
                        responses["400"]["content"]["application/json"]["schema"]["$ref"],
                        "#/components/schemas/ErrorEnvelope",
                        "{method} {path} does not reference ErrorEnvelope"
                    );
                }
            }
        }

        assert_eq!(operation_count, 362);
    }

    #[test]
    fn admin_mutations_document_request_and_csrf() {
        let document = document();
        let create = &document["paths"]["/api/v1/admin/catalog/products"]["post"];
        let parameters = create["parameters"].as_array().expect("parameters");
        assert!(
            parameters
                .iter()
                .any(|parameter| parameter["name"] == "x-store-id" && parameter["in"] == "header")
        );
        assert!(
            parameters
                .iter()
                .any(|parameter| parameter["name"] == "x-csrf-token"
                    && parameter["in"] == "header")
        );
        assert_eq!(
            create["requestBody"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/CreateProductRequest"
        );

        let storefront = &document["paths"]["/api/v1/store/carts"]["post"];
        let parameters = storefront["parameters"].as_array().expect("parameters");
        assert!(
            parameters
                .iter()
                .any(|parameter| parameter["name"] == "x-csrf-token")
        );
    }

    #[test]
    fn pagination_and_core_schemas_exist() {
        let document = document();
        let schemas = document["components"]["schemas"]
            .as_object()
            .expect("schemas");
        for name in [
            "ErrorEnvelope",
            "PageInfo",
            "Pagination",
            "PageMeta",
            "WebhookEnvelope",
            "PaymentWebhook",
            "Store",
            "User",
            "Product",
            "Variant",
            "Cart",
            "Checkout",
            "Order",
            "Customer",
            "WebhookEndpoint",
            "Theme",
            "Money",
            "Totals",
        ] {
            assert!(schemas.contains_key(name), "missing schema {name}");
        }

        let parameters = &document["paths"]["/api/v1/admin/catalog/products"]["get"]["parameters"];
        let parameters = parameters.as_array().expect("parameters");
        assert!(
            parameters
                .iter()
                .any(|parameter| parameter["name"] == "limit" && parameter["in"] == "query")
        );
        assert!(
            parameters
                .iter()
                .any(|parameter| parameter["name"] == "offset" && parameter["in"] == "query")
        );
    }

    #[test]
    fn every_schema_reference_resolves() {
        let document = document();
        let schemas = document["components"]["schemas"]
            .as_object()
            .expect("schemas");
        let mut stack = vec![document.clone()];
        while let Some(value) = stack.pop() {
            match value {
                Value::Object(map) => {
                    if let Some(name) = map
                        .get("$ref")
                        .and_then(Value::as_str)
                        .and_then(|reference| reference.strip_prefix("#/components/schemas/"))
                    {
                        assert!(
                            schemas.contains_key(name),
                            "dangling schema reference #/components/schemas/{name}"
                        );
                    }
                    stack.extend(map.values().cloned());
                }
                Value::Array(items) => stack.extend(items),
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn handler_returns_the_document() {
        let response = handler().await;
        assert_eq!(response.0["openapi"], "3.1.0");
    }
}
