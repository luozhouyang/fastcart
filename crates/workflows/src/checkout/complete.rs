//! Completing a checkout and placing its order.

use std::collections::HashMap;

use fastcart_kernel::{AppError, CheckoutId, Metadata, PaymentSessionId, now_timestamp};
use fastcart_module_checkout::{CheckoutSession, CheckoutStatus};
use fastcart_module_order::{
    CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput, CreateOrderTaxLineInput,
    OrderShippingMethod,
};
use fastcart_module_payment::PaymentStatus as PaymentSessionStatus;

use crate::cart::CartContext;

use super::*;

impl CheckoutWorkflow {
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
}
