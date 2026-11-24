use super::*;

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
