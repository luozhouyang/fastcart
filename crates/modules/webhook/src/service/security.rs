//! Destination validation, secret handling and signing.

use super::*;

/// Strips query parameters and fragments so audit payloads never persist
/// secrets that callers may have embedded in the endpoint URL.
pub(super) fn sanitize_audit_url(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(mut url) => {
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => raw.to_owned(),
    }
}

pub(super) fn validate_url(raw: &str, allow_private: bool) -> Result<String, AppError> {
    let url = raw.trim();
    if url.is_empty() {
        return Err(AppError::validation("webhook url is required"));
    }
    if url.len() > MAX_URL_LENGTH {
        return Err(AppError::validation(format!(
            "webhook url must not exceed {MAX_URL_LENGTH} bytes"
        )));
    }
    if url
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(AppError::validation(
            "webhook url must not contain whitespace or control characters",
        ));
    }
    let parsed = Url::parse(url)
        .map_err(|error| AppError::validation(format!("invalid webhook url: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::validation(
            "webhook url must use the http or https scheme",
        ));
    }
    let Some(host) = parsed.host_str().filter(|host| !host.is_empty()) else {
        return Err(AppError::validation("webhook url must include a host"));
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::validation(
            "webhook url must not include credentials",
        ));
    }
    if !allow_private {
        if parsed.scheme() != "https" {
            return Err(AppError::validation(
                "webhook url must use https (private destinations are disabled)",
            ));
        }
        validate_public_host(host)?;
    }
    Ok(url.to_owned())
}

pub(super) fn normalize_events(events: Vec<String>) -> Result<Vec<String>, AppError> {
    if events.is_empty() {
        return Err(AppError::validation(
            "at least one webhook event subscription is required",
        ));
    }
    if events.len() > MAX_EVENTS {
        return Err(AppError::validation(format!(
            "a webhook endpoint supports at most {MAX_EVENTS} event subscriptions"
        )));
    }
    let mut normalized = std::collections::BTreeSet::new();
    for event in events {
        let event = event.trim();
        if event == "*" {
            return Ok(vec!["*".to_owned()]);
        }
        let valid = !event.is_empty()
            && event.len() <= MAX_EVENT_LENGTH
            && event
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if !valid {
            return Err(AppError::validation(format!(
                "invalid webhook event type: {event}"
            )));
        }
        normalized.insert(event.to_owned());
    }
    if normalized.is_empty() {
        return Err(AppError::validation(
            "at least one webhook event subscription is required",
        ));
    }
    Ok(normalized.into_iter().collect())
}

pub(super) fn validate_secret(secret: &str) -> Result<(), AppError> {
    let valid = (16..=256).contains(&secret.len())
        && !secret
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace());
    if valid {
        Ok(())
    } else {
        Err(AppError::validation(
            "webhook secret must be 16 to 256 printable characters without whitespace",
        ))
    }
}

pub(super) fn generate_secret() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("whsec_{}", hex::encode(bytes))
}

pub(super) fn sign(secret: &str, timestamp: Timestamp, payload: &str) -> Result<String, AppError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|error| AppError::internal(format!("invalid webhook secret: {error}")))?;
    mac.update(format!("{timestamp}.{payload}").as_bytes());
    Ok(format!("v1={}", hex::encode(mac.finalize().into_bytes())))
}

pub(super) fn truncate_bytes(body: &[u8]) -> String {
    let end = body.len().min(MAX_RESPONSE_BODY_BYTES);
    let end = (0..=end)
        .rev()
        .find(|index| {
            body.get(*index)
                .is_none_or(|byte| *byte & 0b1100_0000 != 0b1000_0000)
        })
        .unwrap_or(0);
    String::from_utf8_lossy(&body[..end]).into_owned()
}

pub(super) async fn read_response_body(response: reqwest::Response) -> String {
    let mut stream = response.bytes_stream();
    let mut body = Vec::with_capacity(MAX_RESPONSE_BODY_BYTES.min(16 * 1024));
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            break;
        };
        let remaining = MAX_RESPONSE_BODY_BYTES.saturating_sub(body.len());
        if remaining == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if body.len() >= MAX_RESPONSE_BODY_BYTES {
            break;
        }
    }
    truncate_bytes(&body)
}

pub(super) fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || octets[0] == 0
                || octets[0] >= 224
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_forbidden_ip(IpAddr::V4(v4)))
        }
    }
}

pub(super) fn validate_public_host(host: &str) -> Result<(), AppError> {
    let normalized = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
        || normalized.ends_with(".internal")
        || normalized.ends_with(".home")
        || normalized.ends_with(".lan")
    {
        return Err(AppError::validation(
            "webhook url must not target a local host",
        ));
    }
    if let Ok(ip) = normalized.parse::<IpAddr>()
        && is_forbidden_ip(ip)
    {
        return Err(AppError::validation(
            "webhook url must not target a private or local address",
        ));
    }
    Ok(())
}

pub(super) async fn ensure_public_resolution(raw: &str) -> Result<(), AppError> {
    let parsed = Url::parse(raw)
        .map_err(|error| AppError::validation(format!("invalid webhook url: {error}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::validation("webhook url must include a host"))?;
    validate_public_host(host)?;
    let port = parsed.port_or_known_default().unwrap_or(443);
    let target = host.trim_matches(['[', ']']).to_owned();
    let addresses = tokio::net::lookup_host((target.as_str(), port))
        .await
        .map_err(|error| {
            AppError::validation(format!("webhook host cannot be resolved: {error}"))
        })?;
    let mut resolved = false;
    for address in addresses {
        resolved = true;
        if is_forbidden_ip(address.ip()) {
            return Err(AppError::validation(
                "webhook host resolves to a private or local address",
            ));
        }
    }
    if !resolved {
        return Err(AppError::validation("webhook host has no addresses"));
    }
    Ok(())
}
