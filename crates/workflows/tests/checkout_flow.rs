//! End-to-end cart, promotion, tax, inventory, payment and order flow.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId, VariantId, now_timestamp};
use fastcart_module_cart::{CartStatus, CreateCartInput};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_checkout::CheckoutStatus;
use fastcart_module_inventory::{CreateInventoryItemInput, CreateLocationInput, InventoryService};
use fastcart_module_payment::{
    ManualProvider, PaymentService, PaymentStatus as PaymentState, ProviderRegistry,
};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_promotion::{CreatePromotionInput, PromotionService, PromotionType};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_module_tax::{
    CreateTaxCategoryInput, CreateTaxRateInput, CreateTaxRegionInput, TaxService,
};
use fastcart_workflows::{CartContext, CartWorkflow, CheckoutWorkflow, CreateCheckoutRequest};

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

#[tokio::test]
async fn complete_checkout_flow_is_consistent_and_idempotent() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();

    // Catalog.
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Classic T-Shirt".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Medium".to_owned(),
                sku: Some("TSHIRT-M".to_owned()),
                manage_inventory: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let variant_id = product.variants[0].id.clone();

    // Pricing.
    let pricing = PricingService::new(db.clone());
    pricing
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 2_000))
        .await
        .unwrap();

    // Inventory with 10 units.
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
            sku: Some("TSHIRT-M".to_owned()),
            variant_id: Some(VariantId::from_string(variant_id.clone())),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, 10, "restock", None)
        .await
        .unwrap();

    // 10% automatic promotion backed by a monetary campaign budget.
    let promotions = PromotionService::new(db.clone());
    let promotion = promotions
        .create_promotion(
            CreatePromotionInput::new("Ten percent", PromotionType::Percentage, 1_000)
                .active()
                .automatic(),
        )
        .await
        .unwrap();
    let campaign = promotions
        .create_campaign(fastcart_module_promotion::CreateCampaignInput {
            name: "Ten percent budget".to_owned(),
            budget_type: fastcart_module_promotion::CampaignBudgetType::Spend,
            currency_code: Some(currency.clone()),
            budget_limit: Some(1_000),
            ..Default::default()
        })
        .await
        .unwrap();
    promotions
        .link_promotion(&campaign.id, &promotion.id)
        .await
        .unwrap();

    // 10% tax for US.
    let tax = TaxService::new(db.clone());
    let _category = tax
        .create_category(CreateTaxCategoryInput {
            name: "Standard".to_owned(),
            code: "standard".to_owned(),
            description: None,
        })
        .await
        .unwrap();
    let region = tax
        .create_region(CreateTaxRegionInput {
            name: "United States".to_owned(),
            country_code: CountryCode::new("US").unwrap(),
            province_code: None,
            metadata: None,
        })
        .await
        .unwrap();
    tax.create_rate(CreateTaxRateInput {
        tax_region_id: region.id.clone(),
        tax_category_id: None,
        name: "US sales tax".to_owned(),
        rate_bps: 1_000,
        is_inclusive: false,
        is_default: true,
        metadata: None,
    })
    .await
    .unwrap();

    // Flat shipping rate.
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
            name: "Standard shipping".to_owned(),
            amount_minor: 500,
            currency: currency.clone(),
            provider_id: None,
            provider_data: None,
            fulfillment_set_id: None,
            is_enabled: Some(true),
            metadata: None,
        })
        .await
        .unwrap();

    // Cart: 2 x 2000 = 4000, 10% off = 400, shipping 500, tax 410.
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
            2,
            &context,
        )
        .await
        .unwrap();
    let cart = carts
        .set_shipping_method(&cart.id, &option.id, &context)
        .await
        .unwrap();

    assert_eq!(cart.totals.item_subtotal, 4_000);
    assert_eq!(cart.totals.discount_total, 400);
    assert_eq!(cart.totals.shipping_total, 500);
    assert_eq!(cart.totals.tax_total, 410);
    assert_eq!(cart.totals.total, 4_510);
    assert_eq!(cart.totals.currency_code, currency);

    // Checkout reserves stock and creates a payment session.
    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let payments = PaymentService::with_registry(db.clone(), providers);
    let checkout_workflow = CheckoutWorkflow::new(db.clone(), payments);
    let checkout = checkout_workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            context.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(
        checkout.status.as_str(),
        fastcart_module_checkout::CheckoutStatus::PaymentPending.as_str()
    );
    assert_eq!(
        inventory
            .variant_availability(&VariantId::from_string(variant_id.clone()))
            .await
            .unwrap(),
        8
    );

    let completed = checkout_workflow
        .complete(&checkout.id, &context)
        .await
        .unwrap();
    assert_eq!(completed.order.total, 4_510);
    assert_eq!(completed.order.subtotal, 4_000);
    assert_eq!(completed.order.discount_total, 400);
    assert_eq!(completed.order.tax_total, 360);
    assert_eq!(
        completed.order.status,
        fastcart_module_order::OrderStatus::Confirmed
    );
    assert_eq!(
        completed.order.payment_status,
        fastcart_module_order::PaymentStatus::Captured
    );
    assert_eq!(completed.payment.status, PaymentState::Captured);
    assert_eq!(
        completed.checkout.status.as_str(),
        fastcart_module_checkout::CheckoutStatus::OrderPlaced.as_str()
    );
    assert_eq!(completed.order.items.len(), 1);
    assert_eq!(completed.order.items[0].product_title, "Classic T-Shirt");
    assert_eq!(completed.order.items[0].quantity, 2);

    // The campaign spends exactly the discount granted to the order.
    assert_eq!(
        promotions
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        400
    );

    // Stock is committed, not merely reserved.
    assert_eq!(
        inventory
            .variant_availability(&VariantId::from_string(variant_id))
            .await
            .unwrap(),
        8
    );
    assert_eq!(
        carts.get_cart(&cart.id).await.unwrap().status,
        CartStatus::Completed
    );

    // Replaying completion returns the same order and payment.
    let replay = checkout_workflow
        .complete(&checkout.id, &context)
        .await
        .unwrap();
    assert_eq!(replay.order.id, completed.order.id);
    assert_eq!(replay.payment.id, completed.payment.id);
}

