use super::support::*;
use super::*;

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
