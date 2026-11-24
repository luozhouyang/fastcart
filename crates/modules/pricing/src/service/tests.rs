use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CurrencyCode, Metadata, Pagination, StoreId};

use super::PricingService;
use crate::domain::{
    CreatePriceInput, CreatePriceListInput, Price, PriceContext, PriceFilter, PriceListRuleInput,
    PriceListStatus, PriceListType, UpdatePriceInput, UpdatePriceListInput, select_price,
};

async fn service() -> PricingService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    PricingService::new(db)
}

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn eur() -> CurrencyCode {
    CurrencyCode::new("EUR").unwrap()
}

fn context(quantity: i64, at: i64) -> PriceContext {
    PriceContext {
        currency: usd(),
        quantity,
        customer_group_id: None,
        sales_channel_id: None,
        at,
    }
}

fn sample_price(id: &str, amount: i64) -> Price {
    Price {
        id: id.to_owned(),
        price_list_id: None,
        price_list_type: None,
        variant_id: "var_1".to_owned(),
        currency_code: usd(),
        amount,
        min_quantity: 1,
        max_quantity: None,
        customer_group_id: None,
        sales_channel_id: None,
        starts_at: None,
        ends_at: None,
        metadata: Metadata::empty(),
        created_at: 0,
        updated_at: 0,
        price_list_rule_count: 0,
    }
}

fn sale_price(id: &str, list_id: &str, amount: i64) -> Price {
    let mut price = sample_price(id, amount);
    price.price_list_id = Some(list_id.to_owned());
    price.price_list_type = Some(PriceListType::Sale);
    price
}

fn override_price(id: &str, list_id: &str, amount: i64) -> Price {
    let mut price = sample_price(id, amount);
    price.price_list_id = Some(list_id.to_owned());
    price.price_list_type = Some(PriceListType::Override);
    price
}

#[test]
fn selector_filters_by_currency() {
    let usd_price = sample_price("price_usd", 1000);
    let mut eur_price = sample_price("price_eur", 900);
    eur_price.currency_code = eur();

    let selected = select_price(&[eur_price, usd_price], &context(1, 100)).unwrap();
    assert_eq!(selected.price_id, "price_usd");
    assert_eq!(selected.amount, 1000);
    assert!(selected.price_list_id.is_none());
}

#[test]
fn selector_uses_quantity_tiers() {
    let base = sample_price("price_base", 1000);
    let mut tier = sample_price("price_tier", 900);
    tier.min_quantity = 10;

    let low = select_price(&[base.clone(), tier.clone()], &context(5, 100)).unwrap();
    assert_eq!(low.price_id, "price_base");
    let high = select_price(&[base, tier], &context(10, 100)).unwrap();
    assert_eq!(high.price_id, "price_tier");
}

#[test]
fn selector_respects_quantity_upper_bounds() {
    let base = sample_price("price_base", 1000);
    let mut tier = sample_price("price_tier", 900);
    tier.min_quantity = 10;
    tier.max_quantity = Some(20);

    assert_eq!(
        select_price(&[base.clone(), tier.clone()], &context(21, 100))
            .unwrap()
            .price_id,
        "price_base"
    );
    assert_eq!(
        select_price(&[base, tier], &context(20, 100))
            .unwrap()
            .price_id,
        "price_tier"
    );
}

#[test]
fn selector_ignores_expired_prices() {
    let mut stale = sample_price("price_stale", 100);
    stale.ends_at = Some(100);
    let mut current = sample_price("price_current", 200);
    current.starts_at = Some(50);
    current.ends_at = Some(200);

    let selected = select_price(&[stale, current], &context(1, 100)).unwrap();
    assert_eq!(selected.price_id, "price_current");
}

