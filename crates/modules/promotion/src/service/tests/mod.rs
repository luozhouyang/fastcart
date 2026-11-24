use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
use fastcart_kernel::{AppError, CurrencyCode, Metadata, Pagination, StoreId};

use super::PromotionService;
use crate::domain::{
    CampaignBudgetType, CreateCampaignInput, CreateCouponInput, CreatePromotionInput, Promotion,
    PromotionCart, PromotionContext, PromotionFilter, PromotionLine, PromotionRule,
    PromotionRuleAttribute, PromotionRuleInput, PromotionRuleOperator, PromotionStatus,
    PromotionTarget, PromotionType, UpdateCampaignInput, UpdatePromotionInput,
    calculate_promotions,
};

async fn service() -> PromotionService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    PromotionService::new(db)
}

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn eur() -> CurrencyCode {
    CurrencyCode::new("EUR").unwrap()
}

fn line(
    line_id: &str,
    product_id: &str,
    variant_id: &str,
    categories: &[&str],
    unit_price: i64,
    quantity: i64,
) -> PromotionLine {
    PromotionLine {
        line_id: line_id.to_owned(),
        product_id: product_id.to_owned(),
        variant_id: variant_id.to_owned(),
        category_ids: categories.iter().map(|value| (*value).to_owned()).collect(),
        unit_price,
        quantity,
        is_shipping: false,
    }
}

fn cart(subtotal: i64, shipping_total: i64, lines: Vec<PromotionLine>) -> PromotionCart {
    PromotionCart {
        currency: usd(),
        subtotal,
        shipping_total,
        customer_id: Some("cust_1".to_owned()),
        customer_group_ids: Vec::new(),
        lines,
        at: 100,
    }
}

fn promotion(id: &str, promotion_type: PromotionType, value: i64, created_at: i64) -> Promotion {
    Promotion {
        id: id.to_owned(),
        code: Some(id.to_ascii_uppercase()),
        title: id.to_owned(),
        promotion_type,
        target: PromotionTarget::Items,
        value,
        status: PromotionStatus::Active,
        is_automatic: false,
        starts_at: None,
        ends_at: None,
        usage_limit: None,
        usage_count: 0,
        per_customer_limit: None,
        priority: 0,
        is_stackable: true,
        metadata: Metadata::empty(),
        rules: Vec::new(),
        created_at,
        updated_at: created_at,
    }
}

fn rule(
    attribute: PromotionRuleAttribute,
    operator: PromotionRuleOperator,
    value: &str,
) -> PromotionRule {
    PromotionRule {
        id: "rule_1".to_owned(),
        promotion_id: "promo_1".to_owned(),
        attribute,
        operator,
        value: value.to_owned(),
        created_at: 0,
    }
}

fn context(promotion: Promotion, rules: Vec<PromotionRule>) -> PromotionContext {
    PromotionContext { promotion, rules }
}

mod calculation;
mod campaigns;
mod coupons;
mod promotions;
mod queries;
mod usage;

/// Creates a store with `count` automatic promotions that each carry two
/// rules and returns the service over it.
///
/// Rule reads are counted on the service instance itself
/// ([`PromotionService::rule_reads`]), so a test that asserts on them
/// cannot be disturbed by the other tests running in this binary: no
/// process-global subscriber, no thread attribution, no shared state.
async fn automatic_promotions(count: usize) -> (StoreDatabase, PromotionService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PromotionService::new(db.clone());
    for index in 0..count {
        service
            .create_promotion(
                CreatePromotionInput::new(
                    format!("Automatic {index}"),
                    PromotionType::Percentage,
                    1000,
                )
                .active()
                .automatic()
                .with_rule(PromotionRuleInput::new(
                    PromotionRuleAttribute::MinimumOrderSubtotal,
                    PromotionRuleOperator::Gte,
                    "5000",
                ))
                .with_rule(PromotionRuleInput::new(
                    PromotionRuleAttribute::ProductId,
                    PromotionRuleOperator::In,
                    r#"["prod_1"]"#,
                )),
            )
            .await
            .unwrap();
    }
    (db, service)
}

/// Lists `count` automatic promotions and returns how many rule reads the
/// listing issued.
async fn automatic_listing_rule_reads(count: usize) -> usize {
    let (_db, service) = automatic_promotions(count).await;
    let before = service.rule_reads();
    let listed = service.list_automatic_promotions(100).await.unwrap();
    assert_eq!(listed.len(), count);
    service.rule_reads() - before
}
