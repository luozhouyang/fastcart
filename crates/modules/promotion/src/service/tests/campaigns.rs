use super::*;

#[tokio::test]
async fn campaign_mutations_append_audit_entries_atomically() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PromotionService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));

    let campaign = service
        .create_campaign(CreateCampaignInput {
            name: "Launch".to_owned(),
            ..CreateCampaignInput::default()
        })
        .await
        .unwrap();
    service
        .update_campaign(
            &campaign.id,
            UpdateCampaignInput {
                name: Some("Launch 2".to_owned()),
                ..UpdateCampaignInput::default()
            },
        )
        .await
        .unwrap();
    service.delete_campaign(&campaign.id).await.unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec!["campaign.created", "campaign.updated", "campaign.deleted"]
    );
    assert!(audits.iter().all(|audit| audit.processed_at.is_none()));
    assert!(
        audits
            .iter()
            .all(|audit| audit.actor_id.as_deref() == Some("user_1"))
    );

    // A failed deletion is a no-op and must not append an audit row.
    assert!(service.delete_campaign(&campaign.id).await.is_err());
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 3);
}

#[tokio::test]
async fn campaign_budgets_are_consumed_atomically() {
    let service = service().await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Budgeted", PromotionType::FixedAmount, 100).active(),
        )
        .await
        .unwrap();
    let campaign = service
        .create_campaign(CreateCampaignInput {
            name: "Launch".to_owned(),
            budget_limit: Some(1),
            ..CreateCampaignInput::default()
        })
        .await
        .unwrap();
    service
        .link_promotion(&campaign.id, &promotion.id)
        .await
        .unwrap();
    assert_eq!(
        service
            .list_campaign_promotions(&campaign.id)
            .await
            .unwrap(),
        vec![promotion.id.clone()]
    );

    service
        .record_usage(&promotion.id, None, None, Some("order_1"), 0, None)
        .await
        .unwrap();
    let exhausted = service
        .record_usage(&promotion.id, None, None, Some("order_2"), 0, None)
        .await
        .unwrap_err();
    assert!(matches!(exhausted, AppError::Conflict(_)));
    let stored = service.get_campaign(&campaign.id).await.unwrap();
    assert_eq!(stored.budget_used, 1);
}

#[tokio::test]
async fn spend_campaign_budgets_track_monetary_amounts() {
    let service = service().await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Spend budget", PromotionType::Percentage, 1000).active(),
        )
        .await
        .unwrap();
    let campaign = service
        .create_campaign(CreateCampaignInput {
            name: "Spend".to_owned(),
            budget_type: CampaignBudgetType::Spend,
            currency_code: Some(usd()),
            budget_limit: Some(1_000),
            ..CreateCampaignInput::default()
        })
        .await
        .unwrap();
    service
        .link_promotion(&campaign.id, &promotion.id)
        .await
        .unwrap();

    service
        .record_usage(&promotion.id, None, None, Some("order_1"), 600, Some("USD"))
        .await
        .unwrap();
    let stored = service.get_campaign(&campaign.id).await.unwrap();
    assert_eq!(stored.budget_used, 600);
    assert_eq!(stored.budget_spent().unwrap().amount_minor(), 600);

    let wrong_currency = service
        .record_usage(&promotion.id, None, None, Some("order_2"), 100, Some("EUR"))
        .await
        .unwrap_err();
    assert!(matches!(wrong_currency, AppError::Conflict(_)));

    let exhausted = service
        .record_usage(&promotion.id, None, None, Some("order_3"), 500, Some("USD"))
        .await
        .unwrap_err();
    assert!(matches!(exhausted, AppError::Conflict(_)));
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        600
    );

    service
        .record_usage(&promotion.id, None, None, Some("order_4"), 400, Some("USD"))
        .await
        .unwrap();
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        1_000
    );
}
