//! Return application services: create, request, receive, write off.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, EventId, LineItemId, OrderId, RefundId, ReturnId, Timestamp, now_timestamp,
    validate_metadata, validate_quantity,
};
use fastcart_module_inventory::{InventoryService, ReverseWriteOffRequest};

use crate::after_sales::{
    CreateReturnInput, OrderReturn, OrderReturnItem, ReceiveReturnInput, ReturnItemStatus,
    ReturnStatus, WriteOffReturnInput,
};
use crate::domain::OrderStatus;
use crate::service::OrderService;

use super::{
    append_after_sales_event, fetch_order_item, normalize_text, sum_active_claims,
    sum_active_exchange_inbound, sum_active_returns,
};

#[derive(sqlx::FromRow)]
struct ReturnRow {
    id: String,
    order_id: String,
    status: String,
    reason: Option<String>,
    refund_id: Option<String>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
    requested_at: Option<i64>,
    received_at: Option<i64>,
    written_off_at: Option<i64>,
    canceled_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct ReturnItemRow {
    id: String,
    return_id: String,
    order_item_id: String,
    quantity: i64,
    received_quantity: i64,
    written_off_quantity: i64,
    reason: Option<String>,
    status: String,
    created_at: i64,
    updated_at: i64,
}

impl ReturnItemRow {
    fn into_domain(self) -> Result<OrderReturnItem, AppError> {
        Ok(OrderReturnItem {
            id: EventId::from_string(self.id),
            return_id: ReturnId::from_string(self.return_id),
            order_item_id: LineItemId::from_string(self.order_item_id),
            quantity: self.quantity,
            received_quantity: self.received_quantity,
            written_off_quantity: self.written_off_quantity,
            reason: self.reason,
            status: ReturnItemStatus::from_stored(&self.status)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}
impl OrderService {
    /// Creates a return request in `draft` state.
    ///
    /// Requested quantities are validated against the purchased quantity minus
    /// every non-canceled return already recorded for the same line.
    pub async fn create_return(
        &self,
        order_id: &OrderId,
        input: CreateReturnInput,
    ) -> Result<OrderReturn, AppError> {
        let order = self.get(order_id).await?;
        if order.status == OrderStatus::Canceled {
            return Err(AppError::conflict(
                "cannot return items from a canceled order",
            ));
        }
        validate_metadata(&input.metadata)?;
        if input.items.is_empty() {
            return Err(AppError::validation(
                "a return must contain at least one item",
            ));
        }
        let reason = normalize_text(input.reason);

        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let mut seen = HashSet::new();
        for item in &input.items {
            validate_quantity(item.quantity, false)?;
            if !seen.insert(item.order_item_id.as_str().to_owned()) {
                return Err(AppError::validation(
                    "a return must not contain duplicate order items",
                ));
            }
            let existing = fetch_order_item(&mut tx, order_id, &item.order_item_id).await?;
            let returned = sum_active_returns(&mut tx, &item.order_item_id).await?;
            let claimed = sum_active_claims(&mut tx, &item.order_item_id).await?;
            let exchanged = sum_active_exchange_inbound(&mut tx, &item.order_item_id).await?;
            let available = existing.quantity - returned - claimed - exchanged;
            if item.quantity > available {
                return Err(AppError::conflict(format!(
                    "cannot return {} of item {}: only {} remain available",
                    item.quantity, item.order_item_id, available
                )));
            }
        }

        let return_id = ReturnId::new();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO order_returns
                (id, order_id, status, reason, refund_id, metadata, created_at, updated_at,
                 requested_at, received_at, written_off_at, canceled_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL, NULL)",
        )
        .bind(return_id.as_str())
        .bind(order_id.as_str())
        .bind(ReturnStatus::Draft.as_str())
        .bind(reason.as_deref())
        .bind(input.refund_id.as_ref().map(RefundId::as_str))
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        for item in &input.items {
            insert_return_item(
                &mut tx,
                &return_id,
                &item.order_item_id,
                item.quantity,
                normalize_text(item.reason.clone()).as_deref(),
                now,
            )
            .await?;
        }

        self.append_audit(
            &mut tx,
            "order.return_created",
            "order_return",
            return_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str()}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_return(order_id, &return_id).await
    }

    /// Loads one return request.
    pub async fn get_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
    ) -> Result<OrderReturn, AppError> {
        self.load_return(order_id, return_id).await
    }

    /// Links an order refund to a return exactly once.
    ///
    /// Repeating the call with the same refund is a no-op. Linking a return
    /// that already points at a different refund is rejected.
    pub async fn attach_return_refund(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        refund_id: &RefundId,
    ) -> Result<OrderReturn, AppError> {
        let current = self.load_return(order_id, return_id).await?;
        if let Some(existing) = current.refund_id.as_ref() {
            if existing == refund_id {
                return Ok(current);
            }
            return Err(AppError::conflict(
                "return is already linked to a different refund",
            ));
        }
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE order_returns SET refund_id = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND (refund_id IS NULL OR refund_id = ?)",
        )
        .bind(refund_id.as_str())
        .bind(now)
        .bind(return_id.as_str())
        .bind(order_id.as_str())
        .bind(refund_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "return refund link changed concurrently",
            ));
        }
        let refunded_amount: Option<i64> =
            sqlx::query_scalar("SELECT amount FROM order_refunds WHERE id = ?")
                .bind(refund_id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
        let currency_code: String =
            sqlx::query_scalar("SELECT currency_code FROM orders WHERE id = ?")
                .bind(order_id.as_str())
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "order.return_refunded",
            "order_return",
            return_id.as_str(),
            serde_json::json!({
                "order_id": order_id.as_str(),
                "refunded_amount": refunded_amount,
                "currency_code": currency_code,
            }),
        )
        .await?;
        append_after_sales_event(
            &mut tx,
            self.database().store_id(),
            "order.return_refunded",
            serde_json::json!({
                "order_id": order_id.as_str(),
                "return_id": return_id.as_str(),
                "refund_id": refund_id.as_str(),
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_return(order_id, return_id).await
    }

    /// Lists return requests for an order, newest first.
    pub async fn list_returns(&self, order_id: &OrderId) -> Result<Vec<OrderReturn>, AppError> {
        self.get(order_id).await?;
        let rows = sqlx::query_as::<_, ReturnRow>(
            "SELECT id, order_id, status, reason, refund_id, metadata, created_at, updated_at,
                    requested_at, received_at, written_off_at, canceled_at
             FROM order_returns WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(order_id.as_str())
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;

        let mut returns = Vec::with_capacity(rows.len());
        for row in rows {
            returns.push(self.return_from_row(row).await?);
        }
        Ok(returns)
    }

    /// Submits a draft return request.
    pub async fn request_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
    ) -> Result<OrderReturn, AppError> {
        let current = self.load_return(order_id, return_id).await?;
        if current.status == ReturnStatus::Requested {
            return Ok(current);
        }
        if current.status != ReturnStatus::Draft {
            return Err(AppError::conflict(format!(
                "return in state {} cannot be requested",
                current.status.as_str()
            )));
        }
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE order_returns
             SET status = ?, requested_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(ReturnStatus::Requested.as_str())
        .bind(now)
        .bind(now)
        .bind(return_id.as_str())
        .bind(order_id.as_str())
        .bind(current.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "return status changed concurrently".to_owned(),
            ));
        }
        self.append_audit(
            &mut tx,
            "order.return_requested",
            "order_return",
            return_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str()}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_return(order_id, return_id).await
    }

