//! Pricing entities, inputs and the deterministic price selector.
//!
//! Money is always represented as an integer amount in minor currency units.
//! Selection is a pure function so the same inputs always produce the same
//! price across storefront, checkout and administrative surfaces.

use std::cmp::Ordering;
use std::str::FromStr;

use fastcart_kernel::{
    AppError, CurrencyCode, Metadata, Money, validate_length, validate_metadata,
    validate_non_negative_amount, validate_quantity,
};
use serde::{Deserialize, Serialize};

/// Whether a price list is a sale or an override list.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceListType {
    /// Discounted prices that are compared against a base price.
    #[default]
    Sale,
    /// Absolute prices that replace a base price.
    Override,
}

impl PriceListType {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sale => "sale",
            Self::Override => "override",
        }
    }
}

impl FromStr for PriceListType {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "sale" => Ok(Self::Sale),
            "override" => Ok(Self::Override),
            other => Err(AppError::validation(format!(
                "invalid price list type: {other}"
            ))),
        }
    }
}

/// Lifecycle state of a price list.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceListStatus {
    /// The list exists but is not applied.
    #[default]
    Draft,
    /// The list is eligible for price selection.
    Active,
}

impl PriceListStatus {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
        }
    }
}

impl FromStr for PriceListStatus {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            other => Err(AppError::validation(format!(
                "invalid price list status: {other}"
            ))),
        }
    }
}

/// An attribute condition attached to a price list.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceListRule {
    /// Rule identifier.
    pub id: String,
    /// Owning price list.
    pub price_list_id: String,
    /// Attribute name, for example `customer_group_id`.
    pub attribute: String,
    /// Comparison operator, `eq` by default.
    pub operator: String,
    /// Serialized rule value.
    pub value: String,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
}

/// Input for a price list rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceListRuleInput {
    /// Attribute name, for example `customer_group_id`.
    pub attribute: String,
    /// Comparison operator. Defaults to `eq`.
    pub operator: Option<String>,
    /// Serialized rule value.
    pub value: String,
}

impl PriceListRuleInput {
    /// Creates an `eq` rule.
    #[must_use]
    pub fn eq(attribute: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            attribute: attribute.into(),
            operator: Some("eq".to_owned()),
            value: value.into(),
        }
    }
}

/// A named collection of prices and rules.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PriceList {
    /// Price list identifier.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Optional description.
    pub description: Option<String>,
    /// Sale or override behaviour.
    pub price_type: PriceListType,
    /// Lifecycle state.
    pub status: PriceListStatus,
    /// Start of the validity window, inclusive.
    pub starts_at: Option<i64>,
    /// End of the validity window, exclusive.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Attribute rules attached to the list.
    pub rules: Vec<PriceListRule>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// Input for creating a price list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CreatePriceListInput {
    /// Human-readable title.
    pub title: String,
    /// Optional description.
    pub description: Option<String>,
    /// Sale or override behaviour.
    pub price_type: PriceListType,
    /// Lifecycle state.
    pub status: PriceListStatus,
    /// Start of the validity window, inclusive.
    pub starts_at: Option<i64>,
    /// End of the validity window, exclusive.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Attribute rules attached to the list.
    pub rules: Vec<PriceListRuleInput>,
}

impl CreatePriceListInput {
    /// Creates an empty draft price list.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    /// Marks the price list active.
    #[must_use]
    pub fn active(mut self) -> Self {
        self.status = PriceListStatus::Active;
        self
    }

    /// Adds an attribute rule.
    #[must_use]
    pub fn with_rule(mut self, rule: PriceListRuleInput) -> Self {
        self.rules.push(rule);
        self
    }

    /// Sets the validity window.
    #[must_use]
    pub fn with_window(mut self, starts_at: Option<i64>, ends_at: Option<i64>) -> Self {
        self.starts_at = starts_at;
        self.ends_at = ends_at;
        self
    }
}

/// Input for updating a price list.
///
/// Nullable attributes use `Option<Option<T>>`: the outer `None` leaves the
/// value unchanged, while `Some(None)` clears it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdatePriceListInput {
    /// New title.
    pub title: Option<String>,
    /// New description.
    pub description: Option<Option<String>>,
    /// New list type.
    pub price_type: Option<PriceListType>,
    /// New lifecycle state.
    pub status: Option<PriceListStatus>,
    /// New start of the validity window.
    pub starts_at: Option<Option<i64>>,
    /// New end of the validity window.
    pub ends_at: Option<Option<i64>>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
    /// Replacement rules when present.
    pub rules: Option<Vec<PriceListRuleInput>>,
}