#[test]
fn selector_prefers_customer_group_scope() {
    let base = sample_price("price_base", 1000);
    let mut scoped = sample_price("price_group", 900);
    scoped.customer_group_id = Some("vip".to_owned());

    let mut scoped_context = context(1, 100);
    scoped_context.customer_group_id = Some("vip".to_owned());
    assert_eq!(
        select_price(&[base.clone(), scoped.clone()], &scoped_context)
            .unwrap()
            .price_id,
        "price_group"
    );

    assert_eq!(
        select_price(&[base, scoped], &context(1, 100))
            .unwrap()
            .price_id,
        "price_base"
    );
}

#[test]
fn selector_prefers_sales_channel_scope() {
    let base = sample_price("price_base", 1000);
    let mut scoped = sample_price("price_channel", 900);
    scoped.sales_channel_id = Some("web".to_owned());

    let mut channel_context = context(1, 100);
    channel_context.sales_channel_id = Some("web".to_owned());
    assert_eq!(
        select_price(&[base, scoped], &channel_context)
            .unwrap()
            .price_id,
        "price_channel"
    );
}

#[test]
fn selector_applies_sale_only_when_it_is_a_discount() {
    let base = sample_price("price_base", 1000);
    let cheaper = sale_price("price_sale", "plist_1", 900);
    let selected = select_price(&[base.clone(), cheaper], &context(1, 100)).unwrap();
    assert_eq!(selected.price_id, "price_sale");
    assert_eq!(selected.price_list_id.as_deref(), Some("plist_1"));
    assert_eq!(selected.price_list_type, Some(PriceListType::Sale));
    assert_eq!(selected.original_amount, Some(1000));

    let expensive = sale_price("price_sale", "plist_1", 1100);
    let selected = select_price(&[base, expensive], &context(1, 100)).unwrap();
    assert_eq!(selected.price_id, "price_base");
    assert_eq!(selected.price_list_type, None);
    assert_eq!(selected.original_amount, None);
}

#[test]
fn selector_uses_sale_when_no_base_price_exists() {
    let only_sale = sale_price("price_sale", "plist_1", 900);
    let selected = select_price(&[only_sale], &context(1, 100)).unwrap();
    assert_eq!(selected.price_id, "price_sale");
    assert_eq!(selected.original_amount, None);
}

#[test]
fn selector_override_replaces_base_and_sale() {
    let base = sample_price("price_base", 1000);
    let sale = sale_price("price_sale", "plist_sale", 700);
    let over = override_price("price_override", "plist_override", 1200);

    let selected = select_price(&[base, sale, over], &context(1, 100)).unwrap();
    assert_eq!(selected.price_id, "price_override");
    assert_eq!(selected.price_list_type, Some(PriceListType::Override));
    assert_eq!(selected.original_amount, Some(1000));
}

#[test]
fn selector_ranks_multiple_matching_lists_deterministically() {
    let base = sample_price("price_base", 1000);

    // More matching rules beat a cheaper but less specific list.
    let mut generic = sale_price("price_generic", "plist_generic", 800);
    generic.price_list_rule_count = 0;
    let mut specific = sale_price("price_specific", "plist_specific", 950);
    specific.price_list_rule_count = 1;
    assert_eq!(
        select_price(
            &[base.clone(), generic.clone(), specific.clone()],
            &context(1, 100)
        )
        .unwrap()
        .price_id,
        "price_specific"
    );

    // With equal specificity the lowest amount wins.
    let mut cheaper = generic.clone();
    cheaper.price_list_rule_count = 1;
    cheaper.amount = 700;
    assert_eq!(
        select_price(&[base.clone(), specific, cheaper], &context(1, 100))
            .unwrap()
            .price_id,
        "price_generic"
    );

    // With equal specificity and amount the newest wins.
    let mut newest = generic;
    newest.price_list_rule_count = 1;
    newest.amount = 700;
    newest.starts_at = Some(50);
    let older = {
        let mut older = newest.clone();
        older.id = "price_older".to_owned();
        older.starts_at = Some(10);
        older
    };
    assert_eq!(
        select_price(&[base, older, newest], &context(1, 100))
            .unwrap()
            .price_id,
        "price_generic"
    );
}

