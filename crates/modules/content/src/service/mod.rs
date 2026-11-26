//! SQLite-backed application service for content pages and navigation.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::{NewAuditOutbox, StoreDatabase, append_audit_outbox, database_error};
use fastcart_kernel::{
    AppError, Metadata, NavigationId, PageId, PageResult, Pagination, Timestamp, now_timestamp,
    validate_handle, validate_length, validate_metadata,
};
use fastcart_storefront::{PageBody, PageThemeRef, StorefrontError, ThemeEditorManifest};
use serde_json::Value;
use sqlx::FromRow;

use crate::domain::{
    CreateNavigationInput, CreatePageInput, CreateRedirectInput, MAX_PAGE_TYPE_LENGTH,
    MAX_REDIRECT_PATH_LENGTH, MAX_TITLE_LENGTH, MAX_URL_LENGTH, Navigation, NavigationItem,
    NavigationItemInput, NavigationTree, NavigationTreeItem, Page, PageRevisionOrigin, PageRoute,
    PageStatus, PageVersion, PageVersionKind, Redirect, UpdateNavigationInput, UpdatePageInput,
    generate_id, resolve_handle, validate_page_content, validate_page_title,
    validate_redirect_status,
};

const PAGE_COLUMNS: &str = "id, title, handle, status, page_type, body, published_body, \
     published_title, published_handle, published_page_type, published_seo, published_metadata, \
     seo, metadata, revision, published_at, created_at, updated_at, deleted_at";

const NAVIGATION_COLUMNS: &str = "id, name, handle, metadata, created_at, updated_at";

const NAVIGATION_ITEM_COLUMNS: &str = "id, navigation_id, parent_id, title, url, rank, metadata";

mod navigation;
mod navigation_items;
mod pages;
mod publishing;
mod reads;
mod redirects;
mod translations;

pub use translations::{
    MAX_TRANSLATION_BYTES, MAX_TRANSLATION_RESOURCE_ID_LENGTH, ResolvedTranslation,
    TRANSLATION_RESOURCE_TYPES, Translation, UpsertTranslationInput, resolve_translation,
};

/// A page whose stored body does not validate against a candidate theme.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct PageValidationIssue {
    /// Page identifier.
    pub page_id: PageId,
    /// Page handle at validation time.
    pub handle: String,
    /// Human-readable validation failure.
    pub error: String,
}

/// Application service for store pages, page versions, navigation and redirects.
#[derive(Clone, Debug)]
pub struct ContentService {
    db: StoreDatabase,
    editor_manifest: Option<ThemeEditorManifest>,
    theme_ref: Option<PageThemeRef>,
    audit_actor: Option<String>,
}

impl ContentService {
    /// Creates a service bound to one store database without a theme editor
    /// manifest. Page bodies are still structurally validated.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self {
            db,
            editor_manifest: None,
            theme_ref: None,
            audit_actor: None,
        }
    }

    /// Creates a service bound to one store database, optionally validating
    /// page bodies against the active theme editor manifest.
    #[must_use]
    pub fn with_editor_manifest(
        db: StoreDatabase,
        editor_manifest: Option<ThemeEditorManifest>,
    ) -> Self {
        Self {
            db,
            editor_manifest,
            theme_ref: None,
            audit_actor: None,
        }
    }

    /// Records the active theme reference stamped into page bodies.
    ///
    /// When set, new and updated bodies carry this reference and stored bodies
    /// whose reference disagrees are rejected during validation.
    #[must_use]
    pub fn with_theme_ref(mut self, theme_ref: Option<PageThemeRef>) -> Self {
        self.theme_ref = theme_ref;
        self
    }

    /// Records content mutations in the store audit outbox.
    ///
    /// When an actor is set, every mutating method appends its audit entry in
    /// the same transaction as the change, so a crash cannot separate them.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit_actor = actor_id;
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
        let Some(actor_id) = self.audit_actor.as_deref() else {
            return Ok(());
        };
        let entry = NewAuditOutbox {
            actor_id: Some(actor_id.to_owned()),
            store_id: Some(self.db.store_id().as_str().to_owned()),
            action: action.to_owned(),
            resource_type: Some(resource_type.to_owned()),
            resource_id: Some(resource_id.to_owned()),
            data,
        };
        append_audit_outbox(&mut **tx, &entry)
            .await
            .map_err(database_error)?;
        Ok(())
    }

    /// Overwrites the body's theme reference with the active theme, when one is
    /// known.
    ///
    /// Empty bodies are promoted to an object so the reference is recorded even
    /// before the first block is authored. Bodies that are neither null nor an
    /// object are left untouched and rejected by structural validation.
    fn stamp_theme_ref(&self, mut body: Value) -> Value {
        let Some(theme_ref) = self.theme_ref.as_ref() else {
            return body;
        };
        if body.is_null() {
            body = Value::Object(serde_json::Map::new());
        }
        if let Some(object) = body.as_object_mut() {
            object.insert(
                "theme".to_owned(),
                serde_json::json!({
                    "id": &theme_ref.id,
                    "version": &theme_ref.version,
                    "schema_version": theme_ref.schema_version,
                }),
            );
        }
        body
    }

    /// Validates a page body structurally and, when a manifest is available,
    /// against the theme editor contract.
    fn validate_body(&self, body: &Value) -> Result<PageBody, AppError> {
        let parsed = PageBody::parse(body).map_err(storefront_error)?;
        if let Some(expected) = self.theme_ref.as_ref()
            && let Some(actual) = parsed.theme.as_ref()
            && (actual.id != expected.id
                || actual.version != expected.version
                || actual.schema_version != expected.schema_version)
        {
            return Err(AppError::validation(format!(
                "page body theme reference {}@{} does not match the active theme {}@{}",
                actual.id, actual.version, expected.id, expected.version
            )));
        }
        if let Some(editor) = self.editor_manifest.as_ref() {
            parsed
                .validate_with_editor(editor)
                .map_err(storefront_error)?;
        }
        Ok(parsed)
    }
}

