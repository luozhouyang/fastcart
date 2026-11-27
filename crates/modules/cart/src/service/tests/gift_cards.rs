use std::collections::BTreeMap;

use fastcart_kernel::AppError;

use super::*;
use crate::domain::GiftCardApplication;

fn gift_card(gift_card_id: &str, currency: &str, balance: i64) -> GiftCardApplication {
    GiftCardApplication {
        gift_card_id: gift_card_id.to_owned(),
        masked_code: format!(
            "****{}",
            &gift_card_id[gift_card_id.len().saturating_sub(4)..]
        ),
        currency_code: currency.to_owned(),
        amount_minor: None,
        balance_minor: balance,
    }
}

async fn priced_cart(service: &CartService, subtotal: i64) -> Cart {
    let cart = create_cart(service, "USD").await;
    service
        .add_item(&cart.id, item_input("USD", variant(), subtotal, 1))
        .await
        .unwrap()
}

#[tokio::test]
async fn applying_a_gift_card_records_it_and_pays_the_total_down() {
    let service = service().await;
    let cart = priced_cart(&service, 2_000).await;

    let cart = service
        .apply_gift_card(&cart.id, gift_card("gift_alpha", "USD", 1_500))
        .await
        .unwrap();

    assert_eq!(cart.gift_cards.len(), 1);
    assert_eq!(cart.gift_cards[0].gift_card_id, "gift_alpha");
    assert_eq!(cart.gift_cards[0].amount, 1_500);
    assert_eq!(cart.gift_cards[0].balance, 1_500);
    assert_eq!(cart.gift_cards[0].applied_amount, 1_500);
    assert_eq!(cart.totals.gift_card_total, 1_500);
    assert_eq!(cart.totals.total, 500);
}

#[tokio::test]
async fn applying_more_than_the_balance_records_the_balance() {
    let service = service().await;
    let cart = priced_cart(&service, 100).await;
    let mut input = gift_card("gift_small", "USD", 900);
    input.amount_minor = Some(5_000);

    let cart = service.apply_gift_card(&cart.id, input).await.unwrap();

    // The request exceeds both the balance and the payable amount; what is
    // stored is the balance, and the cart only spends what it owes.
    assert_eq!(cart.gift_cards[0].amount, 900);
    assert_eq!(cart.gift_cards[0].balance, 900);
    assert_eq!(cart.gift_cards[0].applied_amount, 100);
    assert_eq!(cart.totals.gift_card_total, 100);
    assert_eq!(cart.totals.total, 0);
}

#[tokio::test]
async fn multiple_cards_pay_in_application_order_and_never_overpay() {
    let service = service().await;
    let cart = priced_cart(&service, 1_000).await;

    let cart = service
        .apply_gift_card(&cart.id, gift_card("gift_first", "USD", 700))
        .await
        .unwrap();
    let cart = service
        .apply_gift_card(&cart.id, gift_card("gift_second", "USD", 700))
        .await
        .unwrap();

    assert_eq!(cart.totals.gift_card_total, 1_000);
    assert_eq!(cart.totals.total, 0);
    // The first card covers its full 700; the second only the 300 left.
    assert_eq!(cart.gift_cards[0].applied_amount, 700);
    assert_eq!(cart.gift_cards[1].applied_amount, 300);
}

#[tokio::test]
async fn a_refreshed_balance_lowers_what_the_cart_quotes() {
    let service = service().await;
    let cart = priced_cart(&service, 1_000).await;
    let cart = service
        .apply_gift_card(&cart.id, gift_card("gift_spent", "USD", 800))
        .await
        .unwrap();
    assert_eq!(cart.totals.total, 200);

    // The card was spent elsewhere after it was applied.
    let cart = service
        .refresh_gift_card_balances(&cart.id, &BTreeMap::from([("gift_spent".to_owned(), 250)]))
        .await
        .unwrap();

    assert_eq!(cart.gift_cards[0].balance, 250);
    assert_eq!(cart.gift_cards[0].applied_amount, 250);
    assert_eq!(cart.totals.gift_card_total, 250);
    assert_eq!(cart.totals.total, 750);
}

