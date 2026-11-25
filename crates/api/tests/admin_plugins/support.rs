use super::*;

pub(crate) struct Fixture {
    pub(crate) app: Router,
    pub(crate) context: Arc<AppContext>,
    pub(crate) store_id: String,
    pub(crate) session: String,
    pub(crate) csrf: String,
    pub(crate) plugins_root: std::path::PathBuf,
    pub(crate) databases: Arc<StoreDatabaseManager>,
    pub(crate) _dir: tempfile::TempDir,
}

/// Storefront runtime that records the stores whose cache it invalidated.
#[derive(Default)]
pub(crate) struct RecordingUniversal {
    pub(crate) invalidations: std::sync::Mutex<Vec<String>>,
}

impl RecordingUniversal {
    pub(crate) fn invalidated(&self) -> Vec<String> {
        self.invalidations.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl fastcart_ssr::UniversalStorefront for RecordingUniversal {
    async fn render(
        &self,
        _request: Request<Body>,
        _store: fastcart_kernel::StoreContext,
        _capabilities: fastcart_ssr::CapabilityGrants,
        _request_id: Option<String>,
        _scheme: String,
    ) -> axum::response::Response {
        (StatusCode::OK, "<h1>universal</h1>").into_response()
    }

    async fn serve_asset(
        &self,
        _store_id: &str,
        _path: String,
        _headers: axum::http::HeaderMap,
    ) -> axum::response::Response {
        StatusCode::NOT_FOUND.into_response()
    }

    async fn activate(
        &self,
        _store_id: &str,
        _deployment_id: &str,
        _artifact_path: String,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn rollback(&self, _store_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn invalidate_cache(&self, store_id: &str) {
        self.invalidations.lock().unwrap().push(store_id.to_owned());
    }
}

pub(crate) async fn fixture() -> Fixture {
    fixture_with(true).await
}

pub(crate) async fn fixture_with(plugins_enabled: bool) -> Fixture {
    fixture_options(plugins_enabled, None).await
}

pub(crate) async fn fixture_options(
    plugins_enabled: bool,
    commerce_addr: Option<SocketAddr>,
) -> Fixture {
    fixture_full(plugins_enabled, commerce_addr, None).await
}

pub(crate) async fn fixture_full(
    plugins_enabled: bool,
    commerce_addr: Option<SocketAddr>,
    universal: Option<Arc<dyn fastcart_ssr::UniversalStorefront>>,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let plugins_root = dir.path().join("plugins");
    let context = Arc::new(
        AppContext::new(
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
        )
        .with_universal_storefront(universal),
    );
    let app = build_router(context.clone());

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
        context,
        store_id,
        session,
        csrf,
        plugins_root,
        databases,
        _dir: dir,
    }
}

impl Fixture {
    pub(crate) fn headers(&self) -> Vec<(&str, String)> {
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

pub(crate) fn manifest(id: &str, version: &str, extra_data: &[&str]) -> Value {
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

pub(crate) fn archive(manifest: &Value, module: &str) -> Vec<u8> {
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

pub(crate) fn append(builder: &mut Builder<&mut GzEncoder<Vec<u8>>>, path: &str, bytes: &[u8]) {
    let mut header = Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, path, bytes).unwrap();
}

pub(crate) fn multipart(boundary: &str, filename: &str, bytes: &[u8]) -> Vec<u8> {
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

pub(crate) async fn request(
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

pub(crate) async fn upload(
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

pub(crate) fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}
