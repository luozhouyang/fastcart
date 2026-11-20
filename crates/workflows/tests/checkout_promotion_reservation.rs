//! Promotion usage is claimed before the money moves and returned when the
//! checkout never becomes an order.
//!
//! These tests drive the whole checkout workflow, because the interesting
//! failure mode used to live in the orchestration: the limit was checked while
//! quoting, the charge happened anyway, and the usage was recorded (or not)
//! after the order existed. A checkout that cannot claim its promotions must
//! fail before any capture.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_cart::{CartStatus, CreateCartInput};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_inventory::{CreateInventoryItemInput, CreateLocationInput, InventoryService};
use fastcart_module_payment::{
    ManualProvider, PaymentService, PaymentStatus as PaymentState, ProviderRegistry,
};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_promotion::{
    CampaignBudgetType, CreateCampaignInput, CreatePromotionInput, PromotionService, PromotionType,
};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_workflows::{CartContext, CartWorkflow, CheckoutWorkflow, CreateCheckoutRequest};

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

async fn count_rows(db: &StoreDatabase, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(db.pool())
        .await
        .expect("count rows")
}

#[tokio::test]
async fn an_exhausted_campaign_budget_rejects_the_checkout_before_capture() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();

    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Budget limited".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                manage_inventory: Some(false),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 1_000))
        .await
        .unwrap();

    let shipping = ShippingService::new(db.clone());
    let profile = shipping
        .create_profile(CreateShippingProfileInput {
            name: "Default".to_owned(),
            profile_type: None,
            metadata: None,
        })
        .await
        .unwrap();
    let option = shipping
        .create_option(CreateShippingOptionInput {
            profile_id: profile.id.clone(),
            name: "Free".to_owned(),
            amount_minor: 0,
            currency: currency.clone(),
            provider_id: None,
            provider_data: None,
            fulfillment_set_id: None,
            is_enabled: Some(true),
            metadata: None,
        })
        .await
        .unwrap();

    // Two automatic promotions. Only the second one is limited, and the limit
    // lives on a campaign whose budget is never consulted while quoting, so
    // both carts keep the discount and reach completion with matching totals.
    let promotions = PromotionService::new(db.clone());
    let unlimited = promotions
        .create_promotion(
            CreatePromotionInput::new("Always on", PromotionType::Percentage, 1_000)
                .active()
                .automatic(),
        )
        .await
        .unwrap();
    let limited = promotions
        .create_promotion(
            CreatePromotionInput::new("Budgeted", PromotionType::Percentage, 1_000)
                .active()
                .automatic(),
        )
        .await
        .unwrap();
    let campaign = promotions
        .create_campaign(CreateCampaignInput {
            name: "One usage".to_owned(),
            budget_type: CampaignBudgetType::Usage,
            currency_code: None,
            budget_limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    promotions
        .link_promotion(&campaign.id, &limited.id)
        .await
        .unwrap();

    let context = CartContext::new(CountryCode::new("US").unwrap());
    let carts = CartWorkflow::new(db.clone());
    let mut checkouts = Vec::new();
    for _ in 0..2 {
        let cart = carts
            .create_cart(CreateCartInput {
                currency_code: currency.to_string(),
                customer_id: None,
                email: Some("buyer@example.com".to_owned()),
                sales_channel_id: None,
                region_id: None,
                metadata: Metadata::empty(),
                expires_in_seconds: None,
            })
            .await
            .unwrap();
        let cart = carts
            .add_item(
                &cart.id,
                &VariantId::from_string(variant_id.clone()),
                1,
                &context,
            )
            .await
            .unwrap();
        carts
            .set_shipping_method(&cart.id, &option.id, &context)
            .await
            .unwrap();
        assert!(
            cart.totals.discount_total > 0,
            "both promotions discount the cart while quoting it"
        );
        checkouts.push((cart, None));
    }

    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let payments = PaymentService::with_registry(db.clone(), providers);
    let workflow = CheckoutWorkflow::new(db.clone(), payments);

    for (cart, checkout) in checkouts.iter_mut() {
        *checkout = Some(
            workflow
                .create(CreateCheckoutRequest::new(
                    cart.id.clone(),
                    "manual",
                    context.clone(),
                ))
                .await
                .unwrap(),
        );
    }
    let first = checkouts[0].1.clone().unwrap();
    let second = checkouts[1].1.clone().unwrap();

    // The first checkout claims the single campaign usage and places its order.
    let placed = workflow.complete(&first.id, &context).await.unwrap();
    assert_eq!(
        promotions
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        1
    );
    assert_eq!(promotions.usage_count(&unlimited.id).await.unwrap(), 1);
    assert_eq!(promotions.usage_count(&limited.id).await.unwrap(), 1);
    // The claims were moved onto the placed order.
    let claimed_order: Option<String> =
        sqlx::query_scalar("SELECT order_id FROM promotion_usage WHERE promotion_id = ?")
            .bind(&limited.id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(claimed_order.as_deref(), Some(placed.order.id.as_str()));

    // The second checkout still holds the discount it quoted, but the budget
    // has nothing left for it. It must be rejected before any capture.
    let error = workflow
        .complete(&second.id, &context)
        .await
        .expect_err("an exhausted budget must reject the checkout");
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(ref message)
            if message.contains("campaign budget exhausted")),
        "unexpected error: {error}"
    );

    let latest = workflow.get(&second.id).await.unwrap();
    assert!(
        latest.order_id.is_none(),
        "the rejected checkout must not have placed an order"
    );
    assert_eq!(count_rows(&db, "orders").await, 1);
    assert_eq!(
        count_rows(&db, "payments").await,
        1,
        "the rejected checkout must not have produced a payment"
    );
    let session_status: String =
        sqlx::query_scalar("SELECT status FROM payment_sessions WHERE id = ?")
            .bind(latest.payment_session_id.as_ref().unwrap().as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_ne!(session_status, "captured");

    // A failing claim must not leave a half-finished claim behind.
    let leftover: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM promotion_usage WHERE order_id = ?")
            .bind(format!("checkout:{}", second.id.as_str()))
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(leftover, 0, "no claim may survive a rejected completion");
    assert_eq!(
        promotions.usage_count(&unlimited.id).await.unwrap(),
        1,
        "a claim taken before the failing one must be released again"
    );
}

#[tokio::test]
async fn a_compensated_checkout_returns_its_promotion_usage() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();

    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Compensated".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                manage_inventory: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 2_000))
        .await
        .unwrap();

    let inventory = InventoryService::new(db.clone());
    let location = inventory
        .create_location(CreateLocationInput {
            name: "Main warehouse".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: Some("COMP-1".to_owned()),
            variant_id: Some(VariantId::from_string(variant_id.clone())),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, 5, "restock", None)
        .await
        .unwrap();

    let shipping = ShippingService::new(db.clone());
    let profile = shipping
        .create_profile(CreateShippingProfileInput {
            name: "Default".to_owned(),
            profile_type: None,
            metadata: None,
        })
        .await
        .unwrap();
    let option = shipping
        .create_option(CreateShippingOptionInput {
            profile_id: profile.id.clone(),
            name: "Free".to_owned(),
            amount_minor: 0,
            currency: currency.clone(),
            provider_id: None,
            provider_data: None,
            fulfillment_set_id: None,
            is_enabled: Some(true),
            metadata: None,
        })
        .await
        .unwrap();

    let promotions = PromotionService::new(db.clone());
    let promotion = promotions
        .create_promotion(
            CreatePromotionInput::new("Compensated discount", PromotionType::Percentage, 1_000)
                .active()
                .automatic()
                .with_usage_limit(10),
        )
        .await
        .unwrap();
    let campaign = promotions
        .create_campaign(CreateCampaignInput {
            name: "Spend budget".to_owned(),
            budget_type: CampaignBudgetType::Spend,
            currency_code: Some(currency.clone()),
            budget_limit: Some(10_000),
            ..Default::default()
        })
        .await
        .unwrap();
    promotions
        .link_promotion(&campaign.id, &promotion.id)
        .await
        .unwrap();

    let context = CartContext::new(CountryCode::new("US").unwrap());
    let carts = CartWorkflow::new(db.clone());
    let cart = carts
        .create_cart(CreateCartInput {
            currency_code: currency.to_string(),
            customer_id: None,
            email: Some("buyer@example.com".to_owned()),
            sales_channel_id: None,
            region_id: None,
            metadata: Metadata::empty(),
            expires_in_seconds: None,
        })
        .await
        .unwrap();
    let cart = carts
        .add_item(
            &cart.id,
            &VariantId::from_string(variant_id.clone()),
            1,
            &context,
        )
        .await
        .unwrap();
    carts
        .set_shipping_method(&cart.id, &option.id, &context)
        .await
        .unwrap();

    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let payments = PaymentService::with_registry(db.clone(), providers);
    let workflow = CheckoutWorkflow::new(db.clone(), payments);

    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            context.clone(),
        ))
        .await
        .unwrap();

    // The reservation expires before completion, so the capture succeeds but
    // the inventory commit fails and the checkout is compensated.
    inventory.release_checkout(&checkout.id).await.unwrap();

    let error = workflow.complete(&checkout.id, &context).await.unwrap_err();
    assert!(
        error.to_string().contains("reservation"),
        "unexpected error: {error}"
    );

    let failed = workflow.get(&checkout.id).await.unwrap();
    assert!(failed.order_id.is_none());
    assert_eq!(
        failed.status,
        fastcart_module_checkout::CheckoutStatus::Failed
    );

    let payment_status: String = sqlx::query_scalar("SELECT status FROM payments LIMIT 1")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(payment_status, "refunded");

    // The refund undone the sale, so the promotion usage and the campaign
    // budget must be back where they started.
    assert_eq!(
        promotions.usage_count(&promotion.id).await.unwrap(),
        0,
        "compensation must return the claimed usage"
    );
    assert_eq!(
        promotions
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        0,
        "compensation must return the reserved budget"
    );
    let leftover: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM promotion_usage WHERE order_id = ?")
            .bind(format!("checkout:{}", checkout.id.as_str()))
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(leftover, 0);
    assert_ne!(
        carts.get_cart(&cart.id).await.unwrap().status,
        CartStatus::Completed,
        "a compensated checkout must not complete its cart"
    );
}

