use super::support::*;
use super::*;

#[tokio::test]
async fn cache_entries_and_invalidation_do_not_cross_stores() {
    let cache = &fixture().await.context.cache;
    let first = StoreId::new();
    let second = StoreId::new();

    cache
        .insert(&first, "catalog", "product-count", &"alpha")
        .await;
    cache
        .insert(&second, "catalog", "product-count", &"beta")
        .await;

    assert_eq!(
        cache
            .get::<String>(&first, "catalog", "product-count")
            .await,
        Some("alpha".to_owned())
    );
    assert_eq!(
        cache
            .get::<String>(&second, "catalog", "product-count")
            .await,
        Some("beta".to_owned())
    );

    // Invalidating a whole store hides only that store's generation.
    cache.invalidate_store(&first);
    assert_eq!(
        cache
            .get::<String>(&first, "catalog", "product-count")
            .await,
        None
    );
    assert_eq!(
        cache
            .get::<String>(&second, "catalog", "product-count")
            .await,
        Some("beta".to_owned())
    );

    // Removing one entry cannot address another store's entry.
    cache
        .insert(&first, "catalog", "product-count", &"alpha-v2")
        .await;
    cache.remove(&second, "catalog", "product-count").await;
    assert_eq!(
        cache
            .get::<String>(&first, "catalog", "product-count")
            .await,
        Some("alpha-v2".to_owned())
    );
    assert_eq!(
        cache
            .get::<String>(&second, "catalog", "product-count")
            .await,
        None
    );
}

#[tokio::test]
async fn jobs_and_events_are_confined_to_their_store_database() {
    let fixture = fixture().await;
    let db_a = fixture
        .context
        .databases
        .database(&fixture.store_a)
        .await
        .unwrap();
    let db_b = fixture
        .context
        .databases
        .database(&fixture.store_b)
        .await
        .unwrap();

    // A job written to Store A's database cannot be claimed by Store B's queue.
    let queue_a = JobQueue::new(db_a.pool().clone());
    let queue_b = JobQueue::new(db_b.pool().clone());
    queue_a
        .enqueue(NewJob::new(
            "isolation_probe",
            serde_json::json!({"store": "a"}),
        ))
        .await
        .unwrap();

    assert!(queue_b.claim("worker-b", 10).await.unwrap().is_empty());
    let claimed = queue_a.claim("worker-a", 10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].job_type, "isolation_probe");
    assert_eq!(claimed[0].json_payload()["store"], "a");

    // Outbox rows also live in the owning store's database and carry its ID.
    let event_a = DomainEventEnvelope::new(
        "order.placed",
        Some(fixture.store_a.clone()),
        serde_json::json!({"order": "a"}),
    );
    let event_b = DomainEventEnvelope::new(
        "order.placed",
        Some(fixture.store_b.clone()),
        serde_json::json!({"order": "b"}),
    );
    append_outbox_event(db_a.pool(), &event_a).await.unwrap();
    append_outbox_event(db_b.pool(), &event_b).await.unwrap();

    let pending_a = fetch_pending_outbox(db_a.pool(), 10).await.unwrap();
    let pending_b = fetch_pending_outbox(db_b.pool(), 10).await.unwrap();
    assert_eq!(pending_a.len(), 1);
    assert_eq!(pending_b.len(), 1);
    assert_eq!(pending_a[0].id, event_a.id.as_str());
    assert_eq!(pending_b[0].id, event_b.id.as_str());
    let envelope_a = pending_a[0]
        .to_envelope(Some(fixture.store_a.clone()))
        .unwrap();
    assert_eq!(
        envelope_a.store_id.as_ref().map(|id| id.as_str()),
        Some(fixture.store_a.as_str())
    );

    // The in-process bus preserves the producing store on every envelope.
    let mut receiver = fixture.context.events.subscribe();
    let _ = fixture.context.events.publish(event_a);
    let _ = fixture.context.events.publish(event_b);
    let received_a = receiver.recv().await.unwrap();
    let received_b = receiver.recv().await.unwrap();
    assert_eq!(
        received_a.store_id.as_ref().map(|id| id.as_str()),
        Some(fixture.store_a.as_str())
    );
    assert_eq!(
        received_b.store_id.as_ref().map(|id| id.as_str()),
        Some(fixture.store_b.as_str())
    );
}

#[tokio::test]
async fn store_domain_changes_invalidate_resolver_state() {
    let fixture = fixture().await;
    seed_product(
        &fixture.context,
        &fixture.store_a,
        "Alpha Domain Product",
        "ALPHA-D",
        true,
    )
    .await;
    seed_product(
        &fixture.context,
        &fixture.store_b,
        "Beta Domain Product",
        "BETA-D",
        true,
    )
    .await;

    let store_a = fixture.store_a.as_str();

    // Prime the resolver cache for the original domain.
    let (status, products, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[("host", "alpha.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["products"][0]["title"], "Alpha Domain Product");

    // Adding a domain invalidates the resolver and the new host resolves.
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/platform/stores/{store_a}/domains"),
        Some(serde_json::json!({
            "hostname": "alpha-alt.example.com",
            "is_primary": false
        })),
        &[
            ("cookie", fixture.admin_auth.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, products, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[("host", "alpha-alt.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["products"][0]["title"], "Alpha Domain Product");

    // The new hostname is cached, then removal must evict it.
    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        &format!("/api/v1/platform/stores/{store_a}/domains/alpha-alt.example.com"),
        None,
        &[
            ("cookie", fixture.admin_auth.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[("host", "alpha-alt.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Store B's domain still resolves to Store B only.
    let (status, products, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[("host", "beta.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["products"][0]["title"], "Beta Domain Product");
}
