//! Confirming an order edit settles both sides of the change.
//!
//! The line rewrite is atomic with the order totals (ADR 0009), and now with
//! the stock movements too: an edit that cannot take its units leaves the order
//! untouched. The money side is deliberately a separate, idempotent call, the
//! same shape claims and exchanges use, and `payment_summary` is the read side
//! that says what is still owed or owed back.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CheckoutId, Metadata, StoreId, VariantId};
use fastcart_module_inventory::{
    CreateInventoryItemInput, CreateLocationInput, InventoryService, ReservationLine,
    ReservationReference,
};
use fastcart_module_order::{
    CreateOrderEditInput, CreateOrderInput, CreateOrderItemInput, FulfillmentInput,
    FulfillmentItemInput, FulfillmentStatus, Order, OrderEditAction, OrderEditItemInput,
    OrderService, OrderStatus,
};
use fastcart_module_payment::{
    CreatePaymentSessionInput, ManualProvider, PaymentService, ProviderRegistry,
};
use fastcart_workflows::AfterSalesWorkflow;

const STOCK: i64 = 20;

#[path = "order_edit_settlement/events.rs"]
mod events;
#[path = "order_edit_settlement/money.rs"]
mod money;
#[path = "order_edit_settlement/stock.rs"]
mod stock;
#[path = "order_edit_settlement/support.rs"]
mod support;