#[tokio::test]
async fn a_placed_checkout_records_usage_against_the_order() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();

    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Placed".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                manage_inventory: Some(false),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 1_000))
        .await
        .unwrap();

    let shipping = ShippingService::new(db.clone());
    let profile = shipping
        .create_profile(CreateShippingProfileInput {
            name: "Default".to_owned(),
            profile_type: None,
            metadata: None,
        })
        .await
        .unwrap();
    let option = shipping
        .create_option(CreateShippingOptionInput {
            profile_id: profile.id.clone(),
            name: "Free".to_owned(),
            amount_minor: 0,
            currency: currency.clone(),
            provider_id: None,
            provider_data: None,
            fulfillment_set_id: None,
            is_enabled: Some(true),
            metadata: None,
        })
        .await
        .unwrap();

    let promotions = PromotionService::new(db.clone());
    let promotion = promotions
        .create_promotion(
            CreatePromotionInput::new("Placed discount", PromotionType::Percentage, 1_000)
                .active()
                .automatic()
                .with_usage_limit(1),
        )
        .await
        .unwrap();

    let context = CartContext::new(CountryCode::new("US").unwrap());
    let carts = CartWorkflow::new(db.clone());
    let cart = carts
        .create_cart(CreateCartInput {
            currency_code: currency.to_string(),
            customer_id: None,
            email: Some("buyer@example.com".to_owned()),
            sales_channel_id: None,
            region_id: None,
            metadata: Metadata::empty(),
            expires_in_seconds: None,
        })
        .await
        .unwrap();
    carts
        .add_item(
            &cart.id,
            &VariantId::from_string(variant_id.clone()),
            1,
            &context,
        )
        .await
        .unwrap();
    carts
        .set_shipping_method(&cart.id, &option.id, &context)
        .await
        .unwrap();

    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let payments = PaymentService::with_registry(db.clone(), providers);
    let workflow = CheckoutWorkflow::new(db.clone(), payments);

    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            context.clone(),
        ))
        .await
        .unwrap();
    let completed = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(completed.payment.status, PaymentState::Captured);
    assert_eq!(promotions.usage_count(&promotion.id).await.unwrap(), 1);

    let recorded_order: Option<String> =
        sqlx::query_scalar("SELECT order_id FROM promotion_usage WHERE promotion_id = ?")
            .bind(&promotion.id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(
        recorded_order.as_deref(),
        Some(completed.order.id.as_str()),
        "a placed order owns its usage row"
    );

    // A replay converges instead of double counting.
    let replayed = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(replayed.order.id, completed.order.id);
    assert_eq!(promotions.usage_count(&promotion.id).await.unwrap(), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM promotion_usage WHERE promotion_id = ?")
            .bind(&promotion.id)
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1
    );
}
