//! Finding members on the same network: a native node sends a small UDP multicast
//! beacon every half minute, and nodes that carry a community with it dial it directly. No
//! service is involved, and nothing in a beacon means anything to someone who doesn't know the
//! community: per community, a tag that changes every ten minutes and the node's endpoint and
//! port sealed under a key derived from the community. A stranger on the network sees that some
//! device sends hoi beacons, which its UDP traffic would show anyway.
//!
//! ```text
//! beacon = "HOIL" 0x01 ‖ n (≤ 8) ‖ n × ( tag[16] ‖ sealed[74] )
//! tag    = BLAKE3-keyed(derive_key("hoi 2026 lan tag v1", community), u64be(now / 600))[..16]
//! sealed = sealed::seal("hoi 2026 lan route v1", derive_key("hoi 2026 lan route key v1", community),
//!                       endpoint[32] ‖ u16be(port))
//! ```
//!
//! Beacons for the current and the previous ten minutes count. Private communities never send
//! one: their members are in their group, not on the community's key.

use std::collections::BTreeSet;
use std::net::IpAddr;

use hoi_types::Id;

use crate::node::Node;

/// The UDP port and IPv4 group beacons go to.
pub const LAN_PORT: u16 = 7848;
pub const LAN_GROUP: [u8; 4] = [239, 255, 70, 74];
/// Every half minute.
pub const LAN_EVERY_SECS: u64 = 30;
const MAGIC: &[u8] = b"HOIL\x01";
const WINDOW: u64 = 600;
const MAX_COMMUNITIES: usize = 8;
const ENTRY: usize = 16 + 24 + 34 + 16;
/// Beacons taken in a minute, from everyone together.
const HEARD_PER_MINUTE: u32 = 120;

fn tag(c: &Id, window: u64) -> [u8; 16] {
    let key = blake3::derive_key("hoi 2026 lan tag v1", &c.0);
    let h = blake3::keyed_hash(&key, &window.to_be_bytes());
    h.as_bytes()[..16].try_into().expect("16 of 32 bytes")
}

fn route_key(c: &Id) -> [u8; 32] {
    blake3::derive_key("hoi 2026 lan route key v1", &c.0)
}

/// What a beacon offers: per community tag, the sealed route.
pub fn parse(beacon: &[u8]) -> Option<Vec<([u8; 16], &[u8])>> {
    let rest = beacon.strip_prefix(MAGIC)?;
    let (&n, rest) = rest.split_first()?;
    if n as usize > MAX_COMMUNITIES || rest.len() != n as usize * ENTRY {
        return None;
    }
    Some(rest.chunks(ENTRY).map(|e| (e[..16].try_into().expect("16 bytes"), &e[16..])).collect())
}

impl Node {
    /// Whether this node sends and takes LAN beacons (on by default for native nodes).
    pub fn set_lan(&mut self, on: bool) {
        self.lan.on = on;
    }

    /// The beacon to send now, for this node's endpoint listening on `port`; `None` when it has
    /// nothing to offer or LAN discovery is off.
    pub fn lan_beacon(&self, port: u16) -> Option<Vec<u8>> {
        if !self.lan.on || port == 0 {
            return None;
        }
        let endpoint = hoi_crypto::Keypair::from_secret(&self.transport_secret_now()?).public();
        let communities: Vec<Id> =
            self.state.joined.keys().filter(|c| !self.is_private_community(c) && self.communities.contains_key(*c)).copied().take(MAX_COMMUNITIES).collect();
        if communities.is_empty() {
            return None;
        }
        let window = self.now() / WINDOW;
        let mut plain = endpoint.0.to_vec();
        plain.extend(port.to_be_bytes());
        let mut out = MAGIC.to_vec();
        out.push(communities.len() as u8);
        for c in &communities {
            out.extend(tag(c, window));
            out.extend(hoi_crypto::sealed::seal("hoi 2026 lan route v1", &route_key(c), &plain).ok()?);
        }
        Some(out)
    }

    /// A beacon heard from `from`: routes to this node's communities' members go on trial.
    pub fn lan_heard(&mut self, beacon: &[u8], from: IpAddr) {
        if !self.lan.on {
            return;
        }
        let now = self.now();
        if now / 60 != self.lan.minute {
            self.lan.minute = now / 60;
            self.lan.heard = 0;
        }
        self.lan.heard += 1;
        if self.lan.heard > HEARD_PER_MINUTE {
            return;
        }
        let Some(entries) = parse(beacon) else { return };
        let own = self.transport_secret_now().map(|s| hoi_crypto::Keypair::from_secret(&s).public());
        let windows = [now / WINDOW, (now / WINDOW).saturating_sub(1)];
        let mine: Vec<Id> = self.state.joined.keys().filter(|c| !self.is_private_community(c)).copied().collect();
        let mut found: BTreeSet<(Id, String)> = BTreeSet::new();
        for (t, sealed) in entries {
            let Some(c) = mine.iter().find(|c| windows.iter().any(|w| tag(c, *w) == t)) else { continue };
            let Ok(plain) = hoi_crypto::sealed::open("hoi 2026 lan route v1", &route_key(c), sealed) else { continue };
            if plain.len() != 34 {
                continue;
            }
            let endpoint = Id(plain[..32].try_into().expect("32 bytes"));
            let port = u16::from_be_bytes([plain[32], plain[33]]);
            if Some(endpoint) == own || port == 0 {
                continue;
            }
            let addr = std::net::SocketAddr::new(from, port);
            found.insert((*c, format!("{}@{addr}", endpoint.hex())));
        }
        for (c, route) in found {
            self.try_routes(c, &[route]);
        }
    }
}

/// LAN discovery's state on a node.
#[derive(Debug)]
pub(crate) struct Lan {
    pub on: bool,
    minute: u64,
    heard: u32,
}

impl Default for Lan {
    fn default() -> Self {
        Lan { on: true, minute: 0, heard: 0 }
    }
}
