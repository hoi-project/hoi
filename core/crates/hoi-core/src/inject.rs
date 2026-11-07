//! The injector (SCATTER.md S7): a device never offers its own new records to its own
//! links before they come back from elsewhere.
//!
//! A new record of an open community this device made (a post's pointer, a vote, a reply, a
//! ledger entry) is not gossiped from here. It goes, with what it depends on (its author's
//! identity and blessing), sealed to an *injector*: a public node this device links to that
//! carries the community and holds pieces (so it has a key). It goes through another link, an
//! *entry* in another place that reaches the injector, so the injector receives it from that
//! forwarder, not from this device. The injector holds it a random moment and takes it in as if
//! a peer had sent it, and passes it on to everyone. The link this device sends through sees an
//! envelope of some size; the injector sees records and the entry's address.
//!
//! The device keeps the record to itself (not in what it offers, not in what it gossips) until
//! it comes back from any link, or for two minutes, after which it offers it as before. Where no
//! injector and entry are linked, the record is gossiped at once, as before.
//!
//! A person's own identity and blessings have no community, and go first hand only
//! to the person's own devices; everyone else gets them with the next injected record they come
//! with (or through an injector of a community whose line needs the blessing). A home node
//! injects what its own devices post as if it were its own. The hold is saved, and so is what
//! came back while held, what went in two hops, and whether this device ever offered its
//! person's records first: together they say whether the node it connects through can tell a
//! post is its person's ([`Node::address_hidden`]).

use hoi_crypto::envelope;
use hoi_sync::{Msg, PieceOp};
use hoi_types::{Id, Kind};

use crate::{Node, Source};

/// How long a device waits for its own record to come back before it offers it itself.
pub(crate) const INJECT_WAIT: u64 = 120;
/// An injector holds what it takes in for a random moment of up to this many seconds, so its
/// passing on does not follow the entry's envelope at once.
const INJECT_JITTER: u64 = 20;
/// Injections an injector holds at most.
const MAX_INJECTIONS: usize = 256;
/// Own records waiting to come back, at most.
const MAX_INJECTING: usize = 512;

impl Node {
    /// The way to inject a record of community `c`: (entry link, injector endpoint, its key).
    fn inject_path(&self, c: Id) -> Option<(String, Id, [u8; 32])> {
        let fw: Vec<(&String, &crate::node::PeerState)> = self.peers.iter().filter(|(_, p)| p.said_hello && p.scatter.forwards).collect();
        let place = |p: &crate::node::PeerState| (p.operator, crate::warmth::network_key(&p.meta));
        let mut options = Vec::new();
        for (_, inj) in fw.iter().filter(|(_, p)| p.scatter.holder.is_some() && p.communities.contains(&c)) {
            let Some(endpoint) = inj.meta.endpoint.as_deref().and_then(|e| Id::from_hex(e).ok()) else { continue };
            let Some(xk) = inj.scatter.holder.as_ref().map(|k| k.xk) else { continue };
            for (entry, _) in fw.iter().filter(|(_, e)| e.scatter.vouches.iter().any(|v| v.endpoint == endpoint) && place(e) != place(inj)) {
                options.push(((*entry).clone(), endpoint, xk));
            }
        }
        if options.is_empty() {
            return None;
        }
        let i = crate::scatter::uniform(options.len());
        Some(options.swap_remove(i))
    }

    /// This device's own new record `id`, before it is gossiped: sent to an injector when one is
    /// linked, and kept from this device's links until it comes back. False when it was not.
    pub(crate) fn inject_own(&mut self, id: Id) -> bool {
        let Some(c) = self.store.get(&id).and_then(|s| s.record.community) else { return false };
        self.inject_in(id, c)
    }

