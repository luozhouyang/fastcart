//! Email message value objects and the `send_email` job payload.
//!
//! Messages are validated before they reach a provider or the persistent queue.
//! Provider identifiers and delivery details live in [`EmailReceipt`] so callers
//! can correlate a send without depending on a specific provider.

use fastcart_kernel::{Metadata, Timestamp, now_timestamp, validate_email, validate_metadata};
use serde::{Deserialize, Serialize};

use crate::provider::EmailError;

/// Job type consumed by the maintenance worker for outbound email.
pub const SEND_EMAIL_JOB_TYPE: &str = "send_email";

/// Maximum accepted subject length in characters.
pub const MAX_SUBJECT_CHARS: usize = 200;

/// Maximum combined text and HTML body size in bytes (256 KiB).
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// A provider-independent transactional email.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmailMessage {
    /// Optional sender address; providers fall back to their configured sender.
    #[serde(default)]
    pub from: Option<String>,
    /// Recipient address.
    pub to: String,
    /// Subject line.
    pub subject: String,
    /// Plain-text body.
    #[serde(default)]
    pub text: Option<String>,
    /// HTML body.
    #[serde(default)]
    pub html: Option<String>,
    /// Extension metadata carried through the queue and provider calls.
    #[serde(default)]
    pub metadata: Metadata,
}

impl EmailMessage {
    /// Creates a message with no bodies and empty metadata.
    ///
    /// Use the `with_*` builders to add a body, sender or metadata, then call
    /// [`Self::validate`] before sending or enqueuing.
    #[must_use]
    pub fn new(to: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            from: None,
            to: to.into(),
            subject: subject.into(),
            text: None,
            html: None,
            metadata: Metadata::empty(),
        }
    }

    /// Sets the sender address.
    #[must_use]
    pub fn with_from(mut self, from: impl Into<String>) -> Self {
        self.from = Some(from.into());
        self
    }

    /// Sets the plain-text body.
    #[must_use]
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Sets the HTML body.
    #[must_use]
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }

    /// Replaces the extension metadata.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Validates addresses, subject, body presence and size, and metadata.
    pub fn validate(&self) -> Result<(), EmailError> {
        validate_email(&self.to, "to").map_err(validation_error)?;
        if let Some(from) = self.from.as_deref() {
            validate_email(from, "from").map_err(validation_error)?;
        }

        let subject_length = self.subject.chars().count();
        if self.subject.trim().is_empty() {
            return Err(EmailError::Validation(
                "subject must not be empty".to_owned(),
            ));
        }
        if subject_length > MAX_SUBJECT_CHARS {
            return Err(EmailError::Validation(format!(
                "subject must not exceed {MAX_SUBJECT_CHARS} characters"
            )));
        }

        let text_length = self.text.as_ref().map_or(0, String::len);
        let html_length = self.html.as_ref().map_or(0, String::len);
        if text_length.saturating_add(html_length) > MAX_BODY_BYTES {
            return Err(EmailError::Validation(format!(
                "combined text and html body must not exceed {MAX_BODY_BYTES} bytes"
            )));
        }

        let has_text = self
            .text
            .as_deref()
            .is_some_and(|body| !body.trim().is_empty());
        let has_html = self
            .html
            .as_deref()
            .is_some_and(|body| !body.trim().is_empty());
        if !has_text && !has_html {
            return Err(EmailError::Validation(
                "email must include a non-empty text or html body".to_owned(),
            ));
        }

        validate_metadata(&self.metadata).map_err(validation_error)?;
        Ok(())
    }
}

fn validation_error(error: fastcart_kernel::AppError) -> EmailError {
    EmailError::Validation(error.to_string())
}

/// Provider acceptance details for a sent message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailReceipt {
    /// Stable provider identifier that accepted the message.
    pub provider: String,
    /// Provider-assigned message identifier.
    pub message_id: String,
    /// Unix timestamp when the provider accepted the message.
    pub accepted_at: Timestamp,
}

/// Serializable payload persisted with a `send_email` job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SendEmailJob {
    /// Recipient address.
    pub to: String,
    /// Optional sender address.
    #[serde(default)]
    pub from: Option<String>,
    /// Subject line.
    pub subject: String,
    /// Plain-text body.
    #[serde(default)]
    pub text: Option<String>,
    /// HTML body.
    #[serde(default)]
    pub html: Option<String>,
    /// Extension metadata.
    #[serde(default)]
    pub metadata: Metadata,
}

impl SendEmailJob {
    /// Job type persisted in the queue.
    pub const JOB_TYPE: &'static str = SEND_EMAIL_JOB_TYPE;

