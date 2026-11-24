use super::*;

#[tokio::test]
async fn listing_automatic_promotions_keeps_rules_grouped_and_ordered() {
    let (_db, service) = automatic_promotions(5).await;

    let listed = service.list_automatic_promotions(100).await.unwrap();
    assert_eq!(listed.len(), 5);
    let mut ids = listed
        .iter()
        .map(|promotion| promotion.id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 5);

    for promotion in &listed {
        assert_eq!(promotion.rules.len(), 2);
        // The batched listing must return exactly what the single load
        // returns for each promotion.
        let single = service.get_promotion(&promotion.id).await.unwrap();
        assert_eq!(promotion, &single);
    }
}

#[tokio::test]
async fn listing_promotions_keeps_rules_grouped_and_ordered() {
    let (_db, service) = automatic_promotions(5).await;
    let manual = service
        .create_promotion(
            CreatePromotionInput::new("Manual", PromotionType::FixedAmount, 100)
                .active()
                .with_rule(PromotionRuleInput::new(
                    PromotionRuleAttribute::Currency,
                    PromotionRuleOperator::Eq,
                    "USD",
                )),
        )
        .await
        .unwrap();

    let page = service
        .list_promotions(
            PromotionFilter::default(),
            Pagination {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.count, 6);
    assert_eq!(page.data.len(), 6);
    for promotion in &page.data {
        let single = service.get_promotion(&promotion.id).await.unwrap();
        assert_eq!(promotion, &single);
        assert_eq!(
            promotion.rules.len(),
            if promotion.id == manual.id { 1 } else { 2 }
        );
    }
}

#[tokio::test]
async fn listing_automatic_promotions_issues_constant_rule_queries() {
    let small = automatic_listing_rule_reads(2).await;
    let large = automatic_listing_rule_reads(12).await;
    assert_eq!(
        large, small,
        "the rule query count must not grow with the number of promotions"
    );
    assert_eq!(large, 1, "all rules must be loaded in a single query");
}

#[tokio::test]
async fn listing_promotions_issues_constant_rule_queries() {
    let small = async {
        let (_db, service) = automatic_promotions(3).await;
        let before = service.rule_reads();
        let page = service
            .list_promotions(
                PromotionFilter::default(),
                Pagination {
                    limit: 50,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.data.len(), 3);
        service.rule_reads() - before
    }
    .await;
    let large = async {
        let (_db, service) = automatic_promotions(15).await;
        let before = service.rule_reads();
        let page = service
            .list_promotions(
                PromotionFilter::default(),
                Pagination {
                    limit: 50,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.data.len(), 15);
        service.rule_reads() - before
    }
    .await;

    assert_eq!(
        large, small,
        "the rule query count must not grow with the number of promotions"
    );
    assert_eq!(large, 1, "all rules must be loaded in a single query");
}

/// More promotion ids than one statement may bind: the batched loader then
/// chunks, and the count grows with the chunks rather than with the ids.
#[tokio::test]
async fn rule_reads_chunk_under_the_parameter_limit() {
    let count = crate::service::SQLITE_PARAMETER_LIMIT + 5;
    let (_db, service) = automatic_promotions(count).await;
    let before = service.rule_reads();

    let listed = service.list_automatic_promotions(100).await.unwrap();

    assert_eq!(listed.len(), count);
    assert_eq!(
        service.rule_reads() - before,
        2,
        "905 ids must be read in two chunks, not one statement per promotion"
    );
}
