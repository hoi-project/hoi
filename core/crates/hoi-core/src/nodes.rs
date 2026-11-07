//! Public nodes pass each other on (PROTOCOL.md §14.2). Nothing in the software names a
//! node, a registry or a relay to start from: a device finds its first node in an invite, in a
//! node link someone shared (`hoi://node?…`, or `#/node?…` in the app's address) or through its
//! home node, and every public node it reaches tells it about the other public nodes that node
//! reached itself. Whoever keeps a node running is a place that everyone who once reached it,
//! and anyone they told, can come back to: to find the open communities it carries, to have a
//! new one carried, and to find the next nodes along.
//!
//! What a list can make a device do is the question, so its routes are only
//! tried. A device takes lists only on links it dialed itself, and from its own devices and
//! home node; it keeps at most [`MAX_CANDIDATES`] routes for an hour, dials a few at a time and
//! drops one whose dial failed. A route that reaches a private network or this machine is passed
//! on, and taken, only between peers that are on such a network themselves. A node goes into the
//! list a device keeps and passes on only once it answered as a public node on a link that
//! device dialed: it sent its own list, which a node does only while it serves strangers. What a
//! node passes on is made of those, never of what it was told.
//!
//! A peer that dialed in proves nothing about anyone else. Its own native routes, on the address
//! its authenticated connection came from, are still worth one dial back: that is how a node
//! that only ever dials out becomes known to the nodes it reaches.

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};

use hoi_sync::Msg;
use hoi_types::{Id, NodeLink};

use crate::Node;
use crate::api::R;
use crate::dto::DirectoryStatus;
use crate::error::ApiError;
use crate::net::{Reach, Role, reach, rtc_parts, valid_peer_hint};
use crate::state::KnownNode;

/// Public nodes a device keeps between starts.
pub(crate) const MAX_KNOWN: usize = 32;
/// A known node that didn't answer as one for this long is forgotten.
const KNOWN_TTL: u64 = 30 * 86_400;
/// Routes other nodes passed on, kept in memory to be tried, and for how long.
const MAX_CANDIDATES: usize = 32;
const CANDIDATE_TTL: u64 = 3600;
/// A node whose dial failed is left alone this long.
const FAILED_PAUSE: u64 = 600;
/// Routes kept for one node, of each kind (WebRTC, native).
const ROUTES_PER_KIND: usize = 2;
/// `nodes` messages taken from one peer per minute; more are ignored.
const NODES_PER_MINUTE: u32 = 6;
/// Routes taken from one list, and from one peer that dialed in.
const TAKE_PER_LIST: usize = 16;
const TAKE_PER_CALLER: usize = 2;
/// Other public nodes a native node keeps a link to, besides its communities' peers.
const DIAL_NATIVE: usize = 3;
/// WebRTC routes a browser or phone tries to find communities when nothing else leads anywhere.
const DISCOVERY_RTC: usize = 8;

/// How far an address reaches, from closest to widest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Scope {
    Machine,
    Local,
    Internet,
}

pub(crate) fn ip_scope(ip: IpAddr) -> Scope {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            if v4.is_loopback() {
                Scope::Machine
            } else if v4.is_private() || v4.is_link_local() || v4.is_broadcast() || (a == 100 && (64..=127).contains(&b)) || a == 0 {
                // Carrier-grade NAT (100.64/10) is a provider's network, not the internet.
                Scope::Local
            } else {
                Scope::Internet
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                Scope::Machine
            } else if v6.is_unique_local() || v6.is_unicast_link_local() {
                Scope::Local
            } else {
                Scope::Internet
            }
        }
    }
}

/// The scope of a route's address. A bare endpoint id has no address (it needs relays this list
/// never carries), so it is no route to pass on.
pub(crate) fn route_scope(route: &str) -> Option<Scope> {
    Some(ip_scope(route_addr(route)?.ip()))
}

/// The address a route dials: `rtc:<fingerprint>@<addr>` or `<endpoint>@<addr>`; none for a bare
/// endpoint id.
pub(crate) fn route_addr(route: &str) -> Option<SocketAddr> {
    match rtc_parts(route) {
        Some((_, addr)) => Some(addr),
        None if route.starts_with("rtc:") => None,
        None => route.split_once('@')?.1.parse::<SocketAddr>().ok(),
    }
}

