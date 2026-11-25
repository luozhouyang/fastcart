use super::support::*;
use super::*;

#[tokio::test]
async fn send_errors_do_not_abort_the_delivery_batch() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = WebhookService::new(db.clone());
    let now = now_timestamp();
    for (id, url) in [
        ("wh_bad_one", "not a url one"),
        ("wh_bad_two", "not a url two"),
    ] {
        sqlx::query(
            "INSERT INTO webhook_endpoints
                 (id, url, secret, is_active, events, metadata, created_at, updated_at)
                 VALUES (?, ?, 'whsec_unused_invalid_secret', 1, ?, '{}', ?, ?)",
        )
        .bind(id)
        .bind(url)
        .bind(r#"["order.placed"]"#)
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();
    }

    let event = DomainEventEnvelope::new("order.placed", None, serde_json::json!({}));
    assert_eq!(service.fan_out(&event).await.unwrap(), 2);

    let report = service.deliver_due(now, 10).await.unwrap();
    assert_eq!(report.attempted, 2);
    assert_eq!(report.retried, 2);
    assert_eq!(report.failed, 0);

    let delivering: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM webhook_deliveries WHERE status = 'delivering'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(delivering, 0);
}

#[tokio::test]
async fn fan_out_is_idempotent_and_honours_subscriptions() {
    let service = service().await;
    let matched = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/orders".to_owned(),
            events: vec!["order.placed".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();
    let wildcard = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/all".to_owned(),
            events: vec!["*".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();
    let unmatched = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/products".to_owned(),
            events: vec!["product.updated".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();

    let event = DomainEventEnvelope::new(
        "order.placed",
        Some(StoreId::new()),
        serde_json::json!({"id": "order_1"}),
    );
    assert_eq!(service.fan_out(&event).await.unwrap(), 2);
    assert_eq!(service.fan_out(&event).await.unwrap(), 0);

    assert_eq!(
        service
            .list_deliveries(&matched.endpoint.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        service
            .list_deliveries(&wildcard.endpoint.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        service
            .list_deliveries(&unmatched.endpoint.id, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn signs_and_records_successful_deliveries() {
    let (url, mock) = spawn_mock(StatusCode::OK).await;
    let service = service().await;
    let secret = endpoint(&service, &url, vec!["order.placed"]).await;
    let event = DomainEventEnvelope::new(
        "order.placed",
        Some(StoreId::new()),
        serde_json::json!({"id": "order_1"}),
    );
    service.fan_out(&event).await.unwrap();

    let report = service.deliver_due(now_timestamp(), 10).await.unwrap();
    assert_eq!(report.attempted, 1);
    assert_eq!(report.delivered, 1);

    let captured = mock.captured.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    let delivery = &captured[0];
    assert_eq!(
        delivery.headers.get("x-fastcart-event").unwrap(),
        "order.placed"
    );
    let timestamp = delivery
        .headers
        .get("x-fastcart-timestamp")
        .unwrap()
        .to_str()
        .unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{timestamp}.{}", delivery.body).as_bytes());
    let expected = format!("v1={}", hex::encode(mac.finalize().into_bytes()));
    let header = delivery
        .headers
        .get("x-fastcart-signature")
        .unwrap()
        .to_str()
        .unwrap();
    let signatures: Vec<&str> = header.split(',').collect();
    assert!(
        signatures.contains(&expected.as_str()),
        "expected the current signature in {header}"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&delivery.body).unwrap()["id"],
        event.id.as_str()
    );

    let endpoints = service.list_endpoints().await.unwrap();
    let deliveries = service.list_deliveries(&endpoints[0].id, 10).await.unwrap();
    assert_eq!(deliveries[0].status, DeliveryStatus::Delivered);
    assert_eq!(deliveries[0].attempts, 1);
    assert_eq!(deliveries[0].response_status, Some(200));
}

#[tokio::test]
async fn retries_failed_deliveries_then_marks_them_failed() {
    let (url, _mock) = spawn_mock(StatusCode::INTERNAL_SERVER_ERROR).await;
    let service = service().await.with_max_attempts(2);
    endpoint(&service, &url, vec!["order.placed"]).await;
    let event = DomainEventEnvelope::new("order.placed", None, serde_json::json!({}));
    service.fan_out(&event).await.unwrap();

    let now = now_timestamp();
    let first = service.deliver_due(now, 10).await.unwrap();
    assert_eq!(first.retried, 1);
    let endpoints = service.list_endpoints().await.unwrap();
    let pending = &service.list_deliveries(&endpoints[0].id, 10).await.unwrap()[0];
    assert_eq!(pending.status, DeliveryStatus::Pending);
    assert_eq!(pending.attempts, 1);
    assert_eq!(
        pending.next_retry_at,
        Some(now.saturating_add(retry_delay_seconds(1)))
    );

    // Not due yet.
    assert_eq!(
        service
            .deliver_due(now.saturating_add(1), 10)
            .await
            .unwrap()
            .attempted,
        0
    );

    let second = service
        .deliver_due(now.saturating_add(3_600), 10)
        .await
        .unwrap();
    assert_eq!(second.failed, 1);
    let failed = &service.list_deliveries(&endpoints[0].id, 10).await.unwrap()[0];
    assert_eq!(failed.status, DeliveryStatus::Failed);
    assert_eq!(failed.attempts, 2);
    assert_eq!(failed.response_status, Some(500));
}

#[tokio::test]
async fn replay_resets_a_delivery_for_another_attempt() {
    let (url, _mock) = spawn_mock(StatusCode::INTERNAL_SERVER_ERROR).await;
    let service = service().await.with_max_attempts(1);
    endpoint(&service, &url, vec!["order.placed"]).await;
    let event = DomainEventEnvelope::new("order.placed", None, serde_json::json!({}));
    service.fan_out(&event).await.unwrap();
    service.deliver_due(now_timestamp(), 10).await.unwrap();

    let endpoints = service.list_endpoints().await.unwrap();
    let delivery = &service.list_deliveries(&endpoints[0].id, 10).await.unwrap()[0];
    assert_eq!(delivery.status, DeliveryStatus::Failed);

    let replayed = service.replay(&delivery.id).await.unwrap();
    assert_eq!(replayed.status, DeliveryStatus::Pending);
    assert_eq!(replayed.attempts, 0);
    assert!(replayed.next_retry_at.is_none());
}

#[tokio::test]
async fn recovers_deliveries_abandoned_by_crashed_workers() {
    let (url, mock) = spawn_mock(StatusCode::OK).await;
    let service = service().await;
    endpoint(&service, &url, vec!["order.placed"]).await;
    service
        .fan_out(&DomainEventEnvelope::new(
            "order.placed",
            None,
            serde_json::json!({}),
        ))
        .await
        .unwrap();

    let endpoints = service.list_endpoints().await.unwrap();
    let delivery = &service.list_deliveries(&endpoints[0].id, 10).await.unwrap()[0];
    sqlx::query("UPDATE webhook_deliveries SET status = 'delivering', updated_at = ? WHERE id = ?")
        .bind(now_timestamp().saturating_sub(600))
        .bind(delivery.id.as_str())
        .execute(service.db.pool())
        .await
        .unwrap();

    let report = service.deliver_due(now_timestamp(), 10).await.unwrap();
    assert_eq!(report.delivered, 1);
    assert_eq!(mock.captured.lock().unwrap().len(), 1);
    let delivered = &service.list_deliveries(&endpoints[0].id, 10).await.unwrap()[0];
    assert_eq!(delivered.status, DeliveryStatus::Delivered);
}

#[tokio::test]
async fn delivery_sweep_does_nothing_for_inactive_endpoints() {
    let (url, mock) = spawn_mock(StatusCode::OK).await;
    let service = service().await;
    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url,
            events: vec!["*".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();
    service
        .update_endpoint(
            &created.endpoint.id,
            UpdateWebhookEndpointInput {
                is_active: Some(false),
                ..Default::default()
            },
        )
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

    assert_eq!(
        service
            .deliver_due(now_timestamp(), 10)
            .await
            .unwrap()
            .attempted,
        0
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(mock.captured.lock().unwrap().is_empty());
}
