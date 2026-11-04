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
const CONTENT_SECURITY_POLICY: HeaderName = HeaderName::from_static("content-security-policy");

/// Inputs that control security response headers.
#[derive(Clone, Debug, Default)]
pub struct SecurityHeadersConfig {
    /// Whether the deployment terminates TLS and should emit HSTS.
    pub cookie_secure: bool,
    /// Origins allowed to frame `/preview` in an iframe.
    ///
    /// When empty no `Content-Security-Policy` is emitted and framing is left
    /// to the browser default; configured deployments should list the admin
    /// application origin so the preview cannot be framed elsewhere.
    pub preview_frame_ancestors: Vec<String>,
}

impl SecurityHeadersConfig {
    /// Builds the preview `frame-ancestors` directive, if configured.
    fn preview_csp(&self) -> Option<HeaderValue> {
        if self.preview_frame_ancestors.is_empty() {
            return None;
        }
        let mut value = String::from("frame-ancestors 'self'");
        for origin in &self.preview_frame_ancestors {
            // Origins are validated when configuration loads; refuse values
            // that could inject additional directives.
            if origin.is_empty()
                || origin
                    .bytes()
                    .any(|byte| !byte.is_ascii_graphic() || byte == b';' || byte == b',')
            {
                continue;
            }
            value.push(' ');
            value.push_str(origin);
        }
        HeaderValue::from_str(&value).ok()
    }
}

/// Adds conservative security headers to every response.
///
/// HSTS is only emitted when the deployment is configured for secure cookies,
/// which indicates the API is served over TLS. Preview responses replace
/// `X-Frame-Options` with a configured `frame-ancestors` policy so only the
/// admin application may embed them.
pub async fn security_headers_middleware(
    State(config): State<SecurityHeadersConfig>,
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
        // control; frame-ancestors narrows embedding to the configured admin
        // origin when one is available.
        headers.remove(X_FRAME_OPTIONS);
        if let Some(policy) = config.preview_csp() {
            headers.insert(CONTENT_SECURITY_POLICY, policy);
        }
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
    if config.cookie_secure {
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

    use super::{SecurityHeadersConfig, security_headers_middleware};

    async fn app(enable_hsts: bool) -> Router {
        app_with(SecurityHeadersConfig {
            cookie_secure: enable_hsts,
            preview_frame_ancestors: Vec::new(),
        })
        .await
    }

    async fn app_with(config: SecurityHeadersConfig) -> Router {
        Router::new()
            .route("/", get(|| async { StatusCode::OK }))
            .route("/preview/page", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn_with_state(
                config,
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
    async fn preview_frame_ancestors_are_enforced_when_configured() {
        let app = app_with(SecurityHeadersConfig {
            cookie_secure: false,
            preview_frame_ancestors: vec!["https://admin.example.com".to_owned()],
        })
        .await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/preview/page?token=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.headers().get("x-frame-options").is_none());
        assert_eq!(
            response.headers()["content-security-policy"],
            "frame-ancestors 'self' https://admin.example.com"
        );

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.headers()["x-frame-options"], "DENY");
        assert!(response.headers().get("content-security-policy").is_none());
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
