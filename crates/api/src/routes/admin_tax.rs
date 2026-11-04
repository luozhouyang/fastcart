//! Admin tax routes: categories, regions, rates and tax calculation.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiQuery, ApiResult, PaginationQuery};
use fastcart_kernel::{
    ActorContext, AppError, CountryCode, Metadata, TaxCategoryId, TaxRateId, TaxRegionId,
};
use fastcart_module_tax::{
    CreateTaxCategoryInput, CreateTaxRateInput, CreateTaxRegionInput, TaxBreakdown, TaxCategory,
    TaxLine, TaxRate, TaxRegion, TaxService, UpdateTaxCategoryInput, UpdateTaxRateInput,
    UpdateTaxRegionInput,
};
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::dto::PageResponse;
use crate::routes::StoreDb;
use crate::state::AppContext;

const READ_PERMISSION: &str = "settings.read";
const WRITE_PERMISSION: &str = "settings.write";

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route(
            "/tax/categories",
            get(list_categories).post(create_category),
        )
        .route(
            "/tax/categories/{id}",
            get(get_category)
                .patch(update_category)
                .delete(delete_category),
        )
        .route("/tax/regions", get(list_regions).post(create_region))
        .route(
            "/tax/regions/{id}",
            get(get_region).patch(update_region).delete(delete_region),
        )
        .route("/tax/rates", get(list_rates).post(create_rate))
        .route(
            "/tax/rates/{id}",
            get(get_rate).patch(update_rate).delete(delete_rate),
        )
        .route("/tax/calculate", post(calculate))
}

/// Builds the documented tax routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_categories, create_category))
        .routes(routes!(get_category, update_category, delete_category))
        .routes(routes!(list_regions, create_region))
        .routes(routes!(get_region, update_region, delete_region))
        .routes(routes!(list_rates, create_rate))
        .routes(routes!(get_rate, update_rate, delete_rate))
        .routes(routes!(calculate))
}

// Categories

#[utoipa::path(
    get,
    path = "/tax/categories",
    tag = "Admin",
    summary = "List tax categories",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<TaxCategory>))
)]
async fn list_categories(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<TaxCategory>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = TaxService::new(db).list_categories(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/tax/categories",
    tag = "Admin",
    summary = "Create a tax category",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateTaxCategoryRequest,
    responses((status = 201, body = TaxCategory))
)]
async fn create_category(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateTaxCategoryRequest>,
) -> ApiResult<(StatusCode, Json<TaxCategory>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let category = TaxService::new(db)
        .create_category(request.into_input())
        .await?;
    Ok((StatusCode::CREATED, Json(category)))
}

#[utoipa::path(
    get,
    path = "/tax/categories/{id}",
    tag = "Admin",
    summary = "Get a tax category",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = TaxCategory))
)]
async fn get_category(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<TaxCategory>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = TaxCategoryId::from_string(id);
    let category = TaxService::new(db).get_category(&id).await?;
    Ok(Json(category))
}

#[utoipa::path(
    patch,
    path = "/tax/categories/{id}",
    tag = "Admin",
    summary = "Update a tax category",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateTaxCategoryRequest,
    responses((status = 200, body = TaxCategory))
)]
async fn update_category(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateTaxCategoryRequest>,
) -> ApiResult<Json<TaxCategory>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = TaxCategoryId::from_string(id);
    let category = TaxService::new(db)
        .update_category(&id, request.into_input())
        .await?;
    Ok(Json(category))
}

#[utoipa::path(
    delete,
    path = "/tax/categories/{id}",
    tag = "Admin",
    summary = "Delete a tax category",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
async fn delete_category(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = TaxCategoryId::from_string(id);
    TaxService::new(db).delete_category(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Regions

#[utoipa::path(
    get,
    path = "/tax/regions",
    tag = "Admin",
    summary = "List tax regions",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<TaxRegion>))
)]
async fn list_regions(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<TaxRegion>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = TaxService::new(db).list_regions(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/tax/regions",
    tag = "Admin",
    summary = "Create a tax region",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateTaxRegionRequest,
    responses((status = 201, body = TaxRegion))
)]
async fn create_region(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateTaxRegionRequest>,
) -> ApiResult<(StatusCode, Json<TaxRegion>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let region = TaxService::new(db)
        .create_region(request.into_input()?)
        .await?;
    Ok((StatusCode::CREATED, Json(region)))
}

#[utoipa::path(
    get,
    path = "/tax/regions/{id}",
    tag = "Admin",
    summary = "Get a tax region",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = TaxRegion))
)]
async fn get_region(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<TaxRegion>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = TaxRegionId::from_string(id);
    let region = TaxService::new(db).get_region(&id).await?;
    Ok(Json(region))
}

