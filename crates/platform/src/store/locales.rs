//! Store locale configuration: the display languages a store publishes.
//!
//! `stores.locale` stays the source locale of the base content; this table
//! records which locales are published and which one is the default display
//! locale. A store always has at most one default, enforced by a partial unique
//! index in `0003_store_locales.sql`.

use super::*;

/// Maximum length of a human-readable locale label.
pub const MAX_LOCALE_LABEL_LENGTH: usize = 120;

/// Text direction of a locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocaleDirection {
    /// Left-to-right script.
    Ltr,
    /// Right-to-left script.
    Rtl,
}

impl LocaleDirection {
    /// Database representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ltr => "ltr",
            Self::Rtl => "rtl",
        }
    }

    /// Parses a stored or requested value. Unknown values are rejected.
    pub fn parse(value: &str) -> PlatformResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ltr" => Ok(Self::Ltr),
            "rtl" => Ok(Self::Rtl),
            other => Err(PlatformError::InvalidInput(format!(
                "invalid locale direction: {other}; expected ltr or rtl"
            ))),
        }
    }
}

/// A locale published by a store.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoreLocale {
    /// Owning store.
    pub store_id: StoreId,
    /// Canonical BCP-47 locale such as `fr-FR`.
    pub locale: String,
    /// Whether this is the default display locale.
    pub is_default: bool,
    /// Native label such as `Français`.
    pub label: Option<String>,
    /// Text direction.
    pub direction: LocaleDirection,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// Input for adding a locale to a store.
#[derive(Clone, Debug, Default)]
pub struct AddStoreLocaleInput {
    /// Requested locale; normalized to canonical BCP-47 form.
    pub locale: String,
    /// Whether the new locale becomes the default display locale.
    pub is_default: bool,
    /// Optional native label.
    pub label: Option<String>,
    /// Text direction; defaults to `ltr`.
    pub direction: Option<String>,
}

/// Input for updating an existing store locale.
#[derive(Clone, Debug, Default)]
pub struct UpdateStoreLocaleInput {
    /// Replacement native label; an empty string clears it.
    pub label: Option<String>,
    /// Replacement text direction.
    pub direction: Option<String>,
}

#[derive(Debug, FromRow)]
struct StoreLocaleRow {
    store_id: String,
    locale: String,
    is_default: i64,
    label: Option<String>,
    direction: String,
    created_at: i64,
    updated_at: i64,
}

