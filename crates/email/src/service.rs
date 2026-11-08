//! Application service coordinating validation, the job queue and providers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use fastcart_infrastructure::{JobQueue, NewJob};
use fastcart_kernel::{JobId, StoreId};
use serde_json::Value;

use crate::message::{EmailMessage, EmailReceipt, SEND_EMAIL_JOB_TYPE, SendEmailJob};
use crate::provider::{EmailError, EmailProviderRegistry, LogEmailProvider};

/// Resolves the provider registry for one store.
pub type StoreEmailFactory = Arc<dyn Fn(&StoreId) -> EmailProviderRegistry + Send + Sync>;

/// Sends transactional email through the default registered provider.
///
/// A service may carry a per-store factory so a multi-store deployment can use
/// different SMTP/Resend credentials without exposing secrets in the store
/// database. The factory is optional: tests and single-store deployments keep
/// using one registry.
#[derive(Clone)]
pub struct EmailService {
    registry: EmailProviderRegistry,
    store_factory: Option<StoreEmailFactory>,
    store_cache: Arc<Mutex<HashMap<String, EmailProviderRegistry>>>,
}

impl std::fmt::Debug for EmailService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmailService")
            .field("default_provider", &self.default_provider_id())
            .field("has_store_factory", &self.store_factory.is_some())
            .finish()
    }
}

