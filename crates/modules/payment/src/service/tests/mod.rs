use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, Metadata, OrderId, PageResult, Pagination, StoreId, now_timestamp,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::PaymentService;
use crate::domain::{
    CreatePaymentSessionInput, ListPaymentsFilter, PaymentStatus, RefundStatus, WebhookStatus,
};
use crate::provider::{
    CaptureRequest, CaptureResult, CreateProviderSession, MANUAL_SIGNATURE_HEADER, ManualProvider,
    PaymentError, PaymentProvider, ProviderSession, RefundRequest, RefundResult, VerifiedWebhook,
};

async fn setup() -> (PaymentService, StoreDatabase) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PaymentService::new(db.clone());
    (service, db)
}

fn manual(secret: Option<&str>) -> Arc<dyn PaymentProvider> {
    Arc::new(ManualProvider::new("manual", secret.map(str::to_owned)))
}

fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

fn headers(secret: &str, body: &[u8]) -> BTreeMap<String, String> {
    BTreeMap::from([(MANUAL_SIGNATURE_HEADER.to_owned(), sign(secret, body))])
}

fn input(provider: &str, amount: i64) -> CreatePaymentSessionInput {
    CreatePaymentSessionInput::new(provider, amount, "usd")
}

async fn create_and_capture(
    service: &PaymentService,
    amount: i64,
) -> (String, crate::domain::Payment) {
    let session = service
        .create_session(input("manual", amount))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();
    (session.id.to_string(), payment)
}

struct TestProvider {
    id: String,
    fail_create: bool,
    fail_capture: bool,
    invalid_capture: bool,
    capture_failures_remaining: AtomicUsize,
    fail_refund: bool,
    invalid_refund: bool,
    create_calls: AtomicUsize,
    capture_calls: AtomicUsize,
    refund_calls: AtomicUsize,
}

impl TestProvider {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            fail_create: false,
            fail_capture: false,
            invalid_capture: false,
            capture_failures_remaining: AtomicUsize::new(0),
            fail_refund: false,
            invalid_refund: false,
            create_calls: AtomicUsize::new(0),
            capture_calls: AtomicUsize::new(0),
            refund_calls: AtomicUsize::new(0),
        }
    }

    fn transient_capture(id: &str) -> Self {
        Self {
            capture_failures_remaining: AtomicUsize::new(1),
            ..Self::new(id)
        }
    }

    fn invalid_capture(id: &str) -> Self {
        Self {
            invalid_capture: true,
            ..Self::new(id)
        }
    }

    fn failing_refund(id: &str) -> Self {
        Self {
            fail_refund: true,
            ..Self::new(id)
        }
    }

    fn invalid_refund(id: &str) -> Self {
        Self {
            invalid_refund: true,
            ..Self::new(id)
        }
    }

    fn failing_create(id: &str) -> Self {
        Self {
            fail_create: true,
            ..Self::new(id)
        }
    }

    fn capture_count(&self) -> usize {
        self.capture_calls.load(Ordering::SeqCst)
    }

    fn refund_count(&self) -> usize {
        self.refund_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl PaymentProvider for TestProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn create_session(
        &self,
        _request: CreateProviderSession,
    ) -> Result<ProviderSession, PaymentError> {
        self.create_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_create {
            return Err(PaymentError::Provider("create failed".to_owned()));
        }
        Ok(ProviderSession {
            provider_session_id: format!("{}_session", self.id),
            status: PaymentStatus::Pending,
            data: serde_json::json!({}),
        })
    }

    async fn capture(&self, request: CaptureRequest) -> Result<CaptureResult, PaymentError> {
        self.capture_calls.fetch_add(1, Ordering::SeqCst);
        if self.invalid_capture {
            return Err(PaymentError::InvalidRequest(
                "capture request is invalid".to_owned(),
            ));
        }
        if self.fail_capture {
            return Err(PaymentError::Provider("capture failed".to_owned()));
        }
        let remaining = self.capture_failures_remaining.load(Ordering::SeqCst);
        if remaining > 0 {
            self.capture_failures_remaining
                .store(remaining - 1, Ordering::SeqCst);
            return Err(PaymentError::Provider("capture transport error".to_owned()));
        }
        Ok(CaptureResult {
            provider_payment_id: format!("{}_payment", self.id),
            amount: request.amount,
            currency_code: request.currency_code,
            status: PaymentStatus::Captured,
        })
    }

    async fn refund(&self, request: RefundRequest) -> Result<RefundResult, PaymentError> {
        self.refund_calls.fetch_add(1, Ordering::SeqCst);
        if self.invalid_refund {
            return Err(PaymentError::InvalidRequest(
                "refund request is invalid".to_owned(),
            ));
        }
        if self.fail_refund {
            return Err(PaymentError::Provider("refund failed".to_owned()));
        }
        Ok(RefundResult {
            provider_refund_id: format!("{}_refund", self.id),
            amount: request.amount,
            currency_code: request.currency_code,
            status: RefundStatus::Succeeded,
        })
    }

    fn verify_webhook(
        &self,
        _headers: &BTreeMap<String, String>,
        _body: &[u8],
    ) -> Result<VerifiedWebhook, PaymentError> {
        Err(PaymentError::InvalidSignature)
    }
}

mod capture;
mod queries;
mod refunds;
mod sessions;
mod webhooks;
