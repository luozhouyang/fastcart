//! Admin inventory HTTP routes.
//!
//! Handlers translate HTTP requests into [`InventoryService`] calls after
//! enforcing the `inventory.read` / `inventory.write` permissions.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiQuery, ApiResult, PaginationQuery};
use fastcart_kernel::{
    CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, Metadata, OrderId, ReservationId,
    VariantId, now_timestamp,
};
use fastcart_module_inventory::{
    CreateInventoryItemInput, CreateLocationInput, InventoryItem, InventoryLevel,
    InventoryLocation, InventoryMovement, InventoryReservation, InventoryService, ReservationLine,
    ReservationReference, UpdateLocationInput,
};
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::dto::PageResponse;
use crate::state::AppContext;

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route(
            "/inventory/locations",
            get(list_locations).post(create_location),
        )
        .route(
            "/inventory/locations/{id}",
            get(get_location).patch(update_location),
        )
        .route("/inventory/items", get(list_items).post(create_item))
        .route("/inventory/items/{id}", get(get_item))
        .route(
            "/inventory/items/by-variant/{variant_id}",
            get(get_item_by_variant),
        )
        .route("/inventory/levels", get(list_levels))
        .route(
            "/inventory/availability/{variant_id}",
            get(get_availability),
        )
        .route("/inventory/adjustments", post(create_adjustment))
        .route("/inventory/transfers", post(create_transfer))
        .route("/inventory/reservations", post(create_reservations))
        .route(
            "/inventory/reservations/release",
            post(release_reservations),
        )
        .route("/inventory/reservations/commit", post(commit_reservations))
        .route("/inventory/reservations/expire", post(expire_reservations))
        .route("/inventory/movements", get(list_movements))
}

/// Builds the documented inventory routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_locations, create_location))
        .routes(routes!(get_location, update_location))
        .routes(routes!(list_items, create_item))
        .routes(routes!(get_item))
        .routes(routes!(get_item_by_variant))
        .routes(routes!(list_levels))
        .routes(routes!(get_availability))
        .routes(routes!(create_adjustment))
        .routes(routes!(create_transfer))
        .routes(routes!(create_reservations))
        .routes(routes!(release_reservations))
        .routes(routes!(commit_reservations))
        .routes(routes!(expire_reservations))
        .routes(routes!(list_movements))
}

// -------------------------------------------------------------------------
// Requests and responses
// -------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = CreateInventoryLocationRequest)]
pub struct CreateLocationRequest {
    name: String,
    address: Option<Metadata>,
    metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = UpdateInventoryLocationRequest)]
pub struct UpdateLocationRequest {
    name: Option<String>,
    address: Option<Metadata>,
    metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = CreateInventoryItemRequest)]
pub struct CreateItemRequest {
    sku: Option<String>,
    variant_id: Option<VariantId>,
    metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct LevelsQuery {
    item_id: Option<InventoryItemId>,
    location_id: Option<InventoryLocationId>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = InventoryAdjustmentRequest)]
pub struct AdjustmentRequest {
    inventory_item_id: InventoryItemId,
    location_id: InventoryLocationId,
    quantity_delta: i64,
    reason: String,
    reference: Option<String>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = InventoryTransferRequest)]
