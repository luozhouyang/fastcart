use super::support::*;
use super::*;

#[tokio::test]
async fn serves_robots_and_a_published_page_sitemap() {
    let universal = Arc::new(RecordingUniversal::default());
    let fixture = fixture(Some(universal.clone())).await;
    let content = ContentService::new(fixture.database.clone());
    for (title, handle, page_type) in [
        ("Home", "home", "home"),
        ("About", "about", "custom"),
        ("Draft", "draft", "custom"),
    ] {
        let page = content
            .create_page(CreatePageInput {
                title: title.to_owned(),
                handle: Some(handle.to_owned()),
                page_type: Some(page_type.to_owned()),
                body: serde_json::json!({ "schema_version": 1, "content": [] }),
                seo: serde_json::json!({}),
                metadata: Metadata::empty(),
            })
            .await
            .unwrap();
        if handle != "draft" {
            content.publish_page(&page.id).await.unwrap();
        }
    }

    let (status, robots) = send(&fixture.app, "/robots.txt", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{robots}");
    assert!(robots.contains("Sitemap: https://shop.example.com/sitemap.xml"));

    // A draft handle change must not leak into the sitemap: the storefront
    // still serves the published handle.
    let contact = content
        .create_page(CreatePageInput {
            title: "Contact".to_owned(),
            handle: Some("contact".to_owned()),
            page_type: Some("custom".to_owned()),
            body: serde_json::json!({ "schema_version": 1, "content": [] }),
            seo: serde_json::json!({}),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    content.publish_page(&contact.id).await.unwrap();
    content
        .update_page(
            &contact.id,
            UpdatePageInput {
                handle: Some("contact-us".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let (status, sitemap) = send(&fixture.app, "/sitemap.xml", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{sitemap}");
    assert!(
        sitemap.contains("<loc>https://shop.example.com/</loc>"),
        "{sitemap}"
    );
    assert!(
        sitemap.contains("<loc>https://shop.example.com/about</loc>"),
        "{sitemap}"
    );
    assert!(
        sitemap.contains("<loc>https://shop.example.com/contact</loc>"),
        "{sitemap}"
    );
    assert!(!sitemap.contains("contact-us"), "{sitemap}");
    assert!(!sitemap.contains("draft"), "{sitemap}");
    assert!(sitemap.contains("<lastmod>"), "{sitemap}");
    assert!(
        universal.seen.lock().unwrap().is_empty(),
        "static discovery routes must not invoke the runtime"
    );
}

#[tokio::test]
async fn discovery_files_work_without_ssr() {
    let fixture = fixture(None).await;
    let (status, robots) = send(&fixture.app, "/robots.txt", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{robots}");
    let (status, sitemap) = send(&fixture.app, "/sitemap.xml", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{sitemap}");
    let (status, _) = send(&fixture.app, "/", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A TLS-terminating proxy can declare the public scheme.
    let (status, robots) = send(
        &fixture.app,
        "/robots.txt",
        "shop.example.com",
        &[("x-forwarded-proto", "http")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{robots}");
    assert!(robots.contains("http://shop.example.com/sitemap.xml"));
}

#[tokio::test]
async fn does_not_render_api_health_or_unknown_host_paths() {
    let universal = Arc::new(RecordingUniversal::default());
    let fixture = fixture(Some(universal.clone())).await;

    let (status, _) = send(&fixture.app, "/api/v1/unknown", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&fixture.app, "/health/nope", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&fixture.app, "/", "unknown.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(universal.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn storefront_injects_enabled_plugin_embeds() {
    let html = "<!doctype html><html><head><title>x</title></head><body>hello</body></html>";
    let fixture = fixture(Some(Arc::new(RecordingUniversal {
        html: Some(html),
        ..Default::default()
    })))
    .await;
    let manifest = serde_json::json!({
        "schema_version": "1",
        "id": "chat",
        "name": "Chat",
        "version": "1.0.0",
        "api_version": 2,
        "runtime": { "engine": "workerd", "compatibility_date": "2026-11-01" },
        "entry": { "type": "worker", "path": "index.js" },
        "capabilities": { "storefront": ["store.read"] },
        "extensions": {
            "storefront": [{
                "kind": "embed",
                "placement": "body:end",
                "entry": "storefront/boot.js",
                "css": "storefront/boot.css",
                "pages": ["*"]
            }]
        }
    });
    sqlx::query(
        "INSERT INTO plugin_installations          (id, plugin_id, digest, version, manifest_json, enabled, settings_json,           settings_revision, granted_scopes, installed_at, updated_at)          VALUES ('pli_test', 'chat', ?, '1.0.0', ?, 1, '{}', 1, '[]', 0, 0)",
    )
    .bind("a".repeat(64))
    .bind(manifest.to_string())
    .execute(fixture.database.pool())
    .await
    .unwrap();

    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", "shop.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes);
    assert!(body.contains("fastcart:extensions-injected"));
    assert!(body.contains("/_fastcart/extensions/runtime.js"));
    assert!(body.contains("/_fastcart/extensions/chat/"));
    assert!(body.contains("boot.css"));
}

#[tokio::test]
async fn extension_items_are_served_from_the_cached_catalog() {
    let fixture = fixture(None).await;
    let manifest = |pages: serde_json::Value| {
        serde_json::json!({
            "schema_version": "1",
            "id": "chat",
            "name": "Chat",
            "version": "1.0.0",
            "api_version": 2,
            "runtime": { "engine": "workerd", "compatibility_date": "2026-11-01" },
            "entry": { "type": "worker", "path": "index.js" },
            "capabilities": { "storefront": ["store.read"] },
            "extensions": {
                "storefront": [{
                    "kind": "embed",
                    "placement": "body:end",
                    "entry": "storefront/boot.js",
                    "pages": pages
                }]
            }
        })
        .to_string()
    };
    let insert = |manifest: String| {
        let pool = fixture.database.pool().clone();
        async move {
            sqlx::query(
                "INSERT INTO plugin_installations \
                 (id, plugin_id, digest, version, manifest_json, enabled, settings_json, \
                  settings_revision, granted_scopes, installed_at, updated_at) \
                 VALUES ('pli_cache', 'chat', ?, '1.0.0', ?, 1, '{}', 1, '[]', 0, 0)",
            )
            .bind("b".repeat(64))
            .bind(manifest)
            .execute(&pool)
            .await
            .unwrap();
        }
    };

    insert(manifest(serde_json::json!(["/products/*"]))).await;

    let store_id = fastcart_kernel::StoreId::from_string(fixture.store_id.clone());
    let manifest_of = async || {
        let catalog =
            fastcart_api::routes::plugin_catalog::load(&fixture.context, &fixture.database)
                .await
                .expect("catalog loads");
        catalog
            .plugin("chat")
            .and_then(|plugin| plugin.manifest.extensions.storefront.first())
            .and_then(|extension| extension.pages.first())
            .cloned()
    };

    assert_eq!(manifest_of().await.as_deref(), Some("/products/*"));

    // A direct database write is invisible until the cached generation moves.
    sqlx::query("UPDATE plugin_installations SET manifest_json = ? WHERE plugin_id = 'chat'")
        .bind(manifest(serde_json::json!(["/collections/*"])))
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(manifest_of().await.as_deref(), Some("/products/*"));

    fixture.context.cache.invalidate_store(&store_id);
    assert_eq!(manifest_of().await.as_deref(), Some("/collections/*"));
}

#[tokio::test]
async fn extension_items_respect_pages_and_exclude_paths() {
    let fixture = fixture(None).await;
    let manifest = serde_json::json!({
        "schema_version": "1",
        "id": "chat",
        "name": "Chat",
        "version": "1.0.0",
        "api_version": 2,
        "runtime": { "engine": "workerd", "compatibility_date": "2026-11-01" },
        "entry": { "type": "worker", "path": "index.js" },
        "capabilities": { "storefront": ["store.read"] },
        "extensions": {
            "storefront": [{
                "kind": "embed",
                "placement": "body:end",
                "entry": "storefront/boot.js",
                "pages": ["/products/*", "/"],
                "exclude_paths": ["/products/hidden/*"]
            }]
        }
    });
    sqlx::query(
        "INSERT INTO plugin_installations \
         (id, plugin_id, digest, version, manifest_json, enabled, settings_json, \
          settings_revision, granted_scopes, installed_at, updated_at) \
         VALUES ('pli_pages', 'chat', ?, '1.0.0', ?, 1, '{}', 1, '[]', 0, 0)",
    )
    .bind("c".repeat(64))
    .bind(manifest.to_string())
    .execute(fixture.database.pool())
    .await
    .unwrap();

    let items = async |path: &str| {
        let items = fastcart_api::routes::storefront_extensions::items_for_path(
            &fixture.context,
            &fixture.database,
            path,
        )
        .await
        .expect("items resolve");
        items.len()
    };

    assert_eq!(items("/").await, 1, "an exact page match loads the embed");
    assert_eq!(
        items("/products/linen-shirt").await,
        1,
        "the prefix matches"
    );
    assert_eq!(items("/products/hidden/draft").await, 0, "exclusions win");
    assert_eq!(items("/collections/all").await, 0, "unlisted paths skip it");
}
