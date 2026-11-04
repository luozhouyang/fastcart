//! SQLite-backed inventory application service.

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, Metadata, OrderId,
    PageResult, Pagination, ReservationId, ReturnId, Timestamp, VariantId, now_timestamp,
    validate_length, validate_metadata,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::domain::{
    CreateInventoryItemInput, CreateLocationInput, InventoryItem, InventoryLevel,
    InventoryLocation, InventoryMovement, InventoryReservation, ReservationLine,
    ReservationReference, ReservationStatus, ReturnRestockLine, UpdateLocationInput,
    available_quantity_from, return_restock_reference, validate_name, validate_positive_quantity,
    validate_reason, validate_ttl,
};

const DEFAULT_LOCATION_ID: &str = "loc_default";
const RETURN_RESTOCK_REASON: &str = "return_restock";

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::now_v7().simple())
}

fn metadata_string(metadata: Option<Metadata>) -> String {
    metadata.unwrap_or_default().to_json_string()
}

fn validate_metadata_option(metadata: Option<&Metadata>) -> Result<(), AppError> {
    if let Some(metadata) = metadata {
        validate_metadata(metadata)?;
    }
    Ok(())
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

async fn ensure_item_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &InventoryItemId,
) -> Result<(), AppError> {
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM inventory_items WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?;
    if existing.is_none() {
        return Err(AppError::not_found("inventory item", id));
    }
    Ok(())
}

