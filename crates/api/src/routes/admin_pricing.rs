//! Admin pricing routes: price lists, prices and deterministic calculation.
//!
//! Pricing is administered with the promotion permissions so that a single
//! catalogue/promotion role can manage every price-affecting surface.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiQuery, ApiResult, PaginationQuery};
use fastcart_kernel::{ActorContext, AppError, CurrencyCode, Metadata, now_timestamp};
use fastcart_module_pricing::{
    CreatePriceInput, CreatePriceListInput, Price, PriceContext, PriceFilter, PriceList,
    PriceListRuleInput, PriceListStatus, PriceListType, PricingService, SelectedPrice,
    UpdatePriceInput, UpdatePriceListInput,
};
use serde::Deserialize;

use crate::dto::PageResponse;
use crate::routes::StoreDb;
use crate::state::AppContext;

const READ_PERMISSION: &str = "promotions.read";
const WRITE_PERMISSION: &str = "promotions.write";

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route(
            "/price-lists",
            get(list_price_lists).post(create_price_list),
        )
        .route(
            "/price-lists/{id}",
            get(get_price_list)
                .patch(update_price_list)
                .delete(delete_price_list),
        )
        .route("/prices", get(list_prices).post(create_price))
        .route("/prices/bulk", post(bulk_upsert_prices))
        .route(
            "/prices/calculate",
            get(calculate_price).post(calculate_prices),
        )
        .route(
            "/prices/{id}",
            get(get_price).patch(update_price).delete(delete_price),
        )
}

// Price lists

