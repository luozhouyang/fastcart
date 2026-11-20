//! Cart pricing, promotion, tax and shipping orchestration.
//!
//! The cart module stores provisional snapshots. This workflow is the only
//! place that turns catalog/pricing/promotion/tax data into those snapshots, so
//! clients can never submit authoritative prices.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::ShippingOptionId;
use fastcart_kernel::{
    AppError, CartId, CountryCode, LineItemId, Metadata, PromotionId, TaxCategoryId, Timestamp,
    VariantId, now_timestamp,
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
use fastcart_module_tax::TaxService;

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
#[derive(Clone, Debug)]
pub struct CartWorkflow {
    cart: CartService,
    catalog: CatalogService,
    pricing: PricingService,
    promotion: PromotionService,
    tax: TaxService,
    shipping: ShippingService,
}

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
            shipping: ShippingService::new(db),
        }
    }

    /// Returns the underlying cart service.
    #[must_use]
    pub fn cart_service(&self) -> &CartService {
        &self.cart
    }

    /// Creates an empty cart.
    pub async fn create_cart(&self, input: CreateCartInput) -> Result<Cart, AppError> {
        self.cart.create_cart(input).await
    }

    /// Reads a cart with its stored totals.
    pub async fn get_cart(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        self.cart.get_cart(cart_id).await
    }

    /// Adds a variant at the authoritative price and recalculates the cart.
    pub async fn add_item(
        &self,
        cart_id: &CartId,
        variant_id: &VariantId,
        quantity: i64,
        context: &CartContext,
    ) -> Result<Cart, AppError> {
        let cart = self.cart.get_cart(cart_id).await?;
        let variant = self.catalog.get_variant(variant_id.as_str()).await?;
        let product = self.catalog.get_product(&variant.product_id).await?;
        let price = self
            .pricing
            .calculate_price(
                variant_id.as_str(),
                &PriceContext {
                    currency: cart.currency_code.clone(),
                    quantity,
                    customer_group_id: context.customer_group_ids.first().cloned(),
                    sales_channel_id: cart
                        .sales_channel_id
                        .clone()
                        .or_else(|| context.sales_channel_id.clone()),
                    at: context.at,
                },
            )
            .await?
            .ok_or_else(|| {
                AppError::conflict(format!(
                    "no price is configured for variant {} in {}",
                    variant_id, cart.currency_code
                ))
            })?;

        self.cart
            .add_item(
                cart_id,
                AddItemInput {
                    product_id: fastcart_kernel::ProductId::from_string(product.id.clone()),
                    variant_id: variant_id.clone(),
                    title: product.title.clone(),
                    variant_title: Some(variant.title.clone()),
                    sku: variant.sku.clone(),
                    thumbnail: product.thumbnail.clone(),
                    unit_price: price.amount,
                    currency_code: price.currency.to_string(),
                    quantity,
                    is_tax_inclusive: context.prices_include_tax,
                    requires_shipping: true,
                    metadata: Metadata::empty(),
                },
            )
            .await?;
        self.recalculate(cart_id, context).await
    }

    /// Selects a shipping option at the resolved rate and recalculates.
    pub async fn set_shipping_method(
        &self,
        cart_id: &CartId,
        option_id: &ShippingOptionId,
        context: &CartContext,
    ) -> Result<Cart, AppError> {
        let cart = self.cart.get_cart(cart_id).await?;
        let option = self.shipping.get_option(option_id).await?;
        let rate = self
            .shipping
            .select_rate(
                option_id,
                &context.country_code,
                context.province_code.as_deref(),
                cart.totals.item_subtotal,
            )
            .await?
            .ok_or_else(|| {
                AppError::conflict(format!(
                    "shipping option {} is not available for this destination",
                    option.id
                ))
            })?;
        if rate.currency != cart.currency_code {
            return Err(AppError::conflict(
                "shipping option currency does not match the cart currency",
            ));
        }
        self.cart
            .set_shipping_method(
                cart_id,
                SetShippingMethodInput {
                    shipping_option_id: option.id.clone(),
                    name: option.name.clone(),
                    amount: rate.amount_minor,
                    currency_code: rate.currency.to_string(),
                    is_tax_inclusive: context.prices_include_tax,
                    data: serde_json::json!({
                        "rate_id": rate.rate_id,
                        "provider_id": option.provider_id,
                        "provider_data": option.provider_data,
                    }),
                },
            )
            .await?;
        self.recalculate(cart_id, context).await
    }

    /// Applies a coupon code and recalculates discounts.
    pub async fn apply_coupon(
        &self,
        cart_id: &CartId,
        code: &str,
        context: &CartContext,
    ) -> Result<Cart, AppError> {
        let cart = self.cart.get_cart(cart_id).await?;
        let coupon = self.promotion.validate_coupon(code, context.at).await?;
        if cart
            .discounts
            .iter()
            .any(|discount| discount.code.as_deref() == Some(coupon.code.as_str()))
        {
            return Err(AppError::conflict("coupon is already applied"));
        }
        self.promotion.get_promotion(&coupon.promotion_id).await?;
        self.cart
            .add_discount(
                cart_id,
                DiscountInput {
                    promotion_id: Some(PromotionId::from_string(coupon.promotion_id.clone())),
                    code: Some(coupon.code.clone()),
                    amount: 0,
                    is_automatic: false,
                    metadata: Metadata::empty(),
                },
            )
            .await?;
        self.recalculate(cart_id, context).await
    }

    /// Removes a coupon code and recalculates discounts.
    pub async fn remove_coupon(
        &self,
        cart_id: &CartId,
        code: &str,
        context: &CartContext,
    ) -> Result<Cart, AppError> {
        let cart = self.cart.get_cart(cart_id).await?;
        let normalized = code.trim().to_ascii_uppercase();
        let discount = cart
            .discounts
            .iter()
            .find(|discount| discount.code.as_deref() == Some(normalized.as_str()))
            .ok_or_else(|| AppError::not_found("coupon", &normalized))?;
        let promotion_id = discount
            .promotion_id
            .clone()
            .ok_or_else(|| AppError::not_found("coupon promotion", &normalized))?;
        self.cart.remove_discount(cart_id, &promotion_id).await?;
        self.recalculate(cart_id, context).await
    }

    /// Recomputes prices, promotion adjustments and tax lines for a cart.
    ///
    /// Stored totals are never trusted: every read after this call is derived
    /// from the rows written here.
    ///
    /// The whole snapshot is derived in memory and then applied in a single
    /// transaction. Deriving first also means a pricing, promotion or tax
    /// failure leaves the cart exactly as it was, instead of a cart whose
    /// discounts were already cleared when the failing step ran.
    pub async fn recalculate(
        &self,
        cart_id: &CartId,
        context: &CartContext,
    ) -> Result<Cart, AppError> {
        let cart = self.cart.get_cart(cart_id).await?;

        // 1. Refresh unit prices from the pricing module, in memory.
        let mut prices = Vec::with_capacity(cart.items.len());
        for item in &cart.items {
            let price = self
                .pricing
                .calculate_price(
                    item.variant_id.as_str(),
                    &PriceContext {
                        currency: cart.currency_code.clone(),
                        quantity: item.quantity,
                        customer_group_id: context.customer_group_ids.first().cloned(),
                        sales_channel_id: cart
                            .sales_channel_id
                            .clone()
                            .or_else(|| context.sales_channel_id.clone()),
                        at: context.at,
                    },
                )
                .await?
                .ok_or_else(|| {
                    AppError::conflict(format!(
                        "no price is configured for variant {} in {}",
                        item.variant_id, cart.currency_code
                    ))
                })?;
            prices.push(ItemPriceInput {
                item_id: item.id.clone(),
                unit_price: price.amount,
            });
        }

        // 2. Build the promotion snapshot from the refreshed prices. Both
        // totals are the undiscounted ones: this pass replaces the discount
        // snapshot, so the promotions being evaluated must see the cart's full
        // value rather than what the previous pass discounted it to.
        let mut category_cache: HashMap<String, Vec<String>> = HashMap::new();
        let mut lines = Vec::with_capacity(cart.items.len());
        for (index, item) in cart.items.iter().enumerate() {
            let product_id = item.product_id.to_string();
            let categories = if let Some(categories) = category_cache.get(&product_id) {
                categories.clone()
            } else {
                let categories = match self.catalog.get_product(&product_id).await {
                    Ok(product) => product.categories.iter().map(|c| c.id.clone()).collect(),
                    Err(_) => Vec::new(),
                };
                category_cache.insert(product_id.clone(), categories.clone());
                categories
            };
            lines.push(PromotionLine {
                line_id: item.id.to_string(),
                product_id,
                variant_id: item.variant_id.to_string(),
                category_ids: categories,
                unit_price: prices[index].unit_price,
                quantity: item.quantity,
                is_shipping: false,
            });
        }
        let item_subtotal = cart
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                prices[index]
                    .unit_price
                    .max(0)
                    .saturating_mul(item.quantity.max(0))
            })
            .fold(0_i64, i64::saturating_add);
        let shipping_subtotal = cart
            .shipping_methods
            .iter()
            .map(|method| method.amount.max(0))
            .fold(0_i64, i64::saturating_add);
        let promotion_cart = PromotionCart {
            currency: cart.currency_code.clone(),
            subtotal: item_subtotal,
            shipping_total: shipping_subtotal,
            customer_id: cart.customer_id.as_ref().map(ToString::to_string),
            customer_group_ids: context.customer_group_ids.clone(),
            lines,
            at: context.at,
        };

        let mut contexts = Vec::new();
        let mut seen = HashSet::new();
        for promotion in self.promotion.list_automatic_promotions(context.at).await? {
            if seen.insert(promotion.id.clone()) {
                contexts.push(PromotionContext {
                    rules: promotion.rules.clone(),
                    promotion,
                });
            }
        }
        for discount in &cart.discounts {
            let Some(code) = discount.code.as_deref() else {
                continue;
            };
            let Ok(coupon) = self.promotion.validate_coupon(code, context.at).await else {
                continue;
            };
            let promotion = self.promotion.get_promotion(&coupon.promotion_id).await?;
            if promotion.status != PromotionStatus::Active {
                continue;
            }
            if seen.insert(promotion.id.clone()) {
                contexts.push(PromotionContext {
                    rules: promotion.rules.clone(),
                    promotion,
                });
            }
        }
        let results = self.promotion.calculate(&contexts, &promotion_cart)?;

        // 3. Group the applications per line, keeping cart order so the write
        // below is deterministic.
        let mut line_adjustments: HashMap<&str, Vec<AdjustmentInput>> = HashMap::new();
        let mut shipping_adjustments = Vec::new();
        for result in &results {
            for application in &result.applications {
                let adjustment = AdjustmentInput {
                    promotion_id: Some(PromotionId::from_string(result.promotion_id.clone())),
                    code: result.code.clone(),
                    amount: application.amount,
                    description: Some(application.description.clone()),
                };
                if application.is_shipping {
                    shipping_adjustments.push(adjustment);
                } else if let Some(line_id) = application.line_id.as_deref() {
                    line_adjustments
                        .entry(line_id)
                        .or_default()
                        .push(adjustment);
                }
            }
        }
        let item_adjustments: Vec<(LineItemId, Vec<AdjustmentInput>)> = cart
            .items
            .iter()
            .map(|item| {
                (
                    item.id.clone(),
                    line_adjustments
                        .remove(item.id.as_str())
                        .unwrap_or_default(),
                )
            })
            .collect();
        let shipping_discount = shipping_adjustments
            .iter()
            .map(|adjustment| adjustment.amount.max(0))
            .fold(0_i64, i64::saturating_add)
            .min(shipping_subtotal);

        // 4. Calculate tax on the discounted line and shipping amounts.
        let variant_ids: Vec<String> = cart
            .items
            .iter()
            .map(|item| item.variant_id.to_string())
            .collect();
        let variants = self.catalog.get_variants(&variant_ids).await?;
        let mut tax_lines = Vec::new();
        for (index, item) in cart.items.iter().enumerate() {
            let line_subtotal = prices[index]
                .unit_price
                .max(0)
                .saturating_mul(item.quantity.max(0));
            let discount: i64 = item_adjustments[index]
                .1
                .iter()
                .map(|entry| entry.amount)
                .sum();
            let taxable = (line_subtotal - discount).max(0);
            let category = variants
                .get(item.variant_id.as_str())
                .and_then(|variant| variant.tax_category_id.clone())
                .map(TaxCategoryId::from_string);
            let tax = match context.tax_region_id.as_deref() {
                Some(region_id) => {
                    self.tax
                        .calculate_for_line_in_region(
                            region_id,
                            category.as_ref(),
                            taxable,
                            Some(context.prices_include_tax),
                        )
                        .await?
                }
                None => {
                    self.tax
                        .calculate_for_line(
                            &context.country_code,
                            context.province_code.as_deref(),
                            category.as_ref(),
                            taxable,
                            Some(context.prices_include_tax),
                        )
                        .await?
                }
            };
            if tax.rate.is_some() {
                tax_lines.push(TaxLineInput {
                    name: tax
                        .rate
                        .as_ref()
                        .map_or_else(|| "Tax".to_owned(), |rate| rate.name.clone()),
                    rate_bps: tax.rate_bps,
                    amount: tax.breakdown.tax_amount,
                    is_tax_inclusive: tax.is_inclusive,
                    cart_item_id: Some(item.id.clone()),
                    shipping_method_id: None,
                });
            }
        }
        if let Some(method) = cart.shipping_methods.first() {
            let taxable = (method.amount - shipping_discount).max(0);
            let tax = match context.tax_region_id.as_deref() {
                Some(region_id) => {
                    self.tax
                        .calculate_for_line_in_region(
                            region_id,
                            None,
                            taxable,
                            Some(context.prices_include_tax),
                        )
                        .await?
                }
                None => {
                    self.tax
                        .calculate_for_line(
                            &context.country_code,
                            context.province_code.as_deref(),
                            None,
                            taxable,
                            Some(context.prices_include_tax),
                        )
                        .await?
                }
            };
            if tax.rate.is_some() {
                tax_lines.push(TaxLineInput {
                    name: tax
                        .rate
                        .as_ref()
                        .map_or_else(|| "Shipping tax".to_owned(), |rate| rate.name.clone()),
                    rate_bps: tax.rate_bps,
                    amount: tax.breakdown.tax_amount,
                    is_tax_inclusive: tax.is_inclusive,
                    cart_item_id: None,
                    shipping_method_id: Some(method.shipping_option_id.to_string()),
                });
            }
        }

        // 5. Apply the whole snapshot in one transaction.
        self.cart
            .apply_recalculation(
                cart_id,
                RecalculationInput {
                    currency_code: cart.currency_code.to_string(),
                    prices,
                    item_adjustments,
                    shipping_adjustments,
                    tax_lines,
                    tax_inclusive: Some(context.prices_include_tax),
                },
            )
            .await
    }

    /// Completes a cart after an order was placed.
    pub async fn complete_cart(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        self.cart.complete_cart(cart_id).await
    }
}
