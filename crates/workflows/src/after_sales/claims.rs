//! Claim acceptance, refunds and replacement fulfillment.

//! The workflow owns the cross-module idempotency boundary. Provider refunds
//! use a stable key per after-sales record, order refunds use the same key as a
//! local idempotency marker, and restock movements use a deterministic
//! reference. Replaying any step therefore never moves money or stock twice.

use fastcart_kernel::{AppError, ClaimId, LineItemId, Metadata, OrderId, VariantId};
use fastcart_module_inventory::StockConsumption;
use fastcart_module_order::{
    AfterSalesFulfillmentItemInput, ClaimItemType, ClaimStatus, CreateAfterSalesFulfillmentInput,
    OrderClaim, RefundInput,
};
use fastcart_module_shipping::FulfillmentItemRequest;

use super::*;
/// A claim's replacement lines as `(replaced line, replacement variant,
/// quantity)` triples.
fn claim_replacements(claim: &OrderClaim) -> Vec<(&LineItemId, Option<&VariantId>, i64)> {
    claim
        .items
        .iter()
        .filter(|item| item.item_type == ClaimItemType::Replace)
        .map(|item| {
            (
                &item.order_item_id,
                item.replacement_variant_id.as_ref(),
                item.quantity,
            )
        })
        .collect()
}

impl AfterSalesWorkflow {
    /// Accepts a requested claim, settling the price difference of its
    /// replacement lines.
    ///
    /// The order module prices no catalog variants, so the workflow resolves
    /// the unit price of every replacement line from the pricing module and
    /// passes it into [`OrderService::accept_claim`], which books the
    /// difference on the order. A replacement variant with no eligible price
    /// settles nothing; accepting an already accepted claim changes nothing.
    pub async fn accept_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
    ) -> Result<OrderClaim, AppError> {
        let order = self.order.get(order_id).await?;
        let claim = self.order.get_claim(order_id, claim_id).await?;
        // Only a requested claim settles a difference; any other state is
        // handled by the order service below, exactly as before.
        let prices = if claim.status == ClaimStatus::Requested {
            self.replacement_prices(&order, &claim_replacements(&claim))
                .await?
        } else {
            Vec::new()
        };
        self.order.accept_claim(order_id, claim_id, &prices).await
    }

    /// Accepts a claim when needed and refunds its refund-type lines.
    ///
    /// Claims without refund-type lines are left accepted with no refund.
    /// Replacement lines are intentionally not fulfilled here: a later flow
    /// should create outbound fulfillments for [`ClaimItemType::Replace`]
    /// lines after the refund settles. Accepting the claim is an implied step
    /// of the refund, so it is not audited separately.
    pub async fn refund_claim(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
        reason: Option<String>,
    ) -> Result<ClaimRefundResult, AppError> {
        let mut claim = self.order.get_claim(order_id, claim_id).await?;
        if claim.status == ClaimStatus::Canceled {
            return Err(AppError::conflict(
                "cannot refund a canceled claim".to_owned(),
            ));
        }
        if claim.status == ClaimStatus::Requested {
            let order = self.order.get(order_id).await?;
            let prices = self
                .replacement_prices(&order, &claim_replacements(&claim))
                .await?;
            claim = self
                .order
                .clone()
                .without_audit()
                .accept_claim(order_id, claim_id, &prices)
                .await?;
        }
        // Accepting the claim may have settled its price difference on the
        // order, so every amount below is measured against the order as it now
        // stands.
        let order = self.order.get(order_id).await?;
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
            let order = self.order.get(order_id).await?;
            let prices = self
                .replacement_prices(&order, &claim_replacements(&claim))
                .await?;
            claim = self
                .order
                .clone()
                .without_audit()
                .accept_claim(order_id, claim_id, &prices)
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
}
