//! Admin plugin installation flow.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::StoreId;
use fastcart_platform::PlatformStore;
use fastcart_plugin::{ContextClaims, TokenActor, TokenSigner};
use flate2::Compression;
use flate2::write::GzEncoder;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tar::{Builder, Header};
use tower::ServiceExt;

const HOST: &str = "shop.example.com";
const RUNTIME_TOKEN: &str = "runtime-secret";

struct Fixture {
    app: Router,
    store_id: String,
    session: String,
    csrf: String,
    plugins_root: std::path::PathBuf,
    databases: Arc<StoreDatabaseManager>,
    _dir: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    fixture_with(true).await
}

async fn fixture_with(plugins_enabled: bool) -> Fixture {
    fixture_options(plugins_enabled, None).await
}

async fn fixture_options(plugins_enabled: bool, commerce_addr: Option<SocketAddr>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let plugins_root = dir.path().join("plugins");
    let context = Arc::new(AppContext::new(
        ApiConfig {
            cookie_secure: false,
            backup_dir: dir.path().join("backups"),
            storage_root: dir.path().join("assets"),
            plugins_root: plugins_root.clone(),
            plugins_enabled,
            plugin_runtime_token: Some(RUNTIME_TOKEN.to_owned()),
            plugin_context_secret: Some(b"test-secret".to_vec()),
            plugin_workerd_binary: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../experiments/workerd/bin/workerd"),
            plugin_runtime_dir: dir.path().join("runtime"),
            plugin_commerce_addr: commerce_addr
                .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 8080))),
            ..ApiConfig::default()
        },
        platform,
        databases.clone(),
        StoreCache::default_capacity(),
    ));
    let app = build_router(context);

    let setup = request(
        &app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(json!({ "email": "owner@example.com", "password": "admin-pass-123" })),
        &[],
    )
    .await;
    assert_eq!(setup.0, StatusCode::OK, "{}", setup.1);
    let session = cookie_value(&setup.2, "fc_admin_session").expect("session");
    let csrf = cookie_value(&setup.2, "fc_csrf").expect("csrf");
    let admin_cookie = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let store = request(
        &app,
        "POST",
        "/api/v1/platform/stores",
        Some(json!({ "name": "Demo", "slug": "demo", "primary_domain": HOST })),
        &[
            ("cookie", admin_cookie.clone()),
            ("x-csrf-token", csrf.clone()),
        ],
    )
    .await;
    assert_eq!(store.0, StatusCode::CREATED, "{}", store.1);
    let store_id = store.1["id"].as_str().unwrap().to_owned();
    databases
        .database(&StoreId::from_string(store_id.clone()))
        .await
        .unwrap();

    Fixture {
        app,
        store_id,
        session,
        csrf,
        plugins_root,
        databases,
        _dir: dir,
    }
}

impl Fixture {
    fn headers(&self) -> Vec<(&str, String)> {
        vec![
            (
                "cookie",
                format!("fc_admin_session={}; fc_csrf={}", self.session, self.csrf),
            ),
            ("x-csrf-token", self.csrf.clone()),
            ("x-store-id", self.store_id.clone()),
        ]
    }
}

fn manifest(id: &str, version: &str, extra_data: &[&str]) -> Value {
    let mut data = vec!["catalog.read"];
    data.extend_from_slice(extra_data);
    json!({
        "schema_version": "1",
        "id": id,
        "name": "Demo Plugin",
        "version": version,
        "api_version": 2,
        "runtime": { "engine": "workerd", "compatibility_date": "2026-11-01" },
        "entry": { "type": "worker", "path": "index.js" },
        "capabilities": {
            "data": data,
            "storage": ["kv.read", "kv.write"],
            "network": ["api.example.com"]
        },
        "extensions": {
            "admin": [
                { "target": "admin.nav.section", "label": "Demo", "route": "/apps/demo" },
                { "target": "admin.page.settings.tabs", "title": "Demo", "entry": "ui/settings.html" }
            ]
        },
        "settings": {
            "schema": {
                "type": "object",
                "properties": { "accent": { "type": "string" } },
                "additionalProperties": false
            }
        },
        "rpc": ["conversations.list"]
    })
}

