//! Allowlisted outbound HTTP for plugins.
//!
//! The plugin worker has `globalOutbound: null`; this is the only path out.
//! The host applies the manifest allowlist, blocks private addresses, strips
//! credentials and enforces daily quotas.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;
use reqwest::Url;
use thiserror::Error;

const DAILY_REQUESTS: u64 = 1_000;
const DAILY_BYTES: u64 = 100 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// One egress request.
#[derive(Clone, Debug)]
pub struct EgressRequest {
    /// Plugin id.
    pub plugin_id: String,
    /// Store id.
    pub store_id: String,
    /// Target URL.
    pub url: String,
    /// HTTP method.
    pub method: String,
    /// Request headers from the plugin.
    pub headers: Vec<(String, String)>,
    /// Request body.
    pub body: Bytes,
    /// Manifest host allowlist.
    pub allowlist: Vec<String>,
    /// Hosts allowed to receive the plugin's Authorization header.
    pub forward_auth: Vec<String>,
}

/// One egress response.
#[derive(Clone, Debug)]
pub struct EgressResponse {
    /// Upstream status.
    pub status: u16,
    /// Content type, when present.
    pub content_type: Option<String>,
    /// Response body.
    pub body: Bytes,
}

/// Egress failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EgressError {
    /// The target is not in the manifest allowlist.
    #[error("host is not allowlisted")]
    NotAllowed,
    /// The target resolves to a non-public address.
    #[error("target address is not public")]
    BlockedAddress,
    /// The request or response exceeds a limit.
    #[error("egress payload exceeds a limit")]
    TooLarge,
    /// The daily quota is exhausted.
    #[error("egress quota exhausted")]
    Quota,
    /// The URL or method is invalid.
    #[error("invalid egress request: {0}")]
    Invalid(String),
    /// The upstream request failed.
    #[error("egress request failed: {0}")]
    Http(String),
}

#[derive(Clone, Copy, Debug)]
struct Usage {
    day: i64,
    requests: u64,
    bytes: u64,
}

/// Shared egress executor with in-memory daily quotas.
pub struct EgressLimiter {
    usage: Mutex<HashMap<String, Usage>>,
}

impl std::fmt::Debug for EgressLimiter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EgressLimiter")
            .finish_non_exhaustive()
    }
}

impl Default for EgressLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl EgressLimiter {
    /// Creates the executor.
    #[must_use]
    pub fn new() -> Self {
        Self {
            usage: Mutex::new(HashMap::new()),
        }
    }

