use super::support::*;
use super::*;

#[tokio::test]
async fn admin_mutations_without_service_audit_are_recorded() {
    let fixture = fixture().await;
    let (store_id, admin_auth, csrf) =
        bootstrap_store_with_admin(&fixture, "audit.example.com").await;
    let store_id_header = store_id.to_string();

    let (status, body) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/tax/categories",
        Some(serde_json::json!({
            "name": "Audit category",
            "code": "audit_category"
        })),
        &[
            ("cookie", admin_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
            ("x-store-id", store_id_header.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let logs = fixture
        .context
        .platform
        .list_audit_logs(Some(&store_id), fastcart_kernel::Pagination::default())
        .await
        .unwrap();
    assert!(
        logs.iter().any(|log| {
            log.action == "admin.post"
                && log.resource_id.as_deref() == Some("/api/v1/admin/tax/categories")
        }),
        "missing fallback audit entry: {logs:?}"
    );
}
