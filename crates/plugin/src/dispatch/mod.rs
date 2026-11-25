//! Event fan-out, delivery retries and scheduled plugin jobs.

pub mod cron;

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::now_timestamp;
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::error::PluginError;
use crate::manifest::PluginManifest;
use crate::runtime::process::{PluginHost, RequestKind, RuntimeRequest};
use crate::runtime::token::{ContextClaims, TokenActor, TokenSigner};

const DELIVERY_LEASE_SECONDS: i64 = 300;
const JOB_LEASE_SECONDS: i64 = 600;

/// Outcome of one dispatch pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DispatchReport {
    /// New delivery rows fanned out from the outbox.
    pub fanned_out: usize,
    /// Deliveries completed.
    pub delivered: usize,
    /// Deliveries retried.
    pub retried: usize,
    /// Deliveries moved to dead letter.
    pub dead: usize,
    /// Scheduled jobs enqueued.
    pub jobs_enqueued: usize,
    /// Jobs completed.
    pub jobs_done: usize,
    /// Jobs returned to the queue.
    pub jobs_retried: usize,
    /// Jobs moved to failed.
    pub jobs_failed: usize,
}

/// Event and job dispatcher for one store.
#[derive(Clone)]
pub struct PluginDispatch {
    host: Arc<PluginHost>,
    tokens: TokenSigner,
}

impl PluginDispatch {
    /// Creates a dispatcher for the shared host.
    #[must_use]
    pub fn new(host: Arc<PluginHost>, tokens: TokenSigner) -> Self {
        Self { host, tokens }
    }

    /// Runs one bounded pass: fan out, deliver, enqueue due jobs, run jobs.
    pub async fn run(&self, database: &StoreDatabase, limit: i64) -> DispatchReport {
        let mut report = DispatchReport::default();
        match Self::fan_out(database, limit).await {
            Ok(count) => report.fanned_out = count,
            Err(error) => tracing::warn!(error = %error, "plugin event fan-out failed"),
        }
        match self.deliver(database, limit).await {
            Ok((delivered, retried, dead)) => {
                report.delivered = delivered;
                report.retried = retried;
                report.dead = dead;
            }
            Err(error) => tracing::warn!(error = %error, "plugin delivery failed"),
        }
        match Self::enqueue_due_jobs(database, now_timestamp()).await {
            Ok(count) => report.jobs_enqueued = count,
            Err(error) => tracing::warn!(error = %error, "plugin job scheduling failed"),
        }
        match self.run_jobs(database, limit).await {
            Ok((done, retried, failed)) => {
                report.jobs_done = done;
                report.jobs_retried = retried;
                report.jobs_failed = failed;
            }
            Err(error) => tracing::warn!(error = %error, "plugin job execution failed"),
        }
        report
    }

    /// Creates delivery rows for outbox events with subscribers.
    pub async fn fan_out(database: &StoreDatabase, limit: i64) -> Result<usize, PluginError> {
        #[derive(FromRow)]
        struct PendingEvent {
            id: String,
            event_type: String,
            version: i64,
            payload: String,
            created_at: i64,
        }
        let events: Vec<PendingEvent> = sqlx::query_as(
            "SELECT id, event_type, version, payload, created_at FROM outbox \
             WHERE processed_at IS NULL OR processed_at >= ? \
             ORDER BY created_at ASC LIMIT ?",
        )
        .bind(now_timestamp().saturating_sub(3_600))
        .bind(limit.clamp(1, 500))
        .fetch_all(database.pool())
        .await?;
        let now = now_timestamp();
        let mut inserted = 0_usize;
        for event in events {
            let subscribers: Vec<String> = sqlx::query_scalar(
                "SELECT s.plugin_id FROM plugin_event_subscriptions s \
                 JOIN plugin_installations i ON i.plugin_id = s.plugin_id AND i.enabled = 1 \
                 WHERE s.event_type = ? AND s.enabled = 1",
            )
            .bind(&event.event_type)
            .fetch_all(database.pool())
            .await?;
            if subscribers.is_empty() {
                continue;
            }
            let data: Value = serde_json::from_str(&event.payload).unwrap_or(Value::Null);
            let payload = json!({
                "event": {
                    "id": event.id,
                    "type": event.event_type,
                    "version": event.version,
                    "occurred_at": event.created_at,
                    "data": data,
                }
            })
            .to_string();
            for plugin_id in subscribers {
                let result = sqlx::query(
                    "INSERT OR IGNORE INTO plugin_deliveries \
                     (id, plugin_id, event_id, event_type, payload, status, attempts, available_at, created_at) \
                     VALUES (?, ?, ?, ?, ?, 'pending', 0, ?, ?)",
                )
                .bind(format!("pdl_{}", uuid::Uuid::now_v7().simple()))
                .bind(&plugin_id)
                .bind(&event.id)
                .bind(&event.event_type)
                .bind(&payload)
                .bind(now)
                .bind(now)
                .execute(database.pool())
                .await?;
                inserted += usize::try_from(result.rows_affected()).unwrap_or(0);
            }
        }
        Ok(inserted)
    }

