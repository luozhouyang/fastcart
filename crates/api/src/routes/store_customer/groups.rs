use super::*;

#[utoipa::path(
    get,
    path = "/customers/me/groups",
    tag = "Store",
    summary = "List the current customer's groups",
    security(("customerSession" = [])),
    responses((status = 200, body = CustomerGroupsResponse))
)]
pub(super) async fn list_groups(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
) -> ApiResult<Json<CustomerGroupsResponse>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let groups = CustomerService::new(db)
        .list_customer_groups(&customer_id)
        .await?;
    Ok(Json(CustomerGroupsResponse {
        customer_groups: groups.into_iter().map(Into::into).collect(),
    }))
}
