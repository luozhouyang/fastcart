use super::support::*;
use super::*;

#[tokio::test]
async fn endpoint_lifecycle_never_leaks_secrets() {
    let service = service().await;
    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/hooks".to_owned(),
            events: vec!["order.placed".to_owned(), "order.placed".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();

    assert!(created.secret.starts_with("whsec_"));
    assert_eq!(created.endpoint.events, vec!["order.placed"]);
    assert!(created.endpoint.is_active);

    let listed = service.list_endpoints().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], created.endpoint);

    let updated = service
        .update_endpoint(
            &created.endpoint.id,
            UpdateWebhookEndpointInput {
                events: Some(vec!["*".to_owned()]),
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.events, vec!["*"]);
    assert!(!updated.is_active);

    let rotated = service.rotate_secret(&created.endpoint.id).await.unwrap();
    assert_ne!(rotated, created.secret);
    assert!(rotated.starts_with("whsec_"));

    service.delete_endpoint(&created.endpoint.id).await.unwrap();
    assert!(service.list_endpoints().await.unwrap().is_empty());
    assert!(
        service
            .get_endpoint(&created.endpoint.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn rejects_private_and_plaintext_destinations_by_default() {
    let service = WebhookService::new(StoreDatabase::in_memory(StoreId::new()).await.unwrap());
    for url in [
        "http://example.com/hook",
        "https://127.0.0.1/hook",
        "https://localhost/hook",
        "https://169.254.169.254/latest/meta-data",
        "https://[::1]/hook",
    ] {
        service
            .create_endpoint(CreateWebhookEndpointInput {
                url: url.to_owned(),
                events: vec!["order.placed".to_owned()],
                ..Default::default()
            })
            .await
            .expect_err("private or plaintext URL was accepted");
    }
}

#[tokio::test]
async fn validates_urls_events_and_secrets() {
    let service = service().await;
    for url in [
        "",
        "ftp://example.com/hook",
        "file:///tmp/hook",
        "https://user:pass@example.com/hook",
        "not a url",
    ] {
        assert!(
            service
                .create_endpoint(CreateWebhookEndpointInput {
                    url: url.to_owned(),
                    events: vec!["order.placed".to_owned()],
                    ..Default::default()
                })
                .await
                .is_err(),
            "expected {url} to be rejected"
        );
    }
    assert!(
        service
            .create_endpoint(CreateWebhookEndpointInput {
                url: "https://example.com/hook".to_owned(),
                events: vec!["Bad Event!".to_owned()],
                ..Default::default()
            })
            .await
            .is_err()
    );
    assert!(
        service
            .create_endpoint(CreateWebhookEndpointInput {
                url: "https://example.com/hook".to_owned(),
                events: vec!["order.placed".to_owned()],
                secret: Some("short".to_owned()),
                ..Default::default()
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn endpoint_mutations_append_audit_entries_atomically() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = WebhookService::new(db.clone())
        .with_private_networks_allowed(true)
        .with_audit_actor(Some("user_1".to_owned()));

    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/hooks".to_owned(),
            events: vec!["order.placed".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();
    service
        .update_endpoint(
            &created.endpoint.id,
            UpdateWebhookEndpointInput {
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    service.rotate_secret(&created.endpoint.id).await.unwrap();
    service.delete_endpoint(&created.endpoint.id).await.unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec![
            "webhook.created",
            "webhook.updated",
            "webhook.secret_rotated",
            "webhook.deleted",
        ]
    );
    assert!(audits.iter().all(|audit| audit.processed_at.is_none()));
    assert!(
        audits
            .iter()
            .all(|audit| audit.actor_id.as_deref() == Some("user_1"))
    );

    // A failed deletion is a no-op and must not append an audit row.
    assert!(service.delete_endpoint(&created.endpoint.id).await.is_err());
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 4);
}

#[tokio::test]
async fn no_op_updates_skip_audit_and_missing_endpoints_return_not_found() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = WebhookService::new(db.clone())
        .with_private_networks_allowed(true)
        .with_audit_actor(Some("user_1".to_owned()));
    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/hooks".to_owned(),
            events: vec!["order.placed".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();

    let unchanged = service
        .update_endpoint(
            &created.endpoint.id,
            UpdateWebhookEndpointInput {
                url: Some(created.endpoint.url.clone()),
                events: Some(created.endpoint.events.clone()),
                is_active: Some(true),
            },
        )
        .await
        .unwrap();
    assert_eq!(unchanged, created.endpoint);
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 1);

    service.delete_endpoint(&created.endpoint.id).await.unwrap();
    let error = service
        .update_endpoint(
            &created.endpoint.id,
            UpdateWebhookEndpointInput {
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(error.is_not_found());
    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec!["webhook.created", "webhook.deleted"]
    );
}

#[tokio::test]
async fn deleting_an_endpoint_removes_its_deliveries() {
    let service = service().await;
    let created = service
        .create_endpoint(CreateWebhookEndpointInput {
            url: "https://example.com/hook".to_owned(),
            events: vec!["*".to_owned()],
            ..Default::default()
        })
        .await
        .unwrap();
    service
        .fan_out(&DomainEventEnvelope::new(
            "order.placed",
            None,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(
        service
            .list_deliveries(&created.endpoint.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );

    service.delete_endpoint(&created.endpoint.id).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webhook_deliveries")
        .fetch_one(service.db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}
