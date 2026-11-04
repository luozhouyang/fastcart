//! Checkout orchestration: cart validation, stock reservation, payment capture
//! and idempotent order placement.

use std::collections::HashMap;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CartId, CheckoutId, Metadata, now_timestamp};
use fastcart_module_catalog::CatalogService;
use fastcart_module_checkout::{
    CheckoutService, CheckoutSession, CheckoutStatus, CreateCheckoutInput,
};
use fastcart_module_inventory::{InventoryService, ReservationLine, ReservationReference};
use fastcart_module_order::{
    CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput, Order, OrderService,
    OrderShippingMethod, OrderStatus, PaymentStatus as OrderPaymentStatus,
};
use fastcart_module_payment::{
    CreatePaymentSessionInput, Payment, PaymentService, PaymentStatus as PaymentSessionStatus,
};
use fastcart_module_promotion::PromotionService;

use crate::cart::{CartContext, CartWorkflow};

const DEFAULT_CHECKOUT_TTL_SECONDS: i64 = 30 * 60;
const RESERVATION_TTL_SECONDS: i64 = 30 * 60;

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
            promotion: PromotionService::new(db),
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

    /// Opens a checkout: recalculates the cart, reserves stock and creates a
    /// provider payment session.
    pub async fn create(
        &self,
        request: CreateCheckoutRequest,
    ) -> Result<CheckoutSession, AppError> {
        let cart = self
            .carts
            .recalculate(&request.cart_id, &request.context)
            .await?;
        if cart.items.is_empty() {
            return Err(AppError::validation("cannot check out an empty cart"));
        }
        let email = request
            .email
            .clone()
            .or_else(|| cart.email.clone())
            .ok_or_else(|| AppError::validation("an email address is required"))?;
        let requires_shipping = cart.items.iter().any(|item| item.requires_shipping);
        if requires_shipping && cart.shipping_methods.is_empty() {
            return Err(AppError::validation(
                "a shipping method is required before checkout",
            ));
        }
        let shipping_method = cart
            .shipping_methods
            .first()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| AppError::validation(error.to_string()))?
            .unwrap_or(serde_json::Value::Null);
        let promotion_codes: Vec<String> = cart
            .discounts
            .iter()
            .filter_map(|discount| discount.code.clone())
            .collect();
        let expires_in_seconds = if request.expires_in_seconds <= 0 {
            DEFAULT_CHECKOUT_TTL_SECONDS
        } else {
            request.expires_in_seconds
        };

        let checkout = self
            .checkout
            .create(CreateCheckoutInput {
                cart_id: cart.id.clone(),
                customer_id: cart.customer_id.clone(),
                email: Some(email),
                currency_code: cart.currency_code.to_string(),
                shipping_address: serde_json::to_value(&cart.shipping_address)
                    .map_err(|error| AppError::validation(error.to_string()))?,
                billing_address: serde_json::to_value(&cart.billing_address)
                    .map_err(|error| AppError::validation(error.to_string()))?,
                shipping_method,
                totals: serde_json::to_value(&cart.totals)
                    .map_err(|error| AppError::validation(error.to_string()))?,
                promotion_codes,
                idempotency_key: request.idempotency_key.clone(),
                expires_in_seconds,
            })
            .await?;

        // An idempotent replay returns the existing session unchanged.
        if checkout.status != CheckoutStatus::Created {
            return Ok(checkout);
        }

        let checkout = self
            .checkout
            .transition(&checkout.id, CheckoutStatus::Validating, None)
            .await?;

        let mut reservation_lines = Vec::new();
        for item in &cart.items {
            let variant = self.catalog.get_variant(item.variant_id.as_str()).await?;
            if variant.manage_inventory && !variant.allow_backorder {
                reservation_lines.push(ReservationLine {
                    variant_id: item.variant_id.clone(),
                    quantity: item.quantity,
                    line_item_id: Some(item.id.clone()),
                });
            }
        }
        if !reservation_lines.is_empty()
            && let Err(error) = self
                .inventory
                .reserve(
                    reservation_lines,
                    ReservationReference::new(checkout.id.clone()),
                    RESERVATION_TTL_SECONDS,
                )
                .await
        {
            let _ = self
                .checkout
                .transition(
                    &checkout.id,
                    CheckoutStatus::Failed,
                    Some(&error.to_string()),
                )
                .await;
            return Err(error);
        }

        let checkout = self
            .checkout
            .transition(&checkout.id, CheckoutStatus::ReadyForPayment, None)
            .await?;

        let session = match self
            .payment
            .create_session(CreatePaymentSessionInput {
                cart_id: Some(cart.id.clone()),
                checkout_id: Some(checkout.id.clone()),
                order_id: None,
                provider: request.payment_provider,
                amount: cart.totals.total,
                currency_code: cart.currency_code.to_string(),
                metadata: Metadata::empty(),
            })
            .await
        {
            Ok(session) => session,
            Err(error) => {
                let _ = self.inventory.release_checkout(&checkout.id).await;
                let _ = self
                    .checkout
                    .transition(
                        &checkout.id,
                        CheckoutStatus::Failed,
                        Some(&error.to_string()),
                    )
                    .await;
                return Err(error);
            }
        };

        match self
            .checkout
            .attach_payment_session(&checkout.id, &session.id)
            .await
        {
            Ok(checkout) => Ok(checkout),
            Err(error) => {
                let _ = self.inventory.release_checkout(&checkout.id).await;
                let _ = self
                    .checkout
                    .transition(
                        &checkout.id,
                        CheckoutStatus::Failed,
                        Some(&error.to_string()),
                    )
                    .await;
                Err(error)
            }
        }
    }

    /// Captures payment and places the order exactly once.
    ///
    /// Replaying a completed checkout returns the existing order and payment.
    /// A retry after a partial failure resumes from the persisted state.
    pub async fn complete(
        &self,
        checkout_id: &CheckoutId,
        context: &CartContext,
    ) -> Result<CompleteCheckoutResult, AppError> {
        let checkout = self.checkout.get(checkout_id).await?;
        let payment_session_id = checkout
            .payment_session_id
            .clone()
            .ok_or_else(|| AppError::conflict("checkout has no payment session"))?;
        if checkout.status == CheckoutStatus::OrderPlaced {
            let order_id = checkout
                .order_id
                .clone()
                .ok_or_else(|| AppError::conflict("checkout has no order"))?;
            let order = self.order.get(&order_id).await?;
            let payment = self
                .payment
                .payment_for_session_id(payment_session_id.as_str())
                .await?;
            if let Ok(cart) = self.carts.get_cart(&checkout.cart_id).await {
                self.record_promotion_usage(&cart, &order).await;
            }
            return Ok(CompleteCheckoutResult {
                order,
                payment,
                checkout,
            });
        }
        if !matches!(
            checkout.status,
            CheckoutStatus::PaymentPending | CheckoutStatus::PaymentConfirmed
        ) {
            return Err(AppError::conflict(format!(
                "cannot complete checkout in status {}",
                checkout.status.as_str()
            )));
        }

        // Validate the cart snapshot before charging the customer. A price or
        // stock change must not result in a captured payment without an order.
        let cart = self.carts.recalculate(&checkout.cart_id, context).await?;
        let totals = serde_json::to_value(&cart.totals)
            .map_err(|error| AppError::validation(error.to_string()))?;
        if totals != checkout.totals {
            return Err(AppError::conflict(
                "cart totals changed since checkout was created",
            ));
        }

        let checkout = if checkout.status == CheckoutStatus::PaymentPending {
            self.checkout
                .transition(checkout_id, CheckoutStatus::PaymentConfirmed, None)
                .await?
        } else {
            checkout
        };

        let payment = self.payment.capture(payment_session_id.as_str()).await?;
        if payment.status != PaymentSessionStatus::Captured {
            return Err(AppError::conflict(format!(
                "payment is in state {} and cannot complete this checkout",
                payment.status
            )));
        }

        let mut items = Vec::with_capacity(cart.items.len());
        let mut subtotal = 0_i64;
        let mut discount_total = 0_i64;
        let mut tax_total = 0_i64;
        let mut items_total = 0_i64;
        for item in &cart.items {
            let line_subtotal = item
                .unit_price
                .checked_mul(item.quantity)
                .ok_or_else(|| AppError::validation("line subtotal overflow"))?;
            let discount: i64 = item.adjustments.iter().map(|entry| entry.amount).sum();
            let line_taxes: Vec<_> = cart
                .tax_lines
                .iter()
                .filter(|line| line.cart_item_id.as_ref() == Some(&item.id))
                .collect();
            let line_tax: i64 = line_taxes.iter().map(|line| line.amount).sum();
            let exclusive_tax: i64 = line_taxes
                .iter()
                .filter(|line| !line.is_tax_inclusive)
                .map(|line| line.amount)
                .sum();
            let line_total = line_subtotal - discount + exclusive_tax;
            subtotal += line_subtotal;
            discount_total += discount;
            tax_total += line_tax;
            items_total += line_total;
            items.push(CreateOrderItemInput {
                product_id: Some(item.product_id.clone()),
                variant_id: Some(item.variant_id.clone()),
                product_title: item.title.clone(),
                variant_title: item.variant_title.clone(),
                sku: item.sku.clone(),
                thumbnail: item.thumbnail.clone(),
                unit_price: item.unit_price,
                quantity: item.quantity,
                subtotal: Some(line_subtotal),
                discount_total: Some(discount),
                tax_total: Some(line_tax),
                total: Some(line_total),
                is_tax_inclusive: item.is_tax_inclusive,
                requires_shipping: item.requires_shipping,
                metadata: item.metadata.clone(),
                adjustments: item
                    .adjustments
                    .iter()
                    .map(|adjustment| CreateAdjustmentInput {
                        promotion_id: adjustment.promotion_id.clone(),
                        code: adjustment.code.clone(),
                        amount: adjustment.amount,
                        description: adjustment.description.clone(),
                    })
                    .collect(),
            });
        }

        let shipping_tax: i64 = cart
            .tax_lines
            .iter()
            .filter(|line| line.shipping_method_id.is_some() && !line.is_tax_inclusive)
            .map(|line| line.amount)
            .sum();
        let adjustments = if shipping_tax == 0 {
            Vec::new()
        } else {
            vec![CreateAdjustmentInput {
                promotion_id: None,
                code: Some("shipping_tax".to_owned()),
                amount: shipping_tax,
                description: Some("Shipping tax".to_owned()),
            }]
        };
        let shipping_total = cart.totals.shipping_total;
        let total = items_total + shipping_total + shipping_tax;
        let shipping_methods = cart
            .shipping_methods
            .iter()
            .map(|method| OrderShippingMethod {
                shipping_option_id: method.shipping_option_id.to_string(),
                name: method.name.clone(),
                provider_id: method
                    .data
                    .get("provider_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("manual")
                    .to_owned(),
                amount: method.amount,
                currency_code: method.currency.to_string(),
                rate_id: method
                    .data
                    .get("rate_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                data: method.data.clone(),
            })
            .collect();

        let order = match self
            .order
            .create_order(CreateOrderInput {
                customer_id: cart.customer_id.clone(),
                email: checkout
                    .email
                    .clone()
                    .or_else(|| cart.email.clone())
                    .ok_or_else(|| AppError::validation("an email address is required"))?,
                currency_code: cart.currency_code.to_string(),
                items,
                adjustments,
                shipping_address: checkout.shipping_address.clone(),
                billing_address: checkout.billing_address.clone(),
                shipping_methods,
                subtotal,
                discount_total,
                shipping_total,
                tax_total,
                total,
                metadata: Metadata::empty(),
                idempotency_key: Some(format!("checkout:{}", checkout.id.as_str())),
            })
            .await
        {
            Ok(order) => order,
            Err(error) => {
                self.compensate(&checkout, &payment, None).await;
                return Err(error);
            }
        };

        let order = match self.finish_order(&checkout, &cart, &payment, &order).await {
            Ok(order) => order,
            Err(error) => {
                self.compensate(&checkout, &payment, Some(&order)).await;
                return Err(error);
            }
        };

        self.record_promotion_usage(&cart, &order).await;
        let checkout = self.checkout.get(&checkout.id).await?;
        Ok(CompleteCheckoutResult {
            order,
            payment,
            checkout,
        })
    }

    async fn finish_order(
        &self,
        checkout: &CheckoutSession,
        cart: &fastcart_module_cart::Cart,
        payment: &Payment,
        order: &Order,
    ) -> Result<Order, AppError> {
        // A concurrent retry may already have completed this checkout. Treat
        // the placed state as success so the loser never compensates a good order.
        let current_checkout = self.checkout.get(&checkout.id).await?;
        if current_checkout.status == CheckoutStatus::OrderPlaced {
            if current_checkout.order_id.as_ref() == Some(&order.id) {
                return self.order.get(&order.id).await;
            }
            return Err(AppError::conflict(
                "checkout was placed for a different order",
            ));
        }

        self.payment
            .assign_order(payment.id.as_str(), &order.id)
            .await?;
        let current = self.order.get(&order.id).await?;
        if current.payment_status != OrderPaymentStatus::Captured {
            self.order
                .set_payment_status(&order.id, OrderPaymentStatus::Captured)
                .await?;
        }
        if self.order.get(&order.id).await?.status == OrderStatus::Pending {
            self.order.confirm(&order.id).await?;
        }
        let order = self.order.get(&order.id).await?;

        self.inventory.commit(&checkout.id, &order.id).await?;

        if let Err(error) = self
            .checkout
            .mark_order_placed(&checkout.id, &order.id)
            .await
        {
            let latest = self.checkout.get(&checkout.id).await?;
            let placed = latest.status == CheckoutStatus::OrderPlaced
                && latest.order_id.as_ref() == Some(&order.id);
            if !placed {
                return Err(error);
            }
        }

        if cart.status == fastcart_module_cart::CartStatus::Open
            && let Err(error) = self.carts.complete_cart(&cart.id).await
        {
            let latest = self.carts.get_cart(&cart.id).await?;
            if latest.status != fastcart_module_cart::CartStatus::Completed {
                return Err(error);
            }
        }
        Ok(order)
    }

    async fn compensate(
        &self,
        checkout: &CheckoutSession,
        payment: &Payment,
        order: Option<&Order>,
    ) {
        if let Some(order) = order {
            let _ = self.inventory.restock_order(&order.id).await;
            let _ = self
                .order
                .cancel(&order.id, Some("checkout compensation"))
                .await;
            let _ = self
                .order
                .set_payment_status(&order.id, OrderPaymentStatus::Refunded)
                .await;
        }
        let _ = self.inventory.release_checkout(&checkout.id).await;
        if let Err(error) = self
            .payment
            .refund(
                payment.id.as_str(),
                payment.amount,
                Some("checkout compensation".to_owned()),
            )
            .await
        {
            tracing::error!(
                payment_id = %payment.id,
                error = %error,
                "checkout compensation refund failed; manual reconciliation is required"
            );
        }
    }

    async fn record_promotion_usage(&self, cart: &fastcart_module_cart::Cart, order: &Order) {
        let mut promotions: HashMap<String, i64> = HashMap::new();
        for item in &cart.items {
            for adjustment in &item.adjustments {
                if let Some(promotion_id) = adjustment.promotion_id.as_ref() {
                    *promotions.entry(promotion_id.to_string()).or_default() +=
                        adjustment.amount.max(0);
                }
            }
        }
        for adjustment in &cart.shipping_adjustments {
            if let Some(promotion_id) = adjustment.promotion_id.as_ref() {
                *promotions.entry(promotion_id.to_string()).or_default() +=
                    adjustment.amount.max(0);
            }
        }
        let customer_id = cart.customer_id.as_ref().map(ToString::to_string);
        for (promotion_id, amount) in promotions {
            let coupon_id = cart
                .discounts
                .iter()
                .find(|discount| {
                    discount.promotion_id.as_ref().map(ToString::to_string)
                        == Some(promotion_id.clone())
                })
                .and_then(|discount| discount.code.clone());
            let coupon_id = match coupon_id {
                Some(code) => match self.promotion.validate_coupon(&code, now_timestamp()).await {
                    Ok(coupon) => Some(coupon.id),
                    Err(_) => None,
                },
                None => None,
            };
            if let Err(error) = self
                .promotion
                .record_usage(
                    &promotion_id,
                    coupon_id.as_deref(),
                    customer_id.as_deref(),
                    Some(order.id.as_str()),
                    amount,
                    Some(&order.currency_code),
                )
                .await
            {
                tracing::warn!(promotion_id = %promotion_id, error = %error, "failed to record promotion usage");
            }
        }
    }
}
