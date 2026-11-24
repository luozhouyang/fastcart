//! Stock reservations, commits, releases and expiry.

use std::collections::HashMap;

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, Metadata, OrderId,
    ReservationId, Timestamp, now_timestamp,
};
use sqlx::{QueryBuilder, Row, Sqlite, Transaction};

use crate::domain::{
    InventoryReservation, ReservationLine, ReservationLinePolicy, ReservationRebinding,
    ReservationReference, ReservationStatus, available_quantity_from, validate_positive_quantity,
    validate_ttl,
};

use super::*;
/// One `(item, location)` level considered by a batched reservation.
#[derive(Debug)]
struct LevelCandidate {
    location_id: String,
    stocked_quantity: i64,
    reserved_quantity: i64,
}

/// One allocated quantity to apply to an `(item, location)` level.
#[derive(Debug)]
struct LevelAllocation {
    item_id: String,
    location_id: String,
    quantity: i64,
}

/// Loads the levels of every item in one pass, in allocation order.
///
/// Rows keep the `loc_default` first, then ascending location identifier order
/// the per-line implementation used, and items without levels simply have no
/// entry.
async fn load_levels_by_item_tx(
    tx: &mut Transaction<'_, Sqlite>,
    item_ids: &[String],
) -> Result<HashMap<String, Vec<LevelCandidate>>, AppError> {
    let mut by_item: HashMap<String, Vec<LevelCandidate>> = HashMap::new();
    for chunk in item_ids.chunks(SQLITE_PARAMETER_LIMIT) {
        let sql = format!(
            "SELECT inventory_item_id, location_id, stocked_quantity, reserved_quantity \
             FROM inventory_levels WHERE inventory_item_id IN ({}) \
             ORDER BY CASE WHEN location_id = ? THEN 0 ELSE 1 END, location_id",
            sql_markers(chunk.len())
        );
        let mut query = sqlx::query(&sql);
        for item_id in chunk {
            query = query.bind(item_id);
        }
        let rows = query
            .bind(DEFAULT_LOCATION_ID)
            .fetch_all(&mut **tx)
            .await
            .map_err(database_error)?;
        for row in rows {
            let item_id: String = row.try_get("inventory_item_id").map_err(database_error)?;
            by_item.entry(item_id).or_default().push(LevelCandidate {
                location_id: row.try_get("location_id").map_err(database_error)?,
                stocked_quantity: row.try_get("stocked_quantity").map_err(database_error)?,
                reserved_quantity: row.try_get("reserved_quantity").map_err(database_error)?,
            });
        }
    }
    Ok(by_item)
}

/// Applies the allocations with one conditional update per item.
///
/// Repeated allocations of the same level are folded into one term so a single
/// statement can carry them; the `stocked_quantity - reserved_quantity >= take`
/// guard of every term still runs inside SQLite, and a level that fails it
/// rolls the whole batch back with the variant's conflict error.
async fn apply_level_allocations(
    tx: &mut Transaction<'_, Sqlite>,
    allocations: &[LevelAllocation],
    now: Timestamp,
    variants_by_item: &HashMap<String, String>,
) -> Result<(), AppError> {
    let mut by_item: Vec<(String, Vec<(String, i64)>)> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();
    for allocation in allocations {
        let position = *positions
            .entry(allocation.item_id.clone())
            .or_insert_with(|| {
                by_item.push((allocation.item_id.clone(), Vec::new()));
                by_item.len() - 1
            });
        let locations = &mut by_item[position].1;
        match locations
            .iter_mut()
            .find(|(location, _)| location == &allocation.location_id)
        {
            Some((_, quantity)) => *quantity += allocation.quantity,
            None => locations.push((allocation.location_id.clone(), allocation.quantity)),
        }
    }

    for (item_id, locations) in &by_item {
        // Five bound parameters per location: two in the `SET` case, one in the
        // `IN` list and two in the guard case.
        for chunk in locations.chunks(SQLITE_PARAMETER_LIMIT / 5) {
            let mut builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
                "UPDATE inventory_levels \
                 SET reserved_quantity = reserved_quantity + CASE location_id",
            );
            push_location_case(&mut builder, chunk);
            builder
                .push(" ELSE 0 END, updated_at = ")
                .push_bind(now)
                .push(" WHERE inventory_item_id = ")
                .push_bind(item_id)
                .push(" AND location_id IN (");
            {
                let mut separated = builder.separated(", ");
                for (location_id, _) in chunk {
                    separated.push_bind(location_id);
                }
            }
            builder.push(") AND stocked_quantity - reserved_quantity >= CASE location_id");
            push_location_case(&mut builder, chunk);
            builder.push(" ELSE 0 END");

            let affected = builder
                .build()
                .execute(&mut **tx)
                .await
                .map_err(database_error)?;
            if affected.rows_affected() != chunk.len() as u64 {
                // Every item belongs to the variant it was loaded for.
                let variant_id = variants_by_item
                    .get(item_id)
                    .map(String::as_str)
                    .unwrap_or(item_id);
                return Err(insufficient_stock_for_variant(variant_id));
            }
        }
    }
    Ok(())
}