    /// Executes one allowlisted request.
    ///
    /// # Errors
    ///
    /// Returns allowlist, SSRF, quota, size and transport errors.
    pub async fn execute(&self, request: EgressRequest) -> Result<EgressResponse, EgressError> {
        if request.body.len() > MAX_REQUEST_BYTES {
            return Err(EgressError::TooLarge);
        }
        let url =
            Url::parse(&request.url).map_err(|error| EgressError::Invalid(error.to_string()))?;
        if url.scheme() != "https" {
            return Err(EgressError::Invalid("https is required".to_owned()));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(EgressError::Invalid("userinfo is not allowed".to_owned()));
        }
        if url.port().is_some_and(|port| port != 443) {
            return Err(EgressError::Invalid("only port 443 is allowed".to_owned()));
        }
        let host = url
            .host_str()
            .ok_or_else(|| EgressError::Invalid("host is required".to_owned()))?;
        if host.parse::<IpAddr>().is_ok() {
            return Err(EgressError::Invalid(
                "IP literals are not allowed".to_owned(),
            ));
        }
        if !allows(&request.allowlist, host) {
            return Err(EgressError::NotAllowed);
        }

        let resolved =
            tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, 443)))
                .await
                .map_err(|_| EgressError::Http("DNS resolution timed out".to_owned()))?
                .map_err(|error| EgressError::Http(error.to_string()))?
                .collect::<Vec<_>>();
        if resolved.is_empty() || resolved.iter().any(|addr| !is_public(addr.ip())) {
            return Err(EgressError::BlockedAddress);
        }
        self.reserve(&request.store_id, &request.plugin_id)?;

        // Pin the resolved address so a second DNS answer cannot rebind the
        // connection between validation and connect.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .resolve(host, resolved[0])
            .build()
            .map_err(|error| EgressError::Http(error.to_string()))?;

        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| EgressError::Invalid("unsupported method".to_owned()))?;
        let forward_auth = allows(&request.forward_auth, host);
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in &request.headers {
            let name = name.to_ascii_lowercase();
            let allowed = matches!(name.as_str(), "content-type" | "accept" | "accept-language")
                || (name == "authorization" && forward_auth);
            if !allowed {
                continue;
            }
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                headers.insert(name, value);
            }
        }
        let mut builder = client.request(method, url).headers(headers);
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| EgressError::Http(error.to_string()))?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .bytes()
            .await
            .map_err(|error| EgressError::Http(error.to_string()))?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(EgressError::TooLarge);
        }
        self.record(&request.store_id, &request.plugin_id, body.len() as u64);
        Ok(EgressResponse {
            status,
            content_type,
            body,
        })
    }

    fn reserve(&self, store_id: &str, plugin_id: &str) -> Result<(), EgressError> {
        let day = chrono_day();
        let mut usage = self.usage.lock().expect("egress usage lock poisoned");
        let entry = usage
            .entry(format!("{store_id}|{plugin_id}"))
            .or_insert(Usage {
                day,
                requests: 0,
                bytes: 0,
            });
        if entry.day != day {
            *entry = Usage {
                day,
                requests: 0,
                bytes: 0,
            };
        }
        if entry.requests >= DAILY_REQUESTS || entry.bytes >= DAILY_BYTES {
            return Err(EgressError::Quota);
        }
        entry.requests += 1;
        Ok(())
    }

    fn record(&self, store_id: &str, plugin_id: &str, bytes: u64) {
        let mut usage = self.usage.lock().expect("egress usage lock poisoned");
        if let Some(entry) = usage.get_mut(&format!("{store_id}|{plugin_id}")) {
            entry.bytes = entry.bytes.saturating_add(bytes);
        }
    }
}

fn chrono_day() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64 / 86_400)
}

/// Returns whether `host` is covered by the allowlist.
///
/// Entries are exact hostnames or `*.example.com` wildcards.
#[must_use]
pub fn allows(allowlist: &[String], host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    allowlist.iter().any(|entry| {
        let entry = entry.to_ascii_lowercase();
        if let Some(suffix) = entry.strip_prefix("*.") {
            host.ends_with(&format!(".{suffix}")) && host.len() > suffix.len() + 1
        } else {
            host == entry
        }
    })
}

/// Returns whether an address is globally routable.
#[must_use]
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_unspecified()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 0 && matches!(octets[2], 0 | 2))
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || octets[0] >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    let segments = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

/// Parsed allowlist hostname, exposed for diagnostics.
#[must_use]
pub fn is_hostname(value: &str) -> bool {
    let mut parts = value.split('.');
    parts.all(|part| {
        !part.is_empty()
            && part.len() <= 63
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }) && value.len() <= 253
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_and_exact_allowlist() {
        let list = vec!["api.example.com".to_owned(), "*.openai.com".to_owned()];
        assert!(allows(&list, "api.example.com"));
        assert!(allows(&list, "api.openai.com"));
        assert!(!allows(&list, "example.com"));
        assert!(!allows(&list, "openai.com"));
        assert!(!allows(&list, "evilopenai.com"));
    }

    #[test]
    fn private_and_special_addresses_are_blocked() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip} must be blocked");
        }
        assert!(is_public("1.1.1.1".parse().unwrap()));
        assert!(is_public("2606:4700:4700::1111".parse().unwrap()));
    }
}
