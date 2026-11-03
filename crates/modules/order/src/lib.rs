//! The `order` commerce module.
//!
//! Domain rules, application services and SQLite repositories for this module
//! live here. The crate is self-contained: cross-module orchestration belongs in
//! `fastcart-workflows`.

pub mod after_sales;
pub mod domain;
pub mod service;
pub mod service_after_sales;

pub use after_sales::{
    ClaimItemInput, ClaimItemStatus, ClaimItemType, ClaimStatus, CreateClaimInput,
    CreateExchangeInput, CreateOrderEditInput, CreateReturnInput, ExchangeItemDirection,
    ExchangeItemInput, ExchangeItemStatus, ExchangeStatus, OrderClaim, OrderClaimItem, OrderEdit,
    OrderEditAction, OrderEditItem, OrderEditItemInput, OrderEditItemStatus, OrderEditStatus,
    OrderExchange, OrderExchangeItem, OrderReturn, OrderReturnItem, ReceiveReturnInput,
    ReceiveReturnItemInput, ReturnItemInput, ReturnItemStatus, ReturnStatus, WriteOffReturnInput,
    WriteOffReturnItemInput,
};
pub use domain::{
    CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput, FulfillmentInput,
    FulfillmentStatus, Order, OrderAdjustment, OrderFilter, OrderFulfillment, OrderItem,
    OrderItemAdjustment, OrderRefund, OrderStatus, OrderStatusHistory, PaymentStatus, RefundInput,
    RefundStatus,
};
pub use service::OrderService;