impl Scope {
    /// How the app names it: `device`, `network` or `internet`.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Scope::Machine => "device",
            Scope::Local => "network",
            Scope::Internet => "internet",
        }
    }
}

fn reaches_of(routes: &[String]) -> BTreeSet<Reach> {
    routes.iter().filter_map(|r| reach(r)).collect()
}

/// At most [`ROUTES_PER_KIND`] of each kind, in order, each once, each with an address.
fn trim_routes(routes: impl IntoIterator<Item = String>) -> Vec<String> {
    let (mut rtc, mut native) = (0, 0);
    let mut out: Vec<String> = Vec::new();
    for r in routes {
        if out.contains(&r) || !valid_peer_hint(&r) || route_scope(&r).is_none() {
            continue;
        }
        let count = if r.starts_with("rtc:") { &mut rtc } else { &mut native };
        if *count < ROUTES_PER_KIND {
            *count += 1;
            out.push(r);
        }
    }
    out
}

impl Node {
    /// Whether this node serves strangers as a public node right now: it accepts other people's
    /// connections (the relay switch, today's cap) and takes carry requests. Only then does it
    /// hand out node lists and a link, and keep links to other public nodes.
    pub(crate) fn is_public_node(&self) -> bool {
        self.accepts_members() && self.accepts_carry()
    }

    /// How close a peer is. Our own devices and home node hear of every scope; any other peer
    /// only of the scope its connection came over, or wider.
    fn peer_scope(&self, peer: &str) -> Scope {
        let Some(p) = self.peers.get(peer) else { return Scope::Internet };
        if p.client || p.home {
            return Scope::Machine;
        }
        let addr =
            p.meta.addr.as_deref().and_then(|a| a.parse::<SocketAddr>().ok()).or_else(|| p.meta.dialed.iter().find_map(|r| rtc_parts(r).map(|(_, a)| a)));
        addr.map_or(Scope::Internet, |a| ip_scope(a.ip()))
    }

    /// Who this device is, by every key and certificate its routes name.
    fn own_reaches(&self) -> BTreeSet<Reach> {
        let mut own = reaches_of(&self.own_routes());
        own.extend(reaches_of(&self.rtc_routes));
        if let Some(t) = &self.transport {
            own.extend(reach(&t.endpoint_id));
        }
        own
    }

    fn failed_lately(&self, reaches: &BTreeSet<Reach>) -> bool {
        let now = self.now();
        reaches.iter().any(|x| self.node_failed.get(x).is_some_and(|at| now.saturating_sub(*at) < FAILED_PAUSE))
    }

    /// Known nodes that answered as public nodes within [`KNOWN_TTL`], freshest first.
    fn answered(&self) -> impl Iterator<Item = &KnownNode> {
        let now = self.now();
        self.state.nodes.iter().filter(move |n| n.seen > 0 && now.saturating_sub(n.seen) < KNOWN_TTL)
    }

    /// Whether we tell `peer` about public nodes: our own devices and home node always, anyone
    /// else only while this node serves strangers.
    fn shares_nodes(&self, peer: &str) -> bool {
        let Some(p) = self.peers.get(peer) else { return false };
        p.said_hello && p.features.contains(hoi_sync::FEATURE_NODES) && (p.client || p.home || (p.role() == Role::Peer && self.is_public_node()))
    }

    /// What we pass on to `peer`: the public nodes we reached ourselves, freshest first, none
    /// it is itself, none we failed to reach lately, no route narrower than the peer is.
    fn node_list_for(&self, peer: &str) -> Vec<String> {
        let scope = self.peer_scope(peer);
        let mut theirs = self.peers.get(peer).map(|p| p.meta.reaches()).unwrap_or_default();
        if let Some(p) = self.peers.get(peer) {
            theirs.extend(reaches_of(&p.routes));
        }
        let mut out: Vec<String> = Vec::new();
        for n in self.answered() {
            let reaches = reaches_of(&n.routes);
            if reaches.iter().any(|x| theirs.contains(x)) || self.failed_lately(&reaches) {
                continue;
            }
            for r in n.routes.iter().filter(|r| route_scope(r).is_some_and(|s| s >= scope)) {
                if out.len() < hoi_sync::MAX_NODE_ROUTES && !out.contains(r) {
                    out.push(r.clone());
                }
            }
        }
        out
    }

