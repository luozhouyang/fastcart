//! Cookie parsing and safe cookie construction.

use std::collections::HashMap;

use axum::http::HeaderMap;
use cookie::{Cookie, SameSite};

/// Parses the `Cookie` header into a name/value map. The first value wins.
#[must_use]
pub fn parse_cookies(headers: &HeaderMap) -> HashMap<String, String> {
    let mut cookies = HashMap::new();
    for header in headers.get_all(axum::http::header::COOKIE) {
        let Ok(value) = header.to_str() else {
            continue;
        };
        for pair in value.split(';') {
            let Ok(cookie) = Cookie::parse(pair.trim()) else {
                continue;
            };
            cookies
                .entry(cookie.name().to_owned())
                .or_insert_with(|| cookie.value().to_owned());
        }
    }
    cookies
}

/// Builds an HttpOnly session cookie.
///
/// `max_age_seconds` of zero creates a session cookie.
#[must_use]
pub fn session_cookie(name: &str, value: &str, max_age_seconds: i64, secure: bool) -> String {
    let mut builder = Cookie::build((name, value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax);
    if max_age_seconds > 0 {
        builder = builder.max_age(cookie::time::Duration::seconds(max_age_seconds));
    }
    if secure {
        builder = builder.secure(true);
    }
    builder.build().to_string()
}

/// Builds a JavaScript-readable CSRF cookie for double-submit protection.
#[must_use]
pub fn csrf_cookie(name: &str, value: &str, max_age_seconds: i64, secure: bool) -> String {
    let mut builder = Cookie::build((name, value))
        .path("/")
        .http_only(false)
        .same_site(SameSite::Lax);
    if max_age_seconds > 0 {
        builder = builder.max_age(cookie::time::Duration::seconds(max_age_seconds));
    }
    if secure {
        builder = builder.secure(true);
    }
    builder.build().to_string()
}

/// Builds a cookie that clears the named value.
#[must_use]
pub fn session_cookie_clear(name: &str, secure: bool) -> String {
    let mut builder = Cookie::build((name, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::ZERO);
    if secure {
        builder = builder.secure(true);
    }
    builder.build().to_string()
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::{csrf_cookie, parse_cookies, session_cookie, session_cookie_clear};

    #[test]
    fn parses_multiple_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_static("a=1; b=two=3; c="),
        );
        let cookies = parse_cookies(&headers);
        assert_eq!(cookies.get("a").map(String::as_str), Some("1"));
        assert_eq!(cookies.get("b").map(String::as_str), Some("two=3"));
        assert_eq!(cookies.get("c").map(String::as_str), Some(""));
    }

    #[test]
    fn session_cookies_are_hardened() {
        let cookie = session_cookie("session", "abc", 3600, true);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("Max-Age=3600"));
        assert!(session_cookie_clear("session", true).contains("Max-Age=0"));
        assert!(!session_cookie("session", "abc", 0, false).contains("Max-Age"));
        assert!(!csrf_cookie("csrf", "x", 3600, false).contains("HttpOnly"));
        assert!(csrf_cookie("csrf", "x", 3600, false).contains("Max-Age=3600"));
    }
}
