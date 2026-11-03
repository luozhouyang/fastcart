//! Process observability: metrics exposition and security headers.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn fixture() -> (Router, tempfile::TempDir) {
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
    (build_router(context), dir)
}

async fn send(app: &Router, uri: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn metrics_and_security_headers_are_exposed() {
    let (app, _dir) = fixture().await;

    let health = send(&app, "/health").await;
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(health.headers()["x-content-type-options"], "nosniff");
    assert_eq!(health.headers()["x-frame-options"], "DENY");
    assert!(health.headers().get("strict-transport-security").is_none());

    let response = send(&app, "/metrics").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    let metrics = response.into_body().collect().await.unwrap().to_bytes();
    let metrics = String::from_utf8_lossy(&metrics);
    assert!(metrics.contains("fastcart_http_requests_total"));
    assert!(metrics.contains("fastcart_http_request_duration_seconds_bucket"));
    assert!(metrics.contains("fastcart_http_response_status_total{class=\"2xx\"}"));
}
