//! orders admin order routes.

//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read`, `orders.write` and `orders.cancel` permissions.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiQuery, ApiResult, PaginationQuery};
use fastcart_kernel::OrderId;
use fastcart_module_inventory::InventoryService;
use fastcart_module_order::{Order, OrderService, OrderStatus, OrderStatusHistory};
use fastcart_workflows::DraftOrderWorkflow;

use super::StoreDb;
use crate::dto::{CreateDraftOrderRequest, PageResponse, UpdateDraftAddressRequest};
use crate::state::AppContext;

/// Builds the routes for this namespace.
use super::*;

#[utoipa::path(
    get,
    path = "/orders",
    tag = "Admin",
    summary = "List orders",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery, OrderListQuery),
    responses((status = 200, body = PageResponse<Order>))
)]
pub(super) async fn list_orders(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
    ApiQuery(query): ApiQuery<OrderListQuery>,
) -> ApiResult<Json<PageResponse<Order>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = OrderService::new(db)
        .list(map_order_filter(query), pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    get,
    path = "/orders/{id}",
    tag = "Admin",
    summary = "Get an order",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Order))
)]
pub(super) async fn get_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).get(&id).await?))
}

#[utoipa::path(
    get,
    path = "/orders/by-number/{display_number}",
    tag = "Admin",
    summary = "Get an order by display number",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Order))
)]
pub(super) async fn get_order_by_number(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(display_number): Path<i64>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    Ok(Json(
        OrderService::new(db)
            .get_by_display_number(display_number)
            .await?,
    ))
}

#[utoipa::path(
    post,
    path = "/orders",
    tag = "Admin",
    summary = "Create a draft order",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateDraftOrderRequest,
    responses((status = 201, body = Order))
)]
pub(super) async fn create_draft_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateDraftOrderRequest>,
) -> ApiResult<(StatusCode, Json<Order>)> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let workflow = DraftOrderWorkflow::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let order = workflow
        .create_draft_order(map_draft_order_input(request))
        .await?;
    Ok((StatusCode::CREATED, Json(order)))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/convert",
    tag = "Admin",
    summary = "Convert a draft order and enter the fulfillment lifecycle",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Order))
)]
pub(super) async fn convert_draft_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let workflow = DraftOrderWorkflow::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let order = workflow
        .convert_draft_order(&OrderId::from_string(id))
        .await?;
    Ok(Json(order))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/shipping-address",
    tag = "Admin",
    summary = "Change a draft order's destination and recalculate its tax",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateDraftAddressRequest,
    responses((status = 200, body = Order))
)]
pub(super) async fn update_draft_address(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateDraftAddressRequest>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let workflow = DraftOrderWorkflow::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let order = workflow
        .update_draft_destination(
            &OrderId::from_string(id),
            request.shipping_address,
            request.billing_address,
        )
        .await?;
    Ok(Json(order))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/confirm",
    tag = "Admin",
    summary = "Confirm an order",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Order))
)]
pub(super) async fn confirm_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let order = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .confirm(&id)
        .await?;
    Ok(Json(order))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/process",
    tag = "Admin",
    summary = "Start processing an order",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Order))
)]
pub(super) async fn process_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let order = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .start_processing(&id)
        .await?;
    Ok(Json(order))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/cancel",
    tag = "Admin",
    summary = "Cancel an order and restock inventory",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CancelOrderRequest,
    responses((status = 200, body = Order))
)]
pub(super) async fn cancel_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CancelOrderRequest>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.cancel")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let orders = OrderService::new(db.clone())
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let current = orders.get(&id).await?;
    if current.status == OrderStatus::Canceled {
        // Restocking is idempotent; retry it after an interrupted cancellation.
        InventoryService::new(db).restock_order(&id).await?;
        return Ok(Json(current));
    }
    let reason = request.reason;
    // Claim the cancellation first so a concurrent fulfillment cannot ship an
    // order that was already restocked.
    let order = orders.cancel(&id, reason.as_deref()).await?;
    InventoryService::new(db).restock_order(&id).await?;
    Ok(Json(order))
}

#[utoipa::path(
    get,
    path = "/orders/{id}/history",
    tag = "Admin",
    summary = "List order status history",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<OrderStatusHistory>))
)]
pub(super) async fn order_history(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<OrderStatusHistory>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).list_history(&id).await?))
}
