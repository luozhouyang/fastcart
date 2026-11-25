use super::*;

impl InventoryService {
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
}

impl InventoryService {
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
}

impl InventoryService {
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
}

impl InventoryService {
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
}

impl InventoryService {
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
}

impl InventoryService {
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
}

impl InventoryService {
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
}

impl InventoryService {
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
