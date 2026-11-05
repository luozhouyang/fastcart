//! Storefront runtime worker pool.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use fastcart_runtime_api::{
    RuntimeBody, RuntimeError, RuntimeRequest, RuntimeResponse, StorefrontRuntime,
};
use futures::StreamExt;
use tokio::sync::Semaphore;

/// A round-robin pool of storefront runtime workers with bounded concurrency
/// and per-request timeouts.
pub struct RuntimePool {
    workers: Vec<Arc<dyn StorefrontRuntime>>,
    next: AtomicUsize,
    inflight: Arc<Semaphore>,
    capacity: usize,
}

impl RuntimePool {
    /// Creates a pool of workers.
    ///
    /// The default in-flight limit is 64 requests per worker.
    pub fn new(workers: Vec<Arc<dyn StorefrontRuntime>>) -> Result<Self, RuntimeError> {
        let capacity = workers.len().saturating_mul(64).max(1);
        Self::with_capacity(workers, capacity)
    }

    /// Creates a pool with an explicit in-flight request limit.
    ///
    /// Requests arriving while the pool is saturated fail fast with
    /// [`RuntimeError::WorkerUnavailable`] instead of growing unbounded queues.
    pub fn with_capacity(
        workers: Vec<Arc<dyn StorefrontRuntime>>,
        capacity: usize,
    ) -> Result<Self, RuntimeError> {
        if workers.is_empty() {
            return Err(RuntimeError::EmptyPool);
        }
        let capacity = capacity.max(1);
        Ok(Self {
            workers,
            next: AtomicUsize::new(0),
            inflight: Arc::new(Semaphore::new(capacity)),
            capacity,
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

    /// Maximum concurrent render requests.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Requests currently occupying a slot.
    #[must_use]
    pub fn inflight(&self) -> usize {
        self.capacity
            .saturating_sub(self.inflight.available_permits())
    }

    fn next_worker(&self) -> &Arc<dyn StorefrontRuntime> {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        &self.workers[index]
    }
}

#[async_trait::async_trait]
impl StorefrontRuntime for RuntimePool {
    async fn render(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        let timeout_ms = request.timeout_ms.max(1);
        let permit = self.inflight.clone().try_acquire_owned().map_err(|_| {
            RuntimeError::WorkerUnavailable("runtime pool is at capacity".to_owned())
        })?;
        let result = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            self.next_worker().render(request),
        )
        .await;
        let mut response = match result {
            Ok(response) => response?,
            Err(_) => return Err(RuntimeError::Timeout(timeout_ms)),
        };
        let RuntimeBody::Stream(stream) = response.body else {
            return Ok(response);
        };
        // A streamed body is still using a worker slot until it ends. Keep the
        // permit inside the stream so dropped or finished bodies release it.
        let stream =
            futures::stream::try_unfold((stream, permit), |(mut stream, permit)| async move {
                match stream.next().await {
                    Some(item) => Ok::<_, RuntimeError>(Some((item?, (stream, permit)))),
                    None => Ok(None),
                }
            });
        response.body = RuntimeBody::Stream(Box::pin(stream));
        Ok(response)
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
    use std::sync::Arc;
    use std::time::Duration;

    use bytes::Bytes;
    use fastcart_kernel::{CurrencyCode, StoreContext, StoreId};
    use fastcart_runtime_api::{
        RuntimeBody, RuntimeError, RuntimeMode, RuntimeRequest, RuntimeResponse, StorefrontRuntime,
    };
    use futures::StreamExt;
    use tokio::sync::Notify;

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

    struct SlowRuntime;

    #[async_trait::async_trait]
    impl StorefrontRuntime for SlowRuntime {
        async fn render(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(RuntimeResponse {
                status: 200,
                headers: Vec::new(),
                body: RuntimeBody::Full(Bytes::new()),
            })
        }

        async fn health_check(&self) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), RuntimeError> {
            Ok(())
        }
    }

    struct StreamingRuntime {
        release: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl StorefrontRuntime for StreamingRuntime {
        async fn render(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
            let release = self.release.clone();
            let first = futures::stream::once(async {
                Ok::<_, RuntimeError>(Bytes::from_static(b"first"))
            });
            let second = futures::stream::once(async move {
                release.notified().await;
                Ok::<_, RuntimeError>(Bytes::from_static(b"second"))
            });
            Ok(RuntimeResponse {
                status: 200,
                headers: Vec::new(),
                body: RuntimeBody::stream(Box::pin(first.chain(second))),
            })
        }

        async fn health_check(&self) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), RuntimeError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct BlockingRuntime {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl StorefrontRuntime for BlockingRuntime {
        async fn render(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
            self.started.notify_waiters();
            self.release.notified().await;
            Ok(RuntimeResponse {
                status: 200,
                headers: Vec::new(),
                body: RuntimeBody::Full(Bytes::new()),
            })
        }

        async fn health_check(&self) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), RuntimeError> {
            Ok(())
        }
    }

    fn request(timeout_ms: u64) -> RuntimeRequest {
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
            theme: None,
            page: None,
            data: None,
            capabilities: std::collections::BTreeMap::new(),
            mode: RuntimeMode::Production,
            timeout_ms,
            streaming: false,
            stream_timeout_ms: timeout_ms,
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
        let pool = RuntimePool::new(vec![Arc::new(FixedRuntime)]).unwrap();

        assert_eq!(pool.len(), 1);
        assert!(!pool.is_empty());
        assert_eq!(pool.capacity(), 64);
        assert_eq!(pool.render(request(1_000)).await.unwrap().status, 200);
        pool.health_check().await.unwrap();
        pool.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn renders_time_out() {
        let pool = RuntimePool::new(vec![Arc::new(SlowRuntime)]).unwrap();
        let error = pool.render(request(10)).await.unwrap_err();
        assert!(matches!(error, RuntimeError::Timeout(10)));
    }

    #[tokio::test]
    async fn streaming_bodies_hold_a_slot_until_they_end() {
        let release = Arc::new(Notify::new());
        let runtime = StreamingRuntime {
            release: release.clone(),
        };
        let pool = RuntimePool::with_capacity(vec![Arc::new(runtime)], 2).unwrap();

        let response = pool.render(request(1_000)).await.unwrap();
        assert_eq!(pool.inflight(), 1);
        let RuntimeBody::Stream(mut stream) = response.body else {
            panic!("expected a streaming body");
        };
        assert_eq!(&stream.next().await.unwrap().unwrap()[..], b"first");
        assert_eq!(pool.inflight(), 1);

        release.notify_one();
        assert_eq!(&stream.next().await.unwrap().unwrap()[..], b"second");
        assert!(stream.next().await.is_none());
        assert_eq!(pool.inflight(), 0);
    }

    #[tokio::test]
    async fn saturated_pools_reject_new_requests() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let runtime = BlockingRuntime {
            started: started.clone(),
            release: release.clone(),
        };
        let pool = Arc::new(RuntimePool::with_capacity(vec![Arc::new(runtime)], 1).unwrap());

        let first = {
            let pool = pool.clone();
            tokio::spawn(async move { pool.render(request(5_000)).await })
        };
        started.notified().await;

        assert_eq!(pool.inflight(), 1);
        let error = pool.render(request(1_000)).await.unwrap_err();
        assert!(matches!(error, RuntimeError::WorkerUnavailable(_)));

        release.notify_waiters();
        first.await.unwrap().unwrap();
        assert_eq!(pool.inflight(), 0);
    }
}
