//! After-sales and order-edit stock reconciliation.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, EventId, InventoryItemId, InventoryLocationId, OrderId, now_timestamp,
};
use sqlx::Row;

use crate::domain::{OrderEditStockLine, available_quantity_from};

use super::*;
impl InventoryService {
    /// Reverses the restock portion of a write-off inside the caller's
    /// transaction.
    ///
    /// Receive-return restocks inventory; writing the item off removes it from
    /// sellable stock. Reversing a write-off (for example when a return is
    /// reopened) must remove only the stock that the receive step actually
    /// restocked, and never drive the level negative. The caller supplies its
    /// transaction so the order-side state change and the inventory movement
    /// stay atomic.
    pub async fn reverse_return_write_off(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        request: ReverseWriteOffRequest<'_>,
    ) -> Result<i64, AppError> {
        let restock_prefix = format!(
            "return-restock:{}:{}:%",
            request.return_id, request.order_item_id
        );
        let restocked: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(quantity_delta), 0) FROM inventory_movements \
             WHERE reason = 'return_restock' AND reference LIKE ?",
        )
        .bind(&restock_prefix)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_error)?;
        if restocked <= 0 {
            return Ok(0);
        }
        let write_off_prefix = format!(
            "return-writeoff:{}:{}:%",
            request.return_id, request.order_item_id
        );
        let already_reversed: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(-quantity_delta), 0) FROM inventory_movements \
             WHERE reason = 'return_writeoff' AND reference LIKE ?",
        )
        .bind(&write_off_prefix)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_error)?;
        let reversible = restocked.saturating_sub(already_reversed);
        let quantity = request.increment.min(reversible);
        if quantity <= 0 {
            return Ok(0);
        }
        let movement = sqlx::query(
            "SELECT inventory_item_id, location_id FROM inventory_movements \
             WHERE reason = 'return_restock' AND reference LIKE ? \
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )
        .bind(&restock_prefix)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;
        let Some(movement) = movement else {
            return Ok(0);
        };
        let item_id: String = movement
            .try_get("inventory_item_id")
            .map_err(database_error)?;
        let location_id: String = movement.try_get("location_id").map_err(database_error)?;
        let reference = format!(
            "return-writeoff:{}:{}:{}",
            request.return_id, request.order_item_id, request.written_off_quantity
        );
        sqlx::query(
            "INSERT INTO inventory_movements \
             (id, inventory_item_id, location_id, quantity_delta, reason, reference, \
              actor_id, metadata, created_at) \
             VALUES (?, ?, ?, ?, 'return_writeoff', ?, NULL, '{}', ?)",
        )
        .bind(format!("imvt_{}", EventId::new().as_str()))
        .bind(&item_id)
        .bind(&location_id)
        .bind(-quantity)
        .bind(&reference)
        .bind(request.now)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
        let affected = sqlx::query(
            "UPDATE inventory_levels \
             SET stocked_quantity = stocked_quantity - ?, updated_at = ? \
             WHERE inventory_item_id = ? AND location_id = ? AND stocked_quantity >= ?",
        )
        .bind(quantity)
        .bind(request.now)
        .bind(&item_id)
        .bind(&location_id)
        .bind(quantity)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
        if affected.rows_affected() != 1 {
            return Err(AppError::conflict(
                "inventory level no longer matches the write-off reversal",
            ));
        }
        Ok(quantity)
    }

    /// Applies the stock changes of a confirmed order edit inside the caller's
    /// transaction.
    ///
    /// The caller supplies its transaction so the line rewrite on `order_items`
    /// and the stock movements commit together: an edit that adds two units and
    /// fails to find them must leave the order untouched.
    ///
    /// A positive delta leaves the warehouse for a line the edit added or grew;
    /// a negative delta returns the units a shrunk or removed line no longer
    /// holds. The levels are guarded in one pass before any write, so a batch
    /// either applies completely or not at all — the same all-or-nothing rule
    /// [`Self::reserve_lines`] gives a checkout.
    ///
    /// Each line carries a deterministic reference, and the partial unique
    /// index on `(reference) WHERE reason = 'order_edit'` makes a replay a
    /// no-op. A retry therefore converges on the same level even when only some
    /// movements were recorded by the previous attempt. Returns the number of
    /// units moved by this call (positive net for consumption).
    pub async fn apply_order_edit_stock(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        order_id: &OrderId,
        lines: &[OrderEditStockLine],
    ) -> Result<i64, AppError> {
        if lines.is_empty() {
            return Ok(0);
        }
        let mut references = HashSet::with_capacity(lines.len());
        for line in lines {
            validate_order_edit_delta(line.delta)?;
            // A repeated reference would be ignored by the unique index and
            // silently leave one line's stock unmoved; reject it instead.
            if !references.insert(line.reference.as_str()) {
                return Err(AppError::validation(format!(
                    "duplicate order edit stock reference {}",
                    line.reference
                )));
            }
        }

        let variants = lines
            .iter()
            .map(|line| line.variant_id.clone())
            .collect::<Vec<_>>();
        let items = load_items_by_variant_tx(tx, &variants).await?;

        let now = now_timestamp();
        // What this order may still return to stock. A reduction may only give
        // back units the order actually took out: the checkout's committed
        // reservations plus what earlier edits consumed, minus what earlier
        // edits already restocked. Without this budget an edit on an order
        // placed outside checkout would invent stock that was never taken.
        let item_ids = items.values().cloned().collect::<Vec<_>>();
        let mut restock_budget = restock_budget_by_item(tx, order_id, &item_ids).await?;

        // Resolve the target level of every line up front. Consumption and
        // restock both follow the order's committed reservation for the
        // variant, then `loc_default`, exactly like the after-sales flows.
        let mut resolved = Vec::with_capacity(lines.len());
        let mut locations: HashMap<String, InventoryLocationId> = HashMap::new();
        for line in lines {
            let Some(item_id) = items.get(line.variant_id.as_str()) else {
                // A variant without an inventory item has no stock ledger, so
                // there is nothing to move in either direction. Returns and
                // exchanges treat such lines the same way: an unmanaged
                // product simply has no level to reconcile against.
                continue;
            };
            let item_id = InventoryItemId::from_string(item_id.clone());
            let delta = if line.delta < 0 {
                let budget = restock_budget
                    .entry(item_id.as_str().to_owned())
                    .or_insert(0);
                let take = (-line.delta).min((*budget).max(0));
                if take == 0 {
                    // The order never took these units, so there is nothing to
                    // return to the shelf.
                    continue;
                }
                *budget -= take;
                -take
            } else {
                line.delta
            };
            let location_id = match locations.get(item_id.as_str()) {
                Some(location_id) => location_id.clone(),
                None => {
                    let location_id = committed_location(tx, order_id, &item_id).await?;
                    locations.insert(item_id.as_str().to_owned(), location_id.clone());
                    location_id
                }
            };
            ensure_level_in_tx(tx, &item_id, &location_id, now).await?;
            resolved.push((
                line.reference.clone(),
                line.variant_id.clone(),
                delta,
                item_id,
                location_id,
            ));
        }

        // A restock only adds stock, so it can always be applied. A
        // consumption must not take reserved stock and must never drive the
        // level below zero; if one level cannot cover the whole batch the
        // batch fails before anything is written. Demand is summed per level
        // first so two lines for the same variant cannot each pass a check
        // that only one of them can honour.
        let mut demand: HashMap<(String, String), (i64, String)> = HashMap::new();
        for (_, variant_id, delta, item_id, location_id) in &resolved {
            if *delta <= 0 {
                continue;
            }
            let entry = demand
                .entry((item_id.as_str().to_owned(), location_id.as_str().to_owned()))
                .or_insert_with(|| (0, variant_id.to_string()));
            entry.0 = entry
                .0
                .checked_add(*delta)
                .ok_or_else(|| AppError::validation("order edit quantity overflow"))?;
        }
        for ((item_id, location_id), (needed, variant_id)) in demand {
            let item_id = InventoryItemId::from_string(item_id);
            let location_id = InventoryLocationId::from_string(location_id);
            let available = available_quantity_in_tx(tx, &item_id, &location_id).await?;
            if available < needed {
                return Err(AppError::conflict(format!(
                    "insufficient stock for variant {variant_id}: {available} available, {needed} needed"
                )));
            }
        }

        let mut moved = 0_i64;
        for (reference, variant_id, delta, item_id, location_id) in resolved {
            // Record the movement first: the partial unique index makes a
            // replayed reference a no-op, so the level below is only touched
            // by the attempt that owns this line.
            let inserted = sqlx::query(
                "INSERT OR IGNORE INTO inventory_movements \
                 (id, inventory_item_id, location_id, quantity_delta, reason, reference, \
                  actor_id, metadata, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, '{}', ?)",
            )
            .bind(new_id("imvt"))
            .bind(item_id.as_str())
            .bind(location_id.as_str())
            .bind(-delta)
            .bind(ORDER_EDIT_REASON)
            .bind(&reference)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
            if inserted.rows_affected() == 0 {
                continue;
            }

            let affected = if delta > 0 {
                sqlx::query(
                    "UPDATE inventory_levels \
                     SET stocked_quantity = stocked_quantity - ?, updated_at = ? \
                     WHERE inventory_item_id = ? AND location_id = ? \
                       AND stocked_quantity - reserved_quantity >= ?",
                )
                .bind(delta)
                .bind(now)
                .bind(item_id.as_str())
                .bind(location_id.as_str())
                .bind(delta)
                .execute(&mut **tx)
                .await
                .map_err(database_error)?
            } else {
                sqlx::query(
                    "UPDATE inventory_levels \
                     SET stocked_quantity = stocked_quantity + ?, updated_at = ? \
                     WHERE inventory_item_id = ? AND location_id = ?",
                )
                .bind(-delta)
                .bind(now)
                .bind(item_id.as_str())
                .bind(location_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(database_error)?
            };
            if affected.rows_affected() != 1 {
                return Err(AppError::conflict(format!(
                    "inventory level no longer matches the order edit movement of variant {variant_id}"
                )));
            }
            moved = moved
                .checked_add(delta)
                .ok_or_else(|| AppError::conflict("moved quantity overflow"))?;
        }
        Ok(moved)
    }
}

