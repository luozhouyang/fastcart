use super::*;

#[tokio::test]
async fn full_flow_captures_and_refunds() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();

    let (session_id, payment) = create_and_capture(&service, 1000).await;
    assert_eq!(payment.status, PaymentStatus::Captured);
    assert_eq!(payment.amount, 1000);
    assert!(payment.provider_payment_id.is_some());
    assert!(payment.captured_at.is_some());

    let partial = service
        .refund(payment.id.as_str(), 400, Some("damaged".to_owned()))
        .await
        .unwrap();
    assert_eq!(partial.status, RefundStatus::Succeeded);
    assert_eq!(partial.amount, 400);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::PartiallyRefunded
    );

    let remainder = service
        .refund(payment.id.as_str(), 600, None)
        .await
        .unwrap();
    assert_eq!(remainder.amount, 600);
    let refunded = service.get_payment(payment.id.as_str()).await.unwrap();
    assert_eq!(refunded.status, PaymentStatus::Refunded);
    assert_eq!(
        service
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()
            .len(),
        2
    );

    let captured_again = service.capture(&session_id).await.unwrap();
    assert_eq!(captured_again.id, payment.id);
    assert_eq!(captured_again.status, PaymentStatus::Refunded);
}

#[tokio::test]
async fn capture_is_idempotent_and_does_not_repeat_provider_call() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("counter"));
    service.register_provider(provider.clone()).unwrap();

    let session = service
        .create_session(input("counter", 1500))
        .await
        .unwrap();
    let first = service.capture(session.id.as_str()).await.unwrap();
    let second = service.capture(session.id.as_str()).await.unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(provider.capture_count(), 1);
}

#[tokio::test]
async fn provider_failures_surface_as_external_service_errors() {
    let (service, db) = setup().await;
    service
        .register_provider(Arc::new(TestProvider::failing_create("broken")))
        .unwrap();
    let error = service
        .create_session(input("broken", 100))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::ExternalService(_)));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payment_sessions")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn concurrent_captures_never_call_the_provider_twice() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("manual"));
    service.register_provider(provider.clone()).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let service = Arc::new(service);
    let first = service.clone();
    let second = service.clone();
    let id = session.id.to_string();
    let (left, right) = tokio::join!(first.capture(&id), second.capture(&id));
    assert!(left.is_ok() || right.is_ok());
    assert_eq!(provider.capture_count(), 1);
    let payments = service
        .list_payments(ListPaymentsFilter {
            order_id: None,
            status: None,
            pagination: Pagination::default(),
        })
        .await
        .unwrap();
    assert_eq!(payments.count, 1);
}

#[tokio::test]
async fn ambiguous_capture_failure_stays_retryable_and_retries_provider() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::transient_capture("flaky"));
    service.register_provider(provider.clone()).unwrap();
    let session = service.create_session(input("flaky", 1000)).await.unwrap();

    let error = service.capture(session.id.as_str()).await.unwrap_err();
    assert!(matches!(error, AppError::ExternalService(_)));
    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Pending);
    assert!(stored.data.get("capture_claim").is_none());
    let attempts = service.list_attempts(session.id.as_str()).await.unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[1].status, PaymentStatus::Failed);
    assert!(attempts[1].error.is_some());

    let payment = service.capture(session.id.as_str()).await.unwrap();
    assert_eq!(payment.status, PaymentStatus::Captured);
    assert_eq!(provider.capture_count(), 2);
    let payments = service
        .list_payments(ListPaymentsFilter {
            order_id: None,
            status: None,
            pagination: Pagination::default(),
        })
        .await
        .unwrap();
    assert_eq!(payments.count, 1);
}

#[tokio::test]
async fn invalid_capture_request_marks_session_and_attempt_failed() {
    let (service, _db) = setup().await;
    service
        .register_provider(Arc::new(TestProvider::invalid_capture("broken")))
        .unwrap();
    let session = service.create_session(input("broken", 1000)).await.unwrap();

    let error = service.capture(session.id.as_str()).await.unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Failed);
    let attempts = service.list_attempts(session.id.as_str()).await.unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[1].status, PaymentStatus::Failed);
    assert!(attempts[1].error.is_some());
}

#[tokio::test]
async fn payment_metadata_is_copied_from_the_session() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let mut metadata = Metadata::empty();
    metadata.insert("source", serde_json::json!("storefront"));
    let mut session_input = input("manual", 990);
    session_input.metadata = metadata;
    let session = service.create_session(session_input).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    assert_eq!(
        payment.metadata.get("source"),
        Some(&serde_json::json!("storefront"))
    );
}
