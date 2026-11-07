//! Finding who carries a community when every route a device holds has gone quiet.
//!
//! Three ways, in the order a device tries them:
//!
//! 1. **`find`**: a device asks the public nodes it is connected to (those that said `find`)
//!    which nodes carry a community it has joined and nobody here serves. A public node answers
//!    `found` from what it knows: its directory's listing, the public nodes it is connected to
//!    that hold the community, itself if it carries it; one that runs a DHT also looks it up
//!    there and answers again when the DHT does. Routes in an answer are only tried
//!    (`try_routes`), never kept until they deliver, as with any routes a stranger suggests.
//! 2. **Trackers** the community names: the browser asks them itself (`public-connections.ts`).
//! 3. **The DHT among public nodes** (hoi-net `dht`): a Mainline DHT with no outside bootstrap.
//!    Public nodes tell each other where their DHT nodes listen (`dht`), announce each open
//!    community they carry under `BLAKE3("hoi dht v1" ‖ key)[..20]` (the key never enters the
//!    DHT), and answer a small hoi query on their answer port with a statement signed by their
//!    endpoint key, checked here before any of its routes is tried.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddrV4;

use hoi_crypto::Keypair;
use hoi_sync::Msg;
use hoi_types::cbor::{self, Value};
use hoi_types::{Admission, Id};

use crate::net::Role;
use crate::node::Node;

/// A device asks again for the same community at most this often (seconds).
pub const FIND_EVERY: u64 = 600;
/// Finds a public node answers per peer per minute.
const FINDS_PER_MINUTE: u32 = 10;
/// Public nodes a device asks at once.
const ASK_AT_ONCE: usize = 3;
/// Peers a public node remembers waiting for a DHT answer, per community, and for how long.
const WAITING_PER_COMMUNITY: usize = 16;
const WAIT_FOR: u64 = 120;
/// Communities a public node announces in the DHT at most, and DHT lookups queued at most.
pub const MAX_DHT_COMMUNITIES: usize = 64;
const MAX_LOOKUPS: usize = 32;
/// Peers whose DHT contacts are kept.
const MAX_CONTACT_PEERS: usize = 64;
/// A DHT answer older than this is refused.
pub const DHT_ANSWER_TTL: u64 = 30 * 60;
/// The statement a DHT answer signs.
const DHT_DOMAIN: &str = "hoi dht node v1";

/// This node's DHT, as the host runs it: where it listens and where it answers hoi queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DhtPorts {
    pub dht: u16,
    pub answer: u16,
}

#[derive(Debug, Default)]
pub(crate) struct Finds {
    /// As a device: when it last asked about each community.
    asked: HashMap<Id, u64>,
    /// As a public node: peers waiting for a DHT answer.
    waiting: BTreeMap<Id, Vec<(String, u64)>>,
    /// As a public node: finds per peer in the current minute.
    rate: HashMap<String, (u64, u32)>,
    /// DHT lookups the host should run.
    lookups: BTreeSet<Id>,
    /// DHT contacts other public nodes gave.
    contacts: HashMap<String, Vec<SocketAddrV4>>,
    /// This node's own DHT, while the host runs one.
    pub(crate) dht: Option<DhtPorts>,
}

/// The DHT key for a community: the first 20 bytes of `BLAKE3("hoi dht v1" ‖ key)`.
pub fn dht_key(community: &Id) -> [u8; 20] {
    let mut h = blake3::Hasher::new();
    h.update(DHT_DOMAIN.as_bytes());
    h.update(community.as_bytes());
    let mut out = [0u8; 20];
    out.copy_from_slice(&h.finalize().as_bytes()[..20]);
    out
}

fn dht_statement(endpoint: &Id, community: &Id, issued_at: u64, routes: &[String]) -> String {
    format!("{DHT_DOMAIN}\n{}\n{}\n{issued_at}\n{}", endpoint.hex(), community.hex(), routes.join(","))
}

