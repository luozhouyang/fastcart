use super::*;

#[utoipa::path(
    get,
    path = "/customers/me/orders",
    tag = "Store",
    summary = "List the current customer's orders",
    params(PaginationQuery),
    security(("customerSession" = [])),
    responses((status = 200, body = OrderListResponse))
)]
pub(super) async fn list_orders(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<OrderListResponse>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let filter = OrderFilter {
        customer_id: Some(customer_id),
        exclude_drafts: true,
        ..OrderFilter::default()
    };
    let page = OrderService::new(db).list(filter, pagination).await?;
    Ok(Json(OrderListResponse::from_page(page)))
}

#[utoipa::path(
    get,
    path = "/customers/me/orders/{order_id}",
    tag = "Store",
    summary = "Get one of the current customer's orders",
    security(("customerSession" = [])),
    responses((status = 200, body = OrderResponse))
)]
pub(super) async fn get_own_order(
    CustomerActor(actor): CustomerActor,
    StoreDb(db): StoreDb,
    Path(order_id): Path<String>,
) -> ApiResult<Json<OrderResponse>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?;
    let order = OrderService::new(db)
        .get(&OrderId::from_string(order_id))
        .await?;
    if order.customer_id.as_ref() != Some(customer_id) || order.is_draft_order() {
        return Err(ApiError::new(AppError::not_found("order", &order.id)));
    }
    Ok(Json(OrderResponse { order }))
}
