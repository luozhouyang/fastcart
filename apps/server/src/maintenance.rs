//! Periodic maintenance worker for expiry and outbox dispatch.

use std::sync::Arc;
use std::time::Duration;

use fastcart_api::AppContext;

/// Runs store maintenance every minute until the process exits.
pub async fn maintenance_loop(context: Arc<AppContext>) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.tick().await;
    loop {
        interval.tick().await;
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
        if report != fastcart_workflows::MaintenanceReport::default() || expired_files > 0 {
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
                failures = report.failures,
                "maintenance sweep completed"
            );
        }
    }
}
