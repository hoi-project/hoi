//! Public nodes keep communities available on request (PROTOCOL.md §10). Anyone who
//! holds a community's signed genesis (a member, or anyone with its invite) may hand it to a
//! node that advertised `carry` in its hello; the node stores and serves the community like a
//! pinner, without being a member: it has no standing, signs nothing, vouches nobody in.
//!
//! A request carries the genesis and nothing else: no routes, so an unpaired stranger never
//! makes a public node dial an address of their choosing. The node learns where the
//! community's members are the way it learns everything else, from peers that prove they hold
//! records.
//!
//! What keeps this bounded, since anyone can mint a community: the operator's policy (off,
//! index-listed only, or requests too) and refusals, a rate per peer and one for the node, a
//! budget of communities and bytes, open admission only, and letting go of what nobody else
//! touched for two weeks, or whose records never came. A community carried on request that is
//! let go is deleted: its records were never this operator's choice. Nothing about who asked is
//! kept: the origin is a word, never a peer or an address.

use std::collections::BTreeSet;

use hoi_sync::Msg;
use hoi_types::{Admission, Body, Id, Kind};

use crate::api::{R, parse_id};
use crate::dto::CarryRequest;
use crate::state::Carried;
use crate::{ApiError, Node};

/// What a native node does with communities nobody paired or joined it into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CarryPolicy {
    /// Only what the operator joined (`--join`) or a paired device brought along.
    #[default]
    Off,
    /// Also what a registry this node mirrors lists (`hoi-node --index`).
    Index,
    /// Also what connected peers ask for with `carry` (a public node, `hoi-node --public`).
    Any,
}

impl CarryPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            CarryPolicy::Off => "off",
            CarryPolicy::Index => "index",
            CarryPolicy::Any => "any",
        }
    }

    pub fn parse(s: &str) -> Option<CarryPolicy> {
        match s {
            "off" => Some(CarryPolicy::Off),
            "index" => Some(CarryPolicy::Index),
            "any" => Some(CarryPolicy::Any),
            _ => None,
        }
    }
}

/// `carry` requests one peer may make, and a requester sends one peer, per minute. Further
/// ones in that minute are ignored and count against the sender.
pub const CARRY_PER_MINUTE: u32 = 4;
/// Requests a node accepts in one minute and in one day, whoever asks: the rest hear `busy`.
pub const ACCEPT_PER_MINUTE: u32 = 8;
pub const ACCEPT_PER_DAY: u32 = 128;
/// Communities a node carries at most on request and from indexes together, by default.
pub const DEFAULT_CARRY_LIMIT: usize = 256;
/// Bytes of records and media the communities carried on request may hold together, by
/// default. Over it, the ones nobody touched for longest are let go first.
pub const DEFAULT_CARRY_BYTES: u64 = 256 << 20;
/// A community carried on request that nobody else delivered or fetched for this long is let go.
pub const CARRY_IDLE: u64 = 14 * 86_400;
/// A community carried on request whose records never came is let go after this long.
pub const CARRY_EMPTY: u64 = 3600;
/// Automatic requests a device sends one public node at a time, for open communities it founded.
const AUTO_REQUESTS_PER_HELLO: usize = 3;
/// A public node offers a community it holds to other public nodes while it sees fewer durable
/// copies than this, its own included (Warmth's `durable_copies`, PROTOCOL.md §20)...
pub const SPREAD_UNTIL: f64 = 3.0;
/// ...and to at most this many of them a day, so a community spreads, not floods.
pub const SPREAD_PER_DAY: u8 = 2;

pub(crate) const ORIGIN_INDEX: &str = "index";
pub(crate) const ORIGIN_REQUEST: &str = "request";

impl Node {
    /// The host's policy and budget. Browsers and phones cannot carry; the policy stays off
    /// there whatever the host says.
    pub fn set_carry_policy(&mut self, policy: CarryPolicy, limit: usize) {
        let policy = if self.can_relay() { policy } else { CarryPolicy::Off };
        let before = self.accepts_carry();
        self.carry_policy = policy;
        self.carry_limit = limit.max(1);
        if before != self.accepts_carry() {
            // The hello's feature list changed: peers learn whether they may ask.
            self.announce();
        }
        self.redeclare();
    }

