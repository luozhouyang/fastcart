//! Checkout orchestration: cart validation, stock reservation, payment capture
//! and idempotent order placement.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CartId, CheckoutId};
use fastcart_module_catalog::CatalogService;
use fastcart_module_checkout::{CheckoutService, CheckoutSession};
use fastcart_module_inventory::InventoryService;
use fastcart_module_order::{Order, OrderService};
use fastcart_module_payment::{Payment, PaymentService};
use fastcart_module_promotion::PromotionService;
use fastcart_module_store::RegionService;

use crate::cart::{CartContext, CartWorkflow};

const DEFAULT_CHECKOUT_TTL_SECONDS: i64 = 30 * 60;
const RESERVATION_TTL_SECONDS: i64 = 30 * 60;
/// Lease for the exclusive completion claim. A crashed runner stops blocking
/// retries once this elapses; capture and transitions are idempotent.
const PLACEMENT_LEASE_SECONDS: i64 = 120;

mod compensation;
mod complete;
mod create;
mod promotions;
mod settlement;

/// Request to open a checkout from a cart.
#[derive(Clone, Debug)]
pub struct CreateCheckoutRequest {
    /// Cart being converted.
    pub cart_id: CartId,
    /// Contact email, defaulting to the cart email.
    pub email: Option<String>,
    /// Registered payment provider identifier.
    pub payment_provider: String,
    /// Client idempotency key.
    pub idempotency_key: Option<String>,
    /// Checkout lifetime in seconds.
    pub expires_in_seconds: i64,
    /// Price, tax and shipping context.
    pub context: CartContext,
}

impl CreateCheckoutRequest {
    /// Creates a request for a cart and provider.
    #[must_use]
    pub fn new(cart_id: CartId, payment_provider: impl Into<String>, context: CartContext) -> Self {
        Self {
            cart_id,
            email: None,
            payment_provider: payment_provider.into(),
            idempotency_key: None,
            expires_in_seconds: DEFAULT_CHECKOUT_TTL_SECONDS,
            context,
        }
    }
}

/// Result of a completed checkout.
#[derive(Clone, Debug)]
pub struct CompleteCheckoutResult {
    /// Placed order.
    pub order: Order,
    /// Captured payment.
    pub payment: Payment,
    /// Final checkout session.
    pub checkout: CheckoutSession,
}

/// Orchestrates checkout, inventory, payment and order modules.
pub struct CheckoutWorkflow {
    carts: CartWorkflow,
    catalog: CatalogService,
    inventory: InventoryService,
    checkout: CheckoutService,
    order: OrderService,
    payment: PaymentService,
    promotion: PromotionService,
    regions: RegionService,
}

impl CheckoutWorkflow {
    /// Creates a workflow bound to one store.
    ///
    /// The payment service must be constructed with the provider registry shared
    /// by the process.
    #[must_use]
    pub fn new(db: StoreDatabase, payment: PaymentService) -> Self {
        Self {
            carts: CartWorkflow::new(db.clone()),
            catalog: CatalogService::new(db.clone()),
            inventory: InventoryService::new(db.clone()),
            checkout: CheckoutService::new(db.clone()),
            order: OrderService::new(db.clone()),
            payment,
            promotion: PromotionService::new(db.clone()),
            regions: RegionService::new(db),
        }
    }

    /// Returns the checkout service.
    #[must_use]
    pub fn checkout_service(&self) -> &CheckoutService {
        &self.checkout
    }

    /// Reads a checkout session.
    pub async fn get(&self, checkout_id: &CheckoutId) -> Result<CheckoutSession, AppError> {
        self.checkout.get(checkout_id).await
    }
}
