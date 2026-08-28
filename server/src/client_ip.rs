//! Real-client-IP resolution for rate limiting behind proxies.
//!
//! Production sits behind Cloudflare → Traefik, and Traefik runs with
//! forwarded-headers untrusted: it strips/overwrites `X-Forwarded-For`, so
//! the socket peer and XFF both collapse to a proxy IP — every visitor lands
//! in the same rate-limit bucket (seen live as site-wide 429/500s on the
//! join-code endpoints). Cloudflare's `CF-Connecting-IP` is a plain custom
//! header that Traefik passes through untouched, so it is the only value
//! that still identifies the actual client. Order of trust:
//!
//! 1. `CF-Connecting-IP` (set by Cloudflare on every proxied request)
//! 2. First `X-Forwarded-For` entry (dev setups without Cloudflare)
//! 3. Socket peer address
//!
//! Both headers are spoofable by a client that reaches the origin directly;
//! like the rest of the rate limiting this is defense-in-depth against
//! accidental floods, not a security boundary (see the trust model).

use axum::http::HeaderMap;
use std::net::IpAddr;

fn header_ip(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Best-effort client IP as a rate-limit key string.
pub fn client_ip_str(headers: &HeaderMap, peer: Option<IpAddr>) -> String {
    header_ip(headers, "cf-connecting-ip")
        .or_else(|| header_ip(headers, "x-forwarded-for"))
        .unwrap_or_else(|| {
            peer.map(|ip| ip.to_string())
                .unwrap_or_else(|| "127.0.0.1".to_string())
        })
}

/// Best-effort client IP as an address, for limiters keyed by `IpAddr`.
/// Header values that fail to parse fall through to the socket peer.
pub fn client_ip_addr(headers: &HeaderMap, peer: Option<IpAddr>) -> Option<IpAddr> {
    header_ip(headers, "cf-connecting-ip")
        .and_then(|s| s.parse().ok())
        .or_else(|| header_ip(headers, "x-forwarded-for").and_then(|s| s.parse().ok()))
        .or(peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::net::Ipv4Addr;

    fn peer() -> Option<IpAddr> {
        Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
    }

    #[test]
    fn cf_connecting_ip_wins_over_xff_and_peer() {
        let mut h = HeaderMap::new();
        h.insert("cf-connecting-ip", HeaderValue::from_static("203.0.113.7"));
        h.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.9"));
        assert_eq!(client_ip_str(&h, peer()), "203.0.113.7");
        assert_eq!(client_ip_addr(&h, peer()), "203.0.113.7".parse().ok());
    }

    #[test]
    fn falls_back_to_first_xff_entry() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.9, 172.16.0.1"),
        );
        assert_eq!(client_ip_str(&h, peer()), "198.51.100.9");
        assert_eq!(client_ip_addr(&h, peer()), "198.51.100.9".parse().ok());
    }

    #[test]
    fn falls_back_to_peer_then_loopback() {
        let h = HeaderMap::new();
        assert_eq!(client_ip_str(&h, peer()), "10.0.0.1");
        assert_eq!(client_ip_str(&h, None), "127.0.0.1");
        assert_eq!(client_ip_addr(&h, None), None);
    }

    #[test]
    fn unparseable_header_ip_falls_through_to_peer_for_addr() {
        let mut h = HeaderMap::new();
        h.insert("cf-connecting-ip", HeaderValue::from_static("not-an-ip"));
        // The string key keeps the raw value (still a stable bucket key)…
        assert_eq!(client_ip_str(&h, peer()), "not-an-ip");
        // …but the typed variant refuses to fabricate an address.
        assert_eq!(client_ip_addr(&h, peer()), peer());
    }
}
