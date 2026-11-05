//! End-to-end theme registry HTTP flow: register, activate, roll back and delete.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    app: Router,
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
        ..ApiConfig::default()
    };
    let context = Arc::new(AppContext::new(
        config,
        platform,
        databases,
        StoreCache::default_capacity(),
    ));
    Fixture {
        app: build_router(context),
        _dir: dir,
    }
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

fn theme_body(id: &str) -> Value {
    json!({
        "manifest": {
            "id": id,
            "name": format!("Theme {id}"),
            "version": "1.0.0",
            "runtime": "node",
            "entry": "dist/server.js",
            "settings_schema": "settings.schema.json",
            "assets_manifest": "assets.json",
            "editor": {
                "entry": "editor.js",
                "manifest": "editor.manifest.json",
                "schema_version": 1
            },
            "requires": { "data_api": 1, "capabilities": [] }
        },
        "bundle_path": format!("themes/{id}"),
        "settings_schema": {
            "type": "object",
            "properties": { "logo": { "type": "string" } },
            "required": ["logo"]
        },
        "settings": { "logo": "/logo.svg" },
        "editor_manifest": {
            "schema_version": 1,
            "categories": {
                "product": { "title": "Product", "components": ["ProductGrid"] }
            },
            "components": {
                "ProductGrid": {
                    "title": "Product Grid",
                    "category": "product",
                    "fields": {
                        "collection": {
                            "type": "data_source",
                            "source": "collection",
                            "required": true
                        },
                        "limit": { "type": "number", "min": 1, "max": 24, "default": 4 }
                    },
                    "defaultProps": { "limit": 4 },
                    "slots": []
                }
            }
        }
    })
}

