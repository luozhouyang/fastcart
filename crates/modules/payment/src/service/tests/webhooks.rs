use super::*;

#[tokio::test]
async fn refund_failed_webhook_reverts_the_payment_status() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO refunds \
             (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
              idempotency_key, reason, created_at, updated_at) \
             VALUES ('refund_revert', ?, NULL, 400, 'USD', 'succeeded', \
                     'manual_refund_revert', NULL, NULL, ?, ?)",
    )
    .bind(payment.id.as_str())
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query("UPDATE payments SET status = 'partially_refunded' WHERE id = ?")
        .bind(payment.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

    let body = r#"{"id":"evt_refund_failed","type":"refund.failed","data":{"provider_refund_id":"manual_refund_revert","provider_payment_id":"unused"}}"#;
    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(record.status, WebhookStatus::Processed);

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
async fn webhook_with_valid_signature_updates_session_state() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_captured_1","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    assert_eq!(record.status, WebhookStatus::Processed);
    assert_eq!(record.event_type, "payment.captured");
    assert_eq!(record.attempts, 1);
    assert!(record.processed_at.is_some());
    let updated = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(updated.status, PaymentStatus::Captured);
}

#[tokio::test]
async fn failed_webhooks_are_retried_instead_of_being_dropped() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_retry","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    sqlx::query(
        "INSERT INTO payment_webhooks
             (id, provider, provider_event_id, event_type, payload, status, attempts,
              processed_at, last_error, created_at)
             VALUES ('wh_retry', 'manual', 'evt_retry', 'payment.captured', '{}',
                     'failed', 1, NULL, 'transient', 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(record.status, WebhookStatus::Processed);
    assert_eq!(record.attempts, 2);
    let updated = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(updated.status, PaymentStatus::Captured);
}

#[tokio::test]
async fn received_webhook_is_applied_by_a_later_delivery() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_received","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    // Simulate a first delivery that inserted the row and then crashed
    // before applying the event.
    sqlx::query(
        "INSERT INTO payment_webhooks
             (id, provider, provider_event_id, event_type, payload, status, attempts,
              processed_at, last_error, created_at)
             VALUES ('wh_received', 'manual', 'evt_received', 'payment.captured', '{}',
                     'received', 0, NULL, NULL, 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    assert_eq!(record.id.as_str(), "wh_received");
    assert_eq!(record.status, WebhookStatus::Processed);
    assert_eq!(record.attempts, 1);
    assert_eq!(
        service
            .get_session(session.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Captured
    );
}

#[tokio::test]
async fn webhook_with_invalid_signature_is_rejected() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let body = br#"{"id":"evt_1","type":"payment.captured","data":{}}"#;

    let error = service
        .process_webhook("manual", headers("wrong-secret", body), body)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Unauthorized));
    let listed = service
        .list_webhooks(None, Pagination::default())
        .await
        .unwrap();
    assert_eq!(listed.count, 0);
}

#[tokio::test]
async fn duplicate_webhook_is_idempotent() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_dup","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    let first = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    let second = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(second.attempts, 1);
    assert_eq!(second.status, WebhookStatus::Processed);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payment_webhooks")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn webhook_capture_creates_one_payment_and_is_idempotent_on_replay() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_async","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_1","amount":1000,"currency":"USD"}}}}"#
    );

    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(record.status, WebhookStatus::Processed);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE provider = 'manual'")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);

    let payment = service
        .payment_for_session_id(session.id.as_str())
        .await
        .unwrap();
    assert_eq!(
        payment.provider_payment_id.as_deref(),
        Some("manual_payment_1")
    );
    assert_eq!(payment.amount, 1000);
    assert_eq!(payment.currency_code, "USD");
    assert_eq!(payment.status, PaymentStatus::Captured);
    let updated = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(updated.status, PaymentStatus::Captured);
    assert_eq!(
        updated.data.get("payment_id"),
        Some(&serde_json::json!(payment.id.as_str()))
    );

    // Replaying the exact same provider event must not create another row.
    service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    // A different event with the same provider identifiers is also
    // idempotent: the unique provider payment id wins.
    let replay = format!(
        r#"{{"id":"evt_async_2","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_1","amount":1000,"currency":"USD"}}}}"#
    );
    service
        .process_webhook(
            "manual",
            headers("secret", replay.as_bytes()),
            replay.as_bytes(),
        )
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE provider = 'manual'")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    let again = service
        .payment_for_session_id(session.id.as_str())
        .await
        .unwrap();
    assert_eq!(again.id, payment.id);
}

