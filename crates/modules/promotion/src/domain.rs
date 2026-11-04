//! Promotion entities, inputs, rules and the deterministic calculator.
//!
//! Promotion evaluation is pure and deterministic. The service layer supplies
//! eligible promotions; this module never depends on wall-clock time or random
//! state. All monetary values are integer minor units.

use std::str::FromStr;

use fastcart_kernel::{
    AppError, CurrencyCode, Metadata, Money, Rounding, validate_length, validate_metadata,
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
            value,
            status: PromotionStatus::Draft,
            is_automatic: false,
            starts_at: None,
            ends_at: None,
            usage_limit: None,
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

/// Applies one promotion against the full cart.
///
/// Returns `None` when the promotion does not match the cart or produces no
/// discount. See [`calculate_promotions`] for stacking behaviour.
pub fn apply_promotion(
    context: &PromotionContext,
    cart: &PromotionCart,
) -> Result<Option<PromotionResult>, AppError> {
    apply_promotion_with_remaining(
        context,
        cart,
        cart.subtotal.max(0),
        cart.shipping_total.max(0),
    )
}

/// Applies promotions in creation/id order against the remaining cart value.
///
/// Each promotion sees the subtotal and shipping left after previous
/// promotions, so no combination of promotions can discount a cart below zero.
/// Promotions that do not match or produce no discount are omitted.
pub fn calculate_promotions(
    contexts: &[PromotionContext],
    cart: &PromotionCart,
) -> Result<Vec<PromotionResult>, AppError> {
    let mut ordered: Vec<&PromotionContext> = contexts.iter().collect();
    ordered.sort_by(|left, right| {
        left.promotion
            .created_at
            .cmp(&right.promotion.created_at)
            .then_with(|| left.promotion.id.cmp(&right.promotion.id))
    });

    let mut remaining_subtotal = cart.subtotal.max(0);
    let mut remaining_shipping = cart.shipping_total.max(0);
    let mut results = Vec::new();
    for context in ordered {
        if let Some(result) =
            apply_promotion_with_remaining(context, cart, remaining_subtotal, remaining_shipping)?
        {
            remaining_subtotal = (remaining_subtotal - result.total_discount).max(0);
            remaining_shipping = (remaining_shipping - result.shipping_discount).max(0);
            results.push(result);
        }
    }
    Ok(results)
}

fn apply_promotion_with_remaining(
    context: &PromotionContext,
    cart: &PromotionCart,
    remaining_subtotal: i64,
    remaining_shipping: i64,
) -> Result<Option<PromotionResult>, AppError> {
    let Some(lines) = eligible_lines(&context.rules, cart) else {
        return Ok(None);
    };
    let has_line_targeting = context.rules.iter().any(|rule| {
        matches!(
            rule.attribute,
            PromotionRuleAttribute::ProductId
                | PromotionRuleAttribute::VariantId
                | PromotionRuleAttribute::CategoryId
        )
    });
    if has_line_targeting && lines.is_empty() {
        return Ok(None);
    }
    let remaining_subtotal = remaining_subtotal.max(0);
    let remaining_shipping = remaining_shipping.max(0);
    match context.promotion.promotion_type {
        PromotionType::Percentage => percentage_result(context, cart, &lines, remaining_subtotal),
        PromotionType::FixedAmount => fixed_amount_result(context, &lines, remaining_subtotal),
        PromotionType::FreeShipping => free_shipping_result(context, cart, remaining_shipping),
    }
}

fn percentage_result(
    context: &PromotionContext,
    cart: &PromotionCart,
    lines: &[&PromotionLine],
    remaining_subtotal: i64,
) -> Result<Option<PromotionResult>, AppError> {
    let basis_points = context.promotion.value;
    if !(0..=10_000).contains(&basis_points) {
        return Err(AppError::validation(format!(
            "percentage promotion value must be between 0 and 10000 basis points: {basis_points}"
        )));
    }
    if basis_points == 0 || remaining_subtotal <= 0 {
        return Ok(None);
    }

    let mut budget = remaining_subtotal;
    let mut applications = Vec::new();
    for line in lines {
        let subtotal = validated_line_subtotal(line)?;
        if subtotal <= 0 {
            continue;
        }
        let discount = Money::new(subtotal, cart.currency.clone())
            .apply_basis_points(basis_points, Rounding::HalfAwayFromZero)?
            .amount_minor();
        let amount = discount.min(budget);
        if amount <= 0 {
            continue;
        }
        budget -= amount;
        applications.push(application(
            context,
            Some(line.line_id.clone()),
            amount,
            false,
            format!("Percentage discount ({})", percentage_label(basis_points)),
        ));
        if budget == 0 {
            break;
        }
    }
    if applications.is_empty() {
        return Ok(None);
    }
    let total_discount = applications
        .iter()
        .map(|application| application.amount)
        .sum();
    Ok(Some(result(context, applications, total_discount, 0)))
}

fn fixed_amount_result(
    context: &PromotionContext,
    lines: &[&PromotionLine],
    remaining_subtotal: i64,
) -> Result<Option<PromotionResult>, AppError> {
    let value = context.promotion.value;
    if value < 0 {
        return Err(AppError::validation(
            "fixed amount promotion value must not be negative",
        ));
    }

    let mut eligible = Vec::new();
    let mut eligible_subtotal = 0_i64;
    for line in lines {
        let subtotal = validated_line_subtotal(line)?;
        if subtotal <= 0 {
            continue;
        }
        eligible_subtotal = eligible_subtotal
            .checked_add(subtotal)
            .ok_or_else(|| AppError::validation("promotion subtotal overflow"))?;
        eligible.push((*line, subtotal));
    }
    if eligible.is_empty() || remaining_subtotal <= 0 {
        return Ok(None);
    }

    let discount = value.min(eligible_subtotal).min(remaining_subtotal);
    if discount <= 0 {
        return Ok(None);
    }

    let last_index = eligible.len() - 1;
    let mut applications = Vec::new();
    let mut allocated = 0_i64;
    for (index, (line, subtotal)) in eligible.iter().enumerate() {
        let share = if index == last_index {
            discount - allocated
        } else {
            let proportional =
                i128::from(*subtotal) * i128::from(discount) / i128::from(eligible_subtotal);
            i64::try_from(proportional)
                .map_err(|_| AppError::validation("discount allocation overflow"))?
        };
        allocated = allocated
            .checked_add(share)
            .ok_or_else(|| AppError::validation("discount allocation overflow"))?;
        if share <= 0 {
            continue;
        }
        applications.push(application(
            context,
            Some(line.line_id.clone()),
            share,
            false,
            "Fixed amount discount".to_owned(),
        ));
    }
    if applications.is_empty() {
        return Ok(None);
    }
    Ok(Some(result(context, applications, discount, 0)))
}

fn free_shipping_result(
    context: &PromotionContext,
    cart: &PromotionCart,
    remaining_shipping: i64,
) -> Result<Option<PromotionResult>, AppError> {
    let discount = cart.shipping_total.max(0).min(remaining_shipping);
    if discount <= 0 {
        return Ok(None);
    }
    let applications = vec![application(
        context,
        None,
        discount,
        true,
        "Free shipping".to_owned(),
    )];
    Ok(Some(result(context, applications, 0, discount)))
}

fn application(
    context: &PromotionContext,
    line_id: Option<String>,
    amount: i64,
    is_shipping: bool,
    description: String,
) -> PromotionApplication {
    PromotionApplication {
        promotion_id: context.promotion.id.clone(),
        code: context.promotion.code.clone(),
        line_id,
        amount,
        is_shipping,
        description,
    }
}

fn result(
    context: &PromotionContext,
    applications: Vec<PromotionApplication>,
    total_discount: i64,
    shipping_discount: i64,
) -> PromotionResult {
    PromotionResult {
        promotion_id: context.promotion.id.clone(),
        code: context.promotion.code.clone(),
        applications,
        total_discount,
        shipping_discount,
    }
}

fn validated_line_subtotal(line: &PromotionLine) -> Result<i64, AppError> {
    if line.unit_price < 0 || line.quantity < 0 {
        return Err(AppError::validation(
            "promotion line prices and quantities must not be negative",
        ));
    }
    line.unit_price
        .checked_mul(line.quantity)
        .ok_or_else(|| AppError::validation("line subtotal overflow"))
}

fn eligible_lines<'a>(
    rules: &[PromotionRule],
    cart: &'a PromotionCart,
) -> Option<Vec<&'a PromotionLine>> {
    let mut lines: Vec<&PromotionLine> =
        cart.lines.iter().filter(|line| !line.is_shipping).collect();

    for rule in rules {
        match rule.attribute {
            PromotionRuleAttribute::MinimumOrderSubtotal => {
                if !numeric_rule_matches(rule, cart.subtotal) {
                    return None;
                }
            }
            PromotionRuleAttribute::Currency => {
                if !string_rule_matches(rule, cart.currency.as_str()) {
                    return None;
                }
            }
            PromotionRuleAttribute::CustomerGroupId => {
                if !cart
                    .customer_group_ids
                    .iter()
                    .any(|group| string_rule_matches(rule, group))
                {
                    return None;
                }
            }
            PromotionRuleAttribute::FirstOrder => {
                if !first_order_matches(rule, cart) {
                    return None;
                }
            }
            PromotionRuleAttribute::ProductId => {
                lines.retain(|line| string_rule_matches(rule, &line.product_id));
            }
            PromotionRuleAttribute::VariantId => {
                lines.retain(|line| string_rule_matches(rule, &line.variant_id));
            }
            PromotionRuleAttribute::CategoryId => {
                lines.retain(|line| {
                    line.category_ids
                        .iter()
                        .any(|category| string_rule_matches(rule, category))
                });
            }
        }
    }
    Some(lines)
}

