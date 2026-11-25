use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::serve;
use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
use fastcart_kernel::{DomainEventEnvelope, StoreId, now_timestamp};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::net::TcpListener;

use super::{CreateWebhookEndpointInput, UpdateWebhookEndpointInput, WebhookService};
use crate::cipher::SecretCipher;
use crate::domain::{DeliveryStatus, retry_delay_seconds};

mod deliveries;
mod endpoints;
mod security;
mod support;
