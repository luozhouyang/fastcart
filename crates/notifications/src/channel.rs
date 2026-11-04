//! Notification channel registry, delivery fan-out and Feishu job processing.
//!
//! Channel endpoints and signing secrets are deliberately absent from the
//! public [`NotificationChannel`] model. They are only read inside the service
//! when a delivery is sent, and they never appear in errors or logs.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fastcart_infrastructure::{JobQueue, NewJob, StoreDatabase, database_error};
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
}

impl NotificationChannelService {
    /// Creates a service with the Feishu adapter registered.
    #[must_use]
    pub fn new() -> Self {
        let mut adapters: HashMap<ChannelKind, Arc<dyn ChannelAdapter>> = HashMap::new();
        adapters.insert(ChannelKind::Feishu, Arc::new(FeishuAdapter));
        Self {
            adapters,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(DELIVERY_TIMEOUT)
                .build()
                .expect("the notification channel HTTP client configuration is valid"),
            allow_local_endpoints: false,
        }
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

    /// Lists active and inactive non-deleted channels in creation order.
    pub async fn list_channels(
        &self,
        db: &StoreDatabase,
    ) -> Result<Vec<NotificationChannel>, AppError> {
        let rows = sqlx::query_as::<_, ChannelRow>(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM notification_channels
             WHERE deleted_at IS NULL
             ORDER BY created_at ASC, id ASC"
        ))
        .fetch_all(db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(ChannelRow::into_channel).collect()
    }

    /// Loads one non-deleted channel.
    pub async fn get_channel(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
    ) -> Result<NotificationChannel, AppError> {
        self.fetch_channel_row(db, id).await?.into_channel()
    }

    /// Registers a new channel.
    pub async fn create_channel(
        &self,
        db: &StoreDatabase,
        input: CreateChannelInput,
    ) -> Result<NotificationChannel, AppError> {
        let name = normalize_name(&input.name)?;
        let endpoint_url = self.validate_url(&input.endpoint_url)?;
        let signing_secret = normalize_secret(input.signing_secret)?;
        let events = normalize_events(&input.events)?;
        validate_config(&input.config)?;
        let config = input.config.clone();

        let id = NotificationChannelId::new();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO notification_channels
             (id, kind, name, endpoint_url, signing_secret, events, config, is_active,
              created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.kind.as_str())
        .bind(&name)
        .bind(&endpoint_url)
        .bind(signing_secret.as_deref())
        .bind(serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_owned()))
        .bind(config.to_string())
        .bind(i64::from(input.is_active))
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .map_err(database_error)?;

        self.get_channel(db, &id).await
    }

    /// Updates a channel. Omitted fields keep their current value.
    pub async fn update_channel(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
        input: UpdateChannelInput,
    ) -> Result<NotificationChannel, AppError> {
        let current = self.fetch_channel_row(db, id).await?;

        let name = match input.name {
            Some(name) => normalize_name(&name)?,
            None => current.name.clone(),
        };
        let endpoint_url = match input.endpoint_url {
            Some(url) => self.validate_url(&url)?,
            None => current.endpoint_url.clone(),
        };
        let signing_secret = match input.signing_secret {
            Some(secret) => normalize_secret(secret)?,
            None => current.signing_secret.clone(),
        };
        let events = match input.events {
            Some(events) => normalize_events(&events)?,
            None => current.events()?,
        };
        let config = match input.config {
            Some(config) => {
                validate_config(&config)?;
                config
            }
            None => current.config()?,
        };
        let is_active = input.is_active.unwrap_or(current.is_active != 0);

        sqlx::query(
            "UPDATE notification_channels
             SET name = ?, endpoint_url = ?, signing_secret = ?, events = ?,
                 config = ?, is_active = ?, updated_at = ?
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(&name)
        .bind(&endpoint_url)
        .bind(signing_secret.as_deref())
        .bind(serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_owned()))
        .bind(config.to_string())
        .bind(i64::from(is_active))
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(db.pool())
        .await
        .map_err(database_error)?;

        self.get_channel(db, id).await
    }

    /// Soft-deletes a channel. Delivery history is retained.
    pub async fn delete_channel(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
    ) -> Result<(), AppError> {
        let now = now_timestamp();
        let result = sqlx::query(
            "UPDATE notification_channels
             SET deleted_at = ?, updated_at = ?
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(id.as_str())
        .execute(db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("notification channel", id));
        }
        Ok(())
    }

