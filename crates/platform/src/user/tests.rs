use fastcart_kernel::Pagination;

use crate::repository::PlatformStore;
use crate::store::CreateStoreInput;
use crate::user::{CreateUserInput, MembershipStatus, NewAudit, UserStatus};

async fn fixture() -> (
    PlatformStore,
    fastcart_kernel::UserId,
    fastcart_kernel::StoreId,
) {
    let platform = PlatformStore::in_memory().await.unwrap();
    let user = platform
        .create_user_with_credential(
            CreateUserInput::new("Owner@Example.com").with_name("Ada", "Lovelace"),
            "hash",
        )
        .await
        .unwrap();
    let store = platform
        .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
        .await
        .unwrap();
    (platform, user.id, store.id)
}

#[tokio::test]
async fn users_are_created_with_normalized_email() {
    let (platform, user_id, _) = fixture().await;
    let user = platform.get_user(&user_id).await.unwrap().unwrap();
    assert_eq!(user.email, "owner@example.com");
    assert_eq!(user.status, UserStatus::Active);
    assert_eq!(
        platform
            .get_password_hash(&user_id)
            .await
            .unwrap()
            .as_deref(),
        Some("hash")
    );
}

#[tokio::test]
async fn duplicate_emails_are_rejected() {
    let (platform, _, _) = fixture().await;
    let error = platform
        .create_user_with_credential(CreateUserInput::new("OWNER@example.com"), "hash")
        .await
        .unwrap_err();
    assert!(matches!(error, crate::PlatformError::EmailConflict(_)));
}

