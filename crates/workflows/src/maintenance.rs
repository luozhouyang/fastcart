//! Periodic maintenance over active store databases.

use std::time::Duration;

use fastcart_email::{EmailService, SEND_EMAIL_JOB_TYPE};
use fastcart_infrastructure::{
    EventBus, JobQueue, JobRecord, Storage, StoreDatabase, StoreDatabaseManager,
    fetch_pending_audits, fetch_pending_outbox, mark_audit_failed, mark_audit_processed,
    mark_outbox_failed, mark_outbox_processed,
};
use fastcart_kernel::{AppError, DomainEventEnvelope, OrderId, Pagination, StoreId, now_timestamp};
use fastcart_module_asset::{
    fetch_pending_asset_purges, mark_asset_purge_failed, mark_asset_purge_processed,
};
use fastcart_module_cart::CartService;
use fastcart_module_catalog::CatalogService;
use fastcart_module_checkout::CheckoutService;
use fastcart_module_inquiry::{InquiryFileService, InquiryService};
use fastcart_module_inventory::InventoryService;
use fastcart_module_order::OrderService;
use fastcart_module_webhook::WebhookService;
use fastcart_notifications::{
    NotificationChannelService, NotificationOutcome, NotificationService, RenderContext,
    SEND_CHANNEL_NOTIFICATION_JOB_TYPE,
};
use fastcart_platform::{NewAudit, PlatformStore, StoreStatus};

/// How long a claimed job may go without a renewal before it is reaped.
const JOB_LEASE_SECONDS: i64 = 300;

/// How often a running handler refreshes its job lease.
const JOB_LEASE_RENEW_SECONDS: u64 = 60;

/// How long a channel delivery claim may exist before it is considered
/// abandoned because its job was never enqueued.
const CHANNEL_CLAIM_LEASE_SECONDS: i64 = 300;

/// Counts produced by one maintenance sweep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaintenanceReport {
    /// Carts marked expired.
    pub carts_expired: u64,
    /// Inventory reservations released by expiry.
    pub reservations_released: u64,
    /// Checkout sessions failed by expiry.
    pub checkouts_expired: u64,
    /// Order-refund rows backfilled from succeeded provider refunds.
    pub refunds_reconciled: u64,
    /// Outbox events dispatched.
    pub outbox_dispatched: u64,
    /// Store audits delivered to the platform audit log.
    pub audits_dispatched: u64,
    /// Expired jobs returned to the pending queue.
    pub jobs_reaped: u64,
    /// Background jobs processed.
    pub jobs_processed: u64,
    /// Webhook deliveries accepted by their endpoints.
    pub webhooks_delivered: u64,
    /// Webhook deliveries that exhausted their retry budget.
    pub webhooks_failed: u64,
    /// Notification emails enqueued.
    pub notifications_sent: u64,
    /// Outbox events already delivered for the same notification template.
    pub notifications_duplicates: u64,
    /// Events with no notification rule or no usable recipient.
    pub notifications_skipped: u64,
    /// Notification dispatch failures retained for a later retry.
    pub notifications_failed: u64,
    /// Channel notifications durably queued.
    pub channel_notifications_queued: u64,
    /// Channel deliveries already queued or sent.
    pub channel_notifications_duplicates: u64,
    /// Channel deliveries that failed while rendering or enqueueing.
    pub channel_notifications_failed: u64,
    /// Abandoned channel delivery claims reset for re-dispatch.
    pub channel_claims_recovered: u64,
    /// Expired inquiry uploads removed from storage.
    pub inquiry_files_expired: u64,
    /// Store-level failures encountered.
    pub failures: u64,
}

/// Runs one maintenance sweep across all active stores.
///
/// Expiration work is idempotent, so overlapping sweeps are safe. Outbox
/// dispatch is best-effort; this in-process bus is not durable storage, and
/// integration consumers are expected to be idempotent.
pub async fn run_maintenance(
    platform: &PlatformStore,
    databases: &StoreDatabaseManager,
    events: &EventBus,
    email: &EmailService,
) -> MaintenanceReport {
    run_maintenance_with_options(platform, databases, events, email, false).await
}

