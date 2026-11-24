use super::*;

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
