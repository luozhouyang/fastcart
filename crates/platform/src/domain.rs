//! Store domain registry helpers and hostname validation.

use fastcart_kernel::StoreId;

/// A hostname mapped to a store.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoreDomain {
    /// Domain identifier.
    pub id: String,
    /// Owning store.
    pub store_id: StoreId,
    /// Normalized lowercase hostname.
    pub hostname: String,
    /// Whether this is the store's primary hostname.
    pub is_primary: bool,
    /// Verification timestamp in Unix seconds.
    pub verified_at: Option<i64>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
}

/// Normalizes a hostname for registry lookups.
///
/// Lowercases the host, removes a trailing root dot and strips an optional port.
/// The caller is responsible for rejecting hosts that are not DNS names.
#[must_use]
pub fn normalize_hostname(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('.');
    let without_port = trimmed.rsplit_once(':').map_or(trimmed, |(name, port)| {
        if port.bytes().all(|byte| byte.is_ascii_digit()) {
            name
        } else {
            trimmed
        }
    });
    without_port.to_ascii_lowercase()
}

/// Validates a normalized hostname.
///
/// Single-label hosts such as `localhost` are accepted for local development.
pub fn validate_hostname(hostname: &str) -> Result<(), String> {
    if hostname.is_empty() || hostname.len() > 253 {
        return Err("hostname must be between 1 and 253 characters".to_owned());
    }
    let labels: Vec<&str> = hostname.split('.').collect();
    for label in labels {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("invalid hostname label in {hostname}"));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(format!("invalid hostname label in {hostname}"));
        }
        if !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(format!("invalid hostname: {hostname}"));
        }
    }
    if hostname.contains('/') || hostname.contains(' ') || hostname.contains('@') {
        return Err(format!("invalid hostname: {hostname}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{normalize_hostname, validate_hostname};

    #[test]
    fn normalizes_case_trailing_dot_and_port() {
        assert_eq!(normalize_hostname("Shop.Example.COM."), "shop.example.com");
        assert_eq!(
            normalize_hostname("shop.example.com:8080"),
            "shop.example.com"
        );
        assert_eq!(normalize_hostname(" shop.example.com "), "shop.example.com");
    }

    #[test]
    fn validates_dns_hostnames() {
        assert!(validate_hostname("shop.example.com").is_ok());
        assert!(validate_hostname("localhost").is_ok());
        assert!(validate_hostname("shop.example.com/path").is_err());
        assert!(validate_hostname("-shop.example.com").is_err());
        assert!(validate_hostname("shop..example.com").is_err());
        assert!(validate_hostname("shop example.com").is_err());
    }
}
