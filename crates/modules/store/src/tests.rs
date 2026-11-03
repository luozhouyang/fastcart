//! Store-module tests.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{Metadata, Pagination, StoreId};

use crate::{
    CreateSalesChannelInput, MAX_SETTING_KEY_LENGTH, MAX_SETTING_VALUE_BYTES, StoreService,
    UpdateSalesChannelInput,
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
