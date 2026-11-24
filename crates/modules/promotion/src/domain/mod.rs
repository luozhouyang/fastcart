//! Promotion entities, inputs, rules and the deterministic calculator.
//!
//! Promotion evaluation is pure and deterministic. The service layer supplies
//! eligible promotions; this module never depends on wall-clock time or random
//! state. All monetary values are integer minor units.

use std::str::FromStr;

use fastcart_kernel::{
    AppError, CurrencyCode, Metadata, Money, Rounding, allocate_proportionally, validate_length,
    validate_metadata,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Supported promotion discount types.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionType {
    /// A percentage of the eligible line subtotal in basis points.
    Percentage,
    /// A fixed amount in minor currency units.
    FixedAmount,
    /// Free shipping on the cart.
    FreeShipping,
}

impl PromotionType {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Percentage => "percentage",
            Self::FixedAmount => "fixed_amount",
            Self::FreeShipping => "free_shipping",
        }
    }
}

impl FromStr for PromotionType {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "percentage" => Ok(Self::Percentage),
            "fixed_amount" => Ok(Self::FixedAmount),
            "free_shipping" => Ok(Self::FreeShipping),
            other => Err(AppError::validation(format!(
                "invalid promotion type: {other}"
            ))),
        }
    }
}

/// The part of the cart a percentage or fixed amount discount applies to.
///
/// `items` is the historic behaviour and the default: the discount is computed
/// per eligible line and recorded as a line adjustment. `order` computes
/// against the item subtotal plus shipping and `shipping` against the shipping
/// total; both produce a single application that consumes the cart's remaining
/// value, so they cannot discount the cart below zero.
///
/// Line-level rules (`product_id`, `variant_id`, `category_id`) still gate
/// eligibility on every target, but the `order` and `shipping` amounts are
/// computed from the cart totals rather than from the eligible lines.
///
/// `FreeShipping` is a shipping discount by definition and only accepts the
/// default target.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionTarget {
    /// The eligible line items.
    #[default]
    Items,
    /// The whole order: the item subtotal plus shipping.
    Order,
    /// Shipping alone.
    Shipping,
}

impl PromotionTarget {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Items => "items",
            Self::Order => "order",
            Self::Shipping => "shipping",
        }
    }
}

impl FromStr for PromotionTarget {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "items" => Ok(Self::Items),
            "order" => Ok(Self::Order),
            "shipping" => Ok(Self::Shipping),
            other => Err(AppError::validation(format!(
                "invalid promotion target: {other}"
            ))),
        }
    }
}

/// Lifecycle state of a promotion.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionStatus {
    /// Prepared but not applied.
    Draft,
    /// Eligible for application.
    Active,
    /// No longer eligible.
    Expired,
    /// Manually disabled.
    Disabled,
}

impl PromotionStatus {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Disabled => "disabled",
        }
    }
}

impl FromStr for PromotionStatus {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            "expired" => Ok(Self::Expired),
            "disabled" => Ok(Self::Disabled),
            other => Err(AppError::validation(format!(
                "invalid promotion status: {other}"
            ))),
        }
    }
}

/// Attributes a promotion rule can restrict.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionRuleAttribute {
    /// Minimum cart subtotal.
    MinimumOrderSubtotal,
    /// Eligible product identifier.
    ProductId,
    /// Eligible variant identifier.
    VariantId,
    /// Eligible category identifier.
    CategoryId,
    /// Required customer group.
    CustomerGroupId,
    /// Required cart currency.
    Currency,
    /// Whether the customer is placing a first order.
    FirstOrder,
}

impl PromotionRuleAttribute {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MinimumOrderSubtotal => "minimum_order_subtotal",
            Self::ProductId => "product_id",
            Self::VariantId => "variant_id",
            Self::CategoryId => "category_id",
            Self::CustomerGroupId => "customer_group_id",
            Self::Currency => "currency",
            Self::FirstOrder => "first_order",
        }
    }
}

impl FromStr for PromotionRuleAttribute {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "minimum_order_subtotal" => Ok(Self::MinimumOrderSubtotal),
            "product_id" => Ok(Self::ProductId),
            "variant_id" => Ok(Self::VariantId),
            "category_id" => Ok(Self::CategoryId),
            "customer_group_id" => Ok(Self::CustomerGroupId),
            "currency" => Ok(Self::Currency),
            "first_order" => Ok(Self::FirstOrder),
            other => Err(AppError::validation(format!(
                "invalid promotion rule attribute: {other}"
            ))),
        }
    }
}