#[test]
fn selector_tie_breaks_are_deterministic() {
    let cheap = sample_price("price_cheap", 1000);
    let expensive = sample_price("price_expensive", 1100);
    assert_eq!(
        select_price(&[expensive, cheap], &context(1, 100))
            .unwrap()
            .price_id,
        "price_cheap"
    );

    let alpha = sample_price("price_a", 1000);
    let beta = sample_price("price_b", 1000);
    assert_eq!(
        select_price(&[beta, alpha], &context(1, 100))
            .unwrap()
            .price_id,
        "price_a"
    );
}

#[test]
fn selector_returns_none_without_a_match() {
    assert!(select_price(&[], &context(1, 100)).is_none());
    let mut price = sample_price("price_future", 1000);
    price.starts_at = Some(200);
    assert!(select_price(&[price], &context(1, 100)).is_none());
}

#[tokio::test]
async fn creates_and_reads_price_lists_with_rules() {
    let service = service().await;
    let input = CreatePriceListInput::new("Summer sale")
        .active()
        .with_rule(PriceListRuleInput::eq("customer_group_id", "\"vip\""));

    let created = service.create_price_list(input).await.unwrap();
    assert_eq!(created.title, "Summer sale");
    assert_eq!(created.status, PriceListStatus::Active);
    assert_eq!(created.rules.len(), 1);

    let fetched = service.get_price_list(&created.id).await.unwrap();
    assert_eq!(fetched, created);
}

