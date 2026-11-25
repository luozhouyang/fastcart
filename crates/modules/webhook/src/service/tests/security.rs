use super::support::*;
use super::*;

#[tokio::test]
async fn secrets_are_encrypted_at_rest_and_still_sign() {
    let (url, mock) = spawn_mock(StatusCode::OK).await;
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = WebhookService::new(db.clone())
        .with_private_networks_allowed(true)
        .with_secret_cipher(Some(test_cipher()));
    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url,
            events: vec!["order.placed".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();

    let stored: String = sqlx::query_scalar("SELECT secret FROM webhook_endpoints WHERE id = ?")
        .bind(created.endpoint.id.as_str())
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert!(stored.starts_with("enc:v1:"));
    assert_ne!(stored, created.secret);
    assert!(!stored.contains(&created.secret));

    let event = DomainEventEnvelope::new("order.placed", None, serde_json::json!({}));
    service.fan_out(&event).await.unwrap();
    let report = service.deliver_due(now_timestamp(), 10).await.unwrap();
    assert_eq!(report.delivered, 1);

    let captured = mock.captured.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    let delivery = &captured[0];
    let timestamp = delivery
        .headers
        .get("x-fastcart-timestamp")
        .unwrap()
        .to_str()
        .unwrap();
    let expected = expected_signature(&created.secret, timestamp, &delivery.body);
    assert_eq!(
        delivery
            .headers
            .get("x-fastcart-signature")
            .unwrap()
            .to_str()
            .unwrap(),
        expected
    );
}

#[tokio::test]
async fn rotation_grace_emits_both_signatures_then_only_the_current() {
    let (url, mock) = spawn_mock(StatusCode::OK).await;
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = WebhookService::new(db.clone())
        .with_private_networks_allowed(true)
        .with_secret_cipher(Some(test_cipher()))
        .with_secret_rotation_grace_seconds(60);
    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url,
            events: vec!["order.placed".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();
    let rotated = service.rotate_secret(&created.endpoint.id).await.unwrap();

    service
        .fan_out(&DomainEventEnvelope::new(
            "order.placed",
            None,
            serde_json::json!({"n": 1}),
        ))
        .await
        .unwrap();
    service.deliver_due(now_timestamp(), 10).await.unwrap();

    {
        let captured = mock.captured.lock().unwrap();
        let delivery = &captured[0];
        let timestamp = delivery
            .headers
            .get("x-fastcart-timestamp")
            .unwrap()
            .to_str()
            .unwrap();
        let header = delivery
            .headers
            .get("x-fastcart-signature")
            .unwrap()
            .to_str()
            .unwrap();
        let signatures: Vec<&str> = header.split(',').collect();
        assert_eq!(signatures.len(), 2, "expected both signatures in {header}");
        assert!(
            signatures.contains(&expected_signature(&rotated, timestamp, &delivery.body).as_str())
        );
        assert!(
            signatures
                .contains(&expected_signature(&created.secret, timestamp, &delivery.body).as_str())
        );
    }

    // Move the rotation beyond the grace window: only the current secret
    // may sign new deliveries.
    sqlx::query("UPDATE webhook_endpoints SET secret_rotated_at = ? WHERE id = ?")
        .bind(now_timestamp().saturating_sub(120))
        .bind(created.endpoint.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();
    service
        .fan_out(&DomainEventEnvelope::new(
            "order.placed",
            None,
            serde_json::json!({"n": 2}),
        ))
        .await
        .unwrap();
    service.deliver_due(now_timestamp(), 10).await.unwrap();

    let captured = mock.captured.lock().unwrap().clone();
    assert_eq!(captured.len(), 2);
    let delivery = &captured[1];
    let timestamp = delivery
        .headers
        .get("x-fastcart-timestamp")
        .unwrap()
        .to_str()
        .unwrap();
    let header = delivery
        .headers
        .get("x-fastcart-signature")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        header,
        expected_signature(&rotated, timestamp, &delivery.body)
    );
}

#[tokio::test]
async fn legacy_plaintext_secrets_still_sign() {
    let (url, mock) = spawn_mock(StatusCode::OK).await;
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = WebhookService::new(db.clone())
        .with_private_networks_allowed(true)
        .with_secret_cipher(Some(test_cipher()));
    let now = now_timestamp();
    let secret = "whsec_legacy_plaintext_secret";
    sqlx::query(
        "INSERT INTO webhook_endpoints
             (id, url, secret, is_active, events, metadata, created_at, updated_at)
             VALUES ('wh_legacy', ?, ?, 1, ?, '{}', ?, ?)",
    )
    .bind(&url)
    .bind(secret)
    .bind(r#"["order.placed"]"#)
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();

    service
        .fan_out(&DomainEventEnvelope::new(
            "order.placed",
            None,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    let report = service.deliver_due(now_timestamp(), 10).await.unwrap();
    assert_eq!(report.delivered, 1);

    let captured = mock.captured.lock().unwrap().clone();
    let delivery = &captured[0];
    let timestamp = delivery
        .headers
        .get("x-fastcart-timestamp")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        delivery
            .headers
            .get("x-fastcart-signature")
            .unwrap()
            .to_str()
            .unwrap(),
        expected_signature(secret, timestamp, &delivery.body)
    );
}
