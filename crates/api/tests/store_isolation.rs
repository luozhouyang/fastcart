//! Store isolation security suite.
//!
//! These tests exercise the isolation guarantees required by the production
//! hardening plan: admin, token, customer, publishable-key, cache, job, event,
//! resolver and internal-surface boundaries. Data is seeded through the module
//! services so each test can assert against real per-store databases.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{
    DatabaseOptions, JobQueue, NewJob, StoreCache, StoreDatabaseManager, append_outbox_event,
    fetch_pending_outbox,
};
use fastcart_kernel::{DomainEventEnvelope, Metadata, StoreId};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput, Product};
use fastcart_module_order::{CreateOrderInput, CreateOrderItemInput, Order, OrderService};
use fastcart_module_store::{
    CreatePublishableApiKeyInput, CreateSalesChannelInput, CreatedPublishableApiKey, RegionService,
    SalesChannel, StoreService,
};
use fastcart_platform::{CreateUserInput, PlatformStore};
use http_body_util::BodyExt;
use tower::ServiceExt;

const PASSWORD: &str = "hunter2hunter2";
const INTERNAL_SECRET: &str = "test-internal-secret";

struct Fixture {
    app: Router,
    context: Arc<AppContext>,
    admin_auth: String,
    csrf: String,
    store_a: StoreId,
    store_b: StoreId,
    _dir: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let config = ApiConfig {
        cookie_secure: false,
        backup_dir: dir.path().join("backups"),
        internal_fetch_secret: Some(INTERNAL_SECRET.to_owned()),
        ..ApiConfig::default()
    };
    let context = Arc::new(AppContext::new(
        config,
        platform,
        databases,
        StoreCache::default_capacity(),
    ));
    let app = build_router(context.clone());

    let (status, _, cookies) = request(
        &app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(serde_json::json!({
            "email": "owner@example.com",
            "password": PASSWORD
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let admin_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let store_a = create_store(
        &app,
        &admin_auth,
        &csrf,
        "Alpha",
        "alpha",
        "alpha.example.com",
    )
    .await;
    let store_b = create_store(&app, &admin_auth, &csrf, "Beta", "beta", "beta.example.com").await;

    Fixture {
        app,
        context,
        admin_auth,
        csrf,
        store_a,
        store_b,
        _dir: dir,
    }
}

async fn create_store(
    app: &Router,
    admin_auth: &str,
    csrf: &str,
    name: &str,
    slug: &str,
    domain: &str,
) -> StoreId {
    let (status, store, _) = request(
        app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": name,
            "slug": slug,
            "primary_domain": domain
        })),
        &[("cookie", admin_auth), ("x-csrf-token", csrf)],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    StoreId::from_string(store["id"].as_str().unwrap())
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value, Vec<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value, cookies)
}

fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

fn admin_headers<'a>(fixture: &'a Fixture, store: &'a str) -> [(&'a str, &'a str); 3] {
    [
        ("cookie", fixture.admin_auth.as_str()),
        ("x-csrf-token", fixture.csrf.as_str()),
        ("x-store-id", store),
    ]
}

async fn seed_product(
    context: &AppContext,
    store_id: &StoreId,
    title: &str,
    sku: &str,
    publish: bool,
) -> Product {
    let db = context.databases.database(store_id).await.unwrap();
    let catalog = CatalogService::new(db);
    let product = catalog
        .create_product(CreateProductInput {
            title: title.to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                sku: Some(sku.to_owned()),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    if publish {
        catalog.publish_product(&product.id).await.unwrap();
    }
    product
}

async fn seed_order(context: &AppContext, store_id: &StoreId, email: &str) -> Order {
    let db = context.databases.database(store_id).await.unwrap();
    OrderService::new(db)
        .create_order(CreateOrderInput {
            email: email.to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Isolation Hoodie".to_owned(),
                unit_price: 1_000,
                quantity: 1,
                ..Default::default()
            }],
            subtotal: 1_000,
            total: 1_000,
            ..Default::default()
        })
        .await
        .unwrap()
}

