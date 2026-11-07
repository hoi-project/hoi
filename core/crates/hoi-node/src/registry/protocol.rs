//! What a registry accepts and how it answers: canonical names, strong keys, the three signed
//! statements (a name claim, a version-bound publication, a node announcement), route syntax
//! and the order public nodes are listed in. Line for line the rules of `registry/protocol.mjs`,
//! so a namespace served by `hoi-node --registry` and one served by the Node service or the
//! Worker accept and refuse the same requests.

use std::collections::BTreeSet;

use hoi_types::{Id, Sig, hex_decode};
use serde_json::{Map, Value, json};

use crate::routes::{is_hex_key, validate_route};

/// Routes a discovery response may carry; the browser client refuses longer lists.
pub const MAX_PUBLIC_NODES: usize = 16;
/// A node's announcement stays in discovery this long after `issuedAt`; nodes re-announce every five minutes.
pub const NODE_TTL_SECONDS: u64 = 30 * 60;
/// An announcement's `issuedAt` may differ from the registry clock by at most this much.
pub const ANNOUNCEMENT_SKEW_SECONDS: u64 = 10 * 60;
/// Live announcements retained; older ones are evicted first.
pub const MAX_ANNOUNCED_NODES: usize = 64;
pub const MAX_ANNOUNCEMENT_ROUTES: usize = 8;
pub const MAX_ANNOUNCEMENT_COMMUNITIES: usize = 64;

/// A public name as the registry stores it, or `None`.
pub fn normalize_name(value: &str) -> Option<String> {
    hoi_core::community_registration_name(value)
}

/// A community key as the registry prints it, or `None`.
pub fn normalize_community_key(value: &str) -> Option<String> {
    is_hex_key(value).then(|| value.to_string())
}

fn is_hex_signature(value: &str) -> bool {
    value.len() == 128 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_protocol(value: &str) -> bool {
    let ok = |b: u8| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-';
    !value.is_empty() && value.len() <= 64 && value.as_bytes()[0].is_ascii_alphanumeric() && value.bytes().all(ok)
}

/// A valid, canonical Ed25519 point outside the small-order subgroup: a key here would verify
/// forged signatures. Lowercase hex only, like everything else the registry stores.
pub fn key_is_strong(hex: &str) -> bool {
    if !is_hex_key(hex) {
        return false;
    }
    let Ok(bytes) = hex_decode(hex) else { return false };
    let Ok(id) = Id::from_slice(&bytes) else { return false };
    // Non-canonical encodings (y >= p, p = 2^255 - 19) are refused as well, sign bit ignored.
    let mut masked = id.0;
    masked[31] &= 0x7f;
    if masked[31] == 0x7f && masked[1..31].iter().all(|b| *b == 0xff) && masked[0] >= 0xed {
        return false;
    }
    hoi_crypto::key_is_strong(&id)
}

fn verify(key: &str, statement: &str, signature: &str) -> bool {
    let (Ok(pk), Ok(sig)) = (Id::from_hex(key), hex_decode(signature)) else { return false };
    let Ok(sig) = Sig::from_slice(&sig) else { return false };
    hoi_crypto::verify(&pk, statement.as_bytes(), &sig)
}

/// `Number.isInteger` on a JSON number: an integral value within the given range.
fn integer_in(value: Option<&Value>, min: i64, max: i64) -> Option<i64> {
    let n = value?;
    let n = n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0 && f.is_finite()).map(|f| f as i64))?;
    (min..=max).contains(&n).then_some(n)
}

fn exact_keys(object: &Map<String, Value>, expected: &str) -> bool {
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys.join(",") == expected
}

/// A verified name claim (`version` 1) or version-bound publication (`version` 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub name: String,
    pub community: String,
    pub signature: String,
    pub protocol_version: Option<String>,
    pub index_version: Option<u32>,
}

