//! Internal plugin API reached through the shared bridge.
//!
//! Every route is authenticated with a short-lived context token minted by the
//! host. The token binds the plugin, the store, the acting principal and the
//! granted scopes; request bodies and responses are size-capped.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use fastcart_http::ApiError;
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CustomerId, OrderId, Pagination, StoreId, now_timestamp};
use fastcart_module_catalog::{CatalogService, ProductFilter, ProductStatus};
use fastcart_module_content::{ContentService, PageStatus};
use fastcart_module_customer::CustomerService;
use fastcart_module_order::{OrderFilter, OrderService};
use fastcart_plugin::{
    ContextClaims, EgressRequest, PluginError, PluginManifest, PluginService, TokenActor,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppContext;

const MAX_KV_VALUE: usize = 256 * 1024;
const MAX_KV_TOTAL: u64 = 10 * 1024 * 1024;
const MAX_KV_KEYS: i64 = 100_000;
const MAX_KV_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;

/// Builds the internal plugin API router.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/context", get(get_context))
        .route("/products", get(list_products))
        .route("/products/{id}", get(get_product))
        .route("/orders", get(list_orders))
        .route("/orders/{id}", get(get_order))
        .route("/customers/{id}", get(get_customer))
        .route("/customer/me", get(get_current_customer))
        .route("/pages/{handle}", get(get_page))
        .route(
            "/kv/{namespace}/{key}",
            get(kv_get).put(kv_put).delete(kv_delete),
        )
        .route("/secrets/{name}", get(get_secret))
        .route("/egress", post(egress))
        .route("/log", post(log))
}

