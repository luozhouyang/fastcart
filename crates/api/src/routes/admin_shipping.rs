//! Admin shipping routes: profiles, flat-rate options, destination rates and
//! option resolution for a destination.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiQuery, ApiResult, PaginationQuery};
use fastcart_kernel::{
    ActorContext, AppError, CountryCode, CurrencyCode, Metadata, ShippingOptionId,
    ShippingProfileId,
};
use fastcart_module_shipping::{
    CreateFulfillmentSetInput, CreateShippingOptionInput, CreateShippingProfileInput,
    CreateShippingRateInput, FulfillmentSet, ResolvedShippingOption, SelectedShippingRate,
    ShippingOption, ShippingProfile, ShippingRate, ShippingService, UpdateFulfillmentSetInput,
    UpdateShippingOptionInput, UpdateShippingProfileInput, UpdateShippingRateInput,
};
use serde::Deserialize;
use serde_json::Value;

use crate::dto::PageResponse;
use crate::routes::StoreDb;
use crate::state::AppContext;

const READ_PERMISSION: &str = "settings.read";
const WRITE_PERMISSION: &str = "settings.write";

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route(
            "/shipping/profiles",
            get(list_profiles).post(create_profile),
        )
        .route(
            "/shipping/profiles/{id}",
            get(get_profile)
                .patch(update_profile)
                .delete(delete_profile),
        )
        .route("/shipping/options", get(list_options).post(create_option))
        .route("/shipping/options-list", get(list_options_for_destination))
        .route(
            "/shipping/options/{id}",
            get(get_option).patch(update_option).delete(delete_option),
        )
        .route(
            "/shipping/options/{id}/rates",
            get(list_rates).post(create_rate),
        )
        .route("/shipping/options/{id}/rate", get(select_rate))
        .route("/shipping/options/{id}/locations", post(add_location))
        .route(
            "/shipping/options/{id}/locations/{location_id}",
            delete(remove_location),
        )
        .route(
            "/shipping/rates/{id}",
            get(get_rate).patch(update_rate).delete(delete_rate),
        )
        .route(
            "/shipping/fulfillment-sets",
            get(list_fulfillment_sets).post(create_fulfillment_set),
        )
        .route(
            "/shipping/fulfillment-sets/{id}",
            get(get_fulfillment_set)
                .patch(update_fulfillment_set)
                .delete(delete_fulfillment_set),
        )
        .route("/shipping/providers", get(list_providers))
}

// Fulfillment sets and providers