fn archive(manifest: &Value, module: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = Builder::new(&mut encoder);
        append(&mut builder, "plugin.json", manifest.to_string().as_bytes());
        append(&mut builder, "index.js", module.as_bytes());
        append(
            &mut builder,
            "ui/settings.html",
            b"<!doctype html><p>settings</p>",
        );
        builder.finish().unwrap();
    }
    encoder.finish().unwrap()
}

fn append(builder: &mut Builder<&mut GzEncoder<Vec<u8>>>, path: &str, bytes: &[u8]) {
    let mut header = Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, path, bytes).unwrap();
}

fn multipart(boundary: &str, filename: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"archive\"; filename=\"{filename}\"\r\nContent-Type: application/gzip\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, String)],
) -> (StatusCode, Value, Vec<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, value);
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
    let cookies = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body, cookies)
}

async fn upload(
    app: &Router,
    uri: &str,
    headers: &[(&str, String)],
    archive: &[u8],
) -> (StatusCode, Value) {
    let boundary = "----fastcart-plugin-boundary";
    let body = multipart(boundary, "plugin.tar.gz", archive);
    let mut builder = Request::builder().method("POST").uri(uri).header(
        "content-type",
        format!("multipart/form-data; boundary={boundary}"),
    );
    for (name, value) in headers {
        builder = builder.header(*name, value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

#[tokio::test]
async fn install_list_enable_disable_and_bundle() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let document = manifest("demo", "1.0.0", &[]);
    let bytes = archive(&document, "export default {};\n// v1");

    let (status, body) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["id"], "demo");
    assert_eq!(body["enabled"], false);
    let digest = body["digest"].as_str().unwrap().to_owned();
    assert!(
        fixture
            .plugins_root
            .join("artifacts")
            .join(&digest)
            .is_dir()
    );

    let (status, list, _) =
        request(&fixture.app, "GET", "/api/v1/admin/plugins", None, &headers).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["plugins"].as_array().unwrap().len(), 1);
    assert_eq!(list["artifacts"].as_array().unwrap().len(), 1);

    let (status, plugin, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{plugin}");
    assert_eq!(plugin["enabled"], true);

    let (status, bundle) = internal_bundle(&fixture, "demo", &digest, RUNTIME_TOKEN).await;
    assert_eq!(status, StatusCode::OK, "{bundle}");
    assert_eq!(bundle["mainModule"], "index.js");
    assert!(
        bundle["modules"]["index.js"]
            .as_str()
            .unwrap()
            .contains("v1")
    );

    let (status, plugin, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/disable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plugin["enabled"], false);

    let (status, _) = internal_bundle(&fixture, "demo", &digest, RUNTIME_TOKEN).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

async fn internal_bundle(
    fixture: &Fixture,
    plugin: &str,
    digest: &str,
    token: &str,
) -> (StatusCode, Value) {
    let uri = format!(
        "/internal/plugins/v1/stores/{}/bundles/{plugin}?digest={digest}",
        fixture.store_id
    );
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

#[tokio::test]
async fn duplicate_install_is_rejected() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn invalid_archives_are_rejected() {
    let fixture = fixture().await;
    let headers = fixture.headers();

    let mut document = manifest("demo", "1.0.0", &[]);
    document["capabilities"]["data"] = json!(["catalog.write"]);
    let bytes = archive(&document, "export default {};");
    let (status, body) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let mut document = manifest("demo", "1.0.0", &[]);
    document["extensions"]["admin"][1]["target"] = json!("admin.page.orders.details.actions");
    let bytes = archive(&document, "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upgrade_requires_scope_acceptance_and_rolls_back() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let v1 = archive(&manifest("demo", "1.0.0", &[]), "export default {};\n// v1");
    let (status, installed) =
        upload(&fixture.app, "/api/v1/admin/plugins/install", &headers, &v1).await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    let v1_digest = installed["digest"].as_str().unwrap().to_owned();

    let v2 = archive(
        &manifest("demo", "1.1.0", &["customers.read"]),
        "export default {};\n// v2",
    );
    let (status, body) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/demo/upgrade",
        &headers,
        &v2,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, upgraded) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/demo/upgrade?accept_new_scopes=true",
        &headers,
        &v2,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{upgraded}");
    assert_eq!(upgraded["version"], "1.1.0");
    assert_eq!(upgraded["previous_digest"], v1_digest.as_str());

    let (status, rolled, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/rollback",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rolled}");
    assert_eq!(rolled["version"], "1.0.0");
    assert_eq!(rolled["digest"], v1_digest.as_str());
}

#[tokio::test]
async fn settings_are_validated_and_revisioned() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _, _) = request(
        &fixture.app,
        "PATCH",
        "/api/v1/admin/plugins/demo/settings",
        Some(json!({ "settings": { "accent": 42 } })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, plugin, _) = request(
        &fixture.app,
        "PATCH",
        "/api/v1/admin/plugins/demo/settings",
        Some(json!({ "settings": { "accent": "#2563eb" } })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{plugin}");
    assert_eq!(plugin["settings"]["accent"], "#2563eb");
    assert_eq!(plugin["settings_revision"], 2);
}

#[tokio::test]
async fn delete_requires_a_disabled_plugin() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        "/api/v1/admin/plugins/demo",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/disable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        "/api/v1/admin/plugins/demo",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn bundle_requires_the_runtime_token() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (_, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    let digest = installed["digest"].as_str().unwrap().to_owned();
    let (status, _) = internal_bundle(&fixture, "demo", &digest, "wrong-token").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let _ = std::io::stdout().flush();
}

#[tokio::test]
async fn install_requires_the_runtime_to_be_enabled() {
    let fixture = fixture_with(false).await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, body) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, list, _) =
        request(&fixture.app, "GET", "/api/v1/admin/plugins", None, &headers).await;
    assert_eq!(status, StatusCode::OK, "listing remains available");
    assert_eq!(list["plugins"].as_array().unwrap().len(), 0);

    let (status, _) = internal_bundle(&fixture, "demo", "deadbeef", RUNTIME_TOKEN).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn users_without_plugin_permissions_are_forbidden() {
    let fixture = fixture().await;
    let owner_headers = fixture.headers();
    let created = request(
        &fixture.app,
        "POST",
        "/api/v1/platform/users",
        Some(json!({
            "email": "agent@example.com",
            "password": "agent-pass-123",
            "store_id": fixture.store_id,
            "roles": ["support-agent"]
        })),
        &owner_headers,
    )
    .await;
    assert_eq!(created.0, StatusCode::CREATED, "{}", created.1);

    let login = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/auth/login",
        Some(json!({ "email": "agent@example.com", "password": "agent-pass-123" })),
        &[],
    )
    .await;
    assert_eq!(login.0, StatusCode::OK, "{}", login.1);
    let session = cookie_value(&login.2, "fc_admin_session").expect("agent session");
    let csrf = cookie_value(&login.2, "fc_csrf").expect("agent csrf");
    let agent_headers = vec![
        (
            "cookie",
            format!("fc_admin_session={session}; fc_csrf={csrf}"),
        ),
        ("x-csrf-token", csrf),
        ("x-store-id", fixture.store_id.clone()),
    ];

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/plugins",
        None,
        &agent_headers,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &agent_headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn failed_installs_are_audited() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let mut document = manifest("demo", "1.0.0", &[]);
    document["capabilities"]["data"] = json!(["catalog.write"]);
    let bytes = archive(&document, "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let database = fixture
        .databases
        .database(&StoreId::from_string(fixture.store_id.clone()))
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_outbox WHERE action = 'plugin.install_failed'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(count, 1);
}

fn context_token(plugin: &str, store: &str, digest: &str, scopes: &[String]) -> String {
    let claims = ContextClaims::new(
        plugin,
        store,
        digest,
        scopes.to_vec(),
        TokenActor::Admin {
            id: "user_1".to_owned(),
            permissions: vec!["plugins.read".to_owned()],
        },
        fastcart_kernel::now_timestamp(),
    );
    TokenSigner::new(b"test-secret")
        .derive(store, plugin, digest)
        .mint(&claims)
        .unwrap()
}

async fn raw(
    app: &Router,
    method: &str,
    uri: &str,
    body: Vec<u8>,
    headers: &[(&str, String)],
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn internal_api_context_kv_and_scopes() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    let digest = installed["digest"].as_str().unwrap().to_owned();
    let scopes: Vec<String> = installed["granted_scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect();
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let token = context_token("demo", &fixture.store_id, &digest, &scopes);
    let (status, body) = raw(
        &fixture.app,
        "GET",
        "/internal/plugins/v1/context",
        vec![],
        &[("authorization", format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let context: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(context["plugin"], "demo");
    assert_eq!(context["store"]["id"], fixture.store_id);
    assert!(context["scopes"].as_array().unwrap().len() >= 3);

    let token = context_token("demo", &fixture.store_id, &digest, &scopes);
    let (status, _) = raw(
        &fixture.app,
        "PUT",
        "/internal/plugins/v1/kv/chat/key",
        b"hello".to_vec(),
        &[("authorization", format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let token = context_token("demo", &fixture.store_id, &digest, &scopes);
    let (status, value) = raw(
        &fixture.app,
        "GET",
        "/internal/plugins/v1/kv/chat/key",
        vec![],
        &[("authorization", format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value, b"hello");

    let read_only = scopes
        .iter()
        .filter(|scope| scope.as_str() != "kv.write")
        .cloned()
        .collect::<Vec<_>>();
    let token = context_token("demo", &fixture.store_id, &digest, &read_only);
    let (status, _) = raw(
        &fixture.app,
        "PUT",
        "/internal/plugins/v1/kv/chat/key",
        b"nope".to_vec(),
        &[("authorization", format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let token = context_token("demo", &fixture.store_id, &digest, &scopes);
    for _ in 0..2 {
        let (status, _) = raw(
            &fixture.app,
            "GET",
            "/internal/plugins/v1/context",
            vec![],
            &[("authorization", format!("Bearer {token}"))],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "one request may make several calls");
    }

    let token = context_token("demo", &fixture.store_id, &digest, &scopes);
    let (status, _) = raw(
        &fixture.app,
        "POST",
        "/internal/plugins/v1/log",
        serde_json::to_vec(&json!({ "level": "info", "message": "hello from plugin" })).unwrap(),
        &[
            ("authorization", format!("Bearer {token}")),
            ("content-type", "application/json".to_owned()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, logs, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/plugins/demo/logs",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{logs}");
    assert_eq!(logs[0]["message"], "hello from plugin");

    let (status, body, _) = request(&fixture.app, "GET", "/health/plugins", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["status"].is_string());
}

#[tokio::test]
async fn egress_rejects_disallowed_targets() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    let digest = installed["digest"].as_str().unwrap().to_owned();
    let scopes: Vec<String> = installed["granted_scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect();
    let _ = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;

    for (url, expected) in [
        ("https://evil.example.com/x", StatusCode::FORBIDDEN),
        ("http://api.example.com/x", StatusCode::BAD_REQUEST),
        ("https://127.0.0.1/x", StatusCode::BAD_REQUEST),
    ] {
        let token = context_token("demo", &fixture.store_id, &digest, &scopes);
        let (status, body) = raw(
            &fixture.app,
            "POST",
            "/internal/plugins/v1/egress",
            serde_json::to_vec(&json!({ "url": url })).unwrap(),
            &[
                ("authorization", format!("Bearer {token}")),
                ("content-type", "application/json".to_owned()),
            ],
        )
        .await;
        assert_eq!(
            status,
            expected,
            "{url}: {}",
            String::from_utf8_lossy(&body)
        );
    }
}

#[tokio::test]
async fn apps_route_hides_disabled_and_unknown_plugins() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = raw(
        &fixture.app,
        "GET",
        "/apps/demo/anything",
        vec![],
        &[("host", HOST.to_owned())],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "{}",
        String::from_utf8_lossy(&body)
    );

    let (status, _) = raw(
        &fixture.app,
        "GET",
        "/apps/unknown/anything",
        vec![],
        &[("host", HOST.to_owned())],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

const APP_PLUGIN_JS: &str = r#"
export default {
  async fetch(request) {
    return Response.json({ ok: true, path: new URL(request.url).pathname });
  },
};
"#;

async fn start_plugin_commerce() -> SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0_u8; 8192];
                loop {
                    let Ok(read) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if let Some(position) =
                        buffer.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        let head = String::from_utf8_lossy(&buffer[..position]).to_string();
                        let length = head
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if buffer.len() < position + 4 + length {
                            continue;
                        }
                        let body = serde_json::json!({
                            "compatibilityDate": "2026-11-01",
                            "mainModule": "index.js",
                            "modules": { "index.js": APP_PLUGIN_JS },
                        })
                        .to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        return;
                    }
                }
            });
        }
    });
    addr
}

#[tokio::test]
async fn apps_forwards_to_an_enabled_plugin() {
    let binary = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../experiments/workerd/bin/workerd");
    if !binary.is_file() {
        eprintln!("skipping: pinned workerd binary is absent");
        return;
    }
    let commerce = start_plugin_commerce().await;
    let fixture = fixture_options(true, Some(commerce)).await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), APP_PLUGIN_JS);
    let (status, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = raw(
        &fixture.app,
        "GET",
        "/apps/demo/hello?x=1",
        vec![],
        &[("host", HOST.to_owned())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["path"], "/hello");
    assert!(value["store"].is_null());
}

#[tokio::test]
async fn repeated_call_ids_are_rejected() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (_, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    let digest = installed["digest"].as_str().unwrap().to_owned();
    let scopes: Vec<String> = installed["granted_scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect();
    let _ = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    let token = context_token("demo", &fixture.store_id, &digest, &scopes);

    for (call_id, expected) in [
        ("call-1", StatusCode::OK),
        ("call-1", StatusCode::UNAUTHORIZED),
        ("call-2", StatusCode::OK),
    ] {
        let (status, _) = raw(
            &fixture.app,
            "GET",
            "/internal/plugins/v1/context",
            vec![],
            &[
                ("authorization", format!("Bearer {token}")),
                ("x-fastcart-call-id", call_id.to_owned()),
            ],
        )
        .await;
        assert_eq!(status, expected, "call id {call_id}");
    }
}

#[tokio::test]
async fn admin_extensions_expose_signed_ui_entries() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/plugins/extensions",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let extensions = body["extensions"].as_array().unwrap();
    assert_eq!(extensions.len(), 2);
    let nav = extensions
        .iter()
        .find(|entry| entry["target"] == "admin.nav.section")
        .unwrap();
    assert_eq!(nav["route"], "/apps/demo");
    let tab = extensions
        .iter()
        .find(|entry| entry["target"] == "admin.page.settings.tabs")
        .unwrap();
    let src = tab["src"].as_str().unwrap();
    assert!(src.starts_with("/_fastcart/plugin-ui/"));
    assert!(src.ends_with(".html?t=") || src.contains(".html?t="));

    let (status, html) = raw(&fixture.app, "GET", src, vec![], &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&html));
    assert!(String::from_utf8_lossy(&html).contains("settings"));

    let bare = src.split("?t=").next().unwrap();
    let (status, _) = raw(&fixture.app, "GET", bare, vec![], &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_rpc_rejects_undeclared_methods() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/rpc",
        Some(json!({ "method": "not.declared", "params": {} })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn event_subscriptions_fan_out_and_jobs_are_manageable() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let mut document = manifest("demo", "1.0.0", &[]);
    document["capabilities"]["events"] = json!(["order.placed"]);
    document["jobs"] = json!([{ "name": "sync", "schedule": "* * * * *" }]);
    let bytes = archive(&document, "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let database = fixture
        .databases
        .database(&StoreId::from_string(fixture.store_id.clone()))
        .await
        .unwrap();
    let subscriptions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM plugin_event_subscriptions WHERE event_type = 'order.placed'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(subscriptions, 1);

    let now = fastcart_kernel::now_timestamp();
    sqlx::query(
        r#"INSERT INTO outbox (id, event_type, version, payload, created_at)
           VALUES ('evt_1', 'order.placed', 1, '{"order_id":"o_1"}', ?)"#,
    )
    .bind(now)
    .execute(database.pool())
    .await
    .unwrap();
    let fanned = fastcart_plugin::PluginDispatch::fan_out(&database, 100)
        .await
        .unwrap();
    assert_eq!(fanned, 1);
    let deliveries: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM plugin_deliveries WHERE event_id = 'evt_1'")
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(deliveries, 1);

    let enqueued = fastcart_plugin::PluginDispatch::enqueue_due_jobs(&database, now)
        .await
        .unwrap();
    assert_eq!(enqueued, 1);

    let (status, list, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/plugins/demo/deliveries",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let delivery_id = list[0]["id"].as_str().unwrap().to_owned();
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/plugins/demo/deliveries/{delivery_id}/replay"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, jobs, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/plugins/demo/jobs",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(jobs.as_array().unwrap().len(), 1);
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/jobs/sync/run",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
}

const WS_PLUGIN_JS: &str = r#"
export default {
  async fetch(request) {
    if (request.headers.get("upgrade") !== "websocket") {
      return new Response("expected upgrade", { status: 400 });
    }
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    server.accept();
    server.send("welcome");
    server.addEventListener("message", (event) => server.send(`echo:${event.data}`));
    return new Response(null, { status: 101, webSocket: client });
  },
};
"#;

async fn start_ws_commerce() -> SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0_u8; 8192];
                loop {
                    let Ok(read) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if let Some(position) =
                        buffer.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        let head = String::from_utf8_lossy(&buffer[..position]).to_string();
                        let length = head
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if buffer.len() < position + 4 + length {
                            continue;
                        }
                        let body = serde_json::json!({
                            "compatibilityDate": "2026-11-01",
                            "mainModule": "index.js",
                            "modules": { "index.js": WS_PLUGIN_JS },
                        })
                        .to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        return;
                    }
                }
            });
        }
    });
    addr
}

async fn websocket_handshake(stream: &mut tokio::net::TcpStream, path: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: shop.example.com\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = stream.read(&mut byte).await.unwrap();
        assert_eq!(read, 1);
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
}

async fn read_ws_text(stream: &mut tokio::net::TcpStream) -> String {
    use tokio::io::AsyncReadExt;
    let mut header = [0_u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    let mut length = usize::from(header[1] & 0x7f);
    if length == 126 {
        let mut extended = [0_u8; 2];
        stream.read_exact(&mut extended).await.unwrap();
        length = usize::from(u16::from_be_bytes(extended));
    }
    assert_eq!(header[1] & 0x80, 0, "server frames are unmasked");
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).await.unwrap();
    String::from_utf8(payload).unwrap()
}

async fn write_ws_text(stream: &mut tokio::net::TcpStream, text: &str) {
    use tokio::io::AsyncWriteExt;
    let payload = text.as_bytes();
    let mask = [1_u8, 2, 3, 4];
    let mut frame = vec![0x81, 0x80 | u8::try_from(payload.len()).unwrap()];
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream.write_all(&frame).await.unwrap();
}

#[tokio::test]
async fn websocket_upgrade_is_tunnelled_to_the_plugin() {
    let binary = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../experiments/workerd/bin/workerd");
    if !binary.is_file() {
        eprintln!("skipping: pinned workerd binary is absent");
        return;
    }
    let commerce = start_ws_commerce().await;
    let fixture = fixture_options(true, Some(commerce)).await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), WS_PLUGIN_JS);
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = fixture.app.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        websocket_handshake(&mut stream, "/apps/demo/ws"),
    )
    .await
    .expect("handshake timeout");
    let welcome = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        read_ws_text(&mut stream),
    )
    .await
    .expect("welcome timeout");
    assert_eq!(welcome, "welcome");

    write_ws_text(&mut stream, "hello").await;
    let echo = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        read_ws_text(&mut stream),
    )
    .await
    .expect("echo timeout");
    assert_eq!(echo, "echo:hello");
}
