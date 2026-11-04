//! End-to-end inquiry form, submission and notification channel HTTP flow.

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
        storage_root: dir.path().join("assets"),
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

fn draft_body() -> Value {
    json!({
        "fields": [
            {
                "key": "name",
                "kind": "text",
                "label": "Name",
                "required": true,
                "validation": { "max_length": 100 }
            },
            {
                "key": "email",
                "kind": "email",
                "label": "Email",
                "required": true
            },
            {
                "key": "message",
                "kind": "textarea",
                "label": "Message",
                "required": true,
                "validation": { "max_length": 5000 }
            },
            {
                "key": "company",
                "kind": "text",
                "label": "Company",
                "required": false,
                "validation": { "max_length": 200 }
            }
        ],
        "settings": {
            "submit_label": "Send",
            "success_message": "Thanks",
            "consent_required": false,
            "allow_attachments": false,
            "notification_fields": ["company"]
        }
    })
}

fn draft_attachment_body() -> Value {
    json!({
        "fields": [
            {
                "key": "name",
                "kind": "text",
                "label": "Name",
                "required": true,
                "validation": { "max_length": 100 }
            },
            {
                "key": "email",
                "kind": "email",
                "label": "Email",
                "required": true
            },
            {
                "key": "message",
                "kind": "textarea",
                "label": "Message",
                "required": true,
                "validation": { "max_length": 5000 }
            },
            {
                "key": "attachments",
                "kind": "file",
                "label": "Attachments",
                "required": false,
                "file": {
                    "max_files": 1,
                    "max_bytes": 5242880,
                    "accept": ["image/png"]
                }
            }
        ],
        "settings": {
            "submit_label": "Send",
            "success_message": "Thanks",
            "consent_required": false,
            "allow_attachments": true,
            "notification_fields": []
        }
    })
}

fn multipart_png(size: usize) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "fastcart-test-boundary";
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    file.resize(size, 0);
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"large.png\"\r\nContent-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&file);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

