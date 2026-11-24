//! Order edit confirmation and its settled price difference.

//! The workflow owns the cross-module idempotency boundary. Provider refunds
//! use a stable key per after-sales record, order refunds use the same key as a
//! local idempotency marker, and restock movements use a deterministic
//! reference. Replaying any step therefore never moves money or stock twice.

use fastcart_kernel::{AppError, OrderEditId, OrderId};
use fastcart_module_order::OrderEditStatus;

use super::*;
impl AfterSalesWorkflow {
    /// Confirms an order edit.
    ///
    /// The line rewrite, the totals and the stock movements are one
    /// transaction inside the order service, so this workflow step only loads
    /// the order aggregate afterwards. Money is deliberately not moved here:
    /// the edit records its `difference_total`, and
    /// [`Self::collect_order_edit_difference`] or
    /// [`Self::refund_order_edit_difference`] settles it explicitly, exactly
    /// like the claim and exchange difference flows.
    pub async fn confirm_order_edit(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
    ) -> Result<OrderEditConfirmation, AppError> {
        let order_edit = self.order.confirm_order_edit(order_id, edit_id).await?;
        let order = self.order.get(order_id).await?;
        Ok(OrderEditConfirmation { order_edit, order })
    }

    /// Makes the positive difference a confirmed order edit settled payable.
    ///
    /// A dearer edit leaves the customer owing the increase. This raises the
    /// order's collection by that difference and opens a session for it;
    /// nothing is charged. Idempotent by the edit, so a replay returns the
    /// session that already collects it.
    pub async fn collect_order_edit_difference(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
        provider: &str,
    ) -> Result<DifferenceCollectionResult, AppError> {
        let edit = self.order.get_order_edit(order_id, edit_id).await?;
        if edit.status != OrderEditStatus::Confirmed {
            return Err(AppError::conflict(
                "an order edit must be confirmed before its difference can be collected",
            ));
        }
        let difference = edit.difference_total.ok_or_else(|| {
            AppError::conflict("the confirmed order edit settled no price difference")
        })?;
        self.collect_difference(
            order_id,
            difference,
            ORDER_EDIT_DIFFERENCE_PREFIX,
            edit_id.as_str(),
            provider,
        )
        .await
    }

    /// Refunds the negative difference a confirmed order edit owes the
    /// customer.
    ///
    /// A cheaper edit leaves the merchant owing the decrease. Paying it back is
    /// the merchant's decision, so it is its own call; the amount is capped by
    /// the captured money the provider still holds and the refund is keyed by
    /// the edit.
    pub async fn refund_order_edit_difference(
        &self,
        order_id: &OrderId,
        edit_id: &OrderEditId,
    ) -> Result<DifferenceRefundResult, AppError> {
        let edit = self.order.get_order_edit(order_id, edit_id).await?;
        if edit.status != OrderEditStatus::Confirmed {
            return Err(AppError::conflict(
                "an order edit must be confirmed before its difference can be refunded",
            ));
        }
        let difference = edit.difference_total.ok_or_else(|| {
            AppError::conflict("the confirmed order edit settled no price difference")
        })?;
        self.refund_difference(
            order_id,
            difference,
            ORDER_EDIT_DIFFERENCE_PREFIX,
            edit_id.as_str(),
            edit.reason.clone(),
        )
        .await
    }

    /// Captured, refunded and outstanding money for one order.
    ///
    /// This is the read side of every difference flow: after an edit, a claim
    /// or an exchange, it answers what the customer still owes or is owed
    /// without the caller having to add up sessions and refunds.
    pub async fn payment_summary(
        &self,
        order_id: &OrderId,
    ) -> Result<OrderPaymentSummary, AppError> {
        let order = self.order.get(order_id).await?;
        let captured_total = self.payment.captured_total_for_order(order_id).await?;
        let refunded_total = self.payment.refunded_total_for_order(order_id).await?;
        let net_paid = captured_total
            .checked_sub(refunded_total)
            .ok_or_else(|| AppError::validation("payment totals overflow"))?;
        let outstanding = order
            .total
            .checked_sub(net_paid)
            .ok_or_else(|| AppError::validation("outstanding amount overflow"))?;
        Ok(OrderPaymentSummary {
            order_id: order.id,
            currency_code: order.currency_code,
            total: order.total,
            captured_total,
            refunded_total,
            net_paid,
            outstanding,
        })
    }
}
