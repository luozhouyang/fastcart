use super::*;

#[utoipa::path(
    get,
    path = "/customers/me/addresses",
    tag = "Store",
    summary = "List the current customer's addresses",
    security(("customerSession" = [])),
    responses((status = 200, body = CustomerAddressListResponse))
)]
pub(super) async fn list_addresses(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
) -> ApiResult<Json<CustomerAddressListResponse>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let addresses = CustomerService::new(db)
        .list_addresses(&customer_id)
        .await?;
    Ok(Json(CustomerAddressListResponse {
        addresses: addresses.into_iter().map(Into::into).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/customers/me/addresses",
    tag = "Store",
    summary = "Add an address for the current customer",
    request_body = AddressRequest,
    security(("customerSession" = [])),
    responses((status = 201, body = CustomerAddressResponse))
)]
pub(super) async fn add_address(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    ApiJson(request): ApiJson<AddressRequest>,
) -> ApiResult<(StatusCode, Json<CustomerAddressResponse>)> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let address = CustomerService::new(db)
        .add_address(&customer_id, address_input(request, None))
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CustomerAddressResponse {
            address: address.into(),
        }),
    ))
}

#[utoipa::path(
    patch,
    path = "/customers/me/addresses/{address_id}",
    tag = "Store",
    summary = "Update the current customer's address",
    request_body = AddressRequest,
    security(("customerSession" = [])),
    responses((status = 200, body = CustomerAddressResponse))
)]
pub(super) async fn update_address(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    Path(address_id): Path<String>,
    ApiJson(request): ApiJson<AddressRequest>,
) -> ApiResult<Json<CustomerAddressResponse>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let service = CustomerService::new(db);
    let existing = service.get_address(&customer_id, &address_id).await?;
    let address = service
        .update_address(
            &customer_id,
            &address_id,
            address_input(request, Some(&existing)),
        )
        .await?;
    Ok(Json(CustomerAddressResponse {
        address: address.into(),
    }))
}

#[utoipa::path(
    delete,
    path = "/customers/me/addresses/{address_id}",
    tag = "Store",
    summary = "Remove the current customer's address",
    security(("customerSession" = [])),
    responses((status = 204))
)]
pub(super) async fn remove_address(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    Path(address_id): Path<String>,
) -> ApiResult<StatusCode> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    CustomerService::new(db)
        .remove_address(&customer_id, &address_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
