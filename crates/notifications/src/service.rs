//! Notification dispatch: render, deduplicate and enqueue email jobs.

use fastcart_email::{EmailError, EmailMessage, EmailService};
use fastcart_infrastructure::{JobQueue, StoreDatabase};
use fastcart_kernel::{
    DomainEventEnvelope, EventId, JobId, Metadata, OrderId, normalize_email, now_timestamp,
    validate_email,
};
use fastcart_module_order::OrderService;
use serde_json::json;

use crate::catalog::{NotificationRule, TemplateRegistry, rule_for};
use crate::context::TemplateContext;

/// Priority used for transactional email jobs.
const EMAIL_PRIORITY: i64 = 10;

/// Errors raised while dispatching one notification.
///
/// Recipient and body values are never included so logs cannot leak addresses
/// or message contents.
#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    /// Persistence failure while reading or writing delivery state.
    #[error("notification delivery state error: {0}")]
    Database(#[from] sqlx::Error),
    /// Template lookup, syntax or rendering failure.
    #[error("notification template error: {0}")]
    Template(#[from] minijinja::Error),
    /// The email service rejected the message or the queue was unavailable.
    #[error("notification email error: {0}")]
    Email(#[from] EmailError),
    /// Loading the referenced order failed for a reason other than absence.
    #[error("notification order lookup failed: {0}")]
    Order(String),
}

/// Result of dispatching one outbox event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationOutcome {
    /// A `send_email` job was enqueued.
    Sent {
        /// Template family used for the message.
        template: &'static str,
        /// Persisted email job identifier.
        job_id: JobId,
    },
    /// The event was already delivered for this template.
    Duplicate {
        /// Template family that was previously delivered.
        template: &'static str,
    },
    /// No rule is configured for the event type.
    Unsupported,
    /// No valid recipient could be resolved from the payload or order.
    MissingRecipient {
        /// Template family that would have been used.
        template: &'static str,
    },
}

impl NotificationOutcome {
    /// Returns true when a new email job was enqueued.
    #[must_use]
    pub fn is_sent(&self) -> bool {
        matches!(self, Self::Sent { .. })
    }
}

/// Transactional notification rendering, deduplication and enqueueing.
#[derive(Clone, Debug)]
pub struct NotificationService {
    templates: TemplateRegistry,
}

impl NotificationService {
    /// Creates a service with the bundled templates and default event rules.
    #[must_use]
    pub fn new() -> Self {
        Self::with_templates(TemplateRegistry::embedded())
    }

    /// Creates a service with a custom template registry.
    ///
    /// The default event rules are still used; register any replacement
    /// template families under the same base names.
    #[must_use]
    pub fn with_templates(templates: TemplateRegistry) -> Self {
        Self { templates }
    }

    /// Dispatches one outbox event.
    ///
    /// Unsupported events and events without a usable recipient are skipped
    /// successfully. Only infrastructure failures return an error, which lets
    /// the maintenance sweep retain the outbox row for a later retry.
    pub async fn dispatch(
        &self,
        db: &StoreDatabase,
        email: &EmailService,
        queue: &JobQueue,
        event: &DomainEventEnvelope,
    ) -> Result<NotificationOutcome, NotificationError> {
        let Some(rule) = rule_for(&event.event_type) else {
            return Ok(NotificationOutcome::Unsupported);
        };

        let order = self.load_order(db, event).await?;
        let Some(recipient) = resolve_recipient(event, order.as_ref()) else {
            return Ok(NotificationOutcome::MissingRecipient {
                template: rule.template,
            });
        };

        let context = TemplateContext::from_event(event, order.as_ref());
        let rendered = self.templates.render(rule.template, &context)?;

        match claim_delivery(db, event, rule).await? {
            DeliveryClaim::Sent => {
                return Ok(NotificationOutcome::Duplicate {
                    template: rule.template,
                });
            }
            DeliveryClaim::Pending | DeliveryClaim::New => {}
        }

        let message = EmailMessage::new(recipient, rendered.subject)
            .with_text(rendered.text)
            .with_html(rendered.html)
            .with_metadata(metadata(event, rule));
        let job_id = match email.enqueue(queue, message, Some(EMAIL_PRIORITY)).await {
            Ok(job_id) => job_id,
            Err(error) => {
                // The job was not queued, so release the claim. A later
                // outbox retry can then deliver the notification.
                release_claim(db, event, rule).await?;
                return Err(error.into());
            }
        };

        mark_delivery_sent(db, event, rule, job_id.as_str()).await?;
        Ok(NotificationOutcome::Sent {
            template: rule.template,
            job_id,
        })
    }

    async fn load_order(
        &self,
        db: &StoreDatabase,
        event: &DomainEventEnvelope,
    ) -> Result<Option<fastcart_module_order::Order>, NotificationError> {
        let Some(order_id) = event.data.get("order_id").and_then(|value| value.as_str()) else {
            return Ok(None);
        };
        match OrderService::new(db.clone())
            .get(&OrderId::from_string(order_id))
            .await
        {
            Ok(order) => Ok(Some(order)),
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(NotificationError::Order(error.to_string())),
        }
    }
}

impl Default for NotificationService {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_recipient(
    event: &DomainEventEnvelope,
    order: Option<&fastcart_module_order::Order>,
) -> Option<String> {
    let candidate = event
        .data
        .get("email")
        .and_then(|value| value.as_str())
        .or_else(|| {
            event
                .data
                .get("customer_email")
                .and_then(|value| value.as_str())
        })
        .map(str::to_owned)
        .or_else(|| order.map(|order| order.email.clone()))?;
    let normalized = normalize_email(&candidate);
    validate_email(&normalized, "recipient").ok()?;
    Some(normalized)
}

fn metadata(event: &DomainEventEnvelope, rule: &NotificationRule) -> Metadata {
    let mut metadata = Metadata::empty();
    metadata.insert("event_id", json!(event.id.as_str()));
    metadata.insert("event_type", json!(event.event_type));
    metadata.insert("template", json!(rule.template));
    if let Some(order_id) = event.data.get("order_id").and_then(|value| value.as_str()) {
        metadata.insert("order_id", json!(order_id));
    }
    metadata
}

enum DeliveryClaim {
    New,
    Pending,
    Sent,
}

async fn claim_delivery(
    db: &StoreDatabase,
    event: &DomainEventEnvelope,
    rule: &NotificationRule,
) -> Result<DeliveryClaim, sqlx::Error> {
    let now = now_timestamp();
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO notification_deliveries
         (id, event_id, event_type, template, status, attempts, created_at)
         VALUES (?, ?, ?, ?, 'pending', 0, ?)",
    )
    .bind(EventId::new().as_str())
    .bind(event.id.as_str())
    .bind(&event.event_type)
    .bind(rule.template)
    .bind(now)
    .execute(db.pool())
    .await?;
    if inserted.rows_affected() > 0 {
        return Ok(DeliveryClaim::New);
    }

    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM notification_deliveries WHERE event_id = ? AND template = ?",
    )
    .bind(event.id.as_str())
    .bind(rule.template)
    .fetch_optional(db.pool())
    .await?;
    if status.as_deref() == Some("sent") {
        return Ok(DeliveryClaim::Sent);
    }

    // A previous attempt claimed the row but never marked it sent (for example,
    // the process crashed before enqueueing). Allow delivery to be retried.
    sqlx::query(
        "UPDATE notification_deliveries
         SET attempts = attempts + 1, event_type = ?
         WHERE event_id = ? AND template = ? AND status = 'pending'",
    )
    .bind(&event.event_type)
    .bind(event.id.as_str())
    .bind(rule.template)
    .execute(db.pool())
    .await?;
    Ok(DeliveryClaim::Pending)
}

async fn mark_delivery_sent(
    db: &StoreDatabase,
    event: &DomainEventEnvelope,
    rule: &NotificationRule,
    job_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE notification_deliveries
         SET status = 'sent', job_id = ?, sent_at = ?, attempts = attempts + 1
         WHERE event_id = ? AND template = ? AND status = 'pending'",
    )
    .bind(job_id)
    .bind(now_timestamp())
    .bind(event.id.as_str())
    .bind(rule.template)
    .execute(db.pool())
    .await?;
    Ok(())
}

async fn release_claim(
    db: &StoreDatabase,
    event: &DomainEventEnvelope,
    rule: &NotificationRule,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM notification_deliveries
         WHERE event_id = ? AND template = ? AND status = 'pending'",
    )
    .bind(event.id.as_str())
    .bind(rule.template)
    .execute(db.pool())
    .await?;
    Ok(())
}
