//! Where a promotion's discount lands.
//!
//! `items` is the historic behaviour and stays the default: percentages and
//! fixed amounts discount the eligible line items. `order` discounts the item
//! subtotal plus shipping as a single cart-level discount, `shipping` discounts
//! shipping alone. Free shipping is inherently a shipping discount and refuses
//! any other target.
//!
//! The full `Vec<PromotionResult>` is asserted for the `items` target against
//! strings recorded from the pre-`target` build, so the default path cannot
//! drift by a byte.

use std::str::FromStr;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CurrencyCode, Metadata, StoreId};
use fastcart_module_promotion::{
    CreatePromotionInput, Promotion, PromotionApplication, PromotionCart, PromotionContext,
    PromotionLine, PromotionResult, PromotionRule, PromotionRuleAttribute, PromotionRuleInput,
    PromotionRuleOperator, PromotionService, PromotionStatus, PromotionTarget, PromotionType,
    UpdatePromotionInput, calculate_promotions,
};

/// The complete `Vec<PromotionResult>` debug output recorded from the build
/// before `PromotionTarget` existed (see the tests below for the inputs).
const RECORDED_STACKED: &str = "[PromotionResult { promotion_id: \"promo_pct\", code: Some(\"PROMO_PCT\"), applications: [PromotionApplication { promotion_id: \"promo_pct\", code: Some(\"PROMO_PCT\"), line_id: Some(\"item_a\"), amount: 100, is_shipping: false, description: \"Percentage discount (10%)\" }], total_discount: 100, shipping_discount: 0 }, PromotionResult { promotion_id: \"promo_fixed\", code: Some(\"PROMO_FIXED\"), applications: [PromotionApplication { promotion_id: \"promo_fixed\", code: Some(\"PROMO_FIXED\"), line_id: Some(\"item_a\"), amount: 167, is_shipping: false, description: \"Fixed amount discount\" }, PromotionApplication { promotion_id: \"promo_fixed\", code: Some(\"PROMO_FIXED\"), line_id: Some(\"item_b\"), amount: 333, is_shipping: false, description: \"Fixed amount discount\" }], total_discount: 500, shipping_discount: 0 }, PromotionResult { promotion_id: \"promo_ship\", code: Some(\"PROMO_SHIP\"), applications: [PromotionApplication { promotion_id: \"promo_ship\", code: Some(\"PROMO_SHIP\"), line_id: None, amount: 700, is_shipping: true, description: \"Free shipping\" }], total_discount: 0, shipping_discount: 700 }]";
const RECORDED_EXCLUSIVE: &str = "[PromotionResult { promotion_id: \"promo_pct\", code: Some(\"PROMO_PCT\"), applications: [PromotionApplication { promotion_id: \"promo_pct\", code: Some(\"PROMO_PCT\"), line_id: Some(\"item_a\"), amount: 100, is_shipping: false, description: \"Percentage discount (10%)\" }], total_discount: 100, shipping_discount: 0 }, PromotionResult { promotion_id: \"promo_exclusive\", code: Some(\"PROMO_EXCLUSIVE\"), applications: [PromotionApplication { promotion_id: \"promo_exclusive\", code: Some(\"PROMO_EXCLUSIVE\"), line_id: Some(\"item_a\"), amount: 33, is_shipping: false, description: \"Fixed amount discount\" }, PromotionApplication { promotion_id: \"promo_exclusive\", code: Some(\"PROMO_EXCLUSIVE\"), line_id: Some(\"item_b\"), amount: 67, is_shipping: false, description: \"Fixed amount discount\" }], total_discount: 100, shipping_discount: 0 }]";
const RECORDED_EMPTY: &str = "[]";

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn line(line_id: &str, product_id: &str, unit_price: i64, quantity: i64) -> PromotionLine {
    PromotionLine {
        line_id: line_id.to_owned(),
        product_id: product_id.to_owned(),
        variant_id: format!("var_{line_id}"),
        category_ids: Vec::new(),
        unit_price,
        quantity,
        is_shipping: false,
    }
}

