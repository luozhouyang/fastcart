//! SQLite-backed inventory application service.

use std::collections::HashMap;

use fastcart_infrastructure::{AuditActor, StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, Metadata, OrderId,
    ReservationId, Timestamp, VariantId,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::domain::{
    InventoryItem, InventoryLevel, InventoryLocation, InventoryMovement, InventoryReservation,
    ReservationStatus,
};

const DEFAULT_LOCATION_ID: &str = "loc_default";
const RETURN_RESTOCK_REASON: &str = "return_restock";
const EXCHANGE_RESTOCK_REASON: &str = "exchange_restock";
const AFTER_SALES_CONSUME_REASON: &str = "after_sales_consume";
const ORDER_EDIT_REASON: &str = "order_edit";

/// Maximum number of bound parameters used in one `IN (...)` statement.
/// SQLite's default limit is 999; staying under it leaves headroom for the
/// other parameters a query may carry.
const SQLITE_PARAMETER_LIMIT: usize = 900;

mod catalog;
mod order_edits;
mod reservations;
mod stock;

#[derive(Debug)]
struct ReservedRow {
    id: ReservationId,
    inventory_item_id: InventoryItemId,
    location_id: InventoryLocationId,
    quantity: i64,
}

fn row_to_reserved(row: &SqliteRow) -> Result<ReservedRow, AppError> {
    Ok(ReservedRow {
        id: ReservationId::from_string(row.try_get::<String, _>("id").map_err(database_error)?),
        inventory_item_id: InventoryItemId::from_string(
            row.try_get::<String, _>("inventory_item_id")
                .map_err(database_error)?,
        ),
        location_id: InventoryLocationId::from_string(
            row.try_get::<String, _>("location_id")
                .map_err(database_error)?,
        ),
        quantity: row.try_get("quantity").map_err(database_error)?,
    })
}

/// Audit entry appended in the same transaction as a restock.
///
/// The calling workflow supplies the action and resource; the service adds the
/// `restocked_units` field once the actual restocked quantity is known.
#[derive(Clone, Debug)]
pub struct RestockAudit {
    /// Audit action name.
    pub action: String,
    /// Resource type.
    pub resource_type: String,
    /// Resource identifier.
    pub resource_id: String,
    /// Base JSON payload.
    pub data: serde_json::Value,
}

/// Request for reversing the stock that receiving placed back after a return
/// item was written off.
#[derive(Clone, Copy, Debug)]
pub struct ReverseWriteOffRequest<'a> {
    /// Return whose item is being reversed.
    pub return_id: &'a str,
    /// Order line the return item refers to.
    pub order_item_id: &'a str,
    /// Quantity originally written off for this return item.
    pub written_off_quantity: i64,
    /// Increment of the write-off being reversed.
    pub increment: i64,
    /// Current timestamp.
    pub now: Timestamp,
}

/// Inventory application service.
#[derive(Clone, Debug)]
pub struct InventoryService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

impl InventoryService {
    /// Creates a service over a migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db, audit: None }
    }

    /// Records audited restocks in the store audit outbox.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

    /// Appends one audit entry to the outbox inside a mutation transaction.
    async fn append_audit(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        data: serde_json::Value,
    ) -> Result<(), AppError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, resource_type, resource_id, data)
                .await
                .map_err(database_error)?;
        }
        Ok(())
    }

    async fn ensure_item_exists(&self, id: &InventoryItemId) -> Result<(), AppError> {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM inventory_items WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if existing.is_none() {
            return Err(AppError::not_found("inventory item", id));
        }
        Ok(())
    }

    async fn ensure_location_exists(&self, id: &InventoryLocationId) -> Result<(), AppError> {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM inventory_locations WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if existing.is_none() {
            return Err(AppError::not_found("inventory location", id));
        }
        Ok(())
    }
}

fn sql_markers(count: usize) -> String {
    vec!["?"; count].join(", ")
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::now_v7().simple())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
}

