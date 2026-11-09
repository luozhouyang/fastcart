use std::path::PathBuf;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use bytes::Bytes;
use fastcart_ssr_workerd::{SsrBody, SsrRequestContext, WorkerdConfig, WorkerdProcess};
use futures::StreamExt;

fn workerd_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("FASTCART_WORKERD_BIN").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../experiments/workerd/bin/workerd");
    path.is_file().then_some(path)
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

async fn collect_body(body: SsrBody) -> Vec<u8> {
    match body {
        SsrBody::Full(body) => body.to_vec(),
        SsrBody::Stream(mut stream) => {
            let mut output = Vec::new();
            while let Some(chunk) = stream.next().await {
                output.extend_from_slice(&chunk.expect("response chunk"));
            }
            output
        }
    }
}

#[tokio::test]
async fn starts_proxies_and_stops_workerd() {
    let Some(binary) = workerd_binary() else {
        eprintln!("skipping: set FASTCART_WORKERD_BIN or install experiments/workerd/bin/workerd");
        return;
    };
    let port = free_port();
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        directory.path().join("worker.js"),
        r#"
export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/__fastcart/health") {
      return Response.json({ status: "ok" });
    }
    if (url.pathname === "/created") {
      return new Response("created", {
        status: 201,
        headers: { "x-ssr-test": "yes" },
      });
    }
    if (url.pathname === "/cookies") {
      const headers = new Headers();
      headers.append("set-cookie", "a=1; Path=/");
      headers.append("set-cookie", "b=2; Path=/");
      return new Response("cookies", { headers });
    }
    return Response.json({
      method: request.method,
      path: url.pathname,
      query: url.search,
      header: request.headers.get("x-test"),
      connection: request.headers.get("connection"),
      store: request.headers.get("x-fastcart-store-id"),
      deployment: request.headers.get("x-fastcart-deployment-id"),
      requestId: request.headers.get("x-fastcart-request-id"),
      forwardedHost: request.headers.get("x-forwarded-host"),
      forwardedProto: request.headers.get("x-forwarded-proto"),
      internalToken: request.headers.get("x-fastcart-internal-token"),
      body: await request.text(),
    });
  },
};
"#,
    )
    .expect("write worker");
    std::fs::write(
        directory.path().join("config.capnp"),
        format!(
            r#"using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
  services = [(name = "main", worker = .mainWorker)],
  sockets = [(name = "http", address = "127.0.0.1:{port}", http = (), service = "main")],
);
const mainWorker :Workerd.Worker = (
  modules = [(name = "worker.js", esModule = embed "worker.js")],
  compatibilityDate = "2026-11-01",
);
"#
        ),
    )
    .expect("write config");

    let mut config = WorkerdConfig::new(
        binary,
        directory.path().join("config.capnp"),
        format!("127.0.0.1:{port}").parse().expect("listen addr"),
    );
    config.health_path = "/__fastcart/health".to_owned();
    config.startup_timeout = Duration::from_secs(10);
    config.shutdown_timeout = Duration::from_secs(5);

    let mut process = WorkerdProcess::start(config).await.expect("start workerd");
    assert!(process.is_running(), "workerd should be running");

    let gateway = process.gateway().expect("gateway");
    let request = Request::builder()
        .method(Method::POST)
        .uri("/echo?x=1")
        .header("x-test", "yes")
        .header(header::CONNECTION, "keep-alive")
        .body(Bytes::from_static(b"hello"))
        .expect("request");
    let response = gateway.forward(request).await.expect("proxy request");
    assert_eq!(response.status, StatusCode::OK);
    let body = collect_body(response.body).await;
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json response");
    assert_eq!(value["method"], "POST");
    assert_eq!(value["path"], "/echo");
    assert_eq!(value["query"], "?x=1");
    assert_eq!(value["header"], "yes");
    assert!(
        value["connection"].is_null(),
        "hop-by-hop header must be removed"
    );
    assert_eq!(value["body"], "hello");

    let created = gateway
        .forward(
            Request::builder()
                .uri("/created")
                .body(Bytes::new())
                .expect("request"),
        )
        .await
        .expect("created request");
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.headers.get("x-ssr-test").unwrap(), "yes");

    let cookies = gateway
        .forward(
            Request::builder()
                .uri("/cookies")
                .body(Bytes::new())
                .expect("request"),
        )
        .await
        .expect("cookies request");
    assert_eq!(
        cookies.headers.get_all(header::SET_COOKIE).iter().count(),
        2
    );

    let health = process.health().await.expect("health");
    assert!(health.status == fastcart_ssr_workerd::RuntimeStatus::Healthy);
    assert!(health.process_id.is_some());

    let context = SsrRequestContext {
        store_id: "store_trusted".to_owned(),
        deployment_id: "deploy_1".to_owned(),
        request_id: "req_1".to_owned(),
        locale: Some("en-US".to_owned()),
        currency: Some("USD".to_owned()),
        market_id: None,
        scheme: "https".to_owned(),
        preview: false,
        internal_token: None,
    };
    let streaming_gateway = gateway.clone().with_internal_token("internal-secret");
    let streaming_request = Request::builder()
        .method(Method::POST)
        .uri("/context")
        .header(header::HOST, "shop.example")
        .header("x-fastcart-store-id", "attacker")
        .header("x-forwarded-proto", "http")
        .body(Body::from("streamed-body"))
        .expect("streaming request");
    let streaming_response = streaming_gateway
        .forward_streaming(streaming_request, &context)
        .await
        .expect("streaming proxy");
    let body = collect_body(streaming_response.body).await;
    let value: serde_json::Value = serde_json::from_slice(&body).expect("context json");
    assert_eq!(value["store"], "store_trusted");
    assert_eq!(value["deployment"], "deploy_1");
    assert_eq!(value["requestId"], "req_1");
    assert_eq!(value["forwardedHost"], "shop.example");
    assert_eq!(value["forwardedProto"], "https");
    assert_eq!(value["internalToken"], "internal-secret");
    assert_eq!(value["body"], "streamed-body");

    process.stop().await.expect("stop workerd");
    assert!(!process.is_running(), "workerd should be stopped");
    process.stop().await.expect("second stop is a no-op");
}

#[tokio::test]
async fn reports_an_early_process_exit() {
    let binary = PathBuf::from("/bin/false");
    if !binary.is_file() {
        eprintln!("skipping: /bin/false is not available");
        return;
    }
    let directory = tempfile::tempdir().expect("temp dir");
    let config_path = directory.path().join("config.capnp");
    std::fs::write(&config_path, "").expect("write config");
    let config = WorkerdConfig::new(
        binary,
        config_path,
        "127.0.0.1:1".parse().expect("listen addr"),
    );
    match WorkerdProcess::start(config).await {
        Err(fastcart_ssr_workerd::SsrError::EarlyExit(_)) => {}
        Err(error) => panic!("expected early exit, got {error:?}"),
        Ok(_) => panic!("expected early exit, got a running process"),
    }
}
