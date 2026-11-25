//! After-sales workflow: return/claim refunds and idempotent restocking.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, Metadata, StoreId, VariantId,
};
use fastcart_module_inventory::{
    CreateInventoryItemInput, CreateLocationInput, InventoryService, ReservationLine,
    ReservationReference,
};
use fastcart_module_order::{
    ClaimItemInput, ClaimItemType, ClaimStatus, CreateClaimInput, CreateExchangeInput,
    CreateOrderInput, CreateOrderItemInput, CreateReturnInput, ExchangeItemInput, Order,
    OrderReturn, OrderService, ReceiveReturnInput, ReceiveReturnItemInput, RefundInput,
    RefundStatus as OrderRefundStatus, ReturnItemInput, ReturnStatus, WriteOffReturnInput,
    WriteOffReturnItemInput,
};
use fastcart_module_payment::{
    CreatePaymentSessionInput, ManualProvider, Payment, PaymentService, ProviderRegistry,
    RefundStatus as PaymentRefundStatus,
};
use fastcart_module_shipping::{
    CreateFulfillmentRequest, FulfillmentProvider, FulfillmentProviderRegistry, FulfillmentReceipt,
};
use fastcart_workflows::AfterSalesWorkflow;
use serde_json::json;

#[path = "after_sales/claims.rs"]
mod claims;
#[path = "after_sales/exchanges.rs"]
mod exchanges;
#[path = "after_sales/refunds.rs"]
mod refunds;
#[path = "after_sales/returns.rs"]
mod returns;
#[path = "after_sales/support.rs"]
mod support;
