//! Invitations a newcomer picks up itself (SCATTER.md S4).
//!
//! Joining a private community whose membership follows its records took two hand-overs: the
//! newcomer gave a member its request, and once a committer added it, the member handed the
//! invitation back. Now only the first is left. A join request carries a one-time X25519 key and
//! up to four holders the newcomer's app reaches (its own choice: it knows no mailbox of the
//! group). The committer that adds the newcomer seals the invitation to that key and stores it,
//! through its own forwarders, on each of those holders it reaches, at numbered slots
//!
//! ```text
//! loc_i = BLAKE3_keyed(derive_key("hoi invitation pickup v1", key), u32(i))
//! ```
//!
//! The newcomer's app looks at slot 0 of each holder, more and more slowly, and joins as soon as
//! one opens. A slot someone else took (a holder never overwrites) moves the writer to the next;
//! a slot that does not open moves the reader on. Anyone who saw the request can compute the
//! locators, but what is stored there opens only with the newcomer's key, and the invitation
//! inside only with its key package: they could fill slots, not read or swap one.
//!
//! The invitation still goes back into the group as before, so a member can hand it on if the
//! holders named are out of the committer's reach.

use std::collections::{BTreeMap, BTreeSet};

use hoi_crypto::envelope::{self, Kind, Reply, Request, Stored, XKey};
use hoi_crypto::scatter as sc;
use hoi_sync::PieceOp;
use hoi_types::Id;
use hoi_types::uri::{b64, unb64};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::Node;
use crate::mailbox::{MailRequest, Mailbox};

/// Holders a request names at most.
pub(crate) const PICKUP_HOLDERS: usize = 4;
/// Slots one holder is tried at before the writer gives it up.
const PICKUP_SLOTS: u32 = 8;
/// How long an invitation waits: as long as the request it answers.
pub(crate) const PICKUP_TTL: u64 = 7 * 86_400;
/// Invitations waiting to be stored, at most.
const MAX_PICKUPS: usize = 32;
/// What an invitation is sealed for.
const INFO: &[u8] = b"hoi invitation v1";
/// The newcomer's looks at a holder: at once, then slower, at most every two minutes.
const POLL_FAST: u64 = 5;
const POLL_SLOW: u64 = 120;
/// An invitation is padded to a multiple of this, so its size says little about the group.
const PAD: usize = 4096;
/// An invitation is at most this long once sealed (a holder keeps pieces of up to 1 MiB).
const MAX_SEALED: usize = 1024 * 1024;

/// Where slot `i` of a pickup key is.
pub(crate) fn pickup_loc(key: &[u8; 32], i: u32) -> [u8; 32] {
    *blake3::keyed_hash(&blake3::derive_key("hoi invitation pickup v1", key), &i.to_be_bytes()).as_bytes()
}

fn aad(key: &[u8; 32]) -> Vec<u8> {
    let mut a = INFO.to_vec();
    a.extend_from_slice(key);
    a
}

/// The invitation, padded and sealed to the newcomer's pickup key.
pub(crate) fn seal_invitation(key: &[u8; 32], invitation: &str) -> Option<Vec<u8>> {
    let mut plain = Vec::with_capacity(invitation.len() + 4 + PAD);
    plain.extend_from_slice(&(invitation.len() as u32).to_be_bytes());
    plain.extend_from_slice(invitation.as_bytes());
    plain.resize(plain.len().div_ceil(PAD) * PAD, 0);
    let sealed = envelope::seal_to(key, INFO, &aad(key), &plain).ok();
    plain.zeroize();
    sealed.filter(|s| s.len() <= MAX_SEALED)
}

/// Opens what [`seal_invitation`] sealed: the invitation, or nothing for anything else.
pub(crate) fn open_invitation(key: &XKey, sealed: &[u8]) -> Option<String> {
    let plain = envelope::open_sealed(key, INFO, &aad(&key.public()), sealed).ok()?;
    let len = u32::from_be_bytes(plain.get(..4)?.try_into().ok()?) as usize;
    if plain.len() % PAD != 0 || len > plain.len() - 4 || plain[4 + len..].iter().any(|b| *b != 0) {
        return None;
    }
    String::from_utf8(plain[4..4 + len].to_vec()).ok()
}

/// What a join request carries for its pickup: the one-time key and the holders the newcomer
/// looks at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pickup {
    pub key: String,
    pub holders: Vec<Mailbox>,
}

