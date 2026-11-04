//! Integration-style tests for the content module service.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, Metadata, NavigationId, PageId, Pagination, StoreId};
use fastcart_storefront::ThemeEditorManifest;
use serde_json::json;

use crate::{
    ContentService, CreateNavigationInput, CreatePageInput, CreateRedirectInput, MAX_BODY_BYTES,
    NavigationItemInput, PageRevisionOrigin, PageStatus, PageVersionKind, UpdatePageInput,
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

    service.delete_page(&page.id).await.unwrap();
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

    let versions = service.list_page_versions(&page.id).await.unwrap();
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
    let versions = service.list_page_versions(&page.id).await.unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[1].kind, PageVersionKind::Draft);
    assert_eq!(versions[1].origin, PageRevisionOrigin::Manual);

    service.publish_page(&page.id).await.unwrap();
    let versions = service.list_page_versions(&page.id).await.unwrap();
    assert_eq!(versions.len(), 3);
    assert_eq!(versions[2].version, 3);
    assert_eq!(versions[2].body, updated.body);
    assert_eq!(versions[2].kind, PageVersionKind::Published);
    assert_eq!(versions[2].origin, PageRevisionOrigin::Manual);
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
        AppError::Conflict(message) => {
            assert!(message.contains("current revision is 2"), "{message}");
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

    let versions = service.list_page_versions(&page.id).await.unwrap();
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
    assert_eq!(service.list_page_versions(&page.id).await.unwrap().len(), 1);
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

    let versions = service.list_page_versions(&page.id).await.unwrap();
    assert_eq!(versions.len(), 3);
    assert_eq!(versions[0].version, 1);
    assert_eq!(versions[1].version, 2);
    assert_eq!(versions[1].kind, PageVersionKind::Draft);
    assert_eq!(versions[2].version, 3);
    assert_eq!(versions[2].kind, PageVersionKind::Published);

    let restored = service.restore_version(&page.id, 1, None).await.unwrap();
    assert_eq!(restored.body, first_body);

    let versions = service.list_page_versions(&page.id).await.unwrap();
    assert_eq!(versions.len(), 4);
    assert_eq!(versions[3].version, 4);
    assert_eq!(versions[3].body, first_body);
    assert_eq!(versions[3].kind, PageVersionKind::Draft);
    assert_eq!(versions[3].origin, PageRevisionOrigin::Manual);

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
            .list_page_versions(&page_id)
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
    assert!(matches!(error, AppError::Conflict(_)));

    let restored = service
        .restore_version(&page.id, 1, Some(updated.revision))
        .await
        .unwrap();
    assert_eq!(restored.body, page.body);
}
