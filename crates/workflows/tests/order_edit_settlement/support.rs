use super::*;

pub(crate) struct Fixture {
    pub(crate) db: StoreDatabase,
    pub(crate) workflow: AfterSalesWorkflow,
    pub(crate) orders: OrderService,
    pub(crate) inventory: InventoryService,
    pub(crate) payments: PaymentService,
    pub(crate) variant_id: VariantId,
    pub(crate) item_id: fastcart_kernel::InventoryItemId,
    pub(crate) location_id: fastcart_kernel::InventoryLocationId,
}

pub(crate) async fn fixture() -> Fixture {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
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
    let variant_id = VariantId::new();
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(variant_id.clone()),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, STOCK, "restock", None)
        .await
        .unwrap();
    Fixture {
        db: db.clone(),
        workflow: AfterSalesWorkflow::new(
            db.clone(),
            PaymentService::with_registry(db.clone(), providers.clone()),
        ),
        orders: OrderService::new(db.clone()),
        inventory,
        payments: PaymentService::with_registry(db, providers),
        variant_id,
        item_id: item.id,
        location_id: location.id,
    }
}

/// Places a one-line order for `quantity` units of the fixture variant.
///
/// The order is created directly, without a checkout, so it holds no committed
/// reservation until [`commit_stock`] runs. That keeps the two inventory
/// sources — a real checkout and a manual order — testable.
pub(crate) async fn order(fixture: &Fixture, quantity: i64, unit_price: i64) -> Order {
    let order = fixture
        .orders
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price,
                quantity,
                variant_id: Some(fixture.variant_id.clone()),
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            subtotal: unit_price * quantity,
            total: unit_price * quantity,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    // Confirmed orders are the ones an admin can fulfil and edit; leave the
    // pending state to the order module's own tests.
    fixture.orders.confirm(&order.id).await.unwrap()
}

/// Takes the order's units out of stock the way checkout does.
pub(crate) async fn commit_stock(fixture: &Fixture, order: &Order, quantity: i64) {
    let checkout = CheckoutId::new();
    let reservations = fixture
        .inventory
        .reserve(
            vec![ReservationLine::new(fixture.variant_id.clone(), quantity)],
            ReservationReference::new(checkout.clone()),
            3_600,
        )
        .await
        .unwrap();
    assert_eq!(reservations.len(), 1);
    fixture
        .inventory
        .commit(&checkout, &order.id)
        .await
        .unwrap();
}

pub(crate) async fn level(fixture: &Fixture) -> i64 {
    fixture
        .inventory
        .get_level(&fixture.item_id, &fixture.location_id)
        .await
        .map(|level| level.stocked_quantity)
        .unwrap_or(0)
}

pub(crate) fn edit(
    action: OrderEditAction,
    order_item_id: Option<fastcart_kernel::LineItemId>,
    before: Option<i64>,
    after: i64,
) -> OrderEditItemInput {
    OrderEditItemInput {
        order_item_id,
        action,
        product_id: None,
        variant_id: None,
        product_title: if action == OrderEditAction::Add {
            Some("Extra".to_owned())
        } else {
            None
        },
        variant_title: None,
        sku: None,
        unit_price: if action == OrderEditAction::Add {
            Some(500)
        } else {
            None
        },
        before_quantity: before,
        after_quantity: after,
        requires_shipping: Some(true),
        is_tax_inclusive: None,
    }
}

pub(crate) fn edited(order: &Order, index: usize, after_quantity: i64) -> CreateOrderEditInput {
    CreateOrderEditInput {
        reason: None,
        metadata: Metadata::default(),
        items: vec![edit(
            OrderEditAction::Update,
            Some(order.items[index].id.clone()),
            Some(order.items[index].quantity),
            after_quantity,
        )],
    }
}

/// Captures the whole order through the sandbox provider, as checkout would.
pub(crate) async fn pay(fixture: &Fixture, order: &Order) {
    let session = fixture
        .payments
        .create_session(CreatePaymentSessionInput {
            order_id: Some(order.id.clone()),
            provider: "manual".to_owned(),
            amount: order.total,
            currency_code: order.currency_code.clone(),
            ..CreatePaymentSessionInput::default()
        })
        .await
        .unwrap();
    fixture.payments.capture(session.id.as_str()).await.unwrap();
}
