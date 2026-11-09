//! Credential model shared by backends and the in-memory manager.

use serde::{Deserialize, Serialize};

/// One internal SSR bearer token and its scope.
#[derive(Clone, Serialize, Deserialize)]
pub struct CredentialToken {
    /// Bearer token value.
    pub token: String,
    /// Store this token may access; `None` means all stores.
    pub store_id: Option<String>,
    /// Unix expiry; `None` means no scheduled expiry.
    pub expires_at: Option<i64>,
    /// Whether this credential is for a preview deployment.
    #[serde(default)]
    pub preview: bool,
}

impl std::fmt::Debug for CredentialToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialToken")
            .field("token", &"<redacted>")
            .field("store_id", &self.store_id)
            .field("expires_at", &self.expires_at)
            .field("preview", &self.preview)
            .finish()
    }
}

/// A complete credential set plus the backend revision it came from.
#[derive(Clone, Debug)]
pub struct CredentialSnapshot {
    /// Backend revision, for compare-and-swap writes.
    pub revision: Option<String>,
    /// Active credentials.
    pub tokens: Vec<CredentialToken>,
}