    /// Delivers due pending rows with exponential backoff.
    pub async fn deliver(
        &self,
        database: &StoreDatabase,
        limit: i64,
    ) -> Result<(usize, usize, usize), PluginError> {
        #[derive(FromRow)]
        struct Delivery {
            id: String,
            plugin_id: String,
            event_id: String,
            payload: String,
            attempts: i64,
        }
        let now = now_timestamp();
        sqlx::query(
            "UPDATE plugin_deliveries SET status = 'pending', locked_at = NULL, locked_by = NULL \
             WHERE status = 'running' AND locked_at < ?",
        )
        .bind(now.saturating_sub(DELIVERY_LEASE_SECONDS))
        .execute(database.pool())
        .await?;
        let rows: Vec<Delivery> = sqlx::query_as(
            "UPDATE plugin_deliveries SET status = 'running', locked_at = ?, locked_by = 'plugin' \
             WHERE id IN (SELECT id FROM plugin_deliveries \
                          WHERE status = 'pending' AND available_at <= ? \
                          ORDER BY available_at ASC LIMIT ?) \
             RETURNING id, plugin_id, event_id, payload, attempts",
        )
        .bind(now)
        .bind(now)
        .bind(limit.clamp(1, 50))
        .fetch_all(database.pool())
        .await?;
        let mut delivered = 0;
        let mut retried = 0;
        let mut dead = 0;
        for row in rows {
            let Some((manifest, digest)) = installation(database, &row.plugin_id).await? else {
                mark_delivery(
                    database,
                    &row.id,
                    "dead",
                    row.attempts,
                    "plugin is not enabled",
                )
                .await?;
                dead += 1;
                continue;
            };
            let actor = TokenActor::Event {
                id: row.event_id.clone(),
            };
            let token = token_for(
                &self.tokens,
                &row.plugin_id,
                database,
                &digest,
                manifest,
                actor,
            )?;
            let body = delivery_body(&row.payload, row.attempts + 1);
            let result = self
                .host
                .forward(RuntimeRequest {
                    plugin_id: row.plugin_id.clone(),
                    digest,
                    store_id: database.store_id().as_str().to_owned(),
                    path_and_query: "/events".to_owned(),
                    method: "POST".to_owned(),
                    headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                    context_token: token,
                    body: body.into(),
                    kind: RequestKind::Event,
                    deadline_override: None,
                })
                .await;
            match result {
                Ok(response) if (200..300).contains(&response.status) => {
                    mark_delivery(database, &row.id, "done", row.attempts + 1, "").await?;
                    delivered += 1;
                }
                Ok(response) if response.status < 500 => {
                    mark_delivery(
                        database,
                        &row.id,
                        "dead",
                        row.attempts + 1,
                        &format!("plugin returned {}", response.status),
                    )
                    .await?;
                    dead += 1;
                }
                Ok(response) => {
                    retry_delivery(
                        database,
                        &row.id,
                        row.attempts + 1,
                        &format!("plugin returned {}", response.status),
                    )
                    .await?;
                    retried += 1;
                }
                Err(error) => {
                    retry_delivery(database, &row.id, row.attempts + 1, &error.to_string()).await?;
                    retried += 1;
                }
            }
        }
        Ok((delivered, retried, dead))
    }