/// Runs one maintenance sweep with explicit delivery options.
///
/// `allow_private_webhooks` is intended for tests and local development only.
pub async fn run_maintenance_with_options(
    platform: &PlatformStore,
    databases: &StoreDatabaseManager,
    events: &EventBus,
    email: &EmailService,
    allow_private_webhooks: bool,
) -> MaintenanceReport {
    run_maintenance_with_notifications(
        platform,
        databases,
        events,
        email,
        allow_private_webhooks,
        None,
    )
    .await
}

/// Runs one maintenance sweep with notification deep-link configuration.
pub async fn run_maintenance_with_notifications(
    platform: &PlatformStore,
    databases: &StoreDatabaseManager,
    events: &EventBus,
    email: &EmailService,
    allow_private_webhooks: bool,
    admin_base_url: Option<String>,
) -> MaintenanceReport {
    let mut report = MaintenanceReport::default();
    let notifications = NotificationService::new();
    let channels =
        NotificationChannelService::new().with_local_endpoints_allowed(allow_private_webhooks);
    let render_context = RenderContext {
        admin_base_url,
        locale: "zh-CN".to_owned(),
    };
    let now = now_timestamp();
    let mut offset = 0_i64;
    loop {
        let page = match platform
            .list_stores(Pagination { limit: 100, offset })
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(error = %error, "maintenance could not list stores");
                report.failures += 1;
                break;
            }
        };
        for store in &page.data {
            if store.status != StoreStatus::Active {
                continue;
            }
            let database = match databases.database(&store.id).await {
                Ok(database) => database,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "maintenance could not open store database");
                    report.failures += 1;
                    continue;
                }
            };
            match CartService::new(database.clone()).expire_due(now).await {
                Ok(count) => report.carts_expired += count,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "cart expiry failed");
                    report.failures += 1;
                }
            }
            match InventoryService::new(database.clone())
                .expire_due(now)
                .await
            {
                Ok(count) => report.reservations_released += count,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "reservation expiry failed");
                    report.failures += 1;
                }
            }
            match CheckoutService::new(database.clone()).expire_due(now).await {
                Ok(count) => report.checkouts_expired += count,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "checkout expiry failed");
                    report.failures += 1;
                }
            }
            match OrderService::new(database.clone())
                .reconcile_provider_refunds()
                .await
            {
                Ok(count) => {
                    if count > 0 {
                        tracing::warn!(
                            store_id = %store.id,
                            count,
                            "backfilled order refunds from provider refunds"
                        );
                    }
                    report.refunds_reconciled += count;
                }
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "refund reconciliation failed");
                    report.failures += 1;
                }
            }
            let queue = JobQueue::new(database.pool().clone());
            match queue.reap_expired(JOB_LEASE_SECONDS).await {
                Ok(count) => report.jobs_reaped += count,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "job lease reap failed");
                    report.failures += 1;
                }
            }
            match queue.claim("maintenance", 25).await {
                Ok(jobs) => {
                    run_claimed_jobs(
                        "maintenance",
                        &store.id,
                        &database,
                        &queue,
                        jobs,
                        email,
                        &channels,
                        now,
                        &mut report,
                    )
                    .await;
                }
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "job claim failed");
                    report.failures += 1;
                }
            }
            match NotificationChannelService::recover_abandoned_claims(
                &database,
                now,
                CHANNEL_CLAIM_LEASE_SECONDS,
            )
            .await
            {
                Ok(recovered) => {
                    report.channel_claims_recovered += recovered;
                    if recovered > 0 {
                        tracing::warn!(
                            store_id = %store.id,
                            recovered,
                            "recovered abandoned notification channel claims"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "channel claim recovery failed");
                    report.failures += 1;
                }
            }
            dispatch_outbox(
                &store.id,
                &database,
                &queue,
                events,
                email,
                &notifications,
                &channels,
                &render_context,
                allow_private_webhooks,
                &mut report,
            )
            .await;
            // Audit rows are written by admin mutations before delivery. A
            // failure here keeps the row pending for the next sweep and never
            // aborts the rest of the maintenance pass.
            match fetch_pending_audits(database.pool(), 100).await {
                Ok(records) => {
                    let mut dispatched = 0_u64;
                    for record in records {
                        let entry = NewAudit {
                            actor_id: record.actor_id.clone(),
                            store_id: record.store_id.clone(),
                            action: record.action.clone(),
                            resource_type: record.resource_type.clone(),
                            resource_id: record.resource_id.clone(),
                            data: serde_json::from_str(&record.data)
                                .unwrap_or(serde_json::Value::Null),
                            ip_address: None,
                        };
                        // The outbox id is the platform audit id, so a retry
                        // after a failed "mark processed" cannot duplicate.
                        match platform.record_audit_with_id(&record.id, &entry).await {
                            Ok(()) => {
                                match mark_audit_processed(database.pool(), &record.id).await {
                                    Ok(()) => {
                                        dispatched += 1;
                                        report.audits_dispatched += 1;
                                    }
                                    Err(error) => {
                                        // Delivery is idempotent by id, so this
                                        // only causes a harmless redelivery
                                        // attempt on the next sweep.
                                        tracing::warn!(store_id = %store.id, audit_id = %record.id, error = %error, "audit processed marking failed");
                                        report.failures += 1;
                                    }
                                }
                            }
                            Err(error) => {
                                let attempt = record.attempts.saturating_add(1);
                                tracing::warn!(
                                    store_id = %store.id,
                                    audit_id = %record.id,
                                    action = %record.action,
                                    attempt,
                                    error = %error,
                                    "audit delivery failed"
                                );
                                if let Err(mark_error) = mark_audit_failed(
                                    database.pool(),
                                    &record.id,
                                    attempt,
                                    &error.to_string(),
                                )
                                .await
                                {
                                    tracing::warn!(store_id = %store.id, audit_id = %record.id, error = %mark_error, "audit failure recording failed");
                                }
                                report.failures += 1;
                            }
                        }
                    }
                    if dispatched > 0 {
                        tracing::debug!(
                            store_id = %store.id,
                            audits_dispatched = dispatched,
                            "store audits dispatched"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "audit outbox read failed");
                    report.failures += 1;
                }
            }
            match WebhookService::new(database.clone())
                .with_private_networks_allowed(allow_private_webhooks)
                .deliver_due(now, 10)
                .await
            {
                Ok(delivery) => {
                    report.webhooks_delivered += delivery.delivered;
                    report.webhooks_failed += delivery.failed;
                    if delivery.retried > 0 || delivery.failed > 0 {
                        tracing::warn!(
                            store_id = %store.id,
                            retried = delivery.retried,
                            failed = delivery.failed,
                            "webhook delivery sweep reported failures"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "webhook delivery sweep failed");
                    report.failures += 1;
                }
            }
        }
        offset += page.data.len() as i64;
        if offset >= page.count || page.data.is_empty() {
            break;
        }
    }
    report
}