#[tokio::test]
async fn inquiry_form_publish_submit_and_notification_channel_flow() {
    let fixture = fixture().await;
    let app = fixture.app;

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
                        "password": "correct-horse-battery-staple"
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

    let (status, store) = request(
        &app,
        "POST",
        "/api/v1/platform/stores",
        Some(json!({
            "name": "Inquiry Store",
            "slug": "inquiry-store",
            "primary_domain": "shop.example.com"
        })),
        &[
            ("cookie", admin_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
        ],
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

    let (status, form) = request(
        &app,
        "POST",
        "/api/v1/admin/inquiry-forms",
        Some(json!({"name": "Contact", "handle": "contact"})),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{form}");
    let form_id = form["id"].as_str().unwrap().to_owned();
    assert_eq!(form["status"], "draft");

    let (status, saved) = request(
        &app,
        "PUT",
        &format!("/api/v1/admin/inquiry-forms/{form_id}/draft"),
        Some(draft_body()),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["fields"].as_array().unwrap().len(), 4);

    let (status, published) = request(
        &app,
        "POST",
        &format!("/api/v1/admin/inquiry-forms/{form_id}/publish"),
        Some(json!({})),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{published}");
    assert_eq!(published["status"], "published");
    let version = published["published_version"].as_i64().unwrap();

    let (status, public) = request(
        &app,
        "GET",
        "/api/v1/store/inquiry-forms/contact",
        None,
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["form"]["handle"], "contact");
    assert_eq!(public["form"]["version"], version);
    assert_eq!(public["form"]["fields"].as_array().unwrap().len(), 4);
    assert!(
        public["form"]["settings"]
            .get("notification_fields")
            .is_none()
    );

    let submission = json!({
        "form_handle": "contact",
        "name": "  Ada Lovelace  ",
        "email": " Ada@Example.com ",
        "message": "Please send a quote.",
        "data": { "company": "Analytical Engines" },
        "consent": false
    });
    let (status, created) = request(
        &app,
        "POST",
        "/api/v1/store/inquiries",
        Some(submission.clone()),
        &[
            ("host", "shop.example.com"),
            ("idempotency-key", "inquiry-1"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let inquiry_id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["status"], "new");

    let (status, duplicate) = request(
        &app,
        "POST",
        "/api/v1/store/inquiries",
        Some(submission),
        &[
            ("host", "shop.example.com"),
            ("idempotency-key", "inquiry-1"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{duplicate}");
    assert_eq!(duplicate["id"], inquiry_id);

    let (status, listed) =
        request(&app, "GET", "/api/v1/admin/inquiries", None, &store_headers).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["pagination"]["total"], 1);
    assert_eq!(listed["data"][0]["email"], "ada@example.com");

    let (status, detail) = request(
        &app,
        "GET",
        &format!("/api/v1/admin/inquiries/{inquiry_id}"),
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["inquiry"]["data"]["company"], "Analytical Engines");
    assert_eq!(detail["revision"]["fields"].as_array().unwrap().len(), 4);

    let (status, count) = request(
        &app,
        "GET",
        "/api/v1/admin/inquiries/unread-count",
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{count}");
    assert_eq!(count["count"], 1);

    let (status, channel) = request(
        &app,
        "POST",
        "/api/v1/admin/notification-channels",
        Some(json!({
            "kind": "feishu",
            "name": "Operations",
            "endpoint_url": "https://open.feishu.cn/open-apis/bot/v2/hook/test-token-abcd",
            "signing_secret": "signing-secret",
            "events": ["inquiry.created", "order.placed"],
            "is_active": true
        })),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    assert_eq!(channel["kind"], "feishu");
    assert_eq!(channel["signing_secret_configured"], true);
    assert!(
        channel["endpoint_url_masked"]
            .as_str()
            .unwrap()
            .contains("****")
    );
    assert!(!channel.to_string().contains("signing-secret"));
    assert!(!channel.to_string().contains("test-token-abcd"));

    let (status, channels) = request(
        &app,
        "GET",
        "/api/v1/admin/notification-channels",
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{channels}");
    assert_eq!(channels.as_array().unwrap().len(), 1);

    let (status, events) = request(
        &app,
        "GET",
        "/api/v1/admin/notification-events",
        None,
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{events}");
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["type"] == "inquiry.created")
    );
}

#[tokio::test]
async fn attachment_upload_route_uses_the_larger_body_limit() {
    let fixture = fixture().await;
    let app = fixture.app;

    let setup = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/platform/auth/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": "uploader@example.com",
                        "password": "correct-horse-battery-staple"
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

    let (status, store) = request(
        &app,
        "POST",
        "/api/v1/platform/stores",
        Some(json!({
            "name": "Upload Store",
            "slug": "upload-store",
            "primary_domain": "upload.example.com"
        })),
        &[
            ("cookie", admin_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    let store_id = store["id"].as_str().unwrap().to_owned();
    let store_headers = [
        ("cookie", admin_auth.as_str()),
        ("x-csrf-token", csrf.as_str()),
        ("x-store-id", store_id.as_str()),
    ];

    let (status, form) = request(
        &app,
        "POST",
        "/api/v1/admin/inquiry-forms",
        Some(json!({"name": "Upload", "handle": "upload"})),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{form}");
    let form_id = form["id"].as_str().unwrap().to_owned();

    let (status, saved) = request(
        &app,
        "PUT",
        &format!("/api/v1/admin/inquiry-forms/{form_id}/draft"),
        Some(draft_attachment_body()),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");

    let (status, published) = request(
        &app,
        "POST",
        &format!("/api/v1/admin/inquiry-forms/{form_id}/publish"),
        Some(json!({})),
        &store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{published}");

    let (content_type, body) = multipart_png(3 * 1024 * 1024);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/store/inquiry-files?form_handle=upload&field_key=attachments")
                .header("host", "upload.example.com")
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::CREATED, "{value}");
    assert!(value["id"].as_str().is_some_and(|id| !id.is_empty()));
    assert!(value["expires_at"].as_i64().is_some());
}
