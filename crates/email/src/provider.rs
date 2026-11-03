//! Provider-independent email delivery interface and registry.
//!
//! A provider owns all external communication. Registries let the platform swap
//! providers (log, memory, HTTP API, SMTP) without changing call sites.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use fastcart_kernel::{AppError, now_timestamp};

use crate::message::{EmailMessage, EmailReceipt, receipt};

/// Errors raised while validating or delivering email.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EmailError {
    /// The message or job payload is structurally invalid.
    #[error("email validation error: {0}")]
    Validation(String),
    /// The provider rejected the message.
    #[error("email provider error: {0}")]
    Provider(String),
    /// No provider was available to accept the message.
    #[error("email provider unavailable: {0}")]
    Unavailable(String),
}

impl From<EmailError> for AppError {
    fn from(error: EmailError) -> Self {
        match error {
            EmailError::Validation(message) => Self::Validation(message),
            EmailError::Provider(message) | EmailError::Unavailable(message) => {
                Self::ExternalService(message)
            }
        }
    }
}

/// External email delivery contract.
#[async_trait]
pub trait EmailProvider: Send + Sync {
    /// Stable provider identifier used as the registry key.
    fn id(&self) -> &str;

    /// Delivers a message and returns provider acceptance details.
    async fn send(&self, message: EmailMessage) -> Result<EmailReceipt, EmailError>;
}

#[derive(Default)]
struct RegistryState {
    providers: BTreeMap<String, Arc<dyn EmailProvider>>,
    default_id: Option<String>,
}

/// Thread-safe registry of available email providers.
#[derive(Clone, Default)]
pub struct EmailProviderRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl EmailProviderRegistry {
    /// Creates an empty registry with no default provider.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, RegistryState> {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, RegistryState> {
        self.state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Registers a provider.
    ///
    /// The first registered provider becomes the default. Returns a conflict
    /// when the identifier is already present.
    pub fn register(&self, provider: Arc<dyn EmailProvider>) -> Result<(), AppError> {
        let id = provider.id().to_owned();
        if id.trim().is_empty() {
            return Err(AppError::validation("email provider id must not be empty"));
        }
        let mut state = self.write();
        if state.providers.contains_key(&id) {
            return Err(AppError::conflict(format!(
                "email provider already registered: {id}"
            )));
        }
        state.providers.insert(id.clone(), provider);
        if state.default_id.is_none() {
            state.default_id = Some(id);
        }
        Ok(())
    }

    /// Sets the provider used when callers do not choose one explicitly.
    pub fn set_default(&self, id: &str) -> Result<(), AppError> {
        let mut state = self.write();
        if !state.providers.contains_key(id) {
            return Err(AppError::not_found("email provider", id));
        }
        state.default_id = Some(id.to_owned());
        Ok(())
    }

    /// Looks up a provider by identifier.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<dyn EmailProvider>> {
        self.read().providers.get(id).cloned()
    }

    /// Returns registered provider identifiers in sorted order.
    #[must_use]
    pub fn list_ids(&self) -> Vec<String> {
        self.read().providers.keys().cloned().collect()
    }

    /// Returns the configured default provider identifier.
    #[must_use]
    pub fn default_id(&self) -> Option<String> {
        self.read().default_id.clone()
    }

    /// Returns the default provider, if one is configured.
    #[must_use]
    pub fn default_provider(&self) -> Option<Arc<dyn EmailProvider>> {
        let state = self.read();
        state
            .default_id
            .as_ref()
            .and_then(|id| state.providers.get(id).cloned())
    }
}

impl std::fmt::Debug for EmailProviderRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmailProviderRegistry")
            .field("providers", &self.list_ids())
            .field("default_id", &self.default_id())
            .finish()
    }
}

/// Development provider that logs acceptance instead of performing I/O.
#[derive(Debug)]
pub struct LogEmailProvider {
    id: String,
    counter: AtomicU64,
}

impl LogEmailProvider {
    /// Creates the default `log` provider.
    #[must_use]
    pub fn new() -> Self {
        Self::with_id("log")
    }

    /// Creates a logging provider with a custom registry identifier.
    #[must_use]
    pub fn with_id(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            counter: AtomicU64::new(0),
        }
    }

    fn next_message_id(&self) -> String {
        let sequence = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{}_{}_{}", self.id, now_timestamp(), sequence)
    }
}

impl Default for LogEmailProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EmailProvider for LogEmailProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn send(&self, message: EmailMessage) -> Result<EmailReceipt, EmailError> {
        // Bodies may contain personal data, so only routing fields are logged.
        let recipient_domain = message
            .to
            .rsplit_once('@')
            .map_or("unknown", |(_, domain)| domain);
        tracing::info!(
            provider = %self.id,
            recipient_domain = %recipient_domain,
            subject = %message.subject,
            "accepted email for delivery"
        );
        Ok(receipt(&self.id, self.next_message_id()))
    }
}

/// In-memory provider that records messages for assertions in tests.
#[derive(Clone, Debug)]
pub struct MemoryEmailProvider {
    id: String,
    messages: Arc<Mutex<Vec<EmailMessage>>>,
    counter: Arc<AtomicU64>,
}

