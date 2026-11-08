//! Signed draft page preview: token issuance helpers and the render route.
//!
//! A preview token is a short-lived, read-only capability scoped to one store
//! and one page. It is presented as a query parameter on `/preview/{path}` and
//! is verified without consulting a session, so the preview iframe never needs
//! admin cookies. Tokens are never logged and are removed from the URL handed
//! to the storefront runtime.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Extension, Query, Request, State};
use axum::http::StatusCode;
use axum::http::Uri;
use axum::http::header::HOST;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fastcart_http::{ApiError, ClientIp, RequestId};
use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CurrencyCode, PageId, Pagination, StoreContext, now_timestamp, validate_handle,
};
use fastcart_module_catalog::CatalogService;
use fastcart_module_content::{ContentService, PageStatus};
use fastcart_module_theme::ThemeService;
use fastcart_runtime_api::{RuntimeError, RuntimeMode, RuntimeRequest, RuntimeTheme};
use fastcart_storefront::page::RESERVED_CONTEXT_KEY;
use fastcart_storefront_data::{DataMode, DataResolutionContext};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use super::storefront::{
    THEME_ASSETS_BASE_URL, build_context_data, capability_grants, compile_page, is_hop_by_hop,
    match_theme_route, resolve_active_editor, runtime_error, runtime_response,
};
use crate::state::AppContext;

/// Lifetime of an issued preview token in seconds.
pub const PREVIEW_TOKEN_TTL_SECONDS: i64 = 15 * 60;

/// Minimum acceptable preview signing-key length in bytes.
pub const MIN_PREVIEW_SECRET_BYTES: usize = 32;

/// Claims carried by a signed preview token.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, serde::Serialize)]
pub struct PreviewTokenPayload {
    /// Store the token is valid for.
    pub store_id: String,
    /// Page the token grants access to.
    pub page_id: String,
    /// Unique token identifier used for revocation.
    pub jti: String,
    /// Unix expiry timestamp. The token is expired when `exp <= now`.
    pub exp: i64,
}

/// Reasons a preview token can be rejected.
///
/// The variants deliberately carry no token material so callers cannot
/// accidentally log the credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewTokenError {
    /// The token does not have the `payload.signature` shape or does not use
    /// valid base64url encoding.
    Malformed,
    /// The signature does not match the payload under the configured secret.
    BadSignature,
    /// The token is past its expiry timestamp.
    Expired,
    /// The token was issued for a different store.
    WrongStore,
}

/// Query parameters accepted by the preview render route.
#[derive(Debug, Deserialize)]
pub(crate) struct PreviewQuery {
    /// Signed preview token.
    token: Option<String>,
    /// Optional resource handle for a dynamic route template.
    ///
    /// Used only when the request path does not already match the template's
    /// route. The value is internal to the preview surface and is removed
    /// before the URL reaches the theme.
    preview_handle: Option<String>,
}

/// Issues a signed preview token for one page.
///
/// The returned expiry is `now + PREVIEW_TOKEN_TTL_SECONDS`. The token
/// identifier is returned alongside it so an administrator can revoke this
/// exact preview before it expires. The secret is used as an HMAC key and is
/// never included in the result.
pub fn issue_preview_token(
    secret: &str,
    store_id: &str,
    page_id: &str,
    now: i64,
) -> Result<IssuedPreviewToken, AppError> {
    if !valid_secret(secret) {
        return Err(AppError::NotImplemented(format!(
            "preview secret must be at least {MIN_PREVIEW_SECRET_BYTES} bytes"
        )));
    }
    let expires_at = now.saturating_add(PREVIEW_TOKEN_TTL_SECONDS);
    let payload = PreviewTokenPayload {
        store_id: store_id.to_owned(),
        page_id: page_id.to_owned(),
        jti: uuid::Uuid::now_v7().simple().to_string(),
        exp: expires_at,
    };
    let jti = payload.jti.clone();
    let payload_json = serde_json::to_vec(&payload).map_err(|error| {
        AppError::internal(format!("could not serialize preview token: {error}"))
    })?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|error| AppError::internal(format!("invalid preview signing key: {error}")))?;
    mac.update(&payload_json);
    let signature = mac.finalize().into_bytes();
    let token = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&payload_json),
        URL_SAFE_NO_PAD.encode(signature)
    );
    Ok(IssuedPreviewToken {
        token,
        expires_at,
        jti,
    })
}

