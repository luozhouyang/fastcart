//! SQLite-backed theme registry and activation service.

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, PageResult, Pagination, ThemeActivationId, ThemeId, now_timestamp,
};
use fastcart_storefront::{
    StorefrontError, ThemeEditorManifest, ThemeManifest, validate_bundle_path,
};
use serde_json::Value;
use sqlx::FromRow;

use crate::domain::{RegisterThemeInput, Theme, UpdateThemeSettingsInput};

const THEME_COLUMNS: &str = "id, manifest, bundle_path, settings_schema, settings, \
     editor_manifest, schema_version, is_active, created_at, updated_at";

/// Application service for the storefront theme registry.
#[derive(Clone, Debug)]
pub struct ThemeService {
    db: StoreDatabase,
}

impl ThemeService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    /// Registers a validated theme bundle.
    pub async fn register(&self, input: RegisterThemeInput) -> Result<Theme, AppError> {
        let RegisterThemeInput {
            manifest,
            bundle_path,
            settings_schema,
            settings,
            editor_manifest,
        } = input;

        manifest.validate().map_err(storefront_error)?;
        validate_bundle_path(&bundle_path).map_err(storefront_error)?;
        if let Some(schema) = settings_schema.as_ref() {
            validate_settings_schema(schema)?;
        }
        let settings = settings.unwrap_or_else(|| serde_json::json!({}));
        validate_settings(settings_schema.as_ref(), &settings)?;

        let schema_version = manifest
            .editor
            .as_ref()
            .map_or(1, |editor| editor.schema_version);
        let editor_manifest = match (manifest.editor.as_ref(), editor_manifest) {
            (Some(editor), Some(raw)) => {
                let parsed = ThemeEditorManifest::parse(&raw).map_err(storefront_error)?;
                if parsed.schema_version != editor.schema_version {
                    return Err(AppError::validation(format!(
                        "editor manifest schema version {} does not match theme manifest \
                         version {}",
                        parsed.schema_version, editor.schema_version
                    )));
                }
                Some(raw)
            }
            (Some(_), None) => {
                return Err(AppError::validation(
                    "theme manifest references an editor manifest but none was supplied",
                ));
            }
            (None, Some(_)) => {
                return Err(AppError::validation(
                    "editor_manifest supplied for a theme that does not declare an editor",
                ));
            }
            (None, None) => None,
        };

        if self.manifest_id_exists(&manifest.id).await? {
            return Err(AppError::conflict(format!(
                "theme already registered: {}",
                manifest.id
            )));
        }

        let id = ThemeId::new();
        let now = now_timestamp();
        let manifest_json = serialize_json(&manifest, "manifest")?;
        let settings_schema_json = match settings_schema.as_ref() {
            Some(schema) => Some(serialize_json(schema, "settings_schema")?),
            None => None,
        };
        let settings_json = serialize_json(&settings, "settings")?;
        let editor_manifest_json = match editor_manifest.as_ref() {
            Some(raw) => Some(serialize_json(raw, "editor_manifest")?),
            None => None,
        };

        sqlx::query(
            "INSERT INTO themes \
             (id, manifest_id, name, version, runtime, manifest, bundle_path, settings_schema, \
              settings, editor_manifest, schema_version, is_active, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(id.as_str())
        .bind(&manifest.id)
        .bind(&manifest.name)
        .bind(&manifest.version)
        .bind(&manifest.runtime)
        .bind(&manifest_json)
        .bind(&bundle_path)
        .bind(settings_schema_json.as_deref())
        .bind(&settings_json)
        .bind(editor_manifest_json.as_deref())
        .bind(i64::from(schema_version))
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_conflict(error, "theme already registered"))?;

        self.get(&id).await
    }

    /// Lists registered themes, newest first.
    pub async fn list(&self, pagination: Pagination) -> Result<PageResult<Theme>, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM themes")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let sql = format!(
            "SELECT {THEME_COLUMNS} FROM themes ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let rows = sqlx::query_as::<_, ThemeRow>(&sql)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let data = rows
            .into_iter()
            .map(ThemeRow::into_theme)
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(PageResult::new(data, count, pagination))
    }

    /// Loads a theme by internal identifier.
    pub async fn get(&self, id: &ThemeId) -> Result<Theme, AppError> {
        self.fetch_row(id).await?.into_theme()
    }

    /// Returns the active theme, if one has been activated.
    pub async fn active(&self) -> Result<Option<Theme>, AppError> {
        let sql = format!("SELECT {THEME_COLUMNS} FROM themes WHERE is_active = 1 LIMIT 1");
        let row = sqlx::query_as::<_, ThemeRow>(&sql)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
        row.map(ThemeRow::into_theme).transpose()
    }

    /// Loads and parses a theme's stored editor manifest.
    ///
    /// Returns a conflict when the theme exists but has no editor manifest or
    /// when the stored payload no longer matches the editor contract.
    pub async fn editor_manifest(&self, id: &ThemeId) -> Result<ThemeEditorManifest, AppError> {
        let theme = self.get(id).await?;
        let raw = theme
            .editor_manifest
            .ok_or_else(|| AppError::conflict(format!("theme has no editor manifest: {id}")))?;
        ThemeEditorManifest::parse(&raw).map_err(|error| {
            AppError::conflict(format!("stored theme editor manifest is invalid: {error}"))
        })
    }

    /// Loads the active theme's editor manifest, when it declares one.
    pub async fn active_editor_manifest(&self) -> Result<Option<ThemeEditorManifest>, AppError> {
        let Some(theme) = self.active().await? else {
            return Ok(None);
        };
        let Some(raw) = theme.editor_manifest else {
            return Ok(None);
        };
        ThemeEditorManifest::parse(&raw).map(Some).map_err(|error| {
            AppError::conflict(format!("stored theme editor manifest is invalid: {error}"))
        })
    }

    /// Replaces a theme's settings after schema validation.
    pub async fn update_settings(
        &self,
        id: &ThemeId,
        input: UpdateThemeSettingsInput,
    ) -> Result<Theme, AppError> {
        let theme = self.fetch_row(id).await?.into_theme()?;
        validate_settings(theme.settings_schema.as_ref(), &input.settings)?;

        let settings_json = serialize_json(&input.settings, "settings")?;
        sqlx::query("UPDATE themes SET settings = ?, updated_at = ? WHERE id = ?")
            .bind(&settings_json)
            .bind(now_timestamp())
            .bind(id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;

        self.get(id).await
    }

    /// Activates a theme. Activating the active theme is a no-op.
    pub async fn activate(&self, id: &ThemeId) -> Result<Theme, AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {THEME_COLUMNS} FROM themes WHERE id = ?");
        let target = sqlx::query_as::<_, ThemeRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("theme", id))?;

        if target.is_active != 0 {
            tx.commit().await.map_err(database_error)?;
            return target.into_theme();
        }

        let previous_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM themes WHERE is_active = 1 LIMIT 1")
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;

        let now = now_timestamp();
        sqlx::query("UPDATE themes SET is_active = 0, updated_at = ? WHERE is_active = 1")
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("UPDATE themes SET is_active = 1, updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query(
            "INSERT INTO theme_activations (id, theme_id, previous_theme_id, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(ThemeActivationId::new().as_str())
        .bind(id.as_str())
        .bind(previous_id.as_deref())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        tx.commit().await.map_err(database_error)?;
        self.get(id).await
    }

    /// Rolls back the most recent activation to the theme active before it.
    pub async fn rollback(&self) -> Result<Theme, AppError> {
        let mut tx = self.db.begin().await?;
        let latest_previous: Option<Option<String>> = sqlx::query_scalar(
            "SELECT previous_theme_id FROM theme_activations \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        let previous_id = latest_previous
            .ok_or_else(|| AppError::conflict("no theme activation to roll back"))?
            .ok_or_else(|| {
                AppError::conflict("the latest theme activation has no previous theme")
            })?;
        let previous_id = ThemeId::from_string(previous_id);

        let previous_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM themes WHERE id = ?")
            .bind(previous_id.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
        if previous_exists == 0 {
            return Err(AppError::conflict(format!(
                "previous theme no longer exists: {previous_id}"
            )));
        }

        let current_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM themes WHERE is_active = 1 LIMIT 1")
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;

        let now = now_timestamp();
        sqlx::query("UPDATE themes SET is_active = 0, updated_at = ? WHERE is_active = 1")
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("UPDATE themes SET is_active = 1, updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(previous_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query(
            "INSERT INTO theme_activations (id, theme_id, previous_theme_id, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(ThemeActivationId::new().as_str())
        .bind(previous_id.as_str())
        .bind(current_id.as_deref())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        tx.commit().await.map_err(database_error)?;
        self.get(&previous_id).await
    }

    /// Deletes an inactive theme.
    ///
    /// The active check is part of the delete statement so a concurrent
    /// activation cannot slip between the check and the delete.
    pub async fn delete(&self, id: &ThemeId) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM themes WHERE id = ? AND is_active = 0")
            .bind(id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            // Distinguish a missing theme from an active one without relying
            // on the pre-delete state.
            let theme = self.fetch_row(id).await?;
            if theme.is_active != 0 {
                return Err(AppError::conflict("cannot delete the active theme"));
            }
            return Err(AppError::not_found("theme", id));
        }
        Ok(())
    }

    async fn manifest_id_exists(&self, manifest_id: &str) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM themes WHERE manifest_id = ?")
            .bind(manifest_id)
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(count > 0)
    }

    async fn fetch_row(&self, id: &ThemeId) -> Result<ThemeRow, AppError> {
        let sql = format!("SELECT {THEME_COLUMNS} FROM themes WHERE id = ?");
        sqlx::query_as::<_, ThemeRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("theme", id))
    }
}

