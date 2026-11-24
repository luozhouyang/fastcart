use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
use fastcart_kernel::{AppError, CurrencyCode, Metadata, Pagination, StoreId};

use super::PromotionService;
use crate::domain::{
    CampaignBudgetType, CreateCampaignInput, CreateCouponInput, CreatePromotionInput, Promotion,
    PromotionCart, PromotionContext, PromotionFilter, PromotionLine, PromotionRule,
    PromotionRuleAttribute, PromotionRuleInput, PromotionRuleOperator, PromotionStatus,
    PromotionTarget, PromotionType, UpdateCampaignInput, UpdatePromotionInput,
    calculate_promotions,
};

async fn service() -> PromotionService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    PromotionService::new(db)
}

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn eur() -> CurrencyCode {
    CurrencyCode::new("EUR").unwrap()
}

fn line(
    line_id: &str,
    product_id: &str,
    variant_id: &str,
    categories: &[&str],
    unit_price: i64,
    quantity: i64,
) -> PromotionLine {
    PromotionLine {
        line_id: line_id.to_owned(),
        product_id: product_id.to_owned(),
        variant_id: variant_id.to_owned(),
        category_ids: categories.iter().map(|value| (*value).to_owned()).collect(),
        unit_price,
        quantity,
        is_shipping: false,
    }
}

fn cart(subtotal: i64, shipping_total: i64, lines: Vec<PromotionLine>) -> PromotionCart {
    PromotionCart {
        currency: usd(),
        subtotal,
        shipping_total,
        customer_id: Some("cust_1".to_owned()),
        customer_group_ids: Vec::new(),
        lines,
        at: 100,
    }
}

fn promotion(id: &str, promotion_type: PromotionType, value: i64, created_at: i64) -> Promotion {
    Promotion {
        id: id.to_owned(),
        code: Some(id.to_ascii_uppercase()),
        title: id.to_owned(),
        promotion_type,
        target: PromotionTarget::Items,
        value,
        status: PromotionStatus::Active,
        is_automatic: false,
        starts_at: None,
        ends_at: None,
        usage_limit: None,
        usage_count: 0,
        per_customer_limit: None,
        priority: 0,
        is_stackable: true,
        metadata: Metadata::empty(),
        rules: Vec::new(),
        created_at,
        updated_at: created_at,
    }
}

fn rule(
    attribute: PromotionRuleAttribute,
    operator: PromotionRuleOperator,
    value: &str,
) -> PromotionRule {
    PromotionRule {
        id: "rule_1".to_owned(),
        promotion_id: "promo_1".to_owned(),
        attribute,
        operator,
        value: value.to_owned(),
        created_at: 0,
    }
}

fn context(promotion: Promotion, rules: Vec<PromotionRule>) -> PromotionContext {
    PromotionContext { promotion, rules }
}

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

