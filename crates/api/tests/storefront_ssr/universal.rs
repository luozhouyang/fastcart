use super::support::*;
use super::*;

#[tokio::test]
async fn universal_runtime_receives_trusted_context() {
    let universal = Arc::new(RecordingUniversal::default());
    let fixture = fixture(Some(universal.clone())).await;
    let request = Request::builder()
        .uri("/products/example")
        .header("host", "shop.example.com")
        .header("x-fastcart-store-id", "attacker")
        .body(Body::empty())
        .unwrap();
    let response = fixture.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"<h1>universal</h1>");
    let seen = universal.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].store.store_id.as_str().starts_with("store_"));
    assert!(seen[0].request_id.is_some());
    assert_eq!(seen[0].scheme, "https");
}

#[tokio::test]
async fn storefront_receives_only_resolvable_capability_grants() {
    let universal = Arc::new(RecordingUniversal::default());
    let fixture = fixture(Some(universal.clone())).await;
    let plugin_manifest = serde_json::json!({
        "schema_version": "1",
        "id": "chat",
        "name": "Live chat",
        "version": "1.0.0",
        "api_version": 2,
        "runtime": { "engine": "workerd", "compatibility_date": "2026-11-01" },
        "entry": { "type": "worker", "path": "worker/index.js" },
        "provides": [{
            "kind": "storefront_capability",
            "id": "chat",
            "version": 1,
            "title": "Live chat"
        }]
    })
    .to_string();
    sqlx::query(
        "INSERT INTO plugin_installations \
         (id, plugin_id, digest, version, manifest_json, enabled, installed_at, updated_at) \
         VALUES ('install_chat', 'chat', 'digest_chat', '1.0.0', ?, 1, 1, 1)",
    )
    .bind(&plugin_manifest)
    .execute(fixture.database.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO plugin_installations \
         (id, plugin_id, digest, version, manifest_json, enabled, installed_at, updated_at) \
         VALUES ('install_disabled', 'disabled', 'digest_disabled', '1.0.0', ?, 0, 1, 1)",
    )
    .bind(&plugin_manifest)
    .execute(fixture.database.pool())
    .await
    .unwrap();
    for (capability_id, plugin_id) in [("chat", "chat"), ("reviews", "chat"), ("media", "disabled")]
    {
        sqlx::query(
            "INSERT INTO store_capability_bindings \
             (capability_id, version, plugin_id, config_json, enabled, created_at, updated_at) \
             VALUES (?, 1, ?, '{\"greeting\":\"hi\"}', 1, 1, 1)",
        )
        .bind(capability_id)
        .bind(plugin_id)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    }

    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/products/example")
                .header("host", "shop.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let seen = universal.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let grant = seen[0].capabilities.get("chat").expect("chat grant");
    assert_eq!(grant.version, 1);
    assert_eq!(grant.endpoint, "/store/capabilities/chat");
    assert_eq!(grant.config["greeting"], "hi");
    // The plugin manifest does not provide `reviews`, and `media`'s plugin is
    // disabled, so neither binding reaches the theme.
    assert!(!seen[0].capabilities.contains_key("reviews"));
    assert!(!seen[0].capabilities.contains_key("media"));
}

#[tokio::test]
async fn internal_ssr_namespace_requires_token_and_uses_path_store() {
    let fixture = fixture(None).await;
    let context_path = format!("/internal/ssr/v1/stores/{}/context", fixture.store_id);

    let authorized = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&context_path)
                .header("authorization", "Bearer ssr-internal-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    let body = authorized.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["store_id"], fixture.store_id);

    let rejected = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&context_path)
                .header("authorization", "Bearer wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);

    let unknown = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/internal/ssr/v1/stores/store_missing/context")
                .header("authorization", "Bearer ssr-internal-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

    let cross_store = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&context_path)
                .header("authorization", "Bearer other-store-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cross_store.status(), StatusCode::FORBIDDEN);

    let preview_read = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&context_path)
                .header("authorization", "Bearer preview-store-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview_read.status(), StatusCode::OK);

    let preview_write = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/stores/{}/carts",
                    fixture.store_id
                ))
                .header("authorization", "Bearer preview-store-secret")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview_write.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn universal_assets_are_served_without_store_resolution() {
    let universal = Arc::new(RecordingUniversal {
        preview: true,
        ..RecordingUniversal::default()
    });
    let active = fixture(Some(universal)).await;
    let preview_token = issue_preview_token(
        "preview-secret-that-is-at-least-32-bytes",
        &active.store_id,
        "page_1",
        now_timestamp(),
    )
    .expect("preview token");
    let preview_token_value = preview_token.token.clone();
    let preview_jti = preview_token.jti;
    let response = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/assets/assets/app.js")
                .header("host", "shop.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.starts_with(b"asset:store_"));
    assert!(body.ends_with(b":assets/app.js"));

    let sveltekit_asset = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_app/immutable/app.js")
                .header("host", "shop.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sveltekit_asset.status(), StatusCode::OK);
    let body = sveltekit_asset
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert!(body.ends_with(b":_app/immutable/app.js"));

    let preview_page = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/preview/?token={preview_token_value}"))
                .header("host", "shop.example.com")
                .header("x-forwarded-proto", "https")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview_page.status(), StatusCode::OK);
    let set_cookie = preview_page
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|value| {
            let value = value.to_str().ok()?;
            value.starts_with("fastcart_preview=").then_some(value)
        })
        .expect("preview asset cookie")
        .to_owned();
    assert!(set_cookie.contains("Path=/_fastcart/preview-assets"));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("Secure"));
    assert!(set_cookie.contains("SameSite=None"));
    assert!(set_cookie.contains("Partitioned"));
    let cookie = set_cookie
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();

    let http_preview = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/preview/?token={preview_token_value}"))
                .header("host", "shop.example.com")
                .header("x-forwarded-proto", "http")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let http_cookie = http_preview
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|value| {
            let value = value.to_str().ok()?;
            value.starts_with("fastcart_preview=").then_some(value)
        })
        .expect("http preview cookie");
    assert!(http_cookie.contains("SameSite=Lax"));
    assert!(!http_cookie.contains("Secure"));
    assert!(!http_cookie.contains("Partitioned"));

    let unauthenticated = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/preview-assets/app.js")
                .header("host", "shop.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::FORBIDDEN);

    let preview_asset = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/preview-assets/app.js")
                .header("host", "shop.example.com")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview_asset.status(), StatusCode::OK);
    assert_eq!(
        preview_asset
            .headers()
            .get("vary")
            .and_then(|value| value.to_str().ok()),
        Some("Cookie")
    );

    let other_store_token = issue_preview_token(
        "preview-secret-that-is-at-least-32-bytes",
        "store_other",
        "page_1",
        now_timestamp(),
    )
    .expect("other store token")
    .token;
    let wrong_store = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/preview-assets/app.js")
                .header("host", "shop.example.com")
                .header("cookie", format!("fastcart_preview={other_store_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_store.status(), StatusCode::FORBIDDEN);

    sqlx::query(
        "INSERT OR REPLACE INTO preview_token_revocations (jti, page_id, expires_at) VALUES (?, ?, ?)",
    )
    .bind(&preview_jti)
    .bind("page_1")
    .bind(now_timestamp() + 900)
    .execute(active.database.pool())
    .await
    .expect("revoke preview token");
    active
        .preview_asset_authorizations
        .invalidate(&format!("{}:{preview_jti}", active.store_id))
        .await;
    let revoked_asset = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/preview-assets/app.js")
                .header("host", "shop.example.com")
                .header("cookie", format!("fastcart_preview={preview_token_value}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked_asset.status(), StatusCode::FORBIDDEN);

    let body = preview_asset
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert!(body.starts_with(b"preview-asset:store_"));
    assert!(body.ends_with(b":app.js"));

    let no_runtime = fixture(None).await;
    let missing = no_runtime
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/assets/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn preview_route_uses_the_universal_preview_deployment() {
    let universal = Arc::new(RecordingUniversal {
        preview: true,
        ..RecordingUniversal::default()
    });
    let fixture = fixture(Some(universal)).await;
    let token = issue_preview_token(
        "preview-secret-that-is-at-least-32-bytes",
        &fixture.store_id,
        "page_test",
        now_timestamp(),
    )
    .expect("preview token");
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/preview/products?token={}", token.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"<h1>universal-preview</h1>");
}
