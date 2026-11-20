//! Tax provider port tests.
//!
//! The built-in provider must reproduce the application service behavior
//! exactly, and a registered provider must really serve the calculations
//! instead of the built-in region/rate logic.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CountryCode, Metadata, StoreId, TaxCategoryId, TaxRateId, TaxRegionId,
};
use fastcart_module_tax::{
    CreateTaxCategoryInput, CreateTaxRateInput, CreateTaxRegionInput, REGION_TAX_PROVIDER_ID,
    RegionRateTaxProvider, TaxBreakdown, TaxCalculationRequest, TaxDestination, TaxLine,
    TaxProvider, TaxProviderRegistry, TaxService, calculate_tax, normalize_province,
};

fn country(code: &str) -> CountryCode {
    CountryCode::new(code).unwrap()
}

async fn setup() -> (StoreDatabase, TaxService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = TaxService::new(db.clone());
    (db, service)
}

async fn create_category(service: &TaxService, code: &str) -> TaxCategoryId {
    service
        .create_category(CreateTaxCategoryInput {
            name: format!("Category {code}"),
            code: code.to_owned(),
            description: None,
        })
        .await
        .unwrap()
        .id
}

async fn create_region(service: &TaxService, code: &str, province: Option<&str>) -> TaxRegionId {
    service
        .create_region(CreateTaxRegionInput {
            name: format!("Region {code}"),
            country_code: country(code),
            province_code: province.map(str::to_owned),
            metadata: None,
        })
        .await
        .unwrap()
        .id
}

async fn create_rate(
    service: &TaxService,
    region_id: &TaxRegionId,
    category_id: Option<&TaxCategoryId>,
    rate_bps: i64,
) -> fastcart_module_tax::TaxRate {
    service
        .create_rate(CreateTaxRateInput {
            tax_region_id: region_id.clone(),
            tax_category_id: category_id.cloned(),
            name: "Rate".to_owned(),
            rate_bps,
            is_inclusive: false,
            is_default: true,
            metadata: None,
        })
        .await
        .unwrap()
}

/// Test double standing in for an external engine.
///
/// It always answers with a recognizable 5% exclusive rate and records the
/// destinations it was asked about, so tests can prove the request reached it.
struct StubProvider {
    id: String,
    jurisdiction: CountryCode,
    destinations: Mutex<Vec<String>>,
}

impl StubProvider {
    fn new(id: &str, jurisdiction: CountryCode) -> Self {
        Self {
            id: id.to_owned(),
            jurisdiction,
            destinations: Mutex::new(Vec::new()),
        }
    }

    fn destinations(&self) -> Vec<String> {
        self.destinations.lock().unwrap().clone()
    }

    fn record(&self, destination: &TaxDestination<'_>) -> (CountryCode, Option<String>) {
        let mut recorded = self.destinations.lock().unwrap();
        match destination {
            TaxDestination::Address {
                country_code,
                province_code,
            } => {
                recorded.push(format!("address {country_code} {province_code:?}"));
                (
                    self.jurisdiction.clone(),
                    normalize_province(*province_code),
                )
            }
            TaxDestination::Region { region_id } => {
                recorded.push(format!("region {region_id}"));
                (self.jurisdiction.clone(), None)
            }
        }
    }
}

#[async_trait]
impl TaxProvider for StubProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn calculate(&self, request: TaxCalculationRequest<'_>) -> Result<TaxLine, AppError> {
        let (country_code, province_code) = self.record(&request.destination);
        let rate = fastcart_module_tax::TaxRate {
            id: TaxRateId::from_string("rate_stub".to_owned()),
            tax_region_id: TaxRegionId::from_string("region_stub".to_owned()),
            tax_category_id: request.category_id.cloned(),
            name: "Stub provider rate".to_owned(),
            rate_bps: 500,
            is_inclusive: false,
            is_default: true,
            metadata: Metadata::empty(),
            created_at: 1,
            updated_at: 1,
        };
        let is_inclusive = request.inclusive_override.unwrap_or(rate.is_inclusive);
        Ok(TaxLine {
            country_code,
            province_code,
            category_id: request.category_id.cloned(),
            rate: Some(rate),
            region: None,
            amount_minor: request.amount_minor,
            rate_bps: 500,
            is_inclusive,
            breakdown: calculate_tax(request.amount_minor, 500, is_inclusive),
        })
    }
}