/// Units an order may still return to stock, per inventory item.
///
/// The budget is the checkout's committed reservations plus what `order_edit`
/// movements consumed, minus what they already restocked. It is read in the
/// caller's transaction, so it sees the same snapshot the movements will be
/// written against.
async fn restock_budget_by_item(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    order_id: &OrderId,
    item_ids: &[String],
) -> Result<HashMap<String, i64>, AppError> {
    let mut budget: HashMap<String, i64> = HashMap::new();
    for chunk in item_ids.chunks(SQLITE_PARAMETER_LIMIT) {
        let placeholders = sql_markers(chunk.len());
        let committed_sql = format!(
            "SELECT inventory_item_id, COALESCE(SUM(quantity), 0) \
             FROM inventory_reservations \
             WHERE order_id = ? AND status = 'committed' \
               AND inventory_item_id IN ({placeholders}) \
             GROUP BY inventory_item_id"
        );
        let mut committed =
            sqlx::query_as::<_, (String, i64)>(&committed_sql).bind(order_id.as_str());
        for item_id in chunk {
            committed = committed.bind(item_id);
        }
        for (item_id, quantity) in committed
            .fetch_all(&mut **tx)
            .await
            .map_err(database_error)?
        {
            budget.insert(item_id, quantity);
        }

        // A reference is `order-edit:{order}:{edit}:{line}`. The prefix range
        // is an exact match that can use the partial unique index on
        // `reference`; the exclusive upper bound is the prefix followed by the
        // highest code point, so every generated (ASCII) reference sorts below
        // it. A `LIKE` prefix would be case-insensitive and could not use the
        // index.
        let edited_sql = format!(
            "SELECT inventory_item_id, COALESCE(SUM(-quantity_delta), 0) \
             FROM inventory_movements \
             WHERE reason = ? AND reference >= ? AND reference < ? \
               AND inventory_item_id IN ({placeholders}) \
             GROUP BY inventory_item_id"
        );
        let edited_from = format!("order-edit:{}:", order_id.as_str());
        let edited_before = format!("order-edit:{}:\u{10FFFF}", order_id.as_str());
        let mut edited = sqlx::query_as::<_, (String, i64)>(&edited_sql)
            .bind(ORDER_EDIT_REASON)
            .bind(edited_from)
            .bind(edited_before);
        for item_id in chunk {
            edited = edited.bind(item_id);
        }
        for (item_id, quantity) in edited.fetch_all(&mut **tx).await.map_err(database_error)? {
            *budget.entry(item_id).or_insert(0) += quantity;
        }
    }
    Ok(budget)
}