/// A DHT answer's routes, if its signature, community and age check out.
pub fn dht_routes(answer: &[u8], community: &Id, now: u64) -> Option<Vec<String>> {
    let Value::M(entries) = cbor::decode(answer).ok()? else { return None };
    let get = |k: &str| entries.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let endpoint = match get("e")? {
        Value::B(b) => Id::from_slice(&b).ok()?,
        _ => return None,
    };
    let c = match get("c")? {
        Value::B(b) => Id::from_slice(&b).ok()?,
        _ => return None,
    };
    let issued_at = match get("t")? {
        Value::U(t) => t,
        _ => return None,
    };
    let routes: Vec<String> = match get("r")? {
        Value::A(v) if v.len() <= hoi_sync::MAX_ROUTES => v.into_iter().map(|r| if let Value::T(s) = r { Some(s) } else { None }).collect::<Option<_>>()?,
        _ => return None,
    };
    let sig: [u8; 64] = match get("s")? {
        Value::B(b) => b.try_into().ok()?,
        _ => return None,
    };
    if c != *community || issued_at > now + 600 || issued_at + DHT_ANSWER_TTL < now || !hoi_crypto::key_is_strong(&endpoint) {
        return None;
    }
    if !hoi_crypto::verify(&endpoint, dht_statement(&endpoint, &c, issued_at, &routes).as_bytes(), &hoi_types::Sig(sig)) {
        return None;
    }
    // native routes must name the endpoint that signed them; WebRTC routes pin a certificate
    let own = endpoint.hex();
    Some(routes.into_iter().filter(|r| crate::net::valid_peer_hint(r) && (r.starts_with("rtc:") || r.split('@').next() == Some(own.as_str()))).collect())
}

/// `ip:port` as an IPv4 DHT contact (the Mainline DHT is IPv4 here).
pub fn parse_contact(s: &str) -> Option<SocketAddrV4> {
    s.parse::<SocketAddrV4>().ok().filter(|a| a.port() != 0 && !a.ip().is_unspecified() && !a.ip().is_broadcast() && !a.ip().is_multicast())
}

impl Node {
    /// The transport key's secret, if this node has one yet.
    pub(crate) fn transport_secret_now(&self) -> Option<[u8; 32]> {
        self.transport_kp.as_ref().map(|k| k.secret())
    }

    /// The host runs a DHT node among public nodes (hoi-net `dht`), or stopped it.
    pub fn set_dht(&mut self, ports: Option<DhtPorts>) {
        if self.finds.dht != ports {
            self.finds.dht = ports;
            // the hello's feature list changed
            self.announce();
        }
    }

    /// Whether this node answers `find`: a public node does.
    pub(crate) fn answers_find(&self) -> bool {
        self.is_public_node()
    }

    /// Whether this node takes part in the DHT among public nodes.
    pub(crate) fn runs_dht(&self) -> bool {
        self.finds.dht.is_some() && self.is_public_node()
    }

    /// Where this node's DHT listens, as others reach it: the IPv4 hosts of its own public
    /// routes, at the DHT port.
    pub(crate) fn own_dht_contacts(&self) -> Vec<String> {
        let Some(p) = self.finds.dht else { return Vec::new() };
        let mut seen = BTreeSet::new();
        self.published_routes()
            .iter()
            .filter_map(|r| r.rsplit_once('@').map(|(_, addr)| addr))
            .filter_map(|addr| addr.parse::<std::net::SocketAddr>().ok())
            .filter_map(|a| match a.ip() {
                std::net::IpAddr::V4(ip) => Some(SocketAddrV4::new(ip, p.dht)),
                _ => None,
            })
            .filter(|a| seen.insert(*a))
            .take(hoi_sync::MAX_DHT_CONTACTS)
            .map(|a| a.to_string())
            .collect()
    }

