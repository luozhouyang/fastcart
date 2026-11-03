//! Tax domain entities, inputs and the pure tax calculator.

use fastcart_kernel::{
    AppError, CountryCode, Metadata, TaxCategoryId, TaxRateId, TaxRegionId, Timestamp,
    validate_length,
};

/// Maximum accepted rate in basis points (100%).
pub const MAX_RATE_BPS: i64 = 100_000;

/// A product tax category such as standard, reduced or zero-rated.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaxCategory {
    /// Category identifier.
    pub id: TaxCategoryId,
    /// Human-readable name.
    pub name: String,
    /// Stable machine-readable code, unique per store.
    pub code: String,
    /// Optional description.
    pub description: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating a tax category.
#[derive(Clone, Debug, Default)]
pub struct CreateTaxCategoryInput {
    /// Human-readable name.
    pub name: String,
    /// Stable unique code.
    pub code: String,
    /// Optional description.
    pub description: Option<String>,
}

/// Input for partially updating a tax category.
#[derive(Clone, Debug, Default)]
pub struct UpdateTaxCategoryInput {
    /// Replacement name.
    pub name: Option<String>,
    /// Replacement code.
    pub code: Option<String>,
    /// Replacement description; `Some(None)` clears it.
    pub description: Option<Option<String>>,
}

/// A country or country+province tax jurisdiction.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaxRegion {
    /// Region identifier.
    pub id: TaxRegionId,
    /// Human-readable name.
    pub name: String,
    /// Country covered by the region.
    pub country_code: CountryCode,
    /// Optional province/state covered by the region.
    pub province_code: Option<String>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating a tax region.
#[derive(Clone, Debug)]
pub struct CreateTaxRegionInput {
    /// Human-readable name.
    pub name: String,
    /// Country covered by the region.
    pub country_code: CountryCode,
    /// Optional province/state.
    pub province_code: Option<String>,
    /// Extension metadata.
    pub metadata: Option<Metadata>,
}

/// Input for partially updating a tax region.
#[derive(Clone, Debug, Default)]
pub struct UpdateTaxRegionInput {
    /// Replacement name.
    pub name: Option<String>,
    /// Replacement country.
    pub country_code: Option<CountryCode>,
    /// Replacement province; `Some(None)` clears it.
    pub province_code: Option<Option<String>>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// A configured tax rate for a region and optional category.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaxRate {
    /// Rate identifier.
    pub id: TaxRateId,
    /// Region the rate applies to.
    pub tax_region_id: TaxRegionId,
    /// Optional category the rate applies to.
    pub tax_category_id: Option<TaxCategoryId>,
    /// Human-readable name.
    pub name: String,
    /// Rate in basis points (1 bp = 0.01%).
    pub rate_bps: i64,
    /// Whether configured prices include this tax.
    pub is_inclusive: bool,
    /// Whether this is the region's default rate.
    pub is_default: bool,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating a tax rate.
#[derive(Clone, Debug)]
pub struct CreateTaxRateInput {
    /// Region the rate applies to.
    pub tax_region_id: TaxRegionId,
    /// Optional category the rate applies to.
    pub tax_category_id: Option<TaxCategoryId>,
    /// Human-readable name.
    pub name: String,
    /// Rate in basis points.
    pub rate_bps: i64,
    /// Whether configured prices include this tax.
    pub is_inclusive: bool,
    /// Whether this is the region's default rate.
    pub is_default: bool,
    /// Extension metadata.
    pub metadata: Option<Metadata>,
}

/// Input for partially updating a tax rate.
#[derive(Clone, Debug, Default)]
pub struct UpdateTaxRateInput {
    /// Replacement category; `Some(None)` clears it.
    pub tax_category_id: Option<Option<TaxCategoryId>>,
    /// Replacement name.
    pub name: Option<String>,
    /// Replacement rate in basis points.
    pub rate_bps: Option<i64>,
    /// Replacement inclusive flag.
    pub is_inclusive: Option<bool>,
    /// Replacement default flag.
    pub is_default: Option<bool>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Split of one monetary amount into tax, net and gross components.
#[derive(
    utoipa::ToSchema,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct TaxBreakdown {
    /// Tax portion in minor units.
    pub tax_amount: i64,
    /// Net portion excluding tax.
    pub net_amount: i64,
    /// Gross portion including tax.
    pub gross_amount: i64,
}

/// A calculated tax line for one cart or order item.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaxLine {
    /// Destination country.
    pub country_code: CountryCode,
    /// Destination province, if any.
    pub province_code: Option<String>,
    /// Product tax category used for resolution.
    pub category_id: Option<TaxCategoryId>,
    /// Matched rate, if any.
    pub rate: Option<TaxRate>,
    /// Matched region, if any.
    pub region: Option<TaxRegion>,
    /// Original amount in minor units.
    pub amount_minor: i64,
    /// Applied rate in basis points.
    pub rate_bps: i64,
    /// Whether the original amount was tax-inclusive.
    pub is_inclusive: bool,
    /// Tax/net/gross split.
    pub breakdown: TaxBreakdown,
}

/// Validates a category or rate name.
pub fn validate_tax_name(name: &str, field: &str) -> Result<(), AppError> {
    validate_length(name, field, 200)
}

/// Validates a category code.
pub fn validate_category_code(code: &str) -> Result<(), AppError> {
    validate_length(code, "code", 80)
}

/// Validates a tax rate expressed in basis points.
pub fn validate_rate_bps(rate_bps: i64) -> Result<(), AppError> {
    if !(0..=MAX_RATE_BPS).contains(&rate_bps) {
        return Err(AppError::validation(format!(
            "rate_bps must be between 0 and {MAX_RATE_BPS}"
        )));
    }
    Ok(())
}

/// Normalizes a province code, trimming whitespace and upper-casing it.
#[must_use]
pub fn normalize_province(province: Option<&str>) -> Option<String> {
    province
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_uppercase)
}

/// Rounds a fraction to the nearest integer, half away from zero.
fn round_half_away(numerator: i128, denominator: i128) -> i128 {
    if denominator == 0 {
        return 0;
    }
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder == 0 {
        return quotient;
    }
    if remainder.abs() * 2 >= denominator.abs() {
        if numerator.signum() == denominator.signum() {
            quotient + 1
        } else {
            quotient - 1
        }
    } else {
        quotient
    }
}

fn clamp_to_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// Calculates a tax split using integer arithmetic only.
///
/// Exclusive amounts are treated as net and tax is added. Inclusive amounts are
/// treated as gross and tax is extracted. Rates are expressed in basis points.
#[must_use]
pub fn calculate_tax(amount_minor: i64, rate_bps: i64, inclusive: bool) -> TaxBreakdown {
    let rate_bps = rate_bps.clamp(0, MAX_RATE_BPS);
    let amount = i128::from(amount_minor);
    let rate = i128::from(rate_bps);
    if inclusive {
        let tax = round_half_away(amount * rate, 10_000 + rate);
        TaxBreakdown {
            tax_amount: clamp_to_i64(tax),
            net_amount: clamp_to_i64(amount - tax),
            gross_amount: amount_minor,
        }
    } else {
        let tax = round_half_away(amount * rate, 10_000);
        TaxBreakdown {
            tax_amount: clamp_to_i64(tax),
            net_amount: amount_minor,
            gross_amount: clamp_to_i64(amount + tax),
        }
    }
}