#[tokio::test]
async fn updates_price_lists_and_replaces_rules() {
    let service = service().await;
    let created = service
        .create_price_list(CreatePriceListInput::new("Sale").active())
        .await
        .unwrap();

    let updated = service
        .update_price_list(
            &created.id,
            UpdatePriceListInput {
                title: Some("Autumn sale".to_owned()),
                price_type: Some(PriceListType::Override),
                rules: Some(vec![PriceListRuleInput::eq("sales_channel_id", "\"web\"")]),
                ..UpdatePriceListInput::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.title, "Autumn sale");
    assert_eq!(updated.price_type, PriceListType::Override);
    assert_eq!(updated.rules.len(), 1);
    assert_eq!(updated.created_at, created.created_at);
}

#[tokio::test]
async fn lists_price_lists_with_pagination() {
    let service = service().await;
    for index in 0..3 {
        service
            .create_price_list(CreatePriceListInput::new(format!("Sale {index}")))
            .await
            .unwrap();
    }

    let page = service
        .list_price_lists(Pagination {
            limit: 2,
            offset: 0,
        })
        .await
        .unwrap();
    assert_eq!(page.count, 3);
    assert_eq!(page.data.len(), 2);
}

#[tokio::test]
async fn rejects_duplicate_price_scope() {
    let service = service().await;
    let input = CreatePriceInput::new("var_1", usd(), 1000);
    service.create_price(input.clone()).await.unwrap();

    let error = service.create_price(input).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn validates_price_inputs() {
    let service = service().await;
    let negative = CreatePriceInput::new("var_1", usd(), -1);
    assert!(matches!(
        service.create_price(negative).await.unwrap_err(),
        AppError::Validation(_)
    ));

    let mut zero_minimum = CreatePriceInput::new("var_1", usd(), 1000);
    zero_minimum.min_quantity = 0;
    assert!(matches!(
        service.create_price(zero_minimum).await.unwrap_err(),
        AppError::Validation(_)
    ));

    let mut inverted_range = CreatePriceInput::new("var_1", usd(), 1000);
    inverted_range.min_quantity = 3;
    inverted_range.max_quantity = Some(2);
    assert!(matches!(
        service.create_price(inverted_range).await.unwrap_err(),
        AppError::Validation(_)
    ));

    let missing_list = CreatePriceInput::new("var_1", usd(), 1000).with_price_list("plist_missing");
    assert!(matches!(
        service.create_price(missing_list).await.unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[tokio::test]
async fn manages_prices_with_filters_updates_and_deletes() {
    let service = service().await;
    let first = service
        .create_price(CreatePriceInput::new("var_1", usd(), 1000))
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_2", usd(), 2000))
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_1", eur(), 900))
        .await
        .unwrap();

    let by_variant = service
        .list_prices(
            PriceFilter {
                variant_id: Some("var_1".to_owned()),
                ..PriceFilter::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_variant.count, 2);

    let by_currency = service
        .list_prices(
            PriceFilter {
                currency_code: Some(eur()),
                ..PriceFilter::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_currency.count, 1);

    let updated = service
        .update_price(
            &first.id,
            UpdatePriceInput {
                amount: Some(750),
                max_quantity: Some(Some(5)),
                ..UpdatePriceInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.amount, 750);
    assert_eq!(updated.max_quantity, Some(5));
    assert_eq!(updated.created_at, first.created_at);

    service.delete_price(&first.id).await.unwrap();
    assert!(matches!(
        service.get_price(&first.id).await.unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service.delete_price(&first.id).await.unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[tokio::test]
async fn bulk_upsert_updates_existing_scope() {
    let service = service().await;
    let initial = vec![
        CreatePriceInput::new("var_1", usd(), 1000),
        CreatePriceInput::new("var_1", usd(), 900).with_quantity_range(10, None),
    ];
    let inserted = service.bulk_upsert_prices(initial).await.unwrap();
    assert_eq!(inserted.len(), 2);

    let updated = service
        .bulk_upsert_prices(vec![
            CreatePriceInput::new("var_1", usd(), 800),
            CreatePriceInput::new("var_1", usd(), 700).with_quantity_range(10, None),
        ])
        .await
        .unwrap();
    assert_eq!(updated.len(), 2);
    assert!(updated.iter().any(|price| price.amount == 800));
    assert!(updated.iter().any(|price| price.amount == 700));

    let all = service
        .list_prices(
            PriceFilter {
                variant_id: Some("var_1".to_owned()),
                ..PriceFilter::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(all.count, 2);
}

#[tokio::test]
async fn price_list_rules_restrict_selection() {
    let service = service().await;
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 1000))
        .await
        .unwrap();
    let list = service
        .create_price_list(
            CreatePriceListInput::new("VIP")
                .active()
                .with_rule(PriceListRuleInput::eq("customer_group_id", "vip")),
        )
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 800).with_price_list(&list.id))
        .await
        .unwrap();

    assert_eq!(
        service
            .calculate_price("var_1", &context(1, 100))
            .await
            .unwrap()
            .unwrap()
            .amount,
        1000
    );
    let mut vip = context(1, 100);
    vip.customer_group_id = Some("vip".to_owned());
    assert_eq!(
        service
            .calculate_price("var_1", &vip)
            .await
            .unwrap()
            .unwrap()
            .amount,
        800
    );
}

#[tokio::test]
async fn calculation_ignores_inactive_and_expired_price_lists() {
    let service = service().await;
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 1000))
        .await
        .unwrap();

    let draft = service
        .create_price_list(CreatePriceListInput::new("Draft sale"))
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 500).with_price_list(&draft.id))
        .await
        .unwrap();
    assert_eq!(
        service
            .calculate_price("var_1", &context(1, 100))
            .await
            .unwrap()
            .unwrap()
            .amount,
        1000
    );

    let active = service
        .create_price_list(CreatePriceListInput::new("Active sale").active())
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 800).with_price_list(&active.id))
        .await
        .unwrap();
    assert_eq!(
        service
            .calculate_price("var_1", &context(1, 100))
            .await
            .unwrap()
            .unwrap()
            .amount,
        800
    );

    let expired = service
        .create_price_list(
            CreatePriceListInput::new("Expired sale")
                .active()
                .with_window(Some(0), Some(50)),
        )
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 100).with_price_list(&expired.id))
        .await
        .unwrap();
    assert_eq!(
        service
            .calculate_price("var_1", &context(1, 100))
            .await
            .unwrap()
            .unwrap()
            .amount,
        800
    );
}

#[tokio::test]
async fn calculation_applies_scopes_and_batch_variants() {
    let service = service().await;
    sqlx::query(
        "INSERT INTO customer_groups (id, name, metadata, created_at, updated_at) \
             VALUES ('vip', 'VIP', '{}', 0, 0)",
    )
    .execute(service.db.pool())
    .await
    .unwrap();
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 1000))
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 900).with_customer_group("vip"))
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_2", usd(), 2000))
        .await
        .unwrap();

    let mut vip_context = context(1, 100);
    vip_context.customer_group_id = Some("vip".to_owned());
    let selected = service
        .calculate_price("var_1", &vip_context)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.amount, 900);
    assert_eq!(
        service
            .calculate_price("var_1", &context(1, 100))
            .await
            .unwrap()
            .unwrap()
            .amount,
        1000
    );

    let batch = service
        .calculate_prices(
            &[
                "var_1".to_owned(),
                "var_2".to_owned(),
                "var_missing".to_owned(),
            ],
            &context(1, 100),
        )
        .await
        .unwrap();
    assert_eq!(batch["var_1"].as_ref().unwrap().amount, 1000);
    assert_eq!(batch["var_2"].as_ref().unwrap().amount, 2000);
    assert!(batch["var_missing"].is_none());
}

