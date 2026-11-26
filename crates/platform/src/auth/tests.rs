use fastcart_kernel::Pagination;

use super::{AuthService, hash_token};
use crate::repository::PlatformStore;
use crate::store::CreateStoreInput;
use crate::user::CreateUserInput;

async fn service() -> AuthService {
    AuthService::new(PlatformStore::in_memory().await.unwrap())
}

#[tokio::test]
async fn passwords_are_hashed_and_verified() {
    let hash = AuthService::hash_password("correct horse battery staple").unwrap();
    assert_ne!(hash, "correct horse battery staple");
    assert!(AuthService::verify_password(
        &hash,
        "correct horse battery staple"
    ));
    assert!(!AuthService::verify_password(&hash, "wrong"));
}

#[tokio::test]
async fn registration_login_and_session_authentication() {
    let auth = service().await;
    let user = auth
        .register(
            CreateUserInput::new("owner@example.com").with_name("Ada", "Lovelace"),
            "hunter2hunter2",
        )
        .await
        .unwrap();

    let session = auth
        .login("OWNER@example.com", "hunter2hunter2")
        .await
        .unwrap();
    assert_eq!(session.user.id, user.id);
    assert!(!session.token.is_empty());
    assert!(!session.csrf_token.is_empty());

    let authenticated = auth.authenticate_session(&session.token).await.unwrap();
    assert_eq!(authenticated.user.id, user.id);
    assert_eq!(authenticated.session_id, session.session_id);
}

/// Argon2 must not run on the async worker: a single-threaded runtime has
/// to keep polling other tasks while the (deliberately slow) hash is
/// computed, otherwise one login attempt starves the whole worker.
#[tokio::test(flavor = "current_thread")]
async fn login_does_not_block_the_async_runtime() {
    let auth = service().await;
    auth.register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
        .await
        .unwrap();

    // The watchdog ticks every millisecond and records the longest time it
    // went unscheduled. Database round trips are sub-millisecond, so any
    // multi-millisecond gap is the Argon2 call being run inline.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let max_gap_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let watchdog = tokio::spawn({
        let stop = stop.clone();
        let max_gap_ms = max_gap_ms.clone();
        async move {
            let mut last = tokio::time::Instant::now();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                let now = tokio::time::Instant::now();
                max_gap_ms.fetch_max(
                    u64::try_from((now - last).as_millis()).unwrap_or(u64::MAX),
                    std::sync::atomic::Ordering::Relaxed,
                );
                last = now;
            }
        }
    });
    tokio::task::yield_now().await;

    let started = std::time::Instant::now();
    auth.login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();
    let elapsed = started.elapsed();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    watchdog.await.unwrap();

    // Compare against the login duration rather than an absolute bound: on a
    // loaded machine the scheduler itself can pause the watchdog for a few
    // milliseconds. An inline hash would block for essentially the whole
    // login, so requiring the gap to stay under half of it keeps the property
    // while tolerating a busy host.
    let gap_ms = max_gap_ms.load(std::sync::atomic::Ordering::Relaxed);
    let budget = u64::try_from(elapsed.as_millis() / 2).unwrap_or(0).max(20);
    assert!(
        gap_ms < budget,
        "the async runtime was blocked for {gap_ms}ms while Argon2 verified a password"
    );
}

