use super::*;

#[tokio::test]
async fn create_form_creates_draft_revision_with_defaults() {
    let db = test_db().await;
    let service = InquiryFormService::new(db);

    let form = service
        .create_form(CreateInquiryFormInput {
            name: "Contact Us".to_owned(),
            handle: None,
            description: Some("  Get in touch  ".to_owned()),
        })
        .await
        .unwrap();

    assert!(form.id.as_str().starts_with("iform_"));
    assert_eq!(form.name, "Contact Us");
    assert_eq!(form.handle, "contact-us");
    assert_eq!(form.description.as_deref(), Some("Get in touch"));
    assert_eq!(form.status, InquiryFormStatus::Draft);
    assert_eq!(form.current_version, 1);
    assert_eq!(form.published_version, None);

    let draft = service
        .get_draft(&form.id)
        .await
        .unwrap()
        .expect("initial draft");
    assert_eq!(draft.version, 1);
    assert!(draft.published_at.is_none());
    assert_eq!(
        draft
            .fields
            .iter()
            .map(|field| field.key.as_str())
            .collect::<Vec<_>>(),
        ["name", "email", "message"]
    );
    assert!(draft.fields.iter().all(|field| field.required));
    assert_eq!(draft.fields[1].kind, FieldKind::Email);
    assert_eq!(draft.fields[2].kind, FieldKind::Textarea);
    assert!(draft.settings.allow_attachments);
    draft.validate().unwrap();

    let duplicate = service
        .create_form(CreateInquiryFormInput {
            name: "Other".to_owned(),
            handle: Some(form.handle.clone()),
            description: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(duplicate, AppError::Conflict(_)));
}

#[tokio::test]
async fn save_draft_publish_and_lookup_by_handle() {
    let db = test_db().await;
    let service = InquiryFormService::new(db.clone());

    let form = service.create_form(form_input("Contact")).await.unwrap();
    let mut fields = default_fields();
    fields.push(text_field("company", false));

    // The initial draft is updated in place instead of creating a revision.
    let saved = service
        .save_draft(
            &form.id,
            SaveInquiryDraftInput {
                fields,
                settings: InquiryFormSettings::default(),
                created_by: Some("user_1".to_owned()),
            },
        )
        .await
        .unwrap();
    assert_eq!(saved.version, 1);
    assert!(saved.published_at.is_none());
    assert_eq!(saved.created_by.as_deref(), Some("user_1"));
    assert!(saved.fields.iter().any(|field| field.key == "company"));

    let published = service.publish(&form.id, None).await.unwrap();
    assert_eq!(published.status, InquiryFormStatus::Published);
    assert_eq!(published.published_version, Some(1));
    assert!(service.get_draft(&form.id).await.unwrap().is_none());

    let found = service.published_by_handle(&form.handle).await.unwrap();
    assert_eq!(found.form.id, form.id);
    assert_eq!(found.revision.version, 1);
    assert!(found.revision.published_at.is_some());
    assert!(
        found
            .revision
            .fields
            .iter()
            .any(|field| field.key == "company")
    );

    let revisions = service.list_revisions(&form.id).await.unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0].version, 1);
}

