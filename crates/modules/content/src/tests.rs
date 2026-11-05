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

#[tokio::test]
async fn creates_reads_updates_and_soft_deletes_pages() {
    let service = service().await;

    let page = service.create_page(page_input("About Us")).await.unwrap();
    assert_eq!(page.handle, "about-us");
    assert_eq!(page.status, PageStatus::Draft);
    assert_eq!(page.page_type, "custom");
    assert!(page.published_at.is_none());
    assert!(page.deleted_at.is_none());

    let fetched = service.get_page(&page.id).await.unwrap();
    assert_eq!(fetched, page);
    let by_handle = service.get_page_by_handle("about-us").await.unwrap();
    assert_eq!(by_handle.id, page.id);

    let updated = service
        .update_page(
            &page.id,
            UpdatePageInput {
                title: Some("About Fastcart".to_owned()),
                handle: Some("about-fastcart".to_owned()),
                page_type: Some("home".to_owned()),
                body: Some(body_with("hero-2", "About")),
                seo: Some(json!({ "title": "About" })),
                base_revision: Some(page.revision),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "About Fastcart");
    assert_eq!(updated.handle, "about-fastcart");
    assert_eq!(updated.page_type, "home");
    assert_eq!(updated.body, body_with("hero-2", "About"));
    assert_eq!(updated.seo, json!({ "title": "About" }));

    let deleted = service.delete_page(&page.id).await.unwrap();
    assert_eq!(deleted.id, page.id);
    assert_eq!(deleted.revision, updated.revision);
    assert_eq!(deleted.status, updated.status);
    assert!(deleted.deleted_at.is_some());
    assert!(service.get_page(&page.id).await.unwrap_err().is_not_found());
    assert!(
        service
            .get_page_by_handle("about-fastcart")
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn rejects_duplicate_page_handles_on_create_and_update() {
    let service = service().await;
    service.create_page(page_input("About Us")).await.unwrap();

    let error = service
        .create_page(CreatePageInput {
            handle: Some("about-us".to_owned()),
            ..page_input("Other")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let second = service.create_page(page_input("Contact Us")).await.unwrap();
    let error = service
        .update_page(
            &second.id,
            UpdatePageInput {
                handle: Some("about-us".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn rejects_non_object_and_oversized_page_content() {
    let service = service().await;

    let error = service
        .create_page(CreatePageInput {
            body: json!([1, 2, 3]),
            ..page_input("Array Body")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .create_page(CreatePageInput {
            seo: json!("not-an-object"),
            ..page_input("Bad Seo")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let oversized = json!({ "payload": "x".repeat(MAX_BODY_BYTES + 1) });
    let error = service
        .create_page(CreatePageInput {
            body: oversized,
            ..page_input("Huge")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let page = service
        .create_page(CreatePageInput {
            body: json!(null),
            seo: json!(null),
            ..page_input("Nulls")
        })
        .await
        .unwrap();
    assert_eq!(page.body, json!(null));
}

#[tokio::test]
async fn publishing_snapshots_body_and_timestamp() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();

    let published = service.publish_page(&page.id).await.unwrap();
    assert_eq!(published.status, PageStatus::Published);
    assert!(published.published_at.is_some());

    let versions = page_versions(&service, &page.id).await;
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].version, 1);
    assert_eq!(versions[0].body, page.body);

    let updated = service
        .update_page(
            &page.id,
            UpdatePageInput {
                body: Some(body_with("hero-2", "Updated")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.status, PageStatus::Published);
    assert_eq!(updated.body, body_with("hero-2", "Updated"));
    let versions = page_versions(&service, &page.id).await;
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version, 2);
    assert_eq!(versions[0].kind, PageVersionKind::Draft);
    assert_eq!(versions[0].origin, PageRevisionOrigin::Manual);

    service.publish_page(&page.id).await.unwrap();
    let versions = page_versions(&service, &page.id).await;
    assert_eq!(versions.len(), 3);
    assert_eq!(versions[0].version, 3);
    assert_eq!(versions[0].body, updated.body);
    assert_eq!(versions[0].kind, PageVersionKind::Published);
    assert_eq!(versions[0].origin, PageRevisionOrigin::Manual);
}

#[tokio::test]
async fn revisions_start_at_one_and_increment_on_writes() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();
    assert_eq!(page.revision, 1);

    let updated = service
        .update_page(
            &page.id,
            UpdatePageInput {
                title: Some("Home".to_owned()),
                base_revision: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.revision, 2);

    let published = service.publish_page(&page.id).await.unwrap();
    assert_eq!(published.revision, 3);

    let draft = service.unpublish_page(&page.id).await.unwrap();
    assert_eq!(draft.revision, 4);

    let archived = service.archive_page(&page.id).await.unwrap();
    assert_eq!(archived.revision, 5);
}

#[tokio::test]
async fn stale_base_revision_returns_conflict_with_current_revision() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();

    let updated = service
        .update_page(
            &page.id,
            UpdatePageInput {
                title: Some("Home 2".to_owned()),
                base_revision: Some(page.revision),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.revision, 2);

    let error = service
        .update_page(
            &page.id,
            UpdatePageInput {
                title: Some("Home 3".to_owned()),
                base_revision: Some(page.revision),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    match error {
        AppError::ConflictDetails { message, details } => {
            assert!(message.contains("current revision is 2"), "{message}");
            assert_eq!(details["current_revision"], 2);
        }
        other => panic!("expected conflict, got {other:?}"),
    }
    assert_eq!(service.get_page(&page.id).await.unwrap().title, "Home 2");
}

#[tokio::test]
async fn body_updates_create_draft_versions_with_origin() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();
    let body = body_with("hero-ai", "AI Draft");

    let updated = service
        .update_page(
            &page.id,
            UpdatePageInput {
                body: Some(body.clone()),
                base_revision: Some(page.revision),
                origin: Some(PageRevisionOrigin::Ai),
                created_by: Some("user_123".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.revision, 2);

    let versions = page_versions(&service, &page.id).await;
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].kind, PageVersionKind::Draft);
    assert_eq!(versions[0].origin, PageRevisionOrigin::Ai);
    assert_eq!(versions[0].created_by.as_deref(), Some("user_123"));
    assert_eq!(versions[0].body, body);

    // Metadata-only updates do not create another body snapshot.
    service
        .update_page(
            &page.id,
            UpdatePageInput {
                title: Some("Renamed".to_owned()),
                base_revision: Some(updated.revision),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page_versions(&service, &page.id).await.len(), 1);
}

#[tokio::test]
async fn searches_pages_and_navigations_case_insensitively() {
    let service = service().await;
    let about = service.create_page(page_input("About Us")).await.unwrap();
    service.create_page(page_input("Contact")).await.unwrap();
    service
        .create_page(CreatePageInput {
            handle: Some("finding-nemo".to_owned()),
            ..page_input("Movie")
        })
        .await
        .unwrap();
    let removed = service
        .create_page(page_input("About Archive"))
        .await
        .unwrap();
    service.delete_page(&removed.id).await.unwrap();

    let matches = service
        .search_pages(Some("ABOUT"), pagination(10, 0))
        .await
        .unwrap();
    assert_eq!(matches.count, 1);
    assert_eq!(matches.data[0].id, about.id);

    // Handle matching works even when the title differs.
    let by_handle = service
        .search_pages(Some("nemo"), pagination(10, 0))
        .await
        .unwrap();
    assert_eq!(by_handle.count, 1);
    assert_eq!(by_handle.data[0].handle, "finding-nemo");

    // LIKE wildcards in user input are escaped rather than expanded.
    assert_eq!(
        service
            .search_pages(Some("%"), pagination(10, 0))
            .await
            .unwrap()
            .count,
        0
    );
    assert_eq!(
        service
            .search_pages(Some("_"), pagination(10, 0))
            .await
            .unwrap()
            .count,
        0
    );
    assert_eq!(
        service
            .search_pages(None, pagination(10, 0))
            .await
            .unwrap()
            .count,
        3
    );

    let main = service
        .create_navigation(navigation_input("Main Menu"))
        .await
        .unwrap();
    service
        .create_navigation(navigation_input("Footer"))
        .await
        .unwrap();
    let found = service
        .search_navigations(Some("main"), pagination(10, 0))
        .await
        .unwrap();
    assert_eq!(found.count, 1);
    assert_eq!(found.data[0].id, main.id);

    let found_by_handle = service
        .search_navigations(Some("footer"), pagination(10, 0))
        .await
        .unwrap();
    assert_eq!(found_by_handle.count, 1);
    assert_eq!(
        service
            .search_navigations(Some("%"), pagination(10, 0))
            .await
            .unwrap()
            .count,
        0
    );
}

#[tokio::test]
async fn unpublish_and_archive_change_page_status() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();
    service.publish_page(&page.id).await.unwrap();

    let draft = service.unpublish_page(&page.id).await.unwrap();
    assert_eq!(draft.status, PageStatus::Draft);
    assert!(draft.published_at.is_none());

    let archived = service.archive_page(&page.id).await.unwrap();
    assert_eq!(archived.status, PageStatus::Archived);
    assert!(archived.published_at.is_none());
}

#[tokio::test]
async fn versions_can_be_listed_and_restored_as_new_snapshots() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();
    let first_body = page.body.clone();
    service.publish_page(&page.id).await.unwrap();

    let second_body = body_with("hero-2", "Second");
    service
        .update_page(
            &page.id,
            UpdatePageInput {
                body: Some(second_body.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    service.publish_page(&page.id).await.unwrap();

    let versions = page_versions(&service, &page.id).await;
    assert_eq!(versions.len(), 3);
    assert_eq!(versions[0].version, 3);
    assert_eq!(versions[0].kind, PageVersionKind::Published);
    assert_eq!(versions[1].version, 2);
    assert_eq!(versions[1].kind, PageVersionKind::Draft);
    assert_eq!(versions[2].version, 1);

    let restored = service.restore_version(&page.id, 1, None).await.unwrap();
    assert_eq!(restored.body, first_body);

    let versions = page_versions(&service, &page.id).await;
    assert_eq!(versions.len(), 4);
    assert_eq!(versions[0].version, 4);
    assert_eq!(versions[0].body, first_body);
    assert_eq!(versions[0].kind, PageVersionKind::Draft);
    assert_eq!(versions[0].origin, PageRevisionOrigin::Manual);

    let error = service
        .restore_version(&page.id, 99, None)
        .await
        .unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn lists_pages_with_status_filters_pagination_and_soft_delete() {
    let service = service().await;
    let alpha = service.create_page(page_input("Alpha")).await.unwrap();
    let beta = service.create_page(page_input("Beta")).await.unwrap();
    let gamma = service.create_page(page_input("Gamma")).await.unwrap();
    service.publish_page(&beta.id).await.unwrap();

    let drafts = service
        .list_pages(Some(PageStatus::Draft), false, pagination(10, 0))
        .await
        .unwrap();
    assert_eq!(drafts.count, 2);
    assert_eq!(drafts.data.len(), 2);
    assert!(
        drafts
            .data
            .iter()
            .all(|page| page.status == PageStatus::Draft)
    );

    let published = service
        .list_pages(Some(PageStatus::Published), false, pagination(10, 0))
        .await
        .unwrap();
    assert_eq!(published.count, 1);
    assert_eq!(published.data[0].id, beta.id);

    let second_page = service
        .list_pages(None, false, pagination(1, 1))
        .await
        .unwrap();
    assert_eq!(second_page.count, 3);
    assert_eq!(second_page.data.len(), 1);

    service.delete_page(&alpha.id).await.unwrap();
    let live = service
        .list_pages(None, false, pagination(10, 0))
        .await
        .unwrap();
    assert_eq!(live.count, 2);
    assert!(live.data.iter().all(|page| page.id != alpha.id));

    let with_deleted = service
        .list_pages(None, true, pagination(10, 0))
        .await
        .unwrap();
    assert_eq!(with_deleted.count, 3);
    assert!(with_deleted.data.iter().any(|page| page.id == alpha.id));
    assert!(with_deleted.data.iter().any(|page| page.id == gamma.id));
}

#[tokio::test]
async fn navigation_crud_and_handle_uniqueness() {
    let service = service().await;
    let navigation = service
        .create_navigation(navigation_input("Main Menu"))
        .await
        .unwrap();
    assert_eq!(navigation.handle, "main-menu");
    assert!(navigation.items.is_empty());

    let fetched = service.get_navigation(&navigation.id).await.unwrap();
    assert_eq!(fetched, navigation);
    let by_handle = service.get_navigation_by_handle("main-menu").await.unwrap();
    assert_eq!(by_handle.id, navigation.id);

    let updated = service
        .update_navigation(
            &navigation.id,
            crate::UpdateNavigationInput {
                name: Some("Footer".to_owned()),
                handle: Some("footer".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Footer");
    assert_eq!(updated.handle, "footer");

    let error = service
        .create_navigation(CreateNavigationInput {
            handle: Some("footer".to_owned()),
            ..navigation_input("Duplicate")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let listed = service.list_navigations(pagination(10, 0)).await.unwrap();
    assert_eq!(listed.count, 1);
    assert_eq!(listed.data[0].id, navigation.id);

    service.delete_navigation(&navigation.id).await.unwrap();
    assert!(
        service
            .get_navigation(&navigation.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn navigation_items_nest_reorder_and_cascade_delete() {
    let service = service().await;
    let navigation = service
        .create_navigation(navigation_input("Main Menu"))
        .await
        .unwrap();

    let products = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                rank: Some(0),
                ..item_input("Products", "/products")
            },
        )
        .await
        .unwrap();
    let shoes = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                parent_id: Some(products.id.clone()),
                rank: Some(5),
                ..item_input("Shoes", "/products/shoes")
            },
        )
        .await
        .unwrap();
    let about = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                rank: Some(1),
                ..item_input("About", "/about")
            },
        )
        .await
        .unwrap();

    let tree = service.get_navigation_tree("main-menu").await.unwrap();
    assert_eq!(tree.items.len(), 2);
    assert_eq!(tree.items[0].id, products.id);
    assert_eq!(tree.items[0].children.len(), 1);
    assert_eq!(tree.items[0].children[0].id, shoes.id);
    assert_eq!(tree.items[1].id, about.id);

    let reordered = service
        .reorder_navigation_items(&navigation.id, &[about.id.clone(), products.id.clone()])
        .await
        .unwrap();
    assert_eq!(reordered[0].id, about.id);
    assert_eq!(reordered[0].rank, 0);
    assert_eq!(reordered[1].id, products.id);
    assert_eq!(reordered[1].rank, 1);

    let updated = service
        .update_navigation_item(
            &about.id,
            NavigationItemInput {
                parent_id: None,
                title: "Company".to_owned(),
                url: "/company".to_owned(),
                rank: Some(2),
                metadata: Metadata::empty(),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "Company");
    assert_eq!(updated.url, "/company");
    assert_eq!(updated.rank, 2);

    service.remove_navigation_item(&products.id).await.unwrap();
    let after = service.get_navigation(&navigation.id).await.unwrap();
    assert_eq!(after.items.len(), 1);
    assert_eq!(after.items[0].id, about.id);
    assert!(
        service
            .remove_navigation_item(&shoes.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn rejects_navigation_item_cycles_and_foreign_parents() {
    let service = service().await;
    let first = service
        .create_navigation(navigation_input("First"))
        .await
        .unwrap();
    let second = service
        .create_navigation(navigation_input("Second"))
        .await
        .unwrap();

    let root = service
        .add_navigation_item(&first.id, item_input("Root", "/root"))
        .await
        .unwrap();
    let child = service
        .add_navigation_item(
            &first.id,
            NavigationItemInput {
                parent_id: Some(root.id.clone()),
                ..item_input("Child", "/child")
            },
        )
        .await
        .unwrap();
    let foreign = service
        .add_navigation_item(&second.id, item_input("Foreign", "/foreign"))
        .await
        .unwrap();

    let error = service
        .update_navigation_item(
            &root.id,
            NavigationItemInput {
                parent_id: Some(child.id.clone()),
                ..item_input("Root", "/root")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .update_navigation_item(
            &root.id,
            NavigationItemInput {
                parent_id: Some(root.id.clone()),
                ..item_input("Root", "/root")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .add_navigation_item(
            &first.id,
            NavigationItemInput {
                parent_id: Some(foreign.id.clone()),
                ..item_input("Cross Nav", "/cross")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .update_navigation_item(
            &root.id,
            NavigationItemInput {
                parent_id: Some("nitem_missing".to_owned()),
                ..item_input("Root", "/root")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn navigation_cycle_check_sees_the_transactional_parent_chain() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));
    let navigation = service
        .create_navigation(navigation_input("Main Menu"))
        .await
        .unwrap();
    let root = service
        .add_navigation_item(&navigation.id, item_input("Root", "/root"))
        .await
        .unwrap();
    let child = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                parent_id: Some(root.id.clone()),
                ..item_input("Child", "/child")
            },
        )
        .await
        .unwrap();
    let grandchild = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                parent_id: Some(child.id.clone()),
                ..item_input("Grandchild", "/grandchild")
            },
        )
        .await
        .unwrap();

    // Reparenting the root below its own grandchild must be rejected even
    // though the ancestor chain is several levels deep.
    let error = service
        .update_navigation_item(
            &root.id,
            NavigationItemInput {
                parent_id: Some(grandchild.id.clone()),
                ..item_input("Root", "/root")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    // The rejected update left the item untouched and wrote no audit entry.
    let navigation = service.get_navigation(&navigation.id).await.unwrap();
    let stored_root = navigation
        .items
        .iter()
        .find(|item| item.id == root.id)
        .expect("root survives a rejected cycle");
    assert!(stored_root.parent_id.is_none());

    let audits = fetch_pending_audits(db.pool(), 20).await.unwrap();
    assert!(
        !audits
            .iter()
            .any(|audit| audit.action == "navigation_item.updated")
    );
}

#[tokio::test]
async fn updating_a_missing_navigation_item_is_not_found_without_audit() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));

    let error = service
        .update_navigation_item("nitem_missing", item_input("Missing", "/missing"))
        .await
        .unwrap_err();
    assert!(error.is_not_found());

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert!(audits.is_empty());
}

#[tokio::test]
async fn navigation_handle_races_map_to_conflicts_without_audit() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));
    let first = service
        .create_navigation(navigation_input("First"))
        .await
        .unwrap();
    let second = service
        .create_navigation(navigation_input("Second"))
        .await
        .unwrap();

    let error = service
        .update_navigation(
            &second.id,
            UpdateNavigationInput {
                handle: Some(first.handle.clone()),
                ..UpdateNavigationInput::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert!(
        !audits
            .iter()
            .any(|audit| audit.action == "navigation.updated")
    );
}

#[tokio::test]
async fn redirects_validate_and_enforce_unique_paths() {
    let service = service().await;

    let error = service
        .create_redirect(CreateRedirectInput {
            from_path: "legacy".to_owned(),
            to_path: "/new".to_owned(),
            status_code: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .create_redirect(CreateRedirectInput {
            from_path: "/legacy".to_owned(),
            to_path: "   ".to_owned(),
            status_code: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .create_redirect(CreateRedirectInput {
            from_path: "/legacy".to_owned(),
            to_path: "/new".to_owned(),
            status_code: Some(200),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let redirect = service
        .create_redirect(CreateRedirectInput {
            from_path: "/legacy".to_owned(),
            to_path: "/new".to_owned(),
            status_code: None,
        })
        .await
        .unwrap();
    assert_eq!(redirect.status_code, 301);

    let fetched = service.get_redirect_by_path("/legacy").await.unwrap();
    assert_eq!(fetched, redirect);

    let error = service
        .create_redirect(CreateRedirectInput {
            from_path: "/legacy".to_owned(),
            to_path: "/other".to_owned(),
            status_code: Some(302),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let listed = service.list_redirects(pagination(10, 0)).await.unwrap();
    assert_eq!(listed.count, 1);

    service.delete_redirect(&redirect.id).await.unwrap();
    assert!(
        service
            .get_redirect_by_path("/legacy")
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn missing_resources_return_not_found_errors() {
    let service = service().await;
    let page_id = PageId::new();
    let navigation_id = NavigationId::new();

    assert!(service.get_page(&page_id).await.unwrap_err().is_not_found());
    assert!(
        service
            .update_page(&page_id, UpdatePageInput::default())
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .delete_page(&page_id)
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .publish_page(&page_id)
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .unpublish_page(&page_id)
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .archive_page(&page_id)
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .list_page_versions(&page_id, pagination(10, 0))
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .restore_version(&page_id, 1, None)
            .await
            .unwrap_err()
            .is_not_found()
    );

    assert!(
        service
            .get_navigation(&navigation_id)
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .update_navigation(&navigation_id, Default::default())
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .delete_navigation(&navigation_id)
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .add_navigation_item(&navigation_id, item_input("Missing", "/missing"))
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .reorder_navigation_items(&navigation_id, &[])
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .remove_navigation_item("nitem_missing")
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .delete_redirect("redirect_missing")
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn rejects_invalid_titles_handles_and_item_fields() {
    let service = service().await;

    let error = service
        .create_page(CreatePageInput {
            title: "   ".to_owned(),
            ..page_input("Ignored")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .create_page(CreatePageInput {
            handle: Some("About Us".to_owned()),
            ..page_input("About")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .create_navigation(CreateNavigationInput {
            name: String::new(),
            handle: None,
            metadata: Metadata::empty(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let navigation = service
        .create_navigation(navigation_input("Main"))
        .await
        .unwrap();

    let error = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                title: "   ".to_owned(),
                ..item_input("Ignored", "/ignored")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                url: "   ".to_owned(),
                ..item_input("Empty Url", "/ignored")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                rank: Some(-1),
                ..item_input("Negative Rank", "/ignored")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn reorder_rejects_duplicate_foreign_and_missing_items() {
    let service = service().await;
    let first = service
        .create_navigation(navigation_input("First"))
        .await
        .unwrap();
    let second = service
        .create_navigation(navigation_input("Second"))
        .await
        .unwrap();
    let first_item = service
        .add_navigation_item(&first.id, item_input("First Item", "/first"))
        .await
        .unwrap();
    let second_item = service
        .add_navigation_item(&second.id, item_input("Second Item", "/second"))
        .await
        .unwrap();

    let error = service
        .reorder_navigation_items(&first.id, &[first_item.id.clone(), first_item.id.clone()])
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .reorder_navigation_items(&first.id, std::slice::from_ref(&second_item.id))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .reorder_navigation_items(&first.id, &["nitem_missing".to_owned()])
        .await
        .unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn metadata_updates_survive_a_theme_that_no_longer_matches_the_body() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let plain = ContentService::new(db.clone());
    let page = plain.create_page(page_input("Legacy Hero")).await.unwrap();

    let editor = ThemeEditorManifest::parse(&json!({
        "schema_version": 1,
        "categories": {
            "product": { "title": "Product", "components": ["ProductGrid"] }
        },
        "components": {
            "ProductGrid": {
                "title": "Product Grid",
                "category": "product",
                "fields": {
                    "collection": {
                        "type": "data_source",
                        "source": "collection",
                        "required": true
                    }
                },
                "defaultProps": {},
                "slots": []
            }
        }
    }))
    .unwrap();
    let themed = ContentService::with_editor_manifest(db, Some(editor));

    let updated = themed
        .update_page(
            &page.id,
            UpdatePageInput {
                title: Some("Renamed".to_owned()),
                base_revision: Some(page.revision),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "Renamed");
    assert_eq!(updated.body, page.body);

    let error = themed
        .update_page(
            &page.id,
            UpdatePageInput {
                body: Some(body_with("hero-2", "New")),
                base_revision: Some(updated.revision),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn stale_restore_reports_a_revision_conflict() {
    let service = service().await;
    let page = service.create_page(page_input("Versioned")).await.unwrap();
    let published = service.publish_page(&page.id).await.unwrap();
    let updated = service
        .update_page(
            &page.id,
            UpdatePageInput {
                body: Some(body_with("hero-2", "Second")),
                base_revision: Some(published.revision),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let error = service
        .restore_version(&page.id, 1, Some(page.revision))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::ConflictDetails { .. }));

    let restored = service
        .restore_version(&page.id, 1, Some(updated.revision))
        .await
        .unwrap();
    assert_eq!(restored.body, page.body);
}

#[tokio::test]
async fn published_projection_stays_stable_while_editing_draft() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();
    assert!(page.published_body.is_none());

    // A page that has never been published is hidden from storefront reads.
    assert!(
        service
            .get_published_page_by_handle("home")
            .await
            .unwrap_err()
            .is_not_found()
    );

    let published = service.publish_page(&page.id).await.unwrap();
    assert_eq!(published.published_body.as_ref(), Some(&page.body));
    let storefront = service.get_published_page_by_handle("home").await.unwrap();
    assert_eq!(storefront.body, page.body);
    assert!(storefront.published_body.is_none());

    let draft_body = body_with("hero-2", "Draft");
    let updated = service
        .update_page(
            &page.id,
            UpdatePageInput {
                body: Some(draft_body.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.body, draft_body);
    assert_eq!(updated.published_body.as_ref(), Some(&page.body));

    // Editing the draft never changes the live body.
    let storefront = service.get_published_page_by_handle("home").await.unwrap();
    assert_eq!(storefront.body, page.body);
    assert!(storefront.published_body.is_none());

    let republished = service.publish_page(&page.id).await.unwrap();
    assert_eq!(republished.published_body.as_ref(), Some(&draft_body));
    assert_eq!(republished.revision, updated.revision + 1);
    let storefront = service.get_published_page_by_handle("home").await.unwrap();
    assert_eq!(storefront.body, draft_body);

    service.unpublish_page(&page.id).await.unwrap();
    assert!(
        service
            .get_published_page_by_handle("home")
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn publish_is_idempotent_and_rejects_archived_pages() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();
    let published = service.publish_page(&page.id).await.unwrap();
    let snapshot_count = page_versions(&service, &page.id).await.len();

    let republished = service.publish_page(&page.id).await.unwrap();
    assert_eq!(republished.revision, published.revision);
    assert_eq!(
        page_versions(&service, &page.id).await.len(),
        snapshot_count
    );

    let archived = service.archive_page(&page.id).await.unwrap();
    assert_eq!(archived.status, PageStatus::Archived);
    let error = service.publish_page(&page.id).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn unpublish_and_archive_are_idempotent_for_matching_states() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();

    // Unpublishing a draft is a no-op.
    let draft = service.unpublish_page(&page.id).await.unwrap();
    assert_eq!(draft.status, PageStatus::Draft);
    assert_eq!(draft.revision, page.revision);

    let published = service.publish_page(&page.id).await.unwrap();
    let unpublished = service.unpublish_page(&page.id).await.unwrap();
    assert_eq!(unpublished.status, PageStatus::Draft);
    assert_eq!(unpublished.revision, published.revision + 1);

    let archived = service.archive_page(&page.id).await.unwrap();
    assert_eq!(archived.status, PageStatus::Archived);
    let archived_again = service.archive_page(&page.id).await.unwrap();
    assert_eq!(archived_again.revision, archived.revision);

    let error = service.unpublish_page(&page.id).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn restore_updates_only_the_draft_body() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();
    let first_body = page.body.clone();
    service.publish_page(&page.id).await.unwrap();

    let second_body = body_with("hero-2", "Second");
    service
        .update_page(
            &page.id,
            UpdatePageInput {
                body: Some(second_body.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    service.publish_page(&page.id).await.unwrap();

    service
        .update_page(
            &page.id,
            UpdatePageInput {
                body: Some(body_with("hero-3", "Third")),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let restored = service.restore_version(&page.id, 1, None).await.unwrap();
    assert_eq!(restored.body, first_body);
    assert_eq!(restored.published_body.as_ref(), Some(&second_body));

    let storefront = service.get_published_page_by_handle("home").await.unwrap();
    assert_eq!(storefront.body, second_body);

    let versions = page_versions(&service, &page.id).await;
    assert_eq!(
        versions.first().map(|version| version.kind),
        Some(PageVersionKind::Draft)
    );
}

#[tokio::test]
async fn home_lookup_returns_the_latest_published_home_page() {
    let service = service().await;

    let about = service.create_page(page_input("About")).await.unwrap();
    service.publish_page(&about.id).await.unwrap();
    assert!(service.get_home_page().await.unwrap().is_none());

    let home = service
        .create_page(CreatePageInput {
            page_type: Some("home".to_owned()),
            ..page_input("Home")
        })
        .await
        .unwrap();
    let published_home = service.publish_page(&home.id).await.unwrap();

    let found = service.get_home_page().await.unwrap().unwrap();
    assert_eq!(found.id, home.id);
    assert_eq!(found.body, published_home.body);
    assert!(found.published_body.is_none());

    // A draft home page is not returned.
    service
        .create_page(CreatePageInput {
            page_type: Some("home".to_owned()),
            ..page_input("Draft Home")
        })
        .await
        .unwrap();
    assert_eq!(service.get_home_page().await.unwrap().unwrap().id, home.id);

    // Draft edits do not change the published home body.
    service
        .update_page(
            &home.id,
            UpdatePageInput {
                body: Some(body_with("hero-2", "Home Draft")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service.get_home_page().await.unwrap().unwrap().body,
        published_home.body
    );

    service.unpublish_page(&home.id).await.unwrap();
    assert!(service.get_home_page().await.unwrap().is_none());
}

#[tokio::test]
async fn page_version_listing_paginates_with_counts() {
    let service = service().await;
    let page = service.create_page(page_input("Home")).await.unwrap();
    service.publish_page(&page.id).await.unwrap();
    for index in 0..3 {
        service
            .update_page(
                &page.id,
                UpdatePageInput {
                    body: Some(body_with("hero", &format!("Draft {index}"))),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    let first = service
        .list_page_versions(&page.id, pagination(2, 0))
        .await
        .unwrap();
    assert_eq!(first.count, 4);
    assert_eq!(first.data.len(), 2);
    assert_eq!(first.data[0].version, 4);
    assert_eq!(first.data[1].version, 3);

    let second = service
        .list_page_versions(&page.id, pagination(2, 2))
        .await
        .unwrap();
    assert_eq!(second.count, 4);
    assert_eq!(second.data.len(), 2);
    assert_eq!(second.data[0].version, 2);
    assert_eq!(second.data[1].version, 1);

    let beyond = service
        .list_page_versions(&page.id, pagination(2, 10))
        .await
        .unwrap();
    assert_eq!(beyond.count, 4);
    assert!(beyond.data.is_empty());
}

#[tokio::test]
async fn published_snapshot_isolates_all_page_fields_until_republish() {
    let service = service().await;
    let mut metadata = Metadata::empty();
    metadata.insert("locale", json!("en"));
    let page = service
        .create_page(CreatePageInput {
            title: "About Us".to_owned(),
            handle: Some("about-us".to_owned()),
            page_type: Some("custom".to_owned()),
            body: body_with("hero-1", "Live"),
            seo: json!({ "title": "Live SEO" }),
            metadata,
        })
        .await
        .unwrap();
    let published = service.publish_page(&page.id).await.unwrap();
    assert_eq!(published.published_body.as_ref(), Some(&page.body));

    // Draft edits to every storefront-visible field stay off the live page.
    let mut draft_metadata = Metadata::empty();
    draft_metadata.insert("locale", json!("fr"));
    let updated = service
        .update_page(
            &page.id,
            UpdatePageInput {
                title: Some("Draft Title".to_owned()),
                handle: Some("draft-handle".to_owned()),
                page_type: Some("landing".to_owned()),
                body: Some(body_with("hero-2", "Draft")),
                seo: Some(json!({ "title": "Draft SEO" })),
                metadata: Some(draft_metadata),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.handle, "draft-handle");

    let live = service
        .get_published_page_by_handle("about-us")
        .await
        .unwrap();
    assert_eq!(live.title, "About Us");
    assert_eq!(live.handle, "about-us");
    assert_eq!(live.page_type, "custom");
    assert_eq!(live.body, page.body);
    assert_eq!(live.seo, page.seo);
    assert_eq!(live.metadata, page.metadata);
    assert!(live.published_body.is_none());

    // The renamed draft handle is not routable until it is published.
    assert!(
        service
            .get_published_page_by_handle("draft-handle")
            .await
            .unwrap_err()
            .is_not_found()
    );

    // The same projection is available by identifier for page data bindings.
    let by_id = service.get_published_page(&page.id).await.unwrap();
    assert_eq!(by_id.title, "About Us");
    assert_eq!(by_id.handle, "about-us");
    assert_eq!(by_id.body, page.body);
    assert!(by_id.published_body.is_none());

    // Publishing copies every field, not only the body.
    let republished = service.publish_page(&page.id).await.unwrap();
    assert_eq!(republished.revision, updated.revision + 1);
    assert_eq!(republished.published_body.as_ref(), Some(&updated.body));
    let live = service
        .get_published_page_by_handle("draft-handle")
        .await
        .unwrap();
    assert_eq!(live.title, "Draft Title");
    assert_eq!(live.handle, "draft-handle");
    assert_eq!(live.page_type, "landing");
    assert_eq!(live.body, updated.body);
    assert_eq!(live.seo, updated.seo);
    assert_eq!(live.metadata, updated.metadata);
    assert!(
        service
            .get_published_page_by_handle("about-us")
            .await
            .unwrap_err()
            .is_not_found()
    );

    // Republishing an unchanged snapshot is fully idempotent.
    let snapshot_count = page_versions(&service, &page.id).await.len();
    let unchanged = service.publish_page(&page.id).await.unwrap();
    assert_eq!(unchanged.revision, republished.revision);
    assert_eq!(unchanged.published_body.as_ref(), Some(&updated.body));
    assert_eq!(
        page_versions(&service, &page.id).await.len(),
        snapshot_count
    );
}

#[tokio::test]
async fn published_home_pages_order_by_publication_and_ignore_draft_edits() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone());

    let first = service
        .create_page(CreatePageInput {
            page_type: Some("home".to_owned()),
            ..page_input("First Home")
        })
        .await
        .unwrap();
    service.publish_page(&first.id).await.unwrap();

    let second = service
        .create_page(CreatePageInput {
            page_type: Some("home".to_owned()),
            ..page_input("Second Home")
        })
        .await
        .unwrap();
    service.publish_page(&second.id).await.unwrap();

    // Publication timestamps have one-second resolution; pin them so ordering
    // by `published_at DESC` is deterministic regardless of test speed.
    sqlx::query("UPDATE pages SET published_at = ? WHERE id = ?")
        .bind(200_i64)
        .bind(first.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE pages SET published_at = ? WHERE id = ?")
        .bind(100_i64)
        .bind(second.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

    let served = service.get_home_page().await.unwrap().unwrap();
    assert_eq!(served.id, first.id);
    assert_eq!(served.body, first.body);

    // Draft routing edits on the other home page cannot change the winner.
    service
        .update_page(
            &second.id,
            UpdatePageInput {
                page_type: Some("landing".to_owned()),
                handle: Some("second-home-renamed".to_owned()),
                body: Some(body_with("hero-2", "Second Draft")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(service.get_home_page().await.unwrap().unwrap().id, first.id);

    // Draft edits to the served home page keep its published snapshot live.
    service
        .update_page(
            &first.id,
            UpdatePageInput {
                title: Some("First Home Draft".to_owned()),
                body: Some(body_with("hero-3", "First Draft")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let served = service.get_home_page().await.unwrap().unwrap();
    assert_eq!(served.id, first.id);
    assert_eq!(served.title, "First Home");
    assert_eq!(served.body, first.body);

    // Republishing the other home page with a newer timestamp changes the
    // served page.
    service
        .update_page(
            &second.id,
            UpdatePageInput {
                page_type: Some("home".to_owned()),
                body: Some(body_with("hero-4", "Second Republished")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    service.publish_page(&second.id).await.unwrap();
    sqlx::query("UPDATE pages SET published_at = ? WHERE id = ?")
        .bind(300_i64)
        .bind(second.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(
        service.get_home_page().await.unwrap().unwrap().id,
        second.id
    );
}

#[tokio::test]
async fn legacy_published_rows_fall_back_to_live_fields() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone());
    let page = service.create_page(page_input("Legacy")).await.unwrap();
    service.publish_page(&page.id).await.unwrap();

    // Simulate a row published before the snapshot columns existed.
    sqlx::query(
        "UPDATE pages SET published_title = NULL, published_handle = NULL, \
         published_page_type = NULL, published_seo = NULL, published_metadata = NULL \
         WHERE id = ?",
    )
    .bind(page.id.as_str())
    .execute(db.pool())
    .await
    .unwrap();

    let live = service
        .get_published_page_by_handle("legacy")
        .await
        .unwrap();
    assert_eq!(live.title, "Legacy");
    assert_eq!(live.handle, "legacy");
    assert_eq!(live.page_type, "custom");
    assert_eq!(live.body, page.body);
    assert_eq!(live.seo, page.seo);
    assert_eq!(live.metadata, page.metadata);
    assert!(live.published_body.is_none());
}

#[tokio::test]
async fn drafts_cannot_take_over_a_published_snapshot_handle() {
    let service = service().await;

    let published = service
        .create_page(CreatePageInput {
            handle: Some("old".to_owned()),
            ..page_input("Published Page")
        })
        .await
        .unwrap();
    service.publish_page(&published.id).await.unwrap();

    // The draft handle moves away, but the published snapshot still owns the
    // `old` storefront handle.
    service
        .update_page(
            &published.id,
            UpdatePageInput {
                handle: Some("new".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .get_published_page_by_handle("old")
            .await
            .unwrap()
            .id,
        published.id
    );

    let error = service
        .create_page(CreatePageInput {
            handle: Some("old".to_owned()),
            ..page_input("Shadow Page")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    assert!(error.to_string().contains("page handle already exists"));
}

#[tokio::test]
async fn draft_handle_update_cannot_claim_a_published_snapshot_handle() {
    let service = service().await;

    let published = service
        .create_page(CreatePageInput {
            handle: Some("live".to_owned()),
            ..page_input("Live Page")
        })
        .await
        .unwrap();
    service.publish_page(&published.id).await.unwrap();
    // Move the live handle away so the draft update is not caught by the
    // plain live-handle pre-check; only the effective-handle index can detect
    // the collision with the published snapshot.
    service
        .update_page(
            &published.id,
            UpdatePageInput {
                handle: Some("live-draft".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let draft = service.create_page(page_input("Draft Page")).await.unwrap();
    let error = service
        .update_page(
            &draft.id,
            UpdatePageInput {
                handle: Some("live".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    assert!(error.to_string().contains("page handle already exists"));
    assert_eq!(
        service
            .get_published_page_by_handle("live")
            .await
            .unwrap()
            .id,
        published.id
    );
}

#[tokio::test]
async fn publishing_a_renamed_draft_cannot_claim_another_published_handle() {
    let service = service().await;

    // The occupant is published under `shared`; its draft handle then moves
    // away, so `shared` remains its live storefront handle.
    let occupant = service
        .create_page(CreatePageInput {
            handle: Some("shared".to_owned()),
            ..page_input("Occupant")
        })
        .await
        .unwrap();
    service.publish_page(&occupant.id).await.unwrap();
    service
        .update_page(
            &occupant.id,
            UpdatePageInput {
                handle: Some("occupant-draft".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // The publisher is published under `first`, so renaming its draft handle
    // to `shared` is legal while its own published snapshot keeps serving
    // `first`.
    let publisher = service
        .create_page(CreatePageInput {
            handle: Some("first".to_owned()),
            ..page_input("Publisher")
        })
        .await
        .unwrap();
    service.publish_page(&publisher.id).await.unwrap();
    service
        .update_page(
            &publisher.id,
            UpdatePageInput {
                handle: Some("shared".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let error = service.publish_page(&publisher.id).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    assert!(
        error
            .to_string()
            .contains("published handle is already used by another page"),
        "unexpected error: {error}"
    );
    assert_eq!(
        service
            .get_published_page_by_handle("shared")
            .await
            .unwrap()
            .id,
        occupant.id
    );
    assert_eq!(
        service
            .get_published_page_by_handle("first")
            .await
            .unwrap()
            .id,
        publisher.id
    );
}

#[tokio::test]
async fn page_can_change_its_own_draft_handle_and_republish() {
    let service = service().await;

    let page = service
        .create_page(CreatePageInput {
            handle: Some("original".to_owned()),
            ..page_input("Self Rename")
        })
        .await
        .unwrap();
    service.publish_page(&page.id).await.unwrap();

    service
        .update_page(
            &page.id,
            UpdatePageInput {
                handle: Some("renamed".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Until republication, the published snapshot still serves the old handle.
    assert_eq!(
        service
            .get_published_page_by_handle("original")
            .await
            .unwrap()
            .id,
        page.id
    );

    let republished = service.publish_page(&page.id).await.unwrap();
    assert_eq!(republished.handle, "renamed");
    assert!(
        service
            .get_published_page_by_handle("original")
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert_eq!(
        service
            .get_published_page_by_handle("renamed")
            .await
            .unwrap()
            .id,
        page.id
    );
}

#[tokio::test]
async fn mutations_append_audit_entries_in_the_same_transaction() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));

    let page = service.create_page(page_input("Home")).await.unwrap();
    service.publish_page(&page.id).await.unwrap();
    service.archive_page(&page.id).await.unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec!["page.created", "page.published", "page.archived"]
    );
    assert!(audits.iter().all(|audit| audit.processed_at.is_none()));
    assert!(
        audits
            .iter()
            .all(|audit| audit.actor_id.as_deref() == Some("user_1"))
    );
}

#[tokio::test]
async fn stamps_the_active_theme_reference_on_authored_bodies() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let expected = theme_ref("atlas", "1.0.0");
    let themed = ContentService::new(db.clone()).with_theme_ref(Some(expected));

    let page = themed.create_page(page_input("Home")).await.unwrap();
    assert_eq!(
        page.body["theme"],
        json!({ "id": "atlas", "version": "1.0.0", "schema_version": 1 })
    );

    // An empty body is promoted to an object so it still records the theme.
    let empty = themed
        .create_page(CreatePageInput {
            body: json!(null),
            ..page_input("Empty")
        })
        .await
        .unwrap();
    assert_eq!(empty.body["theme"]["id"], "atlas");

    // A stale reference supplied by an external writer is overwritten, not
    // rejected, because the active theme is authoritative on write.
    let outdated = themed
        .create_page(CreatePageInput {
            body: json!({
                "schema_version": 1,
                "theme": { "id": "stale", "version": "0.1.0", "schema_version": 1 },
                "content": [{
                    "type": "Hero",
                    "props": { "id": "hero-1", "title": "Outdated" }
                }]
            }),
            ..page_input("Outdated")
        })
        .await
        .unwrap();
    assert_eq!(outdated.body["theme"]["id"], "atlas");
    assert_eq!(outdated.body["theme"]["version"], "1.0.0");

    let updated = themed
        .update_page(
            &outdated.id,
            UpdatePageInput {
                body: Some(json!({ "schema_version": 1, "content": [] })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.body["theme"]["id"], "atlas");

    // A body-less update leaves a legacy body and its absent theme untouched.
    let legacy = ContentService::new(db.clone())
        .create_page(page_input("Legacy"))
        .await
        .unwrap();
    assert!(legacy.body.get("theme").is_none());
    let renamed = themed
        .update_page(
            &legacy.id,
            UpdatePageInput {
                title: Some("Legacy Renamed".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(renamed.body.get("theme").is_none());
}

#[tokio::test]
async fn theme_activation_validates_bodies_against_the_candidate_editor() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let expected = theme_ref("atlas", "1.0.0");
    // A body stamped by another theme still passes when its blocks are valid
    // under the candidate editor: activation is the migration point.
    let other_theme = json!({
        "schema_version": 1,
        "theme": { "id": "other", "version": "9.9.9", "schema_version": 1 },
        "content": []
    });
    let page = ContentService::new(db.clone())
        .create_page(CreatePageInput {
            body: other_theme,
            ..page_input("External")
        })
        .await
        .unwrap();

    let themed = ContentService::new(db).with_theme_ref(Some(expected));
    let issues = themed.validate_all_pages(&editor_manifest()).await.unwrap();
    assert!(issues.is_empty(), "{issues:?}");

    // Publishing through a service bound to the active theme still rejects a
    // foreign reference, because writes must target the active theme.
    let error = themed.publish_page(&page.id).await.unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    assert!(error.to_string().contains("theme reference"));
}

#[tokio::test]
async fn validate_all_pages_skips_empty_and_deleted_pages() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let expected = theme_ref("atlas", "1.0.0");
    let plain = ContentService::new(db.clone());
    let themed = ContentService::new(db).with_theme_ref(Some(expected.clone()));
    let manifest = editor_manifest();

    // Empty and unversioned legacy bodies carry no theme constraint and pass.
    plain
        .create_page(CreatePageInput {
            body: json!({}),
            ..page_input("Empty")
        })
        .await
        .unwrap();
    plain.create_page(page_input("Legacy")).await.unwrap();

    // Deleted pages are not validated.
    let removed = themed.create_page(page_input("Removed")).await.unwrap();
    themed.delete_page(&removed.id).await.unwrap();

    // Archived pages are withdrawn from the storefront and cannot block a
    // theme switch, even when their old body does not fit the candidate theme.
    let archived = plain
        .create_page(CreatePageInput {
            body: json!({
                "schema_version": 1,
                "content": [{ "type": "ArchivedUnknown", "props": { "id": "archived" } }]
            }),
            ..page_input("Archived")
        })
        .await
        .unwrap();
    plain.archive_page(&archived.id).await.unwrap();

    // A body with a valid theme reference but an unknown block is reported.
    let bad = themed
        .create_page(CreatePageInput {
            body: json!({
                "schema_version": 1,
                "content": [{ "type": "Unknown", "props": { "id": "x" } }]
            }),
            ..page_input("Bad Block")
        })
        .await
        .unwrap();

    let issues = themed.validate_all_pages(&manifest).await.unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].page_id, bad.id);
    assert_eq!(issues[0].handle, "bad-block");
    assert!(issues[0].error.contains("unknown block type"));
}
