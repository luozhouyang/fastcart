//! Form authoring, revisions and publishing.

use super::*;

impl InquiryFormService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db, audit: None }
    }

    /// Records form mutations in the store audit outbox.
    ///
    /// When an actor is set, every mutating method appends its audit entry in
    /// the same transaction as the change, so a crash cannot separate them.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

    /// Appends one audit entry to the outbox inside a mutation transaction.
    async fn append_audit(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        data: Value,
    ) -> Result<(), AppError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, resource_type, resource_id, data)
                .await
                .map_err(database_error)?;
        }
        Ok(())
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

        self.append_audit(
            &mut tx,
            "inquiry_form.created",
            "inquiry_form",
            id.as_str(),
            serde_json::json!({"handle": handle}),
        )
        .await?;
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

    /// Lists published forms newest first, optionally filtered by name or
    /// handle.
    ///
    /// The query is matched case-insensitively and LIKE wildcards in user
    /// input are escaped. An absent or blank query matches every published
    /// form.
    pub async fn list_published_forms(
        &self,
        query: Option<&str>,
        pagination: Pagination,
    ) -> Result<PageResult<InquiryForm>, AppError> {
        let pattern = like_pattern(query);
        let conditions = if pattern.is_some() {
            " WHERE status = 'published' AND (LOWER(name) LIKE LOWER(?) ESCAPE '\\' \
             OR LOWER(handle) LIKE LOWER(?) ESCAPE '\\')"
        } else {
            " WHERE status = 'published'"
        };

        let count_sql = format!("SELECT COUNT(*) FROM inquiry_forms{conditions}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(pattern) = pattern.as_deref() {
            count_query = count_query.bind(pattern).bind(pattern);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT {FORM_COLUMNS} FROM inquiry_forms{conditions} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query_as::<_, InquiryFormRow>(&list_sql);
        if let Some(pattern) = pattern.as_deref() {
            list_query = list_query.bind(pattern).bind(pattern);
        }
        let rows = list_query
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
        let mut tx = self.db.begin().await?;
        let existing: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT name, description FROM inquiry_forms WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
        let Some((previous_name, previous_description)) = existing else {
            return Err(AppError::not_found("inquiry form", id));
        };
        let name = match input.name {
            Some(name) => {
                validate_length(&name, "name", MAX_NAME_LENGTH)?;
                name.trim().to_owned()
            }
            None => previous_name.clone(),
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
            None => previous_description.clone(),
        };
        let changed = name != previous_name || description != previous_description;

        let result = sqlx::query(
            "UPDATE inquiry_forms SET name = ?, description = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&name)
        .bind(&description)
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("inquiry form", id));
        }
        if changed {
            self.append_audit(
                &mut tx,
                "inquiry_form.updated",
                "inquiry_form",
                id.as_str(),
                serde_json::json!({}),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
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
            // Saving an identical draft is a no-op: skip both the write and
            // its audit entry.
            if draft.fields == fields_json && draft.settings == settings_json {
                return draft.into_revision();
            }
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
            self.append_audit(
                &mut tx,
                "inquiry_form.draft_saved",
                "inquiry_form_revision",
                &draft.id,
                serde_json::json!({"version": draft.version}),
            )
            .await?;
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

        self.append_audit(
            &mut tx,
            "inquiry_form.draft_saved",
            "inquiry_form_revision",
            revision_id.as_str(),
            serde_json::json!({"version": version}),
        )
        .await?;
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
        let current: Option<(String, Option<i64>)> =
            sqlx::query_as("SELECT status, published_version FROM inquiry_forms WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
        let Some((status, published_version)) = current else {
            return Err(AppError::not_found("inquiry form", id));
        };
        // There is no unarchive path, so publishing must not silently revive
        // an archived form.
        if status == InquiryFormStatus::Archived.as_str() {
            return Err(AppError::conflict(
                "an archived inquiry form cannot be published",
            ));
        }
        let no_op = status == InquiryFormStatus::Published.as_str()
            && published_version == Some(revision.version);
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

        if !no_op {
            self.append_audit(
                &mut tx,
                "inquiry_form.published",
                "inquiry_form",
                id.as_str(),
                serde_json::json!({"version": revision.version}),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        self.get_form(id).await
    }

    /// Archives a form, removing it from storefront lookups.
    pub async fn archive(&self, id: &InquiryFormId) -> Result<InquiryForm, AppError> {
        let existing = self.get_form(id).await?;
        let changed = existing.status != InquiryFormStatus::Archived;

        let mut tx = self.db.begin().await?;
        let result = sqlx::query(
            "UPDATE inquiry_forms SET status = 'archived', updated_at = ? WHERE id = ?",
        )
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("inquiry form", id));
        }
        if changed {
            self.append_audit(
                &mut tx,
                "inquiry_form.archived",
                "inquiry_form",
                id.as_str(),
                serde_json::json!({}),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
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

    /// Loads the current draft revision by handle for editor previews.
    ///
    /// Archived forms are hidden. A form with no draft falls back to its
    /// published revision so preview never turns into an error just because the
    /// editor has not saved a draft yet.
    pub async fn draft_by_handle(&self, handle: &str) -> Result<PublishedInquiryForm, AppError> {
        let handle = handle.trim();
        let sql = format!(
            "SELECT {FORM_COLUMNS} FROM inquiry_forms \
             WHERE handle = ? AND status <> 'archived'"
        );
        let form = sqlx::query_as::<_, InquiryFormRow>(&sql)
            .bind(handle)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("inquiry form", handle))?
            .into_form()?;
        let revision = match self.get_draft(&form.id).await? {
            Some(revision) => revision,
            None => {
                let version = form.published_version.ok_or_else(|| {
                    AppError::not_found("published inquiry form revision", handle)
                })?;
                self.get_revision(&form.id, version).await?
            }
        };
        Ok(PublishedInquiryForm { form, revision })
    }
}
