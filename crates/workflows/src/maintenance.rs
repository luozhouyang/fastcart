//! Periodic maintenance over active store databases.

use fastcart_infrastructure::{
    EventBus, StoreDatabaseManager, fetch_pending_outbox, mark_outbox_failed, mark_outbox_processed,
};
use fastcart_kernel::{Pagination, now_timestamp};
use fastcart_module_cart::CartService;
use fastcart_module_checkout::CheckoutService;
use fastcart_module_inventory::InventoryService;
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
            match fetch_pending_outbox(database.pool(), 100).await {
                Ok(records) => {
                    for record in records {
                        if let Some(event) = record.to_envelope(Some(store.id.clone())) {
                            let _ = events.publish(event);
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
        }
        offset += page.data.len() as i64;
        if offset >= page.count || page.data.is_empty() {
            break;
        }
    }
    report
}
