//! Order fulfillment records.

use std::collections::HashMap;

use fastcart_infrastructure::{append_outbox_event, database_error};
use fastcart_kernel::{
    AppError, DomainEventEnvelope, EventId, FulfillmentId, LineItemId, OrderId, VariantId,
    now_timestamp, validate_length,
};

use crate::domain::{FulfillmentInput, FulfillmentStatus, Order, OrderStatus};

use super::{OrderService, insert_history, json_text};

impl OrderService {
    /// Records a fulfillment and updates the order fulfillment status.
    pub async fn fulfill(&self, id: &OrderId, input: FulfillmentInput) -> Result<Order, AppError> {
        let order = self.load_order(id).await?;
        if !matches!(
            order.status,
            OrderStatus::Confirmed | OrderStatus::Processing
        ) {
            return Err(AppError::conflict(format!(
                "order in state {} cannot be fulfilled",
                order.status.as_str()
            )));
        }
        if order.fulfillment_status == FulfillmentStatus::Fulfilled && !input.partial {
            return Err(AppError::conflict(
                "order is already fully fulfilled".to_owned(),
            ));
        }

        if input.partial && input.items.is_empty() {
            return Err(AppError::validation(
                "partial fulfillments require at least one item",
            ));
        }
        let provider_data = if input.provider_data.is_null() {
            serde_json::json!({})
        } else {
            input.provider_data
        };
        if !provider_data.is_object() {
            return Err(AppError::validation(
                "fulfillment provider_data must be a JSON object",
            ));
        }
        let provider_id = input.provider_id.unwrap_or_else(|| "manual".to_owned());
        validate_length(provider_id.trim(), "provider_id", 64)?;

        let now = now_timestamp();
        let shipped_at = input.shipped_at.unwrap_or(now);

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let fulfilled_rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT fi.order_item_id, COALESCE(SUM(fi.quantity), 0)
             FROM order_fulfillment_items fi
             JOIN order_fulfillments f ON f.id = fi.fulfillment_id
             WHERE f.order_id = ?
             GROUP BY fi.order_item_id",
        )
        .bind(id.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
        let fulfilled: HashMap<String, i64> = fulfilled_rows.into_iter().collect();

        let mut remaining: Vec<(LineItemId, Option<VariantId>, i64)> = order
            .items
            .iter()
            .filter(|item| item.requires_shipping)
            .filter_map(|item| {
                let already = fulfilled.get(item.id.as_str()).copied().unwrap_or_default();
                let left = item.quantity.saturating_sub(already);
                (left > 0).then(|| (item.id.clone(), item.variant_id.clone(), left))
            })
            .collect();

        let requested: Vec<(LineItemId, Option<VariantId>, i64)> = if input.items.is_empty() {
            remaining.clone()
        } else {
            let mut selected = Vec::with_capacity(input.items.len());
            let mut seen = std::collections::HashSet::new();
            for requested_item in &input.items {
                if requested_item.quantity <= 0 {
                    return Err(AppError::validation(
                        "fulfillment item quantities must be positive",
                    ));
                }
                if !seen.insert(requested_item.order_item_id.as_str().to_owned()) {
                    return Err(AppError::validation(
                        "fulfillment items must not repeat an order item",
                    ));
                }
                let Some((line_id, variant_id, left)) = remaining
                    .iter()
                    .find(|(line_id, _, _)| line_id == &requested_item.order_item_id)
                else {
                    return Err(AppError::validation(
                        "fulfillment item does not have a remaining shippable quantity",
                    ));
                };
                if requested_item.quantity > *left {
                    return Err(AppError::validation(format!(
                        "cannot fulfill {} units; only {left} remain",
                        requested_item.quantity
                    )));
                }
                selected.push((line_id.clone(), variant_id.clone(), requested_item.quantity));
            }
            selected
        };

        if requested.is_empty() {
            return Err(AppError::conflict(
                "order has no remaining shippable items".to_owned(),
            ));
        }
        for (line_id, _, quantity) in &requested {
            if let Some(entry) = remaining.iter_mut().find(|(id, _, _)| id == line_id) {
                entry.2 -= quantity;
            }
        }
        let fully_fulfilled = remaining.iter().all(|(_, _, left)| *left == 0);
        let next_status = if input.partial || !fully_fulfilled {
            FulfillmentStatus::PartiallyFulfilled
        } else {
            FulfillmentStatus::Fulfilled
        };
        let note = if next_status == FulfillmentStatus::Fulfilled {
            "fulfillment recorded"
        } else {
            "partial fulfillment recorded"
        };
        let next_order_status = if next_status == FulfillmentStatus::Fulfilled {
            OrderStatus::Fulfilled
        } else {
            order.status
        };

        let fulfillment_id = FulfillmentId::new();
        sqlx::query(
            "INSERT INTO order_fulfillments
                (id, order_id, status, provider_id, provider_fulfillment_id, provider_data,
                 tracking_numbers, shipped_at, delivered_at, metadata, created_at, updated_at)
             VALUES (?, ?, 'fulfilled', ?, ?, ?, ?, ?, NULL, '{}', ?, ?)",
        )
        .bind(fulfillment_id.as_str())
        .bind(id.as_str())
        .bind(provider_id.trim())
        .bind(input.provider_fulfillment_id.as_deref())
        .bind(json_text(&provider_data)?)
        .bind(json_text(&input.tracking_numbers)?)
        .bind(shipped_at)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        for (line_id, variant_id, quantity) in &requested {
            sqlx::query(
                "INSERT INTO order_fulfillment_items
                    (id, fulfillment_id, order_item_id, variant_id, quantity, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(EventId::new().as_str())
            .bind(fulfillment_id.as_str())
            .bind(line_id.as_str())
            .bind(variant_id.as_ref().map(VariantId::as_str))
            .bind(quantity)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        let updated = sqlx::query(
            "UPDATE orders SET fulfillment_status = ?, status = ?, updated_at = ?
             WHERE id = ? AND status = ?",
        )
        .bind(next_status.as_str())
        .bind(next_order_status.as_str())
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
            next_order_status,
            Some(order.status),
            Some(note),
            now,
        )
        .await?;
        let event = DomainEventEnvelope::new(
            "order.fulfilled",
            Some(self.db.store_id().clone()),
            serde_json::json!({
                "order_id": id.as_str(),
                "fulfillment_id": fulfillment_id.as_str(),
                "provider_id": provider_id.trim(),
                "fulfillment_status": next_status.as_str(),
                "partial": next_status != FulfillmentStatus::Fulfilled,
            }),
        );
        append_outbox_event(&mut *tx, &event)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "order.fulfilled",
            "order",
            id.as_str(),
            serde_json::json!({
                "fulfillment_status": next_status.as_str(),
                "status": next_order_status.as_str(),
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.load_order(id).await
    }
}
