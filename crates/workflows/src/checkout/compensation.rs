//! Rolling a failed completion back.

use fastcart_module_checkout::{CheckoutSession, CheckoutStatus};
use fastcart_module_order::{Order, OrderStatus, PaymentStatus as OrderPaymentStatus};
use fastcart_module_payment::Payment;

use super::*;

impl CheckoutWorkflow {
    /// Undoes a failed completion attempt.
    ///
    /// Compensation is fenced by the placement token: a runner whose lease was
    /// taken over must not release reservations or refund a payment that the
    /// new owner is still placing. A terminal checkout (`failed` / `canceled`)
    /// can still hold captured money, so it is refunded even though it no
    /// longer carries a claim.
    pub(super) async fn compensate(
        &self,
        checkout: &CheckoutSession,
        payment: &Payment,
        combination_payments: &[Payment],
        order: Option<&Order>,
        placement_token: &str,
    ) {
        // Re-read persisted state before undoing anything. A concurrent runner
        // may have placed the order after this runner observed a failure; only
        // compensate when the checkout is still unplaced.
        let latest_checkout = match self.checkout.get(&checkout.id).await {
            Ok(latest) => Some(latest),
            Err(error) => {
                tracing::warn!(
                    checkout_id = %checkout.id,
                    error = %error,
                    "could not re-read checkout before compensation"
                );
                None
            }
        };
        let Some(latest_checkout) = latest_checkout else {
            return;
        };
        if latest_checkout.status == CheckoutStatus::OrderPlaced
            || order.is_some_and(|order| latest_checkout.order_id.as_ref() == Some(&order.id))
        {
            return;
        }
        if !latest_checkout.status.is_terminal() {
            match self
                .checkout
                .holds_placement(&checkout.id, placement_token)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(
                        checkout_id = %checkout.id,
                        "checkout placement was taken over; skipping compensation"
                    );
                    return;
                }
                Err(error) => {
                    tracing::warn!(
                        checkout_id = %checkout.id,
                        error = %error,
                        "could not verify the placement claim; skipping compensation"
                    );
                    return;
                }
            }
        }

        let mut refundable = true;
        if let Some(order) = order {
            match self.order.get(&order.id).await {
                // Another runner already compensated this order.
                Ok(latest) if latest.status == OrderStatus::Canceled => {}
                Ok(latest)
                    if matches!(latest.status, OrderStatus::Pending | OrderStatus::Confirmed) =>
                {
                    if let Err(error) = self.inventory.restock_order(&order.id).await {
                        tracing::warn!(
                            order_id = %order.id,
                            error = %error,
                            "checkout compensation could not restock inventory"
                        );
                    }
                    if let Err(error) = self
                        .order
                        .cancel(&order.id, Some("checkout compensation"))
                        .await
                    {
                        tracing::warn!(
                            order_id = %order.id,
                            error = %error,
                            "checkout compensation could not cancel the order"
                        );
                    }
                    if let Err(error) = self
                        .order
                        .set_payment_status(&order.id, OrderPaymentStatus::Refunded)
                        .await
                    {
                        tracing::warn!(
                            order_id = %order.id,
                            error = %error,
                            "checkout compensation could not mark the order refunded"
                        );
                    }
                }
                Ok(latest) => {
                    // The order moved into fulfilment: refunding and canceling
                    // it automatically would undo an operator's decision.
                    tracing::error!(
                        order_id = %order.id,
                        status = latest.status.as_str(),
                        "order cannot be compensated automatically; manual reconciliation is required"
                    );
                    refundable = false;
                }
                Err(error) => {
                    tracing::warn!(
                        order_id = %order.id,
                        error = %error,
                        "could not re-read order before compensation"
                    );
                    // Refunding without knowing whether the order is still
                    // pending could leave a paid, active order behind.
                    refundable = false;
                }
            }
        }
        if !refundable {
            return;
        }

        let _ = self.inventory.release_checkout(&checkout.id).await;
        // The claims were taken before the capture, under the same placement
        // fence as the refund above, and are given back with it: the customer
        // never got the discount, so the promotion counters must not keep it.
        self.release_promotion_usage(&checkout.id).await;
        let refunded = self
            .refund_compensation(checkout, payment, combination_payments)
            .await;

        // Mark the checkout failed only once the money is back with the
        // customer. A failed refund keeps the session retryable so a later
        // attempt can finish the compensation instead of hiding captured money
        // behind a terminal state.
        if refunded
            && matches!(
                latest_checkout.status,
                CheckoutStatus::PaymentPending | CheckoutStatus::PaymentConfirmed
            )
            && let Err(error) = self
                .checkout
                .transition(
                    &checkout.id,
                    CheckoutStatus::Failed,
                    Some("checkout compensation"),
                )
                .await
        {
            tracing::warn!(
                checkout_id = %checkout.id,
                error = %error,
                "failed to mark the compensated checkout as failed"
            );
        }
    }

    /// Refunds every payment a failed completion captured.
    ///
    /// A single payment keeps the historical refund key, so an earlier build
    /// retrying the same compensation still deduplicates against it. A
    /// combination payment keys every refund separately: one key identifies one
    /// logical refund, and these are different money.
    async fn refund_compensation(
        &self,
        checkout: &CheckoutSession,
        payment: &Payment,
        combination_payments: &[Payment],
    ) -> bool {
        let refunds: Vec<(&Payment, String)> = if combination_payments.is_empty() {
            vec![(
                payment,
                format!("checkout-compensation:{}", checkout.id.as_str()),
            )]
        } else {
            std::iter::once(payment)
                .chain(combination_payments.iter())
                .map(|candidate| {
                    (
                        candidate,
                        format!(
                            "checkout-compensation:{}:{}",
                            checkout.id.as_str(),
                            candidate.id.as_str()
                        ),
                    )
                })
                .collect()
        };
        let mut refunded = true;
        for (candidate, refund_key) in refunds {
            if let Err(error) = self
                .payment
                .refund_with_idempotency(
                    candidate.id.as_str(),
                    candidate.amount,
                    Some("checkout compensation".to_owned()),
                    Some(refund_key),
                )
                .await
            {
                tracing::error!(
                    payment_id = %candidate.id,
                    error = %error,
                    "checkout compensation refund failed; manual reconciliation is required"
                );
                refunded = false;
            }
        }
        refunded
    }
}