pub struct TransferRequest {
    inventory_item_id: InventoryItemId,
    from_location_id: InventoryLocationId,
    to_location_id: InventoryLocationId,
    quantity: i64,
    reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct ReservationLineRequest {
    variant_id: VariantId,
    quantity: i64,
    line_item_id: Option<LineItemId>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct ReservationRequest {
    lines: Vec<ReservationLineRequest>,
    checkout_id: CheckoutId,
    ttl_seconds: i64,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct ReleaseReservationsRequest {
    #[serde(default)]
    reservation_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CommitReservationsRequest {
    checkout_id: CheckoutId,
    order_id: OrderId,
}

#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct MovementsQuery {
    item_id: InventoryItemId,
    limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[schema(as = InventoryAvailability)]
pub struct AvailabilityResponse {
    variant_id: VariantId,
    available: i64,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ReleasedResponse {
    released: u64,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CommittedResponse {
    committed: u64,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct TransferResponse {
    from: InventoryLevel,
    to: InventoryLevel,
}

// -------------------------------------------------------------------------
// Pure request mapping
// -------------------------------------------------------------------------

fn map_create_location(request: CreateLocationRequest) -> CreateLocationInput {
    CreateLocationInput {
        name: request.name,
        address: request.address,
        metadata: request.metadata,
    }
}

fn map_update_location(request: UpdateLocationRequest) -> UpdateLocationInput {
    UpdateLocationInput {
        name: request.name,
        address: request.address,
        metadata: request.metadata,
    }
}

fn map_create_item(request: CreateItemRequest) -> CreateInventoryItemInput {
    CreateInventoryItemInput {
        sku: request.sku,
        variant_id: request.variant_id,
        metadata: request.metadata,
    }
}

fn map_reservation_lines(lines: Vec<ReservationLineRequest>) -> Vec<ReservationLine> {
    lines
        .into_iter()
        .map(|line| ReservationLine {
            variant_id: line.variant_id,
            quantity: line.quantity,
            line_item_id: line.line_item_id,
        })
        .collect()
}

// -------------------------------------------------------------------------
// Locations
// -------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/inventory/locations",
    tag = "Admin",
    summary = "List inventory locations",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<InventoryLocation>))
)]
async fn list_locations(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<InventoryLocation>>> {
    actor
        .require_permission("inventory.read")
        .map_err(ApiError::new)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = InventoryService::new(db).list_locations(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/inventory/locations",
    tag = "Admin",
    summary = "Create an inventory location",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateLocationRequest,
    responses((status = 201, body = InventoryLocation))
)]
async fn create_location(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateLocationRequest>,
) -> ApiResult<(StatusCode, Json<InventoryLocation>)> {
    actor
        .require_permission("inventory.write")
        .map_err(ApiError::new)?;
    let location = InventoryService::new(db)
        .create_location(map_create_location(request))
        .await?;
    Ok((StatusCode::CREATED, Json(location)))
}

#[utoipa::path(
    get,
    path = "/inventory/locations/{id}",
    tag = "Admin",
    summary = "Get an inventory location",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = InventoryLocation))
)]
async fn get_location(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<InventoryLocation>> {
    actor
        .require_permission("inventory.read")
        .map_err(ApiError::new)?;
    let id = InventoryLocationId::from_string(id);
    Ok(Json(InventoryService::new(db).get_location(&id).await?))
}

#[utoipa::path(
    patch,
    path = "/inventory/locations/{id}",
    tag = "Admin",
    summary = "Update an inventory location",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateLocationRequest,
    responses((status = 200, body = InventoryLocation))
)]
async fn update_location(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateLocationRequest>,
) -> ApiResult<Json<InventoryLocation>> {
    actor
        .require_permission("inventory.write")
        .map_err(ApiError::new)?;
    let id = InventoryLocationId::from_string(id);
    let location = InventoryService::new(db)
        .update_location(&id, map_update_location(request))
        .await?;
    Ok(Json(location))
}

// -------------------------------------------------------------------------
// Items
// -------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/inventory/items",
    tag = "Admin",
    summary = "List inventory items",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<InventoryItem>))
)]
async fn list_items(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<InventoryItem>>> {
    actor
        .require_permission("inventory.read")
        .map_err(ApiError::new)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = InventoryService::new(db).list_items(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/inventory/items",
    tag = "Admin",
    summary = "Create an inventory item",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateItemRequest,
    responses((status = 201, body = InventoryItem))
)]
async fn create_item(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateItemRequest>,
) -> ApiResult<(StatusCode, Json<InventoryItem>)> {
    actor
        .require_permission("inventory.write")
        .map_err(ApiError::new)?;
    let item = InventoryService::new(db)
        .create_item(map_create_item(request))
        .await?;
    Ok((StatusCode::CREATED, Json(item)))
}

#[utoipa::path(
    get,
    path = "/inventory/items/{id}",
    tag = "Admin",
    summary = "Get an inventory item",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = InventoryItem))
)]
async fn get_item(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<InventoryItem>> {
    actor
        .require_permission("inventory.read")
        .map_err(ApiError::new)?;
    let id = InventoryItemId::from_string(id);
    Ok(Json(InventoryService::new(db).get_item(&id).await?))
}

#[utoipa::path(
    get,
    path = "/inventory/items/by-variant/{variant_id}",
    tag = "Admin",
    summary = "Get an inventory item by variant",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = InventoryItem))
)]
async fn get_item_by_variant(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(variant_id): Path<String>,
) -> ApiResult<Json<InventoryItem>> {
    actor
        .require_permission("inventory.read")
        .map_err(ApiError::new)?;
    let variant_id = VariantId::from_string(variant_id);
    Ok(Json(
        InventoryService::new(db)
            .get_item_by_variant(&variant_id)
            .await?,
    ))
}

