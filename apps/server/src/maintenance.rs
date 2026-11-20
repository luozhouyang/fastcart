//! Periodic maintenance worker for expiry and outbox dispatch.

use std::sync::Arc;
use std::time::Duration;

use fastcart_api::AppContext;
use fastcart_kernel::Pagination;
use fastcart_plugin::PluginDispatch;

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
            let asset_purges = fastcart_workflows::cleanup_asset_purges(
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
                || asset_purges > 0
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
                    asset_purges_processed = asset_purges,
                    inquiries_deleted = retention.inquiries_deleted,
                    deliveries_deleted = retention.deliveries_deleted,
                    retention_failures = retention.failures,
                    failures = report.failures,
                    "maintenance sweep completed"
                );
            }
        }
        if tick.is_multiple_of(360) {
            let pruned = prune_plugin_artifacts(&context).await;
            if pruned > 0 {
                tracing::info!(pruned, "plugin artifact sweep completed");
            }
        }
        tick = tick.wrapping_add(1);
    }
}

/// Runs the plugin dispatcher every two seconds so event delivery stays under
/// the five-second target; log retention runs every ten minutes.
pub async fn plugin_dispatch_loop(context: Arc<AppContext>) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    interval.tick().await;
    let mut tick = 0_u64;
    loop {
        interval.tick().await;
        run_plugin_dispatch(&context).await;
        if tick.is_multiple_of(300) {
            retain_plugin_logs(&context).await;
        }
        tick = tick.wrapping_add(1);
    }
}

async fn retain_plugin_logs(context: &AppContext) {
    let mut offset = 0_i64;
    loop {
        let Ok(page) = context
            .platform
            .list_stores(Pagination { limit: 100, offset })
            .await
        else {
            return;
        };
        for store in &page.data {
            let Ok(database) = context.databases.database(&store.id).await else {
                continue;
            };
            if let Err(error) =
                fastcart_plugin::dispatch::prune_logs(&database, fastcart_kernel::now_timestamp())
                    .await
            {
                tracing::warn!(store = %store.id, error = %error, "plugin log retention failed");
            }
        }
        if page.data.len() < 100 {
            break;
        }
        offset += 100;
    }
}

/// Fans out plugin events and runs due plugin deliveries/jobs for every store.
async fn run_plugin_dispatch(context: &AppContext) {
    let Some(host) = &context.plugin_host else {
        return;
    };
    let dispatcher = PluginDispatch::new(host.clone(), context.plugin_tokens.clone());
    let mut offset = 0_i64;
    loop {
        let page = match context
            .platform
            .list_stores(Pagination { limit: 100, offset })
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(error = %error, "plugin dispatch could not list stores");
                return;
            }
        };
        for store in &page.data {
            let Ok(database) = context.databases.database(&store.id).await else {
                continue;
            };
            let report = dispatcher.run(&database, 25).await;
            if let Err(error) =
                fastcart_plugin::dispatch::prune_logs(&database, fastcart_kernel::now_timestamp())
                    .await
            {
                tracing::warn!(store = %store.id, error = %error, "plugin log retention failed");
            }
            if report != fastcart_plugin::DispatchReport::default() {
                tracing::info!(
                    store = %store.id,
                    fanned_out = report.fanned_out,
                    delivered = report.delivered,
                    retried = report.retried,
                    dead = report.dead,
                    jobs_enqueued = report.jobs_enqueued,
                    jobs_done = report.jobs_done,
                    jobs_retried = report.jobs_retried,
                    jobs_failed = report.jobs_failed,
                    "plugin dispatch completed"
                );
            }
        }
        if page.data.len() < 100 {
            break;
        }
        offset += 100;
    }
}

/// Deletes plugin artifacts that no store references, including rollback targets.
async fn prune_plugin_artifacts(context: &AppContext) -> usize {
    let mut referenced = std::collections::HashSet::new();
    let mut offset = 0_i64;
    loop {
        let pagination = Pagination { limit: 100, offset };
        let page = match context.platform.list_stores(pagination).await {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(error = %error, "plugin artifact sweep could not list stores");
                return 0;
            }
        };
        for store in &page.data {
            let Ok(database) = context.databases.database(&store.id).await else {
                continue;
            };
            match fastcart_plugin::service::referenced_digests(&database).await {
                Ok(digests) => referenced.extend(digests),
                Err(error) => {
                    tracing::warn!(store = %store.id, error = %error, "plugin artifact sweep could not read store references");
                }
            }
        }
        if page.data.len() < 100 {
            break;
        }
        offset += 100;
    }
    match context
        .plugin_artifacts
        .prune(&referenced, std::time::Duration::from_secs(3_600))
    {
        Ok(removed) => removed.len(),
        Err(error) => {
            tracing::warn!(error = %error, "plugin artifact sweep failed");
            0
        }
    }
}