    /// Lists deliveries for one channel, newest first.
    pub async fn list_deliveries(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
        limit: i64,
    ) -> Result<Vec<ChannelDelivery>, AppError> {
        let rows = sqlx::query_as::<_, DeliveryRow>(&format!(
            "SELECT {DELIVERY_COLUMNS} FROM notification_channel_deliveries
             WHERE channel_id = ?
             ORDER BY created_at DESC, id DESC
             LIMIT ?"
        ))
        .bind(id.as_str())
        .bind(limit.clamp(1, MAX_DELIVERY_LIMIT))
        .fetch_all(db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(DeliveryRow::into_delivery).collect()
    }

    /// Fans one domain event out to every subscribed channel.
    ///
    /// A unique `(channel_id, event_id)` delivery row makes replays
    /// idempotent. Rendering failures are isolated to one channel; an enqueue
    /// failure returns an error so the outbox can retry the whole event.
    pub async fn dispatch(
        &self,
        db: &StoreDatabase,
        queue: &JobQueue,
        event: &DomainEventEnvelope,
        context: &RenderContext,
    ) -> Result<ChannelDispatchReport, AppError> {
        let rows = sqlx::query_as::<_, ChannelRow>(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM notification_channels
             WHERE is_active = 1 AND deleted_at IS NULL
             ORDER BY created_at ASC, id ASC"
        ))
        .fetch_all(db.pool())
        .await
        .map_err(database_error)?;

        let mut report = ChannelDispatchReport::default();
        for row in rows {
            let events = match row.events() {
                Ok(events) => events,
                Err(_) => {
                    report.skipped += 1;
                    tracing::warn!(channel_id = %row.id, "notification channel has invalid events");
                    continue;
                }
            };
            if !events
                .iter()
                .any(|event_type| event_type == &event.event_type)
            {
                continue;
            }
            let Ok(kind) = ChannelKind::parse(&row.kind) else {
                report.skipped += 1;
                tracing::warn!(channel_id = %row.id, "notification channel has unknown kind");
                continue;
            };
            let Some(adapter) = self.adapters.get(&kind) else {
                report.skipped += 1;
                tracing::warn!(channel_id = %row.id, "notification channel adapter is not registered");
                continue;
            };

            let channel_id = NotificationChannelId::from_string(row.id.clone());
            let now = now_timestamp();
            let candidate_id = NotificationDeliveryId::new();
            let inserted = sqlx::query(
                "INSERT OR IGNORE INTO notification_channel_deliveries
                 (id, channel_id, event_id, event_type, status, attempts, created_at, updated_at)
                 VALUES (?, ?, ?, ?, 'pending', 0, ?, ?)",
            )
            .bind(candidate_id.as_str())
            .bind(channel_id.as_str())
            .bind(event.id.as_str())
            .bind(&event.event_type)
            .bind(now)
            .bind(now)
            .execute(db.pool())
            .await
            .map_err(database_error)?;

            let delivery_id = if inserted.rows_affected() > 0 {
                candidate_id
            } else {
                let existing = sqlx::query_as::<_, DeliveryRow>(&format!(
                    "SELECT {DELIVERY_COLUMNS} FROM notification_channel_deliveries
                     WHERE channel_id = ? AND event_id = ?"
                ))
                .bind(channel_id.as_str())
                .bind(event.id.as_str())
                .fetch_optional(db.pool())
                .await
                .map_err(database_error)?
                .ok_or_else(|| {
                    AppError::internal("notification delivery claim disappeared during dispatch")
                })?;
                let status = DeliveryStatus::parse(&existing.status)?;
                if matches!(status, DeliveryStatus::Queued | DeliveryStatus::Sent) {
                    report.duplicates += 1;
                    continue;
                }
                NotificationDeliveryId::from_string(existing.id)
            };

            let message = match adapter.render(db, event, context).await {
                Ok(message) => message,
                Err(error) => {
                    mark_delivery_failed(db, &delivery_id, &error.to_string()).await?;
                    report.failed += 1;
                    tracing::warn!(
                        channel_id = %channel_id,
                        event_id = %event.id,
                        event_type = %event.event_type,
                        "notification channel render failed"
                    );
                    continue;
                }
            };

            let payload = json!({
                "channel_id": channel_id.as_str(),
                "delivery_id": delivery_id.as_str(),
                "event_id": event.id.as_str(),
                "event_type": event.event_type,
                "message": message,
            });
            let job = match queue
                .enqueue(
                    NewJob::new(SEND_CHANNEL_NOTIFICATION_JOB_TYPE, payload)
                        .with_priority(CHANNEL_PRIORITY),
                )
                .await
            {
                Ok(job) => job,
                Err(error) => {
                    mark_delivery_failed(
                        db,
                        &delivery_id,
                        "notification channel job could not be enqueued",
                    )
                    .await?;
                    tracing::warn!(
                        channel_id = %channel_id,
                        event_id = %event.id,
                        event_type = %event.event_type,
                        "notification channel job enqueue failed"
                    );
                    return Err(AppError::database(error.to_string()));
                }
            };

            sqlx::query(
                "UPDATE notification_channel_deliveries
                 SET status = 'queued', job_id = ?, last_error = NULL, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&job.id)
            .bind(now_timestamp())
            .bind(delivery_id.as_str())
            .execute(db.pool())
            .await
            .map_err(database_error)?;
            report.queued += 1;
        }

        Ok(report)
    }