// -------------------------------------------------------------------------
// Levels, availability and movements
// -------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/inventory/levels",
    tag = "Admin",
    summary = "List inventory levels",
    security(("adminSession" = []), ("apiToken" = [])),
    params(LevelsQuery),
    responses((status = 200, body = Vec<InventoryLevel>))
)]
async fn list_levels(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<LevelsQuery>,
) -> ApiResult<Json<Vec<InventoryLevel>>> {
    actor
        .require_permission("inventory.read")
        .map_err(ApiError::new)?;
    let service = InventoryService::new(db);
    match (query.item_id, query.location_id) {
        (Some(item_id), _) => Ok(Json(service.list_levels(&item_id).await?)),
        (None, Some(location_id)) => {
            Ok(Json(service.list_levels_for_location(&location_id).await?))
        }
        (None, None) => Err(ApiError::validation("item_id or location_id is required")),
    }
}

#[utoipa::path(
    get,
    path = "/inventory/availability/{variant_id}",
    tag = "Admin",
    summary = "Get variant availability",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = AvailabilityResponse))
)]
async fn get_availability(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(variant_id): Path<String>,
) -> ApiResult<Json<AvailabilityResponse>> {
    actor
        .require_permission("inventory.read")
        .map_err(ApiError::new)?;
    let variant_id = VariantId::from_string(variant_id);
    let available = InventoryService::new(db)
        .variant_availability(&variant_id)
        .await?;
    Ok(Json(AvailabilityResponse {
        variant_id,
        available,
    }))
}

#[utoipa::path(
    get,
    path = "/inventory/movements",
    tag = "Admin",
    summary = "List inventory movements",
    security(("adminSession" = []), ("apiToken" = [])),
    params(MovementsQuery),
    responses((status = 200, body = Vec<InventoryMovement>))
)]
async fn list_movements(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<MovementsQuery>,
) -> ApiResult<Json<Vec<InventoryMovement>>> {
    actor
        .require_permission("inventory.read")
        .map_err(ApiError::new)?;
    let movements = InventoryService::new(db)
        .list_movements(&query.item_id)
        .await?;
    let limit = query.limit.unwrap_or(100);
    Ok(Json(movements.into_iter().take(limit).collect()))
}

// -------------------------------------------------------------------------
// Stock changes
// -------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/inventory/adjustments",
    tag = "Admin",
    summary = "Adjust stock",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = AdjustmentRequest,
    responses((status = 200, body = InventoryLevel))
)]
async fn create_adjustment(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<AdjustmentRequest>,
) -> ApiResult<Json<InventoryLevel>> {
    actor
        .require_permission("inventory.write")
        .map_err(ApiError::new)?;
    // The inventory service derives movement references internally; the optional
    // client reference is accepted for forward compatibility.
    let _ = request.reference;
    let level = InventoryService::new(db)
        .adjust_stock(
            &request.inventory_item_id,
            &request.location_id,
            request.quantity_delta,
            &request.reason,
            actor.user_id.as_ref().map(|user_id| user_id.as_str()),
        )
        .await?;
    Ok(Json(level))
}

