use fastcart_kernel::{Pagination, StoreId};

use super::{CreateStoreInput, StoreStatus, UpdateStoreInput};
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