    /// Enqueues scheduled jobs whose cron matches the current minute.
    pub async fn enqueue_due_jobs(
        database: &StoreDatabase,
        now: i64,
    ) -> Result<usize, PluginError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT plugin_id, manifest_json FROM plugin_installations WHERE enabled = 1",
        )
        .fetch_all(database.pool())
        .await?;
        let minute = now - now.rem_euclid(60);
        let mut inserted = 0;
        for (plugin_id, manifest_json) in rows {
            let Ok(manifest) = PluginManifest::parse(manifest_json.as_bytes()) else {
                continue;
            };
            for job in &manifest.jobs {
                let Some(schedule) = &job.schedule else {
                    continue;
                };
                if !cron::due(schedule, now) {
                    continue;
                }
                let id = format!("pjob_{plugin_id}_{}_{minute}", job.name);
                let result = sqlx::query(
                    "INSERT OR IGNORE INTO plugin_jobs \
                     (id, plugin_id, name, payload, status, attempts, available_at, created_at, updated_at) \
                     VALUES (?, ?, ?, '{}', 'pending', 0, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&plugin_id)
                .bind(&job.name)
                .bind(now)
                .bind(now)
                .bind(now)
                .execute(database.pool())
                .await?;
                inserted += usize::try_from(result.rows_affected()).unwrap_or(0);
            }
        }
        Ok(inserted)
    }

    /// Runs due plugin jobs.
    pub async fn run_jobs(
        &self,
        database: &StoreDatabase,
        limit: i64,
    ) -> Result<(usize, usize, usize), PluginError> {
        #[derive(FromRow)]
        struct Job {
            id: String,
            plugin_id: String,
            name: String,
            payload: String,
            attempts: i64,
            max_attempts: i64,
        }
        let now = now_timestamp();
        sqlx::query(
            "UPDATE plugin_jobs SET status = 'pending', locked_at = NULL, locked_by = NULL \
             WHERE status = 'running' AND locked_at < ?",
        )
        .bind(now.saturating_sub(JOB_LEASE_SECONDS))
        .execute(database.pool())
        .await?;
        let rows: Vec<Job> = sqlx::query_as(
            "UPDATE plugin_jobs SET status = 'running', locked_at = ?, locked_by = 'plugin' \
             WHERE id IN (SELECT id FROM plugin_jobs \
                          WHERE status = 'pending' AND available_at <= ? \
                          ORDER BY available_at ASC LIMIT ?) \
             RETURNING id, plugin_id, name, payload, attempts, max_attempts",
        )
        .bind(now)
        .bind(now)
        .bind(limit.clamp(1, 20))
        .fetch_all(database.pool())
        .await?;
        let mut done = 0;
        let mut retried = 0;
        let mut failed = 0;
        for row in rows {
            let Some((manifest, digest)) = installation(database, &row.plugin_id).await? else {
                mark_job(
                    database,
                    &row.id,
                    "failed",
                    row.attempts + 1,
                    "plugin is not enabled",
                )
                .await?;
                failed += 1;
                continue;
            };
            let declared = manifest.jobs.iter().any(|job| job.name == row.name);
            if !declared {
                mark_job(
                    database,
                    &row.id,
                    "failed",
                    row.attempts + 1,
                    "job is not declared",
                )
                .await?;
                failed += 1;
                continue;
            }
            let actor = TokenActor::Job { id: row.id.clone() };
            let token = token_for(
                &self.tokens,
                &row.plugin_id,
                database,
                &digest,
                manifest,
                actor,
            )?;
            let payload: Value = serde_json::from_str(&row.payload).unwrap_or(Value::Null);
            let body = json!({
                "id": row.id,
                "name": row.name,
                "attempt": row.attempts + 1,
                "scheduled_at": now,
                "payload": payload,
            })
            .to_string()
            .into_bytes();
            let result = self
                .host
                .forward(RuntimeRequest {
                    plugin_id: row.plugin_id.clone(),
                    digest,
                    store_id: database.store_id().as_str().to_owned(),
                    path_and_query: format!("/jobs/{}", row.name),
                    method: "POST".to_owned(),
                    headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                    context_token: token,
                    body: body.into(),
                    kind: RequestKind::Job,
                    deadline_override: None,
                })
                .await;
            match result {
                Ok(response) if (200..300).contains(&response.status) => {
                    mark_job(database, &row.id, "completed", row.attempts + 1, "").await?;
                    done += 1;
                }
                Ok(response) if response.status < 500 || row.attempts + 1 >= row.max_attempts => {
                    mark_job(
                        database,
                        &row.id,
                        "failed",
                        row.attempts + 1,
                        &format!("plugin returned {}", response.status),
                    )
                    .await?;
                    failed += 1;
                }
                Ok(response) => {
                    retry_job(
                        database,
                        &row.id,
                        row.attempts + 1,
                        &format!("plugin returned {}", response.status),
                    )
                    .await?;
                    retried += 1;
                }
                Err(error) => {
                    if row.attempts + 1 >= row.max_attempts {
                        mark_job(
                            database,
                            &row.id,
                            "failed",
                            row.attempts + 1,
                            &error.to_string(),
                        )
                        .await?;
                        failed += 1;
                    } else {
                        retry_job(database, &row.id, row.attempts + 1, &error.to_string()).await?;
                        retried += 1;
                    }
                }
            }
        }
        Ok((done, retried, failed))
    }

    /// Replays one dead or failed delivery.
    pub async fn replay_delivery(
        database: &StoreDatabase,
        plugin_id: &str,
        delivery_id: &str,
    ) -> Result<(), PluginError> {
        sqlx::query(
            "UPDATE plugin_deliveries SET status = 'pending', attempts = 0, available_at = ?, \
             last_error = NULL, locked_at = NULL, locked_by = NULL \
             WHERE id = ? AND plugin_id = ?",
        )
        .bind(now_timestamp())
        .bind(delivery_id)
        .bind(plugin_id)
        .execute(database.pool())
        .await?;
        Ok(())
    }

    /// Enqueues a manual run of a declared job.
    pub async fn enqueue_job(
        database: &StoreDatabase,
        plugin_id: &str,
        name: &str,
    ) -> Result<(), PluginError> {
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO plugin_jobs \
             (id, plugin_id, name, payload, status, attempts, available_at, created_at, updated_at) \
             VALUES (?, ?, ?, '{}', 'pending', 0, ?, ?, ?)",
        )
        .bind(format!("pjob_manual_{}", uuid::Uuid::now_v7().simple()))
        .bind(plugin_id)
        .bind(name)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await?;
        Ok(())
    }
}

