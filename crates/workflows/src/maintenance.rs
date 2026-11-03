//! Periodic maintenance over active store databases.

use fastcart_email::EmailService;
use fastcart_infrastructure::{
    EventBus, JobQueue, StoreDatabaseManager, fetch_pending_outbox, mark_outbox_failed,
    mark_outbox_processed,
};
use fastcart_kernel::{Pagination, now_timestamp};
use fastcart_module_cart::CartService;
use fastcart_module_catalog::CatalogService;
use fastcart_module_checkout::CheckoutService;
use fastcart_module_inventory::InventoryService;
use fastcart_module_webhook::WebhookService;
use fastcart_platform::{PlatformStore, StoreStatus};

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
    /// Expired jobs returned to the pending queue.
    pub jobs_reaped: u64,
    /// Background jobs processed.
    pub jobs_processed: u64,
    /// Webhook deliveries accepted by their endpoints.
    pub webhooks_delivered: u64,
    /// Webhook deliveries that exhausted their retry budget.
    pub webhooks_failed: u64,
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
    let mut report = MaintenanceReport::default();
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
                    for job in jobs {
                        let outcome = match job.job_type.as_str() {
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
                                    .map_err(fastcart_kernel::AppError::from),
                                Err(error) => Err(fastcart_kernel::AppError::validation(format!(
                                    "invalid send_email payload: {error}"
                                ))),
                            },
                            other => Err(fastcart_kernel::AppError::validation(format!(
                                "no handler for job type {other}"
                            ))),
                        };
                        match outcome {
                            Ok(()) => match queue.complete(&job.id, "maintenance").await {
                                Ok(true) => report.jobs_processed += 1,
                                Ok(false) => {
                                    tracing::warn!(
                                        store_id = %store.id,
                                        job_id = %job.id,
                                        "job completion rejected because the lease was lost"
                                    );
                                    report.failures += 1;
                                }
                                Err(error) => {
                                    tracing::warn!(store_id = %store.id, error = %error, "job completion failed");
                                    report.failures += 1;
                                }
                            },
                            Err(error) => {
                                tracing::warn!(store_id = %store.id, job_type = %job.job_type, error = %error, "job failed");
                                let _ =
                                    queue.fail(&job.id, &error.to_string(), "maintenance").await;
                                report.failures += 1;
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "job claim failed");
                    report.failures += 1;
                }
            }
            match fetch_pending_outbox(database.pool(), 100).await {
                Ok(records) => {
                    let webhooks = WebhookService::new(database.clone())
                        .with_private_networks_allowed(allow_private_webhooks);
                    for record in records {
                        let mut fan_out_failed = false;
                        if let Some(event) = record.to_envelope(Some(store.id.clone())) {
                            // Fan out before publishing so a database failure
                            // keeps the outbox row retryable.
                            if let Err(error) = webhooks.fan_out(&event).await {
                                tracing::warn!(store_id = %store.id, event_id = %event.id, error = %error, "webhook fan-out failed");
                                report.failures += 1;
                                fan_out_failed = true;
                            } else {
                                let _ = events.publish(event);
                            }
                        }
                        if fan_out_failed {
                            if let Err(error) = mark_outbox_failed(
                                database.pool(),
                                &record.id,
                                "webhook fan-out failed",
                            )
                            .await
                            {
                                tracing::warn!(store_id = %store.id, error = %error, "outbox failure recording failed");
                            }
                            continue;
                        }
                        if let Err(error) = mark_outbox_processed(database.pool(), &record.id).await
                        {
                            tracing::warn!(store_id = %store.id, error = %error, "outbox dispatch failed");
                            let _ =
                                mark_outbox_failed(database.pool(), &record.id, &error.to_string())
                                    .await;
                            report.failures += 1;
                        } else {
                            report.outbox_dispatched += 1;
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(store_id = %store.id, error = %error, "outbox read failed");
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
