//! Talking to peers over whatever pipe the host provides. The host owns sockets; the node owns
//! the conversation. Pipes today: the home-node WebSocket (desktop or hoi-node serving your
//! browser and phone), iroh QUIC between native nodes, and WebRTC data channels from browsers
//! and phones to native nodes (`rtc:` routes, hoi-net `rtc`).
//!
//! Routes: an invite carries the inviter's own routes, the routes from the invite it joined with,
//! and the routes of members it met since (`LocalState::learned`). So anyone whose node is
//! running can let newcomers in, not only whoever made the invite. Routes a registry or an
//! announcement suggested are only tried (`Node::trial_routes`): they are saved, and passed on,
//! once they delivered records.
//!
//! Strangers learn only what they need: a hello to anyone but our own devices names in the
//! clear just the communities whose routes led to that connection, and lists the others as
//! hashes salted for that connection, which only a peer that already knows a key can match.

use std::collections::{BTreeMap, BTreeSet};

use hoi_sync::Msg;
use hoi_types::{Id, Kind, Pair, Wire};
use hoi_validate::Strike;

use crate::api::R;
use crate::dto::{CommunitySyncStatus, ConnectionStatus, HomeNode, NetStatus, PairPreview, Serving, TransportStatus};
use crate::error::ApiError;
use crate::node::{Node, PeerState, Source, priority};
use crate::state::{LearnedRoute, PairedDevice};

/// Messages a peer may send per minute before it starts losing score.
const MSGS_PER_MINUTE: u32 = 3000;
/// Communities a paired device can make its home node join.
const MAX_CLIENT_COMMUNITIES: usize = 256;
/// Member routes remembered per community.
const MAX_LEARNED: usize = 16;
/// Public nodes an invite to an open community names besides its carriers.
const INVITE_ENTRY_NODES: usize = 2;
/// A member route nobody confirmed for this long is forgotten.
const LEARNED_TTL: u64 = 30 * 86_400;
/// Own routes put in front of an invite: direct addresses, then WebRTC routes.
const OWN_DIRECT: usize = 4;
const OWN_RTC: usize = 3;
/// WebRTC routes a browser tries when it has no home node.
const MAX_RTC_CANDIDATES: usize = 8;
/// Devices a home node keeps a credential for; the least recently seen goes first.
const MAX_PAIRED: usize = 32;
/// Separate the digests of device credentials and of spent pairing links from other hashes.
const CREDENTIAL_CONTEXT: &str = "hoi 2026 pairing credential v1";
const SPENT_LINK_CONTEXT: &str = "hoi 2026 spent pairing link v1";
use crate::state::{Joined, PairInfo};

fn digest(context: &str, secret: &str) -> Id {
    Id(blake3::derive_key(context, secret.as_bytes()))
}

/// Who a route reaches: an iroh endpoint (its key, hex) or a WebRTC node (its certificate's
/// fingerprint). Addresses change; these don't.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Reach {
    Endpoint(Id),
    Cert(String),
}

pub(crate) fn reach(route: &str) -> Option<Reach> {
    if let Some(rest) = route.strip_prefix("rtc:") {
        return rest.split_once('@').map(|(fp, _)| Reach::Cert(fp.to_string()));
    }
    Id::from_hex(route.split_once('@').map_or(route, |(id, _)| id)).ok().map(Reach::Endpoint)
}

/// The fingerprint and address of an `rtc:` route.
pub(crate) fn rtc_parts(route: &str) -> Option<(&str, std::net::SocketAddr)> {
    let (fp, addr) = route.strip_prefix("rtc:")?.split_once('@')?;
    Some((fp, addr.parse().ok()?))
}

/// What the host knows about a connection. The node never sees sockets; this is all it needs to
/// say how a connection travels, remember a member's route, and apply the relay switch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerMeta {
    /// "quic", "webrtc" or "websocket".
    pub transport: String,
    /// The bytes never leave this machine (a loopback WebSocket).
    pub local: bool,
    /// A WebSocket protected by TLS (`wss://`).
    pub tls: bool,
    /// The other side dialed us.
    pub incoming: bool,
    /// The authenticated iroh endpoint, and the UDP address it came from, when direct.
    pub endpoint: Option<String>,
    pub addr: Option<String>,
    /// The `rtc:` routes the host dialed for this connection: one node's addresses, sharing the
    /// certificate fingerprint the DTLS handshake pinned. Says which communities' routes led here.
    pub dialed: Vec<String>,
    /// On a WebRTC link: both certificates and the username fragment, for the binding
    /// (crate::bind).
    pub rtc: Option<crate::bind::RtcChannel>,
    /// A direct link with another browser (crate::signal): the node that passed the offer and
    /// the other side's handle there.
    pub direct: Option<(String, [u8; 16])>,
}

impl PeerMeta {
    fn for_role(role: Role) -> PeerMeta {
        match role {
            Role::Peer => PeerMeta { transport: "quic".into(), ..Default::default() },
            Role::Client => PeerMeta { transport: "websocket".into(), incoming: true, ..Default::default() },
            Role::Home => PeerMeta { transport: "websocket".into(), ..Default::default() },
        }
    }

    /// Who the other end proved to be: its authenticated iroh endpoint, or the node whose
    /// certificate we pinned when we dialed it over WebRTC.
    pub(crate) fn reaches(&self) -> BTreeSet<Reach> {
        let endpoint = self.endpoint.iter().filter_map(|e| Id::from_hex(e).ok()).map(Reach::Endpoint);
        endpoint.chain(self.dialed.iter().filter(|r| r.starts_with("rtc:")).filter_map(|r| reach(r))).collect()
    }
}

/// How a connection's bytes travel, in the words the traffic view uses.
pub(crate) fn encryption_words(m: &PeerMeta) -> String {
    match m.transport.as_str() {
        "quic" => "encrypted · QUIC, TLS 1.3",
        "webrtc" => "encrypted · WebRTC, DTLS",
        "websocket" if m.local => "stays on this device",
        "websocket" if m.tls => "encrypted · TLS",
        "websocket" => "not encrypted",
        _ => "unknown",
    }
    .into()
}

/// Who is on the other end of a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// We dialed our own home node.
    Home,
    /// A browser or phone dialed us (we serve as a home node).
    Client,
    /// Any other peer.
    Peer,
}

impl Node {
    /// A persistent transport identity, independent of posting keys and the public hello id.
    /// A new key is committed before it is returned; storage failure prevents endpoint startup.
    pub fn transport_secret(&mut self) -> R<[u8; 32]> {
        if self.transport_kp.is_none() {
            self.transport_kp = Some(hoi_crypto::Keypair::generate()?);
            self.mark_keys();
            if let Err(error) = self.commit() {
                self.transport_kp = None;
                self.mark_keys();
                return Err(error);
            }
        }
        Ok(self.transport_kp.as_ref().expect("transport key initialized").secret())
    }

    /// The WebRTC DTLS certificate and key (DER). The host makes one with its WebRTC stack the
    /// first time and hands it over with [`Node::set_rtc_cert`]; its fingerprint is in every
    /// pairing link and invite, so it is kept with the other secrets.
    pub fn rtc_cert(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        self.rtc_cert.clone()
    }

    pub fn set_rtc_cert(&mut self, cert: Vec<u8>, key: Vec<u8>) -> R<()> {
        self.rtc_cert = Some((cert, key));
        self.mark_keys();
        self.commit()
    }

    /// Called by the native host whenever its WebRTC sockets (re)bind: `rtc:<fp>@<addr>` routes.
    pub fn set_rtc_routes(&mut self, routes: Vec<String>) {
        let routes: Vec<String> = routes.into_iter().filter(|r| r.starts_with("rtc:") && valid_peer_hint(r)).take(hoi_types::uri::MAX_PAIR_ROUTES).collect();
        if routes != self.rtc_routes {
            self.rtc_routes = routes;
            self.announce();
        }
    }

    /// Dial candidates for the native transport: explicitly accepted invites, paired devices,
    /// members met before, routes on trial for a public join and, while this node serves
    /// strangers, a few public nodes (crate::nodes). Never default relays. Under the consent
    /// gate (a desktop or a phone), a node whose notice waits for an answer or was declined is
    /// left out, as `rtc_candidates` leaves it out for a browser (PROTOCOL.md §14.5).
    pub fn bootstrap_hints(&self) -> Vec<(String, Vec<String>)> {
        self.bootstrap_hints_with_forwarding(true)
    }

    /// Long-lived transport candidates, excluding temporary links dialed for forwarded pieces.
    pub fn persistent_bootstrap_hints(&self) -> Vec<(String, Vec<String>)> {
        self.bootstrap_hints_with_forwarding(false)
    }

    fn bootstrap_hints_with_forwarding(&self, forwarding: bool) -> Vec<(String, Vec<String>)> {
        let mut peers = BTreeMap::<String, BTreeSet<String>>::new();
        let now = self.now();
        for (c, hints) in &self.state.bootstrap {
            if !self.state.joined.contains_key(c) || self.is_private_community(c) {
                continue;
            }
            for p in hints.peers.iter().filter(|p| !p.starts_with("rtc:")) {
                peers.entry(p.clone()).or_default().extend(hints.relays.iter().cloned());
            }
        }
        for (c, routes) in &self.trial_routes {
            if !self.state.joined.contains_key(c) || self.is_private_community(c) {
                continue;
            }
            let relays = self.state.bootstrap.get(c).map(|h| h.relays.clone()).unwrap_or_default();
            for p in routes.iter().filter(|p| !p.starts_with("rtc:")) {
                peers.entry(p.clone()).or_default().extend(relays.iter().cloned());
            }
        }
        for (c, learned) in &self.state.learned {
            if !self.state.joined.contains_key(c) || self.is_private_community(c) {
                continue;
            }
            let relays = self.state.bootstrap.get(c).map(|h| h.relays.clone()).unwrap_or_default();
            for l in learned.iter().filter(|l| !l.hint.starts_with("rtc:") && now.saturating_sub(l.seen) < LEARNED_TTL) {
                peers.entry(l.hint.clone()).or_default().extend(relays.iter().cloned());
            }
        }
        for p in self.directory_hints() {
            peers.entry(p).or_default();
        }
        // The holders this device's private groups keep their mailboxes on, and while one it
        // owns still needs some, holders and public nodes to choose from (crate::mailbox).
        for r in self.mailbox_dial_routes().into_iter().filter(|r| !r.starts_with("rtc:")) {
            peers.entry(r).or_default();
        }
        // Holders this node, forwarding, has requests waiting for (crate::forward).
        if forwarding {
            for r in self.forward_dial_routes().into_iter().filter(|r| !r.starts_with("rtc:")) {
                peers.entry(r).or_default();
            }
        }
        peers.into_iter().filter(|(p, _)| !self.consent_blocks(p)).map(|(p, rs)| (p, rs.into_iter().collect())).collect()
    }

