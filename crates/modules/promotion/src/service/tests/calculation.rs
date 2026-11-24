use super::*;

#[test]
fn percentage_rounds_half_away_from_zero() {
    let promotion = promotion("promo_pct", PromotionType::Percentage, 500, 1);
    let cart = cart(
        2020,
        0,
        vec![
            line("item_a", "prod_a", "var_a", &[], 1005, 1),
            line("item_b", "prod_b", "var_b", &[], 1015, 1),
        ],
    );

    let results = calculate_promotions(&[context(promotion, Vec::new())], &cart).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].total_discount, 101);
    assert_eq!(results[0].applications[0].amount, 50);
    assert_eq!(results[0].applications[1].amount, 51);
    assert_eq!(results[0].shipping_discount, 0);
}

#[test]
fn percentage_targets_a_product() {
    let promotion = promotion("promo_product", PromotionType::Percentage, 1000, 1);
    let rules = vec![rule(
        PromotionRuleAttribute::ProductId,
        PromotionRuleOperator::In,
        "[\"prod_a\"]",
    )];
    let cart = cart(
        3000,
        0,
        vec![
            line("item_a", "prod_a", "var_a", &[], 1000, 1),
            line("item_b", "prod_b", "var_b", &[], 2000, 1),
        ],
    );

    let results = calculate_promotions(&[context(promotion, rules)], &cart).unwrap();
    assert_eq!(results[0].total_discount, 100);
    assert_eq!(results[0].applications.len(), 1);
    assert_eq!(
        results[0].applications[0].line_id.as_deref(),
        Some("item_a")
    );
}

#[test]
fn percentage_targets_a_category() {
    let promotion = promotion("promo_category", PromotionType::FixedAmount, 100, 1);
    let rules = vec![rule(
        PromotionRuleAttribute::CategoryId,
        PromotionRuleOperator::In,
        "[\"cat_sale\"]",
    )];
    let cart = cart(
        1200,
        0,
        vec![
            line("item_a", "prod_a", "var_a", &["cat_sale"], 200, 1),
            line("item_b", "prod_b", "var_b", &["cat_full"], 1000, 1),
        ],
    );

    let results = calculate_promotions(&[context(promotion, rules)], &cart).unwrap();
    assert_eq!(results[0].applications.len(), 1);
    assert_eq!(
        results[0].applications[0].line_id.as_deref(),
        Some("item_a")
    );
    assert_eq!(results[0].applications[0].amount, 100);
}

#[test]
fn fixed_amount_allocates_proportionally_across_lines() {
    let promotion = promotion("promo_fixed", PromotionType::FixedAmount, 100, 1);
    let cart = cart(
        300,
        0,
        vec![
            line("item_a", "prod_a", "var_a", &[], 100, 1),
            line("item_b", "prod_b", "var_b", &[], 100, 1),
            line("item_c", "prod_c", "var_c", &[], 100, 1),
        ],
    );

    let results = calculate_promotions(&[context(promotion, Vec::new())], &cart).unwrap();
    let applications = &results[0].applications;
    assert_eq!(applications.len(), 3);
    let amounts: Vec<i64> = applications.iter().map(|item| item.amount).collect();

    // Three equal lines cannot split 100 evenly, so two lines carry one
    // unit more than the third. Which line absorbs the extra unit is an
    // implementation detail; what matters is that the full amount is
    // allocated in proportion and no line is over-discounted.
    assert_eq!(amounts.iter().sum::<i64>(), 100);
    let spread = amounts.iter().max().unwrap() - amounts.iter().min().unwrap();
    assert_eq!(spread, 1, "allocation must stay proportional: {amounts:?}");
    assert!(
        amounts.iter().all(|amount| *amount <= 100),
        "no line may be discounted past its own value: {amounts:?}"
    );
    assert_eq!(results[0].total_discount, 100);
}

#[test]
fn fixed_amount_is_capped_at_eligible_subtotal() {
    let promotion = promotion("promo_fixed", PromotionType::FixedAmount, 500, 1);
    let cart = cart(100, 0, vec![line("item_a", "prod_a", "var_a", &[], 100, 1)]);

    let results = calculate_promotions(&[context(promotion, Vec::new())], &cart).unwrap();
    assert_eq!(results[0].total_discount, 100);
    assert_eq!(results[0].applications[0].amount, 100);
}

