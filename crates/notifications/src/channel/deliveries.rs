//! Delivery records and recovery of abandoned sends.

use super::*;

impl NotificationChannelService {
    /// Lists deliveries for one channel, newest first.
    pub async fn list_deliveries(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
        limit: i64,
    ) -> Result<Vec<ChannelDelivery>, AppError> {
        let rows = sqlx::query_as::<_, DeliveryRow>(&format!(
            "SELECT {DELIVERY_COLUMNS} FROM notification_channel_deliveries
             WHERE channel_id = ?
             ORDER BY created_at DESC, id DESC
             LIMIT ?"
        ))
        .bind(id.as_str())
        .bind(limit.clamp(1, MAX_DELIVERY_LIMIT))
        .fetch_all(db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(DeliveryRow::into_delivery).collect()
    }
}

impl NotificationChannelService {
    /// Deletes terminal delivery records older than `cutoff`.
    ///
    /// Pending or queued rows are retained because they still describe work
    /// that the job queue may retry.
    pub async fn delete_deliveries_before(
        &self,
        db: &StoreDatabase,
        cutoff: i64,
    ) -> Result<u64, AppError> {
        let result = sqlx::query(
            "DELETE FROM notification_channel_deliveries
             WHERE created_at < ? AND status IN ('sent', 'failed', 'skipped')",
        )
        .bind(cutoff)
        .execute(db.pool())
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected())
    }
}

impl NotificationChannelService {
    /// Resets delivery claims that were never followed by a job enqueue.
    ///
    /// Dispatch persists a claimed `job_id` before enqueueing, so a crash in
    /// between leaves a `queued` row that no worker will ever process. Jobs are
    /// never deleted, so a missing `jobs` row proves the enqueue never
    /// happened. Rows whose job still exists — pending, running, completed or
    /// failed — are left untouched to avoid enqueueing a duplicate send.
    ///
    /// Returns the number of recovered deliveries.
    pub async fn recover_abandoned_claims(
        db: &StoreDatabase,
        now: i64,
        lease_seconds: i64,
    ) -> Result<u64, AppError> {
        let cutoff = now.saturating_sub(lease_seconds.max(0));
        let result = sqlx::query(
            "UPDATE notification_channel_deliveries
             SET status = 'pending', job_id = NULL, claimed_at = NULL, updated_at = ?
             WHERE status = 'queued'
               AND claimed_at IS NOT NULL
               AND claimed_at <= ?
               AND NOT EXISTS (
                   SELECT 1 FROM jobs
                   WHERE jobs.id = notification_channel_deliveries.job_id
               )",
        )
        .bind(now)
        .bind(cutoff)
        .execute(db.pool())
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected())
    }
}

pub(super) async fn update_channel_error(
    db: &StoreDatabase,
    id: &NotificationChannelId,
    error: &str,
) -> Result<(), ChannelError> {
    sqlx::query("UPDATE notification_channels SET last_error = ?, updated_at = ? WHERE id = ?")
        .bind(error)
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(db.pool())
        .await
        .map_err(|error| retryable_db("channel update failed", &database_error(error)))?;
    Ok(())
}

pub(super) enum ChannelDeliveryClaim {
    /// The caller now owns the delivery row and may enqueue its job.
    Claimed {
        delivery_id: NotificationDeliveryId,
        job_id: JobId,
    },
    /// Another attempt already queued, sent or skipped this delivery.
    Duplicate,
}