/// A container for every base, sale and override price of one variant.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PriceSet {
    /// Price set identifier.
    pub id: String,
    /// Human-readable title.
    pub title: Option<String>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// A price for one variant in one currency.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Price {
    /// Price identifier.
    pub id: String,
    /// Optional owning price list. `None` denotes a base price.
    pub price_list_id: Option<String>,
    /// Type of the owning price list, or `None` for a base price.
    ///
    /// Resolved when the price is loaded so selection can distinguish sale
    /// discounts from absolute overrides without a second query.
    pub price_list_type: Option<PriceListType>,
    /// Variant the price applies to.
    pub variant_id: String,
    /// ISO-style currency code.
    pub currency_code: CurrencyCode,
    /// Amount in minor currency units.
    pub amount: i64,
    /// Minimum quantity in this tier.
    pub min_quantity: i64,
    /// Optional maximum quantity in this tier.
    pub max_quantity: Option<i64>,
    /// Optional customer group scope.
    pub customer_group_id: Option<String>,
    /// Optional sales channel scope.
    pub sales_channel_id: Option<String>,
    /// Start of the validity window, inclusive.
    pub starts_at: Option<i64>,
    /// End of the validity window, exclusive.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
    /// Number of matching rules attached to the owning price list.
    ///
    /// Internal ranking input only: it never leaves the service boundary.
    #[serde(skip)]
    #[schema(ignore)]
    pub price_list_rule_count: i64,
}

/// Input for creating a price.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatePriceInput {
    /// Optional owning price list.
    pub price_list_id: Option<String>,
    /// Variant the price applies to.
    pub variant_id: String,
    /// ISO-style currency code.
    pub currency_code: CurrencyCode,
    /// Amount in minor currency units.
    pub amount: i64,
    /// Minimum quantity in this tier.
    pub min_quantity: i64,
    /// Optional maximum quantity in this tier.
    pub max_quantity: Option<i64>,
    /// Optional customer group scope.
    pub customer_group_id: Option<String>,
    /// Optional sales channel scope.
    pub sales_channel_id: Option<String>,
    /// Start of the validity window, inclusive.
    pub starts_at: Option<i64>,
    /// End of the validity window, exclusive.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
}

impl CreatePriceInput {
    /// Creates a base price with a minimum quantity of one.
    #[must_use]
    pub fn new(variant_id: impl Into<String>, currency_code: CurrencyCode, amount: i64) -> Self {
        Self {
            price_list_id: None,
            variant_id: variant_id.into(),
            currency_code,
            amount,
            min_quantity: 1,
            max_quantity: None,
            customer_group_id: None,
            sales_channel_id: None,
            starts_at: None,
            ends_at: None,
            metadata: Metadata::empty(),
        }
    }

    /// Assigns the price to a price list.
    #[must_use]
    pub fn with_price_list(mut self, price_list_id: impl Into<String>) -> Self {
        self.price_list_id = Some(price_list_id.into());
        self
    }

    /// Sets the quantity tier.
    #[must_use]
    pub fn with_quantity_range(mut self, min_quantity: i64, max_quantity: Option<i64>) -> Self {
        self.min_quantity = min_quantity;
        self.max_quantity = max_quantity;
        self
    }

    /// Scopes the price to a customer group.
    #[must_use]
    pub fn with_customer_group(mut self, customer_group_id: impl Into<String>) -> Self {
        self.customer_group_id = Some(customer_group_id.into());
        self
    }

    /// Scopes the price to a sales channel.
    #[must_use]
    pub fn with_sales_channel(mut self, sales_channel_id: impl Into<String>) -> Self {
        self.sales_channel_id = Some(sales_channel_id.into());
        self
    }

    /// Sets the validity window.
    #[must_use]
    pub fn with_window(mut self, starts_at: Option<i64>, ends_at: Option<i64>) -> Self {
        self.starts_at = starts_at;
        self.ends_at = ends_at;
        self
    }
}

