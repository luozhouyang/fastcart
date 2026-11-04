//! Application services for returns, claims, exchanges and order edits.
//!
//! Every mutation validates quantities against the immutable order snapshot and
//! current post-purchase records, then performs an idempotent, status-guarded
//! transition. Historical `order_items` rows are never updated.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, ClaimId, EventId, ExchangeId, FulfillmentId, LineItemId, OrderEditId, OrderId,
    ProductId, RefundId, ReturnId, Timestamp, VariantId, now_timestamp, validate_length,
    validate_metadata, validate_quantity,
};
use sqlx::SqliteConnection;

use crate::after_sales::{
    AfterSalesFulfillment, AfterSalesFulfillmentItem, ClaimItemStatus, ClaimStatus,
    CreateAfterSalesFulfillmentInput, CreateClaimInput, CreateExchangeInput, CreateOrderEditInput,
    CreateReturnInput, ExchangeItemDirection, ExchangeItemStatus, ExchangeStatus, OrderClaim,
    OrderClaimItem, OrderEdit, OrderEditAction, OrderEditItem, OrderEditItemStatus,
    OrderEditStatus, OrderExchange, OrderExchangeItem, OrderReturn, OrderReturnItem,
    ReceiveReturnInput, ReturnItemStatus, ReturnStatus, WriteOffReturnInput,
};
use crate::domain::OrderStatus;
use crate::service::OrderService;

// -------------------------------------------------------------------------
// Persistence rows
// -------------------------------------------------------------------------

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

#[derive(sqlx::FromRow)]
struct ClaimRow {
    id: String,
    order_id: String,
    status: String,
    reason: Option<String>,
    refund_id: Option<String>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
    accepted_at: Option<i64>,
    canceled_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct ClaimItemRow {
    id: String,
    claim_id: String,
    order_item_id: String,
    quantity: i64,
    item_type: String,
    replacement_variant_id: Option<String>,
    reason: Option<String>,
    status: String,
    created_at: i64,
    updated_at: i64,
}

impl ClaimItemRow {
    fn into_domain(self) -> Result<OrderClaimItem, AppError> {
        Ok(OrderClaimItem {
            id: EventId::from_string(self.id),
            claim_id: ClaimId::from_string(self.claim_id),
            order_item_id: LineItemId::from_string(self.order_item_id),
            quantity: self.quantity,
            item_type: crate::after_sales::ClaimItemType::from_stored(&self.item_type)?,
            replacement_variant_id: self.replacement_variant_id.map(VariantId::from_string),
            reason: self.reason,
            status: ClaimItemStatus::from_stored(&self.status)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ExchangeRow {
    id: String,
    order_id: String,
    status: String,
    reason: Option<String>,
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
            status: ExchangeItemStatus::from_stored(&self.status)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct AfterSalesFulfillmentRow {
    id: String,
    order_id: String,
    claim_id: Option<String>,
    exchange_id: Option<String>,
    status: String,
    provider_id: String,
    provider_fulfillment_id: Option<String>,
    provider_data: String,
    tracking_numbers: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct AfterSalesFulfillmentItemRow {
    id: String,
    fulfillment_id: String,
    variant_id: String,
    quantity: i64,
    created_at: i64,
}

impl AfterSalesFulfillmentItemRow {
    fn into_domain(self) -> AfterSalesFulfillmentItem {
        AfterSalesFulfillmentItem {
            id: EventId::from_string(self.id),
            fulfillment_id: FulfillmentId::from_string(self.fulfillment_id),
            variant_id: VariantId::from_string(self.variant_id),
            quantity: self.quantity,
            created_at: self.created_at,
        }
    }
}

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
            status: OrderEditItemStatus::from_stored(&self.status)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ExistingOrderItem {
    product_id: Option<String>,
    variant_id: Option<String>,
    product_title: String,
    variant_title: Option<String>,
    sku: Option<String>,
    unit_price: i64,
    quantity: i64,
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
    delta: i64,
}

// -------------------------------------------------------------------------
// Returns
// -------------------------------------------------------------------------

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
            let available = existing.quantity - returned;
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
        let updated = sqlx::query(
            "UPDATE order_returns SET refund_id = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND (refund_id IS NULL OR refund_id = ?)",
        )
        .bind(refund_id.as_str())
        .bind(now_timestamp())
        .bind(return_id.as_str())
        .bind(order_id.as_str())
        .bind(refund_id.as_str())
        .execute(self.database().pool())
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "return refund link changed concurrently",
            ));
        }
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
        .execute(self.database().pool())
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "return status changed concurrently".to_owned(),
            ));
        }
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
        for item in &current.items {
            let increment = increments
                .get(item.order_item_id.as_str())
                .copied()
                .unwrap_or(0);
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
        }

        let next = if fully_written_off {
            ReturnStatus::WrittenOff
        } else {
            current.status
        };
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

        tx.commit().await.map_err(database_error)?;
        self.load_return(order_id, return_id).await
    }

    /// Cancels a draft, requested or partially received return.
    pub async fn cancel_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
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

// -------------------------------------------------------------------------
// Claims
// -------------------------------------------------------------------------

