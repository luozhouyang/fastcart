use super::*;

#[tokio::test]
async fn creates_session_and_attempt() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let mut input = input("manual", 2500);
    input.metadata.insert("checkout", serde_json::json!("v1"));
    input.cart_id = Some(fastcart_kernel::CartId::new());

    let session = service.create_session(input).await.unwrap();
    assert_eq!(session.amount, 2500);
    assert_eq!(session.currency_code, "USD");
    assert_eq!(session.status, PaymentStatus::Pending);
    assert!(session.provider_session_id.is_some());
    assert_eq!(session.data.get("checkout"), Some(&serde_json::json!("v1")));

    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored, session);
    let attempts = service.list_attempts(session.id.as_str()).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, PaymentStatus::Pending);
    assert_eq!(attempts[0].amount, 2500);
}

#[tokio::test]
async fn rejects_unknown_provider_negative_amount_and_bad_currency() {
    let (service, _db) = setup().await;

    let missing = service
        .create_session(input("missing", 100))
        .await
        .unwrap_err();
    assert!(matches!(missing, AppError::NotFound(_)));

    service.register_provider(manual(Some("secret"))).unwrap();
    let negative = service
        .create_session(input("manual", -1))
        .await
        .unwrap_err();
    assert!(matches!(negative, AppError::Validation(_)));

    let bad_currency = service
        .create_session(CreatePaymentSessionInput::new("manual", 100, "us"))
        .await
        .unwrap_err();
    assert!(matches!(bad_currency, AppError::Validation(_)));
}

#[tokio::test]
async fn cancel_cannot_overwrite_a_session_captured_after_the_read() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();

    // Simulate cancel observing `pending`, then a capture completing
    // before the stale cancel write is issued.
    let stale = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stale.status, PaymentStatus::Pending);
    let payment = service.capture(session.id.as_str()).await.unwrap();
    assert_eq!(payment.status, PaymentStatus::Captured);

    let error = service
        .transition_session(&stale, PaymentStatus::Canceled, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Captured);
}

#[tokio::test]
async fn cancel_conflicts_while_a_capture_claim_is_live() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let now = now_timestamp();
    // Simulate capture() having claimed the session but not yet completed
    // its provider call. The status is still `pending`.
    sqlx::query(
        "UPDATE payment_sessions
             SET data = json_set(data, '$.capture_claim', 'capture:live',
                                 '$.capture_claim_at', ?),
                 updated_at = ?
             WHERE id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(session.id.as_str())
    .execute(db.pool())
    .await
    .unwrap();

    let error = service
        .cancel_session(session.id.as_str())
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Pending);
    assert!(stored.data.get("capture_claim").is_some());
}

#[tokio::test]
async fn fail_and_cancel_respect_the_state_machine() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();

    let canceled_session = service.create_session(input("manual", 100)).await.unwrap();
    let canceled = service
        .cancel_session(canceled_session.id.as_str())
        .await
        .unwrap();
    assert_eq!(canceled.status, PaymentStatus::Canceled);
    let capture_error = service
        .capture(canceled_session.id.as_str())
        .await
        .unwrap_err();
    assert!(matches!(capture_error, AppError::Conflict(_)));

    let failed_session = service.create_session(input("manual", 100)).await.unwrap();
    let failed = service
        .fail_session(failed_session.id.as_str(), "declined")
        .await
        .unwrap();
    assert_eq!(failed.status, PaymentStatus::Failed);
    assert_eq!(
        failed.data.get("last_error"),
        Some(&serde_json::json!("declined"))
    );

    let captured_session = service.create_session(input("manual", 100)).await.unwrap();
    service.capture(captured_session.id.as_str()).await.unwrap();
    let cancel_error = service
        .cancel_session(captured_session.id.as_str())
        .await
        .unwrap_err();
    assert!(matches!(cancel_error, AppError::Conflict(_)));
}

#[tokio::test]
async fn missing_resources_return_not_found() {
    let (service, _db) = setup().await;
    assert!(matches!(
        service.get_session("missing").await.unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service.get_payment("missing").await.unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service.get_webhook("missing").await.unwrap_err(),
        AppError::NotFound(_)
    ));
}
