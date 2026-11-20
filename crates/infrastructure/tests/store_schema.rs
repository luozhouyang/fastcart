//! Store schema invariants that hot query paths depend on.
//!
//! These are behavioural tests about the migrated schema rather than the
//! migration files themselves: they fail whenever a future migration drops an
//! index that a query in `crates/modules/cart` relies on.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::StoreId;
use sqlx::Row;

/// Returns the columns covered by `index`, in order.
async fn index_columns(db: &StoreDatabase, index: &str) -> Vec<String> {
    sqlx::query(&format!("PRAGMA index_info('{index}')"))
        .fetch_all(db.pool())
        .await
        .expect("index_info")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect()
}

/// Returns the indexes declared on `table`, together with their columns.
async fn indexes_on(db: &StoreDatabase, table: &str) -> Vec<(String, Vec<String>)> {
    let rows = sqlx::query(&format!("PRAGMA index_list('{table}')"))
        .fetch_all(db.pool())
        .await
        .expect("index_list");
    let mut indexes = Vec::new();
    for row in rows {
        let name: String = row.get("name");
        let columns = index_columns(db, &name).await;
        indexes.push((name, columns));
    }
    indexes
}

/// Asserts that some index on `table` leads with `column`.
async fn assert_indexed(db: &StoreDatabase, table: &str, column: &str) {
    let indexes = indexes_on(db, table).await;
    let found = indexes
        .iter()
        .any(|(_, columns)| columns.first().map(String::as_str) == Some(column));
    assert!(
        found,
        "no index on {table} leads with {column}; the cart aggregate reloads \
         adjustments by this column on every read, so it degrades to a full \
         table scan. indexes: {indexes:?}"
    );
}

#[tokio::test]
async fn cart_adjustment_lookups_are_indexed() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();

    // `CartService::load_aggregate` joins both adjustment tables and filters by
    // the parent id, which is a foreign key column SQLite does not index
    // automatically.
    assert_indexed(&db, "cart_item_adjustments", "cart_item_id").await;
    assert_indexed(
        &db,
        "cart_shipping_method_adjustments",
        "shipping_method_id",
    )
    .await;
}