    /// WebRTC routes from accepted invites, routes on trial for public joins and members met
    /// before. These may supplement a home node that does not carry the community.
    pub fn rtc_candidates(&self) -> Vec<String> {
        let now = self.now();
        let home: BTreeSet<&String> = self.state.home.iter().flat_map(|h| h.rtc.iter()).collect();
        let mut out: Vec<String> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut learned: Vec<&LearnedRoute> = self
            .state
            .learned
            .iter()
            .filter(|(c, _)| self.state.joined.contains_key(c) && !self.is_private_community(c))
            .flat_map(|(_, l)| l.iter())
            .filter(|l| now.saturating_sub(l.seen) < LEARNED_TTL)
            .collect();
        learned.sort_by_key(|l| std::cmp::Reverse(l.seen));
        let usable = |c: &&Id| self.state.joined.contains_key(*c) && !self.is_private_community(c);
        let explicit = self.state.bootstrap.iter().filter(|(c, _)| usable(c)).flat_map(|(_, h)| h.peers.iter());
        let trial = self.trial_routes.iter().filter(|(c, _)| usable(c)).flat_map(|(_, routes)| routes.iter());
        let mailboxes = self.mailbox_dial_routes();
        for r in mailboxes.iter().chain(explicit).chain(trial).chain(learned.into_iter().map(|l| &l.hint)) {
            if r.starts_with("rtc:")
                && !home.contains(r)
                && !self.rtc_routes.contains(r)
                && valid_peer_hint(r)
                && !self.consent_blocks(r)
                && seen.insert(r.clone())
            {
                out.push(r.clone());
            }
        }
        out.truncate(MAX_RTC_CANDIDATES);
        out
    }

    /// Whether this node puts its internet addresses where strangers see them: a headless node
    /// (its operator's server, its flags), or a desktop the person made a public node. Any
    /// other desktop publishes only addresses on this machine and its own network.
    pub(crate) fn shares_internet_routes(&self) -> bool {
        self.platform == "node" || self.is_public_node()
    }

    /// Our routes as strangers, invites and the hints our devices pass on may carry them.
    pub(crate) fn published_routes(&self) -> Vec<String> {
        let mut out = self.own_routes();
        if !self.shares_internet_routes() {
            out.retain(|r| crate::nodes::route_scope(r).is_none_or(|s| s < crate::nodes::Scope::Internet));
        }
        out
    }

    /// This node's own routes, in the order others should try them.
    pub(crate) fn own_routes(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(t) = &self.transport {
            out.extend(t.peers.iter().filter(|p| p.contains('@')).take(OWN_DIRECT).cloned());
            if !t.relays.is_empty() {
                out.push(t.endpoint_id.clone());
            }
        }
        out.extend(self.rtc_routes.iter().take(OWN_RTC).cloned());
        out.retain(|r| valid_peer_hint(r));
        out.truncate(hoi_sync::MAX_ROUTES);
        out
    }

    /// Remembers how to reach a member we just exchanged a community's records with: the address
    /// its authenticated QUIC connection came from, the routes it gave for that same endpoint,
    /// and a couple of WebRTC routes for browsers, but only ones bound to this connection: the
    /// certificate we pinned when we dialed it, or the address its QUIC connection came from.
    /// Anything else would let a peer point newcomers at someone else's address. Routes on
    /// trial that led here have now delivered records, so they are kept from now on.
    fn remember_member(&mut self, peer: &str, community: Id) {
        let Some(p) = self.peers.get(peer) else { return };
        // A peer that publishes no address of its own is remembered nowhere.
        if p.client
            || p.home
            || p.features.contains(hoi_sync::FEATURE_UNLISTED)
            || !self.state.joined.contains_key(&community)
            || self.is_private_community(&community)
        {
            return;
        }
        let reaches = p.meta.reaches();
        let source = p.meta.addr.as_deref().and_then(|a| a.parse::<std::net::SocketAddr>().ok()).map(|a| a.ip().to_canonical());
        let bound =
            |r: &String| rtc_parts(r).is_some_and(|(fp, addr)| reaches.contains(&Reach::Cert(fp.to_string())) || source == Some(addr.ip().to_canonical()));
        let mut hints: Vec<String> = Vec::new();
        if let Some(ep) = &p.meta.endpoint {
            if let Some(addr) = &p.meta.addr {
                hints.push(format!("{ep}@{addr}"));
            }
            hints.extend(p.routes.iter().filter(|r| r.split_once('@').is_some_and(|(id, _)| id == ep)).take(OWN_DIRECT).cloned());
        }
        let tried: Vec<String> = self
            .trial_routes
            .get(&community)
            .map(|t| t.iter().filter(|r| reach(r).is_some_and(|x| reaches.contains(&x))).cloned().collect())
            .unwrap_or_default();
        hints.extend(tried.iter().filter(|r| !r.starts_with("rtc:")).take(OWN_DIRECT).cloned());
        let mut rtc: Vec<String> = Vec::new();
        for r in p.meta.dialed.iter().chain(&p.routes).filter(|r| bound(r)) {
            if rtc.len() < 2 && !rtc.contains(r) {
                rtc.push(r.clone());
            }
        }
        hints.extend(rtc);
        let mut seen = BTreeSet::new();
        hints.retain(|h| valid_peer_hint(h) && !self.rtc_routes.contains(h) && seen.insert(h.clone()));
        if let Some(t) = self.trial_routes.get_mut(&community) {
            t.retain(|r| !tried.contains(r));
        }
        if hints.is_empty() {
            return;
        }
        let now = self.now();
        let list = self.state.learned.entry(community).or_default();
        let before = list.clone();
        list.retain(|l| !hints.contains(&l.hint) && now.saturating_sub(l.seen) < LEARNED_TTL);
        for (i, h) in hints.into_iter().enumerate() {
            list.insert(i, LearnedRoute { hint: h, seen: now });
        }
        list.truncate(MAX_LEARNED);
        // Seen times change on every exchange; write them down at most hourly.
        let changed = before.len() != list.len() || before.iter().zip(list.iter()).any(|(a, b)| a.hint != b.hint || b.seen.saturating_sub(a.seen) > 3600);
        if changed {
            self.mark_state();
        }
    }

    /// Called by the native transport after binding and whenever its advertised address changes.
    pub fn set_transport_address(&mut self, endpoint_id: Option<String>, peers: Vec<String>, relays: Vec<String>) {
        let next = endpoint_id.map(|endpoint_id| {
            let error = self.transport.as_ref().filter(|t| t.endpoint_id == endpoint_id).and_then(|t| t.error.clone());
            TransportStatus { endpoint_id, peers, relays, error }
        });
        if self.transport != next {
            self.transport = next;
            self.announce();
        }
    }

    pub fn set_transport_error(&mut self, error: Option<String>) {
        if let Some(transport) = &mut self.transport {
            transport.error = error;
        }
    }

    /// Route observed by the transport, not inferred from whether a relay is configured.
    pub fn set_peer_route(&mut self, peer: &str, route: &str) {
        if let Some(p) = self.peers.get_mut(peer) {
            p.route = route.to_string();
        }
    }

    pub(crate) fn remember_hints(&mut self, community: hoi_types::Id, peers: &[String], relays: &[String]) {
        if !self.state.joined.contains_key(&community) || self.is_private_community(&community) {
            return;
        }
        let h = self.state.bootstrap.entry(community).or_default();
        // Prefer freshly supplied routes so IP/relay changes cannot get stuck behind a full cache.
        let mut seen = BTreeSet::new();
        let next_peers: Vec<String> =
            peers.iter().chain(&h.peers).filter(|p| valid_peer_hint(p) && seen.insert((*p).clone())).take(hoi_sync::MAX_HINT_PEERS).cloned().collect();
        seen.clear();
        let next_relays: Vec<String> =
            relays.iter().chain(&h.relays).filter(|r| valid_relay_hint(r) && seen.insert((*r).clone())).take(hoi_sync::MAX_HINT_RELAYS).cloned().collect();
        if h.peers == next_peers && h.relays == next_relays {
            return;
        }
        h.peers = next_peers;
        h.relays = next_relays;
        self.mark_state();
    }

    /// Trackers an accepted invite named, kept until the community's own limits say.
    pub(crate) fn remember_trackers(&mut self, community: Id, trackers: &[String]) {
        if trackers.is_empty() || !self.state.joined.contains_key(&community) || self.is_private_community(&community) {
            return;
        }
        let t: Vec<String> = trackers.iter().filter(|t| hoi_types::uri::tracker_ok(t)).take(hoi_types::uri::MAX_TRACKERS).cloned().collect();
        let h = self.state.bootstrap.entry(community).or_default();
        if h.trackers != t {
            h.trackers = t;
            self.mark_state();
        }
    }

    /// Trackers to ask for a community when no route answers: its own limits' first, then those
    /// an invite named. Never any the app or a site chose.
    pub fn trackers_of(&self, community: &Id) -> Vec<String> {
        let own = self.communities.get(community).map(|c| c.limits.trackers.clone()).unwrap_or_default();
        let invited = self.state.bootstrap.get(community).map(|h| h.trackers.clone()).unwrap_or_default();
        let mut seen = BTreeSet::new();
        own.into_iter().chain(invited).filter(|t| seen.insert(t.clone())).take(2 * hoi_types::uri::MAX_TRACKERS).collect()
    }

    /// Whether `endpoint` belongs to a member of a community this node carries, as far as it
    /// knows: connected now and sharing one, or a route of one it learned or was invited with.
    /// What the packet relay's members-only mode admits.
    pub fn relay_member(&self, endpoint: &Id) -> bool {
        let hex = endpoint.hex();
        let names = |r: &String| r.split('@').next() == Some(hex.as_str());
        self.peers.values().any(|p| p.meta.endpoint.as_deref() == Some(hex.as_str()) && p.communities.iter().any(|c| self.state.joined.contains_key(c)))
            || self.state.learned.values().flatten().any(|l| names(&l.hint))
            || self.state.bootstrap.values().any(|h| h.peers.iter().any(names))
    }

    /// Trackers this public node announces itself to: those of the open communities it
    /// carries, at most `max`. A node that isn't public announces nowhere.
    pub fn tracker_targets(&self, max: usize) -> Vec<String> {
        if !self.is_public_node() {
            return Vec::new();
        }
        let mut seen = BTreeSet::new();
        self.state
            .joined
            .keys()
            .filter(|c| !self.is_private_community(c))
            .filter_map(|c| self.communities.get(c))
            .filter(|st| st.limits.admission == hoi_types::Admission::Open)
            .flat_map(|st| st.limits.trackers.iter().cloned())
            .filter(|t| seen.insert(t.clone()))
            .take(max)
            .collect()
    }

    /// Routes a registry or an announcement suggested for a public join. Anyone can announce
    /// routes, so they are only tried: kept in memory for this attempt, freshest first, never
    /// saved, never put in invites or catalogs. One that delivers the community's records is
    /// kept from then on (`remember_member`).
    pub(crate) fn try_routes(&mut self, community: Id, peers: &[String]) {
        if peers.is_empty() || !self.state.joined.contains_key(&community) || self.is_private_community(&community) {
            return;
        }
        let old = self.trial_routes.remove(&community).unwrap_or_default();
        let mut seen = BTreeSet::new();
        let next: Vec<String> =
            peers.iter().chain(&old).filter(|p| valid_peer_hint(p) && seen.insert((*p).clone())).take(hoi_sync::MAX_HINT_PEERS).cloned().collect();
        self.trial_routes.insert(community, next);
    }

