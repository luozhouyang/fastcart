use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    CurrencyCode, CustomerId, LineItemId, Metadata, Pagination, ProductId, PromotionId,
    ShippingOptionId, StoreId, VariantId, now_timestamp,
};

use super::CartService;
use crate::domain::{
    AddItemInput, AddressInput, AdjustmentInput, Cart, CartStatus, CreateCartInput, DiscountInput,
    SetShippingMethodInput, TaxLineInput, UpdateItemInput,
};

async fn service() -> CartService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    CartService::new(db)
}

fn create_input(currency: &str) -> CreateCartInput {
    CreateCartInput::new(currency)
}

async fn create_cart(service: &CartService, currency: &str) -> Cart {
    service.create_cart(create_input(currency)).await.unwrap()
}

fn variant() -> VariantId {
    VariantId::new()
}

fn item_input(
    currency: &str,
    variant_id: VariantId,
    unit_price: i64,
    quantity: i64,
) -> AddItemInput {
    AddItemInput {
        product_id: ProductId::new(),
        variant_id,
        title: "Test product".to_owned(),
        variant_title: None,
        sku: Some("SKU-1".to_owned()),
        thumbnail: None,
        unit_price,
        currency_code: currency.to_owned(),
        quantity,
        is_tax_inclusive: false,
        requires_shipping: true,
        metadata: Metadata::empty(),
    }
}

fn shipping_input(currency: &str, amount: i64) -> SetShippingMethodInput {
    SetShippingMethodInput {
        shipping_option_id: ShippingOptionId::new(),
        name: "Standard".to_owned(),
        amount,
        currency_code: currency.to_owned(),
        is_tax_inclusive: false,
        data: serde_json::json!({ "provider": "test" }),
    }
}

fn adjustment(amount: i64) -> AdjustmentInput {
    AdjustmentInput {
        promotion_id: Some(PromotionId::new()),
        code: Some("PROMO".to_owned()),
        amount,
        description: Some("Line promotion".to_owned()),
    }
}

fn tax_line(amount: i64, inclusive: bool) -> TaxLineInput {
    TaxLineInput {
        name: "Sales tax".to_owned(),
        rate_bps: 1_000,
        amount,
        is_tax_inclusive: inclusive,
        cart_item_id: None,
        shipping_method_id: None,
    }
}

#[tokio::test]
async fn creates_and_gets_a_cart() {
    let service = service().await;
    let cart = create_cart(&service, "usd").await;

    assert_eq!(cart.currency_code.as_str(), "USD");
    assert_eq!(cart.status, CartStatus::Open);
    assert!(cart.region_id.is_none());
    assert!(cart.items.is_empty());
    assert_eq!(cart.totals.total, 0);
    assert!(cart.expires_at.is_some());

    let loaded = service.get_cart(&cart.id).await.unwrap();
    assert_eq!(loaded.id, cart.id);
}

#[tokio::test]
async fn persists_the_resolved_region_id() {
    let service = service().await;
    let cart = service
        .create_cart(CreateCartInput {
            region_id: Some("reg_test".to_owned()),
            ..create_input("USD")
        })
        .await
        .unwrap();

    assert_eq!(cart.region_id.as_deref(), Some("reg_test"));
    let loaded = service.get_cart(&cart.id).await.unwrap();
    assert_eq!(loaded.region_id.as_deref(), Some("reg_test"));

    // Carts without a region keep a NULL column and load as `None`.
    let legacy = create_cart(&service, "USD").await;
    let stored: Option<String> = sqlx::query_scalar("SELECT region_id FROM carts WHERE id = ?")
        .bind(legacy.id.as_str())
        .fetch_one(service.db.pool())
        .await
        .unwrap();
    assert!(stored.is_none());
}

#[tokio::test]
async fn adding_the_same_variant_increments_quantity() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let variant = variant();

    let cart = service
        .add_item(&cart.id, item_input("USD", variant.clone(), 1_000, 2))
        .await
        .unwrap();
    assert_eq!(cart.items.len(), 1);
    assert_eq!(cart.items[0].quantity, 2);

    let cart = service
        .add_item(&cart.id, item_input("USD", variant, 1_000, 3))
        .await
        .unwrap();
    assert_eq!(cart.items.len(), 1);
    assert_eq!(cart.items[0].quantity, 5);
    assert_eq!(cart.totals.item_subtotal, 5_000);
}