fn storefront_error(error: StorefrontError) -> AppError {
    AppError::validation(error.to_string())
}

fn unique_conflict(error: sqlx::Error, message: &str) -> AppError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            AppError::conflict(message)
        }
        _ => database_error(error),
    }
}

fn validate_settings_schema(schema: &Value) -> Result<(), AppError> {
    if !schema.is_object() {
        return Err(AppError::validation(
            "settings_schema must be a JSON object",
        ));
    }
    jsonschema::validator_for(schema)
        .map_err(|error| AppError::validation(format!("invalid settings schema: {error}")))?;
    Ok(())
}

fn validate_settings(schema: Option<&Value>, settings: &Value) -> Result<(), AppError> {
    let Some(schema) = schema else {
        return Ok(());
    };
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| AppError::validation(format!("invalid settings schema: {error}")))?;
    validator
        .validate(settings)
        .map_err(|error| AppError::validation(format!("invalid theme settings: {error}")))
}

fn serialize_json<T: serde::Serialize>(value: &T, field: &str) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::internal(format!("could not serialize {field}: {error}")))
}

#[derive(Debug, FromRow)]
struct ThemeRow {
    id: String,
    manifest: String,
    bundle_path: String,
    settings_schema: Option<String>,
    settings: String,
    editor_manifest: Option<String>,
    schema_version: i64,
    is_active: i64,
    created_at: i64,
    updated_at: i64,
}

