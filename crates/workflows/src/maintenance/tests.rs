use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use fastcart_email::{
    EmailError, EmailMessage, EmailProvider, EmailProviderRegistry, EmailReceipt, EmailService,
    SEND_EMAIL_JOB_TYPE, SendEmailJob,
};
use fastcart_infrastructure::{
    DatabaseOptions, EventBus, JobQueue, LocalStorage, NewAuditOutbox, NewJob, Storage,
    StoreDatabaseManager, append_audit_outbox, append_outbox_event, fetch_pending_audits,
};
use fastcart_kernel::{DomainEventEnvelope, Pagination, PaymentId, now_timestamp};
use fastcart_module_asset::{AssetService, CreateAssetInput, fetch_pending_asset_purges};
use fastcart_module_order::{
    CreateOrderInput, CreateOrderItemInput, OrderService, PaymentStatus, RefundInput,
};
use fastcart_platform::{CreateStoreInput, PlatformStore, StoreProvisioningService};

use super::retention::cleanup_asset_purges;
use super::run_maintenance_with_options;

/// Stores are independent databases, so one store's slow work must not
/// serialize the sweep. The probe records how many email deliveries were
/// in flight at once; a serial sweep can never exceed one.
#[tokio::test]
async fn maintenance_sweeps_stores_concurrently() {
    struct ConcurrencyProbe {
        in_flight: AtomicUsize,
        peak: AtomicUsize,
    }

    struct SlowEmailProvider {
        probe: Arc<ConcurrencyProbe>,
    }

    #[async_trait]
    impl EmailProvider for SlowEmailProvider {
        fn id(&self) -> &str {
            "slow-probe"
        }

        async fn send(&self, _message: EmailMessage) -> Result<EmailReceipt, EmailError> {
            let in_flight = self.probe.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.probe.peak.fetch_max(in_flight, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            self.probe.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(EmailReceipt {
                provider: self.id().to_owned(),
                message_id: "probe".to_owned(),
                accepted_at: now_timestamp(),
            })
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    for index in 0..3 {
        let store = platform
            .create_store(CreateStoreInput::new(
                format!("Concurrency Store {index}"),
                format!("concurrency-store-{index}"),
                format!("concurrency{index}.example.com"),
            ))
            .await
            .unwrap();
        StoreProvisioningService::new(platform.clone(), databases.clone())
            .provision(&store.id, None)
            .await
            .unwrap();
        let database = databases.database(&store.id).await.unwrap();
        JobQueue::new(database.pool().clone())
            .enqueue(NewJob::new(
                SEND_EMAIL_JOB_TYPE,
                serde_json::json!({
                    "to": "buyer@example.com",
                    "subject": "Maintenance",
                    "text": "hello",
                }),
            ))
            .await
            .unwrap();
    }

    let probe = Arc::new(ConcurrencyProbe {
        in_flight: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
    });
    let registry = EmailProviderRegistry::new();
    registry
        .register(Arc::new(SlowEmailProvider {
            probe: probe.clone(),
        }))
        .unwrap();
    registry.set_default("slow-probe").unwrap();

    let report = run_maintenance_with_options(
        &platform,
        &databases,
        &EventBus::new(8),
        &EmailService::new(registry),
        true,
    )
    .await;

    assert_eq!(report.jobs_processed, 3);
    assert_eq!(report.failures, 0);
    let peak = probe.peak.load(Ordering::SeqCst);
    assert!(
        peak >= 2,
        "stores must be swept concurrently, but at most {peak} store(s) did work at once"
    );
}

#[tokio::test]
async fn maintenance_enqueues_exactly_one_notification_email() {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let store = platform
        .create_store(CreateStoreInput::new(
            "Notification Store",
            "notification-store",
            "notify.example.com",
        ))
        .await
        .unwrap();
    StoreProvisioningService::new(platform.clone(), databases.clone())
        .provision(&store.id, None)
        .await
        .unwrap();
    let database = databases.database(&store.id).await.unwrap();

    let event = DomainEventEnvelope::new(
        "order.placed",
        Some(store.id.clone()),
        serde_json::json!({
            "order_id": "order_missing",
            "display_number": 7,
            "email": "buyer@example.com",
            "currency_code": "USD",
            "total": 2500
        }),
    );
    append_outbox_event(database.pool(), &event).await.unwrap();

    let report = run_maintenance_with_options(
        &platform,
        &databases,
        &EventBus::new(8),
        &EmailService::with_default_registry(),
        true,
    )
    .await;

    assert_eq!(report.outbox_dispatched, 1);
    assert_eq!(report.notifications_sent, 1);
    assert_eq!(report.notifications_duplicates, 0);
    assert_eq!(report.notifications_skipped, 0);
    assert_eq!(report.notifications_failed, 0);
    assert_eq!(report.failures, 0);

    let queue = JobQueue::new(database.pool().clone());
    let jobs = queue.claim("test", 10).await.unwrap();
    assert_eq!(jobs.len(), 1, "a supported event must enqueue one job");
    assert_eq!(jobs[0].job_type, SEND_EMAIL_JOB_TYPE);
    let payload: SendEmailJob = serde_json::from_str(&jobs[0].payload).unwrap();
    assert_eq!(payload.to, "buyer@example.com");
    assert!(payload.html.as_deref().is_some_and(|html| !html.is_empty()));
    assert!(queue.claim("test", 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn maintenance_reconciles_a_failed_refund_against_the_order() {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let store = platform
        .create_store(CreateStoreInput::new(
            "Refund Store",
            "refund-store",
            "refund.example.com",
        ))
        .await
        .unwrap();
    StoreProvisioningService::new(platform.clone(), databases.clone())
        .provision(&store.id, None)
        .await
        .unwrap();
    let database = databases.database(&store.id).await.unwrap();

    let orders = OrderService::new(database.clone());
    let order = orders
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 1,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_000,
            total: 1_000,
            shipping_address: serde_json::json!({}),
            billing_address: serde_json::json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    let payment = PaymentId::new();
    let refunded = orders
        .add_refund(
            &order.id,
            RefundInput {
                payment_id: payment.clone(),
                amount: 1_000,
                currency: "USD".to_owned(),
                reason: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(refunded.payment_status, PaymentStatus::Refunded);

    let event = DomainEventEnvelope::new(
        "payment.refund_failed",
        Some(store.id.clone()),
        serde_json::json!({
            "payment_id": payment.as_str(),
            "order_id": order.id.as_str(),
            "refund_id": "refund_provider",
            "amount": 1_000,
            "currency_code": "USD",
        }),
    );
    append_outbox_event(database.pool(), &event).await.unwrap();

    let report = run_maintenance_with_options(
        &platform,
        &databases,
        &EventBus::new(8),
        &EmailService::with_default_registry(),
        true,
    )
    .await;

    assert_eq!(report.failures, 0);
    let order = OrderService::new(database.clone())
        .get(&order.id)
        .await
        .unwrap();
    assert_eq!(order.payment_status, PaymentStatus::Captured);
    assert_eq!(order.refunds.len(), 1);
    assert_eq!(order.refunds[0].status, "failed");
}

#[tokio::test]
async fn maintenance_purges_blobs_left_by_asset_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let store = platform
        .create_store(CreateStoreInput::new(
            "Asset Store",
            "asset-store",
            "assets.example.com",
        ))
        .await
        .unwrap();
    StoreProvisioningService::new(platform.clone(), databases.clone())
        .provision(&store.id, None)
        .await
        .unwrap();
    let database = databases.database(&store.id).await.unwrap();

    let storage = LocalStorage::new(dir.path().join("objects"));
    let asset = AssetService::new(database.clone())
        .create_asset(CreateAssetInput {
            filename: "logo.png".to_owned(),
            content_type: "image/png".to_owned(),
            size_bytes: 4,
            storage_key: "assets/logo.png".to_owned(),
            alt: None,
            width: None,
            height: None,
            created_by: None,
        })
        .await
        .unwrap();
    storage
        .put(&asset.storage_key, b"png!", Some("image/png"))
        .await
        .unwrap();

    AssetService::new(database.clone())
        .delete_asset(&asset.id)
        .await
        .unwrap();
    assert_eq!(
        fetch_pending_asset_purges(database.pool(), 10)
            .await
            .unwrap()
            .len(),
        1,
        "deletion must record a pending blob purge"
    );

    let purged = cleanup_asset_purges(&platform, &databases, &storage).await;

    assert_eq!(purged, 1);
    assert!(
        fetch_pending_asset_purges(database.pool(), 10)
            .await
            .unwrap()
            .is_empty(),
        "purged rows must be marked processed"
    );
    assert!(!storage.exists(&asset.storage_key).await.unwrap());
}

#[tokio::test]
async fn maintenance_delivers_pending_store_audits() {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let store = platform
        .create_store(CreateStoreInput::new(
            "Audit Store",
            "audit-store",
            "audit.example.com",
        ))
        .await
        .unwrap();
    StoreProvisioningService::new(platform.clone(), databases.clone())
        .provision(&store.id, None)
        .await
        .unwrap();
    let database = databases.database(&store.id).await.unwrap();

    append_audit_outbox(
        database.pool(),
        &NewAuditOutbox {
            actor_id: Some("user_admin".to_owned()),
            store_id: Some(store.id.to_string()),
            action: "product.updated".to_owned(),
            resource_type: Some("product".to_owned()),
            resource_id: Some("prod_1".to_owned()),
            data: serde_json::json!({"price": 1250}),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        fetch_pending_audits(database.pool(), 10)
            .await
            .unwrap()
            .len(),
        1
    );

    let report = run_maintenance_with_options(
        &platform,
        &databases,
        &EventBus::new(8),
        &EmailService::with_default_registry(),
        true,
    )
    .await;

    assert_eq!(report.audits_dispatched, 1);
    assert_eq!(report.failures, 0);
    assert!(
        fetch_pending_audits(database.pool(), 10)
            .await
            .unwrap()
            .is_empty(),
        "delivered audit rows must be marked processed"
    );

    let logs = platform
        .list_audit_logs(
            Some(&store.id),
            Pagination {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].action, "product.updated");
    assert_eq!(logs[0].actor_id.as_deref(), Some("user_admin"));
    assert_eq!(logs[0].resource_id.as_deref(), Some("prod_1"));
}