    /// Sends `peer` our list of public nodes, with the holders we vouch for, unless it has this
    /// very list already. An empty list still says something to a stranger: this node serves
    /// strangers.
    pub(crate) fn send_nodes(&mut self, peer: &str) {
        if !self.shares_nodes(peer) {
            return;
        }
        let routes = self.node_list_for(peer);
        let holders = self.vouches();
        let Some(p) = self.peers.get_mut(peer) else { return };
        let list = (routes, holders);
        if p.sent_nodes.as_ref() == Some(&list) {
            return;
        }
        p.sent_nodes = Some(list.clone());
        let (routes, holders) = list;
        self.send_msg(peer, Msg::Nodes { routes, holders });
    }

    pub(crate) fn broadcast_nodes(&mut self) {
        let peers: Vec<String> = self.peers.keys().cloned().collect();
        for peer in peers {
            self.send_nodes(&peer);
        }
    }

    pub(crate) fn receive_nodes(&mut self, peer: &str, routes: Vec<String>, holders: Vec<hoi_sync::Vouch>) -> bool {
        let now = self.now();
        let Some(p) = self.peers.get_mut(peer) else { return false };
        if !p.features.contains(hoi_sync::FEATURE_NODES) {
            return false;
        }
        if p.nodes_minute != now / 60 {
            p.nodes_minute = now / 60;
            p.nodes_msgs = 0;
        }
        p.nodes_msgs += 1;
        if p.nodes_msgs > NODES_PER_MINUTE {
            return true;
        }
        let own = p.client || p.home;
        let dialed = p.role() == Role::Peer && !p.meta.incoming;
        if !own && !dialed {
            // A peer that dialed in makes this node dial nobody else; its own native routes on
            // the address its connection came from are worth one dial back, unless it publishes
            // no address of its own.
            if !p.features.contains(hoi_sync::FEATURE_UNLISTED) {
                let back = self.caller_routes(peer);
                self.take_candidates(back, now);
            }
            return true;
        }
        if dialed {
            self.note_public_node(peer);
        }
        // Who vouches for which holder counts only from the person's own devices and links this
        // device dialed, as the list does (crate::scatter).
        self.note_vouches(peer, holders);
        let scope = self.peer_scope(peer);
        self.take_candidates(routes.into_iter().filter(|r| route_scope(r).is_some_and(|s| s >= scope)).take(TAKE_PER_LIST).collect(), now);
        true
    }

    /// An authenticated native peer's own routes on the address its connection came from.
    fn caller_routes(&self, peer: &str) -> Vec<String> {
        let Some(p) = self.peers.get(peer) else { return Vec::new() };
        let (Some(ep), Some(addr)) = (&p.meta.endpoint, p.meta.addr.as_deref().and_then(|a| a.parse::<SocketAddr>().ok())) else { return Vec::new() };
        let same_host = |r: &String| {
            r.split_once('@').is_some_and(|(id, a)| id == ep && a.parse::<SocketAddr>().is_ok_and(|a| a.ip().to_canonical() == addr.ip().to_canonical()))
        };
        let mut out = vec![format!("{ep}@{addr}")];
        out.extend(p.routes.iter().filter(|r| same_host(r)).cloned());
        let mut seen = BTreeSet::new();
        out.retain(|r| seen.insert(r.clone()));
        out.truncate(TAKE_PER_CALLER);
        out
    }