/// A freshly issued preview credential and its revocable identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedPreviewToken {
    /// Signed `payload.signature` token.
    pub token: String,
    /// Unix timestamp when the token expires.
    pub expires_at: i64,
    /// Identifier used to revoke this token early.
    pub jti: String,
}

/// Verifies a preview token for a store at a point in time.
///
/// The signature is compared in constant time and always checked before the
/// payload is trusted. Expiry and store binding are then enforced.
pub fn verify_preview_token(
    secret: &str,
    token: &str,
    store_id: &str,
    now: i64,
) -> Result<PreviewTokenPayload, PreviewTokenError> {
    let payload = verify_preview_token_unbound(secret, token, now)?;
    if payload.store_id != store_id {
        return Err(PreviewTokenError::WrongStore);
    }
    Ok(payload)
}

/// Verifies a preview token without binding it to a store.
///
/// The token carries its own store identifier, so callers that only need to
/// render (the preview route) can resolve the store from the signed payload
/// instead of the request Host header.
pub fn verify_preview_token_unbound(
    secret: &str,
    token: &str,
    now: i64,
) -> Result<PreviewTokenPayload, PreviewTokenError> {
    let (payload_b64, signature_b64) = token.split_once('.').ok_or(PreviewTokenError::Malformed)?;
    if signature_b64.contains('.') {
        return Err(PreviewTokenError::Malformed);
    }
    let payload_json = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| PreviewTokenError::Malformed)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| PreviewTokenError::Malformed)?;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| PreviewTokenError::Malformed)?;
    mac.update(&payload_json);
    mac.verify_slice(&signature)
        .map_err(|_| PreviewTokenError::BadSignature)?;

    let payload: PreviewTokenPayload =
        serde_json::from_slice(&payload_json).map_err(|_| PreviewTokenError::Malformed)?;
    if payload.exp <= now {
        return Err(PreviewTokenError::Expired);
    }
    Ok(payload)
}

