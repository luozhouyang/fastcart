use super::runtime::{raw, start_plugin_commerce};
use super::support::*;
use super::*;

#[tokio::test]
async fn capability_route_requires_a_bound_enabled_plugin() {
    let commerce = start_plugin_commerce().await;
    let fixture = fixture_options(true, Some(commerce)).await;
    let headers = fixture.headers();
    let mut document = manifest("demo", "1.0.0", &[]);
    document["provides"] = json!([{
        "kind": "storefront_capability",
        "id": "chat",
        "version": 1,
        "title": "Live chat"
    }]);
    let bytes = archive(&document, "export default {};");
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

#[tokio::test]
async fn upgrades_and_rollbacks_cannot_drop_bound_capabilities() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let mut v1 = manifest("demo", "1.0.0", &[]);
    v1["provides"] = json!([{
        "kind": "storefront_capability",
        "id": "chat",
        "version": 1,
        "title": "Live chat"
    }]);
    let bytes = archive(&v1, "export default {};\n// v1");
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
    let (status, binding, _) = request(
        &fixture.app,
        "PUT",
        "/api/v1/admin/capabilities/chat/binding",
        Some(json!({ "plugin_id": "demo", "version": 1 })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{binding}");

    let mut v2 = manifest("demo", "1.1.0", &[]);
    v2["provides"] = json!([{
        "kind": "storefront_capability",
        "id": "chat",
        "version": 2,
        "title": "Live chat"
    }]);
    let v2_bytes = archive(&v2, "export default {};\n// v2");
    let (status, body) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/demo/upgrade",
        &headers,
        &v2_bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("chat@1")),
        "{body}"
    );

    let mut v3 = manifest("demo", "1.2.0", &[]);
    v3["provides"] = json!([]);
    let v3_bytes = archive(&v3, "export default {};\n// v3");
    let (status, body) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/demo/upgrade",
        &headers,
        &v3_bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        "/api/v1/admin/capabilities/chat/binding",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, upgraded) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/demo/upgrade",
        &headers,
        &v2_bytes,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{upgraded}");

    let (status, binding, _) = request(
        &fixture.app,
        "PUT",
        "/api/v1/admin/capabilities/chat/binding",
        Some(json!({ "plugin_id": "demo", "version": 2 })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{binding}");
    let (status, rolled, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/rollback",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{rolled}");
    assert!(
        rolled["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("chat@2")),
        "{rolled}"
    );

    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        "/api/v1/admin/capabilities/chat/binding",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, rolled, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/rollback",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rolled}");
}

#[tokio::test]
async fn admin_binds_capabilities_to_enabled_providers() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let mut document = manifest("demo", "1.0.0", &[]);
    document["provides"] = json!([{
        "kind": "storefront_capability",
        "id": "chat",
        "version": 1,
        "title": "Live chat",
        "config_schema": {
            "type": "object",
            "properties": { "greeting": { "type": "string" } },
            "additionalProperties": false
        }
    }]);
    let bytes = archive(&document, "export default {};");
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

    let (status, catalog, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/capabilities",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{catalog}");
    let contributions = catalog["contributions"].as_array().unwrap();
    assert_eq!(contributions.len(), 1);
    assert_eq!(contributions[0]["id"], "chat");
    assert_eq!(contributions[0]["version"], 1);
    assert_eq!(contributions[0]["plugin_id"], "demo");
    assert!(catalog["bindings"].as_array().unwrap().is_empty());

    let (status, binding, _) = request(
        &fixture.app,
        "PUT",
        "/api/v1/admin/capabilities/chat/binding",
        Some(json!({
            "plugin_id": "demo",
            "version": 1,
            "config": { "greeting": "Hi" }
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{binding}");
    assert_eq!(binding["capability_id"], "chat");
    assert_eq!(binding["plugin_id"], "demo");
    assert_eq!(binding["config"]["greeting"], "Hi");
    assert_eq!(binding["enabled"], true);

    let (status, catalog, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/capabilities",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(catalog["bindings"].as_array().unwrap().len(), 1);

    let (status, _, _) = request(
        &fixture.app,
        "PUT",
        "/api/v1/admin/capabilities/chat/binding",
        Some(json!({
            "plugin_id": "demo",
            "version": 1,
            "config": { "unknown": true }
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _, _) = request(
        &fixture.app,
        "PUT",
        "/api/v1/admin/capabilities/chat/binding",
        Some(json!({ "plugin_id": "demo", "version": 2 })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) = request(
        &fixture.app,
        "PUT",
        "/api/v1/admin/capabilities/chat/binding",
        Some(json!({ "plugin_id": "missing", "version": 1 })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        "/api/v1/admin/capabilities/chat/binding",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, catalog, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/capabilities",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(catalog["bindings"].as_array().unwrap().is_empty());
}
