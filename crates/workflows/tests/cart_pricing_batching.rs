//! `CartWorkflow::recalculate` must price a cart without a query per line.
//!
//! Pricing is selected per variant and line, but one lookup answers every line
//! whose quantity falls in the same tier, so a cart of `n` lines must not cost
//! `n` price lookups.
//!
//! This file holds a single test on purpose. The statement recorder counts
//! process-wide, so a second test running beside it — even against its own
//! database — could issue a pricing lookup while the count is open and be
//! attributed to this test's worker thread. What a recalculation selects at
//! all is covered by `cart_recalculate` instead.

use std::fmt::Debug;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::ThreadId;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CartId, CountryCode, CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_cart::{Cart, CreateCartInput};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_workflows::{CartContext, CartWorkflow};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata as TracingMetadata, Subscriber};

/// Records the SQL text of every statement sqlx executes, tagged with the
/// thread that executed it.
///
/// sqlx logs one `tracing` event per statement from the worker thread that owns
/// the connection, and `tracing` only emits those events while a global
/// subscriber is installed, so the recorder installs itself process-wide the
/// first time it is used. Tests in this binary are told apart by the worker
/// thread of their own pool, which the recorder discovers through a marker
/// query.
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
    async fn worker_thread(db: &StoreDatabase) -> ThreadId {
        let recorder = Self::global();
        let before = recorder.records.lock().unwrap().len();
        sqlx::query("SELECT 'cart_pricing_marker' AS marker FROM prices LIMIT 1")
            .fetch_all(db.pool())
            .await
            .unwrap();
        let records = recorder.records.lock().unwrap();
        records[before..]
            .iter()
            .find(|record| record.sql.contains("cart_pricing_marker"))
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

/// The statement text of the pricing module's eligible-price lookup.
const PRICE_LOOKUP_NEEDLE: &str = "FROM prices p";

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn context() -> CartContext {
    CartContext::new(CountryCode::new("US").unwrap())
}

async fn create_cart(workflow: &CartWorkflow, sales_channel_id: Option<String>) -> Cart {
    workflow
        .create_cart(CreateCartInput {
            currency_code: usd().to_string(),
            customer_id: None,
            email: Some("buyer@example.com".to_owned()),
            sales_channel_id,
            region_id: None,
            metadata: Metadata::empty(),
            expires_in_seconds: None,
        })
        .await
        .unwrap()
}

/// Builds a store with `line_count` published variants, each priced at 1 000,
/// and a cart holding one unit of every variant.
async fn fixture(line_count: usize) -> (StoreDatabase, CartWorkflow, CartId, CartContext) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Mug".to_owned(),
            variants: (0..line_count)
                .map(|index| CreateVariantInput {
                    title: format!("Variant {index}"),
                    sku: Some(format!("MUG-{index}")),
                    manage_inventory: Some(false),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();

    let pricing = PricingService::new(db.clone());
    for variant in &product.variants {
        pricing
            .create_price(CreatePriceInput::new(&variant.id, usd(), 1_000))
            .await
            .unwrap();
    }

    let workflow = CartWorkflow::new(db.clone());
    let context = context();
    let cart = create_cart(&workflow, None).await;
    for variant in &product.variants {
        workflow
            .add_item(
                &cart.id,
                &VariantId::from_string(variant.id.clone()),
                1,
                &context,
            )
            .await
            .unwrap();
    }
    (db, workflow, cart.id, context)
}

/// Counts the pricing lookups one recalculation of a `line_count` cart issues.
async fn recalculate_price_lookups(recorder: &StatementRecorder, line_count: usize) -> usize {
    let (db, workflow, cart_id, context) = fixture(line_count).await;
    let thread = StatementRecorder::worker_thread(&db).await;

    let checkpoint = recorder.checkpoint();
    let cart = workflow.recalculate(&cart_id, &context).await.unwrap();
    assert_eq!(cart.items.len(), line_count);

    recorder
        .statements_since(checkpoint, thread, PRICE_LOOKUP_NEEDLE)
        .len()
}

#[tokio::test]
async fn recalculate_looks_prices_up_in_a_constant_number_of_queries() {
    let recorder = StatementRecorder::global();

    let small = recalculate_price_lookups(&recorder, 2).await;
    let large = recalculate_price_lookups(&recorder, 8).await;
    assert_eq!(
        large, small,
        "the pricing lookup count must not grow with the number of lines"
    );
    assert_eq!(
        small, 1,
        "lines with the same quantity share one price lookup"
    );
}
