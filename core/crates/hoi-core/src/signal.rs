//! Direct links between browsers. Two members' browsers linked to the same native node
//! open a WebRTC data channel to each other, and records and media flow between them without
//! going through that node.
//!
//! - **The node** says `signal`. To each browser that said `direct` it gives a random handle for
//!   the link, and lists to it, per community, the handles of the other such browsers that share
//!   that community (`here`). It passes `sig` (an offer or an answer, opaque to it) from one
//!   such browser to another that shares a community with it, naming the sender by its handle.
//!   Its WebRTC port answers plain STUN, so each browser learns the address it is seen from
//!   (hoi-net `rtc`). It sees who talks to whom by handle, as it sees their links anyway.
//! - **A browser** with "Direct links" on says `direct`. Of two handles in a `here`, the lower one
//!   offers: the host makes an offer with the node as its STUN server, and the answer comes back
//!   the same way. The data channel then carries the usual sync protocol, as a link to a peer:
//!   every record is checked as from anyone, and the hello names communities only hashed
//!   (PROTOCOL.md §14). The browsers learn each other's addresses, which is why it is off at
//!   first. At most [`MAX_DIRECT`] such links, and none for private communities.

use std::collections::{BTreeMap, BTreeSet};

use hoi_sync::Msg;
use hoi_types::Id;
use serde::{Deserialize, Serialize};

use crate::node::Node;

/// Direct links a browser keeps at most.
pub const MAX_DIRECT: usize = 4;
/// `sig` messages a node passes on for one link per minute at most.
const SIGS_PER_MINUTE: u32 = 30;
/// A handle offered to and not linked is offered again after this long.
const RETRY_SECS: u64 = 600;

/// A link's handle at a node (random per link, crate::signal).
type Handle = [u8; 16];
/// Which of two direct links to the same browser both sides keep: the lowest of the offering
/// side's key, the lower handle and the higher handle.
type Rank = ([u8; 16], Handle, Handle);

/// For the browser's host (core.worker.ts): start a link, or hand on a peer's offer or answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "t")]
pub enum DirectEvent {
    /// Make an offer to `to` (a handle, hex) through the node at `via`, with `stun` (`stun:`
    /// addresses of that node) as ICE servers, and pass it on with [`Node::signal`].
    Offer { via: String, to: String, stun: Vec<String> },
    /// An offer or an answer from `from` through `via`. An offer is answered with
    /// [`Node::signal`]; an answer completes the link this side offered.
    Signal { via: String, from: String, stun: Vec<String>, data: Vec<u8> },
    /// Close this direct link: another one goes to the same browser.
    Close { peer: String },
}

#[derive(Debug, Default)]
pub(crate) struct Signals {
    /// On a node: the host's WebRTC listener runs, so STUN is answered and signals passed.
    pub relay_on: bool,
    /// On a node: each browser link's handle, and the `here` lists last sent it.
    pub handles: BTreeMap<String, [u8; 16]>,
    pub sent_here: BTreeMap<String, BTreeMap<Id, Vec<[u8; 16]>>>,
    /// On a node: `sig` messages passed for a link this minute.
    pub pace: BTreeMap<String, (u64, u32)>,
    /// On a browser: per node, our handle there and the handles it listed, per community.
    pub here: BTreeMap<String, (Handle, BTreeMap<Id, Vec<Handle>>)>,
    /// On a browser: links tried lately, by node and the other side's handle: when, and whether
    /// this side offered. One is tried again only after [`RETRY_SECS`].
    pub tried: BTreeMap<(String, [u8; 16]), (u64, bool)>,
    /// For the host.
    pub due: Vec<DirectEvent>,
    /// On a browser: a random key for this run of the app, sent with every offer and answer, so
    /// the other side links with it once however many handles it has. And the other sides'
    /// keys, by node and handle; and each open direct link's other key and rank.
    pub me: Option<[u8; 16]>,
    pub keys: BTreeMap<(String, [u8; 16]), [u8; 16]>,
    pub links: BTreeMap<String, ([u8; 16], Rank)>,
}