/// Counts produced by retention cleanup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetentionReport {
    /// Inquiries and their attachments deleted.
    pub inquiries_deleted: u64,
    /// Terminal channel delivery rows deleted.
    pub deliveries_deleted: u64,
    /// Store-level failures encountered.
    pub failures: u64,
}

/// Runs the fast outbox dispatcher and notification job worker.
///
/// This is separate from [`run_maintenance_with_notifications`] so notifications
/// can be delivered on a short cadence without re-running every expiry sweep.
/// Only email and channel notification jobs are claimed here; all other job
/// types remain available to the slower maintenance worker.
pub async fn run_outbox_and_notification_jobs(
    platform: &PlatformStore,
    databases: &StoreDatabaseManager,
    events: &EventBus,
    email: &EmailService,
    allow_private_webhooks: bool,
    admin_base_url: Option<String>,
) -> MaintenanceReport {
    let mut report = MaintenanceReport::default();
    let notifications = NotificationService::new();
    let channels =
        NotificationChannelService::new().with_local_endpoints_allowed(allow_private_webhooks);
    let render_context = RenderContext {
        admin_base_url,
        locale: "zh-CN".to_owned(),
    };
    let now = now_timestamp();
    let mut offset = 0_i64;
    loop {
        let page = match platform
            .list_stores(Pagination { limit: 100, offset })
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(error = %error, "outbox sweep could not list stores");
                report.failures += 1;
                break;
            }
        };
        for store in &page.data {
            if store.status != StoreStatus::Active {
                continue;
            }
            let database = match databases.database(&store.id).await {
                Ok(database) => database,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "outbox sweep could not open store database");
                    report.failures += 1;
                    continue;
                }
            };
            let queue = JobQueue::new(database.pool().clone());
            match NotificationChannelService::recover_abandoned_claims(
                &database,
                now,
                CHANNEL_CLAIM_LEASE_SECONDS,
            )
            .await
            {
                Ok(recovered) => {
                    report.channel_claims_recovered += recovered;
                    if recovered > 0 {
                        tracing::warn!(
                            store_id = %store.id,
                            recovered,
                            "recovered abandoned notification channel claims"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "channel claim recovery failed");
                    report.failures += 1;
                }
            }
            dispatch_outbox(
                &store.id,
                &database,
                &queue,
                events,
                email,
                &notifications,
                &channels,
                &render_context,
                allow_private_webhooks,
                &mut report,
            )
            .await;
            match queue
                .claim_types(
                    "notifications",
                    25,
                    &[SEND_EMAIL_JOB_TYPE, SEND_CHANNEL_NOTIFICATION_JOB_TYPE],
                )
                .await
            {
                Ok(jobs) => {
                    run_claimed_jobs(
                        "notifications",
                        &store.id,
                        &database,
                        &queue,
                        jobs,
                        email,
                        &channels,
                        now,
                        &mut report,
                    )
                    .await;
                }
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "notification job claim failed");
                    report.failures += 1;
                }
            }
        }
        offset += page.data.len() as i64;
        if offset >= page.count || page.data.is_empty() {
            break;
        }
    }
    report
}

