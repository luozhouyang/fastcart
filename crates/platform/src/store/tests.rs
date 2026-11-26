use fastcart_kernel::{Pagination, StoreId};

use super::{
    AddStoreLocaleInput, CreateStoreInput, LocaleDirection, StoreStatus, UpdateStoreInput,
    UpdateStoreLocaleInput,
};
use crate::repository::PlatformStore;

async fn store() -> PlatformStore {
    PlatformStore::in_memory().await.unwrap()
}

fn input(slug: &str, domain: &str) -> CreateStoreInput {
    CreateStoreInput::new("Demo Store", slug, domain)
}

#[tokio::test]
async fn creates_store_and_primary_domain() {
    let platform = store().await;
    let store = platform
        .create_store(input("demo", "Shop.Example.com"))
        .await
        .unwrap();

    assert_eq!(store.slug, "demo");
    assert_eq!(store.status, StoreStatus::Provisioning);
    assert_eq!(store.primary_domain.as_deref(), Some("shop.example.com"));

    let resolved = platform
        .get_store_by_domain("shop.example.com:443")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.id, store.id);
}

#[tokio::test]
async fn duplicate_slugs_and_domains_are_rejected() {
    let platform = store().await;
    platform
        .create_store(input("demo", "demo.example.com"))
        .await
        .unwrap();

    let slug_error = platform
        .create_store(input("demo", "other.example.com"))
        .await
        .unwrap_err();
    assert!(matches!(slug_error, crate::PlatformError::SlugConflict(_)));

    let domain_error = platform
        .create_store(input("other", "demo.example.com"))
        .await
        .unwrap_err();
    assert!(matches!(
        domain_error,
        crate::PlatformError::DomainConflict(_)
    ));
}

#[tokio::test]
async fn lifecycle_transitions_are_enforced() {
    let platform = store().await;
    let store = platform
        .create_store(input("demo", "demo.example.com"))
        .await
        .unwrap();

    assert!(
        platform
            .update_status(&store.id, StoreStatus::Deleted)
            .await
            .is_err()
    );
    let active = platform
        .update_status(&store.id, StoreStatus::Active)
        .await
        .unwrap();
    assert!(active.status.can_serve_traffic());

    let suspended = platform
        .update_status(&store.id, StoreStatus::Suspended)
        .await
        .unwrap();
    assert!(!suspended.status.can_serve_traffic());
    platform
        .update_status(&store.id, StoreStatus::Active)
        .await
        .unwrap();
}

#[tokio::test]
async fn domains_can_be_added_and_made_primary() {
    let platform = store().await;
    let store = platform
        .create_store(input("demo", "demo.example.com"))
        .await
        .unwrap();

    let added = platform
        .add_domain(&store.id, "www.example.com", false)
        .await
        .unwrap();
    assert!(!added.is_primary);

    let primary = platform
        .set_primary_domain(&store.id, "www.example.com")
        .await
        .unwrap();
    assert!(primary.is_primary);

    let domains = platform.list_domains(&store.id).await.unwrap();
    assert_eq!(domains.len(), 2);
    assert_eq!(domains.iter().filter(|domain| domain.is_primary).count(), 1);

    assert!(
        platform
            .remove_domain(&store.id, "www.example.com")
            .await
            .is_err()
    );
    platform
        .remove_domain(&store.id, "demo.example.com")
        .await
        .unwrap();
}

#[tokio::test]
async fn lists_stores_with_pagination() {
    let platform = store().await;
    for index in 0..3 {
        platform
            .create_store(input(
                &format!("store-{index}"),
                &format!("store-{index}.example.com"),
            ))
            .await
            .unwrap();
    }
    let page = platform
        .list_stores(Pagination::new(Some(2), Some(0)).unwrap())
        .await
        .unwrap();
    assert_eq!(page.data.len(), 2);
    assert_eq!(page.count, 3);
}

