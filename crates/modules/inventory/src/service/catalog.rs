//! Inventory locations, items, levels and movement reads.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, InventoryItemId, InventoryLocationId, Metadata, PageResult, Pagination, VariantId,
    now_timestamp, validate_length, validate_metadata,
};
use sqlx::Row;

use crate::domain::{
    CreateInventoryItemInput, CreateLocationInput, InventoryItem, InventoryLevel,
    InventoryLocation, InventoryMovement, UpdateLocationInput, available_quantity_from,
    validate_name,
};

use super::*;
fn metadata_string(metadata: Option<Metadata>) -> String {
    metadata.unwrap_or_default().to_json_string()
}

fn validate_metadata_option(metadata: Option<&Metadata>) -> Result<(), AppError> {
    if let Some(metadata) = metadata {
        validate_metadata(metadata)?;
    }
    Ok(())
}

impl InventoryService {
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
        .map_err(|error| {
            // The pre-checks above race with concurrent creators; the unique
            // indexes are the source of truth, so surface those races as
            // conflicts rather than internal database errors.
            if is_unique_violation(&error) {
                AppError::conflict("inventory item already exists for this sku or variant")
            } else {
                database_error(error)
            }
        })?;
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
            .map_err(|error| {
                // The pre-check above races with concurrent attaches; the
                // unique variant index is the source of truth.
                if is_unique_violation(&error) {
                    AppError::conflict(format!(
                        "variant already has an inventory item: {variant_id}"
                    ))
                } else {
                    database_error(error)
                }
            })?;
        self.get_item(item_id).await
    }

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

    /// Returns total available quantities for many variants in one query.
    ///
    /// Variants without an inventory item are absent from the result; callers
    /// should treat missing entries as zero. This is the batch counterpart of
    /// [`Self::variant_availability`] used by storefront product listings.
    pub async fn variant_availability_many(
        &self,
        variant_ids: &[VariantId],
    ) -> Result<std::collections::HashMap<String, i64>, AppError> {
        if variant_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let placeholders = std::iter::repeat_n("?", variant_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT ii.variant_id, il.stocked_quantity, il.reserved_quantity \
             FROM inventory_items ii \
             JOIN inventory_levels il ON il.inventory_item_id = ii.id \
             WHERE ii.variant_id IN ({placeholders})"
        );
        let mut query = sqlx::query(&sql);
        for variant_id in variant_ids {
            query = query.bind(variant_id.as_str());
        }
        let rows = query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut totals = std::collections::HashMap::new();
        for row in &rows {
            let variant_id: String = row.try_get("variant_id").map_err(database_error)?;
            let stocked: i64 = row.try_get("stocked_quantity").map_err(database_error)?;
            let reserved: i64 = row.try_get("reserved_quantity").map_err(database_error)?;
            let total = totals.entry(variant_id).or_insert(0_i64);
            *total = total.saturating_add(available_quantity_from(stocked, reserved));
        }
        Ok(totals)
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
}