/// Deletes inquiries and terminal channel deliveries past their retention.
pub async fn cleanup_retention(
    platform: &PlatformStore,
    databases: &StoreDatabaseManager,
    storage: &dyn Storage,
    inquiry_retention_days: i64,
    delivery_retention_days: i64,
) -> RetentionReport {
    let mut report = RetentionReport::default();
    let now = now_timestamp();
    let inquiry_cutoff = now.saturating_sub(inquiry_retention_days.max(1).saturating_mul(86_400));
    let delivery_cutoff = now.saturating_sub(delivery_retention_days.max(1).saturating_mul(86_400));
    let channels = NotificationChannelService::new();
    let mut offset = 0_i64;
    loop {
        let page = match platform
            .list_stores(Pagination { limit: 100, offset })
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(error = %error, "retention cleanup could not list stores");
                report.failures += 1;
                break;
            }
        };
        for store in &page.data {
            if store.status != StoreStatus::Active {
                continue;
            }
            let database = match databases.database(&store.id).await {
                Ok(database) => database,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "retention cleanup could not open store database");
                    report.failures += 1;
                    continue;
                }
            };
            let inquiries = InquiryService::new(database.clone());
            loop {
                let batch = match inquiries.older_than(inquiry_cutoff, 100).await {
                    Ok(batch) => batch,
                    Err(error) => {
                        tracing::warn!(store_id = %store.id, error = %error, "inquiry retention scan failed");
                        report.failures += 1;
                        break;
                    }
                };
                if batch.is_empty() {
                    break;
                }
                let batch_len = batch.len();
                let mut deleted = 0_u64;
                for inquiry in batch {
                    let files = match inquiries.files_for(&inquiry.id).await {
                        Ok(files) => files,
                        Err(error) => {
                            tracing::warn!(store_id = %store.id, inquiry_id = %inquiry.id, error = %error, "inquiry attachment scan failed");
                            report.failures += 1;
                            continue;
                        }
                    };
                    let mut storage_ok = true;
                    for file in files {
                        if let Err(error) = storage.delete(&file.storage_key).await {
                            tracing::warn!(store_id = %store.id, inquiry_id = %inquiry.id, file_id = %file.id, error = %error, "inquiry attachment retention delete failed");
                            report.failures += 1;
                            storage_ok = false;
                            break;
                        }
                    }
                    if !storage_ok {
                        continue;
                    }
                    match inquiries.delete(&inquiry.id).await {
                        Ok(()) => {
                            deleted += 1;
                            report.inquiries_deleted += 1;
                        }
                        Err(error) => {
                            tracing::warn!(store_id = %store.id, inquiry_id = %inquiry.id, error = %error, "inquiry retention delete failed");
                            report.failures += 1;
                        }
                    }
                }
                // A failing attachment must not make the scan spin forever.
                if deleted == 0 || batch_len < 100 {
                    break;
                }
            }
            match channels
                .delete_deliveries_before(&database, delivery_cutoff)
                .await
            {
                Ok(deleted) => report.deliveries_deleted += deleted,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "notification delivery retention cleanup failed");
                    report.failures += 1;
                }
            }
        }
        offset += page.data.len() as i64;
        if offset >= page.count || page.data.is_empty() {
            break;
        }
    }
    report
}