impl ThemeRow {
    fn into_theme(self) -> Result<Theme, AppError> {
        let manifest: ThemeManifest = serde_json::from_str(&self.manifest).map_err(|error| {
            AppError::internal(format!("invalid stored theme manifest: {error}"))
        })?;
        let settings_schema: Option<Value> = match self.settings_schema {
            Some(raw) => Some(serde_json::from_str(&raw).map_err(|error| {
                AppError::internal(format!("invalid stored theme settings schema: {error}"))
            })?),
            None => None,
        };
        let settings: Value = serde_json::from_str(&self.settings).map_err(|error| {
            AppError::internal(format!("invalid stored theme settings: {error}"))
        })?;
        let editor_manifest: Option<Value> = match self.editor_manifest {
            Some(raw) => Some(serde_json::from_str(&raw).map_err(|error| {
                AppError::internal(format!("invalid stored theme editor manifest: {error}"))
            })?),
            None => None,
        };
        let schema_version = u32::try_from(self.schema_version).map_err(|_| {
            AppError::internal(format!(
                "invalid stored theme schema version: {}",
                self.schema_version
            ))
        })?;

        Ok(Theme {
            id: ThemeId::from_string(self.id),
            manifest,
            bundle_path: self.bundle_path,
            settings_schema,
            settings,
            editor_manifest,
            schema_version,
            is_active: self.is_active != 0,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}