impl StoreLocaleRow {
    fn into_locale(self) -> StoreLocale {
        StoreLocale {
            store_id: StoreId::from_string(self.store_id),
            locale: self.locale,
            is_default: self.is_default != 0,
            label: self.label,
            direction: LocaleDirection::parse(&self.direction).unwrap_or(LocaleDirection::Ltr),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

const STORE_LOCALE_COLUMNS: &str =
    "store_id, locale, is_default, label, direction, created_at, updated_at";

/// Trims an optional label and rejects one that is too long.
fn normalize_label(label: Option<String>) -> PlatformResult<Option<String>> {
    let Some(label) = label else {
        return Ok(None);
    };
    let label = label.trim().to_owned();
    if label.is_empty() {
        return Ok(None);
    }
    if label.chars().count() > MAX_LOCALE_LABEL_LENGTH {
        return Err(PlatformError::InvalidInput(format!(
            "locale label must not exceed {MAX_LOCALE_LABEL_LENGTH} characters"
        )));
    }
    Ok(Some(label))
}

impl PlatformStore {
    /// Lists the locales configured for a store, default first.
    pub async fn list_store_locales(&self, store_id: &StoreId) -> PlatformResult<Vec<StoreLocale>> {
        let rows = sqlx::query_as::<_, StoreLocaleRow>(&format!(
            "SELECT {STORE_LOCALE_COLUMNS} FROM store_locales
             WHERE store_id = ? ORDER BY is_default DESC, locale ASC"
        ))
        .bind(store_id.as_str())
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(StoreLocaleRow::into_locale).collect())
    }
}

impl PlatformStore {
    /// Adds a locale to a store.
    ///
    /// The locale is normalized through [`Locale`]. The first locale added to a
    /// store always becomes its default; otherwise `is_default` is honored and
    /// clears the previous default in the same transaction.
    pub async fn add_store_locale(
        &self,
        store_id: &StoreId,
        input: AddStoreLocaleInput,
    ) -> PlatformResult<StoreLocale> {
        if self.get_store(store_id).await?.is_none() {
            return Err(PlatformError::StoreNotFound(store_id.to_string()));
        }
        let locale = Locale::new(input.locale)?;
        let label = normalize_label(input.label)?;
        let direction = match input.direction.as_deref() {
            Some(value) => LocaleDirection::parse(value)?,
            None => LocaleDirection::Ltr,
        };
        let now = now_timestamp();
        let mut transaction = self.pool().begin().await?;

        let existing: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM store_locales WHERE store_id = ?")
                .bind(store_id.as_str())
                .fetch_one(&mut *transaction)
                .await?;
        let is_default = input.is_default || existing == 0;
        if is_default {
            sqlx::query(
                "UPDATE store_locales SET is_default = 0, updated_at = ? \
                 WHERE store_id = ? AND is_default = 1",
            )
            .bind(now)
            .bind(store_id.as_str())
            .execute(&mut *transaction)
            .await?;
        }

        let insert = sqlx::query(
            "INSERT INTO store_locales
             (store_id, locale, is_default, label, direction, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(store_id.as_str())
        .bind(locale.as_str())
        .bind(i64::from(is_default))
        .bind(&label)
        .bind(direction.as_str())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = insert {
            return Err(if unique_violation(&error) {
                PlatformError::Conflict(format!(
                    "locale already configured for this store: {locale}"
                ))
            } else {
                error.into()
            });
        }
        transaction.commit().await?;

        Ok(StoreLocale {
            store_id: store_id.clone(),
            locale: locale.to_string(),
            is_default,
            label,
            direction,
            created_at: now,
            updated_at: now,
        })
    }
}

impl PlatformStore {
    /// Updates the label or direction of an existing store locale.
    pub async fn update_store_locale(
        &self,
        store_id: &StoreId,
        locale: &str,
        input: UpdateStoreLocaleInput,
    ) -> PlatformResult<StoreLocale> {
        let locale = Locale::new(locale)?;
        let current = self
            .get_store_locale(store_id, locale.as_str())
            .await?
            .ok_or_else(|| PlatformError::NotFound(format!("locale not found: {locale}")))?;
        // `None` leaves the field unchanged; an empty label clears it.
        let label = if input.label.is_some() {
            normalize_label(input.label)?
        } else {
            current.label
        };
        let direction = match input.direction.as_deref() {
            Some(value) => LocaleDirection::parse(value)?,
            None => current.direction,
        };
        sqlx::query(
            "UPDATE store_locales SET label = ?, direction = ?, updated_at = ? \
             WHERE store_id = ? AND locale = ?",
        )
        .bind(&label)
        .bind(direction.as_str())
        .bind(now_timestamp())
        .bind(store_id.as_str())
        .bind(locale.as_str())
        .execute(self.pool())
        .await?;
        self.get_store_locale(store_id, locale.as_str())
            .await?
            .ok_or_else(|| PlatformError::NotFound(format!("locale not found: {locale}")))
    }
}

impl PlatformStore {
    /// Fetches one configured store locale.
    pub async fn get_store_locale(
        &self,
        store_id: &StoreId,
        locale: &str,
    ) -> PlatformResult<Option<StoreLocale>> {
        let locale = Locale::new(locale)?;
        let row = sqlx::query_as::<_, StoreLocaleRow>(&format!(
            "SELECT {STORE_LOCALE_COLUMNS} FROM store_locales \
             WHERE store_id = ? AND locale = ?"
        ))
        .bind(store_id.as_str())
        .bind(locale.as_str())
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(StoreLocaleRow::into_locale))
    }
}

impl PlatformStore {
    /// Removes a locale. The default locale cannot be removed directly.
    pub async fn remove_store_locale(
        &self,
        store_id: &StoreId,
        locale: &str,
    ) -> PlatformResult<()> {
        let locale = Locale::new(locale)?;
        let is_default: Option<i64> = sqlx::query_scalar(
            "SELECT is_default FROM store_locales WHERE store_id = ? AND locale = ?",
        )
        .bind(store_id.as_str())
        .bind(locale.as_str())
        .fetch_optional(self.pool())
        .await?;
        match is_default {
            None => Err(PlatformError::NotFound(format!(
                "locale not found: {locale}"
            ))),
            Some(1) => Err(PlatformError::Conflict(
                "cannot remove the default locale; assign another default locale first".to_owned(),
            )),
            Some(_) => {
                sqlx::query("DELETE FROM store_locales WHERE store_id = ? AND locale = ?")
                    .bind(store_id.as_str())
                    .bind(locale.as_str())
                    .execute(self.pool())
                    .await?;
                Ok(())
            }
        }
    }
}

impl PlatformStore {
    /// Makes an existing locale the default display locale.
    ///
    /// Clearing the previous default and promoting the target happen in one
    /// transaction so the partial unique index can never see two defaults.
    pub async fn set_default_store_locale(
        &self,
        store_id: &StoreId,
        locale: &str,
    ) -> PlatformResult<StoreLocale> {
        let locale = Locale::new(locale)?;
        let now = now_timestamp();
        let mut transaction = self.pool().begin().await?;
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT locale FROM store_locales WHERE store_id = ? AND locale = ?",
        )
        .bind(store_id.as_str())
        .bind(locale.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        if exists.is_none() {
            return Err(PlatformError::NotFound(format!(
                "locale not found: {locale}"
            )));
        }
        sqlx::query(
            "UPDATE store_locales SET is_default = 0, updated_at = ? \
             WHERE store_id = ? AND is_default = 1",
        )
        .bind(now)
        .bind(store_id.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE store_locales SET is_default = 1, updated_at = ? \
             WHERE store_id = ? AND locale = ?",
        )
        .bind(now)
        .bind(store_id.as_str())
        .bind(locale.as_str())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_store_locale(store_id, locale.as_str())
            .await?
            .ok_or_else(|| PlatformError::NotFound(format!("locale not found: {locale}")))
    }
}
