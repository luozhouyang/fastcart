//! Order status transitions and history.

use fastcart_infrastructure::{append_outbox_event, database_error};
use fastcart_kernel::{AppError, DomainEventEnvelope, OrderId, VariantId, now_timestamp};
use fastcart_module_inventory::{InventoryService, OrderConversionStockLine};

use crate::domain::{
    DraftConversionSnapshot, DraftOrderTaxLineInput, FulfillmentStatus, Order, OrderStatus,
    OrderStatusHistory,
};

use super::reads::HistoryRow;
use super::{OrderService, insert_history};

/// What conversion priced and reserved before it took the write lock.
struct DraftConversionPlan<'a> {
    /// Lines and destination the tax and stock inputs describe.
    snapshot: &'a DraftConversionSnapshot,
    /// Item-scoped tax lines, or `None` when the draft has no destination and
    /// its existing tax must be left untouched.
    tax_lines: Option<Vec<DraftOrderTaxLineInput>>,
    /// Stock the converted order takes, one line per managed variant.
    stock_lines: Vec<OrderConversionStockLine>,
}

impl OrderService {
    /// Converts an admin-authored draft into a pending order.
    ///
    /// This is the lifecycle-only conversion: it moves the draft into
    /// `pending`, so `confirm`, `start_processing` and `fulfill` apply
    /// afterwards, but it does not re-price tax or take stock. Callers that
    /// place a real order use [`Self::convert_priced_draft_order`], which also
    /// commits the stock and applies the tax the conversion was priced with.
    pub async fn convert_draft_order(&self, id: &OrderId) -> Result<Order, AppError> {
        self.convert_draft_order_inner(id, None).await
    }

    /// Converts a draft whose tax and stock were priced from `snapshot`.
    ///
    /// Conversion cannot hold the database lock while the tax provider is
    /// called, so the inputs are computed outside a transaction and validated
    /// against the live rows here. Everything that changes — the applied tax,
    /// the stock movements, the `pending` status, the history, the audit entry
    /// and the `order.placed` event — commits in one `BEGIN IMMEDIATE`
    /// transaction. A concurrent edit confirmation or destination change
    /// therefore either lands before this transaction and fails the snapshot
    /// check, or waits behind it and applies to an already-placed order. A
    /// draft can never be placed with a line the tax and stock inputs did not
    /// cover.
    ///
    /// The stock is committed at conversion, not merely reserved: a
    /// checkout-placed order commits its reservations at settlement in this
    /// platform, so a converted draft takes its units the same way. Returns the
    /// converted order.
    pub async fn convert_priced_draft_order(
        &self,
        id: &OrderId,
        snapshot: &DraftConversionSnapshot,
        tax_lines: Option<Vec<DraftOrderTaxLineInput>>,
        stock_lines: Vec<OrderConversionStockLine>,
    ) -> Result<Order, AppError> {
        self.convert_draft_order_inner(
            id,
            Some(DraftConversionPlan {
                snapshot,
                tax_lines,
                stock_lines,
            }),
        )
        .await
    }

    async fn convert_draft_order_inner(
        &self,
        id: &OrderId,
        plan: Option<DraftConversionPlan<'_>>,
    ) -> Result<Order, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let current = sqlx::query_as::<_, (String, String, i64, i64, i64, String, String)>(
            "SELECT status, shipping_address, tax_total, total, display_number, email, \
                    currency_code
             FROM orders WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("order", id))?;
        let (
            status,
            shipping_address,
            before_tax_total,
            before_total,
            display_number,
            email,
            currency_code,
        ) = current;
        let status = OrderStatus::from_stored(&status)?;
        if status != OrderStatus::Draft {
            return Err(AppError::conflict(format!(
                "only draft orders can be converted, found {}",
                status.as_str()
            )));
        }

