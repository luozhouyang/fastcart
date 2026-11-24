use axum::body::Body;
use axum::http::Request;
use fastcart_kernel::{CurrencyCode, StoreContext, StoreId};
use fastcart_ssr_workerd::artifact::{
    ArtifactManifest, AssetMetadata, BuildMetadata, EntryMetadata, FrameworkMetadata,
    RenderingMetadata, RuntimeMetadata,
};
use std::path::PathBuf;

use super::*;
use crate::config::SsrDeploymentConfig;
use fastcart_ssr_credentials::CredentialToken;

fn workerd_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("FASTCART_WORKERD_BIN").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../experiments/workerd/bin/workerd");
    path.is_file().then_some(path)
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("port");
    listener.local_addr().expect("addr").port()
}

fn manifest() -> ArtifactManifest {
    ArtifactManifest {
        schema_version: "1".to_owned(),
        artifact_id: "server_fixture".to_owned(),
        framework: FrameworkMetadata {
            name: "fixture".to_owned(),
            version: "1.0.0".to_owned(),
        },
        runtime: RuntimeMetadata {
            engine: "workerd".to_owned(),
            compatibility_date: "2026-11-01".to_owned(),
            compatibility_flags: vec![],
            required_capabilities: vec!["fetch".to_owned()],
        },
        entry: EntryMetadata {
            entry_type: "worker".to_owned(),
            path: "worker/index.js".to_owned(),
            export: "default".to_owned(),
        },
        assets: AssetMetadata {
            directory: "client".to_owned(),
            base_path: "/_fastcart/assets/".to_owned(),
        },
        rendering: RenderingMetadata {
            mode: "hybrid".to_owned(),
            streaming: true,
        },
        build: BuildMetadata {
            build_id: "build_server".to_owned(),
            created_at: "2026-11-01T10:00:00Z".to_owned(),
        },
    }
}

fn artifact(root: &std::path::Path) -> std::path::PathBuf {
    let source = root.join("source");
    std::fs::create_dir_all(source.join("worker")).expect("worker");
    std::fs::create_dir_all(source.join("client")).expect("client");
    std::fs::write(
        source.join("worker/index.js"),
        r#"
export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/__fastcart/health") {
      return Response.json({ status: "ok" });
    }
    if (url.pathname === "/cached") {
      globalThis.__fastcartCacheCount = (globalThis.__fastcartCacheCount || 0) + 1;
      return Response.json(
        { count: globalThis.__fastcartCacheCount },
        { headers: { "cache-control": "public, max-age=60" } },
      );
    }
    return Response.json({
      path: url.pathname,
      store: request.headers.get("x-fastcart-store-id"),
      deployment: request.headers.get("x-fastcart-deployment-id"),
      requestId: request.headers.get("x-fastcart-request-id"),
      proto: request.headers.get("x-forwarded-proto"),
      token: request.headers.get("x-fastcart-internal-token"),
    });
  },
};
"#,
    )
    .expect("worker");
    std::fs::write(source.join("client/app.js"), "console.log('asset');").expect("asset");
    std::fs::write(
        source.join("fastcart-ssr.json"),
        serde_json::to_vec_pretty(&manifest()).expect("manifest"),
    )
    .expect("manifest");
    source
}