async fn list_price_lists(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<PriceList>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = PricingService::new(db).list_price_lists(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn create_price_list(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreatePriceListRequest>,
) -> ApiResult<(StatusCode, Json<PriceList>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let input = request.into_input();
    let price_list = PricingService::new(db).create_price_list(input).await?;
    Ok((StatusCode::CREATED, Json(price_list)))
}

async fn get_price_list(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<PriceList>> {
    require_permission(&actor, READ_PERMISSION)?;
    let price_list = PricingService::new(db).get_price_list(&id).await?;
    Ok(Json(price_list))
}

async fn update_price_list(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdatePriceListRequest>,
) -> ApiResult<Json<PriceList>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let price_list = PricingService::new(db)
        .update_price_list(&id, request.into_input())
        .await?;
    Ok(Json(price_list))
}

async fn delete_price_list(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    PricingService::new(db).delete_price_list(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Prices

async fn list_prices(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<ListPricesQuery>,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<Price>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let filter = query.into_filter()?;
    let page = PricingService::new(db)
        .list_prices(filter, pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn create_price(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<PriceRequest>,
) -> ApiResult<(StatusCode, Json<Price>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let price = PricingService::new(db)
        .create_price(request.into_input()?)
        .await?;
    Ok((StatusCode::CREATED, Json(price)))
}

async fn get_price(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Price>> {
    require_permission(&actor, READ_PERMISSION)?;
    let price = PricingService::new(db).get_price(&id).await?;
    Ok(Json(price))
}

async fn update_price(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdatePriceRequest>,
) -> ApiResult<Json<Price>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let price = PricingService::new(db)
        .update_price(&id, request.into_input()?)
        .await?;
    Ok(Json(price))
}

async fn delete_price(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    PricingService::new(db).delete_price(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn bulk_upsert_prices(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<BulkPricesRequest>,
) -> ApiResult<Json<Vec<Price>>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let inputs = request
        .prices
        .into_iter()
        .map(PriceRequest::into_input)
        .collect::<ApiResult<Vec<_>>>()?;
    let prices = PricingService::new(db).bulk_upsert_prices(inputs).await?;
    Ok(Json(prices))
}

// Calculation

async fn calculate_price(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<CalculatePriceQuery>,
) -> ApiResult<Json<Option<SelectedPrice>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let context = query.into_context()?;
    let selected = PricingService::new(db)
        .calculate_price(&context.variant_id, &context.price_context)
        .await?;
    Ok(Json(selected))
}

async fn calculate_prices(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CalculatePricesRequest>,
) -> ApiResult<Json<HashMap<String, Option<SelectedPrice>>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let (variant_ids, price_context) = request.into_parts()?;
    let selected = PricingService::new(db)
        .calculate_prices(&variant_ids, &price_context)
        .await?;
    Ok(Json(selected))
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

#[derive(Debug, Deserialize)]
struct PriceListRuleRequest {
    attribute: String,
    operator: Option<String>,
    value: String,
}

impl From<PriceListRuleRequest> for PriceListRuleInput {
    fn from(request: PriceListRuleRequest) -> Self {
        Self {
            attribute: request.attribute,
            operator: request.operator,
            value: request.value,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CreatePriceListRequest {
    title: String,
    description: Option<String>,
    price_type: Option<PriceListType>,
    status: Option<PriceListStatus>,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    metadata: Option<Metadata>,
    #[serde(default)]
    rules: Vec<PriceListRuleRequest>,
}

impl CreatePriceListRequest {
    fn into_input(self) -> CreatePriceListInput {
        CreatePriceListInput {
            title: self.title,
            description: self.description,
            price_type: self.price_type.unwrap_or_default(),
            status: self.status.unwrap_or_default(),
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            metadata: self.metadata.unwrap_or_default(),
            rules: self.rules.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct UpdatePriceListRequest {
    title: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    description: Option<Option<String>>,
    price_type: Option<PriceListType>,
    status: Option<PriceListStatus>,
    #[serde(default, deserialize_with = "nullable")]
    starts_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "nullable")]
    ends_at: Option<Option<i64>>,
    metadata: Option<Metadata>,
    rules: Option<Vec<PriceListRuleRequest>>,
}

impl UpdatePriceListRequest {
    fn into_input(self) -> UpdatePriceListInput {
        UpdatePriceListInput {
            title: self.title,
            description: self.description,
            price_type: self.price_type,
            status: self.status,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            metadata: self.metadata,
            rules: self
                .rules
                .map(|rules| rules.into_iter().map(Into::into).collect()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ListPricesQuery {
    price_list_id: Option<String>,
    variant_id: Option<String>,
    currency: Option<String>,
}

impl ListPricesQuery {
    fn into_filter(self) -> ApiResult<PriceFilter> {
        let currency_code = self
            .currency
            .map(CurrencyCode::new)
            .transpose()
            .map_err(AppError::from)?;
        Ok(PriceFilter {
            variant_id: self.variant_id,
            price_list_id: self.price_list_id,
            currency_code,
        })
    }
}

#[derive(Debug, Deserialize)]
struct PriceRequest {
    price_list_id: Option<String>,
    variant_id: String,
    currency_code: String,
    amount: i64,
    min_quantity: Option<i64>,
    max_quantity: Option<i64>,
    customer_group_id: Option<String>,
    sales_channel_id: Option<String>,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    metadata: Option<Metadata>,
}

impl PriceRequest {
    fn into_input(self) -> ApiResult<CreatePriceInput> {
        let currency_code = CurrencyCode::new(self.currency_code).map_err(AppError::from)?;
        let mut input = CreatePriceInput::new(self.variant_id, currency_code, self.amount);
        input.price_list_id = self.price_list_id;
        input.min_quantity = self.min_quantity.unwrap_or(1);
        input.max_quantity = self.max_quantity;
        input.customer_group_id = self.customer_group_id;
        input.sales_channel_id = self.sales_channel_id;
        input.starts_at = self.starts_at;
        input.ends_at = self.ends_at;
        input.metadata = self.metadata.unwrap_or_default();
        Ok(input)
    }
}

#[derive(Debug, Deserialize)]
struct UpdatePriceRequest {
    #[serde(default, deserialize_with = "nullable")]
    price_list_id: Option<Option<String>>,
    variant_id: Option<String>,
    currency_code: Option<String>,
    amount: Option<i64>,
    min_quantity: Option<i64>,
    #[serde(default, deserialize_with = "nullable")]
    max_quantity: Option<Option<i64>>,
    #[serde(default, deserialize_with = "nullable")]
    customer_group_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    sales_channel_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    starts_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "nullable")]
    ends_at: Option<Option<i64>>,
    metadata: Option<Metadata>,
}

impl UpdatePriceRequest {
    fn into_input(self) -> ApiResult<UpdatePriceInput> {
        let currency_code = self
            .currency_code
            .map(CurrencyCode::new)
            .transpose()
            .map_err(AppError::from)?;
        Ok(UpdatePriceInput {
            price_list_id: self.price_list_id,
            variant_id: self.variant_id,
            currency_code,
            amount: self.amount,
            min_quantity: self.min_quantity,
            max_quantity: self.max_quantity,
            customer_group_id: self.customer_group_id,
            sales_channel_id: self.sales_channel_id,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            metadata: self.metadata,
        })
    }
}

#[derive(Debug, Deserialize)]
struct BulkPricesRequest {
    prices: Vec<PriceRequest>,
}

#[derive(Debug, Deserialize)]
struct CalculatePriceQuery {
    variant_id: String,
    currency: String,
    quantity: Option<i64>,
    customer_group_id: Option<String>,
    sales_channel_id: Option<String>,
}

struct CalculatePriceParts {
    variant_id: String,
    price_context: PriceContext,
}

impl CalculatePriceQuery {
    fn into_context(self) -> ApiResult<CalculatePriceParts> {
        let currency = CurrencyCode::new(self.currency).map_err(AppError::from)?;
        Ok(CalculatePriceParts {
            variant_id: self.variant_id,
            price_context: PriceContext {
                currency,
                quantity: self.quantity.unwrap_or(1),
                customer_group_id: self.customer_group_id,
                sales_channel_id: self.sales_channel_id,
                at: now_timestamp(),
            },
        })
    }
}

#[derive(Debug, Deserialize)]
struct CalculatePricesRequest {
    variant_ids: Vec<String>,
    currency: String,
    quantity: Option<i64>,
    customer_group_id: Option<String>,
    sales_channel_id: Option<String>,
}

impl CalculatePricesRequest {
    fn into_parts(self) -> ApiResult<(Vec<String>, PriceContext)> {
        let currency = CurrencyCode::new(self.currency).map_err(AppError::from)?;
        Ok((
            self.variant_ids,
            PriceContext {
                currency,
                quantity: self.quantity.unwrap_or(1),
                customer_group_id: self.customer_group_id,
                sales_channel_id: self.sales_channel_id,
                at: now_timestamp(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{PriceRequest, UpdatePriceListRequest};

    #[test]
    fn nullable_fields_distinguish_null_from_absence() {
        let absent: UpdatePriceListRequest = serde_json::from_str("{}").expect("request");
        assert_eq!(absent.description, None);

        let cleared: UpdatePriceListRequest =
            serde_json::from_str(r#"{"description":null}"#).expect("request");
        assert_eq!(cleared.description, Some(None));

        let replaced: UpdatePriceListRequest =
            serde_json::from_str(r#"{"description":"new"}"#).expect("request");
        assert_eq!(replaced.description, Some(Some("new".to_owned())));
    }

    #[test]
    fn maps_price_requests_to_module_inputs() {
        let request: PriceRequest =
            serde_json::from_str(r#"{"variant_id":"var_1","currency_code":"usd","amount":1500}"#)
                .expect("request");
        let input = request.into_input().expect("input");

        assert_eq!(input.variant_id, "var_1");
        assert_eq!(input.currency_code.as_str(), "USD");
        assert_eq!(input.amount, 1500);
        assert_eq!(input.min_quantity, 1);
    }
}
