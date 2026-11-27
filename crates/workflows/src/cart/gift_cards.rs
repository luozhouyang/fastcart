//! Gift cards as a payment method on a cart.

use super::*;

/// Masks a redemption code, keeping only its last four characters.
///
/// The full code is a bearer credential: the storefront payload carries the
/// masked form, while the payment module keeps the code itself. Codes shorter
/// than four characters are masked entirely, because showing them would reveal
/// the whole credential.
pub fn mask_gift_card_code(code: &str) -> String {
    let characters: Vec<char> = code.trim().chars().collect();
    if characters.len() <= 4 {
        return "*".repeat(characters.len());
    }
    let visible = characters.len() - 4;
    let mut masked = "*".repeat(visible);
    masked.extend(characters[visible..].iter());
    masked
}

impl CartWorkflow {
    /// Applies a gift card code to an open cart.
    ///
    /// The payment module owns the card, so the workflow validates it there —
    /// it must exist, be active, be priced in the cart's currency, still hold a
    /// balance and not already be applied — and then hands the cart a snapshot.
    /// The amount applied is the whole remaining balance; a cart whose total is
    /// smaller simply does not spend the rest.
    pub async fn apply_gift_card(&self, cart_id: &CartId, code: &str) -> Result<Cart, AppError> {
        let code = code.trim();
        if code.is_empty() {
            return Err(AppError::validation("gift card code must not be empty"));
        }
        let cart = self.cart.get_cart(cart_id).await?;
        let card = self.payment.find_gift_card_by_code(code).await?;
        if card.status != GiftCardStatus::Active {
            return Err(AppError::conflict(format!(
                "gift card {} is not active",
                card.id
            )));
        }
        if card.balance_minor <= 0 {
            return Err(AppError::validation(format!(
                "gift card {} has no remaining balance",
                card.id
            )));
        }
        if card.currency_code != cart.currency_code.as_str() {
            return Err(AppError::validation(format!(
                "gift card currency {} does not match the cart currency {}",
                card.currency_code, cart.currency_code
            )));
        }
        if cart
            .gift_cards
            .iter()
            .any(|applied| applied.gift_card_id == card.id)
        {
            return Err(AppError::conflict(
                "gift card is already applied to this cart",
            ));
        }
        self.cart
            .apply_gift_card(
                cart_id,
                GiftCardApplication {
                    gift_card_id: card.id,
                    masked_code: mask_gift_card_code(code),
                    currency_code: card.currency_code,
                    amount_minor: None,
                    balance_minor: card.balance_minor,
                },
            )
            .await
    }

    /// Removes an applied gift card from an open cart.
    ///
    /// Nothing was spent yet — the cart only stores a quote — so this is a
    /// plain delete. The next checkout quotes without the card.
    pub async fn remove_gift_card(
        &self,
        cart_id: &CartId,
        gift_card_id: &str,
    ) -> Result<Cart, AppError> {
        self.cart.remove_gift_card(cart_id, gift_card_id).await
    }

    /// Refreshes the balance snapshot of every applied gift card.
    ///
    /// The cart module never reads the payment-owned card, so pricing asks the
    /// owning module for the live balance and hands the snapshot down. A card
    /// that no longer exists fails the recalculation instead of being priced as
    /// free money; [`Self::apply_gift_card`] is how a card gets onto a cart,
    /// and [`Self::remove_gift_card`] how it leaves.
    ///
    /// `credits` adds back money this cart already spent on a checkout that is
    /// still being completed. A checkout redeems before the order is placed, so
    /// a retry after a crash would otherwise re-price the cart against a balance
    /// the same checkout had already drawn down.
    pub(super) async fn refresh_gift_card_balances(
        &self,
        cart: &Cart,
        credits: &BTreeMap<String, i64>,
    ) -> Result<Cart, AppError> {
        if cart.gift_cards.is_empty() || !cart.status.is_open() {
            return Ok(cart.clone());
        }
        let mut balances = BTreeMap::new();
        for applied in &cart.gift_cards {
            let card = self.payment.get_gift_card(&applied.gift_card_id).await?;
            let credit = credits
                .get(&applied.gift_card_id)
                .copied()
                .unwrap_or(0)
                .max(0);
            balances.insert(
                applied.gift_card_id.clone(),
                card.balance_minor.saturating_add(credit),
            );
        }
        self.cart
            .refresh_gift_card_balances(&cart.id, &balances)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::mask_gift_card_code;

    #[test]
    fn masking_keeps_only_the_last_four_characters() {
        assert_eq!(
            mask_gift_card_code("12345678901234567890"),
            "****************7890"
        );
        assert_eq!(mask_gift_card_code("  GIFT-EUR-1  "), "******UR-1");
        assert_eq!(mask_gift_card_code("AB12"), "****");
        assert_eq!(mask_gift_card_code("A1"), "**");
    }
}
