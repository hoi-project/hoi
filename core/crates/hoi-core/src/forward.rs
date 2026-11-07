//! Forwarding, phase S3 (SCATTER.md §5.8): a public node passes sealed `piece` requests on to the
//! holder they name and the holder's sealed answer back, so a holder never sees the address of
//! the device that stores or fetches, and the forwarder never sees a piece.
//!
//! ```text
//! device ──piece { op, rid: a, to: H, env }──► forwarder ──piece { op, rid: b, env }──► holder H
//! device ◄──piece { answer, rid: a, env' }──── forwarder ◄──piece { answer, rid: b, env' }── H
//! ```
//!
//! A forwarder reads nothing it passes: `env` is sealed to the holder and `env'` to the asker's
//! one-time key. What it keeps is what routing an answer needs while it is on its way (which
//! link asked, under which id) and counters; nothing about which holder a device used outlives
//! the answer. It passes requests only to holders it is linked to now, or to holders it saw
//! itself and can dial (its own node list's holders: nobody makes it dial an address of their
//! choosing), queueing a few for a short while until the link is up. A holder it cannot reach
//! gets the asker a `gone` at once, so the asker tries another way rather than waiting.

use std::collections::BTreeMap;

use hoi_sync::{Msg, PieceOp};
use hoi_types::Id;

use crate::Node;
use crate::net::Role;

/// Requests waiting for a link to their holder, in all, and their bytes.
const MAX_WAITING: usize = 128;
const MAX_WAITING_BYTES: usize = 32 << 20;
/// How long a request waits for its holder's link before the asker hears `gone`.
const WAIT_FOR: u64 = 20;
/// Answers on their way back, at most: more and the oldest are forgotten (their askers time out).
const MAX_RELAYS: usize = 4096;
/// Requests a link that only forwards (another node's forwarder, never a device) may send a
/// holder per minute, and put bytes: many devices' requests arrive over it.
pub(crate) const FORWARDER_REQUESTS_PER_MINUTE: u32 = 6000;
pub(crate) const FORWARDER_PUT_BYTES_PER_MINUTE: u64 = 128 << 20;

/// An answer this node waits to pass back.
struct Relay {
    asker: String,
    rid: u32,
    at: u64,
}

/// A request waiting for a link to its holder.
struct Waiting {
    asker: String,
    rid: u32,
    op: PieceOp,
    env: Vec<u8>,
    at: u64,
}

/// What this node does as a forwarder: in memory only.
#[derive(Default)]
pub(crate) struct Forwarding {
    /// (holder link, id on that link) → who asked.
    relays: BTreeMap<(String, u32), Relay>,
    /// Holder endpoint → requests waiting for a link to it.
    waiting: BTreeMap<Id, Vec<Waiting>>,
    /// Recent actual forwarding activity, removed with the connection.
    used: BTreeMap<String, u64>,
    /// Totals, for `admin pieces`: requests passed on, answered `gone`, and passed to another
    /// forwarder that vouches for their holder.
    pub passed: u64,
    pub gone: u64,
    pub relayed: u64,
}

impl Node {
    /// Whether this node passes `piece` requests on: a public node, which serves strangers and
    /// has an endpoint key. Linked peers hear of it in hellos (`fw`).
    pub(crate) fn forwards_pieces(&self) -> bool {
        self.transport_kp.is_some() && self.is_public_node()
    }

    /// The linked peer whose authenticated endpoint is `holder` and that holds pieces.
    fn holder_link(&self, holder: &Id) -> Option<String> {
        let hex = holder.hex();
        self.peers
            .iter()
            .find(|(_, p)| p.said_hello && p.role() == Role::Peer && p.scatter.holder.is_some() && p.meta.endpoint.as_deref() == Some(hex.as_str()))
            .map(|(k, _)| k.clone())
    }

    /// A request with `to` from a linked peer: pass it on, queue it while its holder is dialed,
    /// or answer `gone`.
    pub(crate) fn forward_piece(&mut self, asker: &str, op: PieceOp, rid: u32, holder: Id, env: Vec<u8>) {
        let now = self.now();
        let own = self.transport_kp.as_ref().map(|k| k.public());
        let asker_endpoint = self.peers.get(asker).and_then(|p| p.meta.endpoint.clone());
        if !self.forwards_pieces() || Some(holder) == own || asker_endpoint.as_deref() == Some(holder.hex().as_str()) {
            self.say_gone(asker, rid);
            return;
        }
        if let Some(link) = self.holder_link(&holder) {
            self.pass_on(asker, rid, op, &link, env, None, now);
            return;
        }
        // A holder this node saw itself and knows how to reach: dial it, and hold the request
        // until the link is up.
        let known = self.scatter.saved.holders.get(&holder).is_some_and(|s| !s.routes.is_empty());
        // One it does not know: a link of its own that forwards and vouches for it passes it on
        // (S6), one hop more and never two: a request that came from a forwarder goes no further
        if !known
            && !self.peers.get(asker).is_some_and(|p| p.scatter.forwards)
            && let Some(link) = self.vouching_link(&holder, asker)
        {
            self.scatter.forward.relayed += 1;
            self.pass_on(asker, rid, op, &link, env, Some(holder), now);
            return;
        }
        let (count, bytes) = self.scatter.forward.waiting.values().flatten().fold((0, 0), |(c, b), w| (c + 1, b + w.env.len()));
        if !known || count >= MAX_WAITING || bytes + env.len() > MAX_WAITING_BYTES {
            self.say_gone(asker, rid);
            return;
        }
        self.scatter.forward.waiting.entry(holder).or_default().push(Waiting { asker: asker.to_string(), rid, op, env, at: now });
    }

