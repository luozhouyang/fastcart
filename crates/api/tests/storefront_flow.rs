//! End-to-end storefront HTTP flow: catalog, cart, checkout and order.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_inventory::{CreateInventoryItemInput, CreateLocationInput, InventoryService};
use fastcart_module_pricing::{CreatePriceInput, CreatePriceListInput, PricingService};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_module_store::{
    CreatePublishableApiKeyInput, CreateRegionInput, CreateSalesChannelInput, RegionService,
    StoreService,
};
use fastcart_module_tax::{CreateTaxRateInput, CreateTaxRegionInput, TaxService};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[path = "storefront_flow/admin.rs"]
mod admin;
#[path = "storefront_flow/cart.rs"]
mod cart;
#[path = "storefront_flow/catalog.rs"]
mod catalog;
#[path = "storefront_flow/checkout.rs"]
mod checkout;
#[path = "storefront_flow/support.rs"]
mod support;
