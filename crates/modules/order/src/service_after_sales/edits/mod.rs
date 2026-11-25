//! Order edit application services and stock/fulfillment helpers.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, EventId, LineItemId, OrderEditId, OrderId, ProductId, VariantId, now_timestamp,
    validate_metadata, validate_quantity,
};
use fastcart_module_inventory::{InventoryService, OrderEditStockLine, order_edit_stock_reference};
use fastcart_module_store::StoreService;
use sqlx::SqliteConnection;

use crate::after_sales::{
    CreateOrderEditInput, OrderEdit, OrderEditAction, OrderEditItem, OrderEditItemStatus,
    OrderEditStatus,
};
use crate::domain::{FulfillmentStatus, OrderStatus};
use crate::service::OrderService;

mod confirm;

use super::{
    append_after_sales_event, fetch_order_item, normalize_text, sum_active_claims,
    sum_active_exchange_inbound, sum_active_returns,
};

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
    difference_total: Option<i64>,
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
    requires_shipping: i64,
    is_tax_inclusive: i64,
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
            requires_shipping: self.requires_shipping != 0,
            is_tax_inclusive: self.is_tax_inclusive != 0,
            status: OrderEditItemStatus::from_stored(&self.status)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}
#[derive(sqlx::FromRow)]
struct ExistingLineAmounts {
    unit_price: i64,
    quantity: i64,
    discount_total: i64,
    tax_total: i64,
    is_tax_inclusive: i64,
}

/// The summed amounts of an order's materialized lines.
#[derive(sqlx::FromRow)]
struct LineTotalsRow {
    subtotal: i64,
    discount_total: i64,
    total: i64,
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
    requires_shipping: bool,
    is_tax_inclusive: bool,
    delta: i64,
}
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
        // Added lines follow the store's pricing convention unless the caller
        // states otherwise, exactly like a draft line: a store that quotes
        // gross prices must not have tax added on top of a new line.
        let default_is_tax_inclusive = StoreService::new(self.database().clone())
            .prices_include_tax()
            .await?;

        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let mut seen = HashSet::new();
        let mut resolved = Vec::with_capacity(input.items.len());
        for item in &input.items {
            let line = resolve_edit_item(&mut tx, order_id, item, default_is_tax_inclusive).await?;
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
                     after_quantity, requires_shipping, is_tax_inclusive, status,
                     created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            .bind(i64::from(item.requires_shipping))
            .bind(i64::from(item.is_tax_inclusive))
            .bind(OrderEditItemStatus::Draft.as_str())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        self.append_audit(
            &mut tx,
            "order.edit_created",
            "order_edit",
            edit_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str()}),
        )
        .await?;
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
                    before_total, after_total, difference_total, created_at, updated_at,
                    confirmed_at, canceled_at
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

    /// Cancels a draft order edit without touching the order.
    pub async fn cancel_order_edit(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
    ) -> Result<OrderEdit, AppError> {
        self.cancel_order_edit_with_reason(order_id, edit_id, None)
            .await
    }

    /// Cancels a draft order edit and records the supplied reason in the audit log.
    pub async fn cancel_order_edit_with_reason(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
        reason: Option<&str>,
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
        self.append_audit(
            &mut tx,
            "order.edit_canceled",
            "order_edit",
            edit_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str(), "reason": reason}),
        )
        .await?;
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
                    before_total, after_total, difference_total, created_at, updated_at,
                    confirmed_at, canceled_at
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
                    after_quantity, requires_shipping, is_tax_inclusive, status,
                    created_at, updated_at
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
            difference_total: row.difference_total,
            created_at: row.created_at,
            updated_at: row.updated_at,
            confirmed_at: row.confirmed_at,
            canceled_at: row.canceled_at,
            items,
        })
    }
}
/// The stock movements a confirmed edit applies.
///
/// Each line contributes one movement with a reference derived from the edit
/// and the edit line, so replaying a confirmation is a no-op for the inventory
/// service. Lines that do not take stock, have no variant, or do not change the
/// quantity contribute nothing; a line whose quantity grows consumes the
/// difference and one that shrinks returns it.
fn edit_stock_lines(edit: &OrderEdit) -> Vec<OrderEditStockLine> {
    let mut lines = Vec::with_capacity(edit.items.len());
    for item in &edit.items {
        if !item.requires_shipping {
            continue;
        }
        let Some(variant_id) = item.variant_id.clone() else {
            continue;
        };
        let Some(delta) = item.after_quantity.checked_sub(item.before_quantity) else {
            continue;
        };
        if delta == 0 {
            continue;
        }
        lines.push(OrderEditStockLine::new(
            variant_id,
            delta,
            order_edit_stock_reference(edit.order_id.as_str(), edit.id.as_str(), item.id.as_str()),
        ));
    }
    lines
}

/// Rejects an edit that would take back units already shipped from a line.
///
/// `fulfill` clamps a requested quantity to what is left, so shrinking a line
/// below its shipped quantity does not fail: it silently makes the already
/// written `order_fulfillment_items` describe more units than the line holds.
/// Cancelling an edit is not a way out either — the units are gone — so the
/// merchant has to record a return instead.
///
/// Only the lines the edit shrinks are checked. Added lines have no shipments
/// and unchanged lines keep the quantity their shipments were validated
/// against.
async fn ensure_fulfillment_headroom(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    order_id: &OrderId,
    edit: &OrderEdit,
) -> Result<(), AppError> {
    for item in &edit.items {
        let Some(order_item_id) = item.order_item_id.as_ref() else {
            continue;
        };
        if item.after_quantity >= item.before_quantity {
            continue;
        }
        // Re-read the shipped quantity inside the transaction instead of
        // trusting the number the edit was prepared with: a fulfillment may
        // have been recorded between prepare and confirm.
        let shipped = shipped_quantity(&mut *tx, order_id, order_item_id).await?;
        if item.after_quantity < shipped {
            return Err(AppError::conflict(format!(
                "cannot reduce item {order_item_id} to {}: {shipped} units are already shipped",
                item.after_quantity
            )));
        }
    }
    Ok(())
}

