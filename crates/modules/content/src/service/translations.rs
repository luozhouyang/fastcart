//! Central translation storage for storefront resources.
//!
//! One row per `(resource_type, resource_id, locale)` holds a JSON object of
//! translated fields. The row set is small (resources times locales), so
//! resolution reads the rows it needs and merges per field in Rust: the
//! requested locale wins, then the caller-provided fallback chain, and any
//! missing field stays absent so the caller can fall back to the source
//! content.

use super::*;
use fastcart_kernel::Locale;

/// Maximum serialized size of a translation field object.
pub const MAX_TRANSLATION_BYTES: usize = 64 * 1024;

/// Maximum length of a translation resource identifier.
pub const MAX_TRANSLATION_RESOURCE_ID_LENGTH: usize = 128;

/// Resource types that may carry translations.
pub const TRANSLATION_RESOURCE_TYPES: &[&str] =
    &["product", "category", "collection", "page", "navigation"];

/// A stored translation row.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Translation {
    /// Row identifier.
    pub id: String,
    /// Translated resource type.
    pub resource_type: String,
    /// Translated resource identifier.
    pub resource_id: String,
    /// Canonical BCP-47 locale.
    pub locale: String,
    /// Translated fields; absent fields fall back to the source content.
    pub fields: Value,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating or replacing one translation row.
#[derive(Clone, Debug, PartialEq)]
pub struct UpsertTranslationInput {
    /// Resource type such as `product`.
    pub resource_type: String,
    /// Resource identifier.
    pub resource_id: String,
    /// Requested locale; normalized to canonical BCP-47 form.
    pub locale: String,
    /// Translated fields; must be a JSON object.
    pub fields: Value,
}

/// Result of resolving one resource's fields for a locale.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedTranslation {
    /// Merged fields for the requested locale and its fallbacks.
    pub fields: Value,
    /// Locale that supplied each returned field.
    pub sources: std::collections::BTreeMap<String, String>,
}

impl ResolvedTranslation {
    /// Returns the locale that supplied `field`, when any locale translated it.
    #[must_use]
    pub fn source_of(&self, field: &str) -> Option<&str> {
        self.sources.get(field).map(String::as_str)
    }
}

#[derive(Debug, FromRow)]
struct TranslationRow {
    id: String,
    resource_type: String,
    resource_id: String,
    locale: String,
    fields_json: String,
    created_at: i64,
    updated_at: i64,
}

