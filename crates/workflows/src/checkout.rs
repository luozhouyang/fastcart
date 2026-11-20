//! Checkout orchestration: cart validation, stock reservation, payment capture
//! and idempotent order placement.

use std::collections::{BTreeMap, HashMap};

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CartId, CheckoutId, Metadata, OrderId, PaymentSessionId, now_timestamp,
};
use fastcart_module_catalog::CatalogService;
use fastcart_module_checkout::{
    CheckoutService, CheckoutSession, CheckoutStatus, CreateCheckoutInput,
};
use fastcart_module_inventory::{
    InventoryService, ReservationLine, ReservationLinePolicy, ReservationRebinding,
    ReservationReference,
};
use fastcart_module_order::{
    CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput, CreateOrderTaxLineInput, Order,
    OrderService, OrderShippingMethod, OrderStatus, PaymentStatus as OrderPaymentStatus,
};
use fastcart_module_payment::{
    CreatePaymentSessionInput, Payment, PaymentService, PaymentStatus as PaymentSessionStatus,
};
use fastcart_module_promotion::{PromotionService, checkout_usage_key};
use fastcart_module_store::{Region, RegionService};

use crate::cart::{CartContext, CartWorkflow};

const DEFAULT_CHECKOUT_TTL_SECONDS: i64 = 30 * 60;
const RESERVATION_TTL_SECONDS: i64 = 30 * 60;
/// Lease for the exclusive completion claim. A crashed runner stops blocking
/// retries once this elapses; capture and transitions are idempotent.
const PLACEMENT_LEASE_SECONDS: i64 = 120;

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

