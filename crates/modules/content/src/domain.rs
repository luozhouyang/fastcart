//! Domain entities, inputs and pure validation rules for content.

use fastcart_kernel::{AppError, Metadata, NavigationId, PageId, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use fastcart_kernel::{slugify, validate_handle, validate_length};

/// Maximum serialized size of a page body.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Maximum length of a page, navigation or item title.
pub const MAX_TITLE_LENGTH: usize = 255;
/// Maximum length of a page type classifier.
pub const MAX_PAGE_TYPE_LENGTH: usize = 64;
/// Maximum length of a navigation item URL.
pub const MAX_URL_LENGTH: usize = 2048;
/// Maximum length of a redirect path.
pub const MAX_REDIRECT_PATH_LENGTH: usize = 2048;

/// Publication state of a page.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageStatus {
    /// Work in progress; not visible to storefronts.
    Draft,
    /// Visible to storefronts.
    Published,
    /// Retired from the storefront but retained for history.
    Archived,
}

impl PageStatus {
    /// Returns the storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Archived => "archived",
        }
    }

    /// Parses a stored status, returning `None` for unknown values.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "published" => Some(Self::Published),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

/// A store content page.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    /// Page identifier.
    pub id: PageId,
    /// Human readable title.
    pub title: String,
    /// URL handle.
    pub handle: String,
    /// Publication state.
    pub status: PageStatus,
    /// Page classifier such as `custom`, `home` or `faq`.
    pub page_type: String,
    /// Structured page body.
    pub body: Value,
    /// Structured SEO metadata.
    pub seo: Value,
    /// Free-form extension metadata.
    pub metadata: Metadata,
    /// Publication timestamp, when published.
    pub published_at: Option<Timestamp>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Soft deletion timestamp, when deleted.
    pub deleted_at: Option<Timestamp>,
}

/// An immutable snapshot of a page body.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageVersion {
    /// Version identifier.
    pub id: String,
    /// Owning page.
    pub page_id: PageId,
    /// Monotonically increasing version number per page.
    pub version: i64,
    /// Snapshotted body.
    pub body: Value,
    /// Actor that created the snapshot, when known.
    pub created_by: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// A navigation menu with its items.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Navigation {
    /// Navigation identifier.
    pub id: NavigationId,
    /// Display name.
    pub name: String,
    /// URL handle.
    pub handle: String,
    /// Free-form extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Flat items ordered by rank.
    pub items: Vec<NavigationItem>,
}

/// A single entry in a navigation menu.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavigationItem {
    /// Item identifier.
    pub id: String,
    /// Owning navigation.
    pub navigation_id: NavigationId,
    /// Parent item, when nested.
    pub parent_id: Option<String>,
    /// Display title.
    pub title: String,
    /// Destination URL.
    pub url: String,
    /// Sort rank among siblings; lower values sort first.
    pub rank: i64,
    /// Free-form extension metadata.
    pub metadata: Metadata,
}

/// A navigation tree with nested items.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavigationTree {
    /// Navigation identifier.
    pub id: NavigationId,
    /// Display name.
    pub name: String,
    /// URL handle.
    pub handle: String,
    /// Free-form extension metadata.
    pub metadata: Metadata,
    /// Root items with nested children.
    pub items: Vec<NavigationTreeItem>,
}

/// A navigation item with its nested children.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavigationTreeItem {
    /// Item identifier.
    pub id: String,
    /// Owning navigation.
    pub navigation_id: NavigationId,
    /// Parent item, when nested.
    pub parent_id: Option<String>,
    /// Display title.
    pub title: String,
    /// Destination URL.
    pub url: String,
    /// Sort rank among siblings.
    pub rank: i64,
    /// Free-form extension metadata.
    pub metadata: Metadata,
    /// Nested child items.
    #[schema(no_recursion)]
    pub children: Vec<NavigationTreeItem>,
}

/// An HTTP redirect registered by the storefront.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Redirect {
    /// Redirect identifier.
    pub id: String,
    /// Source path, always starting with `/`.
    pub from_path: String,
    /// Destination path.
    pub to_path: String,
    /// HTTP status code.
    pub status_code: i64,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// Input for creating a page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreatePageInput {
    /// Page title.
    pub title: String,
    /// Optional handle; defaults to a slug of the title.
    #[serde(default)]
    pub handle: Option<String>,
    /// Optional page classifier; defaults to `custom`.
    #[serde(default)]
    pub page_type: Option<String>,
    /// Structured body; must be a JSON object or null.
    #[serde(default)]
    pub body: Value,
    /// Structured SEO metadata; must be a JSON object or null.
    #[serde(default)]
    pub seo: Value,
    /// Free-form extension metadata.
    #[serde(default)]
    pub metadata: Metadata,
}