impl TranslationRow {
    fn into_translation(self) -> Translation {
        Translation {
            id: self.id,
            resource_type: self.resource_type,
            resource_id: self.resource_id,
            locale: self.locale,
            fields: parse_json(&self.fields_json),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

const TRANSLATION_COLUMNS: &str =
    "id, resource_type, resource_id, locale, fields_json, created_at, updated_at";

/// Validates a resource type against the allowed list.
fn validate_resource_type(resource_type: &str) -> Result<String, AppError> {
    let resource_type = resource_type.trim().to_ascii_lowercase();
    if !TRANSLATION_RESOURCE_TYPES.contains(&resource_type.as_str()) {
        return Err(AppError::validation(format!(
            "unsupported translation resource_type: {resource_type}; expected one of {}",
            TRANSLATION_RESOURCE_TYPES.join(", ")
        )));
    }
    Ok(resource_type)
}

/// Validates a resource identifier.
fn validate_resource_id(resource_id: &str) -> Result<String, AppError> {
    let resource_id = resource_id.trim().to_owned();
    if resource_id.is_empty() || resource_id.chars().count() > MAX_TRANSLATION_RESOURCE_ID_LENGTH {
        return Err(AppError::validation(format!(
            "resource_id must be between 1 and {MAX_TRANSLATION_RESOURCE_ID_LENGTH} characters"
        )));
    }
    Ok(resource_id)
}

/// Validates translated fields: a JSON object under the size cap.
fn validate_fields(fields: &Value) -> Result<String, AppError> {
    let object = fields
        .as_object()
        .ok_or_else(|| AppError::validation("translation fields must be a JSON object"))?;
    if object.contains_key("") {
        return Err(AppError::validation(
            "translation field names must not be empty",
        ));
    }
    let serialized = serialize_json(fields, "translation fields")?;
    if serialized.len() > MAX_TRANSLATION_BYTES {
        return Err(AppError::validation(format!(
            "translation fields must not exceed {MAX_TRANSLATION_BYTES} bytes"
        )));
    }
    Ok(serialized)
}

impl ContentService {
    /// Creates or replaces one translation row.
    ///
    /// The locale is normalized through [`Locale`], the resource type must be in
    /// the allowed list and the fields must be a JSON object under the size cap.
    pub async fn upsert_translation(
        &self,
        input: UpsertTranslationInput,
    ) -> Result<Translation, AppError> {
        let resource_type = validate_resource_type(&input.resource_type)?;
        let resource_id = validate_resource_id(&input.resource_id)?;
        let locale = Locale::new(input.locale)?;
        let fields_json = validate_fields(&input.fields)?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;

        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM translations \
             WHERE resource_type = ? AND resource_id = ? AND locale = ?",
        )
        .bind(&resource_type)
        .bind(&resource_id)
        .bind(locale.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;

        let id = match existing {
            Some(id) => {
                sqlx::query("UPDATE translations SET fields_json = ?, updated_at = ? WHERE id = ?")
                    .bind(&fields_json)
                    .bind(now)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(database_error)?;
                id
            }
            None => {
                let id = generate_id("tr");
                sqlx::query(
                    "INSERT INTO translations \
                     (id, resource_type, resource_id, locale, fields_json, created_at, updated_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&resource_type)
                .bind(&resource_id)
                .bind(locale.as_str())
                .bind(&fields_json)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|error| unique_conflict(error, "translation already exists"))?;
                id
            }
        };

        self.append_audit(
            &mut tx,
            "translation.upserted",
            "translation",
            &id,
            serde_json::json!({
                "resource_type": resource_type,
                "resource_id": resource_id,
                "locale": locale.as_str(),
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_translation(&resource_type, &resource_id, locale.as_str())
            .await
    }
}

impl ContentService {
    /// Loads one translation row.
    pub async fn get_translation(
        &self,
        resource_type: &str,
        resource_id: &str,
        locale: &str,
    ) -> Result<Translation, AppError> {
        let resource_type = validate_resource_type(resource_type)?;
        let resource_id = validate_resource_id(resource_id)?;
        let locale = Locale::new(locale)?;
        let row = sqlx::query_as::<_, TranslationRow>(&format!(
            "SELECT {TRANSLATION_COLUMNS} FROM translations \
             WHERE resource_type = ? AND resource_id = ? AND locale = ?"
        ))
        .bind(&resource_type)
        .bind(&resource_id)
        .bind(locale.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| {
            AppError::not_found(
                "translation",
                format!("{resource_type}/{resource_id}/{locale}"),
            )
        })?;
        Ok(row.into_translation())
    }
}

impl ContentService {
    /// Lists every translation of one resource, ordered by locale.
    pub async fn list_translations(
        &self,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<Vec<Translation>, AppError> {
        let resource_type = validate_resource_type(resource_type)?;
        let resource_id = validate_resource_id(resource_id)?;
        let rows = sqlx::query_as::<_, TranslationRow>(&format!(
            "SELECT {TRANSLATION_COLUMNS} FROM translations \
             WHERE resource_type = ? AND resource_id = ? ORDER BY locale ASC"
        ))
        .bind(&resource_type)
        .bind(&resource_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows
            .into_iter()
            .map(TranslationRow::into_translation)
            .collect())
    }
}

impl ContentService {
    /// Deletes one translation row.
    ///
    /// A missing row is reported as not found so callers can distinguish a
    /// successful delete from a typo in the identifier or locale.
    pub async fn delete_translation(
        &self,
        resource_type: &str,
        resource_id: &str,
        locale: &str,
    ) -> Result<(), AppError> {
        let resource_type = validate_resource_type(resource_type)?;
        let resource_id = validate_resource_id(resource_id)?;
        let locale = Locale::new(locale)?;
        let mut tx = self.db.begin().await?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM translations \
             WHERE resource_type = ? AND resource_id = ? AND locale = ?",
        )
        .bind(&resource_type)
        .bind(&resource_id)
        .bind(locale.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        let Some(id) = existing else {
            return Err(AppError::not_found(
                "translation",
                format!("{resource_type}/{resource_id}/{locale}"),
            ));
        };
        sqlx::query("DELETE FROM translations WHERE id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "translation.deleted",
            "translation",
            &id,
            serde_json::json!({
                "resource_type": resource_type,
                "resource_id": resource_id,
                "locale": locale.as_str(),
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}

impl ContentService {
    /// Loads a resource's translations and resolves them for a requested
    /// locale.
    ///
    /// This is the typed read path storefront callers use: the returned fields
    /// are the merged translation object, and any field still absent must be
    /// read from the source content.
    pub async fn resolve_translations(
        &self,
        resource_type: &str,
        resource_id: &str,
        requested: &str,
        chain: &[String],
    ) -> Result<ResolvedTranslation, AppError> {
        let requested = Locale::new(requested)?;
        let rows = self.list_translations(resource_type, resource_id).await?;
        Ok(resolve_translation(&rows, requested.as_str(), chain))
    }
}

/// Merges one resource's translation rows for a requested locale.
///
/// `chain` is the fallback order after the requested locale, for example
/// `[default_locale]`. Fields are merged per field: the first locale in the
/// chain that defines a field wins, and a field no locale defines stays absent
/// so the caller can fall back to the source content. Nulls count as missing.
#[must_use]
pub fn resolve_translation(
    rows: &[Translation],
    requested: &str,
    chain: &[String],
) -> ResolvedTranslation {
    let mut order = vec![requested.to_owned()];
    for locale in chain {
        if !order.iter().any(|existing| existing == locale) {
            order.push(locale.clone());
        }
    }

    let mut resolved = ResolvedTranslation {
        fields: Value::Object(serde_json::Map::new()),
        sources: std::collections::BTreeMap::new(),
    };
    let fields = resolved
        .fields
        .as_object_mut()
        .expect("resolved fields are an object");
    for locale in &order {
        let Some(row) = rows.iter().find(|row| &row.locale == locale) else {
            continue;
        };
        let Some(object) = row.fields.as_object() else {
            continue;
        };
        for (name, value) in object {
            if value.is_null() || fields.contains_key(name) {
                continue;
            }
            fields.insert(name.clone(), value.clone());
            resolved.sources.insert(name.clone(), locale.clone());
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translation(locale: &str, fields: serde_json::Value) -> Translation {
        Translation {
            id: format!("tr_{locale}"),
            resource_type: "product".to_owned(),
            resource_id: "prd_1".to_owned(),
            locale: locale.to_owned(),
            fields,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn resolves_fields_per_field_across_the_fallback_chain() {
        let rows = vec![
            translation(
                "fr-FR",
                serde_json::json!({"title": "Chaise", "description": null}),
            ),
            translation(
                "en-US",
                serde_json::json!({"title": "Chair", "description": "A chair"}),
            ),
        ];

        let resolved =
            resolve_translation(&rows, "fr-CA", &["fr-FR".to_owned(), "en-US".to_owned()]);
        assert_eq!(resolved.fields["title"], "Chaise");
        assert_eq!(resolved.fields["description"], "A chair");
        assert_eq!(resolved.source_of("title"), Some("fr-FR"));
        assert_eq!(resolved.source_of("description"), Some("en-US"));
    }

    #[test]
    fn resolved_locale_wins_over_the_chain() {
        let rows = vec![
            translation("fr-FR", serde_json::json!({"title": "Chaise"})),
            translation("fr-CA", serde_json::json!({"title": "Banc"})),
        ];

        let resolved = resolve_translation(&rows, "fr-CA", &["fr-FR".to_owned()]);
        assert_eq!(resolved.fields["title"], "Banc");
        assert_eq!(resolved.source_of("title"), Some("fr-CA"));
    }

    #[test]
    fn untranslated_fields_stay_absent() {
        let rows = vec![translation("fr-FR", serde_json::json!({"title": "Chaise"}))];
        let resolved = resolve_translation(&rows, "fr-FR", &[]);
        assert_eq!(resolved.fields.as_object().unwrap().len(), 1);
        assert!(resolved.source_of("description").is_none());
    }

    #[test]
    fn duplicate_chain_entries_are_ignored() {
        let rows = vec![translation("en-US", serde_json::json!({"title": "Chair"}))];
        let resolved =
            resolve_translation(&rows, "en-US", &["en-US".to_owned(), "en-US".to_owned()]);
        assert_eq!(resolved.fields["title"], "Chair");
    }

    #[test]
    fn resource_type_validation_rejects_unknown_values() {
        assert!(validate_resource_type("product").is_ok());
        assert!(validate_resource_type(" PRODUCT ").is_ok());
        assert!(validate_resource_type("order").is_err());
        assert!(validate_resource_type("").is_err());
    }

    #[test]
    fn fields_must_be_an_object_under_the_size_cap() {
        assert!(validate_fields(&serde_json::json!({"title": "x"})).is_ok());
        assert!(validate_fields(&serde_json::json!(["x"])).is_err());
        assert!(validate_fields(&serde_json::json!("x")).is_err());
        let huge = serde_json::json!({"title": "x".repeat(MAX_TRANSLATION_BYTES)});
        assert!(validate_fields(&huge).is_err());
    }
}