async fn list_fulfillment_sets(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<FulfillmentSet>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = ShippingService::new(db)
        .list_fulfillment_sets(pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn create_fulfillment_set(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateFulfillmentSetRequest>,
) -> ApiResult<(StatusCode, Json<FulfillmentSet>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let set = ShippingService::new(db)
        .create_fulfillment_set(request.into_input())
        .await?;
    Ok((StatusCode::CREATED, Json(set)))
}

async fn get_fulfillment_set(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<FulfillmentSet>> {
    require_permission(&actor, READ_PERMISSION)?;
    Ok(Json(
        ShippingService::new(db).get_fulfillment_set(&id).await?,
    ))
}

async fn update_fulfillment_set(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateFulfillmentSetRequest>,
) -> ApiResult<Json<FulfillmentSet>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    Ok(Json(
        ShippingService::new(db)
            .update_fulfillment_set(&id, request.into_input())
            .await?,
    ))
}

async fn delete_fulfillment_set(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    ShippingService::new(db).delete_fulfillment_set(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_providers(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
) -> ApiResult<Json<Vec<String>>> {
    require_permission(&actor, READ_PERMISSION)?;
    Ok(Json(
        context
            .shipping_providers
            .list()
            .into_iter()
            .map(|provider| provider.id().to_owned())
            .collect(),
    ))
}

// Profiles

async fn list_profiles(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<ShippingProfile>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = ShippingService::new(db).list_profiles(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn create_profile(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateShippingProfileRequest>,
) -> ApiResult<(StatusCode, Json<ShippingProfile>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let profile = ShippingService::new(db)
        .create_profile(request.into_input())
        .await?;
    Ok((StatusCode::CREATED, Json(profile)))
}

async fn get_profile(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ShippingProfile>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = ShippingProfileId::from_string(id);
    let profile = ShippingService::new(db).get_profile(&id).await?;
    Ok(Json(profile))
}

async fn update_profile(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateShippingProfileRequest>,
) -> ApiResult<Json<ShippingProfile>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingProfileId::from_string(id);
    let profile = ShippingService::new(db)
        .update_profile(&id, request.into_input())
        .await?;
    Ok(Json(profile))
}

async fn delete_profile(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingProfileId::from_string(id);
    ShippingService::new(db).delete_profile(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Options

async fn list_options(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<ListOptionsQuery>,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<ShippingOption>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let service = ShippingService::new(db);
    let page = match query.profile_id {
        Some(profile_id) => {
            let profile_id = ShippingProfileId::from_string(profile_id);
            service
                .list_options_for_profile(&profile_id, pagination)
                .await?
        }
        None => service.list_options(pagination).await?,
    };
    Ok(Json(PageResponse::from_page(page)))
}

async fn create_option(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateShippingOptionRequest>,
) -> ApiResult<(StatusCode, Json<ShippingOption>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let input = request.into_input()?;
    let provider_id = input.provider_id.as_deref().unwrap_or("manual");
    ensure_provider(&context, provider_id)?;
    let option = ShippingService::new(db).create_option(input).await?;
    Ok((StatusCode::CREATED, Json(option)))
}

async fn get_option(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ShippingOption>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    let option = ShippingService::new(db).get_option(&id).await?;
    Ok(Json(option))
}

async fn update_option(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateShippingOptionRequest>,
) -> ApiResult<Json<ShippingOption>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    let input = request.into_input()?;
    if let Some(provider_id) = input.provider_id.as_deref() {
        ensure_provider(&context, provider_id)?;
    }
    let option = ShippingService::new(db).update_option(&id, input).await?;
    Ok(Json(option))
}

async fn delete_option(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    ShippingService::new(db).delete_option(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Rates

async fn list_rates(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<ShippingRate>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    let rates = ShippingService::new(db).list_rates(&id).await?;
    Ok(Json(rates))
}

async fn create_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateShippingRateRequest>,
) -> ApiResult<(StatusCode, Json<ShippingRate>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let input = request.into_input(ShippingOptionId::from_string(id))?;
    let rate = ShippingService::new(db).create_rate(input).await?;
    Ok((StatusCode::CREATED, Json(rate)))
}

async fn get_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ShippingRate>> {
    require_permission(&actor, READ_PERMISSION)?;
    let rate = ShippingService::new(db).get_rate(&id).await?;
    Ok(Json(rate))
}

async fn update_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateShippingRateRequest>,
) -> ApiResult<Json<ShippingRate>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let rate = ShippingService::new(db)
        .update_rate(&id, request.into_input()?)
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
    ShippingService::new(db).delete_rate(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Locations

async fn add_location(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<AddLocationRequest>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    ShippingService::new(db)
        .assign_location(&id, &request.location_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_location(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, location_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    ShippingService::new(db)
        .remove_location(&id, &location_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// Resolution

async fn list_options_for_destination(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<DestinationOptionsQuery>,
) -> ApiResult<Json<Vec<ResolvedShippingOption>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let country = CountryCode::new(query.country).map_err(AppError::from)?;
    let options = ShippingService::new(db)
        .list_options_for(&country, query.subtotal.unwrap_or(0))
        .await?;
    Ok(Json(options))
}

async fn select_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiQuery(query): ApiQuery<SelectRateQuery>,
) -> ApiResult<Json<Option<SelectedShippingRate>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    let country = CountryCode::new(query.country).map_err(AppError::from)?;
    let rate = ShippingService::new(db)
        .select_rate(
            &id,
            &country,
            query.province.as_deref(),
            query.subtotal.unwrap_or(0),
        )
        .await?;
    Ok(Json(rate))
}

// Helpers

fn require_permission(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_permission(permission).map_err(ApiError::new)
}

fn ensure_provider(context: &AppContext, provider_id: &str) -> ApiResult<()> {
    if context.shipping_providers.get(provider_id).is_none() {
        return Err(ApiError::new(AppError::validation(format!(
            "unknown fulfillment provider: {provider_id}"
        ))));
    }
    Ok(())
}

/// Distinguishes an absent field from an explicit JSON `null`.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn parse_currency(value: String) -> ApiResult<CurrencyCode> {
    CurrencyCode::new(value).map_err(|error| ApiError::new(AppError::from(error)))
}

fn parse_country(value: Option<String>) -> ApiResult<Option<CountryCode>> {
    value
        .map(CountryCode::new)
        .transpose()
        .map_err(|error| ApiError::new(AppError::from(error)))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateShippingProfileRequest {
    name: String,
    profile_type: Option<String>,
    metadata: Option<Metadata>,
}

impl CreateShippingProfileRequest {
    fn into_input(self) -> CreateShippingProfileInput {
        CreateShippingProfileInput {
            name: self.name,
            profile_type: self.profile_type,
            metadata: self.metadata,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateShippingProfileRequest {
    name: Option<String>,
    profile_type: Option<String>,
    metadata: Option<Metadata>,
}

impl UpdateShippingProfileRequest {
    fn into_input(self) -> UpdateShippingProfileInput {
        UpdateShippingProfileInput {
            name: self.name,
            profile_type: self.profile_type,
            metadata: self.metadata,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ListOptionsQuery {
    profile_id: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateShippingOptionRequest {
    profile_id: String,
    name: String,
    amount_minor: i64,
    currency: String,
    provider_id: Option<String>,
    provider_data: Option<Value>,
    fulfillment_set_id: Option<String>,
    is_enabled: Option<bool>,
    metadata: Option<Metadata>,
}

impl CreateShippingOptionRequest {
    fn into_input(self) -> ApiResult<CreateShippingOptionInput> {
        Ok(CreateShippingOptionInput {
            profile_id: ShippingProfileId::from_string(self.profile_id),
            name: self.name,
            amount_minor: self.amount_minor,
            currency: parse_currency(self.currency)?,
            provider_id: self.provider_id,
            provider_data: self.provider_data,
            fulfillment_set_id: self.fulfillment_set_id,
            is_enabled: self.is_enabled,
            metadata: self.metadata,
        })
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateShippingOptionRequest {
    name: Option<String>,
    amount_minor: Option<i64>,
    currency: Option<String>,
    provider_id: Option<String>,
    provider_data: Option<Value>,
    #[serde(default, deserialize_with = "nullable")]
    fulfillment_set_id: Option<Option<String>>,
    is_enabled: Option<bool>,
    metadata: Option<Metadata>,
}

impl UpdateShippingOptionRequest {
    fn into_input(self) -> ApiResult<UpdateShippingOptionInput> {
        let currency = self.currency.map(parse_currency).transpose()?;
        Ok(UpdateShippingOptionInput {
            name: self.name,
            amount_minor: self.amount_minor,
            currency,
            provider_id: self.provider_id,
            provider_data: self.provider_data,
            fulfillment_set_id: self.fulfillment_set_id,
            is_enabled: self.is_enabled,
            metadata: self.metadata,
        })
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateFulfillmentSetRequest {
    name: String,
    #[serde(rename = "type")]
    set_type: Option<String>,
    metadata: Option<Metadata>,
}

impl CreateFulfillmentSetRequest {
    fn into_input(self) -> CreateFulfillmentSetInput {
        CreateFulfillmentSetInput {
            name: self.name,
            set_type: self.set_type,
            metadata: self.metadata,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateFulfillmentSetRequest {
    name: Option<String>,
    #[serde(rename = "type")]
    set_type: Option<String>,
    metadata: Option<Metadata>,
}

impl UpdateFulfillmentSetRequest {
    fn into_input(self) -> UpdateFulfillmentSetInput {
        UpdateFulfillmentSetInput {
            name: self.name,
            set_type: self.set_type,
            metadata: self.metadata,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateShippingRateRequest {
    country_code: Option<String>,
    province_code: Option<String>,
    amount_minor: i64,
    currency: String,
    min_subtotal: Option<i64>,
    max_subtotal: Option<i64>,
    metadata: Option<Metadata>,
}

impl CreateShippingRateRequest {
    fn into_input(self, option_id: ShippingOptionId) -> ApiResult<CreateShippingRateInput> {
        Ok(CreateShippingRateInput {
            shipping_option_id: option_id,
            country_code: parse_country(self.country_code)?,
            province_code: self.province_code,
            amount_minor: self.amount_minor,
            currency: parse_currency(self.currency)?,
            min_subtotal: self.min_subtotal,
            max_subtotal: self.max_subtotal,
            metadata: self.metadata,
        })
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateShippingRateRequest {
    #[serde(default, deserialize_with = "nullable")]
    country_code: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    province_code: Option<Option<String>>,
    amount_minor: Option<i64>,
    currency: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    min_subtotal: Option<Option<i64>>,
    #[serde(default, deserialize_with = "nullable")]
    max_subtotal: Option<Option<i64>>,
    metadata: Option<Metadata>,
}

impl UpdateShippingRateRequest {
    fn into_input(self) -> ApiResult<UpdateShippingRateInput> {
        let country_code = match self.country_code {
            Some(value) => Some(parse_country(value)?),
            None => None,
        };
        let currency = self.currency.map(parse_currency).transpose()?;
        Ok(UpdateShippingRateInput {
            country_code,
            province_code: self.province_code,
            amount_minor: self.amount_minor,
            currency,
            min_subtotal: self.min_subtotal,
            max_subtotal: self.max_subtotal,
            metadata: self.metadata,
        })
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = AddShippingLocationRequest)]
pub struct AddLocationRequest {
    location_id: String,
}

#[derive(Debug, Deserialize)]
struct DestinationOptionsQuery {
    country: String,
    subtotal: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SelectRateQuery {
    country: String,
    province: Option<String>,
    subtotal: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::{parse_country, parse_currency};

    #[test]
    fn normalizes_currency_and_country_codes() {
        assert_eq!(
            parse_currency("usd".to_owned()).expect("currency").as_str(),
            "USD"
        );
        assert_eq!(
            parse_country(Some("us".to_owned()))
                .expect("country")
                .expect("country")
                .as_str(),
            "US"
        );
        assert!(parse_country(None).expect("country").is_none());
        assert!(parse_currency("us".to_owned()).is_err());
    }
}
