//! Fan-out, delivery claiming, sending and retries.

use super::security::{ensure_public_resolution, read_response_body, truncate_bytes};
use super::*;

impl WebhookService {
    /// Creates one delivery per subscribed active endpoint.
    ///
    /// The unique `(endpoint_id, event_id)` index makes fan-out idempotent, so
    /// an outbox replay can never duplicate a delivery.
    pub async fn fan_out(&self, event: &DomainEventEnvelope) -> Result<u64, AppError> {
        let payload = serde_json::to_string(event)
            .map_err(|error| AppError::internal(format!("event serialization failed: {error}")))?;
        let endpoints = sqlx::query_as::<_, EndpointRow>(&format!(
            "SELECT {ENDPOINT_COLUMNS} FROM webhook_endpoints WHERE is_active = 1"
        ))
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;

        let now = now_timestamp();
        let mut created = 0_u64;
        for endpoint in endpoints {
            let events = endpoint.events()?;
            if !events
                .iter()
                .any(|pattern| pattern == "*" || pattern == &event.event_type)
            {
                continue;
            }
            let id = WebhookDeliveryId::new();
            let result = sqlx::query(
                "INSERT OR IGNORE INTO webhook_deliveries
                 (id, endpoint_id, event_id, event_type, payload, status, attempts,
                  created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, 'pending', 0, ?, ?)",
            )
            .bind(id.as_str())
            .bind(endpoint.id.as_str())
            .bind(event.id.as_str())
            .bind(&event.event_type)
            .bind(&payload)
            .bind(now)
            .bind(now)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
            created += result.rows_affected();
        }
        Ok(created)
    }
}

impl WebhookService {
    /// Lists deliveries for an endpoint, newest first.
    pub async fn list_deliveries(
        &self,
        endpoint_id: &WebhookId,
        limit: i64,
    ) -> Result<Vec<WebhookDelivery>, AppError> {
        let rows = sqlx::query_as::<_, DeliveryRow>(&format!(
            "SELECT {DELIVERY_COLUMNS} FROM webhook_deliveries
             WHERE endpoint_id = ?
             ORDER BY created_at DESC, id DESC
             LIMIT ?"
        ))
        .bind(endpoint_id.as_str())
        .bind(limit.clamp(1, 100))
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(DeliveryRow::into_delivery).collect()
    }
}

