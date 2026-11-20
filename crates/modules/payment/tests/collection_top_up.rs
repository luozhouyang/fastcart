//! A collection can be topped up when the order owes more than it did.
//!
//! An order holds the collection its checkout opened. Money raised after
//! placement — the price difference of a dearer replacement, say — is money the
//! order owes on top of that, so the collection's amount grows and a session
//! pays the increase. Capture still insists the captured total covers the
//! amount, so a top-up that nobody paid leaves the collection short rather than
//! looking settled.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, StoreId};
use fastcart_module_payment::{
    CreatePaymentSessionInput, ManualProvider, PaymentCollectionStatus, PaymentService,
    PaymentStatus, ProviderRegistry,
};

async fn service() -> PaymentService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    PaymentService::with_registry(db, providers)
}

/// Opens a collection and pays it with one captured session.
async fn paid_collection(payments: &PaymentService, amount: i64) -> String {
    let collection = payments
        .create_collection("chk_top_up", amount, "USD")
        .await
        .unwrap();
    let session = payments
        .create_session(CreatePaymentSessionInput {
            checkout_id: Some(fastcart_kernel::CheckoutId::from_string("chk_top_up")),
            provider: "manual".to_owned(),
            amount,
            currency_code: "USD".to_owned(),
            ..CreatePaymentSessionInput::default()
        })
        .await
        .unwrap();
    payments
        .attach_session_to_collection(collection.id.as_str(), session.id.as_str())
        .await
        .unwrap();
    payments.capture(session.id.as_str()).await.unwrap();
    collection.id.to_string()
}

#[tokio::test]
async fn a_top_up_is_paid_before_the_collection_counts_as_settled() {
    let payments = service().await;
    let collection_id = paid_collection(&payments, 1_000).await;
    assert_eq!(
        payments.collection_status(&collection_id).await.unwrap(),
        PaymentCollectionStatus::Captured
    );

    // The order owes 400 more.
    let raised = payments
        .raise_collection_amount(&collection_id, 400)
        .await
        .unwrap();
    assert_eq!(raised.amount, 1_400);
    assert_eq!(
        payments.collection_status(&collection_id).await.unwrap(),
        PaymentCollectionStatus::PartiallyCaptured,
        "the increase is not paid yet, so the collection is no longer settled"
    );

    let session = payments
        .create_session(CreatePaymentSessionInput {
            checkout_id: Some(fastcart_kernel::CheckoutId::from_string("chk_top_up")),
            provider: "manual".to_owned(),
            amount: 400,
            currency_code: "USD".to_owned(),
            ..CreatePaymentSessionInput::default()
        })
        .await
        .unwrap();
    payments
        .attach_session_to_collection(&collection_id, session.id.as_str())
        .await
        .unwrap();
    let captured = payments.capture_collection(&collection_id).await.unwrap();

    assert_eq!(
        captured.iter().map(|payment| payment.amount).sum::<i64>(),
        1_400,
        "both the original payment and the top-up are captured"
    );
    assert_eq!(
        payments.collection_status(&collection_id).await.unwrap(),
        PaymentCollectionStatus::Captured
    );
    assert!(
        captured
            .iter()
            .all(|payment| payment.status == PaymentStatus::Captured),
        "{captured:?}"
    );
}

#[tokio::test]
async fn a_top_up_must_be_positive_and_on_a_known_collection() {
    let payments = service().await;
    let collection_id = paid_collection(&payments, 1_000).await;

    for amount in [0, -100] {
        let error = payments
            .raise_collection_amount(&collection_id, amount)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AppError::Validation(_)),
            "unexpected error for {amount}: {error:?}"
        );
    }
    assert_eq!(
        payments
            .get_collection(&collection_id)
            .await
            .unwrap()
            .amount,
        1_000,
        "a rejected top-up leaves the amount alone"
    );

    let error = payments
        .raise_collection_amount("pcoll_missing", 100)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AppError::NotFound(_)),
        "unexpected error: {error:?}"
    );
}
