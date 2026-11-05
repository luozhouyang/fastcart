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
    fastcart_module_asset::Asset,
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
    crate::routes::admin_workflows::WorkflowExecutionDto,
    crate::routes::admin_workflows::WorkflowStepDto,
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
    fastcart_module_order::AfterSalesFulfillment,
    fastcart_module_order::AfterSalesFulfillmentItem,
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
    crate::dto::ApiTokenDto,
    crate::dto::CreateApiTokenRequest,
    crate::dto::CreateStoreRequest,
    crate::dto::CreateUserRequest,
    crate::dto::CreatedApiTokenResponse,
    crate::dto::DeleteStoreRequest,
    crate::dto::DomainDto,
    crate::dto::DomainRequest,
    crate::dto::LoginRequest,
    crate::dto::LoginResponse,
    crate::dto::MeResponse,
    crate::dto::MembershipDto,
    crate::dto::MembershipRequest,
    crate::dto::PageInfo,
    crate::dto::PrimaryDomainRequest,
    crate::dto::ProvisioningResponse,
    crate::dto::RoleDto,
    crate::dto::SetupRequest,
    crate::dto::StoreDto,
    crate::dto::UpdateStoreRequest,
    crate::dto::UserDto,
    crate::dto::CheckoutDto,
    crate::dto::CartResponse,
    crate::dto::CategoryListResponse,
    crate::dto::CheckoutCompleteResponse,
    crate::dto::CheckoutHistoryResponse,
    crate::dto::CheckoutResponse,
    crate::dto::CollectionListResponse,
    crate::dto::CustomerAddressDto,
    crate::dto::CustomerAddressListResponse,
    crate::dto::CustomerAddressResponse,
    crate::dto::CustomerDto,
    crate::dto::CustomerGroupDto,
    crate::dto::CustomerGroupsResponse,
    crate::dto::CustomerResponse,
    crate::dto::CustomerStatusDto,
    crate::dto::LineItemDto,
    crate::dto::NavigationResponse,
    crate::dto::OrderListResponse,
    crate::dto::OrderResponse,
    crate::dto::PaymentWebhook,
    crate::dto::ProductListResponse,
    crate::dto::ProductResponse,
    crate::dto::ProductSearchResponse,
    crate::dto::RegionListResponse,
    crate::dto::RegionResponse,
    crate::dto::ShippingOptionsResponse,
    crate::dto::StorePageResponse,
    crate::dto::StoreThemeDto,
    crate::dto::StoreThemeResponse,
    crate::dto::ThemeDto,
    crate::dto::ThemeManifestDto,
    crate::dto::TotalsDto,
    crate::dto::VariantAvailabilityResponse,
    crate::dto::VariantDto,
    crate::dto::WebhookEnvelope,
    crate::dto::WebhookSignature,
    crate::routes::admin_catalog::ReindexResponse,
    crate::routes::admin_catalog::UpdateCategoryRequest,
    crate::routes::admin_catalog::UpdateCollectionRequest,
    crate::routes::admin_catalog::UpdateProductRequest,
    crate::routes::admin_catalog::UpdateSalesChannelRequest,
    crate::routes::admin_catalog::UpdateVariantRequest,
    crate::routes::admin_content::CreateNavigationRequest,
    crate::routes::admin_content::CreatePageRequest,
    crate::routes::admin_content::CreateRedirectRequest,
    crate::routes::admin_content::NavigationItemRequest,
    crate::routes::admin_content::PreviewTokenResponse,
    crate::routes::admin_content::RevokePreviewTokenRequest,
    crate::routes::admin_content::UpdateNavigationRequest,
    crate::routes::admin_content::UpdatePageRequest,
    crate::routes::admin_customers::AdminAddressRequest,
    crate::routes::admin_data_sources::DataPickerItem,
    crate::routes::admin_data_sources::DataPickerResponse,
    crate::routes::admin_theme::RegisterThemeRequest,
    crate::routes::admin_theme::UpdateThemeSettingsRequest,
    crate::routes::health::HealthStatus,
    crate::routes::health::LivenessStatus,
    crate::routes::health::ReadinessStatus,
    crate::routes::store_customer::AddressRequest,
    crate::routes::store_customer::CustomerSessionResponse,
    crate::routes::store_customer::PasswordResetRequest,
    crate::routes::admin_catalog::CollectionProductsRequest,
    crate::routes::admin_catalog::CreateCategoryRequest,
    crate::routes::admin_catalog::CreateCollectionRequest,
    crate::routes::admin_catalog::CreateImageRequest,
    crate::routes::admin_catalog::CreateOptionRequest,
    crate::routes::admin_catalog::CreateProductRequest,
    crate::routes::admin_catalog::CreateSalesChannelRequest,
    crate::routes::admin_catalog::CreateVariantRequest,
    crate::routes::admin_catalog::ImageReorderRequest,
    crate::routes::admin_catalog::ProductSalesChannelsRequest,
    crate::routes::admin_catalog::ProductTagRequest,
    crate::routes::admin_catalog::SettingRequest,
    crate::routes::admin_catalog::SettingsRequest,
    crate::routes::admin_catalog::TagRequest,
    crate::routes::admin_content::NavigationItemReorderRequest,
    crate::routes::admin_customers::CreateGroupRequest,
    crate::routes::admin_customers::GroupCustomerRequest,
    crate::routes::admin_customers::UpdateCustomerRequest,
    crate::routes::admin_inventory::AdjustmentRequest,
    crate::routes::admin_inventory::AvailabilityResponse,
    crate::routes::admin_inventory::CommitReservationsRequest,
    crate::routes::admin_inventory::CommittedResponse,
    crate::routes::admin_inventory::CreateItemRequest,
    crate::routes::admin_inventory::CreateLocationRequest,
    crate::routes::admin_inventory::ReleaseReservationsRequest,
    crate::routes::admin_inventory::ReleasedResponse,
    crate::routes::admin_inventory::ReservationLineRequest,
    crate::routes::admin_inventory::ReservationRequest,
    crate::routes::admin_inventory::TransferRequest,
    crate::routes::admin_inventory::TransferResponse,
    crate::routes::admin_inventory::UpdateLocationRequest,
    crate::routes::admin_orders::CancelOrderRequest,
    crate::routes::admin_orders::ClaimFulfillmentResponse,
    crate::routes::admin_orders::ClaimRefundResponse,
    crate::routes::admin_orders::ExchangeFulfillmentResponse,
    crate::routes::admin_orders::FulfillAfterSalesRequest,
    crate::routes::admin_orders::FulfillOrderRequest,
    crate::routes::admin_orders::FulfillmentItemRequest,
    crate::routes::admin_orders::RefundOrderRequest,
    crate::routes::admin_orders::ReturnRefundResponse,
    crate::routes::admin_payments::CreateRefundRequest,
    crate::routes::admin_payments::FailSessionRequest,
    crate::routes::admin_payments::RefundDto,
    crate::routes::admin_pricing::BulkPricesRequest,
    crate::routes::admin_pricing::CalculatePricesRequest,
    crate::routes::admin_pricing::CreatePriceListRequest,
    crate::routes::admin_pricing::PriceListRuleRequest,
    crate::routes::admin_pricing::PriceRequest,
    crate::routes::admin_pricing::UpdatePriceListRequest,
    crate::routes::admin_pricing::UpdatePriceRequest,
    crate::routes::admin_promotions::CouponValidationResponse,
    crate::routes::admin_promotions::CreateCouponRequest,
    crate::routes::admin_promotions::CreatePromotionRequest,
    crate::routes::admin_promotions::PromotionRuleRequest,
    crate::routes::admin_promotions::UpdateCouponRequest,
    crate::routes::admin_promotions::UpdatePromotionRequest,
    crate::routes::admin_promotions::ValidateCouponRequest,
    crate::routes::admin_shipping::AddLocationRequest,
    crate::routes::admin_shipping::CreateFulfillmentSetRequest,
    crate::routes::admin_shipping::CreateShippingOptionRequest,
    crate::routes::admin_shipping::CreateShippingProfileRequest,
    crate::routes::admin_shipping::CreateShippingRateRequest,
    crate::routes::admin_shipping::UpdateFulfillmentSetRequest,
    crate::routes::admin_shipping::UpdateShippingOptionRequest,
    crate::routes::admin_shipping::UpdateShippingProfileRequest,
    crate::routes::admin_shipping::UpdateShippingRateRequest,
    crate::routes::admin_tax::CreateTaxCategoryRequest,
    crate::routes::admin_tax::CreateTaxRateRequest,
    crate::routes::admin_tax::CreateTaxRegionRequest,
    crate::routes::admin_tax::TaxCalculateLineRequest,
    crate::routes::admin_tax::TaxCalculateRequest,
    crate::routes::admin_tax::TaxCalculationResponse,
    crate::routes::admin_tax::UpdateTaxCategoryRequest,
    crate::routes::admin_tax::UpdateTaxRateRequest,
    crate::routes::admin_tax::UpdateTaxRegionRequest,
    crate::routes::admin_webhooks::CreateWebhookRequest,
    crate::routes::admin_webhooks::CreatedWebhookDto,
    crate::routes::admin_webhooks::RotatedSecretDto,
    crate::routes::admin_webhooks::UpdateWebhookRequest,
    crate::routes::admin_webhooks::WebhookEndpointDto,
    crate::routes::admin_inquiry::CreateInquiryFormRequest,
    crate::routes::admin_inquiry::InquiryDetail,
    crate::routes::admin_inquiry::InquiryFormDetail,
    crate::routes::admin_inquiry::PublishInquiryFormRequest,
    crate::routes::admin_inquiry::SaveInquiryDraftRequest,
    crate::routes::admin_inquiry::UnreadCountResponse,
    crate::routes::admin_inquiry::UpdateInquiryFormRequest,
    crate::routes::admin_inquiry::UpdateInquiryStatusRequest,
    crate::routes::admin_notifications::CreateNotificationChannelRequest,
    crate::routes::admin_notifications::NotificationChannelDto,
    crate::routes::admin_notifications::NotificationDeliveryDto,
    crate::routes::admin_notifications::NotificationEventDto,
    crate::routes::admin_notifications::TestChannelResponse,
    crate::routes::admin_notifications::UpdateNotificationChannelRequest,
    crate::routes::store_inquiry::CreateInquiryRequest,
    crate::routes::store_inquiry::CreateInquiryResponse,
    crate::routes::store_inquiry::PublicInquiryForm,
    crate::routes::store_inquiry::PublicInquiryFormResponse,
    crate::routes::store_inquiry::PublicInquiryFormSettings,
    crate::routes::store_inquiry::UploadFileResponse,
    fastcart_module_inquiry::FieldKind,
    fastcart_module_inquiry::FieldOption,
    fastcart_module_inquiry::FieldValidation,
    fastcart_module_inquiry::FileFieldConfig,
    fastcart_module_inquiry::Inquiry,
    fastcart_module_inquiry::InquiryFile,
    fastcart_module_inquiry::InquiryForm,
    fastcart_module_inquiry::InquiryFormField,
    fastcart_module_inquiry::InquiryFormRevision,
    fastcart_module_inquiry::InquiryFormSettings,
    fastcart_module_inquiry::InquiryFormStatus,
    fastcart_module_inquiry::InquiryStatus,
    crate::routes::store_cart::AddItemRequest,
    crate::routes::store_cart::CartAddressType,
    crate::routes::store_cart::CompleteCheckoutRequest,
    crate::routes::store_cart::CreateCartRequest,
    crate::routes::store_cart::CreateCheckoutRequestDto,
    crate::routes::store_cart::DiscountRequest,
    crate::routes::store_cart::SetAddressRequest,
    crate::routes::store_cart::SetEmailRequest,
    crate::routes::store_cart::SetShippingMethodRequest,
    crate::routes::store_cart::UpdateItemRequest,
    crate::routes::store_catalog::StoreProduct,
    crate::routes::store_customer::AcceptedResponse,
    crate::routes::store_customer::ChangePasswordRequest,
    crate::routes::store_customer::CustomerLoginRequest,
    crate::routes::store_customer::EmailRequest,
    crate::routes::store_customer::EmailVerifiedResponse,
    crate::routes::store_customer::RegisterRequest,
    crate::routes::store_customer::ResetPasswordRequest,
    crate::routes::store_customer::TokenRequest,
    crate::routes::store_customer::UpdateProfileRequest,
    crate::routes::store_inventory::VariantAvailability,
)))]
struct GeneratedSchemas;