/// Checks a claim body as the API received it: exactly the expected fields, a canonical name,
/// a strong key, and the community key's signature over the exact statement.
pub fn verified_claim(value: &Value, version: u8) -> Option<Claim> {
    let object = value.as_object()?;
    let expected = if version == 2 { "community,indexVersion,name,protocolVersion,signature" } else { "community,name,signature" };
    if !exact_keys(object, expected) {
        return None;
    }
    let name = normalize_name(object.get("name")?.as_str()?)?;
    let community = object.get("community")?.as_str()?;
    let signature = object.get("signature")?.as_str()?;
    if !key_is_strong(community) || !is_hex_signature(signature) {
        return None;
    }
    let key = Id::from_hex(community).ok()?;
    let (statement, protocol_version, index_version) = if version == 2 {
        let protocol = object.get("protocolVersion")?.as_str()?;
        if !is_protocol(protocol) {
            return None;
        }
        let index = integer_in(object.get("indexVersion"), 1, i32::MAX as i64)? as u32;
        (hoi_core::community_publication_statement(&name, &key, protocol, index), Some(protocol.to_string()), Some(index))
    } else {
        (hoi_core::community_registration_statement(&name, &key), None, None)
    };
    verify(community, &statement, signature).then(|| Claim {
        name,
        community: community.to_string(),
        signature: signature.to_string(),
        protocol_version,
        index_version,
    })
}

/// A node announcing its own public routes, signed by its transport (endpoint) key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    pub endpoint: String,
    pub routes: Vec<String>,
    pub communities: Vec<String>,
    pub issued_at: u64,
}

impl Announcement {
    pub fn to_json(&self) -> Value {
        json!({ "endpoint": self.endpoint, "routes": self.routes, "communities": self.communities, "issuedAt": self.issued_at })
    }
}

/// Checks an announcement body. `now` is the registry clock in Unix seconds; stale and
/// future-dated announcements are refused, so a captured announcement cannot be replayed later,
/// and one signed for another registry (`registry` not among `registries`, the origins this one
/// answers to) cannot be replayed here at all. The version 1 statement named no registry and is
/// refused.
pub fn verified_announcement(value: &Value, now: u64, registries: &[String]) -> Option<Announcement> {
    let object = value.as_object()?;
    if !exact_keys(object, "communities,endpoint,issuedAt,registry,routes,signature") {
        return None;
    }
    let registry = object.get("registry")?.as_str()?;
    if !registries.iter().any(|r| r == registry) {
        return None;
    }
    let endpoint = object.get("endpoint")?.as_str()?;
    let signature = object.get("signature")?.as_str()?;
    if !key_is_strong(endpoint) || !is_hex_signature(signature) {
        return None;
    }
    let issued_at = integer_in(object.get("issuedAt"), 0, 9_007_199_254_740_991)? as u64;
    if issued_at.abs_diff(now) > ANNOUNCEMENT_SKEW_SECONDS {
        return None;
    }
    let routes: Vec<String> = object.get("routes")?.as_array()?.iter().map(|r| r.as_str().map(String::from)).collect::<Option<_>>()?;
    if routes.is_empty() || routes.len() > MAX_ANNOUNCEMENT_ROUTES || routes.iter().collect::<BTreeSet<_>>().len() != routes.len() {
        return None;
    }
    let communities: Vec<String> = object.get("communities")?.as_array()?.iter().map(|c| c.as_str().map(String::from)).collect::<Option<_>>()?;
    if communities.len() > MAX_ANNOUNCEMENT_COMMUNITIES || communities.iter().collect::<BTreeSet<_>>().len() != communities.len() {
        return None;
    }
    for route in &routes {
        validate_route(route, true).ok()?;
        // A bare key or native route names this node's own endpoint; it cannot point at someone else's.
        if !route.starts_with("rtc:") && route.split('@').next() != Some(endpoint) {
            return None;
        }
    }
    for (i, community) in communities.iter().enumerate() {
        if normalize_community_key(community).is_none() || (i > 0 && community.as_str() <= communities[i - 1].as_str()) {
            return None;
        }
    }
    let key = Id::from_hex(endpoint).ok()?;
    let ids: Vec<Id> = communities.iter().map(|c| Id::from_hex(c)).collect::<Result<_, _>>().ok()?;
    let statement = hoi_core::node_announcement_statement(registry, &key, issued_at, &routes, &ids);
    verify(endpoint, &statement, signature).then(|| Announcement { endpoint: endpoint.to_string(), routes, communities, issued_at })
}