    /// Receives some or all requested return quantities.
    ///
    /// An empty `items` list receives every outstanding quantity. The return is
    /// marked `received` only once every requested line is fully received.
    pub async fn receive_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        input: ReceiveReturnInput,
    ) -> Result<OrderReturn, AppError> {
        let current = self.load_return(order_id, return_id).await?;
        if current.status == ReturnStatus::Received {
            return Ok(current);
        }
        if !matches!(
            current.status,
            ReturnStatus::Requested | ReturnStatus::PartiallyReceived
        ) {
            return Err(AppError::conflict(format!(
                "return in state {} cannot be received",
                current.status.as_str()
            )));
        }
        let increments = validate_receive_input(&current, &input.items)?;
        let now = now_timestamp();

        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let mut all_received = true;
        for item in &current.items {
            let increment = increments
                .get(item.order_item_id.as_str())
                .copied()
                .unwrap_or(0);
            let received = item.received_quantity + increment;
            if received < item.quantity {
                all_received = false;
            }
            let status = if received >= item.quantity {
                ReturnItemStatus::Received
            } else {
                ReturnItemStatus::Requested
            };
            let updated = sqlx::query(
                "UPDATE order_return_items
                 SET received_quantity = ?, status = ?, updated_at = ?
                 WHERE id = ? AND return_id = ? AND received_quantity = ?",
            )
            .bind(received)
            .bind(status.as_str())
            .bind(now)
            .bind(item.id.as_str())
            .bind(return_id.as_str())
            .bind(item.received_quantity)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if updated.rows_affected() == 0 {
                return Err(AppError::conflict(
                    "return item changed concurrently".to_owned(),
                ));
            }
        }

        let next = if all_received {
            ReturnStatus::Received
        } else {
            ReturnStatus::PartiallyReceived
        };
        let received_at = if all_received {
            Some(current.received_at.unwrap_or(now))
        } else {
            current.received_at
        };
        let updated = sqlx::query(
            "UPDATE order_returns
             SET status = ?, received_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(next.as_str())
        .bind(received_at)
        .bind(now)
        .bind(return_id.as_str())
        .bind(order_id.as_str())
        .bind(current.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "return status changed concurrently".to_owned(),
            ));
        }
        append_after_sales_event(
            &mut tx,
            self.database().store_id(),
            "order.return_received",
            serde_json::json!({
                "order_id": order_id.as_str(),
                "return_id": return_id.as_str(),
                "status": next.as_str(),
            }),
        )
        .await?;

        tx.commit().await.map_err(database_error)?;
        self.load_return(order_id, return_id).await
    }

    /// Writes off received items instead of restocking them.
    ///
    /// An empty `items` list writes off every outstanding received quantity.
    /// Written-off quantities can never exceed the received quantity.
    pub async fn write_off_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        input: WriteOffReturnInput,
    ) -> Result<OrderReturn, AppError> {
        let current = self.load_return(order_id, return_id).await?;
        if current.status == ReturnStatus::WrittenOff {
            return Ok(current);
        }
        if !matches!(
            current.status,
            ReturnStatus::Received | ReturnStatus::PartiallyReceived
        ) {
            return Err(AppError::conflict(format!(
                "return in state {} cannot be written off",
                current.status.as_str()
            )));
        }
        let increments = validate_write_off_input(&current, &input.items)?;
        let now = now_timestamp();

        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let mut fully_written_off = true;
        let mut changed = false;
        let mut reversed_units = 0_i64;
        for item in &current.items {
            let increment = increments
                .get(item.order_item_id.as_str())
                .copied()
                .unwrap_or(0);
            if increment > 0 {
                changed = true;
            }
            let written_off = item.written_off_quantity + increment;
            if written_off < item.received_quantity {
                fully_written_off = false;
            }
            let status = if item.received_quantity >= item.quantity && written_off >= item.quantity
            {
                ReturnItemStatus::WrittenOff
            } else if item.received_quantity >= item.quantity {
                ReturnItemStatus::Received
            } else {
                ReturnItemStatus::Requested
            };
            let updated = sqlx::query(
                "UPDATE order_return_items
                 SET written_off_quantity = ?, status = ?, updated_at = ?
                 WHERE id = ? AND return_id = ? AND written_off_quantity = ?",
            )
            .bind(written_off)
            .bind(status.as_str())
            .bind(now)
            .bind(item.id.as_str())
            .bind(return_id.as_str())
            .bind(item.written_off_quantity)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if updated.rows_affected() == 0 {
                return Err(AppError::conflict(
                    "return item changed concurrently".to_owned(),
                ));
            }
            if increment > 0 {
                // Written-off units are received but not resold, so reverse
                // the stock that receiving placed back on the shelf.
                reversed_units += InventoryService::reverse_return_write_off(
                    &mut tx,
                    ReverseWriteOffRequest {
                        return_id: return_id.as_str(),
                        order_item_id: item.order_item_id.as_str(),
                        written_off_quantity: written_off,
                        increment,
                        now,
                    },
                )
                .await?;
            }
        }

        let next = if fully_written_off {
            ReturnStatus::WrittenOff
        } else {
            current.status
        };
        changed |= next != current.status;
        let written_off_at = if next == ReturnStatus::WrittenOff {
            Some(current.written_off_at.unwrap_or(now))
        } else {
            current.written_off_at
        };
        let updated = sqlx::query(
            "UPDATE order_returns
             SET status = ?, written_off_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(next.as_str())
        .bind(written_off_at)
        .bind(now)
        .bind(return_id.as_str())
        .bind(order_id.as_str())
        .bind(current.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "return status changed concurrently".to_owned(),
            ));
        }

        if changed {
            self.append_audit(
                &mut tx,
                "order.return_written_off",
                "order_return",
                return_id.as_str(),
                serde_json::json!({
                    "order_id": order_id.as_str(),
                    "status": next.as_str(),
                    "reversed_units": reversed_units,
                }),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        self.load_return(order_id, return_id).await
    }

    /// Cancels a draft, requested or partially received return.
    pub async fn cancel_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
    ) -> Result<OrderReturn, AppError> {
        self.cancel_return_with_reason(order_id, return_id, None)
            .await
    }

    /// Cancels a return and records the caller-supplied reason in the audit log.
    pub async fn cancel_return_with_reason(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        reason: Option<&str>,
    ) -> Result<OrderReturn, AppError> {
        let current = self.load_return(order_id, return_id).await?;
        if current.status == ReturnStatus::Canceled {
            return Ok(current);
        }
        if !current.status.is_cancelable() {
            return Err(AppError::conflict(format!(
                "return in state {} cannot be canceled",
                current.status.as_str()
            )));
        }
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE order_returns
             SET status = ?, canceled_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(ReturnStatus::Canceled.as_str())
        .bind(now)
        .bind(now)
        .bind(return_id.as_str())
        .bind(order_id.as_str())
        .bind(current.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "return status changed concurrently".to_owned(),
            ));
        }
        sqlx::query("UPDATE order_return_items SET status = ?, updated_at = ? WHERE return_id = ?")
            .bind(ReturnItemStatus::Canceled.as_str())
            .bind(now)
            .bind(return_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "order.return_canceled",
            "order_return",
            return_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str(), "reason": reason}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_return(order_id, return_id).await
    }

    async fn load_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
    ) -> Result<OrderReturn, AppError> {
        let row = sqlx::query_as::<_, ReturnRow>(
            "SELECT id, order_id, status, reason, refund_id, metadata, created_at, updated_at,
                    requested_at, received_at, written_off_at, canceled_at
             FROM order_returns WHERE id = ? AND order_id = ?",
        )
        .bind(return_id.as_str())
        .bind(order_id.as_str())
        .fetch_optional(self.database().pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("order return", return_id))?;
        self.return_from_row(row).await
    }

    async fn return_from_row(&self, row: ReturnRow) -> Result<OrderReturn, AppError> {
        let item_rows = sqlx::query_as::<_, ReturnItemRow>(
            "SELECT id, return_id, order_item_id, quantity, received_quantity,
                    written_off_quantity, reason, status, created_at, updated_at
             FROM order_return_items WHERE return_id = ? ORDER BY rowid ASC",
        )
        .bind(&row.id)
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let items = item_rows
            .into_iter()
            .map(ReturnItemRow::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(OrderReturn {
            id: ReturnId::from_string(row.id),
            order_id: OrderId::from_string(row.order_id),
            status: ReturnStatus::from_stored(&row.status)?,
            reason: row.reason,
            refund_id: row.refund_id.map(RefundId::from_string),
            metadata: fastcart_kernel::Metadata::from_json_str(&row.metadata),
            created_at: row.created_at,
            updated_at: row.updated_at,
            requested_at: row.requested_at,
            received_at: row.received_at,
            written_off_at: row.written_off_at,
            canceled_at: row.canceled_at,
            items,
        })
    }
}
async fn insert_return_item(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    return_id: &ReturnId,
    order_item_id: &LineItemId,
    quantity: i64,
    reason: Option<&str>,
    now: Timestamp,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO order_return_items
            (id, return_id, order_item_id, quantity, received_quantity,
             written_off_quantity, reason, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, 0, 0, ?, ?, ?, ?)",
    )
    .bind(EventId::new().as_str())
    .bind(return_id.as_str())
    .bind(order_item_id.as_str())
    .bind(quantity)
    .bind(reason)
    .bind(ReturnItemStatus::Requested.as_str())
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}
fn validate_receive_input(
    current: &OrderReturn,
    inputs: &[crate::after_sales::ReceiveReturnItemInput],
) -> Result<HashMap<String, i64>, AppError> {
    let mut increments = HashMap::new();
    if inputs.is_empty() {
        for item in &current.items {
            let remaining = item.quantity - item.received_quantity;
            if remaining > 0 {
                increments.insert(item.order_item_id.as_str().to_owned(), remaining);
            }
        }
        return Ok(increments);
    }
    let mut seen = HashSet::new();
    for input in inputs {
        validate_quantity(input.quantity, false)?;
        if !seen.insert(input.order_item_id.as_str().to_owned()) {
            return Err(AppError::validation(
                "receive input must not contain duplicate order items",
            ));
        }
        let item = current
            .items
            .iter()
            .find(|item| item.order_item_id == input.order_item_id)
            .ok_or_else(|| {
                AppError::validation(format!(
                    "order item {} is not part of this return",
                    input.order_item_id
                ))
            })?;
        let remaining = item.quantity - item.received_quantity;
        if input.quantity > remaining {
            return Err(AppError::conflict(format!(
                "cannot receive {} of item {}: only {} remain outstanding",
                input.quantity, input.order_item_id, remaining
            )));
        }
        increments.insert(input.order_item_id.as_str().to_owned(), input.quantity);
    }
    Ok(increments)
}

