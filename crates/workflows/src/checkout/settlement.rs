//! Payment capture, order finishing and reservation rebinding.

use fastcart_kernel::{AppError, CheckoutId, OrderId, PaymentSessionId};
use fastcart_module_checkout::{CheckoutSession, CheckoutStatus};
use fastcart_module_inventory::ReservationRebinding;
use fastcart_module_order::{Order, OrderStatus, PaymentStatus as OrderPaymentStatus};
use fastcart_module_payment::Payment;

use super::*;

impl CheckoutWorkflow {
    /// Captures the payments that settle a checkout's payment collection.
    ///
    /// Returns the payment of the checkout's own session — the one replays and
    /// the completion result report — together with the payments of any further
    /// session a combination payment was split into.
    ///
    /// A collection that is just the checkout's own session (and a checkout
    /// from before collections existed) keeps the original single-session
    /// capture: same provider idempotency key, same errors, same concurrency
    /// fence. A collection split across several sessions is captured as a
    /// collection, which only succeeds when the captures cover its amount.
    pub(super) async fn capture_checkout_payments(
        &self,
        checkout_id: &CheckoutId,
        payment_session_id: &PaymentSessionId,
    ) -> Result<(Payment, Vec<Payment>), AppError> {
        let collection = self
            .payment
            .collection_for_checkout(checkout_id.as_str())
            .await?;
        let sessions = match collection.as_ref() {
            Some(collection) => {
                self.payment
                    .list_collection_sessions(collection.id.as_str())
                    .await?
            }
            None => Vec::new(),
        };
        // A combination payment is the only case that needs the collection
        // path: everything else captures the checkout's one session.
        let Some(collection) = collection.as_ref().filter(|_| sessions.len() > 1) else {
            let payment = self.payment.capture(payment_session_id.as_str()).await?;
            return Ok((payment, Vec::new()));
        };
        if !sessions
            .iter()
            .any(|session| session.id == *payment_session_id)
        {
            // The checkout's money is not part of its collection, so capturing
            // the collection would place an order the customer never paid for.
            return Err(AppError::conflict(
                "checkout payment collection does not hold the checkout's payment session",
            ));
        }
        let captured = self
            .payment
            .capture_collection(collection.id.as_str())
            .await?;
        let payment = self
            .payment
            .payment_for_session_id(payment_session_id.as_str())
            .await?;
        let additional = captured
            .into_iter()
            .filter(|candidate| candidate.id != payment.id)
            .collect();
        Ok((payment, additional))
    }

    /// Links the checkout's payment collection to the order it paid for.
    ///
    /// The collection is the money the checkout owed, and the order is where
    /// that money ends up. A checkout with no collection — one from before
    /// collections existed — has nothing to link. The link is idempotent, so a
    /// replay converges on the same record instead of failing.
    pub(super) async fn link_collection_to_order(
        &self,
        checkout_id: &CheckoutId,
        order_id: &OrderId,
    ) -> Result<(), AppError> {
        let Some(collection) = self
            .payment
            .collection_for_checkout(checkout_id.as_str())
            .await?
        else {
            return Ok(());
        };
        self.payment
            .assign_collection_to_order(collection.id.as_str(), order_id)
            .await?;
        Ok(())
    }