/// Renders a draft page through the configured runtime using a preview token.
///
/// Both `/preview` and `/preview/{*path}` are handled here. The signed token
/// names the store, so preview works on any admin host without the store
/// domain having to match. The token is the only credential: no admin session
/// is required.
pub(crate) async fn render(
    State(context): State<Arc<AppContext>>,
    client_ip: Option<Extension<ClientIp>>,
    Query(query): Query<PreviewQuery>,
    request: Request,
) -> Response {
    let client = client_ip
        .map(|Extension(ip)| ip.as_key())
        .unwrap_or_else(|| "unknown".to_owned());
    if !context.preview_limiter.check(&client) {
        return ApiError::new(AppError::RateLimited).into_response();
    }
    let Some(secret) = configured_secret(&context) else {
        return ApiError::new(AppError::NotImplemented(
            "page preview is not configured".to_owned(),
        ))
        .into_response();
    };
    let Some(runtime) = context.storefront_runtime.clone() else {
        return ApiError::not_found("storefront route").into_response();
    };
    let now = now_timestamp();
    let Some(token) = query.token.as_deref() else {
        return rejected_token();
    };
    let payload = match verify_preview_token_unbound(secret, token, now) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::debug!(reason = ?error, "preview token rejected");
            return rejected_token();
        }
    };
    let store_id = fastcart_kernel::StoreId::from_string(payload.store_id.clone());
    let store = match context.platform.get_store(&store_id).await {
        Ok(Some(store)) if store.ensure_active().is_ok() => store,
        Ok(_) => return ApiError::not_found("store").into_response(),
        Err(error) => return ApiError::new(AppError::from(error)).into_response(),
    };

    let domain = store.primary_domain.clone().unwrap_or_default();
    let database = match context.databases.database(&store.id).await {
        Ok(database) => database,
        Err(error) => return ApiError::new(AppError::from(error)).into_response(),
    };
    match is_revoked(&database, &payload.jti, now).await {
        Ok(true) => return rejected_token(),
        Ok(false) => {}
        Err(error) => return ApiError::new(error).into_response(),
    }
    let currency = match CurrencyCode::new(store.currency_code.clone()) {
        Ok(currency) => currency,
        Err(error) => return ApiError::new(AppError::from(error)).into_response(),
    };
    let theme = match ThemeService::new(database.clone()).active().await {
        Ok(Some(theme)) => theme,
        Ok(None) => return ApiError::not_found("active theme").into_response(),
        Err(error) => return ApiError::new(error).into_response(),
    };
    let page = match ContentService::new(database.clone())
        .get_page(&PageId::from_string(payload.page_id))
        .await
    {
        Ok(page) => page,
        Err(error) if error.is_not_found() => {
            return ApiError::not_found("page").into_response();
        }
        Err(error) => return ApiError::new(error).into_response(),
    };
    // An archived page is withdrawn from every surface, including the editor
    // preview. The token stays valid but no longer renders anything.
    if page.status == PageStatus::Archived {
        return ApiError::not_found("page").into_response();
    }

    // A dynamic route template is previewed with a concrete resource so that
    // `data.context.route` matches production. The path wins; otherwise an
    // explicit handle is used, and finally the first published record of the
    // template's kind. A missing resource leaves the route unset rather than
    // failing the preview.
    let page_type = page.page_type.trim();
    let mut route_context: Option<serde_json::Value> = None;
    if theme.manifest.routes.contains_key(page_type) {
        let matched = match_theme_route(&theme.manifest.routes, preview_path(request.uri()))
            .filter(|route| route.kind == page_type);
        let handle = if let Some(route) = matched {
            Some(route.handle)
        } else if let Some(handle) = query
            .preview_handle
            .as_deref()
            .map(str::trim)
            .filter(|handle| !handle.is_empty())
        {
            if validate_handle(handle, "handle").is_ok() {
                Some(handle.to_owned())
            } else {
                tracing::debug!(handle, "ignored invalid preview handle");
                None
            }
        } else {
            first_published_handle(&database, page_type).await
        };
        if let Some(handle) = handle {
            let resolution = DataResolutionContext {
                currency: currency.clone(),
                sales_channel_id: None,
                customer_group_id: None,
                mode: DataMode::Preview,
                now,
            };
            match context
                .storefront_data
                .resolve_route(&database, page_type, &handle, &resolution)
                .await
            {
                Ok(Some(resource)) => {
                    route_context = Some(serde_json::json!({
                        "kind": page_type,
                        "handle": handle,
                        "resource": resource,
                    }));
                }
                Ok(None) => {
                    tracing::debug!(
                        kind = page_type,
                        handle = %handle,
                        "preview route resource not found"
                    );
                }
                Err(error) => {
                    context.metrics.record_storefront_data_failure();
                    tracing::warn!(
                        kind = page_type,
                        handle = %handle,
                        error = %error,
                        "preview route resolution failed"
                    );
                }
            }
        }
    }

    let editor = resolve_active_editor(&context, &store.id, &theme).await;
    let compiled = match compile_page(&page, editor.manifest.as_deref()) {
        Ok(compiled) => compiled,
        Err(error) => {
            return ApiError::new(AppError::validation(format!(
                "page body is invalid for preview: {error}"
            )))
            .into_response();
        }
    };

    let mut context_entry = build_context_data(
        &context,
        &database,
        request.headers(),
        &store,
        &domain,
        &currency,
        &theme,
        DataMode::Preview,
    )
    .await;
    if let Some(route) = route_context {
        context_entry["route"] = route;
    }
    let mut data: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    data.insert(RESERVED_CONTEXT_KEY.to_owned(), context_entry);
    if !compiled.bindings.is_empty() {
        let resolution = DataResolutionContext {
            currency: currency.clone(),
            sales_channel_id: None,
            customer_group_id: None,
            mode: DataMode::Preview,
            now,
        };
        match context
            .storefront_data
            .resolve(&database, &compiled.bindings, &resolution)
            .await
        {
            Ok(bag) => data.extend(bag),
            Err(error) => {
                context.metrics.record_storefront_data_failure();
                tracing::warn!(error = %error, "preview data resolution failed");
            }
        }
    }
    let data = Some(serde_json::to_value(data).unwrap_or(serde_json::Value::Null));

    let request_id = request
        .extensions()
        .get::<RequestId>()
        .map(|id| id.0.clone())
        .unwrap_or_default();
    let url = preview_runtime_url(request.uri());
    let headers = request
        .headers()
        .iter()
        .filter(|(name, _)| !is_hop_by_hop(name.as_str()) && name.as_str() != HOST)
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    let capabilities = capability_grants(&context, &store.id, &database, &theme).await;
    let runtime_request = RuntimeRequest {
        request_id: request_id.clone(),
        store: StoreContext {
            store_id: store.id.clone(),
            domain,
            locale: store.locale.clone(),
            currency,
            timezone: store.timezone.clone(),
        },
        method: "GET".to_owned(),
        url,
        headers,
        body: None,
        theme: Some(RuntimeTheme {
            id: theme.manifest.id.clone(),
            version: theme.manifest.version.clone(),
            runtime: theme.manifest.runtime.clone(),
            bundle_path: theme.bundle_path.clone(),
            entry: theme.manifest.entry.clone(),
            settings: theme
                .draft_settings
                .clone()
                .unwrap_or_else(|| theme.settings.clone()),
            assets_manifest: theme.assets_manifest.clone(),
            assets_base_url: Some(THEME_ASSETS_BASE_URL.to_owned()),
        }),
        page: Some(compiled.page),
        data,
        capabilities,
        mode: RuntimeMode::Preview,
        timeout_ms: context.config.storefront_timeout_ms.max(1),
        streaming: context.config.storefront_streaming_enabled,
        stream_timeout_ms: context.config.storefront_stream_timeout_ms.max(1),
    };

    match runtime.render(runtime_request).await {
        Ok(response) => {
            let mut response =
                runtime_response(response, context.config.storefront_stream_timeout_ms.max(1));
            // Draft previews must never be cached by browsers or shared
            // proxies, even if the theme emits permissive headers.
            response.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            response.headers_mut().insert(
                axum::http::header::HeaderName::from_static("x-robots-tag"),
                axum::http::HeaderValue::from_static("noindex, nofollow"),
            );
            // Keep the token-bearing preview URL out of Referer headers and
            // restrict which origins may frame a draft.
            response.headers_mut().insert(
                axum::http::header::HeaderName::from_static("referrer-policy"),
                axum::http::HeaderValue::from_static("no-referrer"),
            );
            let ancestors = &context.config.preview_frame_ancestors;
            if !ancestors.is_empty() {
                let mut frame_ancestors = String::from("frame-ancestors 'self'");
                for ancestor in ancestors {
                    frame_ancestors.push(' ');
                    frame_ancestors.push_str(ancestor);
                }
                if let Ok(value) = axum::http::HeaderValue::from_str(&frame_ancestors) {
                    response.headers_mut().insert(
                        axum::http::header::HeaderName::from_static("content-security-policy"),
                        value,
                    );
                }
            }
            response
        }
        Err(RuntimeError::Timeout(timeout_ms)) => runtime_error(
            StatusCode::GATEWAY_TIMEOUT,
            "RUNTIME_TIMEOUT",
            &format!("preview rendering timed out after {timeout_ms} ms"),
            &request_id,
        ),
        Err(RuntimeError::WorkerUnavailable(message)) => {
            tracing::warn!(error = %message, "preview runtime unavailable");
            let mut response = runtime_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "RUNTIME_UNAVAILABLE",
                "storefront preview is temporarily unavailable",
                &request_id,
            );
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from_static("5"),
            );
            response
        }
        Err(error) => {
            tracing::warn!(error = %error, "preview runtime failed");
            runtime_error(
                StatusCode::BAD_GATEWAY,
                "RUNTIME_ERROR",
                "preview rendering failed",
                &request_id,
            )
        }
    }
}

