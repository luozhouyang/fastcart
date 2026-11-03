//! Tax service and calculator tests.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CountryCode, Pagination, StoreId, TaxCategoryId, TaxRegionId, now_timestamp,
};

use crate::domain::{
    CreateTaxCategoryInput, CreateTaxRateInput, CreateTaxRegionInput, TaxBreakdown,
    UpdateTaxCategoryInput, UpdateTaxRateInput, calculate_tax,
};
use crate::service::TaxService;

fn country(code: &str) -> CountryCode {
    CountryCode::new(code).unwrap()
}

async fn setup() -> (StoreDatabase, TaxService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = TaxService::new(db.clone());
    (db, service)
}

async fn category(service: &TaxService, code: &str) -> TaxCategoryId {
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

async fn region(service: &TaxService, code: &str, province: Option<&str>) -> TaxRegionId {
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

async fn rate(
    service: &TaxService,
    region_id: &TaxRegionId,
    category_id: Option<&TaxCategoryId>,
    rate_bps: i64,
    is_inclusive: bool,
    is_default: bool,
) -> crate::domain::TaxRate {
    service
        .create_rate(CreateTaxRateInput {
            tax_region_id: region_id.clone(),
            tax_category_id: category_id.cloned(),
            name: "Rate".to_owned(),
            rate_bps,
            is_inclusive,
            is_default,
            metadata: None,
        })
        .await
        .unwrap()
}

#[test]
fn exclusive_tax_rounds_half_away_from_zero() {
    assert_eq!(
        calculate_tax(1005, 500, false),
        TaxBreakdown {
            tax_amount: 50,
            net_amount: 1005,
            gross_amount: 1055,
        }
    );
    assert_eq!(calculate_tax(1015, 500, false).tax_amount, 51);
    assert_eq!(calculate_tax(999, 1500, false).tax_amount, 150);
    assert_eq!(
        calculate_tax(0, 2000, false),
        TaxBreakdown {
            tax_amount: 0,
            net_amount: 0,
            gross_amount: 0,
        }
    );
    assert_eq!(calculate_tax(1234, 0, false).tax_amount, 0);
}

#[test]
fn inclusive_tax_extracts_the_tax_portion() {
    assert_eq!(
        calculate_tax(1000, 2000, true),
        TaxBreakdown {
            tax_amount: 167,
            net_amount: 833,
            gross_amount: 1000,
        }
    );
    assert_eq!(
        calculate_tax(1005, 500, true),
        TaxBreakdown {
            tax_amount: 48,
            net_amount: 957,
            gross_amount: 1005,
        }
    );
    assert_eq!(
        calculate_tax(5000, 0, true),
        TaxBreakdown {
            tax_amount: 0,
            net_amount: 5000,
            gross_amount: 5000,
        }
    );
}

#[tokio::test]
async fn category_crud_unique_codes_and_pagination() {
    let (_db, service) = setup().await;
    let standard = category(&service, "standard").await;
    let reduced = category(&service, "reduced").await;

    assert_eq!(
        service.get_category(&standard).await.unwrap().code,
        "standard"
    );
    assert!(matches!(
        service
            .create_category(CreateTaxCategoryInput {
                name: "Duplicate".to_owned(),
                code: "standard".to_owned(),
                description: None,
            })
            .await,
        Err(AppError::Conflict(_))
    ));

    let page = service
        .list_categories(Pagination {
            limit: 1,
            offset: 0,
        })
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 1);

    let updated = service
        .update_category(
            &standard,
            UpdateTaxCategoryInput {
                name: Some("Standard rate".to_owned()),
                description: Some(Some("Most goods".to_owned())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Standard rate");
    assert_eq!(updated.description.as_deref(), Some("Most goods"));

    assert!(matches!(
        service
            .update_category(
                &reduced,
                UpdateTaxCategoryInput {
                    code: Some("standard".to_owned()),
                    ..Default::default()
                },
            )
            .await,
        Err(AppError::Conflict(_))
    ));

    service.delete_category(&reduced).await.unwrap();
    assert!(matches!(
        service.get_category(&reduced).await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn regions_are_unique_by_country_and_province() {
    let (_db, service) = setup().await;
    let _us = region(&service, "US", None).await;
    let ca = region(&service, "US", Some("ca")).await;
    assert_eq!(
        service
            .get_region(&ca)
            .await
            .unwrap()
            .province_code
            .as_deref(),
        Some("CA")
    );

    assert!(matches!(
        service
            .create_region(CreateTaxRegionInput {
                name: "Duplicate country".to_owned(),
                country_code: country("US"),
                province_code: None,
                metadata: None,
            })
            .await,
        Err(AppError::Conflict(_))
    ));
    assert!(matches!(
        service
            .create_region(CreateTaxRegionInput {
                name: "Duplicate province".to_owned(),
                country_code: country("US"),
                province_code: Some("CA".to_owned()),
                metadata: None,
            })
            .await,
        Err(AppError::Conflict(_))
    ));

    let ny = region(&service, "US", Some("NY")).await;
    assert!(matches!(
        service
            .update_region(
                &ny,
                crate::domain::UpdateTaxRegionInput {
                    province_code: Some(Some("CA".to_owned())),
                    ..Default::default()
                },
            )
            .await,
        Err(AppError::Conflict(_))
    ));

    let page = service
        .list_regions(Pagination {
            limit: 10,
            offset: 0,
        })
        .await
        .unwrap();
    assert_eq!(page.count, 3);
    assert!(matches!(
        service
            .create_region(CreateTaxRegionInput {
                name: "  ".to_owned(),
                country_code: country("CA"),
                province_code: None,
                metadata: None,
            })
            .await,
        Err(AppError::Validation(_))
    ));
}

#[tokio::test]
async fn rate_crud_and_validation() {
    let (_db, service) = setup().await;
    let region_id = region(&service, "US", None).await;
    let standard = category(&service, "standard").await;

    assert!(matches!(
        service
            .create_rate(CreateTaxRateInput {
                tax_region_id: region_id.clone(),
                tax_category_id: None,
                name: "Negative".to_owned(),
                rate_bps: -1,
                is_inclusive: false,
                is_default: false,
                metadata: None,
            })
            .await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service
            .create_rate(CreateTaxRateInput {
                tax_region_id: region_id.clone(),
                tax_category_id: None,
                name: "Too high".to_owned(),
                rate_bps: 100_001,
                is_inclusive: false,
                is_default: false,
                metadata: None,
            })
            .await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service
            .create_rate(CreateTaxRateInput {
                tax_region_id: TaxRegionId::new(),
                tax_category_id: None,
                name: "Missing region".to_owned(),
                rate_bps: 100,
                is_inclusive: false,
                is_default: false,
                metadata: None,
            })
            .await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        service
            .create_rate(CreateTaxRateInput {
                tax_region_id: region_id.clone(),
                tax_category_id: Some(TaxCategoryId::new()),
                name: "Missing category".to_owned(),
                rate_bps: 100,
                is_inclusive: false,
                is_default: false,
                metadata: None,
            })
            .await,
        Err(AppError::NotFound(_))
    ));

    let created = rate(&service, &region_id, Some(&standard), 2000, false, false).await;
    assert_eq!(created.rate_bps, 2000);
    assert_eq!(service.list_rates(&region_id).await.unwrap().len(), 1);

    let updated = service
        .update_rate(
            &created.id,
            UpdateTaxRateInput {
                rate_bps: Some(1500),
                is_inclusive: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.rate_bps, 1500);
    assert!(updated.is_inclusive);

    service.delete_rate(&created.id).await.unwrap();
    assert!(service.list_rates(&region_id).await.unwrap().is_empty());
    assert!(matches!(
        service.delete_rate(&created.id).await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn exact_region_is_preferred_over_country_region() {
    let (_db, service) = setup().await;
    let country_region = region(&service, "US", None).await;
    let province_region = region(&service, "US", Some("CA")).await;
    rate(&service, &country_region, None, 1000, false, true).await;
    rate(&service, &province_region, None, 2500, false, true).await;

    let (resolved, resolved_region) = service
        .resolve_rate(&country("US"), Some("ca"), None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.rate_bps, 2500);
    assert_eq!(resolved_region.id, province_region);

    let (country_rate, _) = service
        .resolve_rate(&country("US"), Some("NY"), None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(country_rate.rate_bps, 1000);

    assert!(
        service
            .resolve_rate(&country("DE"), None, None, None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn category_specific_rates_beat_default_and_uncategorized_rates() {
    let (_db, service) = setup().await;
    let region_id = region(&service, "US", None).await;
    let standard = category(&service, "standard").await;
    let reduced = category(&service, "reduced").await;
    rate(&service, &region_id, None, 1000, false, true).await;
    rate(&service, &region_id, Some(&standard), 800, false, false).await;
    rate(&service, &region_id, Some(&reduced), 400, false, false).await;

    let (standard_rate, _) = service
        .resolve_rate(&country("US"), None, Some(&standard), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(standard_rate.rate_bps, 800);

    let (default_rate, _) = service
        .resolve_rate(&country("US"), None, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(default_rate.rate_bps, 1000);

    let uncategorized = category(&service, "uncategorized").await;
    let (fallback, _) = service
        .resolve_rate(&country("US"), None, Some(&uncategorized), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fallback.rate_bps, 1000);
}

#[tokio::test]
async fn exact_region_wins_even_when_country_region_has_category_rate() {
    let (_db, service) = setup().await;
    let country_region = region(&service, "US", None).await;
    let province_region = region(&service, "US", Some("CA")).await;
    let standard = category(&service, "standard").await;
    rate(
        &service,
        &country_region,
        Some(&standard),
        500,
        false,
        false,
    )
    .await;
    rate(&service, &province_region, None, 900, false, true).await;

    let (resolved, resolved_region) = service
        .resolve_rate(&country("US"), Some("CA"), Some(&standard), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.rate_bps, 900);
    assert_eq!(resolved_region.id, province_region);
}

#[tokio::test]
async fn uncategorized_rate_is_used_when_no_default_exists() {
    let (_db, service) = setup().await;
    let region_id = region(&service, "GB", None).await;
    rate(&service, &region_id, None, 2000, false, false).await;

    let (resolved, _) = service
        .resolve_rate(&country("GB"), None, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.rate_bps, 2000);
}

#[tokio::test]
async fn calculate_for_line_splits_exclusive_and_inclusive_amounts() {
    let (_db, service) = setup().await;
    let region_id = region(&service, "US", None).await;
    let standard = category(&service, "standard").await;
    rate(&service, &region_id, Some(&standard), 2000, false, false).await;

    let exclusive = service
        .calculate_for_line(&country("us"), None, Some(&standard), 10_000, None)
        .await
        .unwrap();
    assert_eq!(exclusive.rate_bps, 2000);
    assert!(!exclusive.is_inclusive);
    assert_eq!(
        exclusive.breakdown,
        TaxBreakdown {
            tax_amount: 2000,
            net_amount: 10_000,
            gross_amount: 12_000,
        }
    );
    assert_eq!(exclusive.country_code, country("US"));

    let inclusive = service
        .calculate_for_line(&country("US"), None, Some(&standard), 10_000, Some(true))
        .await
        .unwrap();
    assert!(inclusive.is_inclusive);
    assert_eq!(
        inclusive.breakdown,
        TaxBreakdown {
            tax_amount: 1667,
            net_amount: 8333,
            gross_amount: 10_000,
        }
    );
}

#[tokio::test]
async fn calculate_for_line_returns_zero_when_no_rate_matches() {
    let (_db, service) = setup().await;
    let line = service
        .calculate_for_line(&country("JP"), Some("13"), None, 4321, None)
        .await
        .unwrap();
    assert!(line.rate.is_none());
    assert!(line.region.is_none());
    assert_eq!(line.rate_bps, 0);
    assert!(!line.is_inclusive);
    assert_eq!(
        line.breakdown,
        TaxBreakdown {
            tax_amount: 0,
            net_amount: 4321,
            gross_amount: 4321,
        }
    );
    assert_eq!(line.province_code.as_deref(), Some("13"));
}

#[tokio::test]
async fn resolve_rate_honours_the_effective_timestamp() {
    let (_db, service) = setup().await;
    let region_id = region(&service, "US", None).await;
    rate(&service, &region_id, None, 1000, false, true).await;
    let now = now_timestamp();

    assert!(
        service
            .resolve_rate(&country("US"), None, None, Some(now - 1))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .resolve_rate(&country("US"), None, None, Some(now + 1))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn deleting_a_region_removes_its_rates() {
    let (_db, service) = setup().await;
    let region_id = region(&service, "US", None).await;
    let created = rate(&service, &region_id, None, 1000, false, true).await;

    service.delete_region(&region_id).await.unwrap();
    assert!(matches!(
        service.get_rate(&created.id).await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        service.list_rates(&region_id).await,
        Err(AppError::NotFound(_))
    ));
}
