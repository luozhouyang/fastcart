//! Outbox dispatch and provider-refund reconciliation.

use fastcart_email::EmailService;
use fastcart_infrastructure::{
    EventBus, JobQueue, StoreDatabase, fetch_pending_outbox, mark_outbox_failed,
    mark_outbox_processed,
};
use fastcart_kernel::{AppError, DomainEventEnvelope, OrderId, StoreId};
use fastcart_module_order::OrderService;
use fastcart_module_webhook::WebhookService;
use fastcart_notifications::{
    NotificationChannelService, NotificationOutcome, NotificationService, RenderContext,
};

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_outbox(
    store_id: &StoreId,
    database: &StoreDatabase,
    queue: &JobQueue,
    events: &EventBus,
    email: &EmailService,
    notifications: &NotificationService,
    channels: &NotificationChannelService,
    render_context: &RenderContext,
    allow_private_webhooks: bool,
    report: &mut MaintenanceReport,
) {
    match fetch_pending_outbox(database.pool(), 100).await {
        Ok(records) => {
            let webhooks = WebhookService::new(database.clone())
                .with_private_networks_allowed(allow_private_webhooks);
            for record in records {
                let mut fan_out_failed = false;
                let mut notification_failed = false;
                let mut channel_failed = false;
                let mut refund_failed = false;
                if let Some(event) = record.to_envelope(Some(store_id.clone())) {
                    // Fan out before publishing so a database failure keeps the
                    // outbox row retryable.
                    if let Err(error) = webhooks.fan_out(&event).await {
                        tracing::warn!(store_id = %store_id, event_id = %event.id, error = %error, "webhook fan-out failed");
                        report.failures += 1;
                        fan_out_failed = true;
                    } else {
                        let _ = events.publish(event.clone());
                    }
                    // Notifications are derived consumers: a failure is retried
                    // with the outbox row but never aborts the sweep.
                    match notifications.dispatch(database, email, queue, &event).await {
                        Ok(NotificationOutcome::Sent { .. }) => {
                            report.notifications_sent += 1;
                        }
                        Ok(NotificationOutcome::Duplicate { .. }) => {
                            report.notifications_duplicates += 1;
                        }
                        Ok(
                            NotificationOutcome::Unsupported
                            | NotificationOutcome::MissingRecipient { .. },
                        ) => {
                            report.notifications_skipped += 1;
                        }
                        Err(error) => {
                            tracing::warn!(
                                store_id = %store_id,
                                event_id = %event.id,
                                event_type = %event.event_type,
                                error = %error,
                                "notification dispatch failed"
                            );
                            report.failures += 1;
                            report.notifications_failed += 1;
                            notification_failed = true;
                        }
                    }
                    // Channel notifications are independent of email: rendering
                    // or transport failures are isolated to one channel.
                    match channels
                        .dispatch(database, queue, &event, render_context)
                        .await
                    {
                        Ok(channel_report) => {
                            report.channel_notifications_queued += channel_report.queued;
                            report.channel_notifications_duplicates += channel_report.duplicates;
                            report.channel_notifications_failed += channel_report.failed;
                        }
                        Err(error) => {
                            tracing::warn!(
                                store_id = %store_id,
                                event_id = %event.id,
                                event_type = %event.event_type,
                                error = %error,
                                "channel notification dispatch failed"
                            );
                            report.failures += 1;
                            report.channel_notifications_failed += 1;
                            channel_failed = true;
                        }
                    }
                    // A provider-reported refund failure must revert the local
                    // order ledger before the outbox row is marked processed.
                    // A consumer error keeps the row retryable.
                    if event.event_type == "payment.refund_failed" {
                        match reconcile_failed_refund(database, &event).await {
                            Ok(()) => {}
                            Err(error) => {
                                tracing::warn!(
                                    store_id = %store_id,
                                    event_id = %event.id,
                                    error = %error,
                                    "order refund failure reconciliation failed"
                                );
                                report.failures += 1;
                                refund_failed = true;
                            }
                        }
                    }
                }
                if fan_out_failed || notification_failed || channel_failed || refund_failed {
                    let reason = if fan_out_failed && notification_failed {
                        "webhook fan-out and notification dispatch failed"
                    } else if fan_out_failed {
                        "webhook fan-out failed"
                    } else if notification_failed && channel_failed {
                        "email and channel notification dispatch failed"
                    } else if notification_failed {
                        "notification dispatch failed"
                    } else if channel_failed {
                        "channel notification dispatch failed"
                    } else {
                        "order refund failure reconciliation failed"
                    };
                    if let Err(error) =
                        mark_outbox_failed(database.pool(), &record.id, reason).await
                    {
                        tracing::warn!(store_id = %store_id, error = %error, "outbox failure recording failed");
                    }
                    continue;
                }
                if let Err(error) = mark_outbox_processed(database.pool(), &record.id).await {
                    tracing::warn!(store_id = %store_id, error = %error, "outbox dispatch failed");
                    let _ =
                        mark_outbox_failed(database.pool(), &record.id, &error.to_string()).await;
                    report.failures += 1;
                } else {
                    report.outbox_dispatched += 1;
                }
            }
        }
        Err(error) => {
            tracing::warn!(store_id = %store_id, error = %error, "outbox read failed");
            report.failures += 1;
        }
    }
}

/// Applies a provider-reported refund failure to the local order ledger.
///
/// Payments without an order are a no-op. Missing fields are consumer errors so
/// the outbox row is retried instead of being silently dropped.
pub(super) async fn reconcile_failed_refund(
    database: &StoreDatabase,
    event: &DomainEventEnvelope,
) -> Result<(), AppError> {
    let Some(order_id) = event
        .data
        .get("order_id")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(());
    };
    let payment_id = event
        .data
        .get("payment_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::validation("payment.refund_failed is missing payment_id"))?;
    let amount = event
        .data
        .get("amount")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| AppError::validation("payment.refund_failed is missing amount"))?;
    OrderService::new(database.clone())
        .mark_payment_refund_failed(&OrderId::from_string(order_id), payment_id, amount)
        .await?;
    Ok(())
}
