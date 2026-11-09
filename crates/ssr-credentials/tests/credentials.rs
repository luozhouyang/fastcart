use std::sync::Arc;

use base64::Engine as _;

use fastcart_ssr_credentials::{
    CredentialBackend, CredentialError, CredentialManager, CredentialMode, CredentialSnapshot,
    CredentialToken, FileCredentialBackend,
};

fn token(value: &str, store: Option<&str>, preview: bool) -> CredentialToken {
    CredentialToken {
        token: value.to_owned(),
        store_id: store.map(str::to_owned),
        expires_at: None,
        preview,
    }
}

#[tokio::test]
async fn file_backend_roundtrip_and_cas() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("credentials.json");
    let backend = FileCredentialBackend::new(&path, false);
    assert!(backend.load().await.expect("empty load").is_none());

    let snapshot = CredentialSnapshot {
        revision: None,
        tokens: vec![token("initial-token-0123456789", Some("store_a"), false)],
    };
    let revision = backend.save(&snapshot, None).await.expect("seed");
    let loaded = backend.load().await.expect("load").expect("snapshot");
    assert_eq!(loaded.revision.as_deref(), Some(revision.as_str()));
    assert_eq!(loaded.tokens.len(), 1);

    let stale = backend
        .save(&snapshot, Some("stale-revision"))
        .await
        .expect_err("stale revision");
    assert!(matches!(
        stale,
        fastcart_ssr_credentials::BackendError::Conflict
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    let read_only = FileCredentialBackend::new(&path, true);
    assert!(matches!(
        read_only.save(&snapshot, Some(&revision)).await,
        Err(fastcart_ssr_credentials::BackendError::ReadOnly)
    ));
}

#[tokio::test]
async fn manager_rotates_persists_and_reloads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("credentials.json");
    let manager = CredentialManager::load_file(
        &path,
        vec![token("initial-token-0123456789", Some("store_a"), false)],
        false,
        true,
    )
    .await
    .expect("manager");
    assert!(manager.validate("initial-token-0123456789").is_some());

    manager
        .rotate(
            "store_a",
            "rotated-token-0123456789".to_owned(),
            None,
            0,
            false,
        )
        .await
        .expect("rotate");
    assert!(manager.validate("rotated-token-0123456789").is_some());
    assert!(manager.validate("initial-token-0123456789").is_none());

    let reloaded = CredentialManager::load_file(&path, Vec::new(), false, false)
        .await
        .expect("reload");
    assert!(reloaded.validate("rotated-token-0123456789").is_some());
    assert!(reloaded.validate("initial-token-0123456789").is_none());
}

#[tokio::test]
async fn empty_backend_without_seed_ignores_configured_tokens() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("credentials.json");
    let manager = CredentialManager::load_file(
        &path,
        vec![token("configured-token-0123456789", None, false)],
        false,
        false,
    )
    .await
    .expect("manager");
    assert!(manager.is_empty());
    assert!(manager.validate("configured-token-0123456789").is_none());
}

#[tokio::test]
async fn read_only_manager_rejects_writes_and_refreshes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("credentials.json");
    let backend = Arc::new(FileCredentialBackend::new(&path, true));
    let initial = CredentialSnapshot {
        revision: None,
        tokens: vec![token("read-only-token-0123456789", None, false)],
    };
    // Write through a writable backend first.
    FileCredentialBackend::new(&path, false)
        .save(&initial, None)
        .await
        .expect("seed");
    let manager = CredentialManager::load(backend, Vec::new(), false, CredentialMode::ReadOnly)
        .await
        .expect("manager");
    assert!(matches!(
        manager
            .rotate("store_a", "new-token-0123456789".to_owned(), None, 0, false)
            .await,
        Err(CredentialError::ReadOnly)
    ));
    assert!(!manager.refresh().await.expect("refresh"));
}

#[tokio::test]
async fn records_backend_metrics() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("credentials.json");
    let metrics = std::sync::Arc::new(fastcart_http::Metrics::new());
    let backend = std::sync::Arc::new(FileCredentialBackend::new(&path, false));
    let manager = CredentialManager::load_with_metrics(
        backend,
        vec![token("metric-token-0123456789", None, false)],
        true,
        CredentialMode::ReadWrite,
        Some(metrics.clone()),
    )
    .await
    .expect("manager");
    manager.refresh().await.expect("refresh");
    manager
        .rotate(
            "store_a",
            "rotated-metric-token-0123456789".to_owned(),
            None,
            0,
            false,
        )
        .await
        .expect("rotate");

    let rendered = metrics.render();
    assert!(rendered.contains("fastcart_ssr_credential_loads_total 2"));
    assert!(rendered.contains("fastcart_ssr_credential_saves_total 2"));
    assert!(rendered.contains("fastcart_ssr_credential_refreshes_total 1"));
    assert!(rendered.contains("fastcart_ssr_credential_tokens 2"));
    assert!(rendered.contains("fastcart_ssr_credential_last_success_timestamp_seconds"));
}

#[tokio::test]
async fn encrypted_file_roundtrip_and_wrong_key_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("credentials.json");
    let key = [7_u8; 32];
    let backend = FileCredentialBackend::with_key(&path, false, key);
    let snapshot = CredentialSnapshot {
        revision: None,
        tokens: vec![token("encrypted-token-0123456789", None, false)],
    };
    backend.save(&snapshot, None).await.expect("save");

    let raw = std::fs::read_to_string(&path).expect("raw file");
    assert!(!raw.contains("encrypted-token-0123456789"));
    assert!(raw.contains("xchacha20poly1305"));

    let loaded = backend.load().await.expect("load").expect("snapshot");
    assert_eq!(loaded.tokens[0].token, "encrypted-token-0123456789");

    let wrong = FileCredentialBackend::with_key(&path, false, [8_u8; 32]);
    assert!(matches!(
        wrong.load().await,
        Err(fastcart_ssr_credentials::BackendError::Invalid(_))
    ));

    let encoded = base64::engine::general_purpose::STANDARD.encode(key);
    assert_eq!(
        FileCredentialBackend::key_from_base64(&encoded).expect("key"),
        key
    );
}

#[tokio::test]
async fn encrypted_backend_reads_legacy_plaintext() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("credentials.json");
    let plain = FileCredentialBackend::new(&path, false);
    plain
        .save(
            &CredentialSnapshot {
                revision: None,
                tokens: vec![token("legacy-token-0123456789", None, false)],
            },
            None,
        )
        .await
        .expect("plain save");

    let encrypted = FileCredentialBackend::with_key(&path, false, [9_u8; 32]);
    let loaded = encrypted
        .load()
        .await
        .expect("legacy load")
        .expect("snapshot");
    assert_eq!(loaded.tokens[0].token, "legacy-token-0123456789");
}