fn validate_rank(rank: i64) -> Result<(), AppError> {
    if rank < 0 {
        return Err(AppError::validation("rank must not be negative"));
    }
    Ok(())
}

/// Returns true when a body carries no authored content.
fn is_empty_body(body: &Value) -> bool {
    body.is_null() || body.as_object().is_some_and(serde_json::Map::is_empty)
}

/// Validates one stored body against a candidate theme's editor manifest.
///
/// The body's own theme reference is deliberately ignored: it records who
/// authored the body, while validation answers whether the candidate theme can
/// still render it.
fn validate_page_body_for_theme(
    body: &Value,
    editor_manifest: &ThemeEditorManifest,
) -> Result<(), String> {
    let parsed = PageBody::parse(body).map_err(|error| error.to_string())?;
    parsed
        .validate_with_editor(editor_manifest)
        .map_err(|error| error.to_string())
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

fn revision_conflict(current: i64) -> AppError {
    AppError::conflict_details(
        format!("page revision conflict: current revision is {current}"),
        serde_json::json!({ "current_revision": current }),
    )
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

async fn next_page_version(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    page_id: &PageId,
) -> Result<i64, AppError> {
    sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) + 1 FROM page_versions WHERE page_id = ?")
        .bind(page_id.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(database_error)
}

#[allow(clippy::too_many_arguments)]
async fn insert_page_version(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    page_id: &PageId,
    version: i64,
    body: &str,
    created_by: Option<&str>,
    kind: PageVersionKind,
    origin: PageRevisionOrigin,
    created_at: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO page_versions \
         (id, page_id, version, body, created_by, kind, origin, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(generate_id("pver"))
    .bind(page_id.as_str())
    .bind(version)
    .bind(body)
    .bind(created_by)
    .bind(kind.as_str())
    .bind(origin.as_str())
    .bind(created_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

fn serialize_json(value: &Value, field: &str) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::internal(format!("could not serialize {field}: {error}")))
}

fn parse_json(value: &str) -> Value {
    match serde_json::from_str(value) {
        Ok(value) => value,
        Err(_) => Value::Null,
    }
}

fn navigation_tree_from(navigation: Navigation) -> NavigationTree {
    NavigationTree {
        id: navigation.id,
        name: navigation.name,
        handle: navigation.handle,
        metadata: navigation.metadata,
        items: build_navigation_tree(navigation.items),
    }
}

fn build_navigation_tree(items: Vec<NavigationItem>) -> Vec<NavigationTreeItem> {
    let mut children: HashMap<Option<String>, Vec<NavigationItem>> = HashMap::new();
    for item in items {
        children
            .entry(item.parent_id.clone())
            .or_default()
            .push(item);
    }
    let mut visited = HashSet::new();
    take_tree_children(&mut children, &None, &mut visited)
}

fn take_tree_children(
    children: &mut HashMap<Option<String>, Vec<NavigationItem>>,
    parent: &Option<String>,
    visited: &mut HashSet<String>,
) -> Vec<NavigationTreeItem> {
    let Some(mut items) = children.remove(parent) else {
        return Vec::new();
    };
    items.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| left.id.cmp(&right.id))
    });
    items
        .into_iter()
        .filter_map(|item| {
            if !visited.insert(item.id.clone()) {
                return None;
            }
            let nested = take_tree_children(children, &Some(item.id.clone()), visited);
            Some(NavigationTreeItem {
                id: item.id,
                navigation_id: item.navigation_id,
                parent_id: item.parent_id,
                title: item.title,
                url: item.url,
                rank: item.rank,
                metadata: item.metadata,
                children: nested,
            })
        })
        .collect()
}

