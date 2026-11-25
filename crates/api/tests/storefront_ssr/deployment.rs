use super::support::*;
use super::*;

#[tokio::test]
async fn deployment_lifecycle_requires_the_internal_token() {
    let fixture = fixture(Some(Arc::new(RecordingUniversal::default()))).await;
    let activate_path = format!("/internal/ssr/v1/deployments/{}/activate", fixture.store_id);
    let body = serde_json::json!({
        "deployment_id": "deploy_next",
        "artifact_path": "/tmp/next-artifact"
    });

    let unauthorized = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&activate_path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::FORBIDDEN);

    let authorized = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&activate_path)
                .header("authorization", "Bearer ssr-internal-secret")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::NO_CONTENT);

    let rollback = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/rollback",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rollback.status(), StatusCode::NO_CONTENT);

    let preview = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/preview/activate",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::NO_CONTENT);

    let deactivate = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/preview/deactivate",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deactivate.status(), StatusCode::NO_CONTENT);

    let rotate_body = serde_json::json!({
        "token": "rotated-token-0123456789",
        "grace_seconds": 300
    });
    let rotated = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/rotate-token",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .header("content-type", "application/json")
                .body(Body::from(rotate_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rotated.status(), StatusCode::NO_CONTENT);

    let new_token = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/internal/ssr/v1/stores/{}/context",
                    fixture.store_id
                ))
                .header("authorization", "Bearer rotated-token-0123456789")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_token.status(), StatusCode::OK);

    let revoked = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/revoke-token",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"token": "rotated-token-0123456789"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let after_revoke = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/internal/ssr/v1/stores/{}/context",
                    fixture.store_id
                ))
                .header("authorization", "Bearer rotated-token-0123456789")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after_revoke.status(), StatusCode::FORBIDDEN);

    let expired = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/rotate-token",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "token": "another-token-0123456789",
                        "expires_at": 1
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::BAD_REQUEST);

    let logs = fixture
        .platform
        .list_audit_logs(
            Some(&fastcart_kernel::StoreId::from_string(
                fixture.store_id.clone(),
            )),
            fastcart_kernel::Pagination {
                limit: 20,
                offset: 0,
            },
        )
        .await
        .expect("audit logs");
    for action in [
        "ssr.deployment.activate",
        "ssr.deployment.rollback",
        "ssr.preview.activate",
        "ssr.preview.deactivate",
        "ssr.credential.rotate",
        "ssr.credential.revoke",
    ] {
        assert!(
            logs.iter().any(|entry| entry.action == action),
            "missing audit action {action}"
        );
    }
}
