//! Draft orders are priced, re-taxed and converted before fulfillment.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId};
use fastcart_module_order::OrderStatus;
use fastcart_module_store::StoreService;
use fastcart_module_tax::{CreateTaxRateInput, CreateTaxRegionInput, TaxService};
use fastcart_workflows::{CreateDraftOrderRequest, DraftOrderLine, DraftOrderWorkflow};
use serde_json::json;

async fn workflow_with_tax() -> (DraftOrderWorkflow, StoreDatabase) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let tax = TaxService::new(db.clone());
    let region = tax
        .create_region(CreateTaxRegionInput {
            name: "Germany".to_owned(),
            country_code: CountryCode::new("DE").unwrap(),
            province_code: None,
            metadata: None,
        })
        .await
        .unwrap();
    tax.create_rate(CreateTaxRateInput {
        tax_region_id: region.id,
        tax_category_id: None,
        name: "VAT".to_owned(),
        rate_bps: 1_900,
        is_inclusive: false,
        is_default: true,
        metadata: None,
    })
    .await
    .unwrap();
    let france = tax
        .create_region(CreateTaxRegionInput {
            name: "France".to_owned(),
            country_code: CountryCode::new("FR").unwrap(),
            province_code: None,
            metadata: None,
        })
        .await
        .unwrap();
    tax.create_rate(CreateTaxRateInput {
        tax_region_id: france.id,
        tax_category_id: None,
        name: "TVA".to_owned(),
        rate_bps: 500,
        is_inclusive: false,
        is_default: true,
        metadata: None,
    })
    .await
    .unwrap();
    (DraftOrderWorkflow::new(db.clone()), db)
}

fn request(address: serde_json::Value, shipping_total: i64) -> CreateDraftOrderRequest {
    CreateDraftOrderRequest {
        customer_id: None,
        email: "buyer@example.com".to_owned(),
        currency_code: CurrencyCode::new("EUR").unwrap().to_string(),
        region_id: None,
        sales_channel_id: None,
        lines: vec![DraftOrderLine {
            product_id: None,
            variant_id: None,
            product_title: "Widget".to_owned(),
            variant_title: None,
            sku: None,
            thumbnail: None,
            unit_price: 1_000,
            quantity: 2,
            is_tax_inclusive: false,
            requires_shipping: true,
            metadata: Metadata::empty(),
        }],
        shipping_address: address,
        billing_address: json!({}),
        shipping_total,
        metadata: Metadata::empty(),
        idempotency_key: None,
    }
}

#[tokio::test]
async fn a_draft_is_priced_through_the_store_tax_provider() {
    let (workflow, _db) = workflow_with_tax().await;

    let draft = workflow
        .create_draft_order(request(
            json!({"country_code": "DE", "province_code": null}),
            500,
        ))
        .await
        .unwrap();

    assert_eq!(draft.status, OrderStatus::Draft);
    assert_eq!(draft.subtotal, 2_000);
    assert_eq!(draft.items[0].tax_total, 380);
    assert_eq!(draft.items[0].total, 2_380);
    // Item tax (380) plus the shipping tax (95) the provider charged on the
    // 500 shipping amount.
    assert_eq!(draft.tax_total, 475);
    assert_eq!(draft.total, 2_975);
    assert_eq!(draft.tax_lines.len(), 2);
    assert_eq!(
        draft.tax_lines.iter().map(|line| line.amount).sum::<i64>(),
        draft.tax_total
    );
}

#[tokio::test]
async fn an_address_without_a_country_is_created_untaxed() {
    let (workflow, _db) = workflow_with_tax().await;

    let draft = workflow
        .create_draft_order(request(json!({"city": "Berlin"}), 0))
        .await
        .unwrap();

    assert_eq!(draft.tax_total, 0);
    assert_eq!(draft.total, 2_000);
    assert!(draft.tax_lines.is_empty());
}

#[tokio::test]
async fn changing_the_destination_re_prices_the_goods() {
    let (workflow, _db) = workflow_with_tax().await;
    let draft = workflow
        .create_draft_order(request(json!({"country_code": "DE"}), 500))
        .await
        .unwrap();

    let updated = workflow
        .update_draft_destination(
            &draft.id,
            json!({"country_code": "FR", "province_code": "IDF"}),
            None,
        )
        .await
        .unwrap();

    assert_eq!(updated.shipping_address["country_code"], "FR");
    assert_eq!(updated.items[0].tax_total, 100);
    assert_eq!(updated.items[0].total, 2_100);
    // Shipping tax belongs to the shipping snapshot and is preserved: the
    // item tax drops from 380 to 100 while the 95 shipping tax remains.
    assert_eq!(updated.tax_total, 195);
    assert_eq!(updated.total, 2_695);
    assert!(
        updated
            .tax_lines
            .iter()
            .any(|line| line.shipping_option_id.is_none() && line.amount == 95),
        "the shipping tax line stays with the shipping"
    );
}

#[tokio::test]
async fn re_taxing_a_placed_order_is_refused() {
    let (workflow, _db) = workflow_with_tax().await;
    let draft = workflow
        .create_draft_order(request(json!({"country_code": "DE"}), 0))
        .await
        .unwrap();
    workflow.convert_draft_order(&draft.id).await.unwrap();

    let error = workflow
        .update_draft_destination(&draft.id, json!({"country_code": "FR"}), None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_converted_draft_enters_the_normal_lifecycle() {
    let (workflow, _db) = workflow_with_tax().await;
    let draft = workflow
        .create_draft_order(request(json!({"country_code": "DE"}), 0))
        .await
        .unwrap();

    let placed = workflow.convert_draft_order(&draft.id).await.unwrap();
    assert_eq!(placed.status, OrderStatus::Pending);

    let confirmed = workflow.order_service().confirm(&placed.id).await.unwrap();
    assert_eq!(confirmed.status, OrderStatus::Confirmed);
}

#[tokio::test]
async fn the_store_provider_setting_still_serves_draft_pricing() {
    let (workflow, db) = workflow_with_tax().await;
    StoreService::new(db)
        .set_setting("tax.provider_id", "missing")
        .await
        .unwrap();

    let error = workflow
        .create_draft_order(request(json!({"country_code": "DE"}), 0))
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );
}
