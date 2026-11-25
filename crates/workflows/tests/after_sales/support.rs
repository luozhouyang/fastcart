use super::*;

pub(crate) struct Fixture {
    pub(crate) db: StoreDatabase,
    pub(crate) workflow: AfterSalesWorkflow,
    pub(crate) orders: OrderService,
    pub(crate) payments: PaymentService,
}

pub(crate) async fn fixture() -> Fixture {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let workflow = AfterSalesWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers.clone()),
    );
    Fixture {
        orders: OrderService::new(db.clone()),
        payments: PaymentService::with_registry(db.clone(), providers),
        db,
        workflow,
    }
}

pub(crate) async fn fixture_with_shipping(shipping: FulfillmentProviderRegistry) -> Fixture {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let workflow = AfterSalesWorkflow::with_shipping_providers(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers.clone()),
        shipping,
    );
    Fixture {
        orders: OrderService::new(db.clone()),
        payments: PaymentService::with_registry(db.clone(), providers),
        db,
        workflow,
    }
}

/// Fulfillment provider that records calls and can be made to fail.
pub(crate) struct TestProvider {
    pub(crate) id: String,
    pub(crate) calls: AtomicUsize,
    pub(crate) failing: AtomicBool,
}

impl TestProvider {
    pub(crate) fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            calls: AtomicUsize::new(0),
            failing: AtomicBool::new(false),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(crate) fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl FulfillmentProvider for TestProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn create_fulfillment(
        &self,
        request: CreateFulfillmentRequest,
    ) -> Result<FulfillmentReceipt, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failing.load(Ordering::SeqCst) {
            return Err(AppError::ExternalService(
                "test provider is unavailable".to_owned(),
            ));
        }
        Ok(FulfillmentReceipt {
            provider_id: self.id.clone(),
            provider_fulfillment_id: format!("test-{}", request.order_id),
            tracking_numbers: request.tracking_numbers,
            data: json!({}),
        })
    }

    async fn cancel_fulfillment(&self, _provider_fulfillment_id: &str) -> Result<(), AppError> {
        Ok(())
    }
}

pub(crate) async fn create_order(
    orders: &OrderService,
    quantity: i64,
    variant_id: Option<VariantId>,
    requires_shipping: bool,
) -> Order {
    orders
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity,
                variant_id,
                requires_shipping,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_000 * quantity,
            total: 1_000 * quantity,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

pub(crate) async fn capture_payment(payments: &PaymentService, order: &Order) -> Payment {
    let session = payments
        .create_session(CreatePaymentSessionInput {
            order_id: Some(order.id.clone()),
            provider: "manual".to_owned(),
            amount: order.total,
            currency_code: order.currency_code.clone(),
            ..CreatePaymentSessionInput::default()
        })
        .await
        .unwrap();
    payments.capture(session.id.as_str()).await.unwrap()
}

pub(crate) async fn requested_return(
    orders: &OrderService,
    order: &Order,
    quantity: i64,
) -> OrderReturn {
    let created = orders
        .create_return(
            &order.id,
            CreateReturnInput {
                reason: Some("damaged".to_owned()),
                refund_id: None,
                metadata: Metadata::empty(),
                items: vec![ReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    orders.request_return(&order.id, &created.id).await.unwrap()
}

pub(crate) async fn stock_order(
    db: &StoreDatabase,
    order: &Order,
    variant: &VariantId,
    stocked: i64,
    reserved: i64,
) -> (InventoryService, InventoryItemId, InventoryLocationId) {
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
            sku: None,
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, stocked, "receiving", None)
        .await
        .unwrap();
    let checkout = CheckoutId::new();
    inventory
        .reserve(
            vec![ReservationLine::new(variant.clone(), reserved)],
            ReservationReference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    inventory.commit(&checkout, &order.id).await.unwrap();
    (inventory, item.id, location.id)
}

pub(crate) fn restock_movements(
    movements: &[fastcart_module_inventory::InventoryMovement],
) -> Vec<&fastcart_module_inventory::InventoryMovement> {
    movements
        .iter()
        .filter(|movement| movement.reason == "return_restock")
        .collect()
}

pub(crate) async fn create_two_line_order(orders: &OrderService) -> Order {
    orders
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![
                CreateOrderItemInput {
                    product_title: "Widget A".to_owned(),
                    unit_price: 1_000,
                    quantity: 1,
                    ..CreateOrderItemInput::default()
                },
                CreateOrderItemInput {
                    product_title: "Widget B".to_owned(),
                    unit_price: 1_000,
                    quantity: 1,
                    ..CreateOrderItemInput::default()
                },
            ],
            subtotal: 2_000,
            total: 2_000,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

/// Stocks a replacement variant at the order's committed location so the
/// consumption flow finds it there.
pub(crate) async fn stock_replacement_variant(
    inventory: &InventoryService,
    order_id: &fastcart_kernel::OrderId,
    variant: &VariantId,
    location: &InventoryLocationId,
    stocked: i64,
) -> InventoryItemId {
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, location, stocked, "receiving", None)
        .await
        .unwrap();
    let checkout = CheckoutId::new();
    inventory
        .reserve(
            vec![ReservationLine::new(variant.clone(), 1)],
            ReservationReference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    inventory.commit(&checkout, order_id).await.unwrap();
    item.id
}
