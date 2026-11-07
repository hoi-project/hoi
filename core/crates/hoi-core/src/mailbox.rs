//! Mailboxes: private groups without a relay (SCATTER.md §5.13, phase S2).
//!
//! A private group agrees, inside its MLS group context, on a few holders in independent places
//! (the owner picks them, a commit changes them, a Welcome gives them to newcomers). Each member
//! writes the group's sealed MLS messages, the same objects the relay used to carry, into its own
//! numbered slots on those holders: slot `seq` of member `leaf` in an epoch lives at
//!
//! ```text
//! loc = BLAKE3_keyed(derive_key("hoi mailbox slot v1", epoch relay secret), u32(leaf) ‖ u32(seq))
//! ```
//!
//! Only members of that epoch can compute it, every membership change starts a fresh set, and
//! holders see random locators, never a listing. To a holder a slot is just a piece: the same
//! sealed `put`, `has` and `get`, the same stamp, the same rule that nothing is overwritten.
//!
//! **The bell.** So that an idle member checks one place rather than every other member's next
//! slot, each epoch has a bell: a second numbered sequence, at the slots of leaf `u32::MAX`, which
//! no member has. Whoever wrote a message rings it on each mailbox: a small random object in the
//! next bell slot that mailbox does not hold yet (a holder never overwrites, so two members
//! ringing at once take two slots). Members probe the bell, one mailbox at a time, each at its own
//! count; a ring sends them over every member's next slot (`has`, a 64-byte answer that carries
//! the length, then `get`), and a member found writing is followed until it stops. Every ten
//! minutes they look at every member anyway, in case a ring was lost. A membership change goes in
//! the slots of the epoch it ends, and rings that epoch's bell, so members who read it move on to
//! the new epoch's slots.
//!
//! Nothing here is new to MLS or to the relay format: a fetched slot goes through
//! `private_receive` like a relay object, and a slot is written from the same outbox.

use std::collections::{BTreeMap, BTreeSet};

use hoi_crypto::envelope::{self, Kind, Reply, Request, Stored, XKey};
use hoi_crypto::scatter as sc;
use hoi_sync::place::{self, Candidate};
use hoi_sync::{Msg, PieceOp};
use hoi_types::Id;
use hoi_types::cbor::{self, Value};
use serde::{Deserialize, Serialize};

use crate::Node;
use crate::private::{CipherObject, DROP_KEY_DUE, KeyWork, MailEpoch};

/// Holders a group's mailboxes are kept on: as many as it gets, at least two.
pub const MAILBOXES: usize = 4;
pub const MIN_MAILBOXES: usize = 2;
/// Confirmations a written slot needs before the message counts as sent.
const WRITE_QUORUM: usize = 2;
/// How long a slot is kept (the relay kept objects as long).
const SLOT_TTL: u64 = 30 * 86_400;
/// How often a member is checked: at once after something new, then less and less often.
const POLL_FAST: u64 = 5;
const POLL_SLOW: u64 = 120;
/// How often the outbox is filled with what members still lack.
const OUTBOX_EVERY: u64 = 15;
/// How slowly, at most, the slots of committers ahead are looked at in a contested epoch.
const CONTEST_POLL: u64 = 600;
/// A group's mailboxes not answering for this long (more than the slowest look at the bell)
/// means this device was away: it catches up again before changing membership by itself.
const AWAY: u64 = 2 * POLL_SLOW + 60;
/// The leaf whose slots are an epoch's bell: none has this index.
const BELL: u32 = u32::MAX;
/// How long a ring is kept: a day or two (holders count in days).
const RING_DAYS: u32 = 2;
/// Every member's next slot is looked at this often even without a ring.
const FULL_SWEEP: u64 = 600;
/// Probes a member gets after a ring before it counts as quiet: a message may be on another
/// mailbox than the one asked first.
const CHASE: u8 = 3;
/// After this long without seeing a listed mailbox, the owner replaces it.
const MAILBOX_STALE: u64 = 2 * 86_400;
/// How often the owner looks at the list, and how often while the group has none yet.
const MAINTAIN_EVERY: u64 = 600;
const CHOOSE_EVERY: u64 = 30;
/// A drop key's holders the list names at most.
pub(crate) const MAX_HOLDERS: usize = 64;
/// Puts and probes in flight per group.
const MAX_INFLIGHT: usize = 32;
/// Messages kept for another try (an attachment that came before its post).
const MAX_RETRY: usize = 64;
const RETRY_FOR: u64 = 3600;

/// A holder a group keeps its mailboxes on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Mailbox {
    pub endpoint: Id,
    /// How members reach it: its published routes, iroh and WebRTC.
    pub routes: Vec<String>,
}

/// The list as the group context extension carries it: canonical CBOR,
/// `{ v: 1, m: [ { e: endpoint, r: [route] } ], d?: drop key, h?: [device] }`. `d`, the group's
/// drop key (Ed25519, public), is what pieces of the group's posts are stored with, so the
/// devices holding its secret can drop a removed post's pieces (SCATTER.md §5.11); `h`, the
/// committers' devices its secret was given to: once one of them leaves the line, a
/// new key is due. Older readers ignore both.
pub(crate) fn encode_list(list: &[Mailbox], drop_key: Option<[u8; 32]>, holders: &[Id]) -> Vec<u8> {
    let mut e = vec![
        ("v", Value::U(1)),
        (
            "m",
            Value::A(
                list.iter()
                    .map(|m| Value::map([("e", Value::B(m.endpoint.0.to_vec())), ("r", Value::A(m.routes.iter().map(|r| Value::T(r.clone())).collect()))]))
                    .collect(),
            ),
        ),
    ];
    if let Some(d) = drop_key {
        e.push(("d", Value::B(d.to_vec())));
        if !holders.is_empty() {
            e.push(("h", Value::A(holders.iter().take(MAX_HOLDERS).map(|h| Value::B(h.0.to_vec())).collect())));
        }
    }
    Value::map(e).encode()
}