/// Input for updating a price.
///
/// Nullable attributes use `Option<Option<T>>`: the outer `None` leaves the
/// value unchanged, while `Some(None)` clears it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdatePriceInput {
    /// New owning price list.
    pub price_list_id: Option<Option<String>>,
    /// New variant.
    pub variant_id: Option<String>,
    /// New currency.
    pub currency_code: Option<CurrencyCode>,
    /// New amount in minor units.
    pub amount: Option<i64>,
    /// New minimum quantity.
    pub min_quantity: Option<i64>,
    /// New maximum quantity.
    pub max_quantity: Option<Option<i64>>,
    /// New customer group scope.
    pub customer_group_id: Option<Option<String>>,
    /// New sales channel scope.
    pub sales_channel_id: Option<Option<String>>,
    /// New start of the validity window.
    pub starts_at: Option<Option<i64>>,
    /// New end of the validity window.
    pub ends_at: Option<Option<i64>>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Filters accepted when listing prices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PriceFilter {
    /// Restrict to one variant.
    pub variant_id: Option<String>,
    /// Restrict to one price list.
    pub price_list_id: Option<String>,
    /// Restrict to one currency.
    pub currency_code: Option<CurrencyCode>,
}

/// Input used to select a price for a variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceContext {
    /// Requested currency.
    pub currency: CurrencyCode,
    /// Requested quantity.
    pub quantity: i64,
    /// Customer group of the buyer, when known.
    pub customer_group_id: Option<String>,
    /// Sales channel of the request, when known.
    pub sales_channel_id: Option<String>,
    /// Evaluation timestamp in Unix seconds.
    pub at: i64,
}

/// The price chosen for a variant.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedPrice {
    /// Winning price identifier.
    pub price_id: String,
    /// Amount in minor currency units.
    pub amount: i64,
    /// Currency of the winning price.
    pub currency: CurrencyCode,
    /// Winning price list, or `None` for a base price.
    pub price_list_id: Option<String>,
    /// Type of the winning price list, or `None` for a base price.
    pub price_list_type: Option<PriceListType>,
    /// Best base amount the winning price was compared against, when any.
    pub original_amount: Option<i64>,
}

impl SelectedPrice {
    /// Converts the selection into a monetary value.
    #[must_use]
    pub fn to_money(&self) -> Money {
        Money::new(self.amount, self.currency.clone())
    }
}

/// Selects the best eligible price.
///
/// Candidates are filtered first:
///
/// 1. the currency must match exactly;
/// 2. the context quantity must fall inside `[min_quantity, max_quantity]`,
///    where `min_quantity` defaults to one;
/// 3. the context timestamp must fall inside the price window
///    `[starts_at, ends_at)`;
/// 4. a customer-group-scoped price only matches the same group, and a
///    sales-channel-scoped price only matches the same channel.
///
/// Eligible candidates are grouped by the type of their price list and the
/// following precedence applies:
///
/// 1. an override list replaces the base price and wins outright;
/// 2. otherwise a sale list only wins when its amount is strictly lower than
///    the best base price, so a sale can never increase the payable amount;
/// 3. when no base price exists, the best sale price is used because there is
///    nothing to discount against;
/// 4. without a sale candidate the best base price wins.
///
/// Candidates of the same class are ranked deterministically:
///
/// 1. more matching price-list rules (more specific targeting) win;
/// 2. customer-group-scoped prices beat unscoped prices;
/// 3. sales-channel-scoped prices beat unscoped prices;
/// 4. a higher `min_quantity` tier wins;
/// 5. the lowest amount wins;
/// 6. the newest `starts_at` wins (a missing start is the oldest);
/// 7. the lexicographically smallest identifier wins as a final tie-break.
#[must_use]
pub fn select_price(candidates: &[Price], context: &PriceContext) -> Option<SelectedPrice> {
    let eligible: Vec<&Price> = candidates
        .iter()
        .filter(|price| price.currency_code == context.currency)
        .filter(|price| quantity_matches(price, context.quantity))
        .filter(|price| window_contains(context.at, price.starts_at, price.ends_at))
        .filter(|price| scope_matches(&price.customer_group_id, &context.customer_group_id))
        .filter(|price| scope_matches(&price.sales_channel_id, &context.sales_channel_id))
        .collect();
    if eligible.is_empty() {
        return None;
    }

    let mut overrides = Vec::new();
    let mut sales = Vec::new();
    let mut bases = Vec::new();
    for price in eligible {
        match price_kind(price) {
            PriceKind::Override => overrides.push(price),
            PriceKind::Sale => sales.push(price),
            PriceKind::Base => bases.push(price),
        }
    }

    let best_override = best_candidate(&overrides);
    let best_sale = best_candidate(&sales);
    let best_base = best_candidate(&bases);

    let (winner, original_amount) = match (best_override, best_base, best_sale) {
        (Some(over), base, _) => (over, base.map(|price| price.amount)),
        (None, Some(base), Some(sale)) => {
            if sale.amount < base.amount {
                (sale, Some(base.amount))
            } else {
                (base, None)
            }
        }
        (None, Some(base), None) => (base, None),
        (None, None, Some(sale)) => (sale, None),
        (None, None, None) => return None,
    };

    Some(SelectedPrice {
        price_id: winner.id.clone(),
        amount: winner.amount,
        currency: winner.currency_code.clone(),
        price_list_id: winner.price_list_id.clone(),
        price_list_type: winner.price_list_type,
        original_amount,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PriceKind {
    Base,
    Sale,
    Override,
}

fn price_kind(price: &Price) -> PriceKind {
    match (&price.price_list_id, price.price_list_type) {
        (None, _) => PriceKind::Base,
        (Some(_), Some(PriceListType::Override)) => PriceKind::Override,
        // A list price without a resolved type is treated as a sale for
        // backwards compatibility with candidates built by older callers.
        (Some(_), _) => PriceKind::Sale,
    }
}

fn best_candidate<'a>(candidates: &[&'a Price]) -> Option<&'a Price> {
    candidates
        .iter()
        .copied()
        .min_by(|left, right| compare_candidates(left, right))
}

