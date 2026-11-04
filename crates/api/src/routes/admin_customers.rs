//! Admin customer HTTP routes.
//!
//! Handlers translate HTTP requests into [`CustomerService`] calls after
//! enforcing the `customers.read` / `customers.write` permissions.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiQuery, ApiResult, PaginationQuery};
use fastcart_kernel::{CustomerId, Metadata};
use fastcart_module_customer::{
    Address, AddressInput, CustomerService, CustomerStatus, UpdateCustomerInput,
};
use serde::Deserialize;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::dto::{CustomerAddressDto, CustomerDto, CustomerGroupDto, PageResponse};
use crate::state::AppContext;

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/customers", get(list_customers))
        .route(
            "/customers/{id}",
            get(get_customer)
                .patch(update_customer)
                .delete(delete_customer),
        )
        .route(
            "/customers/{id}/addresses",
            get(list_addresses).post(create_address),
        )
        .route(
            "/customers/{id}/addresses/{address_id}",
            axum::routing::patch(update_address).delete(delete_address),
        )
        .route("/customer-groups", get(list_groups).post(create_group))
        .route(
            "/customer-groups/{group_id}/customers",
            post(add_group_customer),
        )
        .route(
            "/customer-groups/{group_id}/customers/{customer_id}",
            axum::routing::delete(remove_group_customer),
        )
        .route("/customers/{id}/groups", get(list_customer_groups))
}

/// Builds the documented customer routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_customers))
        .routes(routes!(get_customer, update_customer, delete_customer))
        .routes(routes!(list_addresses, create_address))
        .routes(routes!(update_address, delete_address))
        .routes(routes!(list_groups, create_group))
        .routes(routes!(add_group_customer))
        .routes(routes!(remove_group_customer))
        .routes(routes!(list_customer_groups))
}

// -------------------------------------------------------------------------
// Requests
// -------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct CustomerListQuery {
    query: Option<String>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateCustomerRequest {
    email: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
    phone: Option<String>,
    #[schema(value_type = Option<String>)]
    status: Option<CustomerStatus>,
    metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct AdminAddressRequest {
    is_default_shipping: bool,
    is_default_billing: bool,
    first_name: Option<String>,
    last_name: Option<String>,
    company: Option<String>,
    address_1: Option<String>,
    address_2: Option<String>,
    city: Option<String>,
    province: Option<String>,
    postal_code: Option<String>,
    country_code: Option<String>,
    phone: Option<String>,
    metadata: Metadata,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateGroupRequest {
    name: String,
    metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct GroupCustomerRequest {
    customer_id: CustomerId,
}

// -------------------------------------------------------------------------
// Pure request mapping
// -------------------------------------------------------------------------

fn map_update_customer(request: UpdateCustomerRequest) -> UpdateCustomerInput {
    UpdateCustomerInput {
        email: request.email,
        first_name: request.first_name,
        last_name: request.last_name,
        phone: request.phone,
        status: request.status,
        metadata: request.metadata,
    }
}

fn map_address_input(request: AdminAddressRequest) -> AddressInput {
    AddressInput {
        is_default_shipping: request.is_default_shipping,
        is_default_billing: request.is_default_billing,
        first_name: request.first_name,
        last_name: request.last_name,
        company: request.company,
        address_1: request.address_1,
        address_2: request.address_2,
        city: request.city,
        province: request.province,
        postal_code: request.postal_code,
        country_code: request.country_code,
        phone: request.phone,
        metadata: request.metadata,
    }
}

// -------------------------------------------------------------------------
// Customers
// -------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/customers",
    tag = "Admin",
    summary = "List customers",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery, CustomerListQuery),
    responses((status = 200, body = PageResponse<CustomerDto>))
)]
async fn list_customers(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
    ApiQuery(query): ApiQuery<CustomerListQuery>,
) -> ApiResult<Json<PageResponse<CustomerDto>>> {
    actor
        .require_permission("customers.read")
        .map_err(ApiError::new)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CustomerService::new(db)
        .list_customers(pagination, query.query.as_deref())
        .await?;
    Ok(Json(PageResponse::from_page(page).map(CustomerDto::from)))
}

#[utoipa::path(
    get,
    path = "/customers/{id}",
    tag = "Admin",
    summary = "Get a customer",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = CustomerDto))
)]
async fn get_customer(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<CustomerDto>> {
    actor
        .require_permission("customers.read")
        .map_err(ApiError::new)?;
    let id = CustomerId::from_string(id);
    let customer = CustomerService::new(db).get_customer(&id).await?;
    Ok(Json(customer.into()))
}

#[utoipa::path(
    patch,
    path = "/customers/{id}",
    tag = "Admin",
    summary = "Update a customer",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateCustomerRequest,
    responses((status = 200, body = CustomerDto))
)]
async fn update_customer(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateCustomerRequest>,
) -> ApiResult<Json<CustomerDto>> {
    actor
        .require_permission("customers.write")
        .map_err(ApiError::new)?;
    let id = CustomerId::from_string(id);
    let customer = CustomerService::new(db)
        .update_customer(&id, map_update_customer(request))
        .await?;
    Ok(Json(customer.into()))
}

#[utoipa::path(
    delete,
    path = "/customers/{id}",
    tag = "Admin",
    summary = "Delete a customer",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
