//! "Delete known posts and comments" (RFC-0002): an explicit, frozen inventory of the person's
//! posts and comments that this device holds, deleted one signed record per target.
//!
//! A preview lists the targets by community and what this device cannot account for: private
//! groups it cannot use, pointer-only entries, generations without drop authority and
//! communities that never finished catching up. Confirming the preview's snapshot ID saves that
//! list; the same ID resumes it and never signs a second deletion for a target. Each target goes
//! through the ordinary author-deletion path ([`Node::author_delete`]) under the existing rate
//! windows, signed with the current time. Propagation then belongs to its RFC-0001 job. A local
//! scan never proves complete publication history, so no preview or status claims it.
//!
//! The inventory links an author to communities and records: it stays in encrypted client
//! storage and is never logged or sent. Holders still see only locators and drop authority.

use std::collections::{BTreeMap, BTreeSet};

use hoi_types::{Body, Id, Kind, Record};
use hoi_validate::Reject;
use serde::{Deserialize, Serialize};

use crate::api::{R, parse_id};
use crate::dto::{BulkDeletionCommunity, BulkDeletionGap, BulkDeletionPreview, BulkDeletionStatus, BulkDeletionTarget};
use crate::{ApiError, Node};

pub(crate) const BLOB: &str = "deletion-batches";
const VERSION: u32 = 1;
/// Saved batches at once; finished ones leave once their targets' lifetime has passed.
const MAX_BATCHES: usize = 64;
/// Targets in one batch, the deletion journal's job capacity. A larger inventory reports the
/// rest as a `too-many` gap for a later preview.
pub(crate) const MAX_TARGETS: usize = crate::deletion::MAX_JOBS;
/// Targets across every saved batch.
const MAX_ALL_TARGETS: usize = 4 * MAX_TARGETS;
/// Gaps kept with one preview or batch; the rest are summed per reason.
const MAX_GAPS: usize = 2 * MAX_TARGETS;
/// Deletions signed per tick: each one mines a stamp.
const SIGN_PER_TICK: usize = 8;
/// Due targets looked at per tick.
const CHECK_PER_TICK: usize = 256;
const RETRY: u64 = 60;
const DAY: u64 = 86_400;
const SNAPSHOT_CONTEXT: &[u8] = b"hoi bulk author deletion snapshot v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Step {
    /// Confirmed and not tried yet.
    Selected,
    /// Waiting for a rate window, journal room, a placement or a remote signature.
    Queued,
    /// Accepted as a signed deletion; its RFC-0001 job carries propagation.
    Signed,
    /// The target or the authority to delete it is gone. Never counted as deleted.
    Failed,
    /// The target's own lifetime ended before it was deleted. Not proof of erasure.
    Expired,
    /// Cancelled before a signed deletion was accepted.
    Cancelled,
}

impl Step {
    fn as_str(self) -> &'static str {
        match self {
            Step::Selected => "selected",
            Step::Queued => "queued",
            Step::Signed => "signed",
            Step::Failed => "failed",
            Step::Expired => "expired",
            Step::Cancelled => "cancelled",
        }
    }
    pub(crate) fn waiting(self) -> bool {
        matches!(self, Step::Selected | Step::Queued)
    }
}

/// Why a target waits or failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Why {
    RateLimited,
    AwaitingSignature,
    DeletionFull,
    PlacementPending,
    RecordAbsent,
    NotAllowed,
    Refused,
}

impl Why {
    fn as_str(self) -> &'static str {
        match self {
            Why::RateLimited => "rate-limited",
            Why::AwaitingSignature => "awaiting-signature",
            Why::DeletionFull => "deletion-full",
            Why::PlacementPending => "placement-pending",
            Why::RecordAbsent => "record-absent",
            Why::NotAllowed => "not-allowed",
            Why::Refused => "refused",
        }
    }
}

