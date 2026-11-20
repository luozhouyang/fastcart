//! Public `/apps/{plugin}` routes forwarded to the plugin host.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use fastcart_http::ApiError;
use fastcart_kernel::{AppError, StoreContext, now_timestamp};
use fastcart_plugin::{
    ContextClaims, PluginError, PluginManifest, RequestKind, RuntimeRequest, RuntimeResponse,
    StreamingResponse, TokenActor,
};

use super::StoreDb;
use crate::state::AppContext;

/// Builds the plugin application routes. The caller must install store layers.
pub fn apps_router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/apps/{plugin_id}", any(forward))
        .route("/apps/{plugin_id}/{*path}", any(forward))
        .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024))
}

#[allow(clippy::too_many_arguments)]
async fn forward(
    State(context): State<Arc<AppContext>>,
    Extension(store): Extension<StoreContext>,
    StoreDb(database): StoreDb,
    Path(params): Path<std::collections::HashMap<String, String>>,
    mut request: axum::extract::Request,
) -> Result<Response, ApiError> {
    let upgrading = is_websocket(request.method(), request.headers());
    let on_upgrade = upgrading.then(|| hyper::upgrade::on(&mut request));
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let headers = parts.headers;
    let uri = parts.uri;

    let plugin_id = params.get("plugin_id").cloned().unwrap_or_default();
    let installation: Option<(String, i64, String)> = sqlx::query_as(
        "SELECT digest, enabled, manifest_json FROM plugin_installations WHERE plugin_id = ?",
    )
    .bind(&plugin_id)
    .fetch_optional(database.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    let Some((digest, enabled, manifest_json)) = installation else {
        return Err(ApiError::new(AppError::not_found("plugin", plugin_id)));
    };
    if enabled == 0 {
        return Err(ApiError::new(AppError::not_found("plugin", plugin_id)));
    }
    let manifest = PluginManifest::parse(manifest_json.as_bytes())
        .map_err(|error| ApiError::new(error.into()))?;
    let Some(host) = context.plugin_host.as_ref() else {
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "plugin runtime unavailable",
                "code": "plugin_unavailable",
            })),
        )
            .into_response());
    };

    let scopes = storefront_scopes(&manifest);
    let digest_for_token = digest.clone();
    let claims = ContextClaims::new(
        plugin_id.clone(),
        store.store_id.as_str(),
        digest_for_token.clone(),
        scopes,
        TokenActor::Storefront { id: None },
        now_timestamp(),
    );
    let token = context
        .plugin_tokens
        .derive(store.store_id.as_str(), &plugin_id, &digest_for_token)
        .mint(&claims)
        .map_err(|error| ApiError::new(AppError::internal(error.to_string())))?;

    let prefix = format!("/apps/{plugin_id}");
    let mut path_and_query = uri.path().strip_prefix(&prefix).unwrap_or("/").to_owned();
    if path_and_query.is_empty() {
        path_and_query.push('/');
    }
    if let Some(query) = uri.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }

    if let Some(on_upgrade) = on_upgrade {
        let response = super::plugin_ws::accept_response(&headers);
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Ok(response);
        }
        let gateway_addr = match host.gateway_addr().await {
            Ok(address) => address,
            Err(error) => return forward_error(error),
        };
        context.metrics.record_plugin_request();
        let tunnel = super::plugin_ws::TunnelRequest {
            gateway_addr,
            plugin_id: plugin_id.clone(),
            digest,
            store_id: store.store_id.as_str().to_owned(),
            path_and_query,
            context_token: token,
            gateway_token: context.plugin_runtime_token.clone().unwrap_or_default(),
        };
        tokio::spawn(async move {
            match on_upgrade.await {
                Ok(upgraded) => {
                    if let Err(error) = super::plugin_ws::tunnel(upgraded, tunnel).await {
                        tracing::debug!(error = %error, "plugin WebSocket tunnel closed");
                    }
                }
                Err(error) => tracing::debug!(error = %error, "plugin WebSocket upgrade failed"),
            }
        });
        return Ok(response);
    }

    let body = axum::body::to_bytes(body, 8 * 1024 * 1024)
        .await
        .map_err(|_| ApiError::validation("request body exceeds 8 MiB"))?;
    let response = host
        .forward_stream(RuntimeRequest {
            plugin_id,
            digest,
            store_id: store.store_id.as_str().to_owned(),
            path_and_query,
            method: method.as_str().to_owned(),
            headers: forward_headers(&headers),
            context_token: token,
            body: body.to_vec(),
            kind: RequestKind::Storefront,
            deadline_override: None,
        })
        .await;
    context.metrics.record_plugin_request();
    match response {
        Ok(streaming) => Ok(build_streaming_response(streaming)),
        Err(error) => {
            if matches!(&error, PluginError::Runtime(message) if message.contains("timed out")) {
                context.metrics.record_plugin_timeout();
            }
            forward_error(error)
        }
    }
}