#[derive(Debug, FromRow)]
struct PageRow {
    id: String,
    title: String,
    handle: String,
    status: String,
    page_type: String,
    body: String,
    published_body: Option<String>,
    published_title: Option<String>,
    published_handle: Option<String>,
    published_page_type: Option<String>,
    published_seo: Option<String>,
    published_metadata: Option<String>,
    seo: String,
    metadata: String,
    revision: i64,
    published_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    deleted_at: Option<i64>,
}

impl PageRow {
    fn into_page(self) -> Page {
        let status = match PageStatus::parse(&self.status) {
            Some(status) => status,
            None => PageStatus::Draft,
        };
        Page {
            id: PageId::from_string(self.id),
            title: self.title,
            handle: self.handle,
            status,
            page_type: self.page_type,
            revision: self.revision,
            body: parse_json(&self.body),
            published_body: self.published_body.as_deref().map(parse_json),
            seo: parse_json(&self.seo),
            metadata: Metadata::from_json_str(&self.metadata),
            published_at: self.published_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
        }
    }

    /// Projects the published snapshot, falling back per field to the live
    /// column for legacy rows that predate a snapshot column.
    fn into_published_page(self) -> Page {
        let status = match PageStatus::parse(&self.status) {
            Some(status) => status,
            None => PageStatus::Draft,
        };
        let body = self
            .published_body
            .as_deref()
            .map_or_else(|| parse_json(&self.body), parse_json);
        let seo = self
            .published_seo
            .as_deref()
            .map_or_else(|| parse_json(&self.seo), parse_json);
        let metadata = self.published_metadata.as_deref().map_or_else(
            || Metadata::from_json_str(&self.metadata),
            Metadata::from_json_str,
        );
        Page {
            id: PageId::from_string(self.id),
            title: self.published_title.unwrap_or(self.title),
            handle: self.published_handle.unwrap_or(self.handle),
            status,
            page_type: self.published_page_type.unwrap_or(self.page_type),
            revision: self.revision,
            body,
            published_body: None,
            seo,
            metadata,
            published_at: self.published_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
        }
    }

    /// Returns true when every published snapshot field equals its live value.
    fn published_matches_live(&self) -> bool {
        self.published_title.as_deref() == Some(self.title.as_str())
            && self.published_handle.as_deref() == Some(self.handle.as_str())
            && self.published_page_type.as_deref() == Some(self.page_type.as_str())
            && self
                .published_body
                .as_deref()
                .is_some_and(|published| parse_json(published) == parse_json(&self.body))
            && self
                .published_seo
                .as_deref()
                .is_some_and(|published| parse_json(published) == parse_json(&self.seo))
            && self
                .published_metadata
                .as_deref()
                .is_some_and(|published| parse_json(published) == parse_json(&self.metadata))
    }
}

#[derive(Debug, FromRow)]
struct PageVersionRow {
    id: String,
    page_id: String,
    version: i64,
    body: String,
    created_by: Option<String>,
    kind: String,
    origin: String,
    created_at: i64,
}

impl PageVersionRow {
    fn into_version(self) -> PageVersion {
        PageVersion {
            id: self.id,
            page_id: PageId::from_string(self.page_id),
            version: self.version,
            body: parse_json(&self.body),
            kind: PageVersionKind::parse(&self.kind).unwrap_or(PageVersionKind::Published),
            origin: PageRevisionOrigin::parse(&self.origin).unwrap_or(PageRevisionOrigin::Manual),
            created_by: self.created_by,
            created_at: self.created_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct NavigationRow {
    id: String,
    name: String,
    handle: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl NavigationRow {
    fn into_navigation(self, items: Vec<NavigationItem>) -> Navigation {
        Navigation {
            id: NavigationId::from_string(self.id),
            name: self.name,
            handle: self.handle,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
            items,
        }
    }
}

#[derive(Debug, FromRow)]
struct NavigationItemRow {
    id: String,
    navigation_id: String,
    parent_id: Option<String>,
    title: String,
    url: String,
    rank: i64,
    metadata: String,
}

impl NavigationItemRow {
    fn into_item(self) -> NavigationItem {
        NavigationItem {
            id: self.id,
            navigation_id: NavigationId::from_string(self.navigation_id),
            parent_id: self.parent_id,
            title: self.title,
            url: self.url,
            rank: self.rank,
            metadata: Metadata::from_json_str(&self.metadata),
        }
    }
}

#[derive(Debug, FromRow)]
struct RedirectRow {
    id: String,
    from_path: String,
    to_path: String,
    status_code: i64,
    created_at: i64,
}

impl RedirectRow {
    fn into_redirect(self) -> Redirect {
        Redirect {
            id: self.id,
            from_path: self.from_path,
            to_path: self.to_path,
            status_code: self.status_code,
            created_at: self.created_at,
        }
    }
}