#[tokio::test]
async fn initialize_render_and_shutdown() {
    let Some(binary) = workerd_binary() else {
        eprintln!("skipping: no workerd binary");
        return;
    };
    let root = tempfile::tempdir().expect("root");
    let source = artifact(root.path());
    let config = SsrConfig {
        enabled: true,
        binary,
        artifact_path: source,
        runtime_dir: root.path().join("runtime"),
        listen: format!("127.0.0.1:{}", free_port())
            .parse()
            .expect("listen"),
        deployment_id: "deploy_server".to_owned(),
        internal_token: Some("server-secret".to_owned()),
        request_timeout_ms: 10_000,
        startup_timeout_ms: 10_000,
        shutdown_timeout_ms: 5_000,
        restart_backoff_ms: 10,
        max_restarts: 1,
        drain_timeout_ms: 100,
        artifact_max_bytes: 10 * 1024 * 1024,
        artifact_max_files: 100,
        artifact_root: None,
        credentials_path: None,
        credentials: crate::config::SsrCredentialsConfig::default(),
        deployments: Vec::new(),
    };
    let orphan = config.runtime_dir.join("artifacts").join("orphan-digest");
    std::fs::create_dir_all(orphan.join("client")).expect("orphan artifact");
    std::fs::write(orphan.join("client/app.js"), "orphan").expect("orphan asset");
    let runtime = SsrService::initialize(
        &config,
        Arc::new(Metrics::new()),
        CredentialManager::default(),
    )
    .await
    .expect("initialize")
    .expect("enabled");
    assert!(!orphan.exists(), "orphan artifact should be swept");
    assert!(
        runtime
            .default
            .as_ref()
            .expect("default deployment")
            .artifact_dir
            .exists(),
        "active artifact must survive the sweep"
    );
    let request = Request::builder()
        .uri("/context")
        .body(Body::empty())
        .expect("request");
    let store = StoreContext {
        store_id: StoreId::from_string("store_01".to_owned()),
        domain: "shop.example".to_owned(),
        locale: "en-US".to_owned(),
        currency: CurrencyCode::new("USD").expect("currency"),
        timezone: "UTC".to_owned(),
    };
    let response = runtime
        .render(
            request,
            store,
            Some("req_server".to_owned()),
            "https".to_owned(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .unwrap(),
        "no-store"
    );
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(value["store"], "store_01");
    assert_eq!(value["deployment"], "deploy_server");
    assert_eq!(value["requestId"], "req_server");
    assert_eq!(value["proto"], "https");
    assert_eq!(value["token"], "server-secret");
    runtime
        .credentials
        .rotate(
            "store_01",
            "rotated-secret-token".to_owned(),
            None,
            300,
            false,
        )
        .await
        .expect("rotate credential");
    let rotated = runtime
        .render(
            Request::builder()
                .uri("/context")
                .body(Body::empty())
                .expect("request"),
            StoreContext {
                store_id: StoreId::from_string("store_01".to_owned()),
                domain: "shop.example".to_owned(),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").expect("currency"),
                timezone: "UTC".to_owned(),
            },
            Some("rotated".to_owned()),
            "https".to_owned(),
        )
        .await;
    let rotated_body = axum::body::to_bytes(rotated.into_body(), 1024)
        .await
        .expect("rotated body");
    let rotated_value: serde_json::Value =
        serde_json::from_slice(&rotated_body).expect("rotated json");
    assert_eq!(rotated_value["token"], "rotated-secret-token");

    let first = runtime
        .render(
            Request::builder()
                .uri("/cached")
                .body(Body::empty())
                .expect("request"),
            StoreContext {
                store_id: StoreId::from_string("store_01".to_owned()),
                domain: "shop.example".to_owned(),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").expect("currency"),
                timezone: "UTC".to_owned(),
            },
            Some("cache_1".to_owned()),
            "https".to_owned(),
        )
        .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = axum::body::to_bytes(first.into_body(), 1024)
        .await
        .expect("first body");
    let first_value: serde_json::Value = serde_json::from_slice(&first_body).expect("first json");
    let second = runtime
        .render(
            Request::builder()
                .uri("/cached")
                .body(Body::empty())
                .expect("request"),
            StoreContext {
                store_id: StoreId::from_string("store_01".to_owned()),
                domain: "shop.example".to_owned(),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").expect("currency"),
                timezone: "UTC".to_owned(),
            },
            Some("cache_2".to_owned()),
            "https".to_owned(),
        )
        .await;
    assert_eq!(
        second.headers().get("x-fastcart-cache").expect("cache hit"),
        "HIT"
    );
    let second_body = axum::body::to_bytes(second.into_body(), 1024)
        .await
        .expect("second body");
    let second_value: serde_json::Value =
        serde_json::from_slice(&second_body).expect("second json");
    assert_eq!(first_value["count"], second_value["count"]);

    runtime.invalidate_cache("store_01").await;
    let third = runtime
        .render(
            Request::builder()
                .uri("/cached")
                .body(Body::empty())
                .expect("request"),
            StoreContext {
                store_id: StoreId::from_string("store_01".to_owned()),
                domain: "shop.example".to_owned(),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").expect("currency"),
                timezone: "UTC".to_owned(),
            },
            Some("cache_3".to_owned()),
            "https".to_owned(),
        )
        .await;
    let third_body = axum::body::to_bytes(third.into_body(), 1024)
        .await
        .expect("third body");
    let third_value: serde_json::Value = serde_json::from_slice(&third_body).expect("third json");
    assert_ne!(first_value["count"], third_value["count"]);

    let asset = runtime
        .serve_asset(
            "store_01",
            "app.js".to_owned(),
            axum::http::HeaderMap::new(),
        )
        .await;
    assert_eq!(asset.status(), StatusCode::OK);
    assert_eq!(
        asset
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .unwrap(),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        asset
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .unwrap(),
        "public, max-age=31536000, immutable"
    );
    let etag = asset
        .headers()
        .get(axum::http::header::ETAG)
        .expect("etag")
        .to_str()
        .expect("etag text")
        .to_owned();
    let asset_body = axum::body::to_bytes(asset.into_body(), 1024)
        .await
        .expect("asset body");
    assert_eq!(&asset_body[..], b"console.log('asset');");
    let mut conditional = axum::http::HeaderMap::new();
    conditional.insert(
        axum::http::header::IF_NONE_MATCH,
        axum::http::HeaderValue::from_str(&etag).expect("if-none-match"),
    );
    let not_modified = runtime
        .serve_asset("store_01", "app.js".to_owned(), conditional)
        .await;
    assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);

    let mut range = axum::http::HeaderMap::new();
    range.insert(
        axum::http::header::RANGE,
        axum::http::HeaderValue::from_static("bytes=0-6"),
    );
    let partial = runtime
        .serve_asset("store_01", "app.js".to_owned(), range)
        .await;
    assert_eq!(partial.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        partial
            .headers()
            .get(axum::http::header::CONTENT_RANGE)
            .unwrap(),
        "bytes 0-6/21"
    );
    let partial_body = axum::body::to_bytes(partial.into_body(), 1024)
        .await
        .expect("partial body");
    assert_eq!(&partial_body[..], b"console");

    let traversal = runtime
        .serve_asset(
            "store_01",
            "../fastcart-ssr.json".to_owned(),
            axum::http::HeaderMap::new(),
        )
        .await;
    assert_eq!(traversal.status(), StatusCode::NOT_FOUND);
    let missing = runtime
        .serve_asset(
            "store_01",
            "missing.js".to_owned(),
            axum::http::HeaderMap::new(),
        )
        .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    runtime.shutdown().await;
}

