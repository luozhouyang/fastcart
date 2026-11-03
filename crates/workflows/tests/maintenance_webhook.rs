//! Maintenance integration: outbox events fan out to signed webhooks.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::serve;
use fastcart_email::{
    EmailMessage, EmailProviderRegistry, EmailService, MemoryEmailProvider, SendEmailJob,
};
use fastcart_infrastructure::{
    DatabaseOptions, EventBus, JobQueue, NewJob, StoreDatabaseManager, append_outbox_event,
};
use fastcart_kernel::DomainEventEnvelope;
use fastcart_module_webhook::{CreateWebhookEndpointInput, WebhookService};
use fastcart_platform::{CreateStoreInput, PlatformStore, StoreProvisioningService};
use fastcart_workflows::run_maintenance;
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
struct Captured {
    headers: HeaderMap,
    body: String,
}

#[derive(Clone)]
struct MockState {
    captured: Arc<Mutex<Vec<Captured>>>,
}

async fn receive(State(state): State<MockState>, headers: HeaderMap, body: String) -> StatusCode {
    state
        .captured
        .lock()
        .unwrap()
        .push(Captured { headers, body });
    StatusCode::OK
}

async fn spawn_mock() -> (String, MockState) {
    let state = MockState {
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

#[tokio::test]
async fn maintenance_dispatches_outbox_events_to_webhooks() {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let store = platform
        .create_store(CreateStoreInput::new(
            "Webhook Store",
            "webhook-store",
            "hooks.example.com",
        ))
        .await
        .unwrap();
    StoreProvisioningService::new(platform.clone(), databases.clone())
        .provision(&store.id, None)
        .await
        .unwrap();

    let database = databases.database(&store.id).await.unwrap();
    let (url, mock) = spawn_mock().await;
    let endpoint = WebhookService::new(database.clone())
        .create_endpoint(CreateWebhookEndpointInput {
            url,
            events: vec!["order.placed".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();

    let event = DomainEventEnvelope::new(
        "order.placed",
        Some(store.id.clone()),
        serde_json::json!({"order_id": "order_1"}),
    );
    append_outbox_event(database.pool(), &event).await.unwrap();

    let message = EmailMessage::new("buyer@example.com", "Order confirmed")
        .with_text("Your order is on its way.");
    JobQueue::new(database.pool().clone())
        .enqueue(NewJob::new(
            SendEmailJob::JOB_TYPE,
            serde_json::to_value(SendEmailJob::from_message(&message)).unwrap(),
        ))
        .await
        .unwrap();
    let provider = Arc::new(MemoryEmailProvider::new("memory"));
    let registry = EmailProviderRegistry::new();
    registry.register(provider.clone()).unwrap();
    let email = EmailService::new(registry);

    let report = run_maintenance(&platform, &databases, &EventBus::new(8), &email).await;
    assert_eq!(report.outbox_dispatched, 1);
    assert_eq!(report.webhooks_delivered, 1);
    assert_eq!(report.webhooks_failed, 0);
    assert_eq!(report.jobs_processed, 1);
    assert_eq!(provider.sent_count(), 1);
    assert_eq!(provider.messages()[0].subject, "Order confirmed");

    let captured = mock.captured.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].headers.get("x-fastcart-event").unwrap(),
        "order.placed"
    );
    assert!(captured[0].headers.contains_key("x-fastcart-signature"));
    let payload: serde_json::Value = serde_json::from_str(&captured[0].body).unwrap();
    assert_eq!(payload["id"], event.id.as_str());
    assert_eq!(payload["data"]["order_id"], "order_1");

    let deliveries = WebhookService::new(database.clone())
        .list_deliveries(&endpoint.endpoint.id, 10)
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status.as_str(), "delivered");
}
