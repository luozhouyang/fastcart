use fastcart_kernel::{CurrencyCode, Metadata};

use super::{
    Promotion, PromotionCart, PromotionContext, PromotionLine, PromotionStatus, PromotionTarget,
    PromotionType, calculate_promotions,
};

fn line(id: &str, unit_price: i64) -> PromotionLine {
    PromotionLine {
        line_id: id.to_owned(),
        product_id: format!("prod_{id}"),
        variant_id: format!("var_{id}"),
        category_ids: Vec::new(),
        unit_price,
        quantity: 1,
        is_shipping: false,
    }
}

fn cart(lines: Vec<PromotionLine>) -> PromotionCart {
    let subtotal = lines.iter().map(|line| line.unit_price).sum();
    PromotionCart {
        currency: CurrencyCode::new("USD").unwrap(),
        subtotal,
        shipping_total: 0,
        customer_id: None,
        customer_group_ids: Vec::new(),
        lines,
        at: 0,
    }
}

fn fixed_amount(value: i64) -> PromotionContext {
    PromotionContext {
        promotion: Promotion {
            id: "promo_1".to_owned(),
            code: None,
            title: "Ten off".to_owned(),
            promotion_type: PromotionType::FixedAmount,
            target: PromotionTarget::Items,
            value,
            status: PromotionStatus::Active,
            is_automatic: true,
            starts_at: None,
            ends_at: None,
            usage_limit: None,
            usage_count: 0,
            per_customer_limit: None,
            priority: 0,
            is_stackable: true,
            metadata: Metadata::empty(),
            rules: Vec::new(),
            created_at: 0,
            updated_at: 0,
        },
        rules: Vec::new(),
    }
}

/// The integer remainder of a proportional split must never push a line's
/// discount above that line's own value. A line discounted below zero makes
/// the checkout's line total negative, which aborts order placement after
/// the customer's payment was already captured.
#[test]
fn fixed_amount_never_discounts_a_line_below_its_value() {
    let cart = cart(vec![line("a", 10), line("b", 1), line("c", 1)]);

    let results = calculate_promotions(&[fixed_amount(10)], &cart).unwrap();

    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(
        result.total_discount,
        result
            .applications
            .iter()
            .map(|application| application.amount)
            .sum::<i64>(),
        "the reported discount must equal the sum of its applications"
    );
    assert_eq!(
        result.total_discount, 10,
        "a cart worth more than the coupon must still grant its full value"
    );
    for application in &result.applications {
        let target = cart
            .lines
            .iter()
            .find(|line| Some(&line.line_id) == application.line_id.as_ref())
            .expect("every application targets a cart line");
        let line_value = target.unit_price * target.quantity;
        assert!(
            application.amount <= line_value,
            "line {} is worth {line_value} but was discounted {}",
            target.line_id,
            application.amount
        );
    }
}
