use super::*;

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
