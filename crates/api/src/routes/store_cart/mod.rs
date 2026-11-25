//! Storefront cart and checkout routes.
//!
//! Cart prices, discounts, tax and shipping are always produced by the cart
//! workflow from server-side data; request bodies never carry authoritative
//! amounts.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use fastcart_email::EmailMessage;
use fastcart_http::{ApiError, ApiJson, ApiQuery, ApiResult, ClientIp, CustomerActor, StoreCtx};
use fastcart_infrastructure::{JobQueue, StoreDatabase};
use fastcart_kernel::{
    ActorContext, AppError, CartId, CountryCode, CurrencyCode, LineItemId, Metadata,
    ShippingOptionId, VariantId, now_timestamp,
};
use fastcart_module_cart::{AddressInput, Cart, CreateCartInput, UpdateItemInput};
use fastcart_module_checkout::{CheckoutService, CheckoutStatus};
use fastcart_module_customer::CustomerService;
use fastcart_module_inventory::InventoryService;
use fastcart_module_payment::PaymentService;
use fastcart_module_store::{RegionService, StoreService};
use fastcart_workflows::{CartContext, CartWorkflow, CheckoutWorkflow, CreateCheckoutRequest};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::{StoreDb, ensure_cart_access};
use crate::dto::{
    CartResponse, CheckoutCompleteResponse, CheckoutHistoryResponse, CheckoutResponse,
};
use crate::state::AppContext;

/// Builds cart and checkout routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/carts", post(create_cart))
        .route("/carts/{cart_id}", get(get_cart))
        .route("/carts/{cart_id}/items", post(add_item).delete(clear_items))
        .route(
            "/carts/{cart_id}/items/{item_id}",
            axum::routing::patch(update_item).delete(remove_item),
        )
        .route("/carts/{cart_id}/email", post(set_email))
        .route("/carts/{cart_id}/customer", post(attach_customer))
        .route("/carts/{cart_id}/addresses", post(set_address))
        .route(
            "/carts/{cart_id}/shipping-method",
            post(set_shipping_method),
        )
        .route("/carts/{cart_id}/discounts", post(apply_discount))
        .route("/carts/{cart_id}/discounts/{code}", delete(remove_discount))
        .route("/checkouts", post(create_checkout))
        .route("/checkouts/{checkout_id}", get(get_checkout))
        .route("/checkouts/{checkout_id}/complete", post(complete_checkout))
        .route("/checkouts/{checkout_id}/cancel", post(cancel_checkout))
        .route("/checkouts/{checkout_id}/history", get(checkout_history))
}

/// Builds the documented cart and checkout routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(create_cart))
        .routes(routes!(get_cart))
        .routes(routes!(add_item, clear_items))
        .routes(routes!(update_item, remove_item))
        .routes(routes!(set_email))
        .routes(routes!(attach_customer))
        .routes(routes!(set_address))
        .routes(routes!(set_shipping_method))
        .routes(routes!(apply_discount))
        .routes(routes!(remove_discount))
        .routes(routes!(create_checkout))
        .routes(routes!(get_checkout))
        .routes(routes!(complete_checkout))
        .routes(routes!(cancel_checkout))
        .routes(routes!(checkout_history))
}

mod carts;
mod checkout;
mod customer;

use carts::*;
use checkout::*;
use customer::*;

#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateCartRequest {
    /// Cart currency.
    pub currency_code: String,
    /// Optional contact email.
    pub email: Option<String>,
    /// Optional sales channel.
    pub sales_channel_id: Option<String>,
    /// Optional store region; resolved and validated before the cart is created.
    pub region_id: Option<String>,
    /// Lifetime in seconds.
    pub expires_in_seconds: Option<i64>,
}

#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct AddItemRequest {
    /// Variant to add.
    pub variant_id: String,
    /// Quantity.
    pub quantity: i64,
    /// Tax destination country.
    pub country_code: Option<String>,
    /// Tax destination province.
    pub province_code: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateItemRequest {
    /// New quantity; zero removes the line.
    pub quantity: i64,
}