/// Rejects a zero delta and a magnitude outside the quantity range.
fn validate_order_edit_delta(delta: i64) -> Result<(), AppError> {
    if delta == 0 {
        return Err(AppError::validation(
            "an order edit stock line must move a non-zero quantity",
        ));
    }
    let magnitude = delta
        .checked_abs()
        .ok_or_else(|| AppError::validation("order edit quantity overflow"))?;
    fastcart_kernel::validate_quantity(magnitude, false)
}

/// The location an order's stock change belongs to.
///
/// The order's committed reservation for the variant is authoritative because
/// it records the location the checkout actually took the unit from; an order
/// placed without reservations falls back to `loc_default`, mirroring
/// [`Self::consume_stock`] and [`Self::restock_return`].
async fn committed_location(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    order_id: &OrderId,
    item_id: &InventoryItemId,
) -> Result<InventoryLocationId, AppError> {
    let location_id: Option<String> = sqlx::query_scalar(
        "SELECT location_id FROM inventory_reservations \
         WHERE order_id = ? AND inventory_item_id = ? AND status = 'committed' \
         ORDER BY CASE WHEN location_id = ? THEN 0 ELSE 1 END, created_at, rowid \
         LIMIT 1",
    )
    .bind(order_id.as_str())
    .bind(item_id.as_str())
    .bind(DEFAULT_LOCATION_ID)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(match location_id {
        Some(location_id) => InventoryLocationId::from_string(location_id),
        None => {
            ensure_default_location_in_tx(tx, now_timestamp()).await?;
            InventoryLocationId::from_string(DEFAULT_LOCATION_ID)
        }
    })
}

/// Sellable quantity of one `(item, location)` pair.
async fn available_quantity_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    item_id: &InventoryItemId,
    location_id: &InventoryLocationId,
) -> Result<i64, AppError> {
    let level = fetch_level_in_tx(tx, item_id, location_id).await?;
    Ok(available_quantity_from(
        level.stocked_quantity,
        level.reserved_quantity,
    ))
}
