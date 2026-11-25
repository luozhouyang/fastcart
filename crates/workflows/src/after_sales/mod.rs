//! After-sales orchestration: refunds for returns and claims, plus inventory
//! restocking when returned goods are received.
//!

//! The workflow owns the cross-module idempotency boundary. Provider refunds
//! use a stable key per after-sales record, order refunds use the same key as a
//! local idempotency marker, and restock movements use a deterministic
//! reference. Replaying any step therefore never moves money or stock twice.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CurrencyCode, LineItemId, Metadata, OrderId, Pagination, VariantId, now_timestamp,
};
use fastcart_module_customer::CustomerService;
use fastcart_module_inventory::InventoryService;
use fastcart_module_order::{
    AfterSalesFulfillment, Order, OrderClaim, OrderEdit, OrderExchange, OrderRefund, OrderReturn,
    OrderService, RefundInput, ReplacementUnitPrice,
};
use fastcart_module_payment::{
    CreatePaymentSessionInput, ListPaymentsFilter, Payment, PaymentCollection, PaymentService,
    PaymentSession, PaymentStatus, Refund,
};
use fastcart_module_pricing::{PriceContext, PricingService};
use fastcart_module_shipping::{
    CreateFulfillmentRequest, FulfillmentItemRequest, FulfillmentProviderRegistry,
};
use serde_json::Value;

mod claims;
mod differences;
mod edits;
mod exchanges;
mod returns;

/// Prefix for provider and local idempotency keys of return refunds.
const RETURN_REFUND_PREFIX: &str = "return-refund";
/// Prefix for provider and local idempotency keys of claim refunds.
const CLAIM_REFUND_PREFIX: &str = "claim-refund";
/// Prefix for the payable and refund keys of an order edit's difference.
const ORDER_EDIT_DIFFERENCE_PREFIX: &str = "order-edit-difference";

/// The money one order has moved.
///
/// `outstanding` is positive when the customer still owes money and negative
/// when the merchant owes a refund, mirroring the way Medusa compares an
/// order's total against its transactions. It is derived from captured and
/// refunded money rather than from a collection amount, because a collection
/// amount can be raised for a difference that was never paid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderPaymentSummary {
    /// Order the summary belongs to.
    pub order_id: OrderId,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Current order total in minor units.
    pub total: i64,
    /// Money actually captured for the order.
    pub captured_total: i64,
    /// Money refunded, including refunds still pending at the provider.
    pub refunded_total: i64,
    /// Captured minus refunded.
    pub net_paid: i64,
    /// Order total minus net paid: positive is owed by the customer, negative
    /// is owed back to them.
    pub outstanding: i64,
}

impl OrderPaymentSummary {
    /// Money owed back to the customer, when the merchant has more than the
    /// order totals.
    #[must_use]
    pub fn refund_due(&self) -> i64 {
        self.outstanding.saturating_neg().max(0)
    }
}

/// Result of confirming an order edit.
#[derive(Clone, Debug)]
pub struct OrderEditConfirmation {
    /// The confirmed edit.
    pub order_edit: OrderEdit,
    /// The order aggregate after the edit.
    pub order: Order,
}

/// Result of refunding a return.
#[derive(Clone, Debug)]
pub struct ReturnRefundResult {
    /// Order aggregate after the refund was recorded.
    pub order: Order,
    /// Return with its `refund_id` link populated.
    pub order_return: OrderReturn,
    /// Order refund row created for this return, when one was needed.
    pub order_refund: Option<OrderRefund>,
    /// Provider-side refund, when a payment refund was attempted.
    pub provider_refund: Option<Refund>,
}

/// Result of refunding an accepted claim.
#[derive(Clone, Debug)]
pub struct ClaimRefundResult {
    /// Order aggregate after the refund was recorded.
    pub order: Order,
    /// Accepted claim with its `refund_id` link populated.
    pub claim: OrderClaim,
    /// Order refund row created for the claim's refund-type lines.
    pub order_refund: Option<OrderRefund>,
    /// Provider-side refund, when a payment refund was attempted.
    pub provider_refund: Option<Refund>,
}

