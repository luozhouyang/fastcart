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
/// How long one attempt owns a delivery claim before another may take over.
const CLAIM_LEASE_SECONDS: i64 = 300;

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
            DeliveryClaim::Sent => Ok(NotificationOutcome::Duplicate {
                template: rule.template,
            }),
            // Another attempt owns an unexpired claim for this event. Treat it
            // like a duplicate so this outbox pass cannot enqueue a second
            // email.
            DeliveryClaim::Busy => Ok(NotificationOutcome::Duplicate {
                template: rule.template,
            }),
            DeliveryClaim::Claimed { token } => {
                let message = EmailMessage::new(recipient, rendered.subject)
                    .with_text(rendered.text)
                    .with_html(rendered.html)
                    .with_metadata(metadata(event, rule));
                let job_id = match email.enqueue(queue, message, Some(EMAIL_PRIORITY)).await {
                    Ok(job_id) => job_id,
                    Err(error) => {
                        // The job was not queued, so release our claim. The
                        // token ensures a concurrent attempt's row is not
                        // deleted by mistake.
                        release_claim(db, event, rule, &token).await?;
                        return Err(error.into());
                    }
                };

                mark_delivery_sent(db, event, rule, job_id.as_str(), &token).await?;
                Ok(NotificationOutcome::Sent {
                    template: rule.template,
                    job_id,
                })
            }
        }
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
    /// This attempt now owns the delivery and must enqueue it.
    Claimed { token: String },
    /// Another attempt owns an unexpired claim.
    Busy,
    /// The delivery was already completed.
    Sent,
}

/// Atomically claims `(event_id, template)` for this attempt.
///
/// The insert claims a new row in one statement. For an existing row the
/// conditional update only succeeds when it is still `pending` or when a
/// previous `claimed` row's lease has expired, and it replaces the owner
/// token. A concurrent attempt can therefore never share ownership.
async fn claim_delivery(
    db: &StoreDatabase,
    event: &DomainEventEnvelope,
    rule: &NotificationRule,
) -> Result<DeliveryClaim, sqlx::Error> {
    let now = now_timestamp();
    let token = EventId::new().as_str().to_owned();
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO notification_deliveries
         (id, event_id, event_type, template, status, attempts, created_at,
          claim_token, claimed_at)
         VALUES (?, ?, ?, ?, 'claimed', 0, ?, ?, ?)",
    )
    .bind(EventId::new().as_str())
    .bind(event.id.as_str())
    .bind(&event.event_type)
    .bind(rule.template)
    .bind(now)
    .bind(&token)
    .bind(now)
    .execute(db.pool())
    .await?;
    if inserted.rows_affected() > 0 {
        return Ok(DeliveryClaim::Claimed { token });
    }

    let lease_cutoff = now.saturating_sub(CLAIM_LEASE_SECONDS);
    let claimed = sqlx::query(
        "UPDATE notification_deliveries
         SET status = 'claimed', claim_token = ?, claimed_at = ?,
             attempts = attempts + 1, event_type = ?
         WHERE event_id = ? AND template = ?
           AND (
               status = 'pending'
               OR (status = 'claimed' AND (claimed_at IS NULL OR claimed_at <= ?))
           )",
    )
    .bind(&token)
    .bind(now)
    .bind(&event.event_type)
    .bind(event.id.as_str())
    .bind(rule.template)
    .bind(lease_cutoff)
    .execute(db.pool())
    .await?;
    if claimed.rows_affected() == 1 {
        return Ok(DeliveryClaim::Claimed { token });
    }

    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM notification_deliveries WHERE event_id = ? AND template = ?",
    )
    .bind(event.id.as_str())
    .bind(rule.template)
    .fetch_optional(db.pool())
    .await?;
    match status.as_deref() {
        Some("sent") => Ok(DeliveryClaim::Sent),
        // An unexpired claim, or a row that changed between the update and
        // this lookup, is owned by another attempt.
        Some(_) => Ok(DeliveryClaim::Busy),
        None => Err(sqlx::Error::RowNotFound),
    }
}

async fn mark_delivery_sent(
    db: &StoreDatabase,
    event: &DomainEventEnvelope,
    rule: &NotificationRule,
    job_id: &str,
    token: &str,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        "UPDATE notification_deliveries
         SET status = 'sent', job_id = ?, sent_at = ?, attempts = attempts + 1,
             claim_token = NULL, claimed_at = NULL
         WHERE event_id = ? AND template = ?
           AND status = 'claimed' AND claim_token = ?",
    )
    .bind(job_id)
    .bind(now_timestamp())
    .bind(event.id.as_str())
    .bind(rule.template)
    .bind(token)
    .execute(db.pool())
    .await?;
    if result.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

async fn release_claim(
    db: &StoreDatabase,
    event: &DomainEventEnvelope,
    rule: &NotificationRule,
    token: &str,
) -> Result<(), sqlx::Error> {
    // Delete only this attempt's row. A concurrent attempt has a different
    // token, so it cannot be released or resurrected by this call.
    sqlx::query(
        "DELETE FROM notification_deliveries
         WHERE event_id = ? AND template = ?
           AND status = 'claimed' AND claim_token = ?",
    )
    .bind(event.id.as_str())
    .bind(rule.template)
    .bind(token)
    .execute(db.pool())
    .await?;
    Ok(())
}