impl Node {
    /// The host's WebRTC listener runs (hoi-net `rtc`), so this node answers STUN and passes
    /// signals between browsers; or it stopped.
    pub fn set_signal_relay(&mut self, on: bool) {
        let on = on && self.can_relay();
        if self.signals.relay_on != on {
            self.signals.relay_on = on;
            self.announce();
        }
    }

    /// Whether this node passes signals between browsers, and answers STUN: a native node whose
    /// WebRTC listener runs, while it takes members.
    pub fn passes_signals(&self) -> bool {
        self.signals.relay_on && self.accepts_members()
    }

    /// Whether this browser takes direct links: the person turned them on.
    pub(crate) fn takes_direct(&self) -> bool {
        !self.can_relay() && self.state.settings.direct_links
    }

    /// Direct links open now, and those tried that haven't opened, together.
    fn direct_in_use(&self) -> usize {
        let open: BTreeSet<&(String, [u8; 16])> = self.peers.values().filter_map(|p| p.meta.direct.as_ref()).collect();
        open.len() + self.signals.tried.keys().filter(|k| !open.contains(k)).count()
    }

    /// For the host: links to start and offers or answers to hand on.
    pub fn take_direct(&mut self) -> Vec<DirectEvent> {
        std::mem::take(&mut self.signals.due)
    }

    // ------------------------------------------------------------------ on the node

    /// Gives each browser link that said `direct` a handle, and sends each the handles of the
    /// others it shares a community with, where that changed.
    pub(crate) fn refresh_here(&mut self) {
        if !self.signals.relay_on {
            return;
        }
        let takers: Vec<(String, BTreeSet<Id>)> = self
            .peers
            .iter()
            .filter(|(_, p)| p.said_hello && p.meta.direct.is_none() && p.features.contains(hoi_sync::FEATURE_DIRECT))
            .map(|(k, p)| (k.clone(), p.communities.iter().filter(|c| !self.is_private_community(c)).copied().collect()))
            .collect();
        self.signals.handles.retain(|k, _| takers.iter().any(|(t, _)| t == k));
        self.signals.sent_here.retain(|k, _| takers.iter().any(|(t, _)| t == k));
        for (k, _) in &takers {
            if !self.signals.handles.contains_key(k) {
                let Ok(h) = hoi_crypto::random_bytes::<16>() else { return };
                self.signals.handles.insert(k.clone(), h);
            }
        }
        for (k, mine) in &takers {
            let you = self.signals.handles[k];
            for c in mine {
                let mut others: Vec<[u8; 16]> =
                    takers.iter().filter(|(o, theirs)| o != k && theirs.contains(c)).map(|(o, _)| self.signals.handles[o]).collect();
                others.sort();
                others.truncate(hoi_sync::MAX_HERE);
                let sent = self.signals.sent_here.entry(k.clone()).or_default();
                if sent.get(c) == Some(&others) {
                    continue;
                }
                sent.insert(*c, others.clone());
                self.send_msg(k, Msg::Here { community: *c, you, handles: others });
            }
        }
    }

    /// A browser passes an offer or an answer to another: only between two links that said
    /// `direct` and share a community, at most [`SIGS_PER_MINUTE`] a minute from each.
    fn pass_sig(&mut self, peer: &str, to: [u8; 16], data: Vec<u8>) -> bool {
        if !self.passes_signals() {
            return true;
        }
        let Some(from) = self.signals.handles.get(peer).copied() else { return true };
        let Some(target) = self.signals.handles.iter().find(|(_, h)| **h == to).map(|(k, _)| k.clone()) else { return true };
        let shared = match (self.peers.get(peer), self.peers.get(&target)) {
            (Some(a), Some(b)) => a.communities.iter().any(|c| b.communities.contains(c) && !self.is_private_community(c)),
            _ => false,
        };
        if !shared || target == peer {
            return true;
        }
        let minute = self.now() / 60;
        let pace = self.signals.pace.entry(peer.to_string()).or_insert((minute, 0));
        if pace.0 != minute {
            *pace = (minute, 0);
        }
        pace.1 += 1;
        if pace.1 > SIGS_PER_MINUTE {
            return !self.peer_scores.penalize(peer, 1.0);
        }
        self.send_msg(&target, Msg::Sig { peer: from, data });
        true
    }