/// Verifies the context token and the installation state.
pub async fn auth_middleware(
    State(context): State<Arc<AppContext>>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = bearer(&request).ok_or_else(|| ApiError::new(AppError::Unauthorized))?;
    let unsigned = fastcart_plugin::TokenSigner::inspect(token)
        .map_err(|_| ApiError::new(AppError::Unauthorized))?;
    let claims = context
        .plugin_tokens
        .derive(&unsigned.store, &unsigned.sub, &unsigned.digest)
        .verify(token, now_timestamp())
        .map_err(|_| ApiError::new(AppError::Unauthorized))?;

    if let Some(call_id) = request
        .headers()
        .get("x-fastcart-call-id")
        .and_then(|value| value.to_str().ok())
    {
        if call_id.is_empty() || call_id.len() > 128 {
            return Err(ApiError::validation("invalid call id"));
        }
        let key = format!("{}:{call_id}", claims.jti);
        if context.plugin_call_replay.get(&key).await.is_some() {
            return Err(ApiError::new(AppError::Unauthorized));
        }
        context.plugin_call_replay.insert(key, ()).await;
    }

    let database = database(&context, &claims).await?;
    let installation: Option<(String, Option<String>, i64)> = sqlx::query_as(
        "SELECT digest, previous_digest, enabled FROM plugin_installations WHERE plugin_id = ?",
    )
    .bind(&claims.sub)
    .fetch_optional(database.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    let Some((digest, previous, enabled)) = installation else {
        return Err(ApiError::new(AppError::Forbidden));
    };
    if enabled != 1 || (claims.digest != digest && previous.as_deref() != Some(&claims.digest)) {
        return Err(ApiError::new(AppError::Forbidden));
    }

    request.extensions_mut().insert(claims);
    Ok(next.run(request).await)
}

fn bearer(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .or_else(|| {
            request
                .headers()
                .get("x-fastcart-context-token")
                .and_then(|value| value.to_str().ok())
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn require_scope(claims: &ContextClaims, scope: &str) -> Result<(), ApiError> {
    if claims.scopes.iter().any(|granted| granted == scope) {
        Ok(())
    } else {
        Err(ApiError::new(AppError::Forbidden))
    }
}

async fn database(context: &AppContext, claims: &ContextClaims) -> Result<StoreDatabase, ApiError> {
    context
        .databases
        .database(&StoreId::from_string(claims.store.clone()))
        .await
        .map_err(|error| ApiError::new(error.into()))
}

async fn installation_manifest(
    database: &StoreDatabase,
    plugin_id: &str,
) -> Result<(PluginManifest, Value), ApiError> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT manifest_json, settings_json FROM plugin_installations WHERE plugin_id = ?",
    )
    .bind(plugin_id)
    .fetch_optional(database.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    let (manifest_json, settings_json) =
        row.ok_or_else(|| ApiError::new(AppError::not_found("plugin", plugin_id)))?;
    let manifest = PluginManifest::parse(manifest_json.as_bytes())
        .map_err(|error| ApiError::new(error.into()))?;
    let settings = serde_json::from_str(&settings_json).unwrap_or(Value::Null);
    Ok((manifest, settings))
}

async fn get_context(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&claims, "core.context")?;
    let database = database(&context, &claims).await?;
    let (_manifest, settings) = installation_manifest(&database, &claims.sub).await?;
    let store_id = StoreId::from_string(claims.store.clone());
    let store = context
        .platform
        .get_store(&store_id)
        .await
        .map_err(|error| ApiError::new(error.into()))?
        .ok_or_else(|| ApiError::new(AppError::not_found("store", claims.store.clone())))?;
    Ok(Json(json!({
        "plugin": claims.sub,
        "store": {
            "id": store.id.as_str(),
            "locale": store.locale,
            "currency": store.currency_code,
            "timezone": store.timezone,
        },
        "scopes": claims.scopes,
        "settings": settings,
    })))
}

fn order_summary(order: &fastcart_module_order::Order) -> Value {
    json!({
        "id": order.id.as_str(),
        "display_number": order.display_number,
        "customer_id": order.customer_id.as_ref().map(|id| id.as_str()),
        "currency": order.currency_code,
        "status": order.status.as_str(),
        "payment_status": order.payment_status.as_str(),
        "fulfillment_status": order.fulfillment_status.as_str(),
        "refund_status": order.refund_status.as_str(),
        "subtotal": order.subtotal,
        "discount_total": order.discount_total,
        "shipping_total": order.shipping_total,
        "tax_total": order.tax_total,
        "total": order.total,
        "created_at": order.created_at,
        "updated_at": order.updated_at,
        "canceled_at": order.canceled_at,
        "items": order.items.iter().map(|item| json!({
            "id": item.id.as_str(),
            "product_id": item.product_id.as_ref().map(|id| id.as_str()),
            "variant_id": item.variant_id.as_ref().map(|id| id.as_str()),
            "product_title": item.product_title,
            "variant_title": item.variant_title,
            "sku": item.sku,
            "quantity": item.quantity,
            "unit_price": item.unit_price,
            "total": item.total,
        })).collect::<Vec<_>>(),
    })
}

async fn list_orders(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&claims, "orders.read")?;
    let database = database(&context, &claims).await?;
    let limit = query.limit.clamp(1, 50);
    let page = OrderService::new(database)
        .list(
            OrderFilter::default(),
            Pagination {
                limit,
                offset: query.offset.max(0),
            },
        )
        .await
        .map_err(ApiError::new)?;
    let orders: Vec<Value> = page.data.iter().map(order_summary).collect();
    Ok(Json(json!({ "orders": orders, "count": page.count })))
}

async fn get_order(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&claims, "orders.read")?;
    let database = database(&context, &claims).await?;
    let order = OrderService::new(database)
        .get(&OrderId::from_string(id))
        .await
        .map_err(ApiError::new)?;
    Ok(Json(order_summary(&order)))
}

fn customer_summary(customer: &fastcart_module_customer::Customer) -> Value {
    json!({
        "id": customer.id.as_str(),
        "first_name": customer.first_name,
        "last_name": customer.last_name,
        "has_account": customer.has_account,
        "status": customer.status.as_str(),
        "created_at": customer.created_at,
        "updated_at": customer.updated_at,
    })
}

async fn get_customer(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&claims, "customers.read")?;
    let database = database(&context, &claims).await?;
    let customer = CustomerService::new(database)
        .get_customer(&CustomerId::from_string(id))
        .await
        .map_err(ApiError::new)?;
    Ok(Json(customer_summary(&customer)))
}

async fn get_current_customer(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&claims, "customer.read")?;
    let TokenActor::Storefront {
        id: Some(customer_id),
    } = &claims.actor
    else {
        return Err(ApiError::new(AppError::Unauthorized));
    };
    let database = database(&context, &claims).await?;
    let customer = CustomerService::new(database)
        .get_customer(&CustomerId::from_string(customer_id.clone()))
        .await
        .map_err(ApiError::new)?;
    Ok(Json(customer_summary(&customer)))
}

