//! SQLite-backed background job queue.
//!
//! Claiming is a single atomic `UPDATE ... RETURNING` statement, so multiple
//! workers can share one store database without double-processing jobs. Leases
//! protect against crashed workers: expired `running` jobs become claimable
//! again.

use fastcart_kernel::{JobId, now_timestamp};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};

/// Status values persisted with a job.
pub mod status {
    /// Waiting to be claimed.
    pub const PENDING: &str = "pending";
    /// Claimed by a worker.
    pub const RUNNING: &str = "running";
    /// Finished successfully.
    pub const COMPLETED: &str = "completed";
    /// Exhausted retries.
    pub const FAILED: &str = "failed";
    /// Canceled before execution.
    pub const CANCELED: &str = "canceled";
}

/// A persisted job row.
#[derive(Clone, Debug, FromRow, serde::Serialize, serde::Deserialize)]
pub struct JobRecord {
    /// Job identifier.
    pub id: String,
    /// Job type consumed by a registered handler.
    #[sqlx(rename = "type")]
    #[serde(rename = "type")]
    pub job_type: String,
    /// JSON payload.
    pub payload: String,
    /// Lifecycle status.
    pub status: String,
    /// Higher priority is claimed first.
    pub priority: i64,
    /// Attempts already made.
    pub attempts: i64,
    /// Maximum attempts before permanent failure.
    pub max_attempts: i64,
    /// Earliest claim timestamp.
    pub available_at: i64,
    /// Lease timestamp.
    pub locked_at: Option<i64>,
    /// Worker identity holding the lease.
    pub locked_by: Option<String>,
    /// Last error message.
    pub last_error: Option<String>,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl JobRecord {
    /// Parses the JSON payload.
    #[must_use]
    pub fn json_payload(&self) -> Value {
        serde_json::from_str(&self.payload).unwrap_or(Value::Null)
    }
}

/// Input for enqueuing a job.
#[derive(Clone, Debug)]
pub struct NewJob {
    /// Job type consumed by a registered handler.
    pub job_type: String,
    /// JSON payload.
    pub payload: Value,
    /// Higher priority is claimed first.
    pub priority: i64,
    /// Maximum attempts before permanent failure.
    pub max_attempts: i64,
    /// Delay before the job becomes claimable, in seconds.
    pub delay_seconds: i64,
}

impl NewJob {
    /// Creates a job with default priority and retry policy.
    #[must_use]
    pub fn new(job_type: impl Into<String>, payload: Value) -> Self {
        Self {
            job_type: job_type.into(),
            payload,
            priority: 0,
            max_attempts: 5,
            delay_seconds: 0,
        }
    }

    /// Sets the job priority.
    #[must_use]
    pub fn with_priority(mut self, priority: i64) -> Self {
        self.priority = priority;
        self
    }

    /// Sets the retry limit.
    #[must_use]
    pub fn with_max_attempts(mut self, max_attempts: i64) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    /// Delays the first attempt.
    #[must_use]
    pub fn with_delay(mut self, seconds: i64) -> Self {
        self.delay_seconds = seconds.max(0);
        self
    }
}

/// Queue operations over one jobs table.
#[derive(Clone, Debug)]
pub struct JobQueue {
    pool: SqlitePool,
}

impl JobQueue {
    /// Creates a queue over the given database.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns the underlying pool.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Enqueues a job and returns the persisted row.
    pub async fn enqueue(&self, job: NewJob) -> Result<JobRecord, sqlx::Error> {
        let now = now_timestamp();
        let id = JobId::new().to_string();
        sqlx::query_as::<_, JobRecord>(
            "INSERT INTO jobs
             (id, type, payload, status, priority, attempts, max_attempts,
              available_at, created_at, updated_at)
             VALUES (?, ?, ?, 'pending', ?, 0, ?, ?, ?, ?)
             RETURNING *",
        )
        .bind(id)
        .bind(job.job_type)
        .bind(job.payload.to_string())
        .bind(job.priority)
        .bind(job.max_attempts)
        .bind(now.saturating_add(job.delay_seconds))
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await
    }

    /// Atomically claims up to `limit` due jobs for a worker.
    pub async fn claim(&self, worker_id: &str, limit: i64) -> Result<Vec<JobRecord>, sqlx::Error> {
        let now = now_timestamp();
        sqlx::query_as::<_, JobRecord>(
            "UPDATE jobs
             SET status = 'running', locked_at = ?, locked_by = ?, updated_at = ?
             WHERE id IN (
                 SELECT id FROM jobs
                 WHERE status = 'pending' AND available_at <= ?
                 ORDER BY priority DESC, available_at ASC, created_at ASC
                 LIMIT ?
             )
             RETURNING *",
        )
        .bind(now)
        .bind(worker_id)
        .bind(now)
        .bind(now)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await
    }

