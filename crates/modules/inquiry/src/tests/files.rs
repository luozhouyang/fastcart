use super::*;

#[tokio::test]
async fn attachment_is_claimed_once_by_an_inquiry() {
    let db = test_db().await;
    let fields = vec![
        text_field("name", false),
        email_field("email", true),
        file_field("attachments", 2, &["image/png"]),
    ];
    let published = publish_custom(&db, "Attachment", fields, InquiryFormSettings::default()).await;
    let file_service = InquiryFileService::new(db.clone());
    let service = InquiryService::new(db.clone());

    let file = file_service
        .create_pending(
            &published.form.handle,
            "attachments",
            "inquiries/store/form/a.png".to_owned(),
            Some("a.png".to_owned()),
            "image/png".to_owned(),
            128,
            "a".repeat(64),
            now_timestamp() + 3_600,
        )
        .await
        .unwrap();
    assert_eq!(file.status, InquiryFileStatus::Pending);
    assert!(file.inquiry_id.is_none());

    let mut input = inquiry_input(&published.form.handle);
    input.email = Some("alice@example.com".to_owned());
    input.name = Some("Alice".to_owned());
    input.message = None;
    input
        .files
        .insert("attachments".to_owned(), vec![file.id.clone()]);

    let inquiry = service.create(input.clone()).await.unwrap();
    let files = service.files_for(&inquiry.id).await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, InquiryFileStatus::Claimed);
    assert_eq!(files[0].inquiry_id.as_ref(), Some(&inquiry.id));
    assert!(files[0].claimed_at.is_some());

    let download = service
        .file_for_download(&inquiry.id, &file.id)
        .await
        .unwrap();
    assert_eq!(download.id, file.id);
    assert_eq!(download.storage_key, "inquiries/store/form/a.png");

    // A different inquiry cannot claim the same upload again.
    let error = service.create(input).await.unwrap_err();
    assert!(matches!(
        error,
        AppError::Validation(_) | AppError::Conflict(_)
    ));
}

#[tokio::test]
async fn file_service_rejects_invalid_uploads_and_expires_pending() {
    let db = test_db().await;
    let fields = vec![
        email_field("email", true),
        file_field("attachments", 2, &["image/png"]),
    ];
    let published = publish_custom(&db, "Attachment", fields, InquiryFormSettings::default()).await;
    let service = InquiryFileService::new(db);

    let now = now_timestamp();
    // Unsupported content type.
    assert!(
        service
            .create_pending(
                &published.form.handle,
                "attachments",
                "key-gif".to_owned(),
                None,
                "image/gif".to_owned(),
                10,
                "b".repeat(64),
                now + 3_600,
            )
            .await
            .is_err()
    );
    // Oversized upload.
    assert!(
        service
            .create_pending(
                &published.form.handle,
                "attachments",
                "key-big".to_owned(),
                None,
                "image/png".to_owned(),
                2 * 1_048_576,
                "c".repeat(64),
                now + 3_600,
            )
            .await
            .is_err()
    );
    // Non-file field.
    assert!(
        service
            .create_pending(
                &published.form.handle,
                "email",
                "key-field".to_owned(),
                None,
                "image/png".to_owned(),
                10,
                "d".repeat(64),
                now + 3_600,
            )
            .await
            .is_err()
    );
    // Expired upload.
    assert!(
        service
            .create_pending(
                &published.form.handle,
                "attachments",
                "key-expired".to_owned(),
                None,
                "image/png".to_owned(),
                10,
                "e".repeat(64),
                now - 1,
            )
            .await
            .is_err()
    );

    let file = service
        .create_pending(
            &published.form.handle,
            "attachments",
            "key-valid".to_owned(),
            Some("photo.png".to_owned()),
            "image/png".to_owned(),
            10,
            "f".repeat(64),
            now + 3_600,
        )
        .await
        .unwrap();
    assert!(service.expired_pending(10).await.unwrap().is_empty());

    service.mark_expired(&file.id).await.unwrap();
    let expired = service.get(&file.id).await.unwrap();
    assert_eq!(expired.status, InquiryFileStatus::Expired);

    assert!(service.mark_expired(&InquiryFileId::new()).await.is_err());
}