#[tokio::test]
async fn webhook_capture_emits_the_captured_event_once() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let mut session_input = input("manual", 1000);
    let order_id = OrderId::new();
    session_input.order_id = Some(order_id.clone());
    let session = service.create_session(session_input).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_event_once","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_event"}}}}"#
    );
    service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    // A redelivery under a different provider event id is still the same
    // capture and must not emit a second lifecycle event.
    let replay = format!(
        r#"{{"id":"evt_event_once_2","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_event"}}}}"#
    );
    service
        .process_webhook(
            "manual",
            headers("secret", replay.as_bytes()),
            replay.as_bytes(),
        )
        .await
        .unwrap();

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM outbox WHERE event_type = 'payment.captured'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count, 1);
    let payload: String =
        sqlx::query_scalar("SELECT payload FROM outbox WHERE event_type = 'payment.captured'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["order_id"], order_id.as_str());
    assert!(payload["payment_id"].as_str().is_some());
}

#[tokio::test]
async fn webhook_capture_reconciles_a_failed_session() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    service
        .fail_session(session.id.as_str(), "previous ambiguity")
        .await
        .unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_recover","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_recover","amount":1000}}}}"#
    );

    service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    let updated = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(updated.status, PaymentStatus::Captured);
    let payment = service
        .payment_for_session_id(session.id.as_str())
        .await
        .unwrap();
    assert_eq!(payment.status, PaymentStatus::Captured);
    assert_eq!(
        payment.provider_payment_id.as_deref(),
        Some("manual_payment_recover")
    );
}

#[tokio::test]
async fn webhook_apply_failure_is_retryable_and_reapplied_on_redelivery() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_apply_retry","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    sqlx::query(
        "CREATE TRIGGER reject_capture BEFORE UPDATE ON payment_sessions \
             WHEN NEW.status = 'captured' BEGIN SELECT RAISE(ABORT, 'transient'); END",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let error = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::ExternalService(_)));
    let status: String = sqlx::query_scalar(
        "SELECT status FROM payment_webhooks WHERE provider_event_id = 'evt_apply_retry'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(status, "failed");

    sqlx::query("DROP TRIGGER reject_capture")
        .execute(db.pool())
        .await
        .unwrap();
    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(record.status, WebhookStatus::Processed);
    assert_eq!(record.attempts, 2);
    assert_eq!(
        service
            .get_session(session.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Captured
    );
}

#[tokio::test]
async fn unknown_webhook_event_is_stored_without_state_changes() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let body = br#"{"id":"evt_unknown","type":"customer.created","data":{"customer_id":"cust_1"}}"#;
    let record = service
        .process_webhook("manual", headers("secret", body), body)
        .await
        .unwrap();

    assert_eq!(record.status, WebhookStatus::Processed);
    assert_eq!(record.event_type, "customer.created");
    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Pending);
}

#[tokio::test]
async fn payment_failed_webhook_marks_session_failed() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_failed","type":"payment.failed","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Failed);
}

#[tokio::test]
async fn refund_succeeded_webhook_updates_refund_and_payment() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    // Persist a pending refund directly so the webhook has a row to update.
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO refunds \
             (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
              reason, created_at, updated_at) \
             VALUES ('refund_pending', ?, NULL, 400, 'USD', 'pending', 'manual_refund_1', \
              NULL, ?, ?)",
    )
    .bind(payment.id.as_str())
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();

    let body = r#"{"id":"evt_refund","type":"refund.succeeded","data":{"provider_refund_id":"manual_refund_1"}}"#;
    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(record.status, WebhookStatus::Processed);

    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Succeeded);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::PartiallyRefunded
    );
}

#[tokio::test]
async fn webhook_unknown_provider_is_rejected() {
    let (service, _db) = setup().await;
    let error = service
        .process_webhook("missing", BTreeMap::new(), b"{}")
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));
}
