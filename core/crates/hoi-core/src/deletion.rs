//! Durable, bounded author deletion work (RFC-0001). Acknowledgements are observations of
//! individual holders, never proof of complete inventory or network-wide erasure.

use std::collections::{BTreeMap, BTreeSet};

use hoi_crypto::scatter as sc;
use hoi_types::scatter::Manifest;
use hoi_types::uri::{b64, unb64};
use hoi_types::{Body, Id, Record, Wire};
use serde::{Deserialize, Serialize};

use crate::api::R;
use crate::dto::DeletionProgress;
use crate::keep::Kept;
use crate::scatter::{DropOutcome, ScatterOutcome};
use crate::{ApiError, Node, Source};

pub(crate) const BLOB: &str = "deletion-journal";
const VERSION: u32 = 1;
pub(crate) const MAX_JOBS: usize = 4096;
const MAX_TARGETS: usize = 65_536;
const MAX_PENDING: usize = 256;
const PER_TICK: usize = 4;
const BATCH: usize = 32;
const DAY: u64 = 86_400;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Outcome {
    Queued,
    Submitted,
    Acknowledged,
    LegacyRemoval,
    Unknown,
    Refused,
    Unreachable,
    Expired,
}

// Capabilities are key-bearing state: no Debug implementation or logging.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub holder: Id,
    pub locator: Id,
    pub expiry_day: u32,
    pub token: Option<[u8; 32]>,
    pub outcome: Outcome,
    pub attempts: u32,
    pub last_attempt: Option<u64>,
    pub next_retry: u64,
}

