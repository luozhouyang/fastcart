//! Storefront runtime worker pool.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use fastcart_runtime_api::{RuntimeError, RuntimeRequest, RuntimeResponse, StorefrontRuntime};

/// A round-robin pool of storefront runtime workers.
pub struct RuntimePool {
    workers: Vec<Arc<dyn StorefrontRuntime>>,
    next: AtomicUsize,
}

impl RuntimePool {
    /// Creates a pool. A pool must contain at least one worker.
    pub fn new(workers: Vec<Arc<dyn StorefrontRuntime>>) -> Result<Self, RuntimeError> {
        if workers.is_empty() {
            return Err(RuntimeError::EmptyPool);
        }
        Ok(Self {
            workers,
            next: AtomicUsize::new(0),
        })
    }

    /// Returns the number of workers in the pool.
    #[must_use]
    pub fn len(&self) -> usize {
        self.workers.len()
    }

    /// Returns whether the pool has no workers. Pools cannot be empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.workers.is_empty()
    }

    fn next_worker(&self) -> &Arc<dyn StorefrontRuntime> {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        &self.workers[index]
    }
}

#[async_trait::async_trait]
impl StorefrontRuntime for RuntimePool {
    async fn render(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        self.next_worker().render(request).await
    }

    async fn health_check(&self) -> Result<(), RuntimeError> {
        for worker in &self.workers {
            worker.health_check().await?;
        }
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        for worker in &self.workers {
            worker.shutdown().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use fastcart_kernel::{CurrencyCode, StoreContext, StoreId};
    use fastcart_runtime_api::{
        RuntimeBody, RuntimeError, RuntimeRequest, RuntimeResponse, StorefrontRuntime,
    };

    use super::RuntimePool;

    #[derive(Debug)]
    struct FixedRuntime;

    #[async_trait::async_trait]
    impl StorefrontRuntime for FixedRuntime {
        async fn render(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
            Ok(RuntimeResponse {
                status: 200,
                headers: Vec::new(),
                body: RuntimeBody::Full(Bytes::from_static(b"ok")),
            })
        }

        async fn health_check(&self) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), RuntimeError> {
            Ok(())
        }
    }

    fn request() -> RuntimeRequest {
        RuntimeRequest {
            request_id: "req_test".to_owned(),
            store: StoreContext {
                store_id: StoreId::from_string("store_test"),
                domain: "test.local".to_owned(),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").unwrap(),
                timezone: "UTC".to_owned(),
            },
            method: "GET".to_owned(),
            url: "/".to_owned(),
            headers: Vec::new(),
            body: None,
            timeout_ms: 1_000,
        }
    }

    #[tokio::test]
    async fn empty_pool_is_rejected() {
        assert!(matches!(
            RuntimePool::new(Vec::new()),
            Err(RuntimeError::EmptyPool)
        ));
    }

    #[tokio::test]
    async fn pool_delegates_to_a_worker() {
        let pool = RuntimePool::new(vec![std::sync::Arc::new(FixedRuntime)]).unwrap();

        assert_eq!(pool.len(), 1);
        assert!(!pool.is_empty());
        assert_eq!(pool.render(request()).await.unwrap().status, 200);
        pool.health_check().await.unwrap();
        pool.shutdown().await.unwrap();
    }
}
