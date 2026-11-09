//! HTTP gateway between the commerce server and workerd.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use reqwest::Url;

use crate::SsrError;

/// A streamed response body.
pub type SsrStream = BoxStream<'static, Result<Bytes, SsrError>>;

/// A response returned by the SSR runtime.
pub struct SsrResponse {
    /// HTTP status.
    pub status: StatusCode,
    /// Response headers with hop-by-hop fields removed.
    pub headers: HeaderMap,
    /// Full or streamed body.
    pub body: SsrBody,
}

/// A complete or streamed SSR body.
pub enum SsrBody {
    /// A buffered body.
    Full(Bytes),
    /// A streaming body.
    Stream(SsrStream),
}

impl std::fmt::Debug for SsrBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full(body) => formatter.debug_tuple("Full").field(&body.len()).finish(),
            Self::Stream(_) => formatter.write_str("Stream(..)"),
        }
    }
}

impl std::fmt::Debug for SsrResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SsrResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body", &self.body)
            .finish()
    }
}

/// Trusted request context injected by the Rust gateway.
#[derive(Clone, Debug, Default)]
pub struct SsrRequestContext {
    /// Resolved store id.
    pub store_id: String,
    /// Active deployment id.
    pub deployment_id: String,
    /// Request correlation id.
    pub request_id: String,
    /// Store locale.
    pub locale: Option<String>,
    /// Store currency.
    pub currency: Option<String>,
    /// Optional market id.
    pub market_id: Option<String>,
    /// Public request scheme, after trusted-proxy handling.
    pub scheme: String,
    /// Whether this request targets a preview deployment.
    pub preview: bool,
}

/// Forwards storefront requests to one workerd service.
#[derive(Clone, Debug)]
pub struct SsrGateway {
    client: reqwest::Client,
    base_url: Url,
    internal_token: Option<String>,
}

impl SsrGateway {
    /// Creates a gateway for a workerd base URL such as
    /// `http://127.0.0.1:8788`.
    pub fn new(
        base_url: impl AsRef<str>,
        request_timeout: std::time::Duration,
    ) -> Result<Self, SsrError> {
        let mut base_url = Url::parse(base_url.as_ref()).map_err(|error| {
            SsrError::InvalidRequest(format!("invalid workerd base URL: {error}"))
        })?;
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(SsrError::InvalidRequest(
                "workerd base URL must use http or https".to_owned(),
            ));
        }
        if base_url.host_str().is_none() {
            return Err(SsrError::InvalidRequest(
                "workerd base URL must include a host".to_owned(),
            ));
        }
        if base_url.path() != "/" {
            return Err(SsrError::InvalidRequest(
                "workerd base URL must not include a path".to_owned(),
            ));
        }
        base_url.set_path("/");
        let client = reqwest::Client::builder()
            .timeout(request_timeout)
            .build()
            .map_err(|error| SsrError::InvalidRequest(format!("invalid HTTP client: {error}")))?;
        Ok(Self {
            client,
            base_url,
            internal_token: None,
        })
    }

    /// Injects a short-lived internal token into every forwarded request.
    #[must_use]
    pub fn with_internal_token(mut self, token: impl Into<String>) -> Self {
        let token = token.into();
        self.internal_token = (!token.is_empty()).then_some(token);
        self
    }

    /// Forwards a buffered request. The response body stays streamed.
    pub async fn forward(
        &self,
        request: axum::http::Request<Bytes>,
    ) -> Result<SsrResponse, SsrError> {
        let (parts, body) = request.into_parts();
        self.forward_parts(
            parts.method,
            parts.uri,
            parts.headers,
            reqwest::Body::from(body),
        )
        .await
    }

    /// Forwards a streaming request body with trusted FastCart context.
    ///
    /// Client-supplied `x-fastcart-*` and forwarding headers are removed before
    /// the trusted values are inserted.
    pub async fn forward_streaming(
        &self,
        request: axum::http::Request<Body>,
        context: &SsrRequestContext,
    ) -> Result<SsrResponse, SsrError> {
        let (mut parts, body) = request.into_parts();
        let original_host = parts
            .headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        strip_gateway_headers(&mut parts.headers);
        insert_header(&mut parts.headers, "x-fastcart-store-id", &context.store_id)?;
        insert_header(
            &mut parts.headers,
            "x-fastcart-deployment-id",
            &context.deployment_id,
        )?;
        insert_header(
            &mut parts.headers,
            "x-fastcart-request-id",
            &context.request_id,
        )?;
        if let Some(locale) = &context.locale {
            insert_header(&mut parts.headers, "x-fastcart-locale", locale)?;
        }
        if let Some(currency) = &context.currency {
            insert_header(&mut parts.headers, "x-fastcart-currency", currency)?;
        }
        if let Some(market_id) = &context.market_id {
            insert_header(&mut parts.headers, "x-fastcart-market-id", market_id)?;
        }
        if let Some(host) = original_host {
            insert_header(&mut parts.headers, "x-forwarded-host", &host)?;
        }
        let scheme = if context.scheme.is_empty() {
            "http"
        } else {
            context.scheme.as_str()
        };
        insert_header(&mut parts.headers, "x-forwarded-proto", scheme)?;
        if let Some(token) = &self.internal_token {
            insert_header(&mut parts.headers, "x-fastcart-internal-token", token)?;
        }
        if context.preview {
            insert_header(&mut parts.headers, "x-fastcart-preview", "1")?;
        }
        let body = reqwest::Body::wrap_stream(body.into_data_stream());
        self.forward_parts(parts.method, parts.uri, parts.headers, body)
            .await
    }

    async fn forward_parts(
        &self,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: reqwest::Body,
    ) -> Result<SsrResponse, SsrError> {
        let path = uri
            .path_and_query()
            .map_or("/", axum::http::uri::PathAndQuery::as_str);
        let url = self
            .base_url
            .join(path)
            .map_err(|error| SsrError::InvalidRequest(format!("invalid SSR path: {error}")))?;
        let headers = filtered_headers(&headers);
        let response = self
            .client
            .request(method, url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|error| SsrError::Request(error.to_string()))?;
        let status = StatusCode::from_u16(response.status().as_u16())
            .map_err(|error| SsrError::Response(format!("invalid status: {error}")))?;
        let headers = filtered_headers(response.headers());
        let stream = response
            .bytes_stream()
            .map(|chunk| chunk.map_err(|error| SsrError::Response(error.to_string())));
        Ok(SsrResponse {
            status,
            headers,
            body: SsrBody::Stream(Box::pin(stream)),
        })
    }
}