#[tokio::test]
async fn saving_an_identical_draft_skips_the_write_and_audit() {
    let db = test_db().await;
    let service = InquiryFormService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));
    let form = service.create_form(form_input("Contact")).await.unwrap();

    let first = service
        .save_draft(
            &form.id,
            SaveInquiryDraftInput {
                fields: default_fields(),
                settings: InquiryFormSettings::default(),
                created_by: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 2);

    let second = service
        .save_draft(
            &form.id,
            SaveInquiryDraftInput {
                fields: default_fields(),
                settings: InquiryFormSettings::default(),
                created_by: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(second.id, first.id);
    // The identical save is not written or audited.
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn editing_after_publish_creates_next_draft_and_keeps_published_version() {
    let db = test_db().await;
    let service = InquiryFormService::new(db);

    let form = service.create_form(form_input("Contact")).await.unwrap();
    service.publish(&form.id, None).await.unwrap();
    assert!(service.get_draft(&form.id).await.unwrap().is_none());

    let mut fields = default_fields();
    fields.push(text_field("company", true));
    let draft = service
        .save_draft(
            &form.id,
            SaveInquiryDraftInput {
                fields,
                settings: InquiryFormSettings::default(),
                created_by: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(draft.version, 2);
    assert!(draft.published_at.is_none());

    let after_edit = service.get_form(&form.id).await.unwrap();
    assert_eq!(after_edit.current_version, 2);
    assert_eq!(after_edit.published_version, Some(1));

    let published = service.published_by_handle(&form.handle).await.unwrap();
    assert_eq!(published.revision.version, 1);

    service.publish(&form.id, None).await.unwrap();
    let republished = service.published_by_handle(&form.handle).await.unwrap();
    assert_eq!(republished.revision.version, 2);

    // Publishing an explicit older revision points the form back at it.
    service.publish(&form.id, Some(1)).await.unwrap();
    let rolled_back = service.published_by_handle(&form.handle).await.unwrap();
    assert_eq!(rolled_back.revision.version, 1);
}

#[tokio::test]
async fn archive_hides_from_published_by_handle() {
    let db = test_db().await;
    let service = InquiryFormService::new(db);
    let form = service.create_form(form_input("Contact")).await.unwrap();
    service.publish(&form.id, None).await.unwrap();

    let archived = service.archive(&form.id).await.unwrap();
    assert_eq!(archived.status, InquiryFormStatus::Archived);

    let error = service.published_by_handle(&form.handle).await.unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn partial_form_update_preserves_unsupplied_fields() {
    let db = test_db().await;
    let service = InquiryFormService::new(db);
    let form = service
        .create_form(CreateInquiryFormInput {
            name: "Contact".to_owned(),
            handle: None,
            description: Some("Get in touch".to_owned()),
        })
        .await
        .unwrap();

    let updated = service
        .update_form(
            &form.id,
            UpdateInquiryFormInput {
                name: Some("Support".to_owned()),
                description: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Support");
    assert_eq!(updated.description.as_deref(), Some("Get in touch"));
}

#[tokio::test]
async fn archived_form_cannot_be_published() {
    let db = test_db().await;
    let service = InquiryFormService::new(db);
    let form = service.create_form(form_input("Contact")).await.unwrap();
    service.publish(&form.id, None).await.unwrap();
    service.archive(&form.id).await.unwrap();

    let error = service.publish(&form.id, Some(1)).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let after = service.get_form(&form.id).await.unwrap();
    assert_eq!(after.status, InquiryFormStatus::Archived);
    assert_eq!(after.published_version, Some(1));
}

#[tokio::test]
async fn update_form_changes_name_and_description_but_not_handle() {
    let db = test_db().await;
    let service = InquiryFormService::new(db);
    let form = service.create_form(form_input("Contact")).await.unwrap();

    let updated = service
        .update_form(
            &form.id,
            UpdateInquiryFormInput {
                name: Some("  Support  ".to_owned()),
                description: Some("Need help?".to_owned()),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.id, form.id);
    assert_eq!(updated.name, "Support");
    assert_eq!(updated.handle, "contact");
    assert_eq!(updated.description.as_deref(), Some("Need help?"));
}

#[tokio::test]
async fn form_mutations_append_audit_entries_in_the_same_transaction() {
    let db = test_db().await;
    let form = InquiryFormService::new(db.clone())
        .create_form(form_input("Audited"))
        .await
        .unwrap();

    let service = InquiryFormService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));
    let published = service.publish(&form.id, None).await.unwrap();
    assert_eq!(published.published_version, Some(1));

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action, "inquiry_form.published");
    assert_eq!(audits[0].actor_id.as_deref(), Some("user_1"));
    assert!(audits[0].processed_at.is_none());

    // Publishing the same revision again is a no-op and must not append.
    service.publish(&form.id, Some(1)).await.unwrap();
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 1);
}
