//! After-sales orchestration: refunds for returns and claims, plus inventory
//! restocking when returned goods are received.
//!
//! The workflow owns the cross-module idempotency boundary. Provider refunds
//! use a stable key per after-sales record, order refunds use the same key as a
//! local idempotency marker, and restock movements use a deterministic
//! reference. Replaying any step therefore never moves money or stock twice.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, ClaimId, ExchangeId, LineItemId, Metadata, OrderId, Pagination, ReturnId,
};
use fastcart_module_inventory::{
    ExchangeRestockLine, InventoryService, RestockAudit, ReturnRestockLine, StockConsumption,
};
use fastcart_module_order::{
    AfterSalesFulfillment, AfterSalesFulfillmentItemInput, ClaimItemType, ClaimStatus,
    CreateAfterSalesFulfillmentInput, ExchangeItemDirection, ExchangeItemStatus, ExchangeStatus,
    Order, OrderClaim, OrderExchange, OrderRefund, OrderReturn, OrderService, ReceiveReturnInput,
    RefundInput, ReturnStatus,
};
use fastcart_module_payment::{ListPaymentsFilter, Payment, PaymentService, PaymentStatus, Refund};
use fastcart_module_shipping::{
    CreateFulfillmentRequest, FulfillmentItemRequest, FulfillmentProviderRegistry,
};

/// Prefix for provider and local idempotency keys of return refunds.
const RETURN_REFUND_PREFIX: &str = "return-refund";
/// Prefix for provider and local idempotency keys of claim refunds.
const CLAIM_REFUND_PREFIX: &str = "claim-refund";

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

/// Orchestrates order, payment and inventory services for after-sales flows.
pub struct AfterSalesWorkflow {
    order: OrderService,
    payment: PaymentService,
    inventory: InventoryService,
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
            inventory: InventoryService::new(db),
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