async fn render_store(runtime: &SsrService, store_id: &str) -> serde_json::Value {
    let response = runtime
        .render(
            Request::builder()
                .uri("/context")
                .body(Body::empty())
                .expect("request"),
            StoreContext {
                store_id: StoreId::from_string(store_id.to_owned()),
                domain: format!("{store_id}.example"),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").expect("currency"),
                timezone: "UTC".to_owned(),
            },
            Some(format!("req_{store_id}")),
            "https".to_owned(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&body).expect("json")
}

async fn render_preview_store(runtime: &SsrService, store_id: &str) -> serde_json::Value {
    let response = runtime
        .render_preview(
            Request::builder()
                .uri("/context")
                .body(Body::empty())
                .expect("request"),
            StoreContext {
                store_id: StoreId::from_string(store_id.to_owned()),
                domain: format!("{store_id}.example"),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").expect("currency"),
                timezone: "UTC".to_owned(),
            },
            Some(format!("preview_{store_id}")),
            "https".to_owned(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&body).expect("json")
}

#[tokio::test]
async fn routes_stores_to_isolated_deployments_and_assets() {
    let Some(binary) = workerd_binary() else {
        eprintln!("skipping: no workerd binary");
        return;
    };
    let root = tempfile::tempdir().expect("root");
    let source_a = artifact(&root.path().join("a"));
    let source_b = artifact(&root.path().join("b"));
    let source_d = artifact(&root.path().join("d"));
    std::fs::write(source_a.join("client/app.js"), "asset-a").expect("asset a");
    std::fs::write(source_a.join("client/old.js"), "old-a").expect("old asset a");
    std::fs::write(source_b.join("client/app.js"), "asset-b").expect("asset b");
    std::fs::write(source_d.join("client/app.js"), "asset-preview").expect("preview asset");
    let config = SsrConfig {
        enabled: true,
        binary,
        artifact_path: std::path::PathBuf::new(),
        runtime_dir: root.path().join("runtime"),
        listen: format!("127.0.0.1:{}", free_port())
            .parse()
            .expect("listen"),
        deployment_id: "unused".to_owned(),
        internal_token: Some("server-secret".to_owned()),
        request_timeout_ms: 10_000,
        startup_timeout_ms: 10_000,
        shutdown_timeout_ms: 5_000,
        restart_backoff_ms: 10,
        max_restarts: 1,
        drain_timeout_ms: 100,
        artifact_max_bytes: 10 * 1024 * 1024,
        artifact_max_files: 100,
        artifact_root: Some(root.path().to_path_buf()),
        credentials_path: None,
        credentials: crate::config::SsrCredentialsConfig::default(),
        deployments: vec![
            SsrDeploymentConfig {
                store_id: "store_a".to_owned(),
                deployment_id: "deploy_a".to_owned(),
                artifact_path: source_a.clone(),
                listen: Some(
                    format!("127.0.0.1:{}", free_port())
                        .parse()
                        .expect("listen a"),
                ),
                preview: false,
                internal_token: None,
            },
            SsrDeploymentConfig {
                store_id: "store_b".to_owned(),
                deployment_id: "deploy_b".to_owned(),
                artifact_path: source_b.clone(),
                listen: Some(
                    format!("127.0.0.1:{}", free_port())
                        .parse()
                        .expect("listen b"),
                ),
                preview: false,
                internal_token: None,
            },
        ],
    };
    let credentials = CredentialManager::new(vec![
        CredentialToken {
            token: "production-token".to_owned(),
            store_id: Some("store_a".to_owned()),
            expires_at: None,
            preview: false,
        },
        CredentialToken {
            token: "preview-token".to_owned(),
            store_id: Some("store_a".to_owned()),
            expires_at: None,
            preview: true,
        },
    ]);
    let runtime = SsrService::initialize(&config, Arc::new(Metrics::new()), credentials)
        .await
        .expect("initialize")
        .expect("enabled");

    for (store_id, deployment_id) in [("store_a", "deploy_a"), ("store_b", "deploy_b")] {
        let request = Request::builder()
            .uri("/context")
            .body(Body::empty())
            .expect("request");
        let store = StoreContext {
            store_id: StoreId::from_string(store_id.to_owned()),
            domain: format!("{store_id}.example"),
            locale: "en-US".to_owned(),
            currency: CurrencyCode::new("USD").expect("currency"),
            timezone: "UTC".to_owned(),
        };
        let response = runtime
            .render(
                request,
                store,
                Some(format!("req_{store_id}")),
                "https".to_owned(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(value["store"], store_id);
        assert_eq!(value["deployment"], deployment_id);
    }

    let missing = runtime
        .render(
            Request::builder()
                .uri("/context")
                .body(Body::empty())
                .expect("request"),
            StoreContext {
                store_id: StoreId::from_string("store_missing".to_owned()),
                domain: "missing.example".to_owned(),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").expect("currency"),
                timezone: "UTC".to_owned(),
            },
            None,
            "https".to_owned(),
        )
        .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    for (store_id, expected) in [("store_a", "asset-a"), ("store_b", "asset-b")] {
        let asset = runtime
            .serve_asset(store_id, "app.js".to_owned(), axum::http::HeaderMap::new())
            .await;
        assert_eq!(asset.status(), StatusCode::OK);
        let body = axum::body::to_bytes(asset.into_body(), 1024)
            .await
            .expect("asset");
        assert_eq!(&body[..], expected.as_bytes());
    }

    let source_c = artifact(&root.path().join("c"));
    std::fs::write(source_c.join("client/app.js"), "asset-c").expect("asset c");
    runtime
        .activate("store_a", "deploy_a2", &source_c)
        .await
        .expect("activate");
    let activated = render_store(&runtime, "store_a").await;
    assert_eq!(activated["deployment"], "deploy_a2");
    let asset = runtime
        .serve_asset("store_a", "app.js".to_owned(), axum::http::HeaderMap::new())
        .await;
    let body = axum::body::to_bytes(asset.into_body(), 1024)
        .await
        .expect("asset");
    assert_eq!(&body[..], b"asset-c");

    assert!(
        runtime
            .activate("store_a", "deploy_bad", &root.path().join("missing"))
            .await
            .is_err()
    );
    let still_active = render_store(&runtime, "store_a").await;
    assert_eq!(still_active["deployment"], "deploy_a2");

    runtime.rollback("store_a").await.expect("rollback");
    let rolled_back = render_store(&runtime, "store_a").await;
    assert_eq!(rolled_back["deployment"], "deploy_a");

    runtime
        .activate_preview("store_a", "deploy_preview", &source_d)
        .await
        .expect("activate preview");
    assert!(runtime.has_preview("store_a").await);
    let preview = render_preview_store(&runtime, "store_a").await;
    assert_eq!(preview["deployment"], "deploy_preview");
    assert_eq!(preview["token"], "preview-token");
    let production = render_store(&runtime, "store_a").await;
    assert_eq!(production["deployment"], "deploy_a");
    assert_eq!(production["token"], "production-token");
    let preview_asset = runtime
        .serve_preview_asset("store_a", "app.js".to_owned(), axum::http::HeaderMap::new())
        .await;
    let preview_asset_body = axum::body::to_bytes(preview_asset.into_body(), 1024)
        .await
        .expect("preview asset");
    assert_eq!(&preview_asset_body[..], b"asset-preview");
    let production_asset = runtime
        .serve_asset("store_a", "app.js".to_owned(), axum::http::HeaderMap::new())
        .await;
    let production_asset_body = axum::body::to_bytes(production_asset.into_body(), 1024)
        .await
        .expect("production asset");
    assert_eq!(&production_asset_body[..], b"asset-a");
    let preview_artifact_dir = runtime
        .previews
        .read()
        .await
        .get("store_a")
        .expect("preview deployment")
        .artifact_dir
        .clone();
    runtime
        .deactivate_preview("store_a")
        .await
        .expect("deactivate preview");
    assert!(!runtime.has_preview("store_a").await);
    let missing_preview_asset = runtime
        .serve_preview_asset("store_a", "app.js".to_owned(), axum::http::HeaderMap::new())
        .await;
    assert_eq!(missing_preview_asset.status(), StatusCode::NOT_FOUND);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!preview_artifact_dir.exists());
    let missing_preview = runtime
        .render_preview(
            Request::builder()
                .uri("/context")
                .body(Body::empty())
                .expect("request"),
            StoreContext {
                store_id: StoreId::from_string("store_a".to_owned()),
                domain: "store_a.example".to_owned(),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").expect("currency"),
                timezone: "UTC".to_owned(),
            },
            None,
            "https".to_owned(),
        )
        .await;
    assert_eq!(missing_preview.status(), StatusCode::NOT_FOUND);
    runtime.shutdown().await;
}

#[tokio::test]
async fn activation_state_survives_restart() {
    let Some(binary) = workerd_binary() else {
        eprintln!("skipping: no workerd binary");
        return;
    };
    let root = tempfile::tempdir().expect("root");
    let source_a = artifact(&root.path().join("a"));
    let source_b = artifact(&root.path().join("b"));
    let runtime_dir = root.path().join("runtime");
    let base = SsrConfig {
        enabled: true,
        binary,
        artifact_path: source_a,
        runtime_dir: runtime_dir.clone(),
        listen: format!("127.0.0.1:{}", free_port())
            .parse()
            .expect("listen"),
        deployment_id: "default".to_owned(),
        internal_token: Some("server-secret".to_owned()),
        request_timeout_ms: 10_000,
        startup_timeout_ms: 10_000,
        shutdown_timeout_ms: 5_000,
        restart_backoff_ms: 10,
        max_restarts: 1,
        drain_timeout_ms: 100,
        artifact_max_bytes: 10 * 1024 * 1024,
        artifact_max_files: 100,
        artifact_root: Some(root.path().to_path_buf()),
        credentials_path: None,
        credentials: crate::config::SsrCredentialsConfig::default(),
        deployments: Vec::new(),
    };
    let runtime = SsrService::initialize(
        &base,
        Arc::new(Metrics::new()),
        CredentialManager::default(),
    )
    .await
    .expect("initialize")
    .expect("enabled");
    runtime
        .activate("store_persist", "deploy_persisted", &source_b)
        .await
        .expect("activate");
    runtime.shutdown().await;

    let restarted = SsrService::initialize(
        &base,
        Arc::new(Metrics::new()),
        CredentialManager::default(),
    )
    .await
    .expect("restart")
    .expect("enabled");
    let value = render_store(&restarted, "store_persist").await;
    assert_eq!(value["deployment"], "deploy_persisted");
    let missing = restarted
        .render(
            Request::builder()
                .uri("/context")
                .body(Body::empty())
                .expect("request"),
            StoreContext {
                store_id: StoreId::from_string("store_other".to_owned()),
                domain: "other.example".to_owned(),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").expect("currency"),
                timezone: "UTC".to_owned(),
            },
            None,
            "https".to_owned(),
        )
        .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    restarted.shutdown().await;
}
