//! Integration-style tests for the content module service.

use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
use fastcart_kernel::{AppError, Metadata, NavigationId, PageId, Pagination, StoreId};
use fastcart_storefront::{PageThemeRef, SUPPORTED_EDITOR_SCHEMA_VERSION, ThemeEditorManifest};
use serde_json::json;

use crate::{
    ContentService, CreateNavigationInput, CreatePageInput, CreateRedirectInput, MAX_BODY_BYTES,
    NavigationItemInput, PageRevisionOrigin, PageStatus, PageVersion, PageVersionKind,
    UpdateNavigationInput, UpdatePageInput,
};

async fn service() -> ContentService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    ContentService::new(db)
}

fn body_with(id: &str, title: &str) -> serde_json::Value {
    json!({
        "schema_version": 1,
        "content": [{
            "type": "Hero",
            "props": { "id": id, "title": title }
        }]
    })
}

fn page_input(title: &str) -> CreatePageInput {
    CreatePageInput {
        title: title.to_owned(),
        handle: None,
        page_type: None,
        body: body_with("hero-1", title),
        seo: json!({ "title": title }),
        metadata: Metadata::empty(),
    }
}

fn navigation_input(name: &str) -> CreateNavigationInput {
    CreateNavigationInput {
        name: name.to_owned(),
        handle: None,
        metadata: Metadata::empty(),
    }
}

fn item_input(title: &str, url: &str) -> NavigationItemInput {
    NavigationItemInput {
        parent_id: None,
        title: title.to_owned(),
        url: url.to_owned(),
        rank: None,
        metadata: Metadata::empty(),
    }
}

fn pagination(limit: i64, offset: i64) -> Pagination {
    Pagination::new(Some(limit), Some(offset)).unwrap()
}

fn theme_ref(id: &str, version: &str) -> PageThemeRef {
    PageThemeRef {
        id: id.to_owned(),
        version: version.to_owned(),
        schema_version: SUPPORTED_EDITOR_SCHEMA_VERSION,
    }
}

fn editor_manifest() -> ThemeEditorManifest {
    ThemeEditorManifest::parse(&json!({
        "schema_version": 1,
        "categories": {
            "hero": { "title": "Hero", "components": ["Hero"] }
        },
        "components": {
            "Hero": {
                "title": "Hero",
                "category": "hero",
                "fields": {
                    "title": { "type": "text", "required": true }
                },
                "defaultProps": {},
                "slots": []
            }
        }
    }))
    .unwrap()
}

async fn page_versions(service: &ContentService, page_id: &PageId) -> Vec<PageVersion> {
    service
        .list_page_versions(page_id, pagination(100, 0))
        .await
        .unwrap()
        .data
}

mod audit;
mod navigation;
mod pages;
mod publishing;
mod redirects;
mod theme;
mod translations;
mod validation;
