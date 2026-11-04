//! The `payment` commerce module.
//!
//! Provider-independent payment sessions, captures, refunds and idempotent
//! webhook processing persist in the per-store SQLite database. Cross-module
//! orchestration belongs in `fastcart-workflows`.

pub mod domain;
pub mod provider;
pub mod service;
pub mod stripe;

pub use domain::{
    CreatePaymentSessionInput, ListPaymentsFilter, Payment, PaymentAttempt, PaymentSession,
    PaymentStatus, PaymentWebhookRecord, Refund, RefundStatus, WebhookStatus,
};
pub use provider::{
    CaptureRequest, CaptureResult, CreateProviderSession, MANUAL_SIGNATURE_HEADER, ManualProvider,
    PaymentError, PaymentProvider, ProviderRegistry, ProviderSession, RefundRequest, RefundResult,
    VerifiedWebhook,
};
pub use service::PaymentService;
pub use stripe::{StripeProvider, map_payment_intent_status, signature_header};
