//! After-sales orchestration: refunds for returns and claims, plus inventory
//! restocking when returned goods are received.
//!
//! The workflow owns the cross-module idempotency boundary. Provider refunds
//! use a stable key per after-sales record, order refunds use the same key as a
//! local idempotency marker, and restock movements use a deterministic
//! reference. Replaying any step therefore never moves money or stock twice.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, ClaimId, LineItemId, OrderId, Pagination, ReturnId};
use fastcart_module_inventory::{InventoryService, ReturnRestockLine};
use fastcart_module_order::{
    ClaimItemType, ClaimStatus, Order, OrderClaim, OrderRefund, OrderReturn, OrderService,
    ReceiveReturnInput, RefundInput, ReturnStatus,
};
use fastcart_module_payment::{ListPaymentsFilter, Payment, PaymentService, PaymentStatus, Refund};

/// Prefix for provider and local idempotency keys of return refunds.
const RETURN_REFUND_PREFIX: &str = "return-refund";
/// Prefix for provider and local idempotency keys of claim refunds.
const CLAIM_REFUND_PREFIX: &str = "claim-refund";

/// Result of refunding a return.
#[derive(Clone, Debug)]
pub struct ReturnRefundResult {
    /// Order aggregate after the refund was recorded.
    pub order: Order,
    /// Return with its `refund_id` link populated.
    pub order_return: OrderReturn,
    /// Order refund row created for this return, when one was needed.
    pub order_refund: Option<OrderRefund>,
    /// Provider-side refund, when a payment refund was attempted.
    pub provider_refund: Option<Refund>,
}

/// Result of refunding an accepted claim.
#[derive(Clone, Debug)]
pub struct ClaimRefundResult {
    /// Order aggregate after the refund was recorded.
    pub order: Order,
    /// Accepted claim with its `refund_id` link populated.
    pub claim: OrderClaim,
    /// Order refund row created for the claim's refund-type lines.
    pub order_refund: Option<OrderRefund>,
    /// Provider-side refund, when a payment refund was attempted.
    pub provider_refund: Option<Refund>,
}

/// Result of receiving returned goods and restocking them.
#[derive(Clone, Debug)]
pub struct ReceiveReturnResult {
    /// Return after the receive transition.
    pub order_return: OrderReturn,
    /// Units actually placed back into inventory by this call.
    pub restocked_units: i64,
}

/// Orchestrates order, payment and inventory services for after-sales flows.
pub struct AfterSalesWorkflow {
    order: OrderService,
    payment: PaymentService,
    inventory: InventoryService,
}

impl AfterSalesWorkflow {
    /// Creates a workflow bound to one store.
    ///
    /// The payment service must be constructed with the provider registry used
    /// by the process, exactly like [`crate::CheckoutWorkflow`].
    #[must_use]
    pub fn new(db: StoreDatabase, payment: PaymentService) -> Self {
        Self {
            order: OrderService::new(db.clone()),
            payment,
            inventory: InventoryService::new(db),
        }
    }

    /// Refunds the requested quantities of a return.
    ///
    /// The refundable amount is proportional to the immutable order line
    /// totals and capped by the order total minus refunds already recorded.
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

        let requested = order_return
            .items
            .iter()
            .map(|item| (item.order_item_id.clone(), item.quantity));
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

