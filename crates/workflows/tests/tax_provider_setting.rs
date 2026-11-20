//! The store's `tax.provider_id` setting decides which provider taxes a cart.
//!
//! The provider port is only pluggable in production when the choice survives
//! the process, so the selection lives in the store's `settings` table, which
//! merchants already configure through the admin settings API. A recalculation
//! reads it once per pass: an absent key keeps the built-in region/rate
//! provider, a registered identifier serves the pass, and an identifier no
//! provider answers to is refused instead of silently reverting to the built-in
//! rates — a store that switched to an external engine must not be told the
//! engine is pricing its carts when it is not.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CountryCode, CurrencyCode, Metadata, StoreId, TaxRateId, TaxRegionId, VariantId,
};
use fastcart_module_cart::{Cart, CreateCartInput};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_store::StoreService;
use fastcart_module_tax::{
    CreateTaxRateInput, CreateTaxRegionInput, REGION_TAX_PROVIDER_ID, TaxCalculationRequest,
    TaxDestination, TaxLine, TaxProvider, TaxRate, TaxService, calculate_tax,
};
use fastcart_workflows::{CartContext, CartWorkflow};

/// Setting key the workflow reads, as a merchant would write it through the
/// admin settings API.
const SETTING_KEY: &str = "tax.provider_id";
/// Identifier of the test provider.
const STUB_PROVIDER_ID: &str = "test5";

fn country(code: &str) -> CountryCode {
    CountryCode::new(code).unwrap()
}

/// Test double standing in for an external engine: a flat 5 % everywhere.
struct StubProvider {
    destinations: Mutex<Vec<String>>,
}

impl StubProvider {
    fn new() -> Self {
        Self {
            destinations: Mutex::new(Vec::new()),
        }
    }

    fn destinations(&self) -> Vec<String> {
        self.destinations.lock().unwrap().clone()
    }
}

#[async_trait]
impl TaxProvider for StubProvider {
    fn id(&self) -> &str {
        STUB_PROVIDER_ID
    }

    async fn calculate(&self, request: TaxCalculationRequest<'_>) -> Result<TaxLine, AppError> {
        let (country_code, province_code) = match request.destination {
            TaxDestination::Address {
                country_code,
                province_code,
            } => {
                self.destinations
                    .lock()
                    .unwrap()
                    .push(format!("address {country_code} {province_code:?}"));
                (country_code.clone(), province_code.map(str::to_owned))
            }
            TaxDestination::Region { region_id } => {
                self.destinations
                    .lock()
                    .unwrap()
                    .push(format!("region {region_id}"));
                (country("US"), None)
            }
        };
        let rate = TaxRate {
            id: TaxRateId::from_string("rate_stub".to_owned()),
            tax_region_id: TaxRegionId::from_string("region_stub".to_owned()),
            tax_category_id: request.category_id.cloned(),
            name: "Stub provider rate".to_owned(),
            rate_bps: 500,
            is_inclusive: false,
            is_default: true,
            metadata: Metadata::empty(),
            created_at: 1,
            updated_at: 1,
        };
        let is_inclusive = request.inclusive_override.unwrap_or(rate.is_inclusive);
        Ok(TaxLine {
            country_code,
            province_code,
            category_id: request.category_id.cloned(),
            rate: Some(rate),
            region: None,
            amount_minor: request.amount_minor,
            rate_bps: 500,
            is_inclusive,
            breakdown: calculate_tax(request.amount_minor, 500, is_inclusive),
        })
    }
}

/// A store with one 2000-cent variant, a 20 % region rate for the destination
/// and a cart holding one unit that `add_item` already recalculated.
struct Store {
    db: StoreDatabase,
    workflow: CartWorkflow,
    context: CartContext,
    cart: Cart,
}

