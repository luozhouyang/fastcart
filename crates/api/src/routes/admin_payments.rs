//! Admin payment HTTP routes.
//!
//! These endpoints expose the provider-independent payment service for the
//! admin namespace. Every handler requires an authenticated store administrator
//! holding the `payments.read` or `payments.write` permission.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use fastcart_http::extractors::ApiQuery;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult, PaginationQuery};
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{ActorContext, Metadata, OrderId};
use fastcart_module_order::{
    OrderService, PaymentStatus as OrderPaymentStatus, RefundStatus as OrderRefundStatus,
};
use fastcart_module_payment::{
    ListPaymentsFilter, Payment, PaymentAttempt, PaymentService, PaymentSession, PaymentStatus,
    PaymentWebhookRecord, Refund,
};

use super::StoreDb;
use crate::dto::PageResponse;
use crate::state::AppContext;

/// Builds the admin payment routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/payments", get(list_payments))
        .route("/payments/{id}", get(get_payment))
        .route(
            "/payments/{id}/refunds",
            get(list_refunds).post(create_refund),
        )
        .route("/payment-sessions/{id}", get(get_payment_session))
        .route("/payment-sessions/{id}/capture", post(capture_session))
        .route("/payment-sessions/{id}/cancel", post(cancel_session))
        .route("/payment-sessions/{id}/fail", post(fail_session))
        .route(
            "/payment-sessions/{id}/attempts",
            get(list_payment_attempts),
        )
        .route("/payment-webhooks", get(list_webhooks))
        .route("/payment-webhooks/{id}", get(get_webhook))
}

/// Filter query accepted by the payment list endpoint.
#[derive(Clone, Debug, serde::Deserialize)]
struct ListPaymentsQuery {
    /// Restricts results to one order.
    order_id: Option<String>,
    /// Restricts results to one payment status.
    status: Option<String>,
    /// Page size.
    limit: Option<i64>,
    /// Records to skip.
    offset: Option<i64>,
}

/// Body for creating a refund.
#[derive(Clone, Debug, serde::Deserialize)]
struct CreateRefundRequest {
    /// Refund amount in minor units.
    amount: i64,
    /// Human-readable refund reason.
    reason: Option<String>,
    /// Optional provider idempotency key.
    idempotency_key: Option<String>,
}

/// Body for failing a payment session.
#[derive(Clone, Debug, Default, serde::Deserialize)]
struct FailSessionRequest {
    /// Failure reason stored on the session and attempt.
    reason: Option<String>,
}

/// HTTP representation of a captured payment.
#[derive(Clone, Debug, serde::Serialize)]
struct PaymentDto {
    id: String,
    order_id: Option<String>,
    provider: String,
    provider_payment_id: Option<String>,
    amount: i64,
    currency_code: String,
    status: String,
    captured_at: Option<i64>,
    metadata: Metadata,
    created_at: i64,
    updated_at: i64,
}

/// HTTP representation of a payment session.
#[derive(Clone, Debug, serde::Serialize)]
struct PaymentSessionDto {
    id: String,
    cart_id: Option<String>,
    checkout_id: Option<String>,
    order_id: Option<String>,
    provider: String,
    provider_session_id: Option<String>,
    amount: i64,
    currency_code: String,
    status: String,
    data: Metadata,
    created_at: i64,
    updated_at: i64,
}

/// HTTP representation of a payment attempt.
#[derive(Clone, Debug, serde::Serialize)]
struct PaymentAttemptDto {
    id: String,
    payment_session_id: String,
    amount: i64,
    currency_code: String,
    status: String,
    provider_reference: Option<String>,
    error: Option<String>,
    created_at: i64,
}

/// HTTP representation of a refund.
#[derive(Clone, Debug, serde::Serialize)]
struct RefundDto {
    id: String,
    payment_id: Option<String>,
    order_id: Option<String>,
    amount: i64,
    currency_code: String,
    status: String,
    provider_refund_id: Option<String>,
    reason: Option<String>,
    created_at: i64,
    updated_at: i64,
}

