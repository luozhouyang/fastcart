//! Admin-authored draft orders: pricing, re-taxation and conversion.
//!
//! A draft order is authored before money or stock are involved. The workflow
//! prices its lines through the store's tax provider, so a draft totals the
//! same way a placed order would, and re-prices the goods whenever the
//! destination changes.
//!
//! Conversion is the point the draft enters the paid lifecycle: the workflow
//! prices the current lines, snapshots what that pricing described, and asks
//! the order service to apply the tax, commit the stock and place the order in
//! one transaction. The snapshot is validated again under the write lock, so a
//! concurrent edit or destination change cannot leave a line untaxed or
//! unstocked. Stock is committed rather than merely reserved because a
//! checkout-placed order in this platform commits its reservations at
//! settlement; conversion is the equivalent placement moment.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CountryCode, CustomerId, Metadata, OrderId, ProductId, TaxCategoryId, VariantId,
    validate_non_negative_amount, validate_quantity,
};
use fastcart_module_catalog::CatalogService;
use fastcart_module_inventory::OrderConversionStockLine;
use fastcart_module_order::{
    CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput, CreateOrderTaxLineInput,
    DraftConversionSnapshot, DraftOrderTaxLineInput, Order, OrderService,
};
use fastcart_module_store::StoreService;
use fastcart_module_tax::{
    TaxCalculationRequest, TaxDestination, TaxLine, TaxProvider, TaxService,
};
use serde_json::Value;

use crate::cart::{no_rate_label, resolve_tax_provider};

/// One line of a draft order before tax is resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct DraftOrderLine {
    /// Product identifier snapshot.
    pub product_id: Option<ProductId>,
    /// Variant identifier snapshot.
    pub variant_id: Option<VariantId>,
    /// Product title snapshot.
    pub product_title: String,
    /// Variant title snapshot.
    pub variant_title: Option<String>,
    /// SKU snapshot.
    pub sku: Option<String>,
    /// Thumbnail URL snapshot.
    pub thumbnail: Option<String>,
    /// Unit price in minor units.
    pub unit_price: i64,
    /// Quantity to buy.
    pub quantity: i64,
    /// Whether the unit price already contains tax; defaults to the store's
    /// `prices_include_tax` setting.
    pub is_tax_inclusive: Option<bool>,
    /// Whether the line requires shipping.
    pub requires_shipping: bool,
    /// Line metadata snapshot.
    pub metadata: Metadata,
}

/// Request to author a draft order.
#[derive(Clone, Debug, Default)]
pub struct CreateDraftOrderRequest {
    /// Optional customer the order is for.
    pub customer_id: Option<CustomerId>,
    /// Contact email.
    pub email: String,
    /// Order currency.
    pub currency_code: String,
    /// Store region the order belongs to.
    pub region_id: Option<String>,
    /// Sales channel the order belongs to.
    pub sales_channel_id: Option<String>,
    /// Lines to buy.
    pub lines: Vec<DraftOrderLine>,
    /// Shipping address snapshot.
    pub shipping_address: Value,
    /// Billing address snapshot.
    pub billing_address: Value,
    /// Shipping amount in minor units.
    pub shipping_total: i64,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Optional idempotency key for retried creations.
    pub idempotency_key: Option<String>,
}

/// Resolves the destination a draft order is taxed for.
///
/// Draft addresses are free-form JSON snapshots. The country is required to
/// resolve a tax region; an address without one is untaxed rather than an
/// error at creation (an operator may not know the destination yet), while
/// re-taxing an existing draft requires one.
fn destination(address: &Value) -> Result<Option<(CountryCode, Option<String>)>, AppError> {
    let Some(object) = address.as_object() else {
        return Ok(None);
    };
    let Some(country) = object.get("country_code").and_then(Value::as_str) else {
        return Ok(None);
    };
    let country = CountryCode::new(country).map_err(AppError::from)?;
    let province = object
        .get("province_code")
        .and_then(Value::as_str)
        .or_else(|| object.get("province").and_then(Value::as_str))
        .map(str::to_owned);
    Ok(Some((country, province)))
}

/// Prices draft lines through the store's tax provider.
pub struct DraftOrderWorkflow {
    orders: OrderService,
    catalog: CatalogService,
    tax: TaxService,
    store: StoreService,
}

