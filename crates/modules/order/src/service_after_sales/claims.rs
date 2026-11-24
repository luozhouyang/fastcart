//! Claim application services: create, accept, cancel and refund.

use std::collections::HashSet;

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, ClaimId, EventId, LineItemId, OrderId, RefundId, VariantId, now_timestamp,
    validate_metadata, validate_quantity,
};

use crate::after_sales::{
    ClaimItemStatus, ClaimItemType, ClaimStatus, CreateClaimInput, OrderClaim, OrderClaimItem,
    ReplacementUnitPrice,
};
use crate::domain::OrderStatus;
use crate::service::OrderService;

use super::{
    DifferenceSource, append_after_sales_event, fetch_order_item, normalize_text,
    settle_price_difference, sum_active_claims, sum_active_exchange_inbound, sum_active_returns,
};

#[derive(sqlx::FromRow)]
struct ClaimRow {
    id: String,
    order_id: String,
    status: String,
    reason: Option<String>,
    refund_id: Option<String>,
    difference_total: Option<i64>,
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
    difference: Option<i64>,
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
            difference: self.difference,
            status: ClaimItemStatus::from_stored(&self.status)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}
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
            let exchanged = sum_active_exchange_inbound(&mut tx, &item.order_item_id).await?;
            let available = existing.quantity - returned - claimed - exchanged;
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

        self.append_audit(
            &mut tx,
            "order.claim_created",
            "order_claim",
            claim_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str()}),
        )
        .await?;
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
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE order_claims SET refund_id = ?, updated_at = ?
             WHERE id = ? AND order_id = ? AND (refund_id IS NULL OR refund_id = ?)",
        )
        .bind(refund_id.as_str())
        .bind(now)
        .bind(claim_id.as_str())
        .bind(order_id.as_str())
        .bind(refund_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict("claim refund link changed concurrently"));
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
            "order.claim_refunded",
            "order_claim",
            claim_id.as_str(),
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
            "order.claim_refunded",
            serde_json::json!({
                "order_id": order_id.as_str(),
                "claim_id": claim_id.as_str(),
                "refund_id": refund_id.as_str(),
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_claim(order_id, claim_id).await
    }

    /// Lists claims for an order.
    pub async fn list_claims(&self, order_id: &OrderId) -> Result<Vec<OrderClaim>, AppError> {
        self.get(order_id).await?;
        let rows = sqlx::query_as::<_, ClaimRow>(
            "SELECT id, order_id, status, reason, refund_id, difference_total, metadata,
                    created_at, updated_at, accepted_at, canceled_at
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

    /// Accepts a requested claim and settles the price difference of its
    /// replacement lines.
    ///
    /// `replacement_prices` carries the caller-resolved unit price of each
    /// replacement, keyed by the historical order line it replaces, because
    /// the order module does not price catalog variants. The difference
    /// `(replacement unit price - current line unit price) * quantity` is
    /// summed over the claim's replace lines and recorded once as a signed
    /// order adjustment; a line the caller could not price settles nothing.
    /// Accepting an already accepted claim returns it unchanged, so a replay
    /// never books the difference twice.
    pub async fn accept_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
        replacement_prices: &[ReplacementUnitPrice],
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
        let replacement_lines = current
            .items
            .iter()
            .filter(|item| item.item_type == ClaimItemType::Replace)
            .map(|item| (item.order_item_id.clone(), item.quantity))
            .collect::<Vec<_>>();
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
        let difference_total = settle_price_difference(
            &mut tx,
            order_id,
            DifferenceSource::Claim,
            claim_id.as_str(),
            &replacement_lines,
            replacement_prices,
            now,
        )
        .await?;
        self.append_audit(
            &mut tx,
            "order.claim_accepted",
            "order_claim",
            claim_id.as_str(),
            serde_json::json!({
                "order_id": order_id.as_str(),
                "difference_total": difference_total,
            }),
        )
        .await?;
        append_after_sales_event(
            &mut tx,
            self.database().store_id(),
            "order.claim_accepted",
            serde_json::json!({
                "order_id": order_id.as_str(),
                "claim_id": claim_id.as_str(),
                "status": ClaimStatus::Accepted.as_str(),
                "difference_total": difference_total,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_claim(order_id, claim_id).await
    }

    /// Cancels a requested claim.
    pub async fn cancel_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
    ) -> Result<OrderClaim, AppError> {
        self.cancel_claim_with_reason(order_id, claim_id, None)
            .await
    }

    /// Cancels a claim and records the caller-supplied reason in the audit log.
    pub async fn cancel_claim_with_reason(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
        reason: Option<&str>,
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
        self.append_audit(
            &mut tx,
            "order.claim_canceled",
            "order_claim",
            claim_id.as_str(),
            serde_json::json!({"order_id": order_id.as_str(), "reason": reason}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.load_claim(order_id, claim_id).await
    }

    pub(super) async fn load_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
    ) -> Result<OrderClaim, AppError> {
        let row = sqlx::query_as::<_, ClaimRow>(
            "SELECT id, order_id, status, reason, refund_id, difference_total, metadata,
                    created_at, updated_at, accepted_at, canceled_at
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
                    replacement_variant_id, reason, difference, status, created_at, updated_at
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
            difference_total: row.difference_total,
            metadata: fastcart_kernel::Metadata::from_json_str(&row.metadata),
            created_at: row.created_at,
            updated_at: row.updated_at,
            accepted_at: row.accepted_at,
            canceled_at: row.canceled_at,
            items,
        })
    }
}
