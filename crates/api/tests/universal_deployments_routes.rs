//! Route-level coverage for `/internal/ssr/v1/deployments/{store_id}/...`.
//!
//! These endpoints activate rollbacks and rotate the server-only SSR
//! credentials, so the tests focus on the boundary: a credential may only act
//! on the store it is scoped to, preview credentials may not act at all, and
//! rotation/revocation take effect immediately.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use fastcart_api::state::{
    CapabilityGrants, InternalSsrCredentials, InternalSsrToken, UniversalStorefront,
};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::StoreContext;
use fastcart_platform::{CreateStoreInput, PlatformStore, StoreProvisioningService};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Token the store A SSR server authenticates with.
const STORE_A_TOKEN: &str = "new-ssr-token-at-least-16";
/// Token the store B SSR server authenticates with.
const STORE_B_TOKEN: &str = "{{SECRET_9ysb4ak4}}";
/// Preview-only credential scoped to store A.
const PREVIEW_TOKEN: &str = "{{SECRET_8qmx6nlg}}";
/// Unscoped operator credential (all stores, no preview).
const OPERATOR_TOKEN: &str = "{{SECRET_qxhy92f6}}";

/// Records every deployment action the routes forward to the runtime.
#[derive(Default)]
struct RecordingUniversal {
    actions: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl UniversalStorefront for RecordingUniversal {
    async fn render(
        &self,
        _request: Request<Body>,
        _store: StoreContext,
        _capabilities: CapabilityGrants,
        _request_id: Option<String>,
        _scheme: String,
    ) -> Response {
        StatusCode::NOT_FOUND.into_response()
    }

    async fn serve_asset(&self, _store_id: &str, _path: String, _headers: HeaderMap) -> Response {
        StatusCode::NOT_FOUND.into_response()
    }

    async fn activate(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: String,
    ) -> Result<(), String> {
        self.actions.lock().unwrap().push(format!(
            "activate:{store_id}:{deployment_id}:{artifact_path}"
        ));
        Ok(())
    }

    async fn rollback(&self, store_id: &str) -> Result<(), String> {
        self.actions
            .lock()
            .unwrap()
            .push(format!("rollback:{store_id}"));
        Ok(())
    }

    async fn activate_preview(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: String,
    ) -> Result<(), String> {
        self.actions.lock().unwrap().push(format!(
            "preview-activate:{store_id}:{deployment_id}:{artifact_path}"
        ));
        Ok(())
    }

    async fn deactivate_preview(&self, store_id: &str) -> Result<(), String> {
        self.actions
            .lock()
            .unwrap()
            .push(format!("preview-deactivate:{store_id}"));
        Ok(())
    }
}

struct Fixture {
    app: Router,
    runtime: Arc<RecordingUniversal>,
    store_a: String,
    store_b: String,
    _dir: tempfile::TempDir,
}

impl Fixture {
    /// Returns every action the runtime recorded, in order.
    fn actions(&self) -> Vec<String> {
        self.runtime.actions.lock().unwrap().clone()
    }