    /// [`Node::inject_own`] through an injector of community `c`: for a record of `c`, or one
    /// without a community (a blessing) that `c`'s nodes need.
    pub(crate) fn inject_in(&mut self, id: Id, c: Id) -> bool {
        let Some(s) = self.store.get(&id) else { return false };
        if s.record.community.is_some_and(|x| x != c) {
            return false;
        }
        // a record kept in pieces is never handed over whole, not even to an injector: its
        // pointer records are what goes out (SCATTER.md S5)
        if self.is_private_community(&c) || s.record.kind() == Kind::Community || self.kept_in_pieces(s) || self.scatter.saved.injecting.len() >= MAX_INJECTING
        {
            return false;
        }
        let Some((entry, injector, xk)) = self.inject_path(c) else { return false };
        let mut wires: Vec<Vec<u8>> = self.carriers(s).into_iter().map(|x| x.wire.clone()).collect();
        wires.push(s.wire.clone());
        let Ok(env) = envelope::seal_inject(&injector.0, &xk, &wires) else { return false };
        let rid = self.next_rid();
        self.send_msg(&entry, Msg::Piece { op: PieceOp::Inject, rid, to: Some(injector), env });
        let until = self.now() + INJECT_WAIT;
        self.scatter.saved.injecting.insert(id, until);
        self.scatter.dirty = true;
        true
    }

    /// Whether a record is this device's person's own, or (on a home node) one of the people
    /// whose devices paired with it.
    pub(crate) fn own_root(&self, root: &Id) -> bool {
        Some(*root) == self.me_root() || self.scatter.saved.client_roots.contains(root)
    }

    /// A person's own identity or blessing: it goes first hand only to the person's own
    /// devices, and to everyone else with the records it comes with.
    pub(crate) fn own_blessing(&self, s: &hoi_store::Stored) -> bool {
        matches!(s.record.kind(), Kind::Identity | Kind::Delegation) && self.own_root(&s.author_root)
    }

    /// Whether offering `s` to a link that is not one of the person's devices tells that link it
    /// came from here: one of this person's own records that never came back from anywhere, or
    /// the genesis of a community this person owns.
    pub(crate) fn first_hand(&self, s: &hoi_store::Stored) -> bool {
        let Some(me) = self.me_root() else { return false };
        if s.from_peers > 0 {
            return false;
        }
        s.author_root == me || matches!(&s.record.body, hoi_types::Body::Community(g) if g.owners.contains(&me))
    }

    /// Notes that this device offered its person's own record first hand to a link that is not
    /// one of the person's devices: from now on that link can tell its records apart.
    pub(crate) fn note_exposed(&mut self) {
        if self.scatter.saved.exposed_at.is_none() {
            self.scatter.saved.exposed_at = Some(self.now());
            self.scatter.dirty = true;
        }
    }

    /// Whether the node this device connects through can't tell that post `s` is its person's:
    /// it is this device's own, in an open community; every pointer record that went out for it
    /// (or the post itself, if it went whole) came back through an injector before this device
    /// offered it; none of its objects' puts went in two hops; and this device never offered its
    /// person's records first hand to anyone but the person's own devices.
    pub(crate) fn address_hidden(&self, s: &hoi_store::Stored) -> bool {
        let saved = &self.scatter.saved;
        let Some(c) = s.record.community else { return false };
        // a device that publishes rosters tells its links whose it is
        if self.state.home.is_some()
            || saved.exposed_at.is_some()
            || self.state.settings.share_holders
            || self.is_private_community(&c)
            || Some(s.author_root) != self.me_root()
        {
            return false;
        }
        if !self.device_kp.as_ref().is_some_and(|d| d.public() == s.record.author) {
            return false;
        }
        let went: Vec<Id> = match saved.own_pointers.get(&s.id) {
            Some(pointers) if !saved.whole.contains(&s.id) => pointers.iter().copied().collect(),
            _ => vec![s.id],
        };
        if !went.iter().all(|id| saved.injected.contains(id)) {
            return false;
        }
        !saved.kept.iter().any(|(object, k)| k.records.contains(&s.id) && saved.two_hop.contains_key(object))
    }

