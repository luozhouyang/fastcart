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

impl InventoryService {
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
        // An increase only adds stock, so it may always be applied: receiving
        // goods against a pending backorder, or restocking the negative level
        // left behind by a committed one, must not be rejected for being
        // partial. A decrease must not consume reserved stock and must never
        // drive the level below zero.
        let affected = if delta > 0 {
            sqlx::query(
                "UPDATE inventory_levels SET stocked_quantity = stocked_quantity + ?, updated_at = ? \
                 WHERE inventory_item_id = ? AND location_id = ?",
            )
            .bind(delta)
            .bind(now)
            .bind(item_id.as_str())
            .bind(location_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?
        } else {
            sqlx::query(
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
            .map_err(database_error)?
        };
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

    /// Brings stock up to the cumulative received quantities of an order
    /// return.
    ///
    /// Each line's `received_quantity` is the target cumulative quantity for
    /// that order line. The service compares it with the movements already
    /// recorded for the line and applies only the missing delta, so replaying a
    /// receive and retrying after the order transaction committed but the
    /// restock failed both converge on the same stock level. Partial receipts
    /// keep distinct references because their cumulative quantities differ.
    ///
    /// The destination location follows the order's committed reservation for
    /// the variant when one exists, otherwise `loc_default`. Lines whose
    /// variant has no inventory item are ignored. Returns the number of units
    /// actually restocked by this call.
    pub async fn restock_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        lines: &[ReturnRestockLine],
    ) -> Result<i64, AppError> {
        self.restock_return_inner(order_id, return_id, lines, None)
            .await
    }

    /// Restocks received return quantities and appends an audit row atomically.
    pub async fn restock_return_audited(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        lines: &[ReturnRestockLine],
        audit: RestockAudit,
    ) -> Result<i64, AppError> {
        self.restock_return_inner(order_id, return_id, lines, Some(audit))
            .await
    }

    async fn restock_return_inner(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        lines: &[ReturnRestockLine],
        audit: Option<RestockAudit>,
    ) -> Result<i64, AppError> {
        if lines.is_empty() && audit.is_none() {
            return Ok(0);
        }
        for line in lines {
            validate_positive_quantity(line.quantity, "quantity")?;
        }
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let variants = lines
            .iter()
            .map(|line| line.variant_id.clone())
            .collect::<Vec<_>>();
        let items = load_items_by_variant_tx(&mut tx, &variants).await?;
        let mut restocked = 0_i64;
        for line in lines {
            let target = line.received_quantity.max(line.quantity);
            if target <= 0 {
                continue;
            }
            let Some(item_id) = items.get(line.variant_id.as_str()) else {
                continue;
            };
            let item_id = InventoryItemId::from_string(item_id.clone());

            // Reconcile against the restock movements already persisted for
            // this return line. A retry after a failed restock sees the
            // original receive row but no movement and restocks the difference.
            let prefix = format!(
                "return-restock:{}:{}:%",
                return_id.as_str(),
                line.order_item_id.as_str()
            );
            let already: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(quantity_delta), 0) FROM inventory_movements \
                 WHERE reason = ? AND reference LIKE ?",
            )
            .bind(RETURN_RESTOCK_REASON)
            .bind(&prefix)
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
            let delta = target.saturating_sub(already);
            if delta <= 0 {
                continue;
            }

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

            let reference = return_restock_reference(return_id, &line.order_item_id, target);
            let inserted = sqlx::query(
                "INSERT OR IGNORE INTO inventory_movements \
                 (id, inventory_item_id, location_id, quantity_delta, reason, reference, \
                  actor_id, metadata, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, '{}', ?)",
            )
            .bind(new_id("imvt"))
            .bind(item_id.as_str())
            .bind(location_id.as_str())
            .bind(delta)
            .bind(RETURN_RESTOCK_REASON)
            .bind(&reference)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if inserted.rows_affected() == 0 {
                continue;
            }

            let affected = sqlx::query(
                "UPDATE inventory_levels \
                 SET stocked_quantity = stocked_quantity + ?, updated_at = ? \
                 WHERE inventory_item_id = ? AND location_id = ?",
            )
            .bind(delta)
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
                .checked_add(delta)
                .ok_or_else(|| AppError::conflict("restocked quantity overflow"))?;
        }

        if let Some(audit) = audit {
            let mut data = audit.data.clone();
            if let Some(object) = data.as_object_mut() {
                object.insert("restocked_units".to_owned(), serde_json::json!(restocked));
            }
            self.append_audit(
                &mut tx,
                &audit.action,
                &audit.resource_type,
                &audit.resource_id,
                data,
            )
            .await?;
        }