/// Units of one order line already recorded as shipped.
async fn shipped_quantity(
    executor: &mut SqliteConnection,
    order_id: &OrderId,
    order_item_id: &LineItemId,
) -> Result<i64, AppError> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(fi.quantity), 0)
         FROM order_fulfillment_items fi
         JOIN order_fulfillments f ON f.id = fi.fulfillment_id
         WHERE f.order_id = ? AND fi.order_item_id = ?",
    )
    .bind(order_id.as_str())
    .bind(order_item_id.as_str())
    .fetch_one(executor)
    .await
    .map_err(database_error)
}

/// The fulfillment state the order's shippable lines and shipments now
/// describe.
///
/// `fulfill` derives the state from the units it just shipped, but an edit
/// moves the line quantities underneath the existing shipments. Without this
/// derivation an order could stay `fulfilled` while an added line is unshipped,
/// and `fulfill` would then refuse to ship the units the customer paid for.
async fn fulfillment_state_after_edit(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    order_id: &OrderId,
) -> Result<FulfillmentStatus, AppError> {
    let lines = sqlx::query_as::<_, (i64, i64)>(
        "SELECT oi.quantity,
                COALESCE((
                    SELECT SUM(fi.quantity)
                    FROM order_fulfillment_items fi
                    JOIN order_fulfillments f ON f.id = fi.fulfillment_id
                    WHERE f.order_id = oi.order_id AND fi.order_item_id = oi.id
                ), 0)
         FROM order_items oi
         WHERE oi.order_id = ? AND oi.requires_shipping = 1",
    )
    .bind(order_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(database_error)?;
    let mut shippable = false;
    let mut shipped = false;
    let mut complete = true;
    for (quantity, fulfilled) in lines {
        if quantity <= 0 {
            continue;
        }
        shippable = true;
        shipped |= fulfilled > 0;
        complete &= fulfilled >= quantity;
    }
    Ok(if !shippable || !shipped {
        FulfillmentStatus::NotFulfilled
    } else if complete {
        FulfillmentStatus::Fulfilled
    } else {
        FulfillmentStatus::PartiallyFulfilled
    })
}

/// Rejects an edit that would leave a line holding fewer units than the
/// returns, claims and exchanges already committed against it.
///
/// Those records keep referencing the same `order_items` row, so shrinking the
/// line below them would make after-sales documents describe more units than
/// the order contains.
async fn ensure_after_sales_headroom(
    executor: &mut SqliteConnection,
    order_item_id: &LineItemId,
    after_quantity: i64,
) -> Result<(), AppError> {
    let committed = sum_active_returns(executor, order_item_id).await?
        + sum_active_claims(executor, order_item_id).await?
        + sum_active_exchange_inbound(executor, order_item_id).await?;
    if after_quantity < committed {
        return Err(AppError::conflict(format!(
            "cannot reduce item {order_item_id} to {after_quantity}: \
             {committed} units are already committed to after-sales records"
        )));
    }
    Ok(())
}

async fn resolve_edit_item(
    executor: &mut SqliteConnection,
    order_id: &OrderId,
    input: &crate::after_sales::OrderEditItemInput,
    default_is_tax_inclusive: bool,
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
                requires_shipping: input.requires_shipping.unwrap_or(true),
                is_tax_inclusive: input.is_tax_inclusive.unwrap_or(default_is_tax_inclusive),
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
            ensure_after_sales_headroom(executor, order_item_id, input.after_quantity).await?;
            // Fail while the edit is still a draft when the merchant tries to
            // take back units that already shipped; `confirm_order_edit`
            // re-checks under its own transaction because a fulfillment may
            // land between prepare and confirm.
            let shipped = shipped_quantity(executor, order_id, order_item_id).await?;
            if input.after_quantity < shipped {
                return Err(AppError::conflict(format!(
                    "cannot reduce item {order_item_id} to {}: {shipped} units are already shipped",
                    input.after_quantity
                )));
            }
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
                requires_shipping: existing.requires_shipping,
                is_tax_inclusive: existing.is_tax_inclusive,
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
/// A reduction only shrinks order-level allocations; an increase (`retained >
/// original`) leaves them untouched because additions are priced explicitly by
/// the edit line rather than diluting the existing allocation. When the base is
/// unchanged or unknown, the amount is returned untouched.
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

/// Scales a line amount with its quantity, in either direction.
///
/// A grown line keeps its effective rate, so its discount and tax grow with
/// it; a shrunk line loses the share represented by the removed units.
/// Flooring matches [`scale_amount`], and a zero original quantity leaves the
/// amount untouched because there is no rate to extrapolate from.
fn scale_line_amount(amount: i64, quantity: i64, original: i64) -> Result<i64, AppError> {
    if original <= 0 || quantity == original {
        return Ok(amount);
    }
    let quantity = quantity.max(0);
    let numerator = i128::from(amount)
        .checked_mul(i128::from(quantity))
        .ok_or_else(|| AppError::validation("allocation overflow"))?;
    let scaled = numerator.div_euclid(i128::from(original));
    i64::try_from(scaled).map_err(|_| AppError::validation("allocation overflow"))
}