#[tokio::test]
async fn updates_removes_and_clears_items() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 500, 1))
        .await
        .unwrap();
    let item_id = cart.items[0].id.clone();

    let cart = service
        .update_item_quantity(&cart.id, &item_id, UpdateItemInput { quantity: 4 })
        .await
        .unwrap();
    assert_eq!(cart.items[0].quantity, 4);
    assert_eq!(cart.totals.item_subtotal, 2_000);

    let cart = service
        .update_item_quantity(&cart.id, &item_id, UpdateItemInput { quantity: 0 })
        .await
        .unwrap();
    assert!(cart.items.is_empty());

    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 100, 1))
        .await
        .unwrap();
    assert_eq!(cart.items.len(), 1);
    let cart = service.clear_items(&cart.id).await.unwrap();
    assert!(cart.items.is_empty());
    assert_eq!(cart.totals.total, 0);
}

#[tokio::test]
async fn rejects_a_mismatched_item_currency() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;

    let error = service
        .add_item(&cart.id, item_input("EUR", variant(), 1_000, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn validates_quantities_and_unknown_items() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;

    let error = service
        .add_item(&cart.id, item_input("USD", variant(), 100, 0))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));

    let missing = LineItemId::new();
    let error = service.remove_item(&cart.id, &missing).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::NotFound(_)));
}

#[tokio::test]
async fn calculates_totals_from_stored_snapshots() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 2))
        .await
        .unwrap();
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 500, 1))
        .await
        .unwrap();

    assert_eq!(cart.totals.item_subtotal, 2_500);
    assert_eq!(cart.totals.subtotal, 2_500);
    assert_eq!(cart.totals.discount_total, 0);
    assert_eq!(cart.totals.total, 2_500);
}

#[tokio::test]
async fn caps_line_adjustments_at_the_line_subtotal() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let item_id = cart.items[0].id.clone();

    let cart = service
        .add_item_adjustment(&cart.id, &item_id, adjustment(1_500))
        .await
        .unwrap();
    assert_eq!(cart.totals.item_discount_total, 1_000);
    assert_eq!(cart.totals.discount_total, 1_000);
    assert_eq!(cart.totals.total, 0);
}

#[tokio::test]
async fn caps_cart_discounts_at_the_item_subtotal() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();

    let cart = service
        .add_discount(
            &cart.id,
            DiscountInput {
                promotion_id: Some(PromotionId::new()),
                code: Some("SAVE".to_owned()),
                amount: 1_500,
                is_automatic: false,
                metadata: Metadata::empty(),
            },
        )
        .await
        .unwrap();

    assert_eq!(cart.discounts.len(), 1);
    assert_eq!(cart.totals.discount_total, 1_000);
    assert_eq!(cart.totals.total, 0);
}

#[tokio::test]
async fn setting_a_shipping_method_replaces_the_previous_selection() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();

    let cart = service
        .set_shipping_method(&cart.id, shipping_input("USD", 500))
        .await
        .unwrap();
    assert_eq!(cart.shipping_methods.len(), 1);
    assert_eq!(cart.totals.shipping_subtotal, 500);
    assert_eq!(cart.totals.shipping_total, 500);
    assert_eq!(cart.totals.total, 1_500);

    let cart = service
        .set_shipping_method(&cart.id, shipping_input("USD", 800))
        .await
        .unwrap();
    assert_eq!(cart.shipping_methods.len(), 1);
    assert_eq!(cart.shipping_methods[0].amount, 800);
    assert_eq!(cart.totals.total, 1_800);
}

#[tokio::test]
async fn shipping_adjustments_reduce_the_shipping_total() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let cart = service
        .set_shipping_method(&cart.id, shipping_input("USD", 500))
        .await
        .unwrap();

    let cart = service
        .replace_shipping_adjustments(&cart.id, vec![adjustment(200)])
        .await
        .unwrap();
    assert_eq!(cart.shipping_adjustments.len(), 1);
    assert_eq!(cart.totals.shipping_discount_total, 200);
    assert_eq!(cart.totals.shipping_total, 300);
    assert_eq!(cart.totals.total, 1_300);
}