impl EmailService {
    /// Creates a service over an existing provider registry.
    #[must_use]
    pub fn new(registry: EmailProviderRegistry) -> Self {
        Self {
            registry,
            store_factory: None,
            store_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Attaches a lazy per-store registry factory.
    #[must_use]
    pub fn with_store_factory(mut self, factory: StoreEmailFactory) -> Self {
        self.store_factory = Some(factory);
        self
    }

    /// Returns the registry for one store, falling back to the global registry.
    #[must_use]
    pub fn registry_for_store(&self, store_id: &StoreId) -> EmailProviderRegistry {
        let Some(factory) = self.store_factory.as_ref() else {
            return self.registry.clone();
        };
        let key = store_id.to_string();
        if let Some(registry) = self
            .store_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
        {
            return registry.clone();
        }
        let registry = factory(store_id);
        self.store_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, registry.clone());
        registry
    }

    /// Creates a service with the logging provider registered as `log`.
    #[must_use]
    pub fn with_default_registry() -> Self {
        let registry = EmailProviderRegistry::new();
        registry
            .register(Arc::new(LogEmailProvider::new()))
            .expect("the log provider id is not registered in a fresh registry");
        registry
            .set_default("log")
            .expect("the log provider was just registered");
        Self::new(registry)
    }

    /// Returns the provider registry used by this service.
    #[must_use]
    pub fn registry(&self) -> &EmailProviderRegistry {
        &self.registry
    }

    /// Returns the identifier of the provider used for sends.
    #[must_use]
    pub fn default_provider_id(&self) -> Option<String> {
        self.registry.default_id()
    }

    /// Validates and sends a message through the default provider.
    pub async fn send(&self, message: EmailMessage) -> Result<EmailReceipt, EmailError> {
        message.validate()?;
        let provider = self.registry.default_provider().ok_or_else(|| {
            EmailError::Unavailable("no default email provider is configured".to_owned())
        })?;
        provider.send(message).await
    }

    /// Validates and sends a message through the store's default provider.
    pub async fn send_for_store(
        &self,
        store_id: &StoreId,
        message: EmailMessage,
    ) -> Result<EmailReceipt, EmailError> {
        message.validate()?;
        let registry = self.registry_for_store(store_id);
        let provider = registry.default_provider().ok_or_else(|| {
            EmailError::Unavailable("no default email provider is configured".to_owned())
        })?;
        provider.send(message).await
    }

    /// Validates a message and enqueues a `send_email` job.
    ///
    /// Higher priorities are claimed first by the worker. The returned
    /// identifier can be used to poll or cancel the job.
    pub async fn enqueue(
        &self,
        queue: &JobQueue,
        message: EmailMessage,
        priority: Option<i64>,
    ) -> Result<JobId, EmailError> {
        message.validate()?;
        let payload =
            serde_json::to_value(SendEmailJob::from_message(&message)).map_err(|error| {
                EmailError::Validation(format!("failed to serialize email job: {error}"))
            })?;
        let mut job = NewJob::new(SEND_EMAIL_JOB_TYPE, payload);
        if let Some(priority) = priority {
            job = job.with_priority(priority);
        }
        let record = queue.enqueue(job).await.map_err(|error| {
            EmailError::Unavailable(format!("failed to enqueue email job: {error}"))
        })?;
        Ok(JobId::from_string(record.id))
    }

    /// Processes a claimed job payload by sending through the default provider.
    ///
    /// The maintenance worker calls this for every `send_email` job. Invalid
    /// payloads return [`EmailError::Validation`] so the worker can fail the job
    /// without retrying forever.
    pub async fn process_job(&self, payload: &Value) -> Result<EmailReceipt, EmailError> {
        let job: SendEmailJob = serde_json::from_value(payload.clone()).map_err(|error| {
            EmailError::Validation(format!("invalid send_email job payload: {error}"))
        })?;
        self.send(job.into_message()).await
    }

    /// Processes a claimed job using the store's provider.
    pub async fn process_job_for_store(
        &self,
        store_id: &StoreId,
        payload: &Value,
    ) -> Result<EmailReceipt, EmailError> {
        let job: SendEmailJob = serde_json::from_value(payload.clone()).map_err(|error| {
            EmailError::Validation(format!("invalid send_email job payload: {error}"))
        })?;
        self.send_for_store(store_id, job.into_message()).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use fastcart_infrastructure::{JobQueue, StoreDatabase};
    use fastcart_kernel::StoreId;
    use serde_json::json;

    use super::{EmailService, SEND_EMAIL_JOB_TYPE};
    use crate::message::{EmailMessage, EmailReceipt};
    use crate::provider::{EmailError, EmailProvider, EmailProviderRegistry, MemoryEmailProvider};

    struct FailingProvider;

    #[async_trait]
    impl EmailProvider for FailingProvider {
        fn id(&self) -> &str {
            "failing"
        }

        async fn send(&self, _message: EmailMessage) -> Result<EmailReceipt, EmailError> {
            Err(EmailError::Provider("provider exploded".to_owned()))
        }
    }

    async fn queue() -> JobQueue {
        let database = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        JobQueue::new(database.pool().clone())
    }

    fn service_with_memory() -> (EmailService, Arc<MemoryEmailProvider>) {
        let provider = Arc::new(MemoryEmailProvider::new("memory"));
        let registry = EmailProviderRegistry::new();
        registry.register(provider.clone()).unwrap();
        (EmailService::new(registry), provider)
    }

    fn message() -> EmailMessage {
        EmailMessage::new("customer@example.com", "Order confirmed")
            .with_from("shop@example.com")
            .with_text("Thanks for your order.")
    }

    #[tokio::test]
    async fn send_uses_the_default_provider() {
        let (service, provider) = service_with_memory();
        let receipt = service.send(message()).await.unwrap();

        assert_eq!(receipt.provider, "memory");
        assert_eq!(service.default_provider_id().as_deref(), Some("memory"));
        assert_eq!(provider.sent_count(), 1);
        assert_eq!(provider.messages(), vec![message()]);
    }

    #[tokio::test]
    async fn default_registry_uses_the_log_provider() {
        let service = EmailService::with_default_registry();
        let receipt = service.send(message()).await.unwrap();

        assert_eq!(service.default_provider_id().as_deref(), Some("log"));
        assert_eq!(receipt.provider, "log");
        assert!(!receipt.message_id.is_empty());
    }

    #[tokio::test]
    async fn send_rejects_invalid_messages_before_the_provider() {
        let (service, provider) = service_with_memory();
        let invalid = EmailMessage::new("not-an-email", "Subject").with_text("body");

        let error = service.send(invalid).await.unwrap_err();
        assert!(matches!(error, EmailError::Validation(_)));
        assert_eq!(provider.sent_count(), 0);
    }

    #[tokio::test]
    async fn send_reports_a_missing_default_provider() {
        let service = EmailService::new(EmailProviderRegistry::new());
        let error = service.send(message()).await.unwrap_err();
        assert!(matches!(error, EmailError::Unavailable(_)));
    }

    #[tokio::test]
    async fn send_propagates_provider_failures() {
        let registry = EmailProviderRegistry::new();
        registry.register(Arc::new(FailingProvider)).unwrap();
        let service = EmailService::new(registry);

        let error = service.send(message()).await.unwrap_err();
        assert_eq!(error, EmailError::Provider("provider exploded".to_owned()));
    }

    #[tokio::test]
    async fn enqueue_validates_messages() {
        let (service, _provider) = service_with_memory();
        let queue = queue().await;
        let invalid = EmailMessage::new("not-an-email", "Subject").with_text("body");

        let error = service.enqueue(&queue, invalid, None).await.unwrap_err();
        assert!(matches!(error, EmailError::Validation(_)));
        assert!(queue.claim("worker", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn enqueue_and_process_job_round_trip() {
        let (service, provider) = service_with_memory();
        let queue = queue().await;
        let message = message();

        let job_id = service
            .enqueue(&queue, message.clone(), Some(7))
            .await
            .unwrap();
        let claimed = queue.claim("worker-1", 10).await.unwrap();

        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, job_id.as_str());
        assert_eq!(claimed[0].job_type, SEND_EMAIL_JOB_TYPE);
        assert_eq!(claimed[0].priority, 7);

        let receipt = service
            .process_job(&claimed[0].json_payload())
            .await
            .unwrap();
        assert_eq!(receipt.provider, "memory");
        assert_eq!(provider.messages(), vec![message]);

        assert!(queue.complete(&claimed[0].id, "worker-1").await.unwrap());
        assert_eq!(
            queue
                .count_by_status(fastcart_infrastructure::jobs::status::COMPLETED)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn process_job_rejects_invalid_payloads() {
        let (service, _provider) = service_with_memory();

        let error = service
            .process_job(&json!({"to": "customer@example.com"}))
            .await;
        assert!(matches!(error, Err(EmailError::Validation(_))));

        let error = service.process_job(&json!("not-an-object")).await;
        assert!(matches!(error, Err(EmailError::Validation(_))));
    }

    #[tokio::test]
    async fn process_job_propagates_validation_errors() {
        let (service, _provider) = service_with_memory();
        let payload = json!({
            "to": "customer@example.com",
            "subject": "Subject"
        });

        let error = service.process_job(&payload).await.unwrap_err();
        assert!(matches!(error, EmailError::Validation(_)));
    }
    #[test]
    fn store_factory_resolves_and_caches_registries() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use fastcart_kernel::StoreId;

        let calls = Arc::new(AtomicUsize::new(0));
        let default_registry = EmailProviderRegistry::new();
        default_registry
            .register(Arc::new(MemoryEmailProvider::new("default")))
            .unwrap();
        let store_registry = EmailProviderRegistry::new();
        store_registry
            .register(Arc::new(MemoryEmailProvider::new("store_provider")))
            .unwrap();

        let calls_for_factory = calls.clone();
        let default_for_factory = default_registry.clone();
        let store_for_factory = store_registry.clone();
        let service =
            EmailService::new(default_registry).with_store_factory(Arc::new(move |store_id| {
                calls_for_factory.fetch_add(1, Ordering::SeqCst);
                if store_id.as_str() == "store_a" {
                    store_for_factory.clone()
                } else {
                    default_for_factory.clone()
                }
            }));

        let store_a = StoreId::from_string("store_a");
        let store_b = StoreId::from_string("store_b");
        assert_eq!(
            service.registry_for_store(&store_a).default_id().as_deref(),
            Some("store_provider")
        );
        assert_eq!(
            service.registry_for_store(&store_b).default_id().as_deref(),
            Some("default")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // Cached: a second lookup does not consult the factory again.
        assert_eq!(
            service.registry_for_store(&store_a).default_id().as_deref(),
            Some("store_provider")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
