//! Public store-scoped capability endpoints resolved from plugin bindings.
//!
//! A theme calls `/store/capabilities/{id}/...`; the host resolves the store's
//! binding to one enabled plugin and forwards the request to that plugin's
//! worker under `/capabilities/{id}/...`. The plugin receives a short-lived
//! storefront token, never the customer session.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use fastcart_http::{ApiError, ClientIp};
use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{ActorContext, AppError, StoreContext, now_timestamp};
use fastcart_plugin::{ContextClaims, PluginError, PluginManifest, RequestKind, RuntimeRequest};
use fastcart_ssr::{CapabilityGrant, CapabilityGrants};

use super::StoreDb;
use super::plugin_runtime::{
    build_streaming_response, client_key, forward_error, forward_headers, is_websocket,
    storefront_actor, storefront_scopes,
};
use crate::state::AppContext;

const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Builds the public capability routes. The caller installs store layers.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/store/capabilities/{capability_id}", any(forward))
        .route("/store/capabilities/{capability_id}/{*path}", any(forward))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
}

#[allow(clippy::too_many_arguments)]
async fn forward(
    State(context): State<Arc<AppContext>>,
    Extension(store): Extension<StoreContext>,
    Extension(actor): Extension<ActorContext>,
    client_ip: Option<Extension<ClientIp>>,
    StoreDb(database): StoreDb,
    Path(params): Path<HashMap<String, String>>,
    mut request: axum::extract::Request,
) -> Result<Response, ApiError> {
    let capability_id = params.get("capability_id").cloned().unwrap_or_default();
    if !valid_capability_id(&capability_id) {
        return Err(ApiError::new(AppError::not_found(
            "capability",
            capability_id,
        )));
    }

    let binding: Option<(String, i64, i64)> = sqlx::query_as(
        "SELECT plugin_id, version, enabled FROM store_capability_bindings \
         WHERE capability_id = ?",
    )
    .bind(&capability_id)
    .fetch_optional(database.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    let Some((plugin_id, version, enabled)) = binding else {
        return Err(ApiError::new(AppError::not_found(
            "capability",
            capability_id,
        )));
    };
    if enabled == 0 {
        return Err(ApiError::new(AppError::not_found(
            "capability",
            capability_id,
        )));
    }

    let installation: Option<(String, i64, String)> = sqlx::query_as(
        "SELECT digest, enabled, manifest_json FROM plugin_installations WHERE plugin_id = ?",
    )
    .bind(&plugin_id)
    .fetch_optional(database.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    let Some((digest, installed, manifest_json)) = installation else {
        return Err(ApiError::new(AppError::not_found("plugin", plugin_id)));
    };
    if installed == 0 {
        return Err(ApiError::new(AppError::not_found("plugin", plugin_id)));
    }
    let Ok(version) = u32::try_from(version) else {
        return Err(ApiError::new(AppError::not_found(
            "capability",
            capability_id,
        )));
    };
    let manifest = PluginManifest::parse(manifest_json.as_bytes())
        .map_err(|error| ApiError::new(error.into()))?;
    // The binding may outlive a plugin update that removed the contribution.
    if manifest
        .storefront_capability(&capability_id, version)
        .is_none()
    {
        return Err(ApiError::new(AppError::not_found(
            "capability",
            capability_id,
        )));
    }

    let limiter_key = format!(
        "{}:capability:{}:{}",
        store.store_id.as_str(),
        capability_id,
        client_key(client_ip.as_ref())
    );
    if !context.plugin_apps_limiter.check(&limiter_key) {
        return Err(ApiError::new(AppError::RateLimited));
    }
    let Some(host) = context.plugin_host.as_ref() else {
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": "plugin runtime unavailable",
                "code": "plugin_unavailable",
            })),
        )
            .into_response());
    };

    let claims = ContextClaims::new(
        plugin_id.clone(),
        store.store_id.as_str(),
        digest.clone(),
        storefront_scopes(&manifest),
        storefront_actor(&actor),
        now_timestamp(),
    );
    let token = context
        .plugin_tokens
        .derive(store.store_id.as_str(), &plugin_id, &digest)
        .mint(&claims)
        .map_err(|error| ApiError::new(AppError::internal(error.to_string())))?;

    let suffix = params.get("path").cloned().unwrap_or_default();
    let mut path_and_query = format!("/capabilities/{capability_id}");
    if !suffix.is_empty() {
        path_and_query.push('/');
        path_and_query.push_str(&suffix);
    }
    if let Some(query) = request.uri().query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }

    if is_websocket(request.method(), request.headers()) {
        let on_upgrade = hyper::upgrade::on(&mut request);
        let response = super::plugin_ws::accept_response(request.headers());
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
            plugin_id,
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
                        tracing::debug!(error = %error, "capability WebSocket tunnel closed");
                    }
                }
                Err(error) => {
                    tracing::debug!(error = %error, "capability WebSocket upgrade failed");
                }
            }
        });
        return Ok(response);
    }

    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| ApiError::validation("request body exceeds 8 MiB"))?;
    let mut headers = forward_headers(&parts.headers);
    headers.push(("x-fastcart-capability-id".to_owned(), capability_id.clone()));
    headers.push((
        "x-fastcart-capability-version".to_owned(),
        version.to_string(),
    ));
    let response = host
        .forward_stream(RuntimeRequest {
            plugin_id,
            digest,
            store_id: store.store_id.as_str().to_owned(),
            path_and_query,
            method: parts.method.as_str().to_owned(),
            headers,
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

/// Resolves the capability grants available to a store's storefront.
///
/// A binding is exposed only when it is enabled, its plugin is installed and
/// enabled, and the plugin's current manifest still declares the bound
/// capability version. Invalid or stale rows are skipped so one bad binding
/// cannot break storefront rendering.
pub(crate) async fn resolve_grants(database: &StoreDatabase) -> Result<CapabilityGrants, AppError> {
    let rows: Vec<(String, i64, String, String, String)> = sqlx::query_as(
        "SELECT b.capability_id, b.version, b.plugin_id, b.config_json, i.manifest_json \
         FROM store_capability_bindings b \
         JOIN plugin_installations i \
           ON i.plugin_id = b.plugin_id AND i.enabled = 1 \
         WHERE b.enabled = 1 \
         ORDER BY b.capability_id ASC",
    )
    .fetch_all(database.pool())
    .await
    .map_err(database_error)?;

    let mut manifests: HashMap<String, Option<PluginManifest>> = HashMap::new();
    let mut grants = CapabilityGrants::new();
    for (capability_id, version, plugin_id, config_json, manifest_json) in rows {
        let Ok(version) = u32::try_from(version) else {
            tracing::warn!(
                capability_id,
                version,
                "ignoring capability binding with an invalid version"
            );
            continue;
        };
        let manifest = manifests
            .entry(plugin_id.clone())
            .or_insert_with(|| PluginManifest::parse(manifest_json.as_bytes()).ok());
        let Some(manifest) = manifest.as_ref() else {
            tracing::warn!(
                plugin_id,
                "ignoring capability binding: invalid plugin manifest"
            );
            continue;
        };
        if manifest
            .storefront_capability(&capability_id, version)
            .is_none()
        {
            tracing::warn!(
                plugin_id,
                capability_id,
                version,
                "ignoring capability binding: manifest no longer provides it"
            );
            continue;
        }
        let config = match serde_json::from_str(&config_json) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!(capability_id, error = %error, "invalid capability config");
                serde_json::Value::Object(serde_json::Map::new())
            }
        };
        grants.insert(
            capability_id.clone(),
            CapabilityGrant {
                version,
                endpoint: format!("/store/capabilities/{capability_id}"),
                config,
            },
        );
    }
    Ok(grants)
}

fn valid_capability_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.' | b'_')
        })
        && !value.starts_with('-')
        && !value.ends_with('-')
}
