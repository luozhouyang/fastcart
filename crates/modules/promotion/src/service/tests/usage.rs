use super::*;

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
