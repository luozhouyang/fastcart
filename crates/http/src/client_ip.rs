//! Trusted client-IP resolution for rate limiting and audit fields.
//!
//! Forwarded headers are only meaningful when the direct peer is a proxy this
//! deployment trusts. Everywhere else the socket peer address wins, so a
//! client cannot rotate `x-forwarded-for` values to bypass a limiter.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, header};
use axum::middleware::Next;
use axum::response::Response;

/// One trusted proxy address or CIDR range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrustedProxy {
    network: IpAddr,
    prefix: u8,
}

impl TrustedProxy {
    /// Parses `192.0.2.10` or `10.0.0.0/8` (IPv6 CIDRs are supported too).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        let (address, prefix) = match value.split_once('/') {
            Some((address, prefix)) => (address.trim(), Some(prefix.trim())),
            None => (value, None),
        };
        let network: IpAddr = address.parse().ok()?;
        let max_prefix = if network.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(prefix) => prefix.parse::<u8>().ok()?,
            None => max_prefix,
        };
        if prefix > max_prefix {
            return None;
        }
        Some(Self { network, prefix })
    }

    /// Returns true when `address` falls inside this range.
    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        let address = normalize(address);
        match (self.network, address) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                let mask = ipv4_mask(self.prefix);
                (u32::from(network) & mask) == (u32::from(address) & mask)
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                let mask = ipv6_mask(self.prefix);
                (u128::from(network) & mask) == (u128::from(address) & mask)
            }
            _ => false,
        }
    }
}

fn ipv4_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    }
}

fn ipv6_mask(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    }
}

/// Collapses IPv4-mapped IPv6 addresses so a v4 proxy CIDR also matches them.
fn normalize(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map_or(IpAddr::V6(address), IpAddr::V4),
        address => address,
    }
}

/// Parsed trusted-proxy configuration.
#[derive(Clone, Debug, Default)]
pub struct ClientIpConfig {
    trusted: Vec<TrustedProxy>,
}

impl ClientIpConfig {
    /// Parses a list of exact addresses or CIDRs. Invalid entries are skipped;
    /// startup validation should reject them before this point.
    #[must_use]
    pub fn parse(values: &[String]) -> Self {
        Self {
            trusted: values
                .iter()
                .filter_map(|value| TrustedProxy::parse(value))
                .collect(),
        }
    }

    /// Returns the raw trusted ranges.
    #[must_use]
    pub fn trusted(&self) -> &[TrustedProxy] {
        &self.trusted
    }

    /// Resolves the client address for a peer and its forwarding headers.
    #[must_use]
    pub fn resolve(&self, peer: Option<IpAddr>, headers: &HeaderMap) -> Option<IpAddr> {
        resolve_client_ip(peer, headers, &self.trusted)
    }
}

/// The client address selected for this request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

impl ClientIp {
    /// Renders the address for limiter keys and audit payloads.
    #[must_use]
    pub fn as_key(self) -> String {
        self.0.to_string()
    }
}

/// Resolves the effective client address.
///
/// An untrusted peer is authoritative and forwarding headers are ignored. For
/// a trusted peer the `x-forwarded-for` chain is walked right to left, skipping
/// trusted proxies; the first untrusted hop is the client. If the chain is
/// absent or entirely trusted, `x-real-ip` then the peer are used.
#[must_use]
pub fn resolve_client_ip(
    peer: Option<IpAddr>,
    headers: &HeaderMap,
    trusted: &[TrustedProxy],
) -> Option<IpAddr> {
    let Some(peer) = peer.map(normalize) else {
        // Direct router tests have no socket address. Do not trust forwarded
        // headers in that case; callers treat the missing address as unknown.
        return None;
    };
    let is_trusted = |address: IpAddr| trusted.iter().any(|proxy| proxy.contains(address));
    if !is_trusted(peer) {
        return Some(peer);
    }

    let mut hops = forwarded_for(headers);
    hops.push(peer);
    for hop in hops.iter().rev() {
        if !is_trusted(*hop) {
            return Some(*hop);
        }
    }

    if let Some(real_ip) = header_ip(headers, "x-real-ip") {
        return Some(real_ip);
    }
    Some(peer)
}

fn forwarded_for(headers: &HeaderMap) -> Vec<IpAddr> {
    headers
        .get_all(header::FORWARDED)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|part| part.trim().strip_prefix("for="))
        .filter_map(parse_forwarded_address)
        .collect::<Vec<_>>()
        .into_iter()
        .chain(
            headers
                .get_all("x-forwarded-for")
                .iter()
                .filter_map(|value| value.to_str().ok())
                .flat_map(|value| value.split(','))
                .filter_map(parse_ip),
        )
        .collect()
}