impl Pickup {
    /// The key and holders, if well formed.
    pub(crate) fn read(&self) -> Option<([u8; 32], Vec<Mailbox>)> {
        let key: [u8; 32] = unb64(&self.key).ok()?.try_into().ok()?;
        let fine = self.holders.len() <= PICKUP_HOLDERS
            && self.holders.iter().all(|h| h.routes.len() <= 4 && h.routes.iter().all(|r| r.len() <= 128))
            && self.holders.iter().map(|h| h.endpoint).collect::<BTreeSet<_>>().len() == self.holders.len();
        fine.then(|| (key, self.holders.clone()))
    }
}

/// An invitation on its way to a newcomer's pickup holders.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PickupOut {
    pub key: Id,
    pub holders: Vec<Mailbox>,
    /// The sealed invitation (base64).
    pub body: String,
    pub until: u64,
    /// Holders that keep it, and those that refused or had no slot left.
    pub stored: BTreeSet<Id>,
    pub refused: BTreeSet<Id>,
    /// The slot each holder is tried at.
    pub next: BTreeMap<Id, u32>,
}

/// The newcomer's looks at its pickup holders, in memory.
#[derive(Debug, Default)]
pub(crate) struct PickupRuntime {
    /// Puts in flight: (key, holder).
    writing: BTreeSet<(Id, Id)>,
    /// Per holder: when it is looked at next, how slowly, whether a look is out, and its slot.
    polls: BTreeMap<Id, (u64, u64, bool, u32)>,
}

impl Node {
    /// Up to [`PICKUP_HOLDERS`] holders this device reaches, in places of their own, for a join
    /// request to name: those it saw long and steadily first.
    pub(crate) fn pickup_holders(&self) -> Vec<Mailbox> {
        let links = self.mail_links();
        let mut all: Vec<&crate::mailbox::MailLink> = links.values().filter(|h| !h.routes.is_empty()).collect();
        all.sort_by_key(|h| (!h.qualified, h.endpoint));
        let candidates: Vec<hoi_sync::place::Candidate> = all.iter().map(|h| h.candidate).collect();
        let take = PICKUP_HOLDERS.min(hoi_sync::place::count_places(&candidates, &[]));
        let picked = hoi_sync::place::place(&candidates, PICKUP_HOLDERS, take, &[], &mut crate::scatter::uniform).unwrap_or_default();
        picked
            .into_iter()
            .map(|i| Mailbox { endpoint: all[i].endpoint, routes: all[i].routes.iter().filter(|r| r.len() <= 128).take(4).cloned().collect() })
            .collect()
    }

    /// Stores an invitation for a newcomer's pickup: sealed to its key, on the holders its
    /// request named, as this device reaches them.
    pub(crate) fn queue_pickup(&mut self, key: [u8; 32], holders: Vec<Mailbox>, invitation: &str) {
        if holders.is_empty() {
            return;
        }
        let Some(sealed) = seal_invitation(&key, invitation) else { return };
        let now = self.now();
        let list = &mut self.scatter.saved.pickups;
        list.retain(|p| p.key != Id(key) && now < p.until);
        if list.len() >= MAX_PICKUPS {
            list.remove(0);
        }
        list.push(PickupOut {
            key: Id(key),
            holders,
            body: b64(&sealed),
            until: now + PICKUP_TTL,
            stored: BTreeSet::new(),
            refused: BTreeSet::new(),
            next: BTreeMap::new(),
        });
        self.scatter.dirty = true;
    }

    /// The routes of pickup holders this device is to store an invitation on and does not reach:
    /// the host dials them, as it dials mailboxes.
    pub(crate) fn pickup_routes(&self) -> Vec<String> {
        let links = self.mail_links();
        let mut out = Vec::new();
        for p in &self.scatter.saved.pickups {
            for h in &p.holders {
                if !links.contains_key(&h.endpoint) && !p.stored.contains(&h.endpoint) && !p.refused.contains(&h.endpoint) {
                    out.extend(h.routes.iter().cloned());
                }
            }
        }
        if let Some((_, holders)) = self.private_pickup() {
            for h in holders.iter().filter(|h| !links.contains_key(&h.endpoint)) {
                out.extend(h.routes.iter().cloned());
            }
        }
        out
    }

