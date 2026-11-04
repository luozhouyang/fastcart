//! Integration-style tests for the asset module service.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, Pagination, StoreId};

use crate::{
    AssetFilter, AssetService, CreateAssetInput, MAX_ALT_LENGTH, MAX_CONTENT_TYPE_LENGTH,
    MAX_FILENAME_LENGTH, MAX_SIZE_BYTES, MAX_STORAGE_KEY_LENGTH,
};

trait ValidationError {
    fn is_validation(&self) -> bool;
}

impl ValidationError for AppError {
    fn is_validation(&self) -> bool {
        matches!(self, Self::Validation(_))
    }
}

async fn service() -> AssetService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    AssetService::new(db)
}

fn asset_input(filename: &str) -> CreateAssetInput {
    CreateAssetInput {
        filename: filename.to_owned(),
        content_type: "image/png".to_owned(),
        size_bytes: 1024,
        storage_key: format!("assets/{filename}"),
        alt: None,
        width: Some(640),
        height: Some(480),
        created_by: Some("user_1".to_owned()),
    }
}

fn pagination(limit: i64, offset: i64) -> Pagination {
    Pagination::new(Some(limit), Some(offset)).unwrap()
}

#[tokio::test]
async fn creates_reads_lists_searches_and_soft_deletes_assets() {
    let service = service().await;

    let mut logo = asset_input("logo.png");
    logo.alt = Some("Company Logo".to_owned());
    let logo = service.create_asset(logo).await.unwrap();
    assert!(logo.id.starts_with("asset_"));
    assert_eq!(logo.filename, "logo.png");
    assert_eq!(logo.content_type, "image/png");
    assert_eq!(logo.size_bytes, 1024);
    assert_eq!(logo.storage_key, "assets/logo.png");
    assert_eq!(logo.alt.as_deref(), Some("Company Logo"));
    assert_eq!(logo.width, Some(640));
    assert_eq!(logo.height, Some(480));
    assert_eq!(logo.created_by.as_deref(), Some("user_1"));
    assert!(logo.deleted_at.is_none());

    let brochure = service
        .create_asset(CreateAssetInput {
            filename: "brochure.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            size_bytes: 4096,
            storage_key: "assets/brochure.pdf".to_owned(),
            alt: None,
            width: None,
            height: None,
            created_by: None,
        })
        .await
        .unwrap();

    let fetched = service.get_asset(&logo.id).await.unwrap();
    assert_eq!(fetched, logo);

    let page = service
        .list_assets(AssetFilter::default(), pagination(1, 0))
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].id, brochure.id);

    let second = service
        .list_assets(AssetFilter::default(), pagination(1, 1))
        .await
        .unwrap();
    assert_eq!(second.count, 2);
    assert_eq!(second.data[0].id, logo.id);

    let by_filename = service
        .list_assets(
            AssetFilter {
                search: Some("LOGO".to_owned()),
            },
            pagination(10, 0),
        )
        .await
        .unwrap();
    assert_eq!(by_filename.count, 1);
    assert_eq!(by_filename.data[0].id, logo.id);

    let by_alt = service
        .list_assets(
            AssetFilter {
                search: Some("company".to_owned()),
            },
            pagination(10, 0),
        )
        .await
        .unwrap();
    assert_eq!(by_alt.count, 1);
    assert_eq!(by_alt.data[0].id, logo.id);

    let deleted = service.delete_asset(&logo.id).await.unwrap();
    assert_eq!(deleted.id, logo.id);
    assert!(deleted.deleted_at.is_some());
    assert!(
        service
            .get_asset(&logo.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .delete_asset(&logo.id)
            .await
            .unwrap_err()
            .is_not_found()
    );

    let remaining = service
        .list_assets(AssetFilter::default(), pagination(10, 0))
        .await
        .unwrap();
    assert_eq!(remaining.count, 1);
    assert_eq!(remaining.data[0].id, brochure.id);
}

#[tokio::test]
async fn list_search_escapes_like_wildcards() {
    let service = service().await;
    service
        .create_asset(asset_input("100%_off.png"))
        .await
        .unwrap();
    service
        .create_asset(asset_input("plain.png"))
        .await
        .unwrap();

    let matches = service
        .list_assets(
            AssetFilter {
                search: Some("%".to_owned()),
            },
            pagination(10, 0),
        )
        .await
        .unwrap();
    assert_eq!(matches.count, 1);
    assert_eq!(matches.data[0].filename, "100%_off.png");
}

#[tokio::test]
async fn create_normalizes_text_fields() {
    let service = service().await;
    let asset = service
        .create_asset(CreateAssetInput {
            filename: "  spaced name.png  ".to_owned(),
            content_type: " image/png ".to_owned(),
            size_bytes: 0,
            storage_key: "  assets/spaced.png  ".to_owned(),
            alt: Some("   ".to_owned()),
            width: Some(0),
            height: Some(0),
            created_by: None,
        })
        .await
        .unwrap();
    assert_eq!(asset.filename, "spaced name.png");
    assert_eq!(asset.content_type, "image/png");
    assert_eq!(asset.storage_key, "assets/spaced.png");
    assert_eq!(asset.alt, None);
    assert_eq!(asset.width, Some(0));
    assert_eq!(asset.height, Some(0));
}

#[tokio::test]
async fn validation_rejects_bad_input() {
    let service = service().await;

    let mut input = asset_input("ok.png");
    input.filename = String::new();
    assert!(
        service
            .create_asset(input.clone())
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.filename = "a".repeat(MAX_FILENAME_LENGTH + 1);
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.storage_key = String::new();
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.storage_key = "a".repeat(MAX_STORAGE_KEY_LENGTH + 1);
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.content_type = String::new();
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.content_type = "a".repeat(MAX_CONTENT_TYPE_LENGTH + 1);
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.content_type = "image/\nng".to_owned();
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.size_bytes = -1;
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.size_bytes = MAX_SIZE_BYTES + 1;
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.alt = Some("a".repeat(MAX_ALT_LENGTH + 1));
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.width = Some(-1);
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    input = asset_input("ok.png");
    input.height = Some(-1);
    assert!(
        service
            .create_asset(input)
            .await
            .unwrap_err()
            .is_validation()
    );

    assert!(
        service
            .get_asset("asset_missing")
            .await
            .unwrap_err()
            .is_not_found()
    );
}