/// A refusal: status, error code and message, worded as the Node service words them.
pub type Refusal = (u16, &'static str, &'static str);

/// What a registry answers to `PUT /v2/nodes` before it touches its database: the announcement
/// to store, or a refusal. `registries` are the origins it answers to (none: it does not know
/// its public URL and takes no announcements); `announcers` the endpoint keys its operator
/// allows, or `None` for any. `admitAnnouncement` in `registry/protocol.mjs`, line for line.
pub fn admit_announcement(value: &Value, now: u64, registries: &[String], announcers: Option<&BTreeSet<String>>) -> Result<Announcement, Refusal> {
    if registries.is_empty() {
        return Err((
            403,
            "announcements-disabled",
            "This registry does not know its own public URL, so it cannot tell which registry an announcement was signed for. Its operator must configure that URL.",
        ));
    }
    if let Some(registry) = value.get("registry").and_then(Value::as_str)
        && !registries.iter().any(|r| r == registry)
    {
        return Err((
            400,
            "wrong-registry",
            "This announcement was signed for another registry. A node signs a separate announcement for each registry it is given.",
        ));
    }
    let Some(announcement) = verified_announcement(value, now, registries) else {
        return Err((
            400,
            "invalid-announcement",
            "The node announcement could not be verified: its transport key must sign this registry's origin, its current routes and community keys, dated within ten minutes.",
        ));
    };
    if announcers.is_some_and(|allowed| !allowed.contains(&announcement.endpoint)) {
        return Err((403, "announcer-not-allowed", "This registry lists only the public nodes its operator allowed."));
    }
    Ok(announcement)
}

/// Refusal of an announcement from a node the operator dropped (`hoi-node admin registry drop-node`).
pub const ANNOUNCER_REFUSED: Refusal = (403, "announcer-refused", "This registry's operator does not list this node.");
/// A listing the operator withholds: still reserved, never served.
pub const WITHHELD: (&str, &str) = ("withheld", "This registry withholds this public name: it stays reserved and is not listed.");

/// The origin a registry answers to, from its public URL as carriers are given it with `--index`
/// (path and all): HTTPS, or plain HTTP on this machine, nothing secret in it.
pub fn registry_origin(value: &str) -> Result<String, String> {
    let error =
        || "The registry URL must be its public https:// address (http:// only on this machine), without credentials, a query or a fragment.".to_string();
    let url = url::Url::parse(value).map_err(|_| error())?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(error());
    }
    let origin = url.origin().ascii_serialization();
    if hoi_core::registry_origin_ok(&origin) { Ok(origin) } else { Err(error()) }
}

/// Without a configured URL a registry answers only to the loopback address a connection reached
/// it at, for development and tests: `http://127.0.0.1:<port>` or `http://[::1]:<port>`, and
/// `http://localhost:<port>`. The address comes from the socket, never from a request header.
pub fn loopback_origins(local: std::net::SocketAddr) -> Vec<String> {
    let port = if local.port() == 80 { String::new() } else { format!(":{}", local.port()) };
    let ip = match local.ip() {
        std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().map(std::net::IpAddr::V4).unwrap_or(std::net::IpAddr::V6(v6)),
        ip => ip,
    };
    match ip {
        std::net::IpAddr::V4(v4) if v4 == std::net::Ipv4Addr::LOCALHOST => vec![format!("http://127.0.0.1{port}"), format!("http://localhost{port}")],
        std::net::IpAddr::V6(v6) if v6.is_loopback() => vec![format!("http://[::1]{port}"), format!("http://localhost{port}")],
        _ => Vec::new(),
    }
}

/// The endpoint keys an operator allows to announce (allowlist mode), or `None` for any.
pub fn validate_announcers(keys: &[String]) -> Result<Option<BTreeSet<String>>, String> {
    if keys.len() > 1024 || !keys.iter().all(|k| key_is_strong(k)) {
        return Err("Allowed announcers must be 64-character lowercase hex endpoint keys, at most 1024.".into());
    }
    Ok((!keys.is_empty()).then(|| keys.iter().cloned().collect()))
}