fn validate_write_off_input(
    current: &OrderReturn,
    inputs: &[crate::after_sales::WriteOffReturnItemInput],
) -> Result<HashMap<String, i64>, AppError> {
    let mut increments = HashMap::new();
    if inputs.is_empty() {
        for item in &current.items {
            let remaining = item.received_quantity - item.written_off_quantity;
            if remaining > 0 {
                increments.insert(item.order_item_id.as_str().to_owned(), remaining);
            }
        }
        return Ok(increments);
    }
    let mut seen = HashSet::new();
    for input in inputs {
        validate_quantity(input.quantity, false)?;
        if !seen.insert(input.order_item_id.as_str().to_owned()) {
            return Err(AppError::validation(
                "write-off input must not contain duplicate order items",
            ));
        }
        let item = current
            .items
            .iter()
            .find(|item| item.order_item_id == input.order_item_id)
            .ok_or_else(|| {
                AppError::validation(format!(
                    "order item {} is not part of this return",
                    input.order_item_id
                ))
            })?;
        let remaining = item.received_quantity - item.written_off_quantity;
        if input.quantity > remaining {
            return Err(AppError::conflict(format!(
                "cannot write off {} of item {}: only {} received units remain",
                input.quantity, input.order_item_id, remaining
            )));
        }
        increments.insert(input.order_item_id.as_str().to_owned(), input.quantity);
    }
    Ok(increments)
}