#[tokio::test]
async fn completion_recovers_after_crash_between_capture_and_confirmation() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();

    // Minimal catalog, price and shipping setup; inventory is not managed here.
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Crash test product".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                manage_inventory: Some(false),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 1_500))
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
            name: "Standard".to_owned(),
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
        .add_item(&cart.id, &VariantId::from_string(variant_id), 1, &context)
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
    // A second service sharing the same provider registry lets the test capture
    // the payment directly, simulating a crash right after provider capture.
    let capture_payments = PaymentService::with_registry(db.clone(), providers.clone());
    let workflow = CheckoutWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    );

    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            context.clone(),
        ))
        .await
        .unwrap();
    let payment_session = checkout.payment_session_id.clone().unwrap();
    let captured = capture_payments
        .capture(payment_session.as_str())
        .await
        .unwrap();
    assert_eq!(captured.status, PaymentState::Captured);

    // The runner crashed after capture, before `payment_confirmed`, leaving a
    // claim whose lease has long expired.
    sqlx::query(
        "UPDATE checkout_sessions
         SET placement_token = 'crashed-runner', placement_claimed_at = ?
         WHERE id = ?",
    )
    .bind(now_timestamp() - 600)
    .bind(checkout.id.as_str())
    .execute(db.pool())
    .await
    .unwrap();
    assert_eq!(
        workflow.get(&checkout.id).await.unwrap().status,
        CheckoutStatus::PaymentPending
    );

    let completed = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(completed.checkout.status, CheckoutStatus::OrderPlaced);
    assert_eq!(completed.payment.status, PaymentState::Captured);
    assert_eq!(
        completed.order.payment_status,
        fastcart_module_order::PaymentStatus::Captured
    );

    let token: Option<String> =
        sqlx::query_scalar("SELECT placement_token FROM checkout_sessions WHERE id = ?")
            .bind(checkout.id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(token.is_none(), "placement claim should be cleared");

    // A replay returns the placed result without capturing or ordering again.
    let replay = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(replay.order.id, completed.order.id);
    assert_eq!(replay.payment.id, completed.payment.id);
}

