//! Server-side resolution of storefront data bindings.
//!
//! Pages store references, never live commerce data. This crate turns the
//! [`BindingRequest`]s collected from a page body into a [`DataBag`] keyed by
//! block identifier. Each binding fragment is cached independently for a short
//! TTL, and cache keys always include the store identifier so entries can never
//! cross stores.
//!
//! Resolution degrades rather than fails for references that are missing,
//! unpublished in production mode, or deleted. A binding that fails with any
//! other error is logged and skipped so the remaining fragments still reach
//! the runtime; the bag as a whole never fails because of one bad reference.
//! The exception is a page where every binding failed with a real error:
//! that points at an unavailable data layer, so the error is surfaced instead
//! of silently rendering an empty page.
//! Concurrency is bounded so one page cannot fan out unbounded database work.

mod card;

use std::collections::BTreeMap;
use std::time::Duration;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CurrencyCode, NavigationId, PageId, Pagination};
use fastcart_module_catalog::{CatalogService, ProductFilter, ProductSort, ProductStatus};
use fastcart_module_content::{ContentService, PageStatus};
use fastcart_module_inquiry::InquiryFormService;
use fastcart_storefront::page::MAX_BINDING_LIMIT;
use fastcart_storefront::{BindingRequest, DataBag, DataBinding};
use futures::StreamExt;
use futures::stream;
use moka::future::Cache;
use serde_json::{Map, Value, json};

use card::build_product_cards;

/// Default lifetime of one cached binding fragment.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(30);

/// Number of products used when a binding does not specify a limit.
pub const DEFAULT_BINDING_LIMIT: u32 = 8;

/// Maximum number of bindings resolved concurrently for one page.
pub const MAX_CONCURRENT_BINDINGS: usize = 8;

/// Maximum number of entries retained by one resolver cache.
const CACHE_CAPACITY: u64 = 2_048;

/// Publication mode used while resolving a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataMode {
    /// Storefront mode: only published, active resources are visible.
    Production,
    /// Editor preview mode: drafts and unpublished resources are visible.
    Preview,
}

impl DataMode {
    /// Returns the stable mode name used in cache keys.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Preview => "preview",
        }
    }
}

/// Request-scoped inputs that affect data resolution.
#[derive(Clone, Debug)]
pub struct DataResolutionContext {
    /// Currency prices are selected in.
    pub currency: CurrencyCode,
    /// Sales channel scoping the request, when known.
    pub sales_channel_id: Option<String>,
    /// Customer group used for price list eligibility, when known.
    pub customer_group_id: Option<String>,
    /// Publication mode.
    pub mode: DataMode,
    /// Evaluation timestamp in Unix seconds.
    pub now: i64,
}

/// Resolves page bindings into storefront data fragments.
///
/// The resolver is cheap to clone and safe to share: the underlying Moka cache
/// is shared by every clone.
#[derive(Clone)]
pub struct StorefrontDataResolver {
    cache: Cache<String, Value>,
}

impl Default for StorefrontDataResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl StorefrontDataResolver {
    /// Creates a resolver with [`DEFAULT_CACHE_TTL`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: build_cache(DEFAULT_CACHE_TTL),
        }
    }

    /// Replaces the cache with one using the supplied fragment TTL.
    #[must_use]
    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache = build_cache(ttl);
        self
    }

    /// Resolves bindings with bounded concurrency and merges fragments by block
    /// id.
    ///
    /// Bindings that target the same block are merged into one JSON object.
    /// Later bindings overwrite earlier ones when they produce the same
    /// semantic key. A binding that fails for a reason other than "not found"
    /// is logged and skipped so the remaining fragments are still returned.
    /// If every binding fails with a real error the resolver returns it,
    /// because that indicates an unavailable data layer rather than a handful
    /// of dangling references.
    pub async fn resolve(
        &self,
        db: &StoreDatabase,
        bindings: &[BindingRequest],
        context: &DataResolutionContext,
    ) -> Result<DataBag, AppError> {
        // Own each request so the buffered futures do not borrow the input
        // slice; borrowed async closures here fail higher-ranked lifetime
        // inference when the resolver future is type-checked through the Axum
        // handler.
        let fragments = stream::iter(bindings.iter().cloned())
            .map(|request| async move { self.resolve_binding(db, &request, context).await })
            .buffered(MAX_CONCURRENT_BINDINGS)
            .collect::<Vec<_>>()
            .await;

        let mut merged: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
        let mut first_error: Option<AppError> = None;
        let mut failures = 0usize;
        for result in fragments {
            match result {
                Ok((block_id, fragment)) => {
                    let entry = merged.entry(block_id).or_default();
                    if let Value::Object(object) = fragment {
                        entry.extend(object);
                    }
                }
                Err(error) if error.is_not_found() => {
                    tracing::debug!(error = %error, "skipping missing storefront binding");
                }
                Err(error) => {
                    tracing::warn!(error = %error, "skipping failed storefront binding");
                    failures += 1;
                    first_error.get_or_insert(error);
                }
            }
        }
        if failures == bindings.len()
            && let Some(error) = first_error
        {
            return Err(error);
        }
        Ok(merged
            .into_iter()
            .map(|(block_id, object)| (block_id, Value::Object(object)))
            .collect())
    }

    async fn resolve_binding(
        &self,
        db: &StoreDatabase,
        request: &BindingRequest,
        context: &DataResolutionContext,
    ) -> Result<(String, Value), AppError> {
        let key = cache_key(db, &request.binding, context)?;
        let fragment = self
            .cache
            .try_get_with(key, resolve_fragment(db, &request.binding, context))
            .await
            .map_err(|error| (*error).clone())?;
        Ok((request.block_id.clone(), fragment))
    }
}

