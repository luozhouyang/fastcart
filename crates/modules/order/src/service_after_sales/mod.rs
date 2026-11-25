//! Application services for returns, claims, exchanges and order edits.
//!
//! Every mutation validates quantities against the current order and
//! post-purchase records, then performs an idempotent, status-guarded
//! transition. Returns, claims and exchanges reference `order_items` rows
//! without rewriting them; a confirmed order edit materializes its changes on
//! them (see `confirm_order_edit`), so the order totals and the lines always
//! describe the same purchase. That same transaction moves the stock the edit
//! changes and records the price difference it settles; collecting or refunding
//! that difference stays an explicit, idempotent call.

mod claims;
mod edits;
mod exchanges;
mod fulfillments;
mod returns;

use std::collections::HashMap;

use fastcart_infrastructure::{append_outbox_event, database_error};
use fastcart_kernel::{
    AppError, DomainEventEnvelope, LineItemId, OrderId, StoreId, Timestamp,
    validate_non_negative_amount,
};
use sqlx::SqliteConnection;

use crate::after_sales::{
    ClaimStatus, ExchangeItemDirection, ExchangeStatus, ReplacementUnitPrice, ReturnStatus,
};

#[derive(sqlx::FromRow)]
struct ExistingOrderItem {
    product_id: Option<String>,
    variant_id: Option<String>,
    product_title: String,
    variant_title: Option<String>,
    sku: Option<String>,
    unit_price: i64,
    quantity: i64,
    requires_shipping: bool,
    is_tax_inclusive: bool,
}

/// Loads one order line from the order's own items.
async fn fetch_order_item(
    executor: &mut SqliteConnection,
    order_id: &OrderId,
    order_item_id: &LineItemId,
) -> Result<ExistingOrderItem, AppError> {
    sqlx::query_as::<_, ExistingOrderItem>(
        "SELECT product_id, variant_id, product_title, variant_title, sku, unit_price, quantity, \
                requires_shipping, is_tax_inclusive
         FROM order_items WHERE id = ? AND order_id = ?",
    )
    .bind(order_item_id.as_str())
    .bind(order_id.as_str())
    .fetch_optional(executor)
    .await
    .map_err(database_error)?
    .ok_or_else(|| AppError::not_found("order item", order_item_id))
}

/// The after-sales record a settled price difference belongs to.
///
/// It names the adjustment code and the deterministic adjustment id, so the
/// difference of one record can never be booked twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DifferenceSource {
    Exchange,
    Claim,
}

impl DifferenceSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Exchange => "exchange",
            Self::Claim => "claim",
        }
    }
}

