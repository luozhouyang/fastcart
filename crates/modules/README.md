# Commerce Module Conventions

Each commerce module is a self-contained crate. Modules may depend on
`fastcart-kernel` and `fastcart-infrastructure`, but **not on each other**.
Cross-module orchestration (cart totals, checkout, order placement) belongs in
`fastcart-workflows`.

This mirrors Medusa's module isolation: a module owns its entities, persistence
and business rules; workflows compose modules through their public services.

## Layout

```text
crates/modules/<name>/
├── Cargo.toml
└── src/
    ├── lib.rs        # public exports
    ├── domain.rs     # entities, enums, inputs, pure rules/calculators
    └── service.rs    # SQLite-backed application service
```

`lib.rs` re-exports the public surface. Keep persistence types (`sqlx` rows)
private.

## Service shape

```rust
use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::AppError;

pub struct ProductService {
    db: StoreDatabase,
}

impl ProductService {
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    pub async fn create_product(&self, input: CreateProductInput)
        -> Result<Product, AppError>
    { /* ... */ }
}
```

Rules:

- Constructors are infallible and take an already-migrated `StoreDatabase`.
- Every query is scoped to `self.db.store_id()` implicitly by the database
  handle. Never accept a database path or a second store identifier.
- Map SQLx errors with `.map_err(database_error)?`; return `AppError` for
  validation, not-found and conflict failures (`AppError::validation`,
  `AppError::not_found`, `AppError::conflict`).
- Validate input with `fastcart_kernel::{validate_handle, validate_email,
  validate_quantity, slugify, ...}`.
- Metadata is `fastcart_kernel::Metadata`; persist with `to_json_string()` and
  read with `Metadata::from_json_str()`.
- Timestamps are Unix seconds from `fastcart_kernel::now_timestamp()`.
- Money is always an `INTEGER` amount in minor units plus a currency code.
  Never use floating point for prices, totals, discounts or tax.
- Pagination uses `Pagination` and `PageResult<T>`.

## Multi-write operations

Use one short transaction for operations that must be atomic (stock
reservations, order placement, usage accounting):

```rust
let mut tx = self.db.begin().await.map_err(database_error)?;
sqlx::query("...").execute(&mut *tx).await.map_err(database_error)?;
tx.commit().await.map_err(database_error)?;
```

Never perform network calls inside a transaction.

## Events

Business methods return entities. Workflows/API layers create
`DomainEventEnvelope` values and persist them through the outbox when reliable
delivery is required. Modules should not depend on the in-process bus.

## Tests

Every public operation needs tests. Use `StoreDatabase::in_memory` for an
isolated, migrated database:

```rust
#[tokio::test]
async fn creates_a_product() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ProductService::new(db);
    // ...
}
```

Cover happy paths plus validation, not-found, conflict and tenant-isolation
behaviour. Integration tests within a module use the same in-memory database.