    /// Accepts a claim when needed and refunds its refund-type lines.
    ///
    /// Claims without refund-type lines are left accepted with no refund.
    /// Replacement lines are intentionally not fulfilled here: a later flow
    /// should create outbound fulfillments for [`ClaimItemType::Replace`]
    /// lines after the refund settles.
    pub async fn refund_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
        reason: Option<String>,
    ) -> Result<ClaimRefundResult, AppError> {
        let order = self.order.get(order_id).await?;
        let mut claim = self.order.get_claim(order_id, claim_id).await?;
        if claim.status == ClaimStatus::Canceled {
            return Err(AppError::conflict(
                "cannot refund a canceled claim".to_owned(),
            ));
        }
        if claim.status == ClaimStatus::Requested {
            claim = self.order.accept_claim(order_id, claim_id).await?;
        }
        let key = format!("{CLAIM_REFUND_PREFIX}:{}", claim_id.as_str());

        if let Some(existing) = claim.refund_id.clone() {
            let order_refund = order
                .refunds
                .iter()
                .find(|refund| refund.id == existing)
                .cloned();
            let provider_refund = self.provider_refund_by_key(order_id, &key).await;
            return Ok(ClaimRefundResult {
                order,
                claim,
                order_refund,
                provider_refund,
            });
        }

        // Complete a link left behind by an interrupted earlier attempt.
        let local = local_key(&key);
        if let Some(existing) = self.order.refund_id_by_idempotency_key(&local).await? {
            let claim = self
                .order
                .attach_claim_refund(order_id, claim_id, &existing)
                .await?;
            let order = self.order.get(order_id).await?;
            let order_refund = order
                .refunds
                .iter()
                .find(|refund| refund.id == existing)
                .cloned();
            let provider_refund = self.provider_refund_by_key(order_id, &key).await;
            return Ok(ClaimRefundResult {
                order,
                claim,
                order_refund,
                provider_refund,
            });
        }

        let requested = claim
            .items
            .iter()
            .filter(|item| item.item_type == ClaimItemType::Refund)
            .map(|item| (item.order_item_id.clone(), item.quantity));
        let refundable = refundable_amount(&order, requested)?;
        let amount = cap_refund(&order, refundable);
        if amount <= 0 {
            return Ok(ClaimRefundResult {
                order,
                claim,
                order_refund: None,
                provider_refund: None,
            });
        }

        let payment = self.captured_payment(order_id).await?;
        let reason = reason.or_else(|| claim.reason.clone());
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
        let claim = self
            .order
            .attach_claim_refund(order_id, claim_id, &order_refund_id)
            .await?;
        let order_refund = order
            .refunds
            .iter()
            .find(|refund| refund.id == order_refund_id)
            .cloned();

        Ok(ClaimRefundResult {
            order,
            claim,
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
            let previously_received = before
                .items
                .iter()
                .find(|previous| previous.order_item_id == item.order_item_id)
                .map_or(0, |previous| previous.received_quantity);
            let newly_received = item.received_quantity - previously_received;
            if newly_received <= 0 {
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
                newly_received,
                item.received_quantity,
            ));
        }

        let restocked_units = if lines.is_empty() {
            0
        } else {
            self.inventory
                .restock_return(order_id, return_id, &lines)
                .await?
        };

        Ok(ReceiveReturnResult {
            order_return,
            restocked_units,
        })
    }

    async fn captured_payment(&self, order_id: &OrderId) -> Result<Payment, AppError> {
        let page = self
            .payment
            .list_payments(ListPaymentsFilter {
                order_id: Some(order_id.clone()),
                status: None,
                pagination: Pagination::default(),
            })
            .await?;
        page.data
            .into_iter()
            .find(|payment| {
                matches!(
                    payment.status,
                    PaymentStatus::Captured | PaymentStatus::PartiallyRefunded
                )
            })
            .ok_or_else(|| {
                AppError::conflict("order has no refundable captured payment".to_owned())
            })
    }

    async fn provider_refund_by_key(&self, order_id: &OrderId, key: &str) -> Option<Refund> {
        let page = self
            .payment
            .list_payments(ListPaymentsFilter {
                order_id: Some(order_id.clone()),
                status: None,
                pagination: Pagination::default(),
            })
            .await
            .ok()?;
        for payment in page.data {
            let refunds = self.payment.list_refunds(payment.id.as_str()).await.ok()?;
            if let Some(refund) = refunds
                .into_iter()
                .find(|refund| refund.idempotency_key.as_deref() == Some(key))
            {
                return Some(refund);
            }
        }
        None
    }
}

/// Local key stored in `idempotency_keys`, namespaced away from provider keys.
fn local_key(key: &str) -> String {
    format!("after-sales:{key}")
}

/// Sums the proportional share of immutable line totals for returned units.
fn refundable_amount(
    order: &Order,
    requested: impl Iterator<Item = (LineItemId, i64)>,
) -> Result<i64, AppError> {
    let mut amount = 0_i64;
    for (order_item_id, quantity) in requested {
        if quantity <= 0 {
            continue;
        }
        let item = order
            .items
            .iter()
            .find(|item| item.id == order_item_id)
            .ok_or_else(|| AppError::not_found("order item", &order_item_id))?;
        if quantity > item.quantity {
            return Err(AppError::conflict(format!(
                "cannot refund {quantity} of item {order_item_id}: only {} were purchased",
                item.quantity
            )));
        }
        let proportional = item
            .total
            .checked_mul(quantity)
            .ok_or_else(|| AppError::validation("refund amount overflow"))?
            / item.quantity;
        amount = amount
            .checked_add(proportional)
            .ok_or_else(|| AppError::validation("refund amount overflow"))?;
    }
    Ok(amount)
}

/// Caps a computed refund by the order total minus refunds already recorded.
fn cap_refund(order: &Order, amount: i64) -> i64 {
    if amount <= 0 {
        return 0;
    }
    let refunded: i64 = order.refunds.iter().map(|refund| refund.amount).sum();
    let remaining = order.total.saturating_sub(refunded).max(0);
    amount.min(remaining)
}
