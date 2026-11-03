//! Shipping service tests.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CountryCode, CurrencyCode, Pagination, ShippingOptionId, ShippingProfileId, StoreId,
};

use crate::domain::{
    CreateFulfillmentSetInput, CreateShippingOptionInput, CreateShippingProfileInput,
    CreateShippingRateInput, UpdateFulfillmentSetInput, UpdateShippingOptionInput,
    UpdateShippingRateInput,
};
use crate::service::ShippingService;

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn country(code: &str) -> CountryCode {
    CountryCode::new(code).unwrap()
}

async fn setup() -> (StoreDatabase, ShippingService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ShippingService::new(db.clone());
    (db, service)
}

async fn profile(service: &ShippingService, name: &str) -> ShippingProfileId {
    service
        .create_profile(CreateShippingProfileInput {
            name: name.to_owned(),
            profile_type: None,
            metadata: None,
        })
        .await
        .unwrap()
        .id
}

async fn option(
    service: &ShippingService,
    profile_id: &ShippingProfileId,
    amount_minor: i64,
) -> ShippingOptionId {
    service
        .create_option(CreateShippingOptionInput {
            profile_id: profile_id.clone(),
            name: "Standard".to_owned(),
            amount_minor,
            currency: usd(),
            provider_id: None,
            provider_data: None,
            fulfillment_set_id: None,
            is_enabled: Some(true),
            metadata: None,
        })
        .await
        .unwrap()
        .id
}

