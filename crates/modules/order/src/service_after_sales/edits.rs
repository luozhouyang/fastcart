//! Order edit application services and stock/fulfillment helpers.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, EventId, LineItemId, OrderEditId, OrderId, ProductId, VariantId, now_timestamp,
    validate_metadata, validate_quantity,
};
use fastcart_module_inventory::{InventoryService, OrderEditStockLine, order_edit_stock_reference};
use sqlx::SqliteConnection;

use crate::after_sales::{
    CreateOrderEditInput, OrderEdit, OrderEditAction, OrderEditItem, OrderEditItemStatus,
    OrderEditStatus,
};
use crate::domain::{FulfillmentStatus, OrderStatus};
use crate::service::OrderService;

use super::{
    append_after_sales_event, fetch_order_item, normalize_text, sum_active_claims,
    sum_active_exchange_inbound, sum_active_returns,
};

#[derive(sqlx::FromRow)]
struct EditRow {
    id: String,
    order_id: String,
    status: String,
    reason: Option<String>,
    metadata: String,
    before_subtotal: i64,
    after_subtotal: i64,
    before_total: i64,
    after_total: i64,
    difference_total: Option<i64>,
    created_at: i64,
    updated_at: i64,
    confirmed_at: Option<i64>,
    canceled_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct EditItemRow {
    id: String,
    edit_id: String,
    order_item_id: Option<String>,
    action: String,
    product_id: Option<String>,
    variant_id: Option<String>,
    product_title: String,
    variant_title: Option<String>,
    sku: Option<String>,
    unit_price: i64,
    before_quantity: i64,
    after_quantity: i64,
    requires_shipping: i64,
    status: String,
    created_at: i64,
    updated_at: i64,
}

impl EditItemRow {
    fn into_domain(self) -> Result<OrderEditItem, AppError> {
        Ok(OrderEditItem {
            id: EventId::from_string(self.id),
            edit_id: OrderEditId::from_string(self.edit_id),
            order_item_id: self.order_item_id.map(LineItemId::from_string),
            action: OrderEditAction::from_stored(&self.action)?,
            product_id: self.product_id.map(ProductId::from_string),
            variant_id: self.variant_id.map(VariantId::from_string),
            product_title: self.product_title,
            variant_title: self.variant_title,
            sku: self.sku,
            unit_price: self.unit_price,
            before_quantity: self.before_quantity,
            after_quantity: self.after_quantity,
            requires_shipping: self.requires_shipping != 0,
            status: OrderEditItemStatus::from_stored(&self.status)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}
#[derive(sqlx::FromRow)]
struct ExistingLineAmounts {
    unit_price: i64,
    quantity: i64,
    discount_total: i64,
    tax_total: i64,
}

/// The summed amounts of an order's materialized lines.
#[derive(sqlx::FromRow)]
struct LineTotalsRow {
    subtotal: i64,
    discount_total: i64,
    total: i64,
}

/// A validated order edit line before it is persisted.
struct ResolvedEditItem {
    order_item_id: Option<LineItemId>,
    action: OrderEditAction,
    product_id: Option<ProductId>,
    variant_id: Option<VariantId>,
    product_title: String,
    variant_title: Option<String>,
    sku: Option<String>,
    unit_price: i64,
    before_quantity: i64,
    after_quantity: i64,
    requires_shipping: bool,
    delta: i64,
}
impl OrderService {
    /// Prepares an order edit in `draft` state.
    ///
    /// The edit records the order totals before and after the change. Nothing
    /// is applied to the order until [`Self::confirm_order_edit`] runs.
    pub async fn create_order_edit(
        &self,
        order_id: &OrderId,
        input: CreateOrderEditInput,
    ) -> Result<OrderEdit, AppError> {
        let order = self.get(order_id).await?;
        if order.status == OrderStatus::Canceled {
            return Err(AppError::conflict("cannot edit a canceled order"));
        }
        validate_metadata(&input.metadata)?;
        if input.items.is_empty() {
            return Err(AppError::validation(
                "an order edit must contain at least one item",
            ));
        }
        let reason = normalize_text(input.reason);

        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let mut seen = HashSet::new();
        let mut resolved = Vec::with_capacity(input.items.len());
        for item in &input.items {
            let line = resolve_edit_item(&mut tx, order_id, item).await?;
            if let Some(order_item_id) = &line.order_item_id
                && !seen.insert(order_item_id.as_str().to_owned())
            {
                return Err(AppError::validation(
                    "an order edit must not touch the same order item twice",
                ));
            }
            resolved.push(line);
        }

        let delta = checked_sum(resolved.iter().map(|item| item.delta))?;
        let after_subtotal = order
            .subtotal
            .checked_add(delta)
            .ok_or_else(|| AppError::validation("edit subtotal overflow"))?;
        let after_total = order
            .total
            .checked_add(delta)
            .ok_or_else(|| AppError::validation("edit total overflow"))?;
        if after_subtotal < 0 || after_total < 0 {
            return Err(AppError::validation(
                "an order edit must not make the order totals negative",
            ));
        }

        let edit_id = OrderEditId::new();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO order_edits
                (id, order_id, status, reason, metadata, before_subtotal, after_subtotal,
                 before_total, after_total, created_at, updated_at, confirmed_at, canceled_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL)",
        )
        .bind(edit_id.as_str())
        .bind(order_id.as_str())
        .bind(OrderEditStatus::Draft.as_str())
        .bind(reason.as_deref())
        .bind(input.metadata.to_json_string())
        .bind(order.subtotal)
        .bind(after_subtotal)
        .bind(order.total)
        .bind(after_total)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        for item in &resolved {
            sqlx::query(
                "INSERT INTO order_edit_items
                    (id, edit_id, order_item_id, action, product_id, variant_id,
                     product_title, variant_title, sku, unit_price, before_quantity,
                     after_quantity, requires_shipping, status, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(EventId::new().as_str())
            .bind(edit_id.as_str())
            .bind(item.order_item_id.as_ref().map(LineItemId::as_str))
            .bind(item.action.as_str())
            .bind(item.product_id.as_ref().map(ProductId::as_str))
            .bind(item.variant_id.as_ref().map(VariantId::as_str))
            .bind(&item.product_title)
            .bind(item.variant_title.as_deref())
            .bind(item.sku.as_deref())
            .bind(item.unit_price)
            .bind(item.before_quantity)
            .bind(item.after_quantity)
            .bind(i64::from(item.requires_shipping))
            .bind(OrderEditItemStatus::Draft.as_str())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        self.append_audit(
            &mut tx,
            "order.edit_created",
            "order_edit",
            edit_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str()}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_order_edit(order_id, &edit_id).await
    }

    /// Loads one order edit.
    pub async fn get_order_edit(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
    ) -> Result<OrderEdit, AppError> {
        self.load_order_edit(order_id, edit_id).await
    }

    /// Lists order edits for an order.
    pub async fn list_order_edits(&self, order_id: &OrderId) -> Result<Vec<OrderEdit>, AppError> {
        self.get(order_id).await?;
        let rows = sqlx::query_as::<_, EditRow>(
            "SELECT id, order_id, status, reason, metadata, before_subtotal, after_subtotal,
                    before_total, after_total, difference_total, created_at, updated_at,
                    confirmed_at, canceled_at
             FROM order_edits WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(order_id.as_str())
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let mut edits = Vec::with_capacity(rows.len());
        for row in rows {
            edits.push(self.edit_from_row(row).await?);
        }
        Ok(edits)
    }

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

    /// Cancels a draft order edit without touching the order.
    pub async fn cancel_order_edit(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
    ) -> Result<OrderEdit, AppError> {
        self.cancel_order_edit_with_reason(order_id, edit_id, None)
            .await
    }

    /// Cancels a draft order edit and records the supplied reason in the audit log.
    pub async fn cancel_order_edit_with_reason(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
        reason: Option<&str>,
    ) -> Result<OrderEdit, AppError> {
        let current = self.load_order_edit(order_id, edit_id).await?;
        if current.status == OrderEditStatus::Canceled {
            return Ok(current);
        }
        if current.status != OrderEditStatus::Draft {
            return Err(AppError::conflict(format!(
                "order edit in state {} cannot be canceled",
                current.status.as_str()
            )));
        }
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE order_edits
             SET status = ?, canceled_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(OrderEditStatus::Canceled.as_str())
        .bind(now)
        .bind(now)
        .bind(edit_id.as_str())
        .bind(order_id.as_str())
        .bind(current.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "order edit status changed concurrently".to_owned(),
            ));
        }
        sqlx::query("UPDATE order_edit_items SET status = ?, updated_at = ? WHERE edit_id = ?")
            .bind(OrderEditItemStatus::Canceled.as_str())
            .bind(now)
            .bind(edit_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "order.edit_canceled",
            "order_edit",
            edit_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str(), "reason": reason}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_order_edit(order_id, edit_id).await
    }

    async fn load_order_edit(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
    ) -> Result<OrderEdit, AppError> {
        let row = sqlx::query_as::<_, EditRow>(
            "SELECT id, order_id, status, reason, metadata, before_subtotal, after_subtotal,
                    before_total, after_total, difference_total, created_at, updated_at,
                    confirmed_at, canceled_at
             FROM order_edits WHERE id = ? AND order_id = ?",
        )
        .bind(edit_id.as_str())
        .bind(order_id.as_str())
        .fetch_optional(self.database().pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("order edit", edit_id))?;
        self.edit_from_row(row).await
    }

    async fn edit_from_row(&self, row: EditRow) -> Result<OrderEdit, AppError> {
        let item_rows = sqlx::query_as::<_, EditItemRow>(
            "SELECT id, edit_id, order_item_id, action, product_id, variant_id,
                    product_title, variant_title, sku, unit_price, before_quantity,
                    after_quantity, requires_shipping, status, created_at, updated_at
             FROM order_edit_items WHERE edit_id = ? ORDER BY rowid ASC",
        )
        .bind(&row.id)
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let items = item_rows
            .into_iter()
            .map(EditItemRow::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(OrderEdit {
            id: OrderEditId::from_string(row.id),
            order_id: OrderId::from_string(row.order_id),
            status: OrderEditStatus::from_stored(&row.status)?,
            reason: row.reason,
            metadata: fastcart_kernel::Metadata::from_json_str(&row.metadata),
            before_subtotal: row.before_subtotal,
            after_subtotal: row.after_subtotal,
            before_total: row.before_total,
            after_total: row.after_total,
            difference_total: row.difference_total,
            created_at: row.created_at,
            updated_at: row.updated_at,
            confirmed_at: row.confirmed_at,
            canceled_at: row.canceled_at,
            items,
        })
    }
}
/// The stock movements a confirmed edit applies.
///
/// Each line contributes one movement with a reference derived from the edit
/// and the edit line, so replaying a confirmation is a no-op for the inventory
/// service. Lines that do not take stock, have no variant, or do not change the
/// quantity contribute nothing; a line whose quantity grows consumes the
/// difference and one that shrinks returns it.
fn edit_stock_lines(edit: &OrderEdit) -> Vec<OrderEditStockLine> {
    let mut lines = Vec::with_capacity(edit.items.len());
    for item in &edit.items {
        if !item.requires_shipping {
            continue;
        }
        let Some(variant_id) = item.variant_id.clone() else {
            continue;
        };
        let Some(delta) = item.after_quantity.checked_sub(item.before_quantity) else {
            continue;
        };
        if delta == 0 {
            continue;
        }
        lines.push(OrderEditStockLine::new(
            variant_id,
            delta,
            order_edit_stock_reference(edit.order_id.as_str(), edit.id.as_str(), item.id.as_str()),
        ));
    }
    lines
}

/// Rejects an edit that would take back units already shipped from a line.
///
/// `fulfill` clamps a requested quantity to what is left, so shrinking a line
/// below its shipped quantity does not fail: it silently makes the already
/// written `order_fulfillment_items` describe more units than the line holds.
/// Cancelling an edit is not a way out either — the units are gone — so the
/// merchant has to record a return instead.
///
/// Only the lines the edit shrinks are checked. Added lines have no shipments
/// and unchanged lines keep the quantity their shipments were validated
/// against.
async fn ensure_fulfillment_headroom(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    order_id: &OrderId,
    edit: &OrderEdit,
) -> Result<(), AppError> {
    for item in &edit.items {
        let Some(order_item_id) = item.order_item_id.as_ref() else {
            continue;
        };
        if item.after_quantity >= item.before_quantity {
            continue;
        }
        // Re-read the shipped quantity inside the transaction instead of
        // trusting the number the edit was prepared with: a fulfillment may
        // have been recorded between prepare and confirm.
        let shipped = shipped_quantity(&mut *tx, order_id, order_item_id).await?;
        if item.after_quantity < shipped {
            return Err(AppError::conflict(format!(
                "cannot reduce item {order_item_id} to {}: {shipped} units are already shipped",
                item.after_quantity
            )));
        }
    }
    Ok(())
}

/// Units of one order line already recorded as shipped.
async fn shipped_quantity(
    executor: &mut SqliteConnection,
    order_id: &OrderId,
    order_item_id: &LineItemId,
) -> Result<i64, AppError> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(fi.quantity), 0)
         FROM order_fulfillment_items fi
         JOIN order_fulfillments f ON f.id = fi.fulfillment_id
         WHERE f.order_id = ? AND fi.order_item_id = ?",
    )
    .bind(order_id.as_str())
    .bind(order_item_id.as_str())
    .fetch_one(executor)
    .await
    .map_err(database_error)
}

/// The fulfillment state the order's shippable lines and shipments now
/// describe.
///
/// `fulfill` derives the state from the units it just shipped, but an edit
/// moves the line quantities underneath the existing shipments. Without this
/// derivation an order could stay `fulfilled` while an added line is unshipped,
/// and `fulfill` would then refuse to ship the units the customer paid for.
async fn fulfillment_state_after_edit(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    order_id: &OrderId,
) -> Result<FulfillmentStatus, AppError> {
    let lines = sqlx::query_as::<_, (i64, i64)>(
        "SELECT oi.quantity,
                COALESCE((
                    SELECT SUM(fi.quantity)
                    FROM order_fulfillment_items fi
                    JOIN order_fulfillments f ON f.id = fi.fulfillment_id
                    WHERE f.order_id = oi.order_id AND fi.order_item_id = oi.id
                ), 0)
         FROM order_items oi
         WHERE oi.order_id = ? AND oi.requires_shipping = 1",
    )
    .bind(order_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(database_error)?;
    let mut shippable = false;
    let mut shipped = false;
    let mut complete = true;
    for (quantity, fulfilled) in lines {
        if quantity <= 0 {
            continue;
        }
        shippable = true;
        shipped |= fulfilled > 0;
        complete &= fulfilled >= quantity;
    }
    Ok(if !shippable || !shipped {
        FulfillmentStatus::NotFulfilled
    } else if complete {
        FulfillmentStatus::Fulfilled
    } else {
        FulfillmentStatus::PartiallyFulfilled
    })
}

/// Rejects an edit that would leave a line holding fewer units than the
/// returns, claims and exchanges already committed against it.
///
/// Those records keep referencing the same `order_items` row, so shrinking the
/// line below them would make after-sales documents describe more units than
/// the order contains.
async fn ensure_after_sales_headroom(
    executor: &mut SqliteConnection,
    order_item_id: &LineItemId,
    after_quantity: i64,
) -> Result<(), AppError> {
    let committed = sum_active_returns(executor, order_item_id).await?
        + sum_active_claims(executor, order_item_id).await?
        + sum_active_exchange_inbound(executor, order_item_id).await?;
    if after_quantity < committed {
        return Err(AppError::conflict(format!(
            "cannot reduce item {order_item_id} to {after_quantity}: \
             {committed} units are already committed to after-sales records"
        )));
    }
    Ok(())
}

async fn resolve_edit_item(
    executor: &mut SqliteConnection,
    order_id: &OrderId,
    input: &crate::after_sales::OrderEditItemInput,
) -> Result<ResolvedEditItem, AppError> {
    match input.action {
        OrderEditAction::Add => {
            if input.order_item_id.is_some() {
                return Err(AppError::validation(
                    "added order edit items must not reference an existing order item",
                ));
            }
            let product_title = input
                .product_title
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    AppError::validation("added order edit items require a product_title")
                })?
                .to_owned();
            fastcart_kernel::validate_length(&product_title, "product_title", 255)?;
            validate_quantity(input.after_quantity, false)?;
            let unit_price = input.unit_price.unwrap_or(0);
            fastcart_kernel::validate_non_negative_amount(unit_price, "unit_price")?;
            let delta = unit_price
                .checked_mul(input.after_quantity)
                .ok_or_else(|| AppError::validation("edit amount overflow"))?;
            Ok(ResolvedEditItem {
                order_item_id: None,
                action: OrderEditAction::Add,
                product_id: input.product_id.clone(),
                variant_id: input.variant_id.clone(),
                product_title,
                variant_title: input.variant_title.clone(),
                sku: input.sku.clone(),
                unit_price,
                before_quantity: 0,
                after_quantity: input.after_quantity,
                requires_shipping: input.requires_shipping.unwrap_or(true),
                delta,
            })
        }
        OrderEditAction::Remove | OrderEditAction::Update => {
            let order_item_id = input.order_item_id.as_ref().ok_or_else(|| {
                AppError::validation("remove and update edit items require an order_item_id")
            })?;
            let existing = fetch_order_item(executor, order_id, order_item_id).await?;
            if let Some(before) = input.before_quantity
                && before != existing.quantity
            {
                return Err(AppError::conflict(format!(
                    "item {} has quantity {}, not the provided before_quantity {}",
                    order_item_id, existing.quantity, before
                )));
            }
            if input.action == OrderEditAction::Remove && input.after_quantity != 0 {
                return Err(AppError::validation(
                    "removed order edit items must have after_quantity 0",
                ));
            }
            validate_quantity(input.after_quantity, true)?;
            ensure_after_sales_headroom(executor, order_item_id, input.after_quantity).await?;
            // Fail while the edit is still a draft when the merchant tries to
            // take back units that already shipped; `confirm_order_edit`
            // re-checks under its own transaction because a fulfillment may
            // land between prepare and confirm.
            let shipped = shipped_quantity(executor, order_id, order_item_id).await?;
            if input.after_quantity < shipped {
                return Err(AppError::conflict(format!(
                    "cannot reduce item {order_item_id} to {}: {shipped} units are already shipped",
                    input.after_quantity
                )));
            }
            let delta_quantity = input.after_quantity - existing.quantity;
            let delta = existing
                .unit_price
                .checked_mul(delta_quantity)
                .ok_or_else(|| AppError::validation("edit amount overflow"))?;
            Ok(ResolvedEditItem {
                order_item_id: Some(order_item_id.clone()),
                action: input.action,
                product_id: existing.product_id.map(ProductId::from_string),
                variant_id: existing.variant_id.map(VariantId::from_string),
                product_title: existing.product_title,
                variant_title: existing.variant_title,
                sku: existing.sku,
                unit_price: existing.unit_price,
                before_quantity: existing.quantity,
                after_quantity: input.after_quantity,
                requires_shipping: existing.requires_shipping,
                delta,
            })
        }
    }
}

fn checked_sum(values: impl Iterator<Item = i64>) -> Result<i64, AppError> {
    let mut total = 0_i64;
    for value in values {
        total = total
            .checked_add(value)
            .ok_or_else(|| AppError::validation("amount overflow"))?;
    }
    Ok(total)
}

/// Scales an order-level amount by `retained / original`, flooring toward
/// negative infinity so the result is deterministic for signed adjustments.
///
/// When the base is unchanged or unknown, the amount is returned untouched.
fn scale_amount(amount: i64, retained: i64, original: i64) -> Result<i64, AppError> {
    if original <= 0 || retained >= original {
        return Ok(amount);
    }
    let retained = retained.max(0);
    let numerator = i128::from(amount)
        .checked_mul(i128::from(retained))
        .ok_or_else(|| AppError::validation("allocation overflow"))?;
    let scaled = numerator.div_euclid(i128::from(original));
    i64::try_from(scaled).map_err(|_| AppError::validation("allocation overflow"))
}
