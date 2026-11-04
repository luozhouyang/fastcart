//! Store-module tests.

use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
use fastcart_kernel::{Metadata, Pagination, StoreId};

use crate::{
    CreatePublishableApiKeyInput, CreateRegionInput, CreateSalesChannelInput,
    MAX_SETTING_KEY_LENGTH, MAX_SETTING_VALUE_BYTES, RegionService, StoreService,
    UpdateRegionInput, UpdateSalesChannelInput, is_secret_setting_key,
};

async fn service() -> StoreService {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("in-memory store database");
    StoreService::new(db)
}

fn channel_input(name: &str) -> CreateSalesChannelInput {
    CreateSalesChannelInput {
        name: name.to_owned(),
        description: None,
        metadata: Metadata::empty(),
    }
}

#[tokio::test]
async fn setting_round_trips() {
    let service = service().await;
    assert!(service.get_setting("store.name").await.unwrap().is_none());

    service.set_setting("store.name", "Fastcart").await.unwrap();
    assert_eq!(
        service.get_setting("store.name").await.unwrap(),
        Some("Fastcart".to_owned())
    );

    let all = service.get_all_settings().await.unwrap();
    assert_eq!(all.get("store.name").map(String::as_str), Some("Fastcart"));
}

#[tokio::test]
async fn settings_are_overwritten_not_duplicated() {
    let service = service().await;
    service.set_setting("locale", "en-US").await.unwrap();
    service.set_setting("locale", "fr-FR").await.unwrap();

    let all = service.get_all_settings().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all.get("locale").map(String::as_str), Some("fr-FR"));
}

