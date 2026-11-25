//! Exchange application services: create, accept, cancel and refund.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, EventId, ExchangeId, LineItemId, OrderId, Timestamp, VariantId, now_timestamp,
    validate_metadata, validate_quantity,
};

use crate::after_sales::{
    CreateExchangeInput, ExchangeItemDirection, ExchangeItemStatus, ExchangeStatus, OrderExchange,
    OrderExchangeItem, ReplacementUnitPrice,
};
use crate::domain::OrderStatus;
use crate::service::OrderService;

use super::{
    DifferenceSource, append_after_sales_event, fetch_order_item, normalize_text,
    settle_price_difference, sum_active_claims, sum_active_exchange_inbound, sum_active_returns,
};

#[derive(sqlx::FromRow)]
struct ExchangeRow {
    id: String,
    order_id: String,
    status: String,
    reason: Option<String>,
    difference_total: Option<i64>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
    accepted_at: Option<i64>,
    canceled_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct ExchangeItemRow {
    id: String,
    exchange_id: String,
    order_item_id: String,
    direction: String,
    quantity: i64,
    replacement_variant_id: Option<String>,
    reason: Option<String>,
    difference: Option<i64>,
    status: String,
    created_at: i64,
    updated_at: i64,
}

impl ExchangeItemRow {
    fn into_domain(self) -> Result<OrderExchangeItem, AppError> {
        Ok(OrderExchangeItem {
            id: EventId::from_string(self.id),
            exchange_id: ExchangeId::from_string(self.exchange_id),
            order_item_id: LineItemId::from_string(self.order_item_id),
            direction: ExchangeItemDirection::from_stored(&self.direction)?,
            quantity: self.quantity,
            replacement_variant_id: self.replacement_variant_id.map(VariantId::from_string),
            reason: self.reason,
            difference: self.difference,
            status: ExchangeItemStatus::from_stored(&self.status)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}
impl OrderService {
    /// Creates an exchange with inbound and outbound lines in `requested` state.
    pub async fn create_exchange(
        &self,
        order_id: &OrderId,
        input: CreateExchangeInput,
    ) -> Result<OrderExchange, AppError> {
        let order = self.get(order_id).await?;
        if order.status == OrderStatus::Canceled {
            return Err(AppError::conflict(
                "cannot exchange items from a canceled order",
            ));
        }
        if order.is_draft_order() {
            return Err(AppError::conflict(
                "cannot exchange items from a draft order; convert it first",
            ));
        }
        validate_metadata(&input.metadata)?;
        if input.inbound_items.is_empty() {
            return Err(AppError::validation(
                "an exchange must contain at least one inbound item",
            ));
        }
        if input.outbound_items.is_empty() {
            return Err(AppError::validation(
                "an exchange must contain at least one outbound item",
            ));
        }
        let reason = normalize_text(input.reason);

        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let mut seen = HashSet::new();
        let mut inbound_by_line: HashMap<&str, i64> = HashMap::new();
        for item in &input.inbound_items {
            validate_quantity(item.quantity, false)?;
            if !seen.insert(item.order_item_id.as_str().to_owned()) {
                return Err(AppError::validation(
                    "an exchange must not contain duplicate inbound order items",
                ));
            }
            *inbound_by_line
                .entry(item.order_item_id.as_str())
                .or_default() += item.quantity;
            let existing = fetch_order_item(&mut tx, order_id, &item.order_item_id).await?;
            let returned = sum_active_returns(&mut tx, &item.order_item_id).await?;
            let claimed = sum_active_claims(&mut tx, &item.order_item_id).await?;
            let exchanged = sum_active_exchange_inbound(&mut tx, &item.order_item_id).await?;
            let available = existing.quantity - returned - claimed - exchanged;
            if item.quantity > available {
                return Err(AppError::conflict(format!(
                    "cannot exchange {} of item {}: only {} remain available",
                    item.quantity, item.order_item_id, available
                )));
            }
        }
        let mut seen_outbound = HashSet::new();
        for item in &input.outbound_items {
            validate_quantity(item.quantity, false)?;
            if !seen_outbound.insert(item.order_item_id.as_str().to_owned()) {
                return Err(AppError::validation(
                    "an exchange must not contain duplicate outbound order items",
                ));
            }
            if item.replacement_variant_id.is_none() {
                return Err(AppError::validation(
                    "outbound exchange items require a replacement_variant_id",
                ));
            }
            // A replacement stands in for the units that come back, and the
            // difference settlement charges it at the replaced line's price.
            // Shipping more than came back would hand the extra units over at
            // that difference rather than at their own price.
            let inbound = inbound_by_line
                .get(item.order_item_id.as_str())
                .copied()
                .unwrap_or_default();
            if item.quantity > inbound {
                return Err(AppError::conflict(format!(
                    "cannot ship {} replacement units for item {}: the exchange takes back {}",
                    item.quantity, item.order_item_id, inbound
                )));
            }
            fetch_order_item(&mut tx, order_id, &item.order_item_id).await?;
        }

        let exchange_id = ExchangeId::new();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO order_exchanges
                (id, order_id, status, reason, metadata, created_at, updated_at,
                 accepted_at, canceled_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL)",
        )
        .bind(exchange_id.as_str())
        .bind(order_id.as_str())
        .bind(ExchangeStatus::Requested.as_str())
        .bind(reason.as_deref())
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        for item in &input.inbound_items {
            insert_exchange_item(
                &mut tx,
                &exchange_id,
                item,
                ExchangeItemDirection::Inbound,
                now,
            )
            .await?;
        }
        for item in &input.outbound_items {
            insert_exchange_item(
                &mut tx,
                &exchange_id,
                item,
                ExchangeItemDirection::Outbound,
                now,
            )
            .await?;
        }

        self.append_audit(
            &mut tx,
            "order.exchange_created",
            "order_exchange",
            exchange_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str()}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_exchange(order_id, &exchange_id).await
    }

    /// Loads one exchange.
    pub async fn get_exchange(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
    ) -> Result<OrderExchange, AppError> {
        self.load_exchange(order_id, exchange_id).await
    }

    /// Lists exchanges for an order.
    pub async fn list_exchanges(&self, order_id: &OrderId) -> Result<Vec<OrderExchange>, AppError> {
        self.get(order_id).await?;
        let rows = sqlx::query_as::<_, ExchangeRow>(
            "SELECT id, order_id, status, reason, difference_total, metadata, created_at,
                    updated_at, accepted_at, canceled_at
             FROM order_exchanges WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(order_id.as_str())
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let mut exchanges = Vec::with_capacity(rows.len());
        for row in rows {
            exchanges.push(self.exchange_from_row(row).await?);
        }
        Ok(exchanges)
    }

    /// Accepts a requested exchange and settles the price difference of its
    /// outbound (replacement) lines.
    ///
    /// `replacement_prices` carries the caller-resolved unit price of each
    /// replacement, keyed by the historical order line it replaces, because
    /// the order module does not price catalog variants. The difference
    /// `(replacement unit price - current line unit price) * quantity` is
    /// summed over the outbound lines and recorded once as a signed order
    /// adjustment; a line the caller could not price settles nothing.
    /// Accepting an already accepted exchange returns it unchanged, so a
    /// replay never books the difference twice.
    pub async fn accept_exchange(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
        replacement_prices: &[ReplacementUnitPrice],
    ) -> Result<OrderExchange, AppError> {
        let current = self.load_exchange(order_id, exchange_id).await?;
        if current.status == ExchangeStatus::Accepted {
            return Ok(current);
        }
        if current.status != ExchangeStatus::Requested {
            return Err(AppError::conflict(format!(
                "exchange in state {} cannot be accepted",
                current.status.as_str()
            )));
        }
        let replacement_lines = current
            .outbound_items
            .iter()
            .map(|item| (item.order_item_id.clone(), item.quantity))
            .collect::<Vec<_>>();
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE order_exchanges
             SET status = ?, accepted_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(ExchangeStatus::Accepted.as_str())
        .bind(now)
        .bind(now)
        .bind(exchange_id.as_str())
        .bind(order_id.as_str())
        .bind(current.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "exchange status changed concurrently".to_owned(),
            ));
        }
        sqlx::query(
            "UPDATE order_exchange_items SET status = ?, updated_at = ? WHERE exchange_id = ?",
        )
        .bind(ExchangeItemStatus::Accepted.as_str())
        .bind(now)
        .bind(exchange_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        let difference_total = settle_price_difference(
            &mut tx,
            order_id,
            DifferenceSource::Exchange,
            exchange_id.as_str(),
            &replacement_lines,
            replacement_prices,
            now,
        )
        .await?;
        append_after_sales_event(
            &mut tx,
            self.database().store_id(),
            "order.exchange_accepted",
            serde_json::json!({
                "order_id": order_id.as_str(),
                "exchange_id": exchange_id.as_str(),
                "status": ExchangeStatus::Accepted.as_str(),
                "difference_total": difference_total,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_exchange(order_id, exchange_id).await
    }

    /// Cancels a requested exchange.
    pub async fn cancel_exchange(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
    ) -> Result<OrderExchange, AppError> {
        self.cancel_exchange_with_reason(order_id, exchange_id, None)
            .await
    }

    /// Cancels an exchange and records the caller-supplied reason in the audit log.
    pub async fn cancel_exchange_with_reason(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
        reason: Option<&str>,
    ) -> Result<OrderExchange, AppError> {
        let current = self.load_exchange(order_id, exchange_id).await?;
        if current.status == ExchangeStatus::Canceled {
            return Ok(current);
        }
        if current.status != ExchangeStatus::Requested {
            return Err(AppError::conflict(format!(
                "exchange in state {} cannot be canceled",
                current.status.as_str()
            )));
        }
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE order_exchanges
             SET status = ?, canceled_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(ExchangeStatus::Canceled.as_str())
        .bind(now)
        .bind(now)
        .bind(exchange_id.as_str())
        .bind(order_id.as_str())
        .bind(current.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "exchange status changed concurrently".to_owned(),
            ));
        }
        sqlx::query(
            "UPDATE order_exchange_items SET status = ?, updated_at = ? WHERE exchange_id = ?",
        )
        .bind(ExchangeItemStatus::Canceled.as_str())
        .bind(now)
        .bind(exchange_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "order.exchange_canceled",
            "order_exchange",
            exchange_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str(), "reason": reason}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_exchange(order_id, exchange_id).await
    }

    pub(super) async fn load_exchange(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
    ) -> Result<OrderExchange, AppError> {
        let row = sqlx::query_as::<_, ExchangeRow>(
            "SELECT id, order_id, status, reason, difference_total, metadata, created_at,
                    updated_at, accepted_at, canceled_at
             FROM order_exchanges WHERE id = ? AND order_id = ?",
        )
        .bind(exchange_id.as_str())
        .bind(order_id.as_str())
        .fetch_optional(self.database().pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("order exchange", exchange_id))?;
        self.exchange_from_row(row).await
    }

    async fn exchange_from_row(&self, row: ExchangeRow) -> Result<OrderExchange, AppError> {
        let item_rows = sqlx::query_as::<_, ExchangeItemRow>(
            "SELECT id, exchange_id, order_item_id, direction, quantity,
                    replacement_variant_id, reason, difference, status, created_at, updated_at
             FROM order_exchange_items WHERE exchange_id = ? ORDER BY rowid ASC",
        )
        .bind(&row.id)
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let mut inbound_items = Vec::new();
        let mut outbound_items = Vec::new();
        for item in item_rows {
            let item = item.into_domain()?;
            match item.direction {
                ExchangeItemDirection::Inbound => inbound_items.push(item),
                ExchangeItemDirection::Outbound => outbound_items.push(item),
            }
        }
        Ok(OrderExchange {
            id: ExchangeId::from_string(row.id),
            order_id: OrderId::from_string(row.order_id),
            status: ExchangeStatus::from_stored(&row.status)?,
            reason: row.reason,
            difference_total: row.difference_total,
            metadata: fastcart_kernel::Metadata::from_json_str(&row.metadata),
            created_at: row.created_at,
            updated_at: row.updated_at,
            accepted_at: row.accepted_at,
            canceled_at: row.canceled_at,
            inbound_items,
            outbound_items,
        })
    }
}
async fn insert_exchange_item(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    exchange_id: &ExchangeId,
    item: &crate::after_sales::ExchangeItemInput,
    direction: ExchangeItemDirection,
    now: Timestamp,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO order_exchange_items
            (id, exchange_id, order_item_id, direction, quantity,
             replacement_variant_id, reason, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(EventId::new().as_str())
    .bind(exchange_id.as_str())
    .bind(item.order_item_id.as_str())
    .bind(direction.as_str())
    .bind(item.quantity)
    .bind(item.replacement_variant_id.as_ref().map(VariantId::as_str))
    .bind(item.reason.as_deref())
    .bind(ExchangeItemStatus::Requested.as_str())
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}