impl DraftOrderWorkflow {
    /// Creates a workflow bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self {
            orders: OrderService::new(db.clone()),
            catalog: CatalogService::new(db.clone()),
            tax: TaxService::new(db.clone()),
            store: StoreService::new(db),
        }
    }

    /// Records draft mutations in the store audit outbox.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.orders = self.orders.with_audit_actor(actor_id);
        self
    }

    /// Returns the order service backing this workflow.
    #[must_use]
    pub fn order_service(&self) -> &OrderService {
        &self.orders
    }

    /// Authors a draft order, pricing its tax for the shipping address.
    ///
    /// The order is created in `draft` status: it takes no stock and no
    /// payment. Line prices are explicit, so the totals are derived from the
    /// lines and the tax provider, exactly as checkout derives them from a
    /// cart.
    pub async fn create_draft_order(
        &self,
        request: CreateDraftOrderRequest,
    ) -> Result<Order, AppError> {
        validate_non_negative_amount(request.shipping_total, "shipping_total")?;
        if request.lines.is_empty() {
            return Err(AppError::validation(
                "a draft order must contain at least one item",
            ));
        }

        let destination = destination(&request.shipping_address)?;
        let variants = self.variant_categories(&request.lines).await?;
        let provider = match destination.as_ref() {
            Some(_) => Some(resolve_tax_provider(&self.tax, &self.store).await?),
            None => None,
        };
        // A unit price is gross or net according to the store's pricing
        // convention unless the caller states otherwise, mirroring how a cart
        // line is created. The resolved flag is stored on the order line so
        // conversion and later edits price tax the same way.
        let default_is_tax_inclusive = self.store.prices_include_tax().await?;

        let mut items = Vec::with_capacity(request.lines.len());
        let mut tax_lines = Vec::new();
        let mut shipping_tax = 0_i64;
        let mut pending_items = Vec::with_capacity(request.lines.len());
        for (index, line) in request.lines.iter().enumerate() {
            validate_non_negative_amount(line.unit_price, "unit_price")?;
            validate_quantity(line.quantity, false)?;
            let is_tax_inclusive = line.is_tax_inclusive.unwrap_or(default_is_tax_inclusive);
            let subtotal = line
                .unit_price
                .checked_mul(line.quantity)
                .ok_or_else(|| AppError::validation("item subtotal overflow"))?;
            let mut exclusive_tax = 0_i64;
            let mut tax_total = 0_i64;
            if let (Some(provider), Some(destination)) = (provider.as_ref(), destination.as_ref()) {
                let tax = self
                    .line_tax(
                        provider,
                        destination,
                        line.variant_id
                            .as_ref()
                            .and_then(|variant| variants.get(variant.as_str()))
                            .and_then(|category| category.as_deref()),
                        subtotal,
                        Some(is_tax_inclusive),
                    )
                    .await?;
                if let Some((name, rate_bps, amount, is_tax_inclusive)) =
                    tax_line_values(&tax, true)
                {
                    if !is_tax_inclusive {
                        exclusive_tax = exclusive_tax
                            .checked_add(amount)
                            .ok_or_else(|| AppError::validation("tax amount overflow"))?;
                    }
                    tax_total = tax_total
                        .checked_add(amount)
                        .ok_or_else(|| AppError::validation("tax amount overflow"))?;
                    tax_lines.push(CreateOrderTaxLineInput {
                        item_index: Some(index),
                        shipping_option_id: None,
                        name,
                        rate_bps,
                        amount,
                        is_tax_inclusive,
                    });
                }
            }
            pending_items.push((exclusive_tax, tax_total));
            items.push(CreateOrderItemInput {
                product_id: line.product_id.clone(),
                variant_id: line.variant_id.clone(),
                product_title: line.product_title.clone(),
                variant_title: line.variant_title.clone(),
                sku: line.sku.clone(),
                thumbnail: line.thumbnail.clone(),
                unit_price: line.unit_price,
                quantity: line.quantity,
                subtotal: Some(subtotal),
                discount_total: Some(0),
                tax_total: Some(tax_total),
                total: None,
                is_tax_inclusive,
                requires_shipping: line.requires_shipping,
                metadata: line.metadata.clone(),
                adjustments: Vec::new(),
            });
        }

        // Fill in each line total from its net price plus the exclusive share
        // of its tax; inclusive tax is already contained in the price.
        for (index, (exclusive_tax, _)) in pending_items.iter().enumerate() {
            let subtotal = items[index].subtotal.unwrap_or_default();
            items[index].total = Some(
                subtotal
                    .checked_add(*exclusive_tax)
                    .ok_or_else(|| AppError::validation("line total overflow"))?,
            );
        }

        let mut adjustments = Vec::new();
        if request.shipping_total > 0 {
            if let (Some(provider), Some(destination)) = (provider.as_ref(), destination.as_ref()) {
                let tax = self
                    .line_tax(
                        provider,
                        destination,
                        None,
                        request.shipping_total,
                        Some(default_is_tax_inclusive),
                    )
                    .await?;
                if let Some((name, rate_bps, amount, is_tax_inclusive)) =
                    tax_line_values(&tax, false)
                {
                    tax_lines.push(CreateOrderTaxLineInput {
                        item_index: None,
                        shipping_option_id: None,
                        name,
                        rate_bps,
                        amount,
                        is_tax_inclusive,
                    });
                    if !is_tax_inclusive {
                        shipping_tax = amount;
                    }
                }
            }
            if shipping_tax > 0 {
                adjustments.push(CreateAdjustmentInput {
                    promotion_id: None,
                    code: Some("shipping_tax".to_owned()),
                    amount: shipping_tax,
                    description: Some("Shipping tax".to_owned()),
                });
            }
        }

        let subtotal = checked_sum(items.iter().filter_map(|item| item.subtotal))?;
        let discount_total = checked_sum(items.iter().filter_map(|item| item.discount_total))?;
        let tax_total = checked_sum(tax_lines.iter().map(|line| line.amount))?;
        let item_total = checked_sum(items.iter().filter_map(|item| item.total))?;
        let total = item_total
            .checked_add(request.shipping_total)
            .and_then(|value| value.checked_add(shipping_tax))
            .ok_or_else(|| AppError::validation("order total overflow"))?;
        if total < 0 {
            return Err(AppError::validation("order total must not be negative"));
        }

        self.orders
            .create_draft_order(CreateOrderInput {
                customer_id: request.customer_id,
                email: request.email,
                currency_code: request.currency_code,
                items,
                adjustments,
                shipping_address: request.shipping_address,
                billing_address: request.billing_address,
                shipping_methods: Vec::new(),
                tax_lines,
                region_id: request.region_id,
                sales_channel_id: request.sales_channel_id,
                subtotal,
                discount_total,
                shipping_total: request.shipping_total,
                tax_total,
                total,
                metadata: request.metadata,
                idempotency_key: request.idempotency_key,
            })
            .await
    }

    /// Re-prices the goods of a draft order for a new destination.
    ///
    /// Only item-scoped tax is recalculated; shipping tax and order-level
    /// adjustments were quoted with the shipping snapshot and stay untouched,
    /// mirroring the allocation rules of an order edit.
    pub async fn update_draft_destination(
        &self,
        order_id: &OrderId,
        shipping_address: Value,
        billing_address: Option<Value>,
    ) -> Result<Order, AppError> {
        let order = self.orders.get(order_id).await?;
        if !order.is_draft_order() {
            return Err(AppError::conflict(
                "only draft orders can have their tax recalculated",
            ));
        }
        let Some((country, province)) = destination(&shipping_address)? else {
            return Err(AppError::validation(
                "a shipping_address with a country_code is required to recalculate tax",
            ));
        };
        let tax_lines = self.item_tax_lines(&order, &(country, province)).await?;

        self.orders
            .recalculate_draft_taxes(order_id, Some(shipping_address), billing_address, tax_lines)
            .await
    }

    /// Converts a draft order into a pending order, taking its stock.
    ///
    /// The goods are re-priced once more for the draft's current destination,
    /// so lines added or changed through order edits enter the lifecycle with
    /// the tax they should carry. The lines and destination that pricing
    /// looked at are snapshotted and validated again by the order service under
    /// the write lock; applying the tax, committing the stock and placing the
    /// order then commit together. A concurrent edit or destination change
    /// makes the conversion conflict instead of placing a line the inputs did
    /// not describe.
    ///
    /// Stock is committed here rather than reserved until fulfillment because
    /// a checkout-placed order commits its reservations at settlement in this
    /// platform; conversion is its placement moment. No payment is taken.
    pub async fn convert_draft_order(&self, order_id: &OrderId) -> Result<Order, AppError> {
        let order = self.orders.get(order_id).await?;
        if !order.is_draft_order() {
            return Err(AppError::conflict(
                "only draft orders can be converted, found a placed order",
            ));
        }
        let snapshot = DraftConversionSnapshot::from_order(&order);
        let tax_lines = match destination(&order.shipping_address)? {
            Some(destination) => Some(self.item_tax_lines(&order, &destination).await?),
            None => None,
        };
        let stock_lines = self.conversion_stock_lines(&order).await?;
        self.orders
            .convert_priced_draft_order(order_id, &snapshot, tax_lines, stock_lines)
            .await
    }

    /// Builds the stock conversion takes for a draft's active lines.
    ///
    /// Only variants that manage inventory take stock; the flag comes from the
    /// catalog, exactly like checkout. Lines whose variant no longer exists are
    /// skipped: there is no policy left to apply, and the order line keeps its
    /// own snapshot of the goods. A managed variant without an inventory item
    /// is still reported, so the inventory service can skip it or record a
    /// backorder exactly as a checkout reservation would.
    async fn conversion_stock_lines(
        &self,
        order: &Order,
    ) -> Result<Vec<OrderConversionStockLine>, AppError> {
        let variant_ids: Vec<String> = order
            .items
            .iter()
            .filter(|item| item.quantity > 0)
            .filter_map(|item| item.variant_id.as_ref().map(ToString::to_string))
            .collect();
        if variant_ids.is_empty() {
            return Ok(Vec::new());
        }
        let variants = self.catalog.get_variants(&variant_ids).await?;
        let mut lines = Vec::new();
        for item in order.items.iter().filter(|item| item.quantity > 0) {
            let Some(variant_id) = item.variant_id.clone() else {
                continue;
            };
            let Some(variant) = variants.get(variant_id.as_str()) else {
                continue;
            };
            if !variant.manage_inventory {
                continue;
            }
            lines.push(OrderConversionStockLine::new(
                variant_id,
                item.id.clone(),
                item.quantity,
                variant.allow_backorder,
            ));
        }
        Ok(lines)
    }

    /// Prices every active item line of a draft for one destination.
    async fn item_tax_lines(
        &self,
        order: &Order,
        destination: &(CountryCode, Option<String>),
    ) -> Result<Vec<DraftOrderTaxLineInput>, AppError> {
        let provider = resolve_tax_provider(&self.tax, &self.store).await?;
        let variant_ids: Vec<String> = order
            .items
            .iter()
            .filter(|item| item.quantity > 0)
            .filter_map(|item| item.variant_id.as_ref().map(ToString::to_string))
            .collect();
        let variants = self.catalog.get_variants(&variant_ids).await?;

        let mut tax_lines = Vec::new();
        for item in order.items.iter().filter(|item| item.quantity > 0) {
            let taxable = item.subtotal.saturating_sub(item.discount_total).max(0);
            let tax = self
                .line_tax(
                    &provider,
                    destination,
                    item.variant_id
                        .as_ref()
                        .and_then(|variant| variants.get(variant.as_str()))
                        .and_then(|variant| variant.tax_category_id.as_deref()),
                    taxable,
                    Some(item.is_tax_inclusive),
                )
                .await?;
            if let Some((name, rate_bps, amount, is_tax_inclusive)) = tax_line_values(&tax, true) {
                tax_lines.push(DraftOrderTaxLineInput {
                    order_item_id: item.id.clone(),
                    name,
                    rate_bps,
                    amount,
                    is_tax_inclusive,
                });
            }
        }
        Ok(tax_lines)
    }

    async fn variant_categories(
        &self,
        lines: &[DraftOrderLine],
    ) -> Result<std::collections::HashMap<String, Option<String>>, AppError> {
        let ids: Vec<String> = lines
            .iter()
            .filter_map(|line| line.variant_id.as_ref().map(ToString::to_string))
            .collect();
        let variants = self.catalog.get_variants(&ids).await?;
        Ok(variants
            .into_iter()
            .map(|(id, variant)| (id, variant.tax_category_id))
            .collect())
    }

    async fn line_tax(
        &self,
        provider: &std::sync::Arc<dyn TaxProvider>,
        destination: &(CountryCode, Option<String>),
        category_id: Option<&str>,
        amount_minor: i64,
        inclusive_override: Option<bool>,
    ) -> Result<TaxLine, AppError> {
        let category = category_id.map(TaxCategoryId::from_string);
        provider
            .calculate(TaxCalculationRequest {
                destination: TaxDestination::Address {
                    country_code: &destination.0,
                    province_code: destination.1.as_deref(),
                },
                category_id: category.as_ref(),
                amount_minor,
                inclusive_override,
            })
            .await
    }
}

/// Turns a provider line into the values an order tax line stores.
///
/// A matched rate carries its name and rate. An unmatched line under a
/// governing region records a zero line so the breakdown says why nothing was
/// collected, exactly like cart recalculation.
fn tax_line_values(tax: &TaxLine, record_no_rate: bool) -> Option<(String, i64, i64, bool)> {
    if let Some(rate) = tax.rate.as_ref() {
        return Some((
            rate.name.clone(),
            tax.rate_bps,
            tax.breakdown.tax_amount,
            tax.is_inclusive,
        ));
    }
    if record_no_rate {
        let region = tax.region.as_ref()?;
        return Some((no_rate_label(&region.name), 0, 0, false));
    }
    None
}

fn checked_sum(values: impl Iterator<Item = i64>) -> Result<i64, AppError> {
    let mut total = 0_i64;
    for value in values {
        total = total
            .checked_add(value)
            .ok_or_else(|| AppError::validation("amount overflow"))?;
    }
    Ok(total)
}
