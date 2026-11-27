//! Cart domain types, service inputs and the pure commercial calculator.
//!
//! Commercial values are integer minor units paired with a [`CurrencyCode`].
//! The calculator never trusts a previously stored total: every call derives a
//! fresh [`CartTotals`] from the line snapshots and adjustment rows.

use fastcart_kernel::{
    AppError, CartId, CountryCode, CurrencyCode, CustomerId, LineItemId, Metadata, ProductId,
    PromotionId, ShippingOptionId, VariantId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default cart lifetime in seconds (30 days).
pub const DEFAULT_CART_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;

/// Largest accepted quantity for a single cart line.
pub const MAX_ITEM_QUANTITY: i64 = 1_000_000;

/// Lifecycle state of a cart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CartStatus {
    /// The cart accepts mutations.
    Open,
    /// The cart was converted into an order.
    Completed,
    /// The cart passed its expiration timestamp.
    Expired,
}

impl CartStatus {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Completed => "completed",
            Self::Expired => "expired",
        }
    }

    /// Parses a stored value. Unknown values are treated as open so a future
    /// state does not make existing carts unreadable.
    #[must_use]
    pub fn from_db(value: &str) -> Self {
        match value {
            "completed" => Self::Completed,
            "expired" => Self::Expired,
            _ => Self::Open,
        }
    }

    /// Returns true when the cart accepts mutations.
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open)
    }
}

/// A postal address snapshot attached to a cart.
///
/// The billing and shipping roles are tracked by the owning cart fields rather
/// than by this value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct Address {
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Company or organization.
    pub company: Option<String>,
    /// First address line.
    pub address_1: Option<String>,
    /// Second address line.
    pub address_2: Option<String>,
    /// City or locality.
    pub city: Option<String>,
    /// State, province or region.
    pub province: Option<String>,
    /// Postal code.
    pub postal_code: Option<String>,
    /// Uppercase two-letter country code.
    pub country_code: Option<String>,
    /// Contact phone number.
    pub phone: Option<String>,
    /// Extension metadata.
    pub metadata: Metadata,
}

/// Input for setting a cart address.
#[derive(Clone, Debug, Default)]
pub struct AddressInput {
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Company or organization.
    pub company: Option<String>,
    /// First address line.
    pub address_1: Option<String>,
    /// Second address line.
    pub address_2: Option<String>,
    /// City or locality.
    pub city: Option<String>,
    /// State, province or region.
    pub province: Option<String>,
    /// Postal code.
    pub postal_code: Option<String>,
    /// Two-letter country code.
    pub country_code: Option<String>,
    /// Contact phone number.
    pub phone: Option<String>,
    /// Extension metadata.
    pub metadata: Metadata,
}

impl AddressInput {
    /// Validates and materializes the address snapshot.
    pub fn try_into_address(self) -> Result<Address, AppError> {
        let country_code = match self.country_code {
            Some(code) => {
                let normalized = CountryCode::new(code)?;
                Some(normalized.as_str().to_owned())
            }
            None => None,
        };
        Ok(Address {
            first_name: self.first_name,
            last_name: self.last_name,
            company: self.company,
            address_1: self.address_1,
            address_2: self.address_2,
            city: self.city,
            province: self.province,
            postal_code: self.postal_code,
            country_code,
            phone: self.phone,
            metadata: self.metadata,
        })
    }
}

/// A priced line item snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CartItem {
    /// Line item identifier.
    pub id: LineItemId,
    /// Owning cart.
    pub cart_id: CartId,
    /// Product identifier.
    pub product_id: ProductId,
    /// Variant identifier.
    pub variant_id: VariantId,
    /// Quantity of units.
    pub quantity: i64,
    /// Product title snapshot.
    pub title: String,
    /// Variant title snapshot.
    pub variant_title: Option<String>,
    /// Stock keeping unit snapshot.
    pub sku: Option<String>,
    /// Thumbnail URL snapshot.
    pub thumbnail: Option<String>,
    /// Unit price in minor units.
    pub unit_price: i64,
    /// Currency of the price snapshot.
    pub currency_code: CurrencyCode,
    /// Whether the unit price already contains tax.
    pub is_tax_inclusive: bool,
    /// Whether the line needs a shipping method.
    pub requires_shipping: bool,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Line-level adjustments applied to this item.
    pub adjustments: Vec<CartAdjustment>,
}

