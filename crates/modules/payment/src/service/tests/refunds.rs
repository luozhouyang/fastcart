use super::*;

#[tokio::test]
async fn rejects_over_refund_before_calling_provider() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("limited"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("limited", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    service
        .refund(payment.id.as_str(), 700, None)
        .await
        .unwrap();
    let over = service
        .refund(payment.id.as_str(), 400, None)
        .await
        .unwrap_err();
    assert!(matches!(over, AppError::Conflict(_)));
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::PartiallyRefunded
    );

    let zero = service
        .refund(payment.id.as_str(), 0, None)
        .await
        .unwrap_err();
    assert!(matches!(zero, AppError::Validation(_)));
}

#[tokio::test]
async fn rejects_refund_before_capture() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();

    let error = service
        .refund(session.id.as_str(), 100, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));
}

#[tokio::test]
async fn definitive_refund_rejection_marks_the_refund_failed() {
    let (service, _db) = setup().await;
    service
        .register_provider(Arc::new(TestProvider::invalid_refund("broken")))
        .unwrap();
    let session = service.create_session(input("broken", 1000)).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let error = service
        .refund(payment.id.as_str(), 100, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Failed);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Captured
    );
}

#[tokio::test]
async fn ambiguous_refund_failure_stays_pending_and_consumes_the_cap() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::failing_refund("unknown"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("unknown", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let error = service
        .refund(payment.id.as_str(), 700, Some("first".to_owned()))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::ExternalService(_)));
    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Pending);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Captured
    );

    // 400 more would fit only if the unknown 700 were excluded from the
    // cap. Pending refunds count, so this must be rejected.
    let over = service
        .refund(payment.id.as_str(), 400, Some("second".to_owned()))
        .await
        .unwrap_err();
    assert!(matches!(over, AppError::Conflict(_)));
    assert_eq!(provider.refund_count(), 1);
    assert_eq!(
        service
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn duplicate_refund_key_returns_the_existing_refund() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("dedupe"));
    service.register_provider(provider.clone()).unwrap();
    let session = service.create_session(input("dedupe", 1000)).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let first = service
        .refund_with_idempotency(
            payment.id.as_str(),
            400,
            None,
            Some("return-refund:dedupe".to_owned()),
        )
        .await
        .unwrap();
    let second = service
        .refund_with_idempotency(
            payment.id.as_str(),
            400,
            None,
            Some("return-refund:dedupe".to_owned()),
        )
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(second.status, RefundStatus::Succeeded);
    assert_eq!(provider.refund_count(), 1);
    assert_eq!(
        service
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn keyless_refund_retry_reuses_the_same_refund() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("keyless"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("keyless", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let first = service
        .refund(payment.id.as_str(), 400, Some("damaged".to_owned()))
        .await
        .unwrap();
    // Only surrounding whitespace differs: the normalized reason must map
    // to the same deterministic idempotency key.
    let second = service
        .refund(payment.id.as_str(), 400, Some(" damaged ".to_owned()))
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(second.status, RefundStatus::Succeeded);
    assert_eq!(provider.refund_count(), 1);
    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].id, first.id);
}

#[tokio::test]
async fn omitted_refund_key_is_derived_and_idempotent() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("derived"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("derived", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let first = service
        .refund_with_idempotency(payment.id.as_str(), 300, Some("goodwill".to_owned()), None)
        .await
        .unwrap();
    let second = service
        .refund_with_idempotency(
            payment.id.as_str(),
            300,
            Some(" goodwill ".to_owned()),
            None,
        )
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(provider.refund_count(), 1);
    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
}

#[tokio::test]
async fn duplicate_full_refund_key_returns_existing_after_payment_is_refunded() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("dedupe-full"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("dedupe-full", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let first = service
        .refund_with_idempotency(
            payment.id.as_str(),
            1000,
            None,
            Some("return-refund:full".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Refunded
    );
    let second = service
        .refund_with_idempotency(
            payment.id.as_str(),
            1000,
            None,
            Some("return-refund:full".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(provider.refund_count(), 1);
}

#[tokio::test]
async fn concurrent_duplicate_refund_keys_return_the_winner() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("dedupe-concurrent"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("dedupe-concurrent", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();
    let service = Arc::new(service);

    let first = service.clone();
    let second = service.clone();
    let payment_id = payment.id.to_string();
    let (left, right) = tokio::join!(
        first.refund_with_idempotency(
            &payment_id,
            400,
            None,
            Some("return-refund:race".to_owned())
        ),
        second.refund_with_idempotency(
            &payment_id,
            400,
            None,
            Some("return-refund:race".to_owned())
        )
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.id, right.id);
    assert_eq!(
        service
            .list_refunds(payment_id.as_str())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn pending_refund_crash_retry_calls_the_provider_and_reconciles() {
    let (service, db) = setup().await;
    let provider = Arc::new(TestProvider::new("crash"));
    service.register_provider(provider.clone()).unwrap();
    let session = service.create_session(input("crash", 1000)).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    // Simulate a crash after the pending refund row was committed but
    // before the provider call returned.
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO refunds \
             (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
              idempotency_key, reason, created_at, updated_at) \
             VALUES ('refund_crash', ?, NULL, 400, 'USD', 'pending', NULL, \
                     'return-refund:crash', NULL, ?, ?)",
    )
    .bind(payment.id.as_str())
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();

    let refund = service
        .refund_with_idempotency(
            payment.id.as_str(),
            400,
            None,
            Some("return-refund:crash".to_owned()),
        )
        .await
        .unwrap();

    assert_eq!(refund.id.as_str(), "refund_crash");
    assert_eq!(refund.status, RefundStatus::Succeeded);
    assert_eq!(provider.refund_count(), 1);
    assert_eq!(
        service
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::PartiallyRefunded
    );
}
