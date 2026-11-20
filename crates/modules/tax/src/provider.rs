//! Tax provider port: pluggable tax calculation backends and their registry.
//!
//! A [`TaxProvider`] calculates the tax for a single line. The built-in
//! [`RegionRateTaxProvider`] applies the tax regions and rates stored in the
//! store database; external engines such as Avalara or TaxJar can be registered
//! in a [`TaxProviderRegistry`] without the application service knowing their
//! concrete type.
//!
//! Providers are asked to price a line for one of the two destinations the
//! application service supports: a destination address, whose governing tax
//! region the provider resolves itself, or an explicit tax region chosen by
//! the caller (for example the region of a region-scoped price list).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{AppError, CountryCode, TaxCategoryId, TaxRegionId, Timestamp};

use crate::domain::{TaxBreakdown, TaxLine, TaxRate, TaxRegion, calculate_tax, normalize_province};
use crate::service::{RATE_COLUMNS, REGION_COLUMNS, row_to_rate, row_to_region};

/// Identifier of the built-in region/rate provider.
pub const REGION_TAX_PROVIDER_ID: &str = "region";

/// The destination a provider must price a line for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaxDestination<'a> {
    /// A destination address: the provider resolves the region that governs
    /// the country and optional province itself, mirroring
    /// [`crate::service::TaxService::calculate_for_line`].
    Address {
        /// Destination country.
        country_code: &'a CountryCode,
        /// Destination province, when known.
        province_code: Option<&'a str>,
    },
    /// An explicit tax region that governs the line regardless of the
    /// destination address, mirroring
    /// [`crate::service::TaxService::calculate_for_line_in_region`].
    ///
    /// Providers that cannot map a store tax region to one of their own
    /// jurisdictions should reject this destination instead of guessing.
    Region {
        /// Tax region identifier.
        region_id: &'a str,
    },
}

/// Provider-independent request to calculate the tax for one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxCalculationRequest<'a> {
    /// Destination the line is taxed for.
    pub destination: TaxDestination<'a>,
    /// Product tax category used for rate resolution.
    pub category_id: Option<&'a TaxCategoryId>,
    /// Taxable amount in minor units.
    pub amount_minor: i64,
    /// Overrides the matched rate's inclusive flag when set.
    pub inclusive_override: Option<bool>,
}

/// A tax calculation backend such as the built-in region rates or an external
/// tax engine.
///
/// Errors are [`AppError`] values because the application service returns them
/// to its callers unchanged; providers should use
/// [`AppError::ExternalService`] for engine-side failures.
#[async_trait]
pub trait TaxProvider: Send + Sync {
    /// Stable provider identifier used by the registry.
    fn id(&self) -> &str;

    /// Calculates the tax for one line.
    ///
    /// The returned [`TaxLine`] follows the same conventions as the
    /// application service: a line no rate applies to is reported as a
    /// zero-tax line rather than an error, and the province code is
    /// normalized.
    async fn calculate(&self, request: TaxCalculationRequest<'_>) -> Result<TaxLine, AppError>;
}

/// Built-in provider backed by the store's tax regions and rates.
///
/// Resolution order is the store configuration model:
///
/// - an exact country+province region is preferred over the country-wide
///   region;
/// - within a region a category-specific rate wins over the region default,
///   which wins over the uncategorized rate;
/// - the most recently created matching rate wins, using the identifier as a
///   stable tie-break for rates created within the same second.
#[derive(Clone, Debug)]
pub struct RegionRateTaxProvider {
    db: StoreDatabase,
}

impl RegionRateTaxProvider {
    /// Creates the built-in provider over a migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    async fn calculate_for_address(
        &self,
        country_code: &CountryCode,
        province_code: Option<&str>,
        category_id: Option<&TaxCategoryId>,
        amount_minor: i64,
        inclusive_override: Option<bool>,
    ) -> Result<TaxLine, AppError> {
        let resolved =
            resolve_rate(&self.db, country_code, province_code, category_id, None).await?;
        let (rate, region, rate_bps, is_inclusive, breakdown) = match resolved {
            Some((rate, region)) => {
                let is_inclusive = inclusive_override.unwrap_or(rate.is_inclusive);
                let breakdown = calculate_tax(amount_minor, rate.rate_bps, is_inclusive);
                (
                    Some(rate.clone()),
                    Some(region),
                    rate.rate_bps,
                    is_inclusive,
                    breakdown,
                )
            }
            None => {
                let breakdown = TaxBreakdown {
                    tax_amount: 0,
                    net_amount: amount_minor,
                    gross_amount: amount_minor,
                };
                (None, None, 0, false, breakdown)
            }
        };
        Ok(TaxLine {
            country_code: country_code.clone(),
            province_code: normalize_province(province_code),
            category_id: category_id.cloned(),
            rate,
            region,
            amount_minor,
            rate_bps,
            is_inclusive,
            breakdown,
        })
    }

