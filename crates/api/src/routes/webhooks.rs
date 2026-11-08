//! Provider webhook callbacks.
//!
//! Webhook routes are public at the network layer, but the store is resolved
//! from the trusted Host header and every provider adapter must verify the
//! signature. Duplicate provider event identifiers are acknowledged without
//! reprocessing.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::HeaderMap;
use axum::middleware;
use axum::routing::post;
use fastcart_http::{ApiResult, ClientIp, StoreResolutionState};
use fastcart_module_payment::{PaymentService, PaymentWebhookRecord};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::{StoreDb, StoreDbState, store_database_middleware};
use crate::state::AppContext;

/// Builds the webhook router.
pub fn router(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    Router::new()
        .route("/payments/{provider}", post(payment_webhook))
        .layer(middleware::from_fn_with_state(
            StoreDbState {
                databases: context.databases.clone(),
            },
            store_database_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            StoreResolutionState {
                resolver: context.resolver.clone(),
            },
            fastcart_http::store_resolution_middleware,
        ))
}

/// Builds the documented provider webhook routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new().routes(routes!(payment_webhook))
}

#[utoipa::path(
    post,
    path = "/payments/{provider}",
    tag = "Webhooks",
    summary = "Receive a payment provider webhook",
    request_body = serde_json::Value,
    responses((status = 200, body = PaymentWebhookRecord))
)]
async fn payment_webhook(
    State(context): State<Arc<AppContext>>,
    StoreDb(database): StoreDb,
    Path(provider): Path<String>,
    client_ip: Option<Extension<ClientIp>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<PaymentWebhookRecord>> {
    let client = client_ip
        .map(|Extension(ip)| ip.as_key())
        .unwrap_or_else(|| "unknown".to_owned());
    if !context.webhook_limiter.check(&client) {
        return Err(fastcart_http::ApiError::new(
            fastcart_kernel::AppError::RateLimited,
        ));
    }
    let header_map: BTreeMap<String, String> = headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_ascii_lowercase(), value.to_owned()))
        })
        .collect();
    let service = PaymentService::with_registry(database, context.payment_providers.clone());
    let record = service
        .process_webhook(&provider, header_map, &body)
        .await?;
    Ok(Json(record))
}