#[tokio::test]
async fn price_sets_are_created_once_per_variant_and_linked_to_prices() {
    let service = service().await;
    let base = service
        .create_price(CreatePriceInput::new("var_1", usd(), 1000))
        .await
        .unwrap();
    let price_set = service
        .get_price_set_for_variant("var_1")
        .await
        .unwrap()
        .expect("price set");
    assert_eq!(price_set.title.as_deref(), Some("Prices for var_1"));

    let stored_set_id: String = sqlx::query_scalar("SELECT price_set_id FROM prices WHERE id = ?")
        .bind(&base.id)
        .fetch_one(service.db.pool())
        .await
        .unwrap();
    assert_eq!(stored_set_id, price_set.id);

    let list = service
        .create_price_list(CreatePriceListInput::new("Sale").active())
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 900).with_price_list(&list.id))
        .await
        .unwrap();
    let same_set = service
        .get_or_create_price_set_for_variant("var_1")
        .await
        .unwrap();
    assert_eq!(same_set.id, price_set.id);

    let other_set = service
        .get_or_create_price_set_for_variant("var_2")
        .await
        .unwrap();
    assert_ne!(other_set.id, price_set.id);
    assert!(
        service
            .get_price_set_for_variant("var_missing")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn calculation_treats_sales_as_discounts_and_overrides_as_replacements() {
    let service = service().await;
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 1000))
        .await
        .unwrap();

    let sale = service
        .create_price_list(CreatePriceListInput::new("Summer sale").active())
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 800).with_price_list(&sale.id))
        .await
        .unwrap();
    let selected = service
        .calculate_price("var_1", &context(1, 100))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.amount, 800);
    assert_eq!(selected.price_list_type, Some(PriceListType::Sale));
    assert_eq!(selected.original_amount, Some(1000));

    let overpriced = service
        .create_price_list(CreatePriceListInput::new("Bad sale").active())
        .await
        .unwrap();
    service
        .create_price(CreatePriceInput::new("var_1", usd(), 1200).with_price_list(&overpriced.id))
        .await
        .unwrap();
    let selected = service
        .calculate_price("var_1", &context(1, 100))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.amount, 800);
    assert_eq!(selected.price_list_id.as_deref(), Some(sale.id.as_str()));

    let override_list = service
        .create_price_list({
            let mut input = CreatePriceListInput::new("Absolute").active();
            input.price_type = PriceListType::Override;
            input
        })
        .await
        .unwrap();
    service
        .create_price(
            CreatePriceInput::new("var_1", usd(), 1500).with_price_list(&override_list.id),
        )
        .await
        .unwrap();
    let selected = service
        .calculate_price("var_1", &context(1, 100))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.amount, 1500);
    assert_eq!(selected.price_list_type, Some(PriceListType::Override));
    assert_eq!(selected.original_amount, Some(1000));
}

