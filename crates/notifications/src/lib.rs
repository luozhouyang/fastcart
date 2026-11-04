//! Transactional notification rendering and dispatch.
//!
//! Notifications subscribe to domain events, render localized templates and
//! enqueue provider-independent email jobs. Delivery records make repeated
//! event handling idempotent.
//!
//! # Delivery semantics
//!
//! A delivery row is claimed as `pending` before the email job is enqueued and
//! is marked `sent` only after the job is durably queued. Every failure before
//! that point releases the claim, so a later outbox retry can enqueue the
//! notification again. A crash after the job is queued but before the delivery
//! row is marked `sent` may produce a duplicate email. Delivery is therefore
//! **at-least-once**: repeated replays of a completed event are suppressed by
//! the unique `(event_id, template)` constraint, but an interrupted attempt can
//! duplicate rather than silently drop a notification.

mod catalog;
mod context;
mod service;

pub use catalog::{NotificationRule, RULES, TemplateRegistry, rule_for};
pub use context::{TemplateContext, TemplateItem, format_amount};
pub use service::{NotificationError, NotificationOutcome, NotificationService};

#[cfg(test)]
mod tests {
    use fastcart_email::{EmailService, SendEmailJob};
    use fastcart_infrastructure::{JobQueue, StoreDatabase, jobs::status};
    use fastcart_kernel::{DomainEventEnvelope, StoreId};
    use serde_json::json;

    use super::{NotificationOutcome, NotificationService, rule_for};

    async fn database() -> StoreDatabase {
        StoreDatabase::in_memory(StoreId::new()).await.unwrap()
    }

    fn service() -> NotificationService {
        NotificationService::new()
    }

    fn email() -> EmailService {
        EmailService::with_default_registry()
    }

    fn order_placed_event(store_id: &StoreId) -> DomainEventEnvelope {
        DomainEventEnvelope::new(
            "order.placed",
            Some(store_id.clone()),
            json!({
                "order_id": "order_missing",
                "display_number": 42,
                "email": "Customer@Example.com",
                "currency_code": "USD",
                "total": 1999
            }),
        )
    }

    async fn delivery_count(db: &StoreDatabase) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM notification_deliveries")
            .fetch_one(db.pool())
            .await
            .unwrap()
    }

    #[test]
    fn maps_the_required_lifecycle_events() {
        for event_type in [
            "order.placed",
            "order.fulfilled",
            "order.refunded",
            "payment.captured",
            "payment.failed",
            "order.return_received",
            "order.claim_accepted",
            "order.exchange_accepted",
        ] {
            assert!(rule_for(event_type).is_some(), "{event_type} is unmapped");
        }
        assert!(rule_for("product.updated").is_none());
    }

    #[tokio::test]
    async fn duplicate_events_enqueue_one_email() {
        let db = database().await;
        let queue = JobQueue::new(db.pool().clone());
        let email = email();
        let service = service();
        let event = order_placed_event(db.store_id());

        let first = service.dispatch(&db, &email, &queue, &event).await.unwrap();
        assert!(matches!(
            first,
            NotificationOutcome::Sent {
                template: "order_confirmation",
                ..
            }
        ));

        let second = service.dispatch(&db, &email, &queue, &event).await.unwrap();
        assert!(matches!(
            second,
            NotificationOutcome::Duplicate {
                template: "order_confirmation"
            }
        ));
        assert_eq!(queue.count_by_status(status::PENDING).await.unwrap(), 1);
        assert_eq!(delivery_count(&db).await, 1);

        let claimed = queue.claim("test", 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].job_type, "send_email");
        let payload: SendEmailJob = serde_json::from_str(&claimed[0].payload).unwrap();
        assert_eq!(payload.to, "customer@example.com");
        assert!(payload.text.as_deref().unwrap().contains("42"));
        let html = payload.html.as_deref().unwrap();
        assert!(
            html.contains("order <strong>#42</strong>"),
            "html was {html}"
        );
    }

    #[tokio::test]
    async fn unknown_events_are_skipped() {
        let db = database().await;
        let queue = JobQueue::new(db.pool().clone());
        let service = service();
        let event = DomainEventEnvelope::new(
            "product.updated",
            Some(db.store_id().clone()),
            json!({"product_id": "prod_1"}),
        );

        let outcome = service
            .dispatch(&db, &email(), &queue, &event)
            .await
            .unwrap();
        assert_eq!(outcome, NotificationOutcome::Unsupported);
        assert!(queue.claim("test", 10).await.unwrap().is_empty());
        assert_eq!(delivery_count(&db).await, 0);
    }

    #[tokio::test]
    async fn events_without_a_recipient_are_skipped() {
        let db = database().await;
        let queue = JobQueue::new(db.pool().clone());
        let service = service();
        let event = DomainEventEnvelope::new(
            "order.refunded",
            Some(db.store_id().clone()),
            json!({
                "order_id": "order_missing",
                "refund_id": "refund_1",
                "amount": 500,
                "currency_code": "USD"
            }),
        );

        let outcome = service
            .dispatch(&db, &email(), &queue, &event)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            NotificationOutcome::MissingRecipient {
                template: "refund_confirmation"
            }
        );
        assert!(queue.claim("test", 10).await.unwrap().is_empty());
        assert_eq!(delivery_count(&db).await, 0);
    }

    #[tokio::test]
    async fn a_stale_pending_claim_is_retried() {
        let db = database().await;
        let queue = JobQueue::new(db.pool().clone());
        let service = service();
        let event = order_placed_event(db.store_id());

        sqlx::query(
            "INSERT INTO notification_deliveries
             (id, event_id, event_type, template, status, attempts, created_at)
             VALUES ('delivery_1', ?, 'order.placed', 'order_confirmation', 'pending', 0, 0)",
        )
        .bind(event.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

        let outcome = service
            .dispatch(&db, &email(), &queue, &event)
            .await
            .unwrap();
        assert!(outcome.is_sent());
        assert_eq!(queue.count_by_status(status::PENDING).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn enqueue_failure_releases_the_delivery_claim() {
        let db = database().await;
        let queue = JobQueue::new(db.pool().clone());
        let service = service();
        let event = order_placed_event(db.store_id());
        sqlx::query("DROP TABLE jobs")
            .execute(db.pool())
            .await
            .unwrap();

        let error = service
            .dispatch(&db, &email(), &queue, &event)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("email"));
        assert_eq!(delivery_count(&db).await, 0);
    }
}