async fn get_page(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Path(handle): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&claims, "content.read")?;
    let database = database(&context, &claims).await?;
    let page = ContentService::new(database)
        .get_page_by_handle(&handle)
        .await
        .map_err(ApiError::new)?;
    if page.status != PageStatus::Published {
        return Err(ApiError::new(AppError::not_found("page", handle)));
    }
    Ok(Json(json!({
        "id": page.id.as_str(),
        "title": page.title,
        "handle": page.handle,
        "page_type": page.page_type,
        "body": page.published_body.unwrap_or(page.body),
        "seo": page.seo,
        "published_at": page.published_at,
    })))
}

#[derive(Clone, Debug, Deserialize)]
pub struct PageQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    20
}

async fn list_products(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&claims, "catalog.read")?;
    let database = database(&context, &claims).await?;
    let limit = query.limit.clamp(1, 100);
    let page = CatalogService::new(database)
        .list_products(
            ProductFilter {
                status: Some(ProductStatus::Active),
                ..ProductFilter::default()
            },
            Pagination {
                limit,
                offset: query.offset.max(0),
            },
        )
        .await
        .map_err(ApiError::new)?;
    Ok(Json(json!({ "products": page.data, "count": page.count })))
}

async fn get_product(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&claims, "catalog.read")?;
    let database = database(&context, &claims).await?;
    let product = CatalogService::new(database)
        .get_product(&id)
        .await
        .map_err(ApiError::new)?;
    if product.status != ProductStatus::Active {
        return Err(ApiError::new(AppError::not_found("product", id)));
    }
    Ok(Json(json!(product)))
}

