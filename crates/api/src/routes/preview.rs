//! Signed draft page preview: token issuance helpers and the render route.
//!
//! A preview token is a short-lived, read-only capability scoped to one store
//! and one page. It is presented as a query parameter on `/preview/{path}` and
//! is verified without consulting a session, so the preview iframe never needs
//! admin cookies. Tokens are never logged and are removed from the URL handed
//! to the storefront runtime.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::http::Uri;
use axum::http::header::HOST;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fastcart_http::{ApiError, RequestId};
use fastcart_kernel::{AppError, CurrencyCode, PageId, StoreContext, now_timestamp};
use fastcart_module_content::{ContentService, PageStatus};
use fastcart_module_theme::ThemeService;
use fastcart_runtime_api::{RuntimeError, RuntimeMode, RuntimeRequest, RuntimeTheme};
use fastcart_storefront::page::RESERVED_CONTEXT_KEY;
use fastcart_storefront_data::{DataMode, DataResolutionContext};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use super::storefront::{
    build_context_data, compile_page, is_hop_by_hop, resolve_active_editor, runtime_error,
    runtime_response,
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
}

/// Issues a signed preview token for one page.
///
/// The returned expiry is `now + PREVIEW_TOKEN_TTL_SECONDS`. The secret is
/// used as an HMAC-SHA256 key and is never included in the result.
pub fn issue_preview_token(
    secret: &str,
    store_id: &str,
    page_id: &str,
    now: i64,
) -> Result<(String, i64), AppError> {
    if !valid_secret(secret) {
        return Err(AppError::NotImplemented(format!(
            "preview secret must be at least {MIN_PREVIEW_SECRET_BYTES} bytes"
        )));
    }
    let expires_at = now.saturating_add(PREVIEW_TOKEN_TTL_SECONDS);
    let payload = PreviewTokenPayload {
        store_id: store_id.to_owned(),
        page_id: page_id.to_owned(),
        exp: expires_at,
    };
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
    Ok((token, expires_at))
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
    if payload.store_id != store_id {
        return Err(PreviewTokenError::WrongStore);
    }
    if payload.exp <= now {
        return Err(PreviewTokenError::Expired);
    }
    Ok(payload)
}

/// Renders a draft page through the configured runtime using a preview token.
///
/// Both `/preview` and `/preview/{*path}` are handled here. The Host header
/// resolves the store exactly like the storefront fallback. The token is the
/// only credential: no admin session is required.
pub(crate) async fn render(
    State(context): State<Arc<AppContext>>,
    Query(query): Query<PreviewQuery>,
    request: Request,
) -> Response {
    let Some(secret) = configured_secret(&context) else {
        return ApiError::new(AppError::NotImplemented(
            "page preview is not configured".to_owned(),
        ))
        .into_response();
    };
    let Some(runtime) = context.storefront_runtime.clone() else {
        return ApiError::not_found("storefront route").into_response();
    };
    let Some(host) = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
    else {
        return ApiError::new(AppError::validation("missing Host header")).into_response();
    };
    let store = match context.resolver.resolve_active(&host).await {
        Ok(store) => store,
        Err(_) => return ApiError::not_found("store").into_response(),
    };
    let now = now_timestamp();
    let Some(token) = query.token.as_deref() else {
        return rejected_token();
    };
    let payload = match verify_preview_token(secret, token, store.id.as_str(), now) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::debug!(reason = ?error, "preview token rejected");
            return rejected_token();
        }
    };

    let database = match context.databases.database(&store.id).await {
        Ok(database) => database,
        Err(error) => return ApiError::new(AppError::from(error)).into_response(),
    };
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

    let mut data: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    data.insert(
        RESERVED_CONTEXT_KEY.to_owned(),
        build_context_data(&store, &host, &currency, &theme, DataMode::Preview),
    );
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
    let runtime_request = RuntimeRequest {
        request_id: request_id.clone(),
        store: StoreContext {
            store_id: store.id.clone(),
            domain: host,
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
            settings: theme.settings.clone(),
        }),
        page: Some(compiled.page),
        data,
        capabilities: BTreeMap::new(),
        mode: RuntimeMode::Preview,
        timeout_ms: context.config.storefront_timeout_ms.max(1),
    };

    match runtime.render(runtime_request).await {
        Ok(response) => {
            let mut response = runtime_response(response);
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
            response
        }
        Err(RuntimeError::Timeout(timeout_ms)) => runtime_error(
            StatusCode::GATEWAY_TIMEOUT,
            "RUNTIME_TIMEOUT",
            &format!("preview rendering timed out after {timeout_ms} ms"),
            &request_id,
        ),
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

/// Maps a preview request URI to the runtime URL.
///
/// The `/preview` prefix is removed, the `token` query parameter is dropped,
/// and the remaining path and query are preserved. `/preview` maps to `/`.
fn preview_runtime_url(uri: &Uri) -> String {
    let rest = uri.path().strip_prefix("/preview").unwrap_or_default();
    let path = if rest.is_empty() { "/" } else { rest };
    match remaining_query(uri.query()) {
        Some(query) => format!("{path}?{query}"),
        None => path.to_owned(),
    }
}

/// Rebuilds a query string without the preview token parameter.
///
/// Keys are percent-decoded before comparison so an encoded spelling such as
/// `to%6ben` cannot smuggle the bearer token into the runtime URL.
fn remaining_query(query: Option<&str>) -> Option<String> {
    let query = query?;
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    let mut kept = false;
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        if key == "token" {
            continue;
        }
        serializer.append_pair(&key, &value);
        kept = true;
    }
    kept.then(|| serializer.finish())
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
        let (token, expires_at) = issue_preview_token(SECRET, "store_1", "page_1", now).unwrap();
        assert_eq!(expires_at, now + PREVIEW_TOKEN_TTL_SECONDS);
        assert_eq!(token.matches('.').count(), 1);

        let payload = verify_preview_token(SECRET, &token, "store_1", now + 1).unwrap();
        assert_eq!(payload.store_id, "store_1");
        assert_eq!(payload.page_id, "page_1");
        assert_eq!(payload.exp, expires_at);
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let (token, _) = issue_preview_token(SECRET, "store_1", "page_1", 0).unwrap();
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
        let (token, _) = issue_preview_token(SECRET, "store_1", "page_1", 0).unwrap();
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
        let (token, expires_at) = issue_preview_token(SECRET, "store_1", "page_1", 0).unwrap();
        assert_eq!(
            verify_preview_token(SECRET, &token, "store_1", expires_at),
            Err(PreviewTokenError::Expired)
        );
    }

    #[test]
    fn wrong_store_is_rejected() {
        let (token, _) = issue_preview_token(SECRET, "store_1", "page_1", 0).unwrap();
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
