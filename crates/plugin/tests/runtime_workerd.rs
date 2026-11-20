//! Real-workerd runtime checks. Skipped when the pinned binary is absent.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fastcart_plugin::runtime::{PluginHost, RequestKind, RuntimeConfig, RuntimeRequest};
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn workerd_binary() -> Option<PathBuf> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../experiments/workerd/bin/workerd");
    path.is_file().then_some(path)
}

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

const PLUGIN_JS: &str = r#"
export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/stream") {
      const encoder = new TextEncoder();
      return new Response(new ReadableStream({
        start(controller) {
          controller.enqueue(encoder.encode("chunk-1"));
          setTimeout(() => {
            controller.enqueue(encoder.encode("-chunk-2"));
            controller.close();
          }, 50);
        },
      }));
    }
    if (url.pathname === "/events" || url.pathname.startsWith("/jobs/")) {
      return Response.json({ ok: true, path: url.pathname });
    }
    if (url.pathname === "/boom") {
      while (true) {}
    }
    let direct = "blocked";
    try {
      await fetch("https://example.com");
      direct = "allowed";
    } catch (error) {
      direct = "blocked";
    }
    const echo = await env.FASTCART_API.fetch("http://api/internal/plugins/v1/echo", {
      method: "POST",
      body: "hello",
    });
    return Response.json({
      id: env.FASTCART_PLUGIN_ID,
      path: url.pathname,
      echo: await echo.text(),
      direct,
      store: request.headers.get("x-fastcart-store-id"),
      token: request.headers.get("x-fastcart-context-token"),
    });
  },
};
"#;

async fn start_commerce() -> std::net::SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let _ = handle(&mut socket).await;
            });
        }
    });
    addr
}

async fn handle(socket: &mut TcpStream) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(head_end) = find(&buffer, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
            let length = content_length(&head);
            if buffer.len() >= head_end + 4 + length {
                let body = buffer[head_end + 4..head_end + 4 + length].to_vec();
                let path = head
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_owned();
                let (status, payload) = if path.contains("/bundles/hello") {
                    let bundle = json!({
                        "compatibilityDate": "2026-11-01",
                        "mainModule": "index.js",
                        "modules": { "index.js": PLUGIN_JS },
                    });
                    (200, bundle.to_string().into_bytes())
                } else if path.starts_with("/internal/plugins/v1/echo") {
                    let mut echo = b"echo:".to_vec();
                    echo.extend_from_slice(&body);
                    (200, echo)
                } else {
                    (404, b"not found".to_vec())
                };
                let response = format!(
                    "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    payload.len()
                );
                socket.write_all(response.as_bytes()).await?;
                socket.write_all(&payload).await?;
                return Ok(());
            }
        }
    }
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn request(path: &str, deadline: Duration) -> RuntimeRequest {
    RuntimeRequest {
        plugin_id: "hello".to_owned(),
        digest: DIGEST.to_owned(),
        store_id: "store_1".to_owned(),
        path_and_query: path.to_owned(),
        method: "GET".to_owned(),
        headers: vec![],
        context_token: "ctx-token".to_owned(),
        body: vec![],
        kind: RequestKind::Admin,
        deadline_override: Some(deadline),
    }
}

async fn host(
    binary: PathBuf,
    commerce: std::net::SocketAddr,
    temp: &tempfile::TempDir,
) -> Arc<PluginHost> {
    Arc::new(
        PluginHost::new(RuntimeConfig {
            binary,
            runtime_dir: temp.path().to_path_buf(),
            commerce_addr: commerce,
            startup_timeout: Duration::from_secs(20),
            watchdog_grace: Duration::from_secs(1),
            ..RuntimeConfig::default()
        })
        .unwrap(),
    )
}

#[tokio::test]
async fn loads_a_plugin_and_blocks_direct_network() {
    let Some(binary) = workerd_binary() else {
        eprintln!("skipping: pinned workerd binary is absent");
        return;
    };
    let commerce = start_commerce().await;
    let temp = tempfile::tempdir().unwrap();
    let host = host(binary, commerce, &temp).await;

    let response = host
        .forward(request("/ping", Duration::from_secs(10)))
        .await
        .expect("forward");
    assert_eq!(response.status, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["id"], "hello");
    assert_eq!(body["path"], "/ping");
    assert_eq!(body["echo"], "echo:hello");
    assert_eq!(body["direct"], "blocked");
    assert_eq!(body["store"], "store_1");
    assert_eq!(body["token"], "ctx-token");

    host.shutdown().await;
}