#[utoipa::path(
    post,
    path = "/inventory/transfers",
    tag = "Admin",
    summary = "Transfer stock between locations",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = TransferRequest,
    responses((status = 200, body = TransferResponse))
)]
async fn create_transfer(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<TransferRequest>,
) -> ApiResult<Json<TransferResponse>> {
    actor
        .require_permission("inventory.write")
        .map_err(ApiError::new)?;
    // Transfers record their own fixed movement reasons in the service.
    let _ = request.reason;
    let (from, to) = InventoryService::new(db)
        .transfer(
            &request.inventory_item_id,
            &request.from_location_id,
            &request.to_location_id,
            request.quantity,
        )
        .await?;
    Ok(Json(TransferResponse { from, to }))
}

// -------------------------------------------------------------------------
// Reservations
// -------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/inventory/reservations",
    tag = "Admin",
    summary = "Reserve stock for a checkout",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = ReservationRequest,
    responses((status = 201, body = Vec<InventoryReservation>))
)]
async fn create_reservations(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<ReservationRequest>,
) -> ApiResult<(StatusCode, Json<Vec<InventoryReservation>>)> {
    actor
        .require_permission("inventory.write")
        .map_err(ApiError::new)?;
    let reservations = InventoryService::new(db)
        .reserve(
            map_reservation_lines(request.lines),
            ReservationReference::new(request.checkout_id),
            request.ttl_seconds,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(reservations)))
}

#[utoipa::path(
    post,
    path = "/inventory/reservations/release",
    tag = "Admin",
    summary = "Release stock reservations",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = ReleaseReservationsRequest,
    responses((status = 200, body = ReleasedResponse))
)]
async fn release_reservations(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<ReleaseReservationsRequest>,
) -> ApiResult<Json<ReleasedResponse>> {
    actor
        .require_permission("inventory.write")
        .map_err(ApiError::new)?;
    let reservation_ids: Vec<ReservationId> = request
        .reservation_ids
        .into_iter()
        .map(ReservationId::from_string)
        .collect();
    let released = InventoryService::new(db).release(&reservation_ids).await?;
    Ok(Json(ReleasedResponse { released }))
}

#[utoipa::path(
    post,
    path = "/inventory/reservations/commit",
    tag = "Admin",
    summary = "Commit stock reservations",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CommitReservationsRequest,
    responses((status = 200, body = CommittedResponse))
)]
async fn commit_reservations(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CommitReservationsRequest>,
) -> ApiResult<Json<CommittedResponse>> {
    actor
        .require_permission("inventory.write")
        .map_err(ApiError::new)?;
    let committed = InventoryService::new(db)
        .commit(&request.checkout_id, &request.order_id)
        .await?;
    Ok(Json(CommittedResponse { committed }))
}

#[utoipa::path(
    post,
    path = "/inventory/reservations/expire",
    tag = "Admin",
    summary = "Expire due stock reservations",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ReleasedResponse))
)]
async fn expire_reservations(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<ReleasedResponse>> {
    actor
        .require_permission("inventory.write")
        .map_err(ApiError::new)?;
    let released = InventoryService::new(db)
        .expire_due(now_timestamp())
        .await?;
    Ok(Json(ReleasedResponse { released }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_reservation_lines() {
        let variant_id = VariantId::from_string("var_1");
        let line_item_id = LineItemId::from_string("item_1");

        let lines = map_reservation_lines(vec![ReservationLineRequest {
            variant_id: variant_id.clone(),
            quantity: 3,
            line_item_id: Some(line_item_id.clone()),
        }]);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].variant_id, variant_id);
        assert_eq!(lines[0].quantity, 3);
        assert_eq!(lines[0].line_item_id, Some(line_item_id));
    }

    #[test]
    fn maps_location_requests() {
        let input = map_create_location(CreateLocationRequest {
            name: "Main".to_owned(),
            address: None,
            metadata: None,
        });
        assert_eq!(input.name, "Main");
        assert!(input.address.is_none());
        assert!(input.metadata.is_none());

        let update = map_update_location(UpdateLocationRequest {
            name: Some("Warehouse".to_owned()),
            address: None,
            metadata: None,
        });
        assert_eq!(update.name.as_deref(), Some("Warehouse"));
    }
}
