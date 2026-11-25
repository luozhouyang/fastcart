use super::*;

pub(crate) struct Fixture {
    pub(crate) app: Router,
    pub(crate) context: Arc<AppContext>,
    pub(crate) _dir: tempfile::TempDir,
}

pub(crate) async fn fixture() -> Fixture {
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
    Fixture {
        app: build_router(context.clone()),
        context,
        _dir: dir,
    }
}

pub(crate) async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
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
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

pub(crate) fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

pub(crate) async fn bootstrap_store(fixture: &Fixture, domain: &str) -> StoreId {
    bootstrap_store_with_admin(fixture, domain).await.0
}

/// Bootstraps a store and returns the admin cookie/CSRF pair for the owner.
pub(crate) async fn bootstrap_store_with_admin(
    fixture: &Fixture,
    domain: &str,
) -> (StoreId, String, String) {
    let setup = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/platform/auth/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "email": "owner@example.com",
                        "password": "hunter2hunter2"
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
        &fixture.app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": "Demo",
            "slug": "demo",
            "primary_domain": domain
        })),
        &[
            ("cookie", admin_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    (
        StoreId::from_string(store["id"].as_str().unwrap()),
        admin_auth,
        csrf,
    )
}