fn header_ip(headers: &HeaderMap, name: &str) -> Option<IpAddr> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_ip)
}

fn parse_ip(value: &str) -> Option<IpAddr> {
    value.trim().trim_matches('"').parse::<IpAddr>().ok()
}

fn parse_forwarded_address(value: &str) -> Option<IpAddr> {
    let value = value.trim().trim_matches('"');
    // `for="[2001:db8::1]:443"` and `for=192.0.2.1:1234`.
    if let Some(rest) = value.strip_prefix('[') {
        return rest.split(']').next().and_then(parse_ip);
    }
    value
        .rsplit_once(':')
        .filter(|(_, port)| port.parse::<u16>().is_ok())
        .map_or_else(|| parse_ip(value), |(address, _)| parse_ip(address))
}

/// Inserts [`ClientIp`] into request extensions when the peer is known.
pub async fn client_ip_middleware(
    State(config): State<ClientIpConfig>,
    mut request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip());
    if let Some(address) = config.resolve(peer, request.headers()) {
        request.extensions_mut().insert(ClientIp(address));
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use axum::http::{HeaderName, HeaderValue};

    fn headers(values: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in values {
            headers.append(
                HeaderName::from_str(name).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    }

    fn trusted(values: &[&str]) -> Vec<TrustedProxy> {
        values
            .iter()
            .map(|value| TrustedProxy::parse(value).unwrap())
            .collect()
    }

    #[test]
    fn parses_exact_addresses_and_cidrs() {
        assert!(TrustedProxy::parse("127.0.0.1").is_some());
        assert!(TrustedProxy::parse("10.0.0.0/8").is_some());
        assert!(TrustedProxy::parse("2001:db8::/32").is_some());
        assert!(TrustedProxy::parse("10.0.0.0/33").is_none());
        assert!(TrustedProxy::parse("not-an-ip").is_none());
    }

    #[test]
    fn cidr_membership_is_correct() {
        let proxy = TrustedProxy::parse("10.0.0.0/8").unwrap();
        assert!(proxy.contains("10.1.2.3".parse().unwrap()));
        assert!(!proxy.contains("11.1.2.3".parse().unwrap()));
        let v6 = TrustedProxy::parse("2001:db8::/32").unwrap();
        assert!(v6.contains("2001:db8::1".parse().unwrap()));
        assert!(!v6.contains("2001:db9::1".parse().unwrap()));
    }

    #[test]
    fn untrusted_peer_ignores_forwarded_headers() {
        let resolved = resolve_client_ip(
            Some("198.51.100.7".parse().unwrap()),
            &headers(&[("x-forwarded-for", "203.0.113.9")]),
            &trusted(&["10.0.0.0/8"]),
        );
        assert_eq!(resolved, Some("198.51.100.7".parse().unwrap()));
    }

    #[test]
    fn trusted_peer_uses_the_rightmost_untrusted_hop() {
        let resolved = resolve_client_ip(
            Some("10.0.0.2".parse().unwrap()),
            &headers(&[("x-forwarded-for", "203.0.113.9, 10.0.0.3")]),
            &trusted(&["10.0.0.0/8"]),
        );
        assert_eq!(resolved, Some("203.0.113.9".parse().unwrap()));
    }

    #[test]
    fn trusted_peer_without_forwarding_headers_uses_peer() {
        let resolved = resolve_client_ip(
            Some("10.0.0.2".parse().unwrap()),
            &HeaderMap::new(),
            &trusted(&["10.0.0.0/8"]),
        );
        assert_eq!(resolved, Some("10.0.0.2".parse().unwrap()));
    }

    #[test]
    fn malformed_forwarded_headers_fall_back_to_peer() {
        let resolved = resolve_client_ip(
            Some("10.0.0.2".parse().unwrap()),
            &headers(&[("x-forwarded-for", "unknown, also-bad")]),
            &trusted(&["10.0.0.0/8"]),
        );
        assert_eq!(resolved, Some("10.0.0.2".parse().unwrap()));
    }

    #[test]
    fn no_peer_returns_none_instead_of_trusting_headers() {
        let resolved = resolve_client_ip(
            None,
            &headers(&[("x-forwarded-for", "203.0.113.9")]),
            &trusted(&["10.0.0.0/8"]),
        );
        assert_eq!(resolved, None);
    }
}
