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
    AfterSalesFulfillment, AfterSalesFulfillmentItem, AfterSalesFulfillmentItemInput,
    ClaimItemInput, ClaimItemStatus, ClaimItemType, ClaimStatus, CreateAfterSalesFulfillmentInput,
    CreateClaimInput, CreateExchangeInput, CreateOrderEditInput, CreateReturnInput,
    ExchangeItemDirection, ExchangeItemInput, ExchangeItemStatus, ExchangeStatus, OrderClaim,
    OrderClaimItem, OrderEdit, OrderEditAction, OrderEditItem, OrderEditItemInput,
    OrderEditItemStatus, OrderEditStatus, OrderExchange, OrderExchangeItem, OrderReturn,
    OrderReturnItem, ReceiveReturnInput, ReceiveReturnItemInput, ReturnItemInput, ReturnItemStatus,
    ReturnStatus, WriteOffReturnInput, WriteOffReturnItemInput,
};
pub use domain::{
    CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput, FulfillmentInput,
    FulfillmentItemInput, FulfillmentStatus, Order, OrderAdjustment, OrderFilter, OrderFulfillment,
    OrderFulfillmentItem, OrderItem, OrderItemAdjustment, OrderRefund, OrderShippingMethod,
    OrderStatus, OrderStatusHistory, PaymentStatus, RefundInput, RefundStatus,
};
pub use service::OrderService;