    /// Refunds the requested quantities of a return.
    ///
    /// The refundable amount is proportional to the immutable order line
    /// totals and capped by the order total minus refunds already recorded.
    /// Repeating the call returns the existing link without moving money
    /// again.
    pub async fn refund_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        reason: Option<String>,
    ) -> Result<ReturnRefundResult, AppError> {
        let order = self.order.get(order_id).await?;
        let order_return = self.order.get_return(order_id, return_id).await?;
        if order_return.status == ReturnStatus::Canceled {
            return Err(AppError::conflict(
                "cannot refund a canceled return".to_owned(),
            ));
        }
        let key = format!("{RETURN_REFUND_PREFIX}:{}", return_id.as_str());

        if let Some(existing) = order_return.refund_id.clone() {
            let order_refund = order
                .refunds
                .iter()
                .find(|refund| refund.id == existing)
                .cloned();
            let provider_refund = self.provider_refund_by_key(order_id, &key).await;
            return Ok(ReturnRefundResult {
                order,
                order_return,
                order_refund,
                provider_refund,
            });
        }

        // A previous attempt may have recorded the order refund and crashed
        // before linking it. Finish that link instead of refunding again.
        let local = local_key(&key);
        if let Some(existing) = self.order.refund_id_by_idempotency_key(&local).await? {
            let order_return = self
                .order
                .attach_return_refund(order_id, return_id, &existing)
                .await?;
            let order = self.order.get(order_id).await?;
            let order_refund = order
                .refunds
                .iter()
                .find(|refund| refund.id == existing)
                .cloned();
            let provider_refund = self.provider_refund_by_key(order_id, &key).await;
            return Ok(ReturnRefundResult {
                order,
                order_return,
                order_refund,
                provider_refund,
            });
        }

        // Only goods that actually arrived can be refunded. A written-off unit
        // is received but was not resold, so it never contributes to a refund.
        if !matches!(
            order_return.status,
            ReturnStatus::Received | ReturnStatus::PartiallyReceived
        ) {
            return Err(AppError::conflict(format!(
                "return in state {} cannot be refunded before goods are received",
                order_return.status.as_str()
            )));
        }
        let requested = order_return.items.iter().filter_map(|item| {
            let refundable = item
                .received_quantity
                .saturating_sub(item.written_off_quantity)
                .max(0);
            (refundable > 0).then(|| (item.order_item_id.clone(), refundable))
        });
        let refundable = refundable_amount(&order, requested)?;
        let amount = cap_refund(&order, refundable);
        if amount <= 0 {
            return Ok(ReturnRefundResult {
                order,
                order_return,
                order_refund: None,
                provider_refund: None,
            });
        }

        let payment = self.captured_payment(order_id).await?;
        let reason = reason.or_else(|| order_return.reason.clone());
        let provider_refund = self
            .payment
            .refund_with_idempotency(
                payment.id.as_str(),
                amount,
                reason.clone(),
                Some(key.clone()),
            )
            .await?;

        let (order, order_refund_id) = self
            .order
            .add_refund_idempotent(
                order_id,
                RefundInput {
                    payment_id: payment.id.clone(),
                    amount,
                    currency: order.currency_code.clone(),
                    reason: reason.clone(),
                },
                &local,
            )
            .await?;
        let order_return = self
            .order
            .attach_return_refund(order_id, return_id, &order_refund_id)
            .await?;
        let order_refund = order
            .refunds
            .iter()
            .find(|refund| refund.id == order_refund_id)
            .cloned();

        Ok(ReturnRefundResult {
            order,
            order_return,
            order_refund,
            provider_refund: Some(provider_refund),
        })
    }

    /// Accepts a claim when needed and refunds its refund-type lines.
    ///
    /// Claims without refund-type lines are left accepted with no refund.
    /// Replacement lines are intentionally not fulfilled here: a later flow
    /// should create outbound fulfillments for [`ClaimItemType::Replace`]
    /// lines after the refund settles.
    pub async fn refund_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
        reason: Option<String>,
    ) -> Result<ClaimRefundResult, AppError> {
        let order = self.order.get(order_id).await?;
        let mut claim = self.order.get_claim(order_id, claim_id).await?;
        if claim.status == ClaimStatus::Canceled {
            return Err(AppError::conflict(
                "cannot refund a canceled claim".to_owned(),
            ));
        }
        if claim.status == ClaimStatus::Requested {
            claim = self
                .order
                .clone()
                .without_audit()
                .accept_claim(order_id, claim_id)
                .await?;
        }
        let key = format!("{CLAIM_REFUND_PREFIX}:{}", claim_id.as_str());

        if let Some(existing) = claim.refund_id.clone() {
            let order_refund = order
                .refunds
                .iter()
                .find(|refund| refund.id == existing)
                .cloned();
            let provider_refund = self.provider_refund_by_key(order_id, &key).await;
            return Ok(ClaimRefundResult {
                order,
                claim,
                order_refund,
                provider_refund,
            });
        }

        // Complete a link left behind by an interrupted earlier attempt.
        let local = local_key(&key);
        if let Some(existing) = self.order.refund_id_by_idempotency_key(&local).await? {
            let claim = self
                .order
                .attach_claim_refund(order_id, claim_id, &existing)
                .await?;
            let order = self.order.get(order_id).await?;
            let order_refund = order
                .refunds
                .iter()
                .find(|refund| refund.id == existing)
                .cloned();
            let provider_refund = self.provider_refund_by_key(order_id, &key).await;
            return Ok(ClaimRefundResult {
                order,
                claim,
                order_refund,
                provider_refund,
            });
        }

        let requested = claim
            .items
            .iter()
            .filter(|item| item.item_type == ClaimItemType::Refund)
            .map(|item| (item.order_item_id.clone(), item.quantity));
        let refundable = refundable_amount(&order, requested)?;
        let amount = cap_refund(&order, refundable);
        if amount <= 0 {
            return Ok(ClaimRefundResult {
                order,
                claim,
                order_refund: None,
                provider_refund: None,
            });
        }

        let payment = self.captured_payment(order_id).await?;
        let reason = reason.or_else(|| claim.reason.clone());
        let provider_refund = self
            .payment
            .refund_with_idempotency(
                payment.id.as_str(),
                amount,
                reason.clone(),
                Some(key.clone()),
            )
            .await?;

        let (order, order_refund_id) = self
            .order
            .add_refund_idempotent(
                order_id,
                RefundInput {
                    payment_id: payment.id.clone(),
                    amount,
                    currency: order.currency_code.clone(),
                    reason: reason.clone(),
                },
                &local,
            )
            .await?;
        let claim = self
            .order
            .attach_claim_refund(order_id, claim_id, &order_refund_id)
            .await?;
        let order_refund = order
            .refunds
            .iter()
            .find(|refund| refund.id == order_refund_id)
            .cloned();

        Ok(ClaimRefundResult {
            order,
            claim,
            order_refund,
            provider_refund: Some(provider_refund),
        })
    }

    /// Receives returned goods and restocks only the newly received units.
    ///
    /// Receiving an already completed return restocks nothing. Repeated or
    /// concurrent calls are safe: the inventory movement reference is derived
    /// from the return, line and cumulative received quantity.
    pub async fn receive_return(
        &self,
        order_id: &OrderId,
        return_id: &ReturnId,
        input: ReceiveReturnInput,
    ) -> Result<ReceiveReturnResult, AppError> {
        let before = self.order.get_return(order_id, return_id).await?;
        let order_return = self
            .order
            .receive_return(order_id, return_id, input)
            .await?;
        let order = self.order.get(order_id).await?;

        let mut lines = Vec::new();
        for item in &order_return.items {
            // Target the full cumulative received quantity for each line. The
            // inventory service reconciles this against movements already
            // recorded, so a retry after a failed restock still restocks.
            if item.received_quantity <= 0 {
                continue;
            }
            let Some(order_item) = order
                .items
                .iter()
                .find(|line| line.id == item.order_item_id)
            else {
                continue;
            };
            if !order_item.requires_shipping {
                continue;
            }
            let Some(variant_id) = order_item.variant_id.clone() else {
                continue;
            };
            lines.push(ReturnRestockLine::new(
                item.order_item_id.clone(),
                variant_id,
                item.received_quantity,
                item.received_quantity,
            ));
        }

        let changed = before.status != order_return.status
            || order_return.items.iter().any(|item| {
                before
                    .items
                    .iter()
                    .find(|previous| previous.order_item_id == item.order_item_id)
                    .is_none_or(|previous| previous.received_quantity != item.received_quantity)
            });
        let restocked_units = if changed {
            self.inventory
                .restock_return_audited(
                    order_id,
                    return_id,
                    &lines,
                    RestockAudit {
                        action: "order.return_received".to_owned(),
                        resource_type: "order_return".to_owned(),
                        resource_id: return_id.as_str().to_owned(),
                        data: serde_json::json!({
                            "order_id": order_id.as_str(),
                            "status": order_return.status.as_str(),
                        }),
                    },
                )
                .await?
        } else if !lines.is_empty() {
            // The receive transaction may have committed before a previous
            // restock attempt failed. Reconcile the missing movements without
            // writing a duplicate receive audit.
            self.inventory
                .restock_return(order_id, return_id, &lines)
                .await?
        } else {
            0
        };

        Ok(ReceiveReturnResult {
            order_return,
            restocked_units,
        })
    }

    /// Fulfills the replacement lines of an accepted claim.
    ///
    /// Only [`ClaimItemType::Replace`] lines with a replacement variant are
    /// shipped; refund lines are handled by [`Self::refund_claim`]. Stock is
    /// consumed idempotently before the provider is called, and a provider
    /// failure reverses the consumption. Replaying the call returns the
    /// existing fulfillment without creating another provider fulfillment or
    /// stock movement.
    pub async fn fulfill_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
        provider_id: &str,
        tracking_numbers: Vec<String>,
    ) -> Result<ClaimFulfillmentResult, AppError> {
        let mut claim = self.order.get_claim(order_id, claim_id).await?;
        if claim.status == ClaimStatus::Canceled {
            return Err(AppError::conflict(
                "cannot fulfill a canceled claim".to_owned(),
            ));
        }
        if claim.status == ClaimStatus::Requested {
            claim = self
                .order
                .clone()
                .without_audit()
                .accept_claim(order_id, claim_id)
                .await?;
        }

        if let Some(existing) = self.order.claim_fulfillment(order_id, claim_id).await? {
            let order = self.order.get(order_id).await?;
            return Ok(ClaimFulfillmentResult {
                order,
                claim,
                fulfillment: Some(existing),
                consumed_units: 0,
            });
        }

        let mut replacements = Vec::new();
        for item in &claim.items {
            if item.item_type != ClaimItemType::Replace {
                continue;
            }
            let variant_id = item.replacement_variant_id.clone().ok_or_else(|| {
                AppError::validation(format!(
                    "replace claim line {} has no replacement_variant_id",
                    item.id
                ))
            })?;
            replacements.push((item.id.as_str().to_owned(), variant_id, item.quantity));
        }

        if replacements.is_empty() {
            let order = self.order.get(order_id).await?;
            return Ok(ClaimFulfillmentResult {
                order,
                claim,
                fulfillment: None,
                consumed_units: 0,
            });
        }

        let consumptions = replacements
            .iter()
            .map(|(line_id, variant_id, quantity)| {
                StockConsumption::new(
                    variant_id.clone(),
                    *quantity,
                    format!("claim-fulfill:{}:{}", claim_id.as_str(), line_id),
                )
            })
            .collect::<Vec<_>>();
        // Consume stock before contacting the provider: a stock failure must
        // not ship goods, and the movement is idempotent across retries.
        let (consumed_units, consumed_references) = self
            .inventory
            .consume_stock_tracked(order_id, &consumptions)
            .await?;

        let receipt = match self
            .create_provider_fulfillment(
                order_id,
                provider_id,
                Some(format!("claim-fulfill:{}", claim_id.as_str())),
                tracking_numbers,
                replacements
                    .iter()
                    .map(|(_, variant_id, quantity)| FulfillmentItemRequest {
                        variant_id: variant_id.as_str().to_owned(),
                        quantity: *quantity,
                    }),
            )
            .await
        {
            Ok(receipt) => receipt,
            Err(error) => {
                // The provider did not accept the replacement, so put the
                // reserved stock back and let a retry consume it again.
                if let Err(restore_error) = self
                    .inventory
                    .restore_consumed_stock(&consumed_references)
                    .await
                {
                    tracing::error!(
                        order_id = %order_id,
                        error = %restore_error,
                        "failed to restore replacement stock after the provider rejected a claim fulfillment"
                    );
                }
                return Err(error);
            }
        };

        let fulfillment = self
            .order
            .create_after_sales_fulfillment_audited(
                CreateAfterSalesFulfillmentInput {
                    order_id: order_id.clone(),
                    claim_id: Some(claim_id.clone()),
                    exchange_id: None,
                    provider_id: receipt.provider_id.clone(),
                    provider_fulfillment_id: Some(receipt.provider_fulfillment_id.clone()),
                    provider_data: receipt.data.clone(),
                    tracking_numbers: receipt.tracking_numbers.clone(),
                    metadata: Metadata::empty(),
                    items: replacements
                        .iter()
                        .map(|(_, variant_id, quantity)| AfterSalesFulfillmentItemInput {
                            variant_id: variant_id.clone(),
                            quantity: *quantity,
                        })
                        .collect(),
                },
                consumed_units,
            )
            .await?;

        let order = self.order.get(order_id).await?;
        let claim = self.order.get_claim(order_id, claim_id).await?;
        Ok(ClaimFulfillmentResult {
            order,
            claim,
            fulfillment: Some(fulfillment),
            consumed_units,
        })
    }

    /// Accepts an exchange and restocks its inbound items exactly once.
    ///
    /// Accepting an already accepted exchange is safe: the movement references
    /// are stable, so a retry restocks zero additional units.
    pub async fn accept_exchange(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
    ) -> Result<ExchangeAcceptResult, AppError> {
        self.accept_exchange_inner(order_id, exchange_id, true)
            .await
    }

    async fn accept_exchange_inner(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
        audit_accept: bool,
    ) -> Result<ExchangeAcceptResult, AppError> {
        let order = self.order.get(order_id).await?;
        let mut exchange = self.order.get_exchange(order_id, exchange_id).await?;
        if exchange.status == ExchangeStatus::Canceled {
            return Err(AppError::conflict(
                "cannot accept a canceled exchange".to_owned(),
            ));
        }
        let was_requested = exchange.status == ExchangeStatus::Requested;
        if was_requested {
            exchange = self.order.accept_exchange(order_id, exchange_id).await?;
        }

        let mut lines = Vec::new();
        for item in &exchange.inbound_items {
            if item.status != ExchangeItemStatus::Accepted {
                continue;
            }
            let Some(order_item) = order
                .items
                .iter()
                .find(|order_item| order_item.id == item.order_item_id)
            else {
                continue;
            };
            if !order_item.requires_shipping {
                continue;
            }
            let Some(variant_id) = order_item.variant_id.clone() else {
                continue;
            };
            lines.push(ExchangeRestockLine::new(
                item.id.as_str(),
                variant_id,
                item.quantity,
            ));
        }

        let restocked_units = if audit_accept && was_requested {
            self.inventory
                .restock_exchange_audited(
                    order_id,
                    exchange_id.as_str(),
                    &lines,
                    RestockAudit {
                        action: "order.exchange_accepted".to_owned(),
                        resource_type: "order_exchange".to_owned(),
                        resource_id: exchange_id.as_str().to_owned(),
                        data: serde_json::json!({"order_id": order_id.as_str()}),
                    },
                )
                .await?
        } else {
            // Restocking is idempotent: retries after an interrupted accept
            // still reconcile any missing stock movements.
            self.inventory
                .restock_exchange(order_id, exchange_id.as_str(), &lines)
                .await?
        };
        let order = self.order.get(order_id).await?;
        Ok(ExchangeAcceptResult {
            order,
            exchange,
            restocked_units,
        })
    }

    /// Fulfills the outbound lines of an accepted exchange.
    ///
    /// Mirrors [`Self::fulfill_claim`]: replacement stock is consumed
    /// idempotently before the provider is called and a provider failure
    /// reverses the consumption. Replaying the call returns the existing
    /// fulfillment.
    pub async fn fulfill_exchange(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
        provider_id: &str,
        tracking_numbers: Vec<String>,
    ) -> Result<ExchangeFulfillmentResult, AppError> {
        let exchange = self
            .accept_exchange_inner(order_id, exchange_id, false)
            .await?
            .exchange;

        if let Some(existing) = self
            .order
            .exchange_fulfillment(order_id, exchange_id)
            .await?
        {
            let order = self.order.get(order_id).await?;
            return Ok(ExchangeFulfillmentResult {
                order,
                exchange,
                fulfillment: Some(existing),
                consumed_units: 0,
            });
        }

        let mut replacements = Vec::new();
        for item in &exchange.outbound_items {
            if item.direction != ExchangeItemDirection::Outbound
                || item.status != ExchangeItemStatus::Accepted
            {
                continue;
            }
            let variant_id = item.replacement_variant_id.clone().ok_or_else(|| {
                AppError::validation(format!(
                    "outbound exchange line {} has no replacement_variant_id",
                    item.id
                ))
            })?;
            replacements.push((item.id.as_str().to_owned(), variant_id, item.quantity));
        }

        if replacements.is_empty() {
            let order = self.order.get(order_id).await?;
            return Ok(ExchangeFulfillmentResult {
                order,
                exchange,
                fulfillment: None,
                consumed_units: 0,
            });
        }

        let consumptions = replacements
            .iter()
            .map(|(line_id, variant_id, quantity)| {
                StockConsumption::new(
                    variant_id.clone(),
                    *quantity,
                    format!("exchange-fulfill:{}:{}", exchange_id.as_str(), line_id),
                )
            })
            .collect::<Vec<_>>();
        // Consume stock before contacting the provider, mirroring claims.
        let (consumed_units, consumed_references) = self
            .inventory
            .consume_stock_tracked(order_id, &consumptions)
            .await?;

        let receipt = match self
            .create_provider_fulfillment(
                order_id,
                provider_id,
                Some(format!("exchange-fulfill:{}", exchange_id.as_str())),
                tracking_numbers,
                replacements
                    .iter()
                    .map(|(_, variant_id, quantity)| FulfillmentItemRequest {
                        variant_id: variant_id.as_str().to_owned(),
                        quantity: *quantity,
                    }),
            )
            .await
        {
            Ok(receipt) => receipt,
            Err(error) => {
                if let Err(restore_error) = self
                    .inventory
                    .restore_consumed_stock(&consumed_references)
                    .await
                {
                    tracing::error!(
                        order_id = %order_id,
                        error = %restore_error,
                        "failed to restore replacement stock after the provider rejected an exchange fulfillment"
                    );
                }
                return Err(error);
            }
        };

        let fulfillment = self
            .order
            .create_after_sales_fulfillment_audited(
                CreateAfterSalesFulfillmentInput {
                    order_id: order_id.clone(),
                    claim_id: None,
                    exchange_id: Some(exchange_id.clone()),
                    provider_id: receipt.provider_id.clone(),
                    provider_fulfillment_id: Some(receipt.provider_fulfillment_id.clone()),
                    provider_data: receipt.data.clone(),
                    tracking_numbers: receipt.tracking_numbers.clone(),
                    metadata: Metadata::empty(),
                    items: replacements
                        .iter()
                        .map(|(_, variant_id, quantity)| AfterSalesFulfillmentItemInput {
                            variant_id: variant_id.clone(),
                            quantity: *quantity,
                        })
                        .collect(),
                },
                consumed_units,
            )
            .await?;

        let order = self.order.get(order_id).await?;
        let exchange = self.order.get_exchange(order_id, exchange_id).await?;
        Ok(ExchangeFulfillmentResult {
            order,
            exchange,
            fulfillment: Some(fulfillment),
            consumed_units,
        })
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
