//! Integration tests for batched return restocking and stock consumption.
//!
//! Multi-line restocks and consumptions must keep their exact quantities and
//! their idempotency: replaying a line never moves stock twice, raising a
//! received quantity only restocks the difference, and a shortage rolls the
//! whole batch back. The item lookup is batched, so the query count does not
//! grow with the number of lines.

use std::fmt::Debug;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::ThreadId;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, OrderId, ReturnId,
    StoreId, VariantId,
};
use fastcart_module_inventory::{
    CreateInventoryItemInput, ExchangeRestockLine, InventoryService, Reference, ReservationLine,
    ReturnRestockLine, StockConsumption, return_restock_reference,
};

/// Records the SQL text of every statement sqlx executes, tagged with the
/// thread that executed it.
///
/// sqlx logs one `tracing` event per statement from the worker thread that owns
/// the connection, and `tracing` only emits those events while a global
/// subscriber is installed, so the recorder installs itself process-wide the
/// first time it is used. Tests running in parallel in this binary are told
/// apart by the worker thread of their own pool, which the recorder discovers
/// through a marker query.
#[derive(Clone, Default)]
struct StatementRecorder {
    records: Arc<Mutex<Vec<Statement>>>,
}

struct Statement {
    thread: ThreadId,
    sql: String,
}

impl StatementRecorder {
    /// Returns the process-wide recorder, installing it on first use.
    fn global() -> Self {
        static RECORDER: OnceLock<StatementRecorder> = OnceLock::new();
        RECORDER
            .get_or_init(|| {
                let recorder = StatementRecorder::default();
                tracing::subscriber::set_global_default(recorder.clone())
                    .expect("the statement recorder must be the global subscriber");
                recorder
            })
            .clone()
    }

    /// Runs a marker query and returns the worker thread that executed it.
    ///
    /// The marker is unique to this test, so the thread logged for it is the
    /// thread that runs every later statement of this test's pool.
    async fn worker_thread(db: &StoreDatabase) -> ThreadId {
        let recorder = Self::global();
        let before = recorder.records.lock().unwrap().len();
        sqlx::query("SELECT 'restock_marker' AS marker FROM inventory_items LIMIT 1")
            .fetch_all(db.pool())
            .await
            .unwrap();
        let records = recorder.records.lock().unwrap();
        records[before..]
            .iter()
            .find(|record| record.sql.contains("restock_marker"))
            .map(|record| record.thread)
            .expect("the marker statement must be logged")
    }

    /// Returns a snapshot length usable with [`Self::statements_since`].
    fn checkpoint(&self) -> usize {
        self.records.lock().unwrap().len()
    }

    /// Returns the statements recorded after `checkpoint` that ran on `thread`
    /// and contain `needle`.
    fn statements_since(&self, checkpoint: usize, thread: ThreadId, needle: &str) -> Vec<String> {
        self.records.lock().unwrap()[checkpoint..]
            .iter()
            .filter(|record| record.thread == thread && record.sql.contains(needle))
            .map(|record| record.sql.clone())
            .collect()
    }
}

impl tracing::Subscriber for StatementRecorder {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        if event.metadata().target() != "sqlx::query" {
            return;
        }
        let mut visitor = StatementVisitor::default();
        event.record(&mut visitor);
        if let Some(sql) = visitor.statement {
            self.records.lock().unwrap().push(Statement {
                thread: std::thread::current().id(),
                sql,
            });
        }
    }

    fn enter(&self, _span: &tracing::span::Id) {}

    fn exit(&self, _span: &tracing::span::Id) {}
}

#[derive(Default)]
struct StatementVisitor {
    statement: Option<String>,
}

impl tracing::field::Visit for StatementVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "db.statement" {
            self.statement = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn Debug) {
        if field.name() == "db.statement" {
            self.statement = Some(format!("{value:?}"));
        }
    }
}

async fn setup() -> (StoreDatabase, InventoryService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = InventoryService::new(db.clone());
    (db, service)
}

