//! Inquiry-module tests.

use std::collections::BTreeMap;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, InquiryFileId, InquiryId, Pagination, StoreId, now_timestamp};
use serde_json::{Value, json};

use crate::{
    CreateInquiryFormInput, CreateInquiryInput, FieldKind, FieldValidation, FileFieldConfig,
    InquiryFileService, InquiryFileStatus, InquiryFormField, InquiryFormService,
    InquiryFormSettings, InquiryFormStatus, InquiryListFilter, InquiryService, InquiryStatus,
    PublishedInquiryForm, SaveInquiryDraftInput, UpdateInquiryFormInput, UpdateInquiryStatusInput,
};

async fn test_db() -> StoreDatabase {
    StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("in-memory store database")
}

fn form_input(name: &str) -> CreateInquiryFormInput {
    CreateInquiryFormInput {
        name: name.to_owned(),
        handle: None,
        description: None,
    }
}

fn text_field(key: &str, required: bool) -> InquiryFormField {
    InquiryFormField {
        key: key.to_owned(),
        kind: FieldKind::Text,
        label: key.to_owned(),
        required,
        placeholder: None,
        help: None,
        default: None,
        options: Vec::new(),
        validation: FieldValidation::default(),
        file: None,
    }
}

fn email_field(key: &str, required: bool) -> InquiryFormField {
    InquiryFormField {
        key: key.to_owned(),
        kind: FieldKind::Email,
        label: key.to_owned(),
        required,
        placeholder: None,
        help: None,
        default: None,
        options: Vec::new(),
        validation: FieldValidation::default(),
        file: None,
    }
}

fn textarea_field(key: &str, required: bool) -> InquiryFormField {
    InquiryFormField {
        key: key.to_owned(),
        kind: FieldKind::Textarea,
        label: key.to_owned(),
        required,
        placeholder: None,
        help: None,
        default: None,
        options: Vec::new(),
        validation: FieldValidation::default(),
        file: None,
    }
}

fn file_field(key: &str, max_files: u32, accept: &[&str]) -> InquiryFormField {
    InquiryFormField {
        key: key.to_owned(),
        kind: FieldKind::File,
        label: key.to_owned(),
        required: false,
        placeholder: None,
        help: None,
        default: None,
        options: Vec::new(),
        validation: FieldValidation::default(),
        file: Some(FileFieldConfig {
            max_files,
            max_bytes: 1_048_576,
            accept: accept.iter().map(|value| (*value).to_owned()).collect(),
        }),
    }
}

fn default_fields() -> Vec<InquiryFormField> {
    vec![
        text_field("name", true),
        email_field("email", true),
        textarea_field("message", true),
    ]
}

fn inquiry_input(handle: &str) -> CreateInquiryInput {
    CreateInquiryInput {
        form_handle: handle.to_owned(),
        name: Some("Alice".to_owned()),
        email: Some("alice@example.com".to_owned()),
        phone: None,
        subject: None,
        message: Some("Hello".to_owned()),
        data: json!({}),
        files: BTreeMap::new(),
        source: None,
        page_url: None,
        locale: None,
        user_agent: None,
        ip_hash: None,
        idempotency_key: None,
        consent: false,
    }
}

async fn publish_custom(
    db: &StoreDatabase,
    name: &str,
    fields: Vec<InquiryFormField>,
    settings: InquiryFormSettings,
) -> PublishedInquiryForm {
    let service = InquiryFormService::new(db.clone());
    let form = service.create_form(form_input(name)).await.unwrap();
    service
        .save_draft(
            &form.id,
            SaveInquiryDraftInput {
                fields,
                settings,
                created_by: None,
            },
        )
        .await
        .unwrap();
    service.publish(&form.id, None).await.unwrap();
    service.published_by_handle(&form.handle).await.unwrap()
}

async fn published_default_form(db: &StoreDatabase) -> PublishedInquiryForm {
    let service = InquiryFormService::new(db.clone());
    let form = service.create_form(form_input("Contact")).await.unwrap();
    service.publish(&form.id, None).await.unwrap();
    service.published_by_handle(&form.handle).await.unwrap()
}

async fn outbox_count(db: &StoreDatabase) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM outbox WHERE event_type = 'inquiry.created'")
        .fetch_one(db.pool())
        .await
        .unwrap()
}

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