/// Session metadata key that records which settled difference a session pays.
///
/// A replay looks for it before opening another session, so collecting the same
/// difference twice cannot leave the order with two payables for one charge.
const DIFFERENCE_SESSION_MARKER: &str = "after_sales_difference";
/// Prefix for provider and local idempotency keys of exchange difference refunds.
const EXCHANGE_DIFFERENCE_REFUND_PREFIX: &str = "exchange-difference";
/// Prefix for provider and local idempotency keys of claim difference refunds.
const CLAIM_DIFFERENCE_REFUND_PREFIX: &str = "claim-difference";

/// Result of making a settled difference payable by the customer.
#[derive(Clone, Debug)]
pub struct DifferenceCollectionResult {
    /// Order aggregate the difference belongs to.
    pub order: Order,
    /// Collection that now expects the difference.
    pub collection: PaymentCollection,
    /// Session that pays the difference.
    pub session: PaymentSession,
}

/// Result of refunding the difference an accepted record owes the customer.
#[derive(Clone, Debug)]
pub struct DifferenceRefundResult {
    /// Order aggregate after the refund.
    pub order: Order,
    /// Order refund row created for this difference, when one was needed.
    pub order_refund: Option<OrderRefund>,
    /// Provider-side refund, when a payment refund was attempted.
    pub provider_refund: Option<Refund>,
}

/// Result of receiving returned goods and restocking them.
#[derive(Clone, Debug)]
pub struct ReceiveReturnResult {
    /// Return after the receive transition.
    pub order_return: OrderReturn,
    /// Units actually placed back into inventory by this call.
    pub restocked_units: i64,
}

/// Result of fulfilling the replacement lines of a claim.
#[derive(Clone, Debug)]
pub struct ClaimFulfillmentResult {
    /// Order aggregate after the fulfillment.
    pub order: Order,
    /// Accepted claim the fulfillment belongs to.
    pub claim: OrderClaim,
    /// Persisted after-sales fulfillment, when the claim had replacement lines.
    pub fulfillment: Option<AfterSalesFulfillment>,
    /// Replacement units consumed from inventory by this call.
    pub consumed_units: i64,
}

/// Result of accepting an exchange and restocking its inbound goods.
#[derive(Clone, Debug)]
pub struct ExchangeAcceptResult {
    /// Order aggregate after the transition.
    pub order: Order,
    /// Accepted exchange.
    pub exchange: OrderExchange,
    /// Inbound units placed back into inventory by this call.
    pub restocked_units: i64,
}

/// Result of fulfilling the outbound lines of an exchange.
#[derive(Clone, Debug)]
pub struct ExchangeFulfillmentResult {
    /// Order aggregate after the fulfillment.
    pub order: Order,
    /// Accepted exchange the fulfillment belongs to.
    pub exchange: OrderExchange,
    /// Persisted after-sales fulfillment, when the exchange had outbound lines.
    pub fulfillment: Option<AfterSalesFulfillment>,
    /// Replacement units consumed from inventory by this call.
    pub consumed_units: i64,
}

/// Orchestrates order, payment, pricing and inventory services for after-sales
/// flows.
pub struct AfterSalesWorkflow {
    order: OrderService,
    payment: PaymentService,
    pricing: PricingService,
    inventory: InventoryService,
    customer: CustomerService,
    shipping: FulfillmentProviderRegistry,
}

impl AfterSalesWorkflow {
    /// Creates a workflow bound to one store using the default fulfillment
    /// providers.
    ///
    /// The payment service must be constructed with the provider registry used
    /// by the process, exactly like [`crate::CheckoutWorkflow`].
    #[must_use]
    pub fn new(db: StoreDatabase, payment: PaymentService) -> Self {
        Self::with_shipping_providers(db, payment, FulfillmentProviderRegistry::with_defaults())
    }