/// The operator's configured routes: at most 16, each a valid route, duplicates dropped.
pub fn validate_public_nodes(nodes: &[String]) -> Result<Vec<String>, String> {
    if nodes.len() > MAX_PUBLIC_NODES {
        return Err(format!("Public nodes must be an array of at most {MAX_PUBLIC_NODES} peer routes."));
    }
    let mut out = Vec::new();
    for route in nodes {
        validate_route(route, false).map_err(String::from)?;
        if !out.contains(route) {
            out.push(route.clone());
        }
    }
    Ok(out)
}

/// The app address a registry points people at: HTTPS (or loopback HTTP), nothing secret in it.
pub fn validate_app_url(value: &str) -> Result<String, String> {
    let error = || "The app URL must be HTTPS (or loopback HTTP), without credentials, a query or a fragment.".to_string();
    let url = url::Url::parse(value).map_err(|_| error())?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && loopback);
    if !scheme_ok || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(error());
    }
    Ok(url.to_string())
}

/// One allowed browser origin: an exact http(s) origin, or the desktop's `tauri://localhost`.
/// The opaque origin `null` is never allowed.
pub fn validate_origin(origin: &str) -> Result<String, String> {
    if origin == "tauri://localhost" {
        return Ok(origin.to_string());
    }
    let error = || "Each allowed origin must be an exact http(s) origin or tauri://localhost, without a path or trailing slash.".to_string();
    let url = url::Url::parse(origin).map_err(|_| error())?;
    if !matches!(url.scheme(), "http" | "https") || url.origin().ascii_serialization() != origin {
        return Err(error());
    }
    Ok(origin.to_string())
}

/// A node as discovery holds it: its routes and the communities it says it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveNode {
    pub endpoint: String,
    pub routes: Vec<String>,
    pub communities: Vec<String>,
    pub issued_at: u64,
}

/// Routes of `nodes` taken one per node per round, so no single announcement fills the list.
fn round_robin(nodes: &[&LiveNode]) -> Vec<String> {
    let mut out = Vec::new();
    let mut next = vec![0usize; nodes.len()];
    loop {
        let mut any = false;
        for (i, node) in nodes.iter().enumerate() {
            if let Some(route) = node.routes.get(next[i]) {
                out.push(route.clone());
                next[i] += 1;
                any = true;
            }
        }
        if !any {
            return out;
        }
    }
}