/// The devices the listed drop key's secret was given to; none for a list without them.
pub(crate) fn decode_holders(bytes: &[u8]) -> Vec<Id> {
    let Ok(Value::M(e)) = cbor::decode(bytes) else { return vec![] };
    match e.into_iter().find(|(k, _)| k == "h").map(|(_, v)| v) {
        Some(Value::A(items)) if items.len() <= MAX_HOLDERS => items
            .into_iter()
            .map(|i| match i {
                Value::B(b) => Id::from_slice(&b).ok(),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default(),
        _ => vec![],
    }
}

/// The group's drop key, if its list names one.
pub(crate) fn decode_drop_key(bytes: &[u8]) -> Option<[u8; 32]> {
    let Value::M(e) = cbor::decode(bytes).ok()? else { return None };
    e.into_iter().find(|(k, _)| k == "d").and_then(|(_, v)| match v {
        Value::B(b) => b.try_into().ok(),
        _ => None,
    })
}

/// Reads a mailbox list; anything malformed reads as no list.
pub(crate) fn decode_list(bytes: &[u8]) -> Vec<Mailbox> {
    let read = || -> Option<Vec<Mailbox>> {
        let Value::M(e) = cbor::decode(bytes).ok()? else { return None };
        let mut m: BTreeMap<String, Value> = e.into_iter().collect();
        if m.remove("v") != Some(Value::U(1)) {
            return None;
        }
        let Some(Value::A(items)) = m.remove("m") else { return None };
        if items.len() > 2 * MAILBOXES {
            return None;
        }
        items
            .into_iter()
            .map(|i| {
                let Value::M(f) = i else { return None };
                let mut f: BTreeMap<String, Value> = f.into_iter().collect();
                let endpoint = match f.remove("e") {
                    Some(Value::B(b)) => Id::from_slice(&b).ok()?,
                    _ => return None,
                };
                let routes = match f.remove("r") {
                    Some(Value::A(r)) if r.len() <= 4 => r
                        .into_iter()
                        .map(|x| match x {
                            Value::T(s) if s.len() <= 128 => Some(s),
                            _ => None,
                        })
                        .collect::<Option<Vec<_>>>()?,
                    _ => return None,
                };
                Some(Mailbox { endpoint, routes })
            })
            .collect()
    };
    read().unwrap_or_default()
}

/// Where member `leaf`'s slot `seq` of an epoch is.
pub(crate) fn slot(key: &[u8; 32], leaf: u32, seq: u32) -> [u8; 32] {
    let mut msg = [0u8; 8];
    msg[..4].copy_from_slice(&leaf.to_be_bytes());
    msg[4..].copy_from_slice(&seq.to_be_bytes());
    *blake3::keyed_hash(key, &msg).as_bytes()
}

/// The slot key of an epoch, from its relay secret.
pub(crate) fn slot_key(relay_secret: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("hoi mailbox slot v1", relay_secret)
}

/// A message on its way into this device's slots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SlotState {
    pub epoch: u64,
    pub leaf: u32,
    pub seq: u32,
    pub loc: Id,
    /// The mailboxes of the epoch it was written in (a membership change goes to the old ones).
    pub mailboxes: Vec<Mailbox>,
    #[serde(default)]
    pub stored: BTreeSet<Id>,
    #[serde(default)]
    pub refused: BTreeSet<Id>,
    /// The slot key of its epoch, for ringing that epoch's bell once it is stored (a commit's
    /// epoch is over by then).
    pub bell: Id,
}

/// One group's progress through its mailboxes, kept between starts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Progress {
    /// The epoch `next_seq` counts in.
    pub write_epoch: u64,
    pub next_seq: u32,
    /// Messages being written, by object id.
    pub slots: BTreeMap<String, SlotState>,
    /// The epoch `read` counts in, and each other member's next slot.
    pub read_epoch: u64,
    pub read: BTreeMap<u32, u32>,
    /// The next bell slot to look at, per mailbox, in `read_epoch`.
    pub bells: BTreeMap<Id, u32>,
    /// The next bell slot to ring, per epoch and mailbox (the current epoch and the one before).
    pub rings: BTreeMap<u64, BTreeMap<Id, u32>>,
}

/// What a mailbox request was for.
#[derive(Debug, Clone)]
pub(crate) enum MailRequest {
    Put {
        c: Id,
        object: String,
        holder: Id,
    },
    Probe {
        c: Id,
        epoch: u64,
        leaf: u32,
        seq: u32,
        holder: Id,
    },
    Fetch {
        c: Id,
        epoch: u64,
        leaf: u32,
        seq: u32,
    },
    /// A look at the bell of mailbox `holder` at `seq`; `extra`: one past the next, beside the
    /// regular look.
    Bell {
        c: Id,
        epoch: u64,
        seq: u32,
        holder: Id,
        extra: bool,
    },
    /// A ring of the bell of epoch `epoch` on `holder`, at `seq`.
    Ring {
        c: Id,
        epoch: u64,
        seq: u32,
        holder: Id,
    },
    /// A look at, and the fetch of, the slot of a committer ahead in a contested epoch.
    ContestProbe {
        c: Id,
        epoch: u64,
        leaf: u32,
        seq: u32,
        holder: Id,
    },
    ContestFetch {
        c: Id,
        epoch: u64,
        leaf: u32,
        seq: u32,
    },
    /// An invitation stored for a newcomer to pick up, at slot `seq` of its key (crate::pickup).
    PickupPut {
        key: Id,
        holder: Id,
        seq: u32,
    },
    /// A newcomer's look at, and fetch of, a slot of its own pickup key.
    PickupProbe {
        holder: Id,
        seq: u32,
    },
    PickupFetch {
        holder: Id,
        seq: u32,
    },
}

#[derive(Debug, Clone, Copy, Default)]
struct Poll {
    next_at: u64,
    backoff: u64,
    busy: bool,
    misses: u32,
    /// Probes left before a member counts as quiet until the next ring or sweep.
    chase: u8,
}

/// A ring still to go out: the bell of an epoch (its slot key), on the mailboxes in `want`.
#[derive(Debug, Clone)]
struct Ring {
    key: [u8; 32],
    want: BTreeSet<Id>,
    inflight: BTreeSet<Id>,
}

/// Mailbox work in memory: when each member is checked next, what is in flight, what waits.
#[derive(Debug, Default)]
pub(crate) struct Runtime {
    polls: BTreeMap<(Id, u32), Poll>,
    writing: BTreeSet<(Id, String, Id)>,
    pub(crate) retry: Vec<(Id, Vec<u8>, u64)>,
    outbox_at: BTreeMap<Id, u64>,
    maintained_at: BTreeMap<Id, u64>,
    turn: usize,
    /// Rings to send, per group and epoch.
    rings: BTreeMap<(Id, u64), Ring>,
    /// When each group's members were last all looked at.
    swept_at: BTreeMap<Id, u64>,
    /// Per member, a slot found past the one it should write next (a look one ahead): read,
    /// but the slot before it is still waited for a few probes, so nothing is read out of
    /// order for good.
    ahead: BTreeMap<(Id, u32), u32>,
    /// Per member, a membership change found in a slot past its next one: held, not followed,
    /// until the slots before it are read (following it would end the epoch, and the member's
    /// earlier slots of it could never be read), or until they count as lost.
    held: BTreeMap<(Id, u32), (u32, Vec<u8>)>,
    /// Per group, the epoch this device reads and since when: a committer changes membership by
    /// itself only once it has read an epoch a while.
    reading: BTreeMap<Id, (u64, u64)>,
    /// Per group, since when its mailboxes have been answering without a stretch of [`AWAY`],
    /// and when they last did: a committer back from silence reads before it commits.
    heard: BTreeMap<Id, (u64, u64)>,
    /// Looks at committers ahead in contested epochs, by group, epoch and leaf.
    contest_polls: BTreeMap<(Id, u64, u32), Poll>,
    /// Invitations stored for newcomers, and a newcomer's looks for its own (crate::pickup).
    pub(crate) pickup: crate::pickup::PickupRuntime,
    /// Requests sent, by what they were for (tests count them).
    #[cfg(test)]
    pub(crate) sent: BTreeMap<&'static str, u64>,
}

#[cfg(test)]
impl Runtime {
    /// Drops every ring still to go out, as if they were lost.
    pub(crate) fn forget_rings(&mut self) {
        self.rings.clear();
    }
}

