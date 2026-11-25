use super::support::*;
use super::*;

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