/// Comparison operator used by promotion rules.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionRuleOperator {
    /// Equality with a scalar or one of a list.
    Eq,
    /// Membership in a list.
    In,
    /// Greater than or equal to a numeric threshold.
    Gte,
    /// Less than or equal to a numeric threshold.
    Lte,
}

impl PromotionRuleOperator {
    /// Database representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::In => "in",
            Self::Gte => "gte",
            Self::Lte => "lte",
        }
    }
}

impl FromStr for PromotionRuleOperator {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "eq" => Ok(Self::Eq),
            "in" => Ok(Self::In),
            "gte" => Ok(Self::Gte),
            "lte" => Ok(Self::Lte),
            other => Err(AppError::validation(format!(
                "invalid promotion rule operator: {other}"
            ))),
        }
    }
}

/// A condition attached to a promotion.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionRule {
    /// Rule identifier.
    pub id: String,
    /// Owning promotion.
    pub promotion_id: String,
    /// Restricted attribute.
    pub attribute: PromotionRuleAttribute,
    /// Comparison operator.
    pub operator: PromotionRuleOperator,
    /// Rule value stored as JSON text.
    pub value: String,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
}

/// Input for a promotion rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionRuleInput {
    /// Restricted attribute.
    pub attribute: PromotionRuleAttribute,
    /// Comparison operator.
    pub operator: PromotionRuleOperator,
    /// Value serialized as JSON text, for example `"\"USD\""` or `[1,2,3]`.
    pub value: String,
}

impl PromotionRuleInput {
    /// Creates a rule.
    #[must_use]
    pub fn new(
        attribute: PromotionRuleAttribute,
        operator: PromotionRuleOperator,
        value: impl Into<String>,
    ) -> Self {
        Self {
            attribute,
            operator,
            value: value.into(),
        }
    }
}

/// A discount campaign.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Promotion {
    /// Promotion identifier.
    pub id: String,
    /// Optional unique coupon-style code.
    pub code: Option<String>,
    /// Human-readable title.
    pub title: String,
    /// Discount type.
    pub promotion_type: PromotionType,
    /// Where the discount applies.
    pub target: PromotionTarget,
    /// Basis points for percentages, minor units for fixed amounts, zero for
    /// free shipping.
    pub value: i64,
    /// Lifecycle state.
    pub status: PromotionStatus,
    /// Whether the promotion applies without a coupon code.
    pub is_automatic: bool,
    /// Start of the validity window, inclusive.
    pub starts_at: Option<i64>,
    /// End of the validity window, exclusive.
    pub ends_at: Option<i64>,
    /// Optional total usage limit.
    pub usage_limit: Option<i64>,
    /// Number of recorded usages.
    pub usage_count: i64,
    /// Optional limit on how many times one customer may redeem the promotion.
    pub per_customer_limit: Option<i64>,
    /// Ordering weight: promotions with a higher priority consume the cart
    /// first, then by `created_at` and `id`. The default of 0 puts every
    /// promotion in the historic `created_at, id` order.
    pub priority: i64,
    /// Whether promotions ordered after this one still apply once it has
    /// discounted. `false` makes the promotion exclusive.
    pub is_stackable: bool,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Rules attached to the promotion.
    pub rules: Vec<PromotionRule>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// Input for creating a promotion.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatePromotionInput {
    /// Optional unique code.
    pub code: Option<String>,
    /// Human-readable title.
    pub title: String,
    /// Discount type.
    pub promotion_type: PromotionType,
    /// Where the discount applies; defaults to the eligible line items.
    pub target: PromotionTarget,
    /// Basis points for percentages, minor units for fixed amounts.
    pub value: i64,
    /// Lifecycle state.
    pub status: PromotionStatus,
    /// Whether the promotion applies without a coupon code.
    pub is_automatic: bool,
    /// Start of the validity window, inclusive.
    pub starts_at: Option<i64>,
    /// End of the validity window, exclusive.
    pub ends_at: Option<i64>,
    /// Optional total usage limit.
    pub usage_limit: Option<i64>,
    /// Optional limit on how many times one customer may redeem the promotion.
    pub per_customer_limit: Option<i64>,
    /// Ordering weight, defaulting to 0 (the `created_at, id` order).
    pub priority: i64,
    /// Whether later promotions still apply once this one has discounted.
    pub is_stackable: bool,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Rules attached to the promotion.
    pub rules: Vec<PromotionRuleInput>,
}