fn build_cache(ttl: Duration) -> Cache<String, Value> {
    Cache::builder()
        .max_capacity(CACHE_CAPACITY)
        .time_to_live(ttl)
        .build()
}

fn cache_key(
    db: &StoreDatabase,
    binding: &DataBinding,
    context: &DataResolutionContext,
) -> Result<String, AppError> {
    let binding = serde_json::to_value(binding)
        .map_err(|error| AppError::internal(format!("could not serialize binding: {error}")))?;
    let key = json!([
        db.store_id().as_str(),
        context.mode.as_str(),
        context.currency.as_str(),
        context.sales_channel_id,
        context.customer_group_id,
        binding,
    ]);
    Ok(key.to_string())
}

async fn resolve_fragment(
    db: &StoreDatabase,
    binding: &DataBinding,
    context: &DataResolutionContext,
) -> Result<Value, AppError> {
    match binding {
        DataBinding::Collection { id, limit } => resolve_collection(db, id, *limit, context).await,
        DataBinding::Product { id } => resolve_product(db, id, context).await,
        DataBinding::ProductList {
            collection_id,
            tag,
            sort,
            limit,
        } => {
            resolve_product_list(
                db,
                collection_id.as_deref(),
                tag.as_deref(),
                sort.as_deref(),
                *limit,
                context,
            )
            .await
        }
        DataBinding::Page { id } => resolve_page(db, id, context).await,
        DataBinding::Navigation { id } => resolve_navigation(db, id).await,
        DataBinding::Asset { .. } => Ok(json!({ "asset": null })),
        DataBinding::Search { query, limit } => resolve_search(db, query, *limit, context).await,
        DataBinding::InquiryForm { handle } => resolve_inquiry_form(db, handle, context).await,
    }
}

async fn resolve_collection(
    db: &StoreDatabase,
    id: &str,
    limit: Option<u32>,
    context: &DataResolutionContext,
) -> Result<Value, AppError> {
    let catalog = CatalogService::new(db.clone());
    let collection = match catalog.get_collection(id).await {
        Ok(collection) => collection,
        Err(error) if error.is_not_found() => {
            return Ok(json!({ "collection": null, "products": [] }));
        }
        Err(error) => return Err(error),
    };
    if context.mode == DataMode::Production && !collection.is_published {
        tracing::debug!(collection_id = id, "hidden unpublished collection");
        return Ok(json!({ "collection": null, "products": [] }));
    }

    let products = catalog
        .list_products(
            ProductFilter {
                status: (context.mode == DataMode::Production).then_some(ProductStatus::Active),
                collection_id: Some(id.to_owned()),
                sales_channel_ids: channel_filter(context),
                include_deleted: false,
                ..ProductFilter::default()
            },
            Pagination {
                limit: binding_limit(limit),
                offset: 0,
            },
        )
        .await?;
    let cards = build_product_cards(db, &products.data, context).await?;
    Ok(json!({
        "collection": {
            "id": collection.id,
            "title": collection.title,
            "handle": collection.handle,
            "description": collection.description,
        },
        "products": cards,
    }))
}

async fn resolve_product(
    db: &StoreDatabase,
    id: &str,
    context: &DataResolutionContext,
) -> Result<Value, AppError> {
    let product = match CatalogService::new(db.clone()).get_product(id).await {
        Ok(product) => product,
        Err(error) if error.is_not_found() => return Ok(json!({ "product": null })),
        Err(error) => return Err(error),
    };
    if context.mode == DataMode::Production && product.status != ProductStatus::Active {
        tracing::debug!(product_id = id, "hidden inactive product");
        return Ok(json!({ "product": null }));
    }

    let card = build_product_cards(db, std::slice::from_ref(&product), context)
        .await?
        .into_iter()
        .next();
    Ok(json!({ "product": card }))
}