    /// Creates a workflow with an explicit fulfillment provider registry.
    #[must_use]
    pub fn with_shipping_providers(
        db: StoreDatabase,
        payment: PaymentService,
        shipping: FulfillmentProviderRegistry,
    ) -> Self {
        Self {
            order: OrderService::new(db.clone()),
            payment,
            pricing: PricingService::new(db.clone()),
            inventory: InventoryService::new(db.clone()),
            customer: CustomerService::new(db),
            shipping,
        }
    }

    /// Records workflow-owned mutations in the store audit outbox.
    ///
    /// The actor is propagated to the order and inventory services so their
    /// audit rows commit with the mutation they describe.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.order = self.order.with_audit_actor(actor_id.clone());
        self.inventory = self.inventory.with_audit_actor(actor_id);
        self
    }
}

impl AfterSalesWorkflow {
    /// Resolves the replacement unit prices of one after-sales record.
    ///
    /// The order module owns no catalog prices, so it cannot price a
    /// replacement by itself: the workflow resolves the price in the order's
    /// currency and hands it to `OrderService::accept_*`, which settles the
    /// difference against the line being replaced. A replacement variant with
    /// no eligible price is left out, because there is no known difference to
    /// settle for it.
    async fn replacement_prices(
        &self,
        order: &Order,
        replacements: &[(&LineItemId, Option<&VariantId>, i64)],
    ) -> Result<Vec<ReplacementUnitPrice>, AppError> {
        let currency = CurrencyCode::new(order.currency_code.clone())?;
        let at = now_timestamp();
        // The replacement belongs to the customer who is exchanging, so it is
        // priced for them: the order itself cannot carry this (the customer's
        // groups are not part of its aggregate), so the workflow looks them up.
        // Groups come back ordered by name, and the first one scopes the price,
        // matching how cart pricing selects a group.
        let customer_group_id = match order.customer_id.as_ref() {
            Some(customer_id) => self
                .customer
                .list_customer_groups(customer_id)
                .await?
                .first()
                .map(|group| group.id.clone()),
            None => None,
        };
        let mut prices = Vec::with_capacity(replacements.len());
        for (order_item_id, variant_id, quantity) in replacements {
            let Some(variant_id) = variant_id else {
                continue;
            };
            let context = PriceContext {
                currency: currency.clone(),
                quantity: *quantity,
                customer_group_id: customer_group_id.clone(),
                sales_channel_id: order.sales_channel_id.clone(),
                at,
            };
            if let Some(selected) = self
                .pricing
                .calculate_price(variant_id.as_str(), &context)
                .await?
            {
                prices.push(ReplacementUnitPrice {
                    order_item_id: (*order_item_id).clone(),
                    unit_price: selected.amount,
                });
            }
        }
        Ok(prices)
    }

    async fn create_provider_fulfillment(
        &self,
        order_id: &OrderId,
        provider_id: &str,
        idempotency_key: Option<String>,
        tracking_numbers: Vec<String>,
        items: impl Iterator<Item = FulfillmentItemRequest>,
    ) -> Result<fastcart_module_shipping::FulfillmentReceipt, AppError> {
        let provider = self
            .shipping
            .get(provider_id)
            .ok_or_else(|| AppError::not_found("fulfillment provider", provider_id))?;
        provider
            .create_fulfillment(CreateFulfillmentRequest {
                order_id: order_id.as_str().to_owned(),
                items: items.collect(),
                tracking_numbers,
                provider_data: serde_json::json!({}),
                idempotency_key,
                metadata: Metadata::empty(),
            })
            .await
    }