fn valid_segment(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

async fn kv_get(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Path((namespace, key)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    require_scope(&claims, "kv.read")?;
    if !valid_segment(&namespace, 64) || !valid_segment(&key, 512) {
        return Err(ApiError::validation("invalid KV namespace or key"));
    }
    let database = database(&context, &claims).await?;
    let value: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT value FROM plugin_kv WHERE plugin_id = ? AND namespace = ? AND key = ? \
         AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(&claims.sub)
    .bind(&namespace)
    .bind(&key)
    .bind(now_timestamp())
    .fetch_optional(database.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    let Some(value) = value else {
        return Err(ApiError::new(AppError::not_found("kv", key)));
    };
    let mut response = Response::new(axum::body::Body::from(value));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    Ok(response)
}

/// Optional expiry for one KV write.
#[derive(Clone, Debug, Deserialize)]
pub struct KvPutQuery {
    /// Time to live in seconds.
    #[serde(default)]
    pub ttl: Option<i64>,
}

async fn kv_put(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Path((namespace, key)): Path<(String, String)>,
    Query(query): Query<KvPutQuery>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    require_scope(&claims, "kv.write")?;
    if !valid_segment(&namespace, 64) || !valid_segment(&key, 512) {
        return Err(ApiError::validation("invalid KV namespace or key"));
    }
    if body.len() > MAX_KV_VALUE {
        return Err(ApiError::validation(format!(
            "KV value exceeds {MAX_KV_VALUE} bytes"
        )));
    }
    let now = now_timestamp();
    let expires_at = match query.ttl {
        None => None,
        Some(ttl) if (1..=MAX_KV_TTL_SECONDS).contains(&ttl) => Some(now + ttl),
        Some(_) => {
            return Err(ApiError::validation(format!(
                "KV ttl must be between 1 and {MAX_KV_TTL_SECONDS} seconds"
            )));
        }
    };
    let database = database(&context, &claims).await?;
    let (count, total): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(LENGTH(value)), 0) FROM plugin_kv \
         WHERE plugin_id = ? AND NOT (namespace = ? AND key = ?) \
         AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(&claims.sub)
    .bind(&namespace)
    .bind(&key)
    .bind(now)
    .fetch_one(database.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    if count >= MAX_KV_KEYS || total.max(0) as u64 + body.len() as u64 > MAX_KV_TOTAL {
        return Err(ApiError::new(AppError::Conflict(
            "plugin KV quota exceeded".to_owned(),
        )));
    }
    sqlx::query(
        "INSERT INTO plugin_kv (plugin_id, namespace, key, value, expires_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(plugin_id, namespace, key) DO UPDATE SET value = excluded.value, \
         expires_at = excluded.expires_at, updated_at = excluded.updated_at",
    )
    .bind(&claims.sub)
    .bind(&namespace)
    .bind(&key)
    .bind(body.to_vec())
    .bind(expires_at)
    .bind(now)
    .execute(database.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    context
        .metrics
        .set_plugin_kv_bytes(total.max(0) as u64 + body.len() as u64);
    Ok(StatusCode::NO_CONTENT)
}

async fn kv_delete(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Path((namespace, key)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    require_scope(&claims, "kv.write")?;
    if !valid_segment(&namespace, 64) || !valid_segment(&key, 512) {
        return Err(ApiError::validation("invalid KV namespace or key"));
    }
    let database = database(&context, &claims).await?;
    sqlx::query("DELETE FROM plugin_kv WHERE plugin_id = ? AND namespace = ? AND key = ?")
        .bind(&claims.sub)
        .bind(&namespace)
        .bind(&key)
        .execute(database.pool())
        .await
        .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_secret(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    require_scope(&claims, "secrets.read")?;
    let database = database(&context, &claims).await?;
    let service = PluginService::new(database, context.plugin_artifacts.clone())
        .with_secret_cipher(context.plugin_secret_cipher.clone());
    let value = service
        .secret(&claims.sub, &name)
        .await
        .map_err(|error| match error {
            PluginError::SecretNotFound(_) => {
                ApiError::new(AppError::not_found("plugin secret", name.clone()))
            }
            other => ApiError::new(other.into()),
        })?;
    let mut response = Response::new(axum::body::Body::from(value));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    Ok(response)
}

#[derive(Clone, Debug, Deserialize)]
pub struct EgressBody {
    pub url: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_base64: Option<String>,
}

fn default_method() -> String {
    "GET".to_owned()
}

async fn egress(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Json(request): Json<EgressBody>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&claims, "network.egress")?;
    let database = database(&context, &claims).await?;
    let (manifest, _settings) = installation_manifest(&database, &claims.sub).await?;
    let response = context
        .plugin_egress
        .execute(EgressRequest {
            plugin_id: claims.sub.clone(),
            store_id: claims.store.clone(),
            url: request.url,
            method: request.method,
            headers: request.headers,
            body: match (request.body_base64, request.body) {
                (Some(encoded), _) => BASE64
                    .decode(encoded)
                    .map_err(|_| ApiError::validation("invalid base64 body"))?,
                (None, Some(body)) => body.into_bytes(),
                (None, None) => Vec::new(),
            },
            allowlist: manifest.capabilities.network,
            forward_auth: manifest.capabilities.network_auth,
        })
        .await
        .map_err(|error| match error {
            fastcart_plugin::EgressError::NotAllowed
            | fastcart_plugin::EgressError::BlockedAddress => ApiError::new(AppError::Forbidden),
            fastcart_plugin::EgressError::Quota => {
                ApiError::new(AppError::Conflict("egress quota exhausted".to_owned()))
            }
            fastcart_plugin::EgressError::TooLarge => {
                ApiError::validation("egress payload exceeds a limit")
            }
            fastcart_plugin::EgressError::Invalid(message) => ApiError::validation(message),
            fastcart_plugin::EgressError::Http(message) => {
                ApiError::new(AppError::ExternalService(message))
            }
        })?;
    context
        .metrics
        .add_plugin_egress_bytes(response.body.len() as u64);
    Ok(Json(json!({
        "status": response.status,
        "content_type": response.content_type,
        "body": BASE64.encode(response.body),
    })))
}

#[derive(Clone, Debug, Deserialize)]
pub struct LogBody {
    #[serde(default = "default_level")]
    pub level: String,
    pub message: String,
    #[serde(default)]
    pub fields: Option<Value>,
}

fn default_level() -> String {
    "info".to_owned()
}

async fn log(
    State(context): State<Arc<AppContext>>,
    Extension(claims): Extension<ContextClaims>,
    Json(body): Json<LogBody>,
) -> Result<StatusCode, ApiError> {
    require_scope(&claims, "core.context")?;
    let level = match body.level.as_str() {
        "debug" | "info" | "warn" | "error" => body.level,
        _ => return Err(ApiError::validation("invalid log level")),
    };
    if body.message.len() > 4096 {
        return Err(ApiError::validation("log message exceeds 4096 bytes"));
    }
    let database = database(&context, &claims).await?;
    sqlx::query(
        "INSERT INTO plugin_logs (plugin_id, level, message, fields_json, created_at)          VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&claims.sub)
    .bind(level)
    .bind(&body.message)
    .bind(body.fields.as_ref().map(ToString::to_string))
    .bind(now_timestamp())
    .execute(database.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    Ok(StatusCode::NO_CONTENT)
}