    /// Every route this node holds for a community: accepted invites, members met, routes on trial.
    fn routes_of(&self, community: &Id) -> impl Iterator<Item = &String> {
        let bootstrap = self.state.bootstrap.get(community).into_iter().flat_map(|h| h.peers.iter());
        let learned = self.state.learned.get(community).into_iter().flat_map(|l| l.iter().map(|l| &l.hint));
        bootstrap.chain(learned).chain(self.trial_routes.get(community).into_iter().flatten())
    }

    /// The public communities we are in whose routes reach the other end of this connection.
    /// Those are what we came to it for; a stranger is told them in the clear.
    fn routed_here(&self, peer: &str) -> BTreeSet<Id> {
        let Some(p) = self.peers.get(peer) else { return BTreeSet::new() };
        let reaches = p.meta.reaches();
        if reaches.is_empty() {
            return BTreeSet::new();
        }
        let leads_here = |c: &Id| self.routes_of(c).any(|r| reach(r).is_some_and(|x| reaches.contains(&x)));
        self.state.joined.keys().filter(|c| !self.is_private_community(c) && leads_here(c)).copied().collect()
    }

    /// Whether a peer we dialed is believed about what it lists of a community. One a route only
    /// on trial led to (a registry's or an announcement's word) proves itself by delivering
    /// records instead, unless a route we trusted before (an accepted invite, a member met)
    /// reaches it too.
    fn believed(&self, peer: &str, community: &Id) -> bool {
        let Some(p) = self.peers.get(peer).filter(|p| !p.meta.incoming) else { return false };
        let reaches = p.meta.reaches();
        let leads_here = |r: &String| reach(r).is_some_and(|x| reaches.contains(&x));
        let bootstrap = self.state.bootstrap.get(community).into_iter().flat_map(|h| h.peers.iter());
        let learned = self.state.learned.get(community).into_iter().flat_map(|l| l.iter().map(|l| &l.hint));
        bootstrap.chain(learned).any(leads_here) || !self.trial_routes.values().flatten().any(leads_here)
    }

    pub(crate) fn community_hints(&self, community: hoi_types::Id) -> (Vec<String>, Vec<String>) {
        self.community_routes(community, 0)
    }

    /// The routes an invite carries: [`Node::community_hints`], then `entry_nodes` public nodes
    /// this device reached, for an open community, so the invite leads into the network even
    /// when none of its members' nodes answers.
    pub(crate) fn invite_hints(&self, community: hoi_types::Id) -> (Vec<String>, Vec<String>) {
        self.community_routes(community, INVITE_ENTRY_NODES)
    }

    fn community_routes(&self, community: hoi_types::Id, entry_nodes: usize) -> (Vec<String>, Vec<String>) {
        if self.is_private_community(&community) {
            return (Vec::new(), Vec::new());
        }
        // A public node's own address is tried first, then the invite we came with, then the
        // nodes we exchanged its records with. A node that isn't public puts its own routes,
        // on this machine and its network only, last: an invite leads to public nodes.
        // Never routes on trial: invites and catalogs pass on only routes that delivered records.
        let own = self.published_routes();
        let mut peers = if self.shares_internet_routes() { own.clone() } else { Vec::new() };
        if let Some(t) = &self.transport
            && !peers.contains(&t.endpoint_id)
        {
            peers.insert(0, t.endpoint_id.clone());
        }
        // Public nodes connected now that hold it or were asked to: a member's browser has no
        // address of its own, and a community made a minute ago has delivered nothing back yet,
        // so without them its invite would lead nowhere.
        peers.extend(self.carrier_routes(&community));
        peers.extend(self.state.bootstrap.get(&community).map(|h| h.peers.clone()).unwrap_or_default());
        let now = self.now();
        let mut learned: Vec<&LearnedRoute> =
            self.state.learned.get(&community).map(|l| l.iter().filter(|l| now.saturating_sub(l.seen) < LEARNED_TTL).collect()).unwrap_or_default();
        learned.sort_by_key(|l| std::cmp::Reverse(l.seen));
        peers.extend(learned.into_iter().map(|l| l.hint.clone()));
        if !self.shares_internet_routes() {
            peers.extend(own);
        }
        if entry_nodes > 0 && self.carriable(&community).is_ok() {
            peers.extend(self.entry_routes(entry_nodes));
        }
        let mut relays = self.state.bootstrap.get(&community).map(|h| h.relays.clone()).unwrap_or_default();
        if let Some(t) = &self.transport {
            relays.splice(0..0, t.relays.iter().cloned());
        }
        let mut seen = BTreeSet::new();
        peers.retain(|p| valid_peer_hint(p) && seen.insert(p.clone()));
        peers.truncate(hoi_sync::MAX_HINT_PEERS);
        seen.clear();
        relays.retain(|r| valid_relay_hint(r) && seen.insert(r.clone()));
        relays.truncate(hoi_sync::MAX_HINT_RELAYS);
        (peers, relays)
    }

    /// Every address of our own, each once, and who hears it: our devices always, strangers and
    /// invites what we publish, the node link while we are a public node.
    fn exposure(&self) -> Vec<crate::dto::Exposure> {
        use crate::nodes::{route_addr, route_scope};
        let published: BTreeSet<std::net::SocketAddr> = self.published_routes().iter().filter_map(|r| route_addr(r)).collect();
        let linked = self.node_link().is_some();
        let forwarded: BTreeSet<String> = self.port_mapping.iter().flat_map(|m| m.public.iter().cloned()).collect();
        let mut seen = BTreeSet::new();
        self.own_routes()
            .iter()
            .filter_map(|r| Some((route_addr(r)?, route_scope(r)?)))
            .filter(|(a, _)| seen.insert(*a))
            .map(|(a, scope)| {
                let mut told = vec!["devices".to_string()];
                if published.contains(&a) {
                    told.push("strangers".into());
                    if linked {
                        told.push("node-link".into());
                    }
                }
                crate::dto::Exposure { address: a.to_string(), scope: scope.word().into(), from_router: forwarded.contains(&a.to_string()), told }
            })
            .collect()
    }

    /// The addresses `peers` dial, each once: whose it is and how far it reaches. `entry`: the
    /// routes of `community`'s invite that only lead into the network (`invite_hints`).
    pub(crate) fn invite_addresses(&self, peers: &[String], community: Option<hoi_types::Id>) -> Vec<crate::dto::InviteAddress> {
        let mine: BTreeSet<std::net::SocketAddr> =
            self.own_routes().iter().chain(self.state.home.iter().flat_map(|h| h.rtc.iter())).filter_map(|r| crate::nodes::route_addr(r)).collect();
        let holding: BTreeSet<std::net::SocketAddr> = match community {
            Some(c) => self.community_hints(c).0.iter().filter_map(|r| crate::nodes::route_addr(r)).collect(),
            None => peers.iter().filter_map(|r| crate::nodes::route_addr(r)).collect(),
        };
        let mut seen = BTreeSet::new();
        peers
            .iter()
            .filter_map(|r| Some((crate::nodes::route_addr(r)?, crate::nodes::route_scope(r)?)))
            .filter(|(a, _)| seen.insert(*a))
            .map(|(a, scope)| crate::dto::InviteAddress {
                address: a.to_string(),
                scope: scope.word().into(),
                whose: if mine.contains(&a) {
                    "yours"
                } else if holding.contains(&a) {
                    "node"
                } else {
                    "entry"
                }
                .into(),
            })
            .collect()
    }

    /// Our hello to `peer`; `named` is what [`Node::routed_here`] found for it.
    fn hello(&self, peer: &str, named: &BTreeSet<Id>) -> Msg {
        let p = self.peers.get(peer);
        let role = p.map_or(Role::Peer, |p| p.role());
        // Our name is for our own devices and our home node. Strangers (members' nodes, public
        // carriers) get no name, and a browser gives them a fresh random id per connection, so
        // one carrier cannot tie a person's visits together across sessions.
        let own = role != Role::Peer;
        let node = match self.transport.as_ref().and_then(|t| Id::from_hex(&t.endpoint_id).ok()) {
            Some(endpoint) => endpoint,
            None if own => self.state.node_id.unwrap_or(Id::ZERO),
            None => hoi_crypto::random_bytes::<32>().map(Id).unwrap_or(Id::ZERO),
        };
        let public = self.state.joined.keys().filter(|c| !self.is_private_community(c));
        // Our devices and home node hear every public community we are in. A stranger hears in
        // the clear only those whose routes led to it (a carrier learns what a member came to it
        // for), and the rest as hashes salted for this connection: a peer that holds a community
        // too finds it, nobody learns a key it didn't know.
        let (communities, salt, hashed) = if own {
            (public.copied().take(hoi_sync::MAX_HELLO).collect(), None, Vec::new())
        } else {
            let salt = p.and_then(|p| p.salt);
            let mut hashed: Vec<Id> =
                salt.map(|s| public.filter(|c| !named.contains(c)).map(|c| hoi_sync::community_hash(&s, c)).collect()).unwrap_or_default();
            // Sorted, so their order says nothing about the keys behind them.
            hashed.sort();
            hashed.truncate(hoi_sync::MAX_HELLO);
            (named.iter().copied().take(hoi_sync::MAX_HELLO).collect(), salt, hashed)
        };
        let token = match role {
            Role::Home => self.state.home.as_ref().map(|h| h.token.clone()),
            Role::Client => p.and_then(|p| p.issued.clone()),
            Role::Peer => None,
        };
        // Our devices may show our pairing link to their next device, and seal `link` messages.
        let (pairing, link_key) = if role == Role::Client { (self.state.serve_token.clone(), self.link_key()) } else { (None, None) };
        // A public node tells strangers who runs it before they say anything.
        let (notice_hash, notice) = match &self.operator_notice {
            Some(n) if role == Role::Peer && self.is_public_node() => (Some(n.hash), Some(n.bytes.clone())),
            _ => (None, None),
        };
        Msg::Hello {
            v: hoi_sync::SYNC_VERSION,
            node,
            name: if own { self.node_label() } else { None },
            token,
            communities,
            salt,
            hashed,
            serves: self.serving.is_some(),
            hints: if role == Role::Peer {
                Vec::new()
            } else {
                self.state
                    .joined
                    .keys()
                    .filter(|c| !self.is_private_community(c))
                    .take(hoi_sync::MAX_HINT_COMMUNITIES)
                    .filter_map(|c| {
                        let (peers, relays) = self.community_hints(*c);
                        (!peers.is_empty()).then_some(hoi_sync::Hints { community: *c, peers, relays })
                    })
                    .collect()
            },
            // Our devices and home node hear every route; anyone else what we publish.
            routes: if own { self.own_routes() } else { self.published_routes() },
            features: self.features(),
            pairing,
            link_key,
            notice_hash,
            notice,
            holder: self.holder_hello(),
            forwards: self.forwards_pieces(),
            rebuilds: self.rebuilds_for(peer),
        }
    }

