//! Notification channel registry, delivery fan-out and Feishu job processing.
//!
//! Channel endpoints and signing secrets are deliberately absent from the
//! public [`NotificationChannel`] model. They are only read inside the service
//! when a delivery is sent, and they never appear in errors or logs.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fastcart_infrastructure::{AuditActor, JobQueue, NewJob, StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, DomainEventEnvelope, EventId, JobId, NotificationChannelId, NotificationDeliveryId,
    OrderId, Timestamp, now_timestamp,
};
use fastcart_module_order::OrderService;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::context::format_amount;
use crate::feishu::{self, InquiryCardData, OrderCardData};

/// Job type consumed by the channel delivery worker.
pub const SEND_CHANNEL_NOTIFICATION_JOB_TYPE: &str = "send_channel_notification";

/// Priority used for channel delivery jobs.
const CHANNEL_PRIORITY: i64 = 10;
/// HTTP timeout for one Feishu request.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum channels returned by one CRUD listing is unbounded; deliveries are
/// always paginated.
const MAX_DELIVERY_LIMIT: i64 = 100;
/// Maximum event subscriptions on one channel.
const MAX_EVENTS: usize = 20;
/// Maximum channel name length in characters.
const MAX_NAME_CHARS: usize = 120;

const CHANNEL_COLUMNS: &str = "id, kind, name, endpoint_url, signing_secret, events, config, \
     is_active, deleted_at, last_error, last_success_at, created_at, updated_at";
const DELIVERY_COLUMNS: &str = "id, channel_id, event_id, event_type, status, job_id, attempts, \
     response_status, response_body, last_error, created_at, updated_at, sent_at";

mod adapter;
mod channels;
mod deliveries;
mod dispatch;

/// Supported notification channel integrations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    /// Feishu/Lark custom bot webhook.
    Feishu,
}

impl ChannelKind {
    /// Stable storage and API representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Feishu => "feishu",
        }
    }

    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "feishu" => Ok(Self::Feishu),
            other => Err(AppError::internal(format!(
                "unknown notification channel kind: {other}"
            ))),
        }
    }
}

/// A stored notification channel without its endpoint URL or secret.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotificationChannel {
    /// Channel identifier.
    pub id: NotificationChannelId,
    /// Integration kind.
    pub kind: ChannelKind,
    /// Operator-facing name.
    pub name: String,
    /// Subscribed event types.
    pub events: Vec<String>,
    /// Whether dispatch should consider this channel.
    pub is_active: bool,
    /// Adapter-specific configuration.
    pub config: Value,
    /// Masked endpoint URL safe for operator display.
    pub endpoint_url_masked: String,
    /// Whether a signing secret is stored.
    pub signing_secret_configured: bool,
    /// Last delivery or test error.
    pub last_error: Option<String>,
    /// Last successful delivery or test timestamp.
    pub last_success_at: Option<Timestamp>,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Last update timestamp.
    pub updated_at: Timestamp,
}

/// Endpoint values used internally to send a message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelConfig {
    /// Webhook URL. Never exposed by the public channel model.
    pub endpoint_url: String,
    /// Optional Feishu signing secret.
    pub signing_secret: Option<String>,
}

/// Values used to build admin deep links and localize messages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderContext {
    /// Admin panel origin, for example `https://admin.example.com`.
    pub admin_base_url: Option<String>,
    /// Locale used for rendering, for example `zh-CN`.
    pub locale: String,
}

/// Outcome category for one delivery attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelError {
    /// The delivery may succeed later; the job queue should retry it.
    Retryable(String),
    /// Retrying cannot succeed without operator intervention.
    Permanent(String),
}

impl std::fmt::Display for ChannelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retryable(message) | Self::Permanent(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ChannelError {}

/// Persisted delivery state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryStatus {
    /// Claimed but not yet queued.
    Pending,
    /// A `send_channel_notification` job was enqueued.
    Queued,
    /// The provider accepted the message.
    Sent,
    /// Delivery failed permanently.
    Failed,
    /// The channel was disabled or deleted before sending.
    Skipped,
}

impl DeliveryStatus {
    /// Stable storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Queued => "queued",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "pending" => Ok(Self::Pending),
            "queued" => Ok(Self::Queued),
            "sent" => Ok(Self::Sent),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            other => Err(AppError::internal(format!(
                "unknown notification delivery status: {other}"
            ))),
        }
    }
}

