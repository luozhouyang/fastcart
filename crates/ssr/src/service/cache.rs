//! Store cache invalidation.

use super::*;

impl SsrService {
    /// Invalidates cached SSR responses for one store.
    pub async fn invalidate_store_cache(&self, store_id: &str) {
        let prefix = format!("{store_id}:");
        let result = self
            .response_cache
            .invalidate_entries_if(move |key, _| key.starts_with(&prefix));
        if let Err(error) = result {
            tracing::warn!(store_id, error = %error, "failed to invalidate cached SSR responses");
        }
    }
}
