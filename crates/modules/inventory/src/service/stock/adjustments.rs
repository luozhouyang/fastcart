use super::*;

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
}

impl InventoryService {
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
}