/// A gift card applied to the cart.
///
/// Only the masked code is stored here: the full code is a bearer credential
/// that stays in the payment module, which owns the card and its live balance.
/// The balance is a snapshot refreshed when the cart is priced, so a card spent
/// somewhere else cannot be sold twice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CartGiftCard {
    /// Gift card identifier.
    pub gift_card_id: String,
    /// Redemption code with every character but the last four masked.
    pub masked_code: String,
    /// Amount the customer asked the card to cover, in minor units.
    pub amount: i64,
    /// Card balance when the cart was last priced, in minor units.
    pub balance: i64,
    /// Amount this card actually contributes to the payable total, in minor
    /// units: the smaller of [`Self::amount`] and [`Self::balance`], capped by
    /// what the cards applied before it leave to pay.
    pub applied_amount: i64,
}

/// Input for [`crate::CartService::apply_gift_card`].
///
/// The workflow validates the card itself through the payment module, which
/// owns it; the cart only checks what its own rows can prove (currency and that
/// the card is not already applied) and stores the snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GiftCardApplication {
    /// Gift card identifier.
    pub gift_card_id: String,
    /// Redemption code masked except for its last four characters.
    pub masked_code: String,
    /// Currency the card was issued in.
    pub currency_code: String,
    /// Amount to apply; `None` applies the whole remaining balance.
    ///
    /// A requested amount larger than the balance records the balance instead
    /// of failing: the card covers what it has and the cart total follows.
    pub amount_minor: Option<i64>,
    /// Card balance the caller read immediately before applying, in minor
    /// units. A negative balance is treated as zero.
    pub balance_minor: i64,
}

/// A cart-level promotion or coupon snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CartDiscount {
    /// Promotion that produced the discount, when known.
    pub promotion_id: Option<PromotionId>,
    /// Coupon or promotion code.
    pub code: Option<String>,
    /// Discount amount in minor units.
    pub amount: i64,
    /// Whether the discount was applied automatically.
    pub is_automatic: bool,
    /// Extension metadata.
    pub metadata: Metadata,
}

/// A selected shipping method snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CartShippingMethod {
    /// Shipping option identifier.
    pub shipping_option_id: ShippingOptionId,
    /// Human-readable option name.
    pub name: String,
    /// Shipping amount in minor units.
    pub amount: i64,
    /// Currency of the shipping amount.
    pub currency: CurrencyCode,
    /// Whether the amount already contains tax.
    pub is_tax_inclusive: bool,
    /// Provider-specific shipping data.
    pub data: Value,
}

/// A line or shipping adjustment snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CartAdjustment {
    /// Promotion that produced the adjustment, when known.
    pub promotion_id: Option<PromotionId>,
    /// Promotion code, when known.
    pub code: Option<String>,
    /// Adjustment amount in minor units.
    pub amount: i64,
    /// Human-readable description.
    pub description: Option<String>,
}

/// A tax line computed by the tax workflow and stored on the cart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CartTaxLine {
    /// Tax rate name, for example `US sales tax`.
    pub name: String,
    /// Tax rate in basis points (`1000` is `10%`).
    pub rate_bps: i64,
    /// Tax amount in minor units.
    pub amount: i64,
    /// Whether the amount is already contained in the taxed prices.
    pub is_tax_inclusive: bool,
    /// Item this tax line applies to, when line-scoped.
    pub cart_item_id: Option<LineItemId>,
    /// Shipping method this tax line applies to, when shipping-scoped.
    pub shipping_method_id: Option<String>,
}

/// Commercial summary derived from the stored cart rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CartTotals {
    /// Sum of `unit_price * quantity` before discounts.
    pub item_subtotal: i64,
    /// Sum of line adjustment amounts, capped per line.
    pub item_discount_total: i64,
    /// Sum of shipping method amounts before shipping discounts.
    pub shipping_subtotal: i64,
    /// Sum of shipping adjustments, capped at the shipping subtotal.
    pub shipping_discount_total: i64,
    /// Item subtotal before discounts.
    pub subtotal: i64,
    /// Item adjustments plus cart-level discounts, capped at the subtotal.
    pub discount_total: i64,
    /// Shipping subtotal after shipping adjustments; never negative.
    pub shipping_total: i64,
    /// Sum of every stored tax line amount (inclusive and exclusive).
    pub tax_total: i64,
    /// Amount covered by applied gift cards, capped at the total they pay for.
    pub gift_card_total: i64,
    /// Payable amount: subtotal - discounts + shipping + exclusive tax -
    /// gift cards, never negative.
    pub total: i64,
    /// Currency of every amount in the summary.
    pub currency_code: CurrencyCode,
}

