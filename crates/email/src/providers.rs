//! Production email providers backed by mature client libraries.
//!
//! `lettre` handles SMTP transport, TLS and MIME construction. The official
//! `resend-rs` SDK handles the Resend HTTP API, including its rate limiter and
//! idempotency headers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use lettre::message::header::ContentType;
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use resend_rs::Resend;
use resend_rs::types::CreateEmailBaseOptions;

use crate::message::{EmailMessage, EmailReceipt, receipt};
use crate::provider::{EmailError, EmailProvider};

/// SMTP transport security mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmtpSecurity {
    /// Implicit TLS, usually port 465.
    ImplicitTls,
    /// STARTTLS upgrade, usually port 587.
    StartTls,
    /// Plaintext SMTP. Only appropriate for a trusted local relay.
    Plain,
}

/// SMTP provider configuration.
#[derive(Clone, Debug)]
pub struct SmtpConfig {
    /// SMTP host.
    pub host: String,
    /// SMTP port.
    pub port: u16,
    /// Optional SMTP username.
    pub username: Option<String>,
    /// Optional SMTP password.
    pub password: Option<String>,
    /// Default envelope sender.
    pub from: String,
    /// Transport security mode.
    pub security: SmtpSecurity,
    /// Per-send timeout.
    pub timeout: Duration,
}

impl SmtpConfig {
    /// Creates a STARTTLS configuration for the common submission port.
    #[must_use]
    pub fn new(host: impl Into<String>, port: u16, from: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            username: None,
            password: None,
            from: from.into(),
            security: SmtpSecurity::StartTls,
            timeout: Duration::from_secs(20),
        }
    }

    /// Sets SMTP credentials.
    #[must_use]
    pub fn with_credentials(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.username = Some(username.into());
        self.password = Some(password.into());
        self
    }

    /// Selects the transport security mode.
    #[must_use]
    pub fn with_security(mut self, security: SmtpSecurity) -> Self {
        self.security = security;
        self
    }
}

/// SMTP-backed transactional email provider.
pub struct SmtpEmailProvider {
    id: String,
    from: String,
    transport: AsyncSmtpTransport<Tokio1Executor>,
    counter: AtomicU64,
}

impl std::fmt::Debug for SmtpEmailProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SmtpEmailProvider")
            .field("id", &self.id)
            .field("from", &self.from)
            .finish_non_exhaustive()
    }
}

impl SmtpEmailProvider {
    /// Builds an SMTP provider from configuration.
    pub fn new(config: SmtpConfig) -> Result<Self, EmailError> {
        let builder = match config.security {
            SmtpSecurity::ImplicitTls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host),
            SmtpSecurity::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
            }
            SmtpSecurity::Plain => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
                &config.host,
            )),
        }
        .map_err(|error| EmailError::Unavailable(format!("invalid SMTP host: {error}")))?;
        let builder = builder.port(config.port).timeout(Some(config.timeout));
        let builder = match (config.username, config.password) {
            (Some(username), Some(password)) => {
                builder.credentials(Credentials::new(username, password))
            }
            _ => builder,
        };
        Ok(Self {
            id: "smtp".to_owned(),
            from: config.from,
            transport: builder.build(),
            counter: AtomicU64::new(0),
        })
    }

    /// Uses a custom registry identifier.
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    fn build_message(&self, message: &EmailMessage) -> Result<Message, EmailError> {
        let from: Mailbox = message
            .from
            .as_deref()
            .unwrap_or(&self.from)
            .parse()
            .map_err(|error| EmailError::Validation(format!("invalid from address: {error}")))?;
        let to: Mailbox = message
            .to
            .parse()
            .map_err(|error| EmailError::Validation(format!("invalid to address: {error}")))?;
        let builder = Message::builder()
            .from(from)
            .to(to)
            .subject(message.subject.clone());
        let builder = match (message.text.as_deref(), message.html.as_deref()) {
            (Some(text), Some(html)) => builder.multipart(MultiPart::alternative_plain_html(
                text.to_owned(),
                html.to_owned(),
            )),
            (Some(text), None) => builder.body(text.to_owned()),
            (None, Some(html)) => builder.header(ContentType::TEXT_HTML).body(html.to_owned()),
            (None, None) => {
                return Err(EmailError::Validation(
                    "email must contain a text or html body".to_owned(),
                ));
            }
        };
        builder.map_err(|error| EmailError::Validation(format!("invalid email: {error}")))
    }
}

#[async_trait]
impl EmailProvider for SmtpEmailProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn send(&self, message: EmailMessage) -> Result<EmailReceipt, EmailError> {
        let email = self.build_message(&message)?;
        let response = self
            .transport
            .send(email)
            .await
            .map_err(|error| EmailError::Provider(format!("SMTP delivery failed: {error}")))?;
        let sequence = self.counter.fetch_add(1, Ordering::Relaxed);
        let message_id = format!("smtp_{}_{}", response.code(), sequence);
        Ok(receipt(&self.id, message_id))
    }
}

/// Resend configuration.
#[derive(Clone, Debug)]
pub struct ResendConfig {
    /// Resend API key.
    pub api_key: String,
    /// Verified sender, e.g. `Acme <orders@example.com>`.
    pub from: String,
}

/// Resend-backed transactional email provider.
pub struct ResendEmailProvider {
    id: String,
    from: String,
    client: Resend,
    counter: AtomicU64,
}

impl std::fmt::Debug for ResendEmailProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResendEmailProvider")
            .field("id", &self.id)
            .field("from", &self.from)
            .finish_non_exhaustive()
    }
}

impl ResendEmailProvider {
    /// Creates a Resend provider.
    #[must_use]
    pub fn new(config: ResendConfig) -> Self {
        Self {
            id: "resend".to_owned(),
            from: config.from,
            client: Resend::new(&config.api_key),
            counter: AtomicU64::new(0),
        }
    }

    /// Uses a custom registry identifier.
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }
}

#[async_trait]
impl EmailProvider for ResendEmailProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn send(&self, message: EmailMessage) -> Result<EmailReceipt, EmailError> {
        let from = message.from.as_deref().unwrap_or(&self.from);
        let mut email = CreateEmailBaseOptions::new(from, [message.to.as_str()], &message.subject);
        if let Some(text) = message.text.as_deref() {
            email = email.with_text(text);
        }
        if let Some(html) = message.html.as_deref() {
            email = email.with_html(html);
        }
        let response =
            self.client.emails.send(email).await.map_err(|error| {
                EmailError::Provider(format!("Resend delivery failed: {error}"))
            })?;
        let _sequence = self.counter.fetch_add(1, Ordering::Relaxed);
        Ok(receipt(&self.id, response.id.as_ref().to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::{ResendConfig, ResendEmailProvider, SmtpConfig, SmtpEmailProvider};
    use crate::provider::EmailProvider;

    #[tokio::test]
    async fn smtp_provider_is_constructed_from_configuration() {
        let provider = SmtpEmailProvider::new(
            SmtpConfig::new("smtp.example.com", 587, "orders@example.com")
                .with_credentials("user", "password"),
        )
        .unwrap();
        assert_eq!(provider.id(), "smtp");
    }

    #[test]
    fn resend_provider_uses_the_official_client() {
        let provider = ResendEmailProvider::new(ResendConfig {
            api_key: "re_test".to_owned(),
            from: "orders@example.com".to_owned(),
        });
        assert_eq!(provider.id(), "resend");
    }
}
