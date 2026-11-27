//! Promotion usage claiming, rollback and hand-over.

use std::collections::BTreeMap;

use fastcart_kernel::{AppError, CheckoutId, now_timestamp};
use fastcart_module_promotion::checkout_usage_key;

use super::*;

impl CheckoutWorkflow {
    /// Promotions the cart applied, with the discount each one granted.
    ///
    /// Keyed by promotion id and ordered, so the claim order is deterministic.
    /// Only the adjustments that were actually granted are listed: a promotion
    /// the calculator dropped is not charged anything.
    fn cart_promotion_discounts(cart: &fastcart_module_cart::Cart) -> BTreeMap<String, i64> {
        let mut promotions: BTreeMap<String, i64> = BTreeMap::new();
        for item in &cart.items {
            for adjustment in &item.adjustments {
                if let Some(promotion_id) = adjustment.promotion_id.as_ref() {
                    *promotions.entry(promotion_id.to_string()).or_default() +=
                        adjustment.amount.max(0);
                }
            }
        }
        for adjustment in &cart.shipping_adjustments {
            if let Some(promotion_id) = adjustment.promotion_id.as_ref() {
                *promotions.entry(promotion_id.to_string()).or_default() +=
                    adjustment.amount.max(0);
            }
        }
        // An order-target promotion is applied to the cart itself and leaves no
        // adjustment behind, so its cart-level row is the only record that it
        // was used. A row that granted nothing — the marker a coupon leaves
        // before anything applies — is not a use and stays out of the claim.
        for discount in &cart.discounts {
            if discount.amount <= 0 {
                continue;
            }
            if let Some(promotion_id) = discount.promotion_id.as_ref() {
                *promotions.entry(promotion_id.to_string()).or_default() += discount.amount;
            }
        }
        promotions
    }

    /// Coupon that unlocked a promotion on this cart, when the customer used one.
    async fn coupon_for_promotion(
        &self,
        cart: &fastcart_module_cart::Cart,
        promotion_id: &str,
    ) -> Option<String> {
        let code = cart
            .discounts
            .iter()
            .find(|discount| {
                discount
                    .promotion_id
                    .as_ref()
                    .is_some_and(|id| id.as_str() == promotion_id)
            })
            .and_then(|discount| discount.code.clone())?;
        match self.promotion.validate_coupon(&code, now_timestamp()).await {
            Ok(coupon) => Some(coupon.id),
            Err(_) => None,
        }
    }

    /// Atomically claims one usage per promotion the cart applies.
    ///
    /// Runs before the capture, so a promotion whose limit, per-customer cap or
    /// campaign budget is used up by a competing checkout rejects the
    /// completion instead of over-issuing the discount. Everything claimed
    /// before the failing one is given back, while this runner still holds the
    /// placement claim.
    pub(super) async fn claim_promotion_usage(
        &self,
        cart: &fastcart_module_cart::Cart,
        checkout_id: &CheckoutId,
        placement_token: &str,
    ) -> Result<(), AppError> {
        let key = checkout_usage_key(checkout_id.as_str());
        let customer_id = cart.customer_id.as_ref().map(ToString::to_string);
        let currency_code = cart.currency_code.to_string();
        let mut claimed = false;
        for (promotion_id, amount) in Self::cart_promotion_discounts(cart) {
            let coupon_id = self.coupon_for_promotion(cart, &promotion_id).await;
            match self
                .promotion
                .reserve_usage(
                    &promotion_id,
                    coupon_id.as_deref(),
                    customer_id.as_deref(),
                    checkout_id.as_str(),
                    amount,
                    Some(&currency_code),
                )
                .await
            {
                Ok(_) => claimed = true,
                Err(error) => {
                    if claimed {
                        self.rollback_promotion_usage(&key, checkout_id, placement_token)
                            .await;
                    }
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Gives back the claims of a completion that is about to fail.
    ///
    /// Fenced by the placement claim, like compensation: a runner whose lease
    /// was taken over must not release the claims the new owner is working with.
    pub(super) async fn rollback_promotion_usage(
        &self,
        key: &str,
        checkout_id: &CheckoutId,
        placement_token: &str,
    ) {
        match self
            .checkout
            .holds_placement(checkout_id, placement_token)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(
                    checkout_id = %checkout_id,
                    "checkout placement was taken over; skipping the promotion claim rollback"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    checkout_id = %checkout_id,
                    error = %error,
                    "could not verify the placement claim; skipping the promotion claim rollback"
                );
                return;
            }
        }
        match self.promotion.release_usage(key).await {
            Ok(released) => tracing::info!(
                checkout_id = %checkout_id,
                released,
                "released the promotion claims of a rejected completion"
            ),
            Err(error) => tracing::error!(
                checkout_id = %checkout_id,
                error = %error,
                "failed to release the promotion claims of a rejected completion; the counters stay consumed"
            ),
        }
    }

    /// Moves the checkout's promotion claims onto the order that was placed.
    ///
    /// The counters are already charged, so this only renames the key. If it
    /// fails the order stands and the usage stays keyed to the checkout: the
    /// counts are right, only the key did not move, and a replay retries it.
    pub(super) async fn reassign_promotion_usage(&self, checkout_id: &CheckoutId, order_id: &str) {
        let key = checkout_usage_key(checkout_id.as_str());
        if let Err(error) = self.promotion.reassign_usage(&key, order_id).await {
            tracing::error!(
                checkout_id = %checkout_id,
                order_id = %order_id,
                error = %error,
                "failed to move the promotion usage onto the placed order"
            );
        }
    }

    /// Gives the promotion claims of a compensated checkout back to its counters.
    pub(super) async fn release_promotion_usage(&self, checkout_id: &CheckoutId) {
        let key = checkout_usage_key(checkout_id.as_str());
        match self.promotion.release_usage(&key).await {
            Ok(released) => tracing::info!(
                checkout_id = %checkout_id,
                released,
                "released the promotion claims of a compensated checkout"
            ),
            Err(error) => tracing::error!(
                checkout_id = %checkout_id,
                error = %error,
                "failed to release the promotion claims of a compensated checkout; manual reconciliation is required"
            ),
        }
    }
}