async fn installation(
    database: &StoreDatabase,
    plugin_id: &str,
) -> Result<Option<(PluginManifest, String)>, PluginError> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT digest, manifest_json FROM plugin_installations \
         WHERE plugin_id = ? AND enabled = 1",
    )
    .bind(plugin_id)
    .fetch_optional(database.pool())
    .await?;
    let Some((digest, manifest_json)) = row else {
        return Ok(None);
    };
    let manifest = PluginManifest::parse(manifest_json.as_bytes())?;
    Ok(Some((manifest, digest)))
}

fn token_for(
    tokens: &TokenSigner,
    plugin_id: &str,
    database: &StoreDatabase,
    digest: &str,
    manifest: PluginManifest,
    actor: TokenActor,
) -> Result<String, PluginError> {
    let scopes = manifest
        .scopes()?
        .into_iter()
        .map(|scope| scope.as_str().to_owned())
        .collect();
    let claims = ContextClaims::new(
        plugin_id,
        database.store_id().as_str(),
        digest,
        scopes,
        actor,
        now_timestamp(),
    );
    tokens
        .derive(database.store_id().as_str(), plugin_id, digest)
        .mint(&claims)
        .map_err(|error| PluginError::Runtime(error.to_string()))
}

fn delivery_body(payload: &str, attempt: i64) -> Vec<u8> {
    let mut value: Value = serde_json::from_str(payload).unwrap_or_else(|_| json!({}));
    if let Value::Object(map) = &mut value {
        map.insert("attempt".to_owned(), json!(attempt));
    }
    value.to_string().into_bytes()
}

