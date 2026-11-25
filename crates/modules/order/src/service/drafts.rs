//! Draft-order maintenance: destination changes with re-priced tax.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::{append_outbox_event, database_error};
use fastcart_kernel::{
    AppError, DomainEventEnvelope, EventId, OrderId, now_timestamp, validate_non_negative_amount,
};
use serde_json::Value;

use crate::domain::{DraftOrderTaxLineInput, FulfillmentStatus, Order, OrderStatus};

use super::{OrderService, json_text};

impl OrderService {
    /// Rewrites the tax of a draft order for a new destination.
    ///
    /// The caller resolves the per-line tax from the tax service and passes the
    /// new lines with the addresses they were priced for. Everything below
    /// runs in one transaction: the destination, the item tax totals and the
    /// item-scoped tax lines change together, and the order totals are read
    /// back from the rows so they always describe the same amounts.
    ///
    /// Shipping-scoped tax lines and order-level adjustments are preserved
    /// with the shipping snapshot, exactly like an order edit: changing the
    /// destination re-prices the goods, not the shipping that was already
    /// quoted.
    pub async fn recalculate_draft_taxes(
        &self,
        id: &OrderId,
        shipping_address: Option<Value>,
        billing_address: Option<Value>,
        tax_lines: Vec<DraftOrderTaxLineInput>,
    ) -> Result<Order, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let current = sqlx::query_as::<_, (String, String, i64, i64, i64)>(
            "SELECT status, fulfillment_status, shipping_total, tax_total, total
             FROM orders WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("order", id))?;
        let (status, fulfillment_status, shipping_total, before_tax_total, before_total) = current;
        let status = OrderStatus::from_stored(&status)?;
        if status != OrderStatus::Draft {
            return Err(AppError::conflict(format!(
                "only draft orders can be re-taxed, found {}",
                status.as_str()
            )));
        }
        if FulfillmentStatus::from_stored(&fulfillment_status)? != FulfillmentStatus::NotFulfilled {
            return Err(AppError::conflict(
                "a fulfilled draft order cannot be re-taxed",
            ));
        }

        let items = sqlx::query_as::<_, (String, i64, i64)>(
            "SELECT id, subtotal, discount_total FROM order_items WHERE order_id = ?",
        )
        .bind(id.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
        let item_ids: HashSet<&str> = items
            .iter()
            .map(|(item_id, _, _)| item_id.as_str())
            .collect();

        // Tax per line: the item's full tax and the share that is added on top
        // of its net price (exclusive tax). Inclusive tax is already contained
        // in the price, so it must not grow the line total again.
        let mut per_item: HashMap<String, (i64, i64)> = HashMap::new();
        for line in &tax_lines {
            if !item_ids.contains(line.order_item_id.as_str()) {
                return Err(AppError::validation(format!(
                    "tax line references order item {}, which the order does not have",
                    line.order_item_id
                )));
            }
            validate_non_negative_amount(line.amount, "tax line amount")?;
            if line.rate_bps < 0 {
                return Err(AppError::validation("tax rate must not be negative"));
            }
            let entry = per_item
                .entry(line.order_item_id.as_str().to_owned())
                .or_insert((0, 0));
            entry.0 = entry
                .0
                .checked_add(line.amount)
                .ok_or_else(|| AppError::validation("tax amount overflow"))?;
            if !line.is_tax_inclusive {
                entry.1 = entry
                    .1
                    .checked_add(line.amount)
                    .ok_or_else(|| AppError::validation("tax amount overflow"))?;
            }
        }

        for (item_id, subtotal, discount_total) in &items {
            let (tax_total, exclusive_tax) = per_item.get(item_id).copied().unwrap_or((0, 0));
            let line_total = subtotal
                .checked_sub(*discount_total)
                .and_then(|value| value.checked_add(exclusive_tax))
                .ok_or_else(|| AppError::validation("line total overflow"))?;
            sqlx::query(
                "UPDATE order_items SET tax_total = ?, total = ?
                 WHERE id = ? AND order_id = ?",
            )
            .bind(tax_total)
            .bind(line_total)
            .bind(item_id)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        sqlx::query("DELETE FROM order_tax_lines WHERE order_id = ? AND order_item_id IS NOT NULL")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        let now = now_timestamp();
        for line in &tax_lines {
            sqlx::query(
                "INSERT INTO order_tax_lines (
                    id, order_id, order_item_id, shipping_option_id, name, rate_bps,
                    amount, is_tax_inclusive, created_at
                 ) VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?)",
            )
            .bind(EventId::new().as_str())
            .bind(id.as_str())
            .bind(line.order_item_id.as_str())
            .bind(&line.name)
            .bind(line.rate_bps)
            .bind(line.amount)
            .bind(i64::from(line.is_tax_inclusive))
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        let next_tax_total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM order_tax_lines WHERE order_id = ?",
        )
        .bind(id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        let item_total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(total), 0) FROM order_items WHERE order_id = ?",
        )
        .bind(id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        let adjustment_total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM order_adjustments WHERE order_id = ?",
        )
        .bind(id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        let next_total = item_total
            .checked_add(shipping_total)
            .and_then(|value| value.checked_add(adjustment_total))
            .ok_or_else(|| AppError::validation("order total overflow"))?;
        if next_tax_total < 0 || next_total < 0 {
            return Err(AppError::conflict(
                "tax recalculation would make the order totals negative",
            ));
        }

        let shipping_json = shipping_address.as_ref().map(json_text).transpose()?;
        let billing_json = billing_address.as_ref().map(json_text).transpose()?;
        let updated = sqlx::query(
            "UPDATE orders
             SET shipping_address = COALESCE(?, shipping_address),
                 billing_address = COALESCE(?, billing_address),
                 tax_total = ?, total = ?, updated_at = ?
             WHERE id = ? AND status = ?",
        )
        .bind(shipping_json.as_deref())
        .bind(billing_json.as_deref())
        .bind(next_tax_total)
        .bind(next_total)
        .bind(now)
        .bind(id.as_str())
        .bind(OrderStatus::Draft.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict("order status changed concurrently"));
        }

        self.append_audit(
            &mut tx,
            "order.taxes_recalculated",
            "order",
            id.as_str(),
            serde_json::json!({
                "before_tax_total": before_tax_total,
                "after_tax_total": next_tax_total,
                "before_total": before_total,
                "after_total": next_total,
            }),
        )
        .await?;
        let event = DomainEventEnvelope::new(
            "order.taxes_recalculated",
            Some(self.db.store_id().clone()),
            serde_json::json!({
                "order_id": id.as_str(),
                "before_total": before_total,
                "after_total": next_total,
                "tax_total": next_tax_total,
            }),
        );
        append_outbox_event(&mut *tx, &event)
            .await
            .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        self.load_order(id).await
    }
}
