//! Trusted store resolution from normalized hostnames, with caching.

use fastcart_infrastructure::StoreCache;

use crate::domain::normalize_hostname;
use crate::error::{PlatformError, PlatformResult};
use crate::repository::PlatformStore;
use crate::store::Store;

const CACHE_NAMESPACE: &str = "store-domain";

/// Resolves incoming hostnames to stores using trusted platform metadata.
#[derive(Clone, Debug)]
pub struct StoreResolver {
    platform: PlatformStore,
    cache: StoreCache,
}

impl StoreResolver {
    /// Creates a resolver backed by the platform registry.
    #[must_use]
    pub fn new(platform: PlatformStore, cache: StoreCache) -> Self {
        Self { platform, cache }
    }

    /// Resolves a hostname to a store, using the cache when possible.
    pub async fn resolve(&self, host: &str) -> PlatformResult<Option<Store>> {
        let hostname = normalize_hostname(host);
        if let Some(store) = self
            .cache
            .get_global::<Store>(CACHE_NAMESPACE, &hostname)
            .await
        {
            return Ok(Some(store));
        }
        let store = self.platform.get_store_by_domain(&hostname).await?;
        if let Some(store) = store.as_ref() {
            self.cache
                .insert_global(CACHE_NAMESPACE, &hostname, store)
                .await;
        }
        Ok(store)
    }

    /// Resolves a hostname and rejects stores that cannot serve traffic.
    pub async fn resolve_active(&self, host: &str) -> PlatformResult<Store> {
        let store = self
            .resolve(host)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(normalize_hostname(host)))?;
        store.ensure_active()?;
        Ok(store)
    }

    /// Removes a hostname from the cache.
    pub async fn invalidate_hostname(&self, host: &str) {
        let hostname = normalize_hostname(host);
        self.cache.remove_global(CACHE_NAMESPACE, &hostname).await;
    }

    /// Clears all resolver entries. Called after lifecycle or domain changes.
    pub async fn invalidate_all(&self) {
        self.cache.clear().await;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use fastcart_infrastructure::StoreCache;

    use super::StoreResolver;
    use crate::repository::PlatformStore;
    use crate::store::{CreateStoreInput, StoreStatus};

    async fn fixture() -> (StoreResolver, PlatformStore) {
        let platform = PlatformStore::in_memory().await.unwrap();
        let resolver = StoreResolver::new(
            platform.clone(),
            StoreCache::new(100, Duration::from_secs(60)),
        );
        (resolver, platform)
    }

    #[tokio::test]
    async fn resolves_registered_domains() {
        let (resolver, platform) = fixture().await;
        platform
            .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
            .await
            .unwrap();

        let store = resolver
            .resolve("DEMO.example.com:443")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(store.slug, "demo");
        // Second lookup is served from cache.
        let cached = resolver.resolve("demo.example.com").await.unwrap().unwrap();
        assert_eq!(cached.id, store.id);
    }

    #[tokio::test]
    async fn unknown_domains_do_not_resolve() {
        let (resolver, _platform) = fixture().await;
        assert!(
            resolver
                .resolve("missing.example.com")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            resolver
                .resolve_active("missing.example.com")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn inactive_stores_are_rejected() {
        let (resolver, platform) = fixture().await;
        let store = platform
            .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
            .await
            .unwrap();

        assert!(resolver.resolve_active("demo.example.com").await.is_err());

        platform
            .update_status(&store.id, StoreStatus::Active)
            .await
            .unwrap();
        resolver.invalidate_all().await;
        assert!(resolver.resolve_active("demo.example.com").await.is_ok());

        platform
            .update_status(&store.id, StoreStatus::Suspended)
            .await
            .unwrap();
        resolver.invalidate_all().await;
        assert!(resolver.resolve_active("demo.example.com").await.is_err());
    }
}