impl CartTotals {
    /// Creates a zeroed summary in the given currency.
    #[must_use]
    pub fn zero(currency_code: CurrencyCode) -> Self {
        Self {
            item_subtotal: 0,
            item_discount_total: 0,
            shipping_subtotal: 0,
            shipping_discount_total: 0,
            subtotal: 0,
            discount_total: 0,
            shipping_total: 0,
            tax_total: 0,
            gift_card_total: 0,
            total: 0,
            currency_code,
        }
    }
}

/// The cart aggregate returned by every service read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Cart {
    /// Cart identifier.
    pub id: CartId,
    /// Cart currency; every line must match it.
    pub currency_code: CurrencyCode,
    /// Authenticated customer, when attached.
    pub customer_id: Option<CustomerId>,
    /// Contact email.
    pub email: Option<String>,
    /// Sales channel identifier.
    pub sales_channel_id: Option<String>,
    /// Store region this cart is priced for, when a region applies.
    pub region_id: Option<String>,
    /// Lifecycle state.
    pub status: CartStatus,
    /// Shipping address snapshot.
    pub shipping_address: Address,
    /// Billing address snapshot.
    pub billing_address: Address,
    /// Line items.
    pub items: Vec<CartItem>,
    /// Cart-level discounts.
    pub discounts: Vec<CartDiscount>,
    /// Gift cards applied as a payment method.
    pub gift_cards: Vec<CartGiftCard>,
    /// Selected shipping methods (at most one).
    pub shipping_methods: Vec<CartShippingMethod>,
    /// Flattened line adjustment snapshots, mirroring each item's adjustments.
    pub item_adjustments: Vec<CartAdjustment>,
    /// Flattened shipping adjustment snapshots.
    pub shipping_adjustments: Vec<CartAdjustment>,
    /// Stored tax lines.
    pub tax_lines: Vec<CartTaxLine>,
    /// Recalculated commercial summary.
    pub totals: CartTotals,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
    /// Completion timestamp in Unix seconds.
    pub completed_at: Option<i64>,
    /// Expiration timestamp in Unix seconds.
    pub expires_at: Option<i64>,
}

/// Input for creating a cart.
#[derive(Clone, Debug)]
pub struct CreateCartInput {
    /// Cart currency code.
    pub currency_code: String,
    /// Optional authenticated customer.
    pub customer_id: Option<CustomerId>,
    /// Optional contact email.
    pub email: Option<String>,
    /// Optional sales channel identifier.
    pub sales_channel_id: Option<String>,
    /// Optional store region identifier resolved by the caller.
    ///
    /// The cart module never reads the store module's `regions` table; callers
    /// resolve a region and pass its identifier here.
    pub region_id: Option<String>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Lifetime in seconds; defaults to 30 days.
    pub expires_in_seconds: Option<i64>,
}

impl CreateCartInput {
    /// Creates input for an anonymous cart in the given currency.
    #[must_use]
    pub fn new(currency_code: impl Into<String>) -> Self {
        Self {
            currency_code: currency_code.into(),
            customer_id: None,
            email: None,
            sales_channel_id: None,
            region_id: None,
            metadata: Metadata::empty(),
            expires_in_seconds: None,
        }
    }
}

/// Input for adding a line item snapshot.
#[derive(Clone, Debug)]
pub struct AddItemInput {
    /// Product identifier.
    pub product_id: ProductId,
    /// Variant identifier.
    pub variant_id: VariantId,
    /// Product title snapshot.
    pub title: String,
    /// Variant title snapshot.
    pub variant_title: Option<String>,
    /// Stock keeping unit snapshot.
    pub sku: Option<String>,
    /// Thumbnail URL snapshot.
    pub thumbnail: Option<String>,
    /// Unit price in minor units.
    pub unit_price: i64,
    /// Currency of the price snapshot.
    pub currency_code: String,
    /// Quantity to add.
    pub quantity: i64,
    /// Whether the unit price already contains tax.
    pub is_tax_inclusive: bool,
    /// Whether the line needs shipping.
    pub requires_shipping: bool,
    /// Extension metadata.
    pub metadata: Metadata,
}

