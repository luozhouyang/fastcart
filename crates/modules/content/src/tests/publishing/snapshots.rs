use super::super::*;

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
