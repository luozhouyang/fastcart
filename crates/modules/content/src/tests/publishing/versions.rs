use super::super::*;

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