    async fn calculate_in_region(
        &self,
        region_id: &str,
        category_id: Option<&TaxCategoryId>,
        amount_minor: i64,
        inclusive_override: Option<bool>,
    ) -> Result<TaxLine, AppError> {
        let region =
            fetch_region(&self.db, &TaxRegionId::from_string(region_id.to_owned())).await?;
        let rates = rates_for_region(&self.db, &region.id, None).await?;
        let rate = pick_rate(&rates, category_id);
        let (rate, rate_bps, is_inclusive, breakdown) = match rate {
            Some(rate) => {
                let is_inclusive = inclusive_override.unwrap_or(rate.is_inclusive);
                let breakdown = calculate_tax(amount_minor, rate.rate_bps, is_inclusive);
                (Some(rate.clone()), rate.rate_bps, is_inclusive, breakdown)
            }
            None => (
                None,
                0,
                false,
                TaxBreakdown {
                    tax_amount: 0,
                    net_amount: amount_minor,
                    gross_amount: amount_minor,
                },
            ),
        };
        Ok(TaxLine {
            country_code: region.country_code.clone(),
            province_code: region.province_code.clone(),
            category_id: category_id.cloned(),
            rate,
            region: Some(region),
            amount_minor,
            rate_bps,
            is_inclusive,
            breakdown,
        })
    }
}

#[async_trait]
impl TaxProvider for RegionRateTaxProvider {
    fn id(&self) -> &str {
        REGION_TAX_PROVIDER_ID
    }

    async fn calculate(&self, request: TaxCalculationRequest<'_>) -> Result<TaxLine, AppError> {
        if request.amount_minor < 0 {
            return Err(AppError::validation("amount_minor must not be negative"));
        }
        match request.destination {
            TaxDestination::Address {
                country_code,
                province_code,
            } => {
                self.calculate_for_address(
                    country_code,
                    province_code,
                    request.category_id,
                    request.amount_minor,
                    request.inclusive_override,
                )
                .await
            }
            TaxDestination::Region { region_id } => {
                self.calculate_in_region(
                    region_id,
                    request.category_id,
                    request.amount_minor,
                    request.inclusive_override,
                )
                .await
            }
        }
    }
}

/// Regions that govern a destination, most specific first.
pub(crate) async fn ordered_regions_for_destination(
    db: &StoreDatabase,
    country_code: &CountryCode,
    province_code: Option<&str>,
) -> Result<Vec<TaxRegion>, AppError> {
    let province_code = normalize_province(province_code);
    let rows = sqlx::query(&format!(
        "SELECT {REGION_COLUMNS} FROM tax_regions WHERE country_code = ?"
    ))
    .bind(country_code.as_str())
    .fetch_all(db.pool())
    .await
    .map_err(database_error)?;
    let regions = rows
        .iter()
        .map(row_to_region)
        .collect::<Result<Vec<_>, _>>()?;

    let mut ordered = Vec::new();
    if province_code.is_some()
        && let Some(region) = regions
            .iter()
            .find(|region| region.province_code == province_code)
    {
        ordered.push(region.clone());
    }
    if let Some(region) = regions.iter().find(|region| region.province_code.is_none()) {
        ordered.push(region.clone());
    }
    Ok(ordered)
}

