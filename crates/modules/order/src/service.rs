//! SQLite-backed order application service.

use std::collections::HashMap;

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CurrencyCode, CustomerId, EventId, FulfillmentId, LineItemId, Metadata, OrderId,
    Pagination, PaymentId, ProductId, RefundId, Timestamp, VariantId, normalize_email,
    now_timestamp, validate_email, validate_length, validate_metadata,
    validate_non_negative_amount, validate_quantity,
};
use serde_json::Value;

use crate::domain::{
    CreateAdjustmentInput, CreateOrderInput, FulfillmentInput, FulfillmentStatus, Order,
    OrderAdjustment, OrderFilter, OrderFulfillment, OrderItem, OrderItemAdjustment, OrderRefund,
    OrderStatus, OrderStatusHistory, PaymentStatus, RefundInput, RefundStatus,
};

/// Scope used for order-creation idempotency keys.
const IDEMPOTENCY_SCOPE: &str = "order.create";

/// Persistence row for `orders`.
#[derive(sqlx::FromRow)]
struct OrderRow {
    id: String,
    display_number: i64,
    customer_id: Option<String>,
    email: String,
    currency_code: String,
    status: String,
    payment_status: String,
    fulfillment_status: String,
    refund_status: String,
    subtotal: i64,
    discount_total: i64,
    shipping_total: i64,
    tax_total: i64,
    total: i64,
    shipping_address: String,
    billing_address: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
    canceled_at: Option<i64>,
}

/// Persistence row for `order_items`.
#[derive(sqlx::FromRow)]
struct OrderItemRow {
    id: String,
    order_id: String,
    product_id: Option<String>,
    variant_id: Option<String>,
    product_title: String,
    variant_title: Option<String>,
    sku: Option<String>,
    thumbnail: Option<String>,
    unit_price: i64,
    quantity: i64,
    subtotal: i64,
    discount_total: i64,
    tax_total: i64,
    total: i64,
    is_tax_inclusive: i64,
    requires_shipping: i64,
    metadata: String,
}

impl OrderItemRow {
    fn into_domain(self, adjustments: Vec<OrderItemAdjustment>) -> OrderItem {
        OrderItem {
            id: LineItemId::from_string(self.id),
            order_id: OrderId::from_string(self.order_id),
            product_id: self.product_id.map(ProductId::from_string),
            variant_id: self.variant_id.map(VariantId::from_string),
            product_title: self.product_title,
            variant_title: self.variant_title,
            sku: self.sku,
            thumbnail: self.thumbnail,
            unit_price: self.unit_price,
            quantity: self.quantity,
            subtotal: self.subtotal,
            discount_total: self.discount_total,
            tax_total: self.tax_total,
            total: self.total,
            is_tax_inclusive: self.is_tax_inclusive != 0,
            requires_shipping: self.requires_shipping != 0,
            metadata: Metadata::from_json_str(&self.metadata),
            adjustments,
        }
    }
}

/// Persistence row for `order_item_adjustments`.
#[derive(sqlx::FromRow)]
struct ItemAdjustmentRow {
    id: String,
    order_item_id: String,
    promotion_id: Option<String>,
    code: Option<String>,
    amount: i64,
    description: Option<String>,
    created_at: i64,
}

impl ItemAdjustmentRow {
    fn into_domain(self) -> OrderItemAdjustment {
        OrderItemAdjustment {
            id: self.id,
            order_item_id: LineItemId::from_string(self.order_item_id),
            promotion_id: self
                .promotion_id
                .map(fastcart_kernel::PromotionId::from_string),
            code: self.code,
            amount: self.amount,
            description: self.description,
            created_at: self.created_at,
        }
    }
}

/// Persistence row for `order_adjustments`.
#[derive(sqlx::FromRow)]
struct AdjustmentRow {
    id: String,
    order_id: String,
    code: String,
    amount: i64,
    description: Option<String>,
    created_at: i64,
}

impl AdjustmentRow {
    fn into_domain(self) -> OrderAdjustment {
        OrderAdjustment {
            id: self.id,
            order_id: OrderId::from_string(self.order_id),
            code: self.code,
            amount: self.amount,
            description: self.description,
            created_at: self.created_at,
        }
    }
}

/// Persistence row for `order_status_history`.
#[derive(sqlx::FromRow)]
struct HistoryRow {
    id: String,
    order_id: String,
    status: String,
    previous_status: Option<String>,
    note: Option<String>,
    created_at: i64,
}

impl HistoryRow {
    fn into_domain(self) -> Result<OrderStatusHistory, AppError> {
        let previous_status = match self.previous_status {
            Some(value) => Some(OrderStatus::from_stored(&value)?),
            None => None,
        };
        Ok(OrderStatusHistory {
            id: self.id,
            order_id: OrderId::from_string(self.order_id),
            status: OrderStatus::from_stored(&self.status)?,
            previous_status,
            note: self.note,
            created_at: self.created_at,
        })
    }
}