/// How this device reaches a holder for mailboxes now: through a forwarder in another place,
/// so the holder never sees this device's address, or over its own link to it.
#[derive(Debug, Clone)]
pub(crate) struct MailLink {
    pub peer: String,
    pub endpoint: Id,
    pub xk: [u8; 32],
    /// The holder, when `peer` is a forwarder that passes requests on to it.
    pub to: Option<Id>,
    pub candidate: Candidate,
    pub routes: Vec<String>,
    pub qualified: bool,
}

impl MailRequest {
    fn group(&self) -> Option<Id> {
        match self {
            MailRequest::Put { c, .. }
            | MailRequest::Probe { c, .. }
            | MailRequest::Fetch { c, .. }
            | MailRequest::Bell { c, .. }
            | MailRequest::Ring { c, .. }
            | MailRequest::ContestProbe { c, .. }
            | MailRequest::ContestFetch { c, .. } => Some(*c),
            MailRequest::PickupPut { .. } | MailRequest::PickupProbe { .. } | MailRequest::PickupFetch { .. } => None,
        }
    }
}

impl Node {
    /// Every holder this device can reach for mailboxes, by endpoint: through a forwarder
    /// that vouches for it and is not in its place when there is one, else over a link to it.
    pub(crate) fn mail_links(&self) -> BTreeMap<Id, MailLink> {
        let forwarders = self.forwarders();
        let reach = self.reachable_holders(&forwarders, None);
        let mut all: Vec<Candidate> = forwarders.iter().map(|f| f.candidate).collect();
        all.extend(reach.iter().map(|h| h.candidate));
        let place = place::places(&all);
        let mut out = BTreeMap::new();
        for (j, h) in reach.iter().enumerate() {
            let own = place[forwarders.len() + j];
            let mut via: Vec<&String> = h.via.iter().filter(|f| forwarders.iter().position(|x| &x.peer == *f).is_some_and(|i| place[i] != own)).collect();
            via.sort();
            let Some(f) = via.first() else { continue };
            out.insert(
                h.endpoint,
                MailLink {
                    peer: (*f).clone(),
                    endpoint: h.endpoint,
                    xk: h.xk,
                    to: Some(h.endpoint),
                    candidate: h.candidate,
                    routes: h.routes.clone(),
                    qualified: h.qualified,
                },
            );
        }
        for h in self.linked_holders(None) {
            out.entry(h.endpoint).or_insert(MailLink {
                peer: h.peer,
                endpoint: h.endpoint,
                xk: h.xk,
                to: None,
                candidate: h.candidate,
                routes: h.routes,
                qualified: h.qualified,
            });
        }
        out
    }

    /// The routes of every mailbox this device's private groups use, for the host to keep
    /// linked (they join `bootstrap_hints` and `rtc_candidates`).
    /// Mailboxes a forwarder reaches are not dialed: they would see this device's address.
    pub(crate) fn mailbox_routes(&self) -> Vec<String> {
        let forwarded: BTreeSet<Id> = self.mail_links().into_values().filter(|l| l.to.is_some()).map(|l| l.endpoint).collect();
        let mut out = BTreeSet::new();
        let mut add = |m: &Mailbox| {
            if !forwarded.contains(&m.endpoint) {
                out.extend(m.routes.iter().cloned());
            }
        };
        for c in self.private_communities_with_mailboxes() {
            if let Some(ep) = self.private_epoch(c) {
                ep.mailboxes.iter().for_each(&mut add);
            }
            if let Some(p) = self.scatter.saved.mailboxes.get(&c) {
                for s in p.slots.values() {
                    s.mailboxes.iter().for_each(&mut add);
                }
            }
        }
        out.into_iter().collect()
    }

    /// Routes the host should dial for mailboxes: the mailboxes in use first, and while a group
    /// this device owns still needs some, holders met before and public nodes it knows of, so it
    /// can find enough in places of their own without anyone setting anything up.
    pub(crate) fn mailbox_dial_routes(&self) -> Vec<String> {
        let mut out = self.mailbox_routes();
        out.extend(self.pickup_routes());
        if self.mailboxes_wanted() {
            let now = self.now();
            let linked: BTreeSet<Id> = self.mail_links().into_keys().collect();
            let mut met: Vec<(&Id, &crate::scatter::Seen)> =
                self.scatter.saved.holders.iter().filter(|(e, s)| !linked.contains(e) && now < s.last.saturating_add(MAILBOX_STALE)).collect();
            met.sort_by(|a, b| b.1.present.total_cmp(&a.1.present));
            for (_, s) in met.into_iter().take(2 * MAILBOXES) {
                out.extend(s.routes.iter().cloned());
            }
            out.extend(self.directory_routes(2 * MAILBOXES));
        }
        let mut seen = BTreeSet::new();
        out.retain(|r| seen.insert(r.clone()));
        out
    }

    /// Whether a group this device owns could use mailboxes and has too few: then the host
    /// dials more holders than it otherwise would.
    pub(crate) fn mailboxes_wanted(&self) -> bool {
        self.private_group_ids()
            .into_iter()
            .filter_map(|c| self.private_epoch(c))
            .any(|e| e.active && (e.owner || e.rank.is_some()) && e.mailboxes.len() < MIN_MAILBOXES)
    }

    /// How many links the host should keep open for mailboxes: one per mailbox in use, up to
    /// [`MAILBOXES`], or that many while a group this device owns is still choosing them.
    pub fn mailbox_links(&self) -> usize {
        if self.mailboxes_wanted() {
            return MAILBOXES;
        }
        let forwarded: BTreeSet<Id> = self.mail_links().into_values().filter(|l| l.to.is_some()).map(|l| l.endpoint).collect();
        let mut endpoints = BTreeSet::new();
        for c in self.private_communities_with_mailboxes() {
            if let Some(ep) = self.private_epoch(c) {
                endpoints.extend(ep.mailboxes.iter().map(|m| m.endpoint).filter(|e| !forwarded.contains(e)));
            }
        }
        endpoints.len().min(MAILBOXES)
    }

    fn private_communities_with_mailboxes(&self) -> Vec<Id> {
        self.private_group_ids().into_iter().filter(|c| self.private_epoch(*c).is_some_and(|e| e.active)).collect()
    }

    /// Gives every message of the epoch `ep` describes that has no slot yet the next slot of
    /// this device, in order. Messages of a group whose epoch has no mailboxes keep none (they
    /// wait for a relay, where there is one).
    pub(crate) fn assign_slots(&mut self, c: Id, ep: &MailEpoch, objects: &[CipherObject]) {
        if ep.mailboxes.is_empty() {
            return;
        }
        let progress = self.scatter.saved.mailboxes.entry(c).or_default();
        if progress.write_epoch != ep.epoch {
            progress.write_epoch = ep.epoch;
            progress.next_seq = 0;
        }
        let mut changed = false;
        let fresh: Vec<&CipherObject> = objects.iter().filter(|o| o.room == ep.room && !progress.slots.contains_key(&o.id)).collect();
        for o in fresh {
            let seq = progress.next_seq;
            progress.next_seq += 1;
            let loc = Id(slot(&ep.slot_key, ep.own, seq));
            progress.slots.insert(
                o.id.clone(),
                SlotState {
                    epoch: ep.epoch,
                    leaf: ep.own,
                    seq,
                    loc,
                    mailboxes: ep.mailboxes.clone(),
                    stored: BTreeSet::new(),
                    refused: BTreeSet::new(),
                    bell: Id(ep.slot_key),
                },
            );
            changed = true;
        }
        if changed {
            self.scatter.dirty = true;
        }
    }