    /// Raises the order's collection by a settled difference and prices a
    /// session to pay it.
    ///
    /// Only a positive difference is money the customer owes; a negative one is
    /// paid back through [`Self::refund_exchange_difference`] instead. An order
    /// holds one collection, so the difference joins the one the checkout
    /// opened — or opens the order's first collection when it has none, which
    /// is the case for an order placed outside a checkout.
    async fn collect_difference(
        &self,
        order_id: &OrderId,
        difference: i64,
        prefix: &str,
        source_id: &str,
        provider: &str,
    ) -> Result<DifferenceCollectionResult, AppError> {
        if difference <= 0 {
            return Err(AppError::conflict(format!(
                "a difference of {difference} is owed to the customer, not by them"
            )));
        }
        let order = self.order.get(order_id).await?;
        if order.is_draft_order() {
            return Err(AppError::conflict(
                "a draft order has no settled difference; convert it before collecting money",
            ));
        }
        let marker = format!("{prefix}:{source_id}");
        let (collection, opened) = match self.payment.collection_for_order(order_id).await? {
            Some(existing) => (existing, false),
            None => (
                self.payment
                    .create_collection_for_order(order_id, difference, &order.currency_code)
                    .await?,
                true,
            ),
        };
        // A session already carrying this marker collects this difference: the
        // payable exists and nothing may be raised or opened a second time.
        let sessions = self
            .payment
            .list_collection_sessions(collection.id.as_str())
            .await?;
        if let Some(existing) = sessions.iter().find(|session| {
            session
                .data
                .get(DIFFERENCE_SESSION_MARKER)
                .is_some_and(|value| value.as_str() == Some(marker.as_str()))
        }) {
            return Ok(DifferenceCollectionResult {
                order,
                collection,
                session: existing.clone(),
            });
        }
        let collection = if opened {
            collection
        } else {
            self.payment
                .raise_collection_amount(collection.id.as_str(), difference)
                .await?
        };
        let mut metadata = Metadata::empty();
        metadata.insert(DIFFERENCE_SESSION_MARKER, Value::String(marker));
        let session = self
            .payment
            .create_session(CreatePaymentSessionInput {
                cart_id: None,
                checkout_id: None,
                order_id: Some(order_id.clone()),
                provider: provider.to_owned(),
                amount: difference,
                currency_code: order.currency_code.clone(),
                metadata,
            })
            .await?;
        self.payment
            .attach_session_to_collection(collection.id.as_str(), session.id.as_str())
            .await?;
        Ok(DifferenceCollectionResult {
            order,
            collection,
            session,
        })
    }

    /// Refunds a settled difference that is owed to the customer.
    ///
    /// Only a negative difference is money the customer is owed. A positive one
    /// is money the merchant is owed, and no card can be charged without the
    /// customer's say-so, so it is refused here rather than silently ignored.
    /// The amount is capped at what the order still holds, and the refund is
    /// keyed by the record it settles, so a replay converges on one refund.
    async fn refund_difference(
        &self,
        order_id: &OrderId,
        difference: i64,
        prefix: &str,
        source_id: &str,
        reason: Option<String>,
    ) -> Result<DifferenceRefundResult, AppError> {
        if difference >= 0 {
            return Err(AppError::conflict(format!(
                "a difference of {difference} is owed by the customer, not to them"
            )));
        }
        let order = self.order.get(order_id).await?;
        if order.is_draft_order() {
            return Err(AppError::conflict(
                "a draft order has no settled difference; convert it before refunding money",
            ));
        }
        let key = format!("{prefix}:{source_id}");
        let local = local_key(&key);
        if let Some(existing) = self.order.refund_id_by_idempotency_key(&local).await? {
            let order_refund = order
                .refunds
                .iter()
                .find(|entry| entry.id == existing)
                .cloned();
            let provider_refund = self.provider_refund_by_key(order_id, &key).await;
            return Ok(DifferenceRefundResult {
                order,
                order_refund,
                provider_refund,
            });
        }
        // Cap by money the provider actually still holds, not by the order
        // total: a cheaper order edit lowers the total, and the refund it owes
        // is measured against what was captured, which the lowered total may no
        // longer cover.
        let captured = self.payment.captured_total_for_order(order_id).await?;
        let refunded = self.payment.refunded_total_for_order(order_id).await?;
        let refundable = captured.saturating_sub(refunded).max(0);
        let amount = difference.saturating_neg().min(refundable);
        if amount <= 0 {
            return Ok(DifferenceRefundResult {
                order,
                order_refund: None,
                provider_refund: None,
            });
        }
        let payment = self.captured_payment(order_id).await?;
        let provider_refund = self
            .payment
            .refund_with_idempotency(
                payment.id.as_str(),
                amount,
                reason.clone(),
                Some(key.clone()),
            )
            .await?;
        let (order, refund_id) = self
            .order
            .add_refund_idempotent_capped(
                order_id,
                RefundInput {
                    payment_id: payment.id.clone(),
                    amount,
                    currency: order.currency_code.clone(),
                    reason,
                },
                &local,
                // The order total may be below what the customer paid once an
                // edit lowered it; captured money is the real ceiling.
                Some(captured),
            )
            .await?;
        let order_refund = order
            .refunds
            .iter()
            .find(|entry| entry.id == refund_id)
            .cloned();
        Ok(DifferenceRefundResult {
            order,
            order_refund,
            provider_refund: Some(provider_refund),
        })
    }

