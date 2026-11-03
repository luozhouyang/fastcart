//! In-process event bus and transactional outbox helpers.
//!
//! The bus is a non-durable notification mechanism. Events that must survive a
//! crash are appended to the outbox inside the same transaction as the business
//! write, then dispatched by a background worker.

use fastcart_kernel::{DomainEventEnvelope, StoreId, now_timestamp};
use sqlx::{Executor, FromRow, Sqlite, SqlitePool};
use tokio::sync::broadcast;

/// Bounded in-process event bus.
#[derive(Clone, Debug)]
pub struct EventBus {
    sender: broadcast::Sender<DomainEventEnvelope>,
}

impl EventBus {
    /// Creates a bus with the given bounded channel capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    /// Publishes an event to current subscribers. Returns the receiver count.
    #[must_use]
    pub fn publish(&self, event: DomainEventEnvelope) -> usize {
        self.sender.send(event).unwrap_or(0)
    }

    /// Subscribes to future events.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEventEnvelope> {
        self.sender.subscribe()
    }

    /// Number of active subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

/// A persisted outbox row.
#[derive(Clone, Debug, FromRow, serde::Serialize, serde::Deserialize)]
pub struct OutboxRecord {
    /// Event identifier.
    pub id: String,
    /// Stable event type.
    pub event_type: String,
    /// Schema version.
    pub version: i64,
    /// JSON payload.
    pub payload: String,
    /// Optional deduplication key.
    pub idempotency_key: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Processing timestamp, null while pending.
    pub processed_at: Option<i64>,
    /// Delivery attempts.
    pub attempts: i64,
    /// Earliest next attempt timestamp.
    pub next_retry_at: Option<i64>,
    /// Last handler error.
    pub last_error: Option<String>,
}

impl OutboxRecord {
    /// Reconstructs the event envelope for dispatch.
    #[must_use]
    pub fn to_envelope(&self, store_id: Option<StoreId>) -> Option<DomainEventEnvelope> {
        let data = serde_json::from_str(&self.payload).ok()?;
        Some(DomainEventEnvelope {
            id: fastcart_kernel::EventId::from_string(self.id.clone()),
            event_type: self.event_type.clone(),
            version: u32::try_from(self.version).unwrap_or(1),
            store_id,
            occurred_at: self.created_at,
            actor_id: None,
            data,
            idempotency_key: self.idempotency_key.clone(),
        })
    }
}

/// Appends an event to the outbox using the caller's transaction or pool.
///
/// Passing a transaction guarantees the outbox row commits atomically with the
/// business change it describes.
pub async fn append_outbox_event<'e, E>(
    executor: E,
    event: &DomainEventEnvelope,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let payload = serde_json::to_string(&event.data).unwrap_or_else(|_| "null".to_owned());
    sqlx::query(
        "INSERT INTO outbox (id, event_type, version, payload, idempotency_key, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(event.id.as_str())
    .bind(&event.event_type)
    .bind(i64::from(event.version))
    .bind(payload)
    .bind(event.idempotency_key.as_deref())
    .bind(event.occurred_at)
    .execute(executor)
    .await?;
    Ok(())
}