/// Resolves the effective rate and region for a destination and category.
///
/// This is the resolution strategy of the built-in provider, shared with
/// [`crate::service::TaxService::resolve_rate`] so both answer identically.
/// When `at` is provided, only rates created at or before that timestamp are
/// considered.
pub(crate) async fn resolve_rate(
    db: &StoreDatabase,
    country_code: &CountryCode,
    province_code: Option<&str>,
    category_id: Option<&TaxCategoryId>,
    at: Option<Timestamp>,
) -> Result<Option<(TaxRate, TaxRegion)>, AppError> {
    for region in ordered_regions_for_destination(db, country_code, province_code).await? {
        let rates = rates_for_region(db, &region.id, at).await?;
        if let Some(rate) = pick_rate(&rates, category_id) {
            return Ok(Some((rate, region)));
        }
    }
    Ok(None)
}

/// Finds the tax region that governs a destination, whether or not any of its
/// rates apply.
///
/// [`resolve_rate`] answers "what rate applies here" and returns `None` both
/// when the store does not tax the destination at all and when a region
/// governs it but no rate matches the line. Those two are not the same
/// situation: the first is a legitimate configuration, the second silently
/// sells the line untaxed, so callers need to tell them apart.
pub(crate) async fn region_for_destination(
    db: &StoreDatabase,
    country_code: &CountryCode,
    province_code: Option<&str>,
) -> Result<Option<TaxRegion>, AppError> {
    Ok(
        ordered_regions_for_destination(db, country_code, province_code)
            .await?
            .into_iter()
            .next(),
    )
}

/// Loads the rates configured for a region, optionally only those created at
/// or before `at`.
pub(crate) async fn rates_for_region(
    db: &StoreDatabase,
    region_id: &TaxRegionId,
    at: Option<Timestamp>,
) -> Result<Vec<TaxRate>, AppError> {
    let rows = match at {
        Some(at) => {
            sqlx::query(&format!(
                "SELECT {RATE_COLUMNS} FROM tax_rates \
                 WHERE tax_region_id = ? AND created_at <= ? ORDER BY id"
            ))
            .bind(region_id.as_str())
            .bind(at)
            .fetch_all(db.pool())
            .await
        }
        None => {
            sqlx::query(&format!(
                "SELECT {RATE_COLUMNS} FROM tax_rates WHERE tax_region_id = ? ORDER BY id"
            ))
            .bind(region_id.as_str())
            .fetch_all(db.pool())
            .await
        }
    }
    .map_err(database_error)?;
    rows.iter().map(row_to_rate).collect()
}

/// Loads a tax region, reporting a missing one consistently.
pub(crate) async fn fetch_region(
    db: &StoreDatabase,
    id: &TaxRegionId,
) -> Result<TaxRegion, AppError> {
    let row = sqlx::query(&format!(
        "SELECT {REGION_COLUMNS} FROM tax_regions WHERE id = ?"
    ))
    .bind(id.as_str())
    .fetch_optional(db.pool())
    .await
    .map_err(database_error)?;
    match row {
        Some(row) => row_to_region(&row),
        None => Err(AppError::not_found("tax region", id)),
    }
}

/// Picks the rate that applies to a category.
///
/// A category-specific rate wins over the region default, which wins over the
/// uncategorized rate.
pub(crate) fn pick_rate(rates: &[TaxRate], category_id: Option<&TaxCategoryId>) -> Option<TaxRate> {
    if let Some(category_id) = category_id
        && let Some(rate) = newest_rate(
            rates
                .iter()
                .filter(|rate| rate.tax_category_id.as_ref() == Some(category_id)),
        )
    {
        return Some(rate.clone());
    }
    if let Some(rate) = newest_rate(rates.iter().filter(|rate| rate.is_default)) {
        return Some(rate.clone());
    }
    newest_rate(rates.iter().filter(|rate| rate.tax_category_id.is_none())).cloned()
}

/// Picks the most recently created rate, using the identifier as a stable
/// tie-break for rates created within the same second.
fn newest_rate<'a>(rates: impl Iterator<Item = &'a TaxRate>) -> Option<&'a TaxRate> {
    rates.max_by(|left, right| {
        (left.created_at, left.id.as_str()).cmp(&(right.created_at, right.id.as_str()))
    })
}

