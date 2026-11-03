//! In-process cache with mandatory store-scoped keys.
//!
//! Every store-level key includes the store identifier. Invalidation bumps a
//! per-store generation counter so stale entries become unreachable without
//! scanning the whole cache.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fastcart_kernel::StoreId;
use moka::future::Cache;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Store-aware JSON cache backed by Moka.
#[derive(Clone, Debug)]
pub struct StoreCache {
    inner: Cache<String, Value>,
    generations: Arc<Mutex<HashMap<String, u64>>>,
}

impl StoreCache {
    /// Creates a cache with the given maximum entry count and entry TTL.
    #[must_use]
    pub fn new(max_capacity: u64, ttl: Duration) -> Self {
        Self {
            inner: Cache::builder()
                .max_capacity(max_capacity.max(1))
                .time_to_live(ttl)
                .build(),
            generations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Creates a cache with the default capacity (10 000) and 60 second TTL.
    #[must_use]
    pub fn default_capacity() -> Self {
        Self::new(10_000, Duration::from_secs(60))
    }

    /// Builds a complete store-scoped cache key.
    #[must_use]
    pub fn key(&self, store_id: &StoreId, namespace: &str, key: &str) -> String {
        let generation = self.generations.lock().map_or(0, |generations| {
            generations.get(store_id.as_str()).copied().unwrap_or(0)
        });
        format!(
            "store:{}:v{generation}:{namespace}:{key}",
            store_id.as_str()
        )
    }

    /// Reads a typed entry.
    pub async fn get<T: DeserializeOwned>(
        &self,
        store_id: &StoreId,
        namespace: &str,
        key: &str,
    ) -> Option<T> {
        let value = self.inner.get(&self.key(store_id, namespace, key)).await?;
        serde_json::from_value(value).ok()
    }

    /// Stores a typed entry.
    pub async fn insert<T: Serialize>(
        &self,
        store_id: &StoreId,
        namespace: &str,
        key: &str,
        value: &T,
    ) {
        if let Ok(value) = serde_json::to_value(value) {
            self.inner
                .insert(self.key(store_id, namespace, key), value)
                .await;
        }
    }

    /// Reads an entry, computing and storing it on a miss.
    pub async fn get_or_insert<T, F, Fut>(
        &self,
        store_id: &StoreId,
        namespace: &str,
        key: &str,
        loader: F,
    ) -> Option<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Option<T>>,
    {
        if let Some(value) = self.get::<T>(store_id, namespace, key).await {
            return Some(value);
        }
        let value = loader().await?;
        self.insert(store_id, namespace, key, &value).await;
        Some(value)
    }

    /// Reads a non-tenant-scoped entry, used for domain resolution before a
    /// store has been identified.
    pub async fn get_global<T: DeserializeOwned>(&self, namespace: &str, key: &str) -> Option<T> {
        let value = self.inner.get(&format!("global:{namespace}:{key}")).await?;
        serde_json::from_value(value).ok()
    }

    /// Stores a non-tenant-scoped entry.
    pub async fn insert_global<T: Serialize>(&self, namespace: &str, key: &str, value: &T) {
        if let Ok(value) = serde_json::to_value(value) {
            self.inner
                .insert(format!("global:{namespace}:{key}"), value)
                .await;
        }
    }

    /// Removes a non-tenant-scoped entry.
    pub async fn remove_global(&self, namespace: &str, key: &str) {
        self.inner
            .invalidate(&format!("global:{namespace}:{key}"))
            .await;
    }

    /// Removes a single entry.
    pub async fn remove(&self, store_id: &StoreId, namespace: &str, key: &str) {
        self.inner
            .invalidate(&self.key(store_id, namespace, key))
            .await;
    }

    /// Invalidates every entry for a store by bumping its generation.
    pub fn invalidate_store(&self, store_id: &StoreId) {
        if let Ok(mut generations) = self.generations.lock() {
            let entry = generations.entry(store_id.as_str().to_owned()).or_insert(0);
            *entry = entry.wrapping_add(1);
        }
    }

    /// Invalidates every entry in the cache.
    pub async fn clear(&self) {
        self.inner.invalidate_all();
        self.inner.run_pending_tasks().await;
        if let Ok(mut generations) = self.generations.lock() {
            generations.clear();
        }
    }

    /// Returns the number of cached entries visible to Moka.
    pub async fn entry_count(&self) -> u64 {
        self.inner.run_pending_tasks().await;
        self.inner.entry_count()
    }
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::StoreId;

    use super::StoreCache;

    #[tokio::test]
    async fn keys_include_the_store() {
        let cache = StoreCache::default_capacity();
        let first = StoreId::new();
        let second = StoreId::new();

        assert!(cache.key(&first, "products", "x").contains(first.as_str()));
        assert_ne!(
            cache.key(&first, "products", "x"),
            cache.key(&second, "products", "x")
        );
    }

    #[tokio::test]
    async fn entries_are_isolated_between_stores() {
        let cache = StoreCache::default_capacity();
        let first = StoreId::new();
        let second = StoreId::new();

        cache.insert(&first, "settings", "currency", &"USD").await;
        assert_eq!(
            cache.get::<String>(&first, "settings", "currency").await,
            Some("USD".to_owned())
        );
        assert_eq!(
            cache.get::<String>(&second, "settings", "currency").await,
            None
        );
    }

    #[tokio::test]
    async fn invalidating_a_store_hides_previous_generation() {
        let cache = StoreCache::default_capacity();
        let store = StoreId::new();

        cache.insert(&store, "settings", "currency", &"USD").await;
        cache.invalidate_store(&store);
        assert_eq!(
            cache.get::<String>(&store, "settings", "currency").await,
            None
        );
    }
}