fn is_websocket(method: &Method, headers: &HeaderMap) -> bool {
    if method != Method::GET {
        return false;
    }
    let upgrade = headers
        .get(header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    let connection = headers
        .get(header::CONNECTION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        });
    upgrade && connection
}

pub(crate) fn record_forward(context: &AppContext, result: &Result<RuntimeResponse, PluginError>) {
    context.metrics.record_plugin_request();
    if let Err(PluginError::Runtime(message)) = result
        && message.contains("timed out")
    {
        context.metrics.record_plugin_timeout();
    }
}

fn storefront_scopes(manifest: &PluginManifest) -> Vec<String> {
    const ALLOWED: [&str; 4] = ["catalog.read", "store.read", "customer.read", "carts.read"];
    manifest
        .scopes()
        .unwrap_or_default()
        .into_iter()
        .map(|scope| scope.as_str().to_owned())
        .filter(|scope| ALLOWED.contains(&scope.as_str()))
        .collect()
}

fn forward_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    const ALLOWED: [&str; 5] = [
        "content-type",
        "accept",
        "accept-language",
        "user-agent",
        "if-none-match",
    ];
    ALLOWED
        .iter()
        .filter_map(|name| {
            headers
                .get(*name)
                .and_then(|value| value.to_str().ok())
                .map(|value| ((*name).to_owned(), value.to_owned()))
        })
        .collect()
}

pub(crate) fn build_response(response: fastcart_plugin::RuntimeResponse) -> Response {
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut output = Response::new(Body::from(response.body));
    *output.status_mut() = status;
    for (name, value) in response.headers {
        if name.eq_ignore_ascii_case("content-length") || name.eq_ignore_ascii_case("set-cookie") {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(name.as_bytes()),
            header::HeaderValue::from_str(&value),
        ) {
            output.headers_mut().insert(name, value);
        }
    }
    output
}

/// Converts a forwarding result into a response, mapping host failures to
/// 504/503 instead of a generic 500.
pub(crate) fn forward_outcome(
    result: Result<RuntimeResponse, PluginError>,
) -> Result<Response, ApiError> {
    match result {
        Ok(response) => Ok(build_response(response)),
        Err(error) => forward_error(error),
    }
}

pub(crate) fn forward_error(error: PluginError) -> Result<Response, ApiError> {
    match error {
        PluginError::Runtime(message) if message.contains("timed out") => Ok((
            StatusCode::GATEWAY_TIMEOUT,
            axum::Json(serde_json::json!({
                "error": "plugin request timed out",
                "code": "gateway_timeout",
            })),
        )
            .into_response()),
        PluginError::Runtime(message) => {
            tracing::warn!(error = %message, "plugin host unavailable");
            Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({
                    "error": "plugin runtime unavailable",
                    "code": "plugin_unavailable",
                })),
            )
                .into_response())
        }
        other => Err(ApiError::new(other.into())),
    }
}

pub(crate) fn build_streaming_response(streaming: StreamingResponse) -> Response {
    let status = StatusCode::from_u16(streaming.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut response = Response::new(Body::from_stream(streaming.stream));
    *response.status_mut() = status;
    for (name, value) in streaming.headers {
        if name.eq_ignore_ascii_case("content-length") || name.eq_ignore_ascii_case("set-cookie") {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(name.as_bytes()),
            header::HeaderValue::from_str(&value),
        ) {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

/// Runtime health exposed to the admin API.
#[derive(Clone, Debug, serde::Serialize, utoipa::ToSchema)]
pub struct RuntimeStatus {
    /// Whether workerd is running.
    pub running: bool,
    /// Current generation id.
    pub generation: Option<String>,
    /// Gateway port.
    pub port: Option<u16>,
    /// Number of restarts.
    pub restarts: u64,
    /// Last error.
    pub last_error: Option<String>,
    /// Uptime in seconds.
    pub uptime_seconds: u64,
}

/// Reads health from the configured host.
pub(crate) async fn status(context: &AppContext) -> RuntimeStatus {
    match &context.plugin_host {
        Some(host) => {
            let health = host.health().await;
            context.metrics.set_plugin_restarts(health.restarts);
            RuntimeStatus {
                running: health.running,
                generation: health.generation,
                port: health.port,
                restarts: health.restarts,
                last_error: health.last_error,
                uptime_seconds: health.uptime_seconds,
            }
        }
        None => RuntimeStatus {
            running: false,
            generation: None,
            port: None,
            restarts: 0,
            last_error: Some("plugin runtime is disabled or misconfigured".to_owned()),
            uptime_seconds: 0,
        },
    }
}