fn string_rule_matches(rule: &PromotionRule, candidate: &str) -> bool {
    match rule.operator {
        PromotionRuleOperator::Eq | PromotionRuleOperator::In => {
            rule_strings(rule).iter().any(|value| value == candidate)
        }
        PromotionRuleOperator::Gte | PromotionRuleOperator::Lte => false,
    }
}

fn numeric_rule_matches(rule: &PromotionRule, candidate: i64) -> bool {
    let values = rule_integers(rule);
    match rule.operator {
        PromotionRuleOperator::Eq | PromotionRuleOperator::In => values.contains(&candidate),
        PromotionRuleOperator::Gte => values
            .first()
            .is_some_and(|threshold| candidate >= *threshold),
        PromotionRuleOperator::Lte => values
            .first()
            .is_some_and(|threshold| candidate <= *threshold),
    }
}

/// The pure calculator has no order history. A `first_order` rule therefore
/// only checks whether a customer is attached to the cart; callers must supply
/// first-order promotions exclusively to genuinely first-time customers.
fn first_order_matches(rule: &PromotionRule, cart: &PromotionCart) -> bool {
    let expected = rule_strings(rule)
        .first()
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(true);
    if expected {
        cart.customer_id.is_some()
    } else {
        cart.customer_id.is_none()
    }
}