fn row_to_location(row: &SqliteRow) -> Result<InventoryLocation, AppError> {
    Ok(InventoryLocation {
        id: InventoryLocationId::from_string(
            row.try_get::<String, _>("id").map_err(database_error)?,
        ),
        name: row.try_get("name").map_err(database_error)?,
        address: parse_metadata(row, "address")?,
        metadata: parse_metadata(row, "metadata")?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_item(row: &SqliteRow) -> Result<InventoryItem, AppError> {
    let variant_id = row
        .try_get::<Option<String>, _>("variant_id")
        .map_err(database_error)?
        .map(VariantId::from_string);
    Ok(InventoryItem {
        id: InventoryItemId::from_string(row.try_get::<String, _>("id").map_err(database_error)?),
        sku: row.try_get("sku").map_err(database_error)?,
        variant_id,
        metadata: parse_metadata(row, "metadata")?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_level(row: &SqliteRow) -> Result<InventoryLevel, AppError> {
    Ok(InventoryLevel {
        id: row.try_get("id").map_err(database_error)?,
        inventory_item_id: InventoryItemId::from_string(
            row.try_get::<String, _>("inventory_item_id")
                .map_err(database_error)?,
        ),
        location_id: InventoryLocationId::from_string(
            row.try_get::<String, _>("location_id")
                .map_err(database_error)?,
        ),
        stocked_quantity: row.try_get("stocked_quantity").map_err(database_error)?,
        reserved_quantity: row.try_get("reserved_quantity").map_err(database_error)?,
        incoming_quantity: row.try_get("incoming_quantity").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_reservation(row: &SqliteRow) -> Result<InventoryReservation, AppError> {
    let status: String = row.try_get("status").map_err(database_error)?;
    let line_item_id = row
        .try_get::<Option<String>, _>("line_item_id")
        .map_err(database_error)?
        .map(LineItemId::from_string);
    let checkout_id = row
        .try_get::<Option<String>, _>("checkout_id")
        .map_err(database_error)?
        .map(CheckoutId::from_string);
    let order_id = row
        .try_get::<Option<String>, _>("order_id")
        .map_err(database_error)?
        .map(OrderId::from_string);
    Ok(InventoryReservation {
        id: ReservationId::from_string(row.try_get::<String, _>("id").map_err(database_error)?),
        inventory_item_id: InventoryItemId::from_string(
            row.try_get::<String, _>("inventory_item_id")
                .map_err(database_error)?,
        ),
        location_id: InventoryLocationId::from_string(
            row.try_get::<String, _>("location_id")
                .map_err(database_error)?,
        ),
        quantity: row.try_get("quantity").map_err(database_error)?,
        status: ReservationStatus::from_db(&status),
        line_item_id,
        checkout_id,
        order_id,
        expires_at: row.try_get("expires_at").map_err(database_error)?,
        metadata: parse_metadata(row, "metadata")?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_movement(row: &SqliteRow) -> Result<InventoryMovement, AppError> {
    Ok(InventoryMovement {
        id: row.try_get("id").map_err(database_error)?,
        inventory_item_id: InventoryItemId::from_string(
            row.try_get::<String, _>("inventory_item_id")
                .map_err(database_error)?,
        ),
        location_id: InventoryLocationId::from_string(
            row.try_get::<String, _>("location_id")
                .map_err(database_error)?,
        ),
        quantity_delta: row.try_get("quantity_delta").map_err(database_error)?,
        reason: row.try_get("reason").map_err(database_error)?,
        reference: row.try_get("reference").map_err(database_error)?,
        actor_id: row.try_get("actor_id").map_err(database_error)?,
        metadata: parse_metadata(row, "metadata")?,
        created_at: row.try_get("created_at").map_err(database_error)?,
    })
}

fn parse_metadata(row: &SqliteRow, column: &str) -> Result<Metadata, AppError> {
    let value: String = row.try_get(column).map_err(database_error)?;
    Ok(Metadata::from_json_str(&value))
}

async fn ensure_level_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    item_id: &InventoryItemId,
    location_id: &InventoryLocationId,
    now: Timestamp,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT OR IGNORE INTO inventory_levels \
         (id, inventory_item_id, location_id, stocked_quantity, reserved_quantity, \
          incoming_quantity, created_at, updated_at) \
         VALUES (?, ?, ?, 0, 0, 0, ?, ?)",
    )
    .bind(new_id("ilvl"))
    .bind(item_id.as_str())
    .bind(location_id.as_str())
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

/// Creates the fallback location when a store has not provisioned one yet.
async fn ensure_default_location_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    now: Timestamp,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT OR IGNORE INTO inventory_locations \
         (id, name, address, metadata, created_at, updated_at) \
         VALUES (?, 'Default location', '{}', '{}', ?, ?)",
    )
    .bind(DEFAULT_LOCATION_ID)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

fn insufficient_stock_for_variant(variant_id: &str) -> AppError {
    AppError::conflict(format!("insufficient stock for variant {variant_id}"))
}

/// Loads the inventory item of every variant in one pass.
///
/// SQLite caps bound parameters per statement, so the variants are read in
/// chunks that comfortably fit.
async fn load_items_by_variant_tx(
    tx: &mut Transaction<'_, Sqlite>,
    variants: &[VariantId],
) -> Result<HashMap<String, String>, AppError> {
    let mut by_variant = HashMap::new();
    for chunk in variants.chunks(SQLITE_PARAMETER_LIMIT) {
        let sql = format!(
            "SELECT id, variant_id FROM inventory_items WHERE variant_id IN ({})",
            sql_markers(chunk.len())
        );
        let mut query = sqlx::query(&sql);
        for variant in chunk {
            query = query.bind(variant.as_str());
        }
        let rows = query.fetch_all(&mut **tx).await.map_err(database_error)?;
        for row in rows {
            let id: String = row.try_get("id").map_err(database_error)?;
            let variant_id: String = row.try_get("variant_id").map_err(database_error)?;
            by_variant.insert(variant_id, id);
        }
    }
    Ok(by_variant)
}

struct Movement<'a> {
    item_id: &'a InventoryItemId,
    location_id: &'a InventoryLocationId,
    quantity_delta: i64,
    reason: &'a str,
    reference: Option<&'a str>,
    actor_id: Option<&'a str>,
    now: Timestamp,
}

async fn insert_movement(
    tx: &mut Transaction<'_, Sqlite>,
    movement: Movement<'_>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO inventory_movements \
         (id, inventory_item_id, location_id, quantity_delta, reason, reference, actor_id, \
          metadata, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, '{}', ?)",
    )
    .bind(new_id("imvt"))
    .bind(movement.item_id.as_str())
    .bind(movement.location_id.as_str())
    .bind(movement.quantity_delta)
    .bind(movement.reason)
    .bind(movement.reference)
    .bind(movement.actor_id)
    .bind(movement.now)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn fetch_level_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    item_id: &InventoryItemId,
    location_id: &InventoryLocationId,
) -> Result<InventoryLevel, AppError> {
    let row = sqlx::query(
        "SELECT id, inventory_item_id, location_id, stocked_quantity, reserved_quantity, \
         incoming_quantity, created_at, updated_at \
         FROM inventory_levels WHERE inventory_item_id = ? AND location_id = ?",
    )
    .bind(item_id.as_str())
    .bind(location_id.as_str())
    .fetch_one(&mut **tx)
    .await
    .map_err(database_error)?;
    row_to_level(&row)
}
