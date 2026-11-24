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

/// Builds the documented shipping routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_profiles, create_profile))
        .routes(routes!(get_profile, update_profile, delete_profile))
        .routes(routes!(list_options, create_option))
        .routes(routes!(list_options_for_destination))
        .routes(routes!(get_option, update_option, delete_option))
        .routes(routes!(list_rates, create_rate))
        .routes(routes!(select_rate))
        .routes(routes!(add_location))
        .routes(routes!(remove_location))
        .routes(routes!(get_rate, update_rate, delete_rate))
        .routes(routes!(list_fulfillment_sets, create_fulfillment_set))
        .routes(routes!(
            get_fulfillment_set,
            update_fulfillment_set,
            delete_fulfillment_set
        ))
        .routes(routes!(list_providers))
}

// Fulfillment sets and providers

mod fulfillment_sets;
mod options;
mod profiles;
mod rates;

use fulfillment_sets::*;
use options::*;
use profiles::*;
use rates::*;

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

#[utoipa::path(
    get,
    path = "/shipping/providers",
    tag = "Admin",
    summary = "List registered fulfillment providers",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<String>))
)]
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

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
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

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct DestinationOptionsQuery {
    country: String,
    subtotal: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
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