impl CreatePromotionInput {
    /// Creates a draft promotion.
    #[must_use]
    pub fn new(title: impl Into<String>, promotion_type: PromotionType, value: i64) -> Self {
        Self {
            code: None,
            title: title.into(),
            promotion_type,
            target: PromotionTarget::Items,
            value,
            status: PromotionStatus::Draft,
            is_automatic: false,
            starts_at: None,
            ends_at: None,
            usage_limit: None,
            per_customer_limit: None,
            priority: 0,
            is_stackable: true,
            metadata: Metadata::empty(),
            rules: Vec::new(),
        }
    }

    /// Sets the promotion code.
    #[must_use]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Sets where the discount applies.
    #[must_use]
    pub fn with_target(mut self, target: PromotionTarget) -> Self {
        self.target = target;
        self
    }

    /// Marks the promotion active.
    #[must_use]
    pub fn active(mut self) -> Self {
        self.status = PromotionStatus::Active;
        self
    }

    /// Marks the promotion automatic.
    #[must_use]
    pub fn automatic(mut self) -> Self {
        self.is_automatic = true;
        self
    }

    /// Sets the validity window.
    #[must_use]
    pub fn with_window(mut self, starts_at: Option<i64>, ends_at: Option<i64>) -> Self {
        self.starts_at = starts_at;
        self.ends_at = ends_at;
        self
    }

    /// Sets the usage limit.
    #[must_use]
    pub fn with_usage_limit(mut self, usage_limit: i64) -> Self {
        self.usage_limit = Some(usage_limit);
        self
    }

    /// Sets how many times one customer may redeem the promotion.
    #[must_use]
    pub fn with_per_customer_limit(mut self, per_customer_limit: i64) -> Self {
        self.per_customer_limit = Some(per_customer_limit);
        self
    }

    /// Sets the ordering weight. A higher priority is applied earlier.
    #[must_use]
    pub fn with_priority(mut self, priority: i64) -> Self {
        self.priority = priority;
        self
    }

    /// Makes the promotion exclusive: once it produces a discount, promotions
    /// ordered after it are skipped for that batch.
    #[must_use]
    pub fn exclusive(mut self) -> Self {
        self.is_stackable = false;
        self
    }

    /// Adds a rule.
    #[must_use]
    pub fn with_rule(mut self, rule: PromotionRuleInput) -> Self {
        self.rules.push(rule);
        self
    }
}

/// Input for updating a promotion.
///
/// Nullable attributes use `Option<Option<T>>`: the outer `None` leaves the
/// value unchanged, while `Some(None)` clears it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdatePromotionInput {
    /// New code.
    pub code: Option<Option<String>>,
    /// New title.
    pub title: Option<String>,
    /// New discount type.
    pub promotion_type: Option<PromotionType>,
    /// New target; `None` keeps the stored one.
    pub target: Option<PromotionTarget>,
    /// New discount value.
    pub value: Option<i64>,
    /// New lifecycle state.
    pub status: Option<PromotionStatus>,
    /// New automatic flag.
    pub is_automatic: Option<bool>,
    /// New start of the validity window.
    pub starts_at: Option<Option<i64>>,
    /// New end of the validity window.
    pub ends_at: Option<Option<i64>>,
    /// New usage limit.
    pub usage_limit: Option<Option<i64>>,
    /// New per-customer limit.
    pub per_customer_limit: Option<Option<i64>>,
    /// New ordering weight; `None` keeps the stored one.
    pub priority: Option<i64>,
    /// New stackability; `None` keeps the stored one, `Some(false)` makes the
    /// promotion exclusive.
    pub is_stackable: Option<bool>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
    /// Replacement rules when present.
    pub rules: Option<Vec<PromotionRuleInput>>,
}

/// Filters accepted when listing promotions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromotionFilter {
    /// Restrict to one status.
    pub status: Option<PromotionStatus>,
    /// Restrict to automatic or non-automatic promotions.
    pub is_automatic: Option<bool>,
    /// Restrict to one discount type.
    pub promotion_type: Option<PromotionType>,
}

/// A coupon that unlocks a promotion.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Coupon {
    /// Coupon identifier.
    pub id: String,
    /// Unique normalized code.
    pub code: String,
    /// Promotion unlocked by the coupon.
    pub promotion_id: String,
    /// Optional coupon usage limit.
    pub usage_limit: Option<i64>,
    /// Number of recorded coupon usages.
    pub usage_count: i64,
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
}

/// Input for creating a coupon.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateCouponInput {
    /// Unique coupon code.
    pub code: String,
    /// Promotion unlocked by the coupon.
    pub promotion_id: String,
    /// Optional coupon usage limit.
    pub usage_limit: Option<i64>,
    /// Start of the validity window, inclusive.
    pub starts_at: Option<i64>,
    /// End of the validity window, exclusive.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
}