/// A provider whose backend is unreachable.
struct FailingProvider;

#[async_trait]
impl TaxProvider for FailingProvider {
    fn id(&self) -> &str {
        "failing"
    }

    async fn calculate(&self, _request: TaxCalculationRequest<'_>) -> Result<TaxLine, AppError> {
        Err(AppError::ExternalService(
            "avalara is unreachable".to_owned(),
        ))
    }
}

#[tokio::test]
async fn built_in_provider_reproduces_the_service_calculations() {
    let (db, service) = setup().await;
    let region_id = create_region(&service, "US", None).await;
    let standard = create_category(&service, "standard").await;
    create_rate(&service, &region_id, Some(&standard), 2000).await;

    let provider = RegionRateTaxProvider::new(db.clone());
    assert_eq!(provider.id(), REGION_TAX_PROVIDER_ID);

    let by_address = provider
        .calculate(TaxCalculationRequest {
            destination: TaxDestination::Address {
                country_code: &country("US"),
                province_code: None,
            },
            category_id: Some(&standard),
            amount_minor: 10_000,
            inclusive_override: None,
        })
        .await
        .unwrap();
    let service_line = service
        .calculate_for_line(&country("US"), None, Some(&standard), 10_000, None)
        .await
        .unwrap();
    assert_eq!(by_address, service_line);

    let by_region = provider
        .calculate(TaxCalculationRequest {
            destination: TaxDestination::Region {
                region_id: region_id.as_str(),
            },
            category_id: Some(&standard),
            amount_minor: 10_000,
            inclusive_override: Some(true),
        })
        .await
        .unwrap();
    let service_region_line = service
        .calculate_for_line_in_region(region_id.as_str(), Some(&standard), 10_000, Some(true))
        .await
        .unwrap();
    assert_eq!(by_region, service_region_line);

    // The same provider is what a default registry activates.
    let registry = TaxProviderRegistry::with_defaults(db.clone());
    assert_eq!(
        registry.active_id().as_deref(),
        Some(REGION_TAX_PROVIDER_ID)
    );
    let registered = registry.get(REGION_TAX_PROVIDER_ID).unwrap();
    let registered_line = registered
        .calculate(TaxCalculationRequest {
            destination: TaxDestination::Address {
                country_code: &country("US"),
                province_code: None,
            },
            category_id: Some(&standard),
            amount_minor: 10_000,
            inclusive_override: None,
        })
        .await
        .unwrap();
    assert_eq!(registered_line, service_line);
}

#[tokio::test]
async fn a_registered_provider_serves_the_calculations() {
    let (db, service) = setup().await;
    let region_id = create_region(&service, "US", None).await;
    let standard = create_category(&service, "standard").await;
    create_rate(&service, &region_id, Some(&standard), 2000).await;

    let registry = TaxProviderRegistry::new();
    let stub = Arc::new(StubProvider::new("stub", country("US")));
    registry.register(stub.clone()).unwrap();
    registry.set_active("stub").unwrap();
    let service = TaxService::with_registry(db.clone(), registry);

    // The configured 20% rate is ignored: the result comes from the stub.
    let exclusive = service
        .calculate_for_line(&country("US"), None, Some(&standard), 10_000, None)
        .await
        .unwrap();
    assert_eq!(
        exclusive.rate.as_ref().unwrap().name.as_str(),
        "Stub provider rate"
    );
    assert_eq!(exclusive.rate_bps, 500);
    assert!(!exclusive.is_inclusive);
    assert_eq!(
        exclusive.breakdown,
        TaxBreakdown {
            tax_amount: 500,
            net_amount: 10_000,
            gross_amount: 10_500,
        }
    );

    // The inclusive override reaches the provider untouched.
    let inclusive = service
        .calculate_for_line(&country("US"), None, Some(&standard), 10_000, Some(true))
        .await
        .unwrap();
    assert!(inclusive.is_inclusive);
    assert_eq!(
        inclusive.breakdown,
        TaxBreakdown {
            tax_amount: 476,
            net_amount: 9524,
            gross_amount: 10_000,
        }
    );

    // The explicit-region call shape reaches the provider too.
    let in_region = service
        .calculate_for_line_in_region(region_id.as_str(), Some(&standard), 10_000, None)
        .await
        .unwrap();
    assert_eq!(
        in_region.rate.as_ref().unwrap().name.as_str(),
        "Stub provider rate"
    );

    assert_eq!(
        stub.destinations(),
        vec![
            "address US None".to_owned(),
            "address US None".to_owned(),
            format!("region {}", region_id.as_str()),
        ]
    );
}

