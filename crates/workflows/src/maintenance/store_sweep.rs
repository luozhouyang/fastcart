//! One store's maintenance pass.

use fastcart_infrastructure::{
    JobQueue, StoreDatabase, fetch_pending_audits, mark_audit_failed, mark_audit_processed,
};
use fastcart_kernel::AppError;
use fastcart_module_cart::CartService;
use fastcart_module_checkout::CheckoutService;
use fastcart_module_inventory::InventoryService;
use fastcart_module_order::OrderService;
use fastcart_module_promotion::PromotionService;
use fastcart_module_webhook::WebhookService;
use fastcart_notifications::NotificationChannelService;
use fastcart_platform::{NewAudit, Store};

use super::jobs::run_claimed_jobs;
use super::outbox::dispatch_outbox;
use super::*;

impl StoreSweep<'_> {
    /// Runs every maintenance step for one active store, in its original order.
    /// Gives back promotion claims held by checkouts that can never complete.
    ///
    /// The promotion module owns usage rows and must not read the checkout
    /// table; the checkout module owns sessions and must not read usage rows.
    /// This is the layer allowed to see both, so it does the join — and it keeps
    /// the work bounded by the outstanding claims rather than by checkout
    /// history, since only checkouts that still hold a claim are considered.
    async fn release_abandoned_promotion_usage(
        &self,
        database: &StoreDatabase,
    ) -> Result<u64, AppError> {
        let claimed = PromotionService::new(database.clone())
            .claimed_checkout_ids()
            .await?;
        if claimed.is_empty() {
            return Ok(0);
        }
        let abandoned = CheckoutService::new(database.clone())
            .abandoned_ids(&claimed)
            .await?;
        if abandoned.is_empty() {
            return Ok(0);
        }
        PromotionService::new(database.clone())
            .release_abandoned_usage(&abandoned)
            .await
    }

    pub(super) async fn maintain_store(&self, store: &Store) -> MaintenanceReport {
        let mut report = MaintenanceReport::default();
        let database = match self.databases.database(&store.id).await {
            Ok(database) => database,
            Err(error) => {
                tracing::warn!(store_id = %store.id, error = %error, "maintenance could not open store database");
                report.failures += 1;
                return report;
            }
        };
        match CartService::new(database.clone())
            .expire_due(self.now)
            .await
        {
            Ok(count) => report.carts_expired += count,
            Err(error) => {
                tracing::warn!(store_id = %store.id, error = %error, "cart expiry failed");
                report.failures += 1;
            }
        }
        match InventoryService::new(database.clone())
            .expire_due(self.now)
            .await
        {
            Ok(count) => report.reservations_released += count,
            Err(error) => {
                tracing::warn!(store_id = %store.id, error = %error, "reservation expiry failed");
                report.failures += 1;
            }
        }
        match CheckoutService::new(database.clone())
            .expire_due(self.now)
            .await
        {
            Ok(count) => report.checkouts_expired += count,
            Err(error) => {
                tracing::warn!(store_id = %store.id, error = %error, "checkout expiry failed");
                report.failures += 1;
            }
        }
        // Runs after checkout expiry so a checkout swept to a terminal state
        // this pass gives its promotion claims back in the same pass. The
        // promotion module cannot see the checkout table and the checkout module
        // cannot see usage rows, so the two reads are joined here.
        match self.release_abandoned_promotion_usage(&database).await {
            Ok(count) => report.promotion_claims_released += count,
            Err(error) => {
                tracing::warn!(store_id = %store.id, error = %error, "promotion claim release failed");
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
                    self.email,
                    self.channels,
                    self.now,
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
            self.now,
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
            self.events,
            self.email,
            self.notifications,
            self.channels,
            self.render_context,
            self.allow_private_webhooks,
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
                        data: serde_json::from_str(&record.data).unwrap_or(serde_json::Value::Null),
                        ip_address: None,
                    };
                    // The outbox id is the platform audit id, so a retry
                    // after a failed "mark processed" cannot duplicate.
                    match self.platform.record_audit_with_id(&record.id, &entry).await {
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
            .with_private_networks_allowed(self.allow_private_webhooks)
            .deliver_due(self.now, 10)
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
        report
    }
}
