//! Store isolation security suite.
//!
//! These tests exercise the isolation guarantees required by the production
//! hardening plan: admin, token, customer, publishable-key, cache, job, event,
//! resolver and internal-surface boundaries. Data is seeded through the module
//! services so each test can assert against real per-store databases.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{
    DatabaseOptions, JobQueue, NewJob, StoreCache, StoreDatabaseManager, append_outbox_event,
    fetch_pending_outbox,
};
use fastcart_kernel::{DomainEventEnvelope, Metadata, StoreId};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput, Product};
use fastcart_module_order::{CreateOrderInput, CreateOrderItemInput, Order, OrderService};
use fastcart_module_store::{
    CreatePublishableApiKeyInput, CreateSalesChannelInput, CreatedPublishableApiKey, RegionService,
    SalesChannel, StoreService,
};
use fastcart_platform::{CreateUserInput, PlatformStore};
use http_body_util::BodyExt;
use tower::ServiceExt;

const PASSWORD: &str = "hunter2hunter2";

#[path = "store_isolation/admin.rs"]
mod admin;
#[path = "store_isolation/customer.rs"]
mod customer;
#[path = "store_isolation/infra.rs"]
mod infra;
#[path = "store_isolation/support.rs"]
mod support;
