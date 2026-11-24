//! fulfillment admin order routes.

//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read`, `orders.write` and `orders.cancel` permissions.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult};
use fastcart_kernel::{Metadata, OrderId};
use fastcart_module_order::{Order, OrderService};
use fastcart_module_shipping::CreateFulfillmentRequest;

use super::StoreDb;
use crate::state::AppContext;

/// Builds the routes for this namespace.
use super::*;

#[utoipa::path(
    post,
    path = "/orders/{id}/fulfill",
    tag = "Admin",
    summary = "Fulfill an order",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = FulfillOrderRequest,
    responses((status = 200, body = Order))
)]
pub(super) async fn fulfill_order(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<FulfillOrderRequest>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let orders =
        OrderService::new(db).with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let current = orders.get(&id).await?;

    let provider_id = request
        .provider_id
        .clone()
        .unwrap_or_else(|| "manual".to_owned());
    let provider = context
        .shipping_providers
        .get(&provider_id)
        .ok_or_else(|| {
            ApiError::new(fastcart_kernel::AppError::validation(format!(
                "unknown fulfillment provider: {provider_id}"
            )))
        })?;

    let provider_items = map_provider_items(&current, &request.items)?;
    if provider_items.is_empty() {
        return Err(ApiError::new(fastcart_kernel::AppError::conflict(
            "order has no remaining shippable items",
        )));
    }

    // Create the provider-side fulfillment before touching the order. A failure
    // here must leave every local record unchanged.
    let idempotency_key = Some(format!(
        "order-fulfill:{}:{}:{}",
        id.as_str(),
        provider_items
            .iter()
            .map(|item| format!("{}x{}", item.variant_id, item.quantity))
            .collect::<Vec<_>>()
            .join(","),
        request.tracking_numbers.join(",")
    ));
    let receipt = provider
        .create_fulfillment(CreateFulfillmentRequest {
            order_id: id.as_str().to_owned(),
            items: provider_items,
            tracking_numbers: request.tracking_numbers.clone(),
            provider_data: request.provider_data.clone(),
            idempotency_key,
            metadata: Metadata::empty(),
        })
        .await
        .map_err(ApiError::new)?;

    let mut input = map_fulfillment_input(request);
    input.provider_id = Some(provider_id);
    input.provider_fulfillment_id = Some(receipt.provider_fulfillment_id);
    if has_provider_data(&receipt.data) {
        input.provider_data = receipt.data;
    }

    let order = orders.fulfill(&id, input).await?;
    Ok(Json(order))
}
