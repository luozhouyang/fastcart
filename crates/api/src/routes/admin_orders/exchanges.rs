//! exchanges admin order routes.

//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read`, `orders.write` and `orders.cancel` permissions.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult};
use fastcart_kernel::{ExchangeId, OrderId};
use fastcart_module_order::{OrderExchange, OrderService};
use fastcart_workflows::AfterSalesWorkflow;

use super::StoreDb;
use crate::dto::{CancelAfterSaleRequest, CreateExchangeRequest};
use crate::state::AppContext;

/// Builds the routes for this namespace.
use super::*;

#[utoipa::path(
    get,
    path = "/orders/{id}/exchanges",
    tag = "Admin",
    summary = "List order exchanges",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<OrderExchange>))
)]
pub(super) async fn list_exchanges(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<OrderExchange>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).list_exchanges(&id).await?))
}

#[utoipa::path(
    get,
    path = "/orders/{id}/exchanges/{exchange_id}",
    tag = "Admin",
    summary = "Get an order exchange",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = OrderExchange))
)]
pub(super) async fn get_exchange(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, exchange_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderExchange>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange_id = ExchangeId::from_string(exchange_id);
    Ok(Json(
        OrderService::new(db)
            .get_exchange(&id, &exchange_id)
            .await?,
    ))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/exchanges",
    tag = "Admin",
    summary = "Create an order exchange",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateExchangeRequest,
    responses((status = 201, body = OrderExchange))
)]
pub(super) async fn create_exchange(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateExchangeRequest>,
) -> ApiResult<(StatusCode, Json<OrderExchange>)> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .create_exchange(&id, map_exchange_input(request))
        .await?;
    Ok((StatusCode::CREATED, Json(exchange)))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/exchanges/{exchange_id}/accept",
    tag = "Admin",
    summary = "Accept an order exchange and settle its replacement price difference",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = OrderExchange))
)]
pub(super) async fn accept_exchange(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, exchange_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderExchange>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange_id = ExchangeId::from_string(exchange_id);
    let workflow = AfterSalesWorkflow::with_shipping_providers(
        db.clone(),
        payment_service(&context, &actor, &db)?,
        context.shipping_providers.clone(),
    )
    .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow.accept_exchange(&id, &exchange_id).await?;
    Ok(Json(result.exchange))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/exchanges/{exchange_id}/refund-difference",
    tag = "Admin",
    summary = "Refund the price difference an accepted exchange owes the customer",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = DifferenceRefundResponse))
)]
pub(super) async fn refund_exchange_difference(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, exchange_id)): Path<(String, String)>,
) -> ApiResult<Json<DifferenceRefundResponse>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange_id = ExchangeId::from_string(exchange_id);
    let workflow = AfterSalesWorkflow::with_shipping_providers(
        db.clone(),
        payment_service(&context, &actor, &db)?,
        context.shipping_providers.clone(),
    )
    .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow
        .refund_exchange_difference(&id, &exchange_id)
        .await?;
    Ok(Json(DifferenceRefundResponse {
        order: result.order,
        order_refund: result.order_refund,
        provider_refund: result.provider_refund,
    }))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/exchanges/{exchange_id}/collect-difference",
    tag = "Admin",
    summary = "Make the price difference an accepted exchange has the customer owing payable",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CollectDifferenceRequest,
    responses((status = 200, body = DifferenceCollectionResponse))
)]
pub(super) async fn collect_exchange_difference(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, exchange_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CollectDifferenceRequest>,
) -> ApiResult<Json<DifferenceCollectionResponse>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange_id = ExchangeId::from_string(exchange_id);
    let workflow = AfterSalesWorkflow::with_shipping_providers(
        db.clone(),
        payment_service(&context, &actor, &db)?,
        context.shipping_providers.clone(),
    )
    .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow
        .collect_exchange_difference(&id, &exchange_id, &request.provider)
        .await?;
    Ok(Json(DifferenceCollectionResponse {
        order: result.order,
        payment_collection: result.collection,
        payment_session: result.session,
    }))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/exchanges/{exchange_id}/fulfill",
    tag = "Admin",
    summary = "Fulfill the outbound lines of an exchange",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = FulfillAfterSalesRequest,
    responses((status = 200, body = ExchangeFulfillmentResponse))
)]
pub(super) async fn fulfill_exchange(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, exchange_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<FulfillAfterSalesRequest>,
) -> ApiResult<Json<ExchangeFulfillmentResponse>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange_id = ExchangeId::from_string(exchange_id);
    let workflow = AfterSalesWorkflow::with_shipping_providers(
        db.clone(),
        payment_service(&context, &actor, &db)?,
        context.shipping_providers.clone(),
    )
    .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow
        .fulfill_exchange(
            &id,
            &exchange_id,
            request.provider_id.as_deref().unwrap_or("manual"),
            request.tracking_numbers,
        )
        .await?;
    Ok(Json(ExchangeFulfillmentResponse {
        order: result.order,
        exchange: result.exchange,
        fulfillment: result.fulfillment,
        consumed_units: result.consumed_units,
    }))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/exchanges/{exchange_id}/cancel",
    tag = "Admin",
    summary = "Cancel an order exchange",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CancelAfterSaleRequest,
    responses((status = 200, body = OrderExchange))
)]
pub(super) async fn cancel_exchange(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, exchange_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CancelAfterSaleRequest>,
) -> ApiResult<Json<OrderExchange>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange_id = ExchangeId::from_string(exchange_id);
    let exchange = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .cancel_exchange_with_reason(&id, &exchange_id, request.reason.as_deref())
        .await?;
    Ok(Json(exchange))
}
