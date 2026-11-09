//! HTTP gateway between the commerce server and workerd.

use axum::http::{HeaderMap, StatusCode};
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

/// Forwards storefront requests to one workerd service.
#[derive(Clone, Debug)]
pub struct SsrGateway {
    client: reqwest::Client,
    base_url: Url,
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
        Ok(Self { client, base_url })
    }

    /// Forwards a request and returns the workerd response. The response body
    /// is exposed as a stream so the caller can preserve SSR streaming.
    pub async fn forward(
        &self,
        request: axum::http::Request<Bytes>,
    ) -> Result<SsrResponse, SsrError> {
        let (parts, body) = request.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .map_or("/", axum::http::uri::PathAndQuery::as_str);
        let url = self
            .base_url
            .join(path)
            .map_err(|error| SsrError::InvalidRequest(format!("invalid SSR path: {error}")))?;
        let headers = filtered_headers(&parts.headers);
        let response = self
            .client
            .request(parts.method, url)
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
