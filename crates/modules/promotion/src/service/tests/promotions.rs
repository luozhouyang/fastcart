use super::*;

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