/// Atomically claims `(channel_id, event_id)` before a job is enqueued.
///
/// The insert is idempotent thanks to the unique constraint. The claim itself
/// is a conditional update: only one dispatcher can move a `pending` or
/// `failed` row to `queued`, and the winning `job_id` is persisted before the
/// job exists. A crash after this point leaves a `queued` row whose job may be
/// missing; replays see it as a duplicate instead of enqueueing another copy.
pub(super) async fn claim_delivery_row(
    db: &StoreDatabase,
    channel_id: &NotificationChannelId,
    event: &DomainEventEnvelope,
) -> Result<ChannelDeliveryClaim, AppError> {
    let now = now_timestamp();
    sqlx::query(
        "INSERT OR IGNORE INTO notification_channel_deliveries
         (id, channel_id, event_id, event_type, status, attempts, created_at, updated_at)
         VALUES (?, ?, ?, ?, 'pending', 0, ?, ?)",
    )
    .bind(NotificationDeliveryId::new().as_str())
    .bind(channel_id.as_str())
    .bind(event.id.as_str())
    .bind(&event.event_type)
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(database_error)?;

    let delivery_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM notification_channel_deliveries
         WHERE channel_id = ? AND event_id = ?",
    )
    .bind(channel_id.as_str())
    .bind(event.id.as_str())
    .fetch_optional(db.pool())
    .await
    .map_err(database_error)?;
    let delivery_id = delivery_id.ok_or_else(|| {
        AppError::internal("notification delivery claim disappeared during dispatch")
    })?;

    let job_id = JobId::new();
    let claimed = sqlx::query(
        "UPDATE notification_channel_deliveries
         SET status = 'queued', job_id = ?, claimed_at = ?, last_error = NULL, updated_at = ?
         WHERE id = ? AND status IN ('pending', 'failed')",
    )
    .bind(job_id.as_str())
    .bind(now)
    .bind(now)
    .bind(&delivery_id)
    .execute(db.pool())
    .await
    .map_err(database_error)?;
    if claimed.rows_affected() == 1 {
        return Ok(ChannelDeliveryClaim::Claimed {
            delivery_id: NotificationDeliveryId::from_string(delivery_id),
            job_id,
        });
    }
    Ok(ChannelDeliveryClaim::Duplicate)
}

pub(super) async fn mark_delivery_failed(
    db: &StoreDatabase,
    id: &NotificationDeliveryId,
    job_id: Option<&str>,
    error: &str,
) -> Result<(), AppError> {
    let now = now_timestamp();
    let query = if job_id.is_some() {
        "UPDATE notification_channel_deliveries
         SET status = 'failed', job_id = NULL, claimed_at = NULL, last_error = ?, updated_at = ?
         WHERE id = ? AND job_id = ?"
    } else {
        "UPDATE notification_channel_deliveries
         SET status = 'failed', job_id = NULL, claimed_at = NULL, last_error = ?, updated_at = ?
         WHERE id = ?"
    };
    let mut query = sqlx::query(query).bind(error).bind(now).bind(id.as_str());
    if let Some(job_id) = job_id {
        query = query.bind(job_id);
    }
    query.execute(db.pool()).await.map_err(database_error)?;
    Ok(())
}

pub(super) async fn mark_delivery_skipped(
    db: &StoreDatabase,
    id: &NotificationDeliveryId,
    job_id: Option<&str>,
    reason: &str,
) -> Result<(), ChannelError> {
    let now = now_timestamp();
    let query = if job_id.is_some() {
        "UPDATE notification_channel_deliveries
         SET status = 'skipped', last_error = ?, updated_at = ?
         WHERE id = ? AND job_id = ?"
    } else {
        "UPDATE notification_channel_deliveries
         SET status = 'skipped', last_error = ?, updated_at = ?
         WHERE id = ?"
    };
    let mut query = sqlx::query(query).bind(reason).bind(now).bind(id.as_str());
    if let Some(job_id) = job_id {
        query = query.bind(job_id);
    }
    query
        .execute(db.pool())
        .await
        .map_err(|error| retryable_db("delivery update failed", &database_error(error)))?;
    Ok(())
}

/// Returns true when the current attempt is the job's last one.
///
/// The job queue increments its attempt counter after this handler returns, so
/// `attempts + 1 >= max_attempts` means this failure will exhaust the job.
pub(super) async fn job_will_exhaust(db: &StoreDatabase, job_id: &str) -> bool {
    sqlx::query_as::<_, (i64, i64)>("SELECT attempts, max_attempts FROM jobs WHERE id = ?")
        .bind(job_id)
        .fetch_optional(db.pool())
        .await
        .ok()
        .flatten()
        .is_some_and(|(attempts, max_attempts)| attempts.saturating_add(1) >= max_attempts)
}

pub(super) async fn load_delivery_row(
    db: &StoreDatabase,
    id: &NotificationDeliveryId,
) -> Result<Option<DeliveryRow>, AppError> {
    sqlx::query_as::<_, DeliveryRow>(&format!(
        "SELECT {DELIVERY_COLUMNS} FROM notification_channel_deliveries WHERE id = ?"
    ))
    .bind(id.as_str())
    .fetch_optional(db.pool())
    .await
    .map_err(database_error)
}

pub(super) async fn load_channel_row_including_deleted(
    db: &StoreDatabase,
    id: &NotificationChannelId,
) -> Result<Option<ChannelRow>, AppError> {
    sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {CHANNEL_COLUMNS} FROM notification_channels WHERE id = ?"
    ))
    .bind(id.as_str())
    .fetch_optional(db.pool())
    .await
    .map_err(database_error)
}
