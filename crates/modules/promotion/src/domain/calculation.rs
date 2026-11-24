//! Deterministic discount calculation.

use super::*;

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

/// Applies promotions in priority order against the remaining cart value.
///
/// Promotions are ordered by `priority` descending, then `created_at`, then
/// `id`. Every priority defaults to 0, so a store that never configures one
/// keeps the historic `created_at, id` order and its discount allocation is
/// unchanged; raising a promotion's priority is the only way to move it.
///
/// Each promotion sees the subtotal and shipping left after previous
/// promotions, so no combination of promotions can discount a cart below zero.
/// Promotions that do not match or produce no discount are omitted.
///
/// A promotion with `is_stackable = false` is exclusive: once it produces a
/// discount it owns the batch and the promotions ordered after it are skipped.
/// A non-stackable promotion that produces no discount — it does not match, or
/// there is nothing left to discount — does not affect the promotions after it.
pub fn calculate_promotions(
    contexts: &[PromotionContext],
    cart: &PromotionCart,
) -> Result<Vec<PromotionResult>, AppError> {
    let mut ordered: Vec<&PromotionContext> = contexts.iter().collect();
    // Every existing row defaults to priority 0, so this key collapses to the
    // historic `created_at, id` order unless a priority is configured: the
    // comparator returns `Equal` for equal priorities and falls through.
    ordered.sort_by(|left, right| {
        right
            .promotion
            .priority
            .cmp(&left.promotion.priority)
            .then_with(|| left.promotion.created_at.cmp(&right.promotion.created_at))
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
            // Exclusivity is earned by discounting: a non-stackable promotion
            // that produced nothing stays out of the way.
            let exclusive = !context.promotion.is_stackable;
            results.push(result);
            if exclusive {
                break;
            }
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
        // Free shipping is inherently a shipping discount and its target is
        // validated to the default, so it keeps its own path.
        PromotionType::FreeShipping => free_shipping_result(context, cart, remaining_shipping),
        PromotionType::Percentage => match context.promotion.target {
            PromotionTarget::Items => percentage_result(context, cart, &lines, remaining_subtotal),
            PromotionTarget::Order => {
                order_percentage_result(context, cart, remaining_subtotal, remaining_shipping)
            }
            PromotionTarget::Shipping => {
                shipping_percentage_result(context, cart, remaining_shipping)
            }
        },
        PromotionType::FixedAmount => match context.promotion.target {
            PromotionTarget::Items => fixed_amount_result(context, &lines, remaining_subtotal),
            PromotionTarget::Order => {
                order_fixed_amount_result(context, cart, remaining_subtotal, remaining_shipping)
            }
            PromotionTarget::Shipping => {
                shipping_fixed_amount_result(context, cart, remaining_shipping)
            }
        },
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

    // Split the discount by value. Handing the whole integer remainder to the
    // last line — as this once did — can push that line's discount past its own
    // value, which makes the line total negative and aborts checkout after the
    // customer's payment was captured.
    let parts: Vec<i64> = eligible.iter().map(|(_, subtotal)| *subtotal).collect();
    let shares = allocate_proportionally(&parts, eligible_subtotal, discount)?;

    let mut applications = Vec::new();
    for ((line, _), share) in eligible.iter().zip(shares) {
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

fn order_percentage_result(
    context: &PromotionContext,
    cart: &PromotionCart,
    remaining_subtotal: i64,
    remaining_shipping: i64,
) -> Result<Option<PromotionResult>, AppError> {
    // The whole order is one base: the item subtotal plus shipping. The
    // discount is capped at what previous promotions left of both, so the
    // sequence still cannot discount the cart below zero.
    let base = cart
        .subtotal
        .max(0)
        .saturating_add(cart.shipping_total.max(0));
    let budget = remaining_subtotal
        .max(0)
        .saturating_add(remaining_shipping.max(0));
    let Some(amount) = whole_target_percentage_amount(context, cart, base, budget)? else {
        return Ok(None);
    };
    let applications = vec![application(
        context,
        None,
        amount,
        false,
        format!(
            "Order percentage discount ({})",
            percentage_label(context.promotion.value)
        ),
    )];
    let (item_discount, shipping_discount) =
        order_discount_split(amount, remaining_subtotal, remaining_shipping);
    Ok(Some(result(
        context,
        applications,
        item_discount,
        shipping_discount,
    )))
}

/// Splits an order-target amount across the two legs of the cart it consumes.
///
/// An order discount is recorded as one cart-level application, but it consumes
/// the remaining order value as a whole: the item leg first, because that is the
/// leg the cart charges a cart-level discount against, then shipping. Booking
/// both legs also keeps the sequence honest — a later promotion only sees what
/// this one really left.
fn order_discount_split(
    amount: i64,
    remaining_subtotal: i64,
    remaining_shipping: i64,
) -> (i64, i64) {
    debug_assert!(amount <= remaining_subtotal.saturating_add(remaining_shipping));
    let item_discount = amount.min(remaining_subtotal);
    (item_discount, amount - item_discount)
}

fn shipping_percentage_result(
    context: &PromotionContext,
    cart: &PromotionCart,
    remaining_shipping: i64,
) -> Result<Option<PromotionResult>, AppError> {
    let base = cart.shipping_total.max(0);
    let budget = remaining_shipping.max(0);
    let Some(amount) = whole_target_percentage_amount(context, cart, base, budget)? else {
        return Ok(None);
    };
    let applications = vec![application(
        context,
        None,
        amount,
        true,
        format!(
            "Shipping percentage discount ({})",
            percentage_label(context.promotion.value)
        ),
    )];
    Ok(Some(result(context, applications, 0, amount)))
}

/// Validates a percentage and rounds it against `base`, capped at the value the
/// previous promotions left.
fn whole_target_percentage_amount(
    context: &PromotionContext,
    cart: &PromotionCart,
    base: i64,
    budget: i64,
) -> Result<Option<i64>, AppError> {
    let basis_points = context.promotion.value;
    if !(0..=10_000).contains(&basis_points) {
        return Err(AppError::validation(format!(
            "percentage promotion value must be between 0 and 10000 basis points: {basis_points}"
        )));
    }
    if basis_points == 0 || base <= 0 || budget <= 0 {
        return Ok(None);
    }
    let discount = Money::new(base, cart.currency.clone())
        .apply_basis_points(basis_points, Rounding::HalfAwayFromZero)?
        .amount_minor();
    let amount = discount.min(budget);
    if amount <= 0 {
        return Ok(None);
    }
    Ok(Some(amount))
}

fn order_fixed_amount_result(
    context: &PromotionContext,
    cart: &PromotionCart,
    remaining_subtotal: i64,
    remaining_shipping: i64,
) -> Result<Option<PromotionResult>, AppError> {
    let value = context.promotion.value;
    if value < 0 {
        return Err(AppError::validation(
            "fixed amount promotion value must not be negative",
        ));
    }
    let base = cart
        .subtotal
        .max(0)
        .saturating_add(cart.shipping_total.max(0));
    let budget = remaining_subtotal
        .max(0)
        .saturating_add(remaining_shipping.max(0));
    let discount = value.min(base).min(budget);
    if discount <= 0 {
        return Ok(None);
    }
    let applications = vec![application(
        context,
        None,
        discount,
        false,
        "Order fixed amount discount".to_owned(),
    )];
    let (item_discount, shipping_discount) =
        order_discount_split(discount, remaining_subtotal, remaining_shipping);
    Ok(Some(result(
        context,
        applications,
        item_discount,
        shipping_discount,
    )))
}

fn shipping_fixed_amount_result(
    context: &PromotionContext,
    cart: &PromotionCart,
    remaining_shipping: i64,
) -> Result<Option<PromotionResult>, AppError> {
    let value = context.promotion.value;
    if value < 0 {
        return Err(AppError::validation(
            "fixed amount promotion value must not be negative",
        ));
    }
    let discount = value
        .min(cart.shipping_total.max(0))
        .min(remaining_shipping.max(0));
    if discount <= 0 {
        return Ok(None);
    }
    let applications = vec![application(
        context,
        None,
        discount,
        true,
        "Shipping fixed amount discount".to_owned(),
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

/// A per-customer limit, when set, must allow at least one redemption: zero or
/// a negative value would make the promotion unusable rather than capped.
pub(crate) fn validate_per_customer_limit(per_customer_limit: Option<i64>) -> Result<(), AppError> {
    if per_customer_limit.is_some_and(|limit| limit <= 0) {
        return Err(AppError::validation(
            "per_customer_limit must be a positive number of redemptions",
        ));
    }
    Ok(())
}

// The arguments are the promotion fields the service has already resolved, one
// per column; bundling them into a struct would only move the list.
#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_promotion_fields(
    title: &str,
    promotion_type: PromotionType,
    target: PromotionTarget,
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
        PromotionType::FreeShipping if target != PromotionTarget::Items => {
            return Err(AppError::validation(
                "free shipping promotions already discount shipping and cannot carry another \
                 target",
            ));
        }
        _ => {}
    }
    validate_metadata(metadata)
}
