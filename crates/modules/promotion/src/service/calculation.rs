//! Cart discount calculation.

//! SQLite-backed promotion service.

use fastcart_kernel::AppError;

use crate::domain::{
    PromotionCart, PromotionContext, PromotionResult, PromotionStatus, calculate_promotions,
    window_contains,
};

/// Application service for promotions, coupons, usage accounting and pricing
/// calculations.
use super::*;

impl PromotionService {
    /// Applies eligible active promotions to a cart in priority order.
    pub fn calculate(
        &self,
        promotions_with_rules: &[PromotionContext],
        cart: &PromotionCart,
    ) -> Result<Vec<PromotionResult>, AppError> {
        let eligible: Vec<PromotionContext> = promotions_with_rules
            .iter()
            .filter(|context| {
                context.promotion.status == PromotionStatus::Active
                    && window_contains(
                        cart.at,
                        context.promotion.starts_at,
                        context.promotion.ends_at,
                    )
                    && context
                        .promotion
                        .usage_limit
                        .is_none_or(|limit| context.promotion.usage_count < limit)
            })
            .cloned()
            .collect();
        calculate_promotions(&eligible, cart)
    }
}
