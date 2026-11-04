//! Baseline security response headers.

use axum::extract::{Request, State};
use axum::http::header::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
const X_FRAME_OPTIONS: HeaderName = HeaderName::from_static("x-frame-options");
const REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");
const PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");
const STRICT_TRANSPORT_SECURITY: HeaderName = HeaderName::from_static("strict-transport-security");

/// Adds conservative security headers to every response.
///
/// HSTS is only emitted when the deployment is configured for secure cookies,
/// which indicates the API is served over TLS.
pub async fn security_headers_middleware(
    State(enable_hsts): State<bool>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let is_preview = path == "/preview" || path.starts_with("/preview/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if is_preview {
        // The admin editor embeds the preview in an iframe on another origin,
        // so framing cannot be denied here. The token itself is the access
        // control. A stricter policy can be layered on with frame-ancestors
        // once the admin origin is configured.
        headers.remove(X_FRAME_OPTIONS);
        headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    } else {
        headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        headers.insert(
            REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        );
    }
    headers.insert(
        PERMISSIONS_POLICY,
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    if enable_hsts {
        headers.insert(
            STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::security_headers_middleware;

    async fn app(enable_hsts: bool) -> Router {
        Router::new()
            .route("/", get(|| async { StatusCode::OK }))
            .route("/preview/page", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn_with_state(
                enable_hsts,
                security_headers_middleware,
            ))
    }

    #[tokio::test]
    async fn adds_security_headers_without_hsts_on_http() {
        let response = app(false)
            .await
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(response.headers()["x-frame-options"], "DENY");
        assert_eq!(
            response.headers()["referrer-policy"],
            "strict-origin-when-cross-origin"
        );
        assert!(
            response
                .headers()
                .get("strict-transport-security")
                .is_none()
        );
    }

    #[tokio::test]
    async fn preview_responses_allow_framing_and_hide_the_referrer() {
        let response = app(false)
            .await
            .oneshot(
                Request::builder()
                    .uri("/preview/page?token=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.headers().get("x-frame-options").is_none());
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    }

    #[tokio::test]
    async fn adds_hsts_for_tls_deployments() {
        let response = app(true)
            .await
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.headers()["strict-transport-security"],
            "max-age=31536000; includeSubDomains"
        );
    }
}