impl MemoryEmailProvider {
    /// Creates a memory provider with the given registry identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            messages: Arc::new(Mutex::new(Vec::new())),
            counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Returns a snapshot of delivered messages.
    #[must_use]
    pub fn messages(&self) -> Vec<EmailMessage> {
        self.messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Returns the number of delivered messages.
    #[must_use]
    pub fn sent_count(&self) -> usize {
        self.messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Removes all recorded messages.
    pub fn clear(&self) {
        self.messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn next_message_id(&self) -> String {
        let sequence = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{}_{}_{}", self.id, now_timestamp(), sequence)
    }
}

impl Default for MemoryEmailProvider {
    fn default() -> Self {
        Self::new("memory")
    }
}

#[async_trait]
impl EmailProvider for MemoryEmailProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn send(&self, message: EmailMessage) -> Result<EmailReceipt, EmailError> {
        self.messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(message);
        Ok(receipt(&self.id, self.next_message_id()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use fastcart_kernel::AppError;

    use super::{
        EmailError, EmailProvider, EmailProviderRegistry, LogEmailProvider, MemoryEmailProvider,
    };
    use crate::message::{EmailMessage, EmailReceipt};

    struct FailingProvider;

    #[async_trait]
    impl EmailProvider for FailingProvider {
        fn id(&self) -> &str {
            "failing"
        }

        async fn send(&self, _message: EmailMessage) -> Result<EmailReceipt, EmailError> {
            Err(EmailError::Provider("boom".to_owned()))
        }
    }

    fn message() -> EmailMessage {
        EmailMessage::new("customer@example.com", "Subject").with_text("body")
    }

    #[test]
    fn registry_rejects_duplicate_ids() {
        let registry = EmailProviderRegistry::new();
        let provider: Arc<dyn EmailProvider> = Arc::new(MemoryEmailProvider::new("memory"));
        registry.register(provider.clone()).unwrap();
        let error = registry.register(provider).unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
        assert_eq!(registry.list_ids(), vec!["memory".to_owned()]);
    }

    #[test]
    fn registry_selects_and_changes_the_default() {
        let registry = EmailProviderRegistry::new();
        assert!(registry.default_provider().is_none());
        assert!(registry.default_id().is_none());

        registry
            .register(Arc::new(MemoryEmailProvider::new("zeta")))
            .unwrap();
        registry
            .register(Arc::new(MemoryEmailProvider::new("alpha")))
            .unwrap();

        assert_eq!(registry.default_id().as_deref(), Some("zeta"));
        assert_eq!(
            registry.list_ids(),
            vec!["alpha".to_owned(), "zeta".to_owned()]
        );

        registry.set_default("alpha").unwrap();
        assert_eq!(registry.default_id().as_deref(), Some("alpha"));

        let error = registry.set_default("missing").unwrap_err();
        assert!(matches!(error, AppError::NotFound(_)));
    }

    #[test]
    fn registry_gets_providers_and_reports_missing_ids() {
        let registry = EmailProviderRegistry::new();
        registry
            .register(Arc::new(MemoryEmailProvider::new("memory")))
            .unwrap();

        let provider = registry.get("memory").unwrap();
        assert_eq!(provider.id(), "memory");
        assert!(registry.get("missing").is_none());
        assert!(format!("{registry:?}").contains("memory"));
    }

    #[test]
    fn registry_rejects_empty_ids() {
        struct EmptyId;

        #[async_trait]
        impl EmailProvider for EmptyId {
            fn id(&self) -> &str {
                ""
            }

            async fn send(&self, _message: EmailMessage) -> Result<EmailReceipt, EmailError> {
                unreachable!("this provider is never used")
            }
        }

        let registry = EmailProviderRegistry::new();
        let error = registry.register(Arc::new(EmptyId)).unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn memory_provider_records_messages() {
        let provider = MemoryEmailProvider::new("memory");
        let message = message();

        let sent = provider.send(message.clone()).await.unwrap();
        assert_eq!(sent.provider, "memory");
        assert!(!sent.message_id.is_empty());
        assert_eq!(provider.sent_count(), 1);
        assert_eq!(provider.messages(), vec![message.clone()]);

        let clone = provider.clone();
        clone.send(message.clone()).await.unwrap();
        assert_eq!(provider.messages(), vec![message.clone(), message.clone()]);

        provider.clear();
        assert_eq!(provider.sent_count(), 0);
    }

    #[tokio::test]
    async fn log_provider_returns_a_receipt_without_external_io() {
        let provider = LogEmailProvider::new();
        let first = provider.send(message()).await.unwrap();
        let second = provider.send(message()).await.unwrap();

        assert_eq!(provider.id(), "log");
        assert_eq!(first.provider, "log");
        assert_ne!(first.message_id, second.message_id);
    }

    #[tokio::test]
    async fn provider_failures_propagate() {
        let registry = EmailProviderRegistry::new();
        registry.register(Arc::new(FailingProvider)).unwrap();
        let provider = registry.default_provider().unwrap();
        let error = provider.send(message()).await.unwrap_err();
        assert_eq!(error, EmailError::Provider("boom".to_owned()));
    }
}