#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct SetEmailRequest {
    /// Contact email.
    pub email: String,
}

/// Address type accepted by the cart address endpoint.
#[derive(utoipa::ToSchema)]
#[schema(rename_all = "snake_case")]
pub enum CartAddressType {
    /// Shipping address.
    Shipping,
    /// Billing address.
    Billing,
}

#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
#[schema(as = SetCartAddressRequest)]
pub struct SetAddressRequest {
    /// `shipping` or `billing`.
    #[serde(rename = "type")]
    #[schema(value_type = CartAddressType)]
    pub address_type: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Company.
    pub company: Option<String>,
    /// First address line.
    pub address_1: Option<String>,
    /// Second address line.
    pub address_2: Option<String>,
    /// City.
    pub city: Option<String>,
    /// Province or state.
    pub province: Option<String>,
    /// Postal code.
    pub postal_code: Option<String>,
    /// Two-letter country code.
    pub country_code: Option<String>,
    /// Phone number.
    pub phone: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct SetShippingMethodRequest {
    /// Shipping option identifier.
    pub shipping_option_id: String,
    /// Destination country override.
    pub country_code: Option<String>,
    /// Destination province override.
    pub province_code: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct DiscountRequest {
    /// Coupon code.
    pub code: String,
}

#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
#[schema(as = CreateCheckoutRequest)]
pub struct CreateCheckoutRequestDto {
    /// Cart being checked out.
    pub cart_id: String,
    /// Optional contact email.
    pub email: Option<String>,
    /// Payment provider; defaults to `manual`.
    #[serde(default = "default_payment_provider")]
    pub payment_provider: String,
    /// Idempotency key.
    pub idempotency_key: Option<String>,
    /// Session lifetime.
    pub expires_in_seconds: Option<i64>,
    /// Tax destination country.
    pub country_code: Option<String>,
    /// Tax destination province.
    pub province_code: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CompleteCheckoutRequest {
    /// Destination country override.
    pub country_code: Option<String>,
    /// Destination province override.
    pub province_code: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DestinationQuery {
    /// Destination country.
    pub country_code: Option<String>,
    /// Destination province.
    pub province_code: Option<String>,
}

/// Enforces the store settings that the Admin exposes under Checkout.
///
/// Settings absent from the map keep their historical permissive behavior:
/// guest checkout and confirmation emails are enabled, phone numbers are
/// optional. Only the exact strings `"false"` / `"true"` opt in, matching the
/// Admin's string-valued settings store.
fn validate_checkout_settings(
    settings: &std::collections::BTreeMap<String, String>,
    customer_id: Option<&fastcart_kernel::CustomerId>,
    phone: Option<&str>,
) -> Result<(), &'static str> {
    if settings
        .get("guest_checkout")
        .is_some_and(|value| value == "false")
        && customer_id.is_none()
    {
        return Err("guest checkout is disabled; sign in before checking out");
    }
    if settings
        .get("require_phone")
        .is_some_and(|value| value == "true")
        && phone.map(str::trim).unwrap_or_default().is_empty()
    {
        return Err("a phone number is required for this store's checkout");
    }
    Ok(())
}

/// Stable rate-limit key for a public storefront write.
///
/// Direct router tests have no socket peer; they share the `unknown` bucket
/// rather than trusting request headers.
fn client_key(client_ip: Option<&Extension<ClientIp>>) -> String {
    client_ip
        .map(|Extension(ip)| ip.as_key())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn default_payment_provider() -> String {
    "manual".to_owned()
}

fn checkout_service(db: StoreDatabase) -> CheckoutService {
    CheckoutService::new(db)
}

/// Rejects a destination country the cart's region does not serve.
///
/// Carts without a region are unrestricted, as are regions without a country
/// list (which region creation never produces, but imported rows may carry).
async fn ensure_region_country(
    db: &StoreDatabase,
    cart: &Cart,
    country_code: Option<&str>,
) -> ApiResult<()> {
    let Some(region_id) = cart.region_id.as_deref() else {
        return Ok(());
    };
    let Some(country_code) = country_code else {
        return Ok(());
    };
    let country =
        CountryCode::new(country_code).map_err(|error| ApiError::new(AppError::from(error)))?;
    let region = RegionService::new(db.clone()).get_region(region_id).await?;
    if !region.countries.is_empty()
        && !region
            .countries
            .iter()
            .any(|allowed| allowed == country.as_str())
    {
        return Err(ApiError::validation(format!(
            "country {} is not served by region {}; supported countries: {}",
            country.as_str(),
            region.name,
            region.countries.join(", ")
        )));
    }
    Ok(())
}

fn parse_cart_id(value: &str) -> ApiResult<CartId> {
    if value.is_empty() || value.len() > 128 {
        return Err(ApiError::validation("invalid cart id"));
    }
    Ok(CartId::from_string(value))
}

async fn cart_context(
    db: &StoreDatabase,
    actor: &ActorContext,
    cart: &Cart,
    country_code: Option<String>,
    province_code: Option<String>,
) -> ApiResult<CartContext> {
    let country = country_code
        .or_else(|| cart.shipping_address.country_code.clone())
        .ok_or_else(|| {
            ApiError::validation(
                "a country_code is required (set a shipping address or pass country_code)",
            )
        })?;
    let country =
        CountryCode::new(country).map_err(|error| ApiError::new(AppError::from(error)))?;
    let tax_region_id = match cart.region_id.as_deref() {
        Some(region_id) => {
            RegionService::new(db.clone())
                .get_region(region_id)
                .await?
                .tax_region_id
        }
        None => None,
    };
    let mut customer_group_ids = Vec::new();
    if let Some(customer_id) = actor.customer_id.as_ref() {
        let groups = CustomerService::new(db.clone())
            .list_customer_groups(customer_id)
            .await?;
        customer_group_ids = groups.into_iter().map(|group| group.id).collect();
    }
    Ok(CartContext {
        country_code: country,
        province_code: province_code.or_else(|| cart.shipping_address.province.clone()),
        customer_group_ids,
        sales_channel_id: cart.sales_channel_id.clone(),
        tax_region_id,
        prices_include_tax: prices_include_tax(db).await,
        at: now_timestamp(),
    })
}

/// Reads the store's `prices_include_tax` setting.
///
/// Stores that quote gross prices must have tax extracted from the listed
/// amount rather than added on top. A missing or unreadable setting keeps the
/// platform default (tax added on top, as most US stores quote prices).
async fn prices_include_tax(db: &StoreDatabase) -> bool {
    // Never fail a cart request over a settings read: fall back to the default.
    match StoreService::new(db.clone()).prices_include_tax().await {
        Ok(inclusive) => inclusive,
        Err(error) => {
            tracing::warn!(error = %error, "failed to read the tax-inclusive pricing setting");
            false
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use std::collections::BTreeMap;

    use fastcart_kernel::CustomerId;

    use super::validate_checkout_settings;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn guest_checkout_setting_blocks_anonymous_carts_only() {
        let settings = map(&[("guest_checkout", "false")]);
        assert!(validate_checkout_settings(&settings, None, None).is_err());

        let customer = CustomerId::from_string("cus_test");
        assert!(validate_checkout_settings(&settings, Some(&customer), None).is_ok());

        // Absent or non-exact values keep the historical permissive behavior.
        assert!(validate_checkout_settings(&BTreeMap::new(), None, None).is_ok());
        assert!(validate_checkout_settings(&map(&[("guest_checkout", "0")]), None, None).is_ok());
    }

    #[test]
    fn require_phone_setting_rejects_missing_or_blank_phone() {
        let settings = map(&[("require_phone", "true")]);
        assert!(validate_checkout_settings(&settings, None, None).is_err());
        assert!(validate_checkout_settings(&settings, None, Some("   ")).is_err());
        assert!(validate_checkout_settings(&settings, None, Some("+1 555 0100")).is_ok());
        assert!(validate_checkout_settings(&BTreeMap::new(), None, None).is_ok());
    }
}