async fn process_job(
    store_id: &StoreId,
    database: &StoreDatabase,
    job: &JobRecord,
    email: &EmailService,
    channels: &NotificationChannelService,
    now: i64,
) -> Result<(), AppError> {
    match job.job_type.as_str() {
        "expire_cart" => CartService::new(database.clone())
            .expire_due(now)
            .await
            .map(|_| ()),
        "expire_reservation" => InventoryService::new(database.clone())
            .expire_due(now)
            .await
            .map(|_| ()),
        "reindex_search" => CatalogService::new(database.clone())
            .reindex_search()
            .await
            .map(|_| ()),
        "dispatch_outbox" => Ok(()),
        "send_email" => match serde_json::from_str(&job.payload) {
            Ok(payload) => email
                .process_job_for_store(store_id, &payload)
                .await
                .map(|_| ())
                .map_err(AppError::from),
            Err(error) => Err(AppError::validation(format!(
                "invalid send_email payload: {error}"
            ))),
        },
        SEND_CHANNEL_NOTIFICATION_JOB_TYPE => match serde_json::from_str(&job.payload) {
            Ok(payload) => match channels.process_job(database, &payload).await {
                Ok(_) => Ok(()),
                Err(fastcart_notifications::ChannelError::Retryable(error)) => {
                    Err(AppError::ExternalService(error))
                }
                Err(fastcart_notifications::ChannelError::Permanent(error)) => {
                    tracing::warn!(
                        store_id = %store_id,
                        job_id = %job.id,
                        error = %error,
                        "channel notification failed permanently"
                    );
                    Ok(())
                }
            },
            Err(error) => Err(AppError::validation(format!(
                "invalid channel notification payload: {error}"
            ))),
        },
        other => Err(AppError::validation(format!(
            "no handler for job type {other}"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_claimed_jobs(
    worker_id: &str,
    store_id: &StoreId,
    database: &StoreDatabase,
    queue: &JobQueue,
    jobs: Vec<JobRecord>,
    email: &EmailService,
    channels: &NotificationChannelService,
    now: i64,
    report: &mut MaintenanceReport,
) {
    for job in jobs {
        // Keep the lease fresh while a handler runs so a slow delivery is not
        // handed to another worker by reap_expired.
        let lease = spawn_lease_heartbeat(queue, &job.id, worker_id);
        let outcome = process_job(store_id, database, &job, email, channels, now).await;
        drop(lease);
        match outcome {
            Ok(()) => match queue.complete(&job.id, worker_id).await {
                Ok(true) => report.jobs_processed += 1,
                Ok(false) => {
                    tracing::warn!(
                        store_id = %store_id,
                        job_id = %job.id,
                        "job completion rejected because the lease was lost"
                    );
                    report.failures += 1;
                }
                Err(error) => {
                    tracing::warn!(store_id = %store_id, error = %error, "job completion failed");
                    report.failures += 1;
                }
            },
            Err(error) => {
                tracing::warn!(store_id = %store_id, job_type = %job.job_type, error = %error, "job failed");
                let _ = queue.fail(&job.id, &error.to_string(), worker_id).await;
                report.failures += 1;
            }
        }
    }
}

/// Aborts the renewal task when the handler finishes or unwinds.
struct LeaseHeartbeat(tokio::task::JoinHandle<()>);

impl Drop for LeaseHeartbeat {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Refreshes one job's lease periodically until the guard is dropped.
fn spawn_lease_heartbeat(queue: &JobQueue, job_id: &str, worker_id: &str) -> LeaseHeartbeat {
    let queue = queue.clone();
    let job_id = job_id.to_owned();
    let worker_id = worker_id.to_owned();
    LeaseHeartbeat(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(JOB_LEASE_RENEW_SECONDS));
        // The first interval tick is immediate; consume it before waiting.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match queue
                .renew_lease(&job_id, &worker_id, JOB_LEASE_SECONDS)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(
                        job_id = %job_id,
                        "job lease renewal rejected because the lease was lost"
                    );
                    break;
                }
                Err(error) => {
                    tracing::warn!(job_id = %job_id, error = %error, "job lease renewal failed");
                }
            }
        }
    }))
}