async fn delete_customer(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    actor
        .require_permission("customers.write")
        .map_err(ApiError::new)?;
    let id = CustomerId::from_string(id);
    CustomerService::new(db).delete_customer(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------------------
// Addresses
// -------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/customers/{id}/addresses",
    tag = "Admin",
    summary = "List customer addresses",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<CustomerAddressDto>))
)]
async fn list_addresses(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<Address>>> {
    actor
        .require_permission("customers.read")
        .map_err(ApiError::new)?;
    let id = CustomerId::from_string(id);
    Ok(Json(CustomerService::new(db).list_addresses(&id).await?))
}

#[utoipa::path(
    post,
    path = "/customers/{id}/addresses",
    tag = "Admin",
    summary = "Add a customer address",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = AdminAddressRequest,
    responses((status = 201, body = CustomerAddressDto))
)]
async fn create_address(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<AdminAddressRequest>,
) -> ApiResult<(StatusCode, Json<Address>)> {
    actor
        .require_permission("customers.write")
        .map_err(ApiError::new)?;
    let id = CustomerId::from_string(id);
    let address = CustomerService::new(db)
        .add_address(&id, map_address_input(request))
        .await?;
    Ok((StatusCode::CREATED, Json(address)))
}

#[utoipa::path(
    patch,
    path = "/customers/{id}/addresses/{address_id}",
    tag = "Admin",
    summary = "Update a customer address",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = AdminAddressRequest,
    responses((status = 200, body = CustomerAddressDto))
)]
async fn update_address(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, address_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<AdminAddressRequest>,
) -> ApiResult<Json<Address>> {
    actor
        .require_permission("customers.write")
        .map_err(ApiError::new)?;
    let id = CustomerId::from_string(id);
    let address = CustomerService::new(db)
        .update_address(&id, &address_id, map_address_input(request))
        .await?;
    Ok(Json(address))
}

#[utoipa::path(
    delete,
    path = "/customers/{id}/addresses/{address_id}",
    tag = "Admin",
    summary = "Remove a customer address",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
async fn delete_address(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, address_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    actor
        .require_permission("customers.write")
        .map_err(ApiError::new)?;
    let id = CustomerId::from_string(id);
    CustomerService::new(db)
        .remove_address(&id, &address_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------------------
// Customer groups
// -------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/customer-groups",
    tag = "Admin",
    summary = "List customer groups",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<CustomerGroupDto>))
)]
async fn list_groups(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<Vec<CustomerGroupDto>>> {
    actor
        .require_permission("customers.read")
        .map_err(ApiError::new)?;
    let groups = CustomerService::new(db).list_groups().await?;
    Ok(Json(groups.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    post,
    path = "/customer-groups",
    tag = "Admin",
    summary = "Create a customer group",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateGroupRequest,
    responses((status = 201, body = CustomerGroupDto))
)]
async fn create_group(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateGroupRequest>,
) -> ApiResult<(StatusCode, Json<CustomerGroupDto>)> {
    actor
        .require_permission("customers.write")
        .map_err(ApiError::new)?;
    let group = CustomerService::new(db)
        .create_group(&request.name, request.metadata.unwrap_or_default())
        .await?;
    Ok((StatusCode::CREATED, Json(group.into())))
}

#[utoipa::path(
    post,
    path = "/customer-groups/{group_id}/customers",
    tag = "Admin",
    summary = "Assign a customer to a group",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = GroupCustomerRequest,
    responses((status = 204, description = "No content"))
)]
async fn add_group_customer(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(group_id): Path<String>,
    ApiJson(request): ApiJson<GroupCustomerRequest>,
) -> ApiResult<StatusCode> {
    actor
        .require_permission("customers.write")
        .map_err(ApiError::new)?;
    CustomerService::new(db)
        .assign_group(&request.customer_id, &group_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/customer-groups/{group_id}/customers/{customer_id}",
    tag = "Admin",
    summary = "Remove a customer from a group",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
async fn remove_group_customer(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((group_id, customer_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    actor
        .require_permission("customers.write")
        .map_err(ApiError::new)?;
    let customer_id = CustomerId::from_string(customer_id);
    CustomerService::new(db)
        .remove_group(&customer_id, &group_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/customers/{id}/groups",
    tag = "Admin",
    summary = "List a customer's groups",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<CustomerGroupDto>))
)]
async fn list_customer_groups(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<CustomerGroupDto>>> {
    actor
        .require_permission("customers.read")
        .map_err(ApiError::new)?;
    let id = CustomerId::from_string(id);
    let groups = CustomerService::new(db).list_customer_groups(&id).await?;
    Ok(Json(groups.into_iter().map(Into::into).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_update_customer_fields() {
        let input = map_update_customer(UpdateCustomerRequest {
            email: Some("new@example.com".to_owned()),
            first_name: Some("Ada".to_owned()),
            last_name: None,
            phone: Some("+1 555 0100".to_owned()),
            status: Some(CustomerStatus::Disabled),
            metadata: Some(Metadata::empty()),
        });

        assert_eq!(input.email.as_deref(), Some("new@example.com"));
        assert_eq!(input.first_name.as_deref(), Some("Ada"));
        assert!(input.last_name.is_none());
        assert_eq!(input.status, Some(CustomerStatus::Disabled));
        assert!(input.metadata.is_some());
    }

    #[test]
    fn maps_address_input() {
        let input = map_address_input(AdminAddressRequest {
            is_default_shipping: true,
            is_default_billing: false,
            address_1: Some("1 Main St".to_owned()),
            city: Some("Springfield".to_owned()),
            postal_code: Some("12345".to_owned()),
            country_code: Some("US".to_owned()),
            metadata: Metadata::empty(),
            ..AdminAddressRequest::default()
        });

        assert!(input.is_default_shipping);
        assert!(!input.is_default_billing);
        assert_eq!(input.address_1.as_deref(), Some("1 Main St"));
        assert_eq!(input.city.as_deref(), Some("Springfield"));
        assert_eq!(input.postal_code.as_deref(), Some("12345"));
        assert_eq!(input.country_code.as_deref(), Some("US"));
    }
}