/// Input for changing a line item quantity.
#[derive(Clone, Copy, Debug)]
pub struct UpdateItemInput {
    /// New quantity. Zero removes the line.
    pub quantity: i64,
}

/// Input for selecting the cart's shipping method.
#[derive(Clone, Debug)]
pub struct SetShippingMethodInput {
    /// Shipping option identifier.
    pub shipping_option_id: ShippingOptionId,
    /// Human-readable option name.
    pub name: String,
    /// Shipping amount in minor units.
    pub amount: i64,
    /// Currency of the shipping amount.
    pub currency_code: String,
    /// Whether the amount already contains tax.
    pub is_tax_inclusive: bool,
    /// Provider-specific shipping data.
    pub data: Value,
}

/// Input for adding or replacing a cart discount.
#[derive(Clone, Debug)]
pub struct DiscountInput {
    /// Promotion that produced the discount, when known.
    pub promotion_id: Option<PromotionId>,
    /// Coupon or promotion code.
    pub code: Option<String>,
    /// Discount amount in minor units.
    pub amount: i64,
    /// Whether the discount was applied automatically.
    pub is_automatic: bool,
    /// Extension metadata.
    pub metadata: Metadata,
}

/// Input for a line or shipping adjustment.
#[derive(Clone, Debug)]
pub struct AdjustmentInput {
    /// Promotion that produced the adjustment, when known.
    pub promotion_id: Option<PromotionId>,
    /// Promotion code, when known.
    pub code: Option<String>,
    /// Adjustment amount in minor units.
    pub amount: i64,
    /// Human-readable description.
    pub description: Option<String>,
}

/// Input for a tax line.
#[derive(Clone, Debug)]
pub struct TaxLineInput {
    /// Tax rate name.
    pub name: String,
    /// Tax rate in basis points.
    pub rate_bps: i64,
    /// Tax amount in minor units.
    pub amount: i64,
    /// Whether the amount is already contained in the taxed prices.
    pub is_tax_inclusive: bool,
    /// Item this tax line applies to, when line-scoped.
    pub cart_item_id: Option<LineItemId>,
    /// Shipping method this tax line applies to, when shipping-scoped.
    pub shipping_method_id: Option<String>,
}