#[tokio::test]
async fn blank_price_scopes_are_normalized_to_none() {
    let service = service().await;
    let mut input = CreatePriceInput::new("var_1", usd(), 1000);
    input.customer_group_id = Some("   ".to_owned());
    input.sales_channel_id = Some(String::new());
    let created = service.create_price(input).await.unwrap();
    assert_eq!(created.customer_group_id, None);
    assert_eq!(created.sales_channel_id, None);

    let mut duplicate = CreatePriceInput::new("var_1", usd(), 900);
    duplicate.customer_group_id = Some(" ".to_owned());
    let error = service.create_price(duplicate).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn non_blank_price_scopes_must_exist() {
    let service = service().await;

    let mut missing_group = CreatePriceInput::new("var_1", usd(), 1000);
    missing_group.customer_group_id = Some("cg_missing".to_owned());
    assert!(matches!(
        service.create_price(missing_group).await.unwrap_err(),
        AppError::NotFound(_)
    ));

    let mut missing_channel = CreatePriceInput::new("var_1", usd(), 1000);
    missing_channel.sales_channel_id = Some("sc_missing".to_owned());
    assert!(matches!(
        service.create_price(missing_channel).await.unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[tokio::test]
async fn bulk_upsert_normalizes_blank_scopes_and_rejects_missing_ids() {
    let service = service().await;
    let mut blank = CreatePriceInput::new("var_1", usd(), 1000);
    blank.customer_group_id = Some("  ".to_owned());
    let created = service.bulk_upsert_prices(vec![blank]).await.unwrap();
    assert_eq!(created[0].customer_group_id, None);

    let mut missing = CreatePriceInput::new("var_2", usd(), 1000);
    missing.sales_channel_id = Some("sc_missing".to_owned());
    assert!(matches!(
        service.bulk_upsert_prices(vec![missing]).await.unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[tokio::test]
async fn price_list_rules_reject_unsupported_attributes_and_operators() {
    let service = service().await;

    let bad_attribute = CreatePriceListInput::new("Bad")
        .with_rule(PriceListRuleInput::eq("unknown_attribute", "value"));
    assert!(matches!(
        service.create_price_list(bad_attribute).await.unwrap_err(),
        AppError::Validation(_)
    ));

    let bad_operator = CreatePriceListInput::new("Bad").with_rule(PriceListRuleInput {
        attribute: "currency".to_owned(),
        operator: Some("gt".to_owned()),
        value: "USD".to_owned(),
    });
    assert!(matches!(
        service.create_price_list(bad_operator).await.unwrap_err(),
        AppError::Validation(_)
    ));

    let trimmed = CreatePriceListInput::new("Good")
        .with_rule(PriceListRuleInput::eq("  currency_code  ", "  USD  "));
    let created = service.create_price_list(trimmed).await.unwrap();
    assert_eq!(created.rules[0].attribute, "currency_code");
    assert_eq!(created.rules[0].value, "USD");
}

#[tokio::test]
async fn updating_missing_prices_and_lists_returns_not_found() {
    let service = service().await;
    assert!(matches!(
        service
            .update_price("price_missing", UpdatePriceInput::default())
            .await
            .unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service
            .update_price_list("plist_missing", UpdatePriceListInput::default())
            .await
            .unwrap_err(),
        AppError::NotFound(_)
    ));
}
