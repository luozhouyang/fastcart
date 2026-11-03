//! The `order` commerce module.
//!
//! Domain rules, application services and SQLite repositories for this module
//! live here. The crate is self-contained: cross-module orchestration belongs in
//! `fastcart-workflows`.

pub mod domain;
pub mod service;

pub use domain::{
    CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput, FulfillmentInput,
    FulfillmentStatus, Order, OrderAdjustment, OrderFilter, OrderFulfillment, OrderItem,
    OrderItemAdjustment, OrderRefund, OrderStatus, OrderStatusHistory, PaymentStatus, RefundInput,
    RefundStatus,
};
pub use service::OrderService;