fn rule_strings(rule: &PromotionRule) -> Vec<String> {
    let raw = rule.value.trim();
    if raw.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Array(items)) => items.into_iter().filter_map(json_scalar).collect(),
        Ok(value) => json_scalar(value).into_iter().collect(),
        Err(_) => vec![raw.to_owned()],
    }
}

fn json_scalar(value: Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        _ => None,
    }
}

fn rule_integers(rule: &PromotionRule) -> Vec<i64> {
    rule_strings(rule)
        .iter()
        .filter_map(|value| value.parse::<i64>().ok())
        .collect()
}

fn percentage_label(basis_points: i64) -> String {
    let whole = basis_points / 100;
    let fraction = basis_points % 100;
    if fraction == 0 {
        format!("{whole}%")
    } else {
        format!("{whole}.{fraction:02}%")
    }
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

pub(crate) fn window_contains(at: i64, starts_at: Option<i64>, ends_at: Option<i64>) -> bool {
    starts_at.is_none_or(|start| at >= start) && ends_at.is_none_or(|end| at < end)
}

pub(crate) fn normalize_code(code: &str) -> Result<String, AppError> {
    let normalized = code.trim().to_ascii_uppercase();
    validate_length(&normalized, "code", 64)?;
    if !normalized
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(AppError::validation(
            "code must contain only letters, digits, hyphens and underscores",
        ));
    }
    Ok(normalized)
}

pub(crate) fn validate_usage_limit(usage_limit: Option<i64>, field: &str) -> Result<(), AppError> {
    if usage_limit.is_some_and(|limit| limit < 0) {
        return Err(AppError::validation(format!(
            "{field} must not be negative"
        )));
    }
    Ok(())
}