    /// Whether every message waiting to go out of group `c` has its place in the order of this
    /// device's slots: then a membership change may follow them at once.
    pub(crate) fn mailbox_ordered(&self, c: Id, objects: &[CipherObject]) -> bool {
        let slots = self.scatter.saved.mailboxes.get(&c).map(|p| &p.slots);
        objects.iter().all(|o| slots.is_some_and(|s| s.contains_key(&o.id)))
    }

    /// One round for every private group with mailboxes: the owner keeps the list fresh, the
    /// outbox is filled, slots are written, members' next slots are checked. Called from
    /// `scatter_tick`.
    pub(crate) fn mailbox_tick(&mut self) {
        let now = self.now();
        for c in self.private_group_ids() {
            let Some(ep) = self.private_epoch(c) else { continue };
            if !ep.active {
                continue;
            }
            // Who keeps the list and carries out the records: in a group that follows its
            // records, the committing devices in their order; elsewhere the owner's.
            let line = if self.follows_records(c) { ep.rank } else { ep.owner.then_some(0) };
            let reading = self.scatter.mail.reading.entry(c).or_insert((ep.epoch, now));
            if reading.0 != ep.epoch {
                *reading = (ep.epoch, now);
            }
            let since = reading.1;
            // read this epoch a while, with mailboxes answering all along
            let heard = self.scatter.mail.heard.get(&c).is_some_and(|h| now < h.1 + AWAY && now >= h.0 + crate::private::CATCH_UP);
            let caught_up = heard && now >= since + crate::private::CATCH_UP;
            // Choosing the first mailboxes needs nothing read; changing them in a group with
            // several committers waits until this device has caught up, like any other change.
            let ready = caught_up || ep.mailboxes.is_empty() || !self.follows_records(c);
            // A group still without mailboxes looks again soon: holders may have linked since.
            let every = if ep.mailboxes.len() < MIN_MAILBOXES { CHOOSE_EVERY } else { MAINTAIN_EVERY };
            if let Some(rank) = line
                && ready
                && self.scatter.mail.maintained_at.get(&c).is_none_or(|t| now >= t + every)
            {
                self.scatter.mail.maintained_at.insert(c, now);
                self.maintain_mailboxes(c, &ep, rank);
            }
            if line.is_some() && !ep.mailboxes.is_empty() && caught_up {
                self.follow_records(c);
            }
            let Some(ep) = self.private_epoch(c) else { continue };
            if ep.mailboxes.is_empty() {
                continue;
            }
            if self.scatter.mail.outbox_at.get(&c).is_none_or(|t| now >= t + OUTBOX_EVERY) {
                self.scatter.mail.outbox_at.insert(c, now);
                let _ = self.private_outbox(&c.hex());
            }
            self.write_slots(c, &ep);
            self.ring_bells(c, &ep);
            self.read_slots(c, &ep);
            self.read_contests(c);
        }
        self.retry_received();
        self.pickup_tick();
    }

    /// Group `c` went back to an earlier epoch's branch, or was joined afresh: what
    /// its slots were written and read up to belongs to the branch let go.
    pub(crate) fn mail_restart(&mut self, c: Id) {
        self.scatter.saved.mailboxes.remove(&c);
        self.scatter.dirty = true;
        let m = &mut self.scatter.mail;
        m.polls.retain(|(g, _), _| *g != c);
        m.writing.retain(|(g, _, _)| *g != c);
        m.rings.retain(|(g, _), _| *g != c);
        m.ahead.retain(|(g, _), _| *g != c);
        m.held.retain(|(g, _), _| *g != c);
        m.contest_polls.retain(|(g, _, _), _| *g != c);
        m.swept_at.remove(&c);
        m.reading.remove(&c);
        m.heard.remove(&c);
        m.outbox_at.remove(&c);
    }

    /// In epochs of group `c` this device followed a later committer's commit out of, the next
    /// slot of each committer ahead, now and then, while the contest lasts.
    fn read_contests(&mut self, c: Id) {
        let now = self.now();
        let contests = self.private_contests(c);
        self.scatter.mail.contest_polls.retain(|(g, e, l), _| *g != c || contests.iter().any(|ct| ct.epoch == *e && ct.ahead.contains_key(l)));
        if contests.is_empty() {
            return;
        }
        let links = self.mail_links();
        for ct in contests {
            let boxes: Vec<&MailLink> = ct.mailboxes.iter().filter_map(|m| links.get(&m.endpoint)).collect();
            if boxes.is_empty() {
                continue;
            }
            for (&leaf, &seq) in &ct.ahead {
                let poll = self.scatter.mail.contest_polls.entry((c, ct.epoch, leaf)).or_insert(Poll {
                    next_at: 0,
                    backoff: POLL_FAST,
                    busy: false,
                    misses: 0,
                    chase: 0,
                });
                if poll.busy || now < poll.next_at {
                    continue;
                }
                poll.busy = true;
                self.scatter.mail.turn = self.scatter.mail.turn.wrapping_add(1);
                let h = boxes[self.scatter.mail.turn % boxes.len()];
                let loc = slot(&ct.slot_key, leaf, seq);
                let Ok(reply) = XKey::generate() else { continue };
                let Ok(env) = envelope::seal_request(&h.endpoint.0, &h.xk, &Request::Has { loc, reply: reply.public() }) else { continue };
                let (peer, holder, to) = (h.peer.clone(), h.endpoint, h.to);
                self.mail_send(&peer, to, PieceOp::Has, Kind::Had, reply, loc, 0, env, MailRequest::ContestProbe { c, epoch: ct.epoch, leaf, seq, holder });
            }
        }
    }

    /// A look at a committer ahead in a contested epoch came back: found, the next at once;
    /// not, later and later.
    fn contest_poll_done(&mut self, c: Id, epoch: u64, leaf: u32, found: bool) {
        let now = self.now();
        if let Some(p) = self.scatter.mail.contest_polls.get_mut(&(c, epoch, leaf)) {
            p.busy = false;
            if found {
                p.backoff = POLL_FAST;
                p.next_at = now;
            } else {
                p.next_at = now + p.backoff;
                p.backoff = (p.backoff * 2).clamp(POLL_FAST, CONTEST_POLL);
            }
        }
    }