impl OrderService {
    /// Creates a claim in `requested` state.
    ///
    /// Claimed quantities can never exceed the purchased quantity minus the
    /// quantities already returned or claimed for the same line.
    pub async fn create_claim(
        &self,
        order_id: &OrderId,
        input: CreateClaimInput,
    ) -> Result<OrderClaim, AppError> {
        let order = self.get(order_id).await?;
        if order.status == OrderStatus::Canceled {
            return Err(AppError::conflict(
                "cannot claim items from a canceled order",
            ));
        }
        validate_metadata(&input.metadata)?;
        if input.items.is_empty() {
            return Err(AppError::validation(
                "a claim must contain at least one item",
            ));
        }
        let reason = normalize_text(input.reason);

        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let mut seen = HashSet::new();
        for item in &input.items {
            validate_quantity(item.quantity, false)?;
            if !seen.insert(item.order_item_id.as_str().to_owned()) {
                return Err(AppError::validation(
                    "a claim must not contain duplicate order items",
                ));
            }
            if item.item_type == crate::after_sales::ClaimItemType::Replace
                && item.replacement_variant_id.is_none()
            {
                return Err(AppError::validation(
                    "replace claims require a replacement_variant_id",
                ));
            }
            let existing = fetch_order_item(&mut tx, order_id, &item.order_item_id).await?;
            let returned = sum_active_returns(&mut tx, &item.order_item_id).await?;
            let claimed = sum_active_claims(&mut tx, &item.order_item_id).await?;
            let available = existing.quantity - returned - claimed;
            if item.quantity > available {
                return Err(AppError::conflict(format!(
                    "cannot claim {} of item {}: only {} remain available",
                    item.quantity, item.order_item_id, available
                )));
            }
        }

        let claim_id = ClaimId::new();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO order_claims
                (id, order_id, status, reason, metadata, created_at, updated_at,
                 accepted_at, canceled_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL)",
        )
        .bind(claim_id.as_str())
        .bind(order_id.as_str())
        .bind(ClaimStatus::Requested.as_str())
        .bind(reason.as_deref())
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        for item in &input.items {
            sqlx::query(
                "INSERT INTO order_claim_items
                    (id, claim_id, order_item_id, quantity, item_type,
                     replacement_variant_id, reason, status, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(EventId::new().as_str())
            .bind(claim_id.as_str())
            .bind(item.order_item_id.as_str())
            .bind(item.quantity)
            .bind(item.item_type.as_str())
            .bind(item.replacement_variant_id.as_ref().map(VariantId::as_str))
            .bind(normalize_text(item.reason.clone()).as_deref())
            .bind(ClaimItemStatus::Requested.as_str())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        tx.commit().await.map_err(database_error)?;
        self.load_claim(order_id, &claim_id).await
    }

    /// Loads one claim.
    pub async fn get_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
    ) -> Result<OrderClaim, AppError> {
        self.load_claim(order_id, claim_id).await
    }

    /// Links an order refund to a claim exactly once.
    ///
    /// Repeating the call with the same refund is a no-op. Linking a claim
    /// that already points at a different refund is rejected.
    pub async fn attach_claim_refund(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
        refund_id: &RefundId,
    ) -> Result<OrderClaim, AppError> {
        let current = self.load_claim(order_id, claim_id).await?;
        if let Some(existing) = current.refund_id.as_ref() {
            if existing == refund_id {
                return Ok(current);
            }
            return Err(AppError::conflict(
                "claim is already linked to a different refund",
            ));
        }
        let updated = sqlx::query(
            "UPDATE order_claims SET refund_id = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND (refund_id IS NULL OR refund_id = ?)",
        )
        .bind(refund_id.as_str())
        .bind(now_timestamp())
        .bind(claim_id.as_str())
        .bind(order_id.as_str())
        .bind(refund_id.as_str())
        .execute(self.database().pool())
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict("claim refund link changed concurrently"));
        }
        self.load_claim(order_id, claim_id).await
    }

    /// Lists claims for an order.
    pub async fn list_claims(&self, order_id: &OrderId) -> Result<Vec<OrderClaim>, AppError> {
        self.get(order_id).await?;
        let rows = sqlx::query_as::<_, ClaimRow>(
            "SELECT id, order_id, status, reason, refund_id, metadata, created_at, updated_at,
                    accepted_at, canceled_at
             FROM order_claims WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(order_id.as_str())
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let mut claims = Vec::with_capacity(rows.len());
        for row in rows {
            claims.push(self.claim_from_row(row).await?);
        }
        Ok(claims)
    }

    /// Accepts a requested claim.
    pub async fn accept_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
    ) -> Result<OrderClaim, AppError> {
        let current = self.load_claim(order_id, claim_id).await?;
        if current.status == ClaimStatus::Accepted {
            return Ok(current);
        }
        if current.status != ClaimStatus::Requested {
            return Err(AppError::conflict(format!(
                "claim in state {} cannot be accepted",
                current.status.as_str()
            )));
        }
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE order_claims
             SET status = ?, accepted_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(ClaimStatus::Accepted.as_str())
        .bind(now)
        .bind(now)
        .bind(claim_id.as_str())
        .bind(order_id.as_str())
        .bind(current.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "claim status changed concurrently".to_owned(),
            ));
        }
        sqlx::query("UPDATE order_claim_items SET status = ?, updated_at = ? WHERE claim_id = ?")
            .bind(ClaimItemStatus::Accepted.as_str())
            .bind(now)
            .bind(claim_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        self.load_claim(order_id, claim_id).await
    }

    /// Cancels a requested claim.
    pub async fn cancel_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
    ) -> Result<OrderClaim, AppError> {
        let current = self.load_claim(order_id, claim_id).await?;
        if current.status == ClaimStatus::Canceled {
            return Ok(current);
        }
        if current.status != ClaimStatus::Requested {
            return Err(AppError::conflict(format!(
                "claim in state {} cannot be canceled",
                current.status.as_str()
            )));
        }
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE order_claims
             SET status = ?, canceled_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(ClaimStatus::Canceled.as_str())
        .bind(now)
        .bind(now)
        .bind(claim_id.as_str())
        .bind(order_id.as_str())
        .bind(current.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict(
                "claim status changed concurrently".to_owned(),
            ));
        }
        sqlx::query("UPDATE order_claim_items SET status = ?, updated_at = ? WHERE claim_id = ?")
            .bind(ClaimItemStatus::Canceled.as_str())
            .bind(now)
            .bind(claim_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        self.load_claim(order_id, claim_id).await
    }

    async fn load_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
    ) -> Result<OrderClaim, AppError> {
        let row = sqlx::query_as::<_, ClaimRow>(
            "SELECT id, order_id, status, reason, refund_id, metadata, created_at, updated_at,
                    accepted_at, canceled_at
             FROM order_claims WHERE id = ? AND order_id = ?",
        )
        .bind(claim_id.as_str())
        .bind(order_id.as_str())
        .fetch_optional(self.database().pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("order claim", claim_id))?;
        self.claim_from_row(row).await
    }

    async fn claim_from_row(&self, row: ClaimRow) -> Result<OrderClaim, AppError> {
        let item_rows = sqlx::query_as::<_, ClaimItemRow>(
            "SELECT id, claim_id, order_item_id, quantity, item_type,
                    replacement_variant_id, reason, status, created_at, updated_at
             FROM order_claim_items WHERE claim_id = ? ORDER BY rowid ASC",
        )
        .bind(&row.id)
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let items = item_rows
            .into_iter()
            .map(ClaimItemRow::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(OrderClaim {
            id: ClaimId::from_string(row.id),
            order_id: OrderId::from_string(row.order_id),
            status: ClaimStatus::from_stored(&row.status)?,
            reason: row.reason,
            refund_id: row.refund_id.map(RefundId::from_string),
            metadata: fastcart_kernel::Metadata::from_json_str(&row.metadata),
            created_at: row.created_at,
            updated_at: row.updated_at,
            accepted_at: row.accepted_at,
            canceled_at: row.canceled_at,
            items,
        })
    }
}