/// Applies a provider-reported refund failure to the local order ledger.
///
/// Payments without an order are a no-op. Missing fields are consumer errors so
/// the outbox row is retried instead of being silently dropped.
async fn reconcile_failed_refund(
    database: &StoreDatabase,
    event: &DomainEventEnvelope,
) -> Result<(), AppError> {
    let Some(order_id) = event
        .data
        .get("order_id")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(());
    };
    let payment_id = event
        .data
        .get("payment_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::validation("payment.refund_failed is missing payment_id"))?;
    let amount = event
        .data
        .get("amount")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| AppError::validation("payment.refund_failed is missing amount"))?;
    OrderService::new(database.clone())
        .mark_payment_refund_failed(&OrderId::from_string(order_id), payment_id, amount)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_outbox(
    store_id: &StoreId,
    database: &StoreDatabase,
    queue: &JobQueue,
    events: &EventBus,
    email: &EmailService,
    notifications: &NotificationService,
    channels: &NotificationChannelService,
    render_context: &RenderContext,
    allow_private_webhooks: bool,
    report: &mut MaintenanceReport,
) {
    match fetch_pending_outbox(database.pool(), 100).await {
        Ok(records) => {
            let webhooks = WebhookService::new(database.clone())
                .with_private_networks_allowed(allow_private_webhooks);
            for record in records {
                let mut fan_out_failed = false;
                let mut notification_failed = false;
                let mut channel_failed = false;
                let mut refund_failed = false;
                if let Some(event) = record.to_envelope(Some(store_id.clone())) {
                    // Fan out before publishing so a database failure keeps the
                    // outbox row retryable.
                    if let Err(error) = webhooks.fan_out(&event).await {
                        tracing::warn!(store_id = %store_id, event_id = %event.id, error = %error, "webhook fan-out failed");
                        report.failures += 1;
                        fan_out_failed = true;
                    } else {
                        let _ = events.publish(event.clone());
                    }
                    // Notifications are derived consumers: a failure is retried
                    // with the outbox row but never aborts the sweep.
                    match notifications.dispatch(database, email, queue, &event).await {
                        Ok(NotificationOutcome::Sent { .. }) => {
                            report.notifications_sent += 1;
                        }
                        Ok(NotificationOutcome::Duplicate { .. }) => {
                            report.notifications_duplicates += 1;
                        }
                        Ok(
                            NotificationOutcome::Unsupported
                            | NotificationOutcome::MissingRecipient { .. },
                        ) => {
                            report.notifications_skipped += 1;
                        }
                        Err(error) => {
                            tracing::warn!(
                                store_id = %store_id,
                                event_id = %event.id,
                                event_type = %event.event_type,
                                error = %error,
                                "notification dispatch failed"
                            );
                            report.failures += 1;
                            report.notifications_failed += 1;
                            notification_failed = true;
                        }
                    }
                    // Channel notifications are independent of email: rendering
                    // or transport failures are isolated to one channel.
                    match channels
                        .dispatch(database, queue, &event, render_context)
                        .await
                    {
                        Ok(channel_report) => {
                            report.channel_notifications_queued += channel_report.queued;
                            report.channel_notifications_duplicates += channel_report.duplicates;
                            report.channel_notifications_failed += channel_report.failed;
                        }
                        Err(error) => {
                            tracing::warn!(
                                store_id = %store_id,
                                event_id = %event.id,
                                event_type = %event.event_type,
                                error = %error,
                                "channel notification dispatch failed"
                            );
                            report.failures += 1;
                            report.channel_notifications_failed += 1;
                            channel_failed = true;
                        }
                    }
                    // A provider-reported refund failure must revert the local
                    // order ledger before the outbox row is marked processed.
                    // A consumer error keeps the row retryable.
                    if event.event_type == "payment.refund_failed" {
                        match reconcile_failed_refund(database, &event).await {
                            Ok(()) => {}
                            Err(error) => {
                                tracing::warn!(
                                    store_id = %store_id,
                                    event_id = %event.id,
                                    error = %error,
                                    "order refund failure reconciliation failed"
                                );
                                report.failures += 1;
                                refund_failed = true;
                            }
                        }
                    }
                }
                if fan_out_failed || notification_failed || channel_failed || refund_failed {
                    let reason = if fan_out_failed && notification_failed {
                        "webhook fan-out and notification dispatch failed"
                    } else if fan_out_failed {
                        "webhook fan-out failed"
                    } else if notification_failed && channel_failed {
                        "email and channel notification dispatch failed"
                    } else if notification_failed {
                        "notification dispatch failed"
                    } else if channel_failed {
                        "channel notification dispatch failed"
                    } else {
                        "order refund failure reconciliation failed"
                    };
                    if let Err(error) =
                        mark_outbox_failed(database.pool(), &record.id, reason).await
                    {
                        tracing::warn!(store_id = %store_id, error = %error, "outbox failure recording failed");
                    }
                    continue;
                }
                if let Err(error) = mark_outbox_processed(database.pool(), &record.id).await {
                    tracing::warn!(store_id = %store_id, error = %error, "outbox dispatch failed");
                    let _ =
                        mark_outbox_failed(database.pool(), &record.id, &error.to_string()).await;
                    report.failures += 1;
                } else {
                    report.outbox_dispatched += 1;
                }
            }
        }
        Err(error) => {
            tracing::warn!(store_id = %store_id, error = %error, "outbox read failed");
            report.failures += 1;
        }
    }
}