#[tokio::test]
async fn creates_updates_lists_and_deletes_promotions() {
    let service = service().await;
    let input = CreatePromotionInput::new("Ten off", PromotionType::FixedAmount, 1000)
        .active()
        .with_code("tenoff")
        .with_rule(PromotionRuleInput::new(
            PromotionRuleAttribute::MinimumOrderSubtotal,
            PromotionRuleOperator::Gte,
            "5000",
        ));
    let created = service.create_promotion(input).await.unwrap();
    assert_eq!(created.code.as_deref(), Some("TENOFF"));
    assert_eq!(created.rules.len(), 1);
    assert_eq!(created.usage_count, 0);

    let fetched = service.get_promotion(&created.id).await.unwrap();
    assert_eq!(fetched, created);

    service
        .create_promotion(CreatePromotionInput::new(
            "Percentage",
            PromotionType::Percentage,
            1000,
        ))
        .await
        .unwrap();
    let page = service
        .list_promotions(
            PromotionFilter::default(),
            Pagination {
                limit: 1,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 1);

    let updated = service
        .update_promotion(
            &created.id,
            UpdatePromotionInput {
                title: Some("Twenty off".to_owned()),
                value: Some(2000),
                rules: Some(Vec::new()),
                ..UpdatePromotionInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "Twenty off");
    assert_eq!(updated.value, 2000);
    assert!(updated.rules.is_empty());
    assert_eq!(updated.created_at, created.created_at);

    service.delete_promotion(&created.id).await.unwrap();
    assert!(matches!(
        service.get_promotion(&created.id).await.unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[tokio::test]
async fn rejects_duplicate_promotion_codes() {
    let service = service().await;
    service
        .create_promotion(
            CreatePromotionInput::new("First", PromotionType::FixedAmount, 100).with_code("SAVE"),
        )
        .await
        .unwrap();
    let error = service
        .create_promotion(
            CreatePromotionInput::new("Second", PromotionType::FixedAmount, 100).with_code("save"),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn validates_promotion_configuration() {
    let service = service().await;

    let percentage_error = service
        .create_promotion(CreatePromotionInput::new(
            "Too much",
            PromotionType::Percentage,
            10_001,
        ))
        .await
        .unwrap_err();
    assert!(matches!(percentage_error, AppError::Validation(_)));

    let negative_error = service
        .create_promotion(CreatePromotionInput::new(
            "Negative",
            PromotionType::FixedAmount,
            -1,
        ))
        .await
        .unwrap_err();
    assert!(matches!(negative_error, AppError::Validation(_)));

    let shipping_error = service
        .create_promotion(CreatePromotionInput::new(
            "Shipping value",
            PromotionType::FreeShipping,
            5,
        ))
        .await
        .unwrap_err();
    assert!(matches!(shipping_error, AppError::Validation(_)));

    let window_error = service
        .create_promotion(
            CreatePromotionInput::new("Window", PromotionType::FixedAmount, 100)
                .with_window(Some(200), Some(100)),
        )
        .await
        .unwrap_err();
    assert!(matches!(window_error, AppError::Validation(_)));
}

#[tokio::test]
async fn manages_coupons_with_filters() {
    let service = service().await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Coupon promo", PromotionType::FixedAmount, 500).active(),
        )
        .await
        .unwrap();
    let coupon = service
        .create_coupon(CreateCouponInput::new("save10", &promotion.id))
        .await
        .unwrap();
    assert_eq!(coupon.code, "SAVE10");

    let fetched = service.get_coupon(&coupon.id).await.unwrap();
    assert_eq!(fetched, coupon);

    let page = service
        .list_coupons(
            crate::domain::CouponFilter {
                promotion_id: Some(promotion.id.clone()),
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(page.count, 1);

    let updated = service
        .update_coupon(
            &coupon.id,
            crate::domain::UpdateCouponInput {
                usage_limit: Some(Some(2)),
                ..crate::domain::UpdateCouponInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.usage_limit, Some(2));

    service.delete_coupon(&coupon.id).await.unwrap();
    assert!(matches!(
        service.get_coupon(&coupon.id).await.unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[tokio::test]
async fn rejects_duplicate_coupon_codes() {
    let service = service().await;
    let promotion = service
        .create_promotion(CreatePromotionInput::new(
            "Coupon promo",
            PromotionType::FixedAmount,
            500,
        ))
        .await
        .unwrap();
    service
        .create_coupon(CreateCouponInput::new("SAVE", &promotion.id))
        .await
        .unwrap();
    let error = service
        .create_coupon(CreateCouponInput::new("save", &promotion.id))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn validates_coupons_against_windows_limits_and_promotion_state() {
    let service = service().await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Coupon promo", PromotionType::FixedAmount, 500).active(),
        )
        .await
        .unwrap();
    let coupon = service
        .create_coupon(
            CreateCouponInput::new("SAVE", &promotion.id).with_window(Some(0), Some(200)),
        )
        .await
        .unwrap();

    assert_eq!(
        service.validate_coupon(&coupon.code, 100).await.unwrap().id,
        coupon.id
    );
    assert!(matches!(
        service
            .validate_coupon(&coupon.code, 200)
            .await
            .unwrap_err(),
        AppError::Conflict(_)
    ));
    assert!(matches!(
        service.validate_coupon("MISSING", 100).await.unwrap_err(),
        AppError::NotFound(_)
    ));

    let draft = service
        .create_promotion(CreatePromotionInput::new(
            "Draft",
            PromotionType::FixedAmount,
            500,
        ))
        .await
        .unwrap();
    service
        .create_coupon(CreateCouponInput::new("DRAFT", &draft.id))
        .await
        .unwrap();
    assert!(matches!(
        service.validate_coupon("DRAFT", 100).await.unwrap_err(),
        AppError::Conflict(_)
    ));
}

#[tokio::test]
async fn records_usage_atomically() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PromotionService::new(db.clone());
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Usage promo", PromotionType::FixedAmount, 500)
                .active()
                .with_usage_limit(5),
        )
        .await
        .unwrap();
    let coupon = service
        .create_coupon(CreateCouponInput::new("USE", &promotion.id))
        .await
        .unwrap();

    let usage = service
        .record_usage(
            &promotion.id,
            Some(&coupon.id),
            Some("cust_1"),
            Some("order_1"),
            0,
            None,
        )
        .await
        .unwrap();
    assert_eq!(usage.promotion_id, promotion.id);
    assert_eq!(usage.coupon_id.as_deref(), Some(coupon.id.as_str()));
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 1);
    assert_eq!(service.coupon_usage_count(&coupon.id).await.unwrap(), 1);

    let recorded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM promotion_usage")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(recorded, 1);
}

#[tokio::test]
async fn record_usage_is_idempotent_per_order() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PromotionService::new(db.clone());
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Idempotent promo", PromotionType::FixedAmount, 500)
                .active()
                .with_usage_limit(5),
        )
        .await
        .unwrap();
    let coupon = service
        .create_coupon(CreateCouponInput::new("ONCEPERORDER", &promotion.id))
        .await
        .unwrap();

    let first = service
        .record_usage(
            &promotion.id,
            Some(&coupon.id),
            Some("cust_1"),
            Some("order_1"),
            0,
            None,
        )
        .await
        .unwrap();
    let repeated = service
        .record_usage(
            &promotion.id,
            Some(&coupon.id),
            Some("cust_1"),
            Some("order_1"),
            250,
            None,
        )
        .await
        .unwrap();

    assert_eq!(first.id, repeated.id);
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 1);
    assert_eq!(service.coupon_usage_count(&coupon.id).await.unwrap(), 1);
    let recorded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM promotion_usage")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(recorded, 1);
}

#[tokio::test]
async fn duplicate_codes_are_reported_as_conflicts() {
    let service = service().await;
    let first = service
        .create_promotion(
            CreatePromotionInput::new("First", PromotionType::FixedAmount, 100).with_code("SHARED"),
        )
        .await
        .unwrap();
    let second = service
        .create_promotion(
            CreatePromotionInput::new("Second", PromotionType::FixedAmount, 100).with_code("OTHER"),
        )
        .await
        .unwrap();
    let error = service
        .update_promotion(
            &second.id,
            UpdatePromotionInput {
                code: Some(Some("shared".to_owned())),
                ..UpdatePromotionInput::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let coupon = service
        .create_coupon(CreateCouponInput::new("COUPON-ONE", &first.id))
        .await
        .unwrap();
    service
        .create_coupon(CreateCouponInput::new("COUPON-TWO", &first.id))
        .await
        .unwrap();
    let error = service
        .update_coupon(
            &coupon.id,
            crate::domain::UpdateCouponInput {
                code: Some("coupon-two".to_owned()),
                ..crate::domain::UpdateCouponInput::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn usage_limits_are_enforced() {
    let service = service().await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Limited", PromotionType::FixedAmount, 500)
                .active()
                .with_usage_limit(1),
        )
        .await
        .unwrap();
    service
        .record_usage(&promotion.id, None, None, None, 0, None)
        .await
        .unwrap();
    assert!(matches!(
        service
            .record_usage(&promotion.id, None, None, None, 0, None)
            .await
            .unwrap_err(),
        AppError::Conflict(_)
    ));

    let coupon_promotion = service
        .create_promotion(
            CreatePromotionInput::new("Coupon limited", PromotionType::FixedAmount, 500).active(),
        )
        .await
        .unwrap();
    let coupon = service
        .create_coupon(CreateCouponInput::new("ONCE", &coupon_promotion.id).with_usage_limit(1))
        .await
        .unwrap();
    service
        .record_usage(&coupon_promotion.id, Some(&coupon.id), None, None, 0, None)
        .await
        .unwrap();
    assert!(matches!(
        service
            .record_usage(&coupon_promotion.id, Some(&coupon.id), None, None, 0, None)
            .await
            .unwrap_err(),
        AppError::Conflict(_)
    ));
}

#[tokio::test]
async fn lists_eligible_automatic_promotions() {
    let service = service().await;
    let eligible = service
        .create_promotion(
            CreatePromotionInput::new("Automatic", PromotionType::FixedAmount, 100)
                .active()
                .automatic()
                .with_window(Some(0), Some(1000)),
        )
        .await
        .unwrap();
    service
        .create_promotion(
            CreatePromotionInput::new("Draft automatic", PromotionType::FixedAmount, 100)
                .automatic(),
        )
        .await
        .unwrap();
    service
        .create_promotion(
            CreatePromotionInput::new("Manual", PromotionType::FixedAmount, 100).active(),
        )
        .await
        .unwrap();
    service
        .create_promotion(
            CreatePromotionInput::new("Expired", PromotionType::FixedAmount, 100)
                .active()
                .automatic()
                .with_window(Some(0), Some(50)),
        )
        .await
        .unwrap();

    let automatic = service.list_automatic_promotions(100).await.unwrap();
    assert_eq!(automatic.len(), 1);
    assert_eq!(automatic[0].id, eligible.id);
    assert_eq!(automatic[0].rules.len(), 0);
}

#[tokio::test]
async fn service_calculation_filters_inactive_promotions() {
    let service = service().await;
    let active = promotion("promo_active", PromotionType::FixedAmount, 100, 1);
    let mut draft = promotion("promo_draft", PromotionType::FixedAmount, 200, 2);
    draft.status = PromotionStatus::Draft;
    let mut expired = promotion("promo_expired", PromotionType::FixedAmount, 300, 3);
    expired.ends_at = Some(50);
    let cart = cart(
        1000,
        0,
        vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
    );

    let results = service
        .calculate(
            &[
                context(active, Vec::new()),
                context(draft, Vec::new()),
                context(expired, Vec::new()),
            ],
            &cart,
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].promotion_id, "promo_active");
}

#[tokio::test]
async fn campaign_mutations_append_audit_entries_atomically() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PromotionService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));

    let campaign = service
        .create_campaign(CreateCampaignInput {
            name: "Launch".to_owned(),
            ..CreateCampaignInput::default()
        })
        .await
        .unwrap();
    service
        .update_campaign(
            &campaign.id,
            UpdateCampaignInput {
                name: Some("Launch 2".to_owned()),
                ..UpdateCampaignInput::default()
            },
        )
        .await
        .unwrap();
    service.delete_campaign(&campaign.id).await.unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec!["campaign.created", "campaign.updated", "campaign.deleted"]
    );
    assert!(audits.iter().all(|audit| audit.processed_at.is_none()));
    assert!(
        audits
            .iter()
            .all(|audit| audit.actor_id.as_deref() == Some("user_1"))
    );

    // A failed deletion is a no-op and must not append an audit row.
    assert!(service.delete_campaign(&campaign.id).await.is_err());
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 3);
}

#[tokio::test]
async fn campaign_budgets_are_consumed_atomically() {
    let service = service().await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Budgeted", PromotionType::FixedAmount, 100).active(),
        )
        .await
        .unwrap();
    let campaign = service
        .create_campaign(CreateCampaignInput {
            name: "Launch".to_owned(),
            budget_limit: Some(1),
            ..CreateCampaignInput::default()
        })
        .await
        .unwrap();
    service
        .link_promotion(&campaign.id, &promotion.id)
        .await
        .unwrap();
    assert_eq!(
        service
            .list_campaign_promotions(&campaign.id)
            .await
            .unwrap(),
        vec![promotion.id.clone()]
    );

    service
        .record_usage(&promotion.id, None, None, Some("order_1"), 0, None)
        .await
        .unwrap();
    let exhausted = service
        .record_usage(&promotion.id, None, None, Some("order_2"), 0, None)
        .await
        .unwrap_err();
    assert!(matches!(exhausted, AppError::Conflict(_)));
    let stored = service.get_campaign(&campaign.id).await.unwrap();
    assert_eq!(stored.budget_used, 1);
}

#[tokio::test]
async fn spend_campaign_budgets_track_monetary_amounts() {
    let service = service().await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Spend budget", PromotionType::Percentage, 1000).active(),
        )
        .await
        .unwrap();
    let campaign = service
        .create_campaign(CreateCampaignInput {
            name: "Spend".to_owned(),
            budget_type: CampaignBudgetType::Spend,
            currency_code: Some(usd()),
            budget_limit: Some(1_000),
            ..CreateCampaignInput::default()
        })
        .await
        .unwrap();
    service
        .link_promotion(&campaign.id, &promotion.id)
        .await
        .unwrap();

    service
        .record_usage(&promotion.id, None, None, Some("order_1"), 600, Some("USD"))
        .await
        .unwrap();
    let stored = service.get_campaign(&campaign.id).await.unwrap();
    assert_eq!(stored.budget_used, 600);
    assert_eq!(stored.budget_spent().unwrap().amount_minor(), 600);

    let wrong_currency = service
        .record_usage(&promotion.id, None, None, Some("order_2"), 100, Some("EUR"))
        .await
        .unwrap_err();
    assert!(matches!(wrong_currency, AppError::Conflict(_)));

    let exhausted = service
        .record_usage(&promotion.id, None, None, Some("order_3"), 500, Some("USD"))
        .await
        .unwrap_err();
    assert!(matches!(exhausted, AppError::Conflict(_)));
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        600
    );

    service
        .record_usage(&promotion.id, None, None, Some("order_4"), 400, Some("USD"))
        .await
        .unwrap();
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        1_000
    );
}

