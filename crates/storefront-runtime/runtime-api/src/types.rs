//! Runtime request and response models.

use bytes::Bytes;
use fastcart_kernel::StoreContext;
use futures::stream::BoxStream;

use crate::error::RuntimeError;

/// Active theme bundle context supplied to the runtime.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RuntimeTheme {
    /// Theme manifest identifier.
    pub id: String,
    /// Theme version.
    pub version: String,
    /// Runtime expected by the theme (`v8` or `node`).
    pub runtime: String,
    /// Bundle-relative path of the theme bundle.
    pub bundle_path: String,
    /// Entry file inside the bundle.
    pub entry: String,
    /// Active theme settings.
    pub settings: serde_json::Value,
}

/// One request handed to a storefront runtime.
#[derive(Clone, Debug)]
pub struct RuntimeRequest {
    /// Correlation identifier.
    pub request_id: String,
    /// Trusted store context resolved by the host.
    pub store: StoreContext,
    /// HTTP method.
    pub method: String,
    /// Request URL including path and query string.
    pub url: String,
    /// Request headers. Repeated headers are preserved as separate entries.
    pub headers: Vec<(String, String)>,
    /// Optional request body.
    pub body: Option<Bytes>,
    /// Active theme bundle, if the request is a storefront render.
    pub theme: Option<RuntimeTheme>,
    /// Execution timeout in milliseconds.
    pub timeout_ms: u64,
}

/// One response produced by a storefront runtime.
#[derive(Debug)]
pub struct RuntimeResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers. Repeated headers are preserved as separate entries.
    pub headers: Vec<(String, String)>,
    /// Response body.
    pub body: RuntimeBody,
}

/// A complete or streaming runtime response body.
pub enum RuntimeBody {
    /// A fully buffered body.
    Full(Bytes),
    /// A streaming body.
    Stream(BoxStream<'static, Result<Bytes, RuntimeError>>),
}

impl std::fmt::Debug for RuntimeBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full(body) => formatter.debug_tuple("Full").field(&body.len()).finish(),
            Self::Stream(_) => formatter.write_str("Stream(..)"),
        }
    }
}

impl RuntimeBody {
    /// Creates a fully buffered body.
    #[must_use]
    pub fn full(body: impl Into<Bytes>) -> Self {
        Self::Full(body.into())
    }

    /// Creates a streaming body.
    #[must_use]
    pub fn stream(stream: BoxStream<'static, Result<Bytes, RuntimeError>>) -> Self {
        Self::Stream(stream)
    }
}
