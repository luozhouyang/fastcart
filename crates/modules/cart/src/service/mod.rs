//! SQLite-backed cart application service.
//!
//! Every mutation first checks that the cart is open, writes inside a short
//! transaction and returns a freshly loaded aggregate. Totals are always
//! derived from stored rows by [`calculate_totals`]; no total is ever accepted
//! from an input payload.

use std::collections::BTreeMap;

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CartId, CurrencyCode, CustomerId, LineItemId, Metadata, PageResult, Pagination,
    ProductId, PromotionId, ShippingOptionId, VariantId, normalize_email, now_timestamp,
    validate_email, validate_length, validate_metadata, validate_non_negative_amount,
    validate_quantity,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::{FromRow, Sqlite, Transaction};

use crate::domain::{
    AddItemInput, Address, AddressInput, AdjustmentInput, Cart, CartAdjustment, CartDiscount,
    CartGiftCard, CartItem, CartShippingMethod, CartStatus, CartTaxLine, CartTotals,
    CreateCartInput, DEFAULT_CART_TTL_SECONDS, DiscountInput, GiftCardApplication,
    RecalculationInput, SetShippingMethodInput, TaxLineInput, UpdateItemInput, recalculate_totals,
};

const CART_COLUMNS: &str = "SELECT id, currency_code, customer_id, email, sales_channel_id, \
     region_id, status, shipping_address, billing_address, metadata, created_at, updated_at, \
     completed_at, expires_at FROM carts WHERE id = ?";

const ITEM_COLUMNS: &str = "SELECT id, cart_id, product_id, variant_id, quantity, title, \
     variant_title, sku, thumbnail, unit_price, currency_code, is_tax_inclusive, \
     requires_shipping, metadata FROM cart_items WHERE cart_id = ? ORDER BY created_at ASC, id ASC";

const ITEM_ADJUSTMENT_COLUMNS: &str = "SELECT a.cart_item_id, a.promotion_id, a.code, a.amount, \
     a.description FROM cart_item_adjustments a JOIN cart_items i ON i.id = a.cart_item_id \
     WHERE i.cart_id = ? ORDER BY a.created_at ASC, a.id ASC";

const DISCOUNT_COLUMNS: &str = "SELECT promotion_id, code, amount, is_automatic, metadata \
     FROM cart_discounts WHERE cart_id = ? ORDER BY created_at ASC, id ASC";

const GIFT_CARD_COLUMNS: &str = "SELECT gift_card_id, masked_code, amount_minor, balance_minor \
     FROM cart_gift_cards WHERE cart_id = ? ORDER BY created_at ASC, id ASC";

const SHIPPING_METHOD_COLUMNS: &str = "SELECT shipping_option_id, name, amount, \
     currency_code, is_tax_inclusive, data FROM cart_shipping_methods WHERE cart_id = ? \
     ORDER BY created_at ASC, id ASC";

const SHIPPING_ADJUSTMENT_COLUMNS: &str = "SELECT a.shipping_method_id, a.promotion_id, a.code, \
     a.amount, a.description FROM cart_shipping_method_adjustments a \
     JOIN cart_shipping_methods m ON m.id = a.shipping_method_id WHERE m.cart_id = ? \
     ORDER BY a.created_at ASC, a.id ASC";

const TAX_LINE_COLUMNS: &str = "SELECT name, rate_bps, amount, is_tax_inclusive, cart_item_id, \
     shipping_method_id FROM cart_tax_lines WHERE cart_id = ? ORDER BY created_at ASC, id ASC";

const ADDRESS_COLUMNS: &str = "SELECT type, first_name, last_name, company, address_1, \
     address_2, city, province, postal_code, country_code, phone, metadata \
     FROM cart_addresses WHERE cart_id = ?";

/// SQLite-backed service owning carts and their commercial snapshots.
#[derive(Clone, Debug)]
pub struct CartService {
    db: StoreDatabase,
}

mod discounts;
mod gift_cards;
mod items;
mod lifecycle;
mod parties;
mod reads;
mod recalculation;
mod shipping;

impl CartService {
    /// Creates a service bound to an already-migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }
}

#[derive(Default)]
struct Addresses {
    shipping: Option<Address>,
    billing: Option<Address>,
}

#[derive(Debug, FromRow)]
struct CartRow {
    id: String,
    currency_code: String,
    customer_id: Option<String>,
    email: Option<String>,
    sales_channel_id: Option<String>,
    region_id: Option<String>,
    status: String,
    shipping_address: String,
    billing_address: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
    completed_at: Option<i64>,
    expires_at: Option<i64>,
}

#[derive(Debug, FromRow)]
struct CartItemRow {
    id: String,
    cart_id: String,
    product_id: String,
    variant_id: String,
    quantity: i64,
    title: String,
    variant_title: Option<String>,
    sku: Option<String>,
    thumbnail: Option<String>,
    unit_price: i64,
    currency_code: String,
    is_tax_inclusive: bool,
    requires_shipping: bool,
    metadata: String,
}

#[derive(Debug, FromRow)]
struct ItemAdjustmentRow {
    cart_item_id: String,
    promotion_id: Option<String>,
    code: Option<String>,
    amount: i64,
    description: Option<String>,
}

#[derive(Debug, FromRow)]
struct DiscountRow {
    promotion_id: Option<String>,
    code: Option<String>,
    amount: i64,
    is_automatic: bool,
    metadata: String,
}