    /// One round: invitations still to be stored go out; a newcomer looks for its own.
    pub(crate) fn pickup_tick(&mut self) {
        self.store_pickups();
        self.look_for_invitation();
    }

    fn store_pickups(&mut self) {
        let now = self.now();
        let before = self.scatter.saved.pickups.len();
        self.scatter.saved.pickups.retain(|p| now < p.until && !p.holders.iter().all(|h| p.stored.contains(&h.endpoint) || p.refused.contains(&h.endpoint)));
        if self.scatter.saved.pickups.len() != before {
            self.scatter.dirty = true;
        }
        if self.scatter.saved.pickups.is_empty() {
            return;
        }
        let links = self.mail_links();
        let mut sends = Vec::new();
        for p in &self.scatter.saved.pickups {
            for h in &p.holders {
                let e = h.endpoint;
                if p.stored.contains(&e) || p.refused.contains(&e) || self.scatter.mail.pickup.writing.contains(&(p.key, e)) {
                    continue;
                }
                if let Some(l) = links.get(&e) {
                    sends.push((p.key, e, l.xk, l.peer.clone(), l.to, p.next.get(&e).copied().unwrap_or(0), p.body.clone(), p.until));
                }
            }
        }
        let drop_secret = self.pickup_drop_secret();
        for (key, holder, xk, peer, to, seq, body, until) in sends {
            let Ok(piece) = unb64(&body) else { continue };
            let loc = pickup_loc(&key.0, seq);
            let expiry_day = until.div_ceil(86_400) as u32;
            let stamp = sc::mine_stamp(&holder.0, &loc, piece.len(), expiry_day);
            let drop_hash = drop_secret.map(|d| sc::drop_hash(&sc::drop_token(&d, &loc))).unwrap_or([0; 32]);
            let Ok(reply) = XKey::generate() else { continue };
            let request = Request::Put { loc, reply: reply.public(), expiry_day, drop_hash, drop_key_id: [0; 16], stamp, piece };
            let Ok(env) = envelope::seal_request(&holder.0, &xk, &request) else { continue };
            self.scatter.mail.pickup.writing.insert((key, holder));
            self.mail_send(&peer, to, PieceOp::Put, Kind::Stored, reply, loc, 0, env, MailRequest::PickupPut { key, holder, seq });
        }
    }

    fn pickup_drop_secret(&self) -> Option<[u8; 32]> {
        let base = self.device_kp.as_ref().or(self.transport_kp.as_ref())?;
        Some(blake3::derive_key("hoi invitation drop secret v1", &base.secret()))
    }

    /// While this device has a join request out, it looks at its pickup holders' slots, each
    /// more and more slowly, and joins as soon as one holds an invitation for it.
    fn look_for_invitation(&mut self) {
        let Some((key, holders)) = self.private_pickup() else {
            self.scatter.mail.pickup.polls.clear();
            return;
        };
        let now = self.now();
        let links = self.mail_links();
        let public = key.public();
        self.scatter.mail.pickup.polls.retain(|h, _| holders.iter().any(|x| x.endpoint == *h));
        for h in &holders {
            let Some(l) = links.get(&h.endpoint) else { continue };
            let poll = self.scatter.mail.pickup.polls.entry(h.endpoint).or_insert((0, POLL_FAST, false, 0));
            if poll.2 || now < poll.0 || poll.3 >= PICKUP_SLOTS {
                continue;
            }
            poll.2 = true;
            let seq = poll.3;
            let loc = pickup_loc(&public, seq);
            let Ok(reply) = XKey::generate() else { continue };
            let Ok(env) = envelope::seal_request(&l.endpoint.0, &l.xk, &Request::Has { loc, reply: reply.public() }) else { continue };
            let (peer, to, holder) = (l.peer.clone(), l.to, l.endpoint);
            self.mail_send(&peer, to, PieceOp::Has, Kind::Had, reply, loc, 0, env, MailRequest::PickupProbe { holder, seq });
        }
    }

