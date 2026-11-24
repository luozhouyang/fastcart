//! Exchange acceptance and replacement fulfillment.

//! The workflow owns the cross-module idempotency boundary. Provider refunds
//! use a stable key per after-sales record, order refunds use the same key as a
//! local idempotency marker, and restock movements use a deterministic
//! reference. Replaying any step therefore never moves money or stock twice.

use fastcart_kernel::{AppError, ExchangeId, LineItemId, Metadata, OrderId, VariantId};
use fastcart_module_inventory::{ExchangeRestockLine, RestockAudit, StockConsumption};
use fastcart_module_order::{
    AfterSalesFulfillmentItemInput, CreateAfterSalesFulfillmentInput, ExchangeItemDirection,
    ExchangeItemStatus, ExchangeStatus, OrderExchange,
};
use fastcart_module_shipping::FulfillmentItemRequest;

use super::*;
/// An exchange's outbound (replacement) lines in the same shape.
fn exchange_replacements(exchange: &OrderExchange) -> Vec<(&LineItemId, Option<&VariantId>, i64)> {
    exchange
        .outbound_items
        .iter()
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
            let prices = self
                .replacement_prices(&order, &exchange_replacements(&exchange))
                .await?;
            exchange = self
                .order
                .accept_exchange(order_id, exchange_id, &prices)
                .await?;
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
}
