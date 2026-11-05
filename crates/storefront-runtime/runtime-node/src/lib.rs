//! Node.js sidecar storefront runtime.
//!
//! This adapter is the documented fallback when embedded V8 cannot support a
//! required TanStack Start capability. The sidecar owns the JavaScript runtime;
//! the host talks to it over a small HTTP contract:
//!
//! - `POST {base}/render` with a `RenderPayload`, optionally authenticated with
//!   a bearer token. The sidecar answers either with a JSON `RenderResult`
//!   (buffered body) or, when the theme streams and the payload asks for it,
//!   with the themed status, headers and raw HTML bytes plus the
//!   `x-fastcart-mode: stream` marker.
//! - `GET {base}/health` for readiness.
//!
//! Rendering is bounded by the request timeout, and sidecar failures are
//! reported as runtime errors instead of leaking into the host.

use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fastcart_kernel::StoreContext;
use fastcart_runtime_api::{
    CapabilityGrant, PageContext, RuntimeBody, RuntimeError, RuntimeMode, RuntimeRequest,
    RuntimeResponse, RuntimeTheme, StorefrontRuntime,
};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};

/// Path used to render a storefront request on the sidecar.
pub const RENDER_PATH: &str = "/render";
/// Path used to check sidecar readiness.
pub const HEALTH_PATH: &str = "/health";
/// Sidecar response header marking a raw streaming body.
pub const STREAM_MODE_HEADER: &str = "x-fastcart-mode";
/// Header value that marks a raw streaming body.
const STREAM_MODE_VALUE: &str = "stream";
/// Maximum decoded response body accepted from the sidecar.
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// Node.js sidecar runtime adapter.
#[derive(Clone, Debug)]
pub struct NodeRuntime {
    base_url: String,
    secret: Option<String>,
    client: reqwest::Client,
}

impl NodeRuntime {
    /// Creates an adapter for a sidecar listening at `base_url`.
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            secret: None,
            client: reqwest::Client::new(),
        }
    }

    /// Authenticates requests to the sidecar with a bearer token.
    #[must_use]
    pub fn with_secret(mut self, secret: impl Into<String>) -> Self {
        self.secret = Some(secret.into());
        self
    }

    /// Uses a preconfigured HTTP client.
    #[must_use]
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Returns the sidecar base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    async fn dispatch(
        &self,
        payload: &RenderPayload,
        timeout_ms: u64,
        stream_timeout_ms: u64,
    ) -> Result<RuntimeResponse, RuntimeError> {
        // The reqwest timeout covers the whole response body. Give streams the
        // larger budget; the stream itself also enforces it per chunk.
        let client_timeout_ms = if payload.streaming {
            stream_timeout_ms.max(timeout_ms)
        } else {
            timeout_ms
        };
        let mut builder = self
            .client
            .post(self.endpoint(RENDER_PATH))
            .timeout(Duration::from_millis(
                client_timeout_ms.saturating_add(1_000),
            ))
            .json(payload);
        if let Some(secret) = &self.secret {
            builder = builder.bearer_auth(secret);
        }

        let response = builder.send().await.map_err(|error| {
            RuntimeError::FetchBridge(format!("node sidecar request failed: {error}"))
        })?;
        if !response.status().is_success() {
            return Err(RuntimeError::FetchBridge(format!(
                "node sidecar returned HTTP {}",
                response.status()
            )));
        }
        if is_stream_response(&response) {
            return stream_response(response, stream_timeout_ms);
        }
        let result: RenderResult = response.json().await.map_err(|error| {
            RuntimeError::FetchBridge(format!("invalid node sidecar response: {error}"))
        })?;

        if !(100..=599).contains(&result.status) {
            return Err(RuntimeError::FetchBridge(format!(
                "node sidecar returned an invalid status: {}",
                result.status
            )));
        }
        let headers = validate_headers(result.headers)?;
        let body = match result.body {
            Some(encoded) => {
                let decoded = STANDARD.decode(encoded).map_err(|error| {
                    RuntimeError::FetchBridge(format!(
                        "invalid node sidecar body encoding: {error}"
                    ))
                })?;
                if decoded.len() > MAX_RESPONSE_BYTES {
                    return Err(RuntimeError::FetchBridge(format!(
                        "node sidecar response exceeded {MAX_RESPONSE_BYTES} bytes"
                    )));
                }
                decoded
            }
            None => Vec::new(),
        };
        Ok(RuntimeResponse {
            status: result.status,
            headers,
            body: RuntimeBody::Full(body.into()),
        })
    }
}