/// Creates a store with `count` automatic promotions that each carry two
/// rules and returns the service over it.
///
/// Rule reads are counted on the service instance itself
/// ([`PromotionService::rule_reads`]), so a test that asserts on them
/// cannot be disturbed by the other tests running in this binary: no
/// process-global subscriber, no thread attribution, no shared state.
async fn automatic_promotions(count: usize) -> (StoreDatabase, PromotionService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PromotionService::new(db.clone());
    for index in 0..count {
        service
            .create_promotion(
                CreatePromotionInput::new(
                    format!("Automatic {index}"),
                    PromotionType::Percentage,
                    1000,
                )
                .active()
                .automatic()
                .with_rule(PromotionRuleInput::new(
                    PromotionRuleAttribute::MinimumOrderSubtotal,
                    PromotionRuleOperator::Gte,
                    "5000",
                ))
                .with_rule(PromotionRuleInput::new(
                    PromotionRuleAttribute::ProductId,
                    PromotionRuleOperator::In,
                    r#"["prod_1"]"#,
                )),
            )
            .await
            .unwrap();
    }
    (db, service)
}

/// Lists `count` automatic promotions and returns how many rule reads the
/// listing issued.
async fn automatic_listing_rule_reads(count: usize) -> usize {
    let (_db, service) = automatic_promotions(count).await;
    let before = service.rule_reads();
    let listed = service.list_automatic_promotions(100).await.unwrap();
    assert_eq!(listed.len(), count);
    service.rule_reads() - before
}

