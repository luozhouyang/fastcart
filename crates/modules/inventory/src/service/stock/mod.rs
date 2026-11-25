//! Stock adjustments, transfers, restocks and consumption.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, InventoryItemId, InventoryLocationId, OrderId, ReturnId, VariantId, now_timestamp,
};
use sqlx::{Row, Sqlite, Transaction};

use crate::domain::{
    ExchangeRestockLine, InventoryLevel, ReturnRestockLine, StockConsumption,
    exchange_restock_reference, return_restock_reference, validate_positive_quantity,
    validate_reason,
};

use super::*;
struct RestockEntry<'a> {
    variant_id: &'a VariantId,
    quantity: i64,
    reference: String,
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

mod adjustments;
mod consumption;
mod restocks;
