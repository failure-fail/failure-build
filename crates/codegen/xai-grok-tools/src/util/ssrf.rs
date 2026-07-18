//! Shared SSRF (Server-Side Request Forgery) IP-blocking predicate.
//!
//! Extracted so any tool that makes outbound requests on the user's behalf
//! (`web_fetch`, `browser_navigate`) can apply the same private/link-local/
//! cloud-metadata blocklist rather than each hand-rolling its own — a
//! headless browser reaching an internal admin panel or a cloud metadata
//! endpoint is at least as dangerous as a text-only fetch doing the same.
//!
//! Reference: [IANA IPv4 Special-Purpose Address Registry](https://www.iana.org/assignments/iana-ipv4-special-registry/)

use std::net::IpAddr;

/// Returns `true` if an IP address is in a private, link-local, or cloud
/// metadata range that should be blocked to prevent SSRF attacks.
///
/// **Allowed:** loopback (`127.x` / `::1`) for local development.
/// **Blocked:** RFC 1918, link-local, CGNAT/cloud metadata, unspecified.
pub fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            // Loopback (127.0.0.0/8) — allowed for local dev servers.
            if octets[0] == 127 {
                return false;
            }
            // RFC 1918: 10.0.0.0/8 — private network.
            if octets[0] == 10 {
                return true;
            }
            // RFC 1918: 172.16.0.0/12 — private network.
            if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                return true;
            }
            // RFC 1918: 192.168.0.0/16 — private network.
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }
            // RFC 3927: 169.254.0.0/16 — link-local.
            // Includes AWS/GCP/Azure metadata endpoint 169.254.169.254.
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
            // RFC 6598: 100.64.0.0/10 — CGNAT / shared address space.
            // Used by some cloud providers for internal metadata services.
            if octets[0] == 100 && (64..=127).contains(&octets[1]) {
                return true;
            }
            // 0.0.0.0 — unspecified address.
            if v4.is_unspecified() {
                return true;
            }
            false
        }
        IpAddr::V6(v6) => {
            // ::1 — loopback, allowed for local dev.
            if v6.is_loopback() {
                return false;
            }
            // :: — unspecified.
            if v6.is_unspecified() {
                return true;
            }
            // IPv4-mapped IPv6 (::ffff:x.x.x.x) — delegate to v4 checks.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_ip(&IpAddr::V4(v4));
            }
            let segments = v6.segments();
            // RFC 4291: fe80::/10 — link-local unicast.
            if segments[0] & 0xffc0 == 0xfe80 {
                return true;
            }
            // RFC 4193: fc00::/7 — unique local address (ULA).
            if segments[0] & 0xfe00 == 0xfc00 {
                return true;
            }
            false
        }
    }
}

/// Resolve `host` via DNS and return the first blocked IP found, if any.
/// `None` means every resolved address is safe to connect to.
pub async fn first_blocked_resolved_ip(host: &str, port: u16) -> Result<Option<IpAddr>, std::io::Error> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(is_blocked_ip(&ip).then_some(ip));
    }
    let addrs: Vec<std::net::SocketAddr> =
        tokio::net::lookup_host(format!("{host}:{port}")).await?.collect();
    Ok(addrs
        .iter()
        .map(|addr| addr.ip())
        .find(is_blocked_ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_rfc1918_ranges() {
        assert!(is_blocked_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"192.168.0.1".parse().unwrap()));
        assert!(!is_blocked_ip(&"172.15.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_link_local_and_metadata() {
        assert!(is_blocked_ip(&"169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn allows_loopback_and_public() {
        assert!(!is_blocked_ip(&"127.0.0.1".parse().unwrap()));
        assert!(!is_blocked_ip(&"8.8.8.8".parse().unwrap()));
    }

    #[tokio::test]
    async fn first_blocked_resolved_ip_flags_private_literal() {
        let blocked = first_blocked_resolved_ip("10.0.0.5", 443).await.unwrap();
        assert_eq!(blocked, Some("10.0.0.5".parse().unwrap()));
    }

    #[tokio::test]
    async fn first_blocked_resolved_ip_allows_public_literal() {
        let blocked = first_blocked_resolved_ip("1.1.1.1", 443).await.unwrap();
        assert_eq!(blocked, None);
    }
}