    // ------------------------------------------------------------------ on the browser

    /// `stun:` addresses of a node this browser reached over WebRTC: the routes it dialed.
    fn stun_of(&self, via: &str) -> Vec<String> {
        let Some(p) = self.peers.get(via) else { return Vec::new() };
        let mut out: Vec<String> = p.meta.dialed.iter().filter_map(|r| crate::net::rtc_parts(r).map(|(_, addr)| format!("stun:{addr}"))).collect();
        out.sort();
        out.dedup();
        out.truncate(4);
        out
    }

    /// A node listed browsers this one shares a community with. The lower handle offers.
    fn on_here(&mut self, via: &str, c: Id, you: [u8; 16], handles: Vec<[u8; 16]>) {
        if !self.takes_direct() || !self.peers.get(via).is_some_and(|p| p.features.contains(hoi_sync::FEATURE_SIGNAL)) {
            return;
        }
        let entry = self.signals.here.entry(via.to_string()).or_insert((you, BTreeMap::new()));
        if entry.0 != you {
            *entry = (you, BTreeMap::new());
        }
        entry.1.insert(c, handles);
        self.offer_direct();
    }

    /// Asks the host for offers to the handles this browser should link with, up to
    /// [`MAX_DIRECT`] links open and tried together.
    pub(crate) fn offer_direct(&mut self) {
        if !self.takes_direct() {
            return;
        }
        let now = self.now();
        let open: BTreeSet<(String, [u8; 16])> = self.peers.values().filter_map(|p| p.meta.direct.clone()).collect();
        self.signals.tried.retain(|k, (at, _)| open.contains(k) || (self.peers.contains_key(&k.0) && *at + RETRY_SECS > now));
        let mut room = MAX_DIRECT.saturating_sub(self.direct_in_use());
        let wanted: BTreeSet<(String, [u8; 16])> =
            self.signals.here.iter().flat_map(|(via, (you, by))| by.values().flatten().filter(move |h| *h > you).map(move |h| (via.clone(), *h))).collect();
        for key in wanted {
            if room == 0 {
                break;
            }
            if self.signals.tried.contains_key(&key) || open.contains(&key) {
                continue;
            }
            let stun = self.stun_of(&key.0);
            self.signals.tried.insert(key.clone(), (now, true));
            self.signals.due.push(DirectEvent::Offer { via: key.0, to: hoi_types::hex_encode(&key.1), stun });
            room -= 1;
        }
    }

    fn my_key(&mut self) -> Option<[u8; 16]> {
        if self.signals.me.is_none() {
            self.signals.me = hoi_crypto::random_bytes::<16>().ok();
        }
        self.signals.me
    }

    /// An offer or an answer from another browser, through `via`: its key, then the
    /// description. One from a browser this one already links with, or is linking with by
    /// another handle, goes no further.
    fn on_sig(&mut self, via: &str, from: [u8; 16], data: Vec<u8>) {
        if !self.takes_direct() || !self.peers.get(via).is_some_and(|p| p.features.contains(hoi_sync::FEATURE_SIGNAL)) {
            return;
        }
        let Some(theirs) = data.get(..16).and_then(|k| <[u8; 16]>::try_from(k).ok()) else { return };
        if Some(theirs) == self.my_key() || data.len() == 16 {
            return;
        }
        let data = data[16..].to_vec();
        let key = (via.to_string(), from);
        let open: BTreeSet<(String, [u8; 16])> = self.peers.values().filter_map(|p| p.meta.direct.clone()).collect();
        let linked =
            self.signals.keys.iter().any(|(other, k)| *k == theirs && *other != key && (open.contains(other) || self.signals.tried.contains_key(other)));
        if linked {
            return;
        }
        match self.signals.tried.get(&key) {
            // the answer to our offer
            Some((_, true)) => {}
            // an offer from a lower handle: it takes a slot, unless this one is already tried
            Some((_, false)) => {}
            None => {
                if self.direct_in_use() >= MAX_DIRECT {
                    return;
                }
                let now = self.now();
                self.signals.tried.insert(key.clone(), (now, false));
            }
        }
        self.signals.keys.insert(key, theirs);
        let stun = self.stun_of(via);
        self.signals.due.push(DirectEvent::Signal { via: via.to_string(), from: hoi_types::hex_encode(&from), stun, data });
    }