/// Applies the backorder allocations of a batch.
///
/// Every allocation is recorded on the default location, at most one per item.
/// Unlike [`apply_level_allocations`] there is no availability guard: the
/// shortfall being recorded is exactly the quantity that no location could
/// cover, so the level's reserved quantity is allowed to pass its stocked
/// quantity. The level row is expected to exist — the caller ensures it — and a
/// missing one fails loudly so the batch rolls back instead of silently losing
/// the backorder.
async fn apply_backorder_allocations(
    tx: &mut Transaction<'_, Sqlite>,
    allocations: &[LevelAllocation],
    now: Timestamp,
) -> Result<(), AppError> {
    for allocation in allocations {
        let affected = sqlx::query(
            "UPDATE inventory_levels \
             SET reserved_quantity = reserved_quantity + ?, updated_at = ? \
             WHERE inventory_item_id = ? AND location_id = ?",
        )
        .bind(allocation.quantity)
        .bind(now)
        .bind(allocation.item_id.as_str())
        .bind(allocation.location_id.as_str())
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
        if affected.rows_affected() != 1 {
            return Err(AppError::conflict(
                "inventory level no longer matches the backorder reservation",
            ));
        }
    }
    Ok(())
}

/// Pushes the `WHEN location THEN quantity` terms of a level update.
fn push_location_case<'a>(builder: &mut QueryBuilder<'a, Sqlite>, locations: &'a [(String, i64)]) {
    for (location_id, quantity) in locations {
        builder
            .push(" WHEN ")
            .push_bind(location_id)
            .push(" THEN ")
            .push_bind(*quantity);
    }
}

/// Inserts the reservations of one batch with a single multi-row insert.
async fn insert_reservations(
    tx: &mut Transaction<'_, Sqlite>,
    reservations: &[InventoryReservation],
    now: Timestamp,
) -> Result<(), AppError> {
    // Twelve bound parameters per row.
    for chunk in reservations.chunks(SQLITE_PARAMETER_LIMIT / 12) {
        let mut builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
            "INSERT INTO inventory_reservations \
             (id, inventory_item_id, location_id, quantity, status, line_item_id, \
              checkout_id, order_id, expires_at, metadata, created_at, updated_at) ",
        );
        builder.push_values(chunk, |mut row, reservation| {
            row.push_bind(reservation.id.as_str())
                .push_bind(reservation.inventory_item_id.as_str())
                .push_bind(reservation.location_id.as_str())
                .push_bind(reservation.quantity)
                .push_bind(reservation.status.as_str())
                .push_bind(reservation.line_item_id.as_ref().map(LineItemId::as_str))
                .push_bind(reservation.checkout_id.as_ref().map(CheckoutId::as_str))
                .push_bind(Option::<String>::None)
                .push_bind(reservation.expires_at)
                .push_bind(reservation.metadata.to_json_string())
                .push_bind(now)
                .push_bind(now);
        });
        builder
            .build()
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
    }
    Ok(())
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

impl InventoryService {
    /// Atomically reserves stock for every line or fails without changing state.
    ///
    /// Locations are visited deterministically: `loc_default` first, then by
    /// ascending location identifier. A line that cannot be fully satisfied
    /// rolls back the entire batch with a conflict naming the variant. The
    /// items, levels and writes of the whole batch are read and written
    /// together, so the query count does not grow with the number of lines.
    ///
    /// Every line is strict. Use [`Self::reserve_lines`] to let a line go to
    /// backorder instead of failing.
    pub async fn reserve(
        &self,
        lines: Vec<ReservationLine>,
        reference: ReservationReference,
        ttl_seconds: i64,
    ) -> Result<Vec<InventoryReservation>, AppError> {
        self.reserve_lines(
            lines.into_iter().map(ReservationLinePolicy::from).collect(),
            reference,
            ttl_seconds,
        )
        .await
    }