async fn ensure_location_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &InventoryLocationId,
) -> Result<(), AppError> {
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM inventory_locations WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?;
    if existing.is_none() {
        return Err(AppError::not_found("inventory location", id));
    }
    Ok(())
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

async fn release_rows_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    rows: &[ReservedRow],
    now: Timestamp,
) -> Result<u64, AppError> {
    for row in rows {
        let affected = sqlx::query(
            "UPDATE inventory_levels \
             SET reserved_quantity = reserved_quantity - ?, updated_at = ? \
             WHERE inventory_item_id = ? AND location_id = ? AND reserved_quantity >= ?",
        )
        .bind(row.quantity)
        .bind(now)
        .bind(row.inventory_item_id.as_str())
        .bind(row.location_id.as_str())
        .bind(row.quantity)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
        if affected.rows_affected() != 1 {
            return Err(AppError::conflict(
                "inventory level no longer matches the reservation",
            ));
        }
        sqlx::query(
            "UPDATE inventory_reservations SET status = 'released', updated_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(row.id.as_str())
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    }
    Ok(u64::try_from(rows.len()).unwrap_or(u64::MAX))
}

/// Application service for locations, items, levels and reservations.
#[derive(Clone, Debug)]
pub struct InventoryService {
    db: StoreDatabase,
}

impl InventoryService {
    /// Creates a service over a migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    // Locations

    /// Creates an inventory location.
    pub async fn create_location(
        &self,
        input: CreateLocationInput,
    ) -> Result<InventoryLocation, AppError> {
        validate_name(&input.name, "name")?;
        validate_metadata_option(input.metadata.as_ref())?;
        let now = now_timestamp();
        let id = InventoryLocationId::new();
        sqlx::query(
            "INSERT INTO inventory_locations (id, name, address, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.name.trim())
        .bind(input.address.unwrap_or_default().to_json_string())
        .bind(metadata_string(input.metadata))
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_location(&id).await
    }

    /// Fetches an inventory location by identifier.
    pub async fn get_location(
        &self,
        id: &InventoryLocationId,
    ) -> Result<InventoryLocation, AppError> {
        let row = sqlx::query(
            "SELECT id, name, address, metadata, created_at, updated_at \
             FROM inventory_locations WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_location(&row),
            None => Err(AppError::not_found("inventory location", id)),
        }
    }

    /// Lists inventory locations.
    pub async fn list_locations(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<InventoryLocation>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM inventory_locations")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query(
            "SELECT id, name, address, metadata, created_at, updated_at \
             FROM inventory_locations ORDER BY created_at, id LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let locations = rows
            .iter()
            .map(row_to_location)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PageResult::new(locations, count, pagination))
    }

    /// Updates an inventory location.
    pub async fn update_location(
        &self,
        id: &InventoryLocationId,
        input: UpdateLocationInput,
    ) -> Result<InventoryLocation, AppError> {
        let existing = self.get_location(id).await?;
        let name = match input.name {
            Some(name) => {
                validate_name(&name, "name")?;
                name.trim().to_owned()
            }
            None => existing.name,
        };
        validate_metadata_option(input.metadata.as_ref())?;
        let address = input.address.unwrap_or(existing.address);
        let metadata = input.metadata.unwrap_or(existing.metadata);
        let now = now_timestamp();
        sqlx::query(
            "UPDATE inventory_locations SET name = ?, address = ?, metadata = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(name)
        .bind(address.to_json_string())
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_location(id).await
    }

    // Items

    /// Creates an inventory item.
    ///
    /// A variant and an SKU may each be attached to exactly one item.
    pub async fn create_item(
        &self,
        input: CreateInventoryItemInput,
    ) -> Result<InventoryItem, AppError> {
        if let Some(sku) = input.sku.as_deref() {
            validate_length(sku, "sku", 120)?;
            let existing: Option<String> =
                sqlx::query_scalar("SELECT id FROM inventory_items WHERE sku = ?")
                    .bind(sku)
                    .fetch_optional(self.db.pool())
                    .await
                    .map_err(database_error)?;
            if existing.is_some() {
                return Err(AppError::conflict(format!("sku already in use: {sku}")));
            }
        }
        if let Some(variant_id) = input.variant_id.as_ref() {
            let existing: Option<String> =
                sqlx::query_scalar("SELECT id FROM inventory_items WHERE variant_id = ?")
                    .bind(variant_id.as_str())
                    .fetch_optional(self.db.pool())
                    .await
                    .map_err(database_error)?;
            if existing.is_some() {
                return Err(AppError::conflict(format!(
                    "variant already has an inventory item: {variant_id}"
                )));
            }
        }
        validate_metadata_option(input.metadata.as_ref())?;
        let now = now_timestamp();
        let id = InventoryItemId::new();
        let sku = input.sku.map(|sku| sku.trim().to_owned());
        sqlx::query(
            "INSERT INTO inventory_items (id, sku, variant_id, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(sku)
        .bind(input.variant_id.as_ref().map(VariantId::as_str))
        .bind(metadata_string(input.metadata))
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_item(&id).await
    }

    /// Fetches an inventory item by identifier.
    pub async fn get_item(&self, id: &InventoryItemId) -> Result<InventoryItem, AppError> {
        let row = sqlx::query(
            "SELECT id, sku, variant_id, metadata, created_at, updated_at \
             FROM inventory_items WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_item(&row),
            None => Err(AppError::not_found("inventory item", id)),
        }
    }

    /// Fetches the inventory item backing a catalog variant.
    pub async fn get_item_by_variant(
        &self,
        variant_id: &VariantId,
    ) -> Result<InventoryItem, AppError> {
        let row = sqlx::query(
            "SELECT id, sku, variant_id, metadata, created_at, updated_at \
             FROM inventory_items WHERE variant_id = ?",
        )
        .bind(variant_id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_item(&row),
            None => Err(AppError::not_found(
                "inventory item for variant",
                variant_id,
            )),
        }
    }

    /// Lists inventory items.
    pub async fn list_items(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<InventoryItem>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM inventory_items")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query(
            "SELECT id, sku, variant_id, metadata, created_at, updated_at \
             FROM inventory_items ORDER BY created_at, id LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let items = rows
            .iter()
            .map(row_to_item)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PageResult::new(items, count, pagination))
    }

    /// Attaches a catalog variant to an existing inventory item.
    pub async fn attach_variant(
        &self,
        item_id: &InventoryItemId,
        variant_id: &VariantId,
    ) -> Result<InventoryItem, AppError> {
        let item = self.get_item(item_id).await?;
        if item.variant_id.as_ref() == Some(variant_id) {
            return Ok(item);
        }
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM inventory_items WHERE variant_id = ? AND id <> ?")
                .bind(variant_id.as_str())
                .bind(item_id.as_str())
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if existing.is_some() {
            return Err(AppError::conflict(format!(
                "variant already has an inventory item: {variant_id}"
            )));
        }
        sqlx::query("UPDATE inventory_items SET variant_id = ?, updated_at = ? WHERE id = ?")
            .bind(variant_id.as_str())
            .bind(now_timestamp())
            .bind(item_id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        self.get_item(item_id).await
    }

    // Levels and availability

    /// Fetches the level for an item at a location.
    pub async fn get_level(
        &self,
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
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_level(&row),
            None => Err(AppError::not_found(
                "inventory level",
                format!("{item_id}@{location_id}"),
            )),
        }
    }

    /// Lists levels for an item across locations.
    pub async fn list_levels(
        &self,
        item_id: &InventoryItemId,
    ) -> Result<Vec<InventoryLevel>, AppError> {
        self.ensure_item_exists(item_id).await?;
        let rows = sqlx::query(
            "SELECT id, inventory_item_id, location_id, stocked_quantity, reserved_quantity, \
             incoming_quantity, created_at, updated_at \
             FROM inventory_levels WHERE inventory_item_id = ? ORDER BY location_id",
        )
        .bind(item_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.iter().map(row_to_level).collect()
    }

    /// Lists levels held at one location.
    pub async fn list_levels_for_location(
        &self,
        location_id: &InventoryLocationId,
    ) -> Result<Vec<InventoryLevel>, AppError> {
        self.ensure_location_exists(location_id).await?;
        let rows = sqlx::query(
            "SELECT id, inventory_item_id, location_id, stocked_quantity, reserved_quantity, \
             incoming_quantity, created_at, updated_at \
             FROM inventory_levels WHERE location_id = ? ORDER BY inventory_item_id",
        )
        .bind(location_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.iter().map(row_to_level).collect()
    }

    /// Returns on-hand minus reserved at one location, never negative.
    pub async fn available_quantity(
        &self,
        item_id: &InventoryItemId,
        location_id: &InventoryLocationId,
    ) -> Result<i64, AppError> {
        self.ensure_item_exists(item_id).await?;
        self.ensure_location_exists(location_id).await?;
        let row = sqlx::query(
            "SELECT stocked_quantity, reserved_quantity FROM inventory_levels \
             WHERE inventory_item_id = ? AND location_id = ?",
        )
        .bind(item_id.as_str())
        .bind(location_id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => Ok(available_quantity_from(
                row.try_get("stocked_quantity").map_err(database_error)?,
                row.try_get("reserved_quantity").map_err(database_error)?,
            )),
            None => Ok(0),
        }
    }

    /// Returns the total available quantity for a variant across all locations.
    pub async fn variant_availability(&self, variant_id: &VariantId) -> Result<i64, AppError> {
        let item_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM inventory_items WHERE variant_id = ?")
                .bind(variant_id.as_str())
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        let Some(item_id) = item_id else {
            return Ok(0);
        };
        let rows = sqlx::query(
            "SELECT stocked_quantity, reserved_quantity FROM inventory_levels \
             WHERE inventory_item_id = ?",
        )
        .bind(item_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let mut total = 0_i64;
        for row in &rows {
            let stocked: i64 = row.try_get("stocked_quantity").map_err(database_error)?;
            let reserved: i64 = row.try_get("reserved_quantity").map_err(database_error)?;
            total = total.saturating_add(available_quantity_from(stocked, reserved));
        }
        Ok(total)
    }

    // Stock changes

    /// Applies a signed stock adjustment and records a movement.
    pub async fn adjust_stock(
        &self,
        item_id: &InventoryItemId,
        location_id: &InventoryLocationId,
        delta: i64,
        reason: &str,
        actor_id: Option<&str>,
    ) -> Result<InventoryLevel, AppError> {
        validate_reason(reason)?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        ensure_item_in_tx(&mut tx, item_id).await?;
        ensure_location_in_tx(&mut tx, location_id).await?;
        ensure_level_in_tx(&mut tx, item_id, location_id, now).await?;
        let affected = sqlx::query(
            "UPDATE inventory_levels SET stocked_quantity = stocked_quantity + ?, updated_at = ? \
             WHERE inventory_item_id = ? AND location_id = ? \
               AND stocked_quantity + ? >= reserved_quantity AND stocked_quantity + ? >= 0",
        )
        .bind(delta)
        .bind(now)
        .bind(item_id.as_str())
        .bind(location_id.as_str())
        .bind(delta)
        .bind(delta)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if affected.rows_affected() != 1 {
            return Err(AppError::conflict(
                "stock adjustment would leave stock negative or below reserved",
            ));
        }
        insert_movement(
            &mut tx,
            Movement {
                item_id,
                location_id,
                quantity_delta: delta,
                reason,
                reference: None,
                actor_id,
                now,
            },
        )
        .await?;
        let level = fetch_level_in_tx(&mut tx, item_id, location_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(level)
    }

    /// Moves stock between two locations, recording both movements.
    pub async fn transfer(
        &self,
        item_id: &InventoryItemId,
        from_location_id: &InventoryLocationId,
        to_location_id: &InventoryLocationId,
        quantity: i64,
    ) -> Result<(InventoryLevel, InventoryLevel), AppError> {
        if from_location_id == to_location_id {
            return Err(AppError::validation(
                "source and destination locations must differ",
            ));
        }
        validate_positive_quantity(quantity, "quantity")?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        ensure_item_in_tx(&mut tx, item_id).await?;
        ensure_location_in_tx(&mut tx, from_location_id).await?;
        ensure_location_in_tx(&mut tx, to_location_id).await?;
        ensure_level_in_tx(&mut tx, item_id, from_location_id, now).await?;
        ensure_level_in_tx(&mut tx, item_id, to_location_id, now).await?;

        let removed = sqlx::query(
            "UPDATE inventory_levels SET stocked_quantity = stocked_quantity - ?, updated_at = ? \
             WHERE inventory_item_id = ? AND location_id = ? \
               AND stocked_quantity - reserved_quantity >= ?",
        )
        .bind(quantity)
        .bind(now)
        .bind(item_id.as_str())
        .bind(from_location_id.as_str())
        .bind(quantity)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if removed.rows_affected() != 1 {
            return Err(AppError::conflict(
                "insufficient available stock at the source location",
            ));
        }
        sqlx::query(
            "UPDATE inventory_levels SET stocked_quantity = stocked_quantity + ?, updated_at = ? \
             WHERE inventory_item_id = ? AND location_id = ?",
        )
        .bind(quantity)
        .bind(now)
        .bind(item_id.as_str())
        .bind(to_location_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        insert_movement(
            &mut tx,
            Movement {
                item_id,
                location_id: from_location_id,
                quantity_delta: -quantity,
                reason: "transfer_out",
                reference: Some(to_location_id.as_str()),
                actor_id: None,
                now,
            },
        )
        .await?;
        insert_movement(
            &mut tx,
            Movement {
                item_id,
                location_id: to_location_id,
                quantity_delta: quantity,
                reason: "transfer_in",
                reference: Some(from_location_id.as_str()),
                actor_id: None,
                now,
            },
        )
        .await?;

        let from_level = fetch_level_in_tx(&mut tx, item_id, from_location_id).await?;
        let to_level = fetch_level_in_tx(&mut tx, item_id, to_location_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok((from_level, to_level))
    }

    // Reservations

    /// Atomically reserves stock for every line or fails without changing state.
    ///
    /// Locations are visited deterministically: `loc_default` first, then by
    /// ascending location identifier. A line that cannot be fully satisfied
    /// rolls back the entire batch with a conflict naming the variant.
    pub async fn reserve(
        &self,
        lines: Vec<ReservationLine>,
        reference: ReservationReference,
        ttl_seconds: i64,
    ) -> Result<Vec<InventoryReservation>, AppError> {
        if lines.is_empty() {
            return Err(AppError::validation(
                "at least one reservation line is required",
            ));
        }
        validate_ttl(ttl_seconds)?;
        for line in &lines {
            validate_positive_quantity(line.quantity, "line quantity")?;
        }

        let now = now_timestamp();
        let expires_at = now.saturating_add(ttl_seconds);
        let mut tx = self.db.begin().await?;
        let mut reservations = Vec::new();

        for line in &lines {
            let item_id: Option<String> =
                sqlx::query_scalar("SELECT id FROM inventory_items WHERE variant_id = ?")
                    .bind(line.variant_id.as_str())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(database_error)?;
            let Some(item_id) = item_id else {
                return Err(AppError::conflict(format!(
                    "insufficient stock for variant {}",
                    line.variant_id
                )));
            };

            let candidates = sqlx::query(
                "SELECT location_id, stocked_quantity, reserved_quantity \
                 FROM inventory_levels WHERE inventory_item_id = ? \
                   AND stocked_quantity - reserved_quantity > 0 \
                 ORDER BY CASE WHEN location_id = ? THEN 0 ELSE 1 END, location_id",
            )
            .bind(&item_id)
            .bind(DEFAULT_LOCATION_ID)
            .fetch_all(&mut *tx)
            .await
            .map_err(database_error)?;

            let mut remaining = line.quantity;
            for candidate in &candidates {
                if remaining == 0 {
                    break;
                }
                let stocked: i64 = candidate
                    .try_get("stocked_quantity")
                    .map_err(database_error)?;
                let reserved: i64 = candidate
                    .try_get("reserved_quantity")
                    .map_err(database_error)?;
                let available = available_quantity_from(stocked, reserved);
                let take = remaining.min(available);
                if take <= 0 {
                    continue;
                }
                let location_id: String =
                    candidate.try_get("location_id").map_err(database_error)?;
                let affected = sqlx::query(
                    "UPDATE inventory_levels \
                     SET reserved_quantity = reserved_quantity + ?, updated_at = ? \
                     WHERE inventory_item_id = ? AND location_id = ? \
                       AND stocked_quantity - reserved_quantity >= ?",
                )
                .bind(take)
                .bind(now)
                .bind(&item_id)
                .bind(&location_id)
                .bind(take)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                if affected.rows_affected() != 1 {
                    return Err(AppError::conflict(format!(
                        "insufficient stock for variant {}",
                        line.variant_id
                    )));
                }

                let reservation = InventoryReservation {
                    id: ReservationId::new(),
                    inventory_item_id: InventoryItemId::from_string(item_id.clone()),
                    location_id: InventoryLocationId::from_string(location_id),
                    quantity: take,
                    status: ReservationStatus::Reserved,
                    line_item_id: line.line_item_id.clone(),
                    checkout_id: Some(reference.checkout_id.clone()),
                    order_id: None,
                    expires_at: Some(expires_at),
                    metadata: Metadata::empty(),
                    created_at: now,
                    updated_at: now,
                };
                sqlx::query(
                    "INSERT INTO inventory_reservations \
                     (id, inventory_item_id, location_id, quantity, status, line_item_id, \
                      checkout_id, order_id, expires_at, metadata, created_at, updated_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(reservation.id.as_str())
                .bind(reservation.inventory_item_id.as_str())
                .bind(reservation.location_id.as_str())
                .bind(reservation.quantity)
                .bind(reservation.status.as_str())
                .bind(reservation.line_item_id.as_ref().map(LineItemId::as_str))
                .bind(reservation.checkout_id.as_ref().map(CheckoutId::as_str))
                .bind(Option::<String>::None)
                .bind(reservation.expires_at)
                .bind(reservation.metadata.to_json_string())
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                reservations.push(reservation);
                remaining -= take;
            }

            if remaining > 0 {
                return Err(AppError::conflict(format!(
                    "insufficient stock for variant {}",
                    line.variant_id
                )));
            }
        }

        tx.commit().await.map_err(database_error)?;
        Ok(reservations)
    }

    /// Fetches a reservation by identifier.
    pub async fn get_reservation(
        &self,
        id: &ReservationId,
    ) -> Result<InventoryReservation, AppError> {
        let row = sqlx::query(
            "SELECT id, inventory_item_id, location_id, quantity, status, line_item_id, \
             checkout_id, order_id, expires_at, metadata, created_at, updated_at \
             FROM inventory_reservations WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_reservation(&row),
            None => Err(AppError::not_found("inventory reservation", id)),
        }
    }

    /// Releases the given reservations, returning how many were released.
    pub async fn release(&self, reservation_ids: &[ReservationId]) -> Result<u64, AppError> {
        if reservation_ids.is_empty() {
            return Ok(0);
        }
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let mut rows = Vec::new();
        for id in reservation_ids {
            let row = sqlx::query(
                "SELECT id, inventory_item_id, location_id, quantity \
                 FROM inventory_reservations WHERE id = ? AND status = 'reserved'",
            )
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            if let Some(row) = row {
                rows.push(row_to_reserved(&row)?);
            }
        }
        let count = release_rows_in_tx(&mut tx, &rows, now).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(count)
    }

    /// Releases every active reservation owned by a checkout.
    pub async fn release_checkout(&self, checkout_id: &CheckoutId) -> Result<u64, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let rows = sqlx::query(
            "SELECT id, inventory_item_id, location_id, quantity \
             FROM inventory_reservations WHERE checkout_id = ? AND status = 'reserved'",
        )
        .bind(checkout_id.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
        let rows = rows
            .iter()
            .map(row_to_reserved)
            .collect::<Result<Vec<_>, _>>()?;
        let count = release_rows_in_tx(&mut tx, &rows, now).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(count)
    }

    /// Commits a checkout's reservations to an order.
    ///
    /// Committed stock leaves the warehouse: both stocked and reserved
    /// quantities are reduced and a movement is recorded.
    pub async fn commit(
        &self,
        checkout_id: &CheckoutId,
        order_id: &OrderId,
    ) -> Result<u64, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let rows = sqlx::query(
            "SELECT id, inventory_item_id, location_id, quantity \
             FROM inventory_reservations WHERE checkout_id = ? AND status = 'reserved'",
        )
        .bind(checkout_id.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
        let rows = rows
            .iter()
            .map(row_to_reserved)
            .collect::<Result<Vec<_>, _>>()?;
        if rows.is_empty() {
            let any_reservations: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM inventory_reservations WHERE checkout_id = ?",
            )
            .bind(checkout_id.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
            if any_reservations == 0 {
                // Digital-only checkouts legitimately have no reservations.
                tx.commit().await.map_err(database_error)?;
                return Ok(0);
            }
            let committed: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM inventory_reservations
                 WHERE checkout_id = ? AND status = 'committed' AND order_id = ?",
            )
            .bind(checkout_id.as_str())
            .bind(order_id.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
            if committed > 0 {
                tx.commit().await.map_err(database_error)?;
                return Ok(0);
            }
            return Err(AppError::conflict(
                "checkout reservations expired or were released before commit",
            ));
        }

        for row in &rows {
            let affected = sqlx::query(
                "UPDATE inventory_levels \
                 SET stocked_quantity = stocked_quantity - ?, \
                     reserved_quantity = reserved_quantity - ?, updated_at = ? \
                 WHERE inventory_item_id = ? AND location_id = ? \
                   AND stocked_quantity >= ? AND reserved_quantity >= ?",
            )
            .bind(row.quantity)
            .bind(row.quantity)
            .bind(now)
            .bind(row.inventory_item_id.as_str())
            .bind(row.location_id.as_str())
            .bind(row.quantity)
            .bind(row.quantity)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if affected.rows_affected() != 1 {
                return Err(AppError::conflict(
                    "inventory level no longer matches the reservation",
                ));
            }
            insert_movement(
                &mut tx,
                Movement {
                    item_id: &row.inventory_item_id,
                    location_id: &row.location_id,
                    quantity_delta: -row.quantity,
                    reason: "commit",
                    reference: Some(order_id.as_str()),
                    actor_id: None,
                    now,
                },
            )
            .await?;
            sqlx::query(
                "UPDATE inventory_reservations \
                 SET status = 'committed', order_id = ?, updated_at = ? WHERE id = ?",
            )
            .bind(order_id.as_str())
            .bind(now)
            .bind(row.id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        tx.commit().await.map_err(database_error)?;
        Ok(u64::try_from(rows.len()).unwrap_or(u64::MAX))
    }

    /// Reverses a committed order's inventory after cancellation.
    ///
    /// Committed reservations for the order are returned to stock, a movement
    /// is recorded and the reservations are marked released. Calling this twice
    /// is safe: the second call finds no committed reservations and returns
    /// zero.
    pub async fn restock_order(&self, order_id: &OrderId) -> Result<u64, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let rows = sqlx::query(
            "SELECT id, inventory_item_id, location_id, quantity \
             FROM inventory_reservations WHERE order_id = ? AND status = 'committed'",
        )
        .bind(order_id.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
        let rows = rows
            .iter()
            .map(row_to_reserved)
            .collect::<Result<Vec<_>, _>>()?;

        for row in &rows {
            let affected = sqlx::query(
                "UPDATE inventory_levels \
                 SET stocked_quantity = stocked_quantity + ?, updated_at = ? \
                 WHERE inventory_item_id = ? AND location_id = ?",
            )
            .bind(row.quantity)
            .bind(now)
            .bind(row.inventory_item_id.as_str())
            .bind(row.location_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if affected.rows_affected() != 1 {
                return Err(AppError::conflict(
                    "inventory level no longer matches the reservation",
                ));
            }
            insert_movement(
                &mut tx,
                Movement {
                    item_id: &row.inventory_item_id,
                    location_id: &row.location_id,
                    quantity_delta: row.quantity,
                    reason: "order_canceled",
                    reference: Some(order_id.as_str()),
                    actor_id: None,
                    now,
                },
            )
            .await?;
            sqlx::query(
                "UPDATE inventory_reservations SET status = 'released', updated_at = ? WHERE id = ?",
            )
            .bind(now)
            .bind(row.id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        tx.commit().await.map_err(database_error)?;
        Ok(u64::try_from(rows.len()).unwrap_or(u64::MAX))
    }

    /// Restocks quantities newly received on an order return.
    ///
    /// Each movement reference is derived from the return, order line and
    /// cumulative received quantity, so replaying a receive neither records a
    /// second movement nor increases stock twice. Partial receipts keep
    /// distinct references because their cumulative quantities differ.
    ///
    /// The destination location follows the order's committed reservation for
    /// the variant when one exists, otherwise `loc_default`. Lines whose
    /// variant has no inventory item are ignored. Returns the number of units
    /// actually restocked.
    pub async fn restock_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        lines: &[ReturnRestockLine],
    ) -> Result<i64, AppError> {
        if lines.is_empty() {
            return Ok(0);
        }
        for line in lines {
            validate_positive_quantity(line.quantity, "quantity")?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let mut restocked = 0_i64;
        for line in lines {
            let item_id: Option<String> =
                sqlx::query_scalar("SELECT id FROM inventory_items WHERE variant_id = ?")
                    .bind(line.variant_id.as_str())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(database_error)?;
            let Some(item_id) = item_id else {
                continue;
            };
            let item_id = InventoryItemId::from_string(item_id);

            let location_id: Option<String> = sqlx::query_scalar(
                "SELECT location_id FROM inventory_reservations \
                 WHERE order_id = ? AND inventory_item_id = ? AND status = 'committed' \
                 ORDER BY CASE WHEN location_id = ? THEN 0 ELSE 1 END, created_at, rowid \
                 LIMIT 1",
            )
            .bind(order_id.as_str())
            .bind(item_id.as_str())
            .bind(DEFAULT_LOCATION_ID)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            let location_id = match location_id {
                Some(location_id) => InventoryLocationId::from_string(location_id),
                None => {
                    ensure_default_location_in_tx(&mut tx, now).await?;
                    InventoryLocationId::from_string(DEFAULT_LOCATION_ID)
                }
            };
            ensure_level_in_tx(&mut tx, &item_id, &location_id, now).await?;

            let reference =
                return_restock_reference(return_id, &line.order_item_id, line.received_quantity);
            let inserted = sqlx::query(
                "INSERT OR IGNORE INTO inventory_movements \
                 (id, inventory_item_id, location_id, quantity_delta, reason, reference, \
                  actor_id, metadata, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, '{}', ?)",
            )
            .bind(new_id("imvt"))
            .bind(item_id.as_str())
            .bind(location_id.as_str())
            .bind(line.quantity)
            .bind(RETURN_RESTOCK_REASON)
            .bind(&reference)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if inserted.rows_affected() == 0 {
                // This receive has already been restocked.
                continue;
            }

            let affected = sqlx::query(
                "UPDATE inventory_levels \
                 SET stocked_quantity = stocked_quantity + ?, updated_at = ? \
                 WHERE inventory_item_id = ? AND location_id = ?",
            )
            .bind(line.quantity)
            .bind(now)
            .bind(item_id.as_str())
            .bind(location_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if affected.rows_affected() != 1 {
                return Err(AppError::conflict(
                    "inventory level no longer matches the restock movement",
                ));
            }
            restocked = restocked
                .checked_add(line.quantity)
                .ok_or_else(|| AppError::conflict("restocked quantity overflow"))?;
        }

        tx.commit().await.map_err(database_error)?;
        Ok(restocked)
    }

    /// Releases reservations whose expiry is at or before `now`.
    pub async fn expire_due(&self, now: Timestamp) -> Result<u64, AppError> {
        let mut tx = self.db.begin().await?;
        let rows = sqlx::query(
            "SELECT id, inventory_item_id, location_id, quantity \
             FROM inventory_reservations \
             WHERE status = 'reserved' AND expires_at IS NOT NULL AND expires_at <= ?",
        )
        .bind(now)
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
        let rows = rows
            .iter()
            .map(row_to_reserved)
            .collect::<Result<Vec<_>, _>>()?;
        let count = release_rows_in_tx(&mut tx, &rows, now).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(count)
    }

    /// Lists stock movements for an item, oldest first.
    pub async fn list_movements(
        &self,
        item_id: &InventoryItemId,
    ) -> Result<Vec<InventoryMovement>, AppError> {
        self.ensure_item_exists(item_id).await?;
        let rows = sqlx::query(
            "SELECT id, inventory_item_id, location_id, quantity_delta, reason, reference, \
             actor_id, metadata, created_at \
             FROM inventory_movements WHERE inventory_item_id = ? ORDER BY created_at, id",
        )
        .bind(item_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.iter().map(row_to_movement).collect()
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
