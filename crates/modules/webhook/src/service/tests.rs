use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::serve;
use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
use fastcart_kernel::{DomainEventEnvelope, StoreId, now_timestamp};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::net::TcpListener;

use super::{CreateWebhookEndpointInput, UpdateWebhookEndpointInput, WebhookService};
use crate::cipher::SecretCipher;
use crate::domain::{DeliveryStatus, retry_delay_seconds};

#[derive(Clone, Debug)]
struct Captured {
    headers: HeaderMap,
    body: String,
}

#[derive(Clone)]
struct MockState {
    captured: Arc<Mutex<Vec<Captured>>>,
    status: StatusCode,
}

async fn receive(State(state): State<MockState>, headers: HeaderMap, body: String) -> StatusCode {
    state
        .captured
        .lock()
        .unwrap()
        .push(Captured { headers, body });
    state.status
}

async fn spawn_mock(status: StatusCode) -> (String, MockState) {
    let state = MockState {
        captured: Arc::default(),
        status,
    };
    let app = Router::new()
        .route("/hook", post(receive))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        serve(listener, app).await.unwrap();
    });
    (format!("http://{address}/hook"), state)
}

async fn service() -> WebhookService {
    WebhookService::new(
        StoreDatabase::in_memory(StoreId::new())
            .await
            .expect("in-memory store database"),
    )
    .with_private_networks_allowed(true)
}

fn test_cipher() -> SecretCipher {
    SecretCipher::from_hex(&"ab".repeat(32)).expect("valid test key")
}

fn expected_signature(secret: &str, timestamp: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{timestamp}.{body}").as_bytes());
    format!("v1={}", hex::encode(mac.finalize().into_bytes()))
}

async fn endpoint(service: &WebhookService, url: &str, events: Vec<&str>) -> String {
    service
        .create_endpoint(CreateWebhookEndpointInput {
            url: url.to_owned(),
            events: events.into_iter().map(str::to_owned).collect(),
            ..Default::default()
        })
        .await
        .unwrap()
        .secret
}

#[tokio::test]
async fn endpoint_lifecycle_never_leaks_secrets() {
    let service = service().await;
    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/hooks".to_owned(),
            events: vec!["order.placed".to_owned(), "order.placed".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();

    assert!(created.secret.starts_with("whsec_"));
    assert_eq!(created.endpoint.events, vec!["order.placed"]);
    assert!(created.endpoint.is_active);

    let listed = service.list_endpoints().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], created.endpoint);

    let updated = service
        .update_endpoint(
            &created.endpoint.id,
            UpdateWebhookEndpointInput {
                events: Some(vec!["*".to_owned()]),
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.events, vec!["*"]);
    assert!(!updated.is_active);

    let rotated = service.rotate_secret(&created.endpoint.id).await.unwrap();
    assert_ne!(rotated, created.secret);
    assert!(rotated.starts_with("whsec_"));

    service.delete_endpoint(&created.endpoint.id).await.unwrap();
    assert!(service.list_endpoints().await.unwrap().is_empty());
    assert!(
        service
            .get_endpoint(&created.endpoint.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn rejects_private_and_plaintext_destinations_by_default() {
    let service = WebhookService::new(StoreDatabase::in_memory(StoreId::new()).await.unwrap());
    for url in [
        "http://example.com/hook",
        "https://127.0.0.1/hook",
        "https://localhost/hook",
        "https://169.254.169.254/latest/meta-data",
        "https://[::1]/hook",
    ] {
        service
            .create_endpoint(CreateWebhookEndpointInput {
                url: url.to_owned(),
                events: vec!["order.placed".to_owned()],
                ..Default::default()
            })
            .await
            .expect_err("private or plaintext URL was accepted");
    }
}

#[tokio::test]
async fn validates_urls_events_and_secrets() {
    let service = service().await;
    for url in [
        "",
        "ftp://example.com/hook",
        "file:///tmp/hook",
        "https://user:pass@example.com/hook",
        "not a url",
    ] {
        assert!(
            service
                .create_endpoint(CreateWebhookEndpointInput {
                    url: url.to_owned(),
                    events: vec!["order.placed".to_owned()],
                    ..Default::default()
                })
                .await
                .is_err(),
            "expected {url} to be rejected"
        );
    }
    assert!(
        service
            .create_endpoint(CreateWebhookEndpointInput {
                url: "https://example.com/hook".to_owned(),
                events: vec!["Bad Event!".to_owned()],
                ..Default::default()
            })
            .await
            .is_err()
    );
    assert!(
        service
            .create_endpoint(CreateWebhookEndpointInput {
                url: "https://example.com/hook".to_owned(),
                events: vec!["order.placed".to_owned()],
                secret: Some("short".to_owned()),
                ..Default::default()
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn endpoint_mutations_append_audit_entries_atomically() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = WebhookService::new(db.clone())
        .with_private_networks_allowed(true)
        .with_audit_actor(Some("user_1".to_owned()));

    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/hooks".to_owned(),
            events: vec!["order.placed".to_owned()],
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
    service.rotate_secret(&created.endpoint.id).await.unwrap();
    service.delete_endpoint(&created.endpoint.id).await.unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec![
            "webhook.created",
            "webhook.updated",
            "webhook.secret_rotated",
            "webhook.deleted",
        ]
    );
    assert!(audits.iter().all(|audit| audit.processed_at.is_none()));
    assert!(
        audits
            .iter()
            .all(|audit| audit.actor_id.as_deref() == Some("user_1"))
    );

    // A failed deletion is a no-op and must not append an audit row.
    assert!(service.delete_endpoint(&created.endpoint.id).await.is_err());
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 4);
}

#[tokio::test]
async fn no_op_updates_skip_audit_and_missing_endpoints_return_not_found() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = WebhookService::new(db.clone())
        .with_private_networks_allowed(true)
        .with_audit_actor(Some("user_1".to_owned()));
    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/hooks".to_owned(),
            events: vec!["order.placed".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();

    let unchanged = service
        .update_endpoint(
            &created.endpoint.id,
            UpdateWebhookEndpointInput {
                url: Some(created.endpoint.url.clone()),
                events: Some(created.endpoint.events.clone()),
                is_active: Some(true),
            },
        )
        .await
        .unwrap();
    assert_eq!(unchanged, created.endpoint);
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 1);

    service.delete_endpoint(&created.endpoint.id).await.unwrap();
    let error = service
        .update_endpoint(
            &created.endpoint.id,
            UpdateWebhookEndpointInput {
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(error.is_not_found());
    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec!["webhook.created", "webhook.deleted"]
    );
}

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
async fn deleting_an_endpoint_removes_its_deliveries() {
    let service = service().await;
    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/hook".to_owned(),
            events: vec!["*".to_owned()],
            ..Default::default()
        })
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
            .list_deliveries(&created.endpoint.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );

    service.delete_endpoint(&created.endpoint.id).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webhook_deliveries")
        .fetch_one(service.db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
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