    /// After a peer's hello: tell a fellow DHT node where ours listens, and ask a node that
    /// answers `find` about communities nobody here serves.
    pub(crate) fn after_hello_find(&mut self, peer: &str) {
        let Some(p) = self.peers.get(peer) else { return };
        let (dht, find) = (p.features.contains(hoi_sync::FEATURE_DHT), p.features.contains(hoi_sync::FEATURE_FIND));
        if dht && self.runs_dht() {
            let contacts = self.own_dht_contacts();
            if !contacts.is_empty() {
                self.send_msg(peer, Msg::Dht { contacts });
            }
        }
        if find {
            self.find_missing();
        }
    }

    /// Open communities this device has joined that no connected peer serves.
    fn unserved(&self) -> Vec<Id> {
        self.state
            .joined
            .keys()
            .filter(|c| !self.is_private_community(c))
            .filter(|c| self.communities.get(c).is_some_and(|st| st.limits.admission == Admission::Open))
            .filter(|c| !self.peers.values().any(|p| p.said_hello && p.communities.contains(c)))
            .copied()
            .collect()
    }

    /// Asks connected public nodes that said `find` about communities nobody here serves, each
    /// at most every [`FIND_EVERY`]. Runs after a hello from such a node and at every sweep.
    pub(crate) fn find_missing(&mut self) {
        let now = self.now();
        let askers: Vec<String> = self
            .peers
            .iter()
            .filter(|(_, p)| p.said_hello && p.role() == Role::Peer && p.features.contains(hoi_sync::FEATURE_FIND))
            .map(|(k, _)| k.clone())
            .take(ASK_AT_ONCE)
            .collect();
        if askers.is_empty() {
            return;
        }
        for c in self.unserved() {
            if self.finds.asked.get(&c).is_some_and(|t| t + FIND_EVERY > now) {
                continue;
            }
            self.finds.asked.insert(c, now);
            for peer in &askers {
                self.send_msg(peer, Msg::Find { community: c });
            }
            // a public node of our own looks in the DHT too
            if self.runs_dht() && self.finds.lookups.len() < MAX_LOOKUPS {
                self.finds.lookups.insert(c);
            }
        }
    }

    /// Routes for `c` this node knows of, for a `found`: its own if it carries `c`, the public
    /// nodes it is connected to that hold it, and its directory's listing.
    fn known_carriers(&self, c: &Id) -> Vec<String> {
        let mut out = Vec::new();
        if self.state.joined.contains_key(c) && self.is_public_node() {
            out.extend(self.published_routes());
        }
        out.extend(self.carrier_routes(c));
        if let Some(l) = self.directory.listings.get(c).filter(|l| l.public) {
            out.extend(l.p.iter().cloned());
        }
        let mut seen = BTreeSet::new();
        out.retain(|r| crate::net::valid_peer_hint(r) && seen.insert(r.clone()));
        out.truncate(hoi_sync::MAX_FOUND_ROUTES);
        out
    }

    pub(crate) fn on_find(&mut self, peer: &str, c: Id) {
        if !self.answers_find() || self.is_private_community(&c) {
            return;
        }
        let now = self.now();
        let minute = now / 60;
        let r = self.finds.rate.entry(peer.to_string()).or_insert((minute, 0));
        if r.0 != minute {
            *r = (minute, 0);
        }
        r.1 += 1;
        if r.1 > FINDS_PER_MINUTE {
            return;
        }
        let routes = self.known_carriers(&c);
        if !routes.is_empty() {
            self.send_msg(peer, Msg::Found { community: c, routes });
        }
        if self.runs_dht() {
            let w = self.finds.waiting.entry(c).or_default();
            w.retain(|(p, at)| p != peer && at + WAIT_FOR > now);
            if w.len() < WAITING_PER_COMMUNITY {
                w.push((peer.to_string(), now));
            }
            if self.finds.lookups.len() < MAX_LOOKUPS {
                self.finds.lookups.insert(c);
            }
        }
    }

    pub(crate) fn on_found(&mut self, peer: &str, c: Id, routes: Vec<String>) {
        let asked = self.finds.asked.contains_key(&c);
        if !asked || !self.peers.get(peer).is_some_and(|p| p.features.contains(hoi_sync::FEATURE_FIND)) {
            return;
        }
        // routes on trial only, like any a stranger suggests; ours are left out
        let own: BTreeSet<String> = self.own_routes().into_iter().collect();
        let routes: Vec<String> = routes.into_iter().filter(|r| !own.contains(r)).collect();
        self.try_routes(c, &routes);
    }

