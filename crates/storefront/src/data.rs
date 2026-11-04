//! Storefront data bindings and the data bag handed to the runtime.
//!
//! Bindings are stored inside page props as reference objects. Resolution
//! happens on the server on every render; live data is never written to page
//! JSON.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One typed data reference extracted from a page body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum DataBinding {
    /// A collection picker reference.
    Collection {
        /// Collection identifier.
        id: String,
        /// Maximum number of products to include.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// A single product picker reference.
    Product {
        /// Product identifier.
        id: String,
    },
    /// A query-shaped product list.
    ProductList {
        /// Restrict to a collection.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        collection_id: Option<String>,
        /// Restrict to a tag value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        /// Sort order accepted by the catalog resolver.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sort: Option<String>,
        /// Maximum number of products.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// A content page picker reference.
    Page {
        /// Page identifier.
        id: String,
    },
    /// A navigation picker reference.
    Navigation {
        /// Navigation identifier.
        id: String,
    },
    /// An asset picker reference.
    Asset {
        /// Asset identifier.
        id: String,
    },
    /// A free-text product search.
    Search {
        /// Search phrase.
        query: String,
        /// Maximum number of products.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// A published storefront inquiry form reference.
    InquiryForm {
        /// Stable storefront form handle.
        handle: String,
    },
}

impl DataBinding {
    /// Returns the stable data source name.
    #[must_use]
    pub fn source(&self) -> &'static str {
        match self {
            Self::Collection { .. } => "collection",
            Self::Product { .. } => "product",
            Self::ProductList { .. } => "product_list",
            Self::Page { .. } => "page",
            Self::Navigation { .. } => "navigation",
            Self::Asset { .. } => "asset",
            Self::Search { .. } => "search",
            Self::InquiryForm { .. } => "inquiry_form",
        }
    }
}

/// A binding tied to the block field that declared it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingRequest {
    /// Block instance identifier.
    pub block_id: String,
    /// Component prop name.
    pub field: String,
    /// Parsed binding.
    pub binding: DataBinding,
}

/// Resolved data keyed by block instance identifier.
///
/// Each value is a JSON object whose keys are semantic (`collection`,
/// `products`, `product`, `page`, `navigation`). Multiple bindings on one
/// block are merged into the same object.
pub type DataBag = BTreeMap<String, Value>;
