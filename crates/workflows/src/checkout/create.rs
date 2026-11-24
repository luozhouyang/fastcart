//! Opening a checkout from a cart.

use fastcart_kernel::{AppError, Metadata, PaymentSessionId};
use fastcart_module_checkout::{CheckoutSession, CheckoutStatus, CreateCheckoutInput};
use fastcart_module_inventory::{ReservationLine, ReservationLinePolicy, ReservationReference};
use fastcart_module_payment::CreatePaymentSessionInput;
use fastcart_module_store::Region;

use super::*;

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

impl CheckoutWorkflow {
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
}