#[utoipa::path(
    patch,
    path = "/tax/regions/{id}",
    tag = "Admin",
    summary = "Update a tax region",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateTaxRegionRequest,
    responses((status = 200, body = TaxRegion))
)]
async fn update_region(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateTaxRegionRequest>,
) -> ApiResult<Json<TaxRegion>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = TaxRegionId::from_string(id);
    let region = TaxService::new(db)
        .update_region(&id, request.into_input()?)
        .await?;
    Ok(Json(region))
}

#[utoipa::path(
    delete,
    path = "/tax/regions/{id}",
    tag = "Admin",
    summary = "Delete a tax region",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
async fn delete_region(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = TaxRegionId::from_string(id);
    TaxService::new(db).delete_region(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Rates

#[utoipa::path(
    get,
    path = "/tax/rates",
    tag = "Admin",
    summary = "List tax rates for a region",
    security(("adminSession" = []), ("apiToken" = [])),
    params(ListRatesQuery),
    responses((status = 200, body = Vec<TaxRate>))
)]
async fn list_rates(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<ListRatesQuery>,
) -> ApiResult<Json<Vec<TaxRate>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let region_id = TaxRegionId::from_string(query.region_id);
    let rates = TaxService::new(db).list_rates(&region_id).await?;
    Ok(Json(rates))
}

#[utoipa::path(
    post,
    path = "/tax/rates",
    tag = "Admin",
    summary = "Create a tax rate",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateTaxRateRequest,
    responses((status = 201, body = TaxRate))
)]
async fn create_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateTaxRateRequest>,
) -> ApiResult<(StatusCode, Json<TaxRate>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let rate = TaxService::new(db)
        .create_rate(request.into_input())
        .await?;
    Ok((StatusCode::CREATED, Json(rate)))
}

#[utoipa::path(
    get,
    path = "/tax/rates/{id}",
    tag = "Admin",
    summary = "Get a tax rate",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = TaxRate))
)]
async fn get_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<TaxRate>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = TaxRateId::from_string(id);
    let rate = TaxService::new(db).get_rate(&id).await?;
    Ok(Json(rate))
}

#[utoipa::path(
    patch,
    path = "/tax/rates/{id}",
    tag = "Admin",
    summary = "Update a tax rate",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateTaxRateRequest,
    responses((status = 200, body = TaxRate))
)]
async fn update_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateTaxRateRequest>,
) -> ApiResult<Json<TaxRate>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = TaxRateId::from_string(id);
    let rate = TaxService::new(db)
        .update_rate(&id, request.into_input())
        .await?;
    Ok(Json(rate))
}

#[utoipa::path(
    delete,
    path = "/tax/rates/{id}",
    tag = "Admin",
    summary = "Delete a tax rate",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
async fn delete_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = TaxRateId::from_string(id);
    TaxService::new(db).delete_rate(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Calculation

#[utoipa::path(
    post,
    path = "/tax/calculate",
    tag = "Admin",
    summary = "Calculate tax for a destination",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = TaxCalculateRequest,
    responses((status = 200, body = TaxCalculationResponse))
)]
async fn calculate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<TaxCalculateRequest>,
) -> ApiResult<Json<TaxCalculationResponse>> {
    require_permission(&actor, READ_PERMISSION)?;
    let country = CountryCode::new(request.country).map_err(AppError::from)?;
    let fallback_category = request.category_id.map(TaxCategoryId::from_string);
    let service = TaxService::new(db);

    let mut lines = Vec::new();
    if request.lines.is_empty() {
        let line = service
            .calculate_for_line(
                &country,
                request.province.as_deref(),
                fallback_category.as_ref(),
                request.amount_minor.unwrap_or(0),
                None,
            )
            .await?;
        lines.push(line);
    } else {
        for line in request.lines {
            let category_id = line
                .category_id
                .map(TaxCategoryId::from_string)
                .or_else(|| fallback_category.clone());
            let calculated = service
                .calculate_for_line(
                    &country,
                    request.province.as_deref(),
                    category_id.as_ref(),
                    line.amount_minor,
                    None,
                )
                .await?;
            lines.push(calculated);
        }
    }

    let totals = lines
        .iter()
        .fold(TaxBreakdown::default(), |totals, line| TaxBreakdown {
            tax_amount: totals.tax_amount.saturating_add(line.breakdown.tax_amount),
            net_amount: totals.net_amount.saturating_add(line.breakdown.net_amount),
            gross_amount: totals
                .gross_amount
                .saturating_add(line.breakdown.gross_amount),
        });

    Ok(Json(TaxCalculationResponse { lines, totals }))
}

// Helpers

fn require_permission(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_permission(permission).map_err(ApiError::new)
}

