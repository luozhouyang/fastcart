use std::path::PathBuf;
use std::time::{Duration, Instant};

use axum::http::Request;
use bytes::Bytes;
use fastcart_ssr::artifact::{
    Artifact, ArtifactLimits, ArtifactManifest, ArtifactStore, AssetMetadata, BuildMetadata,
    EntryMetadata, FrameworkMetadata, RenderingMetadata, RuntimeMetadata,
};
use fastcart_ssr::{RuntimeGroupId, RuntimeManager, RuntimeManagerConfig, RuntimeStatus};

fn workerd_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("FASTCART_WORKERD_BIN").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../experiments/workerd/bin/workerd");
    path.is_file().then_some(path)
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    listener.local_addr().expect("address").port()
}

fn manifest() -> ArtifactManifest {
    ArtifactManifest {
        schema_version: "1".to_owned(),
        artifact_id: "manager_fixture".to_owned(),
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
            build_id: "build_manager".to_owned(),
            created_at: "2026-11-01T10:00:00Z".to_owned(),
        },
    }
}

fn fixture(store_root: &std::path::Path) -> Artifact {
    let source = tempfile::tempdir().expect("source");
    std::fs::create_dir_all(source.path().join("worker")).expect("worker dir");
    std::fs::create_dir_all(source.path().join("client/assets")).expect("client dir");
    std::fs::write(
        source.path().join("worker/index.js"),
        r#"
export default {
  async fetch(request) {
    if (new URL(request.url).pathname === "/__fastcart/health") {
      return Response.json({ status: "ok" });
    }
    return new Response("manager-ok");
  },
};
"#,
    )
    .expect("worker");
    std::fs::write(source.path().join("client/assets/app.js"), "console.log(1)").expect("asset");
    std::fs::write(
        source.path().join("fastcart-ssr.json"),
        serde_json::to_vec_pretty(&manifest()).expect("manifest json"),
    )
    .expect("manifest");
    let store = ArtifactStore::new(store_root.join("artifacts"));
    store
        .import(source.path(), ArtifactLimits::default())
        .expect("artifact import")
}

#[tokio::test]
async fn manages_start_restart_health_and_stop() {
    let Some(binary) = workerd_binary() else {
        eprintln!("skipping: set FASTCART_WORKERD_BIN or install experiments/workerd/bin/workerd");
        return;
    };
    let root = tempfile::tempdir().expect("runtime root");
    let artifact = fixture(root.path());
    let port = free_port();
    let mut config = RuntimeManagerConfig::new(root.path(), binary);
    config.restart_backoff = Duration::from_millis(40);
    config.max_restarts = 1;
    let mut manager = RuntimeManager::new(config).expect("manager");
    let group = RuntimeGroupId::new("default").expect("group");

    let health = manager
        .start_group(
            group.clone(),
            artifact,
            format!("127.0.0.1:{port}").parse().expect("listen"),
        )
        .await
        .expect("start group");
    assert_eq!(health.status, RuntimeStatus::Healthy);
    assert!(health.process_id.is_some());

    let gateway = manager.gateway(&group).expect("gateway");
    let response = gateway
        .forward(
            Request::builder()
                .uri("/")
                .body(Bytes::new())
                .expect("request"),
        )
        .await
        .expect("proxy");
    assert_eq!(response.status, axum::http::StatusCode::OK);

    let config_path = root.path().join("groups/default/config.capnp");
    let config_text = std::fs::read_to_string(config_path).expect("generated config");
    assert!(config_text.contains("worker/index.js"));
    assert!(config_text.contains("compatibilityDate = \"2026-11-01\""));

    let started = Instant::now();
    let health = manager.restart_group(&group).await.expect("restart");
    assert_eq!(health.status, RuntimeStatus::Healthy);
    assert!(started.elapsed() >= Duration::from_millis(30));

    assert!(matches!(
        manager.restart_group(&group).await,
        Err(fastcart_ssr::SsrError::RestartLimit(_))
    ));
    assert_eq!(manager.status(&group), Some(RuntimeStatus::Failed));

    manager.stop_group(&group).await.expect("stop");
    assert_eq!(manager.status(&group), Some(RuntimeStatus::Stopped));
    assert_eq!(
        manager.health(&group).await.expect("health").status,
        RuntimeStatus::Stopped
    );
}
