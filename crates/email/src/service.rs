//! Application service coordinating validation, the job queue and providers.

use std::sync::Arc;

use fastcart_infrastructure::{JobQueue, NewJob};
use fastcart_kernel::JobId;
use serde_json::Value;

use crate::message::{EmailMessage, EmailReceipt, SEND_EMAIL_JOB_TYPE, SendEmailJob};
use crate::provider::{EmailError, EmailProviderRegistry, LogEmailProvider};

/// Sends transactional email through the default registered provider.
#[derive(Clone, Debug)]
pub struct EmailService {
    registry: EmailProviderRegistry,
}

impl EmailService {
    /// Creates a service over an existing provider registry.
    #[must_use]
    pub fn new(registry: EmailProviderRegistry) -> Self {
        Self { registry }
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

        queue.complete(&claimed[0].id).await.unwrap();
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
}