    /// A look at a pickup slot came back: nothing there yet, or an invitation to open and join.
    /// `past`: the slot it looked at held something that is not this device's invitation.
    fn pickup_poll_done(&mut self, holder: Id, past: Option<u32>, found: bool) {
        let now = self.now();
        if let Some(p) = self.scatter.mail.pickup.polls.get_mut(&holder) {
            p.2 = false;
            if let Some(seq) = past {
                p.3 = p.3.max(seq + 1);
            }
            if found || past.is_some() {
                p.0 = now;
                p.1 = POLL_FAST;
            } else {
                p.0 = now + p.1;
                p.1 = (p.1 * 2).clamp(POLL_FAST, POLL_SLOW);
            }
        }
    }

    pub(crate) fn on_pickup_answer(&mut self, request: MailRequest, answer: Option<Reply>) {
        match request {
            MailRequest::PickupPut { key, holder, seq } => {
                self.scatter.mail.pickup.writing.remove(&(key, holder));
                let Some(p) = self.scatter.saved.pickups.iter_mut().find(|p| p.key == key) else { return };
                match answer {
                    Some(Reply::Stored(Stored::Ok)) => {
                        p.stored.insert(holder);
                    }
                    // someone took the slot: the next one, a few times
                    Some(Reply::Stored(Stored::Taken)) if seq + 1 < PICKUP_SLOTS => {
                        p.next.insert(holder, seq + 1);
                    }
                    Some(Reply::Stored(_)) => {
                        p.refused.insert(holder);
                    }
                    _ => {}
                }
                self.scatter.dirty = true;
            }
            MailRequest::PickupProbe { holder, seq } => match answer {
                Some(Reply::Had(Some(len))) => {
                    let Some((key, _)) = self.private_pickup() else { return };
                    let links = self.mail_links();
                    let Some(l) = links.get(&holder) else {
                        self.pickup_poll_done(holder, None, false);
                        return;
                    };
                    let loc = pickup_loc(&key.public(), seq);
                    let Ok(reply) = XKey::generate() else { return };
                    let Ok(env) = envelope::seal_request(&holder.0, &l.xk, &Request::Get { loc, reply: reply.public(), want_len: len }) else { return };
                    let (peer, to) = (l.peer.clone(), l.to);
                    self.mail_send(&peer, to, PieceOp::Get, Kind::Got, reply, loc, len as usize, env, MailRequest::PickupFetch { holder, seq });
                }
                _ => self.pickup_poll_done(holder, None, false),
            },
            MailRequest::PickupFetch { holder, seq } => {
                let Some(Reply::Got(Some(body))) = answer else {
                    self.pickup_poll_done(holder, None, false);
                    return;
                };
                let Some((key, _)) = self.private_pickup() else { return };
                // what does not open, or does not let this device join, is not its invitation
                let joined = open_invitation(&key, &body).is_some_and(|invitation| self.private_join(&invitation).is_ok());
                if !joined {
                    self.pickup_poll_done(holder, Some(seq), false);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invitations_open_only_with_their_key_and_pad_to_whole_blocks() {
        let key = XKey::generate().unwrap();
        let other = XKey::generate().unwrap();
        let sealed = seal_invitation(&key.public(), "hoi:private:abc").unwrap();
        assert_eq!(sealed.len(), PAD + envelope::OVERHEAD, "a short invitation fills one block");
        assert_eq!(open_invitation(&key, &sealed).as_deref(), Some("hoi:private:abc"));
        assert_eq!(open_invitation(&other, &sealed), None);
        let mut bad = sealed.clone();
        bad[40] ^= 1;
        assert_eq!(open_invitation(&key, &bad), None);
        assert_eq!(seal_invitation(&key.public(), &"x".repeat(PAD)).unwrap().len(), 2 * PAD + envelope::OVERHEAD);
        assert_ne!(pickup_loc(&key.public(), 0), pickup_loc(&key.public(), 1));
        assert_ne!(pickup_loc(&key.public(), 0), pickup_loc(&other.public(), 0));
    }

    #[test]
    fn a_pickup_names_at_most_four_distinct_holders() {
        let h = |i: u8| Mailbox { endpoint: Id([i; 32]), routes: vec![] };
        let key = b64(&[7; 32]);
        assert!(Pickup { key: key.clone(), holders: (0..4).map(h).collect() }.read().is_some());
        assert!(Pickup { key: key.clone(), holders: (0..5).map(h).collect() }.read().is_none());
        assert!(Pickup { key: key.clone(), holders: vec![h(1), h(1)] }.read().is_none());
        assert!(Pickup { key: b64(&[7; 31]), holders: vec![] }.read().is_none());
    }
}
