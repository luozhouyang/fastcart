//! Order creation: validation, idempotency and insertion.

use fastcart_infrastructure::{append_outbox_event, database_error};
use fastcart_kernel::{
    AppError, CurrencyCode, CustomerId, DomainEventEnvelope, EventId, LineItemId, Metadata,
    OrderId, ProductId, VariantId, normalize_email, now_timestamp, validate_email, validate_length,
    validate_metadata, validate_non_negative_amount, validate_quantity,
};
use serde_json::Value;

use crate::domain::{
    CreateAdjustmentInput, CreateOrderInput, CreateOrderTaxLineInput, FulfillmentStatus, Order,
    OrderShippingMethod, OrderStatus, PaymentStatus, RefundStatus,
};

use super::{IDEMPOTENCY_SCOPE, OrderService, insert_history, is_unique_violation, json_text};

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
    shipping_methods: Vec<OrderShippingMethod>,
    tax_lines: Vec<CreateOrderTaxLineInput>,
    region_id: Option<String>,
    sales_channel_id: Option<String>,
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

impl OrderService {
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
        self.create_order_with_status(input, OrderStatus::Pending)
            .await
    }

    /// Creates an admin-authored draft order.
    ///
    /// A draft is an order that has not entered the fulfillment lifecycle: it
    /// takes no stock, holds no payment and publishes `order.draft_created`
    /// instead of `order.placed`. Its lines can still be changed through the
    /// order-edit flow, and [`Self::convert_draft_order`] turns it into a
    /// pending order.
    pub async fn create_draft_order(&self, input: CreateOrderInput) -> Result<Order, AppError> {
        self.create_order_with_status(input, OrderStatus::Draft)
            .await
    }

    async fn create_order_with_status(
        &self,
        input: CreateOrderInput,
        status: OrderStatus,
    ) -> Result<Order, AppError> {
        if let Some(key) = input.idempotency_key.as_deref()
            && let Some(existing) = self.find_order_id_by_key(key).await?
        {
            return self.load_order(&existing).await;
        }

        let resolved = resolve_order(input)?;
        let mut retried = false;
        loop {
            match self.insert_order(&resolved, status).await {
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

    async fn insert_order(
        &self,
        order: &ResolvedOrder,
        status: OrderStatus,
    ) -> Result<OrderId, CreateAttemptError> {
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
                billing_address, shipping_methods, metadata, region_id,
                sales_channel_id, created_at, updated_at, canceled_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(order_id.as_str())
        .bind(display_number)
        .bind(order.customer_id.as_ref().map(|value| value.as_str()))
        .bind(&order.email)
        .bind(&order.currency_code)
        .bind(status.as_str())
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
        .bind(json_text(&order.shipping_methods)?)
        .bind(order.metadata.to_json_string())
        .bind(order.region_id.as_deref())
        .bind(order.sales_channel_id.as_deref())
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

        let mut item_ids = Vec::with_capacity(order.items.len());
        for item in &order.items {
            let item_id = LineItemId::new();
            item_ids.push(item_id.clone());
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

        for line in &order.tax_lines {
            let order_item_id = match line.item_index {
                Some(index) => Some(
                    item_ids
                        .get(index)
                        .ok_or_else(|| {
                            CreateAttemptError::Failed(AppError::validation(format!(
                                "tax line references item {index}, which the order does not have"
                            )))
                        })?
                        .as_str()
                        .to_owned(),
                ),
                None => None,
            };
            sqlx::query(
                "INSERT INTO order_tax_lines (
                    id, order_id, order_item_id, shipping_option_id, name, rate_bps,
                    amount, is_tax_inclusive, created_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(EventId::new().as_str())
            .bind(order_id.as_str())
            .bind(order_item_id.as_deref())
            .bind(line.shipping_option_id.as_deref())
            .bind(&line.name)
            .bind(line.rate_bps)
            .bind(line.amount)
            .bind(if line.is_tax_inclusive { 1 } else { 0 })
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;
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

        let history_note = if status.is_draft() {
            "draft order created"
        } else {
            "order created"
        };
        insert_history(&mut tx, &order_id, status, None, Some(history_note), now)
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
                    // Another checkout with the same key committed first.
                    // Return the winner instead of failing the caller.
                    tx.rollback()
                        .await
                        .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;
                    if let Some(existing) = self
                        .find_order_id_by_key(key)
                        .await
                        .map_err(CreateAttemptError::Failed)?
                    {
                        return Ok(existing);
                    }
                    return Err(CreateAttemptError::Failed(AppError::conflict(
                        "idempotency key was used concurrently".to_owned(),
                    )));
                }
                return Err(CreateAttemptError::Failed(database_error(error)));
            }
        }

        let event_name = if status.is_draft() {
            "order.draft_created"
        } else {
            "order.placed"
        };
        let event = DomainEventEnvelope::new(
            event_name,
            Some(self.db.store_id().clone()),
            serde_json::json!({
                "order_id": order_id.as_str(),
                "display_number": display_number,
                "email": order.email,
                "currency_code": order.currency_code,
                "total": order.total,
            }),
        );
        append_outbox_event(&mut *tx, &event)
            .await
            .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;

        tx.commit()
            .await
            .map_err(|error| CreateAttemptError::Failed(database_error(error)))?;
        Ok(order_id)
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
        // A line discounted past its own value yields a negative line total,
        // and every total derived from it — including the amount the customer
        // was charged — stops meaning anything.
        if discount_total > subtotal {
            return Err(AppError::validation(format!(
                "item discount_total {discount_total} exceeds its subtotal {subtotal}"
            )));
        }
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
    // Tax is validated against its breakdown rather than against the item
    // totals: an order can also be taxed on shipping, and a total nobody can
    // reconcile against a rate is not reportable.
    let mut computed_tax_total = 0_i64;
    for line in &input.tax_lines {
        if let Some(index) = line.item_index
            && index >= items.len()
        {
            return Err(AppError::validation(format!(
                "tax line references item {index}, which the order does not have"
            )));
        }
        if line.rate_bps < 0 {
            return Err(AppError::validation("tax rate must not be negative"));
        }
        computed_tax_total = computed_tax_total
            .checked_add(line.amount)
            .ok_or_else(|| AppError::validation("tax amount overflow"))?;
    }
    if input.tax_total != computed_tax_total {
        return Err(AppError::validation(format!(
            "order tax_total {} does not match its tax lines {computed_tax_total}",
            input.tax_total
        )));
    }
    if computed_tax > computed_tax_total {
        return Err(AppError::validation(format!(
            "item taxes {computed_tax} exceed the order tax lines {computed_tax_total}"
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
        shipping_methods: input.shipping_methods,
        tax_lines: input.tax_lines,
        region_id: input.region_id,
        sales_channel_id: input.sales_channel_id,
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