#[tokio::test]
async fn rejects_a_card_priced_in_another_currency() {
    let service = service().await;
    let cart = priced_cart(&service, 1_000).await;

    let rejected = service
        .apply_gift_card(&cart.id, gift_card("gift_eur", "EUR", 500))
        .await
        .unwrap_err();
    assert!(matches!(rejected, AppError::Validation(_)), "{rejected}");
    assert_eq!(
        service.get_cart(&cart.id).await.unwrap().gift_cards.len(),
        0
    );
}

#[tokio::test]
async fn rejects_a_duplicate_application() {
    let service = service().await;
    let cart = priced_cart(&service, 1_000).await;
    service
        .apply_gift_card(&cart.id, gift_card("gift_twice", "USD", 500))
        .await
        .unwrap();

    let rejected = service
        .apply_gift_card(&cart.id, gift_card("gift_twice", "USD", 500))
        .await
        .unwrap_err();
    assert!(matches!(rejected, AppError::Conflict(_)), "{rejected}");
    assert_eq!(
        service.get_cart(&cart.id).await.unwrap().gift_cards.len(),
        1
    );
}

#[tokio::test]
async fn rejects_invalid_input_and_missing_cards() {
    let service = service().await;
    let cart = priced_cart(&service, 1_000).await;

    let mut empty_id = gift_card("gift_ok", "USD", 100);
    empty_id.gift_card_id = "  ".to_owned();
    let rejected = service
        .apply_gift_card(&cart.id, empty_id)
        .await
        .unwrap_err();
    assert!(matches!(rejected, AppError::Validation(_)), "{rejected}");

    let mut empty_code = gift_card("gift_ok", "USD", 100);
    empty_code.masked_code = String::new();
    let rejected = service
        .apply_gift_card(&cart.id, empty_code)
        .await
        .unwrap_err();
    assert!(matches!(rejected, AppError::Validation(_)), "{rejected}");

    let mut negative = gift_card("gift_ok", "USD", 100);
    negative.amount_minor = Some(-1);
    let rejected = service
        .apply_gift_card(&cart.id, negative)
        .await
        .unwrap_err();
    assert!(matches!(rejected, AppError::Validation(_)), "{rejected}");

    let removed = service
        .remove_gift_card(&cart.id, "gift_missing")
        .await
        .unwrap_err();
    assert!(matches!(removed, AppError::NotFound(_)), "{removed}");
}

#[tokio::test]
async fn removing_a_card_restores_the_total() {
    let service = service().await;
    let cart = priced_cart(&service, 1_000).await;
    let cart = service
        .apply_gift_card(&cart.id, gift_card("gift_removed", "USD", 400))
        .await
        .unwrap();
    assert_eq!(cart.totals.total, 600);

    let cart = service
        .remove_gift_card(&cart.id, "gift_removed")
        .await
        .unwrap();
    assert!(cart.gift_cards.is_empty());
    assert_eq!(cart.totals.gift_card_total, 0);
    assert_eq!(cart.totals.total, 1_000);

    // Removing it again reports it as gone rather than succeeding silently.
    let rejected = service
        .remove_gift_card(&cart.id, "gift_removed")
        .await
        .unwrap_err();
    assert!(matches!(rejected, AppError::NotFound(_)), "{rejected}");
}

#[tokio::test]
async fn gift_cards_cannot_be_changed_on_a_completed_cart() {
    let service = service().await;
    let cart = priced_cart(&service, 1_000).await;
    service.complete_cart(&cart.id).await.unwrap();

    let rejected = service
        .apply_gift_card(&cart.id, gift_card("gift_late", "USD", 100))
        .await
        .unwrap_err();
    assert!(matches!(rejected, AppError::Conflict(_)), "{rejected}");
}