async fn rate(
    service: &ShippingService,
    option_id: &ShippingOptionId,
    amount_minor: i64,
    country_code: Option<&str>,
    province_code: Option<&str>,
    min_subtotal: Option<i64>,
    max_subtotal: Option<i64>,
) -> String {
    service
        .create_rate(CreateShippingRateInput {
            shipping_option_id: option_id.clone(),
            country_code: country_code.map(country),
            province_code: province_code.map(str::to_owned),
            amount_minor,
            currency: usd(),
            min_subtotal,
            max_subtotal,
            metadata: None,
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn profile_crud_and_pagination() {
    let (_db, service) = setup().await;
    let first = profile(&service, "Default").await;
    let _second = profile(&service, "Digital").await;

    assert_eq!(
        service.get_profile(&first).await.unwrap().profile_type,
        "default"
    );
    let page = service
        .list_profiles(Pagination {
            limit: 1,
            offset: 0,
        })
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 1);

    let updated = service
        .update_profile(
            &first,
            crate::domain::UpdateShippingProfileInput {
                name: Some("General".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "General");

    service.delete_profile(&first).await.unwrap();
    assert!(matches!(
        service.get_profile(&first).await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        service.delete_profile(&first).await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn options_belong_to_profiles_and_validate() {
    let (_db, service) = setup().await;
    let profile_id = profile(&service, "Default").await;
    let option_id = option(&service, &profile_id, 500).await;
    let _other = option(&service, &profile_id, 900).await;

    let fetched = service.get_option(&option_id).await.unwrap();
    assert_eq!(fetched.amount_minor, 500);
    assert_eq!(fetched.price_type, "flat");
    assert!(fetched.is_enabled);

    let page = service
        .list_options_for_profile(
            &profile_id,
            Pagination {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.count, 2);

    let updated = service
        .update_option(
            &option_id,
            UpdateShippingOptionInput {
                amount_minor: Some(750),
                is_enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.amount_minor, 750);
    assert!(!updated.is_enabled);

    assert!(matches!(
        service
            .create_option(CreateShippingOptionInput {
                profile_id: ShippingProfileId::new(),
                name: "Missing".to_owned(),
                amount_minor: 100,
                currency: usd(),
                provider_id: None,
                provider_data: None,
                fulfillment_set_id: None,
                is_enabled: None,
                metadata: None,
            })
            .await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        service
            .create_option(CreateShippingOptionInput {
                profile_id: profile_id.clone(),
                name: "Negative".to_owned(),
                amount_minor: -1,
                currency: usd(),
                provider_id: None,
                provider_data: None,
                fulfillment_set_id: None,
                is_enabled: None,
                metadata: None,
            })
            .await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service
            .create_option(CreateShippingOptionInput {
                profile_id,
                name: "  ".to_owned(),
                amount_minor: 100,
                currency: usd(),
                provider_id: None,
                provider_data: None,
                fulfillment_set_id: None,
                is_enabled: None,
                metadata: None,
            })
            .await,
        Err(AppError::Validation(_))
    ));
}

#[tokio::test]
async fn rate_specificity_prefers_province_then_country_then_wildcard() {
    let (_db, service) = setup().await;
    let profile_id = profile(&service, "Default").await;
    let option_id = option(&service, &profile_id, 999).await;
    rate(&service, &option_id, 100, None, None, None, None).await;
    rate(&service, &option_id, 200, Some("US"), None, None, None).await;
    let province_rate = rate(
        &service,
        &option_id,
        300,
        Some("US"),
        Some("ca"),
        None,
        None,
    )
    .await;

    let canada = service
        .select_rate(&option_id, &country("US"), Some("CA"), 500)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(canada.amount_minor, 300);
    assert_eq!(canada.rate_id.as_deref(), Some(province_rate.as_str()));
    assert!(!canada.is_fallback);

    let new_york = service
        .select_rate(&option_id, &country("US"), Some("NY"), 500)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new_york.amount_minor, 200);

    let germany = service
        .select_rate(&option_id, &country("DE"), None, 500)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(germany.amount_minor, 100);
}

#[tokio::test]
async fn rate_subtotal_ranges_are_inclusive() {
    let (_db, service) = setup().await;
    let profile_id = profile(&service, "Default").await;
    let option_id = option(&service, &profile_id, 999).await;
    rate(&service, &option_id, 500, None, None, Some(0), Some(1000)).await;
    rate(
        &service,
        &option_id,
        800,
        Some("US"),
        None,
        Some(1000),
        None,
    )
    .await;

    let small = service
        .select_rate(&option_id, &country("US"), None, 500)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(small.amount_minor, 500);

    let boundary = service
        .select_rate(&option_id, &country("US"), None, 1000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(boundary.amount_minor, 800);

    let large = service
        .select_rate(&option_id, &country("US"), None, 2500)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(large.amount_minor, 800);

    let outside = service
        .select_rate(&option_id, &country("FR"), None, 2500)
        .await
        .unwrap();
    assert!(outside.is_none());

    let wildcard_option = option(&service, &profile_id, 700).await;
    rate(
        &service,
        &wildcard_option,
        600,
        None,
        None,
        Some(0),
        Some(1000),
    )
    .await;
    let unrestricted = service
        .select_rate(&wildcard_option, &country("FR"), None, 2500)
        .await
        .unwrap()
        .unwrap();
    assert!(unrestricted.is_fallback);
    assert_eq!(unrestricted.amount_minor, 700);
}

#[tokio::test]
async fn option_flat_amount_is_used_when_no_rate_matches() {
    let (_db, service) = setup().await;
    let profile_id = profile(&service, "Default").await;
    let option_id = option(&service, &profile_id, 450).await;

    let selected = service
        .select_rate(&option_id, &country("US"), None, 100)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.rate_id, None);
    assert!(selected.is_fallback);
    assert_eq!(selected.amount_minor, 450);
    assert_eq!(selected.currency, usd());

    assert!(
        service
            .select_rate(&ShippingOptionId::new(), &country("US"), None, 100)
            .await
            .unwrap()
            .is_none()
    );

    let restricted = option(&service, &profile_id, 800).await;
    rate(&service, &restricted, 850, Some("US"), None, None, None).await;
    assert!(
        service
            .select_rate(&restricted, &country("DE"), None, 100)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .select_rate(&restricted, &country("US"), None, 100)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn disabled_options_are_excluded() {
    let (_db, service) = setup().await;
    let profile_id = profile(&service, "Default").await;
    let enabled = option(&service, &profile_id, 500).await;
    let disabled = option(&service, &profile_id, 700).await;
    service
        .update_option(
            &disabled,
            UpdateShippingOptionInput {
                is_enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert!(
        service
            .select_rate(&disabled, &country("US"), None, 100)
            .await
            .unwrap()
            .is_none()
    );

    let resolved = service.list_options_for(&country("US"), 100).await.unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].option.id, enabled);
    assert_eq!(resolved[0].amount_minor, 500);
}

#[tokio::test]
async fn list_options_for_resolves_each_enabled_option() {
    let (_db, service) = setup().await;
    let profile_id = profile(&service, "Default").await;
    let first = option(&service, &profile_id, 500).await;
    let second = option(&service, &profile_id, 900).await;
    rate(&service, &second, 650, Some("US"), None, None, None).await;

    let resolved = service.list_options_for(&country("US"), 200).await.unwrap();
    assert_eq!(resolved.len(), 2);
    let by_id = |id: &ShippingOptionId| {
        resolved
            .iter()
            .find(|entry| &entry.option.id == id)
            .unwrap()
            .clone()
    };
    assert_eq!(by_id(&first).amount_minor, 500);
    assert_eq!(by_id(&second).amount_minor, 650);
    assert!(by_id(&second).rate_id.is_some());

    let other_country = service.list_options_for(&country("DE"), 200).await.unwrap();
    assert_eq!(other_country.len(), 1);
    assert_eq!(other_country[0].option.id, first);
    assert!(other_country[0].rate_id.is_none());
}

#[tokio::test]
async fn rate_crud_and_validation() {
    let (_db, service) = setup().await;
    let profile_id = profile(&service, "Default").await;
    let option_id = option(&service, &profile_id, 500).await;

    assert!(matches!(
        service
            .create_rate(CreateShippingRateInput {
                shipping_option_id: option_id.clone(),
                country_code: Some(country("US")),
                province_code: None,
                amount_minor: 100,
                currency: usd(),
                min_subtotal: Some(2000),
                max_subtotal: Some(1000),
                metadata: None,
            })
            .await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service
            .create_rate(CreateShippingRateInput {
                shipping_option_id: option_id.clone(),
                country_code: None,
                province_code: None,
                amount_minor: -1,
                currency: usd(),
                min_subtotal: None,
                max_subtotal: None,
                metadata: None,
            })
            .await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service
            .create_rate(CreateShippingRateInput {
                shipping_option_id: ShippingOptionId::new(),
                country_code: None,
                province_code: None,
                amount_minor: 100,
                currency: usd(),
                min_subtotal: None,
                max_subtotal: None,
                metadata: None,
            })
            .await,
        Err(AppError::NotFound(_))
    ));

    let rate_id = rate(
        &service,
        &option_id,
        400,
        Some("US"),
        Some("ca"),
        None,
        None,
    )
    .await;
    let fetched = service.get_rate(&rate_id).await.unwrap();
    assert_eq!(fetched.province_code.as_deref(), Some("CA"));
    assert_eq!(service.list_rates(&option_id).await.unwrap().len(), 1);

    let updated = service
        .update_rate(
            &rate_id,
            UpdateShippingRateInput {
                country_code: Some(None),
                amount_minor: Some(425),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(updated.country_code.is_none());
    assert_eq!(updated.amount_minor, 425);
    assert_eq!(updated.province_code.as_deref(), Some("CA"));

    service.delete_rate(&rate_id).await.unwrap();
    assert!(service.list_rates(&option_id).await.unwrap().is_empty());
    assert!(matches!(
        service.delete_rate(&rate_id).await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn option_locations_can_be_assigned_and_removed() {
    let (_db, service) = setup().await;
    let profile_id = profile(&service, "Default").await;
    let option_id = option(&service, &profile_id, 500).await;

    service
        .assign_location(&option_id, "loc_default")
        .await
        .unwrap();
    service
        .assign_location(&option_id, "loc_default")
        .await
        .unwrap();
    service
        .assign_location(&option_id, "loc_secondary")
        .await
        .unwrap();
    assert_eq!(
        service.list_locations(&option_id).await.unwrap(),
        vec!["loc_default".to_owned(), "loc_secondary".to_owned()]
    );

    service
        .remove_location(&option_id, "loc_default")
        .await
        .unwrap();
    assert_eq!(
        service.list_locations(&option_id).await.unwrap(),
        vec!["loc_secondary".to_owned()]
    );

    assert!(matches!(
        service
            .assign_location(&ShippingOptionId::new(), "loc_default")
            .await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn tied_rates_are_ordered_by_identifier() {
    let (_db, service) = setup().await;
    let profile_id = profile(&service, "Default").await;
    let option_id = option(&service, &profile_id, 999).await;
    let first = rate(&service, &option_id, 100, Some("US"), None, None, None).await;
    let second = rate(&service, &option_id, 200, Some("US"), None, None, None).await;
    let expected = first.min(second);

    let selected = service
        .select_rate(&option_id, &country("US"), None, 100)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.rate_id.as_deref(), Some(expected.as_str()));
}

#[tokio::test]
async fn fulfillment_sets_are_managed_and_linked_to_options() {
    let (_db, service) = setup().await;
    let sets = service
        .list_fulfillment_sets(Pagination {
            limit: 10,
            offset: 0,
        })
        .await
        .unwrap();
    assert!(sets.data.iter().any(|set| set.id == "fset_default"));

    let set = service
        .create_fulfillment_set(CreateFulfillmentSetInput {
            name: "Express".to_owned(),
            set_type: Some("shipping".to_owned()),
            metadata: None,
        })
        .await
        .unwrap();
    let profile_id = profile(&service, "Default").await;
    let option_id = service
        .create_option(CreateShippingOptionInput {
            profile_id: profile_id.clone(),
            name: "Express".to_owned(),
            amount_minor: 900,
            currency: usd(),
            provider_id: Some("manual".to_owned()),
            provider_data: Some(serde_json::json!({"zone": "eu"})),
            fulfillment_set_id: Some(set.id.clone()),
            is_enabled: Some(true),
            metadata: None,
        })
        .await
        .unwrap()
        .id;
    let option = service.get_option(&option_id).await.unwrap();
    assert_eq!(option.provider_id, "manual");
    assert_eq!(option.provider_data, serde_json::json!({"zone": "eu"}));
    assert_eq!(option.fulfillment_set_id.as_deref(), Some(set.id.as_str()));

    let updated = service
        .update_fulfillment_set(
            &set.id,
            UpdateFulfillmentSetInput {
                name: Some("Express shipping".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Express shipping");

    let cleared = service
        .update_option(
            &option_id,
            UpdateShippingOptionInput {
                fulfillment_set_id: Some(None),
                provider_data: Some(serde_json::json!({})),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(cleared.fulfillment_set_id.is_none());

    let missing_set = service
        .create_option(CreateShippingOptionInput {
            profile_id,
            name: "Missing set".to_owned(),
            amount_minor: 100,
            currency: usd(),
            provider_id: None,
            provider_data: None,
            fulfillment_set_id: Some("fset_missing".to_owned()),
            is_enabled: None,
            metadata: None,
        })
        .await;
    assert!(matches!(missing_set, Err(AppError::NotFound(_))));

    let invalid_data = service
        .create_option(CreateShippingOptionInput {
            profile_id: ShippingProfileId::new(),
            name: "Bad data".to_owned(),
            amount_minor: 100,
            currency: usd(),
            provider_id: None,
            provider_data: Some(serde_json::json!([])),
            fulfillment_set_id: None,
            is_enabled: None,
            metadata: None,
        })
        .await;
    assert!(matches!(invalid_data, Err(AppError::Validation(_))));

    service.delete_fulfillment_set(&set.id).await.unwrap();
    assert!(matches!(
        service.get_fulfillment_set(&set.id).await,
        Err(AppError::NotFound(_))
    ));
}