    /// Queues our hello for `peer`, remembering which communities it named in the clear.
    fn send_hello(&mut self, peer: &str) {
        let named = self.routed_here(peer);
        let h = self.hello(peer, &named);
        if let Some(p) = self.peers.get_mut(peer) {
            p.named = named;
            p.hello_sent = true;
        }
        self.send_msg(peer, h);
        // on a WebRTC link this node accepted, the binding follows the hello (crate::bind)
        self.send_bind(peer);
    }

    /// The host said how a connection travels after our hello was queued. If that changes what
    /// the hello names, a hello still waiting to go out is replaced; one already sent is followed
    /// by a new one, which replaces it (PROTOCOL.md §14).
    pub(crate) fn rehello(&mut self, peer: &str) {
        let named = self.routed_here(peer);
        let bytes = self.hello(peer, &named).encode();
        if let Some(p) = self.peers.get_mut(peer) {
            p.named = named;
        }
        let waiting = self.outbox.iter_mut().find(|(to, _)| to == peer).filter(|(_, b)| matches!(Msg::decode(b), Ok(Msg::Hello { .. })));
        match waiting {
            Some(slot) => slot.1 = bytes,
            None => self.queue_out(peer.to_string(), bytes, 0, 0),
        }
    }

    /// Media bytes from a peer, on the sync link or over iroh-blobs (crate::blobs). False when
    /// the link should close: the peer sent bytes that don't hash to what it said, too often.
    pub(crate) fn take_media(&mut self, peer: &str, id: Id, data: Vec<u8>) -> bool {
        // Private media arrives inside authenticated group messages, never raw sync.
        if !self.store.media_users(&id).any(|s| self.plaintext_forwardable(s)) {
            return true;
        }
        let size = data.len() as u64;
        let had = self.store.has_blob(&id);
        if !self.on_blob(id, data) {
            if self.peer_scores.penalize(peer, 5.0) {
                self.send_msg(peer, Msg::Bye { reason: "media that doesn't match its hash".into() });
                return false;
            }
            return true;
        }
        if !had && self.store.has_blob(&id) {
            self.traffic.media_in += size;
        }
        // A reference alone cannot disclose private bytes. Receiving the entire
        // hash-verified public blob independently proves those bytes are public.
        if self.store.has_blob(&id) && self.state.private_blobs.remove(&id) {
            self.mark_state();
        }
        self.pay_owed(id, peer);
        self.request_missing_media();
        true
    }

    fn features(&self) -> Vec<String> {
        let mut fx = vec![
            hoi_sync::FEATURE_LINK.to_string(),
            hoi_sync::FEATURE_HOME_SCATTER.to_string(),
            hoi_sync::FEATURE_CATALOG.to_string(),
            hoi_sync::FEATURE_PROFILE.to_string(),
            hoi_sync::FEATURE_PROFILE_COLOR.to_string(),
            hoi_sync::FEATURE_NODES.to_string(),
            hoi_sync::FEATURE_DIRECTORY.to_string(),
            hoi_sync::FEATURE_RANGE.to_string(),
        ];
        if self.scatter.pieces.as_ref().is_some_and(|store| store.durable_drops()) {
            // Keep the capability inside the existing 16-feature limit. It is also signed
            // in the holder key so a forwarding peer cannot upgrade a legacy holder.
            fx.insert(0, hoi_sync::FEATURE_DROP_JOURNAL.to_string());
        }
        let plan = self.plan();
        if hoi_crypto::scatter::size::size(10, 0, plan.places() as u32)
            .is_some_and(|s| plan.forwarder_places() >= 2 && plan.forwarder_places() * (s.k - 1) >= s.n)
        {
            fx.push("scatter-store".into());
        }
        if self.can_relay() {
            // A copy that outlives a tab: a desktop app or hoi-node, whatever it is doing.
            fx.push(hoi_sync::FEATURE_KEEPS.to_string());
        }
        if self.accepts_carry() {
            fx.push(hoi_sync::FEATURE_CARRY.to_string());
        }
        if !self.shares_internet_routes() {
            fx.push(hoi_sync::FEATURE_UNLISTED.to_string());
        }
        if self.answers_find() {
            fx.push(hoi_sync::FEATURE_FIND.to_string());
        }
        if self.runs_dht() {
            fx.push(hoi_sync::FEATURE_DHT.to_string());
        }
        if self.serves_blobs() {
            fx.push(hoi_sync::FEATURE_BLOBS.to_string());
        }
        if self.passes_signals() {
            fx.push(hoi_sync::FEATURE_SIGNAL.to_string());
        }
        if self.takes_direct() {
            fx.push(hoi_sync::FEATURE_DIRECT.to_string());
        }
        if fx.len() > hoi_sync::SEND_FEATURES {
            // Placement offload is optional; preserve protocol and durability features when
            // a holder offers every service understood by older 16-feature readers.
            fx.retain(|feature| feature != "scatter-store");
        }
        debug_assert!(fx.len() <= hoi_sync::SEND_FEATURES);
        fx.truncate(hoi_sync::SEND_FEATURES);
        fx
    }

    /// The key `link` messages between this node and its devices are sealed with.
    pub(crate) fn link_key(&self) -> Option<[u8; 32]> {
        self.state.link_key.as_deref().and_then(|k| hoi_types::uri::unb64(k).ok()).and_then(|k| k.try_into().ok())
    }

    /// A device that dialed us shows its own credential, or the pairing link's token. The link is
    /// good for one device: that device gets a credential of its own and the link is replaced, so
    /// a link left in a screenshot or a browser history pairs nobody else. Returns the new
    /// credential when one was issued; `None` turns the device away.
    fn admit_device(&mut self, token: Option<&str>) -> Option<Option<String>> {
        use subtle::ConstantTimeEq;
        let theirs = token?;
        let presented = digest(CREDENTIAL_CONTEXT, theirs);
        let now = self.now();
        // Every entry is compared, in constant time, so timing doesn't tell which one matched.
        let mut found = None;
        for (i, d) in self.state.paired.iter().enumerate() {
            if bool::from(d.credential.0.ct_eq(&presented.0)) {
                found = Some(i);
            }
        }
        if let Some(i) = found {
            // Seen times only decide who goes first when the list is full: written at most hourly.
            let d = &mut self.state.paired[i];
            if now.saturating_sub(d.seen) > 3600 {
                d.seen = now;
                self.mark_state();
            }
            return Some(None);
        }
        let link = self.state.serve_token.as_ref()?;
        if link.len() != theirs.len() || !bool::from(link.as_bytes().ct_eq(theirs.as_bytes())) {
            return None;
        }
        let credential = hoi_types::uri::b64(&hoi_crypto::random_bytes::<32>().ok()?);
        let next = hoi_types::uri::b64(&hoi_crypto::random_bytes::<18>().ok()?);
        if self.state.paired.len() >= MAX_PAIRED
            && let Some(i) = self.state.paired.iter().enumerate().min_by_key(|(_, d)| d.seen).map(|(i, _)| i)
        {
            self.state.paired.remove(i);
        }
        self.state.paired.push(PairedDevice { credential: digest(CREDENTIAL_CONTEXT, &credential), seen: now });
        self.state.serve_token = Some(next);
        self.mark_state();
        Some(Some(credential))
    }

    /// What our home node tells only its own devices: our credential once we spent its pairing
    /// link, the link to show our next device, and the key for `link` messages.
    fn take_home_secrets(&mut self, token: Option<String>, pairing: Option<String>, link_key: Option<[u8; 32]>) {
        let Some(h) = self.state.home.as_mut() else { return };
        let mut changed = false;
        if let Some(credential) = token.filter(|t| !t.is_empty() && *t != h.token) {
            h.spent = Some(digest(SPENT_LINK_CONTEXT, &h.token));
            h.token = credential;
            changed = true;
        }
        if pairing.is_some() && h.next != pairing {
            h.next = pairing;
            changed = true;
        }
        let key = link_key.map(|k| hoi_types::uri::b64(&k));
        if key.is_some() && h.link_key != key {
            h.link_key = key;
            changed = true;
        }
        if changed {
            self.mark_state();
        }
    }

    /// The home node's WebRTC routes follow its address changes. Until it delivered records, only
    /// new addresses for a certificate the pairing already named are taken.
    fn refresh_home_rtc(&mut self, routes: &[String]) {
        let Some(h) = self.state.home.as_mut() else { return };
        let known: BTreeSet<String> = h.rtc.iter().filter_map(|r| rtc_parts(r)).map(|(fp, _)| fp.to_string()).collect();
        let rtc: Vec<String> = routes
            .iter()
            .filter(|r| r.starts_with("rtc:") && (h.proven || rtc_parts(r).is_some_and(|(fp, _)| known.contains(fp))))
            .take(hoi_types::uri::MAX_PAIR_ROUTES)
            .cloned()
            .collect();
        if !rtc.is_empty() && h.rtc != rtc {
            h.rtc = rtc;
            self.mark_state();
        }
    }

    /// Our home node delivered valid records: it is the node we meant to pair with, or at least
    /// one that holds our communities. From now on its routes count.
    fn home_proven(&mut self, peer: &str) {
        if let Some(h) = self.state.home.as_mut().filter(|h| !h.proven) {
            h.proven = true;
            self.mark_state();
        }
        let Some(p) = self.peers.get_mut(peer) else { return };
        let (hints, routes, advertised) = (std::mem::take(&mut p.pending_hints), p.routes.clone(), p.communities.clone());
        for h in hints.into_iter().filter(|h| advertised.contains(&h.community)) {
            self.remember_hints(h.community, &h.peers, &h.relays);
        }
        self.refresh_home_rtc(&routes);
    }

    /// What other devices call this node: "mira's computer", a headless node's own name, or,
    /// with no key here (the desktop companion), the name the host gave it.
    fn node_label(&self) -> Option<String> {
        let Some(me) = self.state.me.as_ref() else { return self.node_name.clone() };
        Some(if self.platform == "node" { me.name.clone() } else { format!("{}'s {}", me.name, self.platform_noun()) })
    }

    /// A name for this node while it holds no key, e.g. the computer's name. Shown to your devices.
    pub fn set_node_name(&mut self, name: Option<String>) {
        self.node_name = name.and_then(|n| crate::api::clean_name(&n).ok());
    }