/// Rejects a payment provider the cart's region does not enable.
///
/// An empty `payment_providers` list means the region does not restrict
/// providers, so the payment registry remains the only gate.
fn validate_region_payment_provider(region: &Region, provider: &str) -> Result<(), AppError> {
    if region.payment_providers.is_empty()
        || region
            .payment_providers
            .iter()
            .any(|allowed| allowed == provider)
    {
        return Ok(());
    }
    Err(AppError::validation(format!(
        "payment provider {provider} is not enabled for region {}; allowed providers: {}",
        region.name,
        region.payment_providers.join(", ")
    )))
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

        // The cart's region constrains the payment providers the checkout may
        // use. Validate before creating a session so a rejected provider cannot
        // leave an orphaned checkout row behind.
        let region_id = cart.region_id.clone();
        if let Some(region_id) = region_id.as_deref() {
            let region = self.regions.get_region(region_id).await?;
            validate_region_payment_provider(&region, &request.payment_provider)?;
        }

        let checkout = self
            .checkout
            .create(CreateCheckoutInput {
                cart_id: cart.id.clone(),
                customer_id: cart.customer_id.clone(),
                email: Some(email),
                currency_code: cart.currency_code.to_string(),
                region_id,
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

        let variant_ids: Vec<String> = cart
            .items
            .iter()
            .map(|item| item.variant_id.to_string())
            .collect();
        let mut variants = self.catalog.get_variants(&variant_ids).await?;
        let mut reservation_lines = Vec::new();
        for item in &cart.items {
            let variant = variants
                .remove(item.variant_id.as_str())
                .ok_or_else(|| AppError::not_found("variant", &item.variant_id))?;
            if variant.manage_inventory {
                // A variant that allows backorders is reserved like any other;
                // the flag only decides whether the shortfall fails the
                // checkout or is recorded as stock the store owes. A variant
                // with no inventory item has no ledger to record against, and
                // `reserve_lines` skips its line rather than failing the
                // checkout.
                reservation_lines.push(ReservationLinePolicy {
                    line: ReservationLine {
                        variant_id: item.variant_id.clone(),
                        quantity: item.quantity,
                        line_item_id: Some(item.id.clone()),
                    },
                    allow_backorder: variant.allow_backorder,
                });
            }
        }
        if !reservation_lines.is_empty()
            && let Err(error) = self
                .inventory
                .reserve_lines(
                    reservation_lines,
                    ReservationReference::new(checkout.id.clone()),
                    RESERVATION_TTL_SECONDS,
                )
                .await
        {
            self.abort_checkout_creation(&checkout, &error, None).await;
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
                self.abort_checkout_creation(&checkout, &error, None).await;
                return Err(error);
            }
        };

        // The collection is the money this checkout owes. It holds the one
        // session that pays it; a combination payment attaches further sessions
        // to the collection before the checkout is completed.
        let collection = match self
            .payment
            .create_collection(
                checkout.id.as_str(),
                cart.totals.total,
                cart.currency_code.as_str(),
            )
            .await
        {
            Ok(collection) => collection,
            Err(error) => {
                self.abort_checkout_creation(&checkout, &error, Some(&session.id))
                    .await;
                return Err(error);
            }
        };
        if let Err(error) = self
            .payment
            .attach_session_to_collection(collection.id.as_str(), session.id.as_str())
            .await
        {
            self.abort_checkout_creation(&checkout, &error, Some(&session.id))
                .await;
            return Err(error);
        }

        match self
            .checkout
            .attach_payment_session(&checkout.id, &session.id)
            .await
        {
            Ok(checkout) => Ok(checkout),
            Err(error) => {
                self.abort_checkout_creation(&checkout, &error, Some(&session.id))
                    .await;
                Err(error)
            }
        }
    }

    /// Releases an incomplete checkout without orphaning a provider session.
    ///
    /// Release failures are retried and logged; the reservations also carry a
    /// TTL and are swept by maintenance if both attempts fail.
    async fn abort_checkout_creation(
        &self,
        checkout: &CheckoutSession,
        error: &AppError,
        payment_session: Option<&PaymentSessionId>,
    ) {
        let latest = self.checkout.get(&checkout.id).await.ok();
        // A concurrent creator may already have attached its session. Leave
        // that checkout and its reservations alone; only clean up our own work.
        let another_runner_owns = latest.as_ref().is_some_and(|session| {
            matches!(
                session.status,
                CheckoutStatus::PaymentPending | CheckoutStatus::OrderPlaced
            )
        });

        if let Some(payment_session) = payment_session {
            let already_attached = latest
                .as_ref()
                .and_then(|session| session.payment_session_id.as_ref())
                == Some(payment_session);
            if !already_attached
                && let Err(cancel_error) =
                    self.payment.cancel_session(payment_session.as_str()).await
            {
                tracing::warn!(
                    checkout_id = %checkout.id,
                    payment_session_id = %payment_session,
                    error = %cancel_error,
                    "failed to cancel the payment session of an aborted checkout"
                );
            }
        }
        if another_runner_owns {
            return;
        }

        let mut released = false;
        for attempt in 1..=2 {
            match self.inventory.release_checkout(&checkout.id).await {
                Ok(_) => {
                    released = true;
                    break;
                }
                Err(release_error) => {
                    tracing::error!(
                        checkout_id = %checkout.id,
                        attempt,
                        error = %release_error,
                        "failed to release checkout reservations"
                    );
                }
            }
        }
        if !released {
            tracing::error!(
                checkout_id = %checkout.id,
                "checkout reservations could not be released; expiry will retry"
            );
        }

        if let Err(transition_error) = self
            .checkout
            .transition(
                &checkout.id,
                CheckoutStatus::Failed,
                Some(&error.to_string()),
            )
            .await
        {
            tracing::warn!(
                checkout_id = %checkout.id,
                error = %transition_error,
                "failed to mark an aborted checkout as failed"
            );
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
            return self.placed_result(&checkout).await;
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

        // Fence concurrent completions before cart validation, capture or any
        // order side effect. The lease bounds how long a crashed runner can
        // block retries; an expired lease is taken over by the next attempt.
        let Some(token) = self
            .checkout
            .claim_placement(checkout_id, now_timestamp(), PLACEMENT_LEASE_SECONDS)
            .await?
        else {
            let latest = self.checkout.get(checkout_id).await?;
            if latest.status == CheckoutStatus::OrderPlaced {
                return self.placed_result(&latest).await;
            }
            return Err(AppError::conflict(
                "checkout completion is already in progress; retry shortly",
            ));
        };

        let result = self
            .complete_claimed(checkout, payment_session_id, &token, context)
            .await;
        if result.is_err() {
            // Releasing after `order_placed` would be pointless because
            // `mark_order_placed` already cleared the claim; skip the write.
            let placed = matches!(
                self.checkout.get(checkout_id).await,
                Ok(latest) if latest.status == CheckoutStatus::OrderPlaced
            );
            if !placed
                && let Err(error) = self.checkout.release_placement(checkout_id, &token).await
            {
                tracing::warn!(
                    checkout_id = %checkout_id,
                    error = %error,
                    "failed to release checkout placement claim"
                );
            }
        }
        result
    }

    /// Completes a checkout whose placement claim is already held.
    async fn complete_claimed(
        &self,
        checkout: CheckoutSession,
        payment_session_id: PaymentSessionId,
        placement_token: &str,
        context: &CartContext,
    ) -> Result<CompleteCheckoutResult, AppError> {
        let checkout_id = checkout.id.clone();

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
        let mut live_items: Vec<(String, i64)> = cart
            .items
            .iter()
            .map(|item| (item.variant_id.to_string(), item.quantity))
            .collect();
        live_items.sort();
        let mut snapshot: Vec<(String, i64)> = checkout
            .cart_items
            .iter()
            .map(|item| (item.variant_id.clone(), item.quantity))
            .collect();
        snapshot.sort();
        if live_items != snapshot {
            return Err(AppError::conflict(
                "cart items changed since checkout was created",
            ));
        }

        // Claim the promotions the cart applies before charging for them. The
        // quote-time filter only decides what is offered, so the claim is what
        // makes a usage limit binding: without it two checkouts can both read
        // the same usage count, both pass, and both place an order. A promotion
        // that cannot be claimed rejects the completion here, before any money
        // moves.
        self.claim_promotion_usage(&cart, &checkout_id, placement_token)
            .await?;

        // Capture before marking the checkout as confirmed. A session that
        // looks paid but was never captured must never reach order placement.
        let (payment, combination_payments) = self
            .capture_checkout_payments(&checkout_id, &payment_session_id)
            .await?;
        if payment.status != PaymentSessionStatus::Captured {
            return Err(AppError::conflict(format!(
                "payment is in state {} and cannot complete this checkout",
                payment.status
            )));
        }

        let checkout = match checkout.status {
            CheckoutStatus::PaymentPending => {
                match self
                    .checkout
                    .transition(&checkout_id, CheckoutStatus::PaymentConfirmed, None)
                    .await
                {
                    Ok(updated) => updated,
                    Err(error) => {
                        let latest = self.checkout.get(&checkout_id).await?;
                        match latest.status {
                            CheckoutStatus::PaymentConfirmed => latest,
                            CheckoutStatus::OrderPlaced => {
                                return self.placed_result(&latest).await;
                            }
                            CheckoutStatus::Failed | CheckoutStatus::Canceled => {
                                // The checkout was expired while we captured.
                                // Refund the customer rather than orphan money.
                                self.compensate(
                                    &latest,
                                    &payment,
                                    &combination_payments,
                                    None,
                                    placement_token,
                                )
                                .await;
                                return Err(error);
                            }
                            _ => return Err(error),
                        }
                    }
                }
            }
            CheckoutStatus::PaymentConfirmed => checkout,
            _ => unreachable!("checkout status was validated above"),
        };

        // Line items carry only their own adjustments, so the cart-level part of
        // the discount has to be attributed to lines here: order placement
        // rebuilds the aggregate from these items, and the order must record the
        // amount the customer was actually charged.
        let item_discounts = fastcart_module_cart::allocate_cart_discounts(&cart)?;

        let mut items = Vec::with_capacity(cart.items.len());
        let mut items_total = 0_i64;
        for (index, item) in cart.items.iter().enumerate() {
            let line_subtotal = item
                .unit_price
                .checked_mul(item.quantity)
                .ok_or_else(|| AppError::validation("line subtotal overflow"))?;
            // The cart caps a line's own adjustments at the line's value, so
            // the order must cap them identically or the two totals diverge.
            let own_discount: i64 = item.adjustments.iter().map(|entry| entry.amount).sum();
            let discount = own_discount.max(0).min(line_subtotal)
                + item_discounts.get(index).copied().unwrap_or(0);
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

        // The per-rate breakdown is snapshotted so the collected tax can still
        // be reconciled against a rate after the cart is gone.
        let item_positions: HashMap<&str, usize> = cart
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| (item.id.as_str(), index))
            .collect();
        let tax_lines: Vec<CreateOrderTaxLineInput> = cart
            .tax_lines
            .iter()
            .map(|line| CreateOrderTaxLineInput {
                item_index: line
                    .cart_item_id
                    .as_ref()
                    .and_then(|id| item_positions.get(id.as_str()).copied()),
                shipping_option_id: line.shipping_method_id.clone(),
                name: line.name.clone(),
                rate_bps: line.rate_bps,
                amount: line.amount,
                is_tax_inclusive: line.is_tax_inclusive,
            })
            .collect();
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
        // The aggregate figures are the cart's own, and the per-item figures
        // above are built to add up to them. Refuse to place an order whose
        // stored total disagrees with the captured amount: an operator cannot
        // reconcile a payment against an order that never matched it.
        let subtotal = cart.totals.item_subtotal;
        let discount_total = cart.totals.discount_total;
        if total != cart.totals.total
            || discount_total
                != items
                    .iter()
                    .map(|item| item.discount_total.unwrap_or(0))
                    .sum::<i64>()
        {
            return Err(AppError::internal(format!(
                "order totals {total}/{discount_total} do not reconcile with the charged cart \
                 total {}/{}",
                cart.totals.total, cart.totals.discount_total
            )));
        }
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
                tax_lines,
                region_id: cart.region_id.clone(),
                sales_channel_id: cart.sales_channel_id.clone(),
                subtotal,
                discount_total,
                shipping_total,
                tax_total: cart.totals.tax_total,
                total,
                metadata: Metadata::empty(),
                idempotency_key: Some(format!("checkout:{}", checkout.id.as_str())),
            })
            .await
        {
            Ok(order) => order,
            Err(error) => {
                self.compensate(
                    &checkout,
                    &payment,
                    &combination_payments,
                    None,
                    placement_token,
                )
                .await;
                return Err(error);
            }
        };

        let order = match self
            .finish_order(
                &checkout,
                &cart,
                &payment,
                &combination_payments,
                &order,
                placement_token,
            )
            .await
        {
            Ok(order) => order,
            Err(error) => {
                self.compensate(
                    &checkout,
                    &payment,
                    &combination_payments,
                    Some(&order),
                    placement_token,
                )
                .await;
                return Err(error);
            }
        };

        self.reassign_promotion_usage(&checkout_id, order.id.as_str())
            .await;
        let checkout = self.checkout.get(&checkout.id).await?;
        Ok(CompleteCheckoutResult {
            order,
            payment,
            checkout,
        })
    }

    async fn placed_result(
        &self,
        checkout: &CheckoutSession,
    ) -> Result<CompleteCheckoutResult, AppError> {
        let order_id = checkout
            .order_id
            .clone()
            .ok_or_else(|| AppError::conflict("checkout has no order"))?;
        let payment_session_id = checkout
            .payment_session_id
            .clone()
            .ok_or_else(|| AppError::conflict("checkout has no payment session"))?;
        let order = self.order.get(&order_id).await?;
        let payment = self
            .payment
            .payment_for_session_id(payment_session_id.as_str())
            .await?;
        // A crash between placement and the claim being moved onto the order
        // leaves the usage keyed to the checkout; replaying converges here.
        self.reassign_promotion_usage(&checkout.id, order.id.as_str())
            .await;
        // A crash between placement and the collection being linked leaves the
        // collection of a placed checkout unnamed. The order is already placed
        // and paid, so a failed link is logged instead of failing the replay;
        // the next replay converges.
        if let Err(error) = self.link_collection_to_order(&checkout.id, &order.id).await {
            tracing::warn!(
                checkout_id = %checkout.id,
                order_id = %order.id,
                error = %error,
                "failed to link the checkout payment collection to its order"
            );
        }
        if let Ok(cart) = self.carts.get_cart(&checkout.cart_id).await {
            // A crash between placement and cart completion leaves an open
            // cart behind an already placed checkout. Completing it here makes
            // replay converge without touching money or stock.
            if cart.status == fastcart_module_cart::CartStatus::Open
                && let Err(error) = self.carts.complete_cart(&cart.id).await
            {
                tracing::warn!(
                    cart_id = %cart.id,
                    error = %error,
                    "failed to complete the cart of an already placed checkout"
                );
            }
        }
        Ok(CompleteCheckoutResult {
            order,
            payment,
            checkout: checkout.clone(),
        })
    }

    /// Captures the payments that settle a checkout's payment collection.
    ///
    /// Returns the payment of the checkout's own session — the one replays and
    /// the completion result report — together with the payments of any further
    /// session a combination payment was split into.
    ///
    /// A collection that is just the checkout's own session (and a checkout
    /// from before collections existed) keeps the original single-session
    /// capture: same provider idempotency key, same errors, same concurrency
    /// fence. A collection split across several sessions is captured as a
    /// collection, which only succeeds when the captures cover its amount.
    async fn capture_checkout_payments(
        &self,
        checkout_id: &CheckoutId,
        payment_session_id: &PaymentSessionId,
    ) -> Result<(Payment, Vec<Payment>), AppError> {
        let collection = self
            .payment
            .collection_for_checkout(checkout_id.as_str())
            .await?;
        let sessions = match collection.as_ref() {
            Some(collection) => {
                self.payment
                    .list_collection_sessions(collection.id.as_str())
                    .await?
            }
            None => Vec::new(),
        };
        // A combination payment is the only case that needs the collection
        // path: everything else captures the checkout's one session.
        let Some(collection) = collection.as_ref().filter(|_| sessions.len() > 1) else {
            let payment = self.payment.capture(payment_session_id.as_str()).await?;
            return Ok((payment, Vec::new()));
        };
        if !sessions
            .iter()
            .any(|session| session.id == *payment_session_id)
        {
            // The checkout's money is not part of its collection, so capturing
            // the collection would place an order the customer never paid for.
            return Err(AppError::conflict(
                "checkout payment collection does not hold the checkout's payment session",
            ));
        }
        let captured = self
            .payment
            .capture_collection(collection.id.as_str())
            .await?;
        let payment = self
            .payment
            .payment_for_session_id(payment_session_id.as_str())
            .await?;
        let additional = captured
            .into_iter()
            .filter(|candidate| candidate.id != payment.id)
            .collect();
        Ok((payment, additional))
    }

    /// Links the checkout's payment collection to the order it paid for.
    ///
    /// The collection is the money the checkout owed, and the order is where
    /// that money ends up. A checkout with no collection — one from before
    /// collections existed — has nothing to link. The link is idempotent, so a
    /// replay converges on the same record instead of failing.
    async fn link_collection_to_order(
        &self,
        checkout_id: &CheckoutId,
        order_id: &OrderId,
    ) -> Result<(), AppError> {
        let Some(collection) = self
            .payment
            .collection_for_checkout(checkout_id.as_str())
            .await?
        else {
            return Ok(());
        };
        self.payment
            .assign_collection_to_order(collection.id.as_str(), order_id)
            .await?;
        Ok(())
    }

    async fn finish_order(
        &self,
        checkout: &CheckoutSession,
        cart: &fastcart_module_cart::Cart,
        payment: &Payment,
        combination_payments: &[Payment],
        order: &Order,
        placement_token: &str,
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

        // Every payment of the collection pays for this order; a combination
        // payment leaves several payments behind.
        self.payment
            .assign_order(payment.id.as_str(), &order.id)
            .await?;
        for additional in combination_payments {
            self.payment
                .assign_order(additional.id.as_str(), &order.id)
                .await?;
        }
        // The collection is the money this order owes, so from here on it must
        // name the order: a later charge, refund or report finds it that way.
        self.link_collection_to_order(&checkout.id, &order.id)
            .await?;
        let mut order = self.order.get(&order.id).await?;
        if order.payment_status != OrderPaymentStatus::Captured {
            match self
                .order
                .set_payment_status(&order.id, OrderPaymentStatus::Captured)
                .await
            {
                Ok(updated) => order = updated,
                Err(error) => {
                    let latest = self.order.get(&order.id).await?;
                    if latest.payment_status == OrderPaymentStatus::Captured {
                        order = latest;
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        // The reservations were recorded for the cart lines, but the order
        // assigned fresh identifiers to those lines. Binding every reservation
        // to the order line it was placed as — before the commit, so a retry
        // after a crash converges on the same mapping — is what keeps the stock
        // a line consumed, and the location it came from, traceable from the
        // order.
        let bindings = reservation_rebindings(cart, &order)?;
        self.inventory
            .rebind_reservations(&checkout.id, &order.id, &bindings)
            .await?;

        // Commit inventory before confirming the order. `confirm` moves the
        // order into a state that compensation must not cancel, so a later
        // `commit` failure (for example because reservations expired while the
        // customer paid) would leave a captured payment and a confirmed order
        // with no stock movement. With the commit first, a failed commit is
        // still compensable: the order is pending, so the refund path runs.
        self.inventory.commit(&checkout.id, &order.id).await?;

        if order.status == OrderStatus::Pending {
            match self.order.confirm(&order.id).await {
                Ok(confirmed) => order = confirmed,
                Err(error) => {
                    // Another runner may have confirmed the same order between
                    // our read and this call. That is success, not a reason to
                    // compensate a good order.
                    let latest = self.order.get(&order.id).await?;
                    if is_placed_order_status(latest.status) {
                        order = latest;
                    } else {
                        return Err(error);
                    }
                }
            }
        }

        if let Err(error) = self
            .checkout
            .mark_order_placed_with_claim(&checkout.id, &order.id, placement_token)
            .await
        {
            // The claim may have been lost to a takeover, but if that runner
            // placed the same order this attempt still succeeded.
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
            // The order is already placed and inventory committed; a cart that
            // expired (or a transient failure here) must not trigger compensation.
            let latest = self.carts.get_cart(&cart.id).await?;
            if latest.status != fastcart_module_cart::CartStatus::Completed
                && latest.status != fastcart_module_cart::CartStatus::Expired
            {
                tracing::warn!(
                    cart_id = %cart.id,
                    error = %error,
                    "cart completion failed after the order was placed"
                );
            }
        }
        Ok(order)
    }

    /// Undoes a failed completion attempt.
    ///
    /// Compensation is fenced by the placement token: a runner whose lease was
    /// taken over must not release reservations or refund a payment that the
    /// new owner is still placing. A terminal checkout (`failed` / `canceled`)
    /// can still hold captured money, so it is refunded even though it no
    /// longer carries a claim.
    async fn compensate(
        &self,
        checkout: &CheckoutSession,
        payment: &Payment,
        combination_payments: &[Payment],
        order: Option<&Order>,
        placement_token: &str,
    ) {
        // Re-read persisted state before undoing anything. A concurrent runner
        // may have placed the order after this runner observed a failure; only
        // compensate when the checkout is still unplaced.
        let latest_checkout = match self.checkout.get(&checkout.id).await {
            Ok(latest) => Some(latest),
            Err(error) => {
                tracing::warn!(
                    checkout_id = %checkout.id,
                    error = %error,
                    "could not re-read checkout before compensation"
                );
                None
            }
        };
        let Some(latest_checkout) = latest_checkout else {
            return;
        };
        if latest_checkout.status == CheckoutStatus::OrderPlaced
            || order.is_some_and(|order| latest_checkout.order_id.as_ref() == Some(&order.id))
        {
            return;
        }
        if !latest_checkout.status.is_terminal() {
            match self
                .checkout
                .holds_placement(&checkout.id, placement_token)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(
                        checkout_id = %checkout.id,
                        "checkout placement was taken over; skipping compensation"
                    );
                    return;
                }
                Err(error) => {
                    tracing::warn!(
                        checkout_id = %checkout.id,
                        error = %error,
                        "could not verify the placement claim; skipping compensation"
                    );
                    return;
                }
            }
        }

        let mut refundable = true;
        if let Some(order) = order {
            match self.order.get(&order.id).await {
                // Another runner already compensated this order.
                Ok(latest) if latest.status == OrderStatus::Canceled => {}
                Ok(latest)
                    if matches!(latest.status, OrderStatus::Pending | OrderStatus::Confirmed) =>
                {
                    if let Err(error) = self.inventory.restock_order(&order.id).await {
                        tracing::warn!(
                            order_id = %order.id,
                            error = %error,
                            "checkout compensation could not restock inventory"
                        );
                    }
                    if let Err(error) = self
                        .order
                        .cancel(&order.id, Some("checkout compensation"))
                        .await
                    {
                        tracing::warn!(
                            order_id = %order.id,
                            error = %error,
                            "checkout compensation could not cancel the order"
                        );
                    }
                    if let Err(error) = self
                        .order
                        .set_payment_status(&order.id, OrderPaymentStatus::Refunded)
                        .await
                    {
                        tracing::warn!(
                            order_id = %order.id,
                            error = %error,
                            "checkout compensation could not mark the order refunded"
                        );
                    }
                }
                Ok(latest) => {
                    // The order moved into fulfilment: refunding and canceling
                    // it automatically would undo an operator's decision.
                    tracing::error!(
                        order_id = %order.id,
                        status = latest.status.as_str(),
                        "order cannot be compensated automatically; manual reconciliation is required"
                    );
                    refundable = false;
                }
                Err(error) => {
                    tracing::warn!(
                        order_id = %order.id,
                        error = %error,
                        "could not re-read order before compensation"
                    );
                    // Refunding without knowing whether the order is still
                    // pending could leave a paid, active order behind.
                    refundable = false;
                }
            }
        }
        if !refundable {
            return;
        }

        let _ = self.inventory.release_checkout(&checkout.id).await;
        // The claims were taken before the capture, under the same placement
        // fence as the refund above, and are given back with it: the customer
        // never got the discount, so the promotion counters must not keep it.
        self.release_promotion_usage(&checkout.id).await;
        let refunded = self
            .refund_compensation(checkout, payment, combination_payments)
            .await;

        // Mark the checkout failed only once the money is back with the
        // customer. A failed refund keeps the session retryable so a later
        // attempt can finish the compensation instead of hiding captured money
        // behind a terminal state.
        if refunded
            && matches!(
                latest_checkout.status,
                CheckoutStatus::PaymentPending | CheckoutStatus::PaymentConfirmed
            )
            && let Err(error) = self
                .checkout
                .transition(
                    &checkout.id,
                    CheckoutStatus::Failed,
                    Some("checkout compensation"),
                )
                .await
        {
            tracing::warn!(
                checkout_id = %checkout.id,
                error = %error,
                "failed to mark the compensated checkout as failed"
            );
        }
    }

    /// Refunds every payment a failed completion captured.
    ///
    /// A single payment keeps the historical refund key, so an earlier build
    /// retrying the same compensation still deduplicates against it. A
    /// combination payment keys every refund separately: one key identifies one
    /// logical refund, and these are different money.
    async fn refund_compensation(
        &self,
        checkout: &CheckoutSession,
        payment: &Payment,
        combination_payments: &[Payment],
    ) -> bool {
        let refunds: Vec<(&Payment, String)> = if combination_payments.is_empty() {
            vec![(
                payment,
                format!("checkout-compensation:{}", checkout.id.as_str()),
            )]
        } else {
            std::iter::once(payment)
                .chain(combination_payments.iter())
                .map(|candidate| {
                    (
                        candidate,
                        format!(
                            "checkout-compensation:{}:{}",
                            checkout.id.as_str(),
                            candidate.id.as_str()
                        ),
                    )
                })
                .collect()
        };
        let mut refunded = true;
        for (candidate, refund_key) in refunds {
            if let Err(error) = self
                .payment
                .refund_with_idempotency(
                    candidate.id.as_str(),
                    candidate.amount,
                    Some("checkout compensation".to_owned()),
                    Some(refund_key),
                )
                .await
            {
                tracing::error!(
                    payment_id = %candidate.id,
                    error = %error,
                    "checkout compensation refund failed; manual reconciliation is required"
                );
                refunded = false;
            }
        }
        refunded
    }

    /// Promotions the cart applied, with the discount each one granted.
    ///
    /// Keyed by promotion id and ordered, so the claim order is deterministic.
    /// Only the adjustments that were actually granted are listed: a promotion
    /// the calculator dropped is not charged anything.
    fn cart_promotion_discounts(cart: &fastcart_module_cart::Cart) -> BTreeMap<String, i64> {
        let mut promotions: BTreeMap<String, i64> = BTreeMap::new();
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
        // An order-target promotion is applied to the cart itself and leaves no
        // adjustment behind, so its cart-level row is the only record that it
        // was used. A row that granted nothing — the marker a coupon leaves
        // before anything applies — is not a use and stays out of the claim.
        for discount in &cart.discounts {
            if discount.amount <= 0 {
                continue;
            }
            if let Some(promotion_id) = discount.promotion_id.as_ref() {
                *promotions.entry(promotion_id.to_string()).or_default() += discount.amount;
            }
        }
        promotions
    }

    /// Coupon that unlocked a promotion on this cart, when the customer used one.
    async fn coupon_for_promotion(
        &self,
        cart: &fastcart_module_cart::Cart,
        promotion_id: &str,
    ) -> Option<String> {
        let code = cart
            .discounts
            .iter()
            .find(|discount| {
                discount
                    .promotion_id
                    .as_ref()
                    .is_some_and(|id| id.as_str() == promotion_id)
            })
            .and_then(|discount| discount.code.clone())?;
        match self.promotion.validate_coupon(&code, now_timestamp()).await {
            Ok(coupon) => Some(coupon.id),
            Err(_) => None,
        }
    }

    /// Atomically claims one usage per promotion the cart applies.
    ///
    /// Runs before the capture, so a promotion whose limit, per-customer cap or
    /// campaign budget is used up by a competing checkout rejects the
    /// completion instead of over-issuing the discount. Everything claimed
    /// before the failing one is given back, while this runner still holds the
    /// placement claim.
    async fn claim_promotion_usage(
        &self,
        cart: &fastcart_module_cart::Cart,
        checkout_id: &CheckoutId,
        placement_token: &str,
    ) -> Result<(), AppError> {
        let key = checkout_usage_key(checkout_id.as_str());
        let customer_id = cart.customer_id.as_ref().map(ToString::to_string);
        let currency_code = cart.currency_code.to_string();
        let mut claimed = false;
        for (promotion_id, amount) in Self::cart_promotion_discounts(cart) {
            let coupon_id = self.coupon_for_promotion(cart, &promotion_id).await;
            match self
                .promotion
                .reserve_usage(
                    &promotion_id,
                    coupon_id.as_deref(),
                    customer_id.as_deref(),
                    checkout_id.as_str(),
                    amount,
                    Some(&currency_code),
                )
                .await
            {
                Ok(_) => claimed = true,
                Err(error) => {
                    if claimed {
                        self.rollback_promotion_usage(&key, checkout_id, placement_token)
                            .await;
                    }
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Gives back the claims of a completion that is about to fail.
    ///
    /// Fenced by the placement claim, like compensation: a runner whose lease
    /// was taken over must not release the claims the new owner is working with.
    async fn rollback_promotion_usage(
        &self,
        key: &str,
        checkout_id: &CheckoutId,
        placement_token: &str,
    ) {
        match self
            .checkout
            .holds_placement(checkout_id, placement_token)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(
                    checkout_id = %checkout_id,
                    "checkout placement was taken over; skipping the promotion claim rollback"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    checkout_id = %checkout_id,
                    error = %error,
                    "could not verify the placement claim; skipping the promotion claim rollback"
                );
                return;
            }
        }
        match self.promotion.release_usage(key).await {
            Ok(released) => tracing::info!(
                checkout_id = %checkout_id,
                released,
                "released the promotion claims of a rejected completion"
            ),
            Err(error) => tracing::error!(
                checkout_id = %checkout_id,
                error = %error,
                "failed to release the promotion claims of a rejected completion; the counters stay consumed"
            ),
        }
    }

    /// Moves the checkout's promotion claims onto the order that was placed.
    ///
    /// The counters are already charged, so this only renames the key. If it
    /// fails the order stands and the usage stays keyed to the checkout: the
    /// counts are right, only the key did not move, and a replay retries it.
    async fn reassign_promotion_usage(&self, checkout_id: &CheckoutId, order_id: &str) {
        let key = checkout_usage_key(checkout_id.as_str());
        if let Err(error) = self.promotion.reassign_usage(&key, order_id).await {
            tracing::error!(
                checkout_id = %checkout_id,
                order_id = %order_id,
                error = %error,
                "failed to move the promotion usage onto the placed order"
            );
        }
    }

    /// Gives the promotion claims of a compensated checkout back to its counters.
    async fn release_promotion_usage(&self, checkout_id: &CheckoutId) {
        let key = checkout_usage_key(checkout_id.as_str());
        match self.promotion.release_usage(&key).await {
            Ok(released) => tracing::info!(
                checkout_id = %checkout_id,
                released,
                "released the promotion claims of a compensated checkout"
            ),
            Err(error) => tracing::error!(
                checkout_id = %checkout_id,
                error = %error,
                "failed to release the promotion claims of a compensated checkout; manual reconciliation is required"
            ),
        }
    }
}

/// Pairs every cart line with the order line it was placed as.
///
/// The pairing is positional: the order is created from the items built out of
/// `cart.items` in order, `OrderService::create_order` writes them in that order
/// and reads them back by rowid, so `order.items[i]` is the line
/// `cart.items[i]` became. The variant of each pair is compared so a mismatch
/// fails loudly instead of binding a reservation to the wrong line.
fn reservation_rebindings(
    cart: &fastcart_module_cart::Cart,
    order: &Order,
) -> Result<Vec<ReservationRebinding>, AppError> {
    if cart.items.len() != order.items.len() {
        return Err(AppError::internal(format!(
            "order {} has {} lines but its cart has {}",
            order.id,
            order.items.len(),
            cart.items.len()
        )));
    }
    cart.items
        .iter()
        .zip(order.items.iter())
        .map(|(cart_item, order_item)| {
            if order_item.variant_id.as_ref() != Some(&cart_item.variant_id) {
                return Err(AppError::internal(format!(
                    "order line {} does not match the cart line {} it was placed from",
                    order_item.id, cart_item.id
                )));
            }
            Ok(ReservationRebinding::new(
                cart_item.id.clone(),
                order_item.id.clone(),
            ))
        })
        .collect()
}

/// Returns true when an order has been accepted into fulfillment.
fn is_placed_order_status(status: OrderStatus) -> bool {
    matches!(
        status,
        OrderStatus::Confirmed
            | OrderStatus::Processing
            | OrderStatus::Fulfilled
            | OrderStatus::Completed
    )
}