    async fn captured_payment(&self, order_id: &OrderId) -> Result<Payment, AppError> {
        let page = self
            .payment
            .list_payments(ListPaymentsFilter {
                order_id: Some(order_id.clone()),
                status: None,
                pagination: Pagination::default(),
            })
            .await?;
        page.data
            .into_iter()
            .find(|payment| {
                matches!(
                    payment.status,
                    PaymentStatus::Captured | PaymentStatus::PartiallyRefunded
                )
            })
            .ok_or_else(|| {
                AppError::conflict("order has no refundable captured payment".to_owned())
            })
    }

    async fn provider_refund_by_key(&self, order_id: &OrderId, key: &str) -> Option<Refund> {
        let page = self
            .payment
            .list_payments(ListPaymentsFilter {
                order_id: Some(order_id.clone()),
                status: None,
                pagination: Pagination::default(),
            })
            .await
            .ok()?;
        for payment in page.data {
            let refunds = self.payment.list_refunds(payment.id.as_str()).await.ok()?;
            if let Some(refund) = refunds
                .into_iter()
                .find(|refund| refund.idempotency_key.as_deref() == Some(key))
            {
                return Some(refund);
            }
        }
        None
    }
}

/// Local key stored in `idempotency_keys`, namespaced away from provider keys.
fn local_key(key: &str) -> String {
    format!("after-sales:{key}")
}

/// Sums the proportional share of immutable line totals for returned units.
fn refundable_amount(
    order: &Order,
    requested: impl Iterator<Item = (LineItemId, i64)>,
) -> Result<i64, AppError> {
    let mut amount = 0_i64;
    for (order_item_id, quantity) in requested {
        if quantity <= 0 {
            continue;
        }
        let item = order
            .items
            .iter()
            .find(|item| item.id == order_item_id)
            .ok_or_else(|| AppError::not_found("order item", &order_item_id))?;
        if quantity > item.quantity {
            return Err(AppError::conflict(format!(
                "cannot refund {quantity} of item {order_item_id}: only {} were purchased",
                item.quantity
            )));
        }
        let proportional = item
            .total
            .checked_mul(quantity)
            .ok_or_else(|| AppError::validation("refund amount overflow"))?
            / item.quantity;
        amount = amount
            .checked_add(proportional)
            .ok_or_else(|| AppError::validation("refund amount overflow"))?;
    }
    Ok(amount)
}

/// Caps a computed refund by the order total minus refunds already recorded.
fn cap_refund(order: &Order, amount: i64) -> i64 {
    if amount <= 0 {
        return 0;
    }
    let refunded: i64 = order.refunds.iter().map(|refund| refund.amount).sum();
    let remaining = order.total.saturating_sub(refunded).max(0);
    amount.min(remaining)
}