#[tokio::test]
async fn inclusive_tax_is_informational_and_not_added_to_the_total() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let cart = service
        .replace_tax_lines(&cart.id, vec![tax_line(100, true)])
        .await
        .unwrap();

    assert_eq!(cart.tax_lines.len(), 1);
    assert_eq!(cart.totals.tax_total, 100);
    assert_eq!(cart.totals.total, 1_000);
}

#[tokio::test]
async fn exclusive_tax_is_added_to_the_total() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let cart = service
        .replace_tax_lines(&cart.id, vec![tax_line(100, false), tax_line(50, true)])
        .await
        .unwrap();

    assert_eq!(cart.totals.tax_total, 150);
    assert_eq!(cart.totals.total, 1_100);
}

#[tokio::test]
async fn stores_shipping_and_billing_addresses_separately() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;

    let shipping = AddressInput {
        first_name: Some("Ada".to_owned()),
        city: Some("London".to_owned()),
        country_code: Some("gb".to_owned()),
        ..AddressInput::default()
    };
    let billing = AddressInput {
        first_name: Some("Grace".to_owned()),
        city: Some("Arlington".to_owned()),
        country_code: Some("us".to_owned()),
        ..AddressInput::default()
    };

    let cart = service
        .set_shipping_address(&cart.id, shipping)
        .await
        .unwrap();
    let cart = service
        .set_billing_address(&cart.id, billing)
        .await
        .unwrap();

    assert_eq!(cart.shipping_address.first_name.as_deref(), Some("Ada"));
    assert_eq!(cart.shipping_address.country_code.as_deref(), Some("GB"));
    assert_eq!(cart.billing_address.first_name.as_deref(), Some("Grace"));
    assert_eq!(cart.billing_address.country_code.as_deref(), Some("US"));
}