        // The lines are re-read under the write lock. A snapshot read before
        // `BEGIN IMMEDIATE` could have been overtaken by an edit confirmation,
        // and the status guard alone cannot catch an emptied or changed line.
        let items = sqlx::query_as::<_, (String, Option<String>, i64, i64, i64, i64)>(
            "SELECT id, variant_id, quantity, subtotal, discount_total, is_tax_inclusive \
             FROM order_items WHERE order_id = ? ORDER BY rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
        if items.iter().all(|(_, _, quantity, _, _, _)| *quantity <= 0) {
            return Err(AppError::conflict(
                "a draft order must contain at least one unit before it can be converted",
            ));
        }

        if let Some(plan) = plan.as_ref() {
            let stored_address: serde_json::Value =
                serde_json::from_str(&shipping_address).unwrap_or_default();
            if stored_address != plan.snapshot.shipping_address
                || !lines_match_snapshot(&items, plan.snapshot)
            {
                return Err(AppError::conflict(
                    "the draft order changed while it was being converted; retry the conversion",
                ));
            }
            if let Some(tax_lines) = plan.tax_lines.as_ref() {
                self.apply_draft_tax_lines(
                    &mut tx,
                    id,
                    None,
                    None,
                    tax_lines,
                    before_tax_total,
                    before_total,
                    now,
                )
                .await?;
            }
            InventoryService::commit_order_conversion_stock(&mut tx, id, &plan.stock_lines).await?;
        }

        let updated =
            sqlx::query("UPDATE orders SET status = ?, updated_at = ? WHERE id = ? AND status = ?")
                .bind(OrderStatus::Pending.as_str())
                .bind(now)
                .bind(id.as_str())
                .bind(OrderStatus::Draft.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict("order status changed concurrently"));
        }
        let total: i64 = sqlx::query_scalar("SELECT total FROM orders WHERE id = ?")
            .bind(id.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
        insert_history(
            &mut tx,
            id,
            OrderStatus::Pending,
            Some(OrderStatus::Draft),
            Some("draft order converted"),
            now,
        )
        .await?;
        let event = DomainEventEnvelope::new(
            "order.placed",
            Some(self.db.store_id().clone()),
            serde_json::json!({
                "order_id": id.as_str(),
                "display_number": display_number,
                "email": email,
                "currency_code": currency_code,
                "total": total,
                "converted_from_draft": true,
            }),
        );
        append_outbox_event(&mut *tx, &event)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "order.draft_converted",
            "order",
            id.as_str(),
            serde_json::json!({"display_number": display_number}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_order(id).await
    }

    /// Moves a pending order to `confirmed`.
    pub async fn confirm(&self, id: &OrderId) -> Result<Order, AppError> {
        let order = self.load_order(id).await?;
        if order.status != OrderStatus::Pending {
            return Err(AppError::conflict(format!(
                "only pending orders can be confirmed, found {}",
                order.status.as_str()
            )));
        }
        self.apply_status(
            id,
            OrderStatus::Pending,
            OrderStatus::Confirmed,
            Some("order confirmed"),
            Some(("order.confirmed", serde_json::Value::Null)),
        )
        .await?;
        self.load_order(id).await
    }

    /// Moves a confirmed order to `processing`.
    pub async fn start_processing(&self, id: &OrderId) -> Result<Order, AppError> {
        let order = self.load_order(id).await?;
        if order.status != OrderStatus::Confirmed {
            return Err(AppError::conflict(format!(
                "only confirmed orders can start processing, found {}",
                order.status.as_str()
            )));
        }
        self.apply_status(
            id,
            OrderStatus::Confirmed,
            OrderStatus::Processing,
            Some("order processing started"),
            Some(("order.processing_started", serde_json::Value::Null)),
        )
        .await?;
        self.load_order(id).await
    }

    /// Cancels a pending, confirmed or processing order.
    pub async fn cancel(&self, id: &OrderId, reason: Option<&str>) -> Result<Order, AppError> {
        let order = self.load_order(id).await?;
        if !order.status.is_cancelable() {
            return Err(AppError::conflict(format!(
                "order in state {} cannot be canceled",
                order.status.as_str()
            )));
        }
        if order.fulfillment_status != FulfillmentStatus::NotFulfilled {
            return Err(AppError::conflict(
                "orders with fulfilled items cannot be canceled",
            ));
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE orders SET status = ?, canceled_at = ?, updated_at = ?
             WHERE id = ? AND status = ?",
        )
        .bind(OrderStatus::Canceled.as_str())
        .bind(now)
        .bind(now)
        .bind(id.as_str())
        .bind(order.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict("order status changed concurrently"));
        }
        insert_history(
            &mut tx,
            id,
            OrderStatus::Canceled,
            Some(order.status),
            reason.or(Some("order canceled")),
            now,
        )
        .await?;
        let event = DomainEventEnvelope::new(
            "order.canceled",
            Some(self.db.store_id().clone()),
            serde_json::json!({"order_id": id.as_str(), "reason": reason}),
        );
        append_outbox_event(&mut *tx, &event)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "order.canceled",
            "order",
            id.as_str(),
            serde_json::json!({"reason": reason}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.load_order(id).await
    }

    async fn apply_status(
        &self,
        id: &OrderId,
        previous: OrderStatus,
        next: OrderStatus,
        note: Option<&str>,
        audit: Option<(&str, serde_json::Value)>,
    ) -> Result<(), AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let updated =
            sqlx::query("UPDATE orders SET status = ?, updated_at = ? WHERE id = ? AND status = ?")
                .bind(next.as_str())
                .bind(now)
                .bind(id.as_str())
                .bind(previous.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict("order status changed concurrently"));
        }
        insert_history(&mut tx, id, next, Some(previous), note, now).await?;
        if let Some((action, data)) = audit {
            self.append_audit(&mut tx, action, "order", id.as_str(), data)
                .await?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Appends an order status history row without changing the order status.
    pub async fn record_history(
        &self,
        order_id: &OrderId,
        status: OrderStatus,
        previous_status: Option<OrderStatus>,
        note: Option<&str>,
    ) -> Result<OrderStatusHistory, AppError> {
        self.load_order(order_id).await?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let history = insert_history(&mut tx, order_id, status, previous_status, note, now).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(history)
    }

    /// Returns the status history for an order, oldest first.
    pub async fn list_history(
        &self,
        order_id: &OrderId,
    ) -> Result<Vec<OrderStatusHistory>, AppError> {
        self.load_order(order_id).await?;
        let rows = sqlx::query_as::<_, HistoryRow>(
            "SELECT id, order_id, status, previous_status, note, created_at
             FROM order_status_history WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(order_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(HistoryRow::into_domain).collect()
    }
}

/// `(id, variant_id, quantity, subtotal, discount_total, is_tax_inclusive)` of
/// one order line, as read from `order_items` in rowid order.
type ConversionItemRow = (String, Option<String>, i64, i64, i64, i64);

/// Returns true when the live order lines are exactly the ones the conversion
/// priced and reserved.
///
/// Quantity and amounts drive both the tax and the stock inputs, the variant
/// drives the inventory item, and the inclusive flag decides whether tax adds
/// to the line total; any difference means the inputs no longer describe the
/// draft.
fn lines_match_snapshot(items: &[ConversionItemRow], snapshot: &DraftConversionSnapshot) -> bool {
    items.len() == snapshot.items.len()
        && items
            .iter()
            .zip(snapshot.items.iter())
            .all(|(row, expected)| {
                let (id, variant_id, quantity, subtotal, discount_total, is_tax_inclusive) = row;
                id == expected.order_item_id.as_str()
                    && variant_id.as_deref() == expected.variant_id.as_ref().map(VariantId::as_str)
                    && *quantity == expected.quantity
                    && *subtotal == expected.subtotal
                    && *discount_total == expected.discount_total
                    && (*is_tax_inclusive != 0) == expected.is_tax_inclusive
            })
}
