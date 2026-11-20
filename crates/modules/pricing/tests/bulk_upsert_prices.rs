//! Integration tests for `PricingService::bulk_upsert_prices`.
//!
//! The batch upsert must return the stored prices in the order of the requested
//! scopes, with every scope field intact, and it must read the batch back in a
//! constant number of queries instead of one query per price.

use std::fmt::Debug;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::ThreadId;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CurrencyCode, Pagination, StoreId, now_timestamp};
use fastcart_module_pricing::{CreatePriceInput, Price, PriceFilter, PricingService};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata as TracingMetadata, Subscriber};

/// Records the SQL text of every statement sqlx executes, tagged with the
/// thread that executed it.
///
/// sqlx logs one `tracing` event per statement from the worker thread that owns
/// the connection, and `tracing` only emits those events while a global
/// subscriber is installed, so the recorder installs itself process-wide the
/// first time it is used. Tests running in the same binary are told apart by
/// the worker thread of their own pool, which the recorder discovers through a
/// marker query.
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
    /// The marker is unique to this test, so the worker thread logged for it is
    /// the thread that runs every later statement of this test's pool.
    async fn worker_thread(db: &StoreDatabase) -> ThreadId {
        let recorder = Self::global();
        let before = recorder.records.lock().unwrap().len();
        sqlx::query("SELECT 'bulk_upsert_marker' AS marker FROM prices LIMIT 1")
            .fetch_all(db.pool())
            .await
            .unwrap();
        let records = recorder.records.lock().unwrap();
        records[before..]
            .iter()
            .find(|record| record.sql.contains("bulk_upsert_marker"))
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

impl Subscriber for StatementRecorder {
    fn enabled(&self, _metadata: &TracingMetadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
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

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[derive(Default)]
struct StatementVisitor {
    statement: Option<String>,
}

impl Visit for StatementVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "db.statement" {
            self.statement = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "db.statement" {
            self.statement = Some(format!("{value:?}"));
        }
    }
}

/// The statement text every read-back of a single price uses.
const READ_BACK_NEEDLE: &str = "FROM prices WHERE id ";

async fn service() -> (StoreDatabase, PricingService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PricingService::new(db.clone());
    (db, service)
}

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn eur() -> CurrencyCode {
    CurrencyCode::new("EUR").unwrap()
}

async fn seed_customer_group(db: &StoreDatabase, id: &str) {
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO customer_groups (id, name, metadata, created_at, updated_at) \
         VALUES (?, ?, '{}', ?, ?)",
    )
    .bind(id)
    .bind(format!("Group {id}"))
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();
}

/// Asserts that one returned price carries exactly the requested scope.
fn assert_scope(price: &Price, input: &CreatePriceInput) {
    assert_eq!(price.variant_id, input.variant_id);
    assert_eq!(price.currency_code.as_str(), input.currency_code.as_str());
    assert_eq!(price.amount, input.amount);
    assert_eq!(price.min_quantity, input.min_quantity);
    assert_eq!(price.max_quantity, input.max_quantity);
    assert_eq!(price.customer_group_id, input.customer_group_id);
    assert_eq!(price.sales_channel_id, input.sales_channel_id);
    assert_eq!(price.starts_at, input.starts_at);
    assert_eq!(price.ends_at, input.ends_at);
}

#[tokio::test]
async fn bulk_upsert_returns_prices_in_request_order() {
    let (db, service) = service().await;
    seed_customer_group(&db, "grp_vip").await;

    let inputs = vec![
        CreatePriceInput::new("var_1", usd(), 1000),
        CreatePriceInput::new("var_1", usd(), 900).with_quantity_range(10, Some(99)),
        CreatePriceInput::new("var_1", eur(), 800),
        CreatePriceInput::new("var_1", usd(), 700)
            .with_customer_group("grp_vip")
            .with_window(Some(10), Some(20)),
        CreatePriceInput::new("var_2", usd(), 600),
    ];
    let prices = service.bulk_upsert_prices(inputs.clone()).await.unwrap();
    assert_eq!(prices.len(), inputs.len());
    for (price, input) in prices.iter().zip(&inputs) {
        assert_scope(price, input);
    }

    // A second batch updates some scopes and inserts new ones; the response must
    // keep following the request order.
    let mut updated_amount = inputs[1].clone();
    updated_amount.amount = 950;
    updated_amount.max_quantity = None;
    let inserted = CreatePriceInput::new("var_3", usd(), 500);
    let second = service
        .bulk_upsert_prices(vec![updated_amount.clone(), inserted.clone()])
        .await
        .unwrap();
    assert_eq!(second.len(), 2);
    assert_scope(&second[0], &updated_amount);
    assert_scope(&second[1], &inserted);
    assert_eq!(second[0].id, prices[1].id, "the update keeps the price id");
    assert_eq!(
        second[0].created_at, prices[1].created_at,
        "the update keeps the creation timestamp"
    );
    assert_ne!(second[1].id, second[0].id);
}

/// Counts the read-back statements issued while upserting `input_count` prices.
async fn read_back_queries(recorder: &StatementRecorder, input_count: usize) -> usize {
    let (db, service) = service().await;
    let thread = StatementRecorder::worker_thread(&db).await;
    let inputs = (0..input_count)
        .map(|index| {
            CreatePriceInput::new("var_1", usd(), 1000).with_quantity_range(index as i64 + 1, None)
        })
        .collect::<Vec<_>>();

    let checkpoint = recorder.checkpoint();
    let prices = service.bulk_upsert_prices(inputs).await.unwrap();
    assert_eq!(prices.len(), input_count);
    recorder
        .statements_since(checkpoint, thread, READ_BACK_NEEDLE)
        .len()
}

#[tokio::test]
async fn bulk_upsert_reads_prices_back_in_constant_queries() {
    let recorder = StatementRecorder::global();
    let small = read_back_queries(&recorder, 2).await;
    let large = read_back_queries(&recorder, 20).await;
    assert_eq!(
        large, small,
        "the read-back query count must not grow with the batch size"
    );
    assert_eq!(large, 1, "the batch must be read back in a single query");
}

#[tokio::test]
async fn bulk_upsert_still_reports_not_found_for_missing_scopes() {
    let (db, service) = service().await;
    seed_customer_group(&db, "grp_vip").await;

    let missing_group =
        CreatePriceInput::new("var_1", usd(), 1000).with_customer_group("grp_ghost");
    let error = service
        .bulk_upsert_prices(vec![missing_group])
        .await
        .unwrap_err();
    assert_eq!(error.code(), "NOT_FOUND");

    // Nothing from the failed batch is persisted.
    let prices = service
        .list_prices(PriceFilter::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(prices.count, 0);
}