#[tokio::test]
async fn attaching_a_different_customer_conflicts() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let first = CustomerId::new();
    let second = CustomerId::new();

    let cart = service
        .attach_customer(&cart.id, &first, Some("One@Example.com".to_owned()))
        .await
        .unwrap();
    assert_eq!(cart.customer_id.as_ref(), Some(&first));
    assert_eq!(cart.email.as_deref(), Some("one@example.com"));

    let error = service
        .attach_customer(&cart.id, &second, None)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn completes_and_expires_carts_and_rejects_mutations() {
    let service = service().await;
    let open = create_cart(&service, "USD").await;
    let completed = service.complete_cart(&open.id).await.unwrap();
    assert_eq!(completed.status, CartStatus::Completed);
    assert!(completed.completed_at.is_some());

    let error = service
        .add_item(&open.id, item_input("USD", variant(), 100, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let expiring = service
        .create_cart(CreateCartInput {
            expires_in_seconds: Some(10),
            ..create_input("USD")
        })
        .await
        .unwrap();
    let count = service.expire_due(i64::MAX).await.unwrap();
    assert_eq!(count, 1);

    let expired = service.get_cart(&expiring.id).await.unwrap();
    assert_eq!(expired.status, CartStatus::Expired);
    let error = service
        .add_item(&expiring.id, item_input("USD", variant(), 100, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn lists_carts_for_a_customer() {
    let service = service().await;
    let customer = CustomerId::new();
    let other = CustomerId::new();

    let first = service
        .create_cart(CreateCartInput {
            customer_id: Some(customer.clone()),
            ..create_input("USD")
        })
        .await
        .unwrap();
    service
        .add_item(&first.id, item_input("USD", variant(), 250, 2))
        .await
        .unwrap();
    service
        .create_cart(CreateCartInput {
            customer_id: Some(customer.clone()),
            ..create_input("EUR")
        })
        .await
        .unwrap();
    service
        .create_cart(CreateCartInput {
            customer_id: Some(other),
            ..create_input("USD")
        })
        .await
        .unwrap();

    let page = service
        .list_carts(Some(&customer), Pagination::default())
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 2);
    let with_items = page
        .data
        .iter()
        .find(|cart| !cart.items.is_empty())
        .unwrap();
    assert_eq!(with_items.totals.item_subtotal, 500);
}

#[tokio::test]
async fn finds_newest_active_cart_for_customer() {
    let service = service().await;
    let customer = CustomerId::new();
    let other = CustomerId::new();

    let first = service
        .create_cart(CreateCartInput {
            customer_id: Some(customer.clone()),
            ..create_input("USD")
        })
        .await
        .unwrap();
    let second = service
        .create_cart(CreateCartInput {
            customer_id: Some(customer.clone()),
            ..create_input("EUR")
        })
        .await
        .unwrap();
    let other_cart = service
        .create_cart(CreateCartInput {
            customer_id: Some(other.clone()),
            ..create_input("USD")
        })
        .await
        .unwrap();

    // Creation timestamps can collide within one clock tick; pin them so
    // the newest-cart ordering is deterministic.
    for (cart_id, created_at) in [
        (&first.id, 1_i64),
        (&second.id, 2_i64),
        (&other_cart.id, 3_i64),
    ] {
        sqlx::query("UPDATE carts SET created_at = ? WHERE id = ?")
            .bind(created_at)
            .bind(cart_id.as_str())
            .execute(service.db.pool())
            .await
            .unwrap();
    }

    let active = service
        .active_cart_for_customer(&customer)
        .await
        .unwrap()
        .expect("customer has an active cart");
    assert_eq!(active.id, second.id);

    // Completing the newest cart falls back to the older open one.
    service.complete_cart(&second.id).await.unwrap();
    let active = service
        .active_cart_for_customer(&customer)
        .await
        .unwrap()
        .expect("older cart is still open");
    assert_eq!(active.id, first.id);

    // Another customer's carts are never returned.
    let active_other = service
        .active_cart_for_customer(&other)
        .await
        .unwrap()
        .expect("other customer has an active cart");
    assert_eq!(active_other.id, other_cart.id);

    // With no open carts the result is None.
    service.complete_cart(&first.id).await.unwrap();
    assert!(
        service
            .active_cart_for_customer(&customer)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .active_cart_for_customer(&CustomerId::new())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn reprices_items_and_replaces_adjustments() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let item_id = cart.items[0].id.clone();

    let cart = service
        .set_item_unit_price(&cart.id, &item_id, 750, "usd")
        .await
        .unwrap();
    assert_eq!(cart.items[0].unit_price, 750);

    let cart = service
        .replace_item_adjustments(&cart.id, &item_id, vec![adjustment(100)])
        .await
        .unwrap();
    assert_eq!(cart.items[0].adjustments.len(), 1);
    assert_eq!(cart.totals.item_discount_total, 100);

    let cart = service
        .replace_item_adjustments(&cart.id, &item_id, vec![adjustment(50)])
        .await
        .unwrap();
    assert_eq!(cart.items[0].adjustments.len(), 1);
    assert_eq!(cart.totals.item_discount_total, 50);

    let error = service
        .set_item_unit_price(&cart.id, &item_id, 100, "EUR")
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn replaces_and_removes_discounts() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let promotion = PromotionId::new();

    let cart = service
        .replace_discounts(
            &cart.id,
            vec![DiscountInput {
                promotion_id: Some(promotion.clone()),
                code: Some("SAVE10".to_owned()),
                amount: 100,
                is_automatic: false,
                metadata: Metadata::empty(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(cart.discounts.len(), 1);
    assert_eq!(cart.totals.total, 900);

    let cart = service.remove_discount(&cart.id, &promotion).await.unwrap();
    assert!(cart.discounts.is_empty());
    assert_eq!(cart.totals.total, 1_000);

    let error = service
        .remove_discount(&cart.id, &promotion)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::NotFound(_)));
}

#[tokio::test]
async fn recalculate_returns_current_totals() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 300, 3))
        .await
        .unwrap();

    let totals = service.recalculate(&cart.id).await.unwrap();
    assert_eq!(totals.item_subtotal, 900);
    assert_eq!(totals.currency_code, CurrencyCode::new("USD").unwrap());

    let missing = fastcart_kernel::CartId::new();
    assert!(service.recalculate(&missing).await.is_err());
}

#[tokio::test]
async fn attach_customer_never_overwrites_a_concurrent_owner() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let first = CustomerId::new();
    let second = CustomerId::new();

    // Simulate another writer attaching a customer after this request
    // loaded the cart but before it updated it.
    sqlx::query("UPDATE carts SET customer_id = ? WHERE id = ?")
        .bind(first.as_str())
        .bind(cart.id.as_str())
        .execute(service.db.pool())
        .await
        .unwrap();

    let error = service
        .attach_customer(&cart.id, &second, None)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let stored = service.get_cart(&cart.id).await.unwrap();
    assert_eq!(stored.customer_id.as_ref(), Some(&first));
}

#[tokio::test]
async fn add_item_on_a_completed_cart_is_rejected() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    service.complete_cart(&cart.id).await.unwrap();

    let error = service
        .add_item(&cart.id, item_input("USD", variant(), 100, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let stored = service.get_cart(&cart.id).await.unwrap();
    assert!(stored.items.is_empty());
}

#[tokio::test]
async fn changing_shipping_method_clears_stale_shipping_tax_lines() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let item_id = cart.items[0].id.clone();

    let first_shipping = shipping_input("USD", 500);
    let first_option = first_shipping.shipping_option_id.clone();
    let cart = service
        .set_shipping_method(&cart.id, first_shipping)
        .await
        .unwrap();

    let cart = service
        .replace_tax_lines(
            &cart.id,
            vec![
                TaxLineInput {
                    name: "Item tax".to_owned(),
                    rate_bps: 1_000,
                    amount: 100,
                    is_tax_inclusive: false,
                    cart_item_id: Some(item_id),
                    shipping_method_id: None,
                },
                TaxLineInput {
                    name: "Shipping tax".to_owned(),
                    rate_bps: 1_000,
                    amount: 50,
                    is_tax_inclusive: false,
                    cart_item_id: None,
                    shipping_method_id: Some(first_option.to_string()),
                },
            ],
        )
        .await
        .unwrap();
    assert_eq!(cart.tax_lines.len(), 2);

    let cart = service
        .set_shipping_method(&cart.id, shipping_input("USD", 700))
        .await
        .unwrap();
    assert_eq!(cart.tax_lines.len(), 1);
    assert_eq!(cart.tax_lines[0].name, "Item tax");
    assert!(
        cart.tax_lines
            .iter()
            .all(|line| line.shipping_method_id.is_none())
    );
}

#[tokio::test]
async fn applying_the_same_discount_twice_updates_one_row() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let promotion = PromotionId::new();
    let discount = |amount| DiscountInput {
        promotion_id: Some(promotion.clone()),
        code: Some("SAVE".to_owned()),
        amount,
        is_automatic: false,
        metadata: Metadata::empty(),
    };

    let cart = service.add_discount(&cart.id, discount(100)).await.unwrap();
    assert_eq!(cart.discounts.len(), 1);

    let cart = service.add_discount(&cart.id, discount(250)).await.unwrap();
    assert_eq!(cart.discounts.len(), 1);
    assert_eq!(cart.discounts[0].amount, 250);
    assert_eq!(cart.totals.discount_total, 250);
}

#[tokio::test]
async fn an_expired_but_unswept_cart_is_not_active() {
    let service = service().await;
    let customer = CustomerId::new();
    let cart = service
        .create_cart(CreateCartInput {
            customer_id: Some(customer.clone()),
            ..create_input("USD")
        })
        .await
        .unwrap();

    sqlx::query("UPDATE carts SET expires_at = ? WHERE id = ?")
        .bind(now_timestamp() - 1)
        .bind(cart.id.as_str())
        .execute(service.db.pool())
        .await
        .unwrap();

    assert!(
        service
            .active_cart_for_customer(&customer)
            .await
            .unwrap()
            .is_none()
    );

    let error = service
        .add_item(&cart.id, item_input("USD", variant(), 100, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn malformed_stored_address_snapshot_is_a_database_error() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;

    sqlx::query("UPDATE carts SET shipping_address = ? WHERE id = ?")
        .bind("{not json")
        .bind(cart.id.as_str())
        .execute(service.db.pool())
        .await
        .unwrap();

    let error = service.get_cart(&cart.id).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Database(_)));
}