/// Calculates the commercial summary from stored rows.
///
/// Rules:
///
/// - Line discounts are capped at that line's subtotal.
/// - Cart-level discounts are added to line discounts and the combined amount
///   is capped at the item subtotal.
/// - Shipping is `shipping_subtotal - shipping adjustments`, never negative.
/// - `tax_total` includes every tax line and is informational.
/// - `total` only adds exclusive tax lines: inclusive tax is already contained
///   in the item and shipping prices and must not be counted twice.
/// - Gift cards pay the total down: each applied card contributes the smaller
///   of the amount the customer stored and the card's current balance, and
///   never more than what the cards applied before it leave to pay. The excess
///   is not spent, so the customer keeps it on the card.
/// - `total` is `subtotal - discounts + shipping + exclusive tax - gift cards`,
///   never negative: a card that would overpay is only charged the remainder.
///
/// All arithmetic saturates instead of overflowing, so a pathological snapshot
/// cannot panic a request.
#[must_use]
pub fn calculate_totals(cart: &Cart, tax_lines: &[CartTaxLine]) -> CartTotals {
    let mut item_subtotal: i64 = 0;
    let mut item_discount_total: i64 = 0;

    for item in &cart.items {
        let line_subtotal = item.unit_price.max(0).saturating_mul(item.quantity.max(0));
        item_subtotal = item_subtotal.saturating_add(line_subtotal);

        let line_adjustments = item
            .adjustments
            .iter()
            .map(|adjustment| adjustment.amount.max(0))
            .fold(0_i64, i64::saturating_add);
        item_discount_total =
            item_discount_total.saturating_add(line_adjustments.min(line_subtotal));
    }

    let shipping_subtotal = cart
        .shipping_methods
        .iter()
        .map(|method| method.amount.max(0))
        .fold(0_i64, i64::saturating_add);

    let shipping_adjustments = cart
        .shipping_adjustments
        .iter()
        .map(|adjustment| adjustment.amount.max(0))
        .fold(0_i64, i64::saturating_add);
    let shipping_discount_total = shipping_adjustments.min(shipping_subtotal);
    let shipping_total = shipping_subtotal.saturating_sub(shipping_discount_total);

    let cart_discounts = cart
        .discounts
        .iter()
        .map(|discount| discount.amount.max(0))
        .fold(0_i64, i64::saturating_add);
    let discount_total = item_discount_total
        .saturating_add(cart_discounts)
        .min(item_subtotal);

    let tax_total = tax_lines
        .iter()
        .map(|line| line.amount.max(0))
        .fold(0_i64, i64::saturating_add);
    let exclusive_tax = tax_lines
        .iter()
        .filter(|line| !line.is_tax_inclusive)
        .map(|line| line.amount.max(0))
        .fold(0_i64, i64::saturating_add);

    let payable = item_subtotal
        .saturating_sub(discount_total)
        .saturating_add(shipping_total)
        .saturating_add(exclusive_tax);

    // Cards pay in the order they were applied and never more than the money
    // still owed, so over-applying one card does not silently overspend it: the
    // unused part stays on the card.
    let applied = applied_gift_card_amounts(&cart.gift_cards, payable);
    let gift_card_total = applied.iter().copied().fold(0_i64, i64::saturating_add);
    let total = payable.saturating_sub(gift_card_total).max(0);

    CartTotals {
        item_subtotal,
        item_discount_total,
        shipping_subtotal,
        shipping_discount_total,
        subtotal: item_subtotal,
        discount_total,
        shipping_total,
        tax_total,
        gift_card_total,
        total,
        currency_code: cart.currency_code.clone(),
    }
}

/// Splits the payable amount across the applied gift cards, in application
/// order.
///
/// Each card is charged the smaller of its stored amount and its current
/// balance, and never more than the money the earlier cards leave to pay. The
/// last element of the result is what the card actually spends; the remainder
/// stays on the card for the next purchase.
fn applied_gift_card_amounts(gift_cards: &[CartGiftCard], payable: i64) -> Vec<i64> {
    let mut remaining = payable.max(0);
    gift_cards
        .iter()
        .map(|card| {
            let applied = card.amount.max(0).min(card.balance.max(0)).min(remaining);
            remaining = remaining.saturating_sub(applied);
            applied
        })
        .collect()
}

/// Recomputes the cart's stored totals in place.
///
/// The per-card [`CartGiftCard::applied_amount`] is refreshed from the same
/// split the totals use, so a consumer that reads the cards instead of the
/// summary sees the same figures.
pub fn recalculate_totals(cart: &mut Cart) {
    cart.totals = calculate_totals(cart, &cart.tax_lines);
    // `total` is the payable amount after cards, so adding the charged amount
    // back returns the money the cards paid against. Both are non-negative and
    // their sum cannot overflow: `total` is capped by the payable amount.
    let payable = cart
        .totals
        .total
        .saturating_add(cart.totals.gift_card_total);
    let applied = applied_gift_card_amounts(&cart.gift_cards, payable);
    for (card, amount) in cart.gift_cards.iter_mut().zip(applied) {
        card.applied_amount = amount;
    }
}

