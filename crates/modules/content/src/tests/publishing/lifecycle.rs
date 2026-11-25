use super::super::*;

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
