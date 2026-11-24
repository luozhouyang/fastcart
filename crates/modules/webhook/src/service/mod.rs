//! SQLite-backed webhook registry and delivery worker.

use std::net::IpAddr;
use std::time::Duration;

use fastcart_infrastructure::{AuditActor, StoreDatabase, database_error};
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

use crate::cipher::SecretCipher;
use crate::domain::{
    CreateWebhookEndpointInput, CreatedWebhookEndpoint, DEFAULT_MAX_ATTEMPTS, DeliveryReport,
    DeliveryStatus, MAX_EVENT_LENGTH, MAX_EVENTS, MAX_RESPONSE_BODY_BYTES, MAX_URL_LENGTH,
    UpdateWebhookEndpointInput, WebhookDelivery, WebhookEndpoint, retry_delay_seconds,
};

/// How long the previous signing secret stays valid after a rotation.
pub const SECRET_ROTATION_GRACE_SECONDS: i64 = 86_400;

const ENDPOINT_COLUMNS: &str = "id, url, secret, secret_previous, secret_rotated_at, is_active, \
     events, metadata, created_at, updated_at";
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
    audit: Option<AuditActor>,
    cipher: Option<SecretCipher>,
    secret_rotation_grace_seconds: i64,
}

mod deliveries;
mod endpoints;
mod security;

use security::sign;

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
            audit: None,
            cipher: SecretCipher::from_env(),
            secret_rotation_grace_seconds: SECRET_ROTATION_GRACE_SECONDS,
        }
    }

    /// Overrides the cipher used to protect signing secrets at rest.
    ///
    /// Passing `None` keeps legacy plaintext storage. Tests use this to
    /// provide a fixed key instead of reading the environment.
    #[must_use]
    pub fn with_secret_cipher(mut self, cipher: Option<SecretCipher>) -> Self {
        self.cipher = cipher;
        self
    }

    /// Overrides how long the previous signing secret remains valid after a
    /// rotation.
    #[must_use]
    pub fn with_secret_rotation_grace_seconds(mut self, seconds: i64) -> Self {
        self.secret_rotation_grace_seconds = seconds.max(0);
        self
    }

    /// Records endpoint mutations in the store audit outbox.
    ///
    /// When an actor is set, every mutating endpoint method appends its audit
    /// entry in the same transaction as the change.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

    /// Appends one audit entry to the outbox inside a mutation transaction.
    async fn append_audit(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        data: serde_json::Value,
    ) -> Result<(), AppError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, resource_type, resource_id, data)
                .await
                .map_err(database_error)?;
        }
        Ok(())
    }

    /// Encrypts a plaintext secret when a cipher is configured.
    fn encrypt_secret(&self, plaintext: &str) -> Result<String, AppError> {
        match &self.cipher {
            Some(cipher) => cipher.encrypt(plaintext),
            None => Ok(plaintext.to_owned()),
        }
    }

    /// Decrypts a stored secret, passing legacy plaintext values through.
    fn decrypt_secret(&self, stored: &str) -> Result<String, AppError> {
        match &self.cipher {
            Some(cipher) => cipher.decrypt(stored),
            None if SecretCipher::is_encrypted(stored) => Err(AppError::internal(
                "webhook signing secret is encrypted but no cipher is configured",
            )),
            None => Ok(stored.to_owned()),
        }
    }

    /// Builds the comma-separated signature header for a delivery.
    ///
    /// During the rotation grace window both the current and previous secrets
    /// sign the same timestamp and payload, so receivers accepting any
    /// matching `v1` value continue to verify without downtime.
    fn signatures(
        &self,
        endpoint: &EndpointRow,
        timestamp: Timestamp,
        payload: &str,
    ) -> Result<String, AppError> {
        let current = self.decrypt_secret(&endpoint.secret)?;
        let mut signatures = vec![sign(&current, timestamp, payload)?];
        if let (Some(previous), Some(rotated_at)) =
            (&endpoint.secret_previous, endpoint.secret_rotated_at)
            && rotated_at.saturating_add(self.secret_rotation_grace_seconds) > timestamp
        {
            let previous = self.decrypt_secret(previous)?;
            signatures.push(sign(&previous, timestamp, payload)?);
        }
        Ok(signatures.join(","))
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
    secret_previous: Option<String>,
    secret_rotated_at: Option<i64>,
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

#[cfg(test)]
mod tests;
