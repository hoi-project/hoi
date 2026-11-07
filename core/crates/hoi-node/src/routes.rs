//! Routes as the registry and the carrier see them: `rtc:<fingerprint>@<ip>:<port>` for
//! browsers and phones, `<endpoint key>@<ip>:<port>` or a bare endpoint key for native nodes,
//! and the two address rules that decide which of them may be announced or listed.
//!
//! Two rules, deliberately different:
//! - [`public_ip`]: what this node is willing to announce about itself. Strict: no private,
//!   loopback, link-local, carrier-grade NAT, documentation, benchmarking or reserved ranges.
//! - [`outside_visitor_networks`]: what a registry accepts from a third party's announcement,
//!   so that nobody can make every device reading the directory probe addresses on its own
//!   LAN or host. The unroutable documentation ranges stay allowed, which keeps the shared test
//!   vectors (`203.0.113.0/24`) working on both sides.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use hoi_types::uri::unb64;

/// Community keys and endpoint keys: 64 lowercase hex characters.
pub fn is_hex_key(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub use hoi_net::public_ip;

/// Whether `ip` lies outside every visitor's own network: no loopback, private, link-local,
/// carrier-grade NAT, unspecified or multicast ranges (the registry's rule, the same as
/// `publicIp` in `registry/protocol.mjs`).
pub fn outside_visitor_networks(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 192 && b == 0 && v4.octets()[2] == 0))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return outside_visitor_networks(IpAddr::V4(v4));
            }
            !(v6.is_unspecified() || v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local() || v6.is_multicast())
        }
    }
}

/// Whether a route of this node is worth announcing: its address must be public, unless the
/// registry itself runs on this machine (then every route counts, for tests and development).
pub fn announceable(route: &str, dev: bool) -> bool {
    let Some((_, addr)) = route.split_once('@') else { return false };
    let Ok(addr) = addr.parse::<SocketAddr>() else { return false };
    if addr.port() == 0 || addr.ip().is_unspecified() || addr.ip().is_multicast() {
        return false;
    }
    dev || public_ip(addr.ip())
}

/// Loopback last: a visitor on another machine tries the useful routes first.
pub fn loopback_last(mut routes: Vec<String>) -> Vec<String> {
    routes.sort_by_key(|r| {
        r.split_once('@').and_then(|(_, a)| a.parse::<SocketAddr>().ok()).is_some_and(|a| a.ip().is_loopback() || a.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST))
    });
    routes
}

