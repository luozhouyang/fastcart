//! Cart pricing, promotion, tax and shipping orchestration.
//!
//! The cart module stores provisional snapshots. This workflow is the only
//! place that turns catalog/pricing/promotion/tax data into those snapshots, so
//! clients can never submit authoritative prices.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::ShippingOptionId;
use fastcart_kernel::{
    AppError, CartId, CountryCode, LineItemId, Metadata, PromotionId, TaxCategoryId, TaxRegionId,
    Timestamp, VariantId, now_timestamp,
};
use fastcart_module_cart::{
    AddItemInput, AdjustmentInput, Cart, CartService, CreateCartInput, DiscountInput,
    ItemPriceInput, RecalculationInput, SetShippingMethodInput, TaxLineInput,
};
use fastcart_module_catalog::CatalogService;
use fastcart_module_pricing::{PriceContext, PricingService};
use fastcart_module_promotion::{
    PromotionCart, PromotionContext, PromotionLine, PromotionService, PromotionStatus,
};
use fastcart_module_shipping::ShippingService;
use fastcart_module_store::StoreService;
use fastcart_module_tax::{TaxCalculationRequest, TaxDestination, TaxProvider, TaxService};

/// Store setting that selects the tax provider a recalculation runs through.
///
/// The value is the identifier of a provider registered in the workflow's
/// registry. The key is absent by default, which keeps the built-in region/rate
/// provider; merchants write it through the admin settings API.
const TAX_PROVIDER_SETTING_KEY: &str = "tax.provider_id";

/// Inputs that affect price and tax selection for a cart.
#[derive(Clone, Debug)]
pub struct CartContext {
    /// Destination country for tax and shipping.
    pub country_code: CountryCode,
    /// Destination province/state, when known.
    pub province_code: Option<String>,
    /// Customer groups used for group-scoped prices and promotions.
    pub customer_group_ids: Vec<String>,
    /// Sales channel used for channel-scoped prices.
    pub sales_channel_id: Option<String>,
    /// Tax region pinned by the cart's store region, when configured.
    pub tax_region_id: Option<String>,
    /// Whether the store's listed prices already include tax.
    ///
    /// Stores that quote gross prices (most of the EU, for example) must have
    /// the tax extracted from the listed amount rather than added on top of it.
    pub prices_include_tax: bool,
    /// Evaluation timestamp.
    pub at: Timestamp,
}

impl CartContext {
    /// Creates a context for a destination at the current time.
    #[must_use]
    pub fn new(country_code: CountryCode) -> Self {
        Self {
            country_code,
            province_code: None,
            customer_group_ids: Vec::new(),
            sales_channel_id: None,
            tax_region_id: None,
            prices_include_tax: false,
            at: now_timestamp(),
        }
    }

    /// Sets the destination province.
    #[must_use]
    pub fn with_province(mut self, province: impl Into<String>) -> Self {
        self.province_code = Some(province.into());
        self
    }

    /// Sets the customer groups.
    #[must_use]
    pub fn with_customer_groups(mut self, groups: Vec<String>) -> Self {
        self.customer_group_ids = groups;
        self
    }

    /// Pins tax calculation to a store-configured tax region.
    #[must_use]
    pub fn with_tax_region(mut self, region_id: impl Into<String>) -> Self {
        self.tax_region_id = Some(region_id.into());
        self
    }

    /// Declares whether the store's listed prices already include tax.
    #[must_use]
    pub fn with_inclusive_pricing(mut self, inclusive: bool) -> Self {
        self.prices_include_tax = inclusive;
        self
    }
}

/// Orchestrates the cart module with catalog, pricing, promotion, tax and
/// shipping services for one store.
///
/// Tax calculations run through the provider the store selected with the
/// `tax.provider_id` setting; see [`Self::tax_service`] for registering
/// providers in the first place.
#[derive(Clone, Debug)]
pub struct CartWorkflow {
    cart: CartService,
    catalog: CatalogService,
    pricing: PricingService,
    promotion: PromotionService,
    tax: TaxService,
    store: StoreService,
    shipping: ShippingService,
}

mod items;
mod lifecycle;
mod recalculation;
mod tax;

pub(crate) use tax::resolve_tax_provider;

impl CartWorkflow {
    /// Creates a workflow bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self {
            cart: CartService::new(db.clone()),
            catalog: CatalogService::new(db.clone()),
            pricing: PricingService::new(db.clone()),
            promotion: PromotionService::new(db.clone()),
            tax: TaxService::new(db.clone()),
            store: StoreService::new(db.clone()),
            shipping: ShippingService::new(db),
        }
    }
}

impl CartWorkflow {
    /// Returns the underlying cart service.
    #[must_use]
    pub fn cart_service(&self) -> &CartService {
        &self.cart
    }
}

impl CartWorkflow {
    /// Returns the tax service whose provider registry serves calculations.
    ///
    /// Registering a provider here — for example an external engine — makes it
    /// available to recalculation, which selects it once the store's
    /// `tax.provider_id` setting names it. Registering alone changes nothing:
    /// the built-in region/rate provider stays in charge until the setting
    /// selects another.
    #[must_use]
    pub fn tax_service(&self) -> &TaxService {
        &self.tax
    }
}

/// The fields on which a stored cart discount and a wanted one must agree.
///
/// The two are different types (a stored row and a write input), and stored rows
/// come back ordered by their row id rather than by the order they were written
/// in, so the two are compared as a multiset of these keys.
fn discount_key(
    promotion_id: Option<&PromotionId>,
    code: Option<&str>,
    amount: i64,
    is_automatic: bool,
) -> (Option<String>, Option<String>, i64, bool) {
    (
        promotion_id.map(ToString::to_string),
        code.map(str::to_owned),
        amount,
        is_automatic,
    )
}

/// The destination a provider is asked to price a line for.
///
/// A tax region pinned by the cart's store region wins, mirroring
/// `TaxService::calculate_for_line_in_region`; otherwise the provider resolves
/// the region that governs the destination address itself, mirroring
/// `TaxService::calculate_for_line`.
fn tax_destination(context: &CartContext) -> TaxDestination<'_> {
    match context.tax_region_id.as_deref() {
        Some(region_id) => TaxDestination::Region { region_id },
        None => TaxDestination::Address {
            country_code: &context.country_code,
            province_code: context.province_code.as_deref(),
        },
    }
}

/// Names the tax line recorded when a region governs a destination but no rate
/// matched the line.
///
/// The name has to fit the cart's 160-character limit even when a merchant
/// chose a long region name, and a rejection here would fail the whole
/// recalculation over a label.
pub(crate) fn no_rate_label(region_name: &str) -> String {
    let label = format!("No tax rate for {region_name}");
    if label.chars().count() > 160 {
        "No matching tax rate".to_owned()
    } else {
        label
    }
}
