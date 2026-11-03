//! Shared HTTP building blocks: error responses, extractors, cookies and
//! middleware for request correlation, store resolution and authentication.

pub mod cookies;
pub mod error;
pub mod extractors;
pub mod middleware;
pub mod rate_limit;

pub use cookies::{csrf_cookie, parse_cookies, session_cookie, session_cookie_clear};
pub use error::{ApiError, ApiResult};
pub use extractors::{AdminActor, ApiJson, CustomerActor, PaginationQuery, StoreCtx};
pub use middleware::{
    AdminAuthState, StoreResolutionState, admin_auth_middleware, request_id_middleware,
    store_resolution_middleware,
};
pub use rate_limit::RateLimiter;

/// Cookie name carrying the admin session token.
pub const ADMIN_SESSION_COOKIE: &str = "fc_admin_session";
/// Cookie name carrying the admin CSRF token (readable by JavaScript).
pub const ADMIN_CSRF_COOKIE: &str = "fc_csrf";
/// Header carrying the admin store selection.
pub const STORE_ID_HEADER: &str = "x-store-id";
/// Header carrying the CSRF token on cookie-authenticated mutations.
pub const CSRF_HEADER: &str = "x-csrf-token";