    /// For the host: pass an offer or an answer to `to` (a handle, hex) through `via`.
    pub fn signal(&mut self, via: &str, to: &str, data: &[u8]) -> crate::api::R<()> {
        let Some(to) = hoi_types::hex_decode(to).ok().and_then(|b| <[u8; 16]>::try_from(b).ok()) else {
            return Err(crate::error::ApiError::invalid("Not a handle."));
        };
        if data.is_empty() || data.len() + 16 > hoi_sync::MAX_SIG {
            return Err(crate::error::ApiError::invalid("That offer is too large."));
        }
        if self.takes_direct() && self.peers.get(via).is_some_and(|p| p.features.contains(hoi_sync::FEATURE_SIGNAL)) {
            let Some(me) = self.my_key() else { return Err(crate::error::ApiError::new("internal", "no randomness")) };
            let mut d = me.to_vec();
            d.extend_from_slice(data);
            self.send_msg(via, Msg::Sig { peer: to, data: d });
        }
        Ok(())
    }

    /// A direct link opened. Of two to the same browser, both sides keep the same one: the
    /// lowest by the offering side's key and the two handles; the host closes the other.
    pub(crate) fn direct_opened(&mut self, peer: &str) {
        let Some(p) = self.peers.get(peer) else { return };
        let Some((via, theirs_h)) = p.meta.direct.clone() else { return };
        let incoming = p.meta.incoming;
        let (Some(theirs), Some(me)) = (self.signals.keys.get(&(via.clone(), theirs_h)).copied(), self.my_key()) else { return };
        let mine_h = self.signals.here.get(&via).map_or([0; 16], |(you, _)| *you);
        let rank = (if incoming { theirs } else { me }, mine_h.min(theirs_h), mine_h.max(theirs_h));
        self.signals.links.insert(peer.to_string(), (theirs, rank));
        let same: Vec<(String, Rank)> =
            self.signals.links.iter().filter(|(k, (t, _))| *t == theirs && self.peers.contains_key(*k)).map(|(k, (_, r))| (k.clone(), *r)).collect();
        let Some(keep) = same.iter().min_by_key(|(_, r)| *r).map(|(k, _)| k.clone()) else { return };
        for (k, _) in same {
            if k != keep {
                self.signals.links.remove(&k);
                self.signals.due.push(DirectEvent::Close { peer: k });
            }
        }
    }

    /// `here` and `sig` as they arrive. False ends the link.
    pub(crate) fn on_signal_msg(&mut self, peer: &str, m: Msg) -> bool {
        let to_node = self.signals.relay_on;
        match m {
            Msg::Here { community, you, handles } if !to_node => self.on_here(peer, community, you, handles),
            Msg::Sig { peer: to, data } if to_node => return self.pass_sig(peer, to, data),
            Msg::Sig { peer: from, data } => self.on_sig(peer, from, data),
            _ => {}
        }
        true
    }

    /// A link went: forget what came through it.
    pub(crate) fn signal_disconnect(&mut self, peer: &str) {
        self.signals.here.remove(peer);
        self.signals.links.remove(peer);
        let open: BTreeSet<(String, [u8; 16])> = self.peers.values().filter_map(|p| p.meta.direct.clone()).collect();
        self.signals.tried.retain(|(via, _), _| via != peer);
        let tried = &self.signals.tried;
        self.signals.keys.retain(|k, _| open.contains(k) || tried.contains_key(k));
        self.signals.pace.remove(peer);
        if self.signals.handles.remove(peer).is_some() {
            self.signals.sent_here.remove(peer);
            self.refresh_here();
        }
    }
}
