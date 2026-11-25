use super::support::*;
use super::*;

#[tokio::test]
async fn install_list_enable_disable_and_bundle() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let document = manifest("demo", "1.0.0", &[]);
    let bytes = archive(&document, "export default {};\n// v1");

    let (status, body) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["id"], "demo");
    assert_eq!(body["enabled"], false);
    let digest = body["digest"].as_str().unwrap().to_owned();
    assert!(
        fixture
            .plugins_root
            .join("artifacts")
            .join(&digest)
            .is_dir()
    );

    let (status, list, _) =
        request(&fixture.app, "GET", "/api/v1/admin/plugins", None, &headers).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["plugins"].as_array().unwrap().len(), 1);
    assert_eq!(list["artifacts"].as_array().unwrap().len(), 1);

    let (status, plugin, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{plugin}");
    assert_eq!(plugin["enabled"], true);

    let (status, bundle) = internal_bundle(&fixture, "demo", &digest, RUNTIME_TOKEN).await;
    assert_eq!(status, StatusCode::OK, "{bundle}");
    assert_eq!(bundle["mainModule"], "index.js");
    assert!(
        bundle["modules"]["index.js"]
            .as_str()
            .unwrap()
            .contains("v1")
    );

    let (status, plugin, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/disable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plugin["enabled"], false);

    let (status, _) = internal_bundle(&fixture, "demo", &digest, RUNTIME_TOKEN).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn duplicate_install_is_rejected() {
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
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn invalid_archives_are_rejected() {
    let fixture = fixture().await;
    let headers = fixture.headers();

    let mut document = manifest("demo", "1.0.0", &[]);
    document["capabilities"]["data"] = json!(["catalog.write"]);
    let bytes = archive(&document, "export default {};");
    let (status, body) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let mut document = manifest("demo", "1.0.0", &[]);
    document["extensions"]["admin"][1]["target"] = json!("admin.page.orders.details.actions");
    let bytes = archive(&document, "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upgrade_requires_scope_acceptance_and_rolls_back() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let v1 = archive(&manifest("demo", "1.0.0", &[]), "export default {};\n// v1");
    let (status, installed) =
        upload(&fixture.app, "/api/v1/admin/plugins/install", &headers, &v1).await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    let v1_digest = installed["digest"].as_str().unwrap().to_owned();

    let v2 = archive(
        &manifest("demo", "1.1.0", &["customers.read"]),
        "export default {};\n// v2",
    );
    let (status, body) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/demo/upgrade",
        &headers,
        &v2,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, upgraded) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/demo/upgrade?accept_new_scopes=true",
        &headers,
        &v2,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{upgraded}");
    assert_eq!(upgraded["version"], "1.1.0");
    assert_eq!(upgraded["previous_digest"], v1_digest.as_str());

    let (status, rolled, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/rollback",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rolled}");
    assert_eq!(rolled["version"], "1.0.0");
    assert_eq!(rolled["digest"], v1_digest.as_str());
}

#[tokio::test]
async fn settings_are_validated_and_revisioned() {
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
        "PATCH",
        "/api/v1/admin/plugins/demo/settings",
        Some(json!({ "settings": { "accent": 42 } })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, plugin, _) = request(
        &fixture.app,
        "PATCH",
        "/api/v1/admin/plugins/demo/settings",
        Some(json!({ "settings": { "accent": "#2563eb" } })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{plugin}");
    assert_eq!(plugin["settings"]["accent"], "#2563eb");
    assert_eq!(plugin["settings_revision"], 2);
}

#[tokio::test]
async fn delete_requires_a_disabled_plugin() {
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

    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        "/api/v1/admin/plugins/demo",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/disable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        "/api/v1/admin/plugins/demo",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn bundle_requires_the_runtime_token() {
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
    let (status, _) = internal_bundle(&fixture, "demo", &digest, "wrong-token").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let _ = std::io::stdout().flush();
}

#[tokio::test]
async fn install_requires_the_runtime_to_be_enabled() {
    let fixture = fixture_with(false).await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, body) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, list, _) =
        request(&fixture.app, "GET", "/api/v1/admin/plugins", None, &headers).await;
    assert_eq!(status, StatusCode::OK, "listing remains available");
    assert_eq!(list["plugins"].as_array().unwrap().len(), 0);

    let (status, _) = internal_bundle(&fixture, "demo", "deadbeef", RUNTIME_TOKEN).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn users_without_plugin_permissions_are_forbidden() {
    let fixture = fixture().await;
    let owner_headers = fixture.headers();
    let created = request(
        &fixture.app,
        "POST",
        "/api/v1/platform/users",
        Some(json!({
            "email": "agent@example.com",
            "password": "agent-pass-123",
            "store_id": fixture.store_id,
            "roles": ["support-agent"]
        })),
        &owner_headers,
    )
    .await;
    assert_eq!(created.0, StatusCode::CREATED, "{}", created.1);

    let login = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/auth/login",
        Some(json!({ "email": "agent@example.com", "password": "agent-pass-123" })),
        &[],
    )
    .await;
    assert_eq!(login.0, StatusCode::OK, "{}", login.1);
    let session = cookie_value(&login.2, "fc_admin_session").expect("agent session");
    let csrf = cookie_value(&login.2, "fc_csrf").expect("agent csrf");
    let agent_headers = vec![
        (
            "cookie",
            format!("fc_admin_session={session}; fc_csrf={csrf}"),
        ),
        ("x-csrf-token", csrf),
        ("x-store-id", fixture.store_id.clone()),
    ];

    let (status, _, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/plugins",
        None,
        &agent_headers,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let bytes = archive(&manifest("demo", "1.0.0", &[]), "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &agent_headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn failed_installs_are_audited() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let mut document = manifest("demo", "1.0.0", &[]);
    document["capabilities"]["data"] = json!(["catalog.write"]);
    let bytes = archive(&document, "export default {};");
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let database = fixture
        .databases
        .database(&StoreId::from_string(fixture.store_id.clone()))
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_outbox WHERE action = 'plugin.install_failed'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(count, 1);
}

async fn internal_bundle(
    fixture: &Fixture,
    plugin: &str,
    digest: &str,
    token: &str,
) -> (StatusCode, Value) {
    let uri = format!(
        "/internal/plugins/v1/stores/{}/bundles/{plugin}?digest={digest}",
        fixture.store_id
    );
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
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
