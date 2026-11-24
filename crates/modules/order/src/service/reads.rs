//! Order aggregate reads and persistence row decoding.

use std::collections::HashMap;

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CustomerId, FulfillmentId, LineItemId, Metadata, OrderId, Pagination, PaymentId,
    ProductId, RefundId, VariantId, normalize_email,
};
use serde_json::Value;

use crate::domain::{
    FulfillmentStatus, Order, OrderAdjustment, OrderFilter, OrderFulfillment, OrderFulfillmentItem,
    OrderItem, OrderItemAdjustment, OrderRefund, OrderShippingMethod, OrderStatus,
    OrderStatusHistory, OrderTaxLine, PaymentStatus, RefundStatus,
};

use super::{IDEMPOTENCY_SCOPE, OrderService};

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
    shipping_methods: String,
    metadata: String,
    region_id: Option<String>,
    sales_channel_id: Option<String>,
    created_at: i64,
    updated_at: i64,
    canceled_at: Option<i64>,
}

impl OrderRow {
    #[allow(clippy::too_many_arguments)]
    fn into_order(
        self,
        items: Vec<OrderItem>,
        tax_lines: Vec<OrderTaxLine>,
        adjustments: Vec<OrderAdjustment>,
        status_history: Vec<OrderStatusHistory>,
        fulfillments: Vec<OrderFulfillment>,
        refunds: Vec<OrderRefund>,
    ) -> Result<Order, AppError> {
        Ok(Order {
            id: OrderId::from_string(self.id),
            display_number: self.display_number,
            customer_id: self.customer_id.map(CustomerId::from_string),
            email: self.email,
            currency_code: self.currency_code,
            status: OrderStatus::from_stored(&self.status)?,
            payment_status: PaymentStatus::from_stored(&self.payment_status)?,
            fulfillment_status: FulfillmentStatus::from_stored(&self.fulfillment_status)?,
            refund_status: RefundStatus::from_stored(&self.refund_status)?,
            subtotal: self.subtotal,
            discount_total: self.discount_total,
            shipping_total: self.shipping_total,
            tax_total: self.tax_total,
            total: self.total,
            shipping_address: json_value(&self.shipping_address),
            billing_address: json_value(&self.billing_address),
            shipping_methods: parse_shipping_methods(&self.shipping_methods)?,
            metadata: Metadata::from_json_str(&self.metadata),
            region_id: self.region_id,
            sales_channel_id: self.sales_channel_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
            canceled_at: self.canceled_at,
            items,
            tax_lines,
            adjustments,
            status_history,
            fulfillments,
            refunds,
        })
    }
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

/// Persistence row for `order_tax_lines`.
#[derive(sqlx::FromRow)]
struct OrderTaxLineRow {
    id: String,
    order_id: String,
    order_item_id: Option<String>,
    shipping_option_id: Option<String>,
    name: String,
    rate_bps: i64,
    amount: i64,
    is_tax_inclusive: i64,
    created_at: i64,
}

impl OrderTaxLineRow {
    fn into_domain(self) -> OrderTaxLine {
        OrderTaxLine {
            id: self.id,
            order_id: OrderId::from_string(self.order_id),
            order_item_id: self.order_item_id.map(LineItemId::from_string),
            shipping_option_id: self.shipping_option_id,
            name: self.name,
            rate_bps: self.rate_bps,
            amount: self.amount,
            is_tax_inclusive: self.is_tax_inclusive != 0,
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
pub(super) struct HistoryRow {
    id: String,
    order_id: String,
    status: String,
    previous_status: Option<String>,
    note: Option<String>,
    created_at: i64,
}

impl HistoryRow {
    pub(super) fn into_domain(self) -> Result<OrderStatusHistory, AppError> {
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
    provider_id: String,
    provider_fulfillment_id: Option<String>,
    provider_data: String,
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
            provider_id: self.provider_id,
            provider_fulfillment_id: self.provider_fulfillment_id,
            provider_data: json_value(&self.provider_data),
            tracking_numbers: json_string_list(&self.tracking_numbers),
            shipped_at: self.shipped_at,
            delivered_at: self.delivered_at,
            metadata: Metadata::from_json_str(&self.metadata),
            items: Vec::new(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Persistence row for `order_fulfillment_items`.
#[derive(sqlx::FromRow)]
struct FulfillmentItemRow {
    id: String,
    fulfillment_id: String,
    order_item_id: String,
    variant_id: Option<String>,
    quantity: i64,
    created_at: i64,
}

impl FulfillmentItemRow {
    fn into_domain(self) -> OrderFulfillmentItem {
        OrderFulfillmentItem {
            id: self.id,
            fulfillment_id: FulfillmentId::from_string(self.fulfillment_id),
            order_item_id: LineItemId::from_string(self.order_item_id),
            variant_id: self.variant_id.map(VariantId::from_string),
            quantity: self.quantity,
            created_at: self.created_at,
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

impl OrderService {
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
                    billing_address, shipping_methods, metadata, region_id, sales_channel_id, \
                    created_at, updated_at, canceled_at
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

        let data = self.load_orders(rows).await?;
        Ok(fastcart_kernel::PageResult::new(data, count, pagination))
    }

    pub(super) async fn find_order_id_by_key(
        &self,
        key: &str,
    ) -> Result<Option<OrderId>, AppError> {
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

    /// Loads a page of orders with a constant number of child-table queries.
    ///
    /// `load_order` is intentionally query-per-aggregate for single-order reads;
    /// list pages must not issue seven queries per row.
    async fn load_orders(&self, rows: Vec<OrderRow>) -> Result<Vec<Order>, AppError> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        let order_placeholders = placeholders(ids.len());

        let items_query_sql = format!(
            "SELECT id, order_id, product_id, variant_id, product_title, variant_title, \
                    sku, thumbnail, unit_price, quantity, subtotal, discount_total, \
                    tax_total, total, is_tax_inclusive, requires_shipping, metadata \
             FROM order_items WHERE order_id IN ({order_placeholders}) \
             ORDER BY order_id ASC, rowid ASC"
        );
        let mut items_query = sqlx::query_as::<_, OrderItemRow>(&items_query_sql);
        for id in &ids {
            items_query = items_query.bind(id);
        }
        let item_rows = items_query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let item_ids: Vec<String> = item_rows.iter().map(|row| row.id.clone()).collect();
        let mut adjustments_by_item: HashMap<String, Vec<OrderItemAdjustment>> = HashMap::new();
        if !item_ids.is_empty() {
            let item_placeholders = placeholders(item_ids.len());
            let query_sql = format!(
                "SELECT a.id, a.order_item_id, a.promotion_id, a.code, a.amount, \
                        a.description, a.created_at \
                 FROM order_item_adjustments a \
                 WHERE a.order_item_id IN ({item_placeholders}) \
                 ORDER BY a.created_at ASC, a.rowid ASC"
            );
            let mut query = sqlx::query_as::<_, ItemAdjustmentRow>(&query_sql);
            for id in &item_ids {
                query = query.bind(id);
            }
            for row in query
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?
            {
                let adjustment = row.into_domain();
                adjustments_by_item
                    .entry(adjustment.order_item_id.as_str().to_owned())
                    .or_default()
                    .push(adjustment);
            }
        }

        let mut items_by_order: HashMap<String, Vec<OrderItem>> = HashMap::new();
        for row in item_rows {
            let order_id = row.order_id.clone();
            let adjustments = adjustments_by_item.remove(&row.id).unwrap_or_default();
            items_by_order
                .entry(order_id)
                .or_default()
                .push(row.into_domain(adjustments));
        }

        let mut adjustments_by_order: HashMap<String, Vec<OrderAdjustment>> = HashMap::new();
        let adjustments_query_sql = format!(
            "SELECT id, order_id, code, amount, description, created_at \
             FROM order_adjustments WHERE order_id IN ({order_placeholders}) \
             ORDER BY created_at ASC, rowid ASC"
        );
        let mut adjustments_query = sqlx::query_as::<_, AdjustmentRow>(&adjustments_query_sql);
        for id in &ids {
            adjustments_query = adjustments_query.bind(id);
        }
        for row in adjustments_query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let order_id = row.order_id.clone();
            adjustments_by_order
                .entry(order_id)
                .or_default()
                .push(row.into_domain());
        }

        let mut history_by_order: HashMap<String, Vec<OrderStatusHistory>> = HashMap::new();
        let history_query_sql = format!(
            "SELECT id, order_id, status, previous_status, note, created_at \
             FROM order_status_history WHERE order_id IN ({order_placeholders}) \
             ORDER BY created_at ASC, rowid ASC"
        );
        let mut history_query = sqlx::query_as::<_, HistoryRow>(&history_query_sql);
        for id in &ids {
            history_query = history_query.bind(id);
        }
        for row in history_query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let order_id = row.order_id.clone();
            let history = row.into_domain()?;
            history_by_order.entry(order_id).or_default().push(history);
        }

        let mut fulfillments_by_order: HashMap<String, Vec<OrderFulfillment>> = HashMap::new();
        let fulfillments_query_sql = format!(
            "SELECT id, order_id, status, provider_id, provider_fulfillment_id, provider_data, \
                    tracking_numbers, shipped_at, delivered_at, metadata, created_at, updated_at \
             FROM order_fulfillments WHERE order_id IN ({order_placeholders}) \
             ORDER BY created_at ASC, rowid ASC"
        );
        let mut fulfillments_query = sqlx::query_as::<_, FulfillmentRow>(&fulfillments_query_sql);
        for id in &ids {
            fulfillments_query = fulfillments_query.bind(id);
        }
        let fulfillment_rows = fulfillments_query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let fulfillment_ids: Vec<String> =
            fulfillment_rows.iter().map(|row| row.id.clone()).collect();
        let mut items_by_fulfillment: HashMap<String, Vec<OrderFulfillmentItem>> = HashMap::new();
        if !fulfillment_ids.is_empty() {
            let fulfillment_placeholders = placeholders(fulfillment_ids.len());
            let query_sql = format!(
                "SELECT id, fulfillment_id, order_item_id, variant_id, quantity, created_at \
                 FROM order_fulfillment_items WHERE fulfillment_id IN ({fulfillment_placeholders}) \
                 ORDER BY created_at ASC, rowid ASC"
            );
            let mut query = sqlx::query_as::<_, FulfillmentItemRow>(&query_sql);
            for id in &fulfillment_ids {
                query = query.bind(id);
            }
            for row in query
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?
            {
                let item = row.into_domain();
                items_by_fulfillment
                    .entry(item.fulfillment_id.as_str().to_owned())
                    .or_default()
                    .push(item);
            }
        }
        for row in fulfillment_rows {
            let order_id = row.order_id.clone();
            let mut fulfillment = row.into_domain();
            fulfillment.items = items_by_fulfillment
                .remove(fulfillment.id.as_str())
                .unwrap_or_default();
            fulfillments_by_order
                .entry(order_id)
                .or_default()
                .push(fulfillment);
        }

        let mut refunds_by_order: HashMap<String, Vec<OrderRefund>> = HashMap::new();
        let refunds_query_sql = format!(
            "SELECT id, order_id, payment_id, amount, currency_code, status, reason, \
                    provider_refund_id, created_at, updated_at \
             FROM order_refunds WHERE order_id IN ({order_placeholders}) \
             ORDER BY created_at ASC, rowid ASC"
        );
        let mut refunds_query = sqlx::query_as::<_, RefundRow>(&refunds_query_sql);
        for id in &ids {
            refunds_query = refunds_query.bind(id);
        }
        for row in refunds_query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let order_id = row.order_id.clone();
            refunds_by_order
                .entry(order_id)
                .or_default()
                .push(row.into_domain());
        }

        let mut tax_lines_by_order: HashMap<String, Vec<OrderTaxLine>> = HashMap::new();
        let tax_lines_query_sql = format!(
            "SELECT id, order_id, order_item_id, shipping_option_id, name, rate_bps, \
                    amount, is_tax_inclusive, created_at \
             FROM order_tax_lines WHERE order_id IN ({order_placeholders}) \
             ORDER BY created_at ASC, rowid ASC"
        );
        let mut tax_lines_query = sqlx::query_as::<_, OrderTaxLineRow>(&tax_lines_query_sql);
        for id in &ids {
            tax_lines_query = tax_lines_query.bind(id);
        }
        for row in tax_lines_query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let order_id = row.order_id.clone();
            tax_lines_by_order
                .entry(order_id)
                .or_default()
                .push(row.into_domain());
        }

        let mut orders = Vec::with_capacity(rows.len());
        for row in rows {
            let id = row.id.clone();
            orders.push(row.into_order(
                items_by_order.remove(&id).unwrap_or_default(),
                tax_lines_by_order.remove(&id).unwrap_or_default(),
                adjustments_by_order.remove(&id).unwrap_or_default(),
                history_by_order.remove(&id).unwrap_or_default(),
                fulfillments_by_order.remove(&id).unwrap_or_default(),
                refunds_by_order.remove(&id).unwrap_or_default(),
            )?);
        }
        Ok(orders)
    }

    pub(super) async fn load_order(&self, id: &OrderId) -> Result<Order, AppError> {
        let row = sqlx::query_as::<_, OrderRow>(
            "SELECT id, display_number, customer_id, email, currency_code, status,
                    payment_status, fulfillment_status, refund_status, subtotal,
                    discount_total, shipping_total, tax_total, total, shipping_address,
                    billing_address, shipping_methods, metadata, region_id, sales_channel_id, \
                    created_at, updated_at, canceled_at
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

        let tax_line_rows = sqlx::query_as::<_, OrderTaxLineRow>(
            "SELECT id, order_id, order_item_id, shipping_option_id, name, rate_bps,
                    amount, is_tax_inclusive, created_at
             FROM order_tax_lines WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let tax_lines = tax_line_rows
            .into_iter()
            .map(OrderTaxLineRow::into_domain)
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
            "SELECT id, order_id, status, provider_id, provider_fulfillment_id, provider_data,
                    tracking_numbers, shipped_at, delivered_at, metadata, created_at, updated_at
             FROM order_fulfillments WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let mut fulfillments: Vec<OrderFulfillment> = fulfillment_rows
            .into_iter()
            .map(FulfillmentRow::into_domain)
            .collect();
        let item_rows = sqlx::query_as::<_, FulfillmentItemRow>(
            "SELECT fi.id, fi.fulfillment_id, fi.order_item_id, fi.variant_id, fi.quantity,
                    fi.created_at
             FROM order_fulfillment_items fi
             JOIN order_fulfillments f ON f.id = fi.fulfillment_id
             WHERE f.order_id = ?
             ORDER BY fi.created_at ASC, fi.rowid ASC",
        )
        .bind(id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        for item in item_rows.into_iter().map(FulfillmentItemRow::into_domain) {
            if let Some(fulfillment) = fulfillments
                .iter_mut()
                .find(|fulfillment| fulfillment.id == item.fulfillment_id)
            {
                fulfillment.items.push(item);
            }
        }

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

        row.into_order(
            items,
            tax_lines,
            adjustments,
            status_history,
            fulfillments,
            refunds,
        )
    }
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
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

fn json_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn json_string_list(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
}

fn parse_shipping_methods(text: &str) -> Result<Vec<OrderShippingMethod>, AppError> {
    serde_json::from_str(text)
        .map_err(|error| AppError::internal(format!("invalid shipping methods: {error}")))
}