    /// A linked forwarder, not `asker`, whose node list vouches for `holder` with its key: the
    /// one whose vouch is steadiest, then by name.
    fn vouching_link(&self, holder: &Id, asker: &str) -> Option<String> {
        self.peers
            .iter()
            .filter(|(k, p)| k.as_str() != asker && p.said_hello && p.scatter.forwards && p.role() == Role::Peer)
            .filter_map(|(k, p)| p.scatter.vouches.iter().find(|v| v.endpoint == *holder && v.key.is_some()).map(|v| (std::cmp::Reverse(v.up), k.clone())))
            .min()
            .map(|(_, k)| k)
    }

    #[allow(clippy::too_many_arguments)]
    fn pass_on(&mut self, asker: &str, rid: u32, op: PieceOp, link: &str, env: Vec<u8>, to: Option<Id>, now: u64) {
        let forwarded = self.next_rid();
        let f = &mut self.scatter.forward;
        if f.relays.len() >= MAX_RELAYS
            && let Some(oldest) = f.relays.iter().min_by_key(|(_, r)| r.at).map(|(k, _)| k.clone())
        {
            f.relays.remove(&oldest);
        }
        f.relays.insert((link.to_string(), forwarded), Relay { asker: asker.to_string(), rid, at: now });
        f.used.insert(link.to_string(), now);
        f.passed += 1;
        self.send_msg(link, Msg::Piece { op, rid: forwarded, to, env });
    }

    fn say_gone(&mut self, asker: &str, rid: u32) {
        self.scatter.forward.gone += 1;
        self.send_msg(asker, Msg::Piece { op: PieceOp::Gone, rid, to: None, env: vec![0] });
    }

    /// An answer from a holder this node passed a request to: back to whoever asked. False when
    /// the answer is not one this node waits to pass on (it is for this node's own request).
    pub(crate) fn forward_answer(&mut self, holder_link: &str, op: PieceOp, rid: u32, env: &[u8]) -> bool {
        let Some(r) = self.scatter.forward.relays.remove(&(holder_link.to_string(), rid)) else { return false };
        if self.peers.contains_key(&r.asker) {
            self.send_msg(&r.asker, Msg::Piece { op, rid: r.rid, to: None, env: env.to_vec() });
        }
        true
    }

    /// A peer said hello: requests waiting for it go out.
    pub(crate) fn forward_flush(&mut self) {
        if self.scatter.forward.waiting.is_empty() {
            return;
        }
        let now = self.now();
        let holders: Vec<Id> = self.scatter.forward.waiting.keys().copied().collect();
        for h in holders {
            let Some(link) = self.holder_link(&h) else { continue };
            for w in self.scatter.forward.waiting.remove(&h).unwrap_or_default() {
                if self.peers.contains_key(&w.asker) {
                    self.pass_on(&w.asker, w.rid, w.op, &link, w.env, None, now);
                }
            }
        }
    }

    /// Requests that waited too long for their holder are answered `gone`; answers that never
    /// came are forgotten (their askers time out on their own).
    pub(crate) fn forward_tick(&mut self) {
        self.forward_flush();
        let now = self.now();
        let mut expired = Vec::new();
        for list in self.scatter.forward.waiting.values_mut() {
            let (old, keep): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(list).into_iter().partition(|w| now >= w.at + WAIT_FOR);
            *list = keep;
            expired.extend(old.into_iter().map(|w| (w.asker, w.rid)));
        }
        self.scatter.forward.waiting.retain(|_, l| !l.is_empty());
        for (asker, rid) in expired {
            if self.peers.contains_key(&asker) {
                self.say_gone(&asker, rid);
            }
        }
        let timeout = crate::scatter::ANSWER_TIMEOUT;
        self.scatter.forward.relays.retain(|_, r| now < r.at + timeout);
    }

    /// A link went away: answers that were to come over it never will (their askers hear
    /// `gone`), and nothing is passed back to it any more.
    pub(crate) fn forward_disconnect(&mut self, peer: &str) {
        self.scatter.forward.used.remove(peer);
        let lost: Vec<(String, u32)> = self.scatter.forward.relays.iter().filter(|((l, _), _)| l == peer).map(|((_, _), r)| (r.asker.clone(), r.rid)).collect();
        self.scatter.forward.relays.retain(|(l, _), r| l != peer && r.asker != peer);
        for list in self.scatter.forward.waiting.values_mut() {
            list.retain(|w| w.asker != peer);
        }
        self.scatter.forward.waiting.retain(|_, l| !l.is_empty());
        for (asker, rid) in lost {
            if self.peers.contains_key(&asker) {
                self.say_gone(&asker, rid);
            }
        }
    }

    /// Routes of holders requests wait for, for the host to dial (they join `bootstrap_hints`).
    pub(crate) fn forward_dial_routes(&self) -> Vec<String> {
        self.scatter.forward.waiting.keys().filter_map(|h| self.scatter.saved.holders.get(h)).flat_map(|s| s.routes.iter().cloned()).collect()
    }

    /// Requests passed on and answered `gone` since start.
    pub fn forward_stats(&self) -> (u64, u64) {
        (self.scatter.forward.passed, self.scatter.forward.gone)
    }

    /// Keep a temporary holder link while replies are pending or it carried work recently.
    pub fn forwarding_active(&self, peer: &str, idle_seconds: u64) -> bool {
        self.scatter.forward.relays.keys().any(|(link, _)| link == peer)
            || self.scatter.forward.used.get(peer).is_some_and(|at| self.now().saturating_sub(*at) < idle_seconds)
    }
}