fn quantity_matches(price: &Price, quantity: i64) -> bool {
    quantity >= price.min_quantity && price.max_quantity.is_none_or(|maximum| quantity <= maximum)
}

fn scope_matches(candidate: &Option<String>, requested: &Option<String>) -> bool {
    candidate.is_none() || candidate == requested
}

fn window_contains(at: i64, starts_at: Option<i64>, ends_at: Option<i64>) -> bool {
    starts_at.is_none_or(|start| at >= start) && ends_at.is_none_or(|end| at < end)
}

fn compare_candidates(left: &Price, right: &Price) -> Ordering {
    right
        .price_list_rule_count
        .cmp(&left.price_list_rule_count)
        .then_with(|| {
            right
                .customer_group_id
                .is_some()
                .cmp(&left.customer_group_id.is_some())
        })
        .then_with(|| {
            right
                .sales_channel_id
                .is_some()
                .cmp(&left.sales_channel_id.is_some())
        })
        .then_with(|| right.min_quantity.cmp(&left.min_quantity))
        .then_with(|| left.amount.cmp(&right.amount))
        .then_with(|| {
            right
                .starts_at
                .unwrap_or(i64::MIN)
                .cmp(&left.starts_at.unwrap_or(i64::MIN))
        })
        .then_with(|| left.id.cmp(&right.id))
}

pub(crate) fn validate_window(
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    resource: &str,
) -> Result<(), AppError> {
    if let (Some(start), Some(end)) = (starts_at, ends_at)
        && start >= end
    {
        return Err(AppError::validation(format!(
            "{resource} starts_at must be before ends_at"
        )));
    }
    Ok(())
}

pub(crate) fn validate_price_list_fields(
    title: &str,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    metadata: &Metadata,
) -> Result<(), AppError> {
    validate_length(title, "price list title", 120)?;
    validate_window(starts_at, ends_at, "price list")?;
    validate_metadata(metadata)
}

pub(crate) fn validate_price_fields(
    variant_id: &str,
    amount: i64,
    min_quantity: i64,
    max_quantity: Option<i64>,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    metadata: &Metadata,
) -> Result<(), AppError> {
    validate_length(variant_id, "variant_id", 128)?;
    validate_non_negative_amount(amount, "price amount")?;
    validate_quantity(min_quantity, false)?;
    if let Some(maximum) = max_quantity {
        validate_quantity(maximum, false)?;
        if maximum < min_quantity {
            return Err(AppError::validation(
                "max_quantity must be greater than or equal to min_quantity",
            ));
        }
    }
    validate_window(starts_at, ends_at, "price")?;
    validate_metadata(metadata)
}

pub(crate) fn normalize_rule_input(
    input: &PriceListRuleInput,
) -> Result<(String, String, String), AppError> {
    let attribute = input.attribute.trim().to_owned();
    if attribute.is_empty() {
        return Err(AppError::validation(
            "price list rule attribute must not be empty",
        ));
    }
    let operator = input
        .operator
        .as_deref()
        .unwrap_or("eq")
        .trim()
        .to_ascii_lowercase();
    if operator.is_empty() {
        return Err(AppError::validation(
            "price list rule operator must not be empty",
        ));
    }
    if input.value.trim().is_empty() {
        return Err(AppError::validation(
            "price list rule value must not be empty",
        ));
    }
    Ok((attribute, operator, input.value.clone()))
}