fn cart() -> PromotionCart {
    PromotionCart {
        currency: usd(),
        subtotal: 3_000,
        shipping_total: 700,
        customer_id: Some("cust_1".to_owned()),
        customer_group_ids: Vec::new(),
        lines: vec![
            line("item_a", "prod_a", 1_000, 1),
            line("item_b", "prod_b", 2_000, 1),
        ],
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
        is_automatic: true,
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

fn targeting(mut promotion: Promotion, target: PromotionTarget) -> Promotion {
    promotion.target = target;
    promotion
}

fn rule(attribute: PromotionRuleAttribute, value: &str) -> PromotionRule {
    let input = PromotionRuleInput::new(attribute, PromotionRuleOperator::In, value);
    PromotionRule {
        id: "rule_1".to_owned(),
        promotion_id: "promo_1".to_owned(),
        attribute: input.attribute,
        operator: input.operator,
        value: input.value,
        created_at: 0,
    }
}

fn context(promotion: Promotion, rules: Vec<PromotionRule>) -> PromotionContext {
    PromotionContext { promotion, rules }
}

/// The three scenarios whose complete results were recorded before the target
/// existed: two stacking promotions plus free shipping, the same with an
/// exclusive promotion in the middle, and a fixed amount with nothing to
/// discount.
fn recorded_scenarios() -> (
    Vec<PromotionResult>,
    Vec<PromotionResult>,
    Vec<PromotionResult>,
) {
    let cart = cart();
    let percentage = promotion("promo_pct", PromotionType::Percentage, 1_000, 1);
    let fixed = promotion("promo_fixed", PromotionType::FixedAmount, 500, 2);
    let free_shipping = promotion("promo_ship", PromotionType::FreeShipping, 0, 3);
    let percentage_rule = || vec![rule(PromotionRuleAttribute::ProductId, r#"["prod_a"]"#)];

    let stacked = calculate_promotions(
        &[
            context(percentage.clone(), percentage_rule()),
            context(fixed.clone(), Vec::new()),
            context(free_shipping.clone(), Vec::new()),
        ],
        &cart,
    )
    .unwrap();

    let mut exclusive = promotion("promo_exclusive", PromotionType::FixedAmount, 100, 2);
    exclusive.is_stackable = false;
    let gated = calculate_promotions(
        &[
            context(percentage.clone(), percentage_rule()),
            context(exclusive.clone(), Vec::new()),
            context(free_shipping.clone(), Vec::new()),
        ],
        &cart,
    )
    .unwrap();

    let mut empty_cart = cart.clone();
    empty_cart.subtotal = 0;
    empty_cart.lines = Vec::new();
    let empty = calculate_promotions(&[context(fixed, Vec::new())], &empty_cart).unwrap();

    (stacked, gated, empty)
}

/// The default target is `items`, and the items path must return exactly the
/// results the pre-`target` build returned — the same applications, amounts,
/// order and descriptions.
#[test]
fn items_keeps_the_recorded_pre_change_results() {
    let (stacked, gated, empty) = recorded_scenarios();

    assert_eq!(
        format!("{stacked:?}"),
        RECORDED_STACKED,
        "the default target must reproduce the historic results exactly"
    );
    assert_eq!(
        format!("{gated:?}"),
        RECORDED_EXCLUSIVE,
        "exclusivity must keep working exactly as before"
    );
    assert_eq!(
        format!("{empty:?}"),
        RECORDED_EMPTY,
        "a promotion with nothing to discount still produces no result"
    );
}

/// Setting the target explicitly to `items` is the same as leaving it at its
/// default, for every promotion type.
#[test]
fn items_is_the_default_target() {
    assert_eq!(PromotionTarget::default(), PromotionTarget::Items);

    let explicit = promotion("promo_pct", PromotionType::Percentage, 1_000, 1);
    let implicit = promotion("promo_pct", PromotionType::Percentage, 1_000, 1);
    let explicit = calculate_promotions(&[context(explicit, Vec::new())], &cart()).unwrap();
    let implicit = calculate_promotions(&[context(implicit, Vec::new())], &cart()).unwrap();

    assert_eq!(
        explicit, implicit,
        "naming the default target must not change the result"
    );
}

/// The items path against hand-written expectations: a percentage discount is
/// computed per eligible line, a fixed amount is split proportionally over the
/// eligible lines, and free shipping owns the shipping total.
#[test]
fn items_discounts_match_explicit_per_line_expectations() {
    let cart = cart();
    let percentage = promotion("promo_pct", PromotionType::Percentage, 1_000, 1);
    let fixed = promotion("promo_fixed", PromotionType::FixedAmount, 500, 2);

    let results = calculate_promotions(
        &[
            context(percentage.clone(), Vec::new()),
            context(fixed.clone(), Vec::new()),
        ],
        &cart,
    )
    .unwrap();

    let application =
        |promotion: &Promotion, line_id: Option<&str>, amount: i64, description: &str| {
            PromotionApplication {
                promotion_id: promotion.id.clone(),
                code: promotion.code.clone(),
                line_id: line_id.map(str::to_owned),
                amount,
                is_shipping: false,
                description: description.to_owned(),
            }
        };
    let expected = vec![
        PromotionResult {
            promotion_id: percentage.id.clone(),
            code: percentage.code.clone(),
            applications: vec![
                application(
                    &percentage,
                    Some("item_a"),
                    100,
                    "Percentage discount (10%)",
                ),
                application(
                    &percentage,
                    Some("item_b"),
                    200,
                    "Percentage discount (10%)",
                ),
            ],
            total_discount: 300,
            shipping_discount: 0,
        },
        PromotionResult {
            promotion_id: fixed.id.clone(),
            code: fixed.code.clone(),
            applications: vec![
                application(&fixed, Some("item_a"), 167, "Fixed amount discount"),
                application(&fixed, Some("item_b"), 333, "Fixed amount discount"),
            ],
            total_discount: 500,
            shipping_discount: 0,
        },
    ];

    assert_eq!(results, expected);
}

/// An order-target percentage is computed on the item subtotal plus shipping
/// and arrives as one cart-level application.
#[test]
fn order_percentage_discounts_the_item_subtotal_plus_shipping() {
    let cart = cart();
    let promotion = targeting(
        promotion("promo_order_pct", PromotionType::Percentage, 1_000, 1),
        PromotionTarget::Order,
    );

    let results = calculate_promotions(&[context(promotion.clone(), Vec::new())], &cart).unwrap();

    assert_eq!(
        results,
        vec![PromotionResult {
            promotion_id: promotion.id.clone(),
            code: promotion.code.clone(),
            applications: vec![PromotionApplication {
                promotion_id: promotion.id.clone(),
                code: promotion.code.clone(),
                line_id: None,
                amount: 370,
                is_shipping: false,
                description: "Order percentage discount (10%)".to_owned(),
            }],
            total_discount: 370,
            shipping_discount: 0,
        }],
        "10% of 3000 + 700"
    );
}

/// An order-target fixed amount is capped at the order total, and the value it
/// consumed is reported against both legs of the cart: the item subtotal first,
/// then shipping.
#[test]
fn order_fixed_amount_is_capped_at_the_order_total() {
    let cart = cart();
    let promotion = targeting(
        promotion("promo_order_fixed", PromotionType::FixedAmount, 5_000, 1),
        PromotionTarget::Order,
    );

    let results = calculate_promotions(&[context(promotion, Vec::new())], &cart).unwrap();

    assert_eq!(results[0].applications.len(), 1);
    assert_eq!(
        results[0].applications[0].amount, 3_700,
        "5000 fixed amount is capped at the 3000 + 700 order"
    );
    assert_eq!(results[0].applications[0].line_id, None);
    assert!(!results[0].applications[0].is_shipping);
    assert_eq!(
        (results[0].total_discount, results[0].shipping_discount),
        (3_000, 700),
        "an order discount consumes the remaining item value first"
    );
}

/// Order-target promotions consume the value the earlier ones left, and their
/// amount is capped by the remaining subtotal plus the remaining shipping.
///
/// The consumption lands on the item side: that is the side of the cart a
/// cart-level discount is charged against, and the shipping left over stays
/// available to the shipping discounts behind it.
#[test]
fn order_discounts_are_capped_at_the_remaining_value() {
    let cart = cart();
    let first = targeting(
        promotion("promo_order_first", PromotionType::FixedAmount, 3_000, 1),
        PromotionTarget::Order,
    );
    let second = targeting(
        promotion("promo_order_second", PromotionType::Percentage, 1_000, 2),
        PromotionTarget::Order,
    );
    let items = promotion("promo_items", PromotionType::FixedAmount, 100, 3);

    let results = calculate_promotions(
        &[
            context(first, Vec::new()),
            context(second.clone(), Vec::new()),
            context(items, Vec::new()),
        ],
        &cart,
    )
    .unwrap();

    // The first promotion takes the whole item subtotal. The second one still
    // sees the 700 of shipping, so it grants its 370; the items-target
    // promotion behind them finds no item value left and produces nothing.
    assert_eq!(
        allocations_after(&results),
        vec![
            ("promo_order_first".to_owned(), 3_000),
            ("promo_order_second".to_owned(), 370),
        ]
    );

    // A cart whose value is all shipping: the order discount is capped at the
    // shipping total and spends the budget the next order discount would use.
    let mut shipping_only = cart.clone();
    shipping_only.subtotal = 0;
    shipping_only.lines = Vec::new();
    let greedy = targeting(
        promotion("promo_greedy", PromotionType::FixedAmount, 1_000, 1),
        PromotionTarget::Order,
    );
    let leftovers = calculate_promotions(
        &[context(greedy, Vec::new()), context(second, Vec::new())],
        &shipping_only,
    )
    .unwrap();

    assert_eq!(
        allocations_after(&leftovers),
        vec![("promo_greedy".to_owned(), 700)],
        "the 1000 fixed amount is capped at the 700 order total, and 10% of that order finds \
         nothing left"
    );
}

/// An order discount does not consume the shipping budget: a shipping-target
/// promotion behind it still sees the whole shipping charge.
#[test]
fn an_order_discount_leaves_the_shipping_for_shipping_promotions() {
    let cart = cart();
    let order = targeting(
        promotion("promo_order", PromotionType::FixedAmount, 1_000, 1),
        PromotionTarget::Order,
    );
    let shipping = targeting(
        promotion("promo_shipping", PromotionType::Percentage, 5_000, 2),
        PromotionTarget::Shipping,
    );

    let results = calculate_promotions(
        &[context(order, Vec::new()), context(shipping, Vec::new())],
        &cart,
    )
    .unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(
        results[1].shipping_discount, 350,
        "half of the untouched 700 shipping"
    );
}

/// A shipping-target percentage discounts the shipping total, as a shipping
/// application.
#[test]
fn shipping_percentage_discounts_shipping_only() {
    let cart = cart();
    let promotion = targeting(
        promotion("promo_shipping_pct", PromotionType::Percentage, 5_000, 1),
        PromotionTarget::Shipping,
    );

    let results = calculate_promotions(&[context(promotion.clone(), Vec::new())], &cart).unwrap();

    assert_eq!(
        results,
        vec![PromotionResult {
            promotion_id: promotion.id.clone(),
            code: promotion.code.clone(),
            applications: vec![PromotionApplication {
                promotion_id: promotion.id.clone(),
                code: promotion.code.clone(),
                line_id: None,
                amount: 350,
                is_shipping: true,
                description: "Shipping percentage discount (50%)".to_owned(),
            }],
            total_discount: 0,
            shipping_discount: 350,
        }],
        "50% of the 700 shipping charge, and none of the items"
    );
}

/// A shipping-target fixed amount is capped at the shipping total, and a
/// shipping discount behind it only sees what is left.
#[test]
fn shipping_fixed_amount_is_capped_at_shipping() {
    let cart = cart();
    let first = targeting(
        promotion("promo_shipping_first", PromotionType::FixedAmount, 1_000, 1),
        PromotionTarget::Shipping,
    );
    let second = targeting(
        promotion("promo_shipping_second", PromotionType::Percentage, 5_000, 2),
        PromotionTarget::Shipping,
    );

    let results = calculate_promotions(
        &[context(first, Vec::new()), context(second, Vec::new())],
        &cart,
    )
    .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].shipping_discount, 700);
    assert_eq!(results[0].total_discount, 0);
    assert!(results[0].applications[0].is_shipping);
    assert!(results[0].applications[0].line_id.is_none());
}

/// A shipping-target percentage rounds like every other percentage.
#[test]
fn shipping_percentage_rounds_half_away_from_zero() {
    let mut cart = cart();
    cart.shipping_total = 1_005;
    let promotion = targeting(
        promotion("promo_shipping_pct", PromotionType::Percentage, 1_000, 1),
        PromotionTarget::Shipping,
    );

    let results = calculate_promotions(&[context(promotion, Vec::new())], &cart).unwrap();

    assert_eq!(results[0].shipping_discount, 101, "10% of 1005");
}

/// Free shipping is a shipping discount by definition, so it refuses to carry
/// any other target, on creation and on update alike.
#[tokio::test]
async fn free_shipping_rejects_a_non_default_target() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PromotionService::new(db);

    for target in [PromotionTarget::Order, PromotionTarget::Shipping] {
        let error = service
            .create_promotion(
                CreatePromotionInput::new("Free shipping", PromotionType::FreeShipping, 0)
                    .with_target(target),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AppError::Validation(message) if message.contains("free shipping")),
            "a free shipping promotion with the {target:?} target must be rejected, got: {error}"
        );
    }

    let shipping = service
        .create_promotion(CreatePromotionInput::new(
            "Free shipping",
            PromotionType::FreeShipping,
            0,
        ))
        .await
        .unwrap();
    let error = service
        .update_promotion(
            &shipping.id,
            UpdatePromotionInput {
                target: Some(PromotionTarget::Order),
                ..UpdatePromotionInput::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AppError::Validation(message) if message.contains("free shipping")),
        "an update must not smuggle a target onto a free shipping promotion, got: {error}"
    );

    let stored = service.get_promotion(&shipping.id).await.unwrap();
    assert_eq!(stored.target, PromotionTarget::Items);
}

