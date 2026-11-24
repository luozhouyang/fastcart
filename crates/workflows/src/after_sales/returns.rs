//! Return refunds and receiving.

//! The workflow owns the cross-module idempotency boundary. Provider refunds
//! use a stable key per after-sales record, order refunds use the same key as a
//! local idempotency marker, and restock movements use a deterministic
//! reference. Replaying any step therefore never moves money or stock twice.

use fastcart_kernel::{AppError, OrderId, ReturnId};
use fastcart_module_inventory::{RestockAudit, ReturnRestockLine};
use fastcart_module_order::{ReceiveReturnInput, RefundInput, ReturnStatus};

use super::*;
impl AfterSalesWorkflow {
    /// Refunds the requested quantities of a return.
    ///
    /// The refundable amount is proportional to the current order line totals
    /// (a confirmed order edit rewrites the lines it touches) and capped by the
    /// order total minus refunds already recorded.
    /// Repeating the call returns the existing link without moving money
    /// again.
    pub async fn refund_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        reason: Option<String>,
    ) -> Result<ReturnRefundResult, AppError> {
        let order = self.order.get(order_id).await?;
        let order_return = self.order.get_return(order_id, return_id).await?;
        if order_return.status == ReturnStatus::Canceled {
            return Err(AppError::conflict(
                "cannot refund a canceled return".to_owned(),
            ));
        }
        let key = format!("{RETURN_REFUND_PREFIX}:{}", return_id.as_str());

        if let Some(existing) = order_return.refund_id.clone() {
            let order_refund = order
                .refunds
                .iter()
                .find(|refund| refund.id == existing)
                .cloned();
            let provider_refund = self.provider_refund_by_key(order_id, &key).await;
            return Ok(ReturnRefundResult {
                order,
                order_return,
                order_refund,
                provider_refund,
            });
        }

        // A previous attempt may have recorded the order refund and crashed
        // before linking it. Finish that link instead of refunding again.
        let local = local_key(&key);
        if let Some(existing) = self.order.refund_id_by_idempotency_key(&local).await? {
            let order_return = self
                .order
                .attach_return_refund(order_id, return_id, &existing)
                .await?;
            let order = self.order.get(order_id).await?;
            let order_refund = order
                .refunds
                .iter()
                .find(|refund| refund.id == existing)
                .cloned();
            let provider_refund = self.provider_refund_by_key(order_id, &key).await;
            return Ok(ReturnRefundResult {
                order,
                order_return,
                order_refund,
                provider_refund,
            });
        }

        // Only goods that actually arrived can be refunded. A written-off unit
        // is received but was not resold, so it never contributes to a refund.
        if !matches!(
            order_return.status,
            ReturnStatus::Received | ReturnStatus::PartiallyReceived
        ) {
            return Err(AppError::conflict(format!(
                "return in state {} cannot be refunded before goods are received",
                order_return.status.as_str()
            )));
        }
        let requested = order_return.items.iter().filter_map(|item| {
            let refundable = item
                .received_quantity
                .saturating_sub(item.written_off_quantity)
                .max(0);
            (refundable > 0).then(|| (item.order_item_id.clone(), refundable))
        });
        let refundable = refundable_amount(&order, requested)?;
        let amount = cap_refund(&order, refundable);
        if amount <= 0 {
            return Ok(ReturnRefundResult {
                order,
                order_return,
                order_refund: None,
                provider_refund: None,
            });
        }

        let payment = self.captured_payment(order_id).await?;
        let reason = reason.or_else(|| order_return.reason.clone());
        let provider_refund = self
            .payment
            .refund_with_idempotency(
                payment.id.as_str(),
                amount,
                reason.clone(),
                Some(key.clone()),
            )
            .await?;

        let (order, order_refund_id) = self
            .order
            .add_refund_idempotent(
                order_id,
                RefundInput {
                    payment_id: payment.id.clone(),
                    amount,
                    currency: order.currency_code.clone(),
                    reason: reason.clone(),
                },
                &local,
            )
            .await?;
        let order_return = self
            .order
            .attach_return_refund(order_id, return_id, &order_refund_id)
            .await?;
        let order_refund = order
            .refunds
            .iter()
            .find(|refund| refund.id == order_refund_id)
            .cloned();

        Ok(ReturnRefundResult {
            order,
            order_return,
            order_refund,
            provider_refund: Some(provider_refund),
        })
    }

    /// Receives returned goods and restocks only the newly received units.
    ///
    /// Receiving an already completed return restocks nothing. Repeated or
    /// concurrent calls are safe: the inventory movement reference is derived
    /// from the return, line and cumulative received quantity.
    pub async fn receive_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        input: ReceiveReturnInput,
    ) -> Result<ReceiveReturnResult, AppError> {
        let before = self.order.get_return(order_id, return_id).await?;
        let order_return = self
            .order
            .receive_return(order_id, return_id, input)
            .await?;
        let order = self.order.get(order_id).await?;

        let mut lines = Vec::new();
        for item in &order_return.items {
            // Target the full cumulative received quantity for each line. The
            // inventory service reconciles this against movements already
            // recorded, so a retry after a failed restock still restocks.
            if item.received_quantity <= 0 {
                continue;
            }
            let Some(order_item) = order
                .items
                .iter()
                .find(|line| line.id == item.order_item_id)
            else {
                continue;
            };
            if !order_item.requires_shipping {
                continue;
            }
            let Some(variant_id) = order_item.variant_id.clone() else {
                continue;
            };
            lines.push(ReturnRestockLine::new(
                item.order_item_id.clone(),
                variant_id,
                item.received_quantity,
                item.received_quantity,
            ));
        }

        let changed = before.status != order_return.status
            || order_return.items.iter().any(|item| {
                before
                    .items
                    .iter()
                    .find(|previous| previous.order_item_id == item.order_item_id)
                    .is_none_or(|previous| previous.received_quantity != item.received_quantity)
            });
        let restocked_units = if changed {
            self.inventory
                .restock_return_audited(
                    order_id,
                    return_id,
                    &lines,
                    RestockAudit {
                        action: "order.return_received".to_owned(),
                        resource_type: "order_return".to_owned(),
                        resource_id: return_id.as_str().to_owned(),
                        data: serde_json::json!({
                            "order_id": order_id.as_str(),
                            "status": order_return.status.as_str(),
                        }),
                    },
                )
                .await?
        } else if !lines.is_empty() {
            // The receive transaction may have committed before a previous
            // restock attempt failed. Reconcile the missing movements without
            // writing a duplicate receive audit.
            self.inventory
                .restock_return(order_id, return_id, &lines)
                .await?
        } else {
            0
        };

        Ok(ReceiveReturnResult {
            order_return,
            restocked_units,
        })
    }
}
