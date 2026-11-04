//! Storefront product card projection.
//!
//! A card is a small, JSON-serializable view of a product. Prices are resolved
//! once per fragment through [`PricingService`] and availability once per
//! variant through [`InventoryService`]; no floating point arithmetic is used.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CurrencyCode, VariantId};
use fastcart_module_catalog::Product;
use fastcart_module_inventory::InventoryService;
use fastcart_module_pricing::{PriceContext, PricingService};
use serde::Serialize;

use crate::DataResolutionContext;

/// A monetary amount rendered for storefront consumption.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct PriceView {
    /// Amount in minor currency units.
    amount_minor: i64,
    /// Decimal string without a currency prefix or symbol.
    amount: String,
    /// ISO currency code.
    currency: String,
}

/// One purchasable variant inside a product card.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct VariantCard {
    /// Variant identifier.
    id: String,
    /// Display title.
    title: String,
    /// Stock keeping unit, when configured.
    sku: Option<String>,
    /// Selected price, or `None` when the variant has no eligible price.
    price: Option<PriceView>,
    /// Whether the variant can currently be purchased.
    available: bool,
}

/// A product presented to the storefront runtime.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ProductCard {
    /// Product identifier.
    id: String,
    /// Display title.
    title: String,
    /// URL-safe unique handle.
    handle: String,
    /// Primary thumbnail URL.
    thumbnail: Option<String>,
    /// Lowest selected price across variants.
    price: Option<PriceView>,
    /// Whether any variant can currently be purchased.
    available: bool,
    /// Variant cards.
    variants: Vec<VariantCard>,
}

/// Builds product cards for a fragment.
///
/// All variant prices are selected with a single pricing call. Availability is
/// checked per variant because inventory items are keyed by variant.
pub(crate) async fn build_product_cards(
    db: &StoreDatabase,
    products: &[Product],
    context: &DataResolutionContext,
) -> Result<Vec<ProductCard>, AppError> {
    let variant_ids: Vec<String> = products
        .iter()
        .flat_map(|product| product.variants.iter().map(|variant| variant.id.clone()))
        .collect();

    let price_context = PriceContext {
        currency: context.currency.clone(),
        quantity: 1,
        customer_group_id: context.customer_group_id.clone(),
        sales_channel_id: context.sales_channel_id.clone(),
        at: context.now,
    };
    let prices = PricingService::new(db.clone())
        .calculate_prices(&variant_ids, &price_context)
        .await?;
    let inventory = InventoryService::new(db.clone());
    let availability = inventory
        .variant_availability_many(
            &variant_ids
                .iter()
                .map(|id| VariantId::from_string(id.as_str()))
                .collect::<Vec<_>>(),
        )
        .await?;

    let mut cards = Vec::with_capacity(products.len());
    for product in products {
        let mut variants = Vec::with_capacity(product.variants.len());
        let mut available = false;
        let mut lowest_price: Option<PriceView> = None;

        for variant in &product.variants {
            let on_hand = availability.get(&variant.id).copied().unwrap_or(0);
            let variant_available =
                !variant.manage_inventory || variant.allow_backorder || on_hand > 0;
            available |= variant_available;

            let price = prices
                .get(&variant.id)
                .and_then(Option::as_ref)
                .map(|selected| price_view(selected.amount, &selected.currency));
            if let Some(price) = price.as_ref()
                && lowest_price
                    .as_ref()
                    .is_none_or(|current| price.amount_minor < current.amount_minor)
            {
                lowest_price = Some(price.clone());
            }

            variants.push(VariantCard {
                id: variant.id.clone(),
                title: variant.title.clone(),
                sku: variant.sku.clone(),
                price,
                available: variant_available,
            });
        }

        cards.push(ProductCard {
            id: product.id.clone(),
            title: product.title.clone(),
            handle: product.handle.clone(),
            thumbnail: product.thumbnail.clone(),
            price: lowest_price,
            available,
            variants,
        });
    }

    Ok(cards)
}

fn price_view(amount_minor: i64, currency: &CurrencyCode) -> PriceView {
    PriceView {
        amount_minor,
        amount: decimal_amount(amount_minor, currency),
        currency: currency.as_str().to_owned(),
    }
}

/// Formats minor units as a decimal string using integer arithmetic only.
fn decimal_amount(amount_minor: i64, currency: &CurrencyCode) -> String {
    let digits = currency.minor_unit_digits();
    if digits == 0 {
        return amount_minor.to_string();
    }
    let divisor = 10_i64.pow(digits);
    let major = amount_minor / divisor;
    let minor = (amount_minor % divisor).abs();
    if amount_minor < 0 && major == 0 {
        format!("-{major}.{minor:0width$}", width = digits as usize)
    } else {
        format!("{major}.{minor:0width$}", width = digits as usize)
    }
}