    pub(super) async fn finish_order(
        &self,
        checkout: &CheckoutSession,
        cart: &fastcart_module_cart::Cart,
        payment: &Payment,
        combination_payments: &[Payment],
        order: &Order,
        placement_token: &str,
    ) -> Result<Order, AppError> {
        // A concurrent retry may already have completed this checkout. Treat
        // the placed state as success so the loser never compensates a good order.
        let current_checkout = self.checkout.get(&checkout.id).await?;
        if current_checkout.status == CheckoutStatus::OrderPlaced {
            if current_checkout.order_id.as_ref() == Some(&order.id) {
                return self.order.get(&order.id).await;
            }
            return Err(AppError::conflict(
                "checkout was placed for a different order",
            ));
        }

        // Every payment of the collection pays for this order; a combination
        // payment leaves several payments behind.
        self.payment
            .assign_order(payment.id.as_str(), &order.id)
            .await?;
        for additional in combination_payments {
            self.payment
                .assign_order(additional.id.as_str(), &order.id)
                .await?;
        }
        // The collection is the money this order owes, so from here on it must
        // name the order: a later charge, refund or report finds it that way.
        self.link_collection_to_order(&checkout.id, &order.id)
            .await?;
        let mut order = self.order.get(&order.id).await?;
        if order.payment_status != OrderPaymentStatus::Captured {
            match self
                .order
                .set_payment_status(&order.id, OrderPaymentStatus::Captured)
                .await
            {
                Ok(updated) => order = updated,
                Err(error) => {
                    let latest = self.order.get(&order.id).await?;
                    if latest.payment_status == OrderPaymentStatus::Captured {
                        order = latest;
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        // The reservations were recorded for the cart lines, but the order
        // assigned fresh identifiers to those lines. Binding every reservation
        // to the order line it was placed as — before the commit, so a retry
        // after a crash converges on the same mapping — is what keeps the stock
        // a line consumed, and the location it came from, traceable from the
        // order.
        let bindings = reservation_rebindings(cart, &order)?;
        self.inventory
            .rebind_reservations(&checkout.id, &order.id, &bindings)
            .await?;

        // Commit inventory before confirming the order. `confirm` moves the
        // order into a state that compensation must not cancel, so a later
        // `commit` failure (for example because reservations expired while the
        // customer paid) would leave a captured payment and a confirmed order
        // with no stock movement. With the commit first, a failed commit is
        // still compensable: the order is pending, so the refund path runs.
        self.inventory.commit(&checkout.id, &order.id).await?;

        if order.status == OrderStatus::Pending {
            match self.order.confirm(&order.id).await {
                Ok(confirmed) => order = confirmed,
                Err(error) => {
                    // Another runner may have confirmed the same order between
                    // our read and this call. That is success, not a reason to
                    // compensate a good order.
                    let latest = self.order.get(&order.id).await?;
                    if is_placed_order_status(latest.status) {
                        order = latest;
                    } else {
                        return Err(error);
                    }
                }
            }
        }

        if let Err(error) = self
            .checkout
            .mark_order_placed_with_claim(&checkout.id, &order.id, placement_token)
            .await
        {
            // The claim may have been lost to a takeover, but if that runner
            // placed the same order this attempt still succeeded.
            let latest = self.checkout.get(&checkout.id).await?;
            let placed = latest.status == CheckoutStatus::OrderPlaced
                && latest.order_id.as_ref() == Some(&order.id);
            if !placed {
                return Err(error);
            }
        }

        if cart.status == fastcart_module_cart::CartStatus::Open
            && let Err(error) = self.carts.complete_cart(&cart.id).await
        {
            // The order is already placed and inventory committed; a cart that
            // expired (or a transient failure here) must not trigger compensation.
            let latest = self.carts.get_cart(&cart.id).await?;
            if latest.status != fastcart_module_cart::CartStatus::Completed
                && latest.status != fastcart_module_cart::CartStatus::Expired
            {
                tracing::warn!(
                    cart_id = %cart.id,
                    error = %error,
                    "cart completion failed after the order was placed"
                );
            }
        }
        Ok(order)
    }
}

/// Pairs every cart line with the order line it was placed as.
///
/// The pairing is positional: the order is created from the items built out of
/// `cart.items` in order, `OrderService::create_order` writes them in that order
/// and reads them back by rowid, so `order.items[i]` is the line
/// `cart.items[i]` became. The variant of each pair is compared so a mismatch
/// fails loudly instead of binding a reservation to the wrong line.
fn reservation_rebindings(
    cart: &fastcart_module_cart::Cart,
    order: &Order,
) -> Result<Vec<ReservationRebinding>, AppError> {
    if cart.items.len() != order.items.len() {
        return Err(AppError::internal(format!(
            "order {} has {} lines but its cart has {}",
            order.id,
            order.items.len(),
            cart.items.len()
        )));
    }
    cart.items
        .iter()
        .zip(order.items.iter())
        .map(|(cart_item, order_item)| {
            if order_item.variant_id.as_ref() != Some(&cart_item.variant_id) {
                return Err(AppError::internal(format!(
                    "order line {} does not match the cart line {} it was placed from",
                    order_item.id, cart_item.id
                )));
            }
            Ok(ReservationRebinding::new(
                cart_item.id.clone(),
                order_item.id.clone(),
            ))
        })
        .collect()
}

/// Returns true when an order has been accepted into fulfillment.
fn is_placed_order_status(status: OrderStatus) -> bool {
    matches!(
        status,
        OrderStatus::Confirmed
            | OrderStatus::Processing
            | OrderStatus::Fulfilled
            | OrderStatus::Completed
    )
}
