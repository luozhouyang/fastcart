//! Internal fetch bridge between the storefront runtime and commerce services.
//!
//! The bridge dispatches runtime-initiated requests straight into the
//! in-process Axum router. No TCP hop or public HTTP surface is involved, and
//! the trusted store context is enforced by overwriting the `Host` header with
//! the resolved store domain before dispatch.

use axum::Router;
use axum::body::Body;
use axum::http::header::{CONNECTION, HOST, TE, TRAILER, TRANSFER_ENCODING, UPGRADE};
use axum::http::{HeaderName, HeaderValue, Method, Request, Uri};
use bytes::Bytes;
use fastcart_kernel::StoreContext;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Logical prefix for internal storefront routes.
pub const INTERNAL_STOREFRONT_PREFIX: &str = "/internal/storefront";

/// A request dispatched from the JavaScript runtime to Rust.
#[derive(Debug)]
pub struct BridgeRequest {
    /// Trusted store context resolved by the host.
    pub store: StoreContext,
    /// HTTP method.
    pub method: String,
    /// Logical path including the internal prefix.
    pub path: String,
    /// Request headers.
    pub headers: Vec<(String, String)>,
    /// Request body.
    pub body: Bytes,
}

/// A response returned to the JavaScript runtime.
#[derive(Debug)]
pub struct BridgeResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Response body.
    pub body: Bytes,
}

/// Errors raised by the internal fetch bridge.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// No internal route matched the request.
    #[error("internal route not found: {0}")]
    NotFound(String),
    /// The bridge rejected a malformed request.
    #[error("invalid bridge request: {0}")]
    Invalid(String),
    /// The bridge failed while dispatching the request.
    #[error("bridge failure: {0}")]
    Internal(String),
}

/// Dispatches internal storefront requests without TCP or public HTTP.
#[async_trait::async_trait]
pub trait InternalFetch: Send + Sync {
    /// Dispatches one internal request.
    async fn fetch(&self, request: BridgeRequest) -> Result<BridgeResponse, BridgeError>;
}

/// An [`InternalFetch`] backed by an in-process Axum router.
///
/// The router is expected to be the internal storefront router: requests are
/// routed only when their path starts with [`INTERNAL_STOREFRONT_PREFIX`].
#[derive(Clone)]
pub struct AxumInternalFetch {
    router: Router,
}

impl AxumInternalFetch {
    /// Creates a bridge around an already state-applied router.
    #[must_use]
    pub fn new(router: Router) -> Self {
        Self { router }
    }

    /// Returns the wrapped router.
    pub fn router(&self) -> &Router {
        &self.router
    }

    fn build_request(request: BridgeRequest) -> Result<Request<Body>, BridgeError> {
        let uri: Uri = request
            .path
            .parse()
            .map_err(|_| BridgeError::Invalid(format!("invalid path: {}", request.path)))?;
        if uri.scheme().is_some()
            || uri.authority().is_some()
            || !uri.path().starts_with(INTERNAL_STOREFRONT_PREFIX)
        {
            return Err(BridgeError::NotFound(request.path));
        }
        let method = Method::from_bytes(request.method.as_bytes())
            .map_err(|_| BridgeError::Invalid(format!("invalid method: {}", request.method)))?;

        let mut builder = Request::builder().method(method).uri(uri);
        for (name, value) in request.headers {
            if is_hop_by_hop(&name) || name.eq_ignore_ascii_case(HOST.as_str()) {
                continue;
            }
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| BridgeError::Invalid(format!("invalid header name: {name}")))?;
            let value = HeaderValue::from_str(&value)
                .map_err(|_| BridgeError::Invalid(format!("invalid header value for {name}")))?;
            builder = builder.header(name, value);
        }
        // The runtime never gets to choose the store: the host is always the
        // domain of the store resolved by the platform middleware.
        builder = builder.header(HOST, store_host(&request.store.domain));
        builder
            .body(Body::from(request.body))
            .map_err(|error| BridgeError::Internal(error.to_string()))
    }
}

#[async_trait::async_trait]
impl InternalFetch for AxumInternalFetch {
    async fn fetch(&self, request: BridgeRequest) -> Result<BridgeResponse, BridgeError> {
        let request = Self::build_request(request)?;
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .map_err(|error| BridgeError::Internal(error.to_string()))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter(|(name, _)| !is_hop_by_hop(name.as_str()))
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|error| BridgeError::Internal(error.to_string()))?
            .to_bytes();
        Ok(BridgeResponse {
            status,
            headers,
            body,
        })
    }
}

