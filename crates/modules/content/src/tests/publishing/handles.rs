use super::super::*;

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