#[tokio::test]
async fn setting_keys_are_trimmed_and_sorted() {
    let service = service().await;
    service.set_setting("  b  ", "2").await.unwrap();
    service.set_setting("a", "1").await.unwrap();

    let all = service.get_all_settings().await.unwrap();
    assert_eq!(
        all.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[tokio::test]
async fn empty_setting_keys_are_rejected() {
    let service = service().await;
    let error = service.set_setting("   ", "value").await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
}

#[tokio::test]
async fn oversized_setting_keys_are_rejected() {
    let service = service().await;
    let key = "k".repeat(MAX_SETTING_KEY_LENGTH + 1);
    let error = service.set_setting(&key, "value").await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
}

#[tokio::test]
async fn oversized_setting_values_are_rejected() {
    let service = service().await;
    let value = "v".repeat(MAX_SETTING_VALUE_BYTES + 1);
    let error = service.set_setting("key", &value).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
}

#[tokio::test]
async fn deletes_existing_and_missing_settings() {
    let service = service().await;
    service.set_setting("feature", "on").await.unwrap();

    assert!(service.delete_setting("feature").await.unwrap());
    assert!(!service.delete_setting("feature").await.unwrap());
    assert!(service.get_setting("feature").await.unwrap().is_none());
}

#[tokio::test]
async fn public_settings_omit_secrets_while_admin_reads_see_them() {
    let service = service().await;
    service.set_setting("store.name", "Fastcart").await.unwrap();
    service
        .set_setting("jwt_private_key", "super-secret")
        .await
        .unwrap();
    service
        .set_setting("stripe_secret_key", "sk_test")
        .await
        .unwrap();

    let public = service.public_settings().await.unwrap();
    assert_eq!(
        public.get("store.name").map(String::as_str),
        Some("Fastcart")
    );
    assert!(!public.contains_key("jwt_private_key"));
    assert!(!public.contains_key("stripe_secret_key"));

    let admin = service.get_all_settings().await.unwrap();
    assert_eq!(
        admin.get("jwt_private_key").map(String::as_str),
        Some("super-secret")
    );
    assert_eq!(
        service
            .get_setting("jwt_private_key")
            .await
            .unwrap()
            .as_deref(),
        Some("super-secret")
    );
    assert!(is_secret_setting_key("jwt_private_key"));
    assert!(is_secret_setting_key("PASSWORD_RESET_TTL"));
    assert!(!is_secret_setting_key("store.name"));
    assert!(!is_secret_setting_key("currency_code"));
}

#[tokio::test]
async fn multi_key_settings_updates_are_atomic() {
    let service = service().await;
    let error = service
        .set_settings(&[
            ("a".to_owned(), "1".to_owned()),
            ("   ".to_owned(), "2".to_owned()),
        ])
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
    assert!(service.get_all_settings().await.unwrap().is_empty());

    service
        .set_settings(&[
            ("a".to_owned(), "1".to_owned()),
            ("b".to_owned(), "2".to_owned()),
        ])
        .await
        .unwrap();
    let all = service.get_all_settings().await.unwrap();
    assert_eq!(all.get("a").map(String::as_str), Some("1"));
    assert_eq!(all.get("b").map(String::as_str), Some("2"));
}

#[tokio::test]
async fn creates_and_reads_a_sales_channel() {
    let service = service().await;
    let channel = service
        .create_sales_channel(CreateSalesChannelInput {
            name: "  Web  ".to_owned(),
            description: Some("  Primary storefront  ".to_owned()),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();

    assert!(channel.id.starts_with("sc_"));
    assert_eq!(channel.name, "Web");
    assert_eq!(channel.description.as_deref(), Some("Primary storefront"));
    assert!(channel.is_enabled());
    assert_eq!(
        service.get_sales_channel(&channel.id).await.unwrap(),
        channel
    );
}

#[tokio::test]
async fn sales_channel_names_are_required() {
    let service = service().await;
    let error = service
        .create_sales_channel(channel_input("   "))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
}

#[tokio::test]
async fn duplicate_sales_channel_names_conflict() {
    let service = service().await;
    service
        .create_sales_channel(channel_input("Web"))
        .await
        .unwrap();
    let error = service
        .create_sales_channel(channel_input("Web"))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn renaming_a_sales_channel_to_an_existing_name_conflicts() {
    let service = service().await;
    service
        .create_sales_channel(channel_input("Web"))
        .await
        .unwrap();
    let other = service
        .create_sales_channel(channel_input("POS"))
        .await
        .unwrap();

    let error = service
        .update_sales_channel(
            &other.id,
            UpdateSalesChannelInput {
                name: Some("Web".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    assert_eq!(
        service.get_sales_channel(&other.id).await.unwrap().name,
        "POS"
    );
}

#[tokio::test]
async fn lists_sales_channels_with_pagination() {
    let service = service().await;
    for name in ["A", "B", "C"] {
        service
            .create_sales_channel(channel_input(name))
            .await
            .unwrap();
    }

    let first = service
        .list_sales_channels(Pagination::new(Some(2), Some(0)).unwrap())
        .await
        .unwrap();
    assert_eq!(first.count, 3);
    assert_eq!(first.data.len(), 2);

    let second = service
        .list_sales_channels(Pagination::new(Some(2), Some(2)).unwrap())
        .await
        .unwrap();
    assert_eq!(second.data.len(), 1);
}

#[tokio::test]
async fn pagination_is_validated() {
    let service = service().await;
    let error = service
        .list_sales_channels(Pagination {
            limit: 0,
            offset: 0,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
}

#[tokio::test]
async fn updates_a_sales_channel() {
    let service = service().await;
    let channel = service
        .create_sales_channel(channel_input("Web"))
        .await
        .unwrap();

    let mut metadata = Metadata::empty();
    metadata.insert("theme", serde_json::json!("modern"));
    let updated = service
        .update_sales_channel(
            &channel.id,
            UpdateSalesChannelInput {
                name: Some("Storefront".to_owned()),
                description: Some("Primary".to_owned()),
                metadata: Some(metadata),
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.name, "Storefront");
    assert_eq!(updated.description.as_deref(), Some("Primary"));
    assert_eq!(
        updated.metadata.get("theme"),
        Some(&serde_json::json!("modern"))
    );
}

#[tokio::test]
async fn updating_a_missing_sales_channel_fails() {
    let service = service().await;
    let error = service
        .update_sales_channel(
            "sc_missing",
            UpdateSalesChannelInput {
                name: Some("Name".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn disables_and_enables_sales_channels() {
    let service = service().await;
    let channel = service
        .create_sales_channel(channel_input("POS"))
        .await
        .unwrap();

    let disabled = service.disable_sales_channel(&channel.id).await.unwrap();
    assert!(disabled.is_disabled);
    assert!(!disabled.is_enabled());

    let enabled = service.enable_sales_channel(&channel.id).await.unwrap();
    assert!(!enabled.is_disabled);
}

#[tokio::test]
async fn toggling_a_missing_sales_channel_fails() {
    let service = service().await;
    assert!(
        service
            .disable_sales_channel("sc_missing")
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn sales_channel_metadata_is_size_validated() {
    let service = service().await;
    let mut metadata = Metadata::empty();
    metadata.insert("blob", serde_json::json!("x".repeat(20_000)));
    let error = service
        .create_sales_channel(CreateSalesChannelInput {
            name: "Web".to_owned(),
            description: None,
            metadata,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
}

async fn regions() -> RegionService {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("in-memory store database");
    RegionService::new(db)
}

#[tokio::test]
async fn regions_validate_currency_country_and_default_switching() {
    let service = regions().await;
    let first = service
        .create_region(CreateRegionInput {
            name: "United States".to_owned(),
            currency_code: "USD".to_owned(),
            countries: vec!["US".to_owned()],
            is_default: true,
            ..CreateRegionInput::default()
        })
        .await
        .unwrap();
    assert_eq!(first.countries, vec!["US".to_owned()]);
    assert!(service.default_region().await.unwrap().is_some());

    let second = service
        .create_region(CreateRegionInput {
            name: "Europe".to_owned(),
            currency_code: "EUR".to_owned(),
            countries: vec!["DE".to_owned(), "FR".to_owned()],
            is_default: true,
            ..CreateRegionInput::default()
        })
        .await
        .unwrap();
    assert_eq!(second.countries, vec!["DE".to_owned(), "FR".to_owned()]);
    assert_eq!(
        service.default_region().await.unwrap().unwrap().id,
        second.id
    );
    assert!(
        service
            .create_region(CreateRegionInput {
                name: "Broken".to_owned(),
                currency_code: "ABC".to_owned(),
                countries: vec!["ZZ".to_owned()],
                ..CreateRegionInput::default()
            })
            .await
            .is_err()
    );
    assert!(service.delete_region(&second.id).await.is_err());
    service
        .update_region(
            &second.id,
            UpdateRegionInput {
                name: Some("Eurozone".to_owned()),
                ..UpdateRegionInput::default()
            },
        )
        .await
        .unwrap();
    let page = service.list_regions(Pagination::default()).await.unwrap();
    assert_eq!(page.count, 2);
}

#[tokio::test]
async fn region_update_default_switch_succeeds_under_partial_index() {
    let service = regions().await;
    let first = service
        .create_region(CreateRegionInput {
            name: "United States".to_owned(),
            currency_code: "USD".to_owned(),
            countries: vec!["US".to_owned()],
            is_default: true,
            ..CreateRegionInput::default()
        })
        .await
        .unwrap();
    let second = service
        .create_region(CreateRegionInput {
            name: "Europe".to_owned(),
            currency_code: "EUR".to_owned(),
            countries: vec!["DE".to_owned()],
            is_default: false,
            ..CreateRegionInput::default()
        })
        .await
        .unwrap();

    let promoted = service
        .update_region(
            &second.id,
            UpdateRegionInput {
                is_default: Some(true),
                ..UpdateRegionInput::default()
            },
        )
        .await
        .unwrap();
    assert!(promoted.is_default);
    let default = service.default_region().await.unwrap().unwrap();
    assert_eq!(default.id, second.id);
    assert!(!service.get_region(&first.id).await.unwrap().is_default);
}

#[tokio::test]
async fn noop_region_update_does_not_append_audit() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("in-memory store database");
    let service = RegionService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));
    let region = service
        .create_region(CreateRegionInput {
            name: "United States".to_owned(),
            currency_code: "USD".to_owned(),
            countries: vec!["US".to_owned()],
            ..CreateRegionInput::default()
        })
        .await
        .unwrap();

    // Nothing supplied means nothing changes, so no audit row is appended.
    service
        .update_region(&region.id, UpdateRegionInput::default())
        .await
        .unwrap();
    // Supplying the existing values is also a no-op.
    service
        .update_region(
            &region.id,
            UpdateRegionInput {
                name: Some(region.name.clone()),
                ..UpdateRegionInput::default()
            },
        )
        .await
        .unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action, "region.created");
}

#[tokio::test]
async fn publishable_keys_are_scoped_and_revocable() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("in-memory store database");
    let store = StoreService::new(db.clone());
    let channel = store
        .create_sales_channel(channel_input("Web"))
        .await
        .unwrap();
    let service = RegionService::new(db);
    let created = service
        .create_publishable_key(CreatePublishableApiKeyInput {
            title: "Storefront".to_owned(),
            sales_channel_ids: vec![channel.id.clone()],
        })
        .await
        .unwrap();
    assert!(created.token.starts_with("pk_"));
    let authenticated = service
        .authenticate_publishable_key(&created.token)
        .await
        .unwrap();
    assert_eq!(authenticated.sales_channel_ids, vec![channel.id.clone()]);
    service
        .revoke_publishable_key(&created.key.id)
        .await
        .unwrap();
    assert!(
        service
            .authenticate_publishable_key(&created.token)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn audited_mutations_append_outbox_rows_once() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("in-memory store database");
    let channel = StoreService::new(db.clone())
        .create_sales_channel(channel_input("Web"))
        .await
        .unwrap();
    let service = RegionService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));

    let region = service
        .create_region(CreateRegionInput {
            name: "United States".to_owned(),
            currency_code: "USD".to_owned(),
            countries: vec!["US".to_owned()],
            ..CreateRegionInput::default()
        })
        .await
        .unwrap();
    service
        .update_region(
            &region.id,
            UpdateRegionInput {
                name: Some("USA".to_owned()),
                ..UpdateRegionInput::default()
            },
        )
        .await
        .unwrap();
    service.delete_region(&region.id).await.unwrap();
    // A repeat delete fails and must not append another row.
    assert!(service.delete_region(&region.id).await.is_err());

    let created = service
        .create_publishable_key(CreatePublishableApiKeyInput {
            title: "Storefront".to_owned(),
            sales_channel_ids: vec![channel.id.clone()],
        })
        .await
        .unwrap();
    service
        .revoke_publishable_key(&created.key.id)
        .await
        .unwrap();
    // A repeat revoke fails and must not append another row.
    assert!(
        service
            .revoke_publishable_key(&created.key.id)
            .await
            .is_err()
    );

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec![
            "region.created",
            "region.updated",
            "region.deleted",
            "publishable_key.created",
            "publishable_key.revoked"
        ]
    );
    assert!(audits.iter().all(|audit| audit.processed_at.is_none()));
    assert!(
        audits
            .iter()
            .all(|audit| audit.actor_id.as_deref() == Some("user_1"))
    );
}