/// Removes expired pending inquiry uploads from object storage and marks them.
///
/// Storage deletion is best-effort: a failure keeps the row pending so the next
/// sweep can retry instead of leaking the object silently.
pub async fn cleanup_inquiry_files(
    platform: &PlatformStore,
    databases: &StoreDatabaseManager,
    storage: &dyn Storage,
) -> u64 {
    let mut removed = 0_u64;
    let mut offset = 0_i64;
    loop {
        let page = match platform
            .list_stores(Pagination { limit: 100, offset })
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(error = %error, "inquiry cleanup could not list stores");
                break;
            }
        };
        for store in &page.data {
            if store.status != StoreStatus::Active {
                continue;
            }
            let database = match databases.database(&store.id).await {
                Ok(database) => database,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "inquiry cleanup could not open store database");
                    continue;
                }
            };
            let service = InquiryFileService::new(database);
            let files = match service.expired_pending(100).await {
                Ok(files) => files,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "expired inquiry upload scan failed");
                    continue;
                }
            };
            for file in files {
                match storage.delete(&file.storage_key).await {
                    Ok(()) => {}
                    Err(error) => {
                        tracing::warn!(store_id = %store.id, file_id = %file.id, error = %error, "inquiry upload cleanup failed");
                        continue;
                    }
                }
                match service.mark_expired(&file.id).await {
                    Ok(()) => removed += 1,
                    Err(error) => {
                        tracing::warn!(store_id = %store.id, file_id = %file.id, error = %error, "inquiry upload state update failed");
                    }
                }
            }
        }
        offset += page.data.len() as i64;
        if offset >= page.count || page.data.is_empty() {
            break;
        }
    }
    removed
}