    /// Authenticated reports from our home are distinct from hiding our address from it.
    pub(crate) fn home_delivery_status(&self, id: Id) -> Option<String> {
        let s = &self.scatter.saved;
        let pointers = s.own_pointers.get(&id);
        if pointers.is_none() && s.home_delivery.get(&id).is_some_and(|s| s == "placing") {
            return Some("placing".into());
        }
        let states: Vec<_> =
            pointers.map(|ids| ids.iter().filter_map(|id| s.home_delivery.get(id)).collect()).unwrap_or_else(|| s.home_delivery.get(&id).into_iter().collect());
        if states.is_empty() {
            return None;
        }
        let state = if states.iter().any(|s| s.as_str() == "pending") {
            "pending"
        } else if states.iter().any(|s| s.as_str() == "fallback") {
            "fallback"
        } else {
            "injected"
        };
        Some(format!("{}-{}", if pointers.is_some() && !s.whole.contains(&id) { "pieces" } else { "whole" }, state))
    }

    /// Drops the marks of records and objects no longer here.
    pub(crate) fn prune_inject_marks(&mut self) {
        let now = self.now();
        let before = (self.scatter.saved.injected.len(), self.scatter.saved.own_pointers.len(), self.scatter.saved.two_hop.len());
        let store = &self.store;
        self.scatter.saved.injected.retain(|id| store.contains(id));
        self.scatter.saved.home_delivery.retain(|id, _| store.contains(id));
        self.scatter.saved.own_pointers.retain(|id, _| store.contains(id));
        let kept = &self.scatter.saved.kept;
        self.scatter.saved.two_hop.retain(|object, at| kept.contains_key(object) || now < *at + 86_400);
        let after = (self.scatter.saved.injected.len(), self.scatter.saved.own_pointers.len(), self.scatter.saved.two_hop.len());
        if before != after {
            self.scatter.dirty = true;
        }
    }

    /// An injection for this node: opened, and held a random moment before it is taken in.
    pub(crate) fn take_injection(&mut self, peer: &str, env: Vec<u8>) -> bool {
        let Some(endpoint) = self.transport_kp.as_ref().map(|k| k.public()) else { return true };
        if !self.forwards_pieces() || self.scatter.injections.len() >= MAX_INJECTIONS {
            return true;
        }
        let opened = self.fresh_keys().iter().find_map(|k| envelope::open_inject(&endpoint.0, k, &env).ok());
        let Some(wires) = opened else {
            let forwarder = self.peers.get(peer).is_some_and(|p| p.scatter.forwards);
            return forwarder || !self.peer_scores.penalize(peer, 2.0);
        };
        let at = self.now() + crate::scatter::uniform(INJECT_JITTER as usize + 1) as u64;
        self.scatter.injections.push((at, wires));
        true
    }

    /// One round: injections whose moment came are taken in and passed on to everyone; own
    /// records that did not come back in time are offered as before.
    pub(crate) fn inject_tick(&mut self) {
        let now = self.now();
        if self.scatter.injections.iter().any(|(at, _)| *at <= now) {
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.scatter.injections).into_iter().partition(|(at, _)| *at <= now);
            self.scatter.injections = later;
            for (_, wires) in due {
                // from nobody this node links to in particular: passed on to every peer
                self.ingest_batch(wires, &|_| Source::Peer { peer: String::new(), live: true }, &mut |_, _, _| {});
            }
        }
        let late: Vec<Id> = self.scatter.saved.injecting.iter().filter(|(_, until)| **until <= now).map(|(id, _)| *id).collect();
        for id in late {
            self.scatter.saved.injecting.remove(&id);
            self.scatter.dirty = true;
            self.gossip(&id, None);
        }
    }

    /// One of this device's own records came back from a link: it is offered as any other now,
    /// and if it was still held, an injector took it.
    pub(crate) fn came_back(&mut self, id: &Id) {
        let now = self.now();
        if let Some(until) = self.scatter.saved.injecting.remove(id) {
            if now < until {
                self.scatter.saved.injected.insert(*id);
            }
            self.scatter.dirty = true;
        }
    }
}
