//! Periodic maintenance over active store databases.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

use fastcart_email::{EmailService, SEND_EMAIL_JOB_TYPE};
use fastcart_infrastructure::{EventBus, JobQueue, StoreDatabaseManager};
use fastcart_kernel::{Pagination, now_timestamp};
use fastcart_notifications::{
    NotificationChannelService, NotificationService, RenderContext,
    SEND_CHANNEL_NOTIFICATION_JOB_TYPE,
};
use fastcart_platform::{PlatformStore, StoreStatus};

mod jobs;
mod outbox;
mod retention;
mod store_sweep;

pub use retention::{cleanup_asset_purges, cleanup_inquiry_files, cleanup_retention};

use jobs::run_claimed_jobs;
use outbox::dispatch_outbox;

/// How long a claimed job may go without a renewal before it is reaped.
const JOB_LEASE_SECONDS: i64 = 300;

/// How many stores one sweep keeps in flight.
const STORE_MAINTENANCE_CONCURRENCY: usize = 4;

/// How often a worker renews the lease of the job it is running.
const JOB_LEASE_RENEW_SECONDS: u64 = 60;

/// How long a claimed notification channel may go without a renewal.
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
    /// Promotion usage claims given back by abandoned checkouts.
    pub promotion_claims_released: u64,
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

/// Shared inputs for one store's maintenance pass.
struct StoreSweep<'a> {
    platform: &'a PlatformStore,
    databases: &'a StoreDatabaseManager,
    events: &'a EventBus,
    email: &'a EmailService,
    notifications: &'a NotificationService,
    channels: &'a NotificationChannelService,
    render_context: &'a RenderContext,
    allow_private_webhooks: bool,
    now: i64,
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

impl MaintenanceReport {
    /// Adds one store's counts into this report.
    fn absorb(&mut self, other: MaintenanceReport) {
        self.carts_expired += other.carts_expired;
        self.reservations_released += other.reservations_released;
        self.checkouts_expired += other.checkouts_expired;
        self.promotion_claims_released += other.promotion_claims_released;
        self.refunds_reconciled += other.refunds_reconciled;
        self.outbox_dispatched += other.outbox_dispatched;
        self.audits_dispatched += other.audits_dispatched;
        self.jobs_reaped += other.jobs_reaped;
        self.jobs_processed += other.jobs_processed;
        self.webhooks_delivered += other.webhooks_delivered;
        self.webhooks_failed += other.webhooks_failed;
        self.notifications_sent += other.notifications_sent;
        self.notifications_duplicates += other.notifications_duplicates;
        self.notifications_skipped += other.notifications_skipped;
        self.notifications_failed += other.notifications_failed;
        self.channel_notifications_queued += other.channel_notifications_queued;
        self.channel_notifications_duplicates += other.channel_notifications_duplicates;
        self.channel_notifications_failed += other.channel_notifications_failed;
        self.channel_claims_recovered += other.channel_claims_recovered;
        self.inquiry_files_expired += other.inquiry_files_expired;
        self.failures += other.failures;
    }
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
    let sweep = StoreSweep {
        platform,
        databases,
        events,
        email,
        notifications: &notifications,
        channels: &channels,
        render_context: &render_context,
        allow_private_webhooks,
        now: now_timestamp(),
    };
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
        // Stores own independent databases, so their sweeps overlap up to a
        // fixed bound while each store's own steps stay strictly ordered.
        let store_reports = join_limited(
            page.data
                .iter()
                .filter(|store| store.status == StoreStatus::Active)
                .map(|store| sweep.maintain_store(store)),
            STORE_MAINTENANCE_CONCURRENCY,
        )
        .await;
        for store_report in store_reports {
            report.absorb(store_report);
        }
        offset += page.data.len() as i64;
        if offset >= page.count || page.data.is_empty() {
            break;
        }
    }
    report
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

/// Runs every future concurrently, keeping at most `limit` in flight.
///
/// Results arrive in completion order. This is the part of
/// `futures::stream::buffer_unordered` a sweep needs, rolled locally so store
/// futures can borrow their inputs instead of spawning.
async fn join_limited<F, T>(futures: impl IntoIterator<Item = F>, limit: usize) -> Vec<T>
where
    F: Future<Output = T>,
{
    let limit = limit.max(1);
    let mut queued: VecDeque<Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut in_flight: Vec<Pin<Box<F>>> = Vec::new();
    let mut finished: Vec<T> = Vec::new();
    std::future::poll_fn(move |context| {
        loop {
            while in_flight.len() < limit && !queued.is_empty() {
                if let Some(future) = queued.pop_front() {
                    in_flight.push(future);
                }
            }
            let mut progressed = false;
            let mut index = 0;
            while index < in_flight.len() {
                match in_flight[index].as_mut().poll(context) {
                    Poll::Ready(value) => {
                        finished.push(value);
                        in_flight.swap_remove(index);
                        progressed = true;
                    }
                    Poll::Pending => index += 1,
                }
            }
            if !progressed {
                break;
            }
        }
        if in_flight.is_empty() && queued.is_empty() {
            Poll::Ready(std::mem::take(&mut finished))
        } else {
            Poll::Pending
        }
    })
    .await
}

#[cfg(test)]
mod tests;
