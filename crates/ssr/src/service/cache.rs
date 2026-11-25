//! Store cache invalidation.

use super::*;

impl SsrService {
    /// Invalidates cached SSR responses for one store.
    pub async fn invalidate_store_cache(&self, store_id: &str) {
        let prefix = format!("{store_id}:");
        self.response_cache
            .write()
            .await
            .retain(|key, _| !key.starts_with(&prefix));
    }
}
