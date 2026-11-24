//! Confirming an edit: stock, fulfillment and difference settlement.

use super::*;

impl OrderService {
    /// Applies a draft edit, rewriting the order lines and recalculating the
    /// order totals from them.
    ///
    /// The edit lands on `order_items`: additions insert a line, quantity
    /// changes rewrite the line's quantity and amounts, and removals keep the
    /// row with a zero quantity (returns, claims and exchanges reference it
    /// with `ON DELETE RESTRICT`). The persisted `orders` amounts are the sums
    /// of the materialized lines, so the order and its lines always describe
    /// the same purchase.
    ///
    /// Allocation semantics:
    ///
    /// * A changed line keeps its effective rate: its discount and tax shrink
    ///   with its quantity.
    /// * Tax follows the base it is levied on: an item-scoped tax line shrinks
    ///   with its line and a shipping-scoped one is preserved with the
    ///   shipping, so the tax lines and `orders.tax_total` stay in step.
    /// * Order-level adjustments shrink with the retained base
    ///   (`retained_subtotal / order.subtotal`, floored) and their flooring
    ///   remainder is distributed to the last row.
    /// * Added lines keep their explicit edit amount and do not inherit any
    ///   existing order-level allocation.
    /// * Shipping is preserved.
    ///
    /// The same transaction moves the stock each line's delta requires (a
    /// positive delta consumes, a negative one returns only units the order
    /// actually took), records the difference between the new total and the
    /// total current at confirmation, and re-derives the fulfillment state so
    /// units added to a fulfilled order can still ship. No money is moved
    /// here; the recorded difference is collected or refunded by an explicit
    /// idempotent call.
    pub async fn confirm_order_edit(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
    ) -> Result<OrderEdit, AppError> {
        let edit = self.load_order_edit(order_id, edit_id).await?;
        if edit.status == OrderEditStatus::Confirmed {
            return Ok(edit);
        }
        if edit.status != OrderEditStatus::Draft {
            return Err(AppError::conflict(format!(
                "order edit in state {} cannot be confirmed",
                edit.status.as_str()
            )));
        }
        // Sum only negative line deltas: additions are explicitly priced by the
        // lines they create and must not dilute the existing allocation rate.
        let mut removed_amount = 0_i64;
        for item in &edit.items {
            let delta_quantity = item
                .after_quantity
                .checked_sub(item.before_quantity)
                .ok_or_else(|| AppError::validation("edit quantity overflow"))?;
            if delta_quantity >= 0 {
                continue;
            }
            let removed = item
                .unit_price
                .checked_mul(delta_quantity)
                .ok_or_else(|| AppError::validation("edit amount overflow"))?;
            removed_amount = removed_amount
                .checked_add(removed)
                .ok_or_else(|| AppError::validation("edit amount overflow"))?;
        }
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;

        // Re-read what the totals are built from inside the write transaction.
        // `self.get` ran before `BEGIN IMMEDIATE`, so another confirmation or
        // fulfillment may have committed in between; this edit has to build on
        // the rows that are current, not on the snapshot it was prepared from.
        let (current_status, current_subtotal, current_shipping_total, current_total) =
            sqlx::query_as::<_, (String, i64, i64, i64)>(
                "SELECT status, subtotal, shipping_total, total FROM orders WHERE id = ?",
            )
            .bind(order_id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("order", order_id))?;
        let current_status = OrderStatus::from_stored(&current_status)?;
        if current_status == OrderStatus::Canceled {
            return Err(AppError::conflict("cannot edit a canceled order"));
        }
        let adjustments = sqlx::query_as::<_, (String, i64)>(
            "SELECT id, amount FROM order_adjustments
             WHERE order_id = ? ORDER BY created_at ASC, rowid ASC",
        )
        .bind(order_id.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
        let retained_subtotal = current_subtotal.saturating_add(removed_amount).max(0);
        let adjustment_total = checked_sum(adjustments.iter().map(|(_, amount)| *amount))?;
        let next_adjustment_total =
            scale_amount(adjustment_total, retained_subtotal, current_subtotal)?;

        // Everything below runs under `BEGIN IMMEDIATE`, so the shipped
        // quantities cannot change between this guard and the line rewrite:
        // a concurrent fulfillment either commits first and is seen here, or
        // waits behind this transaction.
        ensure_fulfillment_headroom(&mut tx, order_id, &edit).await?;

        // Apply the edit to the materialized lines first. A confirmed edit has
        // to leave `orders` and `order_items` describing the same purchase, so
        // the order totals below are read back from these rows.
        //
        // Rows are never deleted: returns, claims and exchanges reference
        // `order_items.id` with ON DELETE RESTRICT (and fulfillments and tax
        // lines cascade), so a removal is expressed as a zero quantity.
        let mut edited_tax_lines: HashMap<String, (i64, i64)> = HashMap::new();
        for item in &edit.items {
            match item.action {
                OrderEditAction::Add => {
                    let line_id = LineItemId::new();
                    let subtotal = item
                        .unit_price
                        .checked_mul(item.after_quantity)
                        .ok_or_else(|| AppError::validation("edit amount overflow"))?;
                    sqlx::query(
                        "INSERT INTO order_items
                            (id, order_id, product_id, variant_id, product_title, variant_title,
                             sku, thumbnail, unit_price, quantity, subtotal, discount_total,
                             tax_total, total, is_tax_inclusive, requires_shipping, metadata)
                         VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, 0, 0, ?, 0, ?, ?)",
                    )
                    .bind(line_id.as_str())
                    .bind(order_id.as_str())
                    .bind(item.product_id.as_ref().map(ProductId::as_str))
                    .bind(item.variant_id.as_ref().map(VariantId::as_str))
                    .bind(&item.product_title)
                    .bind(item.variant_title.as_deref())
                    .bind(item.sku.as_deref())
                    .bind(item.unit_price)
                    .bind(item.after_quantity)
                    .bind(subtotal)
                    .bind(subtotal)
                    .bind(i64::from(item.requires_shipping))
                    .bind(serde_json::json!({"order_edit_id": edit_id.as_str()}).to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(database_error)?;
                    // Point the edit line at the row it materialized so later
                    // edits and audits can address the line.
                    sqlx::query(
                        "UPDATE order_edit_items SET order_item_id = ?, updated_at = ?
                         WHERE id = ? AND edit_id = ?",
                    )
                    .bind(line_id.as_str())
                    .bind(now)
                    .bind(item.id.as_str())
                    .bind(edit_id.as_str())
                    .execute(&mut *tx)
                    .await
                    .map_err(database_error)?;
                }
                OrderEditAction::Update | OrderEditAction::Remove => {
                    let line_id = item.order_item_id.clone().ok_or_else(|| {
                        AppError::validation(
                            "remove and update edit items require an order_item_id",
                        )
                    })?;
                    let current = sqlx::query_as::<_, ExistingLineAmounts>(
                        "SELECT unit_price, quantity, discount_total, tax_total
                         FROM order_items WHERE id = ? AND order_id = ?",
                    )
                    .bind(line_id.as_str())
                    .bind(order_id.as_str())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(database_error)?
                    .ok_or_else(|| AppError::not_found("order item", &line_id))?;
                    if current.quantity != item.before_quantity {
                        return Err(AppError::conflict(format!(
                            "order item {} has quantity {}, not the {} the edit was prepared with",
                            line_id, current.quantity, item.before_quantity
                        )));
                    }
                    ensure_after_sales_headroom(&mut tx, &line_id, item.after_quantity).await?;
                    let subtotal = current
                        .unit_price
                        .checked_mul(item.after_quantity)
                        .ok_or_else(|| AppError::validation("edit amount overflow"))?;
                    // The line keeps the same effective rate: its discount and
                    // tax shrink with its quantity.
                    let discount_total = scale_amount(
                        current.discount_total,
                        item.after_quantity,
                        current.quantity,
                    )?;
                    let tax_total =
                        scale_amount(current.tax_total, item.after_quantity, current.quantity)?;
                    let total = subtotal
                        .checked_sub(discount_total)
                        .and_then(|value| value.checked_add(tax_total))
                        .ok_or_else(|| AppError::validation("order item total overflow"))?;
                    sqlx::query(
                        "UPDATE order_items
                         SET quantity = ?, subtotal = ?, discount_total = ?, tax_total = ?,
                             total = ?
                         WHERE id = ? AND order_id = ?",
                    )
                    .bind(item.after_quantity)
                    .bind(subtotal)
                    .bind(discount_total)
                    .bind(tax_total)
                    .bind(total)
                    .bind(line_id.as_str())
                    .bind(order_id.as_str())
                    .execute(&mut *tx)
                    .await
                    .map_err(database_error)?;
                    edited_tax_lines.insert(
                        line_id.as_str().to_owned(),
                        (item.before_quantity, item.after_quantity),
                    );
                }
            }
        }

        // Order totals are the sums of the materialized lines, the identity
        // `create_order` guarantees. They are read back from SQLite rather than
        // accumulated in memory so untouched lines are included too.
        let line_totals = sqlx::query_as::<_, LineTotalsRow>(
            "SELECT COALESCE(SUM(subtotal), 0) AS subtotal,
                    COALESCE(SUM(discount_total), 0) AS discount_total,
                    COALESCE(SUM(total), 0) AS total
             FROM order_items WHERE order_id = ?",
        )
        .bind(order_id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        let next_subtotal = line_totals.subtotal;
        let next_discount = line_totals.discount_total;
        let next_total = line_totals
            .total
            .checked_add(current_shipping_total)
            .and_then(|value| value.checked_add(next_adjustment_total))
            .ok_or_else(|| AppError::validation("order total overflow"))?;
        if next_subtotal < 0 || next_total < 0 {
            return Err(AppError::conflict(
                "the confirmed edit would make the order totals negative",
            ));
        }

        // Tax follows the base it is levied on: a tax line tied to an edited
        // line shrinks with it, and a shipping-scoped tax line is left alone
        // because shipping is preserved. `orders.tax_total` is then the sum of
        // the tax lines — the identity `create_order` enforces — so per-line
        // flooring can never leave the breakdown and the total apart.
        let tax_lines = sqlx::query_as::<_, (String, i64, Option<String>)>(
            "SELECT id, amount, order_item_id FROM order_tax_lines
             WHERE order_id = ? ORDER BY created_at ASC, rowid ASC",
        )
        .bind(order_id.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
        let mut next_tax = 0_i64;
        for (id, amount, order_item_id) in tax_lines {
            let scaled = match order_item_id
                .as_ref()
                .and_then(|order_item_id| edited_tax_lines.get(order_item_id))
            {
                Some((before, after)) => scale_amount(amount, *after, *before)?,
                None => amount,
            };
            next_tax = next_tax
                .checked_add(scaled)
                .ok_or_else(|| AppError::validation("tax line overflow"))?;
            if scaled != amount {
                sqlx::query("UPDATE order_tax_lines SET amount = ? WHERE id = ?")
                    .bind(scaled)
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(database_error)?;
            }
        }

        // Move the stock the edit changes. The guard above rejects taking back
        // shipped units; this applies the rest of the delta: an added or grown
        // line consumes stock, a shrunk or removed line returns it. It shares
        // this transaction, so a line rewrite and its stock movement commit or
        // roll back together, and each movement carries a reference unique per
        // edit line so a retry converges instead of moving stock twice.
        let stock_lines = edit_stock_lines(&edit);
        if !stock_lines.is_empty() {
            InventoryService::apply_order_edit_stock(&mut tx, order_id, &stock_lines).await?;
        }

        // The edit changes the units below the existing shipments, so the
        // fulfillment state has to be derived again: an order that was
        // `fulfilled` and gained a line is only partially fulfilled now, and
        // `fulfill` refuses to ship anything while it stays `fulfilled`.
        let next_fulfillment = fulfillment_state_after_edit(&mut tx, order_id).await?;
        // `fulfill` only accepts confirmed or processing orders. Moving the
        // order back to `processing` when shipped units no longer cover the
        // lines keeps the added units reachable; the reverse transition
        // records that the remaining units disappeared with the edit.
        let next_order_status = match (current_status, next_fulfillment) {
            (OrderStatus::Fulfilled, FulfillmentStatus::PartiallyFulfilled) => {
                Some(OrderStatus::Processing)
            }
            (OrderStatus::Processing, FulfillmentStatus::Fulfilled) => Some(OrderStatus::Fulfilled),
            _ => None,
        };

        let updated_order = sqlx::query(
            "UPDATE orders
             SET subtotal = ?, discount_total = ?, shipping_total = ?, tax_total = ?,
                 total = ?, fulfillment_status = ?, status = ?, updated_at = ?
             WHERE id = ? AND status = ?",
        )
        .bind(next_subtotal)
        .bind(next_discount)
        .bind(current_shipping_total)
        .bind(next_tax)
        .bind(next_total)
        .bind(next_fulfillment.as_str())
        .bind(next_order_status.unwrap_or(current_status).as_str())
        .bind(now)
        .bind(order_id.as_str())
        .bind(current_status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated_order.rows_affected() == 0 {
            return Err(AppError::conflict("order status changed concurrently"));
        }
        if let Some(next_order_status) = next_order_status {
            crate::service::insert_history(
                &mut tx,
                order_id,
                next_order_status,
                Some(current_status),
                Some("order edit changed the fulfillment state"),
                now,
            )
            .await?;
        }

        if retained_subtotal != current_subtotal && !adjustments.is_empty() {
            // Give the last adjustment the flooring remainder so the rows add
            // up to the scaled order-level total exactly.
            let last_index = adjustments.len() - 1;
            let mut allocated = 0_i64;
            for (index, (id, amount)) in adjustments.iter().enumerate() {
                let amount = if index == last_index {
                    next_adjustment_total
                        .checked_sub(allocated)
                        .ok_or_else(|| AppError::validation("adjustment overflow"))?
                } else {
                    scale_amount(*amount, retained_subtotal, current_subtotal)?
                };
                allocated = allocated
                    .checked_add(amount)
                    .ok_or_else(|| AppError::validation("adjustment overflow"))?;
                sqlx::query(
                    "UPDATE order_adjustments SET amount = ? WHERE id = ? AND order_id = ?",
                )
                .bind(amount)
                .bind(id)
                .bind(order_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            }
        }

        // The difference this confirmation actually leaves on the order,
        // measured against the total that was current when the transaction
        // began (which may differ from `edit.before_total` if another change
        // landed after the edit was prepared). It is recorded once, here, so
        // collecting or refunding it afterwards is the same idempotent call
        // the claim and exchange flows use.
        let difference_total = next_total
            .checked_sub(current_total)
            .ok_or_else(|| AppError::validation("edit difference overflow"))?;
        let updated_edit = sqlx::query(
            "UPDATE order_edits
             SET status = ?, after_subtotal = ?, after_total = ?, difference_total = ?,
                 confirmed_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(OrderEditStatus::Confirmed.as_str())
        .bind(next_subtotal)
        .bind(next_total)
        .bind(difference_total)
        .bind(now)
        .bind(now)
        .bind(edit_id.as_str())
        .bind(order_id.as_str())
        .bind(OrderEditStatus::Draft.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated_edit.rows_affected() == 0 {
            return Err(AppError::conflict(
                "order edit status changed concurrently".to_owned(),
            ));
        }
        sqlx::query("UPDATE order_edit_items SET status = ?, updated_at = ? WHERE edit_id = ?")
            .bind(OrderEditItemStatus::Confirmed.as_str())
            .bind(now)
            .bind(edit_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "order.edit_confirmed",
            "order_edit",
            edit_id.as_str(),
            serde_json::json!({
                "order_id": order_id.as_str(),
                "after_total": next_total,
                "difference_total": difference_total,
            }),
        )
        .await?;
        // A confirmed edit is a customer-visible change to the purchase, so it
        // publishes the same way returns and claim acceptances do; subscribers
        // can react to the new totals and the settled difference.
        append_after_sales_event(
            &mut tx,
            self.database().store_id(),
            "order.edit_confirmed",
            serde_json::json!({
                "order_id": order_id.as_str(),
                "order_edit_id": edit_id.as_str(),
                "before_total": current_total,
                "after_total": next_total,
                "difference_total": difference_total,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_order_edit(order_id, edit_id).await
    }
}