/// HTTP representation of a persisted webhook event.
#[derive(Clone, Debug, serde::Serialize)]
struct PaymentWebhookDto {
    id: String,
    provider: String,
    provider_event_id: String,
    event_type: String,
    payload: String,
    status: String,
    attempts: i64,
    processed_at: Option<i64>,
    last_error: Option<String>,
    created_at: i64,
}

fn require_permission(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_user().map_err(ApiError::new)?;
    actor.require_permission(permission).map_err(ApiError::new)
}

fn payment_service(context: &AppContext, db: &StoreDatabase) -> PaymentService {
    PaymentService::with_registry(db.clone(), context.payment_providers.clone())
}

async fn list_payments(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<ListPaymentsQuery>,
) -> ApiResult<Json<PageResponse<PaymentDto>>> {
    require_permission(&actor, "payments.read")?;
    let pagination = PaginationQuery {
        limit: query.limit,
        offset: query.offset,
    }
    .pagination()?;
    let mut filter = ListPaymentsFilter::new();
    if let Some(order_id) = query.order_id.as_deref().filter(|value| !value.is_empty()) {
        filter.order_id = Some(OrderId::from_string(order_id));
    }
    if let Some(status) = query.status.as_deref() {
        filter.status =
            Some(PaymentStatus::parse(status).ok_or_else(|| {
                ApiError::validation(format!("unknown payment status: {status}"))
            })?);
    }
    filter.pagination = pagination;

    let service = payment_service(&context, &db);
    let page = service.list_payments(filter).await?;
    Ok(Json(PageResponse::from_page(
        page.map(|payment| payment_dto(&payment)),
    )))
}