    pub(crate) fn on_dht_contacts(&mut self, peer: &str, contacts: Vec<String>) {
        if !self.runs_dht() || !self.peers.get(peer).is_some_and(|p| p.features.contains(hoi_sync::FEATURE_DHT)) {
            return;
        }
        let parsed: Vec<SocketAddrV4> = contacts.iter().filter_map(|c| parse_contact(c)).take(hoi_sync::MAX_DHT_CONTACTS).collect();
        if parsed.is_empty() || (self.finds.contacts.len() >= MAX_CONTACT_PEERS && !self.finds.contacts.contains_key(peer)) {
            return;
        }
        self.finds.contacts.insert(peer.to_string(), parsed);
    }

    /// DHT nodes of other public nodes, to bootstrap from (no outside routers).
    pub fn dht_contacts(&self) -> Vec<SocketAddrV4> {
        let mut seen = BTreeSet::new();
        self.finds.contacts.values().flatten().filter(|a| seen.insert(**a)).copied().collect()
    }

    /// Communities to announce in the DHT: the open ones this public node carries.
    pub fn dht_communities(&self) -> Vec<Id> {
        if !self.runs_dht() {
            return Vec::new();
        }
        self.state
            .joined
            .keys()
            .filter(|c| !self.is_private_community(c))
            .filter(|c| self.communities.get(c).is_some_and(|st| st.limits.admission == Admission::Open))
            .take(MAX_DHT_COMMUNITIES)
            .copied()
            .collect()
    }

    /// DHT lookups the host should run now, each handed out once.
    pub fn dht_lookups(&mut self) -> Vec<Id> {
        std::mem::take(&mut self.finds.lookups).into_iter().collect()
    }

    /// What a DHT lookup found for `c`: answers waiting peers, and tries the routes here too.
    pub fn dht_found(&mut self, c: Id, routes: Vec<String>) {
        let now = self.now();
        let mut seen = BTreeSet::new();
        let routes: Vec<String> =
            routes.into_iter().filter(|r| crate::net::valid_peer_hint(r) && seen.insert(r.clone())).take(hoi_sync::MAX_FOUND_ROUTES).collect();
        if routes.is_empty() {
            return;
        }
        for (peer, at) in self.finds.waiting.remove(&c).unwrap_or_default() {
            if at + WAIT_FOR > now && self.peers.contains_key(&peer) {
                self.send_msg(&peer, Msg::Found { community: c, routes: routes.clone() });
            }
        }
        if self.state.joined.contains_key(&c) {
            self.try_routes(c, &routes);
        }
    }

    /// The signed answer to a DHT query for `c`, when this public node carries it.
    pub fn dht_answer(&self, c: &Id) -> Option<Vec<u8>> {
        if !self.runs_dht() || !self.dht_communities().contains(c) {
            return None;
        }
        let kp = Keypair::from_secret(&self.transport_secret_now()?);
        let endpoint = kp.public();
        let own = endpoint.hex();
        let routes: Vec<String> = self
            .published_routes()
            .into_iter()
            .filter(|r| r.starts_with("rtc:") || r.split('@').next() == Some(own.as_str()))
            .take(hoi_sync::MAX_ROUTES)
            .collect();
        if routes.is_empty() {
            return None;
        }
        let now = self.now();
        let sig = kp.sign(dht_statement(&endpoint, c, now, &routes).as_bytes());
        Some(
            Value::map([
                ("c", Value::B(c.0.to_vec())),
                ("e", Value::B(endpoint.0.to_vec())),
                ("r", Value::A(routes.into_iter().map(Value::T).collect())),
                ("s", Value::B(sig.0.to_vec())),
                ("t", Value::U(now)),
            ])
            .encode(),
        )
    }
}