#[derive(Debug, FromRow)]
struct GiftCardRow {
    gift_card_id: String,
    masked_code: String,
    amount_minor: i64,
    balance_minor: i64,
}

#[derive(Debug, FromRow)]
struct ShippingMethodRow {
    shipping_option_id: String,
    name: String,
    amount: i64,
    currency_code: String,
    is_tax_inclusive: bool,
    data: String,
}

#[derive(Debug, FromRow)]
struct ShippingAdjustmentRow {
    promotion_id: Option<String>,
    code: Option<String>,
    amount: i64,
    description: Option<String>,
}

#[derive(Debug, FromRow)]
struct TaxLineRow {
    name: String,
    rate_bps: i64,
    amount: i64,
    is_tax_inclusive: bool,
    cart_item_id: Option<String>,
    shipping_method_id: Option<String>,
}

#[derive(Debug, FromRow)]
struct AddressRow {
    #[sqlx(rename = "type")]
    address_type: String,
    first_name: Option<String>,
    last_name: Option<String>,
    company: Option<String>,
    address_1: Option<String>,
    address_2: Option<String>,
    city: Option<String>,
    province: Option<String>,
    postal_code: Option<String>,
    country_code: Option<String>,
    phone: Option<String>,
    metadata: String,
}

async fn ensure_item_belongs_to_cart(
    tx: &mut Transaction<'_, Sqlite>,
    cart_id: &CartId,
    item_id: &LineItemId,
) -> Result<(), AppError> {
    let found: Option<String> =
        sqlx::query_scalar("SELECT id FROM cart_items WHERE id = ? AND cart_id = ?")
            .bind(item_id.as_str())
            .bind(cart_id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?;
    if found.is_none() {
        return Err(AppError::not_found("cart item", item_id));
    }
    Ok(())
}

async fn insert_adjustment(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
    parent_column: &str,
    parent_id: &str,
    input: &AdjustmentInput,
    created_at: i64,
) -> Result<(), AppError> {
    let sql = format!(
        "INSERT INTO {table} (id, {parent_column}, promotion_id, code, amount, description, \
         created_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
    );
    sqlx::query(&sql)
        .bind(row_id("adj"))
        .bind(parent_id)
        .bind(input.promotion_id.as_ref().map(PromotionId::as_str))
        .bind(input.code.as_deref())
        .bind(input.amount)
        .bind(input.description.as_deref())
        .bind(created_at)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    Ok(())
}

fn validate_discount_input(input: &DiscountInput) -> Result<(), AppError> {
    validate_non_negative_amount(input.amount, "discount amount")?;
    if let Some(code) = &input.code {
        validate_length(code, "code", 120)?;
    }
    validate_metadata(&input.metadata)
}

fn validate_adjustment_input(input: &AdjustmentInput) -> Result<(), AppError> {
    validate_non_negative_amount(input.amount, "adjustment amount")?;
    if let Some(code) = &input.code {
        validate_length(code, "code", 120)?;
    }
    if let Some(description) = &input.description {
        validate_length(description, "description", 500)?;
    }
    Ok(())
}

fn validate_tax_line_input(input: &TaxLineInput) -> Result<(), AppError> {
    validate_length(&input.name, "name", 160)?;
    if input.rate_bps < 0 {
        return Err(AppError::validation("rate_bps must not be negative"));
    }
    validate_non_negative_amount(input.amount, "tax amount")
}

async fn upsert_discount(
    tx: &mut Transaction<'_, Sqlite>,
    cart_id: &CartId,
    input: &DiscountInput,
    created_at: i64,
) -> Result<(), AppError> {
    // The partial unique indexes on (cart_id, promotion_id) and (cart_id, code)
    // make retried or concurrent applications idempotent: a matching row is
    // refreshed instead of duplicated.
    sqlx::query(
        "INSERT INTO cart_discounts (id, cart_id, promotion_id, code, amount, is_automatic, \
         metadata, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT DO UPDATE SET amount = excluded.amount, \
         is_automatic = excluded.is_automatic, metadata = excluded.metadata",
    )
    .bind(row_id("disc"))
    .bind(cart_id.as_str())
    .bind(input.promotion_id.as_ref().map(PromotionId::as_str))
    .bind(input.code.as_deref())
    .bind(input.amount)
    .bind(input.is_automatic)
    .bind(input.metadata.to_json_string())
    .bind(created_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

fn json_string<T: Serialize>(value: &T) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::internal(format!("failed to serialize cart snapshot: {error}")))
}

fn parse_address(raw: &str) -> Result<Address, AppError> {
    serde_json::from_str(raw)
        .map_err(|error| AppError::database(format!("malformed cart address snapshot: {error}")))
}

fn parse_data(raw: &str) -> Result<Value, AppError> {
    serde_json::from_str(raw)
        .map_err(|error| AppError::database(format!("malformed shipping snapshot: {error}")))
}

/// Generates an opaque surrogate key for a child row.
///
/// The kernel identifier generator supplies the unique, time-ordered suffix;
/// the `cart_` prefix it normally carries is replaced with a table hint.
fn row_id(prefix: &str) -> String {
    let generated = CartId::new();
    let raw = generated.as_str();
    let suffix = raw.strip_prefix("cart_").unwrap_or(raw);
    format!("{prefix}_{suffix}")
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod discount_allocation_tests;

#[cfg(test)]
mod recalculation_tests;