#[tokio::test]
async fn listing_automatic_promotions_keeps_rules_grouped_and_ordered() {
    let (_db, service) = automatic_promotions(5).await;

    let listed = service.list_automatic_promotions(100).await.unwrap();
    assert_eq!(listed.len(), 5);
    let mut ids = listed
        .iter()
        .map(|promotion| promotion.id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 5);

    for promotion in &listed {
        assert_eq!(promotion.rules.len(), 2);
        // The batched listing must return exactly what the single load
        // returns for each promotion.
        let single = service.get_promotion(&promotion.id).await.unwrap();
        assert_eq!(promotion, &single);
    }
}

#[tokio::test]
async fn listing_promotions_keeps_rules_grouped_and_ordered() {
    let (_db, service) = automatic_promotions(5).await;
    let manual = service
        .create_promotion(
            CreatePromotionInput::new("Manual", PromotionType::FixedAmount, 100)
                .active()
                .with_rule(PromotionRuleInput::new(
                    PromotionRuleAttribute::Currency,
                    PromotionRuleOperator::Eq,
                    "USD",
                )),
        )
        .await
        .unwrap();

    let page = service
        .list_promotions(
            PromotionFilter::default(),
            Pagination {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.count, 6);
    assert_eq!(page.data.len(), 6);
    for promotion in &page.data {
        let single = service.get_promotion(&promotion.id).await.unwrap();
        assert_eq!(promotion, &single);
        assert_eq!(
            promotion.rules.len(),
            if promotion.id == manual.id { 1 } else { 2 }
        );
    }
}

#[tokio::test]
async fn listing_automatic_promotions_issues_constant_rule_queries() {
    let small = automatic_listing_rule_reads(2).await;
    let large = automatic_listing_rule_reads(12).await;
    assert_eq!(
        large, small,
        "the rule query count must not grow with the number of promotions"
    );
    assert_eq!(large, 1, "all rules must be loaded in a single query");
}

#[tokio::test]
async fn listing_promotions_issues_constant_rule_queries() {
    let small = async {
        let (_db, service) = automatic_promotions(3).await;
        let before = service.rule_reads();
        let page = service
            .list_promotions(
                PromotionFilter::default(),
                Pagination {
                    limit: 50,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.data.len(), 3);
        service.rule_reads() - before
    }
    .await;
    let large = async {
        let (_db, service) = automatic_promotions(15).await;
        let before = service.rule_reads();
        let page = service
            .list_promotions(
                PromotionFilter::default(),
                Pagination {
                    limit: 50,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.data.len(), 15);
        service.rule_reads() - before
    }
    .await;

    assert_eq!(
        large, small,
        "the rule query count must not grow with the number of promotions"
    );
    assert_eq!(large, 1, "all rules must be loaded in a single query");
}

/// More promotion ids than one statement may bind: the batched loader then
/// chunks, and the count grows with the chunks rather than with the ids.
#[tokio::test]
async fn rule_reads_chunk_under_the_parameter_limit() {
    let count = crate::service::SQLITE_PARAMETER_LIMIT + 5;
    let (_db, service) = automatic_promotions(count).await;
    let before = service.rule_reads();

    let listed = service.list_automatic_promotions(100).await.unwrap();

    assert_eq!(listed.len(), count);
    assert_eq!(
        service.rule_reads() - before,
        2,
        "905 ids must be read in two chunks, not one statement per promotion"
    );
}