async fn seed_storefront_catalog(
    context: &AppContext,
    store_id: &StoreId,
    title: &str,
    sku: &str,
) -> (Product, SalesChannel, CreatedPublishableApiKey) {
    let db = context.databases.database(store_id).await.unwrap();
    let channel = StoreService::new(db.clone())
        .create_sales_channel(CreateSalesChannelInput {
            name: format!("{title} channel"),
            description: None,
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    let key = RegionService::new(db.clone())
        .create_publishable_key(CreatePublishableApiKeyInput {
            title: format!("{title} key"),
            sales_channel_ids: vec![channel.id.clone()],
        })
        .await
        .unwrap();
    let catalog = CatalogService::new(db);
    let product = catalog
        .create_product(CreateProductInput {
            title: title.to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                sku: Some(sku.to_owned()),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    catalog
        .set_product_sales_channels(&product.id, std::slice::from_ref(&channel.id))
        .await
        .unwrap();
    (product, channel, key)
}

#[tokio::test]
async fn admin_cannot_read_or_mutate_another_stores_catalog_or_orders() {
    let fixture = fixture().await;
    let product_a = seed_product(
        &fixture.context,
        &fixture.store_a,
        "Alpha Hoodie",
        "ALPHA-1",
        false,
    )
    .await;
    let product_b = seed_product(
        &fixture.context,
        &fixture.store_b,
        "Beta Hoodie",
        "BETA-1",
        false,
    )
    .await;
    let order_a = seed_order(&fixture.context, &fixture.store_a, "alpha@example.com").await;
    let order_b = seed_order(&fixture.context, &fixture.store_b, "beta@example.com").await;

    let store_a = fixture.store_a.as_str();
    let store_b = fixture.store_b.as_str();

    // Store B only sees its own catalog.
    let (status, products, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/products",
        None,
        &admin_headers(&fixture, store_b),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["pagination"]["total"], 1);
    assert_eq!(products["data"][0]["id"], product_b.id);

    // Store A products are not addressable through Store B.
    let (status, _, _) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/admin/catalog/products/{}", product_a.id),
        None,
        &admin_headers(&fixture, store_b),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) = request(
        &fixture.app,
        "PATCH",
        &format!("/api/v1/admin/catalog/products/{}", product_a.id),
        Some(serde_json::json!({"title": "Hijacked"})),
        &admin_headers(&fixture, store_b),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The attempted cross-store write left Store A untouched.
    let (status, product, _) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/admin/catalog/products/{}", product_a.id),
        None,
        &admin_headers(&fixture, store_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{product}");
    assert_eq!(product["title"], "Alpha Hoodie");

    // Orders are equally isolated.
    let (status, orders, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/orders",
        None,
        &admin_headers(&fixture, store_b),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{orders}");
    assert_eq!(orders["pagination"]["total"], 1);
    assert_eq!(orders["data"][0]["id"], order_b.id.as_str());

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/admin/orders/{}", order_a.id.as_str()),
        None,
        &admin_headers(&fixture, store_b),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/orders/{}/cancel", order_a.id.as_str()),
        Some(serde_json::json!({})),
        &admin_headers(&fixture, store_b),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Store A's order is still intact and uncanceled.
    let (status, order, _) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/admin/orders/{}", order_a.id.as_str()),
        None,
        &admin_headers(&fixture, store_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{order}");
    assert_eq!(order["status"], "pending");
}

#[tokio::test]
async fn admin_session_and_api_token_cannot_select_another_store() {
    let fixture = fixture().await;
    let store_a = fixture.store_a.as_str();
    let store_b = fixture.store_b.as_str();

    // A user who only belongs to Store A cannot select Store B.
    let member = fixture
        .context
        .auth
        .register(
            CreateUserInput::new("member@example.com"),
            "member-password",
        )
        .await
        .unwrap();
    fixture
        .context
        .platform
        .upsert_membership(&member.id, &fixture.store_a, &["owner"])
        .await
        .unwrap();

    let (status, _, cookies) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/auth/login",
        Some(serde_json::json!({
            "email": "member@example.com",
            "password": "member-password"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let member_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/products",
        None,
        &[("cookie", member_auth.as_str()), ("x-store-id", store_a)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/products",
        None,
        &[("cookie", member_auth.as_str()), ("x-store-id", store_b)],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A Store A token cannot be replayed against a real Store B.
    let (status, created, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/platform/api-tokens",
        Some(serde_json::json!({
            "name": "store-a-automation",
            "store_id": store_a,
            "scopes": ["products.read"]
        })),
        &[
            ("cookie", fixture.admin_auth.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let token = created["token"].as_str().unwrap();

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/products",
        None,
        &[
            ("authorization", &format!("Bearer {token}")),
            ("x-store-id", store_a),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/products",
        None,
        &[
            ("authorization", &format!("Bearer {token}")),
            ("x-store-id", store_b),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn customer_sessions_are_scoped_to_the_resolved_store() {
    let fixture = fixture().await;

    // The same email can identify distinct customer rows in each store.
    let (status, _, cookies_a) = request(
        &fixture.app,
        "POST",
        "/api/v1/store/customers",
        Some(serde_json::json!({
            "email": "buyer@example.com",
            "password": PASSWORD
        })),
        &[("host", "alpha.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session_a = cookie_value(&cookies_a, "fc_customer_session").unwrap();

    let (status, _, cookies_b) = request(
        &fixture.app,
        "POST",
        "/api/v1/store/customers",
        Some(serde_json::json!({
            "email": "buyer@example.com",
            "password": PASSWORD
        })),
        &[("host", "beta.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session_b = cookie_value(&cookies_b, "fc_customer_session").unwrap();

    let cookie_a = format!("fc_customer_session={session_a}");
    let cookie_b = format!("fc_customer_session={session_b}");

    // Store A's session is not recognized by Store B, even for the same email.
    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/customers/me",
        None,
        &[("host", "beta.example.com"), ("cookie", cookie_a.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/customers/me/orders",
        None,
        &[("host", "beta.example.com"), ("cookie", cookie_a.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Store B's session is not recognized by Store A.
    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/customers/me",
        None,
        &[("host", "alpha.example.com"), ("cookie", cookie_b.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Each session still works against its own store.
    let (status, me, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/customers/me",
        None,
        &[("host", "alpha.example.com"), ("cookie", cookie_a.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["customer"]["email"], "buyer@example.com");
}

#[tokio::test]
async fn publishable_keys_do_not_cross_store_boundaries() {
    let fixture = fixture().await;
    let (product_a, channel_a, key_a) =
        seed_storefront_catalog(&fixture.context, &fixture.store_a, "Alpha Shirt", "ALPHA-S").await;
    let (product_b, channel_b, key_b) =
        seed_storefront_catalog(&fixture.context, &fixture.store_b, "Beta Shirt", "BETA-S").await;

    // Store A's key lists only Store A's product and channel.
    let (status, products, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[
            ("host", "alpha.example.com"),
            ("x-publishable-api-key", key_a.token.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["count"], 1);
    assert_eq!(products["products"][0]["id"], product_a.id);
    assert_eq!(
        products["products"][0]["sales_channel_ids"],
        serde_json::json!([channel_a.id])
    );

    // Store A's key cannot authenticate against Store B's database.
    let (status, body, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[
            ("host", "beta.example.com"),
            ("x-publishable-api-key", key_a.token.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.get("products").is_none());

    // The inverse is blocked as well.
    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[
            ("host", "alpha.example.com"),
            ("x-publishable-api-key", key_b.token.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Store B's key still sees Store B's own catalog.
    let (status, products, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[
            ("host", "beta.example.com"),
            ("x-publishable-api-key", key_b.token.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["count"], 1);
    assert_eq!(products["products"][0]["id"], product_b.id);
    assert_eq!(
        products["products"][0]["sales_channel_ids"],
        serde_json::json!([channel_b.id])
    );
}

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

#[tokio::test]
async fn admin_membership_is_checked_against_the_selected_store() {
    let fixture = fixture().await;
    let member = fixture
        .context
        .auth
        .register(
            CreateUserInput::new("store-a-member@example.com"),
            "member-password",
        )
        .await
        .unwrap();
    fixture
        .context
        .platform
        .upsert_membership(&member.id, &fixture.store_a, &["admin"])
        .await
        .unwrap();

    let (status, _, cookies) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/auth/login",
        Some(serde_json::json!({
            "email": "store-a-member@example.com",
            "password": "member-password"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let auth = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/products",
        None,
        &[
            ("cookie", auth.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The same session is rejected when a store the user does not belong to is
    // selected, before the store database middleware can open it.
    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/products",
        None,
        &[
            ("cookie", auth.as_str()),
            ("x-store-id", fixture.store_b.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn internal_storefront_surface_requires_the_secret_and_resolves_per_host() {
    let fixture = fixture().await;
    let product_a = seed_product(
        &fixture.context,
        &fixture.store_a,
        "Alpha Internal",
        "ALPHA-I",
        true,
    )
    .await;
    let product_b = seed_product(
        &fixture.context,
        &fixture.store_b,
        "Beta Internal",
        "BETA-I",
        true,
    )
    .await;

    // Missing and incorrect secrets are rejected.
    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/internal/storefront/products",
        None,
        &[("host", "alpha.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/internal/storefront/products",
        None,
        &[
            ("host", "alpha.example.com"),
            ("x-fastcart-internal-secret", "wrong-secret"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The correct secret still resolves the trusted host, not a client store.
    let (status, products, _) = request(
        &fixture.app,
        "GET",
        "/internal/storefront/products",
        None,
        &[
            ("host", "alpha.example.com"),
            ("x-fastcart-internal-secret", INTERNAL_SECRET),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["count"], 1);
    assert_eq!(products["products"][0]["id"], product_a.id);

    let (status, products, _) = request(
        &fixture.app,
        "GET",
        "/internal/storefront/products",
        None,
        &[
            ("host", "beta.example.com"),
            ("x-fastcart-internal-secret", INTERNAL_SECRET),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["count"], 1);
    assert_eq!(products["products"][0]["id"], product_b.id);
}