#[tokio::test]
async fn replacing_the_default_provider_switches_the_strategy() {
    let (db, service) = setup().await;
    let region_id = create_region(&service, "US", None).await;
    create_rate(&service, &region_id, None, 2000).await;

    let registry = TaxProviderRegistry::with_defaults(db.clone());
    let stub: Arc<dyn TaxProvider> =
        Arc::new(StubProvider::new(REGION_TAX_PROVIDER_ID, country("US")));
    assert!(registry.replace(stub).is_some());
    assert_eq!(registry.list().len(), 1);

    let service = TaxService::with_registry(db.clone(), registry);
    let line = service
        .calculate_for_line(&country("US"), None, None, 10_000, None)
        .await
        .unwrap();
    assert_eq!(line.rate_bps, 500);
    assert_eq!(line.rate.unwrap().name.as_str(), "Stub provider rate");
}

#[tokio::test]
async fn unregistered_lookups_and_empty_registries_fail_clearly() {
    let (db, _service) = setup().await;
    let registry = TaxProviderRegistry::new();
    assert!(registry.list().is_empty());
    assert!(registry.active_id().is_none());
    assert!(registry.get("missing").is_none());
    assert!(matches!(registry.active(), Err(AppError::Internal(_))));
    assert!(matches!(
        registry.set_active("missing"),
        Err(AppError::NotFound(_))
    ));

    let stub = Arc::new(StubProvider::new("stub", country("US")));
    registry.register(stub.clone()).unwrap();
    assert!(matches!(
        registry.register(stub),
        Err(AppError::Conflict(_))
    ));
    assert!(matches!(
        registry.register(Arc::new(StubProvider::new("", country("US")))),
        Err(AppError::Validation(_))
    ));
    registry.set_active("stub").unwrap();
    assert_eq!(registry.active_id().as_deref(), Some("stub"));
    assert_eq!(registry.list().len(), 1);

    // A service built over an empty registry has no fallback: it reports the
    // missing provider instead of silently reverting to region/rate logic.
    // `TaxService::new` always installs the built-in provider, so the public
    // `new` behavior is unchanged.
    let empty_service = TaxService::with_registry(db.clone(), TaxProviderRegistry::new());
    let error = empty_service
        .calculate_for_line(&country("US"), None, None, 100, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Internal(_)));
    let error = empty_service
        .calculate_for_line_in_region("region_1", None, 100, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Internal(_)));
    assert_eq!(
        TaxService::new(db).providers().active_id().as_deref(),
        Some(REGION_TAX_PROVIDER_ID)
    );
}

#[tokio::test]
async fn provider_failures_surface_unchanged() {
    let (db, _service) = setup().await;
    let registry = TaxProviderRegistry::new();
    registry.register(Arc::new(FailingProvider)).unwrap();
    registry.set_active("failing").unwrap();
    let service = TaxService::with_registry(db, registry);

    let error = service
        .calculate_for_line(&country("US"), None, None, 100, None)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        AppError::ExternalService("avalara is unreachable".to_owned())
    );
}

#[tokio::test]
async fn negative_amounts_are_rejected_before_the_provider_runs() {
    let (db, _service) = setup().await;
    let registry = TaxProviderRegistry::new();
    let stub = Arc::new(StubProvider::new("stub", country("US")));
    registry.register(stub.clone()).unwrap();
    registry.set_active("stub").unwrap();
    let service = TaxService::with_registry(db, registry);

    let error = service
        .calculate_for_line(&country("US"), None, None, -1, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    let error = service
        .calculate_for_line_in_region("region_1", None, -1, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    assert!(stub.destinations().is_empty());
}

#[test]
fn registry_lists_providers_by_identifier() {
    let registry = TaxProviderRegistry::new();
    registry
        .register(Arc::new(StubProvider::new("zeta", country("US"))))
        .unwrap();
    registry
        .register(Arc::new(StubProvider::new("alpha", country("US"))))
        .unwrap();
    let ids: Vec<String> = registry.list().iter().map(|p| p.id().to_owned()).collect();
    assert_eq!(ids, vec!["alpha".to_owned(), "zeta".to_owned()]);
    assert!(registry.get("alpha").is_some());
}
