//! Clearing local data without stranding deletion work (RFC-0003).
//!
//! Clearing this device ([`Node::wipe`], or recovery while the vault is locked) is always
//! possible. It abandons whatever deletion work the device holds: that work is lost, never
//! reported as done. Clearing *after* deleting posts from the network is a separate, guarded
//! command. [`Node::cleanup_readiness`] says whether any saved deletion still needs this device,
//! and [`Node::clear_after_deletion`] clears only if that is still so and nothing changed since
//! the person looked. Both run under the same `&mut Node` that signs, retries and records
//! deletion work, so a deletion that starts in between refuses the clear before any key goes.
//!
//! Readiness reads local state only. Nothing here is sent, logged or stored.

use std::collections::BTreeSet;

use hoi_types::{Body, Id};

use crate::api::{R, parse_id};
use crate::dto::{CleanupBlock, CleanupReadiness};
use crate::{ApiError, Node};

const INVENTORY_CONTEXT: &[u8] = b"hoi cleanup inventory revision v1";
const DELETIONS_CONTEXT: &[u8] = b"hoi cleanup deletion revision v1";

impl Node {
    /// Whether clearing local data now strands no deletion work, and why not.
    ///
    /// Blocked while a deletion of the person's is not signed yet (a confirmed batch target, or
    /// a request waiting for the person's computer to sign it), while a signed deletion of theirs
    /// still needs this device under RFC-0001 (dissemination, suppression, drops or gaps until
    /// its validity horizon; acknowledgements alone never end it), and while this device holds
    /// the only drop authority for unfinished pieces of someone else's deleted post. The two
    /// revisions cover the confirmed inventory (with what a new preview would list) and every
    /// deletion job; [`Node::clear_after_deletion`] needs them back unchanged.
    pub fn cleanup_readiness(&self) -> CleanupReadiness {
        let now = self.now();
        let me = self.me_root();
        let mut until = 0u64;

        let (snapshot, unlisted) = me.map(|root| self.bulk_unlisted(root)).unwrap_or((Id::ZERO, 0));
        let mut inventory = INVENTORY_CONTEXT.to_vec();
        inventory.extend_from_slice(&snapshot.0);
        let mut unsigned = 0u32;
        let mut asked: BTreeSet<Id> = BTreeSet::new();
        for (id, b) in &self.bulk.saved.batches {
            inventory.extend_from_slice(&id.0);
            for (tid, t) in &b.targets {
                inventory.extend_from_slice(&tid.0);
                inventory.extend(serde_json::to_vec(&t.step).unwrap_or_default());
                inventory.extend_from_slice(&t.job.unwrap_or(Id::ZERO).0);
                // A lapsed target can no longer be signed, whether or not a tick noticed yet.
                if t.step.waiting() && t.expires_at > now {
                    unsigned += 1;
                    until = until.max(t.expires_at);
                    asked.extend(t.signing);
                }
            }
        }

        let mut deletions = DELETIONS_CONTEXT.to_vec();
        // A browser's deletion its computer has not signed yet exists nowhere else.
        for (id, (record, _)) in &self.remote.pending {
            if matches!(record.body, Body::Delete) && self.signing_pending(id, now) {
                deletions.extend_from_slice(&id.0);
                if !asked.contains(id) {
                    unsigned += 1;
                }
                until = until.max(record.expires_at);
            }
        }
        let (mut in_effect, mut holder_drops) = (0u32, 0u32);
        for (id, j) in &self.deletions.saved.jobs {
            deletions.extend_from_slice(&id.0);
            let blocking = if Some(j.author) == me {
                // RFC-0001: the person's own deletions need this device until their horizon.
                j.needs_local_state(now).then(|| j.local_state_until()).inspect(|_| in_effect += 1)
            } else {
                // Someone else's deletion travels without this device; pieces it placed itself
                // (a repair) can only be dropped from here.
                j.drops_until(now).inspect(|_| holder_drops += 1)
            };
            deletions.push(blocking.is_some() as u8);
            until = until.max(blocking.unwrap_or(0));
        }

        let blocked: Vec<CleanupBlock> = [("unsigned", unsigned), ("in-effect", in_effect), ("holder-drops", holder_drops)]
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(reason, count)| CleanupBlock { reason: reason.into(), count })
            .collect();
        CleanupReadiness {
            ready: blocked.is_empty(),
            blocked,
            until: (until > 0).then_some(until),
            unlisted,
            inventory: Id::hash(&inventory).hex(),
            deletions: Id::hash(&deletions).hex(),
        }
    }

    /// Clears local data like [`Node::wipe`], but only while [`Node::cleanup_readiness`] still
    /// says ready with exactly these revisions. Anything that changed in between (a new
    /// deletion, a new post, a batch that moved on) refuses the clear and leaves every key,
    /// record and job as it was, so the person can look again.
    pub fn clear_after_deletion(&mut self, inventory: &str, deletions: &str) -> R<()> {
        let (inventory, deletions) = (parse_id(inventory)?, parse_id(deletions)?);
        // Readiness counts only what storage accepted; a failing store refuses here.
        self.commit()?;
        let now = self.cleanup_readiness();
        if !now.ready {
            return Err(ApiError::new("cleanup-blocked", "Deletion work still needs this device. Nothing was cleared."));
        }
        if now.inventory != inventory.hex() || now.deletions != deletions.hex() {
            return Err(ApiError::new("cleanup-changed", "Your posts or deletions changed since you checked. Nothing was cleared; check again."));
        }
        self.wipe_all()
    }
}
