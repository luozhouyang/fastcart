use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::serve;
use fastcart_infrastructure::{
    JobQueue, NewJob, StoreDatabase, fetch_pending_audits, jobs::status,
};
use fastcart_kernel::{DomainEventEnvelope, JobId, StoreId};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::{
    ChannelDispatchReport, ChannelError, ChannelJobOutcome, ChannelKind, CreateChannelInput,
    DeliveryStatus, NotificationChannelService, RenderContext, UpdateChannelInput,
    mask_endpoint_url, supported_channel_events,
};

#[derive(Clone)]
struct MockState {
    responses: Arc<Mutex<VecDeque<(u16, String)>>>,
    captured: Arc<Mutex<Vec<Value>>>,
}

async fn receive(State(state): State<MockState>, body: String) -> (StatusCode, String) {
    state
        .captured
        .lock()
        .unwrap()
        .push(serde_json::from_str(&body).unwrap_or(Value::Null));
    let (status, body) = state
        .responses
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or((200, r#"{"code":0}"#.to_owned()));
    (StatusCode::from_u16(status).unwrap(), body)
}

async fn spawn_mock(responses: Vec<(u16, String)>) -> (String, MockState) {
    let state = MockState {
        responses: Arc::new(Mutex::new(responses.into())),
        captured: Arc::default(),
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

async fn database() -> StoreDatabase {
    StoreDatabase::in_memory(StoreId::new()).await.unwrap()
}

fn service() -> NotificationChannelService {
    NotificationChannelService::new().with_local_endpoints_allowed(true)
}

fn context() -> RenderContext {
    RenderContext {
        admin_base_url: Some("https://admin.example.com".to_owned()),
        locale: "zh-CN".to_owned(),
    }
}

fn order_event() -> DomainEventEnvelope {
    DomainEventEnvelope::new(
        "order.placed",
        None,
        json!({
            "order_id": "order_missing",
            "display_number": 42,
            "email": "Buyer@Example.com",
            "currency_code": "USD",
            "total": 1999
        }),
    )
}

async fn create_channel(
    service: &NotificationChannelService,
    db: &StoreDatabase,
    url: String,
    events: Vec<&str>,
) -> super::NotificationChannel {
    service
        .create_channel(
            db,
            CreateChannelInput {
                kind: ChannelKind::Feishu,
                name: "Operations".to_owned(),
                endpoint_url: url,
                signing_secret: Some("test-secret".to_owned()),
                events: events.into_iter().map(str::to_owned).collect(),
                is_active: true,
                config: json!({}),
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn dispatch_queues_one_delivery_and_is_idempotent() {
    let db = database().await;
    let queue = JobQueue::new(db.pool().clone());
    let service = service();
    let (url, _mock) = spawn_mock(vec![]).await;
    let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
    let event = order_event();

    let report = service
        .dispatch(&db, &queue, &event, &context())
        .await
        .unwrap();
    assert_eq!(
        report,
        ChannelDispatchReport {
            queued: 1,
            duplicates: 0,
            failed: 0,
            skipped: 0,
        }
    );
    assert_eq!(queue.count_by_status(status::PENDING).await.unwrap(), 1);

    let deliveries = service.list_deliveries(&db, &channel.id, 10).await.unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, DeliveryStatus::Queued);
    assert_eq!(deliveries[0].event_id, event.id);
    assert_eq!(deliveries[0].event_type, "order.placed");
    assert!(deliveries[0].job_id.is_some());

    let replay = service
        .dispatch(&db, &queue, &event, &context())
        .await
        .unwrap();
    assert_eq!(replay.duplicates, 1);
    assert_eq!(replay.queued, 0);
    assert_eq!(queue.count_by_status(status::PENDING).await.unwrap(), 1);
    assert_eq!(
        service
            .list_deliveries(&db, &channel.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn concurrent_dispatches_enqueue_one_job() {
    let db = database().await;
    let queue = JobQueue::new(db.pool().clone());
    let service = service();
    let (url, _mock) = spawn_mock(vec![]).await;
    let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
    let event = order_event();

    let render_context = context();
    let (first, second) = tokio::join!(
        service.dispatch(&db, &queue, &event, &render_context),
        service.dispatch(&db, &queue, &event, &render_context),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.queued + second.queued, 1);
    assert_eq!(first.duplicates + second.duplicates, 1);
    assert_eq!(queue.count_by_status(status::PENDING).await.unwrap(), 1);
    assert_eq!(
        service
            .list_deliveries(&db, &channel.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_stale_job_id_does_not_send_or_mutate_the_delivery() {
    let db = database().await;
    let queue = JobQueue::new(db.pool().clone());
    let service = service();
    let (url, mock) = spawn_mock(vec![]).await;
    let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
    service
        .dispatch(&db, &queue, &order_event(), &context())
        .await
        .unwrap();

    let claimed = queue.claim("worker", 10).await.unwrap();
    let job_id = claimed[0].id.clone();
    let mut payload = claimed[0].json_payload();
    payload["job_id"] = json!("stale-job-id");

    let outcome = service.process_job(&db, &payload).await.unwrap();
    assert_eq!(outcome, ChannelJobOutcome::Failed);
    assert!(mock.captured.lock().unwrap().is_empty());

    let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
    assert_eq!(delivery.status, DeliveryStatus::Queued);
    assert_eq!(delivery.attempts, 0);
    assert_eq!(
        delivery.job_id.as_ref().map(ToString::to_string),
        Some(job_id)
    );
}

#[tokio::test]
async fn a_terminal_delivery_is_not_sent_again() {
    let db = database().await;
    let queue = JobQueue::new(db.pool().clone());
    let service = service();
    let (url, mock) = spawn_mock(vec![(
        200,
        r#"{"code":19022,"msg":"ip not allowed"}"#.to_owned(),
    )])
    .await;
    let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
    service
        .dispatch(&db, &queue, &order_event(), &context())
        .await
        .unwrap();

    let claimed = queue.claim("worker", 10).await.unwrap();
    let payload = claimed[0].json_payload();
    assert_eq!(
        service.process_job(&db, &payload).await.unwrap(),
        ChannelJobOutcome::Failed
    );
    assert_eq!(
        service.process_job(&db, &payload).await.unwrap(),
        ChannelJobOutcome::Failed
    );
    assert_eq!(mock.captured.lock().unwrap().len(), 1);
    let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
    assert_eq!(delivery.status, DeliveryStatus::Failed);
}

#[tokio::test]
async fn an_exhausted_retry_marks_the_delivery_failed() {
    let db = database().await;
    let queue = JobQueue::new(db.pool().clone());
    let service = service();
    let (url, _mock) = spawn_mock(vec![
        (500, "temporary failure".to_owned()),
        (500, "temporary failure".to_owned()),
    ])
    .await;
    let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
    service
        .dispatch(&db, &queue, &order_event(), &context())
        .await
        .unwrap();

    let claimed = queue.claim("worker", 10).await.unwrap();
    let job = &claimed[0];
    sqlx::query("UPDATE jobs SET attempts = max_attempts - 1 WHERE id = ?")
        .bind(&job.id)
        .execute(db.pool())
        .await
        .unwrap();

    let outcome = service.process_job(&db, &job.json_payload()).await;
    assert!(matches!(outcome, Err(ChannelError::Retryable(_))));
    let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
    assert_eq!(delivery.status, DeliveryStatus::Failed);
    assert!(delivery.last_error.is_some());

    assert!(!queue.fail(&job.id, "exhausted", "worker").await.unwrap());
    assert_eq!(queue.count_by_status(status::FAILED).await.unwrap(), 1);
}

#[tokio::test]
async fn retryable_provider_failure_is_retried_then_sent() {
    let db = database().await;
    let queue = JobQueue::new(db.pool().clone());
    let service = service();
    let (url, mock) = spawn_mock(vec![
        (500, "temporary failure".to_owned()),
        (200, r#"{"code":0}"#.to_owned()),
    ])
    .await;
    let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
    let event = order_event();
    service
        .dispatch(&db, &queue, &event, &context())
        .await
        .unwrap();

    let claimed = queue.claim("worker", 10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    let payload = claimed[0].json_payload();

    let first = service.process_job(&db, &payload).await;
    assert!(matches!(first, Err(ChannelError::Retryable(_))));
    let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
    assert_eq!(delivery.status, DeliveryStatus::Queued);
    assert_eq!(delivery.attempts, 1);
    assert!(delivery.last_error.is_some());

    let second = service.process_job(&db, &payload).await.unwrap();
    assert_eq!(second, ChannelJobOutcome::Sent);
    let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
    assert_eq!(delivery.status, DeliveryStatus::Sent);
    assert_eq!(delivery.attempts, 2);
    assert_eq!(delivery.response_status, Some(200));
    assert!(delivery.sent_at.is_some());

    let channel = service.get_channel(&db, &channel.id).await.unwrap();
    assert!(channel.last_success_at.is_some());
    assert!(channel.last_error.is_none());

    let captured = mock.captured.lock().unwrap().clone();
    assert_eq!(captured.len(), 2);
    for body in captured {
        let timestamp: i64 = body["timestamp"].as_str().unwrap().parse().unwrap();
        let expected = crate::feishu::sign("test-secret", timestamp).unwrap();
        assert_eq!(body["sign"].as_str().unwrap(), expected);
        assert_eq!(body["msg_type"], "interactive");
    }
}

#[tokio::test]
async fn permanent_provider_failure_marks_the_delivery_failed() {
    let db = database().await;
    let queue = JobQueue::new(db.pool().clone());
    let service = service();
    let (url, _mock) = spawn_mock(vec![(
        200,
        r#"{"code":19022,"msg":"ip not allowed"}"#.to_owned(),
    )])
    .await;
    let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
    service
        .dispatch(&db, &queue, &order_event(), &context())
        .await
        .unwrap();

    let claimed = queue.claim("worker", 10).await.unwrap();
    let payload = claimed[0].json_payload();
    let outcome = service.process_job(&db, &payload).await.unwrap();
    assert_eq!(outcome, ChannelJobOutcome::Failed);

    let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
    assert_eq!(delivery.status, DeliveryStatus::Failed);
    assert_eq!(delivery.attempts, 1);
    assert!(delivery.last_error.as_deref().unwrap().contains("19022"));
    let channel = service.get_channel(&db, &channel.id).await.unwrap();
    assert!(channel.last_error.is_some());
}

#[tokio::test]
async fn inactive_channels_are_skipped_during_job_processing() {
    let db = database().await;
    let queue = JobQueue::new(db.pool().clone());
    let service = service();
    let (url, _mock) = spawn_mock(vec![]).await;
    let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;
    service
        .dispatch(&db, &queue, &order_event(), &context())
        .await
        .unwrap();
    let claimed = queue.claim("worker", 10).await.unwrap();
    let payload = claimed[0].json_payload();

    service
        .update_channel(
            &db,
            &channel.id,
            UpdateChannelInput {
                is_active: Some(false),
                ..UpdateChannelInput::default()
            },
        )
        .await
        .unwrap();

    let outcome = service.process_job(&db, &payload).await.unwrap();
    assert_eq!(outcome, ChannelJobOutcome::Skipped);
    let delivery = &service.list_deliveries(&db, &channel.id, 10).await.unwrap()[0];
    assert_eq!(delivery.status, DeliveryStatus::Skipped);
}

#[tokio::test]
async fn channel_crud_masks_endpoints_and_validates_input() {
    let db = database().await;
    let service = service();
    let channel = service
        .create_channel(
            &db,
            CreateChannelInput {
                kind: ChannelKind::Feishu,
                name: "Ops".to_owned(),
                endpoint_url: "https://open.feishu.cn/open-apis/bot/v2/hook/abcdef123456"
                    .to_owned(),
                signing_secret: Some("secret".to_owned()),
                events: vec!["order.placed".to_owned(), "inquiry.created".to_owned()],
                is_active: true,
                config: json!({"card": "compact"}),
            },
        )
        .await
        .unwrap();
    assert_eq!(channel.kind, ChannelKind::Feishu);
    assert_eq!(channel.events, vec!["order.placed", "inquiry.created"]);
    assert_eq!(
        service.list_channels(&db).await.unwrap(),
        vec![channel.clone()]
    );

    assert_eq!(
        mask_endpoint_url("https://open.feishu.cn/open-apis/bot/v2/hook/abcdef123456"),
        "https://open.feishu.cn/open-apis/bot/v2/hook/****3456"
    );
    assert_eq!(mask_endpoint_url("not a url"), "****");

    let secret: Option<String> =
        sqlx::query_scalar("SELECT signing_secret FROM notification_channels WHERE id = ?")
            .bind(channel.id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(secret.as_deref(), Some("secret"));

    let updated = service
        .update_channel(
            &db,
            &channel.id,
            UpdateChannelInput {
                name: Some("Renamed".to_owned()),
                signing_secret: Some(None),
                events: Some(vec!["inquiry.created".to_owned()]),
                is_active: Some(false),
                config: Some(json!({"card": "full"})),
                ..UpdateChannelInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.events, vec!["inquiry.created"]);
    assert!(!updated.is_active);
    assert_eq!(updated.config, json!({"card": "full"}));
    let secret: Option<String> =
        sqlx::query_scalar("SELECT signing_secret FROM notification_channels WHERE id = ?")
            .bind(channel.id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(secret.is_none());

    service
        .update_channel(
            &db,
            &channel.id,
            UpdateChannelInput {
                signing_secret: Some(Some("new-secret".to_owned())),
                ..UpdateChannelInput::default()
            },
        )
        .await
        .unwrap();
    let secret: Option<String> =
        sqlx::query_scalar("SELECT signing_secret FROM notification_channels WHERE id = ?")
            .bind(channel.id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(secret.as_deref(), Some("new-secret"));

    service.delete_channel(&db, &channel.id).await.unwrap();
    assert!(service.list_channels(&db).await.unwrap().is_empty());
    assert!(
        service
            .get_channel(&db, &channel.id)
            .await
            .unwrap_err()
            .is_not_found()
    );

    for input in [
        CreateChannelInput {
            name: "Ops".to_owned(),
            endpoint_url: "https://example.com/hook".to_owned(),
            events: vec!["order.placed".to_owned()],
            config: json!({}),
            ..valid_input()
        },
        CreateChannelInput {
            events: vec!["product.updated".to_owned()],
            ..valid_input()
        },
        CreateChannelInput {
            name: " ".to_owned(),
            ..valid_input()
        },
        CreateChannelInput {
            config: json!([]),
            ..valid_input()
        },
    ] {
        assert!(
            service.create_channel(&db, input).await.is_err(),
            "invalid channel input was accepted"
        );
    }

    let event_types: Vec<&str> = supported_channel_events()
        .iter()
        .map(|(event_type, _, _, _)| *event_type)
        .collect();
    assert_eq!(event_types, vec!["order.placed", "inquiry.created"]);
}

fn valid_input() -> CreateChannelInput {
    CreateChannelInput {
        kind: ChannelKind::Feishu,
        name: "Ops".to_owned(),
        endpoint_url: "https://open.feishu.cn/open-apis/bot/v2/hook/token1234".to_owned(),
        signing_secret: None,
        events: vec!["order.placed".to_owned()],
        is_active: true,
        config: json!({}),
    }
}

#[tokio::test]
async fn test_channel_sends_without_writing_a_delivery() {
    let db = database().await;
    let service = service();
    let (url, mock) = spawn_mock(vec![]).await;
    let channel = create_channel(&service, &db, url, vec!["order.placed"]).await;

    let result = service
        .test_channel(&db, &channel.id, &context())
        .await
        .unwrap();
    assert!(result.ok);
    assert_eq!(result.status, Some(200));
    assert!(result.error.is_none());
    assert!(
        service
            .list_deliveries(&db, &channel.id, 10)
            .await
            .unwrap()
            .is_empty()
    );

    let captured = mock.captured.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0]["msg_type"], "text");
    assert_eq!(captured[0]["content"]["text"], "Fastcart 通知测试");
    assert!(captured[0]["sign"].is_string());

    let channel = service.get_channel(&db, &channel.id).await.unwrap();
    assert!(channel.last_success_at.is_some());
}

#[tokio::test]
async fn retention_deletes_only_terminal_old_deliveries() {
    let db = database().await;
    let service = service();
    let channel = create_channel(
        &service,
        &db,
        "https://open.feishu.cn/open-apis/bot/v2/hook/token1234".to_owned(),
        vec!["order.placed"],
    )
    .await;
    let now = fastcart_kernel::now_timestamp();
    for (id, event_id, status, created_at) in [
        ("old-sent", "event-old-sent", "sent", 0),
        ("old-queued", "event-old-queued", "queued", 0),
        ("recent-sent", "event-recent-sent", "sent", now),
    ] {
        sqlx::query(
            "INSERT INTO notification_channel_deliveries
                 (id, channel_id, event_id, event_type, status, created_at, updated_at)
                 VALUES (?, ?, ?, 'order.placed', ?, ?, ?)",
        )
        .bind(id)
        .bind(channel.id.as_str())
        .bind(event_id)
        .bind(status)
        .bind(created_at)
        .bind(created_at)
        .execute(db.pool())
        .await
        .unwrap();
    }

    let deleted = service
        .delete_deliveries_before(&db, now - 1)
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    let remaining = service.list_deliveries(&db, &channel.id, 10).await.unwrap();
    let statuses: Vec<DeliveryStatus> = remaining.iter().map(|row| row.status).collect();
    assert_eq!(statuses, vec![DeliveryStatus::Sent, DeliveryStatus::Queued]);
}

#[tokio::test]
async fn abandoned_channel_claims_are_recovered_without_duplicating_queued_jobs() {
    let db = database().await;
    let queue = JobQueue::new(db.pool().clone());
    let service = service();
    let channel = create_channel(
        &service,
        &db,
        "https://open.feishu.cn/open-apis/bot/v2/hook/token1234".to_owned(),
        vec!["order.placed"],
    )
    .await;
    let now = fastcart_kernel::now_timestamp();
    let stale = now - 1_000;

    for (id, event_id, status, job_id, claimed_at) in [
        (
            "abandoned",
            "event-abandoned",
            "queued",
            Some("job_missing"),
            Some(stale),
        ),
        (
            "pending-job",
            "event-pending-job",
            "queued",
            Some("job_pending"),
            Some(stale),
        ),
        (
            "running-job",
            "event-running-job",
            "queued",
            Some("job_running"),
            Some(stale),
        ),
        (
            "fresh",
            "event-fresh",
            "queued",
            Some("job_fresh"),
            Some(now),
        ),
        ("sent", "event-sent", "sent", Some("job_sent"), Some(stale)),
    ] {
        sqlx::query(
            "INSERT INTO notification_channel_deliveries
                 (id, channel_id, event_id, event_type, status, job_id, claimed_at,
                  created_at, updated_at)
                 VALUES (?, ?, ?, 'order.placed', ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(channel.id.as_str())
        .bind(event_id)
        .bind(status)
        .bind(job_id)
        .bind(claimed_at)
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();
    }

    queue
        .enqueue_with_id(
            &JobId::from_string("job_pending"),
            NewJob::new("send_channel_notification", json!({})),
        )
        .await
        .unwrap();
    queue
        .enqueue_with_id(
            &JobId::from_string("job_running"),
            NewJob::new("send_channel_notification", json!({})),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE jobs SET status = 'running' WHERE id = 'job_running'")
        .execute(db.pool())
        .await
        .unwrap();

    let recovered = NotificationChannelService::recover_abandoned_claims(&db, now, 300)
        .await
        .unwrap();
    assert_eq!(recovered, 1);

    let rows: Vec<(String, String, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT id, status, job_id, claimed_at
             FROM notification_channel_deliveries ORDER BY id",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    let by_id: std::collections::HashMap<_, _> = rows
        .into_iter()
        .map(|(id, status, job_id, claimed_at)| (id, (status, job_id, claimed_at)))
        .collect();
    assert_eq!(by_id["abandoned"].0, "pending");
    assert!(by_id["abandoned"].1.is_none());
    assert!(by_id["abandoned"].2.is_none());
    for id in ["pending-job", "running-job", "fresh"] {
        assert_eq!(by_id[id].0, "queued");
        assert!(by_id[id].1.is_some());
        assert!(by_id[id].2.is_some());
    }
    assert_eq!(by_id["sent"].0, "sent");

    // A second sweep has nothing left to recover.
    assert_eq!(
        NotificationChannelService::recover_abandoned_claims(&db, now, 300)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn mutations_append_audit_entries_and_skip_noops() {
    let db = database().await;
    let service = service().with_audit_actor(Some("user_1".to_owned()));

    let channel = service.create_channel(&db, valid_input()).await.unwrap();
    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action, "notification_channel.created");
    assert_eq!(audits[0].actor_id.as_deref(), Some("user_1"));
    assert!(audits[0].processed_at.is_none());

    // An update that resolves to the current values is a no-op.
    service
        .update_channel(&db, &channel.id, UpdateChannelInput::default())
        .await
        .unwrap();
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 1);

    service.delete_channel(&db, &channel.id).await.unwrap();
    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(audits.len(), 2);
    assert_eq!(audits[1].action, "notification_channel.deleted");

    // Deleting an already deleted channel is a no-op error.
    assert!(service.delete_channel(&db, &channel.id).await.is_err());
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 2);
}