// -------------------------------------------------------------------------
// Exchanges
// -------------------------------------------------------------------------

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
        for item in &input.inbound_items {
            validate_quantity(item.quantity, false)?;
            if !seen.insert(item.order_item_id.as_str().to_owned()) {
                return Err(AppError::validation(
                    "an exchange must not contain duplicate inbound order items",
                ));
            }
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
            "SELECT id, order_id, status, reason, metadata, created_at, updated_at,
                    accepted_at, canceled_at
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

    /// Accepts a requested exchange.
    pub async fn accept_exchange(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
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
        tx.commit().await.map_err(database_error)?;
        self.load_exchange(order_id, exchange_id).await
    }

    /// Cancels a requested exchange.
    pub async fn cancel_exchange(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
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
        tx.commit().await.map_err(database_error)?;
        self.load_exchange(order_id, exchange_id).await
    }

    async fn load_exchange(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
    ) -> Result<OrderExchange, AppError> {
        let row = sqlx::query_as::<_, ExchangeRow>(
            "SELECT id, order_id, status, reason, metadata, created_at, updated_at,
                    accepted_at, canceled_at
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
                    replacement_variant_id, reason, status, created_at, updated_at
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

// -------------------------------------------------------------------------
// After-sales fulfillments (claim replacements and exchange outbound)
// -------------------------------------------------------------------------

impl OrderService {
    /// Persists a provider-backed replacement fulfillment for a claim or
    /// exchange.
    ///
    /// Exactly one of `claim_id` and `exchange_id` must be set, and the
    /// referenced record must belong to `order_id`. Calling this again for the
    /// same after-sales record returns the existing fulfillment instead of
    /// writing another row.
    pub async fn create_after_sales_fulfillment(
        &self,
        input: CreateAfterSalesFulfillmentInput,
    ) -> Result<AfterSalesFulfillment, AppError> {
        match (&input.claim_id, &input.exchange_id) {
            (Some(_), None) | (None, Some(_)) => {}
            _ => {
                return Err(AppError::validation(
                    "an after-sales fulfillment must reference exactly one claim or exchange",
                ));
            }
        }
        if input.items.is_empty() {
            return Err(AppError::validation(
                "an after-sales fulfillment must contain at least one item",
            ));
        }
        for item in &input.items {
            validate_quantity(item.quantity, false)?;
        }
        validate_metadata(&input.metadata)?;
        let provider_id = input.provider_id.trim();
        if provider_id.is_empty() {
            return Err(AppError::validation(
                "provider_id must not be empty".to_owned(),
            ));
        }
        validate_length(provider_id, "provider_id", 64)?;
        let provider_data = if input.provider_data.is_null() {
            serde_json::json!({})
        } else {
            input.provider_data
        };
        if !provider_data.is_object() {
            return Err(AppError::validation(
                "after-sales fulfillment provider_data must be a JSON object",
            ));
        }

        self.get(&input.order_id).await?;
        if let Some(claim_id) = input.claim_id.as_ref() {
            self.load_claim(&input.order_id, claim_id).await?;
            if let Some(existing) = self.claim_fulfillment(&input.order_id, claim_id).await? {
                return Ok(existing);
            }
        }
        if let Some(exchange_id) = input.exchange_id.as_ref() {
            self.load_exchange(&input.order_id, exchange_id).await?;
            if let Some(existing) = self
                .exchange_fulfillment(&input.order_id, exchange_id)
                .await?
            {
                return Ok(existing);
            }
        }

        let id = FulfillmentId::new();
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        sqlx::query(
            "INSERT INTO after_sales_fulfillments
                (id, order_id, claim_id, exchange_id, status, provider_id,
                 provider_fulfillment_id, provider_data, tracking_numbers, metadata,
                 created_at, updated_at)
             VALUES (?, ?, ?, ?, 'fulfilled', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.order_id.as_str())
        .bind(input.claim_id.as_ref().map(ClaimId::as_str))
        .bind(input.exchange_id.as_ref().map(ExchangeId::as_str))
        .bind(provider_id)
        .bind(input.provider_fulfillment_id.as_deref())
        .bind(
            serde_json::to_string(&provider_data)
                .map_err(|error| AppError::validation(format!("invalid provider_data: {error}")))?,
        )
        .bind(
            serde_json::to_string(&input.tracking_numbers).map_err(|error| {
                AppError::validation(format!("invalid tracking numbers: {error}"))
            })?,
        )
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        for item in &input.items {
            sqlx::query(
                "INSERT INTO after_sales_fulfillment_items
                    (id, fulfillment_id, variant_id, quantity, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(EventId::new().as_str())
            .bind(id.as_str())
            .bind(item.variant_id.as_str())
            .bind(item.quantity)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        tx.commit().await.map_err(database_error)?;
        self.load_after_sales_fulfillment(&input.order_id, &id)
            .await
    }

    /// Loads one after-sales fulfillment.
    pub async fn get_after_sales_fulfillment(
        &self,
        order_id: &OrderId,
        fulfillment_id: &FulfillmentId,
    ) -> Result<AfterSalesFulfillment, AppError> {
        self.load_after_sales_fulfillment(order_id, fulfillment_id)
            .await
    }

    /// Returns the single fulfillment recorded for a claim, when present.
    pub async fn claim_fulfillment(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
    ) -> Result<Option<AfterSalesFulfillment>, AppError> {
        self.find_after_sales_fulfillment(order_id, "claim_id", claim_id.as_str())
            .await
    }

    /// Returns the single fulfillment recorded for an exchange, when present.
    pub async fn exchange_fulfillment(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
    ) -> Result<Option<AfterSalesFulfillment>, AppError> {
        self.find_after_sales_fulfillment(order_id, "exchange_id", exchange_id.as_str())
            .await
    }

    /// Lists every after-sales fulfillment for an order, oldest first.
    pub async fn list_after_sales_fulfillments(
        &self,
        order_id: &OrderId,
    ) -> Result<Vec<AfterSalesFulfillment>, AppError> {
        self.get(order_id).await?;
        let rows = sqlx::query_as::<_, AfterSalesFulfillmentRow>(
            "SELECT id, order_id, claim_id, exchange_id, status, provider_id,
                    provider_fulfillment_id, provider_data, tracking_numbers, metadata,
                    created_at, updated_at
             FROM after_sales_fulfillments WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(order_id.as_str())
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let mut fulfillments = Vec::with_capacity(rows.len());
        for row in rows {
            fulfillments.push(self.after_sales_fulfillment_from_row(row).await?);
        }
        Ok(fulfillments)
    }

    async fn find_after_sales_fulfillment(
        &self,
        order_id: &OrderId,
        column: &str,
        value: &str,
    ) -> Result<Option<AfterSalesFulfillment>, AppError> {
        // `column` is a private constant chosen by the caller, never user input.
        let sql = format!(
            "SELECT id, order_id, claim_id, exchange_id, status, provider_id,
                    provider_fulfillment_id, provider_data, tracking_numbers, metadata,
                    created_at, updated_at
             FROM after_sales_fulfillments
             WHERE order_id = ? AND {column} = ?
             ORDER BY created_at ASC, rowid ASC LIMIT 1"
        );
        let row = sqlx::query_as::<_, AfterSalesFulfillmentRow>(&sql)
            .bind(order_id.as_str())
            .bind(value)
            .fetch_optional(self.database().pool())
            .await
            .map_err(database_error)?;
        match row {
            Some(row) => self.after_sales_fulfillment_from_row(row).await.map(Some),
            None => Ok(None),
        }
    }

    async fn load_after_sales_fulfillment(
        &self,
        order_id: &OrderId,
        fulfillment_id: &FulfillmentId,
    ) -> Result<AfterSalesFulfillment, AppError> {
        let row = sqlx::query_as::<_, AfterSalesFulfillmentRow>(
            "SELECT id, order_id, claim_id, exchange_id, status, provider_id,
                    provider_fulfillment_id, provider_data, tracking_numbers, metadata,
                    created_at, updated_at
             FROM after_sales_fulfillments WHERE id = ? AND order_id = ?",
        )
        .bind(fulfillment_id.as_str())
        .bind(order_id.as_str())
        .fetch_optional(self.database().pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("after-sales fulfillment", fulfillment_id))?;
        self.after_sales_fulfillment_from_row(row).await
    }

    async fn after_sales_fulfillment_from_row(
        &self,
        row: AfterSalesFulfillmentRow,
    ) -> Result<AfterSalesFulfillment, AppError> {
        let item_rows = sqlx::query_as::<_, AfterSalesFulfillmentItemRow>(
            "SELECT id, fulfillment_id, variant_id, quantity, created_at
             FROM after_sales_fulfillment_items WHERE fulfillment_id = ?
             ORDER BY rowid ASC",
        )
        .bind(&row.id)
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let items = item_rows
            .into_iter()
            .map(AfterSalesFulfillmentItemRow::into_domain)
            .collect();
        Ok(AfterSalesFulfillment {
            id: FulfillmentId::from_string(row.id),
            order_id: OrderId::from_string(row.order_id),
            claim_id: row.claim_id.map(ClaimId::from_string),
            exchange_id: row.exchange_id.map(ExchangeId::from_string),
            status: row.status,
            provider_id: row.provider_id,
            provider_fulfillment_id: row.provider_fulfillment_id,
            provider_data: serde_json::from_str(&row.provider_data)
                .unwrap_or(serde_json::Value::Null),
            tracking_numbers: serde_json::from_str(&row.tracking_numbers).unwrap_or_default(),
            metadata: fastcart_kernel::Metadata::from_json_str(&row.metadata),
            created_at: row.created_at,
            updated_at: row.updated_at,
            items,
        })
    }
}

// -------------------------------------------------------------------------
// Order edits
// -------------------------------------------------------------------------

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
                     after_quantity, status, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            .bind(OrderEditItemStatus::Draft.as_str())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

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
                    before_total, after_total, created_at, updated_at, confirmed_at, canceled_at
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

    /// Applies a draft edit and recalculates the order totals.
    ///
    /// Only the `orders` totals and order-level adjustment rows change;
    /// immutable `order_items` snapshots are left untouched.
    ///
    /// Allocation semantics:
    ///
    /// * Removals shrink the base that the original order-level discount, tax
    ///   and adjustments covered. Those amounts are scaled by
    ///   `retained_subtotal / order.subtotal` (floored), so a removal keeps the
    ///   same effective rate.
    /// * Added (and quantity-increasing) lines keep their explicit edit amount
    ///   and do not inherit any existing order-level allocation.
    /// * Shipping is preserved.
    /// * `total` is persisted as
    ///   `subtotal - discount_total + shipping_total + tax_total + adjustments`
    ///   which is the same formula used when the order was created.
    ///
    /// Order-level adjustment rows are scaled too, distributing the flooring
    /// remainder to the last row so their sum always equals the persisted
    /// `discount`, `tax` and adjustment totals.
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
        let order = self.get(order_id).await?;
        if order.status == OrderStatus::Canceled {
            return Err(AppError::conflict("cannot edit a canceled order"));
        }
        let delta_subtotal = edit
            .after_subtotal
            .checked_sub(edit.before_subtotal)
            .ok_or_else(|| AppError::validation("edit subtotal overflow"))?;
        let next_subtotal = order
            .subtotal
            .checked_add(delta_subtotal)
            .ok_or_else(|| AppError::validation("order subtotal overflow"))?;

        // Sum only negative line deltas: additions are explicitly priced in
        // `next_subtotal` and must not dilute the existing allocation rate.
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
        let retained_subtotal = order.subtotal.saturating_add(removed_amount).max(0);
        let adjustment_total = checked_sum(order.adjustments.iter().map(|value| value.amount))?;
        let next_discount = scale_amount(order.discount_total, retained_subtotal, order.subtotal)?;
        let next_tax = scale_amount(order.tax_total, retained_subtotal, order.subtotal)?;
        let next_adjustment_total =
            scale_amount(adjustment_total, retained_subtotal, order.subtotal)?;
        let next_total = next_subtotal
            .checked_sub(next_discount)
            .and_then(|value| value.checked_add(order.shipping_total))
            .and_then(|value| value.checked_add(next_tax))
            .and_then(|value| value.checked_add(next_adjustment_total))
            .ok_or_else(|| AppError::validation("order total overflow"))?;
        if next_subtotal < 0 || next_total < 0 {
            return Err(AppError::conflict(
                "the confirmed edit would make the order totals negative",
            ));
        }

        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let updated_order = sqlx::query(
            "UPDATE orders
             SET subtotal = ?, discount_total = ?, shipping_total = ?, tax_total = ?,
                 total = ?, updated_at = ?
             WHERE id = ? AND status = ?",
        )
        .bind(next_subtotal)
        .bind(next_discount)
        .bind(order.shipping_total)
        .bind(next_tax)
        .bind(next_total)
        .bind(now)
        .bind(order_id.as_str())
        .bind(order.status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated_order.rows_affected() == 0 {
            return Err(AppError::conflict("order status changed concurrently"));
        }

        if retained_subtotal != order.subtotal && !order.adjustments.is_empty() {
            // Give the last adjustment the flooring remainder so the rows add
            // up to the scaled order-level total exactly.
            let last_index = order.adjustments.len() - 1;
            let mut allocated = 0_i64;
            for (index, adjustment) in order.adjustments.iter().enumerate() {
                let amount = if index == last_index {
                    next_adjustment_total
                        .checked_sub(allocated)
                        .ok_or_else(|| AppError::validation("adjustment overflow"))?
                } else {
                    scale_amount(adjustment.amount, retained_subtotal, order.subtotal)?
                };
                allocated = allocated
                    .checked_add(amount)
                    .ok_or_else(|| AppError::validation("adjustment overflow"))?;
                sqlx::query(
                    "UPDATE order_adjustments SET amount = ? WHERE id = ? AND order_id = ?",
                )
                .bind(amount)
                .bind(&adjustment.id)
                .bind(order_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            }
        }

        let updated_edit = sqlx::query(
            "UPDATE order_edits
             SET status = ?, after_subtotal = ?, after_total = ?, confirmed_at = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND status = ?",
        )
        .bind(OrderEditStatus::Confirmed.as_str())
        .bind(next_subtotal)
        .bind(next_total)
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
        tx.commit().await.map_err(database_error)?;
        self.load_order_edit(order_id, edit_id).await
    }

    /// Cancels a draft order edit without touching the order.
    pub async fn cancel_order_edit(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
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
                    before_total, after_total, created_at, updated_at, confirmed_at, canceled_at
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
                    after_quantity, status, created_at, updated_at
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
            created_at: row.created_at,
            updated_at: row.updated_at,
            confirmed_at: row.confirmed_at,
            canceled_at: row.canceled_at,
            items,
        })
    }
}

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

async fn fetch_order_item(
    executor: &mut SqliteConnection,
    order_id: &OrderId,
    order_item_id: &LineItemId,
) -> Result<ExistingOrderItem, AppError> {
    sqlx::query_as::<_, ExistingOrderItem>(
        "SELECT product_id, variant_id, product_title, variant_title, sku, unit_price, quantity
         FROM order_items WHERE id = ? AND order_id = ?",
    )
    .bind(order_item_id.as_str())
    .bind(order_id.as_str())
    .fetch_optional(executor)
    .await
    .map_err(database_error)?
    .ok_or_else(|| AppError::not_found("order item", order_item_id))
}

async fn sum_active_returns(
    executor: &mut SqliteConnection,
    order_item_id: &LineItemId,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(ri.quantity), 0)
         FROM order_return_items ri
         JOIN order_returns r ON r.id = ri.return_id
         WHERE ri.order_item_id = ? AND r.status != ?",
    )
    .bind(order_item_id.as_str())
    .bind(ReturnStatus::Canceled.as_str())
    .fetch_one(executor)
    .await
    .map_err(database_error)
}

async fn sum_active_claims(
    executor: &mut SqliteConnection,
    order_item_id: &LineItemId,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(ci.quantity), 0)
         FROM order_claim_items ci
         JOIN order_claims c ON c.id = ci.claim_id
         WHERE ci.order_item_id = ? AND c.status != ?",
    )
    .bind(order_item_id.as_str())
    .bind(ClaimStatus::Canceled.as_str())
    .fetch_one(executor)
    .await
    .map_err(database_error)
}

async fn sum_active_exchange_inbound(
    executor: &mut SqliteConnection,
    order_item_id: &LineItemId,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(ei.quantity), 0)
         FROM order_exchange_items ei
         JOIN order_exchanges e ON e.id = ei.exchange_id
         WHERE ei.order_item_id = ? AND ei.direction = ? AND e.status != ?",
    )
    .bind(order_item_id.as_str())
    .bind(ExchangeItemDirection::Inbound.as_str())
    .bind(ExchangeStatus::Canceled.as_str())
    .fetch_one(executor)
    .await
    .map_err(database_error)
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

fn normalize_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use fastcart_infrastructure::StoreDatabase;
    use fastcart_kernel::{OrderId, PaymentId, RefundId, StoreId, VariantId};
    use serde_json::json;

    use super::OrderService;
    use crate::after_sales::{
        ClaimItemInput, ClaimItemType, ClaimStatus, CreateClaimInput, CreateExchangeInput,
        CreateOrderEditInput, CreateReturnInput, ExchangeItemInput, ExchangeStatus,
        OrderEditAction, OrderEditItemInput, OrderEditStatus, ReceiveReturnInput,
        ReceiveReturnItemInput, ReturnItemInput, ReturnStatus, WriteOffReturnInput,
    };
    use crate::domain::{CreateOrderInput, CreateOrderItemInput, Order, OrderStatus, RefundInput};

    async fn service() -> OrderService {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        OrderService::new(db)
    }

    async fn order(service: &OrderService, quantity: i64) -> Order {
        let unit_price = 1_000;
        service
            .create_order(CreateOrderInput {
                email: "buyer@example.com".to_owned(),
                currency_code: "USD".to_owned(),
                items: vec![CreateOrderItemInput {
                    product_title: "Widget".to_owned(),
                    unit_price,
                    quantity,
                    ..CreateOrderItemInput::default()
                }],
                subtotal: unit_price * quantity,
                total: unit_price * quantity,
                shipping_address: json!({}),
                billing_address: json!({}),
                ..CreateOrderInput::default()
            })
            .await
            .unwrap()
    }

    fn return_input(order: &Order, quantity: i64) -> CreateReturnInput {
        CreateReturnInput {
            reason: Some("damaged".to_owned()),
            refund_id: None,
            metadata: fastcart_kernel::Metadata::default(),
            items: vec![ReturnItemInput {
                order_item_id: order.items[0].id.clone(),
                quantity,
                reason: Some("broken".to_owned()),
            }],
        }
    }

    #[tokio::test]
    async fn return_quantity_cannot_exceed_purchased() {
        let service = service().await;
        let order = order(&service, 2).await;

        let error = service
            .create_return(&order.id, return_input(&order, 3))
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

        let created = service
            .create_return(&order.id, return_input(&order, 2))
            .await
            .unwrap();
        assert_eq!(created.status, ReturnStatus::Draft);

        let error = service
            .create_return(&order.id, return_input(&order, 1))
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn return_lifecycle_is_idempotent_and_quantity_checked() {
        let service = service().await;
        let order = order(&service, 2).await;
        let created = service
            .create_return(&order.id, return_input(&order, 2))
            .await
            .unwrap();

        let requested = service
            .request_return(&order.id, &created.id)
            .await
            .unwrap();
        assert_eq!(requested.status, ReturnStatus::Requested);
        let again = service
            .request_return(&order.id, &created.id)
            .await
            .unwrap();
        assert_eq!(again.status, ReturnStatus::Requested);

        // Receiving more than requested is rejected.
        let error = service
            .receive_return(
                &order.id,
                &created.id,
                ReceiveReturnInput {
                    items: vec![ReceiveReturnItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 3,
                    }],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

        let partial = service
            .receive_return(
                &order.id,
                &created.id,
                ReceiveReturnInput {
                    items: vec![ReceiveReturnItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 1,
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(partial.status, ReturnStatus::PartiallyReceived);
        assert_eq!(partial.items[0].received_quantity, 1);

        let received = service
            .receive_return(&order.id, &created.id, ReceiveReturnInput::default())
            .await
            .unwrap();
        assert_eq!(received.status, ReturnStatus::Received);
        assert_eq!(received.items[0].received_quantity, 2);

        // Repeating the receive after completion is a no-op.
        let again = service
            .receive_return(&order.id, &created.id, ReceiveReturnInput::default())
            .await
            .unwrap();
        assert_eq!(again.status, ReturnStatus::Received);

        let written_off = service
            .write_off_return(&order.id, &created.id, WriteOffReturnInput::default())
            .await
            .unwrap();
        assert_eq!(written_off.status, ReturnStatus::WrittenOff);
        assert_eq!(written_off.items[0].written_off_quantity, 2);

        let error = service
            .cancel_return(&order.id, &created.id)
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn write_off_cannot_exceed_received() {
        let service = service().await;
        let order = order(&service, 3).await;
        let created = service
            .create_return(&order.id, return_input(&order, 3))
            .await
            .unwrap();
        service
            .request_return(&order.id, &created.id)
            .await
            .unwrap();
        service
            .receive_return(
                &order.id,
                &created.id,
                ReceiveReturnInput {
                    items: vec![ReceiveReturnItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 1,
                    }],
                },
            )
            .await
            .unwrap();

        let error = service
            .write_off_return(
                &order.id,
                &created.id,
                WriteOffReturnInput {
                    items: vec![crate::after_sales::WriteOffReturnItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 2,
                    }],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn claims_respect_returned_quantities_and_transitions() {
        let service = service().await;
        let order = order(&service, 4).await;
        let returned = service
            .create_return(&order.id, return_input(&order, 1))
            .await
            .unwrap();
        service
            .request_return(&order.id, &returned.id)
            .await
            .unwrap();

        let claim = service
            .create_claim(
                &order.id,
                CreateClaimInput {
                    reason: None,
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![ClaimItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 3,
                        item_type: ClaimItemType::Refund,
                        replacement_variant_id: None,
                        reason: None,
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(claim.status, ClaimStatus::Requested);

        let error = service
            .create_claim(
                &order.id,
                CreateClaimInput {
                    reason: None,
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![ClaimItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 1,
                        item_type: ClaimItemType::Refund,
                        replacement_variant_id: None,
                        reason: None,
                    }],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

        let accepted = service.accept_claim(&order.id, &claim.id).await.unwrap();
        assert_eq!(accepted.status, ClaimStatus::Accepted);
        let again = service.accept_claim(&order.id, &claim.id).await.unwrap();
        assert_eq!(again.status, ClaimStatus::Accepted);

        let error = service
            .cancel_claim(&order.id, &claim.id)
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn exchanges_track_inbound_and_outbound_items() {
        let service = service().await;
        let order = order(&service, 2).await;
        let variant = VariantId::new();

        let exchange = service
            .create_exchange(
                &order.id,
                CreateExchangeInput {
                    reason: Some("wrong size".to_owned()),
                    metadata: fastcart_kernel::Metadata::default(),
                    inbound_items: vec![ExchangeItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 1,
                        replacement_variant_id: None,
                        reason: None,
                    }],
                    outbound_items: vec![ExchangeItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 1,
                        replacement_variant_id: Some(variant.clone()),
                        reason: None,
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(exchange.status, ExchangeStatus::Requested);
        assert_eq!(exchange.inbound_items.len(), 1);
        assert_eq!(exchange.outbound_items.len(), 1);
        assert_eq!(
            exchange.outbound_items[0].replacement_variant_id.as_ref(),
            Some(&variant)
        );

        let error = service
            .create_exchange(
                &order.id,
                CreateExchangeInput {
                    reason: None,
                    metadata: fastcart_kernel::Metadata::default(),
                    inbound_items: vec![ExchangeItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 2,
                        replacement_variant_id: None,
                        reason: None,
                    }],
                    outbound_items: vec![ExchangeItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 1,
                        replacement_variant_id: Some(VariantId::new()),
                        reason: None,
                    }],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

        let accepted = service
            .accept_exchange(&order.id, &exchange.id)
            .await
            .unwrap();
        assert_eq!(accepted.status, ExchangeStatus::Accepted);
    }

    #[tokio::test]
    async fn order_edits_recalculate_totals_without_mutating_items() {
        let service = service().await;
        let order = order(&service, 2).await;

        let added = service
            .create_order_edit(
                &order.id,
                CreateOrderEditInput {
                    reason: Some("add gift".to_owned()),
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![OrderEditItemInput {
                        order_item_id: None,
                        action: OrderEditAction::Add,
                        product_id: None,
                        variant_id: None,
                        product_title: Some("Gift card".to_owned()),
                        variant_title: None,
                        sku: Some("GIFT-1".to_owned()),
                        unit_price: Some(500),
                        before_quantity: None,
                        after_quantity: 1,
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(added.status, OrderEditStatus::Draft);
        assert_eq!(added.before_total, 2_000);
        assert_eq!(added.after_total, 2_500);
        assert_eq!(added.items[0].before_quantity, 0);
        assert_eq!(added.items[0].after_quantity, 1);

        let confirmed = service
            .confirm_order_edit(&order.id, &added.id)
            .await
            .unwrap();
        assert_eq!(confirmed.status, OrderEditStatus::Confirmed);
        let reloaded = service.get(&order.id).await.unwrap();
        assert_eq!(reloaded.subtotal, 2_500);
        assert_eq!(reloaded.total, 2_500);
        // Historical line items are untouched.
        assert_eq!(reloaded.items.len(), 1);
        assert_eq!(reloaded.items[0].quantity, 2);
        assert_eq!(reloaded.items[0].unit_price, 1_000);

        let again = service
            .confirm_order_edit(&order.id, &added.id)
            .await
            .unwrap();
        assert_eq!(again.status, OrderEditStatus::Confirmed);

        let error = service
            .cancel_order_edit(&order.id, &added.id)
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn order_edits_validate_before_quantities_and_overflow() {
        let service = service().await;
        let order = order(&service, 2).await;

        let error = service
            .create_order_edit(
                &order.id,
                CreateOrderEditInput {
                    reason: None,
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![OrderEditItemInput {
                        order_item_id: Some(order.items[0].id.clone()),
                        action: OrderEditAction::Remove,
                        product_id: None,
                        variant_id: None,
                        product_title: None,
                        variant_title: None,
                        sku: None,
                        unit_price: None,
                        before_quantity: Some(1),
                        after_quantity: 0,
                    }],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

        let update = service
            .create_order_edit(
                &order.id,
                CreateOrderEditInput {
                    reason: None,
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![OrderEditItemInput {
                        order_item_id: Some(order.items[0].id.clone()),
                        action: OrderEditAction::Update,
                        product_id: None,
                        variant_id: None,
                        product_title: None,
                        variant_title: None,
                        sku: None,
                        unit_price: None,
                        before_quantity: Some(2),
                        after_quantity: 1,
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(update.after_total, 1_000);
        let confirmed = service
            .confirm_order_edit(&order.id, &update.id)
            .await
            .unwrap();
        assert_eq!(confirmed.status, OrderEditStatus::Confirmed);
        assert_eq!(service.get(&order.id).await.unwrap().total, 1_000);
    }

    #[tokio::test]
    async fn confirming_an_edit_keeps_all_order_totals_consistent() {
        let service = service().await;
        let order = service
            .create_order(CreateOrderInput {
                email: "buyer@example.com".to_owned(),
                currency_code: "USD".to_owned(),
                items: vec![CreateOrderItemInput {
                    product_title: "Widget".to_owned(),
                    unit_price: 1_000,
                    quantity: 2,
                    subtotal: Some(2_000),
                    discount_total: Some(200),
                    tax_total: Some(180),
                    total: Some(1_980),
                    requires_shipping: true,
                    ..CreateOrderItemInput::default()
                }],
                subtotal: 2_000,
                discount_total: 200,
                shipping_total: 500,
                tax_total: 180,
                total: 2_480,
                shipping_address: json!({}),
                billing_address: json!({}),
                ..CreateOrderInput::default()
            })
            .await
            .unwrap();
        assert_eq!(order.total, 2_480);

        let edit = service
            .create_order_edit(
                &order.id,
                CreateOrderEditInput {
                    reason: Some("add gift".to_owned()),
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![OrderEditItemInput {
                        order_item_id: None,
                        action: OrderEditAction::Add,
                        product_id: None,
                        variant_id: None,
                        product_title: Some("Gift card".to_owned()),
                        variant_title: None,
                        sku: Some("GIFT-1".to_owned()),
                        unit_price: Some(500),
                        before_quantity: None,
                        after_quantity: 1,
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(edit.after_subtotal, 2_500);
        assert_eq!(edit.after_total, 2_980);

        let confirmed = service
            .confirm_order_edit(&order.id, &edit.id)
            .await
            .unwrap();
        assert_eq!(confirmed.after_subtotal, 2_500);
        assert_eq!(confirmed.after_total, 2_980);

        let reloaded = service.get(&order.id).await.unwrap();
        assert_eq!(reloaded.subtotal, 2_500);
        assert_eq!(reloaded.discount_total, 200);
        assert_eq!(reloaded.shipping_total, 500);
        assert_eq!(reloaded.tax_total, 180);
        assert_eq!(reloaded.total, 2_980);
        assert_eq!(
            reloaded.total,
            reloaded.subtotal - reloaded.discount_total
                + reloaded.shipping_total
                + reloaded.tax_total
        );
        // The historical line snapshot is untouched.
        assert_eq!(reloaded.items.len(), 1);
        assert_eq!(reloaded.items[0].quantity, 2);
        assert_eq!(reloaded.items[0].total, 1_980);
    }

    #[tokio::test]
    async fn idempotent_order_refunds_do_not_write_twice() {
        let service = service().await;
        let order = order(&service, 2).await;
        let input = RefundInput {
            payment_id: PaymentId::new(),
            amount: 500,
            currency: "USD".to_owned(),
            reason: Some("return".to_owned()),
        };

        let (first_order, first_id) = service
            .add_refund_idempotent(&order.id, input.clone(), "after-sales:return-refund:ret_1")
            .await
            .unwrap();
        let (second_order, second_id) = service
            .add_refund_idempotent(&order.id, input, "after-sales:return-refund:ret_1")
            .await
            .unwrap();

        assert_eq!(first_id, second_id);
        assert_eq!(first_order.refunds.len(), 1);
        assert_eq!(second_order.refunds.len(), 1);
        assert_eq!(second_order.refunds[0].id, first_id);
        assert_eq!(second_order.refunds[0].amount, 500);
    }

    #[tokio::test]
    async fn after_sales_refund_links_are_idempotent() {
        let service = service().await;
        let order = order(&service, 2).await;

        let created = service
            .create_return(&order.id, return_input(&order, 1))
            .await
            .unwrap();
        let refund = RefundId::new();
        let linked = service
            .attach_return_refund(&order.id, &created.id, &refund)
            .await
            .unwrap();
        assert_eq!(linked.refund_id.as_ref(), Some(&refund));
        let linked_again = service
            .attach_return_refund(&order.id, &created.id, &refund)
            .await
            .unwrap();
        assert_eq!(linked_again.refund_id.as_ref(), Some(&refund));
        let other = RefundId::new();
        assert!(matches!(
            service
                .attach_return_refund(&order.id, &created.id, &other)
                .await,
            Err(fastcart_kernel::AppError::Conflict(_))
        ));

        let claim = service
            .create_claim(
                &order.id,
                CreateClaimInput {
                    reason: None,
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![ClaimItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 1,
                        item_type: ClaimItemType::Refund,
                        replacement_variant_id: None,
                        reason: None,
                    }],
                },
            )
            .await
            .unwrap();
        let claim_refund = RefundId::new();
        let linked_claim = service
            .attach_claim_refund(&order.id, &claim.id, &claim_refund)
            .await
            .unwrap();
        assert_eq!(linked_claim.refund_id.as_ref(), Some(&claim_refund));
        let linked_claim_again = service
            .attach_claim_refund(&order.id, &claim.id, &claim_refund)
            .await
            .unwrap();
        assert_eq!(linked_claim_again.refund_id.as_ref(), Some(&claim_refund));
    }

    #[tokio::test]
    async fn canceled_orders_reject_after_sales_flows() {
        let service = service().await;
        let order = order(&service, 1).await;
        let canceled = service.cancel(&order.id, None).await.unwrap();
        assert_eq!(canceled.status, OrderStatus::Canceled);

        let error = service
            .create_return(&order.id, return_input(&order, 1))
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn missing_order_or_line_is_not_found() {
        let service = service().await;
        let order = order(&service, 1).await;

        let missing_order = service
            .create_return(
                &OrderId::new(),
                CreateReturnInput {
                    reason: None,
                    refund_id: None,
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![ReturnItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 1,
                        reason: None,
                    }],
                },
            )
            .await
            .unwrap_err();
        assert!(missing_order.is_not_found());

        let missing_line = service
            .create_return(
                &order.id,
                CreateReturnInput {
                    reason: None,
                    refund_id: None,
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![ReturnItemInput {
                        order_item_id: fastcart_kernel::LineItemId::new(),
                        quantity: 1,
                        reason: None,
                    }],
                },
            )
            .await
            .unwrap_err();
        assert!(missing_line.is_not_found());
    }
}

#[cfg(test)]
mod allocation_tests {
    use fastcart_infrastructure::StoreDatabase;
    use fastcart_kernel::StoreId;
    use serde_json::json;

    use super::OrderService;
    use crate::after_sales::{CreateOrderEditInput, OrderEditAction, OrderEditItemInput};
    use crate::domain::{CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput};

    #[tokio::test]
    async fn confirming_a_removal_scales_order_level_allocations_proportionally() {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        let service = OrderService::new(db);
        let order = service
            .create_order(CreateOrderInput {
                email: "buyer@example.com".to_owned(),
                currency_code: "USD".to_owned(),
                items: vec![CreateOrderItemInput {
                    product_title: "Widget".to_owned(),
                    unit_price: 1_000,
                    quantity: 2,
                    subtotal: Some(2_000),
                    discount_total: Some(201),
                    tax_total: Some(179),
                    total: Some(1_978),
                    requires_shipping: true,
                    ..CreateOrderItemInput::default()
                }],
                adjustments: vec![CreateAdjustmentInput {
                    code: Some("handling".to_owned()),
                    amount: -101,
                    ..CreateAdjustmentInput::default()
                }],
                subtotal: 2_000,
                discount_total: 201,
                shipping_total: 500,
                tax_total: 179,
                total: 2_377,
                shipping_address: json!({}),
                billing_address: json!({}),
                ..CreateOrderInput::default()
            })
            .await
            .unwrap();

        let edit = service
            .create_order_edit(
                &order.id,
                CreateOrderEditInput {
                    reason: Some("remove one".to_owned()),
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![OrderEditItemInput {
                        order_item_id: Some(order.items[0].id.clone()),
                        action: OrderEditAction::Update,
                        product_id: None,
                        variant_id: None,
                        product_title: None,
                        variant_title: None,
                        sku: None,
                        unit_price: None,
                        before_quantity: Some(2),
                        after_quantity: 1,
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(edit.after_subtotal, 1_000);
        assert_eq!(edit.after_total, 1_377);

        let confirmed = service
            .confirm_order_edit(&order.id, &edit.id)
            .await
            .unwrap();
        let reloaded = service.get(&order.id).await.unwrap();

        // Retained subtotal is 1000/2000 of the original base.
        assert_eq!(reloaded.subtotal, 1_000);
        assert_eq!(reloaded.discount_total, 100);
        assert_eq!(reloaded.tax_total, 89);
        assert_eq!(reloaded.shipping_total, 500);
        assert_eq!(reloaded.adjustments.len(), 1);
        // floor(-101 * 1000 / 2000) = -51
        assert_eq!(reloaded.adjustments[0].amount, -51);
        assert_eq!(reloaded.total, 1_438);
        assert_eq!(confirmed.after_subtotal, 1_000);
        assert_eq!(confirmed.after_total, 1_438);
        assert_eq!(
            reloaded.total,
            reloaded.subtotal - reloaded.discount_total
                + reloaded.shipping_total
                + reloaded.tax_total
                + reloaded
                    .adjustments
                    .iter()
                    .map(|value| value.amount)
                    .sum::<i64>()
        );
        // Historical line snapshots are never mutated.
        assert_eq!(reloaded.items[0].quantity, 2);
        assert_eq!(reloaded.items[0].subtotal, 2_000);
        assert_eq!(reloaded.items[0].discount_total, 201);
    }

    #[tokio::test]
    async fn additions_keep_explicit_amounts_and_do_not_scale_allocations() {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        let service = OrderService::new(db);
        let order = service
            .create_order(CreateOrderInput {
                email: "buyer@example.com".to_owned(),
                currency_code: "USD".to_owned(),
                items: vec![CreateOrderItemInput {
                    product_title: "Widget".to_owned(),
                    unit_price: 1_000,
                    quantity: 2,
                    subtotal: Some(2_000),
                    discount_total: Some(200),
                    tax_total: Some(180),
                    total: Some(1_980),
                    requires_shipping: true,
                    ..CreateOrderItemInput::default()
                }],
                subtotal: 2_000,
                discount_total: 200,
                shipping_total: 500,
                tax_total: 180,
                total: 2_480,
                shipping_address: json!({}),
                billing_address: json!({}),
                ..CreateOrderInput::default()
            })
            .await
            .unwrap();

        let edit = service
            .create_order_edit(
                &order.id,
                CreateOrderEditInput {
                    reason: None,
                    metadata: fastcart_kernel::Metadata::default(),
                    items: vec![OrderEditItemInput {
                        order_item_id: None,
                        action: OrderEditAction::Add,
                        product_id: None,
                        variant_id: None,
                        product_title: Some("Gift card".to_owned()),
                        variant_title: None,
                        sku: None,
                        unit_price: Some(500),
                        before_quantity: None,
                        after_quantity: 1,
                    }],
                },
            )
            .await
            .unwrap();
        let confirmed = service
            .confirm_order_edit(&order.id, &edit.id)
            .await
            .unwrap();
        let reloaded = service.get(&order.id).await.unwrap();
        assert_eq!(reloaded.subtotal, 2_500);
        assert_eq!(reloaded.discount_total, 200);
        assert_eq!(reloaded.tax_total, 180);
        assert_eq!(reloaded.total, 2_980);
        assert_eq!(confirmed.after_total, 2_980);
    }
}