/// Builds the OpenAPI 3.1 document.
#[must_use]
pub fn document() -> Value {
    static DOCUMENT: OnceLock<Value> = OnceLock::new();
    DOCUMENT.get_or_init(build_document).clone()
}

fn build_document() -> Value {
    let generated = serde_json::to_value(generated_api()).unwrap_or(Value::Null);
    let paths = generated
        .get("paths")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Map::new()));

    let mut component_schemas = schemas();
    if let Some(generated_schemas) = generated
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
    {
        let target = component_schemas
            .as_object_mut()
            .expect("component schemas are always an object");
        for (name, schema) in generated_schemas {
            target.insert(name.clone(), schema.clone());
        }
    }

    let mut document = json!({
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
            "schemas": component_schemas,
        },
    });
    normalize_operations(&mut document);
    document
}

/// Adds metadata shared by every operation, including the standard error
/// envelope, authentication headers and operation identifiers.
fn normalize_operations(document: &mut Value) {
    let Some(paths) = document.get_mut("paths").and_then(Value::as_object_mut) else {
        return;
    };
    for (path, item) in paths.iter_mut() {
        let tag = tag_for(path);
        let path_parameters = path_parameters(path);
        let Some(operations) = item.as_object_mut() else {
            continue;
        };
        for (method, operation) in operations.iter_mut() {
            if method == "parameters" {
                continue;
            }
            let Some(operation) = operation.as_object_mut() else {
                continue;
            };

            let mut parameters = operation
                .get("parameters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for parameter in &path_parameters {
                let name = parameter["name"].as_str().unwrap_or_default();
                if !parameters
                    .iter()
                    .any(|existing| existing["name"].as_str() == Some(name))
                {
                    parameters.push(parameter.clone());
                }
            }
            if path.starts_with("/api/v1/admin")
                && !parameters
                    .iter()
                    .any(|parameter| parameter["name"] == "x-store-id")
            {
                parameters.push(store_header_parameter());
            }
            let mutating = !matches!(method.as_str(), "get" | "head" | "options");
            let cookie_auth = operation
                .get("security")
                .and_then(Value::as_array)
                .is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry.get("adminSession").is_some()
                            || entry.get("customerSession").is_some()
                    })
                });
            if mutating
                && (cookie_auth || path.starts_with("/api/v1/admin"))
                && !parameters
                    .iter()
                    .any(|parameter| parameter["name"] == "x-csrf-token")
            {
                parameters.push(csrf_parameter());
            }
            if path.starts_with("/internal/storefront")
                && !parameters
                    .iter()
                    .any(|parameter| parameter["name"] == "x-fastcart-internal-secret")
            {
                parameters.push(internal_secret_parameter());
            }
            if path.starts_with("/api/v1/webhooks")
                && !parameters
                    .iter()
                    .any(|parameter| parameter["name"] == "x-webhook-signature")
            {
                parameters.push(webhook_signature_parameter());
            }
            operation.insert("parameters".to_owned(), Value::Array(parameters));

            operation.insert(
                "operationId".to_owned(),
                Value::String(operation_id(
                    Method::parse(method).unwrap_or(Method::Get),
                    path,
                )),
            );
            if operation
                .get("summary")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                operation.insert(
                    "summary".to_owned(),
                    Value::String(format!("{} {}", method.to_uppercase(), path)),
                );
            }
            operation.insert("tags".to_owned(), json!([tag]));
            if tag != "Operations" {
                let responses = operation
                    .get_mut("responses")
                    .and_then(Value::as_object_mut)
                    .expect("generated operations always have responses");
                for (code, description) in ERROR_RESPONSES {
                    responses
                        .entry((*code).to_owned())
                        .or_insert_with(|| error_response(description));
                }
            }
        }
    }
}