    fn platform_noun(&self) -> &'static str {
        match self.platform.as_str() {
            "desktop" => "computer",
            "mobile" => "phone",
            "node" => "home node",
            _ => "browser",
        }
    }

    /// Queues a message, counting what it carries for the traffic view.
    pub(crate) fn send_msg(&mut self, peer: &str, m: Msg) {
        let (records, media) = match &m {
            Msg::Recs { recs } => (recs.len() as u64, 0),
            Msg::Blob { data, .. } => (0, data.len() as u64),
            _ => (0, 0),
        };
        self.queue_out(peer.to_string(), m.encode(), records, media);
    }

    pub(crate) fn queue_out(&mut self, peer: String, bytes: Vec<u8>, records: u64, media: u64) {
        let now = self.now();
        self.count_out(&peer, bytes.len(), records, media, now);
        self.outbox.push((peer, bytes));
    }

    /// The host opened a connection. Queues our hello, except to a device that dialed us: it
    /// hears nothing, not even that a hoi node listens here, before it showed its credential.
    pub fn net_connect(&mut self, peer: &str, role: Role) {
        // Under the consent gate a stranger we dialed hears nothing until its hello said who runs
        // it; whether we dialed it is known at `set_peer_meta`, a moment later (crate::notice).
        let awaiting = self.consent_gate && role == Role::Peer;
        self.peers.insert(
            peer.to_string(),
            PeerState {
                communities: BTreeSet::new(),
                name: None,
                serves: false,
                client: role == Role::Client,
                home: role == Role::Home,
                route: if role == Role::Peer { "unknown" } else { "websocket" }.into(),
                said_hello: false,
                sent_have: BTreeSet::new(),
                wants: BTreeMap::new(),
                pending_wants: BTreeMap::new(),
                ranging: BTreeMap::new(),
                sync_progress: None,
                owed: BTreeSet::new(),
                msgs: 0,
                minute: 0,
                meta: PeerMeta::for_role(role),
                routes: Vec::new(),
                features: BTreeSet::new(),
                catalog: BTreeMap::new(),
                sent_catalog: None,
                catalog_msgs: 0,
                catalog_minute: 0,
                gossip_cursor: None,
                gossip_ids: Vec::new(),
                gossip_at: 0,
                listed_new: (0, 0),
                traffic: crate::traffic::PeerTraffic { since: self.now(), ..Default::default() },
                link_msgs: 0,
                link_minute: 0,
                offers: BTreeMap::new(),
                carry_asked: BTreeSet::new(),
                carry_msgs: 0,
                carry_minute: 0,
                carry_sent: 0,
                carry_sent_minute: 0,
                salt: if role == Role::Peer { hoi_crypto::random_bytes::<16>().ok() } else { None },
                named: BTreeSet::new(),
                their_hashes: None,
                issued: None,
                pending_hints: Vec::new(),
                sent_nodes: None,
                nodes_msgs: 0,
                nodes_minute: 0,
                awaiting_hello: awaiting,
                operator: None,
                scatter: Default::default(),
                node_key: None,
                hello_sent: false,
                bind_sent: false,
                bound: false,
                blobs_refused: BTreeSet::new(),
            },
        );
        if role == Role::Home {
            self.net_error = None;
        }
        if role != Role::Client && !awaiting {
            self.send_hello(peer);
        }
    }

    /// What the host knows about a connection; call right after [`Node::net_connect`].
    pub fn set_peer_meta(&mut self, peer: &str, meta: PeerMeta) {
        let Some(p) = self.peers.get_mut(peer) else { return };
        if meta.transport == "webrtc" || (meta.transport == "websocket" && !meta.local) {
            p.route = meta.transport.clone();
        }
        let direct = meta.direct.is_some();
        p.meta = meta;
        // a WebRTC link learned its channel after our hello went out: the binding follows it
        if p.hello_sent {
            self.send_bind(peer);
        }
        if direct {
            self.direct_opened(peer);
        }
        let Some(p) = self.peers.get_mut(peer) else { return };
        if p.awaiting_hello {
            if p.meta.incoming {
                // It dialed us: we are the node here, and it hears our hello at once.
                p.awaiting_hello = false;
                self.send_hello(peer);
            }
            // We dialed it: nothing until its hello (crate::notice).
            return;
        }
        // Now we know where this connection goes: a stranger hears which communities we came for.
        if p.role() == Role::Peer && self.routed_here(peer) != self.peers[peer].named {
            self.rehello(peer);
        }
    }

    /// Hosts reconnect a stalled catch-up even if unrelated traffic keeps the socket alive.
    /// A healthy idle connection has no outstanding record IDs and is never stale here.
    pub fn sync_stalled(&self, peer: &str) -> bool {
        self.peers
            .get(peer)
            .is_some_and(|p| p.wants.values().any(|(_, ids)| !ids.is_empty()) && p.sync_progress.is_some_and(|at| self.now().saturating_sub(at) >= 120))
    }

    pub fn net_disconnect(&mut self, peer: &str) {
        self.consent_closed.remove(peer);
        self.blobs.trying.retain(|_, (p, _)| p != peer);
        let was_direct = self.peers.get(peer).is_some_and(|p| p.meta.direct.is_some());
        if let Some(p) = self.peers.remove(peer) {
            for c in p.catalog.keys() {
                self.mark_changed(*c);
            }
        }
        self.peer_scores.forget(peer);
        self.link_peer_gone(peer);
        self.scatter_disconnect(peer);
        self.signal_disconnect(peer);
        if was_direct {
            self.offer_direct();
        }
    }

    pub fn set_net_error(&mut self, e: Option<String>) {
        self.net_error = e;
    }

    /// The host reports the VPN it was told to use only; `None` without one.
    pub fn set_vpn_status(&mut self, v: Option<crate::dto::VpnStatus>) {
        if self.vpn_status != v {
            self.vpn_status = v;
            self.settle_dirty();
        }
    }

    /// The host's port mapper reports what the router said; `None` while nothing is asked.
    pub fn set_port_mapping(&mut self, m: Option<crate::dto::PortMapping>) {
        if self.port_mapping != m {
            self.port_mapping = m;
            self.settle_dirty();
        }
    }

    pub fn take_outbox(&mut self) -> Vec<(String, Vec<u8>)> {
        // RFC-0001: network deletion cannot outrun its recoverable authority and targets.
        if self.deletions.dirty && self.commit().is_err() {
            return Vec::new();
        }
        self.filter_rebuilt_outbox();
        std::mem::take(&mut self.outbox)
    }

    /// Handles one message. Returns false when the host should drop the connection.
    pub fn net_receive(&mut self, peer: &str, bytes: &[u8]) -> bool {
        let keep = self.net_receive_inner(peer, bytes);
        self.settle_dirty();
        keep
    }

    fn net_receive_inner(&mut self, peer: &str, bytes: &[u8]) -> bool {
        let now = self.now();
        if !self.peers.contains_key(peer) {
            return false;
        }
        self.count_in(peer, bytes.len(), now);
        // Someone else's device that dialed in: only while relaying is on and under today's cap.
        // A direct link between browsers is the person's own choice (crate::signal).
        if self.peers.get(peer).is_some_and(|p| !p.client && !p.home && p.meta.incoming && p.meta.direct.is_none()) && !self.accepts_members() {
            let reason = if self.state.settings.relay { "this node is done relaying for today" } else { "this node isn't relaying right now" };
            self.send_msg(peer, Msg::Bye { reason: reason.into() });
            return false;
        }
        let Some(st) = self.peers.get_mut(peer) else { return false };
        let client = st.client;
        let said_hello = st.said_hello;
        if now / 60 != st.minute {
            st.minute = now / 60;
            st.msgs = 0;
        }
        st.msgs += 1;
        if st.msgs > MSGS_PER_MINUTE && self.peer_scores.penalize(peer, 1.0) {
            self.send_msg(peer, Msg::Bye { reason: "too many messages".into() });
            return false;
        }
        let msg = match Msg::decode(bytes) {
            Ok(m) => m,
            Err(_) => return !self.peer_scores.penalize(peer, 5.0),
        };
        if !said_hello && !matches!(msg, Msg::Hello { .. } | Msg::Bye { .. }) {
            // everything starts with a hello
            return false;
        }
        if said_hello && self.awaits_bind(peer) && !matches!(msg, Msg::Hello { .. } | Msg::Bind { .. } | Msg::Bye { .. }) {
            // a node we dialed over WebRTC binds the channel to its key before anything else
            return false;
        }
        match msg {
            Msg::Hello {
                v,
                node,
                token,
                communities,
                salt,
                hashed,
                name,
                serves,
                hints,
                routes,
                features,
                pairing,
                link_key,
                notice_hash,
                notice,
                holder,
                forwards,
                rebuilds,
            } => {
                // A device that dialed us shows its credential before anything else: a missing or
                // wrong one ends the link without a word, so a web page can't even learn that a
                // home node listens here. A link that showed one stays trusted.
                let mut spent = false;
                if client && !said_hello {
                    let Some(issued) = self.admit_device(token.as_deref()) else { return false };
                    spent = issued.is_some();
                    if let Some(p) = self.peers.get_mut(peer) {
                        p.issued = issued;
                    }
                }
                if v != hoi_sync::SYNC_VERSION {
                    self.send_msg(peer, Msg::Bye { reason: format!("this node speaks sync version {}", hoi_sync::SYNC_VERSION) });
                    return false;
                }
                // A person's device reads a stranger's operator notice before it says anything:
                // a notice not accepted yet ends the link here, to be read first.
                if !client && self.consent_gate && self.peers.get(peer).is_some_and(|p| p.role() == Role::Peer) {
                    match self.consent_check(peer, &node, notice_hash.as_ref(), notice.as_deref()) {
                        crate::notice::Consent::Proceed => {}
                        crate::notice::Consent::Close => return false,
                        crate::notice::Consent::Penalize => return !self.peer_scores.penalize(peer, 5.0),
                    }
                }
                // A peer cannot opt private groups back into plaintext sync by guessing IDs.
                let mut communities: BTreeSet<Id> = communities.into_iter().filter(|c| !self.is_private_community(c)).collect();
                // Salted hashes find only communities we hold too; any other key stays unknown.
                // They are kept for this connection, so one we take on later is found as well.
                let their_hashes = salt.map(|salt| (salt, hashed.into_iter().collect::<BTreeSet<Id>>()));
                if let Some((salt, hashed)) = &their_hashes {
                    let shared: Vec<Id> = self
                        .state
                        .joined
                        .keys()
                        .filter(|c| !self.is_private_community(c) && hashed.contains(&hoi_sync::community_hash(salt, c)))
                        .copied()
                        .collect();
                    communities.extend(shared);
                }
                if client {
                    // A home node keeps your communities warm: join what your devices are in.
                    let now = self.now();
                    let mut joined_any = false;
                    for c in &communities {
                        if !self.state.joined.contains_key(c) && self.state.joined.len() < MAX_CLIENT_COMMUNITIES {
                            self.state.joined.insert(*c, Joined { at: now, pinning: true });
                            self.mark_state();
                            joined_any = true;
                        }
                    }
                    // The device showed its credential: now it gets our hello (with its own
                    // credential if it just spent the link), with whatever it made us join.
                    self.send_hello(peer);
                    if joined_any {
                        self.announce();
                    }
                    if spent {
                        // Our other devices show the next pairing link from now on.
                        let others: Vec<String> =
                            self.peers.iter().filter(|(k, p)| p.client && p.said_hello && k.as_str() != peer).map(|(k, _)| k.clone()).collect();
                        for other in others {
                            self.send_hello(&other);
                        }
                    }
                }
                let routes: Vec<String> = routes.into_iter().filter(|r| valid_peer_hint(r)).collect();
                let hints: Vec<hoi_sync::Hints> = hints.into_iter().filter(|h| communities.contains(&h.community)).collect();
                // Unpaired internet peers cannot make this node dial arbitrary addresses. A
                // token-authenticated device can supply explicit invite routes; so can our home
                // node, once it delivered records: a pairing link could have come from anyone.
                if client {
                    for h in hints {
                        self.remember_hints(h.community, &h.peers, &h.relays);
                    }
                } else if self.peers.get(peer).is_some_and(|p| p.home) {
                    self.take_home_secrets(token, pairing, link_key);
                    if self.state.home.as_ref().is_some_and(|h| h.proven) {
                        for h in hints {
                            self.remember_hints(h.community, &h.peers, &h.relays);
                        }
                    } else if let Some(p) = self.peers.get_mut(peer) {
                        p.pending_hints = hints;
                    }
                    self.refresh_home_rtc(&routes);
                }
                // Who runs it, when its notice (checked against its hash) names them: Warmth
                // counts nodes of one operator as one place.
                let operator = match (&notice_hash, &notice) {
                    (Some(h), Some(bytes)) if hoi_types::notice::hash_of(bytes) == *h => crate::warmth::operator_key(bytes),
                    _ => None,
                };
                if let Some(p) = self.peers.get_mut(peer) {
                    p.operator = operator;
                    p.communities = communities;
                    p.their_hashes = their_hashes;
                    // A person can leave, cancel or retry a public join on the same pipe.
                    // A later re-subscription needs a fresh inventory, not the old sent flag.
                    p.sent_have.retain(|c| p.communities.contains(c));
                    p.offers.retain(|c, _| p.communities.contains(c));
                    p.name = name;
                    p.serves = serves && p.home;
                    p.said_hello = true;
                    p.node_key = Some(node);
                    p.routes = routes;
                    p.features = features.into_iter().collect();
                }
                self.note_holder_key(peer, holder);
                if let Some(p) = self.peers.get_mut(peer) {
                    p.scatter.forwards = forwards;
                    // only a home node rebuilds for us
                    p.scatter.rebuilds = rebuilds && p.home;
                }
                // requests this node holds for that holder go out now
                self.forward_flush();
                // Our hello waited for theirs (the consent gate): it goes first, before anything else.
                if self.peers.get(peer).is_some_and(|p| p.awaiting_hello) {
                    if let Some(p) = self.peers.get_mut(peer) {
                        p.awaiting_hello = false;
                    }
                    self.send_hello(peer);
                }
                self.send_haves(peer);
                self.send_catalog(peer);
                self.send_nodes(peer);
                self.send_link_request();
                self.auto_request_carry(peer);
                self.after_hello_find(peer);
                self.refresh_here();
            }
            Msg::Have { community, ids } => {
                if !self.state.joined.contains_key(&community) || self.is_private_community(&community) {
                    return true;
                }
                // A peer we dialed (its endpoint authenticated, from a route we already trusted)
                // that lists records we hold besides the genesis is a member worth pointing
                // newcomers at. A peer that dialed us, or that a route only on trial led to,
                // proves it by delivering records instead.
                let genesis = self.store.genesis_of(&community).map(|g| g.id);
                let proves_records = self.believed(peer, &community) && ids.iter().any(|i| Some(*i) != genesis && self.store.contains(i));
                // What the peer holds of this community's live window, for Warmth: everything it
                // listed that is not gone here (deleted or removed records are nobody's copy).
                let listed = ids.iter().filter(|i| Some(**i) != genesis && !self.store.is_deleted(i) && !self.state.skeletons.contains_key(i)).count() as u64;
                let lists_genesis = genesis.is_some_and(|g| ids.contains(&g));
                let missing: BTreeSet<Id> =
                    ids.into_iter().filter(|i| !self.store.contains(i) && !self.store.is_deleted(i) && !self.state.skeletons.contains_key(i)).collect();
                if let Some(p) = self.peers.get_mut(peer) {
                    let other_pending: usize = p.pending_wants.iter().filter(|(c, _)| **c != community).map(|(_, ids)| ids.len()).sum::<usize>()
                        + p.wants.iter().filter(|(c, _)| **c != community).map(|(_, (_, ids))| ids.len()).sum::<usize>();
                    if other_pending + missing.len() > hoi_sync::MAX_HAVE {
                        self.send_msg(peer, Msg::Bye { reason: "too much pending catch-up".into() });
                        return false;
                    }
                    p.pending_wants.insert(community, missing);
                    p.wants.insert(community, (now, BTreeSet::new()));
                    p.offers.insert(community, crate::node::Offer { listed, given: 0, delivered: 0, genesis: lists_genesis });
                }
                if proves_records {
                    self.remember_member(peer, community);
                }
                // A member that lists the genesis holds a copy too: a community carried on
                // request that is all definition so far (made a minute ago, nothing posted yet)
                // is not empty while one is around, and its invites keep working.
                // Once it has posts, delivering or fetching them is what counts, as before.
                if lists_genesis && self.store.in_community(&community).nth(1).is_none() {
                    self.note_other_holder(community);
                }
                self.next_want_page(peer, community, now);
                if let Some(p) = self.peers.get_mut(peer) {
                    p.communities.insert(community);
                }
                self.send_haves(peer);
                // posts already here may still lack their media, and this peer may have it
                self.request_missing_media();
            }
            Msg::Range { community, count, starter, entries } => return self.on_range(peer, community, count, starter, entries),
            Msg::Want { ids } => {
                let subscribed = self.peers.get(peer).map(|p| p.communities.clone()).unwrap_or_default();
                let own_device = self.peers.get(peer).is_some_and(|p| p.client || p.home);
                let mut first_hand = false;
                let mut out: Vec<(u8, u64, Vec<u8>)> = Vec::new();
                let mut sent = BTreeSet::new();
                let mut response_bytes = 0usize;
                // Community records handed over, per community: the peer holds them now (Warmth).
                let mut served: BTreeMap<Id, u64> = BTreeMap::new();
                let mut served_genesis: BTreeSet<Id> = BTreeSet::new();
                for id in ids.iter().take(hoi_sync::MAX_WANT) {
                    let Some(s) = self.store.get(id) else { continue };
                    if s.record.community.is_some_and(|c| !subscribed.contains(&c))
                        || !self.plaintext_forwardable(s)
                        || (s.record.kind() == Kind::Profile && !self.peers.get(peer).is_some_and(|p| p.features.contains(hoi_sync::FEATURE_PROFILE)))
                        || (s.record.kind() == Kind::ProfileColor
                            && !self.peers.get(peer).is_some_and(|p| p.features.contains(hoi_sync::FEATURE_PROFILE_COLOR)))
                        // a person's own blessing that never came back goes to their own devices
                        // only
                        || (!own_device && s.from_peers == 0 && self.own_blessing(s))
                    {
                        continue;
                    }
                    first_hand |= !own_device && self.first_hand(s);
                    let carriers = self.carriers(s);
                    let next_bytes = carriers.iter().filter(|x| !sent.contains(&x.id)).map(|x| x.wire.len()).sum::<usize>()
                        + if sent.contains(id) { 0 } else { s.wire.len() };
                    // Older or hostile peers can request 5,000 maximum-size records in one
                    // frame. Bound allocation before constructing the response, not just the
                    // transport queue after all bytes have already been cloned.
                    if response_bytes + next_bytes > hoi_sync::MAX_BLOB {
                        break;
                    }
                    response_bytes += next_bytes;
                    for x in carriers {
                        if sent.insert(x.id) {
                            out.push((priority(x.record.kind()), x.record.created_at, x.wire.clone()));
                        }
                    }
                    if sent.insert(*id) {
                        out.push((priority(s.record.kind()), s.record.created_at, s.wire.clone()));
                        if let Some(c) = s.record.community {
                            if s.record.kind() == Kind::Community {
                                served_genesis.insert(c);
                            } else {
                                *served.entry(c).or_default() += 1;
                            }
                        }
                    }
                }
                out.sort_by_key(|a| (a.0, a.1));
                if first_hand && !out.is_empty() {
                    self.note_exposed();
                }
                for m in hoi_sync::batches(out.into_iter().map(|x| x.2).collect()) {
                    self.send_msg(peer, m);
                }
                if let Some(p) = self.peers.get_mut(peer) {
                    for c in &served_genesis {
                        p.offers.entry(*c).or_default().genesis = true;
                    }
                    for (c, n) in &served {
                        p.offers.entry(*c).or_default().given += n;
                    }
                }
                // Someone asked for records of a carried community: it is read, not junk.
                for c in served.keys() {
                    self.note_other_holder(*c);
                }
            }
            Msg::Recs { recs } => {
                let undecodable = recs.iter().filter(|w| Wire::decode(w).is_err()).count();
                let arrived: BTreeSet<Id> = recs.iter().filter_map(|w| Wire::decode(w).ok().map(|x| x.1)).collect();
                let mut drop = undecodable > 0 && self.peer_scores.penalize(peer, 2.0 * undecodable as f64);
                let p = peer.to_string();
                let mut accepted = 0u64;
                let mut refused = undecodable as u64;
                // Communities this peer delivered new, valid records of, the genesis aside: anyone
                // with an invite can pass a genesis on.
                let mut carried: BTreeSet<Id> = BTreeSet::new();
                // What it showed it holds, for Warmth: valid records of each community, new or
                // already here, and whether the genesis was among them.
                let mut delivered: BTreeMap<Id, (u64, bool)> = BTreeMap::new();
                // Recent records count as live gossip; older ones as catch-up.
                self.ingest_batch(recs, &|created| Source::Peer { peer: p.clone(), live: created + hoi_validate::CLOCK_SKEW >= now }, &mut |n, wire, res| {
                    match res {
                        Ok(a) => {
                            accepted += 1;
                            if let Some(c) = a.community {
                                let d = delivered.entry(c).or_default();
                                if a.kind == Kind::Community {
                                    d.1 = true;
                                } else {
                                    d.0 += 1;
                                    carried.insert(c);
                                }
                            }
                            n.peer_scores.reward(&p)
                        }
                        Err(e) => {
                            if e != hoi_validate::Reject::Duplicate {
                                refused += 1;
                            } else if let Ok((w, _)) = Wire::decode(wire)
                                && let Some(c) = w.record.community
                            {
                                let d = delivered.entry(c).or_default();
                                if w.record.kind() == Kind::Community {
                                    d.1 = true;
                                } else {
                                    d.0 += 1;
                                }
                            }
                            // Forged identity work costs us an Argon2id run each: one is one too many,
                            // however many good records the peer brought before.
                            let weight = if matches!(e, hoi_validate::Reject::BadIdentity(_)) { hoi_spam::MAX_SCORE - hoi_spam::DISCONNECT_AT } else { 2.0 };
                            if e.strike() == Strike::Peer && n.peer_scores.penalize(&p, weight) {
                                drop = true;
                            }
                        }
                    }
                });
                self.count_records_in(peer, accepted, refused);
                if let Some(p) = self.peers.get_mut(peer) {
                    for (c, (n, genesis)) in &delivered {
                        let o = p.offers.entry(*c).or_default();
                        o.delivered += n;
                        o.genesis |= genesis;
                    }
                }
                if !carried.is_empty() && self.peers.get(peer).is_some_and(|p| p.home) {
                    self.home_proven(peer);
                }
                // A peer that delivers valid records of a community holds it: remember its route.
                for c in carried {
                    self.remember_member(peer, c);
                    self.note_other_holder(c);
                }
                if drop {
                    self.send_msg(peer, Msg::Bye { reason: "too many invalid records".into() });
                    return false;
                }
                let mut completed_pages = Vec::new();
                if let Some(p) = self.peers.get_mut(peer) {
                    if p.wants.values().any(|(_, ids)| ids.iter().any(|id| arrived.contains(id))) {
                        p.sync_progress = Some(now);
                    }
                    for (c, (_, ids)) in &mut p.wants {
                        ids.retain(|i| !arrived.contains(i));
                        if ids.is_empty() {
                            completed_pages.push(*c);
                        }
                    }
                    for pending in p.pending_wants.values_mut() {
                        pending.retain(|i| !arrived.contains(i));
                    }
                }
                for c in completed_pages {
                    self.next_want_page(peer, c, now);
                }
                // a genesis may have just arrived: offer what we have now, to every peer that
                // carries it (a home node learns a community from one device, serves them all)
                let peers: Vec<String> = self.peers.keys().cloned().collect();
                for p in peers {
                    self.send_haves(&p);
                }
                // standing first: whose media may be fetched depends on it
                self.settle_dirty();
                self.request_missing_media();
            }
            Msg::BWant { ids } => self.on_bwant(peer, &ids),
            Msg::Blob { id, data } => return self.take_media(peer, id, data),
            m @ (Msg::Here { .. } | Msg::Sig { .. }) => return self.on_signal_msg(peer, m),
            Msg::Link { mailbox, sealed } => return self.on_link(peer, mailbox, sealed),
            Msg::Catalog { entries } => return self.receive_catalog(peer, entries),
            Msg::Nodes { routes, holders } => return self.receive_nodes(peer, routes, holders),
            Msg::Carry { genesis } => return self.on_carry(peer, genesis),
            Msg::Carried { community, ok, reason } => self.on_carried(peer, community, ok, reason),
            Msg::Piece { op, rid, to, env } => return self.on_piece(peer, op, rid, to, env),
            Msg::Rebuild { rid, manifest } => self.on_rebuild(peer, rid, &manifest),
            Msg::HomeScatter { rid, reply, payload } => self.on_home_scatter(peer, rid, reply, &payload),
            Msg::Rebuilt { rid, content } => self.on_rebuilt(peer, rid, content),
            Msg::Find { community } => self.on_find(peer, community),
            Msg::Found { community, routes } => self.on_found(peer, community, routes),
            Msg::Dht { contacts } => self.on_dht_contacts(peer, contacts),
            Msg::Bind { sig } => return self.on_bind(peer, sig),
            Msg::Bye { reason } => {
                if self.peers.get(peer).is_some_and(|p| p.serves) {
                    self.net_error = Some(reason);
                }
                return false;
            }
        }
        true
    }

    /// Continue a catch-up only when its previous bounded request has been answered.
    fn next_want_page(&mut self, peer: &str, community: Id, now: u64) {
        if self.is_private_community(&community) {
            return;
        }
        let Some(p) = self.peers.get_mut(peer) else { return };
        // One active page across all communities prevents parallel catch-ups from multiplying
        // the response burst beyond the per-link byte budget.
        if p.wants.values().any(|(_, ids)| !ids.is_empty()) {
            return;
        }
        let Some((started, waiting)) = p.wants.get_mut(&community) else { return };
        if !waiting.is_empty() {
            return;
        }
        let at = *started;
        let Some(pending) = p.pending_wants.get_mut(&community) else { return };
        // 200 maximum-size records plus their carriers fit the bounded transport queue.
        let page: Vec<Id> = pending.iter().take(hoi_sync::BATCH).copied().collect();
        for id in &page {
            pending.remove(id);
            waiting.insert(*id);
        }
        if page.is_empty() {
            if p.ranging.contains_key(&community) {
                return; // the range session may still find more
            }
            p.wants.remove(&community);
            p.pending_wants.remove(&community);
            self.state.synced.insert(community, at);
            self.state.sync_completed.insert(community, now);
            self.mark_state();
        } else {
            p.sync_progress = Some(now);
            self.send_msg(peer, Msg::Want { ids: page });
        }
    }

    /// Sends `have` for every community both sides carry that we haven't offered yet.
    pub(crate) fn send_haves(&mut self, peer: &str) {
        let Some(p) = self.peers.get(peer) else { return };
        if !p.said_hello {
            return;
        }
        let ranged = p.features.contains(hoi_sync::FEATURE_RANGE);
        let todo: Vec<Id> = p
            .communities
            .iter()
            // a range session learns what the peer holds, so it starts before this node holds
            // anything of the community, even its genesis; a `have` list needs something to list
            .filter(|c| {
                !p.sent_have.contains(*c)
                    && self.state.joined.contains_key(*c)
                    && (ranged || self.communities.contains_key(*c))
                    && !self.is_private_community(c)
            })
            .copied()
            .collect();
        let now = self.now();
        for c in todo {
            if ranged {
                // learn what the peer holds that this node lacks; the peer starts its own
                let items = self.range_items(peer, &c);
                let entries = hoi_sync::range::initiate(&items);
                if let Some(p) = self.peers.get_mut(peer) {
                    p.sent_have.insert(c);
                    p.ranging.insert(c, (now, false));
                    p.wants.entry(c).or_insert((now, BTreeSet::new()));
                    p.pending_wants.entry(c).or_default();
                }
                self.send_msg(peer, Msg::Range { community: c, count: items.len() as u64, starter: true, entries });
                continue;
            }
            let ids: Vec<Id> = self.offered(peer, &c, now).map(|s| s.id).take(hoi_sync::MAX_HAVE).collect();
            if let Some(p) = self.peers.get_mut(peer) {
                p.sent_have.insert(c);
            }
            self.send_msg(peer, Msg::Have { community: c, ids });
        }
    }

    /// What this node offers `peer` of a community: its live records the peer may have.
    fn offered<'a>(&'a self, peer: &str, c: &Id, now: u64) -> impl Iterator<Item = &'a hoi_store::Stored> + 'a {
        let fx = self.peers.get(peer).map(|p| (p.features.contains(hoi_sync::FEATURE_PROFILE), p.features.contains(hoi_sync::FEATURE_PROFILE_COLOR)));
        let (profiles, colors) = fx.unwrap_or_default();
        self.store.in_community(c).filter(move |s| {
            s.record.expires_at > now
                && self.forwardable(s)
                && (s.record.kind() != Kind::Profile || profiles)
                && (s.record.kind() != Kind::ProfileColor || colors)
        })
    }

    /// The same records in reconciliation order, the newest [`hoi_sync::MAX_HAVE`] of them.
    fn range_items(&self, peer: &str, c: &Id) -> Vec<hoi_sync::range::Item> {
        let mut items: Vec<hoi_sync::range::Item> = self.offered(peer, c, self.now()).map(|s| (s.record.created_at, s.id)).collect();
        items.sort_unstable();
        let extra = items.len().saturating_sub(hoi_sync::MAX_HAVE);
        items.drain(..extra);
        items
    }

    /// A range message. To the side that began the session it brings ids to ask for;
    /// the other side always answers, so the one that began knows when it is done.
    fn on_range(&mut self, peer: &str, community: Id, count: u64, theirs_began: bool, entries: Vec<hoi_sync::range::Entry>) -> bool {
        if !self.state.joined.contains_key(&community) || self.is_private_community(&community) {
            return true;
        }
        let now = self.now();
        let items = self.range_items(peer, &community);
        if theirs_began {
            let step = hoi_sync::range::reconcile(&items, &entries, false);
            self.send_msg(peer, Msg::Range { community, count: items.len() as u64, starter: false, entries: step.reply });
            // matching ranges show the peer knows ids of records held here, besides the genesis
            if step.shared >= 2 && self.believed(peer, &community) {
                self.remember_member(peer, community);
            }
            if let Some(p) = self.peers.get_mut(peer) {
                p.communities.insert(community);
            }
            self.send_haves(peer);
            return true;
        }
        let Some(&(_, heard)) = self.peers.get(peer).and_then(|p| p.ranging.get(&community)) else { return true };
        let step = hoi_sync::range::reconcile(&items, &entries, true);
        let lack: BTreeSet<Id> =
            step.lack.into_iter().filter(|i| !self.store.contains(i) && !self.store.is_deleted(i) && !self.state.skeletons.contains_key(i)).collect();
        if let Some(p) = self.peers.get_mut(peer) {
            let pending: usize = p.pending_wants.values().map(BTreeSet::len).sum::<usize>() + p.wants.values().map(|(_, ids)| ids.len()).sum::<usize>();
            if pending + lack.len() > hoi_sync::MAX_HAVE {
                self.send_msg(peer, Msg::Bye { reason: "too much pending catch-up".into() });
                return false;
            }
            p.pending_wants.entry(community).or_default().extend(lack);
            p.wants.entry(community).or_insert((now, BTreeSet::new()));
            if !heard {
                // the peer reconciles this community, so it holds the genesis
                p.offers.insert(community, crate::node::Offer { listed: count.saturating_sub(1), given: 0, delivered: 0, genesis: true });
            }
            if step.open {
                p.ranging.insert(community, (now, true));
            } else {
                p.ranging.remove(&community);
            }
        }
        if step.open {
            self.send_msg(peer, Msg::Range { community, count: items.len() as u64, starter: true, entries: step.reply });
        }
        if step.shared >= 2 && self.believed(peer, &community) {
            self.remember_member(peer, community);
        }
        // a community that is all definition so far is not empty while a member is around
        if !heard && self.store.in_community(&community).nth(1).is_none() {
            self.note_other_holder(community);
        }
        self.next_want_page(peer, community, now);
        self.request_missing_media();
        true
    }

    /// Range sessions a peer stopped answering end after two minutes; what they found is asked
    /// for as usual.
    pub(crate) fn sweep_ranging(&mut self) {
        let now = self.now();
        let mut ended = Vec::new();
        for (key, p) in &mut self.peers {
            p.ranging.retain(|c, (at, _)| {
                let live = *at + 120 > now;
                if !live {
                    ended.push((key.clone(), *c));
                }
                live
            });
        }
        for (peer, c) in ended {
            self.next_want_page(&peer, c, now);
        }
    }

    /// Tells connected peers what we carry now (after joining or leaving).
    pub(crate) fn announce(&mut self) {
        let peers: Vec<String> = self.peers.iter().filter(|(_, p)| p.said_hello).map(|(k, _)| k.clone()).collect();
        for peer in peers {
            self.announce_to(&peer);
        }
    }

    /// A fresh hello, inventory and catalog for one peer.
    pub(crate) fn announce_to(&mut self, peer: &str) {
        if !self.peers.get(peer).is_some_and(|p| p.said_hello) {
            return;
        }
        self.rematch(peer);
        self.send_hello(peer);
        self.send_haves(peer);
        self.send_catalog(peer);
        self.send_nodes(peer);
        self.auto_request_carry(peer);
    }

    /// Communities we took on since the peer's last hello may be among the salted hashes it
    /// listed then: a member already connected is found without asking it again.
    pub(crate) fn rematch(&mut self, peer: &str) {
        let Some((salt, hashed)) = self.peers.get(peer).and_then(|p| p.their_hashes.clone()) else { return };
        let known = &self.peers[peer].communities;
        let found: Vec<Id> = self
            .state
            .joined
            .keys()
            .filter(|c| !known.contains(c) && !self.is_private_community(c) && hashed.contains(&hoi_sync::community_hash(&salt, c)))
            .copied()
            .collect();
        if let Some(p) = self.peers.get_mut(peer) {
            p.communities.extend(found);
        }
    }

    /// The address paired browsers reach this node at, while it serves them.
    pub fn serving_url(&self) -> Option<&str> {
        self.serving.as_deref()
    }

    /// The web app this node's pairing links open (`https://hoi.sh/app/`).
    pub fn web_base(&self) -> &str {
        &self.web_base
    }

    /// Desktop and hoi-node: start or stop serving browsers and phones at `url`.
    pub fn set_serving(&mut self, url: Option<&str>) -> R<NetStatus> {
        self.serving = url.map(String::from);
        if self.serving.is_some() && self.state.serve_token.is_none() {
            self.state.serve_token = Some(hoi_types::uri::b64(&hoi_crypto::random_bytes::<18>()?));
            self.mark_state();
        }
        if self.serving.is_some() && self.link_key().is_none() {
            self.state.link_key = Some(hoi_types::uri::b64(&hoi_crypto::random_bytes::<32>()?));
            self.mark_state();
        }
        Ok(self.net_status())
    }

    /// Issues a new pairing link and forgets every device's credential: all of them must pair
    /// again. Their links open now end with the next message they send.
    pub fn rotate_pairing(&mut self) -> R<NetStatus> {
        self.state.serve_token = Some(hoi_types::uri::b64(&hoi_crypto::random_bytes::<18>()?));
        self.state.link_key = Some(hoi_types::uri::b64(&hoi_crypto::random_bytes::<32>()?));
        self.state.paired.clear();
        self.mark_state();
        let clients: Vec<String> = self.peers.iter().filter(|(_, p)| p.client).map(|(k, _)| k.clone()).collect();
        for c in clients {
            self.net_disconnect(&c);
        }
        Ok(self.net_status())
    }

    pub fn home(&self) -> Option<PairInfo> {
        self.state.home.clone()
    }

    /// What a pairing link says, before pairing: where it points, the name it gives, and whether
    /// this device already paired through it. The link is masked; the app asks here.
    pub fn preview_pair(&self, link: &str) -> R<PairPreview> {
        let p = Pair::parse(link).map_err(|e| ApiError::invalid(format!("That pairing link doesn't work: {e}.")))?;
        let paired_here = self
            .state
            .home
            .as_ref()
            .is_some_and(|h| h.spent == Some(digest(SPENT_LINK_CONTEXT, &p.token)) || h.next.as_deref() == Some(p.token.as_str()) || h.token == p.token);
        let routes = p.rtc.iter().filter_map(|r| r.strip_prefix("rtc:")).map(String::from).collect();
        Ok(PairPreview { name: p.name, url: p.url, routes, paired_here })
    }

    pub fn pair(&mut self, link: &str) -> R<NetStatus> {
        let p = Pair::parse(link).map_err(|e| ApiError::invalid(format!("That pairing link doesn't work: {e}.")))?;
        // The link this device already paired with is spent; the credential it got still works.
        if self.state.home.as_ref().is_some_and(|h| h.spent == Some(digest(SPENT_LINK_CONTEXT, &p.token))) {
            return Ok(self.net_status());
        }
        let rtc = p.rtc.into_iter().filter(|r| valid_peer_hint(r)).collect();
        self.state.home = Some(PairInfo { url: p.url, token: p.token, name: p.name, rtc, spent: None, next: None, link_key: None, proven: false });
        self.net_error = None;
        self.mark_state();
        Ok(self.net_status())
    }

    pub fn unpair(&mut self) -> NetStatus {
        self.state.home = None;
        self.net_error = None;
        self.mark_state();
        self.net_status()
    }

    pub fn net_status(&self) -> NetStatus {
        let home_peer = self.peers.values().find(|p| p.home && p.serves && p.said_hello);
        let native_peers = self.peers.values().filter(|p| !p.client && !p.home && p.said_hello).count();
        let clients = self.peers.values().filter(|p| p.client && p.said_hello).count() as u64;
        let peers = self.peers.values().filter(|p| p.said_hello).count() as u64;
        let (mode, label) = if let Some(h) = home_peer {
            let via = self.state.home.as_ref().and_then(|x| x.name.clone()).or_else(|| h.name.clone()).unwrap_or_else(|| "your home node".into());
            ("home-node", format!("via {via}"))
        } else if native_peers > 0 && matches!(self.platform.as_str(), "web" | "mobile") {
            // A browser or phone reaches members' computers over WebRTC, never as a full peer.
            ("iroh", if native_peers == 1 { "via a member's computer".to_string() } else { format!("via {native_peers} members' computers") })
        } else if native_peers > 0 {
            ("iroh", format!("connected to {native_peers} peers"))
        } else if self.serving.is_some() && clients > 0 {
            ("local", format!("serving {clients} of your devices"))
        } else if self.state.home.is_some() {
            ("local", if self.net_error.is_some() { "home node unreachable".to_string() } else { "looking for your home node".to_string() })
        } else if let Some(error) = &self.net_error {
            ("local", error.clone())
        } else {
            let where_ = match self.platform.as_str() {
                "web" => "only in this browser",
                "desktop" => "only on this computer",
                "mobile" => "only on this phone",
                _ => "only on this device",
            };
            ("local", where_.to_string())
        };
        NetStatus {
            transport: self.transport.clone(),
            communities: self
                .state
                .joined
                .keys()
                .map(|c| CommunitySyncStatus {
                    community: c.hex(),
                    connected_peers: self.peers.values().filter(|p| p.said_hello && p.communities.contains(c)).count() as u64,
                    last_sync: self.state.sync_completed.get(c).copied(),
                })
                .collect(),
            connections: self
                .peers
                .iter()
                .map(|(id, p)| ConnectionStatus {
                    peer: id.clone(),
                    role: if p.client {
                        "client"
                    } else if p.home {
                        "home"
                    } else {
                        "peer"
                    }
                    .into(),
                    route: p.route.clone(),
                    connected: p.said_hello,
                    transport: p.meta.transport.clone(),
                    encryption: encryption_words(&p.meta),
                    kind: if p.home {
                        "home"
                    } else if p.client {
                        "device"
                    } else if p.features.contains(hoi_sync::FEATURE_CARRY) {
                        "public"
                    } else if p.features.contains(hoi_sync::FEATURE_KEEPS) {
                        "node"
                    } else {
                        "browser"
                    }
                    .into(),
                    keeps: p.features.contains(hoi_sync::FEATURE_KEEPS),
                    incoming: p.meta.incoming,
                    direct: p.meta.direct.is_some(),
                    communities: p
                        .communities
                        .iter()
                        .filter(|c| self.state.joined.contains_key(*c) && !self.is_private_community(c))
                        .map(|c| c.hex())
                        .collect(),
                })
                .collect(),
            last_sync: self.state.sync_completed.iter().filter(|(c, _)| self.state.joined.contains_key(c)).map(|(_, at)| *at).max(),
            mode: mode.into(),
            label,
            peers,
            home_node: self.state.home.as_ref().map(|h| HomeNode {
                url: h.url.clone(),
                connected: home_peer.is_some(),
                name: h.name.clone(),
                error: self.net_error.clone(),
                // The home node's current link for the next device, never this device's credential.
                pairing_web: h
                    .next
                    .as_ref()
                    .map(|next| Pair { url: h.url.clone(), token: next.clone(), name: h.name.clone(), rtc: h.rtc.clone() }.to_web(&self.web_base))
                    .unwrap_or_default(),
                phone_can_reach: phone_can_reach(&h.url, &h.rtc),
                via: home_peer.map(|p| p.meta.transport.clone()),
            }),
            serving: self.serving.as_ref().map(|url| {
                let pair =
                    Pair { url: url.clone(), token: self.state.serve_token.clone().unwrap_or_default(), name: self.node_label(), rtc: self.rtc_routes.clone() };
                Serving {
                    url: url.clone(),
                    pairing: pair.to_uri(),
                    pairing_web: pair.to_web(&self.web_base),
                    phone_can_reach: phone_can_reach(url, &self.rtc_routes),
                    clients,
                }
            }),
            rtc: self.rtc_routes.clone(),
            relaying: self.accepts_members(),
            directory: self.directory_status(),
            node_link: self.node_link(),
            port_mapping: self.port_mapping.clone(),
            exposure: self.exposure(),
            vpn: self.vpn_status.clone(),
            pending_notices: self.pending_notices(),
        }
    }
}