/// Persistence row for `order_fulfillments`.
#[derive(sqlx::FromRow)]
struct FulfillmentRow {
    id: String,
    order_id: String,
    status: String,
    tracking_numbers: String,
    shipped_at: Option<i64>,
    delivered_at: Option<i64>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl FulfillmentRow {
    fn into_domain(self) -> OrderFulfillment {
        OrderFulfillment {
            id: FulfillmentId::from_string(self.id),
            order_id: OrderId::from_string(self.order_id),
            status: self.status,
            tracking_numbers: json_string_list(&self.tracking_numbers),
            shipped_at: self.shipped_at,
            delivered_at: self.delivered_at,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Persistence row for `order_refunds`.
#[derive(sqlx::FromRow)]
struct RefundRow {
    id: String,
    order_id: String,
    payment_id: Option<String>,
    amount: i64,
    currency_code: String,
    status: String,
    reason: Option<String>,
    provider_refund_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl RefundRow {
    fn into_domain(self) -> OrderRefund {
        OrderRefund {
            id: RefundId::from_string(self.id),
            order_id: OrderId::from_string(self.order_id),
            payment_id: self.payment_id.map(PaymentId::from_string),
            amount: self.amount,
            currency_code: self.currency_code,
            status: self.status,
            reason: self.reason,
            provider_refund_id: self.provider_refund_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// A validated line item with all derived amounts resolved.
struct ResolvedItem {
    product_id: Option<ProductId>,
    variant_id: Option<VariantId>,
    product_title: String,
    variant_title: Option<String>,
    sku: Option<String>,
    thumbnail: Option<String>,
    unit_price: i64,
    quantity: i64,
    subtotal: i64,
    discount_total: i64,
    tax_total: i64,
    total: i64,
    is_tax_inclusive: bool,
    requires_shipping: bool,
    metadata: Metadata,
    adjustments: Vec<CreateAdjustmentInput>,
}

/// A validated order ready to be inserted.
struct ResolvedOrder {
    customer_id: Option<CustomerId>,
    email: String,
    currency_code: String,
    items: Vec<ResolvedItem>,
    adjustments: Vec<CreateAdjustmentInput>,
    shipping_address: Value,
    billing_address: Value,
    subtotal: i64,
    discount_total: i64,
    shipping_total: i64,
    tax_total: i64,
    total: i64,
    metadata: Metadata,
    idempotency_key: Option<String>,
}

/// Why an order insert attempt did not produce a new order.
enum CreateAttemptError {
    /// The unique display-number index was hit; the transaction was rolled back.
    DisplayNumberConflict,
    /// Any other failure.
    Failed(AppError),
}

impl From<AppError> for CreateAttemptError {
    fn from(error: AppError) -> Self {
        Self::Failed(error)
    }
}

/// Order application service.
#[derive(Clone, Debug)]
pub struct OrderService {
    db: StoreDatabase,
}

impl OrderService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    /// Places an order from a validated snapshot.
    ///
    /// The order, its items, adjustments and initial history row are written in
    /// one transaction. When an idempotency key has already been used, the
    /// existing order is returned and nothing new is written.
    ///
    /// Totals are validated rather than silently recomputed:
    ///
    /// * `subtotal` must equal the sum of item subtotals.
    /// * `discount_total` must equal the sum of item discounts.
    /// * `tax_total` must equal the sum of item taxes.
    /// * `total` must equal the sum of item totals plus `shipping_total` plus
    ///   the signed sum of order-level adjustments.
    ///
    /// Item `subtotal` and `total` inputs are optional and are derived from
    /// `unit_price * quantity`, discount and tax when omitted.
    pub async fn create_order(&self, input: CreateOrderInput) -> Result<Order, AppError> {
        if let Some(key) = input.idempotency_key.as_deref() {
            if let Some(existing) = self.find_order_id_by_key(key).await? {
                return self.load_order(&existing).await;
            }
        }

        let resolved = resolve_order(input)?;
        let mut retried = false;
        loop {
            match self.insert_order(&resolved).await {
                Ok(order_id) => return self.load_order(&order_id).await,
                Err(CreateAttemptError::DisplayNumberConflict) if !retried => retried = true,
                Err(CreateAttemptError::DisplayNumberConflict) => {
                    return Err(AppError::conflict(
                        "could not allocate a unique order display number".to_owned(),
                    ));
                }
                Err(CreateAttemptError::Failed(error)) => return Err(error),
            }
        }
    }

    /// Loads a full order aggregate by identifier.
    pub async fn get(&self, id: &OrderId) -> Result<Order, AppError> {
        self.load_order(id).await
    }

    /// Loads a full order aggregate by its sequential display number.
    pub async fn get_by_display_number(&self, display_number: i64) -> Result<Order, AppError> {
        let id = sqlx::query_scalar::<_, String>("SELECT id FROM orders WHERE display_number = ?")
            .bind(display_number)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
        match id {
            Some(id) => self.load_order(&OrderId::from_string(id)).await,
            None => Err(AppError::not_found("order display number", display_number)),
        }
    }

    /// Lists orders matching `filter`, newest first.
    pub async fn list(
        &self,
        filter: OrderFilter,
        pagination: Pagination,
    ) -> Result<fastcart_kernel::PageResult<Order>, AppError> {
        if pagination.limit < 1 || pagination.limit > Pagination::MAX_LIMIT {
            return Err(AppError::validation(format!(
                "limit must be between 1 and {}",
                Pagination::MAX_LIMIT
            )));
        }
        if pagination.offset < 0 {
            return Err(AppError::validation(
                "offset must not be negative".to_owned(),
            ));
        }

        let (where_clause, binds) = build_filter(&filter);

        let count_sql = format!("SELECT COUNT(*) FROM orders{where_clause}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        for bind in &binds {
            count_query = match bind {
                FilterBind::Text(value) => count_query.bind(value.as_str()),
                FilterBind::Integer(value) => count_query.bind(*value),
            };
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT id, display_number, customer_id, email, currency_code, status,
                    payment_status, fulfillment_status, refund_status, subtotal,
                    discount_total, shipping_total, tax_total, total, shipping_address,
                    billing_address, metadata, created_at, updated_at, canceled_at
             FROM orders{where_clause}
             ORDER BY created_at DESC, display_number DESC
             LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query_as::<_, OrderRow>(&list_sql);
        for bind in &binds {
            list_query = match bind {
                FilterBind::Text(value) => list_query.bind(value.as_str()),
                FilterBind::Integer(value) => list_query.bind(*value),
            };
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut data = Vec::with_capacity(rows.len());
        for row in rows {
            data.push(self.load_order(&OrderId::from_string(row.id)).await?);
        }
        Ok(fastcart_kernel::PageResult::new(data, count, pagination))
    }

    /// Updates the payment status of an order.
    ///
    /// Used by checkout workflows after a provider capture or refund. Order
    /// status and payment status remain independent dimensions.
    pub async fn set_payment_status(
        &self,
        id: &OrderId,
        status: PaymentStatus,
    ) -> Result<Order, AppError> {
        let order = self.load_order(id).await?;
        if order.payment_status == status {
            return Ok(order);
        }
        let allowed = match order.payment_status {
            PaymentStatus::Awaiting | PaymentStatus::Authorized => {
                matches!(status, PaymentStatus::Captured | PaymentStatus::Failed)
            }
            PaymentStatus::Captured => matches!(
                status,
                PaymentStatus::PartiallyRefunded | PaymentStatus::Refunded
            ),
            PaymentStatus::PartiallyRefunded => matches!(status, PaymentStatus::Refunded),
            PaymentStatus::Refunded | PaymentStatus::Failed => false,
        };
        if !allowed {
            return Err(AppError::conflict(format!(
                "cannot transition payment status from {} to {}",
                order.payment_status.as_str(),
                status.as_str()
            )));
        }
        sqlx::query("UPDATE orders SET payment_status = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(now_timestamp())
            .bind(id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        self.get(id).await
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

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query("UPDATE orders SET status = ?, canceled_at = ?, updated_at = ? WHERE id = ?")
            .bind(OrderStatus::Canceled.as_str())
            .bind(now)
            .bind(now)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        insert_history(
            &mut tx,
            id,
            OrderStatus::Canceled,
            Some(order.status),
            reason.or(Some("order canceled")),
            now,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.load_order(id).await
    }

    /// Records a fulfillment and updates the order fulfillment status.
    pub async fn fulfill(&self, id: &OrderId, input: FulfillmentInput) -> Result<Order, AppError> {
        let order = self.load_order(id).await?;
        if matches!(order.status, OrderStatus::Canceled | OrderStatus::Completed) {
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

        let now = now_timestamp();
        let shipped_at = input.shipped_at.unwrap_or(now);
        let next_status = if input.partial {
            FulfillmentStatus::PartiallyFulfilled
        } else {
            FulfillmentStatus::Fulfilled
        };
        let note = if input.partial {
            "partial fulfillment recorded"
        } else {
            "fulfillment recorded"
        };

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "INSERT INTO order_fulfillments
                (id, order_id, status, tracking_numbers, shipped_at, delivered_at,
                 metadata, created_at, updated_at)
             VALUES (?, ?, 'fulfilled', ?, ?, NULL, '{}', ?, ?)",
        )
        .bind(FulfillmentId::new().as_str())
        .bind(id.as_str())
        .bind(json_text(&input.tracking_numbers)?)
        .bind(shipped_at)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        sqlx::query("UPDATE orders SET fulfillment_status = ?, updated_at = ? WHERE id = ?")
            .bind(next_status.as_str())
            .bind(now)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;

        insert_history(
            &mut tx,
            id,
            order.status,
            Some(order.status),
            Some(note),
            now,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.load_order(id).await
    }

    /// Records a refund and updates refund and payment status.
    ///
    /// Refunds are cumulative: the sum of all refunds may never exceed the
    /// order total. When the cumulative refund reaches the order total the
    /// order is marked refunded, otherwise partially refunded.
    pub async fn add_refund(&self, id: &OrderId, input: RefundInput) -> Result<Order, AppError> {
        if input.amount <= 0 {
            return Err(AppError::validation(
                "refund amount must be greater than zero".to_owned(),
            ));
        }
        let currency = CurrencyCode::new(&input.currency)?.as_str().to_owned();
        let order = self.load_order(id).await?;
        if currency != order.currency_code {
            return Err(AppError::validation(format!(
                "refund currency {currency} does not match order currency {}",
                order.currency_code
            )));
        }

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let refunded: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM order_refunds WHERE order_id = ?",
        )
        .bind(id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        let total_refunded = refunded
            .checked_add(input.amount)
            .ok_or_else(|| AppError::validation("refund amount overflow"))?;
        if total_refunded > order.total {
            return Err(AppError::conflict(format!(
                "refund of {} exceeds the refundable order total of {}",
                input.amount, order.total
            )));
        }
        let refund_status = if total_refunded >= order.total {
            RefundStatus::Refunded
        } else {
            RefundStatus::PartiallyRefunded
        };
        let payment_status = if total_refunded >= order.total {
            PaymentStatus::Refunded
        } else {
            PaymentStatus::PartiallyRefunded
        };

        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO order_refunds
                (id, order_id, payment_id, amount, currency_code, status, reason,
                 provider_refund_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'pending', ?, NULL, ?, ?)",
        )
        .bind(RefundId::new().as_str())
        .bind(id.as_str())
        .bind(input.payment_id.as_str())
        .bind(input.amount)
        .bind(&currency)
        .bind(input.reason.as_deref())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        sqlx::query(
            "UPDATE orders SET refund_status = ?, payment_status = ?, updated_at = ? WHERE id = ?",
        )
        .bind(refund_status.as_str())
        .bind(payment_status.as_str())
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        insert_history(
            &mut tx,
            id,
            order.status,
            Some(order.status),
            Some("refund recorded"),
            now,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.load_order(id).await
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

    async fn apply_status(
        &self,
        id: &OrderId,
        previous: OrderStatus,
        next: OrderStatus,
        note: Option<&str>,
    ) -> Result<(), AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query("UPDATE orders SET status = ?, updated_at = ? WHERE id = ?")
            .bind(next.as_str())
            .bind(now)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        insert_history(&mut tx, id, next, Some(previous), note, now).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    async fn find_order_id_by_key(&self, key: &str) -> Result<Option<OrderId>, AppError> {
        let resource = sqlx::query_scalar::<_, String>(
            "SELECT resource_id FROM idempotency_keys WHERE key = ? AND scope = ?",
        )
        .bind(key)
        .bind(IDEMPOTENCY_SCOPE)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(resource.map(OrderId::from_string))
    }

    async fn insert_order(&self, order: &ResolvedOrder) -> Result<OrderId, CreateAttemptError> {
        let mut tx = self
            .db
            .begin()
            .await
            .map_err(|error| CreateAttemptError::Failed(AppError::from(error)))?;

        if let Some(key) = order.idempotency_key.as_deref() {
            let existing = sqlx::query_scalar::<_, String>(
                "SELECT resource_id FROM idempotency_keys WHERE key = ? AND scope = ?",
            )
            .bind(key)
            .bind(IDEMPOTENCY_SCOPE)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;
            if let Some(existing) = existing {
                tx.commit()
                    .await
                    .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;
                return Ok(OrderId::from_string(existing));
            }
        }

        let display_number: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(display_number), 0) + 1 FROM orders")
                .fetch_one(&mut *tx)
                .await
                .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;

        let order_id = OrderId::new();
        let now = now_timestamp();
        let insert = sqlx::query(
            "INSERT INTO orders (
                id, display_number, customer_id, email, currency_code, status,
                payment_status, fulfillment_status, refund_status, subtotal,
                discount_total, shipping_total, tax_total, total, shipping_address,
                billing_address, metadata, created_at, updated_at, canceled_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(order_id.as_str())
        .bind(display_number)
        .bind(order.customer_id.as_ref().map(|value| value.as_str()))
        .bind(&order.email)
        .bind(&order.currency_code)
        .bind(OrderStatus::Pending.as_str())
        .bind(PaymentStatus::Awaiting.as_str())
        .bind(FulfillmentStatus::NotFulfilled.as_str())
        .bind(RefundStatus::NotRefunded.as_str())
        .bind(order.subtotal)
        .bind(order.discount_total)
        .bind(order.shipping_total)
        .bind(order.tax_total)
        .bind(order.total)
        .bind(json_text(&order.shipping_address)?)
        .bind(json_text(&order.billing_address)?)
        .bind(order.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await;

        if let Err(error) = insert {
            if is_unique_violation(&error) {
                return Err(CreateAttemptError::DisplayNumberConflict);
            }
            return Err(CreateAttemptError::Failed(database_error(error)));
        }

        for item in &order.items {
            let item_id = LineItemId::new();
            sqlx::query(
                "INSERT INTO order_items (
                    id, order_id, product_id, variant_id, product_title, variant_title,
                    sku, thumbnail, unit_price, quantity, subtotal, discount_total,
                    tax_total, total, is_tax_inclusive, requires_shipping, metadata
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(item_id.as_str())
            .bind(order_id.as_str())
            .bind(item.product_id.as_ref().map(|value| value.as_str()))
            .bind(item.variant_id.as_ref().map(|value| value.as_str()))
            .bind(&item.product_title)
            .bind(item.variant_title.as_deref())
            .bind(item.sku.as_deref())
            .bind(item.thumbnail.as_deref())
            .bind(item.unit_price)
            .bind(item.quantity)
            .bind(item.subtotal)
            .bind(item.discount_total)
            .bind(item.tax_total)
            .bind(item.total)
            .bind(if item.is_tax_inclusive { 1 } else { 0 })
            .bind(if item.requires_shipping { 1 } else { 0 })
            .bind(item.metadata.to_json_string())
            .execute(&mut *tx)
            .await
            .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;

            for adjustment in &item.adjustments {
                let adjustment_id = EventId::new();
                sqlx::query(
                    "INSERT INTO order_item_adjustments
                        (id, order_item_id, promotion_id, code, amount, description, created_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(adjustment_id.as_str())
                .bind(item_id.as_str())
                .bind(adjustment.promotion_id.as_ref().map(|value| value.as_str()))
                .bind(adjustment.code.as_deref())
                .bind(adjustment.amount)
                .bind(adjustment.description.as_deref())
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;
            }
        }

        for adjustment in &order.adjustments {
            let adjustment_id = EventId::new();
            sqlx::query(
                "INSERT INTO order_adjustments
                    (id, order_id, code, amount, description, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(adjustment_id.as_str())
            .bind(order_id.as_str())
            .bind(adjustment.code.as_deref().unwrap_or("order_adjustment"))
            .bind(adjustment.amount)
            .bind(adjustment.description.as_deref())
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;
        }

        insert_history(
            &mut tx,
            &order_id,
            OrderStatus::Pending,
            None,
            Some("order created"),
            now,
        )
        .await
        .map_err(CreateAttemptError::Failed)?;

        if let Some(key) = order.idempotency_key.as_deref() {
            let response = serde_json::json!({"order_id": order_id.as_str()}).to_string();
            let result = sqlx::query(
                "INSERT INTO idempotency_keys (key, scope, resource_id, response, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(key)
            .bind(IDEMPOTENCY_SCOPE)
            .bind(order_id.as_str())
            .bind(response)
            .bind(now)
            .execute(&mut *tx)
            .await;
            if let Err(error) = result {
                if is_unique_violation(&error) {
                    return Err(CreateAttemptError::Failed(AppError::conflict(
                        "idempotency key was used concurrently".to_owned(),
                    )));
                }
                return Err(CreateAttemptError::Failed(database_error(error)));
            }
        }

        tx.commit()
            .await
            .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;
        Ok(order_id)
    }

    async fn load_order(&self, id: &OrderId) -> Result<Order, AppError> {
        let row = sqlx::query_as::<_, OrderRow>(
            "SELECT id, display_number, customer_id, email, currency_code, status,
                    payment_status, fulfillment_status, refund_status, subtotal,
                    discount_total, shipping_total, tax_total, total, shipping_address,
                    billing_address, metadata, created_at, updated_at, canceled_at
             FROM orders WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        let row = match row {
            Some(row) => row,
            None => return Err(AppError::not_found("order", id)),
        };

        let item_rows = sqlx::query_as::<_, OrderItemRow>(
            "SELECT id, order_id, product_id, variant_id, product_title, variant_title,
                    sku, thumbnail, unit_price, quantity, subtotal, discount_total,
                    tax_total, total, is_tax_inclusive, requires_shipping, metadata
             FROM order_items WHERE order_id = ?
             ORDER BY rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;

        let adjustment_rows = sqlx::query_as::<_, ItemAdjustmentRow>(
            "SELECT a.id, a.order_item_id, a.promotion_id, a.code, a.amount,
                    a.description, a.created_at
             FROM order_item_adjustments a
             JOIN order_items i ON i.id = a.order_item_id
             WHERE i.order_id = ?
             ORDER BY a.created_at ASC, a.rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;

        let mut item_adjustments: HashMap<String, Vec<OrderItemAdjustment>> = HashMap::new();
        for row in adjustment_rows {
            let adjustment = row.into_domain();
            item_adjustments
                .entry(adjustment.order_item_id.as_str().to_owned())
                .or_default()
                .push(adjustment);
        }
        let items = item_rows
            .into_iter()
            .map(|row| {
                let adjustments = item_adjustments.remove(&row.id).unwrap_or_default();
                row.into_domain(adjustments)
            })
            .collect();

        let order_adjustment_rows = sqlx::query_as::<_, AdjustmentRow>(
            "SELECT id, order_id, code, amount, description, created_at
             FROM order_adjustments WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let adjustments = order_adjustment_rows
            .into_iter()
            .map(AdjustmentRow::into_domain)
            .collect();

        let history_rows = sqlx::query_as::<_, HistoryRow>(
            "SELECT id, order_id, status, previous_status, note, created_at
             FROM order_status_history WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let status_history = history_rows
            .into_iter()
            .map(HistoryRow::into_domain)
            .collect::<Result<Vec<_>, _>>()?;

        let fulfillment_rows = sqlx::query_as::<_, FulfillmentRow>(
            "SELECT id, order_id, status, tracking_numbers, shipped_at, delivered_at,
                    metadata, created_at, updated_at
             FROM order_fulfillments WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let fulfillments = fulfillment_rows
            .into_iter()
            .map(FulfillmentRow::into_domain)
            .collect();

        let refund_rows = sqlx::query_as::<_, RefundRow>(
            "SELECT id, order_id, payment_id, amount, currency_code, status, reason,
                    provider_refund_id, created_at, updated_at
             FROM order_refunds WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let refunds = refund_rows
            .into_iter()
            .map(RefundRow::into_domain)
            .collect();

        Ok(Order {
            id: OrderId::from_string(row.id),
            display_number: row.display_number,
            customer_id: row.customer_id.map(CustomerId::from_string),
            email: row.email,
            currency_code: row.currency_code,
            status: OrderStatus::from_stored(&row.status)?,
            payment_status: PaymentStatus::from_stored(&row.payment_status)?,
            fulfillment_status: FulfillmentStatus::from_stored(&row.fulfillment_status)?,
            refund_status: RefundStatus::from_stored(&row.refund_status)?,
            subtotal: row.subtotal,
            discount_total: row.discount_total,
            shipping_total: row.shipping_total,
            tax_total: row.tax_total,
            total: row.total,
            shipping_address: json_value(&row.shipping_address),
            billing_address: json_value(&row.billing_address),
            metadata: Metadata::from_json_str(&row.metadata),
            created_at: row.created_at,
            updated_at: row.updated_at,
            canceled_at: row.canceled_at,
            items,
            adjustments,
            status_history,
            fulfillments,
            refunds,
        })
    }
}

async fn insert_history(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    order_id: &OrderId,
    status: OrderStatus,
    previous_status: Option<OrderStatus>,
    note: Option<&str>,
    created_at: Timestamp,
) -> Result<OrderStatusHistory, AppError> {
    let id = EventId::new();
    sqlx::query(
        "INSERT INTO order_status_history
            (id, order_id, status, previous_status, note, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.as_str())
    .bind(order_id.as_str())
    .bind(status.as_str())
    .bind(previous_status.map(OrderStatus::as_str))
    .bind(note)
    .bind(created_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(OrderStatusHistory {
        id: id.as_str().to_owned(),
        order_id: order_id.clone(),
        status,
        previous_status,
        note: note.map(str::to_owned),
        created_at,
    })
}

/// A bound value used by the dynamic order filter.
enum FilterBind {
    Text(String),
    Integer(i64),
}

fn build_filter(filter: &OrderFilter) -> (String, Vec<FilterBind>) {
    let mut conditions = Vec::new();
    let mut binds = Vec::new();

    if let Some(status) = filter.status {
        conditions.push("status = ?");
        binds.push(FilterBind::Text(status.as_str().to_owned()));
    }
    if let Some(payment_status) = filter.payment_status {
        conditions.push("payment_status = ?");
        binds.push(FilterBind::Text(payment_status.as_str().to_owned()));
    }
    if let Some(customer_id) = &filter.customer_id {
        conditions.push("customer_id = ?");
        binds.push(FilterBind::Text(customer_id.as_str().to_owned()));
    }
    if let Some(email) = &filter.email {
        conditions.push("email = ?");
        binds.push(FilterBind::Text(normalize_email(email)));
    }
    if let Some(from) = filter.from {
        conditions.push("created_at >= ?");
        binds.push(FilterBind::Integer(from));
    }
    if let Some(to) = filter.to {
        conditions.push("created_at <= ?");
        binds.push(FilterBind::Integer(to));
    }

    if conditions.is_empty() {
        (String::new(), binds)
    } else {
        (format!(" WHERE {}", conditions.join(" AND ")), binds)
    }
}

fn resolve_order(input: CreateOrderInput) -> Result<ResolvedOrder, AppError> {
    validate_email(&input.email, "email")?;
    let email = normalize_email(&input.email);
    let currency_code = CurrencyCode::new(&input.currency_code)?.as_str().to_owned();
    validate_metadata(&input.metadata)?;
    validate_non_negative_amount(input.subtotal, "subtotal")?;
    validate_non_negative_amount(input.discount_total, "discount_total")?;
    validate_non_negative_amount(input.shipping_total, "shipping_total")?;
    validate_non_negative_amount(input.tax_total, "tax_total")?;
    validate_non_negative_amount(input.total, "total")?;
    if input.items.is_empty() {
        return Err(AppError::validation(
            "an order must contain at least one item".to_owned(),
        ));
    }

    let mut items = Vec::with_capacity(input.items.len());
    for item in input.items {
        validate_length(&item.product_title, "product_title", 255)?;
        validate_quantity(item.quantity, false)?;
        validate_non_negative_amount(item.unit_price, "unit_price")?;
        validate_metadata(&item.metadata)?;

        let computed_subtotal = item
            .unit_price
            .checked_mul(item.quantity)
            .ok_or_else(|| AppError::validation("item subtotal overflow"))?;
        let subtotal = match item.subtotal {
            Some(value) => {
                validate_non_negative_amount(value, "item subtotal")?;
                if value != computed_subtotal {
                    return Err(AppError::validation(format!(
                        "item subtotal {value} does not match unit_price * quantity \
                         ({computed_subtotal})"
                    )));
                }
                value
            }
            None => computed_subtotal,
        };
        let discount_total = item.discount_total.unwrap_or(0);
        validate_non_negative_amount(discount_total, "item discount_total")?;
        let tax_total = item.tax_total.unwrap_or(0);
        validate_non_negative_amount(tax_total, "item tax_total")?;
        let total = match item.total {
            Some(value) => {
                validate_non_negative_amount(value, "item total")?;
                value
            }
            None => subtotal
                .checked_sub(discount_total)
                .and_then(|value| value.checked_add(tax_total))
                .ok_or_else(|| AppError::validation("item total overflow"))?,
        };

        items.push(ResolvedItem {
            product_id: item.product_id,
            variant_id: item.variant_id,
            product_title: item.product_title,
            variant_title: item.variant_title,
            sku: item.sku,
            thumbnail: item.thumbnail,
            unit_price: item.unit_price,
            quantity: item.quantity,
            subtotal,
            discount_total,
            tax_total,
            total,
            is_tax_inclusive: item.is_tax_inclusive,
            requires_shipping: item.requires_shipping,
            metadata: item.metadata,
            adjustments: item.adjustments,
        });
    }

    let computed_subtotal = checked_sum(items.iter().map(|item| item.subtotal))?;
    let computed_discount = checked_sum(items.iter().map(|item| item.discount_total))?;
    let computed_tax = checked_sum(items.iter().map(|item| item.tax_total))?;
    let computed_item_total = checked_sum(items.iter().map(|item| item.total))?;

    if input.subtotal != computed_subtotal {
        return Err(AppError::validation(format!(
            "order subtotal {} does not match item subtotals {computed_subtotal}",
            input.subtotal
        )));
    }
    if input.discount_total != computed_discount {
        return Err(AppError::validation(format!(
            "order discount_total {} does not match item discounts {computed_discount}",
            input.discount_total
        )));
    }
    if input.tax_total != computed_tax {
        return Err(AppError::validation(format!(
            "order tax_total {} does not match item taxes {computed_tax}",
            input.tax_total
        )));
    }

    let adjustment_total = checked_sum(input.adjustments.iter().map(|value| value.amount))?;
    let expected_total = computed_item_total
        .checked_add(input.shipping_total)
        .and_then(|value| value.checked_add(adjustment_total))
        .ok_or_else(|| AppError::validation("order total overflow"))?;
    if input.total != expected_total {
        return Err(AppError::validation(format!(
            "order total {} does not match the computed total {expected_total}",
            input.total
        )));
    }
    if expected_total < 0 {
        return Err(AppError::validation(
            "order total must not be negative".to_owned(),
        ));
    }

    Ok(ResolvedOrder {
        customer_id: input.customer_id,
        email,
        currency_code,
        items,
        adjustments: input.adjustments,
        shipping_address: input.shipping_address,
        billing_address: input.billing_address,
        subtotal: input.subtotal,
        discount_total: input.discount_total,
        shipping_total: input.shipping_total,
        tax_total: input.tax_total,
        total: input.total,
        metadata: input.metadata,
        idempotency_key: input
            .idempotency_key
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
    })
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

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
}

fn json_text(value: &impl serde::Serialize) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::validation(format!("invalid JSON value: {error}")))
}

fn json_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn json_string_list(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use fastcart_infrastructure::StoreDatabase;
    use fastcart_kernel::{
        CustomerId, OrderId, Pagination, PaymentId, ProductId, StoreId, VariantId, now_timestamp,
    };
    use serde_json::json;

    use super::OrderService;
    use crate::domain::{
        CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput, FulfillmentInput,
        FulfillmentStatus, OrderFilter, OrderStatus, PaymentStatus, RefundInput, RefundStatus,
    };

    async fn service() -> OrderService {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        OrderService::new(db)
    }

    fn item(unit_price: i64, quantity: i64) -> CreateOrderItemInput {
        CreateOrderItemInput {
            product_title: "Widget".to_owned(),
            unit_price,
            quantity,
            ..CreateOrderItemInput::default()
        }
    }

    fn order_input(items: Vec<CreateOrderItemInput>) -> CreateOrderInput {
        let subtotal: i64 = items
            .iter()
            .map(|item| item.unit_price * item.quantity)
            .sum();
        CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items,
            subtotal,
            total: subtotal,
            shipping_address: json!({"city": "Berlin"}),
            billing_address: json!({"city": "Berlin"}),
            ..CreateOrderInput::default()
        }
    }

    #[tokio::test]
    async fn creates_order_with_immutable_snapshots() {
        let service = service().await;
        let mut line = item(1_250, 2);
        line.product_id = Some(ProductId::new());
        line.variant_id = Some(VariantId::new());
        line.product_title = "Original title".to_owned();
        line.variant_title = Some("Large".to_owned());
        line.sku = Some("SKU-1".to_owned());
        line.thumbnail = Some("https://example.com/a.png".to_owned());
        line.is_tax_inclusive = true;
        line.requires_shipping = false;
        line.metadata.insert("color", json!("red"));
        line.adjustments = vec![CreateAdjustmentInput {
            code: Some("SAVE".to_owned()),
            amount: -100,
            ..CreateAdjustmentInput::default()
        }];

        let order = service.create_order(order_input(vec![line])).await.unwrap();

        assert_eq!(order.status, OrderStatus::Pending);
        assert_eq!(order.payment_status, PaymentStatus::Awaiting);
        assert_eq!(order.fulfillment_status, FulfillmentStatus::NotFulfilled);
        assert_eq!(order.refund_status, RefundStatus::NotRefunded);
        assert_eq!(order.display_number, 1);
        assert_eq!(order.subtotal, 2_500);
        assert_eq!(order.total, 2_500);
        assert_eq!(order.items.len(), 1);
        let snapshot = &order.items[0];
        assert_eq!(snapshot.product_title, "Original title");
        assert_eq!(snapshot.variant_title.as_deref(), Some("Large"));
        assert_eq!(snapshot.sku.as_deref(), Some("SKU-1"));
        assert_eq!(snapshot.unit_price, 1_250);
        assert_eq!(snapshot.quantity, 2);
        assert!(snapshot.is_tax_inclusive);
        assert!(!snapshot.requires_shipping);
        assert_eq!(snapshot.adjustments.len(), 1);
        assert_eq!(order.status_history.len(), 1);
    }

    #[tokio::test]
    async fn create_is_idempotent_and_writes_nothing_extra() {
        let service = service().await;
        let mut input = order_input(vec![item(1_000, 1)]);
        input.idempotency_key = Some("order-key".to_owned());

        let first = service.create_order(input.clone()).await.unwrap();
        let second = service.create_order(input).await.unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.items.len(), 1);
        let page = service
            .list(OrderFilter::default(), Pagination::default())
            .await
            .unwrap();
        assert_eq!(page.count, 1);
    }

    #[tokio::test]
    async fn display_numbers_increment() {
        let service = service().await;
        let first = service
            .create_order(order_input(vec![item(100, 1)]))
            .await
            .unwrap();
        let second = service
            .create_order(order_input(vec![item(200, 1)]))
            .await
            .unwrap();

        assert_eq!(first.display_number, 1);
        assert_eq!(second.display_number, 2);
        let found = service.get_by_display_number(2).await.unwrap();
        assert_eq!(found.id, second.id);
        assert!(service.get_by_display_number(99).await.is_err());
    }

    #[tokio::test]
    async fn lists_orders_with_filters_and_pagination() {
        let service = service().await;
        let customer = CustomerId::new();

        let mut first_input = order_input(vec![item(100, 1)]);
        first_input.email = "first@example.com".to_owned();
        first_input.customer_id = Some(customer.clone());
        let first = service.create_order(first_input).await.unwrap();

        let mut second_input = order_input(vec![item(200, 1)]);
        second_input.email = "second@example.com".to_owned();
        service.create_order(second_input).await.unwrap();

        let mut third_input = order_input(vec![item(300, 1)]);
        third_input.email = "third@example.com".to_owned();
        service.create_order(third_input).await.unwrap();

        let all = service
            .list(OrderFilter::default(), Pagination::default())
            .await
            .unwrap();
        assert_eq!(all.count, 3);

        let filtered = service
            .list(
                OrderFilter {
                    email: Some("FIRST@example.com".to_owned()),
                    ..OrderFilter::default()
                },
                Pagination::default(),
            )
            .await
            .unwrap();
        assert_eq!(filtered.count, 1);
        assert_eq!(filtered.data[0].id, first.id);

        let by_customer = service
            .list(
                OrderFilter {
                    customer_id: Some(customer),
                    ..OrderFilter::default()
                },
                Pagination::default(),
            )
            .await
            .unwrap();
        assert_eq!(by_customer.count, 1);

        let page = service
            .list(
                OrderFilter::default(),
                Pagination {
                    limit: 1,
                    offset: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.count, 3);
        assert_eq!(page.data.len(), 1);

        let future = now_timestamp() + 3_600;
        let none = service
            .list(
                OrderFilter {
                    from: Some(future),
                    ..OrderFilter::default()
                },
                Pagination::default(),
            )
            .await
            .unwrap();
        assert_eq!(none.count, 0);
    }

    #[tokio::test]
    async fn filters_orders_by_status() {
        let service = service().await;
        let first = service
            .create_order(order_input(vec![item(100, 1)]))
            .await
            .unwrap();
        service.confirm(&first.id).await.unwrap();
        service
            .create_order(order_input(vec![item(200, 1)]))
            .await
            .unwrap();

        let pending = service
            .list(
                OrderFilter {
                    status: Some(OrderStatus::Pending),
                    ..OrderFilter::default()
                },
                Pagination::default(),
            )
            .await
            .unwrap();
        assert_eq!(pending.count, 1);

        let confirmed = service
            .list(
                OrderFilter {
                    status: Some(OrderStatus::Confirmed),
                    payment_status: Some(PaymentStatus::Awaiting),
                    ..OrderFilter::default()
                },
                Pagination::default(),
            )
            .await
            .unwrap();
        assert_eq!(confirmed.count, 1);
        assert_eq!(confirmed.data[0].id, first.id);
    }

    #[tokio::test]
    async fn confirm_and_start_processing_follow_the_lifecycle() {
        let service = service().await;
        let order = service
            .create_order(order_input(vec![item(100, 1)]))
            .await
            .unwrap();

        let confirmed = service.confirm(&order.id).await.unwrap();
        assert_eq!(confirmed.status, OrderStatus::Confirmed);
        let processing = service.start_processing(&order.id).await.unwrap();
        assert_eq!(processing.status, OrderStatus::Processing);

        let error = service.confirm(&order.id).await.unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn cancel_records_history_and_is_idempotent_safe() {
        let service = service().await;
        let order = service
            .create_order(order_input(vec![item(100, 1)]))
            .await
            .unwrap();

        let canceled = service
            .cancel(&order.id, Some("customer request"))
            .await
            .unwrap();
        assert_eq!(canceled.status, OrderStatus::Canceled);
        assert!(canceled.canceled_at.is_some());
        let history = service.list_history(&order.id).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].note.as_deref(), Some("customer request"));

        let error = service.cancel(&order.id, None).await.unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn full_fulfillment_records_a_fulfillment() {
        let service = service().await;
        let order = service
            .create_order(order_input(vec![item(100, 2)]))
            .await
            .unwrap();

        let fulfilled = service
            .fulfill(
                &order.id,
                FulfillmentInput {
                    tracking_numbers: vec!["TRACK-1".to_owned()],
                    shipped_at: None,
                    partial: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(fulfilled.fulfillment_status, FulfillmentStatus::Fulfilled);
        assert_eq!(fulfilled.fulfillments.len(), 1);
        assert_eq!(
            fulfilled.fulfillments[0].tracking_numbers,
            vec!["TRACK-1".to_owned()]
        );
        assert!(fulfilled.fulfillments[0].shipped_at.is_some());
    }

    #[tokio::test]
    async fn partial_fulfillment_is_tracked_separately() {
        let service = service().await;
        let order = service
            .create_order(order_input(vec![item(100, 3)]))
            .await
            .unwrap();

        let partial = service
            .fulfill(
                &order.id,
                FulfillmentInput {
                    tracking_numbers: vec!["TRACK-2".to_owned()],
                    shipped_at: Some(1_700_000_000),
                    partial: true,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            partial.fulfillment_status,
            FulfillmentStatus::PartiallyFulfilled
        );
        assert_eq!(partial.fulfillments[0].shipped_at, Some(1_700_000_000));

        let full = service
            .fulfill(
                &order.id,
                FulfillmentInput {
                    tracking_numbers: vec!["TRACK-3".to_owned()],
                    shipped_at: None,
                    partial: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(full.fulfillment_status, FulfillmentStatus::Fulfilled);
        assert_eq!(full.fulfillments.len(), 2);
    }

    #[tokio::test]
    async fn refunds_progress_from_partial_to_full() {
        let service = service().await;
        let order = service
            .create_order(order_input(vec![item(1_000, 2)]))
            .await
            .unwrap();
        let payment = PaymentId::new();

        let partial = service
            .add_refund(
                &order.id,
                RefundInput {
                    payment_id: payment.clone(),
                    amount: 500,
                    currency: "USD".to_owned(),
                    reason: Some("damaged".to_owned()),
                },
            )
            .await
            .unwrap();
        assert_eq!(partial.refund_status, RefundStatus::PartiallyRefunded);
        assert_eq!(partial.payment_status, PaymentStatus::PartiallyRefunded);
        assert_eq!(partial.refunds.len(), 1);

        let full = service
            .add_refund(
                &order.id,
                RefundInput {
                    payment_id: payment,
                    amount: 1_500,
                    currency: "USD".to_owned(),
                    reason: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(full.refund_status, RefundStatus::Refunded);
        assert_eq!(full.payment_status, PaymentStatus::Refunded);
        assert_eq!(full.refunds.len(), 2);
    }

    #[tokio::test]
    async fn rejects_over_refunds() {
        let service = service().await;
        let order = service
            .create_order(order_input(vec![item(1_000, 1)]))
            .await
            .unwrap();

        service
            .add_refund(
                &order.id,
                RefundInput {
                    payment_id: PaymentId::new(),
                    amount: 600,
                    currency: "USD".to_owned(),
                    reason: None,
                },
            )
            .await
            .unwrap();

        let error = service
            .add_refund(
                &order.id,
                RefundInput {
                    payment_id: PaymentId::new(),
                    amount: 500,
                    currency: "USD".to_owned(),
                    reason: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn validates_currency_email_and_amounts() {
        let service = service().await;

        let mut input = order_input(vec![item(100, 1)]);
        input.currency_code = "US".to_owned();
        assert!(service.create_order(input).await.is_err());

        let mut input = order_input(vec![item(100, 1)]);
        input.email = "bad".to_owned();
        assert!(service.create_order(input).await.is_err());

        let mut input = order_input(vec![item(-100, 1)]);
        input.subtotal = -100;
        input.total = -100;
        assert!(service.create_order(input).await.is_err());

        let mut input = order_input(vec![item(100, 0)]);
        input.subtotal = 0;
        input.total = 0;
        assert!(service.create_order(input).await.is_err());

        let mut input = order_input(vec![item(100, 1)]);
        input.total = 999;
        assert!(service.create_order(input).await.is_err());

        let input = order_input(Vec::new());
        assert!(service.create_order(input).await.is_err());
    }

    #[tokio::test]
    async fn rejects_invalid_refund_amounts_and_currencies() {
        let service = service().await;
        let order = service
            .create_order(order_input(vec![item(1_000, 1)]))
            .await
            .unwrap();

        let zero = service
            .add_refund(
                &order.id,
                RefundInput {
                    payment_id: PaymentId::new(),
                    amount: 0,
                    currency: "USD".to_owned(),
                    reason: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(zero, fastcart_kernel::AppError::Validation(_)));

        let wrong_currency = service
            .add_refund(
                &order.id,
                RefundInput {
                    payment_id: PaymentId::new(),
                    amount: 100,
                    currency: "EUR".to_owned(),
                    reason: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            wrong_currency,
            fastcart_kernel::AppError::Validation(_)
        ));
    }

    #[tokio::test]
    async fn order_level_adjustments_must_be_in_the_total() {
        let service = service().await;
        let mut input = order_input(vec![item(1_000, 1)]);
        input.adjustments = vec![CreateAdjustmentInput {
            code: Some("GIFT".to_owned()),
            amount: 250,
            ..CreateAdjustmentInput::default()
        }];
        let error = service.create_order(input).await.unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));

        let mut input = order_input(vec![item(1_000, 1)]);
        input.adjustments = vec![CreateAdjustmentInput {
            code: Some("GIFT".to_owned()),
            amount: 250,
            ..CreateAdjustmentInput::default()
        }];
        input.total = 1_250;
        let order = service.create_order(input).await.unwrap();
        assert_eq!(order.total, 1_250);
        assert_eq!(order.adjustments.len(), 1);
    }

    #[tokio::test]
    async fn records_and_lists_history() {
        let service = service().await;
        let order = service
            .create_order(order_input(vec![item(100, 1)]))
            .await
            .unwrap();
        service
            .record_history(
                &order.id,
                OrderStatus::Pending,
                Some(OrderStatus::Pending),
                Some("manual note"),
            )
            .await
            .unwrap();

        let history = service.list_history(&order.id).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].note.as_deref(), Some("manual note"));
    }

    #[tokio::test]
    async fn missing_order_is_not_found() {
        let service = service().await;
        assert!(
            service
                .get(&OrderId::new())
                .await
                .unwrap_err()
                .is_not_found()
        );
        assert!(
            service
                .list(OrderFilter::default(), Pagination::default())
                .await
                .unwrap()
                .data
                .is_empty()
        );
    }
}