/// Settles the price difference of one accepted after-sales record.
///
/// `replacement_lines` are the record's `(replaced order line, quantity)`
/// pairs. The difference is summed over the lines the caller priced as
/// `(replacement unit price - replaced unit price) * quantity`, where the
/// replaced unit price is the *current* `order_items` row: a confirmed order
/// edit rewrites the line an after-sales record points at, so the row is read
/// here rather than snapshotted when the record was created.
///
/// The total is stored on the record, and when it is not zero it is booked as
/// a single signed order-level adjustment, which keeps
/// `orders.total == sum(order_items.total) + shipping_total + adjustments`.
/// Both writes are guarded, so replaying an acceptance can never settle the
/// same difference twice.
async fn settle_price_difference(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    order_id: &OrderId,
    source: DifferenceSource,
    source_id: &str,
    replacement_lines: &[(LineItemId, i64)],
    replacement_prices: &[ReplacementUnitPrice],
    now: Timestamp,
) -> Result<i64, AppError> {
    let mut priced: HashMap<&str, i64> = HashMap::with_capacity(replacement_prices.len());
    for price in replacement_prices {
        validate_non_negative_amount(price.unit_price, "replacement unit price")?;
        if priced
            .insert(price.order_item_id.as_str(), price.unit_price)
            .is_some()
        {
            return Err(AppError::validation(
                "replacement prices must not contain duplicate order items",
            ));
        }
        if !replacement_lines
            .iter()
            .any(|(line, _)| line == &price.order_item_id)
        {
            return Err(AppError::validation(format!(
                "{} {source_id} does not replace order item {}",
                source.as_str(),
                price.order_item_id
            )));
        }
    }

    let mut difference_total = 0_i64;
    for (order_item_id, quantity) in replacement_lines {
        let Some(replacement_price) = priced.get(order_item_id.as_str()) else {
            // The caller resolved no price for this replacement, so there is no
            // known difference to settle for it.
            continue;
        };
        let replaced = fetch_order_item(&mut *tx, order_id, order_item_id).await?;
        let difference = replacement_price
            .checked_sub(replaced.unit_price)
            .and_then(|value| value.checked_mul(*quantity))
            .ok_or_else(|| AppError::validation("price difference overflow"))?;
        // The replacement line carries its own share next to the total it adds
        // up to, so the money can be traced back to the line that moved it. The
        // write is as replay-proof as the total's: the same line settles the
        // same difference, and only the first acceptance gets this far.
        match source {
            DifferenceSource::Exchange => sqlx::query(
                "UPDATE order_exchange_items SET difference = ?, updated_at = ?
                 WHERE exchange_id = ? AND direction = 'outbound' AND order_item_id = ?",
            )
            .bind(difference)
            .bind(now)
            .bind(source_id)
            .bind(order_item_id.as_str()),
            DifferenceSource::Claim => sqlx::query(
                "UPDATE order_claim_items SET difference = ?, updated_at = ?
                 WHERE claim_id = ? AND order_item_id = ?",
            )
            .bind(difference)
            .bind(now)
            .bind(source_id)
            .bind(order_item_id.as_str()),
        }
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
        difference_total = difference_total
            .checked_add(difference)
            .ok_or_else(|| AppError::validation("price difference overflow"))?;
    }

    // The conditional write claims the settlement for this record. The
    // deterministic adjustment id below is the second guard: even a call that
    // reached the insert twice could not book a second row.
    let source_label = source.as_str();
    let claimed = match source {
        DifferenceSource::Exchange => sqlx::query(
            "UPDATE order_exchanges SET difference_total = ?
             WHERE id = ? AND order_id = ? AND difference_total IS NULL",
        ),
        DifferenceSource::Claim => sqlx::query(
            "UPDATE order_claims SET difference_total = ?
             WHERE id = ? AND order_id = ? AND difference_total IS NULL",
        ),
    }
    .bind(difference_total)
    .bind(source_id)
    .bind(order_id.as_str())
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    if claimed.rows_affected() == 0 {
        return Err(AppError::conflict(format!(
            "{source_label} {source_id} already settled a price difference"
        )));
    }
    if difference_total == 0 {
        // The replacement and the replaced line are priced alike: the
        // difference is settled, but an adjustment that moves nothing would
        // only be noise in the order's books.
        return Ok(0);
    }

    let (status, current_total) =
        sqlx::query_as::<_, (String, i64)>("SELECT status, total FROM orders WHERE id = ?")
            .bind(order_id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("order", order_id))?;
    let next_total = current_total
        .checked_add(difference_total)
        .ok_or_else(|| AppError::validation("order total overflow"))?;
    if next_total < 0 {
        return Err(AppError::conflict(format!(
            "a price difference of {difference_total} would make the order total negative"
        )));
    }
    let updated =
        sqlx::query("UPDATE orders SET total = ?, updated_at = ? WHERE id = ? AND status = ?")
            .bind(next_total)
            .bind(now)
            .bind(order_id.as_str())
            .bind(&status)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
    if updated.rows_affected() == 0 {
        return Err(AppError::conflict("order status changed concurrently"));
    }
    sqlx::query(
        "INSERT INTO order_adjustments (id, order_id, code, amount, description, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(format!("{source_label}-difference:{source_id}"))
    .bind(order_id.as_str())
    .bind(format!("{source_label}_difference"))
    .bind(difference_total)
    .bind(format!(
        "replacement price difference of {source_label} {source_id}"
    ))
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;

    Ok(difference_total)
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

/// Trims an optional text field and treats an empty value as absent.
fn normalize_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

async fn append_after_sales_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    store_id: &StoreId,
    event_type: &str,
    payload: serde_json::Value,
) -> Result<(), AppError> {
    let event = DomainEventEnvelope::new(event_type, Some(store_id.clone()), payload);
    append_outbox_event(&mut **tx, &event)
        .await
        .map_err(database_error)
}

#[cfg(test)]
mod allocation_tests;
#[cfg(test)]
mod tests;