/// Returns whether the sidecar answered with a raw streaming body.
fn is_stream_response(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(STREAM_MODE_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case(STREAM_MODE_VALUE))
}

/// Converts a raw sidecar stream into a bounded [`RuntimeBody::Stream`].
///
/// Theme headers are carried as ordinary HTTP headers. The mode marker and
/// hop-by-hop headers are internal to the bridge and never reach the client.
fn stream_response(
    response: reqwest::Response,
    stream_timeout_ms: u64,
) -> Result<RuntimeResponse, RuntimeError> {
    let status = response.status().as_u16();
    let mut headers = Vec::new();
    for (name, value) in response.headers() {
        let name = name.as_str();
        if name.eq_ignore_ascii_case(STREAM_MODE_HEADER)
            || name.eq_ignore_ascii_case("content-length")
            || is_hop_by_hop(name)
        {
            continue;
        }
        let value = value.to_str().map_err(|_| {
            RuntimeError::FetchBridge("node sidecar returned an invalid header".to_owned())
        })?;
        headers.push((name.to_owned(), value.to_owned()));
    }
    let headers = validate_headers(headers)?;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(stream_timeout_ms.max(1));
    let chunks = response.bytes_stream();
    let limited = stream::try_unfold(
        (chunks, 0usize, deadline),
        move |(mut chunks, total, deadline)| async move {
            let stream_budget = stream_timeout_ms;
            let next = tokio::time::timeout_at(deadline, chunks.next())
                .await
                .map_err(|_| RuntimeError::Timeout(stream_budget))?;
            let Some(item) = next else {
                return Ok(None);
            };
            let chunk = item.map_err(|error| {
                RuntimeError::FetchBridge(format!("node sidecar stream failed: {error}"))
            })?;
            let total = total.saturating_add(chunk.len());
            if total > MAX_RESPONSE_BYTES {
                return Err(RuntimeError::FetchBridge(format!(
                    "node sidecar response exceeded {MAX_RESPONSE_BYTES} bytes"
                )));
            }
            Ok(Some((chunk, (chunks, total, deadline))))
        },
    );
    Ok(RuntimeResponse {
        status,
        headers,
        body: RuntimeBody::Stream(Box::pin(limited)),
    })
}

/// Returns true for headers that must not be forwarded between connections.
fn is_hop_by_hop(name: &str) -> bool {
    [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ]
    .iter()
    .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

#[async_trait::async_trait]
impl StorefrontRuntime for NodeRuntime {
    async fn render(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        let timeout_ms = request.timeout_ms.max(1);
        let stream_timeout_ms = request.stream_timeout_ms.max(1);
        let payload = RenderPayload {
            request_id: request.request_id,
            store: StorePayload::from(&request.store),
            method: request.method,
            url: request.url,
            headers: request.headers,
            body: request.body.as_ref().map(|body| STANDARD.encode(body)),
            theme: request.theme,
            page: request.page,
            data: request.data,
            capabilities: request.capabilities,
            mode: request.mode,
            timeout_ms,
            streaming: request.streaming,
            stream_timeout_ms,
        };

        match tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            self.dispatch(&payload, timeout_ms, stream_timeout_ms),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(RuntimeError::Timeout(timeout_ms)),
        }
    }

    async fn health_check(&self) -> Result<(), RuntimeError> {
        let response = self
            .client
            .get(self.endpoint(HEALTH_PATH))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .map_err(|error| {
                RuntimeError::WorkerUnavailable(format!("node sidecar unreachable: {error}"))
            })?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(RuntimeError::WorkerUnavailable(format!(
                "node sidecar unhealthy: HTTP {}",
                response.status()
            )))
        }
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
}

fn validate_headers(headers: Vec<(String, String)>) -> Result<Vec<(String, String)>, RuntimeError> {
    headers
        .into_iter()
        .map(|(name, value)| {
            let valid_name = !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && byte != b':');
            let valid_value = !value
                .bytes()
                .any(|byte| byte.is_ascii_control() && byte != b'\t');
            if valid_name && valid_value {
                Ok((name, value))
            } else {
                Err(RuntimeError::FetchBridge(
                    "node sidecar returned an invalid header".to_owned(),
                ))
            }
        })
        .collect()
}

#[derive(Debug, Serialize)]
struct RenderPayload {
    request_id: String,
    store: StorePayload,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    theme: Option<RuntimeTheme>,
    page: Option<PageContext>,
    data: Option<serde_json::Value>,
    capabilities: BTreeMap<String, CapabilityGrant>,
    mode: RuntimeMode,
    timeout_ms: u64,
    streaming: bool,
    stream_timeout_ms: u64,
}

