//! Customer-facing return requests.
//!
//! Customers may request and follow returns for their own orders only. Handlers
//! never take a customer identifier from the client: ownership always comes
//! from the authenticated actor, and an order that belongs to someone else
//! answers `not found` so its existence does not leak.

use super::*;

use crate::dto::{CreateCustomerReturnRequest, CustomerReturnView};

#[utoipa::path(
    post,
    path = "/customers/me/orders/{order_id}/returns",
    tag = "Store",
    summary = "Request a return for one of the current customer's orders",
    request_body = CreateCustomerReturnRequest,
    security(("customerSession" = [])),
    responses((status = 201, body = CustomerReturnView))
)]
pub(super) async fn create_own_return(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    Path(order_id): Path<String>,
    ApiJson(request): ApiJson<CreateCustomerReturnRequest>,
) -> ApiResult<(StatusCode, Json<CustomerReturnView>)> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let order_return = OrderService::new(db)
        .create_customer_return(
            &customer_id,
            &OrderId::from_string(order_id),
            request.into_input(),
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CustomerReturnView::from(order_return)),
    ))
}

#[utoipa::path(
    get,
    path = "/customers/me/orders/{order_id}/returns",
    tag = "Store",
    summary = "List the current customer's returns for an order",
    security(("customerSession" = [])),
    responses((status = 200, body = Vec<CustomerReturnView>))
)]
pub(super) async fn list_own_returns(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    Path(order_id): Path<String>,
) -> ApiResult<Json<Vec<CustomerReturnView>>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let returns = OrderService::new(db)
        .list_customer_returns(&customer_id, &OrderId::from_string(order_id))
        .await?;
    Ok(Json(
        returns.into_iter().map(CustomerReturnView::from).collect(),
    ))
}

#[utoipa::path(
    get,
    path = "/customers/me/orders/{order_id}/returns/{return_id}",
    tag = "Store",
    summary = "Get one of the current customer's returns",
    security(("customerSession" = [])),
    responses((status = 200, body = CustomerReturnView))
)]
pub(super) async fn get_own_return(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    Path((order_id, return_id)): Path<(String, String)>,
) -> ApiResult<Json<CustomerReturnView>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let order_return = OrderService::new(db)
        .get_customer_return(
            &customer_id,
            &OrderId::from_string(order_id),
            &ReturnId::from_string(return_id),
        )
        .await?;
    Ok(Json(CustomerReturnView::from(order_return)))
}