    /// A peer we dialed answered as a public node: it goes to the front of the nodes we keep,
    /// with the routes bound to this connection (the rules of `remember_member`): the
    /// address its authenticated QUIC connection came from and the routes it gave for that same
    /// endpoint, the WebRTC routes we dialed with its pinned certificate, and its own WebRTC
    /// routes with that certificate or on the address its QUIC connection came from.
    fn note_public_node(&mut self, peer: &str) {
        let Some(p) = self.peers.get(peer) else { return };
        let reaches = p.meta.reaches();
        if reaches.is_empty() {
            return;
        }
        let source = p.meta.addr.as_deref().and_then(|a| a.parse::<SocketAddr>().ok()).map(|a| a.ip().to_canonical());
        let mut routes: Vec<String> = Vec::new();
        if let Some(ep) = &p.meta.endpoint {
            if let Some(addr) = &p.meta.addr {
                routes.push(format!("{ep}@{addr}"));
            }
            routes.extend(p.routes.iter().filter(|r| !r.starts_with("rtc:") && r.split_once('@').is_some_and(|(id, _)| id == ep)).cloned());
        }
        let bound =
            |r: &&String| rtc_parts(r).is_some_and(|(fp, addr)| reaches.contains(&Reach::Cert(fp.to_string())) || source == Some(addr.ip().to_canonical()));
        routes.extend(p.meta.dialed.iter().chain(&p.routes).filter(bound).cloned());
        let own = self.own_reaches();
        let routes = trim_routes(routes.into_iter().filter(|r| reach(r).is_some_and(|x| !own.contains(&x))));
        if routes.is_empty() {
            return;
        }
        let mut all = reaches.clone();
        all.extend(reaches_of(&routes));
        let now = self.now();
        let before: Vec<Vec<String>> = self.answered().map(|n| n.routes.clone()).collect();
        let was_seen = self.state.nodes.iter().find(|n| reaches_of(&n.routes).iter().any(|x| all.contains(x))).map(|n| n.seen);
        self.state.nodes.retain(|n| !reaches_of(&n.routes).iter().any(|x| all.contains(x)) && (n.seen == 0 || now.saturating_sub(n.seen) < KNOWN_TTL));
        self.state.nodes.insert(0, KnownNode { routes: routes.clone(), seen: now });
        self.state.nodes.truncate(MAX_KNOWN);
        self.node_candidates.retain(|(r, _)| reach(r).is_none_or(|x| !all.contains(&x)));
        self.node_failed.retain(|x, _| !all.contains(x));
        let after: Vec<Vec<String>> = self.answered().map(|n| n.routes.clone()).collect();
        // Seen times change on every link; written down at most hourly.
        if before != after || was_seen.is_none_or(|s| now.saturating_sub(s) > 3600) {
            self.mark_state();
        }
        if before != after {
            self.broadcast_nodes();
        }
    }

    fn take_candidates(&mut self, routes: Vec<String>, now: u64) {
        let own = self.own_reaches();
        let home: BTreeSet<Reach> = self.state.home.iter().flat_map(|h| reaches_of(&h.rtc)).collect();
        let known: BTreeSet<String> = self.answered().flat_map(|n| n.routes.iter().cloned()).collect();
        self.node_candidates.retain(|(_, at)| now.saturating_sub(*at) < CANDIDATE_TTL);
        for r in routes {
            if !valid_peer_hint(&r) || route_scope(&r).is_none() || known.contains(&r) || self.consent_blocks(&r) {
                continue;
            }
            let Some(x) = reach(&r) else { continue };
            if own.contains(&x) || home.contains(&x) {
                continue;
            }
            self.node_candidates.retain(|(c, _)| *c != r);
            self.node_candidates.insert(0, (r, now));
        }
        self.node_candidates.truncate(MAX_CANDIDATES);
    }

    /// The host could not reach `route` (or the endpoint id it names): routes other nodes passed
    /// on for that node go, and a node we keep is left alone for a while.
    pub fn dial_failed(&mut self, route: &str) {
        let Some(x) = reach(route) else { return };
        self.node_candidates.retain(|(r, _)| reach(r).as_ref() != Some(&x));
        let now = self.now();
        self.node_failed.retain(|_, at| now.saturating_sub(*at) < FAILED_PAUSE);
        if self.node_failed.len() < 4 * MAX_KNOWN {
            self.node_failed.insert(x, now);
        }
    }

