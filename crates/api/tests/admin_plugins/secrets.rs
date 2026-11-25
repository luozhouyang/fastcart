use serde_json::json;

use super::runtime::{context_token, raw};
use super::support::*;
use super::*;

async fn install_enabled(fixture: &Fixture, document: &Value) -> (String, Vec<String>) {
    let headers = fixture.headers();
    let bytes = archive(document, "export default {};");
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
    (digest, scopes)
}

#[tokio::test]
async fn secrets_are_write_only_and_scoped() {
    let fixture = fixture().await;
    let mut document = manifest("demo", "1.0.0", &[]);
    document["capabilities"]["secrets"] = json!(["secrets.read"]);
    let (digest, scopes) = install_enabled(&fixture, &document).await;
    assert!(scopes.iter().any(|scope| scope == "secrets.read"));

    let secret = "test-api-key-value";
    let (status, _, _) = request(
        &fixture.app,
        "PUT",
        "/api/v1/admin/plugins/demo/secrets/api_key",
        Some(json!({ "value": secret })),
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, list, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/plugins/demo/secrets",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["name"], "api_key");
    assert!(!list.to_string().contains(secret), "values are write-only");

    let token = context_token("demo", &fixture.store_id, &digest, &scopes);
    let (status, body) = raw(
        &fixture.app,
        "GET",
        "/internal/plugins/v1/secrets/api_key",
        vec![],
        &[("authorization", format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body, secret.as_bytes());

    let without_scope: Vec<String> = scopes
        .iter()
        .filter(|scope| scope.as_str() != "secrets.read")
        .cloned()
        .collect();
    let token = context_token("demo", &fixture.store_id, &digest, &without_scope);
    let (status, _) = raw(
        &fixture.app,
        "GET",
        "/internal/plugins/v1/secrets/api_key",
        vec![],
        &[("authorization", format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        "/api/v1/admin/plugins/demo/secrets/api_key",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let token = context_token("demo", &fixture.store_id, &digest, &scopes);
    let (status, _) = raw(
        &fixture.app,
        "GET",
        "/internal/plugins/v1/secrets/api_key",
        vec![],
        &[("authorization", format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn removing_a_plugin_purges_secrets_kv_and_logs() {
    let fixture = fixture().await;
    let document = manifest("demo", "1.0.0", &[]);
    let (digest, _) = install_enabled(&fixture, &document).await;
    let _ = digest;

    let (status, _, _) = request(
        &fixture.app,
        "PUT",
        "/api/v1/admin/plugins/demo/secrets/api_key",
        Some(json!({ "value": "test-api-key-value" })),
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let database = fixture
        .databases
        .database(&StoreId::from_string(fixture.store_id.clone()))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO plugin_kv (plugin_id, namespace, key, value, expires_at, updated_at) \
         VALUES ('demo', 'ns', 'key', X'01', NULL, 1)",
    )
    .execute(database.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO plugin_logs (plugin_id, level, message, fields_json, created_at) \
         VALUES ('demo', 'info', 'hello', NULL, 1)",
    )
    .execute(database.pool())
    .await
    .unwrap();

    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/disable",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        "/api/v1/admin/plugins/demo",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    for table in ["plugin_secrets", "plugin_kv", "plugin_logs"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(count, 0, "{table} must be purged");
    }
}