/// Thread-safe registry of available tax providers.
///
/// Exactly one provider is active at a time, and [`crate::service::TaxService`]
/// calculations run through it. [`Self::with_defaults`] registers and activates
/// the built-in region/rate provider; [`crate::service::TaxService::new`] uses
/// it, so the application service behavior is unchanged until another provider
/// is activated.
///
/// Store configuration selects a registered provider as well: the cart workflow
/// reads the store's `tax.provider_id` setting on every recalculation, serves
/// that pass from the provider the setting names, and leaves this registry's
/// active provider untouched. Callers that calculate outside that path pick a
/// provider at runtime with [`Self::set_active`].
#[derive(Clone, Default)]
pub struct TaxProviderRegistry {
    providers: Arc<RwLock<HashMap<String, Arc<dyn TaxProvider>>>>,
    active: Arc<RwLock<Option<String>>>,
}

impl TaxProviderRegistry {
    /// Creates an empty registry with no active provider.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a registry with the built-in region/rate provider registered
    /// and active over the given store database.
    #[must_use]
    pub fn with_defaults(db: StoreDatabase) -> Self {
        let registry = Self::new();
        registry
            .register(Arc::new(RegionRateTaxProvider::new(db)))
            .expect("the built-in tax provider is not present in a fresh registry");
        registry
            .set_active(REGION_TAX_PROVIDER_ID)
            .expect("the built-in tax provider was just registered");
        registry
    }

    /// Registers a provider, failing when its identifier is already present.
    pub fn register(&self, provider: Arc<dyn TaxProvider>) -> Result<(), AppError> {
        let id = provider.id().to_owned();
        if id.trim().is_empty() {
            return Err(AppError::validation("tax provider id must not be empty"));
        }
        let mut providers = self
            .providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if providers.contains_key(&id) {
            return Err(AppError::conflict(format!(
                "tax provider already registered: {id}"
            )));
        }
        providers.insert(id, provider);
        Ok(())
    }

    /// Inserts or replaces a provider, returning the previous implementation.
    ///
    /// The active provider is unchanged unless the replaced identifier is the
    /// active one, in which case the replacement serves subsequent
    /// calculations.
    pub fn replace(&self, provider: Arc<dyn TaxProvider>) -> Option<Arc<dyn TaxProvider>> {
        let id = provider.id().to_owned();
        self.providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, provider)
    }

    /// Looks up a provider by identifier.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<dyn TaxProvider>> {
        self.providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Returns all registered providers ordered by identifier.
    #[must_use]
    pub fn list(&self) -> Vec<Arc<dyn TaxProvider>> {
        let mut providers: Vec<_> = self
            .providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        providers.sort_by(|left, right| left.id().cmp(right.id()));
        providers
    }

    /// Makes a registered provider the active one.
    ///
    /// Fails with [`AppError::NotFound`] when the identifier is not
    /// registered.
    pub fn set_active(&self, id: &str) -> Result<(), AppError> {
        if self.get(id).is_none() {
            return Err(AppError::not_found("tax provider", id));
        }
        *self
            .active
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(id.to_owned());
        Ok(())
    }

    /// Identifier of the active provider, when one is selected.
    #[must_use]
    pub fn active_id(&self) -> Option<String> {
        self.active
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Returns the provider calculations run through.
    ///
    /// Fails with [`AppError::Internal`] when no provider has been activated.
    pub fn active(&self) -> Result<Arc<dyn TaxProvider>, AppError> {
        let Some(id) = self.active_id() else {
            return Err(AppError::internal(
                "no tax provider is registered: activate one on the tax provider registry",
            ));
        };
        self.get(&id)
            .ok_or_else(|| AppError::internal(format!("active tax provider is missing: {id}")))
    }
}

impl std::fmt::Debug for TaxProviderRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaxProviderRegistry")
            .field("active", &self.active_id())
            .field(
                "providers",
                &self
                    .list()
                    .iter()
                    .map(|provider| provider.id().to_owned())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}