/// Distinguishes an absent field from an explicit JSON `null`.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateTaxCategoryRequest {
    name: String,
    code: String,
    description: Option<String>,
}

impl CreateTaxCategoryRequest {
    fn into_input(self) -> CreateTaxCategoryInput {
        CreateTaxCategoryInput {
            name: self.name,
            code: self.code,
            description: self.description,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateTaxCategoryRequest {
    name: Option<String>,
    code: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    description: Option<Option<String>>,
}

impl UpdateTaxCategoryRequest {
    fn into_input(self) -> UpdateTaxCategoryInput {
        UpdateTaxCategoryInput {
            name: self.name,
            code: self.code,
            description: self.description,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateTaxRegionRequest {
    name: String,
    country_code: String,
    province_code: Option<String>,
    metadata: Option<Metadata>,
}

impl CreateTaxRegionRequest {
    fn into_input(self) -> ApiResult<CreateTaxRegionInput> {
        Ok(CreateTaxRegionInput {
            name: self.name,
            country_code: CountryCode::new(self.country_code).map_err(AppError::from)?,
            province_code: self.province_code,
            metadata: self.metadata,
        })
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateTaxRegionRequest {
    name: Option<String>,
    country_code: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    province_code: Option<Option<String>>,
    metadata: Option<Metadata>,
}

impl UpdateTaxRegionRequest {
    fn into_input(self) -> ApiResult<UpdateTaxRegionInput> {
        let country_code = self
            .country_code
            .map(CountryCode::new)
            .transpose()
            .map_err(AppError::from)?;
        Ok(UpdateTaxRegionInput {
            name: self.name,
            country_code,
            province_code: self.province_code,
            metadata: self.metadata,
        })
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct ListRatesQuery {
    region_id: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateTaxRateRequest {
    region_id: String,
    category_id: Option<String>,
    name: String,
    rate_bps: i64,
    is_inclusive: Option<bool>,
    is_default: Option<bool>,
    metadata: Option<Metadata>,
}

impl CreateTaxRateRequest {
    fn into_input(self) -> CreateTaxRateInput {
        CreateTaxRateInput {
            tax_region_id: TaxRegionId::from_string(self.region_id),
            tax_category_id: self.category_id.map(TaxCategoryId::from_string),
            name: self.name,
            rate_bps: self.rate_bps,
            is_inclusive: self.is_inclusive.unwrap_or(false),
            is_default: self.is_default.unwrap_or(false),
            metadata: self.metadata,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateTaxRateRequest {
    #[serde(default, deserialize_with = "nullable")]
    category_id: Option<Option<String>>,
    name: Option<String>,
    rate_bps: Option<i64>,
    is_inclusive: Option<bool>,
    is_default: Option<bool>,
    metadata: Option<Metadata>,
}

impl UpdateTaxRateRequest {
    fn into_input(self) -> UpdateTaxRateInput {
        UpdateTaxRateInput {
            tax_category_id: self
                .category_id
                .map(|category_id| category_id.map(TaxCategoryId::from_string)),
            name: self.name,
            rate_bps: self.rate_bps,
            is_inclusive: self.is_inclusive,
            is_default: self.is_default,
            metadata: self.metadata,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TaxCalculateLineRequest {
    amount_minor: i64,
    category_id: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TaxCalculateRequest {
    country: String,
    province: Option<String>,
    category_id: Option<String>,
    amount_minor: Option<i64>,
    #[serde(default)]
    lines: Vec<TaxCalculateLineRequest>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TaxCalculationResponse {
    lines: Vec<TaxLine>,
    totals: TaxBreakdown,
}

#[cfg(test)]
mod tests {
    use super::{TaxCalculateRequest, UpdateTaxCategoryRequest};

    #[test]
    fn maps_calculation_lines_from_json() {
        let request: TaxCalculateRequest = serde_json::from_str(
            r#"{"country":"us","lines":[{"amount_minor":1000},{"amount_minor":2500,"category_id":"taxcat_1"}]}"#,
        )
        .expect("request");

        assert_eq!(request.country, "us");
        assert!(request.amount_minor.is_none());
        assert_eq!(request.lines.len(), 2);
        assert_eq!(request.lines[0].amount_minor, 1000);
        assert_eq!(request.lines[1].category_id.as_deref(), Some("taxcat_1"));
    }

    #[test]
    fn nullable_description_distinguishes_null_from_absence() {
        let absent: UpdateTaxCategoryRequest = serde_json::from_str("{}").expect("request");
        assert_eq!(absent.description, None);

        let cleared: UpdateTaxCategoryRequest =
            serde_json::from_str(r#"{"description":null}"#).expect("request");
        assert_eq!(cleared.description, Some(None));
    }
}
