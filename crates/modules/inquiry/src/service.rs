//! SQLite-backed application services for inquiry forms, submissions and files.
//!
//! Forms are edited as a mutable draft revision and published by flipping
//! `published_at` on an immutable snapshot. Submissions validate against the
//! published revision, claim pending attachment uploads and append an
//! `inquiry.created` outbox event in the same transaction.

use std::collections::{BTreeMap, HashSet};

use fastcart_infrastructure::{StoreDatabase, append_outbox_event, database_error};
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

/// Application service for inquiry form identities and revisions.
#[derive(Clone, Debug)]
pub struct InquiryFormService {
    db: StoreDatabase,
}

impl InquiryFormService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    /// Creates a form with an initial draft revision containing sensible
    /// defaults. The handle defaults to a slug of the name.
    pub async fn create_form(
        &self,
        input: CreateInquiryFormInput,
    ) -> Result<InquiryForm, AppError> {
        let CreateInquiryFormInput {
            name,
            handle,
            description,
        } = input;

        validate_length(&name, "name", MAX_NAME_LENGTH)?;
        let name = name.trim().to_owned();
        let handle = match handle {
            Some(handle) => {
                let handle = handle.trim().to_owned();
                validate_handle(&handle, "handle")?;
                handle
            }
            None => {
                let mut handle = default_handle(&name);
                if handle.is_empty() {
                    handle = format!("form-{}", &uuid::Uuid::now_v7().simple().to_string()[..8]);
                }
                validate_handle(&handle, "handle")?;
                handle
            }
        };
        let description = description
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let existing: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM inquiry_forms WHERE handle = ?")
                .bind(&handle)
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?;
        if existing > 0 {
            return Err(AppError::conflict(format!(
                "inquiry form handle already exists: {handle}"
            )));
        }

        let fields = default_form_fields();
        let settings = InquiryFormSettings::default();
        validate_fields(&fields)?;
        settings.validate(&fields)?;
        let fields_json = serialize_json(&fields, "fields")?;
        let settings_json = serialize_json(&settings, "settings")?;

        let id = InquiryFormId::new();
        let revision_id = InquiryRevisionId::new();
        let now = now_timestamp();

        let mut tx = self.db.begin().await?;
        sqlx::query(
            "INSERT INTO inquiry_forms \
             (id, name, handle, description, status, current_version, published_version, \
              created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'draft', 1, NULL, ?, ?)",
        )
        .bind(id.as_str())
        .bind(&name)
        .bind(&handle)
        .bind(&description)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_conflict(error, "inquiry form handle already exists"))?;

        sqlx::query(
            "INSERT INTO inquiry_form_revisions \
             (id, form_id, version, fields, settings, created_by, created_at, published_at) \
             VALUES (?, ?, 1, ?, ?, NULL, ?, NULL)",
        )
        .bind(revision_id.as_str())
        .bind(id.as_str())
        .bind(&fields_json)
        .bind(&settings_json)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        tx.commit().await.map_err(database_error)?;
        self.get_form(&id).await
    }

    /// Lists forms newest first.
    pub async fn list_forms(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<InquiryForm>, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM inquiry_forms")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let sql = format!(
            "SELECT {FORM_COLUMNS} FROM inquiry_forms ORDER BY created_at DESC, id DESC \
             LIMIT ? OFFSET ?"
        );
        let rows = sqlx::query_as::<_, InquiryFormRow>(&sql)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let data = rows
            .into_iter()
            .map(InquiryFormRow::into_form)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PageResult::new(data, count, pagination))
    }

    /// Loads a form by identifier.
    pub async fn get_form(&self, id: &InquiryFormId) -> Result<InquiryForm, AppError> {
        let sql = format!("SELECT {FORM_COLUMNS} FROM inquiry_forms WHERE id = ?");
        sqlx::query_as::<_, InquiryFormRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("inquiry form", id))?
            .into_form()
    }

    /// Updates the form name and description. Handles are immutable.
    pub async fn update_form(
        &self,
        id: &InquiryFormId,
        input: UpdateInquiryFormInput,
    ) -> Result<InquiryForm, AppError> {
        let existing = self.get_form(id).await?;
        let name = match input.name {
            Some(name) => {
                validate_length(&name, "name", MAX_NAME_LENGTH)?;
                name.trim().to_owned()
            }
            None => existing.name,
        };
        let description = match input.description {
            Some(description) => {
                let description = description.trim();
                if description.is_empty() {
                    None
                } else {
                    Some(description.to_owned())
                }
            }
            None => existing.description,
        };

        let result = sqlx::query(
            "UPDATE inquiry_forms SET name = ?, description = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&name)
        .bind(&description)
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("inquiry form", id));
        }
        self.get_form(id).await
    }

    /// Saves the current draft, updating it in place or creating the next
    /// revision when the previous draft was already published.
    pub async fn save_draft(
        &self,
        id: &InquiryFormId,
        input: SaveInquiryDraftInput,
    ) -> Result<InquiryFormRevision, AppError> {
        let SaveInquiryDraftInput {
            fields,
            settings,
            created_by,
        } = input;

        validate_fields(&fields)?;
        settings.validate(&fields)?;
        // Ensures the form exists before opening a write transaction.
        self.get_form(id).await?;

        let fields_json = serialize_json(&fields, "fields")?;
        let settings_json = serialize_json(&settings, "settings")?;
        let now = now_timestamp();

        let mut tx = self.db.begin().await?;
        let draft = sqlx::query_as::<_, InquiryRevisionRow>(&format!(
            "SELECT {REVISION_COLUMNS} FROM inquiry_form_revisions \
             WHERE form_id = ? AND published_at IS NULL"
        ))
        .bind(id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;

        if let Some(draft) = draft {
            sqlx::query(
                "UPDATE inquiry_form_revisions \
                 SET fields = ?, settings = ?, created_by = ?, created_at = ? WHERE id = ?",
            )
            .bind(&fields_json)
            .bind(&settings_json)
            .bind(created_by.as_deref())
            .bind(now)
            .bind(&draft.id)
            .execute(&mut *tx)
            .await
            .map_err(|error| unique_conflict(error, "inquiry form draft changed concurrently"))?;
            tx.commit().await.map_err(database_error)?;

            return Ok(InquiryFormRevision {
                id: InquiryRevisionId::from_string(draft.id),
                form_id: id.clone(),
                version: draft.version,
                fields,
                settings,
                created_by,
                created_at: now,
                published_at: None,
            });
        }

        let current_version: i64 =
            sqlx::query_scalar("SELECT current_version FROM inquiry_forms WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?
                .ok_or_else(|| AppError::not_found("inquiry form", id))?;
        let version = current_version + 1;
        let revision_id = InquiryRevisionId::new();

        sqlx::query(
            "INSERT INTO inquiry_form_revisions \
             (id, form_id, version, fields, settings, created_by, created_at, published_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(revision_id.as_str())
        .bind(id.as_str())
        .bind(version)
        .bind(&fields_json)
        .bind(&settings_json)
        .bind(created_by.as_deref())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_conflict(error, "inquiry form draft changed concurrently"))?;

        sqlx::query("UPDATE inquiry_forms SET current_version = ?, updated_at = ? WHERE id = ?")
            .bind(version)
            .bind(now)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;

        tx.commit().await.map_err(database_error)?;

        Ok(InquiryFormRevision {
            id: revision_id,
            form_id: id.clone(),
            version,
            fields,
            settings,
            created_by,
            created_at: now,
            published_at: None,
        })
    }

    /// Loads the current editable draft revision, if one exists.
    pub async fn get_draft(
        &self,
        id: &InquiryFormId,
    ) -> Result<Option<InquiryFormRevision>, AppError> {
        let sql = format!(
            "SELECT {REVISION_COLUMNS} FROM inquiry_form_revisions \
             WHERE form_id = ? AND published_at IS NULL"
        );
        sqlx::query_as::<_, InquiryRevisionRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .map(InquiryRevisionRow::into_revision)
            .transpose()
    }

    /// Loads one revision by version.
    pub async fn get_revision(
        &self,
        id: &InquiryFormId,
        version: i64,
    ) -> Result<InquiryFormRevision, AppError> {
        let sql = format!(
            "SELECT {REVISION_COLUMNS} FROM inquiry_form_revisions \
             WHERE form_id = ? AND version = ?"
        );
        sqlx::query_as::<_, InquiryRevisionRow>(&sql)
            .bind(id.as_str())
            .bind(version)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("inquiry revision", format!("{id}@{version}")))?
            .into_revision()
    }

    /// Lists all revisions newest first.
    pub async fn list_revisions(
        &self,
        id: &InquiryFormId,
    ) -> Result<Vec<InquiryFormRevision>, AppError> {
        let sql = format!(
            "SELECT {REVISION_COLUMNS} FROM inquiry_form_revisions \
             WHERE form_id = ? ORDER BY version DESC"
        );
        let rows = sqlx::query_as::<_, InquiryRevisionRow>(&sql)
            .bind(id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        rows.into_iter()
            .map(InquiryRevisionRow::into_revision)
            .collect()
    }

    /// Publishes a revision. When no version is supplied the current draft is
    /// published.
    pub async fn publish(
        &self,
        id: &InquiryFormId,
        version: Option<i64>,
    ) -> Result<InquiryForm, AppError> {
        let revision = match version {
            Some(version) => self.get_revision(id, version).await?,
            None => self.get_draft(id).await?.ok_or_else(|| {
                AppError::conflict("inquiry form has no draft revision to publish")
            })?,
        };
        revision.validate()?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE inquiry_form_revisions SET published_at = ? \
             WHERE id = ? AND published_at IS NULL",
        )
        .bind(now)
        .bind(revision.id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        let updated = sqlx::query(
            "UPDATE inquiry_forms SET published_version = ?, status = 'published', \
             updated_at = ? WHERE id = ?",
        )
        .bind(revision.version)
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::not_found("inquiry form", id));
        }

        tx.commit().await.map_err(database_error)?;
        self.get_form(id).await
    }

    /// Archives a form, removing it from storefront lookups.
    pub async fn archive(&self, id: &InquiryFormId) -> Result<InquiryForm, AppError> {
        let result = sqlx::query(
            "UPDATE inquiry_forms SET status = 'archived', updated_at = ? WHERE id = ?",
        )
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("inquiry form", id));
        }
        self.get_form(id).await
    }

    /// Loads a published form and the revision storefronts should render.
    pub async fn published_by_handle(
        &self,
        handle: &str,
    ) -> Result<PublishedInquiryForm, AppError> {
        let handle = handle.trim();
        sqlx::query_as::<_, PublishedFormRow>(
            "SELECT f.id AS form_id, f.name AS form_name, f.handle AS form_handle, \
                    f.description AS form_description, f.status AS form_status, \
                    f.current_version AS form_current_version, \
                    f.published_version AS form_published_version, \
                    f.created_at AS form_created_at, f.updated_at AS form_updated_at, \
                    r.id AS revision_id, r.form_id AS revision_form_id, \
                    r.version AS revision_version, r.fields AS revision_fields, \
                    r.settings AS revision_settings, r.created_by AS revision_created_by, \
                    r.created_at AS revision_created_at, \
                    r.published_at AS revision_published_at \
             FROM inquiry_forms f \
             JOIN inquiry_form_revisions r \
               ON r.form_id = f.id AND r.version = f.published_version \
             WHERE f.handle = ? AND f.status = 'published'",
        )
        .bind(handle)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("inquiry form", handle))?
        .into_published()
    }
}

