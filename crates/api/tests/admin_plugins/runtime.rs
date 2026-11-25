use super::support::*;
use super::*;

const APP_PLUGIN_JS: &str = r#"
export default {
  async fetch(request) {
    return Response.json({ ok: true, path: new URL(request.url).pathname });
  },
};
"#;

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