#[tokio::test]
async fn memberships_grant_role_permissions() {
    let (platform, user_id, store_id) = fixture().await;
    assert!(!platform.is_member(&user_id, &store_id).await.unwrap());

    let membership = platform
        .upsert_membership(&user_id, &store_id, &["product-manager"])
        .await
        .unwrap();
    assert_eq!(membership.status, MembershipStatus::Active);
    assert_eq!(membership.roles, vec!["product-manager"]);

    let permissions = platform.permissions_for(&user_id, &store_id).await.unwrap();
    assert!(permissions.contains("products.write"));
    assert!(permissions.contains("inventory.write"));
    assert!(!permissions.contains("users.write"));

    platform
        .revoke_membership(&user_id, &store_id)
        .await
        .unwrap();
    assert!(
        platform
            .permissions_for(&user_id, &store_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn unknown_roles_are_rejected() {
    let (platform, user_id, store_id) = fixture().await;
    assert!(
        platform
            .upsert_membership(&user_id, &store_id, &["wizard"])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn audit_logs_are_recorded() {
    let (platform, _, store_id) = fixture().await;
    platform
        .record_audit(NewAudit {
            actor_id: Some("user_1".to_owned()),
            store_id: Some(store_id.to_string()),
            action: "store.created".to_owned(),
            resource_type: Some("store".to_owned()),
            resource_id: Some(store_id.to_string()),
            data: serde_json::json!({"ok": true}),
            ip_address: None,
        })
        .await
        .unwrap();
    let logs = platform
        .list_audit_logs(Some(&store_id), Pagination::default())
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].action, "store.created");
}

#[tokio::test]
async fn audit_redelivery_with_the_same_id_is_ignored() {
    let (platform, _, store_id) = fixture().await;
    let entry = NewAudit {
        actor_id: Some("user_1".to_owned()),
        store_id: Some(store_id.to_string()),
        action: "page.updated".to_owned(),
        resource_type: Some("page".to_owned()),
        resource_id: Some("page_1".to_owned()),
        data: serde_json::json!({"revision": 2}),
        ip_address: None,
    };
    platform
        .record_audit_with_id("audit_delivery_1", &entry)
        .await
        .unwrap();
    platform
        .record_audit_with_id("audit_delivery_1", &entry)
        .await
        .unwrap();

    let logs = platform
        .list_audit_logs(Some(&store_id), Pagination::default())
        .await
        .unwrap();
    assert_eq!(logs.len(), 1, "outbox retries must not duplicate rows");
}

#[tokio::test]
async fn duplicate_roles_are_deduplicated_and_audited_once() {
    let (platform, user_id, store_id) = fixture().await;
    let audited = platform
        .clone()
        .with_audit_actor(Some("admin_1".to_owned()));

    let membership = audited
        .upsert_membership(&user_id, &store_id, &["owner", "owner"])
        .await
        .unwrap();
    assert_eq!(membership.roles, vec!["owner"]);

    let role_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM membership_roles WHERE membership_id = ?")
            .bind(&membership.id)
            .fetch_one(audited.pool())
            .await
            .unwrap();
    assert_eq!(role_rows, 1);

    let logs = audited
        .list_audit_logs(Some(&store_id), Pagination::default())
        .await
        .unwrap();
    let upserts = logs
        .iter()
        .filter(|log| log.action == "membership.upserted")
        .count();
    assert_eq!(upserts, 1, "one mutation must produce one audit row");
}

#[tokio::test]
async fn last_owner_cannot_be_demoted_or_removed() {
    let (platform, owner_id, store_id) = fixture().await;
    platform
        .upsert_membership(&owner_id, &store_id, &["owner"])
        .await
        .unwrap();

    let demote = platform
        .upsert_membership(&owner_id, &store_id, &["admin"])
        .await
        .unwrap_err();
    assert!(matches!(demote, crate::PlatformError::Conflict(_)));
    assert_eq!(
        platform
            .get_membership(&owner_id, &store_id)
            .await
            .unwrap()
            .unwrap()
            .roles,
        vec!["owner"]
    );

    let remove = platform
        .revoke_membership(&owner_id, &store_id)
        .await
        .unwrap_err();
    assert!(matches!(remove, crate::PlatformError::Conflict(_)));

    let second = platform
        .create_user_with_credential(CreateUserInput::new("second@example.com"), "hash")
        .await
        .unwrap();
    platform
        .upsert_membership(&second.id, &store_id, &["owner"])
        .await
        .unwrap();

    // With a second owner present the first can be demoted and removed.
    platform
        .upsert_membership(&owner_id, &store_id, &["admin"])
        .await
        .unwrap();
    platform
        .revoke_membership(&owner_id, &store_id)
        .await
        .unwrap();

    // The second owner is now the last one and is protected again.
    let remove_last = platform
        .revoke_membership(&second.id, &store_id)
        .await
        .unwrap_err();
    assert!(matches!(remove_last, crate::PlatformError::Conflict(_)));
}

#[tokio::test]
async fn rbac_mutations_append_audits_without_secrets() {
    let (platform, _, store_id) = fixture().await;
    let audited = platform
        .clone()
        .with_audit_actor(Some("admin_1".to_owned()));
    let credential_hash = "credential-secret-value";

    let created = audited
        .create_user_with_credential(CreateUserInput::new("member@example.com"), credential_hash)
        .await
        .unwrap();
    audited
        .upsert_membership(&created.id, &store_id, &["admin", "admin"])
        .await
        .unwrap();
    audited
        .revoke_membership(&created.id, &store_id)
        .await
        .unwrap();

    let logs = audited
        .list_audit_logs(None, Pagination::default())
        .await
        .unwrap();
    let actor_logs: Vec<_> = logs
        .iter()
        .filter(|log| log.actor_id.as_deref() == Some("admin_1"))
        .collect();
    assert_eq!(actor_logs.len(), 3);
    for action in ["user.created", "membership.upserted", "membership.removed"] {
        assert!(
            actor_logs.iter().any(|log| log.action == action),
            "missing audit action {action}"
        );
    }
    for log in actor_logs {
        let record = serde_json::to_string(log).unwrap();
        assert!(
            !record.contains(credential_hash),
            "audit rows must never contain credential material"
        );
    }
}

#[tokio::test]
async fn verifying_an_unknown_user_is_not_found() {
    let platform = PlatformStore::in_memory().await.unwrap();
    let error = platform
        .verify_user_email(&fastcart_kernel::UserId::new())
        .await
        .unwrap_err();
    assert!(matches!(error, crate::PlatformError::NotFound(_)));
}
