//! Request correlation, trusted store resolution and admin authentication.

use std::collections::BTreeSet;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::Response;
use fastcart_kernel::{ActorContext, AuthenticationMethod, CurrencyCode, StoreContext, StoreId};
use fastcart_platform::{AuthService, StoreResolver};
use subtle::ConstantTimeEq;

use crate::cookies::parse_cookies;
use crate::error::{ApiError, REQUEST_ID};
use crate::{ADMIN_CSRF_COOKIE, ADMIN_SESSION_COOKIE, CSRF_HEADER, STORE_ID_HEADER};

/// Correlation identifier attached to every request.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// The raw authentication token presented by the client.
///
/// Handlers use this only for logout and token rotation; authorization uses the
/// resolved [`fastcart_kernel::ActorContext`].
#[derive(Clone, Debug)]
pub struct SessionToken(pub String);

/// Adds `x-request-id` to requests and responses and scopes error rendering.
pub async fn request_id_middleware(mut request: Request, next: Next) -> Response {
    let id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
        .map_or_else(|| uuid::Uuid::now_v7().to_string(), str::to_owned);

    request.extensions_mut().insert(RequestId(id.clone()));
    let mut response = REQUEST_ID.scope(id.clone(), next.run(request)).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

/// Configuration for the admin authentication middleware.
#[derive(Clone, Debug)]
pub struct AdminAuthState {
    /// Authentication service backed by the platform database.
    pub auth: AuthService,
    /// Whether a store must be selected with `x-store-id`.
    pub require_store: bool,
}

/// Authenticates admin requests by session cookie or bearer API token.
///
/// Cookie-authenticated unsafe requests must present the double-submit CSRF
/// token from the `fc_csrf` cookie in the `x-csrf-token` header.
pub async fn admin_auth_middleware(
    State(state): State<AdminAuthState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let unsafe_method = is_unsafe_method(request.method());
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let cookies = parse_cookies(request.headers());
    let cookie_token = cookies.get(ADMIN_SESSION_COOKIE).cloned();

    let token = bearer
        .clone()
        .or(cookie_token.clone())
        .ok_or(ApiError::new(fastcart_kernel::AppError::Unauthorized))?;

    if bearer.is_none() && unsafe_method {
        let csrf_cookie = cookies
            .get(ADMIN_CSRF_COOKIE)
            .ok_or(ApiError::new(fastcart_kernel::AppError::Forbidden))?;
        let csrf_header = request
            .headers()
            .get(CSRF_HEADER)
            .and_then(|value| value.to_str().ok())
            .ok_or(ApiError::new(fastcart_kernel::AppError::Forbidden))?;
        if !bool::from(csrf_cookie.as_bytes().ct_eq(csrf_header.as_bytes())) {
            return Err(ApiError::new(fastcart_kernel::AppError::Forbidden));
        }
    }

    let requested_store = request
        .headers()
        .get(STORE_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(StoreId::from_string);

    if state.require_store && requested_store.is_none() {
        return Err(ApiError::validation(format!(
            "{} header is required",
            STORE_ID_HEADER
        )));
    }

    let actor = if token.starts_with("fc_") {
        if requested_store.is_none() {
            return Err(ApiError::new(fastcart_kernel::AppError::Forbidden));
        }
        let token = state.auth.authenticate_api_token(&token).await?;
        let Some(token_store) = token.store_id.as_ref() else {
            return Err(ApiError::new(fastcart_kernel::AppError::Forbidden));
        };
        if let Some(requested) = &requested_store {
            if requested != token_store {
                return Err(ApiError::new(fastcart_kernel::AppError::Forbidden));
            }
            if !state
                .auth
                .platform()
                .is_member(&token.user_id, requested)
                .await?
            {
                return Err(ApiError::new(fastcart_kernel::AppError::Forbidden));
            }
        }
        ActorContext {
            user_id: Some(token.user_id),
            customer_id: None,
            api_token_id: Some(token.token_id),
            store_id: requested_store,
            permissions: token.scopes.into_iter().collect::<BTreeSet<_>>(),
            authentication: AuthenticationMethod::ApiToken,
        }
    } else {
        let (session, permissions) = match &requested_store {
            Some(store_id) => state.auth.authorize_for_store(&token, store_id).await?,
            None => (
                state.auth.authenticate_session(&token).await?,
                BTreeSet::new(),
            ),
        };
        ActorContext::user(
            session.user.id,
            requested_store,
            permissions,
            AuthenticationMethod::Session,
        )
    };

    request.extensions_mut().insert(actor);
    request.extensions_mut().insert(SessionToken(token));
    Ok(next.run(request).await)
}

/// Configuration for the storefront store-resolution middleware.
#[derive(Clone, Debug)]
pub struct StoreResolutionState {
    /// Resolver backed by the platform registry and cache.
    pub resolver: StoreResolver,
}

/// Resolves the request `Host` to a trusted [`StoreContext`].
///
/// Unknown, suspended and deleted stores produce a controlled not-found error.
pub async fn store_resolution_middleware(
    State(state): State<StoreResolutionState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ApiError::validation("missing Host header"))?;

    let store = state.resolver.resolve_active(&host).await?;
    let currency = CurrencyCode::new(store.currency_code.clone()).map_err(|_| {
        ApiError::new(fastcart_kernel::AppError::Internal(
            "store currency is invalid".to_owned(),
        ))
    })?;
    request.extensions_mut().insert(StoreContext {
        store_id: store.id,
        domain: store
            .primary_domain
            .unwrap_or_else(|| host.to_ascii_lowercase()),
        locale: store.locale,
        currency,
        timezone: store.timezone,
    });
    Ok(next.run(request).await)
}

fn is_unsafe_method(method: &Method) -> bool {
    !matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Json;
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Extension;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
    use fastcart_kernel::{ActorContext, StoreContext};
    use fastcart_platform::{
        AuthService, CreateStoreInput, CreateUserInput, PlatformStore, StoreProvisioningService,
        StoreResolver, StoreStatus,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::{
        AdminAuthState, StoreResolutionState, admin_auth_middleware, request_id_middleware,
        store_resolution_middleware,
    };

    async fn actor_handler(Extension(actor): Extension<ActorContext>) -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "user_id": actor.user_id.map(|id| id.to_string()),
            "permissions": actor.permissions,
        }))
    }

    async fn store_handler(Extension(store): Extension<StoreContext>) -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "store_id": store.store_id.to_string(),
            "currency": store.currency.to_string(),
        }))
    }

    #[tokio::test]
    async fn request_ids_are_propagated_and_rendered_in_errors() {
        async fn failing() -> Result<Json<()>, super::ApiError> {
            Err(super::ApiError::not_found("nope"))
        }
        let app = Router::new()
            .route("/fail", get(failing))
            .layer(middleware::from_fn(request_id_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/fail")
                    .header("x-request-id", "req_123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response
                .headers()
                .get("x-request-id")
                .unwrap()
                .to_str()
                .unwrap(),
            "req_123"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["request_id"], "req_123");
    }

    #[tokio::test]
    async fn admin_requests_require_a_session_and_store_membership() {
        let platform = PlatformStore::in_memory().await.unwrap();
        let auth = AuthService::new(platform.clone());
        let user = auth
            .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
            .await
            .unwrap();
        let store = platform
            .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
            .await
            .unwrap();
        let session = auth
            .login("owner@example.com", "hunter2hunter2")
            .await
            .unwrap();

        let app = |state: AdminAuthState| {
            Router::new()
                .route("/me", get(actor_handler))
                .layer(middleware::from_fn_with_state(state, admin_auth_middleware))
        };

        let state = AdminAuthState {
            auth: auth.clone(),
            require_store: true,
        };

        let unauthorized = app(state.clone())
            .oneshot(Request::builder().uri("/me").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let forbidden = app(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/me")
                    .header("x-store-id", store.id.as_str())
                    .header("authorization", format!("Bearer {}", session.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

        platform
            .upsert_membership(&user.id, &store.id, &["owner"])
            .await
            .unwrap();

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/me")
                    .header("x-store-id", store.id.as_str())
                    .header("authorization", format!("Bearer {}", session.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["permissions"][0], "customers.read");
    }

    #[tokio::test]
    async fn api_tokens_are_rejected_without_a_store_scope() {
        let platform = PlatformStore::in_memory().await.unwrap();
        let auth = AuthService::new(platform.clone());
        let user = auth
            .register(CreateUserInput::new("owner@example.com"), "hunter2hunter2")
            .await
            .unwrap();
        let store = platform
            .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
            .await
            .unwrap();
        platform
            .upsert_membership(&user.id, &store.id, &["owner"])
            .await
            .unwrap();
        let token = auth
            .create_api_token(
                &user.id,
                Some(&store.id),
                "automation",
                &["products.read".into()],
                None,
            )
            .await
            .unwrap();
        let app =
            Router::new()
                .route("/me", get(actor_handler))
                .layer(middleware::from_fn_with_state(
                    AdminAuthState {
                        auth,
                        require_store: false,
                    },
                    admin_auth_middleware,
                ));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/me")
                    .header("authorization", format!("Bearer {}", token.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn suspended_stores_fail_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let platform = PlatformStore::in_memory().await.unwrap();
        let databases = Arc::new(StoreDatabaseManager::new(
            dir.path().join("stores"),
            DatabaseOptions::default(),
        ));
        let provisioning = StoreProvisioningService::new(platform.clone(), databases);
        let store = platform
            .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
            .await
            .unwrap();
        provisioning.provision(&store.id, None).await.unwrap();

        let resolver = StoreResolver::new(platform.clone(), StoreCache::default_capacity());
        let app =
            Router::new()
                .route("/", get(store_handler))
                .layer(middleware::from_fn_with_state(
                    StoreResolutionState { resolver },
                    store_resolution_middleware,
                ));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("host", "demo.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        platform
            .update_status(&store.id, StoreStatus::Suspended)
            .await
            .unwrap();
        // Rebuild resolver to avoid the cached active store.
        let resolver = StoreResolver::new(platform, StoreCache::default_capacity());
        let app =
            Router::new()
                .route("/", get(store_handler))
                .layer(middleware::from_fn_with_state(
                    StoreResolutionState { resolver },
                    store_resolution_middleware,
                ));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("host", "demo.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