pub(crate) fn validate_promotion_fields(
    title: &str,
    promotion_type: PromotionType,
    value: i64,
    usage_limit: Option<i64>,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    metadata: &Metadata,
) -> Result<(), AppError> {
    validate_length(title, "promotion title", 120)?;
    validate_window(starts_at, ends_at, "promotion")?;
    validate_usage_limit(usage_limit, "usage_limit")?;
    match promotion_type {
        PromotionType::Percentage if !(0..=10_000).contains(&value) => {
            return Err(AppError::validation(
                "percentage promotion value must be between 0 and 10000 basis points",
            ));
        }
        PromotionType::FixedAmount if value < 0 => {
            return Err(AppError::validation(
                "fixed amount promotion value must not be negative",
            ));
        }
        PromotionType::FreeShipping if value != 0 => {
            return Err(AppError::validation(
                "free shipping promotion value must be zero",
            ));
        }
        _ => {}
    }
    validate_metadata(metadata)
}

/// How a campaign budget is consumed.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignBudgetType {
    /// Each promotion usage consumes one unit.
    #[default]
    Usage,
    /// Each usage consumes the discounted amount in the campaign currency.
    Spend,
}

impl CampaignBudgetType {
    /// Database representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Spend => "spend",
        }
    }
}

impl std::str::FromStr for CampaignBudgetType {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "usage" => Ok(Self::Usage),
            "spend" => Ok(Self::Spend),
            other => Err(AppError::validation(format!(
                "invalid campaign budget type: {other}"
            ))),
        }
    }
}

/// A campaign grouping promotions under a shared budget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Campaign {
    /// Campaign identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Whether the budget counts usages or monetary spend.
    pub budget_type: CampaignBudgetType,
    /// Currency of a monetary budget; absent for usage budgets.
    pub currency_code: Option<CurrencyCode>,
    /// Maximum usages or spend in minor units, or unlimited.
    pub budget_limit: Option<i64>,
    /// Usages or spend already consumed.
    pub budget_used: i64,
    /// Start of the validity window.
    pub starts_at: Option<i64>,
    /// End of the validity window.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl Campaign {
    /// Consumed budget expressed as a monetary amount, when this is a spend
    /// campaign.
    #[must_use]
    pub fn budget_spent(&self) -> Option<Money> {
        match (self.budget_type, self.currency_code.clone()) {
            (CampaignBudgetType::Spend, Some(currency)) => {
                Some(Money::new(self.budget_used, currency))
            }
            _ => None,
        }
    }
}

/// Input for creating a campaign.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateCampaignInput {
    /// Display name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Budget accounting mode; defaults to usage.
    #[serde(default)]
    pub budget_type: CampaignBudgetType,
    /// Currency required for spend budgets.
    #[serde(default)]
    pub currency_code: Option<CurrencyCode>,
    /// Maximum usages or spend in minor units.
    pub budget_limit: Option<i64>,
    /// Start timestamp.
    pub starts_at: Option<i64>,
    /// End timestamp.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
}

/// Input for partially updating a campaign.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateCampaignInput {
    /// New name.
    pub name: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New budget accounting mode.
    pub budget_type: Option<CampaignBudgetType>,
    /// New budget currency; `Some(None)` clears it.
    pub currency_code: Option<Option<CurrencyCode>>,
    /// New budget limit; `Some(None)` makes it unlimited.
    pub budget_limit: Option<Option<i64>>,
    /// New start.
    pub starts_at: Option<Option<i64>>,
    /// New end.
    pub ends_at: Option<Option<i64>>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Validates the campaign budget fields.
pub(crate) fn validate_campaign_budget(
    budget_type: CampaignBudgetType,
    currency_code: Option<&CurrencyCode>,
    budget_limit: Option<i64>,
) -> Result<(), AppError> {
    if budget_limit.is_some_and(|limit| limit <= 0) {
        return Err(AppError::validation("campaign budget must be positive"));
    }
    match budget_type {
        CampaignBudgetType::Spend => {
            if currency_code.is_none() {
                return Err(AppError::validation(
                    "spend campaign budgets require a currency_code",
                ));
            }
            if budget_limit.is_none() {
                return Err(AppError::validation(
                    "spend campaign budgets require a budget_limit",
                ));
            }
        }
        CampaignBudgetType::Usage => {
            if currency_code.is_some() {
                return Err(AppError::validation(
                    "usage campaign budgets must not set currency_code",
                ));
            }
        }
    }
    Ok(())
}