#[tokio::test]
async fn updates_store_attributes() {
    let platform = store().await;
    let store = platform
        .create_store(input("demo", "demo.example.com"))
        .await
        .unwrap();
    let updated = platform
        .update_store(
            &store.id,
            UpdateStoreInput {
                name: Some("Renamed".into()),
                locale: Some("fr-FR".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.locale, "fr-FR");
}

#[tokio::test]
async fn rejects_invalid_input() {
    let platform = store().await;
    let mut invalid = input("Bad Slug", "demo.example.com");
    invalid.slug = "Bad Slug".into();
    assert!(platform.create_store(invalid).await.is_err());

    let invalid = input("demo", "http://demo.example.com");
    assert!(platform.create_store(invalid).await.is_err());
}

#[tokio::test]
async fn missing_store_is_not_found() {
    let platform = store().await;
    assert!(platform.get_store(&StoreId::new()).await.unwrap().is_none());
}

fn locale_input(locale: &str) -> AddStoreLocaleInput {
    AddStoreLocaleInput {
        locale: locale.to_owned(),
        ..Default::default()
    }
}

#[tokio::test]
async fn adds_normalizes_and_lists_store_locales() {
    let platform = store().await;
    let store = platform
        .create_store(input("demo", "demo.example.com"))
        .await
        .unwrap();

    let first = platform
        .add_store_locale(&store.id, locale_input("en_us"))
        .await
        .unwrap();
    assert_eq!(first.locale, "en-US", "locale is normalized");
    assert!(first.is_default, "the first locale becomes the default");
    assert_eq!(first.direction, LocaleDirection::Ltr);

    let second = platform
        .add_store_locale(
            &store.id,
            AddStoreLocaleInput {
                locale: "fr-FR".to_owned(),
                is_default: false,
                label: Some("  Français  ".to_owned()),
                direction: Some("rtl".to_owned()),
            },
        )
        .await
        .unwrap();
    assert!(!second.is_default);
    assert_eq!(second.label.as_deref(), Some("Français"));
    assert_eq!(second.direction, LocaleDirection::Rtl);

    let locales = platform.list_store_locales(&store.id).await.unwrap();
    assert_eq!(locales.len(), 2);
    assert_eq!(locales[0].locale, "en-US", "the default sorts first");
    assert_eq!(locales[1].locale, "fr-FR");

    let duplicate = platform
        .add_store_locale(&store.id, locale_input("fr_FR"))
        .await
        .unwrap_err();
    assert!(matches!(duplicate, crate::PlatformError::Conflict(_)));

    let invalid = platform
        .add_store_locale(&store.id, locale_input("not a locale"))
        .await
        .unwrap_err();
    assert!(matches!(invalid, crate::PlatformError::Kernel(_)));

    let invalid_direction = platform
        .add_store_locale(
            &store.id,
            AddStoreLocaleInput {
                locale: "de-DE".to_owned(),
                direction: Some("up".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        invalid_direction,
        crate::PlatformError::InvalidInput(_)
    ));

    let missing_store = platform
        .add_store_locale(&StoreId::new(), locale_input("fr-FR"))
        .await
        .unwrap_err();
    assert!(matches!(
        missing_store,
        crate::PlatformError::StoreNotFound(_)
    ));
}

#[tokio::test]
async fn setting_the_default_locale_clears_the_previous_one() {
    let platform = store().await;
    let store = platform
        .create_store(input("demo", "demo.example.com"))
        .await
        .unwrap();
    platform
        .add_store_locale(&store.id, locale_input("en-US"))
        .await
        .unwrap();
    platform
        .add_store_locale(&store.id, locale_input("fr-FR"))
        .await
        .unwrap();

    let default = platform
        .set_default_store_locale(&store.id, "fr_FR")
        .await
        .unwrap();
    assert!(default.is_default);
    assert_eq!(default.locale, "fr-FR");

    let locales = platform.list_store_locales(&store.id).await.unwrap();
    assert_eq!(locales.iter().filter(|locale| locale.is_default).count(), 1);
    // The default sorts first, so the previous default moves after it.
    assert_eq!(locales[0].locale, "fr-FR");
    assert!(locales[0].is_default, "fr-FR is the new default");
    assert!(!locales[1].is_default, "en-US is no longer the default");

    // The partial unique index accepts the swap only after the clear.
    let default_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM store_locales WHERE store_id = ? AND is_default = 1",
    )
    .bind(store.id.as_str())
    .fetch_one(platform.pool())
    .await
    .unwrap();
    assert_eq!(default_count, 1);

    let missing = platform
        .set_default_store_locale(&store.id, "de-DE")
        .await
        .unwrap_err();
    assert!(matches!(missing, crate::PlatformError::NotFound(_)));
}

#[tokio::test]
async fn the_schema_rejects_a_second_default_locale() {
    let platform = store().await;
    let store = platform
        .create_store(input("demo", "demo.example.com"))
        .await
        .unwrap();
    platform
        .add_store_locale(&store.id, locale_input("en-US"))
        .await
        .unwrap();

    let duplicate_default = sqlx::query(
        "INSERT INTO store_locales
         (store_id, locale, is_default, label, direction, created_at, updated_at)
         VALUES (?, 'fr-FR', 1, NULL, 'ltr', 0, 0)",
    )
    .bind(store.id.as_str())
    .execute(platform.pool())
    .await;
    assert!(
        duplicate_default.is_err(),
        "the partial unique index must reject a second default"
    );

    // Non-default rows are unconstrained by the partial index.
    sqlx::query(
        "INSERT INTO store_locales
         (store_id, locale, is_default, label, direction, created_at, updated_at)
         VALUES (?, 'fr-FR', 0, NULL, 'ltr', 0, 0)",
    )
    .bind(store.id.as_str())
    .execute(platform.pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn removing_the_default_locale_is_rejected() {
    let platform = store().await;
    let store = platform
        .create_store(input("demo", "demo.example.com"))
        .await
        .unwrap();
    platform
        .add_store_locale(&store.id, locale_input("en-US"))
        .await
        .unwrap();
    platform
        .add_store_locale(&store.id, locale_input("fr-FR"))
        .await
        .unwrap();

    let error = platform
        .remove_store_locale(&store.id, "en-US")
        .await
        .unwrap_err();
    assert!(matches!(error, crate::PlatformError::Conflict(_)));

    platform
        .remove_store_locale(&store.id, "fr-FR")
        .await
        .unwrap();
    let locales = platform.list_store_locales(&store.id).await.unwrap();
    assert_eq!(locales.len(), 1);
    assert!(locales[0].is_default);

    let missing = platform
        .remove_store_locale(&store.id, "fr-FR")
        .await
        .unwrap_err();
    assert!(matches!(missing, crate::PlatformError::NotFound(_)));

    // Promoting another locale makes it protected again.
    platform
        .add_store_locale(
            &store.id,
            AddStoreLocaleInput {
                locale: "fr-FR".to_owned(),
                is_default: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let error = platform
        .remove_store_locale(&store.id, "fr-FR")
        .await
        .unwrap_err();
    assert!(matches!(error, crate::PlatformError::Conflict(_)));
}

#[tokio::test]
async fn adding_a_default_locale_clears_the_previous_default() {
    let platform = store().await;
    let store = platform
        .create_store(input("demo", "demo.example.com"))
        .await
        .unwrap();
    platform
        .add_store_locale(&store.id, locale_input("en-US"))
        .await
        .unwrap();
    let added = platform
        .add_store_locale(
            &store.id,
            AddStoreLocaleInput {
                locale: "de-DE".to_owned(),
                is_default: true,
                label: Some("Deutsch".to_owned()),
                direction: None,
            },
        )
        .await
        .unwrap();
    assert!(added.is_default);

    let locales = platform.list_store_locales(&store.id).await.unwrap();
    let defaults: Vec<&str> = locales
        .iter()
        .filter(|locale| locale.is_default)
        .map(|locale| locale.locale.as_str())
        .collect();
    assert_eq!(defaults, ["de-DE"]);
}

#[tokio::test]
async fn updates_locale_label_and_direction() {
    let platform = store().await;
    let store = platform
        .create_store(input("demo", "demo.example.com"))
        .await
        .unwrap();
    platform
        .add_store_locale(
            &store.id,
            AddStoreLocaleInput {
                locale: "ar-SA".to_owned(),
                label: Some("العربية".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let updated = platform
        .update_store_locale(
            &store.id,
            "ar_SA",
            UpdateStoreLocaleInput {
                label: None,
                direction: Some("RTL".to_owned()),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.locale, "ar-SA");
    assert_eq!(updated.label.as_deref(), Some("العربية"));
    assert_eq!(updated.direction, LocaleDirection::Rtl);

    let cleared = platform
        .update_store_locale(
            &store.id,
            "ar-SA",
            UpdateStoreLocaleInput {
                label: Some("  ".to_owned()),
                direction: None,
            },
        )
        .await
        .unwrap();
    assert!(cleared.label.is_none());
    assert_eq!(cleared.direction, LocaleDirection::Rtl);

    let missing = platform
        .update_store_locale(
            &store.id,
            "fr-FR",
            UpdateStoreLocaleInput {
                label: Some("Français".to_owned()),
                direction: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(missing, crate::PlatformError::NotFound(_)));
}