#[derive(Debug, Serialize)]
struct StorePayload {
    id: String,
    domain: String,
    locale: String,
    currency: String,
    timezone: String,
}

impl From<&StoreContext> for StorePayload {
    fn from(store: &StoreContext) -> Self {
        Self {
            id: store.store_id.to_string(),
            domain: store.domain.clone(),
            locale: store.locale.clone(),
            currency: store.currency.to_string(),
            timezone: store.timezone.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RenderResult {
    status: u16,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    body: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, serve};
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use fastcart_kernel::{CurrencyCode, StoreContext, StoreId};
    use fastcart_runtime_api::{
        PageContext, RuntimeBody, RuntimeError, RuntimeMode, RuntimeRequest, RuntimeTheme,
        StorefrontRuntime,
    };
    use futures::StreamExt;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use super::NodeRuntime;

    type Seen = Arc<Mutex<Option<Value>>>;

    async fn spawn(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            serve(listener, app).await.unwrap();
        });
        format!("http://{address}")
    }

    fn request(timeout_ms: u64) -> RuntimeRequest {
        RuntimeRequest {
            request_id: "req_1".to_owned(),
            store: StoreContext {
                store_id: StoreId::from_string("store_1"),
                domain: "shop.example.com".to_owned(),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").unwrap(),
                timezone: "UTC".to_owned(),
            },
            method: "POST".to_owned(),
            url: "/products?page=2".to_owned(),
            headers: vec![("accept".to_owned(), "text/html".to_owned())],
            body: Some("hello".into()),
            theme: Some(RuntimeTheme {
                id: "default".to_owned(),
                version: "1.0.0".to_owned(),
                runtime: "node".to_owned(),
                bundle_path: "themes/default".to_owned(),
                entry: "dist/server.js".to_owned(),
                settings: json!({"brand_color": "#111111"}),
                assets_manifest: Some(json!({"entries": {"client": "dist/client"}})),
                assets_base_url: Some("/api/v1/store/theme-assets".to_owned()),
            }),
            page: Some(PageContext {
                id: "page_1".to_owned(),
                handle: "home".to_owned(),
                status: "published".to_owned(),
                page_type: "home".to_owned(),
                title: "Home".to_owned(),
                seo: json!({"title": "Home"}),
                body: json!({"schema_version": 1, "content": []}),
            }),
            data: Some(json!({"grid-1": {"products": []}})),
            capabilities: BTreeMap::new(),
            mode: RuntimeMode::Production,
            timeout_ms,
            streaming: false,
            stream_timeout_ms: timeout_ms,
        }
    }

    async fn render(
        State(seen): State<Seen>,
        headers: HeaderMap,
        Json(payload): Json<Value>,
    ) -> Json<Value> {
        let authenticated = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some("Bearer s3cret");
        *seen.lock().unwrap() = Some(payload.clone());
        let encodes_hello = payload["body"].as_str() == Some(STANDARD.encode("hello").as_str())
            && payload["method"] == "POST"
            && payload["url"] == "/products?page=2"
            && payload["store"]["domain"] == "shop.example.com"
            && payload["theme"]["id"] == "default"
            && payload["theme"]["entry"] == "dist/server.js"
            && payload["timeout_ms"] == 500;
        Json(json!({
            "status": if encodes_hello && authenticated { 201 } else { 400 },
            "headers": [["x-node", "rendered"], ["set-cookie", "a=1"]],
            "body": STANDARD.encode("rendered page"),
        }))
    }

    async fn failing_render() -> (StatusCode, &'static str) {
        (StatusCode::INTERNAL_SERVER_ERROR, "boom")
    }

    async fn invalid_body() -> Json<Value> {
        Json(json!({"status": 200, "headers": [], "body": "not-base64!!!"}))
    }

    async fn slow_render() -> Json<Value> {
        tokio::time::sleep(Duration::from_millis(200)).await;
        Json(json!({"status": 200, "headers": [], "body": STANDARD.encode("late")}))
    }

    async fn unhealthy() -> StatusCode {
        StatusCode::SERVICE_UNAVAILABLE
    }