async fn get_payment(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<PaymentDto>> {
    require_permission(&actor, "payments.read")?;
    let service = payment_service(&context, &db);
    let payment = service.get_payment(&id).await?;
    Ok(Json(payment_dto(&payment)))
}

async fn list_refunds(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<RefundDto>>> {
    require_permission(&actor, "payments.read")?;
    let service = payment_service(&context, &db);
    let refunds = service.list_refunds(&id).await?;
    Ok(Json(refunds.iter().map(refund_dto).collect()))
}

async fn create_refund(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateRefundRequest>,
) -> ApiResult<(StatusCode, Json<RefundDto>)> {
    require_permission(&actor, "payments.write")?;
    let service = payment_service(&context, &db);
    let refund = match request.idempotency_key {
        Some(idempotency_key) => {
            service
                .refund_with_idempotency(&id, request.amount, request.reason, Some(idempotency_key))
                .await?
        }
        None => service.refund(&id, request.amount, request.reason).await?,
    };
    if let Some(order_id) = refund.order_id.clone() {
        let payment = service.get_payment(&id).await?;
        let (payment_status, refund_status) = match payment.status {
            PaymentStatus::Refunded => (OrderPaymentStatus::Refunded, OrderRefundStatus::Refunded),
            PaymentStatus::PartiallyRefunded => (
                OrderPaymentStatus::PartiallyRefunded,
                OrderRefundStatus::PartiallyRefunded,
            ),
            _ => (OrderPaymentStatus::Captured, OrderRefundStatus::NotRefunded),
        };
        let orders = OrderService::new(db);
        orders.set_payment_status(&order_id, payment_status).await?;
        orders.set_refund_status(&order_id, refund_status).await?;
    }
    Ok((StatusCode::CREATED, Json(refund_dto(&refund))))
}

async fn get_payment_session(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<PaymentSessionDto>> {
    require_permission(&actor, "payments.read")?;
    let service = payment_service(&context, &db);
    let session = service.get_session(&id).await?;
    Ok(Json(payment_session_dto(&session)))
}

async fn capture_session(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<PaymentDto>> {
    require_permission(&actor, "payments.write")?;
    let service = payment_service(&context, &db);
    let payment = service.capture(&id).await?;
    Ok(Json(payment_dto(&payment)))
}

async fn cancel_session(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<PaymentSessionDto>> {
    require_permission(&actor, "payments.write")?;
    let service = payment_service(&context, &db);
    let session = service.cancel_session(&id).await?;
    Ok(Json(payment_session_dto(&session)))
}

async fn fail_session(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<FailSessionRequest>,
) -> ApiResult<Json<PaymentSessionDto>> {
    require_permission(&actor, "payments.write")?;
    let reason = request
        .reason
        .unwrap_or_else(|| "payment session failed".to_owned());
    let service = payment_service(&context, &db);
    let session = service.fail_session(&id, reason).await?;
    Ok(Json(payment_session_dto(&session)))
}

async fn list_payment_attempts(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<PaymentAttemptDto>>> {
    require_permission(&actor, "payments.read")?;
    let service = payment_service(&context, &db);
    let attempts = service.list_attempts(&id).await?;
    Ok(Json(attempts.iter().map(payment_attempt_dto).collect()))
}

async fn list_webhooks(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<PaymentWebhookDto>>> {
    require_permission(&actor, "payments.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let service = payment_service(&context, &db);
    let page = service.list_webhooks(None, pagination).await?;
    Ok(Json(PageResponse::from_page(
        page.map(|webhook| payment_webhook_dto(&webhook)),
    )))
}

async fn get_webhook(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<PaymentWebhookDto>> {
    require_permission(&actor, "payments.read")?;
    let service = payment_service(&context, &db);
    let webhook = service.get_webhook(&id).await?;
    Ok(Json(payment_webhook_dto(&webhook)))
}

fn payment_dto(payment: &Payment) -> PaymentDto {
    PaymentDto {
        id: payment.id.to_string(),
        order_id: payment.order_id.as_ref().map(ToString::to_string),
        provider: payment.provider.clone(),
        provider_payment_id: payment.provider_payment_id.clone(),
        amount: payment.amount,
        currency_code: payment.currency_code.clone(),
        status: payment.status.as_str().to_owned(),
        captured_at: payment.captured_at,
        metadata: payment.metadata.clone(),
        created_at: payment.created_at,
        updated_at: payment.updated_at,
    }
}

fn payment_session_dto(session: &PaymentSession) -> PaymentSessionDto {
    PaymentSessionDto {
        id: session.id.to_string(),
        cart_id: session.cart_id.as_ref().map(ToString::to_string),
        checkout_id: session.checkout_id.as_ref().map(ToString::to_string),
        order_id: session.order_id.as_ref().map(ToString::to_string),
        provider: session.provider.clone(),
        provider_session_id: session.provider_session_id.clone(),
        amount: session.amount,
        currency_code: session.currency_code.clone(),
        status: session.status.as_str().to_owned(),
        data: session.data.clone(),
        created_at: session.created_at,
        updated_at: session.updated_at,
    }
}

fn payment_attempt_dto(attempt: &PaymentAttempt) -> PaymentAttemptDto {
    PaymentAttemptDto {
        id: attempt.id.to_string(),
        payment_session_id: attempt.payment_session_id.to_string(),
        amount: attempt.amount,
        currency_code: attempt.currency_code.clone(),
        status: attempt.status.as_str().to_owned(),
        provider_reference: attempt.provider_reference.clone(),
        error: attempt.error.clone(),
        created_at: attempt.created_at,
    }
}

fn refund_dto(refund: &Refund) -> RefundDto {
    RefundDto {
        id: refund.id.to_string(),
        payment_id: refund.payment_id.as_ref().map(ToString::to_string),
        order_id: refund.order_id.as_ref().map(ToString::to_string),
        amount: refund.amount,
        currency_code: refund.currency_code.clone(),
        status: refund.status.as_str().to_owned(),
        provider_refund_id: refund.provider_refund_id.clone(),
        reason: refund.reason.clone(),
        created_at: refund.created_at,
        updated_at: refund.updated_at,
    }
}

fn payment_webhook_dto(webhook: &PaymentWebhookRecord) -> PaymentWebhookDto {
    PaymentWebhookDto {
        id: webhook.id.to_string(),
        provider: webhook.provider.clone(),
        provider_event_id: webhook.provider_event_id.clone(),
        event_type: webhook.event_type.clone(),
        payload: webhook.payload.clone(),
        status: webhook.status.as_str().to_owned(),
        attempts: webhook.attempts,
        processed_at: webhook.processed_at,
        last_error: webhook.last_error.clone(),
        created_at: webhook.created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        payment_attempt_dto, payment_dto, payment_session_dto, payment_webhook_dto, refund_dto,
    };
    use fastcart_kernel::{
        Metadata, OrderId, PaymentAttemptId, PaymentId, PaymentSessionId, RefundId, WebhookId,
    };
    use fastcart_module_payment::{
        Payment, PaymentAttempt, PaymentSession, PaymentStatus, PaymentWebhookRecord, Refund,
        RefundStatus, WebhookStatus,
    };

    #[test]
    fn maps_payment_entities_to_dtos() {
        let payment = Payment {
            id: PaymentId::new(),
            order_id: Some(OrderId::new()),
            provider: "manual".to_owned(),
            provider_payment_id: Some("manual_pay_1".to_owned()),
            amount: 1500,
            currency_code: "USD".to_owned(),
            status: PaymentStatus::Captured,
            captured_at: Some(10),
            metadata: Metadata::empty(),
            created_at: 10,
            updated_at: 11,
        };
        let dto = payment_dto(&payment);
        assert_eq!(dto.id, payment.id.to_string());
        assert_eq!(
            dto.order_id,
            payment.order_id.as_ref().map(ToString::to_string)
        );
        assert_eq!(dto.status, "captured");
        assert_eq!(dto.amount, 1500);

        let session = PaymentSession {
            id: PaymentSessionId::new(),
            cart_id: None,
            checkout_id: None,
            order_id: payment.order_id.clone(),
            provider: "manual".to_owned(),
            provider_session_id: Some("manual_session_1".to_owned()),
            amount: 1500,
            currency_code: "USD".to_owned(),
            status: PaymentStatus::Pending,
            data: Metadata::empty(),
            created_at: 10,
            updated_at: 10,
        };
        let dto = payment_session_dto(&session);
        assert_eq!(dto.id, session.id.to_string());
        assert_eq!(dto.status, "pending");
        assert_eq!(dto.provider_session_id.as_deref(), Some("manual_session_1"));

        let attempt = PaymentAttempt {
            id: PaymentAttemptId::new(),
            payment_session_id: session.id.clone(),
            amount: 1500,
            currency_code: "USD".to_owned(),
            status: PaymentStatus::Captured,
            provider_reference: Some("manual_pay_1".to_owned()),
            error: None,
            created_at: 11,
        };
        let dto = payment_attempt_dto(&attempt);
        assert_eq!(dto.payment_session_id, session.id.to_string());
        assert_eq!(dto.status, "captured");
    }

    #[test]
    fn maps_refunds_and_webhooks_to_dtos() {
        let refund = Refund {
            id: RefundId::new(),
            payment_id: Some(PaymentId::new()),
            order_id: Some(OrderId::new()),
            amount: 400,
            currency_code: "USD".to_owned(),
            status: RefundStatus::Succeeded,
            provider_refund_id: Some("manual_refund_1".to_owned()),
            idempotency_key: None,
            reason: Some("damaged".to_owned()),
            created_at: 20,
            updated_at: 20,
        };
        let dto = refund_dto(&refund);
        assert_eq!(dto.id, refund.id.to_string());
        assert_eq!(dto.status, "succeeded");
        assert_eq!(dto.reason.as_deref(), Some("damaged"));

        let webhook = PaymentWebhookRecord {
            id: WebhookId::new(),
            provider: "manual".to_owned(),
            provider_event_id: "evt_1".to_owned(),
            event_type: "payment.captured".to_owned(),
            payload: "{}".to_owned(),
            status: WebhookStatus::Processed,
            attempts: 1,
            processed_at: Some(30),
            last_error: None,
            created_at: 30,
        };
        let dto = payment_webhook_dto(&webhook);
        assert_eq!(dto.id, webhook.id.to_string());
        assert_eq!(dto.status, "processed");
        assert_eq!(dto.attempts, 1);
    }
}
