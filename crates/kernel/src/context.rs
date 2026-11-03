//! Trusted request and store contexts.

use serde::{Deserialize, Serialize};

use crate::ids::StoreId;
use crate::money::CurrencyCode;

/// The trusted context of the store serving a request.
///
/// Store context is established by the store resolver on the Rust side. Clients
/// must never be able to select an arbitrary store through a header or query
/// parameter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoreContext {
    /// Resolved store identifier.
    pub store_id: StoreId,
    /// Normalized hostname that resolved to this store.
    pub domain: String,
    /// Store locale, for example `en-US`.
    pub locale: String,
    /// Store display currency.
    pub currency: CurrencyCode,
    /// IANA time zone identifier.
    pub timezone: String,
}

/// Request-scoped context propagated explicitly through the application.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RequestContext {
    /// Correlation identifier for logging and error responses.
    pub request_id: String,
    /// Resolved store, when the route is store-scoped.
    pub store: Option<StoreContext>,
    /// Authenticated actor, when applicable.
    pub actor_id: Option<String>,
}

impl RequestContext {
    /// Creates a request context with a fresh correlation identifier.
    #[must_use]
    pub fn new() -> Self {
        Self {
            request_id: uuid::Uuid::now_v7().to_string(),
            store: None,
            actor_id: None,
        }
    }
}