    async fn slow_stream() -> axum::response::Response {
        use axum::body::Bytes;
        use axum::response::IntoResponse;

        let body = axum::body::Body::from_stream(futures::stream::once(async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok::<_, std::io::Error>(Bytes::from_static(b"late"))
        }));
        (
            StatusCode::OK,
            [("x-fastcart-mode", "stream"), ("content-type", "text/html")],
            body,
        )
            .into_response()
    }

    async fn streaming_render() -> axum::response::Response {
        use axum::body::Bytes;
        use axum::response::IntoResponse;

        let body = axum::body::Body::from_stream(futures::stream::iter([
            Ok::<_, std::io::Error>(Bytes::from_static(b"<html>")),
            Ok(Bytes::from_static(b"</html>")),
        ]));
        (
            StatusCode::OK,
            [
                ("x-fastcart-mode", "stream"),
                ("content-type", "text/html; charset=utf-8"),
                ("x-theme", "default"),
                ("transfer-encoding", "chunked"),
            ],
            body,
        )
            .into_response()
    }

    #[tokio::test]
    async fn renders_through_the_sidecar() {
        let seen: Seen = Arc::default();
        let app = Router::new()
            .route("/render", post(render))
            .route("/health", get(|| async { StatusCode::OK }))
            .with_state(seen.clone());
        let runtime = NodeRuntime::new(spawn(app).await).with_secret("s3cret");

        let response = runtime.render(request(500)).await.unwrap();
        assert_eq!(response.status, 201);
        let RuntimeBody::Full(body) = response.body else {
            unreachable!()
        };
        assert_eq!(&body[..], b"rendered page");
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "set-cookie" && value == "a=1")
        );

        let payload = seen.lock().unwrap().clone().unwrap();
        assert_eq!(payload["request_id"], "req_1");
        assert_eq!(payload["store"]["id"], "store_1");
        assert_eq!(payload["store"]["currency"], "USD");

        runtime.health_check().await.unwrap();
    }

    #[tokio::test]
    async fn streams_raw_sidecar_bodies() {
        let app = Router::new().route("/render", post(streaming_render));
        let runtime = NodeRuntime::new(spawn(app).await);

        let mut request = request(500);
        request.streaming = true;
        request.stream_timeout_ms = 2_000;
        let response = runtime.render(request).await.unwrap();

        assert_eq!(response.status, 200);
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "x-theme" && value == "default")
        );
        assert!(
            !response
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("x-fastcart-mode"))
        );
        assert!(
            !response
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("transfer-encoding"))
        );

        let RuntimeBody::Stream(mut stream) = response.body else {
            panic!("expected a streaming body");
        };
        let mut collected = Vec::new();
        while let Some(chunk) = stream.next().await {
            collected.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(collected, b"<html></html>");
    }

    #[tokio::test]
    async fn enforces_the_stream_budget() {
        let app = Router::new().route("/render", post(slow_stream));
        let runtime = NodeRuntime::new(spawn(app).await);

        let mut request = request(500);
        request.streaming = true;
        request.stream_timeout_ms = 50;
        let response = runtime.render(request).await.unwrap();
        let RuntimeBody::Stream(mut stream) = response.body else {
            panic!("expected a streaming body");
        };
        let error = stream.next().await.expect("one chunk").unwrap_err();
        assert!(matches!(error, RuntimeError::Timeout(50)));
    }

    #[tokio::test]
    async fn reports_sidecar_failures() {
        let failing = Router::new().route("/render", post(failing_render));
        let runtime = NodeRuntime::new(spawn(failing).await);
        assert!(matches!(
            runtime.render(request(500)).await.unwrap_err(),
            RuntimeError::FetchBridge(_)
        ));

        let invalid = Router::new().route("/render", post(invalid_body));
        let runtime = NodeRuntime::new(spawn(invalid).await);
        assert!(matches!(
            runtime.render(request(500)).await.unwrap_err(),
            RuntimeError::FetchBridge(_)
        ));

        let unhealthy_app = Router::new().route("/health", get(unhealthy));
        let runtime = NodeRuntime::new(spawn(unhealthy_app).await);
        assert!(matches!(
            runtime.health_check().await.unwrap_err(),
            RuntimeError::WorkerUnavailable(_)
        ));

        // An unreachable sidecar is a worker failure, not a panic.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let runtime = NodeRuntime::new(format!("http://{address}"));
        assert!(matches!(
            runtime.health_check().await.unwrap_err(),
            RuntimeError::WorkerUnavailable(_)
        ));
    }

    #[tokio::test]
    async fn honours_request_timeouts() {
        let app = Router::new().route("/render", post(slow_render));
        let runtime = NodeRuntime::new(spawn(app).await);
        assert!(matches!(
            runtime.render(request(10)).await.unwrap_err(),
            RuntimeError::Timeout(10)
        ));
        runtime.shutdown().await.unwrap();
    }
}