fn generated_api() -> utoipa::openapi::OpenApi {
    use utoipa_axum::router::OpenApiRouter;

    let platform = OpenApiRouter::new()
        .merge(crate::routes::platform::openapi_router())
        .merge(crate::routes::auth::openapi_router())
        .merge(crate::routes::auth::setup_openapi_router());

    let admin = OpenApiRouter::new()
        .merge(crate::routes::admin::openapi_router())
        .merge(crate::routes::auth::openapi_router())
        .merge(crate::routes::admin_assets::openapi_router())
        .merge(crate::routes::admin_campaigns::openapi_router())
        .merge(crate::routes::admin_catalog::openapi_router())
        .merge(crate::routes::admin_content::openapi_router())
        .merge(crate::routes::admin_customers::openapi_router())
        .merge(crate::routes::admin_data_sources::openapi_router())
        .merge(crate::routes::admin_inventory::openapi_router())
        .merge(crate::routes::admin_orders::openapi_router())
        .merge(crate::routes::admin_payments::openapi_router())
        .merge(crate::routes::admin_pricing::openapi_router())
        .merge(crate::routes::admin_promotions::openapi_router())
        .merge(crate::routes::admin_regions::openapi_router())
        .merge(crate::routes::admin_shipping::openapi_router())
        .merge(crate::routes::admin_tax::openapi_router())
        .merge(crate::routes::admin_theme::openapi_router())
        .merge(crate::routes::admin_webhooks::openapi_router())
        .merge(crate::routes::admin_inquiry::openapi_router())
        .merge(crate::routes::admin_notifications::openapi_router())
        .merge(crate::routes::admin_workflows::openapi_router());

    let store = OpenApiRouter::new()
        .merge(crate::routes::store_assets::openapi_router())
        .merge(crate::routes::store_cart::openapi_router())
        .merge(crate::routes::store_catalog::openapi_router())
        .merge(crate::routes::store_content::openapi_router())
        .merge(crate::routes::store_customer::openapi_router())
        .merge(crate::routes::store_inquiry::openapi_router())
        .merge(crate::routes::store_inventory::openapi_router())
        .merge(crate::routes::store_regions::openapi_router())
        .merge(crate::routes::store_theme::openapi_router());

    let root: OpenApiRouter<std::sync::Arc<AppContext>> = OpenApiRouter::new()
        .merge(crate::routes::health::openapi_router())
        .nest("/api/v1/platform", platform)
        .nest("/api/v1/admin", admin)
        .nest("/api/v1/store", store.clone())
        .nest("/internal/storefront", store)
        .nest(
            "/api/v1/webhooks",
            crate::routes::webhooks::openapi_router(),
        );

    root.split_for_parts().1
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

    fn parse(value: &str) -> Option<Self> {
        match value {
            "get" => Some(Self::Get),
            "post" => Some(Self::Post),
            "patch" => Some(Self::Patch),
            "put" => Some(Self::Put),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
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
// Operation building
// -------------------------------------------------------------------------

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


        "PageMeta": object(&[
            ("limit", integer()),
            ("offset", integer()),
            ("total", integer()),
            ("has_more", boolean()),
        ], &["limit", "offset", "total"]),


        "JsonObject": json!({"type": "object", "additionalProperties": true}),


        "SettingsMap": map_of(string()),


        "SettingValue": object(&[
            ("key", string()),
            ("value", string()),
        ], &["key", "value"]),


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

fn string() -> Value {
    json!({"type": "string"})
}

fn integer() -> Value {
    json!({"type": "integer", "format": "int64"})
}

fn boolean() -> Value {
    json!({"type": "boolean"})
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
        assert_eq!(count("/api/v1/admin"), 192);
        assert_eq!(count("/api/v1/store"), 45);
        assert_eq!(count("/api/v1/webhooks"), 1);
        assert_eq!(count("/internal/storefront"), 45);
        assert_eq!(count("/health") + count("/metrics"), 4);
        assert_eq!(paths.len(), 307);

        for key in [
            "/api/v1/platform/stores",
            "/api/v1/platform/stores/{store_id}/domains",
            "/api/v1/admin/catalog/products",
            "/api/v1/admin/orders/{id}/refunds",
            "/api/v1/admin/content/pages/{id}/preview-token",
            "/api/v1/admin/inquiry-forms",
            "/api/v1/admin/inquiries",
            "/api/v1/admin/notification-channels",
            "/api/v1/admin/themes",
            "/api/v1/store/checkouts/{checkout_id}/complete",
            "/api/v1/store/inquiries",
            "/api/v1/store/inquiry-forms/{handle}",
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

        assert_eq!(operation_count, 422);
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
    fn page_response_supports_utoipa_generics() {
        let mut schemas = Vec::new();
        <crate::dto::PageResponse<crate::dto::StoreDto> as utoipa::ToSchema>::schemas(&mut schemas);
        assert!(!schemas.is_empty());
        let mut product_schemas = Vec::new();
        <crate::dto::PageResponse<fastcart_module_catalog::Product> as utoipa::ToSchema>::schemas(
            &mut product_schemas,
        );
        assert!(!product_schemas.is_empty());
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