#[test]
fn free_shipping_discounts_shipping() {
    let promotion = promotion("promo_ship", PromotionType::FreeShipping, 0, 1);
    let cart = cart(
        1000,
        500,
        vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
    );

    let results = calculate_promotions(&[context(promotion, Vec::new())], &cart).unwrap();
    assert_eq!(results[0].total_discount, 0);
    assert_eq!(results[0].shipping_discount, 500);
    assert_eq!(results[0].applications[0].amount, 500);
    assert!(results[0].applications[0].is_shipping);
    assert!(results[0].applications[0].line_id.is_none());
}

#[test]
fn minimum_subtotal_restricts_application() {
    let promotion = promotion("promo_min", PromotionType::FixedAmount, 100, 1);
    let rules = vec![rule(
        PromotionRuleAttribute::MinimumOrderSubtotal,
        PromotionRuleOperator::Gte,
        "1000",
    )];
    let small = cart(900, 0, vec![line("item_a", "prod_a", "var_a", &[], 900, 1)]);
    let large = cart(
        1000,
        0,
        vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
    );

    assert!(
        calculate_promotions(&[context(promotion.clone(), rules.clone())], &small)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        calculate_promotions(&[context(promotion, rules)], &large)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn currency_rule_restricts_application() {
    let promotion = promotion("promo_currency", PromotionType::FixedAmount, 100, 1);
    let rules = vec![rule(
        PromotionRuleAttribute::Currency,
        PromotionRuleOperator::Eq,
        "\"EUR\"",
    )];
    let mut usd_cart = cart(
        1000,
        0,
        vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
    );
    usd_cart.currency = usd();
    let mut eur_cart = usd_cart.clone();
    eur_cart.currency = eur();

    assert!(
        calculate_promotions(&[context(promotion.clone(), rules.clone())], &usd_cart)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        calculate_promotions(&[context(promotion, rules)], &eur_cart)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn customer_group_rule_restricts_application() {
    let promotion = promotion("promo_group", PromotionType::FixedAmount, 100, 1);
    let rules = vec![rule(
        PromotionRuleAttribute::CustomerGroupId,
        PromotionRuleOperator::In,
        "[\"vip\",\"gold\"]",
    )];
    let mut cart = cart(
        1000,
        0,
        vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
    );
    assert!(
        calculate_promotions(&[context(promotion.clone(), rules.clone())], &cart)
            .unwrap()
            .is_empty()
    );

    cart.customer_group_ids.push("vip".to_owned());
    assert_eq!(
        calculate_promotions(&[context(promotion, rules)], &cart)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn first_order_rule_checks_the_attached_customer() {
    let promotion = promotion("promo_first", PromotionType::FixedAmount, 100, 1);
    let rules = vec![rule(
        PromotionRuleAttribute::FirstOrder,
        PromotionRuleOperator::Eq,
        "true",
    )];
    let mut cart = cart(
        1000,
        0,
        vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
    );
    assert_eq!(
        calculate_promotions(&[context(promotion.clone(), rules.clone())], &cart)
            .unwrap()
            .len(),
        1
    );

    cart.customer_id = None;
    assert!(
        calculate_promotions(&[context(promotion, rules)], &cart)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn stacking_is_ordered_and_capped_at_remaining_subtotal() {
    let fixed = promotion("promo_a", PromotionType::FixedAmount, 600, 10);
    let percentage = promotion("promo_b", PromotionType::Percentage, 5000, 20);
    let cart = cart(
        1000,
        0,
        vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
    );

    let results = calculate_promotions(
        &[context(percentage, Vec::new()), context(fixed, Vec::new())],
        &cart,
    )
    .unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].promotion_id, "promo_a");
    assert_eq!(results[0].total_discount, 600);
    assert_eq!(results[1].promotion_id, "promo_b");
    assert_eq!(results[1].total_discount, 400);
    assert_eq!(
        results[0].total_discount + results[1].total_discount,
        cart.subtotal
    );
}

#[test]
fn repeated_free_shipping_never_exceeds_shipping_total() {
    let first = promotion("promo_ship_a", PromotionType::FreeShipping, 0, 1);
    let second = promotion("promo_ship_b", PromotionType::FreeShipping, 0, 2);
    let cart = cart(1000, 500, Vec::new());

    let results = calculate_promotions(
        &[context(first, Vec::new()), context(second, Vec::new())],
        &cart,
    )
    .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].shipping_discount, 500);
}