    /// Public nodes to look at in order: links opened here first, then the nodes reached
    /// before, freshest first, then what other nodes passed on. Each node's routes stay
    /// together; nodes failed lately and nodes already connected are left out.
    fn directory_groups(&self) -> Vec<Vec<String>> {
        let now = self.now();
        let connected: BTreeSet<Reach> = self.peers.values().filter(|p| p.said_hello).flat_map(|p| p.meta.reaches()).collect();
        let own = self.own_reaches();
        let seeds = self.state.nodes.iter().filter(|n| n.seen == 0);
        let fresh: Vec<Vec<String>> = seeds.chain(self.answered()).map(|n| n.routes.clone()).collect();
        let mut candidates: Vec<&(String, u64)> = self.node_candidates.iter().filter(|(_, at)| now.saturating_sub(*at) < CANDIDATE_TTL).collect();
        candidates.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
        let mut groups = fresh;
        groups.extend(candidates.into_iter().map(|(r, _)| vec![r.clone()]));
        let mut seen = BTreeSet::new();
        groups
            .into_iter()
            .filter(|g| {
                let reaches = reaches_of(g);
                !reaches.is_empty()
                    && !reaches.iter().any(|x| connected.contains(x) || own.contains(x))
                    && !self.failed_lately(&reaches)
                    && !g.iter().any(|r| self.consent_blocks(r))
                    && reaches.iter().all(|x| seen.insert(x.clone()))
            })
            .collect()
    }

    /// WebRTC routes a browser or phone tries to find communities when no route of a community
    /// it is in leads anywhere: public nodes, by [`Node::directory_groups`]. Never the home node.
    pub fn discovery_candidates(&self) -> Vec<String> {
        let home: BTreeSet<&String> = self.state.home.iter().flat_map(|h| h.rtc.iter()).collect();
        let mut out = Vec::new();
        for g in self.directory_groups() {
            for r in g.iter().filter(|r| r.starts_with("rtc:") && !home.contains(r)) {
                if out.len() < DISCOVERY_RTC && !out.contains(r) {
                    out.push(r.clone());
                }
            }
        }
        out
    }

    /// Routes of up to `groups` public nodes this device knows of, every route of each, for a
    /// private group that still needs mailboxes (crate::mailbox). Never the home node.
    pub(crate) fn directory_routes(&self, groups: usize) -> Vec<String> {
        let home: BTreeSet<&String> = self.state.home.iter().flat_map(|h| h.rtc.iter()).collect();
        self.directory_groups().into_iter().take(groups).flatten().filter(|r| !home.contains(r)).collect()
    }

    /// Native routes of a few public nodes for a native node that serves strangers, so it hears
    /// of new nodes and passes them on; its communities' peers are dialed anyway.
    pub(crate) fn directory_hints(&self) -> Vec<String> {
        if !self.is_public_node() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for g in self.directory_groups().into_iter().filter(|g| g.iter().any(|r| !r.starts_with("rtc:"))).take(DIAL_NATIVE) {
            out.extend(g.into_iter().filter(|r| !r.starts_with("rtc:")));
        }
        out
    }

    /// The routes this device keeps for the public node behind `peer`, as it answered on a link
    /// this device dialed (crate::nodes keeps them by the rules of `note_public_node`): never
    /// what a caller said about itself.
    fn kept_routes(&self, peer: &str) -> Vec<String> {
        let Some(p) = self.peers.get(peer) else { return Vec::new() };
        let reaches = p.meta.reaches();
        if reaches.is_empty() {
            return Vec::new();
        }
        self.answered().find(|n| reaches_of(&n.routes).iter().any(|x| reaches.contains(x))).map(|n| n.routes.clone()).unwrap_or_default()
    }

    /// Public nodes connected now, that this device dialed, which hold `c`, list it or were
    /// just asked to keep it: where a newcomer finds the community while every member is away.
    pub(crate) fn carrier_routes(&self, c: &Id) -> Vec<String> {
        let mut out = Vec::new();
        for (key, p) in &self.peers {
            let public = p.said_hello
                && p.role() == Role::Peer
                && !p.meta.incoming
                && p.features.contains(hoi_sync::FEATURE_CARRY)
                && !p.features.contains(hoi_sync::FEATURE_UNLISTED);
            if public && (p.communities.contains(c) || p.carry_asked.contains(c) || p.catalog.contains_key(c)) {
                out.extend(self.kept_routes(key));
            }
        }
        out
    }

