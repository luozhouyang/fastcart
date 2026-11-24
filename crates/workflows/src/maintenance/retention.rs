//! Retention cleanup for inquiries, assets and store data.

use fastcart_infrastructure::{Storage, StoreDatabaseManager};
use fastcart_kernel::{Pagination, now_timestamp};
use fastcart_module_asset::{
    fetch_pending_asset_purges, mark_asset_purge_failed, mark_asset_purge_processed,
};
use fastcart_module_inquiry::{InquiryFileService, InquiryService};
use fastcart_notifications::NotificationChannelService;
use fastcart_platform::{PlatformStore, StoreStatus};

use super::*;

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