/// Returns true when the token identifier has been explicitly revoked.
///
/// Signature verification happens first; the denylist is only consulted for a
/// cryptographically valid, unexpired token bound to the current store.
async fn is_revoked(db: &StoreDatabase, jti: &str, now: i64) -> Result<bool, AppError> {
    let row = sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM preview_token_revocations WHERE jti = ? AND expires_at > ?",
    )
    .bind(jti)
    .bind(now)
    .fetch_optional(db.pool())
    .await
    .map_err(database_error)?;
    Ok(row.is_some())
}

/// Returns the configured preview secret, treating blank or weak values as
/// absent so the preview surface stays disabled instead of weakening HMAC.
fn configured_secret(context: &AppContext) -> Option<&str> {
    context
        .config
        .preview_secret
        .as_deref()
        .filter(|value| valid_secret(value))
}

/// Returns true when a secret is long enough to be used as an HMAC key.
pub(crate) fn valid_secret(secret: &str) -> bool {
    secret.trim().len() >= MIN_PREVIEW_SECRET_BYTES
}

/// Builds the generic rejection returned for every invalid token.
fn rejected_token() -> Response {
    ApiError::new(AppError::Unauthorized).into_response()
}

/// Returns the site-relative path behind the `/preview` prefix.
fn preview_path(uri: &Uri) -> &str {
    let rest = uri.path().strip_prefix("/preview").unwrap_or_default();
    if rest.is_empty() { "/" } else { rest }
}

