use super::*;

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

#[tokio::test]
async fn system_pages_are_seeded_and_idempotent() {
    let service = service().await;

    service.ensure_system_pages().await.unwrap();
    let cart = service.get_published_page_by_handle("cart").await.unwrap();
    let checkout = service
        .get_published_page_by_handle("checkout")
        .await
        .unwrap();
    assert_eq!(cart.title, "Cart");
    assert_eq!(checkout.title, "Checkout");
    assert_eq!(cart.body["content"][0]["type"], "Heading");

    // A second bootstrap must not create duplicates or change the existing page.
    service.ensure_system_pages().await.unwrap();
    let again = service.get_published_page_by_handle("cart").await.unwrap();
    assert_eq!(again.id, cart.id);
}