    /// A committer picks the group's mailboxes when it has none, and replaces any not seen for
    /// a while: holders this device saw long and steadily first, others when there are too few,
    /// each in a place of its own, with routes others can reach. The same commit carries the
    /// drop key's work: a key for a list without one, the secret to committers' devices
    /// that lack it, or a new key once a device it was given to has left the line.
    fn maintain_mailboxes(&mut self, c: Id, ep: &MailEpoch, rank: usize) {
        let now = self.now();
        let links = self.mail_links();
        let alive = |m: &Mailbox| {
            links.contains_key(&m.endpoint) || self.scatter.saved.holders.get(&m.endpoint).is_some_and(|s| now < s.last.saturating_add(MAILBOX_STALE))
        };
        let mut chosen: Vec<Mailbox> = ep.mailboxes.iter().filter(|m| alive(m)).cloned().collect();
        let fine = !ep.mailboxes.is_empty() && chosen.len() == ep.mailboxes.len() && chosen.len() >= MAILBOXES;
        // Holders seen long and steadily first, any reachable holder when there are too few;
        // each in a place of its own, with routes other members can dial. Holders reached
        // through forwarders count as much as linked ones: they are what keeps this device's
        // address from the mailboxes.
        let reachable: Vec<&MailLink> = links.values().collect();
        for steady in [true, false] {
            if fine || chosen.len() >= MAILBOXES {
                break;
            }
            let pool: Vec<&&MailLink> =
                reachable.iter().filter(|h| !h.routes.is_empty() && (h.qualified || !steady) && !chosen.iter().any(|m| m.endpoint == h.endpoint)).collect();
            let mut candidates: Vec<Candidate> = pool.iter().map(|h| h.candidate).collect();
            let mut used = Vec::new();
            for h in reachable.iter().filter(|h| chosen.iter().any(|m| m.endpoint == h.endpoint)) {
                candidates.push(h.candidate);
                used.push(candidates.len() - 1);
            }
            let take = (MAILBOXES - chosen.len()).min(place::count_places(&candidates, &used));
            if take == 0 {
                continue;
            }
            if let Some(picked) = place::place(&candidates, MAILBOXES, take, &used, &mut crate::scatter::uniform) {
                for i in picked.into_iter().filter(|&i| i < pool.len()) {
                    let h = pool[i];
                    chosen.push(Mailbox { endpoint: h.endpoint, routes: h.routes.iter().filter(|r| r.len() <= 128).take(4).cloned().collect() });
                }
            }
        }
        let list_change = !fine && chosen.len() >= MIN_MAILBOXES && chosen != ep.mailboxes;
        let (work, turns) = if ep.mailboxes.is_empty() { (KeyWork::Keep, 0) } else { self.drop_key_work(c) };
        let key_change = work != KeyWork::Keep;
        if !list_change {
            self.not_due(c, Id::ZERO);
        }
        if !key_change {
            self.not_due(c, DROP_KEY_DUE);
        }
        // a committer later in line makes a change once those ahead stayed silent; one
        // that lacks the key waits until those that hold it had their turn to give it
        let list_ready = list_change && self.waited_turn(c, Id::ZERO, rank);
        let key_ready = key_change && self.waited_turn(c, DROP_KEY_DUE, turns);
        if !list_ready && !key_ready {
            return;
        }
        let list = if list_ready { chosen } else { ep.mailboxes.clone() };
        if self.private_set_mailboxes(c, list, if key_ready { work } else { KeyWork::Keep }).is_ok() {
            if list_ready {
                self.not_due(c, Id::ZERO);
            }
            if key_ready {
                self.not_due(c, DROP_KEY_DUE);
            }
        }
    }

    fn write_slots(&mut self, c: Id, ep: &MailEpoch) {
        let now = self.now();
        let objects: BTreeMap<String, CipherObject> = self.private_pending(c).into_iter().map(|o| (o.id.clone(), o)).collect();
        let links = self.mail_links();
        // A group of one has nobody to read what an epoch without mailboxes said.
        if ep.members == 1 {
            let unslotted: Vec<String> =
                objects.keys().filter(|id| !self.scatter.saved.mailboxes.get(&c).is_some_and(|p| p.slots.contains_key(*id))).cloned().collect();
            if !unslotted.is_empty() {
                let _ = self.private_ack(&c.hex(), &unslotted);
            }
        }
        let Some(progress) = self.scatter.saved.mailboxes.get(&c).cloned() else { return };
        let mut done = Vec::new();
        let mut sends = Vec::new();
        let mut inflight = self.scatter.mail.writing.iter().filter(|(g, _, _)| *g == c).count();
        for (id, s) in &progress.slots {
            let Some(object) = objects.get(id) else {
                // acknowledged some other way (a relay): nothing left to write
                done.push(id.clone());
                continue;
            };
            let need = WRITE_QUORUM.min(s.mailboxes.len()).max(1);
            let settled = s.mailboxes.iter().all(|m| s.stored.contains(&m.endpoint) || s.refused.contains(&m.endpoint));
            if s.stored.len() >= need || (settled && !s.stored.is_empty()) {
                done.push(id.clone());
                // Stored: the bell of its epoch rings on every mailbox, after it.
                let ring =
                    self.scatter.mail.rings.entry((c, s.epoch)).or_insert_with(|| Ring { key: s.bell.0, want: BTreeSet::new(), inflight: BTreeSet::new() });
                ring.want.extend(s.mailboxes.iter().map(|m| m.endpoint));
                continue;
            }
            for m in &s.mailboxes {
                if s.stored.contains(&m.endpoint)
                    || s.refused.contains(&m.endpoint)
                    || self.scatter.mail.writing.contains(&(c, id.clone(), m.endpoint))
                    || inflight >= MAX_INFLIGHT
                {
                    continue;
                }
                if let Some(h) = links.get(&m.endpoint) {
                    sends.push((id.clone(), object.clone(), s.loc, h.endpoint, h.xk, h.peer.clone(), h.to));
                    inflight += 1;
                }
            }
        }
        if !done.is_empty() {
            let acked: Vec<String> = done.iter().filter(|id| objects.contains_key(*id)).cloned().collect();
            if !acked.is_empty() {
                let _ = self.private_ack(&c.hex(), &acked);
            }
            if let Some(p) = self.scatter.saved.mailboxes.get_mut(&c) {
                for id in &done {
                    p.slots.remove(id);
                }
            }
            self.scatter.dirty = true;
        }
        let drop_secret = self.mailbox_drop_secret();
        for (id, object, loc, holder, xk, peer, to) in sends {
            let Ok(body) = hoi_types::uri::unb64(&object.body) else { continue };
            let expiry_day = ((object.expires.max(now + 3600)).min(now + SLOT_TTL)).div_ceil(86_400) as u32;
            let stamp = sc::mine_stamp(&holder.0, &loc.0, body.len(), expiry_day);
            let drop_hash = drop_secret.map(|d| sc::drop_hash(&sc::drop_token(&d, &loc.0))).unwrap_or([0; 32]);
            let Ok(reply) = XKey::generate() else { continue };
            let request = Request::Put { loc: loc.0, reply: reply.public(), expiry_day, drop_hash, drop_key_id: [0; 16], stamp, piece: body };
            let Ok(env) = envelope::seal_request(&holder.0, &xk, &request) else { continue };
            self.scatter.mail.writing.insert((c, id.clone(), holder));
            self.mail_send(&peer, to, PieceOp::Put, Kind::Stored, reply, loc.0, 0, env, MailRequest::Put { c, object: id, holder });
        }
    }