impl WebhookService {
    /// Loads one delivery.
    pub async fn get_delivery(&self, id: &WebhookDeliveryId) -> Result<WebhookDelivery, AppError> {
        let row = sqlx::query_as::<_, DeliveryRow>(&format!(
            "SELECT {DELIVERY_COLUMNS} FROM webhook_deliveries WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("webhook delivery", id))?;
        row.into_delivery()
    }
}

impl WebhookService {
    /// Requeues a delivery for another attempt.
    ///
    /// A delivery currently claimed by a worker cannot be replayed concurrently.
    pub async fn replay(&self, id: &WebhookDeliveryId) -> Result<WebhookDelivery, AppError> {
        let result = sqlx::query(
            "UPDATE webhook_deliveries
             SET status = 'pending', attempts = 0, response_status = NULL,
                 response_body = NULL, next_retry_at = NULL, updated_at = ?
             WHERE id = ? AND status != 'delivering'",
        )
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            let existing = self.get_delivery(id).await?;
            if existing.status == DeliveryStatus::Delivering {
                return Err(AppError::conflict(
                    "webhook delivery is currently being processed",
                ));
            }
            return Err(AppError::not_found("webhook delivery", id));
        }
        self.get_delivery(id).await
    }
}

impl WebhookService {
    /// Claims and sends deliveries that are due, recording every outcome.
    ///
    /// Network calls happen outside any transaction. The claim is a single
    /// atomic `UPDATE ... RETURNING`, so overlapping workers cannot send the
    /// same delivery concurrently.
    pub async fn deliver_due(
        &self,
        now: Timestamp,
        limit: i64,
    ) -> Result<DeliveryReport, AppError> {
        self.recover_stale(now).await?;
        let rows = self.claim_due(now, limit).await?;
        let mut report = DeliveryReport::default();

        for row in rows {
            report.attempted += 1;
            let endpoint = sqlx::query_as::<_, EndpointRow>(&format!(
                "SELECT {ENDPOINT_COLUMNS} FROM webhook_endpoints WHERE id = ?"
            ))
            .bind(&row.endpoint_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
            let Some(endpoint) = endpoint else {
                // The endpoint was deleted after the claim; the delivery row
                // cascades away with it.
                report.skipped += 1;
                continue;
            };
            if endpoint.is_active == 0 {
                self.reschedule_pending(&row.id, now.saturating_add(300))
                    .await?;
                report.skipped += 1;
                continue;
            }

            let outcome = match self.send(&row, &endpoint, now).await {
                Ok(outcome) => outcome,
                // A delivery-specific failure (for example a host that no
                // longer resolves) must not abandon the rest of the claimed
                // batch. Record it and continue with the next delivery.
                Err(error) => self.record_send_failure(&row, now, &error).await?,
            };
            match outcome {
                SendOutcome::Delivered => report.delivered += 1,
                SendOutcome::Retried => report.retried += 1,
                SendOutcome::Failed => report.failed += 1,
            }
        }
        Ok(report)
    }
}

impl WebhookService {
    /// Records a delivery-level send failure and reschedules it when attempts
    /// remain. Infrastructure failures while persisting the outcome propagate.
    async fn record_send_failure(
        &self,
        row: &DeliveryRow,
        now: Timestamp,
        error: &AppError,
    ) -> Result<SendOutcome, AppError> {
        let retry = row.attempts < i64::from(self.max_attempts);
        let status = if retry {
            DeliveryStatus::Pending
        } else {
            DeliveryStatus::Failed
        };
        let next_retry_at = retry.then(|| now.saturating_add(retry_delay_seconds(row.attempts)));
        let response_body = truncate_bytes(error.to_string().as_bytes());
        sqlx::query(
            "UPDATE webhook_deliveries
             SET status = ?, response_status = NULL, response_body = ?,
                 next_retry_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(&response_body)
        .bind(next_retry_at)
        .bind(now_timestamp())
        .bind(&row.id)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(if retry {
            SendOutcome::Retried
        } else {
            SendOutcome::Failed
        })
    }
}

impl WebhookService {
    pub(super) async fn fetch_endpoint_row(&self, id: &WebhookId) -> Result<EndpointRow, AppError> {
        sqlx::query_as::<_, EndpointRow>(&format!(
            "SELECT {ENDPOINT_COLUMNS} FROM webhook_endpoints WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("webhook endpoint", id))
    }
}

impl WebhookService {
    /// Returns deliveries claimed by a worker that never recorded an outcome
    /// (for example after a process crash) to the pending queue.
    async fn recover_stale(&self, now: Timestamp) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE webhook_deliveries
             SET status = 'pending', next_retry_at = ?, updated_at = ?
             WHERE status = 'delivering' AND updated_at <= ?",
        )
        .bind(now)
        .bind(now)
        .bind(now.saturating_sub(300))
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }
}

impl WebhookService {
    async fn claim_due(&self, now: Timestamp, limit: i64) -> Result<Vec<DeliveryRow>, AppError> {
        let query = format!(
            "UPDATE webhook_deliveries
             SET status = 'delivering', attempts = attempts + 1, updated_at = ?1
             WHERE id IN (
                 SELECT d.id FROM webhook_deliveries d
                 JOIN webhook_endpoints e ON e.id = d.endpoint_id
                 WHERE d.status = 'pending'
                   AND (d.next_retry_at IS NULL OR d.next_retry_at <= ?1)
                   AND e.is_active = 1
                 ORDER BY d.created_at ASC, d.id ASC
                 LIMIT ?2
             )
             RETURNING {DELIVERY_COLUMNS}"
        );
        sqlx::query_as::<_, DeliveryRow>(&query)
            .bind(now)
            .bind(limit.clamp(1, 100))
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)
    }
}

impl WebhookService {
    async fn send(
        &self,
        row: &DeliveryRow,
        endpoint: &EndpointRow,
        now: Timestamp,
    ) -> Result<SendOutcome, AppError> {
        if !self.allow_private_networks {
            ensure_public_resolution(&endpoint.url).await?;
        }
        let timestamp = now_timestamp();
        let signature = self.signatures(endpoint, timestamp, &row.payload)?;
        let response = self
            .client
            .post(&endpoint.url)
            .header("content-type", "application/json")
            .header("x-fastcart-event", &row.event_type)
            .header("x-fastcart-delivery", &row.id)
            .header("x-fastcart-timestamp", timestamp.to_string())
            .header("x-fastcart-signature", signature)
            .timeout(DELIVERY_TIMEOUT)
            .body(row.payload.clone())
            .send()
            .await;

        let (delivered, response_status, response_body) = match response {
            Ok(response) => {
                let status = i64::from(response.status().as_u16());
                let successful = response.status().is_success();
                let body = read_response_body(response).await;
                (successful, Some(status), Some(body))
            }
            Err(error) => (
                false,
                None,
                Some(truncate_bytes(error.to_string().as_bytes())),
            ),
        };

        let outcome = if delivered {
            sqlx::query(
                "UPDATE webhook_deliveries
                 SET status = 'delivered', response_status = ?, response_body = ?,
                     next_retry_at = NULL, updated_at = ?
                 WHERE id = ?",
            )
            .bind(response_status)
            .bind(&response_body)
            .bind(timestamp)
            .bind(&row.id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
            SendOutcome::Delivered
        } else {
            let retry = row.attempts < i64::from(self.max_attempts);
            let status = if retry {
                DeliveryStatus::Pending
            } else {
                DeliveryStatus::Failed
            };
            let next_retry_at =
                retry.then(|| now.saturating_add(retry_delay_seconds(row.attempts)));
            sqlx::query(
                "UPDATE webhook_deliveries
                 SET status = ?, response_status = ?, response_body = ?,
                     next_retry_at = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(status.as_str())
            .bind(response_status)
            .bind(&response_body)
            .bind(next_retry_at)
            .bind(timestamp)
            .bind(&row.id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
            if retry {
                SendOutcome::Retried
            } else {
                SendOutcome::Failed
            }
        };
        Ok(outcome)
    }
}

impl WebhookService {
    async fn reschedule_pending(&self, id: &str, next_retry_at: Timestamp) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE webhook_deliveries
             SET status = 'pending', attempts = MAX(attempts - 1, 0),
                 next_retry_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(next_retry_at)
        .bind(now_timestamp())
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }
}
