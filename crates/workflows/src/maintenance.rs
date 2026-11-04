//! Periodic maintenance over active store databases.

use fastcart_email::{EmailService, SEND_EMAIL_JOB_TYPE};
use fastcart_infrastructure::{
    EventBus, JobQueue, JobRecord, Storage, StoreDatabase, StoreDatabaseManager,
    fetch_pending_audits, fetch_pending_outbox, mark_audit_failed, mark_audit_processed,
    mark_outbox_failed, mark_outbox_processed,
};
use fastcart_kernel::{AppError, Pagination, StoreId, now_timestamp};
use fastcart_module_cart::CartService;
use fastcart_module_catalog::CatalogService;
use fastcart_module_checkout::CheckoutService;
use fastcart_module_inquiry::{InquiryFileService, InquiryService};
use fastcart_module_inventory::InventoryService;
use fastcart_module_webhook::WebhookService;
use fastcart_notifications::{
    NotificationChannelService, NotificationOutcome, NotificationService, RenderContext,
    SEND_CHANNEL_NOTIFICATION_JOB_TYPE,
};
use fastcart_platform::{NewAudit, PlatformStore, StoreStatus};

/// Counts produced by one maintenance sweep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaintenanceReport {
    /// Carts marked expired.
    pub carts_expired: u64,
    /// Inventory reservations released by expiry.
    pub reservations_released: u64,
    /// Checkout sessions failed by expiry.
    pub checkouts_expired: u64,
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
            let queue = JobQueue::new(database.pool().clone());
            match queue.reap_expired(300).await {
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
                        match platform.record_audit(entry).await {
                            Ok(()) => {
                                match mark_audit_processed(database.pool(), &record.id).await {
                                    Ok(()) => {
                                        dispatched += 1;
                                        report.audits_dispatched += 1;
                                    }
                                    Err(error) => {
                                        // Delivery succeeded; a failed mark
                                        // only causes a harmless duplicate on
                                        // the next sweep.
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
                .process_job(&payload)
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
        let outcome = process_job(store_id, database, &job, email, channels, now).await;
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
                }
                if fan_out_failed || notification_failed || channel_failed {
                    let reason = if fan_out_failed && notification_failed {
                        "webhook fan-out and notification dispatch failed"
                    } else if fan_out_failed {
                        "webhook fan-out failed"
                    } else if notification_failed && channel_failed {
                        "email and channel notification dispatch failed"
                    } else if notification_failed {
                        "notification dispatch failed"
                    } else {
                        "channel notification dispatch failed"
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fastcart_email::{EmailService, SEND_EMAIL_JOB_TYPE, SendEmailJob};
    use fastcart_infrastructure::{
        DatabaseOptions, EventBus, JobQueue, NewAuditOutbox, StoreDatabaseManager,
        append_audit_outbox, append_outbox_event, fetch_pending_audits,
    };
    use fastcart_kernel::{DomainEventEnvelope, Pagination};
    use fastcart_platform::{CreateStoreInput, PlatformStore, StoreProvisioningService};

    use super::run_maintenance_with_options;

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
