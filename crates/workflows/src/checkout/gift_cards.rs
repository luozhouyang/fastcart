//! Gift card redemption during checkout placement.

use std::collections::BTreeMap;

use fastcart_kernel::{AppError, CheckoutId};
use fastcart_module_payment::GiftCardRedemptionStatus;

use super::*;

/// One card's share of the checkout, as the cart quoted it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GiftCardCharge {
    /// Gift card that pays.
    pub gift_card_id: String,
    /// Amount the card pays for this checkout, in minor units.
    pub amount_minor: i64,
}

impl CheckoutWorkflow {
    /// Returns the amounts this checkout already redeemed, keyed by card.
    ///
    /// Only still-redeemed rows count: a compensated checkout's cards were
    /// credited back, so pricing the cart against them is correct again.
    pub(super) async fn gift_card_credits_for_checkout(
        &self,
        checkout_id: &CheckoutId,
    ) -> Result<BTreeMap<String, i64>, AppError> {
        let mut credits = BTreeMap::new();
        for redemption in self
            .payment
            .gift_card_redemptions_for_checkout(checkout_id)
            .await?
        {
            if redemption.status != GiftCardRedemptionStatus::Redeemed {
                continue;
            }
            *credits.entry(redemption.gift_card_id).or_default() += redemption.amount_minor;
        }
        Ok(credits)
    }

    /// Redeems every applied gift card for the checkout, before the order is
    /// placed.
    ///
    /// Each card's amount is the one the cart quoted — the smaller of what the
    /// customer asked for and the live balance at pricing time. A card that no
    /// longer covers that amount rejects the completion with a conflict instead
    /// of being silently reduced: the customer agreed to a total, and changing
    /// the payment behind it without their consent is not something a checkout
    /// an operator cannot see should do.
    ///
    /// The guarded decrement and the redemption record share one
    /// `BEGIN IMMEDIATE` transaction, so concurrent completions of different
    /// checkouts holding the same card cannot both spend it. A retried
    /// completion of the same checkout finds its record and returns it without
    /// spending again.
    pub(super) async fn redeem_checkout_gift_cards(
        &self,
        checkout_id: &CheckoutId,
        cart: &fastcart_module_cart::Cart,
    ) -> Result<Vec<GiftCardCharge>, AppError> {
        let charges: Vec<GiftCardCharge> = cart
            .gift_cards
            .iter()
            .filter(|applied| applied.applied_amount > 0)
            .map(|applied| GiftCardCharge {
                gift_card_id: applied.gift_card_id.clone(),
                amount_minor: applied.applied_amount,
            })
            .collect();
        let quoted: i64 = charges.iter().map(|charge| charge.amount_minor).sum();
        if quoted != cart.totals.gift_card_total {
            return Err(AppError::internal(format!(
                "cart gift card charges {quoted} do not match its gift card total {}",
                cart.totals.gift_card_total
            )));
        }
        // Validate that the cards are still usable before spending any of them.
        // A card disabled between quoting and completion must not leave the
        // other cards already drained for an order that then fails.
        for charge in &charges {
            let card = self.payment.get_gift_card(&charge.gift_card_id).await?;
            if card.status != fastcart_module_payment::GiftCardStatus::Active {
                return Err(AppError::conflict(format!(
                    "gift card {} is no longer active",
                    charge.gift_card_id
                )));
            }
            if card.currency_code != cart.currency_code.as_str() {
                return Err(AppError::conflict(format!(
                    "gift card {} is priced in {}, not {}",
                    charge.gift_card_id, card.currency_code, cart.currency_code
                )));
            }
        }
        let mut redeemed = Vec::with_capacity(charges.len());
        for charge in charges {
            match self
                .payment
                .redeem_gift_card_for_checkout(
                    checkout_id,
                    &charge.gift_card_id,
                    charge.amount_minor,
                )
                .await
            {
                Ok(_) => redeemed.push(charge),
                Err(error) => {
                    // Only this checkout's own redemptions are given back: a
                    // concurrent completion owns its own records.
                    self.reverse_checkout_gift_cards(checkout_id).await;
                    return Err(error);
                }
            }
        }
        Ok(redeemed)
    }

    /// Credits back the cards this checkout redeemed and has not placed.
    ///
    /// Called from compensation and from a failed redemption loop. Idempotent:
    /// already-reversed rows are skipped, so a retried compensation cannot
    /// credit a card twice.
    pub(super) async fn reverse_checkout_gift_cards(&self, checkout_id: &CheckoutId) {
        match self
            .payment
            .reverse_gift_card_redemptions(checkout_id)
            .await
        {
            Ok(0) => {}
            Ok(reversed) => tracing::info!(
                checkout_id = %checkout_id,
                reversed,
                "credited back the gift cards of a rejected completion"
            ),
            Err(error) => tracing::error!(
                checkout_id = %checkout_id,
                error = %error,
                "failed to credit back the gift cards of a rejected completion; manual \
                 reconciliation is required"
            ),
        }
    }

    /// Reverses this checkout's redemptions only while `token` still holds the
    /// placement claim.
    ///
    /// A runner whose lease was taken over must not credit cards the new owner
    /// is placing with. A claim that cannot be verified is treated like a lost
    /// one: leaving the customer's money on the card is repaired by the next
    /// attempt or by manual reconciliation, whereas double spending is not.
    pub(super) async fn reverse_claimed_gift_cards(
        &self,
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
                    "checkout placement was taken over; skipping the gift card reversal"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    checkout_id = %checkout_id,
                    error = %error,
                    "could not verify the placement claim; skipping the gift card reversal"
                );
                return;
            }
        }
        self.reverse_checkout_gift_cards(checkout_id).await;
    }

    /// Names the order the checkout's gift card redemptions paid for.
    ///
    /// A failure is logged rather than failing the replay: the order stands and
    /// the cards are spent, only the ledger row still names the checkout. The
    /// next replay converges.
    pub(super) async fn assign_gift_card_redemptions(
        &self,
        checkout_id: &CheckoutId,
        order_id: &fastcart_kernel::OrderId,
    ) {
        if let Err(error) = self
            .payment
            .assign_gift_card_redemptions_to_order(checkout_id, order_id)
            .await
        {
            tracing::error!(
                checkout_id = %checkout_id,
                order_id = %order_id,
                error = %error,
                "failed to move the gift card redemptions onto the placed order"
            );
        }
    }
}