/// Input for updating a page. Omitted fields keep their current value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdatePageInput {
    /// Replacement title.
    #[serde(default)]
    pub title: Option<String>,
    /// Replacement handle.
    #[serde(default)]
    pub handle: Option<String>,
    /// Replacement page classifier.
    #[serde(default)]
    pub page_type: Option<String>,
    /// Replacement body.
    #[serde(default)]
    pub body: Option<Value>,
    /// Replacement SEO metadata.
    #[serde(default)]
    pub seo: Option<Value>,
    /// Replacement extension metadata.
    #[serde(default)]
    pub metadata: Option<Metadata>,
}

/// Input for creating a navigation menu.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateNavigationInput {
    /// Display name.
    pub name: String,
    /// Optional handle; defaults to a slug of the name.
    #[serde(default)]
    pub handle: Option<String>,
    /// Free-form extension metadata.
    #[serde(default)]
    pub metadata: Metadata,
}

/// Input for updating a navigation menu. Omitted fields keep their value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateNavigationInput {
    /// Replacement name.
    #[serde(default)]
    pub name: Option<String>,
    /// Replacement handle.
    #[serde(default)]
    pub handle: Option<String>,
    /// Replacement extension metadata.
    #[serde(default)]
    pub metadata: Option<Metadata>,
}

/// Input describing a navigation item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavigationItemInput {
    /// Parent item; `None` makes the item a root. During updates this replaces
    /// the current parent.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Display title.
    pub title: String,
    /// Destination URL.
    pub url: String,
    /// Sort rank; defaults to `0` on creation and keeps the current rank on
    /// update.
    #[serde(default)]
    pub rank: Option<i64>,
    /// Free-form extension metadata.
    #[serde(default)]
    pub metadata: Metadata,
}

/// Input for creating a redirect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateRedirectInput {
    /// Source path; must start with `/`.
    pub from_path: String,
    /// Destination path.
    pub to_path: String,
    /// HTTP status code; one of 301, 302, 307 or 308. Defaults to 301.
    #[serde(default)]
    pub status_code: Option<i64>,
}

/// Validates a page title.
pub(crate) fn validate_page_title(title: &str) -> Result<(), AppError> {
    validate_length(title, "title", MAX_TITLE_LENGTH)
}

/// Resolves an optional handle, defaulting to a slug of `source`.
pub(crate) fn resolve_handle(source: &str, handle: Option<String>) -> Result<String, AppError> {
    let handle = match handle {
        Some(handle) => handle,
        None => slugify(source),
    };
    validate_handle(&handle, "handle")?;
    Ok(handle)
}

/// Validates that a body or SEO payload is an object or null and within bounds.
pub(crate) fn validate_page_content(body: &Value, seo: &Value) -> Result<(), AppError> {
    validate_json_object(body, "body")?;
    validate_json_object(seo, "seo")?;

    let serialized = serde_json::to_string(body)
        .map_err(|error| AppError::internal(format!("could not serialize body: {error}")))?;
    if serialized.len() > MAX_BODY_BYTES {
        return Err(AppError::validation(format!(
            "body must not exceed {MAX_BODY_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_json_object(value: &Value, field: &str) -> Result<(), AppError> {
    if value.is_object() || value.is_null() {
        Ok(())
    } else {
        Err(AppError::validation(format!(
            "{field} must be a JSON object or null"
        )))
    }
}

/// Validates a redirect status code.
pub(crate) fn validate_redirect_status(status_code: i64) -> Result<(), AppError> {
    if matches!(status_code, 301 | 302 | 307 | 308) {
        Ok(())
    } else {
        Err(AppError::validation(
            "status_code must be one of 301, 302, 307 or 308",
        ))
    }
}

/// Generates an opaque identifier with the given prefix.
pub(crate) fn generate_id(prefix: &str) -> String {
    let raw = PageId::new();
    let suffix = raw.as_str().trim_start_matches("page_");
    format!("{prefix}_{suffix}")
}