impl Store {
    async fn new() -> Self {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        let currency = CurrencyCode::new("USD").unwrap();

        let catalog = CatalogService::new(db.clone());
        let product = catalog
            .create_product(CreateProductInput {
                title: "Mug".to_owned(),
                variants: vec![CreateVariantInput {
                    title: "Default".to_owned(),
                    sku: Some("MUG-1".to_owned()),
                    manage_inventory: Some(false),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        catalog.publish_product(&product.id).await.unwrap();
        let variant_id = product.variants[0].id.clone();
        PricingService::new(db.clone())
            .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 2_000))
            .await
            .unwrap();

        let tax = TaxService::new(db.clone());
        let region = tax
            .create_region(CreateTaxRegionInput {
                name: "United States".to_owned(),
                country_code: country("US"),
                province_code: None,
                metadata: None,
            })
            .await
            .unwrap();
        tax.create_rate(CreateTaxRateInput {
            tax_region_id: region.id.clone(),
            tax_category_id: None,
            name: "Sales tax".to_owned(),
            rate_bps: 2_000,
            is_inclusive: false,
            is_default: true,
            metadata: None,
        })
        .await
        .unwrap();

        let context = CartContext::new(country("US"));
        let workflow = CartWorkflow::new(db.clone());
        let cart = workflow
            .create_cart(CreateCartInput {
                currency_code: currency.to_string(),
                customer_id: None,
                email: Some("buyer@example.com".to_owned()),
                sales_channel_id: None,
                region_id: None,
                metadata: Metadata::empty(),
                expires_in_seconds: None,
            })
            .await
            .unwrap();
        let cart = workflow
            .add_item(&cart.id, &VariantId::from_string(variant_id), 1, &context)
            .await
            .unwrap();

        Self {
            db,
            workflow,
            context,
            cart,
        }
    }

    /// Registers the test provider on the workflow's registry, as an
    /// application would register an external engine.
    fn register_stub(&self) -> Arc<StubProvider> {
        let stub = Arc::new(StubProvider::new());
        self.workflow
            .tax_service()
            .providers()
            .register(stub.clone())
            .unwrap();
        stub
    }

    async fn set_provider(&self, value: &str) {
        StoreService::new(self.db.clone())
            .set_setting(SETTING_KEY, value)
            .await
            .unwrap();
    }

    async fn recalculate(&self) -> Result<Cart, AppError> {
        self.workflow
            .recalculate(&self.cart.id, &self.context)
            .await
    }
}

/// The control: nothing configured still taxes at the region rate.
#[tokio::test]
async fn an_unset_provider_setting_keeps_the_built_in_region_rates() {
    let store = Store::new().await;

    assert_eq!(store.cart.totals.tax_total, 400);
    assert_eq!(store.cart.tax_lines.len(), 1);
    assert_eq!(store.cart.tax_lines[0].name, "Sales tax");
    assert_eq!(store.cart.tax_lines[0].rate_bps, 2_000);
    assert_eq!(store.cart.totals.total, 2_400);

    let cart = store.recalculate().await.unwrap();
    assert_eq!(cart.totals.tax_total, 400);
    assert_eq!(cart.totals.total, 2_400);
}

/// Naming the built-in provider is the same configuration as naming none.
#[tokio::test]
async fn setting_the_built_in_identifier_matches_the_unset_behaviour() {
    let store = Store::new().await;
    let before = store.recalculate().await.unwrap();

    store.set_provider(REGION_TAX_PROVIDER_ID).await;
    let after = store.recalculate().await.unwrap();

    assert_eq!(after.tax_lines, before.tax_lines);
    assert_eq!(after.totals, before.totals);
}

/// The core proof: the setting, not the registry's active provider, decides
/// which provider taxes the pass.
#[tokio::test]
async fn the_configured_provider_serves_the_recalculation() {
    let store = Store::new().await;
    let stub = store.register_stub();
    store.set_provider(STUB_PROVIDER_ID).await;

    let cart = store.recalculate().await.unwrap();

    assert_eq!(
        cart.totals.tax_total, 100,
        "5 % of 2000 must come from the stub, not the configured 20 %"
    );
    assert_eq!(cart.totals.total, 2_100);
    assert_eq!(cart.tax_lines.len(), 1);
    assert_eq!(cart.tax_lines[0].name, "Stub provider rate");
    assert_eq!(cart.tax_lines[0].rate_bps, 500);
    assert_eq!(cart.tax_lines[0].amount, 100);
    assert!(
        !stub.destinations().is_empty(),
        "the configured provider must have been asked for the line"
    );

    // Selecting a provider is a per-pass decision: the registry's own active
    // provider is not switched behind the scenes.
    assert_eq!(
        store
            .workflow
            .tax_service()
            .providers()
            .active_id()
            .as_deref(),
        Some(REGION_TAX_PROVIDER_ID)
    );
}

/// Registering a provider is not selecting one: until the setting names it, the
/// pass keeps the built-in rates.
#[tokio::test]
async fn a_registered_provider_is_not_used_until_the_setting_names_it() {
    let store = Store::new().await;
    let stub = store.register_stub();

    let cart = store.recalculate().await.unwrap();

    assert_eq!(cart.totals.tax_total, 400);
    assert!(
        stub.destinations().is_empty(),
        "an unselected provider must not be asked: {:?}",
        stub.destinations()
    );
}

/// An identifier no provider answers to fails the pass, names the setting and
/// the value, and leaves every cart row as it was.
#[tokio::test]
async fn an_unregistered_provider_identifier_is_refused_without_touching_the_cart() {
    let store = Store::new().await;
    let before = store.workflow.get_cart(&store.cart.id).await.unwrap();
    store.set_provider("avalara").await;

    let error = store.recalculate().await.unwrap_err();
    assert!(
        matches!(&error, AppError::Conflict(message)
            if message.contains(SETTING_KEY) && message.contains("avalara")),
        "the refusal must name the setting and the value: {error:?}"
    );

    let after = store.workflow.get_cart(&store.cart.id).await.unwrap();
    assert_eq!(
        after, before,
        "a refused recalculation must leave the stored cart untouched"
    );
    assert_eq!(
        after.totals.tax_total, 400,
        "the last good calculation is still what the cart holds"
    );
}

/// A present-but-blank value names no provider either and is refused the same
/// way; clearing the selection means deleting the key.
#[tokio::test]
async fn a_blank_provider_identifier_is_refused_like_any_unknown_one() {
    let store = Store::new().await;
    store.set_provider("").await;

    let error = store.recalculate().await.unwrap_err();
    assert!(
        matches!(&error, AppError::Conflict(message) if message.contains(SETTING_KEY)),
        "a blank value must not silently revert to the built-in rates: {error:?}"
    );
}

/// The setting is read on every pass, so switching back reaches the next
/// recalculation instead of sticking to whatever served the first one.
#[tokio::test]
async fn changing_the_identifier_back_reaches_the_next_recalculation() {
    let store = Store::new().await;
    store.register_stub();

    store.set_provider(STUB_PROVIDER_ID).await;
    let from_stub = store.recalculate().await.unwrap();
    assert_eq!(from_stub.totals.tax_total, 100);

    store.set_provider(REGION_TAX_PROVIDER_ID).await;
    let back_to_built_in = store.recalculate().await.unwrap();
    assert_eq!(back_to_built_in.totals.tax_total, 400);
    assert_eq!(back_to_built_in.tax_lines[0].rate_bps, 2_000);

    store.set_provider(STUB_PROVIDER_ID).await;
    let from_stub_again = store.recalculate().await.unwrap();
    assert_eq!(from_stub_again.totals.tax_total, 100);
}