    /// Sends the rings due for group `c`: on each mailbox, a small random object in the next
    /// bell slot this device has not seen taken there.
    fn ring_bells(&mut self, c: Id, ep: &MailEpoch) {
        let now = self.now();
        let links = self.mail_links();
        // Only this epoch's bell and the one before (a commit rings the epoch it ended) matter.
        let floor = ep.epoch.saturating_sub(1);
        self.scatter.mail.rings.retain(|(g, e), _| *g != c || *e >= floor);
        let progress = self.scatter.saved.mailboxes.entry(c).or_default();
        if progress.rings.keys().any(|e| *e < floor) {
            progress.rings.retain(|e, _| *e >= floor);
            self.scatter.dirty = true;
        }
        let keys: Vec<(Id, u64)> = self.scatter.mail.rings.keys().filter(|(g, _)| *g == c).copied().collect();
        let mut sends = Vec::new();
        for key in keys {
            let Some(ring) = self.scatter.mail.rings.get_mut(&key) else { continue };
            let due: Vec<Id> = ring.want.iter().filter(|m| !ring.inflight.contains(m)).copied().collect();
            for m in due {
                let Some(h) = links.get(&m) else { continue };
                ring.want.remove(&m);
                ring.inflight.insert(m);
                sends.push((key.1, ring.key, m, h.xk, h.peer.clone(), h.to));
            }
            if ring.want.is_empty() && ring.inflight.is_empty() {
                self.scatter.mail.rings.remove(&key);
            }
        }
        let today = (now / 86_400) as u32;
        for (epoch, key, holder, xk, peer, to) in sends {
            let seq = self.scatter.saved.mailboxes.get(&c).and_then(|p| p.rings.get(&epoch)).and_then(|r| r.get(&holder)).copied().unwrap_or(0);
            let loc = slot(&key, BELL, seq);
            let Ok(body) = hoi_crypto::random_bytes::<32>() else { continue };
            let stamp = sc::mine_stamp(&holder.0, &loc, body.len(), today + RING_DAYS);
            let Ok(reply) = XKey::generate() else { continue };
            let request = Request::Put {
                loc,
                reply: reply.public(),
                expiry_day: today + RING_DAYS,
                drop_hash: [0; 32],
                drop_key_id: [0; 16],
                stamp,
                piece: body.to_vec(),
            };
            let Ok(env) = envelope::seal_request(&holder.0, &xk, &request) else { continue };
            self.mail_send(&peer, to, PieceOp::Put, Kind::Stored, reply, loc, 0, env, MailRequest::Ring { c, epoch, seq, holder });
        }
    }