    /// The deployment URL for one store and action.
    fn url(&self, store: &str, action: &str) -> String {
        format!("/internal/ssr/v1/deployments/{store}/{action}")
    }
}

async fn fixture() -> Fixture {
    fixture_with(false).await
}

/// Builds the fixture with an in-memory writable credential manager, or with a
/// read-only file-backed one.
async fn fixture_with(read_only_backend: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    // Both stores must be active before any deployment call is accepted.
    let store_a = platform
        .create_store(CreateStoreInput::new("Alpha", "alpha", "alpha.example.com"))
        .await
        .unwrap();
    let store_b = platform
        .create_store(CreateStoreInput::new("Beta", "beta", "beta.example.com"))
        .await
        .unwrap();
    let provisioning = StoreProvisioningService::new(platform.clone(), databases.clone());
    for store in [&store_a, &store_b] {
        provisioning.provision(&store.id, None).await.unwrap();
    }

    let runtime = Arc::new(RecordingUniversal::default());
    let tokens = vec![
        InternalSsrToken {
            token: STORE_A_TOKEN.to_owned(),
            store_id: Some(store_a.id.to_string()),
            expires_at: None,
            preview: false,
        },
        InternalSsrToken {
            token: STORE_B_TOKEN.to_owned(),
            store_id: Some(store_b.id.to_string()),
            expires_at: None,
            preview: false,
        },
        InternalSsrToken {
            token: PREVIEW_TOKEN.to_owned(),
            store_id: Some(store_a.id.to_string()),
            expires_at: None,
            preview: true,
        },
        InternalSsrToken {
            token: OPERATOR_TOKEN.to_owned(),
            store_id: None,
            expires_at: None,
            preview: false,
        },
    ];
    let credentials = if read_only_backend {
        // A deployment-owned secret store cannot be rotated from the API.
        InternalSsrCredentials::load_file(
            dir.path().join("ssr-credentials.json"),
            tokens,
            true,
            true,
        )
        .await
        .unwrap()
    } else {
        InternalSsrCredentials::new(tokens)
    };
    let context = Arc::new(
        AppContext::new(
            ApiConfig {
                cookie_secure: false,
                backup_dir: dir.path().join("backups"),
                internal_ssr_credentials: credentials,
                ..ApiConfig::default()
            },
            platform,
            databases,
            StoreCache::default_capacity(),
        )
        .with_universal_storefront(Some(runtime.clone() as Arc<dyn UniversalStorefront>)),
    );

    Fixture {
        app: build_router(context),
        runtime,
        store_a: store_a.id.to_string(),
        store_b: store_b.id.to_string(),
        _dir: dir,
    }
}

/// Sends one deployment request. `auth` is the raw credential value, sent as a
/// bearer token unless a header name is given.
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    auth: Option<(&str, &str)>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some((header, value)) = auth {
        // Only the `authorization` header carries the `Bearer ` scheme; the
        // dedicated internal header expects the raw token.
        if header.eq_ignore_ascii_case("authorization") {
            builder = builder.header(header, format!("Bearer {value}"));
        } else {
            builder = builder.header(header, value);
        }
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
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

async fn post(
    fixture: &Fixture,
    uri: &str,
    body: Option<Value>,
    token: &str,
) -> (StatusCode, Value) {
    send(
        &fixture.app,
        "POST",
        uri,
        body,
        Some(("authorization", token)),
    )
    .await
}

/// A body that deserializes for `action`, so the request reaches the scope
/// check instead of failing in the JSON extractor.
fn action_body(action: &str) -> Option<Value> {
    match action {
        "activate" | "preview/activate" => {
            Some(json!({"deployment_id": "dep_1", "artifact_path": "/srv/artifacts/1"}))
        }
        "rotate-token" => Some(json!({"token": "{{SECRET_a6pn94xu}}"})),
        "revoke-token" => Some(json!({"token": "{{SECRET_9ysb4ak4}}"})),
        "rollback" | "preview/deactivate" => None,
        other => panic!("unknown deployment action {other}"),
    }
}

#[tokio::test]
async fn deployments_reject_missing_preview_and_cross_store_credentials() {
    let fixture = fixture().await;
    let activate = fixture.url(&fixture.store_a, "activate");
    let body = action_body("activate");

    // No credential at all.
    let (status, error) = send(&fixture.app, "POST", &activate, body.clone(), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
    assert_eq!(error["error"]["code"], "FORBIDDEN");

    // A made-up credential.
    let (status, _) = post(&fixture, &activate, body.clone(), "not-a-real-token").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Positive control: the preview credential is valid for its store on the
    // storefront surface, so the rejections below are the deployment rules at
    // work and not a rejected token.
    let (status, regions) = send(
        &fixture.app,
        "GET",
        &format!("/internal/ssr/v1/stores/{}/regions", fixture.store_a),
        None,
        Some(("authorization", PREVIEW_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{regions}");

    // A preview credential may never activate, roll back or rotate.
    for action in [
        "activate",
        "rollback",
        "preview/activate",
        "preview/deactivate",
        "rotate-token",
        "revoke-token",
    ] {
        let (status, _) = post(
            &fixture,
            &fixture.url(&fixture.store_a, action),
            action_body(action),
            PREVIEW_TOKEN,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "preview on {action}");
    }

    // Store B's credential cannot act on store A, and vice versa.
    for action in [
        "activate",
        "rollback",
        "preview/activate",
        "preview/deactivate",
    ] {
        let (status, _) = post(
            &fixture,
            &fixture.url(&fixture.store_a, action),
            action_body(action),
            STORE_B_TOKEN,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "store B on A/{action}");
    }
    let (status, _) = post(
        &fixture,
        &fixture.url(&fixture.store_b, "rollback"),
        None,
        STORE_A_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "store A on B/rollback");

    // Store B's credential cannot rotate or revoke store A's token.
    let rotate_body = json!({"token": "new-ssr-token-at-least-16"});
    for action in ["rotate-token", "revoke-token"] {
        let (status, _) = post(
            &fixture,
            &fixture.url(&fixture.store_a, action),
            Some(rotate_body.clone()),
            STORE_B_TOKEN,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "store B on A/{action}");
    }

    assert!(
        fixture.actions().is_empty(),
        "no rejected request may reach the runtime: {:?}",
        fixture.actions()
    );
}

#[tokio::test]
async fn activation_rollback_and_preview_flow() {
    let fixture = fixture().await;

    let (status, body) = post(
        &fixture,
        &fixture.url(&fixture.store_a, "activate"),
        Some(json!({"deployment_id": "dep_1", "artifact_path": "/srv/artifacts/1"})),
        STORE_A_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (status, _) = post(
        &fixture,
        &fixture.url(&fixture.store_a, "rollback"),
        None,
        STORE_A_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = post(
        &fixture,
        &fixture.url(&fixture.store_a, "preview/activate"),
        Some(json!({"deployment_id": "dep_2", "artifact_path": "/srv/preview/2"})),
        STORE_A_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = post(
        &fixture,
        &fixture.url(&fixture.store_a, "preview/deactivate"),
        None,
        STORE_A_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert_eq!(
        fixture.actions(),
        vec![
            format!("activate:{}:dep_1:/srv/artifacts/1", fixture.store_a),
            format!("rollback:{}", fixture.store_a),
            format!("preview-activate:{}:dep_2:/srv/preview/2", fixture.store_a),
            format!("preview-deactivate:{}", fixture.store_a),
        ]
    );

    // The documented header alternative to `Authorization: Bearer`.
    let (status, _) = send(
        &fixture.app,
        "POST",
        &fixture.url(&fixture.store_a, "rollback"),
        None,
        Some(("x-fastcart-internal-token", STORE_A_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A body missing its deployment fields is rejected by the JSON extractor
    // before the store is even resolved.
    let (status, _) = post(
        &fixture,
        &fixture.url(&fixture.store_a, "activate"),
        Some(json!({"deployment_id": "dep_3"})),
        STORE_A_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn unknown_stores_are_not_found_for_unscoped_credentials() {
    let fixture = fixture().await;

    // An unscoped credential may act for a live store...
    let (status, body) = post(
        &fixture,
        &fixture.url(&fixture.store_a, "rollback"),
        None,
        OPERATOR_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // ...but a store that does not exist is a not-found, not a runtime call.
    let ghost = fixture.url("store_ghost", "rollback");
    let (status, error) = post(&fixture, &ghost, None, OPERATOR_TOKEN).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    assert_eq!(error["error"]["code"], "NOT_FOUND");

    // A store-scoped credential cannot reach even a live store it is not
    // scoped to, so the ghost case never leaks through it.
    let (status, _) = post(&fixture, &ghost, None, STORE_A_TOKEN).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    assert_eq!(
        fixture.actions(),
        vec![format!("rollback:{}", fixture.store_a)],
        "only the live, permitted store reached the runtime"
    );
}

#[tokio::test]
async fn a_read_only_credential_backend_refuses_rotation_and_revocation() {
    let fixture = fixture_with(true).await;
    let rollback = fixture.url(&fixture.store_a, "rollback");

    // Reading is unaffected: authorization and activation still work.
    let (status, body) = post(&fixture, &rollback, None, STORE_A_TOKEN).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    for (action, body) in [
        ("rotate-token", json!({"token": "{{SECRET_a6pn94xu}}"})),
        (
            "revoke-token",
            json!({"token": "new-ssr-token-at-least-16"}),
        ),
    ] {
        let (status, error) = post(
            &fixture,
            &fixture.url(&fixture.store_a, action),
            Some(body),
            OPERATOR_TOKEN,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{action}: {error}");
        assert_eq!(error["error"]["code"], "CONFLICT", "{action}");
    }

    // Nothing changed: the original credential is still the working one.
    let (status, _) = post(&fixture, &rollback, None, STORE_A_TOKEN).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn token_rotation_takes_effect_immediately_and_revocation_kills_it() {
    let fixture = fixture().await;
    let rollback = fixture.url(&fixture.store_a, "rollback");
    let rotated = "{{SECRET_a6pn94xu}}";

    // The original credential works.
    let (status, _) = post(&fixture, &rollback, None, STORE_A_TOKEN).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A too-short replacement token is rejected before any rotation happens.
    let (status, error) = post(
        &fixture,
        &fixture.url(&fixture.store_a, "rotate-token"),
        Some(json!({"token": "short"})),
        OPERATOR_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["error"]["code"], "VALIDATION_ERROR");

    // An expiry that is already past is equally rejected.
    let (status, error) = post(
        &fixture,
        &fixture.url(&fixture.store_a, "rotate-token"),
        Some(json!({"token": "new-ssr-token-at-least-16", "expires_at": 1})),
        OPERATOR_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");

    // A real rotation with no grace period retires the old credential at once.
    let (status, error) = post(
        &fixture,
        &fixture.url(&fixture.store_a, "rotate-token"),
        Some(json!({"token": rotated, "grace_seconds": 0})),
        OPERATOR_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{error}");

    let (status, _) = post(&fixture, &rollback, None, STORE_A_TOKEN).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the replaced token is retired"
    );
    let (status, _) = post(&fixture, &rollback, None, rotated).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the replacement works");

    // Revoking the replacement kills it immediately.
    let (status, _) = post(
        &fixture,
        &fixture.url(&fixture.store_a, "revoke-token"),
        Some(json!({"token": rotated})),
        OPERATOR_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = post(&fixture, &rollback, None, rotated).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Store B's own credential was untouched by any of this.
    let (status, _) = post(
        &fixture,
        &fixture.url(&fixture.store_b, "rollback"),
        None,
        STORE_B_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}
