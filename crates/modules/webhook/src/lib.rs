//! The `webhook` commerce module.
//!
//! Webhook endpoints, event subscriptions, HMAC-signed deliveries and retry
//! policy. The module is self-contained: workflows fan out outbox events into
//! deliveries and the maintenance sweep dispatches them.

pub mod cipher;
pub mod domain;
mod service;

pub use cipher::SecretCipher;
pub use domain::{
    CreateWebhookEndpointInput, CreatedWebhookEndpoint, DEFAULT_MAX_ATTEMPTS, DeliveryReport,
    DeliveryStatus, MAX_EVENT_LENGTH, MAX_EVENTS, MAX_RESPONSE_BODY_BYTES, MAX_URL_LENGTH,
    UpdateWebhookEndpointInput, WebhookDelivery, WebhookEndpoint, retry_delay_seconds,
};
pub use service::{SECRET_ROTATION_GRACE_SECONDS, WebhookService};