    /// A few public nodes this device reached, freshest first, for an invite to an open
    /// community: a newcomer whose other routes lead nowhere still lands in the network, hears
    /// of the next nodes and finds the community from there.
    pub(crate) fn entry_routes(&self, nodes: usize) -> Vec<String> {
        self.answered().filter(|n| !self.failed_lately(&reaches_of(&n.routes))).take(nodes).flat_map(|n| n.routes.iter().cloned()).collect()
    }

    /// The person opened a node link (or the operator gave one with `hoi-node --node`): its
    /// routes are kept and tried first, but passed on only once the node answered as one.
    pub fn open_node_link(&mut self, link: &str) -> R<DirectoryStatus> {
        let parsed =
            NodeLink::parse(link).map_err(|_| ApiError::invalid("That isn't a node link. A node link starts with hoi://node? or has #/node? in it."))?;
        // A person reads the operator's notice in the link, and accepts it, before connecting.
        self.link_consent(&parsed)?;
        let own = self.own_reaches();
        let routes = trim_routes(parsed.rtc.into_iter().chain(parsed.peers).filter(|r| reach(r).is_some_and(|x| !own.contains(&x))));
        if routes.is_empty() {
            return Err(ApiError::invalid("This link names no address another device could reach, or it is this device's own."));
        }
        let reaches = reaches_of(&routes);
        match self.state.nodes.iter_mut().find(|n| reaches_of(&n.routes).iter().any(|x| reaches.contains(x))) {
            Some(n) => n.routes = trim_routes(routes.iter().cloned().chain(n.routes.iter().cloned())),
            None => {
                self.state.nodes.insert(0, KnownNode { routes, seen: 0 });
                self.state.nodes.truncate(MAX_KNOWN);
            }
        }
        self.node_failed.retain(|x, _| !reaches.contains(x));
        self.mark_state();
        Ok(self.directory_status())
    }

    pub fn directory_status(&self) -> DirectoryStatus {
        let now = self.now();
        let known: Vec<BTreeSet<Reach>> = self.answered().map(|n| reaches_of(&n.routes)).collect();
        let connected = self.peers.values().filter(|p| p.said_hello && !p.client && !p.home).filter(|p| {
            let r = p.meta.reaches();
            known.iter().any(|k| k.iter().any(|x| r.contains(x)))
        });
        DirectoryStatus {
            known: known.len() as u64,
            waiting: (self.state.nodes.iter().filter(|n| n.seen == 0).count()
                + self.node_candidates.iter().filter(|(_, at)| now.saturating_sub(*at) < CANDIDATE_TTL).count()) as u64,
            connected: connected.count() as u64,
            listings: self.directory.len() as u64,
        }
    }

    /// A link anyone can open to reach this node, while it serves strangers: its routes on the
    /// internet first, then those on its own network.
    pub(crate) fn node_link(&self) -> Option<String> {
        if !self.is_public_node() {
            return None;
        }
        let mut rtc: Vec<String> = self.rtc_routes.iter().filter(|r| route_scope(r).is_some()).cloned().collect();
        rtc.sort_by_key(|r| std::cmp::Reverse(route_scope(r)));
        let mut peers: Vec<String> =
            self.transport.iter().flat_map(|t| t.peers.iter()).filter(|r| !r.starts_with("rtc:") && route_scope(r).is_some()).cloned().collect();
        peers.sort_by_key(|r| std::cmp::Reverse(route_scope(r)));
        if rtc.is_empty() && peers.is_empty() {
            return None;
        }
        // The operator's notice rides along, signed, so it is read before anyone connects.
        let (notice, signature) = match &self.operator_notice {
            Some(n) if n.signature.is_some() => (Some(n.bytes.clone()), n.signature),
            _ => (None, None),
        };
        Some(NodeLink { rtc, peers, notice, signature }.to_web(&self.web_base))
    }
}