    /// How many bytes the communities carried on request may hold together.
    pub fn set_carry_bytes(&mut self, bytes: u64) {
        self.carry_bytes = bytes.max(1);
        self.redeclare();
    }

    /// Communities this node refuses to carry, whoever asks and whatever an index lists. One
    /// already here on an index's or a peer's account is let go and deleted now; one the
    /// operator joined on purpose stays, that choice was theirs.
    pub fn set_refused(&mut self, ids: &[String]) -> R<()> {
        let mut refused = BTreeSet::new();
        for id in ids {
            refused.insert(parse_id(id)?);
        }
        let drop: Vec<Id> = refused.iter().filter(|c| self.state.carried.contains_key(c)).copied().collect();
        self.refused = refused;
        let dropped = !drop.is_empty();
        for c in drop {
            let name = self.community_label(&c);
            self.evict_carried(&c);
            self.notice(format!("Stopped carrying c/{name}: refused by the operator."));
        }
        if dropped {
            self.announce();
        }
        Ok(())
    }

    pub fn refused(&self) -> Vec<String> {
        self.refused.iter().map(|c| c.hex()).collect()
    }

    /// Refused by a `--refuse` flag of this run, or suppressed for good (crate::suppress).
    pub(crate) fn is_refused(&self, c: &Id) -> bool {
        self.refused.contains(c) || self.suppressed_community(c)
    }

    pub fn carry_policy(&self) -> CarryPolicy {
        self.carry_policy
    }

    pub fn carry_limit(&self) -> usize {
        self.carry_limit
    }

    /// This node takes `carry` requests from connected peers right now: the host's policy says
    /// `any`, and on the desktop the person turned on *Be a public node* too.
    /// `hoi-node` is public by its operator's flags alone.
    pub(crate) fn accepts_carry(&self) -> bool {
        self.can_relay() && self.carry_policy == CarryPolicy::Any && (self.platform != "desktop" || self.state.settings.public_node)
    }

    /// Communities carried for an index or on request, pending ones included.
    pub fn carried_count(&self) -> usize {
        self.state.carried.len()
    }

    /// Remembers why this node carries `c`. An index listing outranks a request: it is never
    /// let go for idleness (the index is the admission list).
    pub(crate) fn note_carried(&mut self, c: Id, origin: &str) {
        let now = self.now();
        match self.state.carried.get_mut(&c) {
            Some(carried) if carried.origin == ORIGIN_REQUEST && origin == ORIGIN_INDEX => carried.origin = ORIGIN_INDEX.into(),
            Some(_) => return,
            None => {
                self.state.carried.insert(c, Carried { since: now, origin: origin.into(), last_other_holder: None });
            }
        }
        self.mark_state();
    }

    /// A peer other than this node delivered records of `c`, or fetched some: the community is
    /// not this node's alone, nor unread. Written down at most hourly.
    pub(crate) fn note_other_holder(&mut self, c: Id) {
        let now = self.now();
        if let Some(carried) = self.state.carried.get_mut(&c)
            && carried.last_other_holder.is_none_or(|at| now.saturating_sub(at) >= 3600)
        {
            carried.last_other_holder = Some(now);
            self.mark_state();
        }
    }

    /// Records and media of one community, in bytes, as the carry budget counts them.
    pub(crate) fn community_bytes(&self, c: &Id) -> u64 {
        let mut bytes = 0u64;
        let mut media = BTreeSet::new();
        for s in self.store.in_community(c) {
            bytes += s.wire.len() as u64;
            if let Body::Post { media: m, .. } = &s.record.body {
                for x in m {
                    if media.insert(x.hash)
                        && let Some(blob) = self.store.blob(&x.hash)
                    {
                        bytes += blob.len() as u64;
                    }
                }
            }
        }
        bytes
    }

