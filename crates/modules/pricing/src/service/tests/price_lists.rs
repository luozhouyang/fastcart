use super::*;

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