fn filtered_headers(source: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in source {
        if is_hop_by_hop(name.as_str()) || name == axum::http::header::HOST {
            continue;
        }
        headers.append(name, value.clone());
    }
    headers
}

fn strip_gateway_headers(headers: &mut HeaderMap) {
    let remove: Vec<HeaderName> = headers
        .keys()
        .filter(|name| {
            let name = name.as_str();
            name.starts_with("x-fastcart-")
                || matches!(
                    name,
                    "forwarded" | "x-forwarded-host" | "x-forwarded-proto" | "x-real-ip"
                )
        })
        .cloned()
        .collect();
    for name in remove {
        headers.remove(name);
    }
}

fn insert_header(headers: &mut HeaderMap, name: &'static str, value: &str) -> Result<(), SsrError> {
    let value = HeaderValue::from_str(value)
        .map_err(|_| SsrError::InvalidRequest(format!("invalid value for {name}")))?;
    headers.insert(HeaderName::from_static(name), value);
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use axum::http::{HeaderValue, header};

    use super::*;

    #[test]
    fn hop_by_hop_headers_are_removed() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
        headers.insert(header::HOST, HeaderValue::from_static("shop.example"));
        headers.insert(header::ACCEPT, HeaderValue::from_static("text/html"));
        let filtered = filtered_headers(&headers);
        assert!(!filtered.contains_key(header::CONNECTION));
        assert!(!filtered.contains_key(header::HOST));
        assert_eq!(filtered.get(header::ACCEPT).unwrap(), "text/html");
    }

    #[test]
    fn gateway_headers_are_stripped_before_injection() {
        let mut headers = HeaderMap::new();
        headers.insert("x-fastcart-store-id", HeaderValue::from_static("attacker"));
        headers.insert("x-forwarded-host", HeaderValue::from_static("attacker"));
        headers.insert(header::COOKIE, HeaderValue::from_static("session=ok"));
        strip_gateway_headers(&mut headers);
        assert!(!headers.contains_key("x-fastcart-store-id"));
        assert!(!headers.contains_key("x-forwarded-host"));
        assert_eq!(headers.get(header::COOKIE).unwrap(), "session=ok");
    }

    #[test]
    fn base_url_must_be_rooted() {
        assert!(
            SsrGateway::new(
                "http://127.0.0.1:8788/prefix",
                std::time::Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(
            SsrGateway::new("ftp://127.0.0.1:8788", std::time::Duration::from_secs(1)).is_err()
        );
        assert!(
            SsrGateway::new("http://127.0.0.1:8788", std::time::Duration::from_secs(1)).is_ok()
        );
    }
}
