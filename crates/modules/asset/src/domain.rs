//! Domain entities, inputs and pure validation rules for media assets.
//!
//! Only asset metadata lives here. The binary payload is stored by the API
//! layer through `fastcart_infrastructure::Storage`; [`Asset::storage_key`]
//! identifies the stored object.

use fastcart_kernel::AppError;
use serde::{Deserialize, Serialize};

/// Maximum length of an asset filename.
pub const MAX_FILENAME_LENGTH: usize = 255;
/// Maximum length of a storage object key.
pub const MAX_STORAGE_KEY_LENGTH: usize = 512;
/// Maximum length of a media content type.
pub const MAX_CONTENT_TYPE_LENGTH: usize = 127;
/// Maximum length of alternative text.
pub const MAX_ALT_LENGTH: usize = 500;
/// Maximum accepted asset size in bytes (100 MiB).
pub const MAX_SIZE_BYTES: i64 = 100 * 1024 * 1024;

/// Storefront URL path serving an asset by identifier.
///
/// The storefront and admin application are served from the same origin in the
/// supported topology, so bindings can store this relative path.
#[must_use]
pub fn public_url(id: &str) -> String {
    format!("/api/v1/store/assets/{id}")
}

/// A store-scoped media asset.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    /// Asset identifier.
    pub id: String,
    /// Original upload filename.
    pub filename: String,
    /// Media type of the stored object.
    pub content_type: String,
    /// Object size in bytes.
    pub size_bytes: i64,
    /// Key of the object in infrastructure storage.
    pub storage_key: String,
    /// Alternative text for accessibility and display fallbacks.
    pub alt: Option<String>,
    /// Intrinsic width in pixels, when known.
    pub width: Option<i64>,
    /// Intrinsic height in pixels, when known.
    pub height: Option<i64>,
    /// Actor that uploaded the asset, when known.
    pub created_by: Option<String>,
    /// Creation timestamp.
    pub created_at: i64,
    /// Soft-deletion timestamp; live assets have `None`.
    pub deleted_at: Option<i64>,
}

/// Input for registering a new media asset.
#[derive(utoipa::ToSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateAssetInput {
    /// Original upload filename.
    pub filename: String,
    /// Media type of the stored object.
    pub content_type: String,
    /// Object size in bytes.
    pub size_bytes: i64,
    /// Key of the object in infrastructure storage.
    pub storage_key: String,
    /// Alternative text for accessibility and display fallbacks.
    pub alt: Option<String>,
    /// Intrinsic width in pixels, when known.
    pub width: Option<i64>,
    /// Intrinsic height in pixels, when known.
    pub height: Option<i64>,
    /// Actor that uploaded the asset, when known.
    pub created_by: Option<String>,
}

impl CreateAssetInput {
    /// Validates the asset metadata against the module limits.
    pub fn validate(&self) -> Result<(), AppError> {
        validate_bounded_text(&self.filename, "filename", MAX_FILENAME_LENGTH)?;
        validate_bounded_text(&self.storage_key, "storage_key", MAX_STORAGE_KEY_LENGTH)?;
        validate_content_type(&self.content_type)?;

        if !(0..=MAX_SIZE_BYTES).contains(&self.size_bytes) {
            return Err(AppError::validation(format!(
                "size_bytes must be between 0 and {MAX_SIZE_BYTES}"
            )));
        }

        if let Some(alt) = self.alt.as_deref()
            && alt.chars().count() > MAX_ALT_LENGTH
        {
            return Err(AppError::validation(format!(
                "alt must not exceed {MAX_ALT_LENGTH} characters"
            )));
        }

        validate_dimension(self.width, "width")?;
        validate_dimension(self.height, "height")?;
        Ok(())
    }
}

/// Filters applied when listing assets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AssetFilter {
    /// Case-insensitive match against filename or alternative text.
    pub search: Option<String>,
}

fn validate_bounded_text(value: &str, field: &str, maximum: usize) -> Result<(), AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation(format!("{field} must not be empty")));
    }
    if trimmed.chars().count() > maximum {
        return Err(AppError::validation(format!(
            "{field} must not exceed {maximum} characters"
        )));
    }
    Ok(())
}

fn validate_content_type(value: &str) -> Result<(), AppError> {
    validate_bounded_text(value, "content_type", MAX_CONTENT_TYPE_LENGTH)?;
    if value.chars().any(char::is_control) {
        return Err(AppError::validation(
            "content_type must not contain control characters",
        ));
    }
    Ok(())
}

fn validate_dimension(value: Option<i64>, field: &str) -> Result<(), AppError> {
    if let Some(value) = value
        && value < 0
    {
        return Err(AppError::validation(format!(
            "{field} must not be negative"
        )));
    }
    Ok(())
}
