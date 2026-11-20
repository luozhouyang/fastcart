//! Storefront payment collection routes.
//!
//! A checkout's payment collection is the money the checkout owes, paid by one
//! or more payment sessions. These routes make that collection usable from the
//! storefront: read what a checkout still holds, and split its payment onto a
//! second session, which is the entry point combination payments need.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use fastcart_http::{ApiError, ApiJson, ApiResult, ClientIp, StoreCtx};
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{ActorContext, AppError, CheckoutId, CurrencyCode, Metadata, StoreContext};
use fastcart_module_checkout::{CheckoutService, CheckoutStatus};
use fastcart_module_payment::{CreatePaymentSessionInput, PaymentCollection, PaymentService};
use fastcart_workflows::CartWorkflow;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::{StoreDb, ensure_cart_access};
use crate::state::AppContext;

/// Builds storefront payment collection routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route(
            "/checkouts/{checkout_id}/payment-collection",
            get(get_payment_collection),
        )
        .route(
            "/checkouts/{checkout_id}/payment-collection/sessions",
            post(split_payment_collection),
        )
}

/// Builds the documented storefront payment collection routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(get_payment_collection))
        .routes(routes!(split_payment_collection))
}

/// A checkout's payment collection together with the sessions that pay it.
#[derive(Clone, Debug, serde::Serialize, utoipa::ToSchema)]
pub struct PaymentCollectionResponse {
    /// The collection, with the status derived from the amounts actually
    /// captured.
    pub payment_collection: PaymentCollection,
    /// Payment sessions attached to the collection, oldest first.
    pub payment_sessions: Vec<fastcart_module_payment::PaymentSession>,
}

/// The payment session a split created.
#[derive(Clone, Debug, serde::Serialize, utoipa::ToSchema)]
pub struct PaymentSessionResponse {
    /// The session just created and attached to the collection.
    pub payment_session: fastcart_module_payment::PaymentSession,
}

/// Body of a split payment request.
#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct SplitPaymentRequest {
    /// Registered payment provider the new session is created with.
    pub provider: String,
    /// Amount in minor units to move onto the new session. It comes out of the
    /// session the checkout was created with, so the collection's sessions keep
    /// covering its amount exactly.
    pub amount: i64,
    /// Currency of the collection; when sent it must match, because a session
    /// may only pay in the currency it is priced in.
    pub currency_code: Option<String>,
}

/// Stable rate-limit key for a public storefront write.
///
/// Direct router tests have no socket peer; they share the `unknown` bucket
/// rather than trusting request headers.
fn client_key(client_ip: Option<&Extension<ClientIp>>) -> String {
    client_ip
        .map(|Extension(ip)| ip.as_key())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn payment_service(
    context: &AppContext,
    store: &StoreContext,
    db: &StoreDatabase,
) -> PaymentService {
    PaymentService::with_registry(db.clone(), context.payment_providers_for(&store.store_id))
}

#[utoipa::path(
    get,
    path = "/checkouts/{checkout_id}/payment-collection",
    tag = "Store",
    summary = "Get the payment collection of a checkout",
    security(("customerSession" = []), ()),
    responses((status = 200, body = PaymentCollectionResponse))
)]
async fn get_payment_collection(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreCtx(store): StoreCtx,
    StoreDb(db): StoreDb,
    Path(checkout_id): Path<String>,
) -> ApiResult<Json<PaymentCollectionResponse>> {
    let checkout_id = CheckoutId::from_string(checkout_id);
    let checkout = CheckoutService::new(db.clone()).get(&checkout_id).await?;
    let cart = CartWorkflow::new(db.clone())
        .get_cart(&checkout.cart_id)
        .await?;
    ensure_cart_access(&cart, &actor)?;

    let payments = payment_service(&context, &store, &db);
    let collection = payments
        .collection_for_checkout(checkout_id.as_str())
        .await?
        .ok_or_else(|| {
            ApiError::new(AppError::not_found(
                "payment collection",
                checkout_id.as_str(),
            ))
        })?;
    let payment_sessions = payments
        .list_collection_sessions(collection.id.as_str())
        .await?;
    // The stored status is a cache of the captured amounts; report the live
    // value so a storefront never reads "open" for a collected payment.
    let status = payments.collection_status(collection.id.as_str()).await?;
    Ok(Json(PaymentCollectionResponse {
        payment_collection: PaymentCollection {
            status,
            ..collection
        },
        payment_sessions,
    }))
}

#[utoipa::path(
    post,
    path = "/checkouts/{checkout_id}/payment-collection/sessions",
    tag = "Store",
    summary = "Split a checkout payment onto another payment session",
    request_body = SplitPaymentRequest,
    security(("customerSession" = []), ()),
    responses((status = 201, body = PaymentSessionResponse))
)]
async fn split_payment_collection(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    client_ip: Option<Extension<ClientIp>>,
    StoreCtx(store): StoreCtx,
    StoreDb(db): StoreDb,
    Path(checkout_id): Path<String>,
    ApiJson(request): ApiJson<SplitPaymentRequest>,
) -> ApiResult<(StatusCode, Json<PaymentSessionResponse>)> {
    if !context
        .checkout_limiter
        .check(&client_key(client_ip.as_ref()))
    {
        return Err(ApiError::new(AppError::RateLimited));
    }
    let checkout_id = CheckoutId::from_string(checkout_id);
    let checkout = CheckoutService::new(db.clone()).get(&checkout_id).await?;
    let cart = CartWorkflow::new(db.clone())
        .get_cart(&checkout.cart_id)
        .await?;
    ensure_cart_access(&cart, &actor)?;

    // A payment may only be split while the checkout is paying: completion
    // accepts the same two states, and anything else is already settled.
    if !matches!(
        checkout.status,
        CheckoutStatus::PaymentPending | CheckoutStatus::PaymentConfirmed
    ) {
        return Err(ApiError::new(AppError::conflict(format!(
            "cannot split the payment of a checkout in status {}",
            checkout.status.as_str()
        ))));
    }
    let primary_session_id = checkout.payment_session_id.clone().ok_or_else(|| {
        ApiError::new(AppError::conflict(
            "checkout has no payment session to split",
        ))
    })?;

    let payments = payment_service(&context, &store, &db);
    let collection = payments
        .collection_for_checkout(checkout_id.as_str())
        .await?
        .ok_or_else(|| {
            ApiError::new(AppError::not_found(
                "payment collection",
                checkout_id.as_str(),
            ))
        })?;
    // The collection owns the currency; a caller may echo it, but never change
    // it, because a session in another currency could not pay this collection.
    let currency_code = match request.currency_code.as_deref() {
        Some(requested) => {
            let requested = CurrencyCode::new(requested)
                .map_err(|error| ApiError::new(AppError::from(error)))?;
            if requested.as_str() != collection.currency_code {
                return Err(ApiError::validation(format!(
                    "payment collection is priced in {}, not {}",
                    collection.currency_code,
                    requested.as_str()
                )));
            }
            collection.currency_code.clone()
        }
        None => collection.currency_code.clone(),
    };

    let payment_session = payments
        .split_collection_session(
            collection.id.as_str(),
            primary_session_id.as_str(),
            CreatePaymentSessionInput {
                cart_id: Some(checkout.cart_id.clone()),
                checkout_id: Some(checkout.id.clone()),
                order_id: None,
                provider: request.provider,
                amount: request.amount,
                currency_code,
                metadata: Metadata::empty(),
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(PaymentSessionResponse { payment_session }),
    ))
}
