//! Storefront SSR fallback and internal bridge security.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use fastcart_api::routes::preview::issue_preview_token;
use fastcart_api::state::{
    CapabilityGrants, InternalSsrCredentials, InternalSsrToken, UniversalStorefront,
};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::{Metadata, StoreContext, now_timestamp};
use fastcart_module_content::{ContentService, CreatePageInput, UpdatePageInput};
use fastcart_module_theme::{RegisterThemeInput, ThemeService};
use fastcart_platform::{CreateStoreInput, PlatformStore, StoreProvisioningService};
use fastcart_storefront::ThemeManifest;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[path = "storefront_ssr/deployment.rs"]
mod deployment;
#[path = "storefront_ssr/discovery.rs"]
mod discovery;
#[path = "storefront_ssr/support.rs"]
mod support;
#[path = "storefront_ssr/universal.rs"]
mod universal;
