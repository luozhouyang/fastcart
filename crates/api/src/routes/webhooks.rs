//! Provider webhook callbacks.
//!
//! Webhook routes are public at the network layer; every provider adapter must
//! verify signatures and process events idempotently. Until a provider is
//! registered, callbacks are rejected as not implemented.

use std::sync::Arc;

use axum::Router;

use super::not_implemented;
use crate::state::AppContext;

/// Builds the webhook router.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new().fallback(not_implemented)
}