    fn mailbox_drop_secret(&self) -> Option<[u8; 32]> {
        let base = self.device_kp.as_ref().or(self.transport_kp.as_ref())?;
        Some(blake3::derive_key("hoi mailbox drop secret v1", &base.secret()))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mail_send(
        &mut self,
        peer: &str,
        to: Option<Id>,
        op: PieceOp,
        kind: Kind,
        reply: XKey,
        loc: [u8; 32],
        want: usize,
        env: Vec<u8>,
        request: MailRequest,
    ) {
        #[cfg(test)]
        {
            let what = match &request {
                MailRequest::Put { .. } => "put",
                MailRequest::Probe { .. } => "probe",
                MailRequest::Fetch { .. } => "fetch",
                MailRequest::Bell { .. } => "bell",
                MailRequest::Ring { .. } => "ring",
                MailRequest::ContestProbe { .. } | MailRequest::ContestFetch { .. } => "contest",
                MailRequest::PickupPut { .. } | MailRequest::PickupProbe { .. } | MailRequest::PickupFetch { .. } => "pickup",
            };
            *self.scatter.mail.sent.entry(what).or_default() += 1;
        }
        let rid = self.next_rid();
        let now = self.now();
        self.scatter.pending.insert((peer.to_string(), rid), crate::scatter::Pending::mail(kind, reply, loc, want, now, request));
        self.send_msg(peer, Msg::Piece { op, rid, to, env });
    }

    fn read_slots(&mut self, c: Id, ep: &MailEpoch) {
        let now = self.now();
        let progress = self.scatter.saved.mailboxes.entry(c).or_default();
        if progress.read_epoch != ep.epoch {
            progress.read_epoch = ep.epoch;
            progress.read.clear();
            progress.bells.clear();
            self.scatter.dirty = true;
            self.scatter.mail.polls.retain(|(g, _), _| *g != c);
            self.scatter.mail.swept_at.remove(&c);
            // slots found ahead belong to the epoch they were found in
            self.scatter.mail.ahead.retain(|(g, _), _| *g != c);
            self.scatter.mail.held.retain(|(g, _), _| *g != c);
        }
        let links = self.mail_links();
        let boxes: Vec<&MailLink> = ep.mailboxes.iter().filter_map(|m| links.get(&m.endpoint)).collect();
        if boxes.is_empty() {
            return;
        }
        // The bell, on one mailbox: every writer rings on all of them, so one is enough. Which
        // one goes by this member's leaf, so readers spread over the mailboxes.
        let h = boxes[ep.own as usize % boxes.len()];
        let bell = self.scatter.saved.mailboxes.get(&c).and_then(|p| p.bells.get(&h.endpoint)).copied().unwrap_or(0);
        // Every member now and then, and at once in a new epoch, whether or not a bell rang; and
        // one past the bell's next slot, so a ring the mailbox lost cannot stall it.
        if self.scatter.mail.swept_at.get(&c).is_none_or(|t| now >= t + FULL_SWEEP) {
            self.scatter.mail.swept_at.insert(c, now);
            self.sweep_members(c, ep, 1);
            self.look_at_bell(c, ep, h, bell + 1, true);
        }
        let poll = self.scatter.mail.polls.entry((c, BELL)).or_insert(Poll { next_at: 0, backoff: POLL_FAST, busy: false, misses: 0, chase: 0 });
        if !poll.busy && now >= poll.next_at {
            poll.busy = true;
            self.look_at_bell(c, ep, h, bell, false);
        }
        // Members a ring or a sweep sent us to, followed while they have more.
        for &leaf in ep.leaves.iter().filter(|&&l| l != ep.own) {
            let Some(poll) = self.scatter.mail.polls.get_mut(&(c, leaf)) else { continue };
            if poll.busy || poll.chase == 0 || now < poll.next_at {
                continue;
            }
            poll.busy = true;
            let base = self.scatter.saved.mailboxes.get(&c).and_then(|p| p.read.get(&leaf)).copied().unwrap_or(0);
            // Now and then look one past the next: a slot every mailbox lost must not stall a member.
            let seq = if poll.misses > 0 && poll.misses % 4 == 3 { base + 1 } else { base };
            self.scatter.mail.turn = self.scatter.mail.turn.wrapping_add(1);
            let h = boxes[self.scatter.mail.turn % boxes.len()];
            let loc = slot(&ep.slot_key, leaf, seq);
            let Ok(reply) = XKey::generate() else { continue };
            let Ok(env) = envelope::seal_request(&h.endpoint.0, &h.xk, &Request::Has { loc, reply: reply.public() }) else { continue };
            let (peer, holder, to) = (h.peer.clone(), h.endpoint, h.to);
            self.mail_send(&peer, to, PieceOp::Has, Kind::Had, reply, loc, 0, env, MailRequest::Probe { c, epoch: ep.epoch, leaf, seq, holder });
        }
    }

    fn look_at_bell(&mut self, c: Id, ep: &MailEpoch, h: &MailLink, seq: u32, extra: bool) {
        let loc = slot(&ep.slot_key, BELL, seq);
        let Ok(reply) = XKey::generate() else { return };
        let Ok(env) = envelope::seal_request(&h.endpoint.0, &h.xk, &Request::Has { loc, reply: reply.public() }) else { return };
        let (peer, holder, to) = (h.peer.clone(), h.endpoint, h.to);
        self.mail_send(&peer, to, PieceOp::Has, Kind::Had, reply, loc, 0, env, MailRequest::Bell { c, epoch: ep.epoch, seq, holder, extra });
    }

    /// Every other member that went quiet is due now, for `chase` probes; those still being
    /// followed keep their pace, so rings in a row cost no more than one.
    fn sweep_members(&mut self, c: Id, ep: &MailEpoch, chase: u8) {
        let now = self.now();
        for &leaf in ep.leaves.iter().filter(|&&l| l != ep.own) {
            let poll = self.scatter.mail.polls.entry((c, leaf)).or_insert(Poll { next_at: 0, backoff: POLL_FAST, busy: false, misses: 0, chase: 0 });
            if poll.chase == 0 {
                poll.chase = chase;
                poll.next_at = poll.next_at.min(now);
                poll.backoff = POLL_FAST;
            }
        }
    }

    /// An answer to a mailbox request, or `None` when it never came.
    pub(crate) fn on_mail_answer(&mut self, request: MailRequest, answer: Option<Reply>) {
        let now = self.now();
        if answer.is_some()
            && let Some(c) = request.group()
        {
            let h = self.scatter.mail.heard.entry(c).or_insert((now, now));
            if now >= h.1 + AWAY {
                h.0 = now;
            }
            h.1 = now;
        }
        match request {
            MailRequest::PickupPut { .. } | MailRequest::PickupProbe { .. } | MailRequest::PickupFetch { .. } => self.on_pickup_answer(request, answer),
            MailRequest::Put { c, object, holder } => {
                self.scatter.mail.writing.remove(&(c, object.clone(), holder));
                let Some(slot_state) = self.scatter.saved.mailboxes.get_mut(&c).and_then(|p| p.slots.get_mut(&object)) else { return };
                match answer {
                    Some(Reply::Stored(Stored::Ok)) => {
                        slot_state.stored.insert(holder);
                    }
                    Some(Reply::Stored(Stored::Taken)) => self.slot_taken(c, &object),
                    Some(Reply::Stored(_)) => {
                        slot_state.refused.insert(holder);
                    }
                    _ => {}
                }
                self.scatter.dirty = true;
            }
            MailRequest::Probe { c, epoch, leaf, seq, holder } => {
                let current = self.private_epoch(c).filter(|e| e.epoch == epoch);
                match (answer, current) {
                    (Some(Reply::Had(Some(len))), Some(ep)) => {
                        let links = self.mail_links();
                        let Some(h) = links.get(&holder) else {
                            self.poll_done(c, leaf, false);
                            return;
                        };
                        let loc = slot(&ep.slot_key, leaf, seq);
                        let Ok(reply) = XKey::generate() else { return };
                        let Ok(env) = envelope::seal_request(&holder.0, &h.xk, &Request::Get { loc, reply: reply.public(), want_len: len }) else { return };
                        let (peer, to) = (h.peer.clone(), h.to);
                        self.mail_send(&peer, to, PieceOp::Get, Kind::Got, reply, loc, len as usize, env, MailRequest::Fetch { c, epoch, leaf, seq });
                    }
                    _ => self.poll_done(c, leaf, false),
                }
            }
            MailRequest::Bell { c, epoch, seq, holder, extra } => {
                let current = self.private_epoch(c).filter(|e| e.epoch == epoch);
                let rang = matches!(answer, Some(Reply::Had(Some(_))));
                if rang && let Some(ep) = current {
                    if let Some(p) = self.scatter.saved.mailboxes.get_mut(&c)
                        && p.read_epoch == epoch
                    {
                        let next = p.bells.entry(holder).or_insert(0);
                        *next = (*next).max(seq + 1);
                        self.scatter.dirty = true;
                    }
                    self.sweep_members(c, &ep, CHASE);
                }
                if !extra {
                    self.poll_done(c, BELL, rang);
                } else if rang {
                    // the regular look is now behind: the next one goes at once
                    if let Some(p) = self.scatter.mail.polls.get_mut(&(c, BELL)) {
                        p.next_at = now;
                        p.backoff = POLL_FAST;
                    }
                }
            }
            MailRequest::Ring { c, epoch, seq, holder } => {
                let Some(ring) = self.scatter.mail.rings.get_mut(&(c, epoch)) else { return };
                ring.inflight.remove(&holder);
                let advance = match answer {
                    Some(Reply::Stored(Stored::Ok)) => true,
                    // someone else rang there first: the next one
                    Some(Reply::Stored(Stored::Taken)) => {
                        ring.want.insert(holder);
                        true
                    }
                    // refused, full or unanswered: members find it at their next sweep
                    _ => false,
                };
                if ring.want.is_empty() && ring.inflight.is_empty() {
                    self.scatter.mail.rings.remove(&(c, epoch));
                }
                if advance {
                    let next = self.scatter.saved.mailboxes.entry(c).or_default().rings.entry(epoch).or_default().entry(holder).or_insert(0);
                    *next = (*next).max(seq + 1);
                    self.scatter.dirty = true;
                }
            }
            MailRequest::ContestProbe { c, epoch, leaf, seq, holder } => match answer {
                Some(Reply::Had(Some(len))) => {
                    let links = self.mail_links();
                    let Some(h) = links.get(&holder) else {
                        self.contest_poll_done(c, epoch, leaf, false);
                        return;
                    };
                    let Some(key) = self.private_contests(c).into_iter().find(|ct| ct.epoch == epoch).map(|ct| ct.slot_key) else { return };
                    let loc = slot(&key, leaf, seq);
                    let Ok(reply) = XKey::generate() else { return };
                    let Ok(env) = envelope::seal_request(&holder.0, &h.xk, &Request::Get { loc, reply: reply.public(), want_len: len }) else { return };
                    let (peer, to) = (h.peer.clone(), h.to);
                    self.mail_send(&peer, to, PieceOp::Get, Kind::Got, reply, loc, len as usize, env, MailRequest::ContestFetch { c, epoch, leaf, seq });
                }
                _ => self.contest_poll_done(c, epoch, leaf, false),
            },
            MailRequest::ContestFetch { c, epoch, leaf, seq } => {
                let Some(Reply::Got(Some(body))) = answer else {
                    self.contest_poll_done(c, epoch, leaf, false);
                    return;
                };
                match self.private_contest_receive(c, epoch, &body) {
                    // went back: the group reads its new epoch from the start
                    Ok(true) => {}
                    // its records may still be on their way: the same slot again later
                    Err(e) if e.code == "private-pending" => self.contest_poll_done(c, epoch, leaf, false),
                    _ => {
                        self.private_contest_read(c, epoch, leaf, seq);
                        self.contest_poll_done(c, epoch, leaf, true);
                    }
                }
            }
            MailRequest::Fetch { c, epoch, leaf, seq } => {
                let still = self.private_epoch(c).is_some_and(|e| e.epoch == epoch);
                match answer {
                    Some(Reply::Got(Some(body))) if still => {
                        let next = self.scatter.saved.mailboxes.get(&c).filter(|p| p.read_epoch == epoch).map(|p| p.read.get(&leaf).copied().unwrap_or(0));
                        if next.is_some_and(|n| seq > n) && self.private_is_commit(c, &body) {
                            // A membership change past the member's next slot ends the epoch:
                            // held until the slots before it are read.
                            self.scatter.mail.held.insert((c, leaf), (seq, body));
                            let a = self.scatter.mail.ahead.entry((c, leaf)).or_insert(seq);
                            *a = (*a).max(seq);
                            self.poll_done(c, leaf, true);
                            return;
                        }
                        self.take_slot(c, epoch, leaf, seq, body);
                        self.poll_done(c, leaf, true);
                    }
                    _ => self.poll_done(c, leaf, false),
                }
            }
        }
    }

    /// A probe of a member's slot, or of the bell, came back: found, look again at once;
    /// not, later and later. A member stops being looked at once its chase runs out, until the
    /// next ring or sweep; the bell never does.
    fn poll_done(&mut self, c: Id, leaf: u32, found: bool) {
        let now = self.now();
        if let Some(p) = self.scatter.mail.polls.get_mut(&(c, leaf)) {
            p.busy = false;
            if found {
                p.backoff = POLL_FAST;
                p.misses = 0;
                p.next_at = now;
                p.chase = CHASE;
            } else {
                p.misses += 1;
                p.next_at = now + p.backoff;
                p.backoff = (p.backoff * 2).clamp(POLL_FAST, POLL_SLOW);
                p.chase = p.chase.saturating_sub(1);
                // a slot every mailbox lost: after a few misses, past it to what was found ahead,
                // and a membership change held there is followed now
                if p.misses >= 4
                    && let Some(a) = self.scatter.mail.ahead.remove(&(c, leaf))
                    && let Some(progress) = self.scatter.saved.mailboxes.get_mut(&c)
                {
                    let epoch = progress.read_epoch;
                    let next = progress.read.entry(leaf).or_insert(0);
                    *next = (*next).max(a);
                    self.scatter.dirty = true;
                    match self.scatter.mail.held.remove(&(c, leaf)) {
                        Some((seq, body)) if seq == a => self.take_slot(c, epoch, leaf, seq, body),
                        _ => {
                            if let Some(progress) = self.scatter.saved.mailboxes.get_mut(&c) {
                                let next = progress.read.entry(leaf).or_insert(0);
                                *next = (*next).max(a + 1);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Slot `seq` of member `leaf` in `epoch`, fetched: received like any message, and read, so
    /// the next one is due at once. A slot found ahead of the next is read, but the next is still
    /// waited for; once the slots before a held membership change are read, it is followed.
    fn take_slot(&mut self, c: Id, epoch: u64, leaf: u32, seq: u32, body: Vec<u8>) {
        let now = self.now();
        let result = self.private_receive(&c.hex(), &hoi_types::uri::b64(&body));
        if let Err(e) = &result
            && e.code == "private-pending"
            && self.scatter.mail.retry.len() < MAX_RETRY
        {
            self.scatter.mail.retry.push((c, body, now + RETRY_FOR));
        }
        let Some(p) = self.scatter.saved.mailboxes.get_mut(&c).filter(|p| p.read_epoch == epoch) else { return };
        let next = p.read.entry(leaf).or_insert(0);
        if seq <= *next {
            *next = (*next).max(seq + 1);
        } else {
            let a = self.scatter.mail.ahead.entry((c, leaf)).or_insert(seq);
            *a = (*a).max(seq);
        }
        let next = *next;
        self.scatter.dirty = true;
        // the slot before a held change is read: the change is next, and its epoch is read out
        if self.scatter.mail.held.get(&(c, leaf)).is_some_and(|(s, _)| *s == next)
            && self.private_epoch(c).is_some_and(|e| e.epoch == epoch)
            && let Some((s, held)) = self.scatter.mail.held.remove(&(c, leaf))
        {
            if self.scatter.mail.ahead.get(&(c, leaf)) == Some(&s) {
                self.scatter.mail.ahead.remove(&(c, leaf));
            }
            self.take_slot(c, epoch, leaf, s, held);
        }
    }

    /// A holder already keeps something else in this message's slot (this device lost its place
    /// after a restart): the message moves to the next slot of its epoch, if it still can.
    fn slot_taken(&mut self, c: Id, object: &str) {
        let Some(ep) = self.private_epoch(c) else { return };
        let Some(progress) = self.scatter.saved.mailboxes.get_mut(&c) else { return };
        let Some(s) = progress.slots.get(object).cloned() else { return };
        if s.epoch != ep.epoch || progress.write_epoch != ep.epoch {
            // an epoch that has passed: its key is gone, the message stays where it is
            return;
        }
        let seq = progress.next_seq;
        progress.next_seq += 1;
        progress.slots.insert(
            object.to_string(),
            SlotState { seq, loc: Id(slot(&ep.slot_key, ep.own, seq)), stored: BTreeSet::new(), refused: BTreeSet::new(), bell: Id(ep.slot_key), ..s },
        );
        self.scatter.dirty = true;
    }

    /// Messages that came before what they depend on (an attachment before its post) get
    /// another try for an hour.
    fn retry_received(&mut self) {
        let now = self.now();
        let waiting = std::mem::take(&mut self.scatter.mail.retry);
        for (c, body, until) in waiting {
            if now > until {
                continue;
            }
            if let Err(e) = self.private_receive(&c.hex(), &hoi_types::uri::b64(&body))
                && e.code == "private-pending"
            {
                self.scatter.mail.retry.push((c, body, until));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_round_trip_and_bad_ones_read_as_none() {
        let list = vec![
            Mailbox { endpoint: Id([1; 32]), routes: vec!["a@192.0.2.4:7843".into(), "rtc:x@192.0.2.4:7844".into()] },
            Mailbox { endpoint: Id([2; 32]), routes: vec![] },
        ];
        assert_eq!(decode_list(&encode_list(&list, None, &[])), list);
        let holders = [Id([5; 32]), Id([6; 32])];
        assert_eq!(decode_list(&encode_list(&list, Some([4; 32]), &holders)), list, "a drop key changes nothing for the list");
        assert_eq!(decode_drop_key(&encode_list(&list, Some([4; 32]), &holders)), Some([4; 32]));
        assert_eq!(decode_holders(&encode_list(&list, Some([4; 32]), &holders)), holders);
        assert_eq!(decode_holders(&encode_list(&list, Some([4; 32]), &[])), vec![]);
        assert_eq!(decode_drop_key(&encode_list(&list, None, &holders)), None);
        assert_eq!(decode_holders(&encode_list(&list, None, &holders)), vec![], "no holders without a key");
        assert!(decode_list(b"nonsense").is_empty());
        let too_many: Vec<Mailbox> = (0..9).map(|i| Mailbox { endpoint: Id([i; 32]), routes: vec![] }).collect();
        assert!(decode_list(&encode_list(&too_many, None, &[])).is_empty());
    }

    #[test]
    fn slots_are_per_member_and_sequence() {
        let k = slot_key(&[7; 32]);
        assert_ne!(slot(&k, 0, 0), slot(&k, 0, 1));
        assert_ne!(slot(&k, 0, 0), slot(&k, 1, 0));
        assert_ne!(slot(&k, 0, 0), slot(&slot_key(&[8; 32]), 0, 0), "a new epoch, new addresses");
    }
}