    /// Marks a job completed only when `worker_id` still holds its lease.
    pub async fn complete(&self, id: &str, worker_id: &str) -> Result<bool, sqlx::Error> {
        let now = now_timestamp();
        let result = sqlx::query(
            "UPDATE jobs SET status = 'completed', locked_at = NULL, locked_by = NULL,
             last_error = NULL, updated_at = ?
             WHERE id = ? AND status = 'running' AND locked_by = ?",
        )
        .bind(now)
        .bind(id)
        .bind(worker_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Records a failure and either reschedules or permanently fails the job.
    ///
    /// Returns true when the job will be retried.
    pub async fn fail(&self, id: &str, error: &str, worker_id: &str) -> Result<bool, sqlx::Error> {
        let row: Option<(i64, i64)> = sqlx::query_as(
            "SELECT attempts, max_attempts FROM jobs
             WHERE id = ? AND status = 'running' AND locked_by = ?",
        )
        .bind(id)
        .bind(worker_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((attempts, max_attempts)) = row else {
            return Ok(false);
        };
        let attempts = attempts.saturating_add(1);
        let now = now_timestamp();
        let retry = attempts < max_attempts;
        let next_at = now.saturating_add(backoff_seconds(attempts));
        sqlx::query(
            "UPDATE jobs SET attempts = ?, status = ?, available_at = ?,
             locked_at = NULL, locked_by = NULL, last_error = ?, updated_at = ?
             WHERE id = ? AND status = 'running' AND locked_by = ?",
        )
        .bind(attempts)
        .bind(if retry {
            status::PENDING
        } else {
            status::FAILED
        })
        .bind(if retry { next_at } else { now })
        .bind(error)
        .bind(now)
        .bind(id)
        .bind(worker_id)
        .execute(&self.pool)
        .await?;
        Ok(retry)
    }

    /// Cancels a pending job.
    pub async fn cancel(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE jobs SET status = 'canceled', updated_at = ?
             WHERE id = ? AND status = 'pending'",
        )
        .bind(now_timestamp())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Returns expired running jobs to the pending state.
    pub async fn reap_expired(&self, lease_seconds: i64) -> Result<u64, sqlx::Error> {
        let cutoff = now_timestamp().saturating_sub(lease_seconds.max(0));
        let result = sqlx::query(
            "UPDATE jobs SET status = 'pending', locked_at = NULL, locked_by = NULL,
             updated_at = ? WHERE status = 'running' AND locked_at IS NOT NULL
             AND locked_at <= ?",
        )
        .bind(now_timestamp())
        .bind(cutoff)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Counts jobs in a status.
    pub async fn count_by_status(&self, status: &str) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = ?")
            .bind(status)
            .fetch_one(&self.pool)
            .await
    }

    /// Returns a job by identifier.
    pub async fn get(&self, id: &str) -> Result<Option<JobRecord>, sqlx::Error> {
        sqlx::query_as::<_, JobRecord>("SELECT * FROM jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
}

fn backoff_seconds(attempts: i64) -> i64 {
    let exponent = u32::try_from(attempts.clamp(1, 10)).unwrap_or(10);
    2_i64.saturating_pow(exponent).min(3_600)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{JobQueue, NewJob, status};
    use crate::database::connect_memory;

    async fn queue() -> JobQueue {
        let pool = connect_memory().await.unwrap();
        sqlx::query(
            "CREATE TABLE jobs (
                id TEXT PRIMARY KEY, type TEXT NOT NULL, payload TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending', priority INTEGER NOT NULL DEFAULT 0,
                attempts INTEGER NOT NULL DEFAULT 0, max_attempts INTEGER NOT NULL DEFAULT 5,
                available_at INTEGER NOT NULL, locked_at INTEGER, locked_by TEXT,
                last_error TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        JobQueue::new(pool)
    }

    #[tokio::test]
    async fn jobs_are_claimed_once_and_completed() {
        let queue = queue().await;
        queue
            .enqueue(NewJob::new("send_email", json!({"to": "a@example.com"})))
            .await
            .unwrap();

        let first = queue.claim("worker-a", 10).await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].job_type, "send_email");
        assert_eq!(first[0].status, status::RUNNING);

        assert!(queue.claim("worker-b", 10).await.unwrap().is_empty());
        assert!(queue.complete(&first[0].id, "worker-a").await.unwrap());
        assert_eq!(queue.count_by_status(status::COMPLETED).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn priority_orders_claims() {
        let queue = queue().await;
        queue
            .enqueue(NewJob::new("low", json!({})).with_priority(1))
            .await
            .unwrap();
        queue
            .enqueue(NewJob::new("high", json!({})).with_priority(10))
            .await
            .unwrap();

        let claimed = queue.claim("worker", 1).await.unwrap();
        assert_eq!(claimed[0].job_type, "high");
        let claimed = queue.claim("worker", 1).await.unwrap();
        assert_eq!(claimed[0].job_type, "low");
    }

    #[tokio::test]
    async fn failures_retry_then_fail_permanently() {
        let queue = queue().await;
        let job = queue
            .enqueue(NewJob::new("flaky", json!({})).with_max_attempts(2))
            .await
            .unwrap();
        queue.claim("worker", 1).await.unwrap();

        assert!(queue.fail(&job.id, "boom", "worker").await.unwrap());
        // Backoff moves availability into the future.
        assert!(queue.claim("worker", 1).await.unwrap().is_empty());

        sqlx::query("UPDATE jobs SET available_at = 0 WHERE id = ?")
            .bind(&job.id)
            .execute(queue.pool())
            .await
            .unwrap();
        queue.claim("worker", 1).await.unwrap();
        assert!(!queue.fail(&job.id, "boom again", "worker").await.unwrap());
        assert_eq!(queue.count_by_status(status::FAILED).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn expired_leases_are_reaped() {
        let queue = queue().await;
        queue.enqueue(NewJob::new("slow", json!({}))).await.unwrap();
        queue.claim("worker", 1).await.unwrap();
        assert_eq!(queue.reap_expired(0).await.unwrap(), 1);
        assert_eq!(queue.count_by_status(status::PENDING).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn delayed_jobs_are_not_claimable_immediately() {
        let queue = queue().await;
        queue
            .enqueue(NewJob::new("later", json!({})).with_delay(3_600))
            .await
            .unwrap();
        assert!(queue.claim("worker", 1).await.unwrap().is_empty());
    }
}
