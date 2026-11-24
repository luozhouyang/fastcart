//! Event fan-out and job processing.

use super::adapter::payload_string;
use super::deliveries::{
    ChannelDeliveryClaim, claim_delivery_row, job_will_exhaust, load_channel_row_including_deleted,
    load_delivery_row, mark_delivery_failed, mark_delivery_skipped, update_channel_error,
};
use super::*;

impl NotificationChannelService {
    /// Fans one domain event out to every subscribed channel.
    ///
    /// A unique `(channel_id, event_id)` delivery row makes replays
    /// idempotent. Rendering failures are isolated to one channel; an enqueue
    /// failure returns an error so the outbox can retry the whole event.
    pub async fn dispatch(
        &self,
        db: &StoreDatabase,
        queue: &JobQueue,
        event: &DomainEventEnvelope,
        context: &RenderContext,
    ) -> Result<ChannelDispatchReport, AppError> {
        let rows = sqlx::query_as::<_, ChannelRow>(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM notification_channels
             WHERE is_active = 1 AND deleted_at IS NULL
             ORDER BY created_at ASC, id ASC"
        ))
        .fetch_all(db.pool())
        .await
        .map_err(database_error)?;

        let mut report = ChannelDispatchReport::default();
        for row in rows {
            let events = match row.events() {
                Ok(events) => events,
                Err(_) => {
                    report.skipped += 1;
                    tracing::warn!(channel_id = %row.id, "notification channel has invalid events");
                    continue;
                }
            };
            if !events
                .iter()
                .any(|event_type| event_type == &event.event_type)
            {
                continue;
            }
            let Ok(kind) = ChannelKind::parse(&row.kind) else {
                report.skipped += 1;
                tracing::warn!(channel_id = %row.id, "notification channel has unknown kind");
                continue;
            };
            let Some(adapter) = self.adapters.get(&kind) else {
                report.skipped += 1;
                tracing::warn!(channel_id = %row.id, "notification channel adapter is not registered");
                continue;
            };

            let channel_id = NotificationChannelId::from_string(row.id.clone());
            let (delivery_id, job_id) = match claim_delivery_row(db, &channel_id, event).await? {
                ChannelDeliveryClaim::Claimed {
                    delivery_id,
                    job_id,
                } => (delivery_id, job_id),
                ChannelDeliveryClaim::Duplicate => {
                    report.duplicates += 1;
                    continue;
                }
            };

            let message = match adapter.render(db, event, context).await {
                Ok(message) => message,
                Err(error) => {
                    mark_delivery_failed(
                        db,
                        &delivery_id,
                        Some(job_id.as_str()),
                        &error.to_string(),
                    )
                    .await?;
                    report.failed += 1;
                    tracing::warn!(
                        channel_id = %channel_id,
                        event_id = %event.id,
                        event_type = %event.event_type,
                        "notification channel render failed"
                    );
                    continue;
                }
            };

            let payload = json!({
                "channel_id": channel_id.as_str(),
                "delivery_id": delivery_id.as_str(),
                "job_id": job_id.as_str(),
                "event_id": event.id.as_str(),
                "event_type": event.event_type,
                "message": message,
            });
            // The job id was persisted in the atomic claim above, so the queued
            // row and the delivery row always reference the same job.
            if let Err(error) = queue
                .enqueue_with_id(
                    &job_id,
                    NewJob::new(SEND_CHANNEL_NOTIFICATION_JOB_TYPE, payload)
                        .with_priority(CHANNEL_PRIORITY),
                )
                .await
            {
                mark_delivery_failed(
                    db,
                    &delivery_id,
                    Some(job_id.as_str()),
                    "notification channel job could not be enqueued",
                )
                .await?;
                tracing::warn!(
                    channel_id = %channel_id,
                    event_id = %event.id,
                    event_type = %event.event_type,
                    "notification channel job enqueue failed"
                );
                return Err(AppError::database(error.to_string()));
            }
            report.queued += 1;
        }

        Ok(report)
    }
}

