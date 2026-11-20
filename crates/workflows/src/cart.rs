//! Cart pricing, promotion, tax and shipping orchestration.
//!
//! The cart module stores provisional snapshots. This workflow is the only
//! place that turns catalog/pricing/promotion/tax data into those snapshots, so
//! clients can never submit authoritative prices.

use std::collections::{BTreeMap, HashMap, HashSet};

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
    ///
    /// Cart-level discounts — the applications that name no line, which is how
    /// an order-target promotion discounts the whole order — are written in a
    /// second transaction, and only when they differ from the rows already
    /// stored. A cart whose promotions produce no such application is written
    /// exactly once, as it always was.
    pub async fn recalculate(
        &self,
        cart_id: &CartId,
        context: &CartContext,
    ) -> Result<Cart, AppError> {
        let cart = self.cart.get_cart(cart_id).await?;

        // 1. Refresh unit prices from the pricing module, in memory. Quantity
        // is the only per-line input to price selection, so every line that
        // shares a quantity is answered by one lookup covering all of its
        // variants; a cart of `n` lines costs one lookup per distinct
        // quantity, not one lookup per line.
        let mut variants_by_quantity: BTreeMap<i64, Vec<String>> = BTreeMap::new();
        for item in &cart.items {
            variants_by_quantity
                .entry(item.quantity)
                .or_default()
                .push(item.variant_id.to_string());
        }
        let mut selected_amounts: HashMap<(i64, String), i64> = HashMap::new();
        for (quantity, variant_ids) in &variants_by_quantity {
            let selected = self
                .pricing
                .calculate_prices(
                    variant_ids,
                    &PriceContext {
                        currency: cart.currency_code.clone(),
                        quantity: *quantity,
                        customer_group_id: context.customer_group_ids.first().cloned(),
                        sales_channel_id: cart
                            .sales_channel_id
                            .clone()
                            .or_else(|| context.sales_channel_id.clone()),
                        at: context.at,
                    },
                )
                .await?;
            for (variant_id, price) in selected {
                if let Some(price) = price {
                    selected_amounts.insert((*quantity, variant_id), price.amount);
                }
            }
        }
        let mut prices = Vec::with_capacity(cart.items.len());
        for item in &cart.items {
            let unit_price = selected_amounts
                .get(&(item.quantity, item.variant_id.to_string()))
                .copied()
                .ok_or_else(|| {
                    AppError::conflict(format!(
                        "no price is configured for variant {} in {}",
                        item.variant_id, cart.currency_code
                    ))
                })?;
            prices.push(ItemPriceInput {
                item_id: item.id.clone(),
                unit_price,
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
        //
        // An application that names no line and is not a shipping discount is a
        // cart-level one: an order-target promotion discounts the whole order,
        // which the cart records as a `cart_discounts` row. Those rows are the
        // cart's own way of carrying a discount that belongs to no single line,
        // and `allocate_cart_discounts` spreads them over the lines again when
        // the cart is placed as an order.
        let mut line_adjustments: HashMap<&str, Vec<AdjustmentInput>> = HashMap::new();
        let mut shipping_adjustments = Vec::new();
        let mut cart_level_discounts: Vec<(String, DiscountInput)> = Vec::new();
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
                } else {
                    // An order-target promotion books its amount on two legs:
                    // what the item value absorbs and what is left over for
                    // shipping. The cart clamps a cart-level row at the item
                    // subtotal, so the shipping leg is recorded as the shipping
                    // adjustment it is — otherwise the part of the discount that
                    // outgrew the items was silently dropped from the totals.
                    let shipping_leg = result.shipping_discount.clamp(0, application.amount);
                    if shipping_leg > 0 {
                        shipping_adjustments.push(AdjustmentInput {
                            amount: shipping_leg,
                            ..adjustment.clone()
                        });
                    }
                    let item_leg = application.amount - shipping_leg;
                    if item_leg > 0 {
                        cart_level_discounts.push((
                            result.promotion_id.clone(),
                            DiscountInput {
                                promotion_id: Some(PromotionId::from_string(
                                    result.promotion_id.clone(),
                                )),
                                // The row keeps the promotion's code so the coupon
                                // flows can still tell which discount is which.
                                code: result.code.clone(),
                                amount: item_leg,
                                is_automatic: true,
                                metadata: Metadata::empty(),
                            },
                        ));
                    }
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

        // 3b. The replacement set for the cart-level discounts. Replacing them
        // is all-or-nothing, so the rows the cart must keep are carried over
        // and the applications above are merged into them.
        let mut wanted_by_promotion: HashMap<String, DiscountInput> = HashMap::new();
        for (promotion_id, discount) in cart_level_discounts {
            wanted_by_promotion
                .entry(promotion_id)
                .and_modify(|existing| {
                    existing.amount = existing.amount.saturating_add(discount.amount)
                })
                .or_insert(discount);
        }
        let mut cart_discounts: Vec<DiscountInput> = Vec::new();
        let mut row_of_promotion: HashMap<String, usize> = HashMap::new();
        for discount in &cart.discounts {
            let promotion_id = discount.promotion_id.as_ref().map(ToString::to_string);
            // A coupon that was applied earlier is a marker row — amount 0,
            // with its discount living in the line adjustments — and dropping
            // it would silently detach the coupon from the cart, so a row that
            // carries a code always survives. A row without a code is only the
            // discount some pass wrote: it is re-derived here, and dropped when
            // no promotion claims it any more.
            let claimed = promotion_id
                .as_ref()
                .is_some_and(|id| wanted_by_promotion.contains_key(id));
            if !claimed && discount.code.is_none() {
                continue;
            }
            if let Some(promotion_id) = promotion_id {
                row_of_promotion.insert(promotion_id, cart_discounts.len());
            }
            cart_discounts.push(DiscountInput {
                promotion_id: discount.promotion_id.clone(),
                code: discount.code.clone(),
                // A row tied to a promotion is derived from this pass; a row
                // without one predates it and keeps its amount.
                amount: if discount.promotion_id.is_some() {
                    0
                } else {
                    discount.amount
                },
                is_automatic: discount.is_automatic,
                metadata: discount.metadata.clone(),
            });
        }
        for (promotion_id, discount) in wanted_by_promotion {
            match row_of_promotion.get(&promotion_id) {
                // The promotion already owns a row — a coupon marker, or the
                // discount a previous pass wrote. Its amount is refreshed and
                // its own code and flags are kept.
                Some(index) => cart_discounts[*index].amount = discount.amount,
                None => {
                    row_of_promotion.insert(promotion_id, cart_discounts.len());
                    cart_discounts.push(discount);
                }
            }
        }
        // A recalculation that would write exactly what is stored does not
        // write at all: a store whose promotions discount no cart-level value
        // keeps the read-only path it always had.
        let discounts_unchanged = {
            let mut stored: Vec<_> = cart
                .discounts
                .iter()
                .map(|discount| {
                    discount_key(
                        discount.promotion_id.as_ref(),
                        discount.code.as_deref(),
                        discount.amount,
                        discount.is_automatic,
                    )
                })
                .collect();
            let mut wanted: Vec<_> = cart_discounts
                .iter()
                .map(|discount| {
                    discount_key(
                        discount.promotion_id.as_ref(),
                        discount.code.as_deref(),
                        discount.amount,
                        discount.is_automatic,
                    )
                })
                .collect();
            stored.sort();
            wanted.sort();
            stored == wanted
        };

        // 4. Calculate tax on the discounted line and shipping amounts.
        let variant_ids: Vec<String> = cart
            .items
            .iter()
            .map(|item| item.variant_id.to_string())
            .collect();
        let variants = self.catalog.get_variants(&variant_ids).await?;
        // "No region covers this destination" is a legitimate configuration;
        // "a region covers it but no rate applies to this line" is a
        // misconfiguration that would otherwise sell the line untaxed with
        // nothing in the data to show why. Resolve the governing region once so
        // the two can be told apart.
        let governing_region = match context.tax_region_id.as_deref() {
            Some(region_id) => self
                .tax
                .get_region(&TaxRegionId::from_string(region_id.to_owned()))
                .await
                .ok(),
            None => {
                self.tax
                    .region_for_destination(&context.country_code, context.province_code.as_deref())
                    .await?
            }
        };
        // A cart-level discount is money off the cart, and a line's share of it
        // comes off that line's price before it is taxed. Line adjustments have
        // always reduced the taxable amount; leaving the cart-level part out
        // charged the customer tax on money the promotion had taken off the
        // bill.
        let line_subtotals: Vec<i64> = cart
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                prices[index]
                    .unit_price
                    .max(0)
                    .saturating_mul(item.quantity.max(0))
            })
            .collect();
        let line_adjustment_totals: Vec<i64> = item_adjustments
            .iter()
            .map(|(_, adjustments)| {
                adjustments
                    .iter()
                    .map(|entry| entry.amount.max(0))
                    .fold(0_i64, i64::saturating_add)
            })
            .collect();
        let cart_level_total = cart_discounts
            .iter()
            .map(|discount| discount.amount.max(0))
            .fold(0_i64, i64::saturating_add);
        let cart_level_shares = fastcart_module_cart::allocate_cart_level_discount(
            &line_subtotals,
            &line_adjustment_totals,
            cart_level_total,
        )?;

        let mut tax_lines = Vec::new();
        for (index, item) in cart.items.iter().enumerate() {
            let line_subtotal = line_subtotals[index];
            let discount: i64 = item_adjustments[index]
                .1
                .iter()
                .map(|entry| entry.amount)
                .sum();
            let taxable = (line_subtotal - discount - cart_level_shares[index]).max(0);
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
            match (&tax.rate, &governing_region) {
                (Some(_), _) => tax_lines.push(TaxLineInput {
                    name: tax
                        .rate
                        .as_ref()
                        .map_or_else(|| "Tax".to_owned(), |rate| rate.name.clone()),
                    rate_bps: tax.rate_bps,
                    amount: tax.breakdown.tax_amount,
                    is_tax_inclusive: tax.is_inclusive,
                    cart_item_id: Some(item.id.clone()),
                    shipping_method_id: None,
                }),
                (None, Some(region)) => tax_lines.push(TaxLineInput {
                    name: no_rate_label(&region.name),
                    rate_bps: 0,
                    amount: 0,
                    is_tax_inclusive: false,
                    cart_item_id: Some(item.id.clone()),
                    shipping_method_id: None,
                }),
                (None, None) => {}
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
            match (&tax.rate, &governing_region) {
                (Some(_), _) => tax_lines.push(TaxLineInput {
                    name: tax
                        .rate
                        .as_ref()
                        .map_or_else(|| "Shipping tax".to_owned(), |rate| rate.name.clone()),
                    rate_bps: tax.rate_bps,
                    amount: tax.breakdown.tax_amount,
                    is_tax_inclusive: tax.is_inclusive,
                    cart_item_id: None,
                    shipping_method_id: Some(method.shipping_option_id.to_string()),
                }),
                (None, Some(region)) => tax_lines.push(TaxLineInput {
                    name: no_rate_label(&region.name),
                    rate_bps: 0,
                    amount: 0,
                    is_tax_inclusive: false,
                    cart_item_id: None,
                    shipping_method_id: Some(method.shipping_option_id.to_string()),
                }),
                (None, None) => {}
            }
        }

        // 5. Apply the whole snapshot in one transaction.
        let cart = self
            .cart
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
            .await?;
        if discounts_unchanged {
            return Ok(cart);
        }
        self.cart.replace_discounts(cart_id, cart_discounts).await
    }

    /// Completes a cart after an order was placed.
    pub async fn complete_cart(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        self.cart.complete_cart(cart_id).await
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

/// Names the tax line recorded when a region governs a destination but no rate
/// matched the line.
///
/// The name has to fit the cart's 160-character limit even when a merchant
/// chose a long region name, and a rejection here would fail the whole
/// recalculation over a label.
fn no_rate_label(region_name: &str) -> String {
    let label = format!("No tax rate for {region_name}");
    if label.chars().count() > 160 {
        "No matching tax rate".to_owned()
    } else {
        label
    }
}