impl Target {
    pub(crate) fn observe(&mut self, outcome: Outcome, now: u64) {
        if matches!(self.outcome, Outcome::Acknowledged | Outcome::Expired) {
            return;
        }
        // A delayed reply cannot turn an expired attempt into an acknowledged removal.
        self.outcome = if now / DAY > self.expiry_day as u64 { Outcome::Expired } else { outcome };
        if matches!(self.outcome, Outcome::Acknowledged | Outcome::Expired) {
            self.token = None;
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Job {
    pub target: Id,
    pub author: Id,
    pub community: Option<Id>,
    /// The already validated signed deletion, for durable dissemination and recovery.
    pub record: String,
    pub expires_at: u64,
    /// Object IDs prevent late placements from reintroducing removed content.
    pub objects: BTreeSet<Id>,
    pub targets: BTreeMap<Id, Target>,
    /// Capacity for already dispatched placements whose holder results have not arrived.
    pub reserved_targets: usize,
    /// Unknown generations cannot be silently counted as removed.
    pub coverage_gaps: BTreeSet<Id>,
    pub record_submitted: bool,
    pub next_publication: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pending {
    wire: String,
    private: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Journal {
    version: u32,
    pub jobs: BTreeMap<Id, Job>,
    pending: BTreeMap<Id, Pending>,
    /// Communities restored from a possibly stale snapshot stay quarantined during catch-up.
    #[serde(default)]
    pub restored: BTreeMap<Id, u64>,
}

impl Default for Journal {
    fn default() -> Self {
        Self { version: VERSION, jobs: BTreeMap::new(), pending: BTreeMap::new(), restored: BTreeMap::new() }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub saved: Journal,
    pub dirty: bool,
    pub published: Vec<DeletionProgress>,
    inflight: BTreeMap<u64, (Id, Vec<Id>)>,
    running: bool,
}

impl Journal {
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > 64 * 1024 * 1024 {
            return None;
        }
        let saved: Self = serde_json::from_slice(bytes).ok()?;
        if saved.version != VERSION
            || saved.jobs.len() > MAX_JOBS
            || saved.pending.len() > MAX_PENDING
            || saved.jobs.values().any(|j| j.targets.len() > MAX_TARGETS || j.reserved_targets > MAX_TARGETS)
            || saved.jobs.values().try_fold(0usize, |total, j| total.checked_add(j.targets.len())?.checked_add(j.reserved_targets))? > MAX_TARGETS
        {
            return None;
        }
        for (id, job) in &saved.jobs {
            if job.record.len() > 4096 {
                return None;
            }
            let (wire, found) = Wire::decode(&unb64(&job.record).ok()?).ok()?;
            if found != *id
                || !matches!(wire.record.body, Body::Delete)
                || wire.record.parent != Some(job.target)
                || wire.record.community != job.community
                || wire.record.expires_at != job.expires_at
                || job.objects.len() > MAX_TARGETS
                || job.coverage_gaps.len() > MAX_TARGETS
                || job.targets.iter().any(|(key, t)| *key != target_id(t.holder, t.locator))
            {
                return None;
            }
        }
        for (id, pending) in &saved.pending {
            if pending.wire.len() > 4096 {
                return None;
            }
            let (wire, found) = Wire::decode(&unb64(&pending.wire).ok()?).ok()?;
            if *id != found || !matches!(wire.record.body, Body::Delete) {
                return None;
            }
        }
        Some(saved)
    }
}

fn target_id(holder: Id, locator: Id) -> Id {
    let mut bytes = holder.0.to_vec();
    bytes.extend_from_slice(&locator.0);
    Id::hash(&bytes)
}

impl Node {
    fn deletion_generations(&self, target: Id) -> Vec<(Kept, Manifest, bool)> {
        let mut out = Vec::new();
        for kept in self.scatter.saved.kept.values().filter(|k| k.records.contains(&target) || k.claim.is_some_and(|c| c.0 == target)) {
            let objects: BTreeSet<Id> = kept.gens.iter().chain(&kept.cleanup_gens).filter_map(|g| g.read()).map(|m| Id(m.object)).collect();
            for generation in kept.gens.iter().chain(&kept.cleanup_gens) {
                if let Some(manifest) = generation.read() {
                    out.push((kept.clone(), manifest, generation.from.is_none()));
                }
            }
            // Capabilities shared by our devices can outlive the repairer's newest/previous
            // generation window. They are still known deletion targets (RFC-0001).
            for cap in self.scatter.saved.drop_caps.values() {
                if let Some(manifest) = cap.read().filter(|m| objects.contains(&Id(m.object)))
                    && !out.iter().any(|(_, m, _)| *m == manifest)
                {
                    out.push((kept.clone(), manifest, false));
                }
            }
        }
        out
    }

    pub(crate) fn deletion_for_target(&self, target: Id) -> Option<Id> {
        self.deletions.saved.jobs.iter().find(|(_, j)| j.target == target).map(|(id, _)| *id)
    }

    pub(crate) fn deletion_admission(&self, record: &Record) -> R<()> {
        if record.parent.is_some_and(|p| self.deletion_for_target(p).is_some()) {
            return Ok(());
        }
        if self.deletions.saved.jobs.len() >= MAX_JOBS {
            return Err(ApiError::new("deletion-full", "Deletion work is full. Existing work is retained; try again after it expires."));
        }
        let count: usize = self.deletions.saved.jobs.values().map(|j| j.targets.len() + j.reserved_targets).sum();
        let additional: usize = record.parent.map(|p| self.deletion_generations(p).iter().map(|(_, m, _)| m.holders.len()).sum()).unwrap_or(0);
        let placements = record.parent.map(|p| self.deletion_placement_reservation(p)).unwrap_or(0);
        if count.saturating_add(additional).saturating_add(placements) > MAX_TARGETS {
            return Err(ApiError::new("deletion-full", "Deletion targets are full. Existing work is retained; try again later."));
        }
        Ok(())
    }

    /// Called after validation and before removing the target or any manifests.
    pub(crate) fn capture_author_deletion(&mut self, id: Id, record: &Record, root: Id) {
        let Some(target) = record.parent else { return };
        if self.deletion_for_target(target).is_some() {
            return;
        }
        let Some(stored) = self.store.get(&id) else { return };
        let generations = self.deletion_generations(target);
        let reserved_targets = generations.iter().map(|(_, m, _)| m.holders.len()).sum::<usize>().saturating_add(self.deletion_placement_reservation(target));
        self.deletions.saved.jobs.insert(
            id,
            Job {
                target,
                author: root,
                community: record.community,
                record: b64(&stored.wire),
                expires_at: record.expires_at,
                objects: BTreeSet::new(),
                targets: BTreeMap::new(),
                reserved_targets,
                coverage_gaps: BTreeSet::new(),
                record_submitted: false,
                next_publication: self.now(),
            },
        );
        self.deletions.dirty = true;
        for (k, m, locally_created) in generations {
            // Admission reserved enough room. A capture error leaves Kept for the
            // suppression scheduler to retry before discarding authority.
            let _ = self.queue_deletion_generation(&k, &m, locally_created);
        }
    }

    pub(crate) fn queue_deletion_generation(&mut self, kept: &Kept, m: &Manifest, locally_created: bool) -> R<bool> {
        let jobs: Vec<Id> = self
            .deletions
            .saved
            .jobs
            .iter()
            .filter(|(_, j)| {
                j.community == kept.community
                    && (kept.records.contains(&j.target) || kept.claim.is_some_and(|c| c.0 == j.target) || j.objects.contains(&Id(m.object)))
            })
            .map(|(id, _)| *id)
            .collect();
        if jobs.is_empty() {
            return Ok(false);
        }
        // A shared object may still carry another live post. Its bytes stay until all of
        // those references are deleted; one author's action must not erase another's data.
        if kept.records.iter().any(|record| {
            !self.store.is_deleted(record)
                && !self.state.skeletons.contains_key(record)
                && !jobs.iter().any(|id| self.deletions.saved.jobs[id].target == *record)
        }) {
            // Keep the manifest and its authority; returning false would fall through to
            // legacy cleanup. An absent record may still be arriving in this same batch.
            return Err(ApiError::new("deletion-pending", "This object still carries other records."));
        }
        let cap = self.scatter.saved.drop_caps.get(&crate::drop_share::cap_id(m)).filter(|cap| cap.read().is_some_and(|old| old == *m)).cloned();
        let secret = locally_created.then(|| self.drop_secret()).flatten();
        let entries: Vec<_> = m
            .holders
            .iter()
            .enumerate()
            .filter(|(_, holder)| **holder != Id::ZERO)
            .map(|(i, holder)| {
                let locator = Id(sc::locator(&m.loc_secret, &m.object, m.generation, i as u8));
                let token = cap.as_ref().and_then(|c| c.tokens.get(&locator)).copied().or_else(|| secret.as_ref().map(|s| sc::drop_token(s, &locator.0)));
                (*holder, locator, token, m.expires_at.div_ceil(DAY) as u32)
            })
            .collect();
        self.capture_deletion_targets(&jobs, Id(m.object), &entries)?;
        // The job now retains only locator-level retry authority. Do not keep or share the
        // old object decryption key merely to repeat a drop.
        if cap.is_some() && self.scatter.saved.drop_caps.remove(&crate::drop_share::cap_id(m)).is_some() {
            self.scatter.dirty = true;
        }
        Ok(true)
    }

    fn capture_deletion_targets(&mut self, jobs: &[Id], object: Id, entries: &[(Id, Id, Option<[u8; 32]>, u32)]) -> R<()> {
        let current: usize = self.deletions.saved.jobs.values().map(|j| j.targets.len() + j.reserved_targets).sum();
        let additional: usize = jobs
            .iter()
            .filter_map(|id| self.deletions.saved.jobs.get(id))
            .map(|j| entries.iter().filter(|(h, l, _, _)| !j.targets.contains_key(&target_id(*h, *l))).count().saturating_sub(j.reserved_targets))
            .sum();
        if current.saturating_add(additional) > MAX_TARGETS {
            return Err(ApiError::new("deletion-full", "Deletion targets are full. Pending placement authority is retained."));
        }
        let now = self.now();
        for id in jobs {
            let job = self.deletions.saved.jobs.get_mut(id).expect("selected deletion job");
            job.objects.insert(object);
            for (holder, locator, token, expiry_day) in entries {
                if !job.targets.contains_key(&target_id(*holder, *locator)) {
                    job.reserved_targets = job.reserved_targets.saturating_sub(1);
                }
                let target = job.targets.entry(target_id(*holder, *locator)).or_insert(Target {
                    holder: *holder,
                    locator: *locator,
                    expiry_day: *expiry_day,
                    token: *token,
                    outcome: Outcome::Queued,
                    attempts: 0,
                    last_attempt: None,
                    next_retry: now,
                });
                if target.token.is_none() && token.is_some() && !matches!(target.outcome, Outcome::Acknowledged | Outcome::Expired) {
                    target.token = *token;
                    target.next_retry = now;
                }
            }
        }
        self.deletions.dirty = true;
        Ok(())
    }

    pub(crate) fn deletion_object_suppressed(&self, object: Id) -> bool {
        let now = self.now();
        self.deletions
            .saved
            .jobs
            .values()
            .any(|j| j.objects.contains(&object) && (j.expires_at > now || j.targets.values().any(|t| now / DAY <= t.expiry_day as u64)))
    }

    pub(crate) fn deletion_placement_gap(&mut self, kept: &Kept, object: Option<Id>) {
        for job in self
            .deletions
            .saved
            .jobs
            .values_mut()
            .filter(|j| j.community == kept.community && (kept.records.contains(&j.target) || kept.claim.is_some_and(|c| c.0 == j.target)))
        {
            if let Some(object) = object {
                job.objects.insert(object);
            }
            job.coverage_gaps.insert(object.unwrap_or(job.target));
            self.deletions.dirty = true;
        }
    }

    pub(crate) fn queue_late_deletion_generation(&mut self, community: Option<Id>, m: &Manifest, from: Option<Id>) -> R<bool> {
        let object = Id(m.object);
        let jobs: Vec<Id> =
            self.deletions.saved.jobs.iter().filter(|(_, j)| j.community == community && j.objects.contains(&object)).map(|(id, _)| *id).collect();
        if jobs.is_empty() {
            return Ok(false);
        }
        let validated_cap = self.scatter.saved.drop_caps.get(&crate::drop_share::cap_id(m)).is_some_and(|c| c.read().is_some_and(|old| old == *m));
        if from.is_none() || validated_cap {
            let kept = Kept { community, ..Default::default() };
            return self.queue_deletion_generation(&kept, m, from.is_none());
        }
        // Once the content key/manifest is discarded an unknown update cannot be validated
        // against it. Suppress it, retaining a bounded, explicit inventory gap instead.
        for id in jobs {
            self.deletions.saved.jobs.get_mut(&id).expect("selected job").coverage_gaps.insert(object);
        }
        self.deletions.dirty = true;
        Ok(true)
    }

    pub(crate) fn defer_author_deletion(&mut self, id: Id, wire: &[u8], private: bool) -> R<()> {
        if !self.deletions.saved.pending.contains_key(&id) && self.deletions.saved.pending.len() >= MAX_PENDING {
            return Err(ApiError::new("deletion-full", "Too many deletions are waiting for their targets."));
        }
        self.deletions.saved.pending.insert(id, Pending { wire: b64(wire), private });
        self.deletions.dirty = true;
        Ok(())
    }

    pub(crate) fn apply_pending_deletions(&mut self, target: Id) {
        let pending: Vec<_> = self
            .deletions
            .saved
            .pending
            .iter()
            .filter_map(|(id, p)| {
                let bytes = unb64(&p.wire).ok()?;
                let (wire, _) = Wire::decode(&bytes).ok()?;
                (wire.record.parent == Some(target)).then_some((*id, bytes, p.private))
            })
            .collect();
        for (id, bytes, private) in pending {
            let source = if private { Source::PrivateRelay } else { Source::Import };
            // Full normal validation checks ownership against the target. An unrelated signed
            // pending claim cannot suppress that target, or block a later genuine claim.
            let Ok((wire, _)) = Wire::decode(&bytes) else { continue };
            if self.deletion_admission(&wire.record).is_err() {
                continue;
            }
            self.deletions.saved.pending.remove(&id);
            self.deletions.dirty = true;
            let _ = self.ingest(&bytes, &source);
        }
    }

    pub fn deletion_progress(&self) -> Vec<DeletionProgress> {
        self.deletions.published.clone()
    }

    pub(crate) fn current_deletion_progress(&self) -> Vec<DeletionProgress> {
        let now = self.now();
        self.deletions
            .saved
            .jobs
            .iter()
            .filter(|(_, j)| Some(j.author) == self.me_root())
            .map(|(id, j)| {
                let mut p = DeletionProgress {
                    id: id.hex(),
                    target: j.target.hex(),
                    local_removed: self.state.skeletons.contains_key(&j.target) || self.store.is_deleted(&j.target),
                    record_submitted: j.record_submitted,
                    queued: 0,
                    submitted: 0,
                    acknowledged: 0,
                    legacy_removal: 0,
                    unknown: 0,
                    refused: 0,
                    unreachable: 0,
                    expired: 0,
                    outstanding: 0,
                    coverage_gaps: j.coverage_gaps.len() as u32,
                    expires_at: j.expires_at,
                    requires_local_state: j.expires_at > now,
                    inventory_complete: false,
                };
                for t in j.targets.values() {
                    if t.token.is_none() && !matches!(t.outcome, Outcome::Acknowledged | Outcome::Expired) && now / DAY <= t.expiry_day as u64 {
                        p.coverage_gaps += 1;
                    }
                    let outcome = if now / DAY > t.expiry_day as u64 && t.outcome != Outcome::Acknowledged { Outcome::Expired } else { t.outcome };
                    match outcome {
                        Outcome::Queued => p.queued += 1,
                        Outcome::Submitted => p.submitted += 1,
                        Outcome::Acknowledged => p.acknowledged += 1,
                        Outcome::LegacyRemoval => p.legacy_removal += 1,
                        Outcome::Unknown => p.unknown += 1,
                        Outcome::Refused => p.refused += 1,
                        Outcome::Unreachable => p.unreachable += 1,
                        Outcome::Expired => p.expired += 1,
                    }
                    if !matches!(outcome, Outcome::Acknowledged | Outcome::Expired) {
                        p.outstanding += 1;
                    }
                    if now / DAY <= t.expiry_day as u64 {
                        p.requires_local_state = true;
                    }
                }
                p
            })
            .collect()
    }

    pub(crate) fn deletion_tick(&mut self) {
        if self.deletions.running {
            return;
        }
        self.deletions.running = true;
        self.run_deletions();
        self.deletions.running = false;
    }

    fn run_deletions(&mut self) {
        let now = self.now();
        let waiting: Vec<Id> = self
            .deletions
            .saved
            .pending
            .values()
            .filter_map(|p| {
                let (wire, _) = Wire::decode(&unb64(&p.wire).ok()?).ok()?;
                wire.record.parent.filter(|id| self.store.contains(id) || self.state.skeletons.contains_key(id))
            })
            .collect();
        for target in waiting {
            self.apply_pending_deletions(target);
        }
        let completed: Vec<Id> = self
            .deletions
            .saved
            .restored
            .iter()
            .filter(|(c, started)| !self.is_private_community(c) && self.state.sync_completed.get(c).is_some_and(|at| at >= *started))
            .map(|(c, _)| *c)
            .collect();
        for c in completed {
            self.deletions.saved.restored.remove(&c);
            self.deletions.dirty = true;
        }
        let finished: Vec<_> = self.deletions.inflight.keys().copied().filter(|op| self.op_done(*op)).collect();
        for op in finished {
            let Some((id, keys)) = self.deletions.inflight.remove(&op) else { continue };
            let outcome = self.take_outcome(op);
            if let Some(job) = self.deletions.saved.jobs.get_mut(&id) {
                for (i, key) in keys.iter().enumerate() {
                    let Some(target) = job.targets.get_mut(key) else { continue };
                    let observed = match &outcome {
                        ScatterOutcome::Dropped(results) => match results.iter().find(|r| r.index as usize == i).map(|r| r.outcome) {
                            Some(DropOutcome::Acknowledged) => Outcome::Acknowledged,
                            Some(DropOutcome::Queued) => Outcome::Queued,
                            Some(DropOutcome::LegacyRemoval) => Outcome::LegacyRemoval,
                            Some(DropOutcome::Unknown) => Outcome::Unknown,
                            Some(DropOutcome::Refused) => Outcome::Refused,
                            _ => Outcome::Unreachable,
                        },
                        _ => Outcome::Unreachable,
                    };
                    target.observe(observed, now);
                }
                self.deletions.dirty = true;
            }
        }
        let before = self.deletions.saved.pending.len();
        self.deletions.saved.pending.retain(|_, p| unb64(&p.wire).ok().and_then(|b| Wire::decode(&b).ok()).is_some_and(|(w, _)| w.record.expires_at > now));
        self.deletions.dirty |= before != self.deletions.saved.pending.len();
        for job in self.deletions.saved.jobs.values_mut() {
            if now > job.expires_at.div_ceil(DAY).saturating_add(1).saturating_mul(DAY) && job.reserved_targets != 0 {
                job.reserved_targets = 0;
                self.deletions.dirty = true;
            }
            for t in job.targets.values_mut() {
                if now / DAY > t.expiry_day as u64 && !matches!(t.outcome, Outcome::Expired | Outcome::Acknowledged) {
                    t.outcome = Outcome::Expired;
                    t.token = None;
                    self.deletions.dirty = true;
                }
            }
        }
        // Capture, suppression and authority must reach storage before any drop is released.
        if self.deletions.dirty && self.commit().is_err() {
            return;
        }
        let busy: BTreeSet<Id> = self.deletions.inflight.values().map(|(id, _)| *id).collect();
        let mut selected: Vec<_> = self
            .deletions
            .saved
            .jobs
            .iter()
            .filter(|(id, _)| !busy.contains(id))
            .filter_map(|(id, j)| {
                let mut eligible: Vec<_> = j
                    .targets
                    .iter()
                    .filter(|(_, t)| {
                        t.token.is_some()
                            && t.next_retry <= now
                            && !matches!(t.outcome, Outcome::Acknowledged | Outcome::Expired)
                            && now / DAY <= t.expiry_day as u64
                    })
                    .collect();
                eligible.sort_by_key(|(key, t)| (t.next_retry, t.attempts, t.last_attempt, **key));
                let next = eligible.first().map(|(_, t)| t.next_retry);
                let targets: Vec<_> =
                    eligible.into_iter().take(BATCH).map(|(key, t)| (*key, (t.holder, t.locator, t.token.expect("selected capability")))).collect();
                next.map(|next| (next, *id, targets))
            })
            .collect();
        // Advancing a queued retry must also advance its turn, across both jobs and targets.
        // Otherwise an unavailable prefix can starve reachable work at every five-second tick.
        selected.sort_by_key(|(next, id, _)| (*next, *id));
        for (_, id, targets) in selected.into_iter().take(PER_TICK) {
            let request: Vec<_> = targets.iter().map(|(_, t)| *t).collect();
            let result = self.scatter_drop_targets(&request);
            let submitted = result.as_ref().map(|op| self.scatter_submitted_indexes(*op)).unwrap_or_default();
            let job = self.deletions.saved.jobs.get_mut(&id).expect("selected job");
            for (index, (key, _)) in targets.iter().enumerate() {
                let target = job.targets.get_mut(key).expect("selected target");
                if !submitted.contains(&(index as u8)) {
                    target.outcome = Outcome::Queued;
                    target.next_retry = now + 5;
                    continue;
                }
                target.attempts = target.attempts.saturating_add(1);
                target.last_attempt = Some(now);
                target.next_retry = now.saturating_add(30u64.saturating_mul(1u64 << target.attempts.min(10)).min(6 * 3600));
                target.outcome = if result.is_ok() { Outcome::Submitted } else { Outcome::Unreachable };
            }
            self.deletions.dirty = true;
            if let Ok(op) = result {
                self.deletions.inflight.insert(op, (id, targets.iter().map(|(key, _)| *key).collect()));
            }
        }
        // Existing public sync and MLS dissemination own the signed record. Re-offer public
        // records periodically; private records retain their existing epoch/order handling.
        let mut publish: Vec<_> = self
            .deletions
            .saved
            .jobs
            .iter()
            .filter(|(_, j)| j.expires_at > now && j.next_publication <= now)
            .filter(|(_, j)| self.peers.values().any(|p| p.said_hello && j.community.is_none_or(|c| p.communities.contains(&c) || p.home || p.client)))
            .map(|(id, j)| (j.next_publication, *id, j.community))
            .collect();
        publish.sort_by_key(|(next, id, _)| (*next, *id));
        for (_, id, community) in publish.into_iter().take(PER_TICK) {
            let submitted = if let Some(c) = community.filter(|c| self.is_private_community(c)) {
                self.private_record_submitted(c, id)
            } else {
                let before = self.outbox.len();
                self.gossip(&id, None);
                self.outbox.len() > before
            };
            if let Some(j) = self.deletions.saved.jobs.get_mut(&id) {
                j.record_submitted |= submitted;
                j.next_publication = now + 300;
                self.deletions.dirty = true;
            }
        }
        // Status remains briefly after expiry; no capabilities survive their piece horizon.
        let before = self.deletions.saved.jobs.len();
        self.deletions.saved.jobs.retain(|_, j| j.expires_at.saturating_add(2 * DAY) > now || j.targets.values().any(|t| now / DAY <= t.expiry_day as u64));
        self.deletions.dirty |= before != self.deletions.saved.jobs.len();
    }

    pub(crate) fn deletion_restore_quarantined(&self, community: Option<Id>) -> bool {
        community.is_some_and(|c| self.deletions.saved.restored.contains_key(&c))
    }

    /// A fresh authenticated private bootstrap releases a restore only after the old
    /// snapshot's private content was discarded by `open_restored`.
    pub(crate) fn deletion_private_caught_up(&mut self, community: Id) {
        if self.deletions.saved.restored.remove(&community).is_some() {
            self.deletions.dirty = true;
        }
    }
}
