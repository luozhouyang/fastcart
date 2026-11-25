//! End-to-end cart, promotion, tax, inventory, payment and order flow.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId, VariantId, now_timestamp};
use fastcart_module_cart::{CartStatus, CreateCartInput};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_checkout::CheckoutStatus;
use fastcart_module_inventory::{CreateInventoryItemInput, CreateLocationInput, InventoryService};
use fastcart_module_payment::{
    ManualProvider, PaymentService, PaymentStatus as PaymentState, ProviderRegistry,
};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_promotion::{CreatePromotionInput, PromotionService, PromotionType};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_module_tax::{
    CreateTaxCategoryInput, CreateTaxRateInput, CreateTaxRegionInput, TaxService,
};
use fastcart_workflows::{CartContext, CartWorkflow, CheckoutWorkflow, CreateCheckoutRequest};

#[path = "checkout_flow/crash.rs"]
mod crash;
#[path = "checkout_flow/flow.rs"]
mod flow;
#[path = "checkout_flow/order.rs"]
mod order;
#[path = "checkout_flow/stock.rs"]
mod stock;
#[path = "checkout_flow/support.rs"]
mod support;