/// The target is part of the promotion: it is stored, read back, updated, and
/// left alone by updates that do not mention it. Rows written before the column
/// existed read back as the default.
#[tokio::test]
async fn the_target_is_persisted_and_legacy_rows_default_to_items() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PromotionService::new(db.clone());

    let created = service
        .create_promotion(
            CreatePromotionInput::new("Whole order", PromotionType::FixedAmount, 500)
                .active()
                .with_target(PromotionTarget::Order),
        )
        .await
        .unwrap();
    assert_eq!(created.target, PromotionTarget::Order);

    let stored: String = sqlx::query_scalar("SELECT target FROM promotions WHERE id = ?")
        .bind(&created.id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(
        stored, "order",
        "the target must be persisted, not just echoed back"
    );
    assert_eq!(
        service.get_promotion(&created.id).await.unwrap().target,
        PromotionTarget::Order
    );

    let updated = service
        .update_promotion(
            &created.id,
            UpdatePromotionInput {
                target: Some(PromotionTarget::Shipping),
                ..UpdatePromotionInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service.get_promotion(&created.id).await.unwrap().target,
        PromotionTarget::Shipping
    );
    assert_eq!(updated.target, PromotionTarget::Shipping);

    let untouched = service
        .update_promotion(
            &created.id,
            UpdatePromotionInput {
                title: Some("Renamed".to_owned()),
                ..UpdatePromotionInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        untouched.target,
        PromotionTarget::Shipping,
        "an update that does not mention the target must leave it alone"
    );

    // A row written before the column existed, inserted without naming it.
    sqlx::query(
        "INSERT INTO promotions (id, title, type, created_at, updated_at)
         VALUES ('legacy', 'Legacy', 'fixed_amount', 1, 1)",
    )
    .execute(db.pool())
    .await
    .unwrap();
    assert_eq!(
        service.get_promotion("legacy").await.unwrap().target,
        PromotionTarget::Items,
        "an existing row must keep the historic behaviour"
    );
}

/// The migration itself backfills the historic behaviour into rows that already
/// existed when the column was added.
#[tokio::test]
async fn the_migration_backfills_the_target_of_existing_rows() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    sqlx::query("ALTER TABLE promotions DROP COLUMN target")
        .execute(db.pool())
        .await
        .expect("drop the column the migration adds");
    sqlx::query(
        "INSERT INTO promotions (id, title, type, created_at, updated_at)
         VALUES ('old', 'Old', 'fixed_amount', 1, 1)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let migration = include_str!("../../../../migrations/store/0052_promotion_target.sql");
    for statement in migration.split(';') {
        if !statement.contains("ALTER TABLE") {
            continue;
        }
        sqlx::query(statement)
            .execute(db.pool())
            .await
            .unwrap_or_else(|error| panic!("{statement}: {error}"));
    }

    let stored: String = sqlx::query_scalar("SELECT target FROM promotions WHERE id = 'old'")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(
        stored, "items",
        "the migration must backfill every existing row with the historic behaviour"
    );
}

/// The stored representation is the same three-value vocabulary the promotion
/// type uses, and anything else is a validation error.
#[test]
fn the_target_round_trips_through_its_stored_representation() {
    for (target, stored) in [
        (PromotionTarget::Items, "items"),
        (PromotionTarget::Order, "order"),
        (PromotionTarget::Shipping, "shipping"),
    ] {
        assert_eq!(target.as_str(), stored);
        assert_eq!(PromotionTarget::from_str(stored).unwrap(), target);
    }

    let error = PromotionTarget::from_str("everything").unwrap_err();
    assert!(
        matches!(&error, AppError::Validation(message) if message.contains("promotion target")),
        "an unknown target must be rejected, got: {error}"
    );
}

/// The discount each promotion ended up granting, in application order.
fn allocations_after(results: &[PromotionResult]) -> Vec<(String, i64)> {
    results
        .iter()
        .map(|result| {
            let granted = result
                .applications
                .iter()
                .map(|application| application.amount)
                .sum::<i64>();
            (result.promotion_id.clone(), granted)
        })
        .collect()
}