        tx.commit().await.map_err(database_error)?;
        Ok(restocked)
    }

    /// Restocks quantities received on an accepted exchange.
    ///
    /// The movement reference is derived from the exchange and its line, so
    /// accepting the same exchange again never increases stock twice. Unknown
    /// variants are ignored, mirroring [`Self::restock_return`].
    pub async fn restock_exchange(
        &self,
        order_id: &OrderId,
        exchange_id: &str,
        lines: &[ExchangeRestockLine],
    ) -> Result<i64, AppError> {
        self.restock_exchange_inner(order_id, exchange_id, lines, None)
            .await
    }

    /// Restocks accepted exchange inbound quantities and appends an audit row
    /// atomically.
    pub async fn restock_exchange_audited(
        &self,
        order_id: &OrderId,
        exchange_id: &str,
        lines: &[ExchangeRestockLine],
        audit: RestockAudit,
    ) -> Result<i64, AppError> {
        self.restock_exchange_inner(order_id, exchange_id, lines, Some(audit))
            .await
    }

    async fn restock_exchange_inner(
        &self,
        order_id: &OrderId,
        exchange_id: &str,
        lines: &[ExchangeRestockLine],
        audit: Option<RestockAudit>,
    ) -> Result<i64, AppError> {
        if lines.is_empty() && audit.is_none() {
            return Ok(0);
        }
        for line in lines {
            validate_positive_quantity(line.quantity, "quantity")?;
        }
        let entries = lines
            .iter()
            .map(|line| RestockEntry {
                variant_id: &line.variant_id,
                quantity: line.quantity,
                reference: exchange_restock_reference(exchange_id, &line.exchange_item_id),
            })
            .collect::<Vec<_>>();
        self.restock_variant_lines(order_id, EXCHANGE_RESTOCK_REASON, &entries, audit.as_ref())
            .await
    }

    /// Consumes stock for after-sales replacement variants.
    ///
    /// The location follows the order's committed reservation for the variant
    /// when one exists, otherwise `loc_default`. Each line carries a unique
    /// reference; replaying the same reference is a no-op. A line whose variant
    /// has no inventory item or whose location does not have enough available
    /// stock fails the whole batch atomically. Returns the number of units
    /// consumed by this call.
    pub async fn consume_stock(
        &self,
        order_id: &OrderId,
        lines: &[StockConsumption],
    ) -> Result<i64, AppError> {
        Ok(self.consume_stock_inner(order_id, lines).await?.0)
    }

    /// Consumes stock and reports which movement references this call inserted.
    ///
    /// Callers that need to undo a later failure can pass these references to
    /// [`Self::restore_consumed_stock`]. References already consumed by another
    /// attempt are not reported, so a concurrent retry never reverses stock it
    /// did not remove.
    pub async fn consume_stock_tracked(
        &self,
        order_id: &OrderId,
        lines: &[StockConsumption],
    ) -> Result<(i64, Vec<String>), AppError> {
        self.consume_stock_inner(order_id, lines).await
    }

    async fn consume_stock_inner(
        &self,
        order_id: &OrderId,
        lines: &[StockConsumption],
    ) -> Result<(i64, Vec<String>), AppError> {
        if lines.is_empty() {
            return Ok((0, Vec::new()));
        }
        for line in lines {
            validate_positive_quantity(line.quantity, "quantity")?;
            validate_reason(&line.reference)?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let variants = lines
            .iter()
            .map(|line| line.variant_id.clone())
            .collect::<Vec<_>>();
        let items = load_items_by_variant_tx(&mut tx, &variants).await?;
        let mut consumed = 0_i64;
        let mut inserted_references = Vec::new();
        for line in lines {
            let Some(item_id) = items.get(line.variant_id.as_str()) else {
                return Err(insufficient_stock_for_variant(line.variant_id.as_str()));
            };
            let item_id = InventoryItemId::from_string(item_id.clone());

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

            let inserted = sqlx::query(
                "INSERT OR IGNORE INTO inventory_movements \
                 (id, inventory_item_id, location_id, quantity_delta, reason, reference, \
                  actor_id, metadata, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, '{}', ?)",
            )
            .bind(new_id("imvt"))
            .bind(item_id.as_str())
            .bind(location_id.as_str())
            .bind(-line.quantity)
            .bind(AFTER_SALES_CONSUME_REASON)
            .bind(&line.reference)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if inserted.rows_affected() == 0 {
                // This replacement has already consumed its stock.
                continue;
            }
            inserted_references.push(line.reference.clone());

            let affected = sqlx::query(
                "UPDATE inventory_levels \
                 SET stocked_quantity = stocked_quantity - ?, updated_at = ? \
                 WHERE inventory_item_id = ? AND location_id = ? \
                   AND stocked_quantity - reserved_quantity >= ?",
            )
            .bind(line.quantity)
            .bind(now)
            .bind(item_id.as_str())
            .bind(location_id.as_str())
            .bind(line.quantity)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if affected.rows_affected() != 1 {
                return Err(AppError::conflict(format!(
                    "insufficient stock for variant {}",
                    line.variant_id
                )));
            }
            consumed = consumed
                .checked_add(line.quantity)
                .ok_or_else(|| AppError::conflict("consumed quantity overflow"))?;
        }

        tx.commit().await.map_err(database_error)?;
        Ok((consumed, inserted_references))
    }

    /// Reverses consumptions inserted by a failed after-sales provider call.
    ///
    /// Only the supplied references are considered, so a concurrent attempt
    /// that already consumed the same lines keeps its stock movement. The
    /// movements are removed rather than offset so a later retry can consume
    /// the replacements again. Returns the number of units restored.
    pub async fn restore_consumed_stock(&self, references: &[String]) -> Result<i64, AppError> {
        if references.is_empty() {
            return Ok(0);
        }
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let mut restored = 0_i64;
        for reference in references {
            let row = sqlx::query(
                "SELECT inventory_item_id, location_id, quantity_delta \
                 FROM inventory_movements WHERE reason = ? AND reference = ?",
            )
            .bind(AFTER_SALES_CONSUME_REASON)
            .bind(reference)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?;
            let Some(row) = row else {
                continue;
            };
            let quantity_delta: i64 = row.try_get("quantity_delta").map_err(database_error)?;
            if quantity_delta >= 0 {
                continue;
            }
            let item_id: String = row.try_get("inventory_item_id").map_err(database_error)?;
            let location_id: String = row.try_get("location_id").map_err(database_error)?;
            let deleted =
                sqlx::query("DELETE FROM inventory_movements WHERE reason = ? AND reference = ?")
                    .bind(AFTER_SALES_CONSUME_REASON)
                    .bind(reference)
                    .execute(&mut *tx)
                    .await
                    .map_err(database_error)?;
            if deleted.rows_affected() == 0 {
                continue;
            }
            let quantity = -quantity_delta;
            let affected = sqlx::query(
                "UPDATE inventory_levels \
                 SET stocked_quantity = stocked_quantity + ?, updated_at = ? \
                 WHERE inventory_item_id = ? AND location_id = ?",
            )
            .bind(quantity)
            .bind(now)
            .bind(&item_id)
            .bind(&location_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if affected.rows_affected() != 1 {
                return Err(AppError::conflict(
                    "inventory level no longer matches the consumed movement",
                ));
            }
            restored = restored
                .checked_add(quantity)
                .ok_or_else(|| AppError::conflict("restored quantity overflow"))?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(restored)
    }

    /// Shared implementation for return and exchange restocking.
    async fn restock_variant_lines(
        &self,
        order_id: &OrderId,
        reason: &str,
        lines: &[RestockEntry<'_>],
        audit: Option<&RestockAudit>,
    ) -> Result<i64, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let variants = lines
            .iter()
            .map(|line| line.variant_id.clone())
            .collect::<Vec<_>>();
        let items = load_items_by_variant_tx(&mut tx, &variants).await?;
        let mut restocked = 0_i64;
        for line in lines {
            let Some(item_id) = items.get(line.variant_id.as_str()) else {
                continue;
            };
            let item_id = InventoryItemId::from_string(item_id.clone());

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
            .bind(reason)
            .bind(&line.reference)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if inserted.rows_affected() == 0 {
                // This restock has already been applied.
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

        if let Some(audit) = audit {
            let mut data = audit.data.clone();
            if let Some(object) = data.as_object_mut() {
                object.insert("restocked_units".to_owned(), serde_json::json!(restocked));
            }
            self.append_audit(
                &mut tx,
                &audit.action,
                &audit.resource_type,
                &audit.resource_id,
                data,
            )
            .await?;
        }

        tx.commit().await.map_err(database_error)?;
        Ok(restocked)
    }
}
