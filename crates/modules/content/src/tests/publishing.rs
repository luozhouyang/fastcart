use super::*;

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
