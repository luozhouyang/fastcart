//! A promotion's place in the discount sequence and whether it stacks.
//!
//! Promotions are applied in `priority DESC, created_at ASC, id ASC` order
//! against the cart value left by the previous ones. A promotion with
//! `is_stackable = false` is exclusive: once it produces a discount, the
//! promotions ordered after it are skipped for that batch.
//!
//! With every priority left at its default of 0 and every promotion stackable,
//! the ordering degrades to the historic `created_at, id` order, so an
//! unconfigured store keeps the discount allocation it had before these columns
//! existed. Legacy rows prove it: an insert that never names the new columns
//! still reads back as priority 0 and stackable.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CurrencyCode, Metadata, StoreId};
use fastcart_module_promotion::{
    CreatePromotionInput, Promotion, PromotionCart, PromotionContext, PromotionLine,
    PromotionResult, PromotionService, PromotionStatus, PromotionType, UpdatePromotionInput,
    calculate_promotions,
};

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

/// The service over a store database migrated by the embeddings under test.
async fn service() -> (StoreDatabase, PromotionService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PromotionService::new(db.clone());
    (db, service)
}

/// A one-line cart worth `subtotal` and without shipping.
fn cart(subtotal: i64) -> PromotionCart {
    PromotionCart {
        currency: usd(),
        subtotal,
        shipping_total: 0,
        customer_id: Some("cust_1".to_owned()),
        customer_group_ids: Vec::new(),
        lines: vec![PromotionLine {
            line_id: "item_1".to_owned(),
            product_id: "prod_1".to_owned(),
            variant_id: "var_1".to_owned(),
            category_ids: Vec::new(),
            unit_price: subtotal,
            quantity: 1,
            is_shipping: false,
        }],
        at: 100,
    }
}