/// Application service for accepting inquiry submissions.
#[derive(Clone, Debug)]
pub struct InquiryService {
    db: StoreDatabase,
}

impl InquiryService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    /// Accepts a submission against the currently published form revision.
    pub async fn create(&self, input: CreateInquiryInput) -> Result<Inquiry, AppError> {
        let CreateInquiryInput {
            form_handle,
            name,
            email,
            phone,
            subject,
            message,
            data,
            files,
            source,
            page_url,
            locale,
            user_agent,
            ip_hash,
            idempotency_key,
            consent,
        } = input;

        let published = InquiryFormService::new(self.db.clone())
            .published_by_handle(&form_handle)
            .await?;
        let form = published.form;
        let revision = published.revision;

        let normalized = validate_submission(
            &revision,
            name.as_deref(),
            email.as_deref(),
            phone.as_deref(),
            subject.as_deref(),
            message.as_deref(),
            &data,
            consent,
        )?;

        let source = normalize_optional_text(source, MAX_SOURCE_LENGTH, "source")?
            .unwrap_or_else(|| DEFAULT_SOURCE.to_owned());
        let page_url = normalize_optional_text(page_url, MAX_PAGE_URL_LENGTH, "page_url")?;
        let locale = normalize_optional_text(locale, MAX_LOCALE_LENGTH, "locale")?;
        let user_agent = normalize_optional_text(user_agent, MAX_USER_AGENT_LENGTH, "user_agent")?;
        let idempotency_key = normalize_optional_text(
            idempotency_key,
            MAX_IDEMPOTENCY_KEY_LENGTH,
            "idempotency_key",
        )?;
        let ip_hash = ip_hash
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        if let Some(key) = idempotency_key.as_deref()
            && let Some(existing) = self.find_by_idempotency(&form.id, key).await?
        {
            return Ok(existing);
        }

        let file_ids = self.validate_attachments(&form, &revision, &files).await?;

        let inquiry_id = InquiryId::new();
        let event_message = normalized
            .message
            .as_deref()
            .map(|value| truncate_chars(value, EVENT_MESSAGE_LIMIT));
        let payload = serde_json::json!({
            "inquiry_id": inquiry_id.as_str(),
            "form_id": form.id.as_str(),
            "form_handle": form.handle.as_str(),
            "form_version": revision.version,
            "name": normalized.name.clone(),
            "email": normalized.email.clone(),
            "phone": normalized.phone.clone(),
            "subject": normalized.subject.clone(),
            "message": event_message,
            "page_url": page_url.clone(),
            "attachment_count": file_ids.len(),
            "custom_fields": notification_custom_fields(&revision, &normalized.data),
        });
        let mut event =
            DomainEventEnvelope::new("inquiry.created", Some(self.db.store_id().clone()), payload);
        // The event id is generated by the envelope; include it in the payload
        // so consumers can correlate the outbox row without parsing headers.
        event.data["event_id"] = Value::String(event.id.as_str().to_owned());

        let data_json = serialize_json(&normalized.data, "data")?;
        let metadata_json = Metadata::empty().to_json_string();
        let now = now_timestamp();

        let mut tx = self.db.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO inquiries \
             (id, event_id, form_id, form_revision_id, form_version, status, name, email, \
              phone, subject, message, data, source, page_url, locale, user_agent, ip_hash, \
              idempotency_key, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, 'new', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(inquiry_id.as_str())
        .bind(event.id.as_str())
        .bind(form.id.as_str())
        .bind(revision.id.as_str())
        .bind(revision.version)
        .bind(normalized.name.as_deref())
        .bind(normalized.email.as_deref())
        .bind(normalized.phone.as_deref())
        .bind(normalized.subject.as_deref())
        .bind(normalized.message.as_deref())
        .bind(&data_json)
        .bind(&source)
        .bind(page_url.as_deref())
        .bind(locale.as_deref())
        .bind(user_agent.as_deref())
        .bind(ip_hash.as_deref())
        .bind(idempotency_key.as_deref())
        .bind(&metadata_json)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await;

        if let Err(error) = inserted {
            if is_unique_violation(&error) {
                tx.rollback().await.map_err(database_error)?;
                if let Some(key) = idempotency_key.as_deref()
                    && let Some(existing) = self.find_by_idempotency(&form.id, key).await?
                {
                    return Ok(existing);
                }
            }
            return Err(unique_conflict(error, "inquiry submission already exists"));
        }

        if !file_ids.is_empty() {
            claim_attachments(&mut tx, &inquiry_id, &form.id, &file_ids, now).await?;
        }

        append_outbox_event(&mut *tx, &event)
            .await
            .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;

        self.get(&inquiry_id).await
    }

    /// Lists submissions newest first with optional filters.
    pub async fn list(
        &self,
        filter: InquiryListFilter,
        pagination: Pagination,
    ) -> Result<PageResult<Inquiry>, AppError> {
        let (conditions, binds) = inquiry_conditions(&filter);

        let count_sql = format!("SELECT COUNT(*) FROM inquiries{conditions}");
        let count = apply_inquiry_binds!(sqlx::query_scalar::<_, i64>(&count_sql), &binds)
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT {INQUIRY_COLUMNS} FROM inquiries{conditions} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let rows = apply_inquiry_binds!(sqlx::query_as::<_, InquiryRow>(&list_sql), &binds)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let data = rows
            .into_iter()
            .map(InquiryRow::into_inquiry)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PageResult::new(data, count, pagination))
    }

    /// Loads a submission by identifier.
    pub async fn get(&self, id: &InquiryId) -> Result<Inquiry, AppError> {
        let sql = format!("SELECT {INQUIRY_COLUMNS} FROM inquiries WHERE id = ?");
        sqlx::query_as::<_, InquiryRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("inquiry", id))?
            .into_inquiry()
    }

    /// Changes a submission's lifecycle status.
    pub async fn update_status(
        &self,
        id: &InquiryId,
        input: UpdateInquiryStatusInput,
    ) -> Result<Inquiry, AppError> {
        let result = sqlx::query("UPDATE inquiries SET status = ?, updated_at = ? WHERE id = ?")
            .bind(input.status.as_str())
            .bind(now_timestamp())
            .bind(id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("inquiry", id));
        }
        self.get(id).await
    }

    /// Counts unread (`new`) submissions.
    pub async fn unread_count(&self) -> Result<i64, AppError> {
        sqlx::query_scalar("SELECT COUNT(*) FROM inquiries WHERE status = 'new'")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)
    }

    /// Lists the attachments claimed by a submission.
    pub async fn files_for(&self, inquiry_id: &InquiryId) -> Result<Vec<InquiryFile>, AppError> {
        let sql = format!(
            "SELECT {FILE_COLUMNS} FROM inquiry_files WHERE inquiry_id = ? \
             ORDER BY created_at ASC, id ASC"
        );
        let rows = sqlx::query_as::<_, InquiryFileRow>(&sql)
            .bind(inquiry_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        rows.into_iter().map(InquiryFileRow::into_file).collect()
    }

    /// Loads one attachment after verifying it belongs to the submission.
    pub async fn file_for_download(
        &self,
        inquiry_id: &InquiryId,
        file_id: &InquiryFileId,
    ) -> Result<InquiryFile, AppError> {
        let file = self.file_service().get(file_id).await?;
        if file.inquiry_id.as_ref() != Some(inquiry_id) {
            return Err(AppError::not_found("inquiry file", file_id));
        }
        Ok(file)
    }

    fn file_service(&self) -> InquiryFileService {
        InquiryFileService::new(self.db.clone())
    }

    async fn find_by_idempotency(
        &self,
        form_id: &InquiryFormId,
        key: &str,
    ) -> Result<Option<Inquiry>, AppError> {
        let sql = format!(
            "SELECT {INQUIRY_COLUMNS} FROM inquiries \
             WHERE form_id = ? AND idempotency_key = ?"
        );
        sqlx::query_as::<_, InquiryRow>(&sql)
            .bind(form_id.as_str())
            .bind(key)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .map(InquiryRow::into_inquiry)
            .transpose()
    }

    async fn validate_attachments(
        &self,
        form: &InquiryForm,
        revision: &InquiryFormRevision,
        files: &BTreeMap<String, Vec<InquiryFileId>>,
    ) -> Result<Vec<InquiryFileId>, AppError> {
        if files.is_empty() {
            if let Some(field) = revision
                .fields
                .iter()
                .find(|field| field.kind == FieldKind::File && field.required)
            {
                return Err(AppError::validation(format!(
                    "field {} is required",
                    field.key
                )));
            }
            return Ok(Vec::new());
        }
        if !revision.settings.allow_attachments {
            return Err(AppError::validation(
                "this inquiry form does not accept attachments",
            ));
        }

        let now = now_timestamp();
        let mut ids = Vec::new();
        let mut seen = HashSet::new();

        for (field_key, field_ids) in files {
            if field_ids.is_empty() {
                continue;
            }
            let field = revision
                .fields
                .iter()
                .find(|field| field.key == *field_key)
                .ok_or_else(|| {
                    AppError::validation(format!("unknown attachment field: {field_key}"))
                })?;
            if field.kind != FieldKind::File {
                return Err(AppError::validation(format!(
                    "field {field_key} is not a file field"
                )));
            }
            let config = field.file.as_ref().ok_or_else(|| {
                AppError::internal(format!("file field {field_key} has no configuration"))
            })?;
            if field_ids.len() > usize::try_from(config.max_files).unwrap_or(usize::MAX) {
                return Err(AppError::validation(format!(
                    "field {field_key} accepts at most {} files",
                    config.max_files
                )));
            }

            for file_id in field_ids {
                if !seen.insert(file_id.as_str().to_owned()) {
                    return Err(AppError::validation(
                        "a file may only be attached once per submission",
                    ));
                }
                let row = load_file_row(&self.db, file_id).await?;
                if row.form_id.as_str() != form.id.as_str() {
                    return Err(AppError::validation(format!(
                        "file {file_id} does not belong to this form"
                    )));
                }
                if row.field_key.as_str() != field_key.as_str() {
                    return Err(AppError::validation(format!(
                        "file {file_id} does not belong to field {field_key}"
                    )));
                }
                if row.status != InquiryFileStatus::Pending.as_str() {
                    return Err(AppError::validation(format!(
                        "file {file_id} is not pending"
                    )));
                }
                let expires_at = row
                    .expires_at
                    .ok_or_else(|| AppError::validation(format!("file {file_id} has no expiry")))?;
                if expires_at <= now {
                    return Err(AppError::validation(format!("file {file_id} has expired")));
                }
                if !config
                    .accept
                    .iter()
                    .any(|accepted| accepted == &row.content_type)
                {
                    return Err(AppError::validation(format!(
                        "field {field_key} does not accept {}",
                        row.content_type
                    )));
                }
                if u64::try_from(row.size_bytes).unwrap_or(u64::MAX) > config.max_bytes {
                    return Err(AppError::validation(format!(
                        "file {file_id} exceeds the field size limit"
                    )));
                }
                ids.push(file_id.clone());
            }
        }

        if ids.len() > MAX_FILES_PER_INQUIRY {
            return Err(AppError::validation(format!(
                "a submission may not exceed {MAX_FILES_PER_INQUIRY} attachments"
            )));
        }
        for field in &revision.fields {
            if field.kind == FieldKind::File
                && field.required
                && files.get(&field.key).is_none_or(Vec::is_empty)
            {
                return Err(AppError::validation(format!(
                    "field {} is required",
                    field.key
                )));
            }
        }
        Ok(ids)
    }
}