async fn mark_delivery(
    database: &StoreDatabase,
    id: &str,
    status: &str,
    attempts: i64,
    error: &str,
) -> Result<(), PluginError> {
    sqlx::query(
        "UPDATE plugin_deliveries SET status = ?, attempts = ?, last_error = NULLIF(?, ''), \
         locked_at = NULL, locked_by = NULL WHERE id = ?",
    )
    .bind(status)
    .bind(attempts)
    .bind(error)
    .bind(id)
    .execute(database.pool())
    .await?;
    Ok(())
}

async fn retry_delivery(
    database: &StoreDatabase,
    id: &str,
    attempts: i64,
    error: &str,
) -> Result<(), PluginError> {
    let status = if attempts >= 5 { "dead" } else { "pending" };
    let backoff =
        5_i64.saturating_mul(2_i64.saturating_pow(u32::try_from(attempts).unwrap_or(1).min(6)));
    sqlx::query(
        "UPDATE plugin_deliveries SET status = ?, attempts = ?, last_error = ?, \
         available_at = ?, locked_at = NULL, locked_by = NULL WHERE id = ?",
    )
    .bind(status)
    .bind(attempts)
    .bind(error)
    .bind(now_timestamp().saturating_add(backoff))
    .bind(id)
    .execute(database.pool())
    .await?;
    Ok(())
}

async fn mark_job(
    database: &StoreDatabase,
    id: &str,
    status: &str,
    attempts: i64,
    error: &str,
) -> Result<(), PluginError> {
    sqlx::query(
        "UPDATE plugin_jobs SET status = ?, attempts = ?, last_error = NULLIF(?, ''), \
         locked_at = NULL, locked_by = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(attempts)
    .bind(error)
    .bind(now_timestamp())
    .bind(id)
    .execute(database.pool())
    .await?;
    Ok(())
}

async fn retry_job(
    database: &StoreDatabase,
    id: &str,
    attempts: i64,
    error: &str,
) -> Result<(), PluginError> {
    let backoff =
        5_i64.saturating_mul(2_i64.saturating_pow(u32::try_from(attempts).unwrap_or(1).min(6)));
    sqlx::query(
        "UPDATE plugin_jobs SET status = 'pending', attempts = ?, last_error = ?, \
         available_at = ?, locked_at = NULL, locked_by = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(attempts)
    .bind(error)
    .bind(now_timestamp().saturating_add(backoff))
    .bind(now_timestamp())
    .bind(id)
    .execute(database.pool())
    .await?;
    Ok(())
}

/// Deletes plugin log lines older than seven days.
pub async fn prune_logs(database: &StoreDatabase, now: i64) -> Result<u64, PluginError> {
    let result = sqlx::query("DELETE FROM plugin_logs WHERE created_at < ?")
        .bind(now.saturating_sub(7 * 86_400))
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected())
}

/// Deletes KV entries whose TTL has elapsed.
pub async fn prune_expired_kv(database: &StoreDatabase, now: i64) -> Result<u64, PluginError> {
    let result =
        sqlx::query("DELETE FROM plugin_kv WHERE expires_at IS NOT NULL AND expires_at <= ?")
            .bind(now)
            .execute(database.pool())
            .await?;
    Ok(result.rows_affected())
}

/// Deletes terminal deliveries and jobs older than thirty days.
///
/// Pending and running rows are kept so a slow retry is never dropped.
pub async fn prune_dispatch_state(
    database: &StoreDatabase,
    now: i64,
) -> Result<(u64, u64), PluginError> {
    let cutoff = now.saturating_sub(30 * 86_400);
    let deliveries = sqlx::query(
        "DELETE FROM plugin_deliveries \
         WHERE status IN ('done', 'dead') AND created_at < ?",
    )
    .bind(cutoff)
    .execute(database.pool())
    .await?;
    let jobs = sqlx::query(
        "DELETE FROM plugin_jobs \
         WHERE status IN ('completed', 'failed') AND updated_at < ?",
    )
    .bind(cutoff)
    .execute(database.pool())
    .await?;
    Ok((deliveries.rows_affected(), jobs.rows_affected()))
}
