use super::*;

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