fn store_host(domain: &str) -> HeaderValue {
    HeaderValue::from_str(domain).unwrap_or_else(|_| HeaderValue::from_static("localhost"))
}

fn is_hop_by_hop(name: &str) -> bool {
    [
        CONNECTION.as_str(),
        "keep-alive",
        TE.as_str(),
        TRAILER.as_str(),
        TRANSFER_ENCODING.as_str(),
        UPGRADE.as_str(),
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
    ]
    .iter()
    .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use bytes::Bytes;
    use fastcart_kernel::{CurrencyCode, StoreContext, StoreId};

    use super::{
        AxumInternalFetch, BridgeError, BridgeRequest, INTERNAL_STOREFRONT_PREFIX, InternalFetch,
    };

    fn store() -> StoreContext {
        StoreContext {
            store_id: StoreId::from_string("store_test"),
            domain: "shop.example.com".to_owned(),
            locale: "en-US".to_owned(),
            currency: CurrencyCode::new("USD").unwrap(),
            timezone: "UTC".to_owned(),
        }
    }

    async fn echo(headers: HeaderMap, body: Bytes) -> impl IntoResponse {
        let host = headers
            .get("host")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let marker = headers
            .get("x-marker")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        (
            StatusCode::CREATED,
            [("x-bridge", "ok")],
            format!(
                "host={host};marker={marker};body={}",
                String::from_utf8_lossy(&body)
            ),
        )
    }

    fn router() -> Router {
        Router::new().route(&format!("{INTERNAL_STOREFRONT_PREFIX}/echo"), post(echo))
    }

    fn request(path: &str) -> BridgeRequest {
        BridgeRequest {
            store: store(),
            method: "POST".to_owned(),
            path: path.to_owned(),
            headers: vec![
                ("host".to_owned(), "evil.example.com".to_owned()),
                ("x-marker".to_owned(), "kept".to_owned()),
                ("connection".to_owned(), "close".to_owned()),
            ],
            body: Bytes::from_static(b"hello"),
        }
    }

    #[tokio::test]
    async fn dispatches_in_process_and_overrides_the_host() {
        let fetch = AxumInternalFetch::new(router());
        let response = fetch
            .fetch(request(&format!(
                "{INTERNAL_STOREFRONT_PREFIX}/echo?trace=1"
            )))
            .await
            .unwrap();

        assert_eq!(response.status, 201);
        assert_eq!(
            String::from_utf8_lossy(&response.body),
            "host=shop.example.com;marker=kept;body=hello"
        );
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "x-bridge" && value == "ok")
        );
        assert!(
            response
                .headers
                .iter()
                .all(|(name, _)| !name.eq_ignore_ascii_case("connection"))
        );
    }

    #[tokio::test]
    async fn rejects_paths_outside_the_internal_prefix() {
        let fetch = AxumInternalFetch::new(router());
        let error = fetch
            .fetch(request("/api/v1/store/products"))
            .await
            .unwrap_err();
        assert!(matches!(error, BridgeError::NotFound(_)));

        let mut absolute = request("https://evil.example.com/internal/storefront/echo");
        absolute.method = "POST".to_owned();
        let error = fetch.fetch(absolute).await.unwrap_err();
        assert!(matches!(error, BridgeError::NotFound(_)));
    }

    #[tokio::test]
    async fn rejects_malformed_requests() {
        let fetch = AxumInternalFetch::new(router());

        let mut bad_method = request(&format!("{INTERNAL_STOREFRONT_PREFIX}/echo"));
        bad_method.method = "not a method".to_owned();
        assert!(matches!(
            fetch.fetch(bad_method).await.unwrap_err(),
            BridgeError::Invalid(_)
        ));

        let mut bad_header = request(&format!("{INTERNAL_STOREFRONT_PREFIX}/echo"));
        bad_header
            .headers
            .push(("bad header".to_owned(), "x".to_owned()));
        assert!(matches!(
            fetch.fetch(bad_header).await.unwrap_err(),
            BridgeError::Invalid(_)
        ));
    }

    #[tokio::test]
    async fn preserves_response_body_and_status() {
        let fetch = AxumInternalFetch::new(
            Router::new().fallback(|| async { (StatusCode::NOT_FOUND, "no route") }),
        );
        let response = fetch
            .fetch(BridgeRequest {
                store: store(),
                method: "GET".to_owned(),
                path: format!("{INTERNAL_STOREFRONT_PREFIX}/missing"),
                headers: Vec::new(),
                body: Bytes::new(),
            })
            .await
            .unwrap();
        assert_eq!(response.status, 404);
        assert_eq!(response.body, Bytes::from_static(b"no route"));
    }
}