async fn resolve_product_list(
    db: &StoreDatabase,
    collection_id: Option<&str>,
    tag: Option<&str>,
    sort: Option<&str>,
    limit: Option<u32>,
    context: &DataResolutionContext,
) -> Result<Value, AppError> {
    // A list scoped to a collection inherits the collection's visibility. In
    // production an unpublished or deleted collection hides its products even
    // when those products remain active.
    if let Some(collection_id) = collection_id {
        let catalog = CatalogService::new(db.clone());
        match catalog.get_collection(collection_id).await {
            Ok(collection) if context.mode == DataMode::Preview || collection.is_published => {}
            Ok(_) => return Ok(json!({ "products": [] })),
            Err(error) if error.is_not_found() => return Ok(json!({ "products": [] })),
            Err(error) => return Err(error),
        }
    }
    let sort = sort.and_then(ProductSort::parse);
    let products = CatalogService::new(db.clone())
        .list_products_sorted(
            ProductFilter {
                status: (context.mode == DataMode::Production).then_some(ProductStatus::Active),
                collection_id: collection_id.map(str::to_owned),
                tag_value: tag.map(str::to_owned),
                sales_channel_ids: channel_filter(context),
                include_deleted: false,
                ..ProductFilter::default()
            },
            sort,
            Pagination {
                limit: binding_limit(limit),
                offset: 0,
            },
        )
        .await?;
    let cards = build_product_cards(db, &products.data, context).await?;
    Ok(json!({ "products": cards }))
}

async fn resolve_search(
    db: &StoreDatabase,
    query: &str,
    limit: Option<u32>,
    context: &DataResolutionContext,
) -> Result<Value, AppError> {
    let products = CatalogService::new(db.clone())
        .list_products(
            ProductFilter {
                status: (context.mode == DataMode::Production).then_some(ProductStatus::Active),
                query: Some(query.to_owned()),
                sales_channel_ids: channel_filter(context),
                include_deleted: false,
                ..ProductFilter::default()
            },
            Pagination {
                limit: binding_limit(limit),
                offset: 0,
            },
        )
        .await?;
    let cards = build_product_cards(db, &products.data, context).await?;
    Ok(json!({ "products": cards }))
}

async fn resolve_page(
    db: &StoreDatabase,
    id: &str,
    context: &DataResolutionContext,
) -> Result<Value, AppError> {
    let page = match ContentService::new(db.clone())
        .get_page(&PageId::from_string(id))
        .await
    {
        Ok(page) => page,
        Err(error) if error.is_not_found() => return Ok(json!({ "page": null })),
        Err(error) => return Err(error),
    };
    if context.mode == DataMode::Production && page.status != PageStatus::Published {
        tracing::debug!(page_id = id, "hidden unpublished page");
        return Ok(json!({ "page": null }));
    }
    Ok(json!({
        "page": {
            "id": page.id.as_str(),
            "title": page.title,
            "handle": page.handle,
            "seo": page.seo,
        },
    }))
}

/// Resolves a published inquiry form by handle.
///
/// Production mode only ever sees published forms because
/// [`InquiryFormService::published_by_handle`] enforces publication. Preview
/// and editor modes also return the published revision; there is no separate
/// draft preview yet.
async fn resolve_inquiry_form(
    db: &StoreDatabase,
    handle: &str,
    _context: &DataResolutionContext,
) -> Result<Value, AppError> {
    let published = InquiryFormService::new(db.clone())
        .published_by_handle(handle)
        .await?;
    let settings = json!({
        "submit_label": published.revision.settings.submit_label,
        "success_message": published.revision.settings.success_message,
        "redirect_url": published.revision.settings.redirect_url,
        "consent_text": published.revision.settings.consent_text,
        "consent_required": published.revision.settings.consent_required,
        "allow_attachments": published.revision.settings.allow_attachments,
    });
    Ok(json!({
        "form": {
            "handle": published.form.handle,
            "version": published.form.published_version.unwrap_or(published.revision.version),
            "fields": published.revision.fields,
            "settings": settings,
        }
    }))
}

async fn resolve_navigation(db: &StoreDatabase, id: &str) -> Result<Value, AppError> {
    match ContentService::new(db.clone())
        .get_navigation_tree_by_id(&NavigationId::from_string(id))
        .await
    {
        Ok(tree) => Ok(json!({ "navigation": tree })),
        Err(error) if error.is_not_found() => Ok(json!({ "navigation": null })),
        Err(error) => Err(error),
    }
}

fn binding_limit(limit: Option<u32>) -> i64 {
    i64::from(
        limit
            .unwrap_or(DEFAULT_BINDING_LIMIT)
            .clamp(1, MAX_BINDING_LIMIT),
    )
}

fn channel_filter(context: &DataResolutionContext) -> Option<Vec<String>> {
    context
        .sales_channel_id
        .clone()
        .map(|channel_id| vec![channel_id])
}

#[cfg(test)]
mod tests;