/// One persisted channel delivery attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelDelivery {
    /// Delivery identifier.
    pub id: NotificationDeliveryId,
    /// Owning channel.
    pub channel_id: NotificationChannelId,
    /// Source domain event.
    pub event_id: EventId,
    /// Source event type.
    pub event_type: String,
    /// Delivery state.
    pub status: DeliveryStatus,
    /// Enqueued job, once queued.
    pub job_id: Option<JobId>,
    /// Number of send attempts.
    pub attempts: i64,
    /// Last HTTP status.
    pub response_status: Option<i64>,
    /// Truncated last response body.
    pub response_body: Option<String>,
    /// Last error, without endpoint or secret values.
    pub last_error: Option<String>,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Last update timestamp.
    pub updated_at: Timestamp,
    /// Successful delivery timestamp.
    pub sent_at: Option<Timestamp>,
}

/// Counters returned by one fan-out pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelDispatchReport {
    /// Deliveries newly enqueued.
    pub queued: u64,
    /// Deliveries already queued or sent.
    pub duplicates: u64,
    /// Deliveries that failed while rendering or enqueueing.
    pub failed: u64,
    /// Channels skipped because of configuration or event mismatch.
    pub skipped: u64,
}

/// Result of processing one delivery job.
#[derive(Clone, Debug, PartialEq)]
pub enum ChannelJobOutcome {
    /// Provider accepted the message.
    Sent,
    /// Channel was disabled or deleted.
    Skipped,
    /// Delivery failed permanently.
    Failed,
}

/// Result of an operator-initiated channel test.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelTestResult {
    /// Whether the provider accepted the test message.
    pub ok: bool,
    /// HTTP status when a response was received.
    pub status: Option<i64>,
    /// Error summary without endpoint or secret values.
    pub error: Option<String>,
}

/// Input for creating a notification channel.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateChannelInput {
    /// Integration kind.
    pub kind: ChannelKind,
    /// Operator-facing name.
    pub name: String,
    /// Webhook URL.
    pub endpoint_url: String,
    /// Optional signing secret.
    pub signing_secret: Option<String>,
    /// Subscribed event types.
    pub events: Vec<String>,
    /// Whether the channel starts active.
    pub is_active: bool,
    /// Adapter-specific configuration.
    pub config: Value,
}

/// Input for updating a notification channel.
///
/// `signing_secret` distinguishes an omitted value (`None`, keep) from an
/// explicit clear (`Some(None)`) and a replacement (`Some(Some(secret))`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateChannelInput {
    /// Replacement name.
    pub name: Option<String>,
    /// Replacement webhook URL.
    pub endpoint_url: Option<String>,
    /// Secret replacement or clear.
    pub signing_secret: Option<Option<String>>,
    /// Replacement event subscriptions.
    pub events: Option<Vec<String>>,
    /// Replacement active flag.
    pub is_active: Option<bool>,
    /// Replacement configuration.
    pub config: Option<Value>,
}

/// Notification channel CRUD, fan-out and job processing.
pub struct NotificationChannelService {
    adapters: HashMap<ChannelKind, Arc<dyn ChannelAdapter>>,
    client: reqwest::Client,
    allow_local_endpoints: bool,
    audit_actor: Option<String>,
}

impl NotificationChannelService {
    /// Creates a service with the Feishu adapter registered.
    #[must_use]
    pub fn new() -> Self {
        let mut adapters: HashMap<ChannelKind, Arc<dyn ChannelAdapter>> = HashMap::new();
        adapters.insert(ChannelKind::Feishu, Arc::new(adapter::FeishuAdapter));
        Self {
            adapters,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(DELIVERY_TIMEOUT)
                .build()
                .expect("the notification channel HTTP client configuration is valid"),
            allow_local_endpoints: false,
            audit_actor: None,
        }
    }

    /// Records channel mutations in the store audit outbox.
    ///
    /// When an actor is set, every mutating method appends its audit entry in
    /// the same transaction as the change, so a crash cannot separate them.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit_actor = actor_id;
        self
    }

    /// Appends one audit entry to the outbox inside a mutation transaction.
    async fn append_audit(
        &self,
        db: &StoreDatabase,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        data: Value,
    ) -> Result<(), AppError> {
        let Some(actor_id) = self.audit_actor.as_deref() else {
            return Ok(());
        };
        AuditActor::new(actor_id, db.store_id().as_str())
            .append(&mut **tx, action, resource_type, resource_id, data)
            .await
            .map_err(database_error)?;
        Ok(())
    }

    /// Allows plain-HTTP loopback endpoints. Tests and local development only.
    #[doc(hidden)]
    #[must_use]
    pub fn with_local_endpoints_allowed(mut self, allowed: bool) -> Self {
        self.allow_local_endpoints = allowed;
        self
    }

    /// Overrides the HTTP client used for provider requests.
    #[must_use]
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    fn validate_url(&self, raw: &str) -> Result<String, AppError> {
        feishu::validate_endpoint_url(raw, self.allow_local_endpoints)
    }

    async fn fetch_channel_row(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
    ) -> Result<ChannelRow, AppError> {
        sqlx::query_as::<_, ChannelRow>(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM notification_channels
             WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.as_str())
        .fetch_optional(db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("notification channel", id))
    }
}

