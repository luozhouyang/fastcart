use super::*;

#[tokio::test]
async fn create_inquiry_validates_normalizes_and_writes_outbox() {
    let db = test_db().await;
    let published = published_default_form(&db).await;
    let service = InquiryService::new(db.clone());

    let mut input = inquiry_input(&published.form.handle);
    input.name = Some("  Alice  ".to_owned());
    input.email = Some(" Alice@Example.COM ".to_owned());
    input.message = Some("  Hello!  ".to_owned());

    let inquiry = service.create(input).await.unwrap();
    assert!(inquiry.id.as_str().starts_with("inq_"));
    assert_eq!(inquiry.name.as_deref(), Some("Alice"));
    assert_eq!(inquiry.email.as_deref(), Some("alice@example.com"));
    assert_eq!(inquiry.message.as_deref(), Some("Hello!"));
    assert_eq!(inquiry.status, InquiryStatus::New);
    assert_eq!(inquiry.source, "contact_form");
    assert_eq!(inquiry.form_id, published.form.id);
    assert_eq!(inquiry.form_revision_id, published.revision.id);
    assert_eq!(inquiry.form_version, 1);
    assert_eq!(inquiry.data, json!({}));
    assert!(inquiry.metadata.is_empty());

    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, payload FROM outbox WHERE event_type = 'inquiry.created'")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, inquiry.event_id);

    let payload: Value = serde_json::from_str(&rows[0].1).unwrap();
    assert_eq!(payload["event_id"], inquiry.event_id);
    assert_eq!(payload["inquiry_id"], inquiry.id.as_str());
    assert_eq!(payload["form_id"], published.form.id.as_str());
    assert_eq!(payload["form_handle"], "contact");
    assert_eq!(payload["form_version"], 1);
    assert_eq!(payload["name"], "Alice");
    assert_eq!(payload["email"], "alice@example.com");
    assert_eq!(payload["message"], "Hello!");
    assert_eq!(payload["attachment_count"], 0);
}

#[tokio::test]
async fn duplicate_idempotency_key_returns_same_inquiry_without_second_outbox_row() {
    let db = test_db().await;
    let published = published_default_form(&db).await;
    let service = InquiryService::new(db.clone());

    let mut input = inquiry_input(&published.form.handle);
    input.idempotency_key = Some("request-key-1".to_owned());

    let first = service.create(input.clone()).await.unwrap();
    let second = service.create(input).await.unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(first.event_id, second.event_id);

    let inquiries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM inquiries")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(inquiries, 1);
    assert_eq!(outbox_count(&db).await, 1);
}

#[tokio::test]
async fn required_custom_field_missing_fails() {
    let db = test_db().await;
    let fields = vec![
        text_field("name", false),
        email_field("email", true),
        textarea_field("message", false),
        text_field("company", true),
    ];
    let published = publish_custom(&db, "Lead", fields, InquiryFormSettings::default()).await;
    let service = InquiryService::new(db);

    let mut input = inquiry_input(&published.form.handle);
    input.message = None;
    input.data = json!({});

    let error = service.create(input).await.unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    assert!(error.to_string().contains("company"));
}

#[tokio::test]
async fn unknown_custom_field_fails() {
    let db = test_db().await;
    let published = published_default_form(&db).await;
    let service = InquiryService::new(db);

    let mut input = inquiry_input(&published.form.handle);
    input.data = json!({"unknown": "value"});

    let error = service.create(input).await.unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    assert!(error.to_string().contains("unknown field"));
}

