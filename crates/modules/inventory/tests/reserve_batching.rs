//! Integration tests for batched reservation allocation.
//!
//! `InventoryService::reserve` must keep the allocation order of the original
//! per-line implementation (`loc_default` first, then ascending location id),
//! must keep failing the whole batch when any line cannot be satisfied, and must
//! not issue a query per line.

use std::fmt::Debug;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::ThreadId;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, StoreId, VariantId, now_timestamp,
};
use fastcart_module_inventory::{
    CreateInventoryItemInput, InventoryService, Reference, ReservationLine, ReservationStatus,
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
        sqlx::query("SELECT 'reserve_marker' AS marker FROM inventory_items LIMIT 1")
            .fetch_all(db.pool())
            .await
            .unwrap();
        let records = recorder.records.lock().unwrap();
        records[before..]
            .iter()
            .find(|record| record.sql.contains("reserve_marker"))
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

/// Inserts a location with a fixed identifier so allocation order is testable.
async fn seed_location(db: &StoreDatabase, id: &str) {
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO inventory_locations (id, name, address, metadata, created_at, updated_at) \
         VALUES (?, ?, '{}', '{}', ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();
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

async fn stock(service: &InventoryService, item: &InventoryItemId, location: &str, quantity: i64) {
    service
        .adjust_stock(
            item,
            &InventoryLocationId::from_string(location),
            quantity,
            "receiving",
            None,
        )
        .await
        .unwrap();
}

/// The allocated `(item, location, quantity)` triples in allocation order.
fn allocation_triples(
    reservations: &[fastcart_module_inventory::InventoryReservation],
) -> Vec<(String, String, i64)> {
    reservations
        .iter()
        .map(|reservation| {
            (
                reservation.inventory_item_id.as_str().to_owned(),
                reservation.location_id.as_str().to_owned(),
                reservation.quantity,
            )
        })
        .collect()
}

#[tokio::test]
async fn allocation_spans_lines_and_locations_with_default_priority() {
    let (db, service) = setup().await;
    for id in ["loc_default", "loc_aa", "loc_bb"] {
        seed_location(&db, id).await;
    }
    let first_variant = VariantId::new();
    let second_variant = VariantId::new();
    let first_item = item_with_variant(&service, &first_variant).await;
    let second_item = item_with_variant(&service, &second_variant).await;
    stock(&service, &first_item, "loc_default", 1).await;
    stock(&service, &first_item, "loc_aa", 2).await;
    stock(&service, &first_item, "loc_bb", 3).await;
    stock(&service, &second_item, "loc_aa", 0).await;
    stock(&service, &second_item, "loc_bb", 5).await;

    let checkout = CheckoutId::new();
    let reservations = service
        .reserve(
            vec![
                ReservationLine::new(first_variant.clone(), 5),
                ReservationLine::new(second_variant.clone(), 4),
            ],
            Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();

    // The first line drains loc_default, then loc_aa, then loc_bb; the second
    // line skips the empty loc_aa level.
    assert_eq!(
        allocation_triples(&reservations),
        vec![
            (first_item.as_str().to_owned(), "loc_default".to_owned(), 1),
            (first_item.as_str().to_owned(), "loc_aa".to_owned(), 2),
            (first_item.as_str().to_owned(), "loc_bb".to_owned(), 2),
            (second_item.as_str().to_owned(), "loc_bb".to_owned(), 4),
        ]
    );
    for reservation in &reservations {
        assert_eq!(reservation.status, ReservationStatus::Reserved);
        assert_eq!(reservation.checkout_id.as_ref(), Some(&checkout));
    }
    assert_eq!(
        service
            .available_quantity(&first_item, &InventoryLocationId::from_string("loc_bb"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        service
            .available_quantity(&second_item, &InventoryLocationId::from_string("loc_bb"))
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn repeated_variant_lines_see_earlier_allocations() {
    let (db, service) = setup().await;
    for id in ["loc_aa", "loc_bb"] {
        seed_location(&db, id).await;
    }
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    stock(&service, &item, "loc_aa", 3).await;
    stock(&service, &item, "loc_bb", 2).await;

    let reservations = service
        .reserve(
            vec![
                ReservationLine::new(variant.clone(), 3),
                ReservationLine::new(variant.clone(), 2),
            ],
            Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap();
    assert_eq!(
        allocation_triples(&reservations),
        vec![
            (item.as_str().to_owned(), "loc_aa".to_owned(), 3),
            (item.as_str().to_owned(), "loc_bb".to_owned(), 2),
        ]
    );

    // The next line has nothing left to take.
    let error = service
        .reserve(
            vec![ReservationLine::new(variant, 1)],
            Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn shortage_on_a_later_line_rolls_the_whole_batch_back() {
    let (db, service) = setup().await;
    for id in ["loc_aa", "loc_bb"] {
        seed_location(&db, id).await;
    }
    let rich_variant = VariantId::new();
    let poor_variant = VariantId::new();
    let rich_item = item_with_variant(&service, &rich_variant).await;
    let poor_item = item_with_variant(&service, &poor_variant).await;
    stock(&service, &rich_item, "loc_aa", 5).await;
    stock(&service, &poor_item, "loc_aa", 1).await;
    stock(&service, &poor_item, "loc_bb", 1).await;

    let error = service
        .reserve(
            vec![
                ReservationLine::new(rich_variant, 3),
                ReservationLine::new(poor_variant.clone(), 3),
            ],
            Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppError::Conflict(message) if message.contains(poor_variant.as_str()))
    );

    // Nothing of the failed batch survives.
    assert_eq!(
        service
            .available_quantity(&rich_item, &InventoryLocationId::from_string("loc_aa"))
            .await
            .unwrap(),
        5
    );
    assert_eq!(
        service
            .available_quantity(&poor_item, &InventoryLocationId::from_string("loc_aa"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        service
            .available_quantity(&poor_item, &InventoryLocationId::from_string("loc_bb"))
            .await
            .unwrap(),
        1
    );
    let reservations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM inventory_reservations")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(reservations, 0);
}

#[tokio::test]
async fn unknown_variants_conflict_like_shortages() {
    let (db, service) = setup().await;
    let known_variant = VariantId::new();
    let unknown_variant = VariantId::new();
    let known_item = item_with_variant(&service, &known_variant).await;
    seed_location(&db, "loc_aa").await;
    stock(&service, &known_item, "loc_aa", 2).await;

    let error = service
        .reserve(
            vec![
                ReservationLine::new(known_variant, 1),
                ReservationLine::new(unknown_variant.clone(), 1),
            ],
            Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppError::Conflict(message) if message.contains(unknown_variant.as_str()))
    );
    assert_eq!(
        service
            .available_quantity(&known_item, &InventoryLocationId::from_string("loc_aa"))
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM inventory_reservations")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0
    );
}

/// Counts the statements matching `needle` while reserving one unit for each of
/// `line_count` variants, each stocked in two locations.
async fn reserve_statements(
    recorder: &StatementRecorder,
    line_count: usize,
    needle: &str,
) -> usize {
    let (db, service) = setup().await;
    seed_location(&db, "loc_aa").await;
    seed_location(&db, "loc_bb").await;
    let mut lines = Vec::with_capacity(line_count);
    for _ in 0..line_count {
        let variant = VariantId::new();
        let item = item_with_variant(&service, &variant).await;
        stock(&service, &item, "loc_aa", 1).await;
        stock(&service, &item, "loc_bb", 1).await;
        lines.push(ReservationLine::new(variant, 1));
    }
    let thread = StatementRecorder::worker_thread(&db).await;

    let checkpoint = recorder.checkpoint();
    let reservations = service
        .reserve(lines, Reference::new(CheckoutId::new()), 60)
        .await
        .unwrap();
    assert_eq!(reservations.len(), line_count);
    recorder.statements_since(checkpoint, thread, needle).len()
}

#[tokio::test]
async fn reserve_uses_a_constant_number_of_queries() {
    let recorder = StatementRecorder::global();

    let small_items = reserve_statements(&recorder, 2, "FROM inventory_items").await;
    let large_items = reserve_statements(&recorder, 8, "FROM inventory_items").await;
    assert_eq!(small_items, 1, "one item lookup for the whole batch");
    assert_eq!(large_items, small_items);

    let small_levels = reserve_statements(&recorder, 2, "FROM inventory_levels").await;
    let large_levels = reserve_statements(&recorder, 8, "FROM inventory_levels").await;
    assert_eq!(small_levels, 1, "one level lookup for the whole batch");
    assert_eq!(large_levels, small_levels);

    let inserts = reserve_statements(&recorder, 8, "INSERT INTO inventory_reservations").await;
    assert_eq!(inserts, 1, "one batched insert for all reservations");

    let updates = reserve_statements(&recorder, 8, "UPDATE inventory_levels").await;
    assert_eq!(updates, 8, "one conditional update per allocated item");
}