/// One public infrastructure route as the registry stores and lists it. `public_only` applies
/// the visitor rule (what a third party announces); an operator's own configuration may still
/// name a loopback address for development. Mirrors `validateRoute` in `registry/protocol.mjs`.
pub fn validate_route(route: &str, public_only: bool) -> Result<(), &'static str> {
    const INVALID: &str = "Invalid public node route.";
    if route.len() > 128 {
        return Err(INVALID);
    }
    if is_hex_key(route) {
        return Ok(());
    }
    let rtc = route.starts_with("rtc:");
    let rest = route.strip_prefix("rtc:").unwrap_or(route);
    let Some((name, addr)) = rest.split_once('@') else {
        return Err("Public nodes need an RTC certificate fingerprint or native endpoint key and a socket address.");
    };
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("Public nodes need an RTC certificate fingerprint or native endpoint key and a socket address.");
    }
    let shape_ok = if rtc { name.len() == 43 } else { is_hex_key(name) };
    if !shape_ok {
        return Err("Public nodes need an RTC certificate fingerprint or native endpoint key and a socket address.");
    }
    // `[v6]:port` or `v4:port`, nothing else (no host names, no zone ids, no trailing junk).
    let (ip, port) = if let Some(inner) = addr.strip_prefix('[') {
        let Some((ip, port)) = inner.split_once("]:") else {
            return Err("Public nodes need an RTC certificate fingerprint or native endpoint key and a socket address.");
        };
        if !ip.bytes().all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.') {
            return Err("Public nodes need an RTC certificate fingerprint or native endpoint key and a socket address.");
        }
        (ip.parse::<IpAddr>().ok().filter(|ip| ip.is_ipv6()), port)
    } else {
        let Some((ip, port)) = addr.rsplit_once(':') else {
            return Err("Public nodes need an RTC certificate fingerprint or native endpoint key and a socket address.");
        };
        if !ip.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return Err("Public nodes need an RTC certificate fingerprint or native endpoint key and a socket address.");
        }
        (ip.parse::<IpAddr>().ok().filter(|ip| ip.is_ipv4()), port)
    };
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Public nodes need an RTC certificate fingerprint or native endpoint key and a socket address.");
    }
    let Some(ip) = ip else { return Err(INVALID) };
    let port: u32 = port.parse().map_err(|_| INVALID)?;
    if !(1..=65535).contains(&port) {
        return Err(INVALID);
    }
    if rtc {
        // The fingerprint is canonical base64url of exactly 32 bytes.
        let Ok(bytes) = unb64(name) else { return Err(INVALID) };
        if bytes.len() != 32 || hoi_types::uri::b64(&bytes) != name {
            return Err(INVALID);
        }
    }
    if ip.is_unspecified() || ip.is_multicast() {
        return Err(INVALID);
    }
    if public_only && !outside_visitor_networks(ip) {
        return Err("Announced routes must name a public address.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_are_announced_unless_the_index_runs_here() {
        let fp = "q".repeat(43);
        let key = "a".repeat(64);
        assert!(announceable(&format!("rtc:{fp}@93.184.216.34:7844"), false));
        assert!(announceable(&format!("{key}@[2606:2800:220:1:248:1893:25c8:1946]:7843"), false));
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.4.5",
            "192.168.1.9",
            "169.254.3.3",
            "100.64.0.1",
            "0.0.0.0",
            "203.0.113.10",
            "[::1]",
            "[fe80::1]",
            "[fd12::1]",
            "[::ffff:192.168.0.2]",
        ] {
            assert!(!announceable(&format!("rtc:{fp}@{private}:7844"), false), "{private}");
            assert!(announceable(&format!("rtc:{fp}@{private}:7844"), true) == !matches!(private, "0.0.0.0"), "{private} in development");
        }
        assert!(!announceable(&key, false), "a bare endpoint key reaches nobody without a relay");
        assert!(!announceable(&format!("rtc:{fp}@93.184.216.34:0"), true));
        assert_eq!(
            loopback_last(vec![format!("rtc:{fp}@127.0.0.1:7844"), format!("rtc:{fp}@93.184.216.34:7844")]),
            vec![format!("rtc:{fp}@93.184.216.34:7844"), format!("rtc:{fp}@127.0.0.1:7844")]
        );
    }

    #[test]
    fn the_registry_rule_keeps_visitors_off_their_own_networks_but_allows_documentation_ranges() {
        let ok = |s: &str| outside_visitor_networks(s.parse().unwrap());
        for public in ["93.184.216.34", "203.0.113.10", "198.18.0.1", "2606:2800:220:1:248:1893:25c8:1946", "2001:db8::1", "::ffff:93.184.216.34"] {
            assert!(ok(public), "{public}");
        }
        for private in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "192.0.0.9",
            "100.64.0.1",
            "100.127.255.254",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "ff02::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(!ok(private), "{private}");
        }
        assert!(ok("172.32.0.1") && ok("192.169.0.1") && ok("100.128.0.1"));
    }

    #[test]
    fn route_syntax_matches_the_registry() {
        let fp = hoi_types::uri::b64(&[7u8; 32]);
        let key = "ab".repeat(32);
        for good in [format!("rtc:{fp}@203.0.113.10:7844"), format!("{key}@203.0.113.10:7843"), key.clone(), format!("{key}@[2001:db8::1]:7843")] {
            assert_eq!(validate_route(&good, true), Ok(()), "{good}");
        }
        assert_eq!(validate_route(&format!("rtc:{fp}@127.0.0.1:9000"), false), Ok(()), "an operator may configure loopback");
        assert!(validate_route(&format!("rtc:{fp}@127.0.0.1:9000"), true).is_err());
        for bad in [
            "https://user:secret@example.com".to_string(),
            format!("rtc:{fp}@0.0.0.0:9000"),
            format!("rtc:{fp}@[0:0:0:0:0:0:0:0]:9000"),
            format!("rtc:{fp}@203.0.113.10:65536"),
            format!("rtc:{fp}@203.0.113.10:0"),
            format!("rtc:{}@203.0.113.10:7844", "q".repeat(42)),
            format!("rtc:{}@203.0.113.10:7844", "+".repeat(43)),
            format!("rtc:{fp}@224.0.0.1:7844"),
            format!("rtc:{fp}@[ff02::1]:7844"),
            format!("rtc:{fp}@example.com:7844"),
            format!("rtc:{fp}@203.0.113.10"),
            format!("{}@203.0.113.10:7843", "AB".repeat(32)),
            format!("{key}@2001:db8::1:7843"),
            format!("{key}@[203.0.113.10]:7843"),
            format!("rtc:{fp}@203.0.113.10:7844 "),
            "x".repeat(129),
        ] {
            assert!(validate_route(&bad, false).is_err(), "{bad}");
        }
        // A fingerprint whose last symbol carries stray bits is not canonical base64url: refused
        // whether the decoder rejects it or the round trip does not match.
        let noncanonical = format!("{}B", &fp[..42]);
        assert!(validate_route(&format!("rtc:{noncanonical}@203.0.113.10:7844"), false).is_err());
    }
}
