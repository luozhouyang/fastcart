//! SQLite-backed webhook registry and delivery worker.

use std::net::IpAddr;
use std::time::Duration;

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, DomainEventEnvelope, Metadata, Timestamp, WebhookDeliveryId, WebhookId,
    now_timestamp, validate_metadata,
};
use futures::StreamExt;
use hmac::{Hmac, Mac};
use rand::RngCore;
use reqwest::Url;
use serde_json::Value;
use sha2::Sha256;
use sqlx::FromRow;

use crate::domain::{
    CreateWebhookEndpointInput, CreatedWebhookEndpoint, DEFAULT_MAX_ATTEMPTS, DeliveryReport,
    DeliveryStatus, MAX_EVENT_LENGTH, MAX_EVENTS, MAX_RESPONSE_BODY_BYTES, MAX_URL_LENGTH,
    UpdateWebhookEndpointInput, WebhookDelivery, WebhookEndpoint, retry_delay_seconds,
};

const ENDPOINT_COLUMNS: &str =
    "id, url, secret, is_active, events, metadata, created_at, updated_at";
const DELIVERY_COLUMNS: &str = "id, endpoint_id, event_id, event_type, payload, status, attempts, \
     response_status, response_body, next_retry_at, created_at, updated_at";
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Application service for webhook endpoints and outbound deliveries.
#[derive(Clone, Debug)]
pub struct WebhookService {
    db: StoreDatabase,
    client: reqwest::Client,
    max_attempts: u32,
    allow_private_networks: bool,
}

