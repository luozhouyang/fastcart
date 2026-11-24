//! Tax rate resolution and calculation.

use super::*;

impl TaxService {
    /// Resolves the effective rate and region for a destination and category.
    ///
    /// An exact country+province region is preferred over a country-only
    /// region. Within a region a category-specific rate wins over a default
    /// rate, which wins over an uncategorized rate. When `at` is provided, only
    /// rates created at or before that timestamp are considered.
    pub async fn resolve_rate(
        &self,
        country_code: &CountryCode,
        province_code: Option<&str>,
        category_id: Option<&TaxCategoryId>,
        at: Option<Timestamp>,
    ) -> Result<Option<(TaxRate, TaxRegion)>, AppError> {
        resolve_rate(&self.db, country_code, province_code, category_id, at).await
    }
}

impl TaxService {
    /// Finds the tax region that governs a destination, whether or not any of
    /// its rates apply.
    ///
    /// [`Self::resolve_rate`] answers "what rate applies here" and returns
    /// `None` both when the store does not tax the destination at all and when
    /// a region governs it but no rate matches the line. Those two are not the
    /// same situation: the first is a legitimate configuration, the second
    /// silently sells the line untaxed, so callers need to tell them apart.
    ///
    /// The ordering matches [`Self::resolve_rate`]: a province-specific region
    /// first, then the country-wide one.
    pub async fn region_for_destination(
        &self,
        country_code: &CountryCode,
        province_code: Option<&str>,
    ) -> Result<Option<TaxRegion>, AppError> {
        region_for_destination(&self.db, country_code, province_code).await
    }
}

impl TaxService {
    /// Calculates tax for one line using a store-configured tax region.
    ///
    /// Region-scoped pricing must use the region's tax region even when the
    /// destination country or province would resolve to a different tax region.
    ///
    /// The calculation runs through the active provider; the built-in provider
    /// reads the region from this service's database and reports a missing one
    /// as [`AppError::NotFound`].
    pub async fn calculate_for_line_in_region(
        &self,
        region_id: &str,
        category_id: Option<&TaxCategoryId>,
        amount_minor: i64,
        inclusive_override: Option<bool>,
    ) -> Result<TaxLine, AppError> {
        if amount_minor < 0 {
            return Err(AppError::validation("amount_minor must not be negative"));
        }
        self.providers
            .active()?
            .calculate(TaxCalculationRequest {
                destination: TaxDestination::Region { region_id },
                category_id,
                amount_minor,
                inclusive_override,
            })
            .await
    }
}

impl TaxService {
    /// Calculates tax for one line, optionally overriding inclusive pricing.
    ///
    /// The calculation runs through the active provider.
    pub async fn calculate_for_line(
        &self,
        country_code: &CountryCode,
        province_code: Option<&str>,
        category_id: Option<&TaxCategoryId>,
        amount_minor: i64,
        inclusive_override: Option<bool>,
    ) -> Result<TaxLine, AppError> {
        if amount_minor < 0 {
            return Err(AppError::validation("amount_minor must not be negative"));
        }
        self.providers
            .active()?
            .calculate(TaxCalculationRequest {
                destination: TaxDestination::Address {
                    country_code,
                    province_code,
                },
                category_id,
                amount_minor,
                inclusive_override,
            })
            .await
    }
}
