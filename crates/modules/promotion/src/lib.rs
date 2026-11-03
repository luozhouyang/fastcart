//! The `promotion` commerce module.
//!
//! Owns promotions, coupons, usage accounting and the deterministic discount
//! calculator. The crate is self-contained: cross-module orchestration belongs
//! in `fastcart-workflows`.

pub mod domain;
pub mod service;

pub use domain::{
    Campaign, Coupon, CouponFilter, CreateCampaignInput, CreateCouponInput, CreatePromotionInput,
    Promotion, PromotionApplication, PromotionCart, PromotionContext, PromotionFilter,
    PromotionLine, PromotionResult, PromotionRule, PromotionRuleAttribute, PromotionRuleInput,
    PromotionRuleOperator, PromotionStatus, PromotionType, PromotionUsage, UpdateCampaignInput,
    UpdateCouponInput, UpdatePromotionInput, apply_promotion, calculate_promotions,
};
pub use service::PromotionService;