#[tokio::test]
async fn unread_count_and_list_filters() {
    let db = test_db().await;
    let published = published_default_form(&db).await;
    let service = InquiryService::new(db.clone());

    let mut alice_input = inquiry_input(&published.form.handle);
    alice_input.name = Some("Alice".to_owned());
    alice_input.email = Some("alice@example.com".to_owned());
    alice_input.message = Some("Order question".to_owned());
    let alice = service.create(alice_input).await.unwrap();

    let mut bob_input = inquiry_input(&published.form.handle);
    bob_input.name = Some("Bob".to_owned());
    bob_input.email = Some("bob@example.com".to_owned());
    bob_input.message = Some("Shipping question".to_owned());
    let bob = service.create(bob_input).await.unwrap();

    let mut carol_input = inquiry_input(&published.form.handle);
    carol_input.name = Some("Carol".to_owned());
    carol_input.email = Some("carol@example.com".to_owned());
    carol_input.message = Some("Return question".to_owned());
    let carol = service.create(carol_input).await.unwrap();

    service
        .update_status(
            &bob.id,
            UpdateInquiryStatusInput {
                status: InquiryStatus::Read,
            },
        )
        .await
        .unwrap();

    assert_eq!(service.unread_count().await.unwrap(), 2);

    let all = service
        .list(InquiryListFilter::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(all.count, 3);
    assert_eq!(all.data.len(), 3);
    assert_eq!(all.data[0].id, carol.id);

    let unread = service
        .list(
            InquiryListFilter {
                status: Some(InquiryStatus::New),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(unread.count, 2);
    assert!(
        unread
            .data
            .iter()
            .all(|inquiry| inquiry.status == InquiryStatus::New)
    );
    assert!(unread.data.iter().any(|inquiry| inquiry.id == alice.id));

    let read = service
        .list(
            InquiryListFilter {
                status: Some(InquiryStatus::Read),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(read.count, 1);
    assert_eq!(read.data[0].id, bob.id);

    let by_query = service
        .list(
            InquiryListFilter {
                query: Some("alice@".to_owned()),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_query.count, 1);
    assert_eq!(by_query.data[0].id, alice.id);

    let by_form = service
        .list(
            InquiryListFilter {
                form_id: Some(published.form.id.clone()),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_form.count, 3);

    let bounded = service
        .list(
            InquiryListFilter {
                from: Some(0),
                to: Some(now_timestamp() + 60),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(bounded.count, 3);

    let empty_range = service
        .list(
            InquiryListFilter {
                from: Some(0),
                to: Some(0),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(empty_range.count, 0);

    let page = service
        .list(
            InquiryListFilter::default(),
            Pagination {
                limit: 1,
                offset: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.count, 3);

    let loaded = service.get(&alice.id).await.unwrap();
    assert_eq!(loaded.id, alice.id);
    assert!(service.get(&InquiryId::new()).await.is_err());
}

#[tokio::test]
async fn inquiries_require_a_published_form() {
    let db = test_db().await;
    let service = InquiryFormService::new(db.clone());
    let form = service.create_form(form_input("Contact")).await.unwrap();

    let error = InquiryService::new(db)
        .create(inquiry_input(&form.handle))
        .await
        .unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn retention_scan_and_delete_remove_old_inquiries() {
    let db = test_db().await;
    let published = published_default_form(&db).await;
    let service = InquiryService::new(db.clone());
    let inquiry = service
        .create(inquiry_input(&published.form.handle))
        .await
        .unwrap();

    // Keep a second inquiry inside the retention window.
    let recent = service
        .create(inquiry_input(&published.form.handle))
        .await
        .unwrap();

    sqlx::query("UPDATE inquiries SET created_at = 0 WHERE id = ?")
        .bind(inquiry.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

    let cutoff = now_timestamp() - 86_400;
    let expired = service.older_than(cutoff, 100).await.unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].id, inquiry.id);

    service.delete(&inquiry.id).await.unwrap();
    assert!(service.get(&inquiry.id).await.is_err());
    assert_eq!(service.get(&recent.id).await.unwrap().id, recent.id);
}

#[tokio::test]
async fn status_changes_append_audit_entries_and_skip_noops() {
    let db = test_db().await;
    let published = published_default_form(&db).await;
    let inquiry = InquiryService::new(db.clone())
        .create(inquiry_input(&published.form.handle))
        .await
        .unwrap();

    let service = InquiryService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));
    service
        .update_status(
            &inquiry.id,
            UpdateInquiryStatusInput {
                status: InquiryStatus::Read,
            },
        )
        .await
        .unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action, "inquiry.status_changed");
    assert_eq!(audits[0].actor_id.as_deref(), Some("user_1"));
    assert!(audits[0].processed_at.is_none());

    // Repeating the same status is a no-op and must not append.
    service
        .update_status(
            &inquiry.id,
            UpdateInquiryStatusInput {
                status: InquiryStatus::Read,
            },
        )
        .await
        .unwrap();
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 1);
}
