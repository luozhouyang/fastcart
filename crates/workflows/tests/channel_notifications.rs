//! Maintenance integration: outbox events fan out to notification channels.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::Json;
use axum::routing::post;
use fastcart_email::EmailService;
use fastcart_infrastructure::{
    DatabaseOptions, EventBus, StoreDatabaseManager, append_outbox_event,
};
use fastcart_kernel::{DomainEventEnvelope, StoreId};
use fastcart_notifications::{
    ChannelKind, CreateChannelInput, DeliveryStatus, NotificationChannelService,
};
use fastcart_platform::{CreateStoreInput, PlatformStore, StoreProvisioningService};
use fastcart_workflows::run_maintenance_with_notifications;
use serde_json::{Value, json};

async fn mock_feishu() -> (String, Arc<Mutex<Vec<Value>>>) {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let state = captured.clone();
    let app = Router::new().route(
        "/hook/{token}",
        post(move |Json(body): Json<Value>| {
            let state = state.clone();
            async move {
                state.lock().unwrap().push(body);
                Json(json!({"code": 0, "msg": "success"}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}/hook/test-token"), captured)
}

#[tokio::test]
async fn maintenance_queues_and_delivers_inquiry_notifications() {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let store = platform
        .create_store(CreateStoreInput::new(
            "Channel Store",
            "channel-store",
            "channel.example.com",
        ))
        .await
        .unwrap();
    StoreProvisioningService::new(platform.clone(), databases.clone())
        .provision(&store.id, None)
        .await
        .unwrap();
    let database = databases.database(&store.id).await.unwrap();

    let (webhook_url, captured) = mock_feishu().await;
    let channels = NotificationChannelService::new().with_local_endpoints_allowed(true);
    let channel = channels
        .create_channel(
            &database,
            CreateChannelInput {
                kind: ChannelKind::Feishu,
                name: "Operations".to_owned(),
                endpoint_url: webhook_url,
                signing_secret: Some("test-secret".to_owned()),
                events: vec!["inquiry.created".to_owned()],
                is_active: true,
                config: json!({}),
            },
        )
        .await
        .unwrap();

    let event = DomainEventEnvelope::new(
        "inquiry.created",
        Some(StoreId::from_string(store.id.as_str())),
        json!({
            "inquiry_id": "inq_test",
            "event_id": "evt_test",
            "form_id": "iform_test",
            "form_handle": "contact",
            "form_version": 1,
            "name": "Ada",
            "email": "ada@example.com",
            "subject": "Quote",
            "message": "Please send a quote.",
            "page_url": "https://channel.example.com/contact",
            "attachment_count": 0,
            "custom_fields": [
                { "key": "company", "label": "Company", "value": "Analytical Engines" }
            ]
        }),
    )
    .with_idempotency_key("inquiry:inq_test");
    append_outbox_event(database.pool(), &event).await.unwrap();

    let events = EventBus::new(8);
    let email = EmailService::with_default_registry();
    let admin_base_url = Some("https://admin.example.com".to_owned());

    let first = run_maintenance_with_notifications(
        &platform,
        &databases,
        &events,
        &email,
        true,
        admin_base_url.clone(),
    )
    .await;
    assert_eq!(first.outbox_dispatched, 1);
    assert_eq!(first.channel_notifications_queued, 1);
    assert_eq!(first.failures, 0);

    let second = run_maintenance_with_notifications(
        &platform,
        &databases,
        &events,
        &email,
        true,
        admin_base_url,
    )
    .await;
    assert_eq!(second.jobs_processed, 1);
    assert_eq!(second.failures, 0);

    let deliveries = channels
        .list_deliveries(&database, &channel.id, 10)
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, DeliveryStatus::Sent);

    let requests = captured.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let body = &requests[0];
    assert_eq!(body["msg_type"], "interactive");
    assert!(body.get("sign").is_some());
    let card = body["card"].to_string();
    assert!(card.contains("Ada"));
    assert!(card.contains("Please send a quote."));
    assert!(card.contains("Analytical Engines"));
    assert!(card.contains("admin.example.com/inbox/inq_test"));
}
