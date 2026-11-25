use super::*;

#[derive(Default)]
pub(crate) struct RecordingUniversal {
    pub(crate) seen: Arc<Mutex<Vec<RecordedContext>>>,
    pub(crate) invalidations: Arc<Mutex<Vec<String>>>,
    pub(crate) preview: bool,
    pub(crate) html: Option<&'static str>,
}

pub(crate) struct RecordedContext {
    pub(crate) store: StoreContext,
    pub(crate) capabilities: CapabilityGrants,
    pub(crate) request_id: Option<String>,
    pub(crate) scheme: String,
}

#[async_trait::async_trait]
impl UniversalStorefront for RecordingUniversal {
    async fn render(
        &self,
        _request: Request<Body>,
        store: StoreContext,
        capabilities: CapabilityGrants,
        request_id: Option<String>,
        scheme: String,
    ) -> Response {
        self.seen.lock().unwrap().push(RecordedContext {
            store,
            capabilities,
            request_id,
            scheme,
        });
        match self.html {
            Some(html) => Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from(html))
                .unwrap(),
            None => (StatusCode::OK, "<h1>universal</h1>").into_response(),
        }
    }

    async fn serve_asset(
        &self,
        store_id: &str,
        path: String,
        _headers: axum::http::HeaderMap,
    ) -> Response {
        (StatusCode::OK, format!("asset:{store_id}:{path}")).into_response()
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

    async fn render_preview(
        &self,
        _request: Request<Body>,
        _store: StoreContext,
        _capabilities: CapabilityGrants,
        _request_id: Option<String>,
        _scheme: String,
    ) -> Response {
        (StatusCode::OK, "<h1>universal-preview</h1>").into_response()
    }

    async fn has_preview(&self, _store_id: &str) -> bool {
        self.preview
    }

    async fn serve_preview_asset(
        &self,
        store_id: &str,
        path: String,
        _headers: axum::http::HeaderMap,
    ) -> Response {
        (StatusCode::OK, format!("preview-asset:{store_id}:{path}")).into_response()
    }

    async fn activate_preview(
        &self,
        _store_id: &str,
        _deployment_id: &str,
        _artifact_path: String,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn deactivate_preview(&self, _store_id: &str) -> Result<(), String> {
        Ok(())
    }
}

pub(crate) struct Fixture {
    pub(crate) app: Router,
    pub(crate) database: fastcart_infrastructure::StoreDatabase,
    pub(crate) platform: PlatformStore,
    pub(crate) preview_asset_authorizations: moka::future::Cache<String, ()>,
    pub(crate) store_id: String,
    pub(crate) _dir: tempfile::TempDir,
}

pub(crate) async fn fixture(universal: Option<Arc<dyn UniversalStorefront>>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let store = platform
        .create_store(CreateStoreInput::new("Demo", "demo", "shop.example.com"))
        .await
        .unwrap();
    StoreProvisioningService::new(platform.clone(), databases.clone())
        .provision(&store.id, None)
        .await
        .unwrap();

    let database = databases.database(&store.id).await.unwrap();
    let themes = ThemeService::new(database.clone());
    let theme = themes
        .register(RegisterThemeInput {
            manifest: ThemeManifest {
                id: "default".to_owned(),
                name: "Default".to_owned(),
                version: "1.0.0".to_owned(),
                requires: None,
            },
            settings_schema: None,
            settings: Some(serde_json::json!({"brand_color": "#111111"})),
        })
        .await
        .unwrap();
    themes.activate(&theme.id).await.unwrap();

    let fixture_platform = platform.clone();
    let context = Arc::new(
        AppContext::new(
            ApiConfig {
                cookie_secure: false,
                backup_dir: dir.path().join("backups"),
                internal_ssr_token: Some("ssr-internal-secret".to_owned()),
                internal_ssr_credentials: InternalSsrCredentials::new(vec![
                    InternalSsrToken {
                        token: "ssr-internal-secret".to_owned(),
                        store_id: Some(store.id.to_string()),
                        expires_at: None,
                        preview: false,
                    },
                    InternalSsrToken {
                        token: "other-store-secret".to_owned(),
                        store_id: Some("store_other".to_owned()),
                        expires_at: None,
                        preview: false,
                    },
                    InternalSsrToken {
                        token: "preview-store-secret".to_owned(),
                        store_id: Some(store.id.to_string()),
                        expires_at: None,
                        preview: true,
                    },
                ]),
                preview_secret: Some("preview-secret-that-is-at-least-32-bytes".to_owned()),
                ..ApiConfig::default()
            },
            platform,
            databases,
            StoreCache::default_capacity(),
        )
        .with_universal_storefront(universal),
    );
    Fixture {
        app: build_router(context.clone()),
        database: database.clone(),
        platform: fixture_platform,
        preview_asset_authorizations: context.preview_asset_authorizations.clone(),
        store_id: store.id.to_string(),
        _dir: dir,
    }
}

pub(crate) async fn send(
    app: &Router,
    uri: &str,
    host: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, String) {
    let mut builder = Request::builder().uri(uri).header("host", host);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}