#[tokio::test]
async fn times_out_and_restarts_the_host() {
    let Some(binary) = workerd_binary() else {
        eprintln!("skipping: pinned workerd binary is absent");
        return;
    };
    let commerce = start_commerce().await;
    let temp = tempfile::tempdir().unwrap();
    let host = host(binary, commerce, &temp).await;

    host.forward(request("/warm", Duration::from_secs(10)))
        .await
        .expect("warm request");

    let timeout = host
        .forward(request("/boom", Duration::from_millis(500)))
        .await;
    assert!(timeout.is_err(), "a synchronous loop must time out");

    tokio::time::sleep(Duration::from_secs(3)).await;
    let health = host.health().await;
    assert!(
        health.restarts >= 1,
        "watchdog should restart the host: {health:?}"
    );

    let recovered = host
        .forward(request("/recovered", Duration::from_secs(10)))
        .await
        .expect("request after restart");
    assert_eq!(recovered.status, 200);
    host.shutdown().await;
}

#[tokio::test]
async fn streams_plugin_responses() {
    let Some(binary) = workerd_binary() else {
        eprintln!("skipping: pinned workerd binary is absent");
        return;
    };
    let commerce = start_commerce().await;
    let temp = tempfile::tempdir().unwrap();
    let host = host(binary, commerce, &temp).await;

    let streaming = host
        .forward_stream(request("/stream", Duration::from_secs(10)))
        .await
        .expect("stream");
    assert_eq!(streaming.status, 200);
    let mut stream = streaming.stream;
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.expect("chunk"));
    }
    assert_eq!(String::from_utf8(body).unwrap(), "chunk-1-chunk-2");
    host.shutdown().await;
}

#[tokio::test]
async fn delivers_events_and_runs_jobs() {
    let Some(binary) = workerd_binary() else {
        eprintln!("skipping: pinned workerd binary is absent");
        return;
    };
    let commerce = start_commerce().await;
    let temp = tempfile::tempdir().unwrap();
    let host = host(binary, commerce, &temp).await;
    let database = fastcart_infrastructure::StoreDatabase::in_memory(
        fastcart_kernel::StoreId::from_string("store_1"),
    )
    .await
    .unwrap();
    let manifest = json!({
        "schema_version": "1",
        "id": "hello",
        "name": "Hello",
        "version": "1.0.0",
        "api_version": 2,
        "runtime": { "engine": "workerd", "compatibility_date": "2026-11-01" },
        "entry": { "type": "worker", "path": "index.js" },
        "capabilities": { "events": ["order.placed"] },
        "jobs": [{ "name": "sync", "schedule": "* * * * *" }]
    });
    let now = fastcart_kernel::now_timestamp();
    sqlx::query(
        "INSERT INTO plugin_installations \
         (id, plugin_id, digest, version, manifest_json, enabled, settings_json, \
          settings_revision, granted_scopes, installed_at, updated_at) \
         VALUES ('pli_1', 'hello', ?, '1.0.0', ?, 1, '{}', 1, '[]', ?, ?)",
    )
    .bind(DIGEST)
    .bind(manifest.to_string())
    .bind(now)
    .bind(now)
    .execute(database.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO plugin_event_subscriptions (plugin_id, event_type, enabled) \
         VALUES ('hello', 'order.placed', 1)",
    )
    .execute(database.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO outbox (id, event_type, version, payload, created_at) \
         VALUES ('evt_1', 'order.placed', 1, '{\"id\":\"o_1\"}', ?)",
    )
    .bind(now)
    .execute(database.pool())
    .await
    .unwrap();

    let dispatcher = fastcart_plugin::PluginDispatch::new(
        host.clone(),
        fastcart_plugin::TokenSigner::new(b"test"),
    );
    let fanned = fastcart_plugin::PluginDispatch::fan_out(&database, 10)
        .await
        .unwrap();
    assert_eq!(fanned, 1);
    let (delivered, retried, dead) = dispatcher.deliver(&database, 10).await.unwrap();
    assert_eq!((delivered, retried, dead), (1, 0, 0));
    let status: String =
        sqlx::query_scalar("SELECT status FROM plugin_deliveries WHERE event_id = 'evt_1'")
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(status, "done");

    fastcart_plugin::PluginDispatch::enqueue_due_jobs(&database, now)
        .await
        .unwrap();
    let (done, retried, failed) = dispatcher.run_jobs(&database, 10).await.unwrap();
    assert_eq!((done, retried, failed), (1, 0, 0));
    let job_status: String = sqlx::query_scalar("SELECT status FROM plugin_jobs LIMIT 1")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(job_status, "completed");
    host.shutdown().await;
}
