//! SQLite-backed application services for inquiry forms, submissions and files.
//!
//! Forms are edited as a mutable draft revision and published by flipping
//! `published_at` on an immutable snapshot. Submissions validate against the
//! published revision, claim pending attachment uploads and append an
//! `inquiry.created` outbox event in the same transaction.

use std::collections::{BTreeMap, HashSet};

use fastcart_infrastructure::{AuditActor, StoreDatabase, append_outbox_event, database_error};
use fastcart_kernel::{
    AppError, DomainEventEnvelope, InquiryFileId, InquiryFormId, InquiryId, InquiryRevisionId,
    Metadata, PageResult, Pagination, Timestamp, now_timestamp, validate_handle, validate_length,
};
use serde_json::Value;
use sqlx::FromRow;

use crate::domain::{
    ALLOWED_FILE_TYPES, CreateInquiryFormInput, CreateInquiryInput, FieldKind, FieldValidation,
    Inquiry, InquiryFile, InquiryFileStatus, InquiryForm, InquiryFormField, InquiryFormRevision,
    InquiryFormSettings, InquiryFormStatus, InquiryListFilter, InquiryStatus, MAX_FILE_BYTES,
    MAX_FILES_PER_INQUIRY, MAX_IDEMPOTENCY_KEY_LENGTH, MAX_LOCALE_LENGTH, MAX_NAME_LENGTH,
    MAX_PAGE_URL_LENGTH, MAX_SOURCE_LENGTH, MAX_USER_AGENT_LENGTH, PublishedInquiryForm,
    SaveInquiryDraftInput, UpdateInquiryFormInput, UpdateInquiryStatusInput, default_handle,
    validate_fields, validate_submission,
};

const EVENT_MESSAGE_LIMIT: usize = 2_000;
const DEFAULT_SOURCE: &str = "contact_form";

const FORM_COLUMNS: &str = "id, name, handle, description, status, current_version, \
     published_version, created_at, updated_at";

const REVISION_COLUMNS: &str =
    "id, form_id, version, fields, settings, created_by, created_at, published_at";

const INQUIRY_COLUMNS: &str = "id, event_id, form_id, form_revision_id, form_version, status, \
     name, email, phone, subject, message, data, source, page_url, locale, idempotency_key, \
     metadata, created_at, updated_at";

const FILE_COLUMNS: &str = "id, form_id, inquiry_id, field_key, storage_key, original_name, \
     content_type, size_bytes, checksum_sha256, status, expires_at, created_at, claimed_at";

enum InquiryBind {
    Text(String),
    Integer(i64),
}

macro_rules! apply_inquiry_binds {
    ($query:expr, $binds:expr) => {{
        let mut query = $query;
        for bind in $binds {
            query = match bind {
                InquiryBind::Text(value) => query.bind(value.clone()),
                InquiryBind::Integer(value) => query.bind(*value),
            };
        }
        query
    }};
}

mod files;
mod forms;
mod inquiries;

/// Application service for inquiry form identities and revisions.
#[derive(Clone, Debug)]
pub struct InquiryFormService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

/// Application service for accepting inquiry submissions.
#[derive(Clone, Debug)]
pub struct InquiryService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

/// Application service for private attachment upload metadata.
#[derive(Clone, Debug)]
pub struct InquiryFileService {
    db: StoreDatabase,
}

fn default_form_fields() -> Vec<InquiryFormField> {
    vec![
        InquiryFormField {
            key: "name".to_owned(),
            kind: FieldKind::Text,
            label: "Name".to_owned(),
            required: true,
            placeholder: None,
            help: None,
            default: None,
            options: Vec::new(),
            validation: FieldValidation::default(),
            file: None,
        },
        InquiryFormField {
            key: "email".to_owned(),
            kind: FieldKind::Email,
            label: "Email".to_owned(),
            required: true,
            placeholder: None,
            help: None,
            default: None,
            options: Vec::new(),
            validation: FieldValidation::default(),
            file: None,
        },
        InquiryFormField {
            key: "message".to_owned(),
            kind: FieldKind::Textarea,
            label: "Message".to_owned(),
            required: true,
            placeholder: None,
            help: None,
            default: None,
            options: Vec::new(),
            validation: FieldValidation::default(),
            file: None,
        },
    ]
}

async fn claim_attachments(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    inquiry_id: &InquiryId,
    form_id: &InquiryFormId,
    file_ids: &[InquiryFileId],
    now: Timestamp,
) -> Result<(), AppError> {
    let placeholders = vec!["?"; file_ids.len()].join(", ");
    let sql = format!(
        "UPDATE inquiry_files SET inquiry_id = ?, status = 'claimed', claimed_at = ? \
         WHERE id IN ({placeholders}) AND status = 'pending' AND form_id = ? AND expires_at > ?"
    );
    let mut query = sqlx::query(&sql).bind(inquiry_id.as_str()).bind(now);
    for file_id in file_ids {
        query = query.bind(file_id.as_str());
    }
    let result = query
        .bind(form_id.as_str())
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    let expected = u64::try_from(file_ids.len()).unwrap_or(u64::MAX);
    if result.rows_affected() != expected {
        return Err(AppError::conflict(
            "one or more attachments expired or were already claimed",
        ));
    }
    Ok(())
}