/// Removes asset blobs whose metadata was deleted but storage cleanup failed.
///
/// Deleting asset metadata commits a durable purge obligation in the store
/// database. A storage failure keeps the row pending with an incremented
/// attempt count so the next sweep retries instead of leaking the object.
pub async fn cleanup_asset_purges(
    platform: &PlatformStore,
    databases: &StoreDatabaseManager,
    storage: &dyn Storage,
) -> u64 {
    let mut purged = 0_u64;
    let mut offset = 0_i64;
    loop {
        let page = match platform
            .list_stores(Pagination { limit: 100, offset })
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(error = %error, "asset purge cleanup could not list stores");
                break;
            }
        };
        for store in &page.data {
            if store.status != StoreStatus::Active {
                continue;
            }
            let database = match databases.database(&store.id).await {
                Ok(database) => database,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "asset purge cleanup could not open store database");
                    continue;
                }
            };
            let purges = match fetch_pending_asset_purges(database.pool(), 100).await {
                Ok(purges) => purges,
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "asset purge scan failed");
                    continue;
                }
            };
            for purge in purges {
                match storage.delete(&purge.storage_key).await {
                    Ok(()) => match mark_asset_purge_processed(database.pool(), &purge.id).await {
                        Ok(()) => purged += 1,
                        Err(error) => {
                            tracing::warn!(store_id = %store.id, purge_id = %purge.id, error = %error, "asset purge state update failed");
                        }
                    },
                    Err(error) => {
                        let attempt = purge.attempts.saturating_add(1);
                        tracing::warn!(
                            store_id = %store.id,
                            purge_id = %purge.id,
                            attempt,
                            error = %error,
                            "asset blob purge failed"
                        );
                        if let Err(mark_error) = mark_asset_purge_failed(
                            database.pool(),
                            &purge.id,
                            attempt,
                            &error.to_string(),
                        )
                        .await
                        {
                            tracing::warn!(store_id = %store.id, purge_id = %purge.id, error = %mark_error, "asset purge failure recording failed");
                        }
                    }
                }
            }
        }
        offset += page.data.len() as i64;
        if offset >= page.count || page.data.is_empty() {
            break;
        }
    }
    purged
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fastcart_email::{EmailService, SEND_EMAIL_JOB_TYPE, SendEmailJob};
    use fastcart_infrastructure::{
        DatabaseOptions, EventBus, JobQueue, LocalStorage, NewAuditOutbox, Storage,
        StoreDatabaseManager, append_audit_outbox, append_outbox_event, fetch_pending_audits,
    };
    use fastcart_kernel::{DomainEventEnvelope, Pagination, PaymentId};
    use fastcart_module_asset::{AssetService, CreateAssetInput, fetch_pending_asset_purges};
    use fastcart_module_order::{
        CreateOrderInput, CreateOrderItemInput, OrderService, PaymentStatus, RefundInput,
    };
    use fastcart_platform::{CreateStoreInput, PlatformStore, StoreProvisioningService};

    use super::{cleanup_asset_purges, run_maintenance_with_options};

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
}