#[tokio::test]
async fn wrong_password_and_unknown_email_fail() {
    let auth = service().await;
    auth.register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
        .await
        .unwrap();
    assert!(
        auth.login("owner@example.com", "wrong-password")
            .await
            .is_err()
    );
    assert!(
        auth.login("missing@example.com", "hunter2hunter2")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn logout_and_password_change_revoke_sessions() {
    let auth = service().await;
    let user = auth
        .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
        .await
        .unwrap();
    let first = auth
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();

    auth.logout(&first.token).await.unwrap();
    assert!(auth.authenticate_session(&first.token).await.is_err());

    let second = auth
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();
    auth.change_password(&user.id, "hunter2hunter2", "new-password-1")
        .await
        .unwrap();
    assert!(auth.authenticate_session(&second.token).await.is_err());
    assert!(
        auth.login("owner@example.com", "hunter2hunter2")
            .await
            .is_err()
    );
    assert!(
        auth.login("owner@example.com", "new-password-1")
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn expired_sessions_are_rejected() {
    let auth = service().await;
    auth.register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
        .await
        .unwrap();
    let session = auth
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();
    sqlx::query("UPDATE admin_sessions SET expires_at = 0 WHERE id = ?")
        .bind(session.session_id.as_str())
        .execute(auth.platform().pool())
        .await
        .unwrap();
    assert!(auth.authenticate_session(&session.token).await.is_err());
}

#[tokio::test]
async fn store_authorization_requires_membership_and_permissions() {
    let auth = service().await;
    let user = auth
        .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
        .await
        .unwrap();
    let store = auth
        .platform()
        .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
        .await
        .unwrap();
    let session = auth
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();

    assert!(
        auth.authorize_for_store(&session.token, &store.id)
            .await
            .is_err()
    );

    auth.platform()
        .upsert_membership(&user.id, &store.id, &["owner"])
        .await
        .unwrap();
    let (_, permissions) = auth
        .authorize_for_store(&session.token, &store.id)
        .await
        .unwrap();
    assert!(permissions.contains("products.write"));
    assert!(permissions.contains("users.write"));
}

#[tokio::test]
async fn api_tokens_are_hashed_and_revocable() {
    let auth = service().await;
    let user = auth
        .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
        .await
        .unwrap();
    let store = auth
        .platform()
        .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
        .await
        .unwrap();
    auth.platform()
        .upsert_membership(&user.id, &store.id, &["owner"])
        .await
        .unwrap();

    let created = auth
        .create_api_token(
            &user.id,
            Some(&store.id),
            "automation",
            &["products.read".into()],
            None,
        )
        .await
        .unwrap();
    assert!(created.token.starts_with("fc_"));

    let stored_hash: String = sqlx::query_scalar("SELECT token_hash FROM api_tokens WHERE id = ?")
        .bind(&created.record.id)
        .fetch_one(auth.platform().pool())
        .await
        .unwrap();
    assert_eq!(stored_hash, hash_token(&created.token));
    assert_ne!(stored_hash, created.token);

    let authenticated = auth.authenticate_api_token(&created.token).await.unwrap();
    assert_eq!(authenticated.user_id, user.id);
    assert_eq!(authenticated.store_id, Some(store.id.clone()));
    assert_eq!(authenticated.scopes, vec!["products.read"]);

    auth.revoke_api_token(&user.id, &created.record.id)
        .await
        .unwrap();
    assert!(auth.authenticate_api_token(&created.token).await.is_err());

    let listed = auth.list_api_tokens(&user.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].revoked_at.is_some());
    assert!(!format!("{:?}", listed[0]).contains(&created.token));
}

#[tokio::test]
async fn api_tokens_require_a_store_and_cannot_escalate_scopes() {
    let auth = service().await;
    let user = auth
        .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
        .await
        .unwrap();
    assert!(
        auth.create_api_token(&user.id, None, "global", &["products.read".into()], None)
            .await
            .is_err()
    );
    let store = auth
        .platform()
        .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
        .await
        .unwrap();
    auth.platform()
        .upsert_membership(&user.id, &store.id, &["support-agent"])
        .await
        .unwrap();
    assert!(
        auth.create_api_token(
            &user.id,
            Some(&store.id),
            "escalated",
            &["products.write".into()],
            None,
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn api_tokens_stop_working_when_the_user_is_disabled() {
    let auth = service().await;
    let user = auth
        .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
        .await
        .unwrap();
    let store = auth
        .platform()
        .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
        .await
        .unwrap();
    auth.platform()
        .upsert_membership(&user.id, &store.id, &["owner"])
        .await
        .unwrap();
    let created = auth
        .create_api_token(
            &user.id,
            Some(&store.id),
            "automation",
            &["products.read".into()],
            None,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE users SET status = 'disabled' WHERE id = ?")
        .bind(user.id.as_str())
        .execute(auth.platform().pool())
        .await
        .unwrap();
    assert!(auth.authenticate_api_token(&created.token).await.is_err());
}

#[tokio::test]
async fn api_tokens_for_stores_require_membership() {
    let auth = service().await;
    let user = auth
        .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
        .await
        .unwrap();
    let store = auth
        .platform()
        .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
        .await
        .unwrap();
    assert!(
        auth.create_api_token(&user.id, Some(&store.id), "t", &[], None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn user_lists_are_paginated() {
    let auth = service().await;
    for index in 0..3 {
        auth.register(
            CreateUserInput::new(format!("user{index}@example.com")),
            "hunter2hunter2",
        )
        .await
        .unwrap();
    }
    let page = auth
        .platform()
        .list_users(Pagination::new(Some(2), Some(0)).unwrap())
        .await
        .unwrap();
    assert_eq!(page.data.len(), 2);
    assert_eq!(page.count, 3);
}
