//! Draft orders are priced, re-taxed and converted before fulfillment.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    CountryCode, CurrencyCode, InventoryItemId, InventoryLocationId, Metadata, StoreId, VariantId,
};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_inventory::{CreateInventoryItemInput, CreateLocationInput, InventoryService};
use fastcart_module_order::{
    CreateOrderEditInput, OrderEditAction, OrderEditItemInput, OrderStatus,
};
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
            is_tax_inclusive: None,
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
async fn conversion_re_prices_lines_added_by_an_edit() {
    let (workflow, _db) = workflow_with_tax().await;
    let draft = workflow
        .create_draft_order(request(json!({"country_code": "DE"}), 0))
        .await
        .unwrap();
    assert_eq!(draft.tax_total, 380);

    let edit = workflow
        .order_service()
        .create_order_edit(
            &draft.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::empty(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Extra".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: Some(false),
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    workflow
        .order_service()
        .confirm_order_edit(&draft.id, &edit.id)
        .await
        .unwrap();

    let converted = workflow.convert_draft_order(&draft.id).await.unwrap();
    assert_eq!(converted.status, OrderStatus::Pending);
    assert_eq!(converted.items.len(), 2);
    assert_eq!(converted.tax_total, 475);
    assert_eq!(converted.total, 2_975);
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

fn variant_request(variant_id: &str, quantity: i64) -> CreateDraftOrderRequest {
    let mut request = request(json!({"country_code": "DE"}), 0);
    request.lines[0].variant_id = Some(VariantId::from_string(variant_id.to_owned()));
    request.lines[0].quantity = quantity;
    request
}

/// Creates a published inventory-managed variant with `stocked` units.
async fn managed_variant(
    db: &StoreDatabase,
    stocked: i64,
    allow_backorder: bool,
) -> (VariantId, InventoryItemId, InventoryLocationId) {
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Managed widget".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                manage_inventory: Some(true),
                allow_backorder: Some(allow_backorder),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let variant = VariantId::from_string(product.variants[0].id.clone());
    let inventory = InventoryService::new(db.clone());
    let location = inventory
        .create_location(CreateLocationInput {
            name: "Warehouse".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, stocked, "restock", None)
        .await
        .unwrap();
    (variant, item.id, location.id)
}

#[tokio::test]
async fn converting_a_draft_commits_its_stock() {
    let (workflow, db) = workflow_with_tax().await;
    let (variant, item, location) = managed_variant(&db, 10, false).await;
    let inventory = InventoryService::new(db.clone());
    let draft = workflow
        .create_draft_order(variant_request(variant.as_str(), 2))
        .await
        .unwrap();
    assert_eq!(inventory.variant_availability(&variant).await.unwrap(), 10);

    let placed = workflow.convert_draft_order(&draft.id).await.unwrap();

    assert_eq!(placed.status, OrderStatus::Pending);
    assert_eq!(
        inventory
            .get_level(&item, &location)
            .await
            .unwrap()
            .stocked_quantity,
        8,
        "conversion must take the units the converted order promises"
    );
    let (status, line_item_id, order_id): (String, String, String) = sqlx::query_as(
        "SELECT status, line_item_id, order_id FROM inventory_reservations WHERE order_id = ?",
    )
    .bind(placed.id.as_str())
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(status, "committed");
    assert_eq!(line_item_id, placed.items[0].id.as_str());
    assert_eq!(order_id, placed.id.as_str());

    workflow
        .order_service()
        .cancel(&placed.id, Some("customer request"))
        .await
        .unwrap();
    assert_eq!(inventory.restock_order(&placed.id).await.unwrap(), 1);
    assert_eq!(
        inventory
            .get_level(&item, &location)
            .await
            .unwrap()
            .stocked_quantity,
        10,
        "cancelling a converted draft restocks through its committed reservation"
    );
}

#[tokio::test]
async fn a_draft_short_on_stock_is_not_converted() {
    let (workflow, db) = workflow_with_tax().await;
    let (variant, item, location) = managed_variant(&db, 1, false).await;
    let inventory = InventoryService::new(db.clone());
    let draft = workflow
        .create_draft_order(variant_request(variant.as_str(), 2))
        .await
        .unwrap();

    let error = workflow.convert_draft_order(&draft.id).await.unwrap_err();

    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );
    assert_eq!(
        workflow
            .order_service()
            .get(&draft.id)
            .await
            .unwrap()
            .status,
        OrderStatus::Draft
    );
    assert_eq!(
        inventory
            .get_level(&item, &location)
            .await
            .unwrap()
            .stocked_quantity,
        1
    );
    let reservations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inventory_reservations WHERE order_id = ?")
            .bind(draft.id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(
        reservations, 0,
        "the failed conversion leaves no reservation"
    );
}

#[tokio::test]
async fn a_backorder_draft_converts_with_negative_stock() {
    let (workflow, db) = workflow_with_tax().await;
    let (variant, item, location) = managed_variant(&db, 1, true).await;
    let inventory = InventoryService::new(db.clone());
    let draft = workflow
        .create_draft_order(variant_request(variant.as_str(), 3))
        .await
        .unwrap();

    let placed = workflow.convert_draft_order(&draft.id).await.unwrap();

    assert_eq!(placed.status, OrderStatus::Pending);
    assert_eq!(
        inventory
            .get_level(&item, &location)
            .await
            .unwrap()
            .stocked_quantity,
        0,
        "the covered unit comes from the stocked location"
    );
    let backorder = inventory
        .get_level(
            &item,
            &InventoryLocationId::from_string("loc_default".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(
        backorder.stocked_quantity, -2,
        "the shortfall stays visible as negative stock on the default location"
    );
}

#[tokio::test]
async fn store_inclusive_pricing_reaches_draft_lines() {
    let (workflow, db) = workflow_with_tax().await;
    StoreService::new(db)
        .set_setting("prices_include_tax", "true")
        .await
        .unwrap();

    let draft = workflow
        .create_draft_order(request(json!({"country_code": "DE"}), 0))
        .await
        .unwrap();

    assert!(draft.items[0].is_tax_inclusive);
    // 2000 gross at 19%: 319 is contained in the price, not added to it.
    assert_eq!(draft.items[0].tax_total, 319);
    assert_eq!(draft.items[0].total, 2_000);
    assert_eq!(draft.tax_total, 319);
    assert_eq!(draft.total, 2_000);
    assert!(draft.tax_lines.iter().all(|line| line.is_tax_inclusive));
}

#[tokio::test]
async fn an_explicit_line_flag_overrides_the_store_convention() {
    let (workflow, db) = workflow_with_tax().await;
    StoreService::new(db)
        .set_setting("prices_include_tax", "true")
        .await
        .unwrap();
    let mut request = request(json!({"country_code": "DE"}), 0);
    request.lines[0].is_tax_inclusive = Some(false);

    let draft = workflow.create_draft_order(request).await.unwrap();

    assert!(!draft.items[0].is_tax_inclusive);
    assert_eq!(draft.items[0].tax_total, 380);
    assert_eq!(draft.items[0].total, 2_380);
    assert_eq!(draft.total, 2_380);
}

#[tokio::test]
async fn an_added_draft_line_inherits_the_store_pricing_convention() {
    let (workflow, db) = workflow_with_tax().await;
    StoreService::new(db)
        .set_setting("prices_include_tax", "true")
        .await
        .unwrap();
    let draft = workflow
        .create_draft_order(request(json!({"country_code": "DE"}), 0))
        .await
        .unwrap();

    let edit = workflow
        .order_service()
        .create_order_edit(
            &draft.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::empty(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Extra".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: Some(false),
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    let confirmed = workflow
        .order_service()
        .confirm_order_edit(&draft.id, &edit.id)
        .await
        .unwrap();
    let added_edit_item = confirmed
        .items
        .iter()
        .find(|item| item.action == OrderEditAction::Add)
        .unwrap();
    assert!(added_edit_item.is_tax_inclusive);
    let added_order_line = workflow
        .order_service()
        .get(&draft.id)
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.product_title == "Extra")
        .unwrap();
    assert!(added_order_line.is_tax_inclusive);

    let converted = workflow.convert_draft_order(&draft.id).await.unwrap();

    let added = converted
        .items
        .iter()
        .find(|item| item.product_title == "Extra")
        .unwrap();
    assert_eq!(added.tax_total, 80);
    assert_eq!(added.total, 500);
    assert_eq!(converted.total, 2_500);
}

#[tokio::test]
async fn inclusive_pricing_covers_draft_shipping() {
    let (workflow, db) = workflow_with_tax().await;
    StoreService::new(db)
        .set_setting("prices_include_tax", "true")
        .await
        .unwrap();

    let draft = workflow
        .create_draft_order(request(json!({"country_code": "DE"}), 500))
        .await
        .unwrap();

    // 2500 gross: 319 is contained in the goods and 80 in the shipping.
    assert_eq!(draft.tax_total, 399);
    assert_eq!(draft.total, 2_500);
    assert!(
        draft
            .tax_lines
            .iter()
            .any(|line| line.order_item_id.is_none() && line.amount == 80 && line.is_tax_inclusive),
        "shipping tax follows the store's inclusive convention"
    );
    assert!(
        !draft
            .adjustments
            .iter()
            .any(|adjustment| adjustment.code == "shipping_tax"),
        "inclusive shipping tax is already inside the shipping amount"
    );
}