#[tokio::test]
async fn checkout_requires_stock_availability() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Limited".to_owned(),
            variants: vec![CreateVariantInput {
                title: "One".to_owned(),
                manage_inventory: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 1_000))
        .await
        .unwrap();
    let inventory = InventoryService::new(db.clone());
    let location = inventory
        .create_location(CreateLocationInput {
            name: "Main".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(VariantId::from_string(variant_id.clone())),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, 1, "restock", None)
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
        .add_item(&cart.id, &VariantId::from_string(variant_id), 2, &context)
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
            name: "Standard".to_owned(),
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
    carts
        .set_shipping_method(&cart.id, &option.id, &context)
        .await
        .unwrap();

    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let payments = PaymentService::with_registry(db.clone(), providers);
    let workflow = CheckoutWorkflow::new(db, payments);
    let error = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            context,
        ))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("insufficient stock"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn expired_reservations_refund_and_fail_the_checkout() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();

    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Last unit".to_owned(),
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
            sku: Some("LAST-1".to_owned()),
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
            name: "Standard".to_owned(),
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
    let capture_payments = PaymentService::with_registry(db.clone(), providers.clone());
    let workflow = CheckoutWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    );

    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            context.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(checkout.status, CheckoutStatus::PaymentPending);
    assert_eq!(
        inventory
            .variant_availability(&VariantId::from_string(variant_id.clone()))
            .await
            .unwrap(),
        4
    );

    // The customer paid, but the reservation lease expired before placement.
    // Maintenance releases reservations independently of the checkout, which
    // is exactly the state that used to leave a captured payment attached to
    // a confirmed order with no stock movement and no refund.
    let payment_session = checkout.payment_session_id.clone().unwrap();
    let captured = capture_payments
        .capture(payment_session.as_str())
        .await
        .unwrap();
    assert_eq!(captured.status, PaymentState::Captured);
    inventory.release_checkout(&checkout.id).await.unwrap();

    let error = workflow.complete(&checkout.id, &context).await.unwrap_err();
    assert!(
        error.to_string().contains("reservation"),
        "unexpected error: {error}"
    );

    let failed = workflow.get(&checkout.id).await.unwrap();
    assert_eq!(failed.status, CheckoutStatus::Failed);
    assert!(failed.order_id.is_none());

    let order_status: String = sqlx::query_scalar("SELECT status FROM orders")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(order_status, "canceled");

    let payment_status: String = sqlx::query_scalar("SELECT status FROM payments LIMIT 1")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(payment_status, "refunded");

    let session_status: String =
        sqlx::query_scalar("SELECT status FROM payment_sessions WHERE id = ?")
            .bind(payment_session.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(session_status, "captured");

    let refunds: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM refunds")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(refunds, 1, "compensation must refund exactly once");

    // The reservation was released before completion, so no commit movement
    // exists and stock stays untouched rather than being decremented twice.
    let (stocked, reserved): (i64, i64) = sqlx::query_as(
        "SELECT stocked_quantity, reserved_quantity FROM inventory_levels \
         WHERE inventory_item_id = ? AND location_id = ?",
    )
    .bind(item.id.as_str())
    .bind(location.id.as_str())
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!((stocked, reserved), (5, 0));

    // A replay of the failed checkout must not create a second order or a
    // second refund.
    assert!(workflow.complete(&checkout.id, &context).await.is_err());
    let orders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(orders, 1);
    let refunds: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM refunds")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(refunds, 1);
}