/// Fetches pending outbox rows that are due for dispatch.
pub async fn fetch_pending_outbox(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<OutboxRecord>, sqlx::Error> {
    let now = now_timestamp();
    let stale_cutoff = now.saturating_sub(300);
    sqlx::query_as::<_, OutboxRecord>(
        "UPDATE outbox
         SET claimed_at = ?, claimed_by = 'maintenance'
         WHERE id IN (
             SELECT id FROM outbox
             WHERE processed_at IS NULL
               AND (next_retry_at IS NULL OR next_retry_at <= ?)
               AND (claimed_at IS NULL OR claimed_at <= ?)
             ORDER BY created_at ASC
             LIMIT ?
         )
         RETURNING id, event_type, version, payload, idempotency_key, created_at,
                   processed_at, attempts, next_retry_at, last_error",
    )
    .bind(now)
    .bind(now)
    .bind(stale_cutoff)
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await
}

/// Marks an outbox row as processed.
pub async fn mark_outbox_processed(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE outbox SET processed_at = ?, last_error = NULL,
         claimed_at = NULL, claimed_by = NULL WHERE id = ?",
    )
    .bind(now_timestamp())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Records a failed dispatch with exponential backoff (capped at one hour).
pub async fn mark_outbox_failed(
    pool: &SqlitePool,
    id: &str,
    error: &str,
) -> Result<(), sqlx::Error> {
    let attempts: Option<i64> = sqlx::query_scalar("SELECT attempts FROM outbox WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let attempts = attempts.unwrap_or(0).saturating_add(1);
    let backoff = 2_i64.saturating_pow(u32::try_from(attempts.min(12)).unwrap_or(12));
    let next_retry_at = now_timestamp().saturating_add(backoff.min(3_600));
    sqlx::query(
        "UPDATE outbox SET attempts = ?, last_error = ?, next_retry_at = ?,
         claimed_at = NULL, claimed_by = NULL WHERE id = ?",
    )
    .bind(attempts)
    .bind(error)
    .bind(next_retry_at)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::{DomainEventEnvelope, StoreId};
    use serde_json::json;

    use super::{
        EventBus, append_outbox_event, fetch_pending_outbox, mark_outbox_failed,
        mark_outbox_processed,
    };
    use crate::database::connect_memory;

    #[tokio::test]
    async fn bus_delivers_only_to_current_subscribers() {
        let bus = EventBus::new(8);
        let event = DomainEventEnvelope::new("product.created", None, json!({"id": "p1"}));

        assert_eq!(bus.publish(event.clone()), 0);

        let mut receiver = bus.subscribe();
        assert_eq!(bus.publish(event.clone()), 1);
        let received = receiver.recv().await.unwrap();
        assert_eq!(received.event_type, "product.created");
    }

    #[tokio::test]
    async fn outbox_round_trip_marks_processed() {
        let pool = connect_memory().await.unwrap();
        sqlx::query(
            "CREATE TABLE outbox (
                id TEXT PRIMARY KEY, event_type TEXT NOT NULL, version INTEGER NOT NULL,
                payload TEXT NOT NULL, idempotency_key TEXT, created_at INTEGER NOT NULL,
                processed_at INTEGER, attempts INTEGER NOT NULL DEFAULT 0,
                next_retry_at INTEGER, last_error TEXT, claimed_at INTEGER, claimed_by TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let store = StoreId::new();
        let event =
            DomainEventEnvelope::new("order.placed", Some(store.clone()), json!({"id": "o1"}));
        append_outbox_event(&pool, &event).await.unwrap();

        let pending = fetch_pending_outbox(&pool, 10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].to_envelope(Some(store)).unwrap().event_type,
            "order.placed"
        );

        mark_outbox_processed(&pool, pending[0].id.as_str())
            .await
            .unwrap();
        assert!(fetch_pending_outbox(&pool, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn outbox_rows_are_claimed_once() {
        let pool = connect_memory().await.unwrap();
        sqlx::query(
            "CREATE TABLE outbox (
                id TEXT PRIMARY KEY, event_type TEXT NOT NULL, version INTEGER NOT NULL,
                payload TEXT NOT NULL, idempotency_key TEXT, created_at INTEGER NOT NULL,
                processed_at INTEGER, attempts INTEGER NOT NULL DEFAULT 0,
                next_retry_at INTEGER, last_error TEXT, claimed_at INTEGER, claimed_by TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let event = DomainEventEnvelope::new("order.placed", None, serde_json::json!({}));
        append_outbox_event(&pool, &event).await.unwrap();
        let first = fetch_pending_outbox(&pool, 10).await.unwrap();
        let second = fetch_pending_outbox(&pool, 10).await.unwrap();
        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
    }

    #[tokio::test]
    async fn failed_outbox_rows_are_retried_later() {
        let pool = connect_memory().await.unwrap();
        sqlx::query(
            "CREATE TABLE outbox (
                id TEXT PRIMARY KEY, event_type TEXT NOT NULL, version INTEGER NOT NULL,
                payload TEXT NOT NULL, idempotency_key TEXT, created_at INTEGER NOT NULL,
                processed_at INTEGER, attempts INTEGER NOT NULL DEFAULT 0,
                next_retry_at INTEGER, last_error TEXT, claimed_at INTEGER, claimed_by TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let event = DomainEventEnvelope::new("cart.created", None, json!({}));
        append_outbox_event(&pool, &event).await.unwrap();
        mark_outbox_failed(&pool, event.id.as_str(), "boom")
            .await
            .unwrap();

        assert!(fetch_pending_outbox(&pool, 10).await.unwrap().is_empty());
        let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM outbox WHERE id = ?")
            .bind(event.id.as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(attempts, 1);
    }
}
