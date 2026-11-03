//! Shipping domain entities, inputs and rate-selection rules.

use fastcart_kernel::{
    AppError, CountryCode, CurrencyCode, Metadata, ShippingOptionId, ShippingProfileId, Timestamp,
    validate_length, validate_non_negative_amount,
};
use serde_json::Value;

/// A group of shipping options fulfilled by the same provider setup.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FulfillmentSet {
    /// Set identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Set type, currently `shipping`.
    pub set_type: String,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating a fulfillment set.
#[derive(Clone, Debug, Default)]
pub struct CreateFulfillmentSetInput {
    /// Human-readable name.
    pub name: String,
    /// Set type; defaults to `shipping`.
    pub set_type: Option<String>,
    /// Extension metadata.
    pub metadata: Option<Metadata>,
}

/// Input for partially updating a fulfillment set.
#[derive(Clone, Debug, Default)]
pub struct UpdateFulfillmentSetInput {
    /// Replacement name.
    pub name: Option<String>,
    /// Replacement set type.
    pub set_type: Option<String>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// A shipping profile groups options that share fulfilment rules.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShippingProfile {
    /// Profile identifier.
    pub id: ShippingProfileId,
    /// Human-readable name.
    pub name: String,
    /// Profile type, currently `default`.
    pub profile_type: String,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating a shipping profile.
#[derive(Clone, Debug, Default)]
pub struct CreateShippingProfileInput {
    /// Human-readable name.
    pub name: String,
    /// Profile type; defaults to `default`.
    pub profile_type: Option<String>,
    /// Extension metadata.
    pub metadata: Option<Metadata>,
}

/// Input for partially updating a shipping profile.
#[derive(Clone, Debug, Default)]
pub struct UpdateShippingProfileInput {
    /// Replacement name.
    pub name: Option<String>,
    /// Replacement profile type.
    pub profile_type: Option<String>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// A purchasable flat-rate shipping choice.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShippingOption {
    /// Option identifier.
    pub id: ShippingOptionId,
    /// Owning profile.
    pub profile_id: ShippingProfileId,
    /// Human-readable name.
    pub name: String,
    /// Pricing model, currently `flat`.
    pub price_type: String,
    /// Flat amount in minor units.
    pub amount_minor: i64,
    /// Currency of the flat amount.
    pub currency: CurrencyCode,
    /// Registered fulfillment provider that owns the option.
    pub provider_id: String,
    /// Provider-specific configuration.
    pub provider_data: Value,
    /// Optional fulfillment set grouping.
    pub fulfillment_set_id: Option<String>,
    /// Whether the option can be offered to customers.
    pub is_enabled: bool,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating a shipping option.
#[derive(Clone, Debug)]
pub struct CreateShippingOptionInput {
    /// Owning profile.
    pub profile_id: ShippingProfileId,
    /// Human-readable name.
    pub name: String,
    /// Flat amount in minor units.
    pub amount_minor: i64,
    /// Currency of the flat amount.
    pub currency: CurrencyCode,
    /// Fulfillment provider identifier; defaults to `manual`.
    pub provider_id: Option<String>,
    /// Provider-specific configuration.
    pub provider_data: Option<Value>,
    /// Optional fulfillment set grouping.
    pub fulfillment_set_id: Option<String>,
    /// Whether the option starts enabled.
    pub is_enabled: Option<bool>,
    /// Extension metadata.
    pub metadata: Option<Metadata>,
}

/// Input for partially updating a shipping option.
#[derive(Clone, Debug, Default)]
pub struct UpdateShippingOptionInput {
    /// Replacement name.
    pub name: Option<String>,
    /// Replacement flat amount.
    pub amount_minor: Option<i64>,
    /// Replacement currency.
    pub currency: Option<CurrencyCode>,
    /// Replacement fulfillment provider.
    pub provider_id: Option<String>,
    /// Replacement provider configuration.
    pub provider_data: Option<Value>,
    /// Replacement fulfillment set; `Some(None)` clears it.
    pub fulfillment_set_id: Option<Option<String>>,
    /// Replacement enabled flag.
    pub is_enabled: Option<bool>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// A destination- and subtotal-specific shipping price.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShippingRate {
    /// Rate identifier.
    pub id: String,
    /// Owning option.
    pub shipping_option_id: ShippingOptionId,
    /// Restrict to one country when present.
    pub country_code: Option<CountryCode>,
    /// Restrict to one province/state when present.
    pub province_code: Option<String>,
    /// Amount in minor units.
    pub amount_minor: i64,
    /// Currency of the amount.
    pub currency: CurrencyCode,
    /// Inclusive lower subtotal bound.
    pub min_subtotal: Option<i64>,
    /// Inclusive upper subtotal bound.
    pub max_subtotal: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// Input for creating a shipping rate.
#[derive(Clone, Debug)]
pub struct CreateShippingRateInput {
    /// Owning option.
    pub shipping_option_id: ShippingOptionId,
    /// Optional country restriction.
    pub country_code: Option<CountryCode>,
    /// Optional province restriction.
    pub province_code: Option<String>,
    /// Amount in minor units.
    pub amount_minor: i64,
    /// Currency of the amount.
    pub currency: CurrencyCode,
    /// Inclusive lower subtotal bound.
    pub min_subtotal: Option<i64>,
    /// Inclusive upper subtotal bound.
    pub max_subtotal: Option<i64>,
    /// Extension metadata.
    pub metadata: Option<Metadata>,
}

/// Input for partially updating a shipping rate.
#[derive(Clone, Debug, Default)]
pub struct UpdateShippingRateInput {
    /// Restricts to a country, or clears the restriction when `Some(None)`.
    pub country_code: Option<Option<CountryCode>>,
    /// Restricts to a province, or clears the restriction when `Some(None)`.
    pub province_code: Option<Option<String>>,
    /// Replacement amount.
    pub amount_minor: Option<i64>,
    /// Replacement currency.
    pub currency: Option<CurrencyCode>,
    /// Replacement lower subtotal bound.
    pub min_subtotal: Option<Option<i64>>,
    /// Replacement upper subtotal bound.
    pub max_subtotal: Option<Option<i64>>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// The rate chosen for a destination and cart subtotal.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SelectedShippingRate {
    /// Option being priced.
    pub option_id: ShippingOptionId,
    /// Matched rate, or `None` when the option's flat amount was used.
    pub rate_id: Option<String>,
    /// Resolved amount in minor units.
    pub amount_minor: i64,
    /// Resolved currency.
    pub currency: CurrencyCode,
    /// True when the option flat amount was used instead of a rate row.
    pub is_fallback: bool,
}

/// An enabled option with its resolved amount for a destination.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedShippingOption {
    /// The option record.
    pub option: ShippingOption,
    /// Resolved amount in minor units.
    pub amount_minor: i64,
    /// Resolved currency.
    pub currency: CurrencyCode,
    /// Matched rate, or `None` when the flat amount was used.
    pub rate_id: Option<String>,
}

/// Validates a profile or option name.
pub fn validate_shipping_name(name: &str, field: &str) -> Result<(), AppError> {
    validate_length(name, field, 200)
}

/// Validates a province code, normalizing whitespace and casing.
pub fn normalize_province(province: Option<&str>) -> Option<String> {
    province
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_uppercase)
}

/// Validates a shipping amount in minor units.
pub fn validate_amount(amount_minor: i64) -> Result<(), AppError> {
    validate_non_negative_amount(amount_minor, "amount")
}

/// Validates that subtotal bounds are non-negative and ordered.
pub fn validate_subtotal_range(
    min_subtotal: Option<i64>,
    max_subtotal: Option<i64>,
) -> Result<(), AppError> {
    for bound in [min_subtotal, max_subtotal].into_iter().flatten() {
        validate_non_negative_amount(bound, "subtotal bound")?;
    }
    if let (Some(minimum), Some(maximum)) = (min_subtotal, max_subtotal) {
        if minimum > maximum {
            return Err(AppError::validation(
                "min_subtotal must not exceed max_subtotal",
            ));
        }
    }
    Ok(())
}

/// Returns a specificity score when the rate applies, otherwise `None`.
///
/// Country matches dominate province matches so that the precedence is
/// province+country > country > wildcard. Subtotal bounds are inclusive.
#[must_use]
pub fn rate_specificity(
    rate: &ShippingRate,
    country_code: &CountryCode,
    province_code: Option<&str>,
    subtotal: i64,
) -> Option<u8> {
    let country_score = match rate.country_code.as_ref() {
        Some(code) if code == country_code => 10,
        None => 0,
        Some(_) => return None,
    };
    let requested_province = normalize_province(province_code);
    let province_score = match (rate.province_code.as_deref(), requested_province.as_deref()) {
        (Some(rate_province), Some(requested)) if rate_province.eq_ignore_ascii_case(requested) => {
            1
        }
        (None, _) => 0,
        (Some(_), _) => return None,
    };
    if rate.min_subtotal.is_some_and(|minimum| subtotal < minimum) {
        return None;
    }
    if rate.max_subtotal.is_some_and(|maximum| subtotal > maximum) {
        return None;
    }
    Some(country_score + province_score)
}