async fn item_with_variant(service: &InventoryService, variant: &VariantId) -> InventoryItemId {
    service
        .create_item(CreateInventoryItemInput {
            sku: Some(format!("sku-{}", variant.as_str())),
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap()
        .id
}

async fn location(service: &InventoryService, name: &str) -> InventoryLocationId {
    service
        .create_location(fastcart_module_inventory::CreateLocationInput {
            name: name.to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap()
        .id
}

async fn stock(
    service: &InventoryService,
    item: &InventoryItemId,
    location: &InventoryLocationId,
    quantity: i64,
) {
    service
        .adjust_stock(item, location, quantity, "receiving", None)
        .await
        .unwrap();
}

async fn available(
    service: &InventoryService,
    item: &InventoryItemId,
    location: &InventoryLocationId,
) -> i64 {
    service.available_quantity(item, location).await.unwrap()
}

#[tokio::test]
async fn return_restock_moves_multi_line_quantities_once() {
    let (_db, service) = setup().await;
    let main = location(&service, "Main").await;
    let first_variant = VariantId::new();
    let second_variant = VariantId::new();
    let first_item = item_with_variant(&service, &first_variant).await;
    let second_item = item_with_variant(&service, &second_variant).await;
    stock(&service, &first_item, &main, 10).await;
    stock(&service, &second_item, &main, 10).await;

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    service
        .reserve(
            vec![
                ReservationLine::new(first_variant.clone(), 3),
                ReservationLine::new(second_variant.clone(), 2),
            ],
            Reference::new(checkout.clone()),
            600,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order).await.unwrap();
    assert_eq!(available(&service, &first_item, &main).await, 7);
    assert_eq!(available(&service, &second_item, &main).await, 8);

    let return_id = ReturnId::new();
    let first_line_item = LineItemId::new();
    let second_line_item = LineItemId::new();
    let unknown_line_item = LineItemId::new();
    let lines = vec![
        ReturnRestockLine::new(first_line_item.clone(), first_variant.clone(), 1, 2),
        ReturnRestockLine::new(second_line_item.clone(), second_variant.clone(), 4, 4),
        // Unknown variants are ignored.
        ReturnRestockLine::new(unknown_line_item, VariantId::new(), 5, 5),
    ];
    assert_eq!(
        service
            .restock_return(&order, &return_id, &lines)
            .await
            .unwrap(),
        6
    );
    assert_eq!(available(&service, &first_item, &main).await, 9);
    assert_eq!(available(&service, &second_item, &main).await, 12);

    // Replaying the same receipt moves nothing.
    assert_eq!(
        service
            .restock_return(&order, &return_id, &lines)
            .await
            .unwrap(),
        0
    );
    assert_eq!(available(&service, &first_item, &main).await, 9);
    assert_eq!(available(&service, &second_item, &main).await, 12);

    // Raising the received quantity of one line restocks only the difference.
    let raised = ReturnRestockLine::new(first_line_item.clone(), first_variant.clone(), 5, 5);
    assert_eq!(
        service
            .restock_return(&order, &return_id, std::slice::from_ref(&raised))
            .await
            .unwrap(),
        3
    );
    assert_eq!(available(&service, &first_item, &main).await, 12);
    assert_eq!(
        service
            .restock_return(&order, &return_id, std::slice::from_ref(&raised))
            .await
            .unwrap(),
        0
    );

    let movements = service.list_movements(&first_item).await.unwrap();
    let restocks = movements
        .iter()
        .filter(|movement| movement.reason == "return_restock")
        .map(|movement| {
            (
                movement.quantity_delta,
                movement.reference.clone().unwrap_or_default(),
                movement.location_id.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        restocks,
        vec![
            (
                2,
                return_restock_reference(&return_id, &first_line_item, 2),
                main.clone(),
            ),
            (
                3,
                return_restock_reference(&return_id, &first_line_item, 5),
                main.clone(),
            ),
        ]
    );
    let second_movements = service.list_movements(&second_item).await.unwrap();
    assert_eq!(
        second_movements
            .iter()
            .filter(|movement| movement.reason == "return_restock")
            .map(|movement| movement.quantity_delta)
            .collect::<Vec<_>>(),
        vec![4]
    );
}

#[tokio::test]
async fn return_restock_falls_back_to_the_default_location_per_line() {
    let (db, service) = setup().await;
    let first_variant = VariantId::new();
    let second_variant = VariantId::new();
    let first_item = item_with_variant(&service, &first_variant).await;
    let second_item = item_with_variant(&service, &second_variant).await;
    let order = OrderId::new();
    let return_id = ReturnId::new();

    let lines = vec![
        ReturnRestockLine::new(LineItemId::new(), first_variant.clone(), 3, 3),
        ReturnRestockLine::new(LineItemId::new(), second_variant.clone(), 2, 2),
    ];
    assert_eq!(
        service
            .restock_return(&order, &return_id, &lines)
            .await
            .unwrap(),
        5
    );
    let default = InventoryLocationId::from_string("loc_default");
    assert_eq!(available(&service, &first_item, &default).await, 3);
    assert_eq!(available(&service, &second_item, &default).await, 2);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM inventory_locations")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1,
        "the default location is created once for the whole batch"
    );
}

#[tokio::test]
async fn consumption_moves_multi_line_quantities_once() {
    let (_db, service) = setup().await;
    let main = location(&service, "Main").await;
    let first_variant = VariantId::new();
    let second_variant = VariantId::new();
    let first_item = item_with_variant(&service, &first_variant).await;
    let second_item = item_with_variant(&service, &second_variant).await;
    stock(&service, &first_item, &main, 5).await;
    stock(&service, &second_item, &main, 5).await;

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    service
        .reserve(
            vec![
                ReservationLine::new(first_variant.clone(), 2),
                ReservationLine::new(second_variant.clone(), 1),
            ],
            Reference::new(checkout.clone()),
            600,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order).await.unwrap();

    let lines = vec![
        StockConsumption::new(first_variant.clone(), 2, "replacement:1"),
        StockConsumption::new(second_variant.clone(), 1, "replacement:2"),
        // An unknown variant fails the whole batch, like a shortage.
        StockConsumption::new(VariantId::new(), 1, "replacement:3"),
    ];
    let error = service.consume_stock(&order, &lines).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    assert_eq!(available(&service, &first_item, &main).await, 3);
    assert_eq!(available(&service, &second_item, &main).await, 4);

    let lines = &lines[..2];
    assert_eq!(
        service.consume_stock(&order, lines).await.unwrap(),
        3,
        "both lines consume their quantities"
    );
    assert_eq!(available(&service, &first_item, &main).await, 1);
    assert_eq!(available(&service, &second_item, &main).await, 3);

    // Replaying the same references consumes nothing more.
    assert_eq!(service.consume_stock(&order, lines).await.unwrap(), 0);
    assert_eq!(available(&service, &first_item, &main).await, 1);
    assert_eq!(available(&service, &second_item, &main).await, 3);

    // A shortage on any line rolls the whole batch back.
    let shortage = [
        StockConsumption::new(first_variant.clone(), 1, "replacement:4"),
        StockConsumption::new(second_variant.clone(), 99, "replacement:5"),
    ];
    assert!(matches!(
        service.consume_stock(&order, &shortage).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(available(&service, &first_item, &main).await, 1);
    assert_eq!(available(&service, &second_item, &main).await, 3);
}

#[tokio::test]
async fn exchange_restock_moves_multi_line_quantities_once() {
    let (_db, service) = setup().await;
    let main = location(&service, "Main").await;
    let first_variant = VariantId::new();
    let second_variant = VariantId::new();
    let first_item = item_with_variant(&service, &first_variant).await;
    let second_item = item_with_variant(&service, &second_variant).await;
    stock(&service, &first_item, &main, 2).await;
    stock(&service, &second_item, &main, 1).await;

    let order = sale_order(&service, &first_variant, 1).await;
    assert_eq!(available(&service, &first_item, &main).await, 1);
    let lines = vec![
        ExchangeRestockLine::new("inbound_1", first_variant.clone(), 2),
        ExchangeRestockLine::new("inbound_2", second_variant.clone(), 3),
    ];
    assert_eq!(
        service
            .restock_exchange(&order, "exch_1", &lines)
            .await
            .unwrap(),
        5
    );
    // The sold variant restocks at its committed location; the variant without
    // a committed reservation falls back to the default location.
    let default = InventoryLocationId::from_string("loc_default");
    assert_eq!(available(&service, &first_item, &main).await, 3);
    assert_eq!(available(&service, &second_item, &main).await, 1);
    assert_eq!(available(&service, &second_item, &default).await, 3);

    assert_eq!(
        service
            .restock_exchange(&order, "exch_1", &lines)
            .await
            .unwrap(),
        0
    );
    assert_eq!(available(&service, &first_item, &main).await, 3);
    assert_eq!(available(&service, &second_item, &default).await, 3);
}

/// Commits one unit of `variant` at `loc_default` for a fresh order.
async fn sale_order(service: &InventoryService, variant: &VariantId, quantity: i64) -> OrderId {
    let checkout = CheckoutId::new();
    let order = OrderId::new();
    service
        .reserve(
            vec![ReservationLine::new(variant.clone(), quantity)],
            Reference::new(checkout.clone()),
            600,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order).await.unwrap();
    order
}

/// Counts the statements matching `needle` while restocking `line_count`
/// return lines.
async fn restock_statements(
    recorder: &StatementRecorder,
    line_count: usize,
    needle: &str,
) -> usize {
    let (db, service) = setup().await;
    let mut lines = Vec::with_capacity(line_count);
    for _ in 0..line_count {
        let variant = VariantId::new();
        item_with_variant(&service, &variant).await;
        lines.push(ReturnRestockLine::new(LineItemId::new(), variant, 1, 1));
    }
    let thread = StatementRecorder::worker_thread(&db).await;

    let checkpoint = recorder.checkpoint();
    let restocked = service
        .restock_return(&OrderId::new(), &ReturnId::new(), &lines)
        .await
        .unwrap();
    assert_eq!(restocked, line_count as i64);
    recorder.statements_since(checkpoint, thread, needle).len()
}

#[tokio::test]
async fn restock_lookups_do_not_grow_with_the_batch() {
    let recorder = StatementRecorder::global();

    let small = restock_statements(&recorder, 2, "FROM inventory_items").await;
    let large = restock_statements(&recorder, 8, "FROM inventory_items").await;
    assert_eq!(small, 1, "one item lookup for the whole batch");
    assert_eq!(large, small);
}
