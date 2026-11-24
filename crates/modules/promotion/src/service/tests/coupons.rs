use super::*;

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
