use super::*;

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
