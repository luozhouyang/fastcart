//! Order status transitions and history.

use fastcart_infrastructure::{append_outbox_event, database_error};
use fastcart_kernel::{AppError, DomainEventEnvelope, OrderId, now_timestamp};

use crate::domain::{FulfillmentStatus, Order, OrderStatus, OrderStatusHistory};

use super::reads::HistoryRow;
use super::{OrderService, insert_history};

impl OrderService {
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
