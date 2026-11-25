//! Admin plugin installation flow.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::StoreId;
use fastcart_platform::PlatformStore;
use fastcart_plugin::{ContextClaims, TokenActor, TokenSigner};
use flate2::Compression;
use flate2::write::GzEncoder;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tar::{Builder, Header};
use tower::ServiceExt;

const HOST: &str = "shop.example.com";
const RUNTIME_TOKEN: &str = "runtime-secret";

#[path = "admin_plugins/installation.rs"]
mod installation;
#[path = "admin_plugins/runtime.rs"]
mod runtime;
#[path = "admin_plugins/support.rs"]
mod support;
#[path = "admin_plugins/websocket.rs"]
mod websocket;
