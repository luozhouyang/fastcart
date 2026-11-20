//! Boundary tests for the monetary arithmetic at the edges of the `i64` range.
//!
//! Commercial amounts are integer minor units, so realistic values sit far
//! below the limits. These tests pin down the behaviour at the limits instead:
//! a saturated value or an explicit error, never a panic and never a silent
//! wrap-around. Each assertion that would silently change meaning under
//! wrapping arithmetic is paired with the wrapped value it must not produce.

use fastcart_kernel::{CurrencyCode, Money, Rounding, allocate_proportionally};

const MAX: i64 = i64::MAX;

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

#[test]
fn addition_past_the_maximum_is_an_error_not_a_wrap() {
    let sum = Money::new(MAX, usd()).checked_add(&Money::new(1, usd()));

    // Plain `+` would panic in a debug build and wrap to the minimum here.
    assert_eq!(MAX.wrapping_add(1), i64::MIN);
    assert!(sum.is_err(), "{sum:?}");
}

#[test]
fn subtraction_past_the_minimum_is_an_error_not_a_wrap() {
    let difference = Money::new(i64::MIN, usd()).checked_sub(&Money::new(1, usd()));

    assert_eq!(i64::MIN.wrapping_sub(1), MAX);
    assert!(difference.is_err(), "{difference:?}");
}

#[test]
fn multiplication_past_the_maximum_is_an_error_not_a_wrap() {
    let product = Money::new(MAX, usd()).checked_mul(2);

    assert_eq!(MAX.wrapping_mul(2), -2);
    assert!(product.is_err(), "{product:?}");
}

#[test]
fn basis_points_of_one_hundred_percent_fit_and_more_is_rejected() {
    // 10 000 basis points is exactly 100%: the largest rate that can keep the
    // amount representable.
    let unchanged = Money::new(MAX, usd())
        .apply_basis_points(10_000, Rounding::HalfAwayFromZero)
        .unwrap();
    assert_eq!(unchanged.amount_minor(), MAX);

    let doubled = Money::new(MAX, usd()).apply_basis_points(20_000, Rounding::HalfAwayFromZero);
    assert!(doubled.is_err(), "{doubled:?}");
}

#[test]
fn allocation_of_the_whole_total_gives_every_part_its_own_value() {
    let shares = allocate_proportionally(&[3, 7, 10], 20, 20).unwrap();

    assert_eq!(shares, vec![3, 7, 10]);
}

#[test]
fn allocation_of_zero_gives_zero_shares() {
    let shares = allocate_proportionally(&[5, 0, 2], 7, 0).unwrap();

    assert_eq!(shares, vec![0, 0, 0]);
    assert_eq!(shares.iter().sum::<i64>(), 0);
}

#[test]
fn zero_valued_parts_never_receive_a_share() {
    let shares = allocate_proportionally(&[0, 7, 0], 7, 7).unwrap();

    assert_eq!(shares, vec![0, 7, 0]);
}

#[test]
fn allocation_of_a_total_at_the_maximum_stays_inside_each_part() {
    // A total of `i64::MAX` is representable, so the split has to be exact.
    // The remaining unit cannot go to the small part: it has no room left.
    let shares = allocate_proportionally(&[MAX - 1, 1], MAX, MAX).unwrap();

    assert_eq!(shares.iter().sum::<i64>(), MAX);
    assert_eq!(shares, vec![MAX - 1, 1]);
}

#[test]
fn allocation_below_a_total_at_the_maximum_stays_exact() {
    let shares = allocate_proportionally(&[MAX - 1, 1], MAX, MAX - 1).unwrap();

    assert_eq!(shares.iter().sum::<i64>(), MAX - 1);
    assert!(
        shares
            .iter()
            .zip([MAX - 1, 1])
            .all(|(share, part)| *share <= part),
        "a share must never exceed its own part: {shares:?}"
    );
}

#[test]
fn allocation_rejects_negative_parts_and_totals() {
    assert!(allocate_proportionally(&[-1, 2], 1, 1).is_err());
    assert!(allocate_proportionally(&[1, 1], -2, 1).is_err());
    assert!(allocate_proportionally(&[1, 1], 2, -1).is_err());
}
