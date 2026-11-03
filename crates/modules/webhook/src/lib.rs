//! The `webhook` commerce module.
//!
//! Webhook endpoints, event subscriptions, HMAC-signed deliveries and retry
//! policy. The module is self-contained: workflows fan out outbox events into
//! deliveries and the maintenance sweep dispatches them.

pub mod domain;
mod service;

pub use domain::{
    CreateWebhookEndpointInput, CreatedWebhookEndpoint, DEFAULT_MAX_ATTEMPTS, DeliveryReport,
    DeliveryStatus, MAX_EVENT_LENGTH, MAX_EVENTS, MAX_RESPONSE_BODY_BYTES, MAX_URL_LENGTH,
    UpdateWebhookEndpointInput, WebhookDelivery, WebhookEndpoint, retry_delay_seconds,
};
pub use service::WebhookService;