    /// Lets go of communities carried on request that nobody else delivered or fetched for
    /// [`CARRY_IDLE`], whose records never arrived within [`CARRY_EMPTY`], or that together
    /// exceed the byte budget (the longest untouched first). Their records are deleted: they
    /// were taken on a stranger's word, not the operator's choice. Called from `sweep`.
    pub(crate) fn sweep_carried(&mut self) {
        let now = self.now();
        let mut gone: Vec<(Id, &'static str)> = Vec::new();
        let mut kept: Vec<(u64, Id, u64)> = Vec::new();
        for (id, c) in &self.state.carried {
            if c.origin != ORIGIN_REQUEST {
                continue;
            }
            let touched = c.last_other_holder.unwrap_or(c.since);
            let records = self.store.in_community(id).count();
            if now.saturating_sub(touched) >= CARRY_IDLE {
                gone.push((*id, "nobody else held or read a copy for two weeks"));
            } else if records <= 1 && c.last_other_holder.is_none() && now.saturating_sub(c.since) >= CARRY_EMPTY {
                gone.push((*id, "nobody brought its records within an hour"));
            } else {
                kept.push((touched, *id, self.community_bytes(id)));
            }
        }
        // Over budget: the longest untouched go first, until what is left fits.
        kept.sort();
        let mut total: u64 = kept.iter().map(|k| k.2).sum();
        for (_, id, bytes) in kept {
            if total <= self.carry_bytes {
                break;
            }
            total -= bytes;
            gone.push((id, "the public node's space for carried communities is used up"));
        }
        if gone.is_empty() {
            return;
        }
        for (c, why) in gone {
            let name = self.community_label(&c);
            self.evict_carried(&c);
            self.notice(format!("Stopped carrying c/{name}: {why}."));
        }
        self.announce();
    }

    /// Leaves `c`, forgets why it was carried, and deletes its records and media here.
    pub(crate) fn evict_carried(&mut self, c: &Id) {
        self.state.carried.remove(c);
        self.state.joined.remove(c);
        self.state.public_joins.remove(c);
        self.state.bootstrap.remove(c);
        self.state.sync_completed.remove(c);
        self.clear_public_join_sync(*c);
        self.purge_community(c);
        self.mark_state();
        self.mark_changed(*c);
    }

    /// Deletes every record of `c` held here, without the marker that keeps a removed record
    /// from coming back: carried again later, the same records are welcome again.
    pub(crate) fn purge_community(&mut self, c: &Id) {
        let ids: Vec<Id> = self.store.in_community(c).map(|s| s.id).collect();
        for id in ids {
            if self.store.remove(&id).is_some() {
                self.state.saved.remove(&id);
                self.changes.push(hoi_store::Change::Delete { id });
            }
        }
        self.communities.remove(c);
        self.dirty.remove(c);
        self.warmth_hints.remove(c);
        for p in self.peers.values_mut() {
            p.offers.remove(c);
        }
    }

    /// Forgets the carry note when a community is left for any reason.
    pub(crate) fn forget_carried(&mut self, c: &Id) {
        if self.state.carried.remove(c).is_some() {
            self.mark_state();
        }
    }

    fn community_label(&self, c: &Id) -> String {
        self.communities.get(c).map(|st| st.genesis.name.clone()).unwrap_or_else(|| c.hex()[..8].to_string())
    }

    /// Whether `c` may be handed to a public node at all: open by its genesis and by its
    /// current rules, and not private.
    pub(crate) fn carriable(&self, c: &Id) -> R<Vec<u8>> {
        if self.is_private_community(c) {
            return Err(ApiError::not_allowed("Private communities travel only through their encrypted relays."));
        }
        let genesis = self.store.genesis_of(c).ok_or_else(|| ApiError::not_found("community"))?;
        let Body::Community(g) = &genesis.record.body else { return Err(ApiError::not_found("community")) };
        let open = g.limits.admission == Admission::Open && self.communities.get(c).is_none_or(|st| st.limits.admission == Admission::Open);
        if !open {
            return Err(ApiError::not_allowed(
                "Public nodes carry open communities only. An invite-only community is kept by its members' own nodes (hoi-node --join).",
            ));
        }
        Ok(genesis.wire.clone())
    }

    /// Sends one request, within the per-minute pace a node tolerates. Returns whether it went.
    fn send_carry(&mut self, peer: &str, c: Id, genesis: &[u8]) -> bool {
        let now = self.now();
        let Some(p) = self.peers.get_mut(peer) else { return false };
        if p.carry_sent_minute != now / 60 {
            p.carry_sent_minute = now / 60;
            p.carry_sent = 0;
        }
        if p.carry_sent >= CARRY_PER_MINUTE {
            return false;
        }
        p.carry_sent += 1;
        p.carry_asked.insert(c);
        self.send_msg(peer, Msg::Carry { genesis: genesis.to_vec() });
        true
    }

    /// Asks every connected node that takes requests to keep `id` available. Returns how many
    /// were asked; their answers arrive as notices, and Warmth shows what actually arrived.
    pub fn request_carry(&mut self, id: &str) -> R<CarryRequest> {
        let c = parse_id(id)?;
        let genesis = self.carriable(&c)?;
        let targets: Vec<String> = self
            .peers
            .iter()
            .filter(|(_, p)| p.said_hello && p.features.contains(hoi_sync::FEATURE_CARRY) && !p.client && !p.communities.contains(&c))
            .map(|(k, _)| k.clone())
            .collect();
        let already: u32 =
            self.peers.values().filter(|p| p.said_hello && p.features.contains(hoi_sync::FEATURE_CARRY) && p.communities.contains(&c)).count() as u32;
        let mut asked = 0u32;
        for peer in &targets {
            if self.send_carry(peer, c, &genesis) {
                asked += 1;
            }
        }
        Ok(CarryRequest { community: c.hex(), asked, carrying: already })
    }

    /// The person is about to share an invite to an open community: the public nodes connected
    /// now that don't hold it yet are asked to keep it, so the invite works while every member
    /// is away (its routes name them, `Node::invite_hints`). For the rest of this run, public
    /// nodes that say hello later are asked too, as for the communities the person founded.
    /// An invite-only or private community is never offered: nothing happens for those.
    pub fn carry_for_invite(&mut self, id: &str) -> R<()> {
        let c = parse_id(id)?;
        let Ok(genesis) = self.carriable(&c) else { return Ok(()) };
        if !self.state.joined.contains_key(&c) {
            return Ok(());
        }
        self.invite_carry.insert(c);
        let targets: Vec<String> = self
            .peers
            .iter()
            .filter(|(_, p)| {
                p.said_hello
                    && p.features.contains(hoi_sync::FEATURE_CARRY)
                    && !p.client
                    && !p.home
                    && !p.communities.contains(&c)
                    && !p.carry_asked.contains(&c)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for peer in targets {
            self.send_carry(&peer, c, &genesis);
        }
        Ok(())
    }

    /// A node that takes requests said hello, or this device made a community: ask that node,
    /// unasked, for the open communities this device's person founded and it doesn't hold yet.
    /// That is how a community made in a browser gets a home on the nodes the browser reaches,
    /// with no registry and nothing configured. Someone else's communities wait for the person
    /// to ask.
    pub(crate) fn auto_request_carry(&mut self, peer: &str) {
        self.spread_carry(peer);
        let Some(me) = self.me_root() else { return };
        let Some(p) = self.peers.get(peer) else { return };
        if !p.said_hello || !p.features.contains(hoi_sync::FEATURE_CARRY) || p.client || p.home {
            return;
        }
        let mine: Vec<Id> = self
            .state
            .joined
            .keys()
            .filter(|c| self.state.joined.contains_key(c) && !self.is_private_community(c) && !p.communities.contains(c) && !p.carry_asked.contains(c))
            .filter(|c| {
                self.communities.get(c).is_some_and(|st| {
                    (st.is_owner(&me) || self.invite_carry.contains(c))
                        && st.limits.admission == Admission::Open
                        && st.genesis.limits.admission == Admission::Open
                })
            })
            .take(AUTO_REQUESTS_PER_HELLO)
            .copied()
            .collect();
        for c in mine {
            let Some(genesis) = self.store.genesis_of(&c).map(|s| s.wire.clone()) else { continue };
            self.send_carry(peer, c, &genesis);
        }
    }

    /// A public node shares the load: the open communities it carries or is in, held by too few
    /// durable nodes as far as it can see, are offered to the public node that just said hello,
    /// within [`SPREAD_PER_DAY`]. That node's own policy and limits still decide.
    fn spread_carry(&mut self, peer: &str) {
        if !self.is_public_node() {
            return;
        }
        let Some(p) = self.peers.get(peer) else { return };
        if !p.said_hello || !p.features.contains(hoi_sync::FEATURE_CARRY) || p.client || p.home {
            return;
        }
        let candidates: BTreeSet<Id> = self
            .state
            .carried
            .keys()
            .chain(self.state.joined.keys())
            .filter(|c| !self.is_private_community(c) && !p.communities.contains(c) && !p.carry_asked.contains(c))
            .copied()
            .collect();
        let day = self.now() / 86_400;
        let mut sent = 0;
        for c in candidates {
            if sent >= AUTO_REQUESTS_PER_HELLO {
                break;
            }
            let today = self.spread.get(&c).filter(|(d, _)| *d == day).map_or(0, |(_, n)| *n);
            if today >= SPREAD_PER_DAY || hoi_sync::warmth::assess(&self.observe(&c)).durable_copies >= SPREAD_UNTIL {
                continue;
            }
            let Ok(genesis) = self.carriable(&c) else { continue };
            if self.send_carry(peer, c, &genesis) {
                self.spread.insert(c, (day, today + 1));
                sent += 1;
            }
        }
        self.spread.retain(|_, (d, _)| *d == day);
    }

    /// One more acceptance fits this minute's and this day's allowance.
    fn accept_allowance(&mut self) -> bool {
        let now = self.now();
        if self.carry_accepted.0 != now / 60 {
            self.carry_accepted = (now / 60, 0);
        }
        if self.carry_accepted_day.0 != now / 86_400 {
            self.carry_accepted_day = (now / 86_400, 0);
        }
        if self.carry_accepted.1 >= ACCEPT_PER_MINUTE || self.carry_accepted_day.1 >= ACCEPT_PER_DAY {
            return false;
        }
        self.carry_accepted.1 += 1;
        self.carry_accepted_day.1 += 1;
        true
    }

    /// A `carry` request arrived. Returns false when the connection should be dropped.
    pub(crate) fn on_carry(&mut self, peer: &str, genesis: Vec<u8>) -> bool {
        let now = self.now();
        let Some(p) = self.peers.get_mut(peer) else { return false };
        if now / 60 != p.carry_minute {
            p.carry_minute = now / 60;
            p.carry_msgs = 0;
        }
        p.carry_msgs += 1;
        if p.carry_msgs > CARRY_PER_MINUTE {
            // Over the pace: no answer, and it counts.
            return !self.peer_scores.penalize(peer, 1.0);
        }
        if !self.accepts_carry() {
            // Not advertised, so not expected: answer, and count it against the peer.
            let community = hoi_validate::decode(&genesis).ok().map(|d| d.wire.record.author).unwrap_or(Id::ZERO);
            self.send_msg(peer, Msg::Carried { community, ok: false, reason: Some("not accepting".into()) });
            return !self.peer_scores.penalize(peer, 1.0);
        }
        // The same checks a catalog listing passes before anyone sees it.
        let Ok(d) = hoi_validate::decode(&genesis) else {
            return !self.peer_scores.penalize(peer, 2.0);
        };
        let r = &d.wire.record;
        let c = r.author;
        let Body::Community(g) = &r.body else {
            return !self.peer_scores.penalize(peer, 2.0);
        };
        let answer = |n: &mut Node, ok: bool, reason: Option<&str>| n.send_msg(peer, Msg::Carried { community: c, ok, reason: reason.map(String::from) });
        if self.store.genesis_of(&c).is_some_and(|known| known.id != d.id)
            || hoi_validate::stamp(&d.id, r.nonce, hoi_validate::stamp_floor(Kind::Community)).is_err()
            || hoi_validate::signature(&d).is_err()
            || hoi_validate::content(r).is_err()
            || hoi_validate::time(r, now, false, None).is_err()
        {
            answer(self, false, Some("invalid"));
            return !self.peer_scores.penalize(peer, 2.0);
        }
        if self.is_refused(&c) {
            answer(self, false, Some("refused"));
            return true;
        }
        if self.is_private_community(&c) {
            answer(self, false, Some("not open"));
            return true;
        }
        let open = g.limits.admission == Admission::Open && self.communities.get(&c).is_none_or(|st| st.limits.admission == Admission::Open);
        if !open {
            answer(self, false, Some("not open"));
            return true;
        }
        if self.state.joined.contains_key(&c) {
            // Already here, by whatever path: say yes. The routes stay the peer's own business.
            answer(self, true, None);
            return true;
        }
        let requests = self.state.carried.values().filter(|x| x.origin == ORIGIN_REQUEST).count();
        if self.state.carried.len() >= self.carry_limit || requests >= self.carry_limit.div_ceil(2) {
            answer(self, false, Some("full"));
            return true;
        }
        if !self.accept_allowance() {
            answer(self, false, Some("busy"));
            return true;
        }
        match self.join_carried(&genesis) {
            Ok(()) => {
                self.note_carried(c, ORIGIN_REQUEST);
                // The requester learns at once that we hold the community and lists what it
                // has; everyone else connected who carries it gets our inventory. Nobody else
                // is told: one small request must not fan out into a hello for every link.
                self.announce_to(peer);
                // Members connected before we held it listed it among their salted hashes.
                let salted: Vec<String> = self
                    .peers
                    .iter()
                    .filter(|(_, p)| p.their_hashes.as_ref().is_some_and(|(salt, hashed)| hashed.contains(&hoi_sync::community_hash(salt, &c))))
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in salted {
                    if let Some(p) = self.peers.get_mut(&k) {
                        p.communities.insert(c);
                    }
                }
                let others: Vec<String> =
                    self.peers.iter().filter(|(k, p)| k.as_str() != peer && p.said_hello && p.communities.contains(&c)).map(|(k, _)| k.clone()).collect();
                for other in others {
                    self.send_haves(&other);
                }
                answer(self, true, None);
            }
            Err(_) => answer(self, false, Some("invalid")),
        }
        true
    }

    /// Joins and pins a community from its genesis, as `--join` does, without a person's key.
    fn join_carried(&mut self, genesis: &[u8]) -> R<()> {
        let c = self.join_genesis(genesis)?;
        if let Some(j) = self.state.joined.get_mut(&c) {
            j.pinning = true;
        }
        self.mark_state();
        Ok(())
    }

    /// A node answered one of this device's requests.
    pub(crate) fn on_carried(&mut self, peer: &str, community: Id, ok: bool, reason: Option<String>) {
        let asked = self.peers.get_mut(peer).is_some_and(|p| p.carry_asked.remove(&community));
        if !asked {
            return; // nothing this node asked for: ignore it
        }
        let name = self.communities.get(&community).map(|st| format!("c/{}", st.genesis.name)).unwrap_or_else(|| "the community".into());
        let text = if ok {
            format!("A public node now keeps {name} available. Warmth shows when its copy is complete.")
        } else {
            match reason.as_deref().unwrap_or("") {
                "full" => format!("A public node has no room for {name} right now."),
                "busy" => format!("A public node is busy right now; {name} was not taken. Ask again later."),
                "refused" => format!("A public node's operator does not carry {name}."),
                "not open" => format!("A public node took no copy of {name}: it carries open communities only."),
                "not accepting" => format!("A public node is not taking communities right now; {name} was not carried."),
                _ => format!("A public node did not take {name}."),
            }
        };
        self.notice(text);
    }

    /// Peers that take requests, for the UI's "ask public nodes" button.
    pub(crate) fn carry_nodes(&self) -> u32 {
        self.peers.values().filter(|p| p.said_hello && p.features.contains(hoi_sync::FEATURE_CARRY) && !p.client).count() as u32
    }

    /// Open communities this node carries on request, for tests and status lines.
    pub fn carried_on_request(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        for (c, carried) in &self.state.carried {
            if carried.origin == ORIGIN_REQUEST {
                out.insert(c.hex());
            }
        }
        out.into_iter().collect()
    }
}