async fn load_file_row(db: &StoreDatabase, id: &InquiryFileId) -> Result<InquiryFileRow, AppError> {
    let sql = format!("SELECT {FILE_COLUMNS} FROM inquiry_files WHERE id = ?");
    sqlx::query_as::<_, InquiryFileRow>(&sql)
        .bind(id.as_str())
        .fetch_optional(db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("inquiry file", id))
}

fn inquiry_conditions(filter: &InquiryListFilter) -> (String, Vec<InquiryBind>) {
    let mut conditions = String::from(" WHERE 1 = 1");
    let mut binds = Vec::new();

    if let Some(status) = filter.status {
        conditions.push_str(" AND status = ?");
        binds.push(InquiryBind::Text(status.as_str().to_owned()));
    }
    if let Some(form_id) = filter.form_id.as_ref() {
        conditions.push_str(" AND form_id = ?");
        binds.push(InquiryBind::Text(form_id.as_str().to_owned()));
    }
    if let Some(pattern) = like_pattern(filter.query.as_deref()) {
        conditions.push_str(
            " AND (LOWER(name) LIKE LOWER(?) ESCAPE '\\' \
               OR LOWER(email) LIKE LOWER(?) ESCAPE '\\' \
               OR LOWER(phone) LIKE LOWER(?) ESCAPE '\\' \
               OR LOWER(message) LIKE LOWER(?) ESCAPE '\\')",
        );
        for _ in 0..4 {
            binds.push(InquiryBind::Text(pattern.clone()));
        }
    }
    if let Some(from) = filter.from {
        conditions.push_str(" AND created_at >= ?");
        binds.push(InquiryBind::Integer(from));
    }
    if let Some(to) = filter.to {
        conditions.push_str(" AND created_at <= ?");
        binds.push(InquiryBind::Integer(to));
    }
    (conditions, binds)
}

/// Builds a case-insensitive `LIKE` pattern, escaping wildcards.
fn like_pattern(query: Option<&str>) -> Option<String> {
    let query = query?.trim();
    if query.is_empty() {
        return None;
    }
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for character in query.chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    Some(pattern)
}