    /// Processes one `send_channel_notification` job payload.
    ///
    /// Retryable provider failures return [`ChannelError::Retryable`] so the
    /// job queue applies its backoff. Permanent failures are recorded on the
    /// delivery row and return [`ChannelJobOutcome::Failed`].
    pub async fn process_job(
        &self,
        db: &StoreDatabase,
        payload: &Value,
    ) -> Result<ChannelJobOutcome, ChannelError> {
        let channel_id = NotificationChannelId::from_string(payload_string(payload, "channel_id")?);
        let delivery_id =
            NotificationDeliveryId::from_string(payload_string(payload, "delivery_id")?);
        let _event_id = payload_string(payload, "event_id")?;
        let _event_type = payload_string(payload, "event_type")?;
        let message = payload
            .get("message")
            .cloned()
            .ok_or_else(|| ChannelError::Permanent("job payload is missing message".to_owned()))?;

        let delivery = load_delivery_row(db, &delivery_id)
            .await
            .map_err(|error| retryable_db("delivery lookup failed", &error))?
            .ok_or_else(|| ChannelError::Permanent("notification delivery not found".to_owned()))?;
        let status = DeliveryStatus::parse(&delivery.status).map_err(|_| {
            ChannelError::Permanent("notification delivery status is invalid".to_owned())
        })?;
        match status {
            DeliveryStatus::Sent => return Ok(ChannelJobOutcome::Sent),
            DeliveryStatus::Skipped => return Ok(ChannelJobOutcome::Skipped),
            DeliveryStatus::Pending | DeliveryStatus::Queued | DeliveryStatus::Failed => {}
        }

        let channel = load_channel_row_including_deleted(db, &channel_id)
            .await
            .map_err(|error| retryable_db("channel lookup failed", &error))?;
        let Some(channel) = channel else {
            mark_delivery_skipped(db, &delivery_id, "notification channel does not exist").await?;
            return Ok(ChannelJobOutcome::Skipped);
        };
        if channel.deleted_at.is_some() || channel.is_active == 0 {
            mark_delivery_skipped(db, &delivery_id, "notification channel is inactive").await?;
            return Ok(ChannelJobOutcome::Skipped);
        }

        let result = feishu::send(
            &self.client,
            &channel.endpoint_url,
            channel.signing_secret.as_deref(),
            &message,
        )
        .await;

        match result.result {
            Ok(()) => {
                let now = now_timestamp();
                sqlx::query(
                    "UPDATE notification_channel_deliveries
                     SET status = 'sent', response_status = ?, response_body = ?,
                         last_error = NULL, sent_at = ?, updated_at = ?,
                         attempts = attempts + 1
                     WHERE id = ?",
                )
                .bind(result.status)
                .bind(result.body.as_deref())
                .bind(now)
                .bind(now)
                .bind(delivery_id.as_str())
                .execute(db.pool())
                .await
                .map_err(|error| retryable_db("delivery update failed", &error))?;
                sqlx::query(
                    "UPDATE notification_channels
                     SET last_success_at = ?, last_error = NULL, updated_at = ?
                     WHERE id = ?",
                )
                .bind(now)
                .bind(now)
                .bind(channel_id.as_str())
                .execute(db.pool())
                .await
                .map_err(|error| retryable_db("channel update failed", &error))?;
                Ok(ChannelJobOutcome::Sent)
            }
            Err(ChannelError::Retryable(message)) => {
                let now = now_timestamp();
                sqlx::query(
                    "UPDATE notification_channel_deliveries
                     SET status = 'queued', response_status = ?, response_body = ?,
                         last_error = ?, updated_at = ?, attempts = attempts + 1
                     WHERE id = ?",
                )
                .bind(result.status)
                .bind(result.body.as_deref())
                .bind(&message)
                .bind(now)
                .bind(delivery_id.as_str())
                .execute(db.pool())
                .await
                .map_err(|error| retryable_db("delivery update failed", &error))?;
                update_channel_error(db, &channel_id, &message).await?;
                Err(ChannelError::Retryable(message))
            }
            Err(ChannelError::Permanent(message)) => {
                let now = now_timestamp();
                sqlx::query(
                    "UPDATE notification_channel_deliveries
                     SET status = 'failed', response_status = ?, response_body = ?,
                         last_error = ?, updated_at = ?, attempts = attempts + 1
                     WHERE id = ?",
                )
                .bind(result.status)
                .bind(result.body.as_deref())
                .bind(&message)
                .bind(now)
                .bind(delivery_id.as_str())
                .execute(db.pool())
                .await
                .map_err(|error| retryable_db("delivery update failed", &error))?;
                update_channel_error(db, &channel_id, &message).await?;
                Ok(ChannelJobOutcome::Failed)
            }
        }
    }

