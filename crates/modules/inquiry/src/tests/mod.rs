//! Inquiry-module tests.

use std::collections::BTreeMap;

use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
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

mod files;
mod forms;
mod inquiries;