/// Spreads the cart's cart-level discounts across its line items.
///
/// [`CartTotals::discount_total`] folds per-line adjustments and cart-level
/// discounts into one figure, but line items only carry their own adjustments.
/// A consumer that rebuilds the aggregate from the items alone — order
/// placement does — would therefore drop the cart-level part and record a
/// total nobody was ever charged, while refunds are capped by that total.
///
/// Each line's share is proportional to the value it still has left after its
/// own adjustments, and the shares sum to exactly the cart-level portion of
/// [`CartTotals::discount_total`]. See [`allocate_cart_level_discount`] for the
/// split itself.
///
/// # Errors
///
/// Returns a validation error only if the shares cannot be distributed, which
/// the clamping in [`allocate_cart_level_discount`] makes unreachable.
pub fn allocate_cart_discounts(cart: &Cart) -> Result<Vec<i64>, AppError> {
    let line_subtotals: Vec<i64> = cart
        .items
        .iter()
        .map(|item| item.unit_price.max(0).saturating_mul(item.quantity.max(0)))
        .collect();
    let line_adjustments: Vec<i64> = cart
        .items
        .iter()
        .map(|item| {
            item.adjustments
                .iter()
                .map(|adjustment| adjustment.amount.max(0))
                .fold(0_i64, i64::saturating_add)
        })
        .collect();
    let requested = cart
        .discounts
        .iter()
        .map(|discount| discount.amount.max(0))
        .fold(0_i64, i64::saturating_add);
    allocate_cart_level_discount(&line_subtotals, &line_adjustments, requested)
}

/// Spreads a cart-level discount over the lines that can still absorb it.
///
/// Each line's share is proportional to the value it has left after its own
/// adjustments, and the shares sum to exactly the amount the lines can absorb —
/// never more than `requested`.
///
/// A cart-level discount is money off the cart rather than off a line, but the
/// line is where the money is: order placement rebuilds each line from its own
/// adjustments, and tax is charged per line on what it actually sells for. Both
/// need the same split, so both ask here.
///
/// # Errors
///
/// Returns a validation error only if the shares cannot be distributed, which
/// the clamping above makes unreachable.
pub fn allocate_cart_level_discount(
    line_subtotals: &[i64],
    line_adjustments: &[i64],
    requested: i64,
) -> Result<Vec<i64>, AppError> {
    // The running sum is kept exactly representable: `allocate_proportionally`
    // requires its parts to sum to the total it is given, and a saturated total
    // does not satisfy that (two lines near `i64::MAX` overflow it again inside
    // the allocator). At these magnitudes the cart's own totals are saturated
    // anyway, so a line that cannot fit loses the excess rather than letting the
    // parts disagree with the total — which would over-allocate discounts.
    let mut headroom: Vec<i64> = Vec::with_capacity(line_subtotals.len());
    let mut total = 0_i64;
    for (index, line_subtotal) in line_subtotals.iter().enumerate() {
        let line_subtotal = (*line_subtotal).max(0);
        let adjustments = line_adjustments
            .get(index)
            .copied()
            .unwrap_or_default()
            .max(0);
        let room = line_subtotal
            .saturating_sub(adjustments.min(line_subtotal))
            .min(i64::MAX - total);
        total += room;
        headroom.push(room);
    }
    let amount = requested.max(0).min(total);
    if amount <= 0 {
        return Ok(vec![0; line_subtotals.len()]);
    }
    fastcart_kernel::allocate_proportionally(&headroom, total, amount).map_err(AppError::from)
}

/// One line's authoritative price refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemPriceInput {
    /// Line item being repriced.
    pub item_id: LineItemId,
    /// Authoritative unit price in minor units.
    pub unit_price: i64,
}

/// Every derived snapshot a recalculation replaces.
///
/// Prices, discounts and tax come from other modules. Writing them one
/// statement at a time left the cart half-updated whenever a later step failed
/// — most visibly with every discount already cleared while the caller saw an
/// error. [`CartService::apply_recalculation`] writes the whole snapshot in one
/// transaction instead.
#[derive(Clone, Debug, Default)]
pub struct RecalculationInput {
    /// Currency every price is expressed in; must match the cart.
    pub currency_code: String,
    /// Authoritative unit prices, keyed by line item.
    pub prices: Vec<ItemPriceInput>,
    /// Replacement adjustments for each line item named here; lines omitted
    /// from this list keep their existing adjustments.
    pub item_adjustments: Vec<(LineItemId, Vec<AdjustmentInput>)>,
    /// Replacement shipping adjustments.
    pub shipping_adjustments: Vec<AdjustmentInput>,
    /// Replacement tax lines; replaces every tax line on the cart.
    pub tax_lines: Vec<TaxLineInput>,
    /// When set, every line and the shipping method are stamped with this
    /// tax-inclusivity flag, so the stored snapshot follows the store's
    /// pricing policy instead of whatever was recorded when the line was added.
    pub tax_inclusive: Option<bool>,
}
