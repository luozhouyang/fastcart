use super::*;

impl InventoryService {
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
}

impl InventoryService {
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
}

impl InventoryService {
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
}

impl InventoryService {
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
}