/// Application service for private attachment upload metadata.
#[derive(Clone, Debug)]
pub struct InquiryFileService {
    db: StoreDatabase,
}

impl InquiryFileService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    /// Records a pending upload after validating it against the published form.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_pending(
        &self,
        form_handle: &str,
        field_key: &str,
        storage_key: String,
        original_name: Option<String>,
        content_type: String,
        size_bytes: i64,
        checksum_sha256: String,
        expires_at: i64,
    ) -> Result<InquiryFile, AppError> {
        let published = InquiryFormService::new(self.db.clone())
            .published_by_handle(form_handle)
            .await?;
        let form = published.form;
        let revision = published.revision;

        if !revision.settings.allow_attachments {
            return Err(AppError::validation(
                "this inquiry form does not accept attachments",
            ));
        }
        let field = revision
            .fields
            .iter()
            .find(|field| field.key == field_key)
            .ok_or_else(|| {
                AppError::validation(format!("unknown attachment field: {field_key}"))
            })?;
        if field.kind != FieldKind::File {
            return Err(AppError::validation(format!(
                "field {field_key} is not a file field"
            )));
        }
        let config = field.file.as_ref().ok_or_else(|| {
            AppError::internal(format!("file field {field_key} has no configuration"))
        })?;

        let content_type = content_type.trim().to_owned();
        if !ALLOWED_FILE_TYPES.contains(&content_type.as_str()) {
            return Err(AppError::validation(format!(
                "unsupported attachment content type: {content_type}"
            )));
        }
        if !config
            .accept
            .iter()
            .any(|accepted| accepted == &content_type)
        {
            return Err(AppError::validation(format!(
                "field {field_key} does not accept {content_type}"
            )));
        }

        if size_bytes <= 0 {
            return Err(AppError::validation(
                "attachment size must be greater than zero",
            ));
        }
        let size = u64::try_from(size_bytes)
            .map_err(|_| AppError::validation("invalid attachment size"))?;
        let max_bytes = config.max_bytes.min(MAX_FILE_BYTES);
        if size > max_bytes {
            return Err(AppError::validation(format!(
                "attachment must not exceed {max_bytes} bytes"
            )));
        }

        let now = now_timestamp();
        if expires_at <= now {
            return Err(AppError::validation(
                "attachment expiry must be in the future",
            ));
        }

        let storage_key = storage_key.trim().to_owned();
        if storage_key.is_empty() {
            return Err(AppError::validation("storage_key must not be empty"));
        }
        let checksum_sha256 = checksum_sha256.trim().to_owned();
        if checksum_sha256.is_empty() {
            return Err(AppError::validation("checksum_sha256 must not be empty"));
        }
        let original_name = original_name
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let id = InquiryFileId::new();
        sqlx::query(
            "INSERT INTO inquiry_files \
             (id, form_id, inquiry_id, field_key, storage_key, original_name, content_type, \
              size_bytes, checksum_sha256, status, expires_at, created_at, claimed_at) \
             VALUES (?, ?, NULL, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, NULL)",
        )
        .bind(id.as_str())
        .bind(form.id.as_str())
        .bind(field_key)
        .bind(&storage_key)
        .bind(&original_name)
        .bind(&content_type)
        .bind(size_bytes)
        .bind(&checksum_sha256)
        .bind(expires_at)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_conflict(error, "attachment storage key already exists"))?;

        self.get(&id).await
    }

    /// Loads one attachment by identifier.
    pub async fn get(&self, id: &InquiryFileId) -> Result<InquiryFile, AppError> {
        load_file_row(&self.db, id).await?.into_file()
    }

    /// Lists pending uploads whose expiry has passed.
    pub async fn expired_pending(&self, limit: i64) -> Result<Vec<InquiryFile>, AppError> {
        let now = now_timestamp();
        let limit = limit.clamp(1, 1_000);
        let sql = format!(
            "SELECT {FILE_COLUMNS} FROM inquiry_files \
             WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at <= ? \
             ORDER BY expires_at ASC, id ASC LIMIT ?"
        );
        let rows = sqlx::query_as::<_, InquiryFileRow>(&sql)
            .bind(now)
            .bind(limit)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        rows.into_iter().map(InquiryFileRow::into_file).collect()
    }

    /// Marks a pending upload as expired. Already claimed or expired rows are
    /// left untouched.
    pub async fn mark_expired(&self, id: &InquiryFileId) -> Result<(), AppError> {
        let result = sqlx::query(
            "UPDATE inquiry_files SET status = 'expired' WHERE id = ? AND status = 'pending'",
        )
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            let exists: Option<i64> =
                sqlx::query_scalar("SELECT 1 FROM inquiry_files WHERE id = ?")
                    .bind(id.as_str())
                    .fetch_optional(self.db.pool())
                    .await
                    .map_err(database_error)?;
            if exists.is_none() {
                return Err(AppError::not_found("inquiry file", id));
            }
        }
        Ok(())
    }
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