#[tokio::test]
async fn theme_registry_admin_and_storefront_flow() {
    let fixture = fixture().await;
    let app = fixture.app;

    // Bootstrap an administrator and a store.
    let setup = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/platform/auth/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": "owner@example.com",
                        "password": "admin-pass-123"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(setup.status(), StatusCode::OK);
    let cookies: Vec<String> = setup
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let admin_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");
    let admin_headers = [
        ("cookie", admin_auth.as_str()),
        ("x-csrf-token", csrf.as_str()),
    ];

    let (status, store) = request(
        &app,
        "POST",
        "/api/v1/platform/stores",
        Some(json!({
            "name": "Demo",
            "slug": "demo",
            "primary_domain": "shop.example.com"
        })),
        &admin_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    let store_id = store["id"].as_str().unwrap().to_owned();
    let store_headers = [
        ("cookie", admin_auth.as_str()),
        ("x-csrf-token", csrf.as_str()),
        ("x-store-id", store_id.as_str()),
    ];
    let host = [("host", "shop.example.com")];

    // Registering a theme returns 201 and never reveals internal rows.
    let (status, first) = request(
        &app,
        "POST",
        "/api/v1/admin/themes",
        Some(theme_body("default")),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["manifest"]["id"], "default");
    assert_eq!(first["bundle_path"], "themes/default");
    assert_eq!(first["settings"]["logo"], "/logo.svg");
    assert_eq!(first["is_active"], false);
    assert_eq!(first["schema_version"], 1);
    let first_id = first["id"].as_str().unwrap().to_owned();

    let (status, editor_manifest) = request(
        &app,
        "GET",
        &format!("/api/v1/admin/themes/{first_id}/editor-manifest"),
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{editor_manifest}");
    assert!(editor_manifest["components"]["ProductGrid"].is_object());

    // Duplicate manifest identifiers conflict.
    let (status, duplicate) = request(
        &app,
        "POST",
        "/api/v1/admin/themes",
        Some(theme_body("default")),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");

    // Invalid manifests and settings are rejected.
    let mut invalid = theme_body("invalid");
    invalid["manifest"]["id"] = json!("Bad Theme");
    let (status, _) = request(
        &app,
        "POST",
        "/api/v1/admin/themes",
        Some(invalid),
        &store_headers,
    )
    .await;
    assert!(status.is_client_error());

    let mut invalid_settings = theme_body("invalid-settings");
    invalid_settings["settings"] = json!({});
    let (status, _) = request(
        &app,
        "POST",
        "/api/v1/admin/themes",
        Some(invalid_settings),
        &store_headers,
    )
    .await;
    assert!(status.is_client_error());

    // Administrative reads.
    let (status, listed) = request(&app, "GET", "/api/v1/admin/themes", None, &store_headers).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["pagination"]["total"], 1);

    let (status, fetched) = request(
        &app,
        "GET",
        &format!("/api/v1/admin/themes/{first_id}"),
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["id"], first_id);

    // No active theme yet.
    let (status, _) = request(
        &app,
        "GET",
        "/api/v1/admin/themes/active",
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = request(&app, "GET", "/api/v1/store/theme", None, &host).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Activate the first theme and expose it publicly.
    let (status, active) = request(
        &app,
        "POST",
        &format!("/api/v1/admin/themes/{first_id}/activate"),
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{active}");
    assert_eq!(active["is_active"], true);

    let (status, public) = request(&app, "GET", "/api/v1/store/theme", None, &host).await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["theme"]["manifest"]["id"], "default");
    assert_eq!(public["theme"]["settings"]["logo"], "/logo.svg");
    assert!(public["theme"].get("settings_schema").is_none());

    let (status, admin_active) = request(
        &app,
        "GET",
        "/api/v1/admin/themes/active",
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{admin_active}");
    assert_eq!(admin_active["id"], first_id);

    // Page writes validate against the active editor manifest and require an
    // optimistic revision token.
    let page_body = json!({
        "schema_version": 1,
        "content": [{
            "type": "ProductGrid",
            "props": {
                "id": "grid-1",
                "collection": {
                    "id": "col_1",
                    "handle": "featured",
                    "label": "Featured"
                },
                "limit": 4
            }
        }],
        "root": { "props": {} },
        "zones": {}
    });
    let (status, page) = request(
        &app,
        "POST",
        "/api/v1/admin/content/pages",
        Some(json!({
            "title": "Home",
            "handle": "home",
            "page_type": "home",
            "body": page_body,
            "seo": {}
        })),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{page}");
    assert_eq!(page["revision"], 1);
    let page_id = page["id"].as_str().unwrap().to_owned();

    let mut updated_body = page_body.clone();
    updated_body["content"][0]["props"]["limit"] = json!(8);
    let (status, updated) = request(
        &app,
        "PATCH",
        &format!("/api/v1/admin/content/pages/{page_id}"),
        Some(json!({ "base_revision": 1, "body": updated_body })),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["revision"], 2);

    let (status, conflict) = request(
        &app,
        "PATCH",
        &format!("/api/v1/admin/content/pages/{page_id}"),
        Some(json!({ "base_revision": 1, "body": page_body })),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");

    // A body that references a component missing from the active theme is
    // rejected before it can be persisted.
    let (status, rejected) = request(
        &app,
        "PATCH",
        &format!("/api/v1/admin/content/pages/{page_id}"),
        Some(json!({
            "base_revision": 2,
            "body": {
                "schema_version": 1,
                "content": [{ "type": "MissingComponent", "props": { "id": "x" } }],
                "root": { "props": {} },
                "zones": {}
            }
        })),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");

    let (status, unchanged) = request(
        &app,
        "GET",
        &format!("/api/v1/admin/content/pages/{page_id}"),
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(unchanged["revision"], 2);
    assert_eq!(
        unchanged["body"]["content"][0]["props"]["limit"],
        json!(8),
        "a rejected body must not be persisted"
    );

    let (status, versions) = request(
        &app,
        "GET",
        &format!("/api/v1/admin/content/pages/{page_id}/versions"),
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{versions}");
    assert_eq!(versions["data"][0]["kind"], "draft");

    let (status, picker) = request(
        &app,
        "GET",
        "/api/v1/admin/data-sources/page?search=Home",
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{picker}");
    assert!(
        picker["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["id"] == page_id))
    );

    // Register and activate a second theme, then roll back to the first.
    let (status, second) = request(
        &app,
        "POST",
        "/api/v1/admin/themes",
        Some(theme_body("alternate")),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let second_id = second["id"].as_str().unwrap().to_owned();

    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/v1/admin/themes/{second_id}/activate"),
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, public) = request(&app, "GET", "/api/v1/store/theme", None, &host).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(public["theme"]["manifest"]["id"], "alternate");

    let (status, rolled_back) = request(
        &app,
        "POST",
        "/api/v1/admin/themes/rollback",
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rolled_back}");
    assert_eq!(rolled_back["id"], first_id);
    assert_eq!(rolled_back["is_active"], true);

    // Settings are schema-validated.
    let (status, _) = request(
        &app,
        "PATCH",
        &format!("/api/v1/admin/themes/{first_id}/settings"),
        Some(json!({ "settings": { "brand_color": "#000" } })),
        &store_headers,
    )
    .await;
    assert!(status.is_client_error());

    let (status, updated) = request(
        &app,
        "PATCH",
        &format!("/api/v1/admin/themes/{first_id}/settings"),
        Some(json!({ "settings": { "logo": "/new-logo.svg" } })),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["settings"]["logo"], "/new-logo.svg");

    // The active theme cannot be deleted. The inactive theme is the target of
    // the latest activation's rollback, so deleting it is refused as well.
    let (status, _) = request(
        &app,
        "DELETE",
        &format!("/api/v1/admin/themes/{first_id}"),
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = request(
        &app,
        "DELETE",
        &format!("/api/v1/admin/themes/{second_id}"),
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = request(
        &app,
        "GET",
        "/api/v1/admin/themes/theme_missing",
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
