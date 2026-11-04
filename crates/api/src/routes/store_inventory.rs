//! Storefront availability and shipping option discovery.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::routing::get;
use fastcart_http::{ApiError, ApiQuery, ApiResult};
use fastcart_kernel::{AppError, CountryCode, VariantId};
use fastcart_module_catalog::CatalogService;
use fastcart_module_inventory::InventoryService;
use fastcart_module_shipping::ShippingService;

use super::StoreDb;
use crate::dto::{ShippingOptionsResponse, VariantAvailabilityResponse};
use crate::state::AppContext;

/// Builds storefront availability and shipping discovery routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/products/{handle}/availability", get(product_availability))
        .route("/shipping/options", get(shipping_options))
}

#[derive(Clone, Debug, serde::Serialize, utoipa::ToSchema)]
pub struct VariantAvailability {
    /// Variant identifier.
    pub variant_id: String,
    /// Available units across locations.
    pub available: i64,
    /// Whether inventory is tracked.
    pub manage_inventory: bool,
    /// Whether backorders are allowed.
    pub allow_backorder: bool,
}

async fn product_availability(
    State(_context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    Path(handle): Path<String>,
) -> ApiResult<Json<VariantAvailabilityResponse>> {
    let product = CatalogService::new(db.clone())
        .get_published_by_handle(&handle)
        .await?;
    let inventory = InventoryService::new(db);
    let mut availability = Vec::with_capacity(product.variants.len());
    for variant in product.variants {
        if variant.deleted_at.is_some() {
            continue;
        }
        let available = inventory
            .variant_availability(&VariantId::from_string(variant.id.clone()))
            .await?;
        availability.push(VariantAvailability {
            variant_id: variant.id,
            available,
            manage_inventory: variant.manage_inventory,
            allow_backorder: variant.allow_backorder,
        });
    }
    Ok(Json(VariantAvailabilityResponse {
        variants: availability,
    }))
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct ShippingOptionsQuery {
    /// Destination country.
    pub country_code: String,
    /// Cart subtotal in minor units.
    pub subtotal: Option<i64>,
}

async fn shipping_options(
    State(_context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<ShippingOptionsQuery>,
) -> ApiResult<Json<ShippingOptionsResponse>> {
    let country = CountryCode::new(query.country_code)
        .map_err(|error| ApiError::new(AppError::from(error)))?;
    let options = ShippingService::new(db)
        .list_options_for(&country, query.subtotal.unwrap_or(0))
        .await?;
    Ok(Json(ShippingOptionsResponse {
        shipping_options: options,
    }))
}
