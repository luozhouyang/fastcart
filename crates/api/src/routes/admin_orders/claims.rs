//! claims admin order routes.

//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read`, `orders.write` and `orders.cancel` permissions.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult};
use fastcart_kernel::{ClaimId, OrderId};
use fastcart_module_order::{OrderClaim, OrderService};
use fastcart_workflows::AfterSalesWorkflow;

use super::StoreDb;
use crate::dto::{CancelAfterSaleRequest, CreateClaimRequest};
use crate::state::AppContext;

/// Builds the routes for this namespace.
use super::*;

#[utoipa::path(
    get,
    path = "/orders/{id}/claims",
    tag = "Admin",
    summary = "List order claims",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<OrderClaim>))
)]
pub(super) async fn list_claims(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<OrderClaim>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).list_claims(&id).await?))
}

#[utoipa::path(
    get,
    path = "/orders/{id}/claims/{claim_id}",
    tag = "Admin",
    summary = "Get an order claim",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = OrderClaim))
)]
pub(super) async fn get_claim(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, claim_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderClaim>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim_id = ClaimId::from_string(claim_id);
    Ok(Json(OrderService::new(db).get_claim(&id, &claim_id).await?))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/claims",
    tag = "Admin",
    summary = "Create an order claim",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateClaimRequest,
    responses((status = 201, body = OrderClaim))
)]
pub(super) async fn create_claim(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateClaimRequest>,
) -> ApiResult<(StatusCode, Json<OrderClaim>)> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .create_claim(&id, map_claim_input(request))
        .await?;
    Ok((StatusCode::CREATED, Json(claim)))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/claims/{claim_id}/accept",
    tag = "Admin",
    summary = "Accept an order claim and settle its replacement price difference",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = OrderClaim))
)]
pub(super) async fn accept_claim(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, claim_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderClaim>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim_id = ClaimId::from_string(claim_id);
    // The workflow is the layer that can price the replacement lines, so the
    // price difference of a `replace` claim is settled here.
    let workflow = AfterSalesWorkflow::new(db.clone(), payment_service(&context, &actor, &db)?)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let claim = workflow.accept_claim(&id, &claim_id).await?;
    Ok(Json(claim))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/claims/{claim_id}/refund",
    tag = "Admin",
    summary = "Refund an accepted order claim",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CancelAfterSaleRequest,
    responses((status = 200, body = ClaimRefundResponse))
)]
pub(super) async fn refund_claim(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, claim_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CancelAfterSaleRequest>,
) -> ApiResult<Json<ClaimRefundResponse>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim_id = ClaimId::from_string(claim_id);
    let workflow = AfterSalesWorkflow::new(db.clone(), payment_service(&context, &actor, &db)?)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow
        .refund_claim(&id, &claim_id, request.reason)
        .await?;
    Ok(Json(ClaimRefundResponse {
        order: result.order,
        claim: result.claim,
        order_refund: result.order_refund,
        provider_refund: result.provider_refund,
    }))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/claims/{claim_id}/cancel",
    tag = "Admin",
    summary = "Cancel an order claim",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CancelAfterSaleRequest,
    responses((status = 200, body = OrderClaim))
)]
pub(super) async fn cancel_claim(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, claim_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CancelAfterSaleRequest>,
) -> ApiResult<Json<OrderClaim>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim_id = ClaimId::from_string(claim_id);
    let claim = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .cancel_claim_with_reason(&id, &claim_id, request.reason.as_deref())
        .await?;
    Ok(Json(claim))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/claims/{claim_id}/collect-difference",
    tag = "Admin",
    summary = "Make the price difference an accepted claim has the customer owing payable",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CollectDifferenceRequest,
    responses((status = 200, body = DifferenceCollectionResponse))
)]
pub(super) async fn collect_claim_difference(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, claim_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CollectDifferenceRequest>,
) -> ApiResult<Json<DifferenceCollectionResponse>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim_id = ClaimId::from_string(claim_id);
    let workflow = AfterSalesWorkflow::new(db.clone(), payment_service(&context, &actor, &db)?)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow
        .collect_claim_difference(&id, &claim_id, &request.provider)
        .await?;
    Ok(Json(DifferenceCollectionResponse {
        order: result.order,
        payment_collection: result.collection,
        payment_session: result.session,
    }))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/claims/{claim_id}/refund-difference",
    tag = "Admin",
    summary = "Refund the price difference an accepted claim owes the customer",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = DifferenceRefundResponse))
)]
pub(super) async fn refund_claim_difference(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, claim_id)): Path<(String, String)>,
) -> ApiResult<Json<DifferenceRefundResponse>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim_id = ClaimId::from_string(claim_id);
    let workflow = AfterSalesWorkflow::new(db.clone(), payment_service(&context, &actor, &db)?)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow.refund_claim_difference(&id, &claim_id).await?;
    Ok(Json(DifferenceRefundResponse {
        order: result.order,
        order_refund: result.order_refund,
        provider_refund: result.provider_refund,
    }))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/claims/{claim_id}/fulfill",
    tag = "Admin",
    summary = "Fulfill the replacement lines of a claim",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = FulfillAfterSalesRequest,
    responses((status = 200, body = ClaimFulfillmentResponse))
)]
pub(super) async fn fulfill_claim(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, claim_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<FulfillAfterSalesRequest>,
) -> ApiResult<Json<ClaimFulfillmentResponse>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim_id = ClaimId::from_string(claim_id);
    let workflow = AfterSalesWorkflow::with_shipping_providers(
        db.clone(),
        payment_service(&context, &actor, &db)?,
        context.shipping_providers.clone(),
    )
    .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow
        .fulfill_claim(
            &id,
            &claim_id,
            request.provider_id.as_deref().unwrap_or("manual"),
            request.tracking_numbers,
        )
        .await?;
    Ok(Json(ClaimFulfillmentResponse {
        order: result.order,
        claim: result.claim,
        fulfillment: result.fulfillment,
        consumed_units: result.consumed_units,
    }))
}