    /// Sends a test message without creating a delivery row.
    pub async fn test_channel(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
        context: &RenderContext,
    ) -> Result<ChannelTestResult, AppError> {
        let _ = context;
        let channel = self.fetch_channel_row(db, id).await?;
        let kind = ChannelKind::parse(&channel.kind)?;
        if kind != ChannelKind::Feishu {
            return Err(AppError::validation(format!(
                "{} channels cannot be tested",
                kind.as_str()
            )));
        }

        let message = json!({
            "msg_type": "text",
            "content": {"text": "Fastcart 通知测试"}
        });
        let result = feishu::send(
            &self.client,
            &channel.endpoint_url,
            channel.signing_secret.as_deref(),
            &message,
        )
        .await;
        let now = now_timestamp();

        match &result.result {
            Ok(()) => {
                sqlx::query(
                    "UPDATE notification_channels
                     SET last_success_at = ?, last_error = NULL, updated_at = ?
                     WHERE id = ?",
                )
                .bind(now)
                .bind(now)
                .bind(id.as_str())
                .execute(db.pool())
                .await
                .map_err(database_error)?;
                Ok(ChannelTestResult {
                    ok: true,
                    status: result.status,
                    error: None,
                })
            }
            Err(error) => {
                sqlx::query(
                    "UPDATE notification_channels SET last_error = ?, updated_at = ?
                     WHERE id = ?",
                )
                .bind(error.to_string())
                .bind(now)
                .bind(id.as_str())
                .execute(db.pool())
                .await
                .map_err(database_error)?;
                Ok(ChannelTestResult {
                    ok: false,
                    status: result.status,
                    error: Some(error.to_string()),
                })
            }
        }
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

struct FeishuAdapter;

#[async_trait]
impl ChannelAdapter for FeishuAdapter {
    async fn render(
        &self,
        db: &StoreDatabase,
        event: &DomainEventEnvelope,
        context: &RenderContext,
    ) -> Result<Value, ChannelError> {
        match event.event_type.as_str() {
            "order.placed" => {
                let data = order_card_data(db, event).await;
                feishu::render_order_card(&data, context)
            }
            "inquiry.created" => {
                let data = inquiry_card_data(event);
                feishu::render_inquiry_card(&data, context)
            }
            other => Err(ChannelError::Permanent(format!(
                "unsupported notification event: {other}"
            ))),
        }
    }
}

async fn order_card_data(db: &StoreDatabase, event: &DomainEventEnvelope) -> OrderCardData {
    let order_id = event
        .data
        .get("order_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let order = match order_id.as_deref() {
        Some(id) => OrderService::new(db.clone())
            .get(&OrderId::from_string(id))
            .await
            .ok(),
        None => None,
    };

    if let Some(order) = order {
        return OrderCardData {
            display_number: order.display_number.to_string(),
            order_id: Some(order.id.as_str().to_owned()),
            amount: format_amount(order.total, Some(&order.currency_code)),
            customer: order.email,
            item_count: order.items.iter().map(|item| item.quantity).sum(),
            created_at: order.created_at,
        };
    }

    let payload = &event.data;
    let currency = payload_string_field(payload, &["currency_code", "currency"]);
    let amount = payload
        .get("total")
        .and_then(Value::as_i64)
        .map(|total| format_amount(total, currency))
        .or_else(|| {
            payload
                .get("total")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            payload
                .get("amount")
                .and_then(Value::as_i64)
                .map(|total| format_amount(total, currency))
        })
        .unwrap_or_else(|| "—".to_owned());
    let item_count = payload
        .get("item_count")
        .and_then(Value::as_i64)
        .or_else(|| {
            payload.get("items").and_then(Value::as_array).map(|items| {
                items
                    .iter()
                    .map(|item| item.get("quantity").and_then(Value::as_i64).unwrap_or(1))
                    .sum()
            })
        })
        .unwrap_or(0);

    OrderCardData {
        display_number: payload
            .get("display_number")
            .and_then(Value::as_i64)
            .map(|number| number.to_string())
            .unwrap_or_else(|| "—".to_owned()),
        order_id,
        amount,
        customer: payload_string_field(payload, &["email", "customer_email"])
            .unwrap_or("—")
            .to_owned(),
        item_count,
        created_at: event.occurred_at,
    }
}

fn inquiry_card_data(event: &DomainEventEnvelope) -> InquiryCardData {
    let payload = &event.data;
    let contact = payload_string_field(payload, &["email", "phone", "contact"])
        .unwrap_or("—")
        .to_owned();
    let attachment_count = payload
        .get("attachment_count")
        .and_then(Value::as_i64)
        .or_else(|| {
            payload
                .get("attachments")
                .and_then(Value::as_array)
                .map(|files| files.len() as i64)
        })
        .or_else(|| {
            payload
                .get("files")
                .and_then(Value::as_object)
                .map(|files| {
                    files
                        .values()
                        .filter_map(Value::as_array)
                        .map(|values| values.len() as i64)
                        .sum()
                })
        })
        .unwrap_or(0);

    let custom_fields = payload
        .get("custom_fields")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let label = item.get("label").and_then(Value::as_str)?;
                    let value = item.get("value").and_then(Value::as_str)?;
                    Some((label.to_owned(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();

    InquiryCardData {
        form_handle: payload_string_field(payload, &["form_handle"])
            .unwrap_or("—")
            .to_owned(),
        inquiry_id: payload
            .get("inquiry_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        name: payload_string_field(payload, &["name"])
            .unwrap_or("—")
            .to_owned(),
        contact,
        subject: payload_string_field(payload, &["subject"])
            .unwrap_or("—")
            .to_owned(),
        message: payload_string_field(payload, &["message"])
            .unwrap_or("")
            .to_owned(),
        attachment_count,
        custom_fields,
    }
}

fn payload_string<'a>(payload: &'a Value, key: &str) -> Result<&'a str, ChannelError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ChannelError::Permanent(format!("job payload is missing {key}")))
}

fn payload_string_field<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
}

fn retryable_db(context: &str, error: &(impl std::fmt::Display + ?Sized)) -> ChannelError {
    ChannelError::Retryable(format!("{context}: {error}"))
}

async fn update_channel_error(
    db: &StoreDatabase,
    id: &NotificationChannelId,
    error: &str,
) -> Result<(), ChannelError> {
    sqlx::query("UPDATE notification_channels SET last_error = ?, updated_at = ? WHERE id = ?")
        .bind(error)
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(db.pool())
        .await
        .map_err(|error| retryable_db("channel update failed", &database_error(error)))?;
    Ok(())
}

async fn mark_delivery_failed(
    db: &StoreDatabase,
    id: &NotificationDeliveryId,
    error: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE notification_channel_deliveries
         SET status = 'failed', last_error = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(error)
    .bind(now_timestamp())
    .bind(id.as_str())
    .execute(db.pool())
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn mark_delivery_skipped(
    db: &StoreDatabase,
    id: &NotificationDeliveryId,
    reason: &str,
) -> Result<(), ChannelError> {
    sqlx::query(
        "UPDATE notification_channel_deliveries
         SET status = 'skipped', last_error = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(reason)
    .bind(now_timestamp())
    .bind(id.as_str())
    .execute(db.pool())
    .await
    .map_err(|error| retryable_db("delivery update failed", &database_error(error)))?;
    Ok(())
}

async fn load_delivery_row(
    db: &StoreDatabase,
    id: &NotificationDeliveryId,
) -> Result<Option<DeliveryRow>, AppError> {
    sqlx::query_as::<_, DeliveryRow>(&format!(
        "SELECT {DELIVERY_COLUMNS} FROM notification_channel_deliveries WHERE id = ?"
    ))
    .bind(id.as_str())
    .fetch_optional(db.pool())
    .await
    .map_err(database_error)
}

async fn load_channel_row_including_deleted(
    db: &StoreDatabase,
    id: &NotificationChannelId,
) -> Result<Option<ChannelRow>, AppError> {
    sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {CHANNEL_COLUMNS} FROM notification_channels WHERE id = ?"
    ))
    .bind(id.as_str())
    .fetch_optional(db.pool())
    .await
    .map_err(database_error)
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

fn normalize_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::validation(format!(
            "notification channel name must be 1 to {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(name.to_owned())
}

fn normalize_secret(secret: Option<String>) -> Result<Option<String>, AppError> {
    let Some(secret) = secret else {
        return Ok(None);
    };
    if secret.is_empty() {
        return Ok(None);
    }
    if secret.chars().count() > 512 || secret.chars().any(char::is_control) {
        return Err(AppError::validation(
            "notification channel signing secret must be at most 512 printable characters",
        ));
    }
    Ok(Some(secret))
}

fn normalize_events(events: &[String]) -> Result<Vec<String>, AppError> {
    if events.is_empty() {
        return Err(AppError::validation(
            "at least one notification event subscription is required",
        ));
    }
    if events.len() > MAX_EVENTS {
        return Err(AppError::validation(format!(
            "a notification channel supports at most {MAX_EVENTS} event subscriptions"
        )));
    }
    let supported: BTreeSet<&'static str> = supported_channel_events()
        .iter()
        .map(|(event_type, _, _, _)| *event_type)
        .collect();
    let mut normalized = Vec::with_capacity(events.len());
    for event in events {
        let event = event.trim();
        if !supported.contains(event) {
            return Err(AppError::validation(format!(
                "unsupported notification event: {event}"
            )));
        }
        if !normalized.iter().any(|existing: &String| existing == event) {
            normalized.push(event.to_owned());
        }
    }
    Ok(normalized)
}

fn validate_config(config: &Value) -> Result<(), AppError> {
    if !config.is_object() {
        return Err(AppError::validation(
            "notification channel config must be a JSON object",
        ));
    }
    Ok(())
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
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::serve;
    use fastcart_infrastructure::{JobQueue, StoreDatabase, jobs::status};
    use fastcart_kernel::{DomainEventEnvelope, StoreId};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use super::{
        ChannelDispatchReport, ChannelError, ChannelJobOutcome, ChannelKind, CreateChannelInput,
        DeliveryStatus, NotificationChannelService, RenderContext, UpdateChannelInput,
        mask_endpoint_url, supported_channel_events,
    };

    #[derive(Clone)]
    struct MockState {
        responses: Arc<Mutex<VecDeque<(u16, String)>>>,
        captured: Arc<Mutex<Vec<Value>>>,
    }

    async fn receive(State(state): State<MockState>, body: String) -> (StatusCode, String) {
        state
            .captured
            .lock()
            .unwrap()
            .push(serde_json::from_str(&body).unwrap_or(Value::Null));
        let (status, body) = state
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or((200, r#"{"code":0}"#.to_owned()));
        (StatusCode::from_u16(status).unwrap(), body)
    }

    async fn spawn_mock(responses: Vec<(u16, String)>) -> (String, MockState) {
        let state = MockState {
            responses: Arc::new(Mutex::new(responses.into())),
            captured: Arc::default(),
        };
        let app = Router::new()
            .route("/hook", post(receive))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            serve(listener, app).await.unwrap();
        });
        (format!("http://{address}/hook"), state)
    }

    async fn database() -> StoreDatabase {
        StoreDatabase::in_memory(StoreId::new()).await.unwrap()
    }

    fn service() -> NotificationChannelService {
        NotificationChannelService::new().with_local_endpoints_allowed(true)
    }

    fn context() -> RenderContext {
        RenderContext {
            admin_base_url: Some("https://admin.example.com".to_owned()),
            locale: "zh-CN".to_owned(),
        }
    }

    fn order_event() -> DomainEventEnvelope {
        DomainEventEnvelope::new(
            "order.placed",
            None,
            json!({
                "order_id": "order_missing",
                "display_number": 42,
                "email": "Buyer@Example.com",
                "currency_code": "USD",
                "total": 1999
            }),
        )
    }

    async fn create_channel(
        service: &NotificationChannelService,
        db: &StoreDatabase,
        url: String,
        events: Vec<&str>,
    ) -> super::NotificationChannel {
        service
            .create_channel(
                db,
                CreateChannelInput {
                    kind: ChannelKind::Feishu,
                    name: "Operations".to_owned(),
                    endpoint_url: url,
                    signing_secret: Some("test-secret".to_owned()),
                    events: events.into_iter().map(str::to_owned).collect(),
                    is_active: true,
                    config: json!({}),
                },
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn dispatch_queues_one_delivery_and_is_idempotent() {
        let db = database().await;
        let queue = JobQueue::new(db.pool().clone());
        let service = service();
        let (url, _mock) = spawn_mock(vec![]).await;
        let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
        let event = order_event();

        let report = service
            .dispatch(&db, &queue, &event, &context())
            .await
            .unwrap();
        assert_eq!(
            report,
            ChannelDispatchReport {
                queued: 1,
                duplicates: 0,
                failed: 0,
                skipped: 0,
            }
        );
        assert_eq!(queue.count_by_status(status::PENDING).await.unwrap(), 1);

        let deliveries = service.list_deliveries(&db, &channel.id, 10).await.unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].status, DeliveryStatus::Queued);
        assert_eq!(deliveries[0].event_id, event.id);
        assert_eq!(deliveries[0].event_type, "order.placed");
        assert!(deliveries[0].job_id.is_some());

        let replay = service
            .dispatch(&db, &queue, &event, &context())
            .await
            .unwrap();
        assert_eq!(replay.duplicates, 1);
        assert_eq!(replay.queued, 0);
        assert_eq!(queue.count_by_status(status::PENDING).await.unwrap(), 1);
        assert_eq!(
            service
                .list_deliveries(&db, &channel.id, 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn retryable_provider_failure_is_retried_then_sent() {
        let db = database().await;
        let queue = JobQueue::new(db.pool().clone());
        let service = service();
        let (url, mock) = spawn_mock(vec![
            (500, "temporary failure".to_owned()),
            (200, r#"{"code":0}"#.to_owned()),
        ])
        .await;
        let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
        let event = order_event();
        service
            .dispatch(&db, &queue, &event, &context())
            .await
            .unwrap();

        let claimed = queue.claim("worker", 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        let payload = claimed[0].json_payload();

        let first = service.process_job(&db, &payload).await;
        assert!(matches!(first, Err(ChannelError::Retryable(_))));
        let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
        assert_eq!(delivery.status, DeliveryStatus::Queued);
        assert_eq!(delivery.attempts, 1);
        assert!(delivery.last_error.is_some());

        let second = service.process_job(&db, &payload).await.unwrap();
        assert_eq!(second, ChannelJobOutcome::Sent);
        let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
        assert_eq!(delivery.status, DeliveryStatus::Sent);
        assert_eq!(delivery.attempts, 2);
        assert_eq!(delivery.response_status, Some(200));
        assert!(delivery.sent_at.is_some());

        let channel = service.get_channel(&db, &channel.id).await.unwrap();
        assert!(channel.last_success_at.is_some());
        assert!(channel.last_error.is_none());

        let captured = mock.captured.lock().unwrap().clone();
        assert_eq!(captured.len(), 2);
        for body in captured {
            let timestamp: i64 = body["timestamp"].as_str().unwrap().parse().unwrap();
            let expected = crate::feishu::sign("test-secret", timestamp).unwrap();
            assert_eq!(body["sign"].as_str().unwrap(), expected);
            assert_eq!(body["msg_type"], "interactive");
        }
    }

    #[tokio::test]
    async fn permanent_provider_failure_marks_the_delivery_failed() {
        let db = database().await;
        let queue = JobQueue::new(db.pool().clone());
        let service = service();
        let (url, _mock) = spawn_mock(vec![(
            200,
            r#"{"code":19022,"msg":"ip not allowed"}"#.to_owned(),
        )])
        .await;
        let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
        service
            .dispatch(&db, &queue, &order_event(), &context())
            .await
            .unwrap();

        let claimed = queue.claim("worker", 10).await.unwrap();
        let payload = claimed[0].json_payload();
        let outcome = service.process_job(&db, &payload).await.unwrap();
        assert_eq!(outcome, ChannelJobOutcome::Failed);

        let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
        assert_eq!(delivery.status, DeliveryStatus::Failed);
        assert_eq!(delivery.attempts, 1);
        assert!(delivery.last_error.as_deref().unwrap().contains("19022"));
        let channel = service.get_channel(&db, &channel.id).await.unwrap();
        assert!(channel.last_error.is_some());
    }

    #[tokio::test]
    async fn inactive_channels_are_skipped_during_job_processing() {
        let db = database().await;
        let queue = JobQueue::new(db.pool().clone());
        let service = service();
        let (url, _mock) = spawn_mock(vec![]).await;
        let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
        service
            .dispatch(&db, &queue, &order_event(), &context())
            .await
            .unwrap();
        let claimed = queue.claim("worker", 10).await.unwrap();
        let payload = claimed[0].json_payload();

        service
            .update_channel(
                &db,
                &channel.id,
                UpdateChannelInput {
                    is_active: Some(false),
                    ..UpdateChannelInput::default()
                },
            )
            .await
            .unwrap();

        let outcome = service.process_job(&db, &payload).await.unwrap();
        assert_eq!(outcome, ChannelJobOutcome::Skipped);
        let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
        assert_eq!(delivery.status, DeliveryStatus::Skipped);
    }

    #[tokio::test]
    async fn channel_crud_masks_endpoints_and_validates_input() {
        let db = database().await;
        let service = service();
        let channel = service
            .create_channel(
                &db,
                CreateChannelInput {
                    kind: ChannelKind::Feishu,
                    name: "Ops".to_owned(),
                    endpoint_url: "https://open.feishu.cn/open-apis/bot/v2/hook/abcdef123456"
                        .to_owned(),
                    signing_secret: Some("secret".to_owned()),
                    events: vec!["order.placed".to_owned(), "inquiry.created".to_owned()],
                    is_active: true,
                    config: json!({"card": "compact"}),
                },
            )
            .await
            .unwrap();
        assert_eq!(channel.kind, ChannelKind::Feishu);
        assert_eq!(channel.events, vec!["order.placed", "inquiry.created"]);
        assert_eq!(
            service.list_channels(&db).await.unwrap(),
            vec![channel.clone()]
        );

        assert_eq!(
            mask_endpoint_url("https://open.feishu.cn/open-apis/bot/v2/hook/abcdef123456"),
            "https://open.feishu.cn/open-apis/bot/v2/hook/****3456"
        );
        assert_eq!(mask_endpoint_url("not a url"), "****");

        let secret: Option<String> =
            sqlx::query_scalar("SELECT signing_secret FROM notification_channels WHERE id = ?")
                .bind(channel.id.as_str())
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(secret.as_deref(), Some("secret"));

        let updated = service
            .update_channel(
                &db,
                &channel.id,
                UpdateChannelInput {
                    name: Some("Renamed".to_owned()),
                    signing_secret: Some(None),
                    events: Some(vec!["inquiry.created".to_owned()]),
                    is_active: Some(false),
                    config: Some(json!({"card": "full"})),
                    ..UpdateChannelInput::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.events, vec!["inquiry.created"]);
        assert!(!updated.is_active);
        assert_eq!(updated.config, json!({"card": "full"}));
        let secret: Option<String> =
            sqlx::query_scalar("SELECT signing_secret FROM notification_channels WHERE id = ?")
                .bind(channel.id.as_str())
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert!(secret.is_none());

        service
            .update_channel(
                &db,
                &channel.id,
                UpdateChannelInput {
                    signing_secret: Some(Some("new-secret".to_owned())),
                    ..UpdateChannelInput::default()
                },
            )
            .await
            .unwrap();
        let secret: Option<String> =
            sqlx::query_scalar("SELECT signing_secret FROM notification_channels WHERE id = ?")
                .bind(channel.id.as_str())
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(secret.as_deref(), Some("new-secret"));

        service.delete_channel(&db, &channel.id).await.unwrap();
        assert!(service.list_channels(&db).await.unwrap().is_empty());
        assert!(
            service
                .get_channel(&db, &channel.id)
                .await
                .unwrap_err()
                .is_not_found()
        );

        for input in [
            CreateChannelInput {
                name: "Ops".to_owned(),
                endpoint_url: "https://example.com/hook".to_owned(),
                events: vec!["order.placed".to_owned()],
                config: json!({}),
                ..valid_input()
            },
            CreateChannelInput {
                events: vec!["product.updated".to_owned()],
                ..valid_input()
            },
            CreateChannelInput {
                name: " ".to_owned(),
                ..valid_input()
            },
            CreateChannelInput {
                config: json!([]),
                ..valid_input()
            },
        ] {
            assert!(
                service.create_channel(&db, input).await.is_err(),
                "invalid channel input was accepted"
            );
        }

        let event_types: Vec<&str> = supported_channel_events()
            .iter()
            .map(|(event_type, _, _, _)| *event_type)
            .collect();
        assert_eq!(event_types, vec!["order.placed", "inquiry.created"]);
    }

    fn valid_input() -> CreateChannelInput {
        CreateChannelInput {
            kind: ChannelKind::Feishu,
            name: "Ops".to_owned(),
            endpoint_url: "https://open.feishu.cn/open-apis/bot/v2/hook/token1234".to_owned(),
            signing_secret: None,
            events: vec!["order.placed".to_owned()],
            is_active: true,
            config: json!({}),
        }
    }

    #[tokio::test]
    async fn test_channel_sends_without_writing_a_delivery() {
        let db = database().await;
        let service = service();
        let (url, mock) = spawn_mock(vec![]).await;
        let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;

        let result = service
            .test_channel(&db, &channel.id, &context())
            .await
            .unwrap();
        assert!(result.ok);
        assert_eq!(result.status, Some(200));
        assert!(result.error.is_none());
        assert!(
            service
                .list_deliveries(&db, &channel.id, 10)
                .await
                .unwrap()
                .is_empty()
        );

        let captured = mock.captured.lock().unwrap().clone();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0]["msg_type"], "text");
        assert_eq!(captured[0]["content"]["text"], "Fastcart 通知测试");
        assert!(captured[0]["sign"].is_string());

        let channel = service.get_channel(&db, &channel.id).await.unwrap();
        assert!(channel.last_success_at.is_some());
    }
}