impl CreateCouponInput {
    /// Creates a coupon with no limits or window.
    #[must_use]
    pub fn new(code: impl Into<String>, promotion_id: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            promotion_id: promotion_id.into(),
            usage_limit: None,
            starts_at: None,
            ends_at: None,
            metadata: Metadata::empty(),
        }
    }

    /// Sets the coupon usage limit.
    #[must_use]
    pub fn with_usage_limit(mut self, usage_limit: i64) -> Self {
        self.usage_limit = Some(usage_limit);
        self
    }

    /// Sets the coupon validity window.
    #[must_use]
    pub fn with_window(mut self, starts_at: Option<i64>, ends_at: Option<i64>) -> Self {
        self.starts_at = starts_at;
        self.ends_at = ends_at;
        self
    }
}

/// Input for updating a coupon.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateCouponInput {
    /// New code.
    pub code: Option<String>,
    /// New promotion.
    pub promotion_id: Option<String>,
    /// New usage limit.
    pub usage_limit: Option<Option<i64>>,
    /// New start of the validity window.
    pub starts_at: Option<Option<i64>>,
    /// New end of the validity window.
    pub ends_at: Option<Option<i64>>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Filters accepted when listing coupons.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CouponFilter {
    /// Restrict to one promotion.
    pub promotion_id: Option<String>,
}

/// A recorded promotion usage.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionUsage {
    /// Usage identifier.
    pub id: String,
    /// Promotion that was used.
    pub promotion_id: String,
    /// Coupon that was used, when any.
    pub coupon_id: Option<String>,
    /// Customer that used the promotion, when known.
    pub customer_id: Option<String>,
    /// Order that recorded the usage, when known.
    pub order_id: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
}

/// A cart line visible to the promotion calculator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionLine {
    /// Line identifier.
    pub line_id: String,
    /// Product identifier.
    pub product_id: String,
    /// Variant identifier.
    pub variant_id: String,
    /// Category identifiers.
    pub category_ids: Vec<String>,
    /// Unit price in minor currency units.
    pub unit_price: i64,
    /// Quantity on the line.
    pub quantity: i64,
    /// Whether the line represents shipping.
    pub is_shipping: bool,
}

/// The cart snapshot evaluated by promotions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionCart {
    /// Cart currency.
    pub currency: CurrencyCode,
    /// Cart subtotal in minor units.
    pub subtotal: i64,
    /// Shipping total in minor units.
    pub shipping_total: i64,
    /// Customer attached to the cart, when authenticated.
    pub customer_id: Option<String>,
    /// Customer groups attached to the cart.
    pub customer_group_ids: Vec<String>,
    /// Cart lines.
    pub lines: Vec<PromotionLine>,
    /// Evaluation timestamp in Unix seconds.
    pub at: i64,
}

/// A promotion paired with the rules to evaluate.
#[derive(Clone, Debug, PartialEq)]
pub struct PromotionContext {
    /// Promotion being evaluated.
    pub promotion: Promotion,
    /// Rules attached to the promotion.
    pub rules: Vec<PromotionRule>,
}

/// One discount produced by a promotion.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionApplication {
    /// Promotion that produced the application.
    pub promotion_id: String,
    /// Promotion code, when configured.
    pub code: Option<String>,
    /// Line receiving the discount, or `None` for shipping.
    pub line_id: Option<String>,
    /// Discount amount in minor currency units.
    pub amount: i64,
    /// Whether this application discounts shipping.
    pub is_shipping: bool,
    /// Human-readable explanation.
    pub description: String,
}

/// The applications produced by one promotion.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionResult {
    /// Promotion that produced the result.
    pub promotion_id: String,
    /// Promotion code, when configured.
    pub code: Option<String>,
    /// Individual applications.
    pub applications: Vec<PromotionApplication>,
    /// Total line discount, excluding shipping.
    pub total_discount: i64,
    /// Total shipping discount.
    pub shipping_discount: i64,
}

mod calculation;
mod campaign;

pub use calculation::{apply_promotion, calculate_promotions};
pub(crate) use calculation::{
    normalize_code, validate_per_customer_limit, validate_promotion_fields, validate_usage_limit,
    validate_window, window_contains,
};
pub(crate) use campaign::validate_campaign_budget;
pub use campaign::{Campaign, CampaignBudgetType, CreateCampaignInput, UpdateCampaignInput};

#[cfg(test)]
mod tests;
