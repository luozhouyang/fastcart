//! Periodic maintenance worker for expiry and outbox dispatch.

use std::sync::Arc;
use std::time::Duration;

use fastcart_api::AppContext;

/// Runs a fast notification loop every ten seconds and the full maintenance
/// sweep every sixty seconds until the process exits.
pub async fn maintenance_loop(context: Arc<AppContext>) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    interval.tick().await;
    let mut tick = 0_u64;
    loop {
        interval.tick().await;
        let outbox = fastcart_workflows::run_outbox_and_notification_jobs(
            &context.platform,
            &context.databases,
            &context.events,
            &context.email,
            false,
            context.config.admin_base_url.clone(),
        )
        .await;
        if outbox != fastcart_workflows::MaintenanceReport::default() {
            tracing::info!(
                outbox_dispatched = outbox.outbox_dispatched,
                jobs_processed = outbox.jobs_processed,
                notifications_sent = outbox.notifications_sent,
                channel_notifications_queued = outbox.channel_notifications_queued,
                channel_notifications_duplicates = outbox.channel_notifications_duplicates,
                channel_notifications_failed = outbox.channel_notifications_failed,
                failures = outbox.failures,
                "notification sweep completed"
            );
        }

        // The slower sweep owns expiry, audit delivery, webhook retries and
        // retention cleanup. Outbox work is idempotent and is normally already
        // done by the ten-second loop, so a repeated pass is harmless.
        if tick.is_multiple_of(6) {
            let report = fastcart_workflows::run_maintenance_with_notifications(
                &context.platform,
                &context.databases,
                &context.events,
                &context.email,
                false,
                context.config.admin_base_url.clone(),
            )
            .await;
            let expired_files = fastcart_workflows::cleanup_inquiry_files(
                &context.platform,
                &context.databases,
                context.storage.as_ref(),
            )
            .await;
            let retention = fastcart_workflows::cleanup_retention(
                &context.platform,
                &context.databases,
                context.storage.as_ref(),
                context.config.inquiry_retention_days,
                context.config.notification_retention_days,
            )
            .await;
            if report != fastcart_workflows::MaintenanceReport::default()
                || expired_files > 0
                || retention != fastcart_workflows::RetentionReport::default()
            {
                tracing::info!(
                    carts_expired = report.carts_expired,
                    reservations_released = report.reservations_released,
                    checkouts_expired = report.checkouts_expired,
                    outbox_dispatched = report.outbox_dispatched,
                    jobs_reaped = report.jobs_reaped,
                    jobs_processed = report.jobs_processed,
                    webhooks_delivered = report.webhooks_delivered,
                    webhooks_failed = report.webhooks_failed,
                    notifications_sent = report.notifications_sent,
                    channel_notifications_queued = report.channel_notifications_queued,
                    channel_notifications_duplicates = report.channel_notifications_duplicates,
                    channel_notifications_failed = report.channel_notifications_failed,
                    inquiry_files_expired = expired_files,
                    inquiries_deleted = retention.inquiries_deleted,
                    deliveries_deleted = retention.deliveries_deleted,
                    retention_failures = retention.failures,
                    failures = report.failures,
                    "maintenance sweep completed"
                );
            }
        }
        tick = tick.wrapping_add(1);
    }
}
