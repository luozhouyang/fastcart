use super::*;

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