/// What a preview cannot account for. Gaps are reported, never counted as deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GapReason {
    /// A private group this device cannot read or write: its posts may be missing here, and
    /// deletions there cannot be delivered.
    PrivateStateUnavailable,
    /// This device never finished catching up in the community (or is restoring a snapshot).
    NotSynchronized,
    /// A post known only by its pointer: the record ordinary deletion needs is not here.
    RecordAbsent,
    /// Piece generations this device holds no drop authority for.
    CapabilityMissing,
    /// The community keeps posts in pieces and this device knows none of this post's.
    GenerationsUnknown,
    /// More targets than one batch holds; preview again after this one.
    TooMany,
}

impl GapReason {
    fn as_str(self) -> &'static str {
        match self {
            GapReason::PrivateStateUnavailable => "private-state-unavailable",
            GapReason::NotSynchronized => "not-synchronized",
            GapReason::RecordAbsent => "record-absent",
            GapReason::CapabilityMissing => "capability-missing",
            GapReason::GenerationsUnknown => "generations-unknown",
            GapReason::TooMany => "too-many",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gap {
    pub reason: GapReason,
    pub community: Option<Id>,
    pub target: Option<Id>,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub community: Id,
    pub comment: bool,
    pub expires_at: u64,
    pub step: Step,
    pub why: Option<Why>,
    /// The accepted signed deletion: the RFC-0001 job's id.
    pub job: Option<Id>,
    /// A deletion this browser asked the person's computer to sign.
    pub signing: Option<Id>,
    pub next_try: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Batch {
    pub author: Id,
    pub confirmed_at: u64,
    pub cancelled_at: Option<u64>,
    pub targets: BTreeMap<Id, Target>,
    /// The gaps disclosed at confirmation, kept with the batch.
    pub gaps: Vec<Gap>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    version: u32,
    pub batches: BTreeMap<Id, Batch>,
}

impl Default for Saved {
    fn default() -> Self {
        Self { version: VERSION, batches: BTreeMap::new() }
    }
}

impl Saved {
    /// Reads saved batches; anything unknown, oversized or inconsistent refuses to open rather
    /// than resume a list the person did not confirm.
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > 64 * 1024 * 1024 {
            return None;
        }
        let saved: Self = serde_json::from_slice(bytes).ok()?;
        if saved.version != VERSION || saved.batches.len() > MAX_BATCHES || saved.batches.values().map(|b| b.targets.len()).sum::<usize>() > MAX_ALL_TARGETS {
            return None;
        }
        for (id, b) in &saved.batches {
            if b.targets.is_empty()
                || b.targets.len() > MAX_TARGETS
                || b.gaps.len() > MAX_GAPS
                || snapshot_id(b.author, b.targets.keys()) != *id
                || b.targets.values().any(|t| (t.step == Step::Signed) != t.job.is_some())
            {
                return None;
            }
        }
        Some(saved)
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub saved: Saved,
    pub dirty: bool,
    /// `published` must be rebuilt at the next successful commit.
    pub stale: bool,
    /// What the last successful commit stored: status never runs ahead of storage.
    pub published: Vec<BulkDeletionStatus>,
    running: bool,
}

/// The snapshot ID binds the author and the exact target list the person confirms.
fn snapshot_id<'a>(author: Id, targets: impl IntoIterator<Item = &'a Id>) -> Id {
    let mut bytes = SNAPSHOT_CONTEXT.to_vec();
    bytes.extend_from_slice(&author.0);
    for id in targets {
        bytes.extend_from_slice(&id.0);
    }
    Id::hash(&bytes)
}

/// Keeps at most `MAX_GAPS`, summing the rest per reason so nothing is dropped silently.
fn bounded(mut gaps: Vec<Gap>) -> Vec<Gap> {
    gaps.sort();
    if gaps.len() <= MAX_GAPS {
        return gaps;
    }
    let rest = gaps.split_off(MAX_GAPS - 8);
    let mut sums: BTreeMap<GapReason, u32> = BTreeMap::new();
    for g in rest {
        let sum = sums.entry(g.reason).or_default();
        *sum = sum.saturating_add(g.count);
    }
    gaps.extend(sums.into_iter().map(|(reason, count)| Gap { reason, community: None, target: None, count }));
    gaps
}

fn gap_dto(g: &Gap) -> BulkDeletionGap {
    BulkDeletionGap { reason: g.reason.as_str().into(), community: g.community.map(|c| c.hex()), target: g.target.map(|t| t.hex()), count: g.count }
}

/// What a scan of this device found.
struct Inventory {
    author: Id,
    targets: BTreeMap<Id, (Id, bool, u64)>,
    gaps: Vec<Gap>,
    capabilities: u32,
}

impl Inventory {
    fn snapshot(&self) -> Id {
        snapshot_id(self.author, self.targets.keys())
    }
}

impl Node {
    /// Lists this person's known posts and comments here, and what this device cannot account
    /// for. Nothing changes until [`Node::confirm_bulk_deletion`] gets the snapshot ID.
    pub fn preview_bulk_deletion(&self) -> R<BulkDeletionPreview> {
        let me = self.need_me()?;
        let inv = self.bulk_inventory(me.root);
        let mut communities: BTreeMap<Id, BulkDeletionCommunity> = BTreeMap::new();
        for (id, (c, comment, _)) in &inv.targets {
            let entry = communities.entry(*c).or_insert_with(|| BulkDeletionCommunity {
                id: c.hex(),
                name: self.communities.get(c).map(|st| st.genesis.name.clone()).unwrap_or_default(),
                posts: Vec::new(),
                comments: Vec::new(),
            });
            if *comment { entry.comments.push(id.hex()) } else { entry.posts.push(id.hex()) }
        }
        Ok(BulkDeletionPreview {
            snapshot: inv.snapshot().hex(),
            communities: communities.into_values().collect(),
            count: inv.targets.len() as u32,
            capabilities: inv.capabilities,
            gaps: inv.gaps.iter().map(gap_dto).collect(),
            inventory_complete: false,
        })
    }

    /// Saves the previewed list and starts deleting it. The same snapshot ID resumes the saved
    /// batch; a list that changed since the preview is refused. Returns the stored status.
    pub fn confirm_bulk_deletion(&mut self, snapshot: &str) -> R<BulkDeletionStatus> {
        let me = self.need_me()?;
        let snapshot = parse_id(snapshot)?;
        if let Some(b) = self.bulk.saved.batches.get(&snapshot) {
            if b.author != me.root {
                return Err(ApiError::not_found("deletion batch"));
            }
            // Reusing the ID resumes; nothing is signed twice (RFC-0002).
            self.commit()?;
            return self.bulk_published(snapshot);
        }
        let inv = self.bulk_inventory(me.root);
        if inv.snapshot() != snapshot {
            return Err(ApiError::invalid("Your known posts and comments changed since the preview. Preview again."));
        }
        if inv.targets.is_empty() {
            return Err(ApiError::invalid("There are no known posts or comments to delete."));
        }
        let saved: usize = self.bulk.saved.batches.values().map(|b| b.targets.len()).sum();
        if self.bulk.saved.batches.len() >= MAX_BATCHES || saved.saturating_add(inv.targets.len()) > MAX_ALL_TARGETS {
            return Err(ApiError::new("deletion-full", "Too many deletion batches are saved. Existing work is retained; try again after it finishes."));
        }
        let now = self.now();
        let targets = inv
            .targets
            .iter()
            .map(|(id, (community, comment, expires_at))| {
                let t = Target {
                    community: *community,
                    comment: *comment,
                    expires_at: *expires_at,
                    step: Step::Selected,
                    why: None,
                    job: None,
                    signing: None,
                    next_try: now,
                };
                (*id, t)
            })
            .collect();
        self.bulk.saved.batches.insert(snapshot, Batch { author: me.root, confirmed_at: now, cancelled_at: None, targets, gaps: inv.gaps });
        self.bulk.dirty = true;
        // Saved before any deletion is signed or success is reported.
        self.commit()?;
        self.bulk_published(snapshot)
    }

    /// Stops targets that have no accepted signed deletion yet. Accepted deletions and their
    /// jobs continue, even if nothing has been sent yet.
    pub fn cancel_bulk_deletion(&mut self, snapshot: &str) -> R<BulkDeletionStatus> {
        let me = self.need_me()?;
        let snapshot = parse_id(snapshot)?;
        let now = self.now();
        let b = self.bulk.saved.batches.get_mut(&snapshot).filter(|b| b.author == me.root).ok_or_else(|| ApiError::not_found("deletion batch"))?;
        if b.cancelled_at.is_none() {
            b.cancelled_at = Some(now);
            for t in b.targets.values_mut().filter(|t| t.step.waiting()) {
                t.step = Step::Cancelled;
            }
            self.bulk.dirty = true;
        }
        self.commit()?;
        self.bulk_published(snapshot)
    }

    /// Saved batches, as last stored.
    pub fn bulk_deletions(&self) -> Vec<BulkDeletionStatus> {
        self.bulk.published.clone()
    }

    /// The snapshot a preview would show now, and how many known posts and comments it lists:
    /// those no deletion covers yet (RFC-0003 cleanup readiness).
    pub(crate) fn bulk_unlisted(&self, author: Id) -> (Id, u32) {
        let inv = self.bulk_inventory(author);
        (inv.snapshot(), inv.targets.len() as u32)
    }

    fn bulk_published(&self, snapshot: Id) -> R<BulkDeletionStatus> {
        let hex = snapshot.hex();
        self.bulk.published.iter().find(|s| s.snapshot == hex).cloned().ok_or_else(|| ApiError::not_found("deletion batch"))
    }

    pub(crate) fn current_bulk_status(&self) -> Vec<BulkDeletionStatus> {
        let me = self.me_root();
        self.bulk
            .saved
            .batches
            .iter()
            .filter(|(_, b)| Some(b.author) == me)
            .map(|(id, b)| {
                let mut s = BulkDeletionStatus {
                    snapshot: id.hex(),
                    confirmed_at: b.confirmed_at,
                    cancelled_at: b.cancelled_at,
                    count: b.targets.len() as u32,
                    selected: 0,
                    queued: 0,
                    signed: 0,
                    failed: 0,
                    expired: 0,
                    cancelled: 0,
                    targets: Vec::with_capacity(b.targets.len()),
                    gaps: b.gaps.iter().map(gap_dto).collect(),
                    inventory_complete: false,
                };
                for (tid, t) in &b.targets {
                    match t.step {
                        Step::Selected => s.selected += 1,
                        Step::Queued => s.queued += 1,
                        Step::Signed => s.signed += 1,
                        Step::Failed => s.failed += 1,
                        Step::Expired => s.expired += 1,
                        Step::Cancelled => s.cancelled += 1,
                    }
                    s.targets.push(BulkDeletionTarget {
                        id: tid.hex(),
                        community: t.community.hex(),
                        kind: if t.comment { "comment" } else { "post" }.into(),
                        state: t.step.as_str().into(),
                        reason: t.why.map(|w| w.as_str().into()),
                        deletion: t.job.map(|j| j.hex()),
                        next_try: t.step.waiting().then_some(t.next_try),
                    });
                }
                s
            })
            .collect()
    }

    /// The person's live posts and comments held here, by their resolved author root: identity,
    /// delegation and rotation validation already happened at ingest. A display name or an old
    /// signing key never selects anything. Edits follow their parent and are not targets.
    fn bulk_inventory(&self, author: Id) -> Inventory {
        let now = self.now();
        let deleting: BTreeSet<Id> = self.deletions.saved.jobs.values().map(|j| j.target).collect();
        let batched: BTreeSet<Id> = self
            .bulk
            .saved
            .batches
            .values()
            .filter(|b| b.author == author)
            .flat_map(|b| b.targets.iter().filter(|(_, t)| t.step.waiting() || t.step == Step::Signed).map(|(id, _)| *id))
            .collect();
        let gone = |id: &Id| self.store.is_deleted(id) || self.state.skeletons.contains_key(id) || deleting.contains(id) || batched.contains(id);
        let mut found: Vec<(u64, Id, Id, bool, u64)> = Vec::new();
        for c in self.store.communities() {
            for s in self.store.by_author(c, &author) {
                let comment = match s.record.kind() {
                    Kind::Post => false,
                    Kind::Comment => true,
                    _ => continue,
                };
                if s.record.community != Some(*c) || s.record.expires_at <= now || gone(&s.id) {
                    continue;
                }
                found.push((s.record.created_at, s.id, *c, comment, s.record.expires_at));
            }
        }
        found.sort();
        found.dedup_by_key(|f| f.1);
        let mut gaps = Vec::new();
        if found.len() > MAX_TARGETS {
            gaps.push(Gap { reason: GapReason::TooMany, community: None, target: None, count: (found.len() - MAX_TARGETS) as u32 });
            found.truncate(MAX_TARGETS);
        }
        let targets: BTreeMap<Id, (Id, bool, u64)> = found.into_iter().map(|(_, id, c, comment, exp)| (id, (c, comment, exp))).collect();

        // Piece generations: drop authority this device holds, and what it lacks.
        let secret = self.drop_secret();
        let mut capabilities = 0u32;
        for (id, (c, _, _)) in &targets {
            let generations = self.deletion_generations(*id);
            let mut missing = 0u32;
            for (_, m, locally_created) in &generations {
                if now / DAY > m.expires_at.div_ceil(DAY) {
                    continue;
                }
                let cap = self.scatter.saved.drop_caps.get(&crate::drop_share::cap_id(m)).is_some_and(|cap| cap.read().is_some_and(|old| old == *m));
                if cap || (*locally_created && secret.is_some()) {
                    capabilities = capabilities.saturating_add(1);
                } else {
                    missing += 1;
                }
            }
            if missing > 0 {
                gaps.push(Gap { reason: GapReason::CapabilityMissing, community: Some(*c), target: Some(*id), count: missing });
            }
            // Kept in pieces as far as this device knows, yet none of its generations is known
            // here (an unfinished placement, or pieces another device holds the manifests of).
            if generations.is_empty() && self.store.get(id).is_some_and(|s| self.kept_in_pieces(s)) {
                gaps.push(Gap { reason: GapReason::GenerationsUnknown, community: Some(*c), target: Some(*id), count: 1 });
            }
        }

        // Pointer-only entries: ordinary deletion needs the record itself.
        let mut absent: BTreeMap<Id, Id> = BTreeMap::new();
        for (id, st) in &self.scatter.standins {
            if st.root == author && !self.store.contains(id) && !gone(id) {
                absent.insert(*id, st.community);
            }
        }
        for k in self.scatter.saved.kept.values() {
            if let (Some((record, _)), Some(who), Some(c)) = (k.claim, k.author, k.community)
                && who == author
                && !self.store.contains(&record)
                && !gone(&record)
            {
                absent.entry(record).or_insert(c);
            }
        }
        gaps.extend(absent.into_iter().map(|(id, c)| Gap { reason: GapReason::RecordAbsent, community: Some(c), target: Some(id), count: 1 }));

        // Sources this device cannot vouch for.
        let mut per: BTreeMap<Id, u32> = BTreeMap::new();
        for (c, _, _) in targets.values() {
            *per.entry(*c).or_default() += 1;
        }
        for c in self.state.joined.keys() {
            let count = per.get(c).copied().unwrap_or(0);
            let private = self.is_private_community(c);
            if private && !self.private_writable(*c) {
                gaps.push(Gap { reason: GapReason::PrivateStateUnavailable, community: Some(*c), target: None, count });
            }
            if self.deletion_restore_quarantined(Some(*c)) || (!private && !self.state.sync_completed.contains_key(c)) {
                gaps.push(Gap { reason: GapReason::NotSynchronized, community: Some(*c), target: None, count });
            }
        }
        Inventory { author, targets, gaps: bounded(gaps), capabilities }
    }

    pub(crate) fn bulk_deletion_tick(&mut self) {
        if self.bulk.running || self.bulk.saved.batches.is_empty() {
            return;
        }
        self.bulk.running = true;
        self.run_bulk();
        self.bulk.running = false;
    }

    fn run_bulk(&mut self) {
        // A confirmation is acted on only once it is stored.
        if self.bulk.dirty && self.commit().is_err() {
            return;
        }
        let now = self.now();
        let Some((me, device)) = self.state.me.as_ref().filter(|m| !m.burned).map(|m| (m.root, m.device)) else { return };
        // Sign only under a live blessing. A revoked device's waiting targets fail; a lapsed
        // blessing waits for its renewal (opening a node sweeps before it renews).
        let blessing = self.store.delegation(&device).map(|d| (d.info.revoked_at, d.info.expires_at));
        let revoked = blessing.is_some_and(|(revoked_at, _)| revoked_at.is_some_and(|at| at <= now));
        let live = blessing.is_some_and(|(revoked_at, expires_at)| revoked_at.is_none() && expires_at > now);
        let jobs: BTreeMap<Id, Id> = self.deletions.saved.jobs.iter().filter(|(_, j)| j.author == me).map(|(id, j)| (j.target, *id)).collect();
        let mut changed = false;
        for b in self.bulk.saved.batches.values_mut().filter(|b| b.author == me) {
            for (id, t) in b.targets.iter_mut() {
                if t.step != Step::Signed
                    && let Some(job) = jobs.get(id)
                {
                    // Signed here, by another device, or by a remote signature after a cancel.
                    *t = Target { step: Step::Signed, why: None, job: Some(*job), signing: None, ..t.clone() };
                    changed = true;
                } else if t.step.waiting() && t.expires_at <= now {
                    *t = Target { step: Step::Expired, why: None, signing: None, ..t.clone() };
                    changed = true;
                } else if t.step.waiting() && revoked {
                    *t = Target { step: Step::Failed, why: Some(Why::NotAllowed), signing: None, ..t.clone() };
                    changed = true;
                }
            }
        }
        let before = self.bulk.saved.batches.len();
        self.bulk.saved.batches.retain(|_, b| {
            b.targets.values().any(|t| t.step.waiting()) || b.targets.values().map(|t| t.expires_at).max().unwrap_or(0).saturating_add(2 * DAY) > now
        });
        changed |= before != self.bulk.saved.batches.len();
        self.bulk.dirty |= changed;
        if !live {
            return;
        }
        let mut due: Vec<(u64, Id, Id)> = self
            .bulk
            .saved
            .batches
            .iter()
            .filter(|(_, b)| b.author == me && b.cancelled_at.is_none())
            .flat_map(|(bid, b)| b.targets.iter().filter(|(_, t)| t.step.waiting() && t.next_try <= now).map(move |(id, t)| (t.next_try, *bid, *id)))
            .collect();
        due.sort();
        let mut signed = 0;
        for (_, batch, target) in due.into_iter().take(CHECK_PER_TICK) {
            // A browser's computer signs a few at a time, leaving room for the person's own posts.
            if signed >= SIGN_PER_TICK || (self.signs_remotely() && self.signing_outstanding(now) >= SIGN_PER_TICK) {
                break;
            }
            signed += self.bulk_attempt(batch, target, me, now) as usize;
            changed = true;
        }
        self.bulk.dirty |= changed;
    }

    /// One target: recheck what it is and who may delete it, then sign through the ordinary
    /// path. Returns whether a stamp was mined.
    fn bulk_attempt(&mut self, batch: Id, target: Id, me: Id, now: u64) -> bool {
        let Some(t) = self.bulk.saved.batches.get(&batch).and_then(|b| b.targets.get(&target)).cloned() else { return false };
        let set = |n: &mut Node, step: Step, why: Option<Why>, signing: Option<Id>, next_try: u64| {
            if let Some(slot) = n.bulk.saved.batches.get_mut(&batch).and_then(|b| b.targets.get_mut(&target)) {
                slot.step = step;
                slot.why = why;
                slot.signing = signing;
                slot.next_try = next_try;
            }
        };
        if t.signing.is_some_and(|record| self.signing_pending(&record, now)) {
            set(self, Step::Queued, Some(Why::AwaitingSignature), t.signing, now + RETRY);
            return false;
        }
        let Some(stored) = self.store.get(&target) else {
            set(self, Step::Failed, Some(Why::RecordAbsent), None, now);
            return false;
        };
        if stored.author_root != me {
            set(self, Step::Failed, Some(Why::NotAllowed), None, now);
            return false;
        }
        if stored.record.expires_at <= now {
            set(self, Step::Expired, None, None, now);
            return false;
        }
        // The current time, never backdated, and one record per target under the same windows
        // and exemptions as any other deletion. Checked before mining a stamp.
        let probe = Record {
            v: 1,
            community: Some(t.community),
            author: Id::ZERO,
            parent: Some(target),
            created_at: now,
            expires_at: stored.record.expires_at.max(now + 60),
            body: Body::Delete,
            nonce: 0,
        };
        if let Err(Reject::RateLimited) = self.check_rate(&probe, me, t.community) {
            let next = self.bulk_rate_retry(t.community, me, now);
            set(self, Step::Queued, Some(Why::RateLimited), None, next);
            return false;
        }
        match self.author_delete(target) {
            Ok(record) if self.signing(&record) => set(self, Step::Queued, Some(Why::AwaitingSignature), Some(record), now + RETRY),
            Ok(record) => {
                let job = self.deletion_for_target(target).unwrap_or(record);
                set(self, Step::Signed, None, None, now);
                if let Some(slot) = self.bulk.saved.batches.get_mut(&batch).and_then(|b| b.targets.get_mut(&target)) {
                    slot.job = Some(job);
                }
            }
            Err(e) => {
                let remote = self.signs_remotely();
                match e.code.as_str() {
                    // a browser's computer that is off or busy: ask again later
                    "rate-limited" | "not-allowed" if remote => set(self, Step::Queued, Some(Why::AwaitingSignature), None, now + RETRY),
                    "rate-limited" => {
                        let next = self.bulk_rate_retry(t.community, me, now);
                        set(self, Step::Queued, Some(Why::RateLimited), None, next)
                    }
                    "deletion-full" => set(self, Step::Queued, Some(Why::DeletionFull), None, now + 10 * RETRY),
                    "deletion-pending" => set(self, Step::Queued, Some(Why::PlacementPending), None, now + RETRY),
                    "not-found" => set(self, Step::Failed, Some(Why::RecordAbsent), None, now),
                    "expired" => set(self, Step::Expired, None, None, now),
                    "not-allowed" | "banned" | "no-identity" => set(self, Step::Failed, Some(Why::NotAllowed), None, now),
                    _ => set(self, Step::Failed, Some(Why::Refused), None, now),
                }
            }
        }
        true
    }

    /// When the author's deletions in `c` next fit every rate window, by their own times.
    fn bulk_rate_retry(&self, c: Id, root: Id, now: u64) -> u64 {
        let times = self.store.author_times(&c, &root, hoi_validate::rate_group(Kind::Delete));
        let mut at = now + 5;
        for (w, max) in hoi_validate::rate_windows(Kind::Delete, false, 0, false) {
            let mut recent: Vec<u64> = times.iter().copied().filter(|x| x.saturating_add(w) > now).collect();
            recent.sort_unstable_by(|a, b| b.cmp(a));
            if let Some(x) = recent.get(max.saturating_sub(1) as usize) {
                at = at.max(x.saturating_add(w));
            }
        }
        at.min(now + DAY)
    }
}