/// Maps a preview request URI to the runtime URL.
///
/// The `/preview` prefix is removed, preview-only query parameters are
/// dropped, and the remaining path and query are preserved. `/preview` maps to
/// `/`.
fn preview_runtime_url(uri: &Uri) -> String {
    let path = preview_path(uri);
    match remaining_query(uri.query()) {
        Some(query) => format!("{path}?{query}"),
        None => path.to_owned(),
    }
}

/// Rebuilds a query string without preview-only parameters.
///
/// Keys are percent-decoded before comparison so an encoded spelling such as
/// `to%6ben` cannot smuggle the bearer token into the runtime URL.
fn remaining_query(query: Option<&str>) -> Option<String> {
    let query = query?;
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    let mut kept = false;
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        if key == "token" || key == "preview_handle" {
            continue;
        }
        serializer.append_pair(&key, &value);
        kept = true;
    }
    kept.then(|| serializer.finish())
}

/// Picks the first published resource of a route kind for auto-previewed
/// templates.
async fn first_published_handle(database: &StoreDatabase, kind: &str) -> Option<String> {
    let pagination = Pagination {
        limit: 1,
        offset: 0,
    };
    let catalog = CatalogService::new(database.clone());
    let result = match kind {
        "product" => catalog.list_published_product_handles(pagination).await,
        "collection" => catalog.list_published_collection_handles(pagination).await,
        _ => return None,
    };
    match result {
        Ok(page) => page.data.into_iter().next(),
        Err(error) => {
            tracing::warn!(
                kind,
                error = %error,
                "could not auto-select a preview route handle"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "preview-test-secret-0123456789abcdef";

    #[test]
    fn weak_secrets_are_rejected() {
        assert!(matches!(
            issue_preview_token("short", "store_1", "page_1", 0),
            Err(AppError::NotImplemented(_))
        ));
    }

    #[test]
    fn valid_token_round_trips() {
        let now = 1_000;
        let issued = issue_preview_token(SECRET, "store_1", "page_1", now).unwrap();
        assert_eq!(issued.expires_at, now + PREVIEW_TOKEN_TTL_SECONDS);
        assert_eq!(issued.token.matches('.').count(), 1);
        assert_eq!(issued.jti.len(), 32, "uuid v7 simple form");

        let payload = verify_preview_token(SECRET, &issued.token, "store_1", now + 1).unwrap();
        assert_eq!(payload.store_id, "store_1");
        assert_eq!(payload.page_id, "page_1");
        assert_eq!(payload.jti, issued.jti);
        assert_eq!(payload.exp, issued.expires_at);
    }

    #[test]
    fn revoked_tokens_keep_their_identifier() {
        let first = issue_preview_token(SECRET, "store_1", "page_1", 0).unwrap();
        let second = issue_preview_token(SECRET, "store_1", "page_1", 0).unwrap();
        assert_ne!(first.jti, second.jti, "each issuance must be revocable");
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let token = issue_preview_token(SECRET, "store_1", "page_1", 0)
            .unwrap()
            .token;
        let (payload, signature) = token.split_once('.').unwrap();
        let mut claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        claims["page_id"] = serde_json::json!("page_2");
        let tampered = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        let token = format!("{tampered}.{signature}");

        assert_eq!(
            verify_preview_token(SECRET, &token, "store_1", 1),
            Err(PreviewTokenError::BadSignature)
        );
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let token = issue_preview_token(SECRET, "store_1", "page_1", 0)
            .unwrap()
            .token;
        let (payload, signature) = token.split_once('.').unwrap();
        let mut bytes = signature.as_bytes().to_vec();
        bytes[0] = if bytes[0] == b'A' { b'B' } else { b'A' };
        let tampered = format!("{payload}.{}", String::from_utf8(bytes).unwrap());

        assert_eq!(
            verify_preview_token(SECRET, &tampered, "store_1", 1),
            Err(PreviewTokenError::BadSignature)
        );
    }

    #[test]
    fn expired_token_is_rejected() {
        let issued = issue_preview_token(SECRET, "store_1", "page_1", 0).unwrap();
        assert_eq!(
            verify_preview_token(SECRET, &issued.token, "store_1", issued.expires_at),
            Err(PreviewTokenError::Expired)
        );
    }

    #[test]
    fn wrong_store_is_rejected() {
        let token = issue_preview_token(SECRET, "store_1", "page_1", 0)
            .unwrap()
            .token;
        assert_eq!(
            verify_preview_token(SECRET, &token, "store_2", 1),
            Err(PreviewTokenError::WrongStore)
        );
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        for token in ["", "no-dot", "a.b.c", "%%%%.%%%%", "not-base64.not-base64"] {
            assert_eq!(
                verify_preview_token(SECRET, token, "store_1", 1),
                Err(PreviewTokenError::Malformed),
                "token {token:?}"
            );
        }
    }

    #[test]
    fn runtime_url_drops_the_token_and_keeps_the_rest() {
        let uri: Uri = "/preview/about?token=abc&x=1&y=2".parse().unwrap();
        assert_eq!(preview_runtime_url(&uri), "/about?x=1&y=2");
        let uri: Uri = "/preview/about?to%6ben=abc&x=1".parse().unwrap();
        assert_eq!(
            preview_runtime_url(&uri),
            "/about?x=1",
            "percent-encoded token keys must be redacted too"
        );
        let uri: Uri = "/preview".parse().unwrap();
        assert_eq!(preview_runtime_url(&uri), "/");
        let uri: Uri = "/preview/?token=abc".parse().unwrap();
        assert_eq!(preview_runtime_url(&uri), "/");
    }
}