fn normalize_optional_text(
    value: Option<String>,
    maximum: usize,
    field: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > maximum {
        return Err(AppError::validation(format!(
            "{field} must not exceed {maximum} characters"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(AppError::validation(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(Some(value.to_owned()))
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

/// Flattens the form's selected notification fields into label/value pairs.
fn notification_custom_fields(revision: &InquiryFormRevision, data: &Value) -> Vec<Value> {
    let Some(object) = data.as_object() else {
        return Vec::new();
    };
    revision
        .settings
        .notification_fields
        .iter()
        .filter_map(|key| {
            let field = revision.fields.iter().find(|field| &field.key == key)?;
            let display = display_value(object.get(key)?);
            if display.trim().is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "key": key,
                "label": field.label,
                "value": display,
            }))
        })
        .collect()
}

fn display_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Array(items) => items
            .iter()
            .map(display_value)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn serialize_json<T: serde::Serialize>(value: &T, field: &str) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::internal(format!("could not serialize {field}: {error}")))
}

fn parse_json_value(value: &str, field: &str) -> Result<Value, AppError> {
    serde_json::from_str(value)
        .map_err(|error| AppError::internal(format!("invalid {field} JSON: {error}")))
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(database) => {
            database.is_unique_violation()
                || database
                    .message()
                    .to_ascii_lowercase()
                    .contains("unique constraint")
        }
        _ => false,
    }
}

fn unique_conflict(error: sqlx::Error, message: &str) -> AppError {
    if is_unique_violation(&error) {
        AppError::conflict(message)
    } else {
        database_error(error)
    }
}

#[derive(Debug, FromRow)]
struct InquiryFormRow {
    id: String,
    name: String,
    handle: String,
    description: Option<String>,
    status: String,
    current_version: i64,
    published_version: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

impl InquiryFormRow {
    fn into_form(self) -> Result<InquiryForm, AppError> {
        let status = InquiryFormStatus::parse(&self.status).ok_or_else(|| {
            AppError::internal(format!("invalid inquiry form status: {}", self.status))
        })?;
        Ok(InquiryForm {
            id: InquiryFormId::from_string(self.id),
            name: self.name,
            handle: self.handle,
            description: self.description,
            status,
            current_version: self.current_version,
            published_version: self.published_version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct InquiryRevisionRow {
    id: String,
    form_id: String,
    version: i64,
    fields: String,
    settings: String,
    created_by: Option<String>,
    created_at: i64,
    published_at: Option<i64>,
}

impl InquiryRevisionRow {
    fn into_revision(self) -> Result<InquiryFormRevision, AppError> {
        let fields: Vec<InquiryFormField> = serde_json::from_str(&self.fields)
            .map_err(|error| AppError::internal(format!("invalid inquiry form fields: {error}")))?;
        let settings: InquiryFormSettings =
            serde_json::from_str(&self.settings).map_err(|error| {
                AppError::internal(format!("invalid inquiry form settings: {error}"))
            })?;
        Ok(InquiryFormRevision {
            id: InquiryRevisionId::from_string(self.id),
            form_id: InquiryFormId::from_string(self.form_id),
            version: self.version,
            fields,
            settings,
            created_by: self.created_by,
            created_at: self.created_at,
            published_at: self.published_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct PublishedFormRow {
    form_id: String,
    form_name: String,
    form_handle: String,
    form_description: Option<String>,
    form_status: String,
    form_current_version: i64,
    form_published_version: Option<i64>,
    form_created_at: i64,
    form_updated_at: i64,
    revision_id: String,
    revision_form_id: String,
    revision_version: i64,
    revision_fields: String,
    revision_settings: String,
    revision_created_by: Option<String>,
    revision_created_at: i64,
    revision_published_at: Option<i64>,
}

impl PublishedFormRow {
    fn into_published(self) -> Result<PublishedInquiryForm, AppError> {
        let form = InquiryFormRow {
            id: self.form_id,
            name: self.form_name,
            handle: self.form_handle,
            description: self.form_description,
            status: self.form_status,
            current_version: self.form_current_version,
            published_version: self.form_published_version,
            created_at: self.form_created_at,
            updated_at: self.form_updated_at,
        }
        .into_form()?;
        let revision = InquiryRevisionRow {
            id: self.revision_id,
            form_id: self.revision_form_id,
            version: self.revision_version,
            fields: self.revision_fields,
            settings: self.revision_settings,
            created_by: self.revision_created_by,
            created_at: self.revision_created_at,
            published_at: self.revision_published_at,
        }
        .into_revision()?;
        Ok(PublishedInquiryForm { form, revision })
    }
}

#[derive(Debug, FromRow)]
struct InquiryRow {
    id: String,
    event_id: String,
    form_id: String,
    form_revision_id: String,
    form_version: i64,
    status: String,
    name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    subject: Option<String>,
    message: Option<String>,
    data: String,
    source: String,
    page_url: Option<String>,
    locale: Option<String>,
    idempotency_key: Option<String>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl InquiryRow {
    fn into_inquiry(self) -> Result<Inquiry, AppError> {
        let status = InquiryStatus::parse(&self.status).ok_or_else(|| {
            AppError::internal(format!("invalid inquiry status: {}", self.status))
        })?;
        let data = parse_json_value(&self.data, "inquiry data")?;
        if !data.is_object() {
            return Err(AppError::internal(
                "inquiry data must be a JSON object".to_owned(),
            ));
        }
        Ok(Inquiry {
            id: InquiryId::from_string(self.id),
            event_id: self.event_id,
            form_id: InquiryFormId::from_string(self.form_id),
            form_revision_id: InquiryRevisionId::from_string(self.form_revision_id),
            form_version: self.form_version,
            status,
            name: self.name,
            email: self.email,
            phone: self.phone,
            subject: self.subject,
            message: self.message,
            data,
            source: self.source,
            page_url: self.page_url,
            locale: self.locale,
            idempotency_key: self.idempotency_key,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct InquiryFileRow {
    id: String,
    form_id: String,
    inquiry_id: Option<String>,
    field_key: String,
    storage_key: String,
    original_name: Option<String>,
    content_type: String,
    size_bytes: i64,
    checksum_sha256: String,
    status: String,
    expires_at: Option<i64>,
    created_at: i64,
    claimed_at: Option<i64>,
}

impl InquiryFileRow {
    fn into_file(self) -> Result<InquiryFile, AppError> {
        let status = InquiryFileStatus::parse(&self.status).ok_or_else(|| {
            AppError::internal(format!("invalid inquiry file status: {}", self.status))
        })?;
        Ok(InquiryFile {
            id: InquiryFileId::from_string(self.id),
            form_id: InquiryFormId::from_string(self.form_id),
            inquiry_id: self.inquiry_id.map(InquiryId::from_string),
            field_key: self.field_key,
            storage_key: self.storage_key,
            original_name: self.original_name,
            content_type: self.content_type,
            size_bytes: self.size_bytes,
            checksum_sha256: self.checksum_sha256,
            status,
            expires_at: self.expires_at,
            created_at: self.created_at,
            claimed_at: self.claimed_at,
        })
    }
}