impl Default for NotificationChannelService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
trait ChannelAdapter: Send + Sync {
    async fn render(
        &self,
        db: &StoreDatabase,
        event: &DomainEventEnvelope,
        context: &RenderContext,
    ) -> Result<Value, ChannelError>;
}

/// Masks a webhook token for display.
///
/// The result keeps the scheme, host and path prefix, then shows `****`
/// followed by the last four characters of the final path segment. Query
/// strings and credentials are never included.
#[must_use]
pub fn mask_endpoint_url(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return "****".to_owned();
    };
    let mut prefix = format!("{}://", parsed.scheme());
    prefix.push_str(parsed.host_str().unwrap_or_default());
    if let Some(port) = parsed.port() {
        prefix.push(':');
        prefix.push_str(&port.to_string());
    }

    let path = parsed.path();
    let (directory, token) = path
        .rsplit_once('/')
        .map_or(("", path), |(directory, token)| (directory, token));
    prefix.push_str(directory);
    prefix.push('/');
    let last_four: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}****{last_four}")
}

/// Returns the built-in notification event catalog.
///
/// Each tuple is `(event_type, label, group, description)`.
#[must_use]
pub fn supported_channel_events()
-> &'static [(&'static str, &'static str, &'static str, &'static str)] {
    &[
        ("order.placed", "新订单", "订单", "订单创建后发送新订单提醒"),
        (
            "inquiry.created",
            "新留言",
            "留言",
            "收到新的表单留言后发送提醒",
        ),
    ]
}

fn retryable_db(context: &str, error: &(impl std::fmt::Display + ?Sized)) -> ChannelError {
    ChannelError::Retryable(format!("{context}: {error}"))
}

#[derive(Debug, sqlx::FromRow)]
struct ChannelRow {
    id: String,
    kind: String,
    name: String,
    endpoint_url: String,
    signing_secret: Option<String>,
    events: String,
    config: String,
    is_active: i64,
    deleted_at: Option<Timestamp>,
    last_error: Option<String>,
    last_success_at: Option<Timestamp>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl ChannelRow {
    fn events(&self) -> Result<Vec<String>, AppError> {
        serde_json::from_str(&self.events)
            .map_err(|error| AppError::internal(format!("invalid channel events: {error}")))
    }

    fn config(&self) -> Result<Value, AppError> {
        serde_json::from_str(&self.config)
            .map_err(|error| AppError::internal(format!("invalid channel config: {error}")))
    }

    fn into_channel(self) -> Result<NotificationChannel, AppError> {
        let kind = ChannelKind::parse(&self.kind)?;
        let events = self.events()?;
        let config = self.config()?;
        let endpoint_url_masked = mask_endpoint_url(&self.endpoint_url);
        let signing_secret_configured = self.signing_secret.is_some();
        Ok(NotificationChannel {
            id: NotificationChannelId::from_string(self.id),
            kind,
            name: self.name,
            events,
            is_active: self.is_active != 0,
            config,
            endpoint_url_masked,
            signing_secret_configured,
            last_error: self.last_error,
            last_success_at: self.last_success_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct DeliveryRow {
    id: String,
    channel_id: String,
    event_id: String,
    event_type: String,
    status: String,
    job_id: Option<String>,
    attempts: i64,
    response_status: Option<i64>,
    response_body: Option<String>,
    last_error: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
    sent_at: Option<Timestamp>,
}

impl DeliveryRow {
    fn into_delivery(self) -> Result<ChannelDelivery, AppError> {
        Ok(ChannelDelivery {
            id: NotificationDeliveryId::from_string(self.id),
            channel_id: NotificationChannelId::from_string(self.channel_id),
            event_id: EventId::from_string(self.event_id),
            event_type: self.event_type,
            status: DeliveryStatus::parse(&self.status)?,
            job_id: self.job_id.map(JobId::from_string),
            attempts: self.attempts,
            response_status: self.response_status,
            response_body: self.response_body,
            last_error: self.last_error,
            created_at: self.created_at,
            updated_at: self.updated_at,
            sent_at: self.sent_at,
        })
    }
}

#[cfg(test)]
mod tests;