/// Discovery routes in the order a client should try them: the operator's configured routes
/// always first (nobody can announce them out of the list), then nodes that announced carrying
/// `community`, then the other live nodes, each group taken one route per node per round so a
/// node announcing eight routes cannot crowd out the rest.
pub fn select_public_nodes(configured: &[String], live: &[LiveNode], community: Option<&str>) -> Vec<String> {
    let carrying: Vec<&LiveNode> = match community {
        Some(c) => live.iter().filter(|n| n.communities.iter().any(|x| x == c)).collect(),
        None => Vec::new(),
    };
    let others: Vec<&LiveNode> = live.iter().filter(|n| !carrying.iter().any(|c| std::ptr::eq(*c, *n))).collect();
    let mut out: Vec<String> = Vec::new();
    for route in configured.iter().cloned().chain(round_robin(&carrying)).chain(round_robin(&others)) {
        if !out.contains(&route) {
            out.push(route);
        }
        if out.len() == MAX_PUBLIC_NODES {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_crypto::Keypair;

    fn pair(seed: u8) -> (Keypair, String) {
        let kp = Keypair::from_secret(&[seed; 32]);
        let hex = kp.public().hex();
        (kp, hex)
    }

    fn signed(kp: &Keypair, statement: &str) -> String {
        hoi_types::hex_encode(&kp.sign(statement.as_bytes()).0)
    }

    fn claim(name: &str, seed: u8) -> Value {
        let (kp, community) = pair(seed);
        let canonical = normalize_name(name).unwrap();
        let statement = hoi_core::community_registration_statement(&canonical, &kp.public());
        json!({ "name": canonical, "community": community, "signature": signed(&kp, &statement) })
    }

    fn publication(name: &str, seed: u8, protocol: &str, index: u32) -> Value {
        let (kp, community) = pair(seed);
        let canonical = normalize_name(name).unwrap();
        let statement = hoi_core::community_publication_statement(&canonical, &kp.public(), protocol, index);
        json!({ "name": canonical, "community": community, "protocolVersion": protocol, "indexVersion": index, "signature": signed(&kp, &statement) })
    }

    #[test]
    fn the_shared_vectors_are_accepted_without_translation() {
        let registration: Value = serde_json::from_str(include_str!("../../../../vectors/community-registration.json")).unwrap();
        let claim = verified_claim(&json!({ "name": registration["name"], "community": registration["community"], "signature": registration["signature"] }), 1)
            .unwrap();
        assert_eq!(claim.name, "garden_1.v2-test");
        assert!(verified_claim(&json!({ "name": "modified", "community": registration["community"], "signature": registration["signature"] }), 1).is_none());
        let publication: Value = serde_json::from_str(include_str!("../../../../vectors/community-publication.json")).unwrap();
        let body = json!({
            "name": publication["name"], "community": publication["community"], "protocolVersion": publication["protocolVersion"],
            "indexVersion": publication["indexVersion"], "signature": publication["signature"],
        });
        let claim = verified_claim(&body, 2).unwrap();
        assert_eq!((claim.protocol_version.as_deref(), claim.index_version), (Some("0.3"), Some(1)));
        assert!(verified_claim(&body, 1).is_none(), "a v2 body is not a v1 claim");
        let announcement: Value = serde_json::from_str(include_str!("../../../../vectors/node-announcement.json")).unwrap();
        let body = json!({
            "registry": announcement["registry"], "endpoint": announcement["endpoint"], "routes": announcement["routes"],
            "communities": announcement["communities"], "issuedAt": announcement["issuedAt"], "signature": announcement["signature"],
        });
        let issued_at = announcement["issuedAt"].as_u64().unwrap();
        let here = vec![registry_origin("https://registry.example/registry").unwrap()];
        let verified = verified_announcement(&body, issued_at, &here).unwrap();
        assert_eq!(verified.routes.len(), 2);
        assert_eq!(verified.communities.len(), 2);
        assert!(verified_announcement(&body, issued_at + 601, &here).is_none(), "stale");
        assert!(verified_announcement(&body, issued_at - 601, &here).is_none(), "future-dated");
        let elsewhere = vec!["https://other-registry.example".to_string()];
        assert!(verified_announcement(&body, issued_at, &elsewhere).is_none(), "signed for another registry");
        assert_eq!(admit_announcement(&body, issued_at, &elsewhere, None).unwrap_err().1, "wrong-registry");
    }

    #[test]
    fn a_registry_answers_to_its_configured_origin_or_to_this_machine_only() {
        assert_eq!(registry_origin("https://hoi.sh/registry/").unwrap(), "https://hoi.sh");
        assert_eq!(registry_origin("https://Registry.Example:443").unwrap(), "https://registry.example");
        assert_eq!(registry_origin("http://127.0.0.1:7845").unwrap(), "http://127.0.0.1:7845");
        for bad in ["http://registry.example", "https://user:pw@registry.example", "https://registry.example/?x=1", "registry.example", "ftp://hoi.sh"] {
            assert!(registry_origin(bad).is_err(), "{bad}");
        }
        assert_eq!(loopback_origins("127.0.0.1:7845".parse().unwrap()), ["http://127.0.0.1:7845", "http://localhost:7845"]);
        assert_eq!(loopback_origins("[::1]:7845".parse().unwrap()), ["http://[::1]:7845", "http://localhost:7845"]);
        assert_eq!(loopback_origins("[::ffff:127.0.0.1]:7845".parse().unwrap()), ["http://127.0.0.1:7845", "http://localhost:7845"]);
        assert!(loopback_origins("203.0.113.4:7845".parse().unwrap()).is_empty(), "reached from outside without a configured URL, it takes no announcements");
        assert!(loopback_origins("0.0.0.0:7845".parse().unwrap()).is_empty());
        let (_, endpoint) = pair(4);
        assert_eq!(validate_announcers(std::slice::from_ref(&endpoint)).unwrap().unwrap().len(), 1);
        assert!(validate_announcers(&[]).unwrap().is_none(), "no list: any node may announce");
        assert!(validate_announcers(&["not a key".into()]).is_err());
        assert!(validate_announcers(&[endpoint.to_uppercase()]).is_err());
    }

    #[test]
    fn an_announcement_is_admitted_only_for_this_registry_and_allowed_keys() {
        let now = 1_000_000u64;
        let here = vec!["https://registry.example".to_string()];
        let (valid, endpoint) = announcement(4, None, vec![], now);
        assert_eq!(admit_announcement(&valid, now, &here, None).unwrap().endpoint, endpoint);
        let unconfigured = admit_announcement(&valid, now, &[], None).unwrap_err();
        assert_eq!((unconfigured.0, unconfigured.1), (403, "announcements-disabled"), "a registry that does not know its URL takes none");
        let allowed: BTreeSet<String> = [endpoint.clone()].into();
        assert!(admit_announcement(&valid, now, &here, Some(&allowed)).is_ok());
        let (stranger, _) = announcement(5, None, vec![], now);
        assert_eq!(admit_announcement(&stranger, now, &here, Some(&allowed)).unwrap_err().0, 403);
        assert_eq!(admit_announcement(&stranger, now, &here, Some(&allowed)).unwrap_err().1, "announcer-not-allowed");
        let mut unnamed = valid.clone();
        unnamed.as_object_mut().unwrap().remove("registry");
        assert_eq!(admit_announcement(&unnamed, now, &here, None).unwrap_err().1, "invalid-announcement", "version 1 bodies name no registry");
        let mut moved = valid.clone();
        moved["registry"] = json!("https://other-registry.example");
        assert_eq!(admit_announcement(&moved, now, &here, None).unwrap_err().1, "wrong-registry");
        let mut restated = valid.clone();
        restated["registry"] = json!("https://registry.example:8443");
        assert_eq!(
            admit_announcement(&restated, now, &["https://registry.example:8443".to_string()], None).unwrap_err().1,
            "invalid-announcement",
            "the registry is signed, not just stated"
        );
    }

    #[test]
    fn a_claim_is_bound_to_its_exact_name_key_and_fields() {
        let valid = claim(" PRIVACY ", 1);
        let got = verified_claim(&valid, 1).unwrap();
        assert_eq!(got.name, "privacy");
        let mut changed = valid.clone();
        changed["name"] = json!("changed");
        assert!(verified_claim(&changed, 1).is_none());
        let mut other = valid.clone();
        other["community"] = json!(pair(2).1);
        assert!(verified_claim(&other, 1).is_none());
        let mut zero = valid.clone();
        zero["signature"] = json!("0".repeat(128));
        assert!(verified_claim(&zero, 1).is_none());
        let mut short = valid.clone();
        short["signature"] = json!(&valid["signature"].as_str().unwrap()[2..]);
        assert!(verified_claim(&short, 1).is_none());
        let mut extra = valid.clone();
        extra["owner"] = json!("unexpected private data");
        assert!(verified_claim(&extra, 1).is_none());
        let mut homoglyph = valid.clone();
        homoglyph["name"] = json!("prıvacy");
        assert!(verified_claim(&homoglyph, 1).is_none());
        let mut kelvin = valid.clone();
        kelvin["name"] = json!("K");
        assert!(verified_claim(&kelvin, 1).is_none());
        let mut weak = valid.clone();
        weak["community"] = json!(format!("01{}", "00".repeat(31)));
        weak["signature"] = json!(format!("01{}", "00".repeat(63)));
        assert!(verified_claim(&weak, 1).is_none());
        assert!(verified_claim(&json!([1]), 1).is_none());
        assert!(verified_claim(&json!(null), 1).is_none());
    }

    #[test]
    fn a_publication_binds_protocol_and_index_and_tolerates_json_integers_only() {
        let valid = publication("garden", 3, "0.3", 1);
        assert!(verified_claim(&valid, 2).is_some());
        for (field, value) in [
            ("protocolVersion", json!("0.4")),
            ("indexVersion", json!(2)),
            ("indexVersion", json!(0)),
            ("indexVersion", json!(1.5)),
            ("indexVersion", json!("1")),
            ("name", json!("elsewhere")),
            ("protocolVersion", json!("0.3\ninjected")),
            ("protocolVersion", json!("")),
        ] {
            let mut bad = valid.clone();
            bad[field] = value;
            assert!(verified_claim(&bad, 2).is_none(), "{field}");
        }
        let mut described = valid.clone();
        described["description"] = json!("Do not store this");
        assert!(verified_claim(&described, 2).is_none());
        let mut float = valid.clone();
        float["indexVersion"] = json!(1.0);
        assert!(verified_claim(&float, 2).is_some(), "JSON.parse makes 1.0 an integer, so does this");
        let mut huge = publication("garden", 3, "0.3", 1);
        huge["indexVersion"] = json!(2_147_483_648u64);
        assert!(verified_claim(&huge, 2).is_none());
    }

    #[test]
    fn weak_and_noncanonical_keys_are_refused() {
        assert!(key_is_strong(&pair(9).1));
        for weak in [
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0100000000000000000000000000000000000000000000000000000000000000",
            "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
            "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a",
            "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
            "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
            "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
            "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ] {
            assert!(!key_is_strong(weak), "{weak}");
        }
        assert!(!key_is_strong(&pair(9).1.to_uppercase()), "uppercase hex is not how keys are written");
        assert!(!key_is_strong(&pair(9).1[..63]));
    }

    fn announcement(seed: u8, routes: Option<Vec<String>>, communities: Vec<String>, issued_at: u64) -> (Value, String) {
        let (kp, endpoint) = pair(seed);
        let routes =
            routes.unwrap_or_else(|| vec![format!("rtc:{}@203.0.113.7:7844", hoi_types::uri::b64(&[7u8; 32])), format!("{endpoint}@203.0.113.7:7843")]);
        let ids: Vec<Id> = communities.iter().map(|c| Id::from_hex(c).unwrap()).collect();
        let registry = "https://registry.example";
        let statement = hoi_core::node_announcement_statement(registry, &kp.public(), issued_at, &routes, &ids);
        let signature = signed(&kp, &statement);
        let body =
            json!({ "registry": registry, "endpoint": endpoint, "routes": routes, "communities": communities, "issuedAt": issued_at, "signature": signature });
        (body, endpoint)
    }

    fn here() -> Vec<String> {
        vec!["https://registry.example".to_string()]
    }

    #[test]
    fn an_announcement_is_bound_to_its_endpoint_dated_bounded_and_public() {
        let now = 1_000_000u64;
        let garden = "a".repeat(64);
        let (valid, endpoint) = announcement(4, None, vec![garden.clone()], now);
        let got = verified_announcement(&valid, now, &here()).unwrap();
        assert_eq!(got.endpoint, endpoint);
        assert_eq!(got.communities, vec![garden.clone()]);
        let mut emptied = valid.clone();
        emptied["communities"] = json!([]);
        assert!(verified_announcement(&emptied, now, &here()).is_none(), "the signature covers the community list");
        assert!(verified_announcement(&valid, now + 601, &here()).is_none());
        assert!(verified_announcement(&valid, now - 601, &here()).is_none());
        assert!(verified_announcement(&valid, now + 600, &here()).is_some());
        let (other, _) = announcement(5, Some(vec![format!("{endpoint}@203.0.113.7:7843")]), vec![], now);
        assert!(verified_announcement(&other, now, &here()).is_none(), "a native route must name the signing endpoint");
        let (private, _) = announcement(4, Some(vec![format!("{endpoint}@192.168.1.4:7843")]), vec![], now);
        assert!(verified_announcement(&private, now, &here()).is_none());
        let (unsorted, _) = announcement(4, None, vec!["b".repeat(64), "a".repeat(64)], now);
        assert!(verified_announcement(&unsorted, now, &here()).is_none());
        let (duplicate, _) = announcement(4, None, vec![garden.clone(), garden.clone()], now);
        assert!(verified_announcement(&duplicate, now, &here()).is_none());
        let (many, _) = announcement(4, None, (0..65u8).map(|i| format!("{i:064x}")).collect(), now);
        assert!(verified_announcement(&many, now, &here()).is_none());
        let (full, _) = announcement(4, None, (0..64u8).map(|i| format!("{i:064x}")).collect(), now);
        assert!(verified_announcement(&full, now, &here()).is_some());
        let (too_many_routes, _) =
            announcement(4, Some((1..=9).map(|i| format!("rtc:{}@203.0.113.{i}:7844", hoi_types::uri::b64(&[i; 32]))).collect()), vec![], now);
        assert!(verified_announcement(&too_many_routes, now, &here()).is_none());
        let (no_routes, _) = announcement(4, Some(vec![]), vec![], now);
        assert!(verified_announcement(&no_routes, now, &here()).is_none());
        let mut extra = valid.clone();
        extra["name"] = json!("lena's pi");
        assert!(verified_announcement(&extra, now, &here()).is_none(), "nothing about a person travels in an announcement");
    }

    #[test]
    fn configured_routes_come_first_then_carriers_one_route_per_round() {
        let live = |endpoint: &str, n: usize, communities: &[&str], issued_at: u64| LiveNode {
            endpoint: endpoint.into(),
            routes: (0..n).map(|i| format!("{endpoint}-{i}")).collect(),
            communities: communities.iter().map(|c| c.to_string()).collect(),
            issued_at,
        };
        let configured = vec!["cfg-0".to_string(), "cfg-1".to_string()];
        let nodes = vec![live("a", 3, &["garden"], 3), live("b", 2, &[], 2), live("c", 1, &["garden"], 1)];
        assert_eq!(select_public_nodes(&configured, &nodes, Some("garden")), ["cfg-0", "cfg-1", "a-0", "c-0", "a-1", "a-2", "b-0", "b-1"]);
        assert_eq!(select_public_nodes(&configured, &nodes, None), ["cfg-0", "cfg-1", "a-0", "b-0", "c-0", "a-1", "b-1", "a-2"]);
        let flood: Vec<LiveNode> = (0..20).map(|i| live(&format!("n{i}"), 8, &[], 100 - i)).collect();
        assert_eq!(select_public_nodes(&[], &flood, None).len(), MAX_PUBLIC_NODES);
        let dup = vec![live("cfg-0", 1, &[], 1)];
        assert_eq!(select_public_nodes(&["cfg-0-0".to_string()], &dup, None), ["cfg-0-0"]);
    }

    #[test]
    fn operator_configuration_is_validated_like_the_node_service() {
        let route = format!("rtc:{}@127.0.0.1:9000", hoi_types::uri::b64(&[7u8; 32]));
        let native = format!("{}@127.0.0.1:9001", "ab".repeat(32));
        assert_eq!(validate_public_nodes(&[route.clone(), route.clone(), native.clone()]).unwrap(), vec![route.clone(), native]);
        for bad in [
            vec!["https://user:secret@example.com".to_string()],
            vec![route.replace("127.0.0.1", "0.0.0.0")],
            vec![route.replace("127.0.0.1", "[0:0:0:0:0:0:0:0]")],
            vec![route.replace("9000", "65536")],
            vec![route.clone(); 17],
        ] {
            assert!(validate_public_nodes(&bad).is_err());
        }
        assert_eq!(validate_app_url("http://localhost:5173").unwrap(), "http://localhost:5173/");
        assert_eq!(validate_app_url("https://hoi.sh/app/").unwrap(), "https://hoi.sh/app/");
        for bad in ["https://user:secret@example.com/", "http://example.com/", "https://hoi.sh/app/?x=1", "https://hoi.sh/app/#x", "hoi.sh"] {
            assert!(validate_app_url(bad).is_err(), "{bad}");
        }
        assert!(validate_origin("https://hoi.example").is_ok());
        assert!(validate_origin("http://127.0.0.1:5173").is_ok());
        assert!(validate_origin("tauri://localhost").is_ok());
        for bad in [
            "null",
            "https://hoi.example/",
            "https://hoi.example/path",
            "tauri://other-host",
            "tauri://localhost/path",
            "ftp://hoi.example",
            "HTTPS://hoi.example",
        ] {
            assert!(validate_origin(bad).is_err(), "{bad}");
        }
    }
}
