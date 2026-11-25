use super::runtime::{raw, start_plugin_commerce};
use super::support::*;
use super::*;

#[tokio::test]
async fn capability_route_requires_a_bound_enabled_plugin() {
    let commerce = start_plugin_commerce().await;
    let fixture = fixture_options(true, Some(commerce)).await;
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
    let database = fixture
        .databases
        .database(&StoreId::from_string(fixture.store_id.clone()))
        .await
        .unwrap();

    let (status, _) = raw(
        &fixture.app,
        "GET",
        "/store/capabilities/chat",
        vec![],
        &[("host", HOST.to_owned())],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unbound capability");

    sqlx::query(
        "INSERT INTO store_capability_bindings \
         (capability_id, version, plugin_id, config_json, enabled, created_at, updated_at) \
         VALUES ('chat', 1, 'demo', '{\"greeting\":\"hi\"}', 1, 1, 1)",
    )
    .execute(database.pool())
    .await
    .unwrap();

    let (status, _) = raw(
        &fixture.app,
        "GET",
        "/store/capabilities/chat",
        vec![],
        &[("host", HOST.to_owned())],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "plugin is still disabled");

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
        "/store/capabilities/chat/hello?x=1",
        vec![],
        &[("host", HOST.to_owned())],
    )
    .await;
    assert!(
        matches!(
            status,
            StatusCode::OK | StatusCode::SERVICE_UNAVAILABLE | StatusCode::BAD_GATEWAY
        ),
        "{}",
        String::from_utf8_lossy(&body)
    );
    if status == StatusCode::OK {
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["path"], "/capabilities/chat/hello");
        let token = value["token"].as_str().expect("context token");
        let claims = TokenSigner::new(b"test-secret")
            .derive(&fixture.store_id, "demo", &digest)
            .verify(token, fastcart_kernel::now_timestamp())
            .unwrap();
        assert_eq!(
            claims.actor,
            TokenActor::Storefront { id: None },
            "the capability route is a storefront request"
        );
        assert!(claims.scopes.contains(&"core.context".to_owned()));
        assert!(claims.scopes.contains(&"catalog.read".to_owned()));
        assert!(!claims.scopes.contains(&"kv.read".to_owned()));
        assert!(!claims.scopes.contains(&"secrets.read".to_owned()));
    }

    sqlx::query("UPDATE store_capability_bindings SET enabled = 0 WHERE capability_id = 'chat'")
        .execute(database.pool())
        .await
        .unwrap();
    let (status, _) = raw(
        &fixture.app,
        "GET",
        "/store/capabilities/chat",
        vec![],
        &[("host", HOST.to_owned())],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "disabled binding");
}