    /// Builds a payload from a message.
    #[must_use]
    pub fn from_message(message: &EmailMessage) -> Self {
        Self {
            to: message.to.clone(),
            from: message.from.clone(),
            subject: message.subject.clone(),
            text: message.text.clone(),
            html: message.html.clone(),
            metadata: message.metadata.clone(),
        }
    }

    /// Rebuilds the message to hand to a provider.
    #[must_use]
    pub fn into_message(self) -> EmailMessage {
        EmailMessage {
            from: self.from,
            to: self.to,
            subject: self.subject,
            text: self.text,
            html: self.html,
            metadata: self.metadata,
        }
    }
}

/// Creates a receipt timestamped with the current wall clock.
#[must_use]
pub(crate) fn receipt(provider: &str, message_id: String) -> EmailReceipt {
    EmailReceipt {
        provider: provider.to_owned(),
        message_id,
        accepted_at: now_timestamp(),
    }
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::{Metadata, validate_metadata};
    use serde_json::json;

    use super::{
        EmailMessage, MAX_BODY_BYTES, MAX_SUBJECT_CHARS, SEND_EMAIL_JOB_TYPE, SendEmailJob,
    };
    use crate::provider::EmailError;

    fn valid_message() -> EmailMessage {
        EmailMessage::new("customer@example.com", "Order confirmed")
            .with_from("shop@example.com")
            .with_text("Thanks for your order.")
    }

    #[test]
    fn accepts_a_valid_message() {
        assert!(valid_message().validate().is_ok());
    }

    #[test]
    fn accepts_an_html_only_message() {
        let message =
            EmailMessage::new("customer@example.com", "Order confirmed").with_html("<p>Thanks</p>");
        assert!(message.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_recipient_and_sender() {
        let invalid_to = EmailMessage::new("not-an-email", "Subject").with_text("body");
        assert!(matches!(
            invalid_to.validate(),
            Err(EmailError::Validation(_))
        ));

        let invalid_from = valid_message().with_from("bad-sender");
        assert!(matches!(
            invalid_from.validate(),
            Err(EmailError::Validation(_))
        ));
    }

    #[test]
    fn rejects_empty_and_oversized_subjects() {
        let empty = EmailMessage::new("customer@example.com", "   ").with_text("body");
        assert!(matches!(empty.validate(), Err(EmailError::Validation(_))));

        let oversized =
            EmailMessage::new("customer@example.com", "x".repeat(MAX_SUBJECT_CHARS + 1))
                .with_text("body");
        assert!(matches!(
            oversized.validate(),
            Err(EmailError::Validation(_))
        ));
    }

    #[test]
    fn requires_a_non_empty_body() {
        let missing = EmailMessage::new("customer@example.com", "Subject");
        assert!(matches!(missing.validate(), Err(EmailError::Validation(_))));

        let empty = EmailMessage::new("customer@example.com", "Subject").with_text("   ");
        assert!(matches!(empty.validate(), Err(EmailError::Validation(_))));
    }

    #[test]
    fn rejects_oversized_bodies() {
        let oversized = EmailMessage::new("customer@example.com", "Subject")
            .with_text("x".repeat(MAX_BODY_BYTES + 1));
        assert!(matches!(
            oversized.validate(),
            Err(EmailError::Validation(_))
        ));
    }

    #[test]
    fn rejects_oversized_metadata() {
        let mut metadata = Metadata::empty();
        metadata.insert(
            "blob",
            json!("x".repeat(fastcart_kernel::validation::MAX_METADATA_BYTES + 1)),
        );
        assert!(validate_metadata(&metadata).is_err());

        let message = EmailMessage::new("customer@example.com", "Subject")
            .with_text("body")
            .with_metadata(metadata);
        assert!(matches!(message.validate(), Err(EmailError::Validation(_))));
    }

    #[test]
    fn job_payload_round_trips_through_json_and_messages() {
        let message = valid_message();
        let job = SendEmailJob::from_message(&message);
        assert_eq!(job.to, message.to);
        assert_eq!(job.into_message(), message);

        let value = serde_json::to_value(SendEmailJob::from_message(&message)).unwrap();
        let decoded: SendEmailJob = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.into_message(), message);
    }

    #[test]
    fn job_payload_tolerates_missing_optional_fields() {
        let job: SendEmailJob = serde_json::from_value(json!({
            "to": "customer@example.com",
            "subject": "Subject"
        }))
        .unwrap();
        assert!(job.from.is_none());
        assert!(job.text.is_none());
        assert!(job.html.is_none());
        assert!(job.metadata.is_empty());
    }

    #[test]
    fn job_type_constant_is_stable() {
        assert_eq!(SEND_EMAIL_JOB_TYPE, "send_email");
        assert_eq!(SendEmailJob::JOB_TYPE, SEND_EMAIL_JOB_TYPE);
    }
}