impl NotificationChannelService {
    /// Processes one `send_channel_notification` job payload.
    ///
    /// Retryable provider failures return [`ChannelError::Retryable`] so the
    /// job queue applies its backoff. Permanent failures are recorded on the
    /// delivery row and return [`ChannelJobOutcome::Failed`].
    pub async fn process_job(
        &self,
        db: &StoreDatabase,
        payload: &Value,
    ) -> Result<ChannelJobOutcome, ChannelError> {
        let channel_id = NotificationChannelId::from_string(payload_string(payload, "channel_id")?);
        let delivery_id =
            NotificationDeliveryId::from_string(payload_string(payload, "delivery_id")?);
        let job_id = payload_string(payload, "job_id")?;
        let _event_id = payload_string(payload, "event_id")?;
        let _event_type = payload_string(payload, "event_type")?;
        let message = payload
            .get("message")
            .cloned()
            .ok_or_else(|| ChannelError::Permanent("job payload is missing message".to_owned()))?;

        let delivery = load_delivery_row(db, &delivery_id)
            .await
            .map_err(|error| retryable_db("delivery lookup failed", &error))?
            .ok_or_else(|| ChannelError::Permanent("notification delivery not found".to_owned()))?;
        let status = DeliveryStatus::parse(&delivery.status).map_err(|_| {
            ChannelError::Permanent("notification delivery status is invalid".to_owned())
        })?;
        match status {
            // Sent, skipped and failed are terminal: a replayed or retried job
            // must not send another message or revive the delivery.
            DeliveryStatus::Sent => return Ok(ChannelJobOutcome::Sent),
            DeliveryStatus::Skipped => return Ok(ChannelJobOutcome::Skipped),
            DeliveryStatus::Failed => return Ok(ChannelJobOutcome::Failed),
            DeliveryStatus::Pending | DeliveryStatus::Queued => {}
        }
        if delivery.job_id.as_deref() != Some(job_id) {
            // A stale job from an earlier claim must neither send nor mutate
            // the row, which may now belong to a newer job.
            tracing::warn!(
                delivery_id = %delivery_id,
                job_id = %job_id,
                "ignoring stale notification channel job"
            );
            return Ok(ChannelJobOutcome::Failed);
        }

        let channel = load_channel_row_including_deleted(db, &channel_id)
            .await
            .map_err(|error| retryable_db("channel lookup failed", &error))?;
        let Some(channel) = channel else {
            mark_delivery_skipped(
                db,
                &delivery_id,
                Some(job_id),
                "notification channel does not exist",
            )
            .await?;
            return Ok(ChannelJobOutcome::Skipped);
        };
        if channel.deleted_at.is_some() || channel.is_active == 0 {
            mark_delivery_skipped(
                db,
                &delivery_id,
                Some(job_id),
                "notification channel is inactive",
            )
            .await?;
            return Ok(ChannelJobOutcome::Skipped);
        }

        let result = feishu::send(
            &self.client,
            &channel.endpoint_url,
            channel.signing_secret.as_deref(),
            &message,
        )
        .await;

        match result.result {
            Ok(()) => {
                let now = now_timestamp();
                sqlx::query(
                    "UPDATE notification_channel_deliveries
                     SET status = 'sent', response_status = ?, response_body = ?,
                         last_error = NULL, sent_at = ?, updated_at = ?,
                         attempts = attempts + 1
                     WHERE id = ? AND job_id = ?",
                )
                .bind(result.status)
                .bind(result.body.as_deref())
                .bind(now)
                .bind(now)
                .bind(delivery_id.as_str())
                .bind(job_id)
                .execute(db.pool())
                .await
                .map_err(|error| retryable_db("delivery update failed", &error))?;
                sqlx::query(
                    "UPDATE notification_channels
                     SET last_success_at = ?, last_error = NULL, updated_at = ?
                     WHERE id = ?",
                )
                .bind(now)
                .bind(now)
                .bind(channel_id.as_str())
                .execute(db.pool())
                .await
                .map_err(|error| retryable_db("channel update failed", &error))?;
                Ok(ChannelJobOutcome::Sent)
            }
            Err(ChannelError::Retryable(message)) => {
                let now = now_timestamp();
                // On the last attempt keep a terminal state so the delivery
                // does not remain queued forever.
                let exhausted = job_will_exhaust(db, job_id).await;
                let status = if exhausted { "failed" } else { "queued" };
                let query = "UPDATE notification_channel_deliveries
                     SET status = ?, response_status = ?, response_body = ?,
                         last_error = ?, updated_at = ?, attempts = attempts + 1
                     WHERE id = ? AND job_id = ?";
                sqlx::query(query)
                    .bind(status)
                    .bind(result.status)
                    .bind(result.body.as_deref())
                    .bind(&message)
                    .bind(now)
                    .bind(delivery_id.as_str())
                    .bind(job_id)
                    .execute(db.pool())
                    .await
                    .map_err(|error| retryable_db("delivery update failed", &error))?;
                update_channel_error(db, &channel_id, &message).await?;
                Err(ChannelError::Retryable(message))
            }
            Err(ChannelError::Permanent(message)) => {
                let now = now_timestamp();
                sqlx::query(
                    "UPDATE notification_channel_deliveries
                     SET status = 'failed', response_status = ?, response_body = ?,
                         last_error = ?, updated_at = ?, attempts = attempts + 1
                     WHERE id = ? AND job_id = ?",
                )
                .bind(result.status)
                .bind(result.body.as_deref())
                .bind(&message)
                .bind(now)
                .bind(delivery_id.as_str())
                .bind(job_id)
                .execute(db.pool())
                .await
                .map_err(|error| retryable_db("delivery update failed", &error))?;
                update_channel_error(db, &channel_id, &message).await?;
                Ok(ChannelJobOutcome::Failed)
            }
        }
    }
}