/// Endpoint ids are hex public keys; a direct route appends @ followed by SocketAddr. WebRTC
/// routes are `rtc:` + the base64url SHA-256 of the node's DTLS certificate + @ + SocketAddr.
pub(crate) fn valid_peer_hint(peer: &str) -> bool {
    if peer.len() > 128 {
        return false;
    }
    let usable = |a: &str| a.parse::<std::net::SocketAddr>().is_ok_and(|a| a.port() != 0 && !a.ip().is_unspecified() && !a.ip().is_multicast());
    if let Some(rest) = peer.strip_prefix("rtc:") {
        return rest
            .split_once('@')
            .is_some_and(|(fp, addr)| fp.len() == 43 && fp.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') && usable(addr));
    }
    let (id, addr) = peer.split_once('@').map_or((peer, None), |(id, addr)| (id, Some(addr)));
    hoi_types::Id::from_hex(id).is_ok()
        && addr.is_none_or(|a| a.parse::<std::net::SocketAddr>().is_ok_and(|a| a.port() != 0 && !a.ip().is_unspecified() && !a.ip().is_multicast()))
}

pub(crate) fn valid_relay_hint(relay: &str) -> bool {
    // The transport additionally parses these as URLs before dialing. Only TLS relay hints,
    // except a relay on this machine (tests and development).
    let loopback = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().any(|p| relay.starts_with(p));
    relay.len() <= 512
        && (relay.starts_with("https://") || loopback)
        && relay.len() > 8
        && !relay.bytes().any(|c| c.is_ascii_control() || c.is_ascii_whitespace())
        && !relay.contains(['@', '?', '#'])
}

/// Whether a pairing link with this address and these routes lets a phone reach the node: a
/// WebRTC route, or a wss:// address (plain ws:// is for this very machine).
fn phone_can_reach(url: &str, rtc: &[String]) -> bool {
    !rtc.is_empty() || url.starts_with("wss://")
}
