//! Identity work checked outside the node lock. An identity record's work
//! costs one Argon2id run (8 MiB, PROTOCOL.md §4.1), and native hosts keep the node behind a
//! mutex that every connection and the UI wait for. So they check the work a peer's message
//! carries before taking the message in:
//!
//! 1. [`IdentityWork::scan`] finds the message's identity records and runs the pipeline's cheap
//!    stages on them (stamp floor, then signature), without the node.
//! 2. [`Node::identity_work_wanted`], under a short lock, keeps those the pipeline would run
//!    Argon2id for: new, not verified before, and taken from this peer at all.
//! 3. [`IdentityWork::verify`] runs Argon2id for them with no lock held, stopping at the first
//!    forgery, which drops the peer anyway.
//! 4. [`Node::net_receive_checked`] takes the message in as [`Node::net_receive`] does; the
//!    pipeline looks the answers up instead of running Argon2id.
//!
//! The pipeline and its order stay as they are. Nothing counts as verified unless
//! [`IdentityWork::verify`] ran Argon2id for it, and the answers live only while their message is
//! taken in; work the scan didn't cover is verified inline, as before. The browser's core runs in
//! a worker with no lock to hold and keeps the inline path.

use std::collections::{BTreeMap, BTreeSet};

use hoi_sync::Msg;
use hoi_types::{Body, Id, Kind, Record, Wire};
use hoi_validate::Reject;

use crate::node::Node;

/// Identity records one message carries, in the order the pipeline takes them, and what their
/// work is worth once verified.
#[derive(Debug, Default)]
pub struct IdentityWork {
    found: Vec<Found>,
    /// Devices the message blesses and authors of its community records: whose identities a
    /// stranger may pass on with them (PROTOCOL.md §7).
    devices: BTreeMap<Id, Id>,
    authors: Vec<Id>,
}

#[derive(Debug)]
struct Found {
    id: Id,
    record: Record,
    /// The bits of work, or why it is forged; `None` until verified.
    bits: Option<Result<u32, Reject>>,
}

impl IdentityWork {
    /// The identity records of a `recs` message whose stamp and signature hold. Pure and cheap:
    /// one hash and one signature check per identity record, no Argon2id.
    pub fn scan(bytes: &[u8]) -> IdentityWork {
        let Ok(Msg::Recs { recs }) = Msg::decode(bytes) else { return IdentityWork::default() };
        let mut w = IdentityWork::default();
        let mut found = Vec::new();
        for bytes in &recs {
            // The batch's carriers, decoded as the pipeline's batch intake does.
            let Ok((wire, _)) = Wire::decode(bytes) else { continue };
            match &wire.record.body {
                Body::Delegation { root, .. } => {
                    w.devices.insert(wire.record.author, *root);
                }
                Body::Identity { .. } => {
                    let Ok(d) = hoi_validate::decode(bytes) else { continue };
                    let cheap = hoi_validate::stamp(&d.id, d.wire.record.nonce, hoi_validate::stamp_floor(Kind::Identity)).is_ok()
                        && hoi_validate::signature(&d).is_ok();
                    if cheap {
                        found.push(Found { id: d.id, record: d.wire.record, bits: None });
                    }
                }
                _ if wire.record.community.is_some() => w.authors.push(wire.record.author),
                _ => {}
            }
        }
        // The pipeline takes a batch's identities oldest first, in a stable order.
        found.sort_by_key(|f| f.record.created_at);
        w.found = found;
        w
    }

    /// Whether any identity work is left to verify.
    pub fn is_empty(&self) -> bool {
        self.found.is_empty()
    }

    pub fn len(&self) -> usize {
        self.found.len()
    }

    /// How many Argon2id runs [`IdentityWork::verify`] made.
    #[cfg(test)]
    pub(crate) fn verified(&self) -> usize {
        self.found.iter().filter(|f| f.bits.is_some()).count()
    }

    /// Runs Argon2id for every record left, in the pipeline's order, with no lock held. Stops at
    /// the first forgery: it drops the peer, and the rest of its batch isn't taken in.
    pub fn verify(mut self) -> IdentityWork {
        for f in &mut self.found {
            let bits = hoi_validate::identity(&f.record);
            let forged = bits.is_err();
            f.bits = Some(bits);
            if forged {
                break;
            }
        }
        self
    }
}

/// Identity records verified ahead for one message at most: as many as a message can carry.
const MAX_CHECKED: usize = hoi_sync::MAX_RECS;

impl Node {
    /// Keeps the identity records of `work` the pipeline would run Argon2id for when `peer`
    /// sends them: not here yet, not verified before, from a root that may be an identity, and
    /// taken from this peer at all (PROTOCOL.md §7). Cheap lookups only, for under the lock.
    pub fn identity_work_wanted(&self, peer: &str, mut work: IdentityWork) -> IdentityWork {
        let Some(p) = self.peers.get(peer).filter(|p| p.said_hello) else { return IdentityWork::default() };
        if !p.client && !p.home && p.meta.incoming && !self.accepts_members() {
            return IdentityWork::default();
        }
        let ours = p.client || p.home;
        let roots = self.roots_of(&work.devices, std::mem::take(&mut work.authors));
        let mut seen = BTreeSet::new();
        work.found.retain(|f| {
            let (root, id) = (f.record.author, f.id);
            let Body::Identity { work, .. } = f.record.body else { return false };
            !self.store.contains(&id)
                && !self.store.is_deleted(&id)
                && !self.state.skeletons.contains_key(&id)
                && self.state.identity_work.get(&root).is_none_or(|(w, _)| *w != work)
                && self.store.delegation(&root).is_none()
                && self.store.burned_at(&root).is_none()
                && (ours || Some(root) == self.me_root() || self.root_known(&root) || roots.contains(&root))
                && seen.insert((root, work))
        });
        work.found.truncate(MAX_CHECKED);
        work
    }

    /// [`Node::net_receive`], with the identity work `work` verified before the lock was taken.
    pub fn net_receive_checked(&mut self, peer: &str, bytes: &[u8], work: IdentityWork) -> bool {
        self.checked_work = work
            .found
            .into_iter()
            .filter_map(|f| match f.record.body {
                Body::Identity { work, .. } => Some(((f.record.author, work), f.bits?)),
                _ => None,
            })
            .collect();
        let keep = self.net_receive(peer, bytes);
        self.checked_work.clear();
        keep
    }
}