    /// Atomically reserves stock for every line, honoring each line's policy.
    ///
    /// Strict lines behave exactly like [`Self::reserve`]: a shortage fails the
    /// whole batch. A line that allows backorder is allocated from on-hand
    /// stock in the same deterministic order while stock lasts, and the
    /// quantity no location can cover is recorded on the default location — so
    /// that level's reserved quantity may exceed its stocked quantity until the
    /// goods arrive. Committing such a reservation drives the stocked quantity
    /// negative, which is what keeps units sold on backorder visible.
    ///
    /// A line that may backorder whose variant has no inventory item is skipped
    /// instead of reserved or rejected: there is no level to record the
    /// shortfall on, so the batch neither reserves it nor fails for it. A
    /// strict line for such a variant still conflicts.
    pub async fn reserve_lines(
        &self,
        lines: Vec<ReservationLinePolicy>,
        reference: ReservationReference,
        ttl_seconds: i64,
    ) -> Result<Vec<InventoryReservation>, AppError> {
        if lines.is_empty() {
            return Err(AppError::validation(
                "at least one reservation line is required",
            ));
        }
        validate_ttl(ttl_seconds)?;
        for policy in &lines {
            validate_positive_quantity(policy.line.quantity, "line quantity")?;
        }

        let now = now_timestamp();
        let expires_at = now.saturating_add(ttl_seconds);
        let mut tx = self.db.begin().await?;

        // The transaction holds SQLite's write lock, so the items and levels
        // read here cannot change while the batch is allocated in memory.
        let variants = lines
            .iter()
            .map(|policy| policy.line.variant_id.clone())
            .collect::<Vec<_>>();
        let items = load_items_by_variant_tx(&mut tx, &variants).await?;
        let item_ids = items.values().cloned().collect::<Vec<_>>();
        let variants_by_item = items
            .iter()
            .map(|(variant, item)| (item.clone(), variant.clone()))
            .collect::<HashMap<_, _>>();
        let mut levels = load_levels_by_item_tx(&mut tx, &item_ids).await?;

        let mut reservations = Vec::new();
        let mut allocations: Vec<LevelAllocation> = Vec::new();
        // At most one entry per item: every backorder goes to the same level.
        let mut backorder_allocations: Vec<LevelAllocation> = Vec::new();
        for policy in &lines {
            let line = &policy.line;
            let Some(item_id) = items.get(line.variant_id.as_str()) else {
                // No inventory item means there is no level to record the line
                // against. A line that may backorder is skipped — nothing can
                // be owed on a ledger that does not exist — while a strict line
                // can never be satisfied and fails the batch.
                if policy.allow_backorder {
                    continue;
                }
                return Err(insufficient_stock_for_variant(line.variant_id.as_str()));
            };

            let mut remaining = line.quantity;
            for candidate in levels.get_mut(item_id).into_iter().flatten() {
                if remaining == 0 {
                    break;
                }
                let available = available_quantity_from(
                    candidate.stocked_quantity,
                    candidate.reserved_quantity,
                );
                let take = remaining.min(available);
                if take <= 0 {
                    continue;
                }
                candidate.reserved_quantity += take;
                remaining -= take;
                reservations.push(InventoryReservation {
                    id: ReservationId::new(),
                    inventory_item_id: InventoryItemId::from_string(item_id.clone()),
                    location_id: InventoryLocationId::from_string(candidate.location_id.clone()),
                    quantity: take,
                    status: ReservationStatus::Reserved,
                    line_item_id: line.line_item_id.clone(),
                    checkout_id: Some(reference.checkout_id.clone()),
                    order_id: None,
                    expires_at: Some(expires_at),
                    metadata: Metadata::empty(),
                    created_at: now,
                    updated_at: now,
                });
                allocations.push(LevelAllocation {
                    item_id: item_id.clone(),
                    location_id: candidate.location_id.clone(),
                    quantity: take,
                });
            }

            if remaining > 0 && policy.allow_backorder {
                // The caller explicitly allows this line to be owed. The
                // quantity no location can cover is recorded on the default
                // location; the location and its level are only created when a
                // shortfall actually happens.
                let backorder_location = InventoryLocationId::from_string(DEFAULT_LOCATION_ID);
                ensure_default_location_in_tx(&mut tx, now).await?;
                ensure_level_in_tx(
                    &mut tx,
                    &InventoryItemId::from_string(item_id.clone()),
                    &backorder_location,
                    now,
                )
                .await?;
                // Lines of the same variant later in the batch must see the
                // backorder, exactly as they see the earlier allocations.
                if let Some(candidates) = levels.get_mut(item_id) {
                    match candidates
                        .iter_mut()
                        .find(|candidate| candidate.location_id == DEFAULT_LOCATION_ID)
                    {
                        Some(candidate) => candidate.reserved_quantity += remaining,
                        None => candidates.push(LevelCandidate {
                            location_id: DEFAULT_LOCATION_ID.to_owned(),
                            stocked_quantity: 0,
                            reserved_quantity: remaining,
                        }),
                    }
                }
                reservations.push(InventoryReservation {
                    id: ReservationId::new(),
                    inventory_item_id: InventoryItemId::from_string(item_id.clone()),
                    location_id: backorder_location,
                    quantity: remaining,
                    status: ReservationStatus::Reserved,
                    line_item_id: line.line_item_id.clone(),
                    checkout_id: Some(reference.checkout_id.clone()),
                    order_id: None,
                    expires_at: Some(expires_at),
                    metadata: Metadata::empty(),
                    created_at: now,
                    updated_at: now,
                });
                match backorder_allocations
                    .iter_mut()
                    .find(|allocation| allocation.item_id == *item_id)
                {
                    Some(allocation) => allocation.quantity += remaining,
                    None => backorder_allocations.push(LevelAllocation {
                        item_id: item_id.clone(),
                        location_id: DEFAULT_LOCATION_ID.to_owned(),
                        quantity: remaining,
                    }),
                }
                remaining = 0;
            }

            if remaining > 0 {
                return Err(insufficient_stock_for_variant(line.variant_id.as_str()));
            }
        }

        apply_level_allocations(&mut tx, &allocations, now, &variants_by_item).await?;
        apply_backorder_allocations(&mut tx, &backorder_allocations, now).await?;
        insert_reservations(&mut tx, &reservations, now).await?;

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

    /// Rebinds a checkout's reservations to the order lines they became.
    ///
    /// Reservations are recorded while a cart still owns the lines, so they
    /// point at cart line identifiers; placing the order assigns fresh line
    /// identifiers to those lines. Rewriting `line_item_id` keeps a reservation
    /// traceable to the order line — and to the location — that consumed it.
    ///
    /// The checkout's committed reservations for `order_id` are rewritten too,
    /// so a runner that committed before it could rebind still converges on
    /// replay. Reservations that were released, that belong to another
    /// checkout, or whose line is not part of `bindings` are left alone, which
    /// makes replaying the same mapping a no-op.
    pub async fn rebind_reservations(
        &self,
        checkout_id: &CheckoutId,
        order_id: &OrderId,
        bindings: &[ReservationRebinding],
    ) -> Result<u64, AppError> {
        if bindings.is_empty() {
            return Ok(0);
        }
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let mut rebound = 0_u64;
        // Three bound parameters per binding plus three for the update itself.
        for chunk in bindings.chunks(SQLITE_PARAMETER_LIMIT / 4) {
            let mut builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
                "UPDATE inventory_reservations SET line_item_id = CASE line_item_id",
            );
            for binding in chunk {
                builder
                    .push(" WHEN ")
                    .push_bind(binding.cart_line_id.as_str())
                    .push(" THEN ")
                    .push_bind(binding.order_item_id.as_str());
            }
            builder
                .push(" ELSE line_item_id END, updated_at = ")
                .push_bind(now)
                .push(" WHERE checkout_id = ")
                .push_bind(checkout_id.as_str())
                .push(" AND (status = 'reserved' OR (status = 'committed' AND order_id = ")
                .push_bind(order_id.as_str())
                .push(")) AND line_item_id IN (");
            {
                let mut separated = builder.separated(", ");
                for binding in chunk {
                    separated.push_bind(binding.cart_line_id.as_str());
                }
            }
            builder.push(")");
            let affected = builder
                .build()
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            rebound += affected.rows_affected();
        }
        tx.commit().await.map_err(database_error)?;
        Ok(rebound)
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
            // `reserved_quantity >= ?` is the guard that keeps a committed
            // reservation from dangling. The stocked quantity is deliberately
            // not guarded: a backorder reservation exceeds the stock on hand,
            // and committing it must drive the level negative so the units owed
            // stay visible until they are restocked. For a reservation that was
            // fully covered by stock, the level cannot pass below zero anyway,
            // because every writer keeps the stocked quantity at or above the
            // reserved quantity.
            let affected = sqlx::query(
                "UPDATE inventory_levels \
                 SET stocked_quantity = stocked_quantity - ?, \
                     reserved_quantity = reserved_quantity - ?, updated_at = ? \
                 WHERE inventory_item_id = ? AND location_id = ? \
                   AND reserved_quantity >= ?",
            )
            .bind(row.quantity)
            .bind(row.quantity)
            .bind(now)
            .bind(row.inventory_item_id.as_str())
            .bind(row.location_id.as_str())
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
}
