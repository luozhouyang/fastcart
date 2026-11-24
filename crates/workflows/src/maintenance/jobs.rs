//! Job leases, claiming and execution.

use std::time::Duration;

use fastcart_email::EmailService;
use fastcart_infrastructure::{JobQueue, JobRecord, StoreDatabase};
use fastcart_kernel::{AppError, StoreId};
use fastcart_module_cart::CartService;
use fastcart_module_catalog::CatalogService;
use fastcart_module_inventory::InventoryService;
use fastcart_notifications::{NotificationChannelService, SEND_CHANNEL_NOTIFICATION_JOB_TYPE};

use super::*;

pub(super) async fn process_job(
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
                .process_job_for_store(store_id, &payload)
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
pub(super) async fn run_claimed_jobs(
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
        // Keep the lease fresh while a handler runs so a slow delivery is not
        // handed to another worker by reap_expired.
        let lease = spawn_lease_heartbeat(queue, &job.id, worker_id);
        let outcome = process_job(store_id, database, &job, email, channels, now).await;
        drop(lease);
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

/// Refreshes one job's lease periodically until the guard is dropped.
fn spawn_lease_heartbeat(queue: &JobQueue, job_id: &str, worker_id: &str) -> LeaseHeartbeat {
    let queue = queue.clone();
    let job_id = job_id.to_owned();
    let worker_id = worker_id.to_owned();
    LeaseHeartbeat(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(JOB_LEASE_RENEW_SECONDS));
        // The first interval tick is immediate; consume it before waiting.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match queue
                .renew_lease(&job_id, &worker_id, JOB_LEASE_SECONDS)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(
                        job_id = %job_id,
                        "job lease renewal rejected because the lease was lost"
                    );
                    break;
                }
                Err(error) => {
                    tracing::warn!(job_id = %job_id, error = %error, "job lease renewal failed");
                }
            }
        }
    }))
}

/// Aborts the renewal task when the handler finishes or unwinds.
struct LeaseHeartbeat(tokio::task::JoinHandle<()>);

impl Drop for LeaseHeartbeat {
    fn drop(&mut self) {
        self.0.abort();
    }
}
