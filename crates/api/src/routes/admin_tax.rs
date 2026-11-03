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

// Categories

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

#[derive(Debug, Deserialize)]
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
