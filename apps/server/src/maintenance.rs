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
        let report = fastcart_workflows::run_maintenance(
            &context.platform,
            &context.databases,
            &context.events,
            &context.email,
        )
        .await;
        if report != fastcart_workflows::MaintenanceReport::default() {
            tracing::info!(
                carts_expired = report.carts_expired,
                reservations_released = report.reservations_released,
                checkouts_expired = report.checkouts_expired,
                outbox_dispatched = report.outbox_dispatched,
                jobs_processed = report.jobs_processed,
                webhooks_delivered = report.webhooks_delivered,
                webhooks_failed = report.webhooks_failed,
                failures = report.failures,
                "maintenance sweep completed"
            );
        }
    }
}