impl WebhookService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self {
            db,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(DELIVERY_TIMEOUT)
                .build()
                .expect("the webhook HTTP client configuration is valid"),
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            allow_private_networks: false,
        }
    }

    /// Allows loopback and private destinations. Only for tests and local
    /// development; production deployments must leave this disabled.
    #[must_use]
    pub fn with_private_networks_allowed(mut self, allowed: bool) -> Self {
        self.allow_private_networks = allowed;
        self
    }

    /// Overrides the HTTP client used for deliveries.
    #[must_use]
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Overrides the number of attempts before a delivery is marked failed.
    #[must_use]
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    /// Registers an endpoint and returns its signing secret exactly once.
    pub async fn create_endpoint(
        &self,
        input: CreateWebhookEndpointInput,
    ) -> Result<CreatedWebhookEndpoint, AppError> {
        let url = validate_url(&input.url, self.allow_private_networks)?;
        let events = normalize_events(input.events)?;
        let secret = match input.secret {
            Some(secret) => {
                validate_secret(&secret)?;
                secret
            }
            None => generate_secret(),
        };
        let metadata = input.metadata.unwrap_or_default();
        validate_metadata(&metadata)?;

        let id = WebhookId::new();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO webhook_endpoints
             (id, url, secret, is_active, events, metadata, created_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(&url)
        .bind(&secret)
        .bind(serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_owned()))
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;

        Ok(CreatedWebhookEndpoint {
            endpoint: self.get_endpoint(&id).await?,
            secret,
        })
    }

    /// Lists endpoints in creation order. Secrets are never returned.
    pub async fn list_endpoints(&self) -> Result<Vec<WebhookEndpoint>, AppError> {
        let rows = sqlx::query_as::<_, EndpointRow>(&format!(
            "SELECT {ENDPOINT_COLUMNS} FROM webhook_endpoints ORDER BY created_at ASC, id ASC"
        ))
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(EndpointRow::into_endpoint).collect()
    }

    /// Loads one endpoint.
    pub async fn get_endpoint(&self, id: &WebhookId) -> Result<WebhookEndpoint, AppError> {
        self.fetch_endpoint_row(id).await?.into_endpoint()
    }

    /// Updates URL, subscriptions or activation.
    pub async fn update_endpoint(
        &self,
        id: &WebhookId,
        input: UpdateWebhookEndpointInput,
    ) -> Result<WebhookEndpoint, AppError> {
        let current = self.fetch_endpoint_row(id).await?;
        let url = match input.url {
            Some(url) => validate_url(&url, self.allow_private_networks)?,
            None => current.url.clone(),
        };
        let events = match input.events {
            Some(events) => normalize_events(events)?,
            None => current.events()?,
        };
        let is_active = input
            .is_active
            .map_or(current.is_active != 0, |active| active);

        sqlx::query(
            "UPDATE webhook_endpoints
             SET url = ?, events = ?, is_active = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&url)
        .bind(serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_owned()))
        .bind(i64::from(is_active))
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;

        self.get_endpoint(id).await
    }

    /// Deletes an endpoint and its delivery history.
    pub async fn delete_endpoint(&self, id: &WebhookId) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM webhook_endpoints WHERE id = ?")
            .bind(id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("webhook endpoint", id));
        }
        Ok(())
    }

    /// Rotates the signing secret and returns it exactly once.
    pub async fn rotate_secret(&self, id: &WebhookId) -> Result<String, AppError> {
        let secret = generate_secret();
        let result =
            sqlx::query("UPDATE webhook_endpoints SET secret = ?, updated_at = ? WHERE id = ?")
                .bind(&secret)
                .bind(now_timestamp())
                .bind(id.as_str())
                .execute(self.db.pool())
                .await
                .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("webhook endpoint", id));
        }
        Ok(secret)
    }

    /// Creates one delivery per subscribed active endpoint.
    ///
    /// The unique `(endpoint_id, event_id)` index makes fan-out idempotent, so
    /// an outbox replay can never duplicate a delivery.
    pub async fn fan_out(&self, event: &DomainEventEnvelope) -> Result<u64, AppError> {
        let payload = serde_json::to_string(event)
            .map_err(|error| AppError::internal(format!("event serialization failed: {error}")))?;
        let endpoints = sqlx::query_as::<_, EndpointRow>(&format!(
            "SELECT {ENDPOINT_COLUMNS} FROM webhook_endpoints WHERE is_active = 1"
        ))
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;

        let now = now_timestamp();
        let mut created = 0_u64;
        for endpoint in endpoints {
            let events = endpoint.events()?;
            if !events
                .iter()
                .any(|pattern| pattern == "*" || pattern == &event.event_type)
            {
                continue;
            }
            let id = WebhookDeliveryId::new();
            let result = sqlx::query(
                "INSERT OR IGNORE INTO webhook_deliveries
                 (id, endpoint_id, event_id, event_type, payload, status, attempts,
                  created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, 'pending', 0, ?, ?)",
            )
            .bind(id.as_str())
            .bind(endpoint.id.as_str())
            .bind(event.id.as_str())
            .bind(&event.event_type)
            .bind(&payload)
            .bind(now)
            .bind(now)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
            created += result.rows_affected();
        }
        Ok(created)
    }

    /// Lists deliveries for an endpoint, newest first.
    pub async fn list_deliveries(
        &self,
        endpoint_id: &WebhookId,
        limit: i64,
    ) -> Result<Vec<WebhookDelivery>, AppError> {
        let rows = sqlx::query_as::<_, DeliveryRow>(&format!(
            "SELECT {DELIVERY_COLUMNS} FROM webhook_deliveries
             WHERE endpoint_id = ?
             ORDER BY created_at DESC, id DESC
             LIMIT ?"
        ))
        .bind(endpoint_id.as_str())
        .bind(limit.clamp(1, 100))
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(DeliveryRow::into_delivery).collect()
    }

    /// Loads one delivery.
    pub async fn get_delivery(&self, id: &WebhookDeliveryId) -> Result<WebhookDelivery, AppError> {
        let row = sqlx::query_as::<_, DeliveryRow>(&format!(
            "SELECT {DELIVERY_COLUMNS} FROM webhook_deliveries WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("webhook delivery", id))?;
        row.into_delivery()
    }

    /// Requeues a delivery for another attempt.
    ///
    /// A delivery currently claimed by a worker cannot be replayed concurrently.
    pub async fn replay(&self, id: &WebhookDeliveryId) -> Result<WebhookDelivery, AppError> {
        let result = sqlx::query(
            "UPDATE webhook_deliveries
             SET status = 'pending', attempts = 0, response_status = NULL,
                 response_body = NULL, next_retry_at = NULL, updated_at = ?
             WHERE id = ? AND status != 'delivering'",
        )
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            let existing = self.get_delivery(id).await?;
            if existing.status == DeliveryStatus::Delivering {
                return Err(AppError::conflict(
                    "webhook delivery is currently being processed",
                ));
            }
            return Err(AppError::not_found("webhook delivery", id));
        }
        self.get_delivery(id).await
    }

    /// Claims and sends deliveries that are due, recording every outcome.
    ///
    /// Network calls happen outside any transaction. The claim is a single
    /// atomic `UPDATE ... RETURNING`, so overlapping workers cannot send the
    /// same delivery concurrently.
    pub async fn deliver_due(
        &self,
        now: Timestamp,
        limit: i64,
    ) -> Result<DeliveryReport, AppError> {
        self.recover_stale(now).await?;
        let rows = self.claim_due(now, limit).await?;
        let mut report = DeliveryReport::default();

        for row in rows {
            report.attempted += 1;
            let endpoint = sqlx::query_as::<_, EndpointRow>(&format!(
                "SELECT {ENDPOINT_COLUMNS} FROM webhook_endpoints WHERE id = ?"
            ))
            .bind(&row.endpoint_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
            let Some(endpoint) = endpoint else {
                // The endpoint was deleted after the claim; the delivery row
                // cascades away with it.
                report.skipped += 1;
                continue;
            };
            if endpoint.is_active == 0 {
                self.reschedule_pending(&row.id, now.saturating_add(300))
                    .await?;
                report.skipped += 1;
                continue;
            }

            let outcome = self.send(&row, &endpoint, now).await?;
            match outcome {
                SendOutcome::Delivered => report.delivered += 1,
                SendOutcome::Retried => report.retried += 1,
                SendOutcome::Failed => report.failed += 1,
            }
        }
        Ok(report)
    }

    async fn fetch_endpoint_row(&self, id: &WebhookId) -> Result<EndpointRow, AppError> {
        sqlx::query_as::<_, EndpointRow>(&format!(
            "SELECT {ENDPOINT_COLUMNS} FROM webhook_endpoints WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("webhook endpoint", id))
    }

    /// Returns deliveries claimed by a worker that never recorded an outcome
    /// (for example after a process crash) to the pending queue.
    async fn recover_stale(&self, now: Timestamp) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE webhook_deliveries
             SET status = 'pending', next_retry_at = ?, updated_at = ?
             WHERE status = 'delivering' AND updated_at <= ?",
        )
        .bind(now)
        .bind(now)
        .bind(now.saturating_sub(300))
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }

    async fn claim_due(&self, now: Timestamp, limit: i64) -> Result<Vec<DeliveryRow>, AppError> {
        let query = format!(
            "UPDATE webhook_deliveries
             SET status = 'delivering', attempts = attempts + 1, updated_at = ?1
             WHERE id IN (
                 SELECT d.id FROM webhook_deliveries d
                 JOIN webhook_endpoints e ON e.id = d.endpoint_id
                 WHERE d.status = 'pending'
                   AND (d.next_retry_at IS NULL OR d.next_retry_at <= ?1)
                   AND e.is_active = 1
                 ORDER BY d.created_at ASC, d.id ASC
                 LIMIT ?2
             )
             RETURNING {DELIVERY_COLUMNS}"
        );
        sqlx::query_as::<_, DeliveryRow>(&query)
            .bind(now)
            .bind(limit.clamp(1, 100))
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)
    }

    async fn send(
        &self,
        row: &DeliveryRow,
        endpoint: &EndpointRow,
        now: Timestamp,
    ) -> Result<SendOutcome, AppError> {
        if !self.allow_private_networks {
            ensure_public_resolution(&endpoint.url).await?;
        }
        let timestamp = now_timestamp();
        let signature = sign(&endpoint.secret, timestamp, &row.payload)?;
        let response = self
            .client
            .post(&endpoint.url)
            .header("content-type", "application/json")
            .header("x-fastcart-event", &row.event_type)
            .header("x-fastcart-delivery", &row.id)
            .header("x-fastcart-timestamp", timestamp.to_string())
            .header("x-fastcart-signature", signature)
            .timeout(DELIVERY_TIMEOUT)
            .body(row.payload.clone())
            .send()
            .await;

        let (delivered, response_status, response_body) = match response {
            Ok(response) => {
                let status = i64::from(response.status().as_u16());
                let successful = response.status().is_success();
                let body = read_response_body(response).await;
                (successful, Some(status), Some(body))
            }
            Err(error) => (
                false,
                None,
                Some(truncate_bytes(error.to_string().as_bytes())),
            ),
        };

        let outcome = if delivered {
            sqlx::query(
                "UPDATE webhook_deliveries
                 SET status = 'delivered', response_status = ?, response_body = ?,
                     next_retry_at = NULL, updated_at = ?
                 WHERE id = ?",
            )
            .bind(response_status)
            .bind(&response_body)
            .bind(timestamp)
            .bind(&row.id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
            SendOutcome::Delivered
        } else {
            let retry = row.attempts < i64::from(self.max_attempts);
            let status = if retry {
                DeliveryStatus::Pending
            } else {
                DeliveryStatus::Failed
            };
            let next_retry_at =
                retry.then(|| now.saturating_add(retry_delay_seconds(row.attempts)));
            sqlx::query(
                "UPDATE webhook_deliveries
                 SET status = ?, response_status = ?, response_body = ?,
                     next_retry_at = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(status.as_str())
            .bind(response_status)
            .bind(&response_body)
            .bind(next_retry_at)
            .bind(timestamp)
            .bind(&row.id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
            if retry {
                SendOutcome::Retried
            } else {
                SendOutcome::Failed
            }
        };
        Ok(outcome)
    }

    async fn reschedule_pending(&self, id: &str, next_retry_at: Timestamp) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE webhook_deliveries
             SET status = 'pending', attempts = MAX(attempts - 1, 0),
                 next_retry_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(next_retry_at)
        .bind(now_timestamp())
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SendOutcome {
    Delivered,
    Retried,
    Failed,
}

#[derive(Debug, FromRow)]
struct EndpointRow {
    id: String,
    url: String,
    secret: String,
    is_active: i64,
    events: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl EndpointRow {
    fn events(&self) -> Result<Vec<String>, AppError> {
        serde_json::from_str(&self.events)
            .map_err(|error| AppError::internal(format!("invalid webhook subscription: {error}")))
    }

    fn into_endpoint(self) -> Result<WebhookEndpoint, AppError> {
        let events = self.events()?;
        Ok(WebhookEndpoint {
            id: WebhookId::from_string(self.id),
            url: self.url,
            events,
            is_active: self.is_active != 0,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct DeliveryRow {
    id: String,
    endpoint_id: String,
    event_id: String,
    event_type: String,
    payload: String,
    status: String,
    attempts: i64,
    response_status: Option<i64>,
    response_body: Option<String>,
    next_retry_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

impl DeliveryRow {
    fn into_delivery(self) -> Result<WebhookDelivery, AppError> {
        let status = DeliveryStatus::parse(&self.status).ok_or_else(|| {
            AppError::internal(format!("unknown delivery status: {}", self.status))
        })?;
        let payload: Value = serde_json::from_str(&self.payload)
            .map_err(|error| AppError::internal(format!("invalid delivery payload: {error}")))?;
        Ok(WebhookDelivery {
            id: WebhookDeliveryId::from_string(self.id),
            endpoint_id: WebhookId::from_string(self.endpoint_id),
            event_id: self.event_id,
            event_type: self.event_type,
            payload,
            status,
            attempts: self.attempts,
            response_status: self.response_status,
            response_body: self.response_body,
            next_retry_at: self.next_retry_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn validate_url(raw: &str, allow_private: bool) -> Result<String, AppError> {
    let url = raw.trim();
    if url.is_empty() {
        return Err(AppError::validation("webhook url is required"));
    }
    if url.len() > MAX_URL_LENGTH {
        return Err(AppError::validation(format!(
            "webhook url must not exceed {MAX_URL_LENGTH} bytes"
        )));
    }
    if url
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(AppError::validation(
            "webhook url must not contain whitespace or control characters",
        ));
    }
    let parsed = Url::parse(url)
        .map_err(|error| AppError::validation(format!("invalid webhook url: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::validation(
            "webhook url must use the http or https scheme",
        ));
    }
    let Some(host) = parsed.host_str().filter(|host| !host.is_empty()) else {
        return Err(AppError::validation("webhook url must include a host"));
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::validation(
            "webhook url must not include credentials",
        ));
    }
    if !allow_private {
        if parsed.scheme() != "https" {
            return Err(AppError::validation(
                "webhook url must use https (private destinations are disabled)",
            ));
        }
        validate_public_host(host)?;
    }
    Ok(url.to_owned())
}

fn normalize_events(events: Vec<String>) -> Result<Vec<String>, AppError> {
    if events.is_empty() {
        return Err(AppError::validation(
            "at least one webhook event subscription is required",
        ));
    }
    if events.len() > MAX_EVENTS {
        return Err(AppError::validation(format!(
            "a webhook endpoint supports at most {MAX_EVENTS} event subscriptions"
        )));
    }
    let mut normalized = std::collections::BTreeSet::new();
    for event in events {
        let event = event.trim();
        if event == "*" {
            return Ok(vec!["*".to_owned()]);
        }
        let valid = !event.is_empty()
            && event.len() <= MAX_EVENT_LENGTH
            && event
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if !valid {
            return Err(AppError::validation(format!(
                "invalid webhook event type: {event}"
            )));
        }
        normalized.insert(event.to_owned());
    }
    if normalized.is_empty() {
        return Err(AppError::validation(
            "at least one webhook event subscription is required",
        ));
    }
    Ok(normalized.into_iter().collect())
}

fn validate_secret(secret: &str) -> Result<(), AppError> {
    let valid = (16..=256).contains(&secret.len())
        && !secret
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace());
    if valid {
        Ok(())
    } else {
        Err(AppError::validation(
            "webhook secret must be 16 to 256 printable characters without whitespace",
        ))
    }
}

fn generate_secret() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("whsec_{}", hex::encode(bytes))
}

fn sign(secret: &str, timestamp: Timestamp, payload: &str) -> Result<String, AppError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|error| AppError::internal(format!("invalid webhook secret: {error}")))?;
    mac.update(format!("{timestamp}.{payload}").as_bytes());
    Ok(format!("v1={}", hex::encode(mac.finalize().into_bytes())))
}

fn truncate_bytes(body: &[u8]) -> String {
    let end = body.len().min(MAX_RESPONSE_BODY_BYTES);
    let end = (0..=end)
        .rev()
        .find(|index| {
            body.get(*index)
                .is_none_or(|byte| *byte & 0b1100_0000 != 0b1000_0000)
        })
        .unwrap_or(0);
    String::from_utf8_lossy(&body[..end]).into_owned()
}

async fn read_response_body(response: reqwest::Response) -> String {
    let mut stream = response.bytes_stream();
    let mut body = Vec::with_capacity(MAX_RESPONSE_BODY_BYTES.min(16 * 1024));
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            break;
        };
        let remaining = MAX_RESPONSE_BODY_BYTES.saturating_sub(body.len());
        if remaining == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if body.len() >= MAX_RESPONSE_BODY_BYTES {
            break;
        }
    }
    truncate_bytes(&body)
}

fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || octets[0] == 0
                || octets[0] >= 224
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_forbidden_ip(IpAddr::V4(v4)))
        }
    }
}

fn validate_public_host(host: &str) -> Result<(), AppError> {
    let normalized = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
        || normalized.ends_with(".internal")
        || normalized.ends_with(".home")
        || normalized.ends_with(".lan")
    {
        return Err(AppError::validation(
            "webhook url must not target a local host",
        ));
    }
    if let Ok(ip) = normalized.parse::<IpAddr>()
        && is_forbidden_ip(ip)
    {
        return Err(AppError::validation(
            "webhook url must not target a private or local address",
        ));
    }
    Ok(())
}

async fn ensure_public_resolution(raw: &str) -> Result<(), AppError> {
    let parsed = Url::parse(raw)
        .map_err(|error| AppError::validation(format!("invalid webhook url: {error}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::validation("webhook url must include a host"))?;
    validate_public_host(host)?;
    let port = parsed.port_or_known_default().unwrap_or(443);
    let target = host.trim_matches(['[', ']']).to_owned();
    let addresses = tokio::net::lookup_host((target.as_str(), port))
        .await
        .map_err(|error| {
            AppError::validation(format!("webhook host cannot be resolved: {error}"))
        })?;
    let mut resolved = false;
    for address in addresses {
        resolved = true;
        if is_forbidden_ip(address.ip()) {
            return Err(AppError::validation(
                "webhook host resolves to a private or local address",
            ));
        }
    }
    if !resolved {
        return Err(AppError::validation("webhook host has no addresses"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use axum::serve;
    use fastcart_infrastructure::StoreDatabase;
    use fastcart_kernel::{DomainEventEnvelope, StoreId, now_timestamp};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use tokio::net::TcpListener;

    use super::{CreateWebhookEndpointInput, UpdateWebhookEndpointInput, WebhookService};
    use crate::domain::{DeliveryStatus, retry_delay_seconds};

    #[derive(Clone, Debug)]
    struct Captured {
        headers: HeaderMap,
        body: String,
    }

    #[derive(Clone)]
    struct MockState {
        captured: Arc<Mutex<Vec<Captured>>>,
        status: StatusCode,
    }

    async fn receive(
        State(state): State<MockState>,
        headers: HeaderMap,
        body: String,
    ) -> StatusCode {
        state
            .captured
            .lock()
            .unwrap()
            .push(Captured { headers, body });
        state.status
    }

    async fn spawn_mock(status: StatusCode) -> (String, MockState) {
        let state = MockState {
            captured: Arc::default(),
            status,
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

    async fn service() -> WebhookService {
        WebhookService::new(
            StoreDatabase::in_memory(StoreId::new())
                .await
                .expect("in-memory store database"),
        )
        .with_private_networks_allowed(true)
    }

    async fn endpoint(service: &WebhookService, url: &str, events: Vec<&str>) -> String {
        service
            .create_endpoint(CreateWebhookEndpointInput {
                url: url.to_owned(),
                events: events.into_iter().map(str::to_owned).collect(),
                ..Default::default()
            })
            .await
            .unwrap()
            .secret
    }

    #[tokio::test]
    async fn endpoint_lifecycle_never_leaks_secrets() {
        let service = service().await;
        let created = service
            .create_endpoint(CreateWebhookEndpointInput {
                url: "https://example.com/hooks".to_owned(),
                events: vec!["order.placed".to_owned(), "order.placed".to_owned()],
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(created.secret.starts_with("whsec_"));
        assert_eq!(created.endpoint.events, vec!["order.placed"]);
        assert!(created.endpoint.is_active);

        let listed = service.list_endpoints().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], created.endpoint);

        let updated = service
            .update_endpoint(
                &created.endpoint.id,
                UpdateWebhookEndpointInput {
                    events: Some(vec!["*".to_owned()]),
                    is_active: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.events, vec!["*"]);
        assert!(!updated.is_active);

        let rotated = service.rotate_secret(&created.endpoint.id).await.unwrap();
        assert_ne!(rotated, created.secret);
        assert!(rotated.starts_with("whsec_"));

        service.delete_endpoint(&created.endpoint.id).await.unwrap();
        assert!(service.list_endpoints().await.unwrap().is_empty());
        assert!(
            service
                .get_endpoint(&created.endpoint.id)
                .await
                .unwrap_err()
                .is_not_found()
        );
    }

    #[tokio::test]
    async fn rejects_private_and_plaintext_destinations_by_default() {
        let service = WebhookService::new(StoreDatabase::in_memory(StoreId::new()).await.unwrap());
        for url in [
            "http://example.com/hook",
            "https://127.0.0.1/hook",
            "https://localhost/hook",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/hook",
        ] {
            service
                .create_endpoint(CreateWebhookEndpointInput {
                    url: url.to_owned(),
                    events: vec!["order.placed".to_owned()],
                    ..Default::default()
                })
                .await
                .expect_err("private or plaintext URL was accepted");
        }
    }

    #[tokio::test]
    async fn validates_urls_events_and_secrets() {
        let service = service().await;
        for url in [
            "",
            "ftp://example.com/hook",
            "file:///tmp/hook",
            "https://user:pass@example.com/hook",
            "not a url",
        ] {
            assert!(
                service
                    .create_endpoint(CreateWebhookEndpointInput {
                        url: url.to_owned(),
                        events: vec!["order.placed".to_owned()],
                        ..Default::default()
                    })
                    .await
                    .is_err(),
                "expected {url} to be rejected"
            );
        }
        assert!(
            service
                .create_endpoint(CreateWebhookEndpointInput {
                    url: "https://example.com/hook".to_owned(),
                    events: vec!["Bad Event!".to_owned()],
                    ..Default::default()
                })
                .await
                .is_err()
        );
        assert!(
            service
                .create_endpoint(CreateWebhookEndpointInput {
                    url: "https://example.com/hook".to_owned(),
                    events: vec!["order.placed".to_owned()],
                    secret: Some("short".to_owned()),
                    ..Default::default()
                })
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn fan_out_is_idempotent_and_honours_subscriptions() {
        let service = service().await;
        let matched = service
            .create_endpoint(CreateWebhookEndpointInput {
                url: "https://example.com/orders".to_owned(),
                events: vec!["order.placed".to_owned()],
                ..Default::default()
            })
            .await
            .unwrap();
        let wildcard = service
            .create_endpoint(CreateWebhookEndpointInput {
                url: "https://example.com/all".to_owned(),
                events: vec!["*".to_owned()],
                ..Default::default()
            })
            .await
            .unwrap();
        let unmatched = service
            .create_endpoint(CreateWebhookEndpointInput {
                url: "https://example.com/products".to_owned(),
                events: vec!["product.updated".to_owned()],
                ..Default::default()
            })
            .await
            .unwrap();

        let event = DomainEventEnvelope::new(
            "order.placed",
            Some(StoreId::new()),
            serde_json::json!({"id": "order_1"}),
        );
        assert_eq!(service.fan_out(&event).await.unwrap(), 2);
        assert_eq!(service.fan_out(&event).await.unwrap(), 0);

        assert_eq!(
            service
                .list_deliveries(&matched.endpoint.id, 10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            service
                .list_deliveries(&wildcard.endpoint.id, 10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            service
                .list_deliveries(&unmatched.endpoint.id, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn signs_and_records_successful_deliveries() {
        let (url, mock) = spawn_mock(StatusCode::OK).await;
        let service = service().await;
        let secret = endpoint(&service, &url, vec!["order.placed"]).await;
        let event = DomainEventEnvelope::new(
            "order.placed",
            Some(StoreId::new()),
            serde_json::json!({"id": "order_1"}),
        );
        service.fan_out(&event).await.unwrap();

        let report = service.deliver_due(now_timestamp(), 10).await.unwrap();
        assert_eq!(report.attempted, 1);
        assert_eq!(report.delivered, 1);

        let captured = mock.captured.lock().unwrap().clone();
        assert_eq!(captured.len(), 1);
        let delivery = &captured[0];
        assert_eq!(
            delivery.headers.get("x-fastcart-event").unwrap(),
            "order.placed"
        );
        let timestamp = delivery
            .headers
            .get("x-fastcart-timestamp")
            .unwrap()
            .to_str()
            .unwrap();
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{timestamp}.{}", delivery.body).as_bytes());
        let expected = format!("v1={}", hex::encode(mac.finalize().into_bytes()));
        assert_eq!(
            delivery.headers.get("x-fastcart-signature").unwrap(),
            expected.as_str()
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&delivery.body).unwrap()["id"],
            event.id.as_str()
        );

        let endpoints = service.list_endpoints().await.unwrap();
        let deliveries = service.list_deliveries(&endpoints[0].id, 10).await.unwrap();
        assert_eq!(deliveries[0].status, DeliveryStatus::Delivered);
        assert_eq!(deliveries[0].attempts, 1);
        assert_eq!(deliveries[0].response_status, Some(200));
    }

    #[tokio::test]
    async fn retries_failed_deliveries_then_marks_them_failed() {
        let (url, _mock) = spawn_mock(StatusCode::INTERNAL_SERVER_ERROR).await;
        let service = service().await.with_max_attempts(2);
        endpoint(&service, &url, vec!["order.placed"]).await;
        let event = DomainEventEnvelope::new("order.placed", None, serde_json::json!({}));
        service.fan_out(&event).await.unwrap();

        let now = now_timestamp();
        let first = service.deliver_due(now, 10).await.unwrap();
        assert_eq!(first.retried, 1);
        let endpoints = service.list_endpoints().await.unwrap();
        let pending = &service.list_deliveries(&endpoints[0].id, 10).await.unwrap()[0];
        assert_eq!(pending.status, DeliveryStatus::Pending);
        assert_eq!(pending.attempts, 1);
        assert_eq!(
            pending.next_retry_at,
            Some(now.saturating_add(retry_delay_seconds(1)))
        );

        // Not due yet.
        assert_eq!(
            service
                .deliver_due(now.saturating_add(1), 10)
                .await
                .unwrap()
                .attempted,
            0
        );

        let second = service
            .deliver_due(now.saturating_add(3_600), 10)
            .await
            .unwrap();
        assert_eq!(second.failed, 1);
        let failed = &service.list_deliveries(&endpoints[0].id, 10).await.unwrap()[0];
        assert_eq!(failed.status, DeliveryStatus::Failed);
        assert_eq!(failed.attempts, 2);
        assert_eq!(failed.response_status, Some(500));
    }

    #[tokio::test]
    async fn replay_resets_a_delivery_for_another_attempt() {
        let (url, _mock) = spawn_mock(StatusCode::INTERNAL_SERVER_ERROR).await;
        let service = service().await.with_max_attempts(1);
        endpoint(&service, &url, vec!["order.placed"]).await;
        let event = DomainEventEnvelope::new("order.placed", None, serde_json::json!({}));
        service.fan_out(&event).await.unwrap();
        service.deliver_due(now_timestamp(), 10).await.unwrap();

        let endpoints = service.list_endpoints().await.unwrap();
        let delivery = &service.list_deliveries(&endpoints[0].id, 10).await.unwrap()[0];
        assert_eq!(delivery.status, DeliveryStatus::Failed);

        let replayed = service.replay(&delivery.id).await.unwrap();
        assert_eq!(replayed.status, DeliveryStatus::Pending);
        assert_eq!(replayed.attempts, 0);
        assert!(replayed.next_retry_at.is_none());
    }

    #[tokio::test]
    async fn recovers_deliveries_abandoned_by_crashed_workers() {
        let (url, mock) = spawn_mock(StatusCode::OK).await;
        let service = service().await;
        endpoint(&service, &url, vec!["order.placed"]).await;
        service
            .fan_out(&DomainEventEnvelope::new(
                "order.placed",
                None,
                serde_json::json!({}),
            ))
            .await
            .unwrap();

        let endpoints = service.list_endpoints().await.unwrap();
        let delivery = &service.list_deliveries(&endpoints[0].id, 10).await.unwrap()[0];
        sqlx::query(
            "UPDATE webhook_deliveries SET status = 'delivering', updated_at = ? WHERE id = ?",
        )
        .bind(now_timestamp().saturating_sub(600))
        .bind(delivery.id.as_str())
        .execute(service.db.pool())
        .await
        .unwrap();

        let report = service.deliver_due(now_timestamp(), 10).await.unwrap();
        assert_eq!(report.delivered, 1);
        assert_eq!(mock.captured.lock().unwrap().len(), 1);
        let delivered = &service.list_deliveries(&endpoints[0].id, 10).await.unwrap()[0];
        assert_eq!(delivered.status, DeliveryStatus::Delivered);
    }

    #[tokio::test]
    async fn deleting_an_endpoint_removes_its_deliveries() {
        let service = service().await;
        let created = service
            .create_endpoint(CreateWebhookEndpointInput {
                url: "https://example.com/hook".to_owned(),
                events: vec!["*".to_owned()],
                ..Default::default()
            })
            .await
            .unwrap();
        service
            .fan_out(&DomainEventEnvelope::new(
                "order.placed",
                None,
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(
            service
                .list_deliveries(&created.endpoint.id, 10)
                .await
                .unwrap()
                .len(),
            1
        );

        service.delete_endpoint(&created.endpoint.id).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webhook_deliveries")
            .fetch_one(service.db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn delivery_sweep_does_nothing_for_inactive_endpoints() {
        let (url, mock) = spawn_mock(StatusCode::OK).await;
        let service = service().await;
        let created = service
            .create_endpoint(CreateWebhookEndpointInput {
                url,
                events: vec!["*".to_owned()],
                ..Default::default()
            })
            .await
            .unwrap();
        service
            .update_endpoint(
                &created.endpoint.id,
                UpdateWebhookEndpointInput {
                    is_active: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        service
            .fan_out(&DomainEventEnvelope::new(
                "order.placed",
                None,
                serde_json::json!({}),
            ))
            .await
            .unwrap();

        assert_eq!(
            service
                .deliver_due(now_timestamp(), 10)
                .await
                .unwrap()
                .attempted,
            0
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(mock.captured.lock().unwrap().is_empty());
    }
}