/// A fixed amount promotion with the historic defaults: priority 0, stackable.
fn promotion(id: &str, created_at: i64, value: i64) -> Promotion {
    Promotion {
        id: id.to_owned(),
        code: None,
        title: id.to_owned(),
        promotion_type: PromotionType::FixedAmount,
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

fn free_shipping(id: &str, created_at: i64) -> Promotion {
    Promotion {
        promotion_type: PromotionType::FreeShipping,
        ..promotion(id, created_at, 0)
    }
}

fn prioritized(mut promotion: Promotion, priority: i64) -> Promotion {
    promotion.priority = priority;
    promotion
}

fn exclusive(mut promotion: Promotion) -> Promotion {
    promotion.is_stackable = false;
    promotion
}

fn context(promotion: Promotion) -> PromotionContext {
    PromotionContext {
        promotion,
        rules: Vec::new(),
    }
}

/// The discount each promotion ended up with, in application order.
fn allocations(results: &[PromotionResult]) -> Vec<(String, i64)> {
    results
        .iter()
        .map(|result| (result.promotion_id.clone(), result.total_discount))
        .collect()
}

/// A promotion stored before the new columns existed: the insert names only the
/// columns that existed then, so the migration has to supply the new values.
#[tokio::test]
async fn rows_predating_the_columns_read_back_as_priority_zero_and_stackable() {
    let (db, service) = service().await;
    sqlx::query(
        "INSERT INTO promotions (id, title, type, created_at, updated_at)
         VALUES ('legacy', 'Legacy', 'fixed_amount', 1, 1)",
    )
    .execute(db.pool())
    .await
    .expect("insert a promotion without the new columns");

    let stored: (i64, i64) =
        sqlx::query_as("SELECT priority, is_stackable FROM promotions WHERE id = 'legacy'")
            .fetch_one(db.pool())
            .await
            .expect("read the defaults the migration supplied");

    assert_eq!(
        stored,
        (0, 1),
        "an existing row must keep the historic behaviour, not become exclusive"
    );

    let promotion = service.get_promotion("legacy").await.unwrap();
    assert_eq!(promotion.priority, 0, "a legacy row is last in line");
    assert!(
        promotion.is_stackable,
        "a legacy row must keep stacking with the promotions around it"
    );
}

/// The same guarantee at the migration itself: columns added to a table that
/// already holds rows must backfill the historic behaviour into those rows.
#[tokio::test]
async fn the_migration_backfills_rows_that_already_existed() {
    let (db, _service) = service().await;
    // Restore the pre-migration shape: drop the columns the migration adds.
    for column in ["priority", "is_stackable"] {
        sqlx::query(&format!("ALTER TABLE promotions DROP COLUMN {column}"))
            .execute(db.pool())
            .await
            .unwrap_or_else(|error| panic!("drop {column}: {error}"));
    }
    sqlx::query(
        "INSERT INTO promotions (id, title, type, created_at, updated_at)
         VALUES ('old', 'Old', 'fixed_amount', 1, 1)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // Run the migration file itself against the populated table.
    let migration =
        include_str!("../../../../migrations/store/0051_promotion_priority_and_stacking.sql");
    for statement in migration.split(';') {
        if !statement.contains("ALTER TABLE") {
            continue;
        }
        sqlx::query(statement)
            .execute(db.pool())
            .await
            .unwrap_or_else(|error| panic!("{statement}: {error}"));
    }

    let stored: (i64, i64) =
        sqlx::query_as("SELECT priority, is_stackable FROM promotions WHERE id = 'old'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(
        stored,
        (0, 1),
        "the migration must backfill every existing row with the historic behaviour"
    );
}

/// Two coupons that both want more than the cart holds: whichever is applied
/// first takes its face value and leaves the rest to the other, so swapping
/// their priorities swaps the allocation.
#[test]
fn higher_priority_consumes_the_cart_first() {
    let cart = cart(100);
    let early = promotion("promo_early", 1, 70);
    let late = promotion("promo_late", 2, 70);

    let baseline =
        calculate_promotions(&[context(early.clone()), context(late.clone())], &cart).unwrap();
    assert_eq!(
        allocations(&baseline),
        vec![
            ("promo_early".to_owned(), 70),
            ("promo_late".to_owned(), 30)
        ],
        "with equal priorities the earlier promotion consumes first"
    );

    let flipped =
        calculate_promotions(&[context(early), context(prioritized(late, 5))], &cart).unwrap();
    assert_eq!(
        allocations(&flipped),
        vec![
            ("promo_late".to_owned(), 70),
            ("promo_early".to_owned(), 30)
        ],
        "raising a promotion's priority must make it consume the cart first"
    );
}

/// The historic behaviour, stated as a test: when nobody configures a priority,
/// the allocation is the `created_at, id` one. Reordering the input cannot
/// change the result either.
#[test]
fn zero_priorities_reproduce_the_created_at_order() {
    let cart = cart(200);
    let first = promotion("promo_first", 1, 50);
    let a = promotion("promo_a", 2, 40);
    let b = promotion("promo_b", 2, 30);
    let c = promotion("promo_c", 3, 20);

    let mut reference = [first.clone(), a.clone(), b.clone(), c.clone()];
    reference.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });

    let shuffled = vec![
        context(b.clone()),
        context(c.clone()),
        context(a.clone()),
        context(first.clone()),
    ];
    let from_shuffled = calculate_promotions(&shuffled, &cart).unwrap();
    let reference_contexts = reference.map(context);
    let from_reference = calculate_promotions(&reference_contexts, &cart).unwrap();

    assert_eq!(
        from_shuffled, from_reference,
        "with every priority at its default the full result must be the historic created_at, id \
         order, down to the per-line applications"
    );
    assert_eq!(
        allocations(&from_shuffled),
        vec![
            ("promo_first".to_owned(), 50),
            ("promo_a".to_owned(), 40),
            ("promo_b".to_owned(), 30),
            ("promo_c".to_owned(), 20),
        ],
        "ties on created_at fall back to the id, as they always did"
    );
}

/// A non-stackable promotion owns the batch once it discounts: promotions
/// ordered after it are skipped, promotions already applied are kept.
#[test]
fn a_non_stackable_promotion_excludes_the_promotions_after_it() {
    let cart = cart(100);
    let before = promotion("promo_before", 1, 20);
    let exclusive = exclusive(promotion("promo_exclusive", 2, 30));
    let after = promotion("promo_after", 3, 40);

    let results = calculate_promotions(
        &[
            context(after.clone()),
            context(exclusive.clone()),
            context(before.clone()),
        ],
        &cart,
    )
    .unwrap();

    assert_eq!(
        allocations(&results),
        vec![
            ("promo_before".to_owned(), 20),
            ("promo_exclusive".to_owned(), 30),
        ],
        "the promotion after the exclusive one must not apply"
    );

    let stackable = calculate_promotions(
        &[
            context(before),
            context(after),
            context(promotion("promo_exclusive", 2, 30)),
        ],
        &cart,
    )
    .unwrap();
    assert_eq!(
        allocations(&stackable).len(),
        3,
        "the same promotions stack when the exclusive flag is off"
    );
}

/// Exclusivity is earned by producing a discount. A non-stackable promotion
/// that matches nothing, or finds nothing left to discount, must not silence
/// the promotions behind it.
#[test]
fn a_non_stackable_promotion_that_discounts_nothing_does_not_block_later_ones() {
    let cart = cart(100);
    let exclusive_free_shipping = exclusive(free_shipping("promo_ship", 1));
    let later = promotion("promo_later", 2, 30);

    let results =
        calculate_promotions(&[context(exclusive_free_shipping), context(later)], &cart).unwrap();

    assert_eq!(
        allocations(&results),
        vec![("promo_later".to_owned(), 30)],
        "a free shipping promotion with no shipping to discount must not block the rest"
    );
}

/// The end-to-end default: promotions written through the service, with nothing
/// configured, still stack and still consume the cart in creation order.
#[tokio::test]
async fn defaults_stack_and_keep_the_historic_creation_order() {
    let (db, service) = service().await;
    let first = service
        .create_promotion(
            CreatePromotionInput::new("First", PromotionType::FixedAmount, 5_000).active(),
        )
        .await
        .unwrap();
    let second = service
        .create_promotion(
            CreatePromotionInput::new("Second", PromotionType::FixedAmount, 4_000).active(),
        )
        .await
        .unwrap();
    assert_eq!(first.priority, 0, "a new promotion starts with priority 0");
    assert!(first.is_stackable, "a new promotion stacks by default");

    // Both rows were written in the same second; pin the creation order the
    // store cannot otherwise distinguish.
    for (id, created_at) in [(&first.id, 1_i64), (&second.id, 2_i64)] {
        sqlx::query("UPDATE promotions SET created_at = ? WHERE id = ?")
            .bind(created_at)
            .bind(id)
            .execute(db.pool())
            .await
            .unwrap();
    }

    let contexts = vec![
        context(service.get_promotion(&second.id).await.unwrap()),
        context(service.get_promotion(&first.id).await.unwrap()),
    ];
    let results = service.calculate(&contexts, &cart(10_000)).unwrap();

    assert_eq!(
        allocations(&results),
        vec![(first.id.clone(), 5_000), (second.id.clone(), 4_000)],
        "an unconfigured store must keep consuming the cart in creation order"
    );
}

/// Priority and stackability are part of the promotion, so creating one has to
/// store them and reading it back has to return them.
#[tokio::test]
async fn creating_a_promotion_stores_priority_and_stackability() {
    let (_db, service) = service().await;
    let created = service
        .create_promotion(
            CreatePromotionInput::new("Exclusive", PromotionType::FixedAmount, 500)
                .with_priority(7)
                .exclusive(),
        )
        .await
        .unwrap();
    assert_eq!(created.priority, 7);
    assert!(!created.is_stackable);

    let stored = service.get_promotion(&created.id).await.unwrap();
    assert_eq!(
        stored.priority, 7,
        "the priority must be persisted, not just echoed back"
    );
    assert!(
        !stored.is_stackable,
        "the stackability must be persisted, not just echoed back"
    );
}

/// An update may change either attribute, leave them alone when it does not
/// mention them, and relax them back to the defaults.
#[tokio::test]
async fn updating_a_promotion_persists_priority_and_stackability() {
    let (_db, service) = service().await;
    let created = service
        .create_promotion(CreatePromotionInput::new(
            "Plain",
            PromotionType::FixedAmount,
            500,
        ))
        .await
        .unwrap();
    assert_eq!((created.priority, created.is_stackable), (0, true));

    let updated = service
        .update_promotion(
            &created.id,
            UpdatePromotionInput {
                priority: Some(5),
                is_stackable: Some(false),
                ..UpdatePromotionInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!((updated.priority, updated.is_stackable), (5, false));

    let stored = service.get_promotion(&created.id).await.unwrap();
    assert_eq!(
        stored.priority, 5,
        "the priority must be persisted, not just echoed back"
    );
    assert!(
        !stored.is_stackable,
        "the stackability must be persisted, not just echoed back"
    );

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
        (untouched.priority, untouched.is_stackable),
        (5, false),
        "an update that does not mention them must leave them alone"
    );

    let relaxed = service
        .update_promotion(
            &created.id,
            UpdatePromotionInput {
                priority: Some(0),
                is_stackable: Some(true),
                ..UpdatePromotionInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!((relaxed.priority, relaxed.is_stackable), (0, true));

    let stored = service.get_promotion(&created.id).await.unwrap();
    assert_eq!(
        (stored.priority, stored.is_stackable),
        (0, true),
        "the relaxed values must be persisted too"
    );
}
