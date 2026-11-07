//! Objects kept in pieces over their whole life (SCATTER.md §5.5, §5.9, §5.10; phase S3).
//!
//! Every object this device sent or received a pointer for is kept track of here: its
//! generations (the newest, and the last one that rebuilt here while the newest has not), what
//! probes found, and whether a repair is due. Pointers and their updates are checked before they
//! count:
//!
//! - an update keeps the object, its sealed object, key and class; within a generation it only
//!   adds fresh indexes within the budget, and a new generation is the next one, with secrets of
//!   its own and a size that meets a level (`Manifest::update_to`, `size::reading`);
//! - one regeneration a day is taken, the earliest when two compete, unless the newest
//!   generation no longer rebuilds here;
//! - a generation whose pieces rebuild into anything but the object is dropped, and its sender
//!   is not heard again for that object; the generation before it stays until a newer one has
//!   rebuilt here once.
//!
//! **Repair** runs on members' durable devices (desktops, nodes). Every few hours (six for a
//! young object, less often as it ages) they ask each index's holder whether it still keeps its
//! piece (`has`, through forwarders). A holder's own "no" loses the index at once; silence loses
//! it only after two failed probes through forwarders in different places at least twelve hours
//! apart, so neither a forwarder dropping probes nor a holder that is briefly away triggers a
//! repair. Once a third of the spare pieces are gone (reachable at `k + ⌈2(n−k)/3⌉`,
//! `size::repair_margin`), the device waits a random 0–6 hours and
//! stands down if someone else's update arrives; otherwise it rebuilds the generation from k
//! pieces and places fresh indexes in places that never held one of the generation, within the
//! budget. With the budget spent it makes the next generation: fresh transform key, locator
//! secret, indexes and places, sized for that generation's smaller share of the risk, at most
//! once a day. An object whose fetches keep failing or stalling (hot) is regenerated once with
//! at least twice the pieces, which spreads the load and tightens the bound. Holders never
//! repair: they cannot.

use std::collections::{BTreeMap, BTreeSet};

use hoi_crypto::scatter::{Shards, size};
use hoi_types::Id;
use hoi_types::scatter::{Manifest, Pointer, Update};
use hoi_types::uri::{b64, unb64};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::Node;
use crate::api::R;
use crate::error::ApiError;
use crate::scatter::{ProbeResult, ScatterOutcome, uniform};

const HOUR: u64 = 3600;
const DAY: u64 = 86_400;
/// How often an object is probed: young ones every six hours, older ones less often.
const PROBE_YOUNG: u64 = 6 * HOUR;
const PROBE_MONTH: u64 = DAY;
const PROBE_OLD: u64 = 3 * DAY;
/// Two failed probes this far apart, through forwarders in different places, lose an index.
pub const LOSS_APART: u64 = 12 * HOUR;
/// A repairer waits up to this long at random before acting.
pub const REPAIR_WAIT: u64 = 6 * HOUR;
/// Generations of one object are at least this far apart.
pub const REGEN_APART: u64 = DAY;
/// Failed or stalled answers in an hour of fetches that make an object hot.
pub const HOT: u32 = 16;
/// Objects kept track of at most; the oldest are let go first.
const MAX_KEPT: usize = 4096;
/// Operations one tick starts at most.
const PER_TICK: usize = 4;
/// How far in the future a pointer's time may be.
const SKEW: u64 = 600;
/// A failed fetch of content is tried again after this long.
const APPLY_AGAIN: u64 = 300;
/// Content that came before what it belongs to is tried again for this long.
const RETRY_FOR: u64 = 3600;
const MAX_RETRY: usize = 64;
/// A drop key replaced in the list is kept this long: the pieces stored with it
/// expire within [`crate::private::RETENTION`], and a member still in the epoch before the
/// change may store with it a while longer.
pub(crate) const KEEP_REPLACED: u64 = crate::private::RETENTION + crate::private::CONTEST;
/// Drop keys a device holds per group at most.
const MAX_HELD: usize = 16;
/// Removed posts' generations waiting for this device's turn to drop, at most.
const MAX_DUE_DROPS: usize = 512;

/// A drop key a committer's device holds for a group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HeldKey {
    /// Its Ed25519 secret, base64.
    pub secret: String,
    /// When this device first saw another key listed in its place.
    #[serde(default)]
    pub replaced_at: Option<u64>,
}

/// A removed post's generation this device drops with the group's drop keys from `at`: at once
/// when it is first in line, else once those ahead had their turn, and then only if an index
/// asked (`check`) is still kept.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DueDrop {
    pub community: Id,
    /// The generation's manifest, canonical CBOR in base64.
    pub manifest: String,
    pub at: u64,
    pub check: bool,
}

fn secret_of(b64: &str) -> Option<[u8; 32]> {
    unb64(b64).ok().and_then(|b| <[u8; 32]>::try_from(b).ok())
}

/// One generation as this device keeps it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Gen {
    /// The manifest, canonical CBOR in base64 (it holds the object's keys: the store this
    /// blob lives in keeps it as it keeps the groups' MLS secrets).
    pub manifest: String,
    pub made_at: u64,
    /// It rebuilt on this device once: the generation before it may be let go.
    pub confirmed: bool,
    /// The member device that sent it; none when made here.
    pub from: Option<Id>,
}

impl Gen {
    pub(crate) fn read(&self) -> Option<Manifest> {
        Manifest::decode(&unb64(&self.manifest).ok()?).ok()
    }
}

/// An object this device keeps track of.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Kept {
    /// The community it belongs to: where updates go.
    pub community: Option<Id>,
    /// The newest generation last, and the one before it: the fallback while the newest does
    /// not rebuild here, and what a competing new generation is checked against.
    pub gens: Vec<Gen>,
    pub expires_at: u64,
    /// The first time this device heard of it.
    pub since: u64,
    /// Failed probes of the newest generation, per index: when, and the forwarder's place.
    pub fails: BTreeMap<u8, Vec<(u64, u64)>>,
    /// Indexes of the newest generation that are gone.
    pub lost: BTreeSet<u8>,
    pub next_probe: u64,
    /// When a repair this device scheduled may act.
    pub repair_at: Option<u64>,
    /// Its content still has to be fetched and applied (a private group's payloads), from then.
    pub apply: bool,
    pub apply_at: u64,
    /// Senders of a generation that rebuilt to something else: not heard again for it.
    pub refused: BTreeSet<Id>,
    /// The newest generation failed to rebuild here last time.
    pub unreadable: bool,
    /// Trouble fetching it this hour (hour, count), and when it was last made larger for it.
    pub trouble: (u64, u32),
    pub hot_at: u64,
    /// This device made it: it may drop its pieces with its own secret.
    pub mine: bool,
    /// The posts and comments it carries (a private group's object holds one), and the media
    /// item it carries a part of: once all of them are deleted or removed here, the object is
    /// let go, and dropped by whoever can drop it.
    pub records: Vec<Id>,
    pub media: Option<Id>,
    /// In an open community (crate::open): what its first pointer said it holds (the record, and
    /// the media item), and who made that pointer (its person); updates must name the same, and
    /// come from them or an established member.
    pub claim: Option<(Id, Option<Id>)>,
    pub author: Option<Id>,
    pub open: bool,
    /// What this device keeps of the object's content sealed to other bytes than the manifest
    /// says: it never repairs the object from its own copy.
    pub copy_mismatch: bool,
}

impl Kept {
    pub(crate) fn newest(&self) -> Option<Manifest> {
        self.gens.last().and_then(Gen::read)
    }

    fn newest_made(&self) -> u64 {
        self.gens.last().map(|g| g.made_at).unwrap_or(0)
    }

    /// The generation to fetch: the newest, or while it does not rebuild here, the one before.
    fn to_load(&self) -> Option<Manifest> {
        if self.unreadable && self.gens.len() > 1 {
            return self.gens[self.gens.len() - 2].read();
        }
        self.newest()
    }
}

/// One object of outgoing payloads: its content (a JSON array of payloads), the posts it
/// carries and the media item it is a part of.
pub(crate) struct OutPart {
    pub content: Vec<u8>,
    pub records: Vec<Id>,
    pub media: Option<Id>,
}

/// What a finished operation was for.
pub(crate) enum Job {
    /// Fetching an object to apply its content.
    Apply {
        object: Id,
        generation: u8,
    },
    Probe {
        object: Id,
        generation: u8,
    },
    /// Fetching k pieces for a repair; `hot`: the next generation is made larger.
    Rebuild {
        object: Id,
        generation: u8,
        hot: bool,
    },
    /// Placing fresh indexes or a new generation: the update goes out when stored.
    Place {
        object: Id,
    },
    /// One object of a private group's outgoing payloads (crate::private).
    Outgoing {
        batch: u64,
    },
    /// Drops sent for an object let go: nothing waits on their answers.
    Dropped,
    /// One index of a removed post's generation asked before this device, later in line,
    /// drops it: only if it is still kept.
    DropCheck {
        community: Id,
        manifest: String,
    },
    /// A rebuild this home node runs for one of its devices (crate::rebuild).
    StoreForDevice {
        peer: String,
        rid: u32,
    },
    ForDevice {
        peer: String,
        rid: u32,
    },
}

/// Payloads of a private group on their way into pieces: their pointers go to the group once
/// every object is stored; if any is not, they go whole, as before.
pub(crate) struct Outgoing {
    pub community: Id,
    /// Operation → the object's content (a JSON array of payloads).
    pub parts: BTreeMap<u64, Vec<u8>>,
    /// Operation → the posts it carries and the media item it is part of.
    pub carries: BTreeMap<u64, (Vec<Id>, Option<Id>)>,
    pub stored: BTreeMap<u64, Manifest>,
    pub failed: bool,
    pub records: Vec<Id>,
    pub media: Vec<(Id, usize)>,
}

/// Repair and fetching in memory.
#[derive(Default)]
pub(crate) struct Runtime {
    /// Objects with an operation in flight.
    busy: BTreeSet<Id>,
    /// Shards rebuilt for a repair, by operation.
    pub(crate) rebuilt: BTreeMap<u64, (Shards, Vec<u8>)>,
    pub(crate) outgoing: BTreeMap<u64, Outgoing>,
    next_batch: u64,
    /// Records and media of each group on their way into pieces: kept out of later batches.
    pub(crate) inflight_records: BTreeMap<Id, BTreeSet<Id>>,
    pub(crate) inflight_media: BTreeMap<Id, BTreeSet<Id>>,
    /// Content that came before what it belongs to: (community, object, content, until).
    retry: Vec<(Id, Id, Vec<u8>, u64)>,
}

/// How often an object `age` seconds old is probed.
fn probe_every(age: u64) -> u64 {
    if age < 7 * DAY {
        PROBE_YOUNG
    } else if age < 30 * DAY {
        PROBE_MONTH
    } else {
        PROBE_OLD
    }
}

fn jitter(span: u64) -> u64 {
    if span == 0 { 0 } else { uniform(span as usize) as u64 }
}

/// Whether failed probes of one index lose it: two through forwarders in different places, at
/// least twelve hours apart.
pub(crate) fn lost_by(fails: &[(u64, u64)]) -> bool {
    fails.iter().any(|a| fails.iter().any(|b| a.1 != b.1 && b.0 >= a.0 + LOSS_APART))
}

impl Node {
    /// A community's level today, or the floor.
    pub(crate) fn community_level(&self, c: Option<Id>) -> u8 {
        c.and_then(|c| self.communities.get(&c)).map(|st| st.limits.scatter.resist).unwrap_or(size::DEFAULT_LEVEL)
    }

    /// A pointer for `community`, from `from` (a member's device; none when made here). Checks
    /// it against what this device knows of the object. Returns whether the object is new.
    /// `apply`: its content is to be fetched and applied (a private group's payloads).
    pub(crate) fn take_pointer(&mut self, community: Option<Id>, pointer: Pointer, from: Option<Id>, apply: bool) -> R<bool> {
        let now = self.now();
        let m = pointer.manifest;
        let refuse = |s: &str| Err(ApiError::new("scatter-update", s.to_string()));
        if pointer.made_at > now + SKEW || m.expires_at <= now {
            return refuse("This pointer is from the future or has expired.");
        }
        let s = size::Size { k: m.k as usize, n: m.n as usize, m: m.m as usize };
        if size::reading(m.generation, m.places, s).is_none() || !size::meets(m.level, m.generation, m.places, s) {
            return refuse("This post is spread thinner than any level allows.");
        }
        let object = Id(m.object);
        let fresh = Gen { manifest: b64(&m.encode()), made_at: pointer.made_at, confirmed: from.is_none(), from };
        let Some(kept) = self.scatter.saved.kept.get_mut(&object) else {
            if self.scatter.saved.kept.len() >= MAX_KEPT {
                let oldest = self.scatter.saved.kept.iter().min_by_key(|(_, k)| k.expires_at).map(|(id, _)| *id);
                if let Some(o) = oldest {
                    self.scatter.saved.kept.remove(&o);
                }
            }
            let every = probe_every(0);
            let kept = Kept {
                community,
                gens: vec![fresh],
                expires_at: m.expires_at,
                since: now,
                next_probe: now + every + jitter(every / 6),
                apply,
                apply_at: now,
                mine: from.is_none(),
                ..Default::default()
            };
            self.scatter.saved.kept.insert(object, kept);
            self.scatter.dirty = true;
            return Ok(true);
        };
        if from.is_some_and(|f| kept.refused.contains(&f)) {
            return refuse("An earlier update from this member did not rebuild.");
        }
        let Some(newest) = kept.newest() else { return refuse("Unreadable pointer kept.") };
        if m.generation < newest.generation {
            return Ok(false);
        }
        match newest.update_to(&m) {
            Ok(Update::Extend) => {
                let last = kept.gens.last_mut().expect("a generation");
                last.manifest = fresh.manifest;
                last.made_at = last.made_at.max(fresh.made_at);
                kept.repair_at = None;
            }
            Ok(Update::Regenerate) => {
                if pointer.made_at < kept.newest_made() + REGEN_APART && !kept.unreadable {
                    return refuse("One new generation a day.");
                }
                // the generation before stays: the fallback while this one does not rebuild here,
                // and what a competing new generation is checked against
                let prev = kept.gens.pop();
                kept.gens = prev.into_iter().chain([fresh]).collect();
                kept.expires_at = kept.expires_at.max(m.expires_at);
                kept.repair_at = None;
                kept.fails.clear();
                kept.lost.clear();
                kept.unreadable = false;
            }
            Err(_) if m.generation == newest.generation && m.generation > 0 => {
                // Two repairers made the same generation: the earliest wins everywhere, then
                // the smaller manifest, so every member ends with the same one.
                let earlier = kept.gens.len() >= 2 && {
                    let prev = &kept.gens[kept.gens.len() - 2];
                    let apart = pointer.made_at >= prev.made_at + REGEN_APART || kept.unreadable;
                    apart && prev.read().is_some_and(|p| p.update_to(&m) == Ok(Update::Regenerate))
                };
                let last = kept.gens.last().expect("a generation");
                let wins = (pointer.made_at, fresh.manifest.as_str()) < (last.made_at, last.manifest.as_str());
                if !earlier || !wins {
                    return Ok(false);
                }
                *kept.gens.last_mut().expect("a generation") = fresh;
                kept.repair_at = None;
                kept.fails.clear();
                kept.lost.clear();
            }
            Err(e) => return Err(e.into()),
        }
        self.scatter.dirty = true;
        Ok(false)
    }

    /// Forgets objects whose pieces expired.
    fn forget_expired(&mut self) {
        let now = self.now();
        let before = self.scatter.saved.kept.len();
        self.scatter.saved.kept.retain(|_, k| k.expires_at > now);
        if self.scatter.saved.kept.len() != before {
            self.scatter.dirty = true;
        }
    }

    /// Starts what kept objects need: fetching content to apply, and on durable devices probes
    /// and repairs. A few at a time.
    pub(crate) fn keep_tick(&mut self) {
        self.forget_expired();
        self.retry_scattered();
        self.tend_drop_keys();
        // removals that ripened since: what they removed is let go now, not at the next view
        self.settle_dirty();
        self.let_go_of_gone();
        self.drops_due();
        let now = self.now();
        let durable = self.can_relay();
        let mut started = 0;
        let ids: Vec<Id> = self.scatter.saved.kept.keys().copied().collect();
        for object in ids {
            if started >= PER_TICK {
                break;
            }
            if self.scatter.repair.busy.contains(&object) {
                continue;
            }
            let Some(k) = self.scatter.saved.kept.get(&object) else { continue };
            let Some(newest) = k.newest() else { continue };
            let Some(target) = k.to_load() else { continue };
            let generation = newest.generation;
            let hot = k.trouble.1 >= HOT && now >= k.hot_at + REGEN_APART && now >= k.newest_made() + REGEN_APART;
            let (apply, repair, probe) = (k.apply && now >= k.apply_at, k.repair_at.is_some_and(|at| now >= at), now >= k.next_probe);
            let lost = k.lost.clone();
            let failed: BTreeMap<u8, BTreeSet<u64>> = k.fails.iter().map(|(i, f)| (*i, f.iter().map(|x| x.1).collect())).collect();
            let since = k.since;
            let job = if apply {
                Some(Job::Apply { object, generation: target.generation })
            } else if !durable {
                None
            } else if repair {
                Some(Job::Rebuild { object, generation: target.generation, hot: false })
            } else if hot {
                if let Some(k) = self.scatter.saved.kept.get_mut(&object) {
                    k.hot_at = now;
                    k.trouble = (0, 0);
                }
                Some(Job::Rebuild { object, generation: target.generation, hot: true })
            } else if probe {
                Some(Job::Probe { object, generation })
            } else {
                None
            };
            let Some(job) = job else { continue };
            let op = match &job {
                Job::Probe { .. } => {
                    let indexes: Vec<u8> = (0..newest.hashes.len() as u8).filter(|i| !lost.contains(i)).collect();
                    let every = probe_every(now.saturating_sub(since));
                    if let Some(k) = self.scatter.saved.kept.get_mut(&object) {
                        k.next_probe = now + every + jitter(every / 6);
                        self.scatter.dirty = true;
                    }
                    self.scatter_probe(&newest, &indexes, &failed)
                }
                Job::Apply { .. } => {
                    if let Some(k) = self.scatter.saved.kept.get_mut(&object) {
                        k.apply_at = now + APPLY_AGAIN;
                    }
                    self.load_kept(&target)
                }
                Job::Rebuild { .. } => {
                    if let Some(k) = self.scatter.saved.kept.get_mut(&object) {
                        k.repair_at = None;
                    }
                    self.scatter_load(&target)
                }
                _ => continue,
            };
            if let Ok(op) = op {
                self.scatter.repair.busy.insert(object);
                self.scatter.jobs.insert(op, job);
                started += 1;
                // it may have finished at once (nothing reachable)
                if self.op_done(op) {
                    self.job_done(op);
                }
            }
        }
    }

    /// An operation finished: what it was for goes on.
    pub(crate) fn job_done(&mut self, op: u64) {
        let Some(job) = self.scatter.jobs.remove(&op) else { return };
        let outcome = self.take_outcome(op);
        let rebuilt = self.scatter.repair.rebuilt.remove(&op);
        match job {
            Job::Outgoing { batch } => self.outgoing_done(batch, op, outcome),
            Job::Dropped => {}
            Job::DropCheck { community, manifest } => {
                // a holder's "no": someone ahead dropped it; anything else, and it goes now
                let gone = matches!(&outcome, ScatterOutcome::Probed(r) if r.iter().any(|x| x.kept == Some(false)));
                if !gone && let Some(m) = unb64(&manifest).ok().and_then(|b| Manifest::decode(&b).ok()) {
                    self.drop_with_keys(community, &m);
                }
            }
            Job::StoreForDevice { peer, rid } => self.stored_for_home_device(&peer, rid, outcome),
            Job::ForDevice { peer, rid } => self.rebuilt_for(&peer, rid, outcome),
            Job::Apply { object, generation } => {
                self.scatter.repair.busy.remove(&object);
                self.applied(object, generation, outcome);
            }
            Job::Probe { object, generation } => {
                self.scatter.repair.busy.remove(&object);
                if let ScatterOutcome::Probed(results) = outcome {
                    self.probed(object, generation, &results);
                }
            }
            Job::Rebuild { object, generation, hot } => {
                self.scatter.repair.busy.remove(&object);
                match (outcome, rebuilt) {
                    (ScatterOutcome::Loaded(_), Some((shards, sealed))) => self.repair(object, generation, hot, shards, sealed),
                    (ScatterOutcome::Failed(why), _) if why == crate::scatter::MISMATCH => self.mismatch(object, generation),
                    _ => {
                        // too few pieces came: try again after the next probes, or, when the
                        // pieces really are too few, make the object again from this device's
                        // own copy
                        let was = self.scatter.saved.kept.get(&object).is_some_and(|k| k.unreadable);
                        if let Some(k) = self.scatter.saved.kept.get_mut(&object) {
                            k.unreadable = true;
                        }
                        self.repair_from_copy(object, was);
                    }
                }
            }
            Job::Place { object } => {
                self.scatter.repair.busy.remove(&object);
                if let ScatterOutcome::Stored(manifest) = outcome {
                    self.publish_update(object, manifest);
                }
            }
        }
    }

    /// Notes failed or stalled answers of a load (crate::scatter): many in an hour make the
    /// object hot.
    pub(crate) fn note_trouble(&mut self, op: u64, trouble: u32) {
        let object = match self.scatter.jobs.get(&op) {
            Some(Job::Apply { object, .. } | Job::Rebuild { object, .. }) => *object,
            _ => return,
        };
        let hour = self.now() / HOUR;
        if let Some(k) = self.scatter.saved.kept.get_mut(&object) {
            if k.trouble.0 != hour {
                k.trouble = (hour, 0);
            }
            k.trouble.1 = k.trouble.1.saturating_add(trouble);
        }
    }

    /// The content of a generation rebuilt here, or why not.
    pub(crate) fn applied(&mut self, object: Id, generation: u8, outcome: ScatterOutcome) {
        match outcome {
            ScatterOutcome::Loaded(content) => {
                let Some(k) = self.scatter.saved.kept.get_mut(&object) else { return };
                k.apply = false;
                k.unreadable = false;
                confirm(k, generation);
                let community = k.community;
                self.scatter.dirty = true;
                if let Some(c) = community {
                    let open = self.scatter.saved.kept.get(&object).is_some_and(|k| k.open);
                    if !open {
                        let (records, media) = crate::private::carried(&content);
                        if let Some(k) = self.scatter.saved.kept.get_mut(&object) {
                            k.records = records;
                            k.media = media;
                        }
                    }
                    self.apply_scattered(c, object, content);
                }
            }
            ScatterOutcome::Failed(why) if why == crate::scatter::MISMATCH => self.mismatch(object, generation),
            // a home node that never answered says nothing about the generation: the next try
            // asks for the same one
            ScatterOutcome::Failed(why) if why == crate::rebuild::HOME_AWAY => {}
            _ => {
                // the next try fetches the generation before it, if one rebuilt here (`to_load`)
                if let Some(k) = self.scatter.saved.kept.get_mut(&object) {
                    k.unreadable = !k.unreadable || k.gens.len() < 2;
                    self.scatter.dirty = true;
                }
                // unreadable here, a new generation refused earlier for coming within a day
                // counts now: the pointer records that carry it are read again
                self.retake_pointers(object);
            }
        }
    }

    /// An open community's pointer records for `object`, read again: an update refused while
    /// the object still read here may count once it does not.
    fn retake_pointers(&mut self, object: Id) {
        let Some(k) = self.scatter.saved.kept.get(&object) else { return };
        let (Some(c), Some(claim), true) = (k.community, k.claim, k.unreadable) else { return };
        let mut pointers: Vec<(u64, hoi_types::Record, Id)> = self
            .store
            .of_kind(&c, hoi_types::Kind::Pointer)
            .filter(|s| matches!(&s.record.body, hoi_types::Body::Pointer { record, media, .. } if (*record, *media) == claim))
            .map(|s| (s.record.created_at, s.record.clone(), s.author_root))
            .collect();
        pointers.sort_by_key(|p| p.0);
        for (_, r, root) in pointers {
            self.on_pointer_record(&r, root);
        }
    }

    /// Whether this device may make an object again from its own copy: a durable device (a
    /// desktop, a node) of a member of the private group, or in an open community of the
    /// object's author or an established member (whose updates readers take).
    fn may_repair_from_copy(&self, k: &Kept) -> bool {
        let Some(c) = k.community else { return false };
        if !self.can_relay() {
            return false;
        }
        if !k.open {
            return true;
        }
        let Some(me) = self.me_root() else { return false };
        k.author == Some(me) || self.communities.get(&c).is_some_and(|st| st.level(&me) >= hoi_moderation::Level::Established)
    }

    /// The sealed object `m` names, made again from what this device keeps of its content: the
    /// same content seals to the same bytes. `Err(true)` when what is kept here seals
    /// to other bytes; `Err(false)` when nothing of it is kept here.
    fn own_sealed(&self, k: &Kept, m: &Manifest) -> Result<Vec<u8>, bool> {
        let Some(c) = k.community else { return Err(false) };
        let (records, media) = match k.claim {
            Some((record, media)) => (vec![record], media),
            None => (k.records.clone(), k.media),
        };
        let contents = match media {
            Some(hash) => {
                let Some(bytes) = self.store.blob(&hash) else { return Err(false) };
                crate::private::media_contents(c, hash, bytes, &records)
            }
            None => {
                let [record] = records.as_slice() else { return Err(false) };
                let Some(s) = self.store.get(record) else { return Err(false) };
                vec![crate::private::records_content(c, &s.wire)]
            }
        };
        contents
            .into_iter()
            .filter(|content| hoi_types::scatter::Class::for_len(content.len()) == Some(m.class))
            .find_map(|content| {
                let (class, sealed) = hoi_crypto::scatter::seal_object(&content, &m.object, &m.key).ok()?;
                (class == m.class && hoi_crypto::scatter::sealed_hash(&sealed) == m.sealed).then_some(sealed)
            })
            .ok_or(true)
    }

    /// Too few pieces of an object are left to rebuild it, or neither generation does: a device
    /// that keeps its content and may repair it makes the next generation from its own copy,
    /// once it seals to the very bytes the manifest names; it publishes nothing otherwise.
    fn repair_from_copy(&mut self, object: Id, was_unreadable: bool) {
        let Some(k) = self.scatter.saved.kept.get(&object) else { return };
        let Some(newest) = k.newest() else { return };
        if k.copy_mismatch || !self.may_repair_from_copy(k) {
            return;
        }
        let reachable = newest.hashes.len().saturating_sub(k.lost.len());
        if reachable >= newest.k as usize && !was_unreadable {
            return;
        }
        let sealed = match self.own_sealed(k, &newest) {
            Ok(sealed) => sealed,
            Err(mismatch) => {
                if mismatch && let Some(k) = self.scatter.saved.kept.get_mut(&object) {
                    k.copy_mismatch = true;
                    self.scatter.dirty = true;
                }
                return;
            }
        };
        let Some(next) = newest.generation.checked_add(1) else { return };
        let community = k.community;
        let level = self.community_level(community);
        self.scatter.next_drop_key = community.and_then(|c| self.community_drop_key(c)).map(|k| hoi_crypto::scatter::drop_key_id(&k));
        self.scatter.placing = community;
        let started = self.start_generation(&newest, &sealed, next, level, 0);
        self.scatter.next_drop_key = None;
        self.scatter.placing = None;
        if let Ok(op) = started {
            self.scatter.repair.busy.insert(object);
            self.scatter.jobs.insert(op, Job::Place { object });
            if self.op_done(op) {
                self.job_done(op);
            }
        }
    }

    /// A generation's pieces rebuilt into something else: dropped, and whoever sent it is not
    /// heard again for this object. The one before it, if any, is used again.
    fn mismatch(&mut self, object: Id, generation: u8) {
        let Some(k) = self.scatter.saved.kept.get_mut(&object) else { return };
        let bad = k.gens.iter().position(|g| g.read().is_some_and(|m| m.generation == generation) && !g.confirmed);
        if let Some(i) = bad {
            let g = k.gens.remove(i);
            if let Some(f) = g.from {
                k.refused.insert(f);
            }
            k.fails.clear();
            k.lost.clear();
            k.apply_at = 0;
        }
        if k.gens.is_empty() {
            self.scatter.saved.kept.remove(&object);
        }
        self.scatter.dirty = true;
    }

    /// What a round of probes found.
    fn probed(&mut self, object: Id, generation: u8, results: &[ProbeResult]) {
        let now = self.now();
        let Some(k) = self.scatter.saved.kept.get_mut(&object) else { return };
        let Some(m) = k.newest().filter(|m| m.generation == generation) else { return };
        for r in results {
            match r.kept {
                Some(true) => {
                    k.fails.remove(&r.index);
                }
                // the holder itself said so: gone at once
                Some(false) => {
                    k.lost.insert(r.index);
                }
                None => {
                    let f = k.fails.entry(r.index).or_default();
                    f.push((now, r.place));
                    if f.len() > 4 {
                        f.remove(0);
                    }
                    if lost_by(f) {
                        k.lost.insert(r.index);
                    }
                }
            }
        }
        let kk = m.k as usize;
        let reachable = m.hashes.len().saturating_sub(k.lost.len());
        if reachable <= kk + size::repair_margin(kk, m.n as usize) && k.repair_at.is_none() {
            k.repair_at = Some(now + jitter(REPAIR_WAIT));
        }
        self.scatter.dirty = true;
    }

    /// Repairs a generation from its rebuilt shards: fresh indexes within the budget, else the
    /// next generation (at most one a day, unless too few pieces are left to rebuild it).
    fn repair(&mut self, object: Id, generation: u8, hot: bool, shards: Shards, sealed: Vec<u8>) {
        let now = self.now();
        let Some(k) = self.scatter.saved.kept.get(&object) else { return };
        let Some(newest) = k.newest() else { return };
        let Some(m) = k.gens.iter().filter_map(Gen::read).find(|m| m.generation == generation) else { return };
        if m.generation != newest.generation {
            // the newest does not rebuild: its successor is made from the one that did
            let Some(next) = newest.generation.checked_add(1) else { return };
            let level = self.community_level(k.community);
            self.scatter.next_drop_key = k.community.and_then(|c| self.community_drop_key(c)).map(|k| hoi_crypto::scatter::drop_key_id(&k));
            self.scatter.placing = k.community;
            let started = self.start_generation(&newest, &sealed, next, level, 0);
            self.scatter.next_drop_key = None;
            self.scatter.placing = None;
            if let Ok(op) = started {
                self.scatter.repair.busy.insert(object);
                self.scatter.jobs.insert(op, Job::Place { object });
                if self.op_done(op) {
                    self.job_done(op);
                }
            }
            return;
        }
        let placed = m.hashes.len();
        let reachable = placed.saturating_sub(k.lost.len());
        let want = (m.n as usize).saturating_sub(reachable);
        let left = (m.m as usize).saturating_sub(placed);
        let community = k.community;
        let made = k.newest_made();
        self.scatter.next_drop_key = community.and_then(|c| self.community_drop_key(c)).map(|k| hoi_crypto::scatter::drop_key_id(&k));
        self.scatter.placing = community;
        let op = if !hot && want == 0 {
            self.scatter.next_drop_key = None;
            self.scatter.placing = None;
            return;
        } else if !hot && left > 0 {
            self.scatter_extend(m.clone(), shards, want.min(left))
        } else if now >= made + REGEN_APART || reachable < m.k as usize {
            let Some(next) = generation.checked_add(1) else { return };
            let level = self.community_level(community);
            let min_n = if hot { 2 * m.n as usize } else { 0 };
            self.start_generation(&m, &sealed, next, level, min_n)
        } else {
            self.scatter.next_drop_key = None;
            self.scatter.placing = None;
            if let Some(k) = self.scatter.saved.kept.get_mut(&object) {
                k.repair_at = Some(made + REGEN_APART);
            }
            return;
        };
        self.scatter.next_drop_key = None;
        self.scatter.placing = None;
        if let Ok(op) = op {
            self.scatter.repair.busy.insert(object);
            self.scatter.jobs.insert(op, Job::Place { object });
            if self.op_done(op) {
                self.job_done(op);
            }
        }
    }

    /// A repair or a new generation is placed: taken here, and sent to the community.
    fn publish_update(&mut self, object: Id, manifest: Manifest) {
        let now = self.now();
        let pointer = Pointer { made_at: now, manifest };
        let community = self.scatter.saved.kept.get(&object).and_then(|k| k.community);
        if self.take_pointer(community, pointer.clone(), None, false).is_err() {
            return;
        }
        if let Some(k) = self.scatter.saved.kept.get_mut(&object) {
            confirm(k, pointer.manifest.generation);
            k.mine = true;
        }
        if let Some(c) = community {
            self.scattered_pointers(c, vec![pointer], Vec::new(), Vec::new());
        }
    }

    // ------------------------------------------------------------------ a private group's payloads

    /// Starts keeping a group's outgoing payloads in pieces, one object per content (a JSON
    /// array of payloads). False when this device cannot scatter now: the caller sends them
    /// whole.
    pub(crate) fn scatter_outgoing(&mut self, c: Id, parts: Vec<OutPart>, records: Vec<Id>, media: Vec<(Id, usize)>, expires_at: u64) -> bool {
        if self.scattering_home(c).is_some() {
            for id in &records {
                self.scatter.saved.home_delivery.insert(*id, "placing".into());
            }
            self.scatter.dirty = true;
            self.mark_changed(c);
        }
        let level = self.community_level(Some(c));
        let drop_key = self.community_drop_key(c).map(|k| hoi_crypto::scatter::drop_key_id(&k));
        let mut contents = BTreeMap::new();
        let mut carries = BTreeMap::new();
        for part in parts {
            self.scatter.next_drop_key = drop_key;
            self.scatter.placing = Some(c);
            match self.scatter_store(&part.content, level, expires_at) {
                Ok(op) => {
                    contents.insert(op, part.content);
                    carries.insert(op, (part.records, part.media));
                }
                Err(_) => {
                    // nothing half done: what started is let go, all go whole
                    for op in contents.keys() {
                        self.drop_op(*op);
                    }
                    return false;
                }
            }
        }
        let batch = self.scatter.repair.next_batch;
        self.scatter.repair.next_batch += 1;
        self.scatter.repair.inflight_records.entry(c).or_default().extend(records.iter().copied());
        self.scatter.repair.inflight_media.entry(c).or_default().extend(media.iter().map(|(h, _)| *h));
        let ops: Vec<u64> = contents.keys().copied().collect();
        self.scatter.repair.outgoing.insert(batch, Outgoing { community: c, parts: contents, carries, stored: BTreeMap::new(), failed: false, records, media });
        for op in ops {
            self.scatter.jobs.insert(op, Job::Outgoing { batch });
            if self.op_done(op) {
                self.job_done(op);
            }
        }
        true
    }

    fn outgoing_done(&mut self, batch: u64, op: u64, outcome: ScatterOutcome) {
        let Some(out) = self.scatter.repair.outgoing.get_mut(&batch) else { return };
        match outcome {
            ScatterOutcome::Stored(m) => {
                out.stored.insert(op, m);
            }
            _ => out.failed = true,
        }
        if out.stored.len() + usize::from(out.failed) < out.parts.len() && !out.failed {
            return;
        }
        if !out.failed && out.stored.len() < out.parts.len() {
            return;
        }
        let out = self.scatter.repair.outgoing.remove(&batch).expect("present");
        // the other parts still running are let go
        for other in out.parts.keys().filter(|o| !out.stored.contains_key(o) && **o != op) {
            self.scatter.jobs.remove(other);
            self.drop_op(*other);
        }
        if let Some(set) = self.scatter.repair.inflight_records.get_mut(&out.community) {
            for r in &out.records {
                set.remove(r);
            }
        }
        if let Some(set) = self.scatter.repair.inflight_media.get_mut(&out.community) {
            for (h, _) in &out.media {
                set.remove(h);
            }
        }
        if out.failed {
            if self.is_private_community(&out.community) {
                self.send_whole(out.community, out.parts.into_values().collect(), out.records, out.media);
            } else {
                let carried: Vec<Id> = out.carries.values().flat_map(|(r, _)| r.clone()).collect();
                self.open_whole(out.community, carried);
            }
            return;
        }
        let open = !self.is_private_community(&out.community);
        let me = self.me_root();
        let now = self.now();
        let mut pointers = Vec::new();
        for (op, m) in out.stored {
            let object = Id(m.object);
            let pointer = Pointer { made_at: now, manifest: m };
            let _ = self.take_pointer(Some(out.community), pointer.clone(), None, false);
            if let (Some(k), Some((records, media))) = (self.scatter.saved.kept.get_mut(&object), out.carries.get(&op)) {
                k.records = records.clone();
                k.media = *media;
                if open {
                    k.open = true;
                    k.claim = records.first().map(|r| (*r, *media));
                    k.author = me;
                    self.scatter.in_pieces.extend(records.iter().copied());
                }
            }
            pointers.push(pointer);
        }
        self.scattered_pointers(out.community, pointers, out.records, out.media);
    }

    /// Content rebuilt for a community: a private group's payloads, or an open community's
    /// record or media (crate::open). One that came before what it belongs to is tried again
    /// for a while.
    fn apply_content(&mut self, c: Id, object: Id, content: &[u8]) -> R<()> {
        if self.is_private_community(&c) { self.private_apply_content(c, content) } else { self.open_apply_content(c, object, content) }
    }

    fn apply_scattered(&mut self, c: Id, object: Id, content: Vec<u8>) {
        if let Err(e) = self.apply_content(c, object, &content)
            && e.code == "private-pending"
            && self.scatter.repair.retry.len() < MAX_RETRY
        {
            let until = self.now() + RETRY_FOR;
            self.scatter.repair.retry.push((c, object, content, until));
        }
    }

    fn retry_scattered(&mut self) {
        if self.scatter.repair.retry.is_empty() {
            return;
        }
        let now = self.now();
        for (c, object, content, until) in std::mem::take(&mut self.scatter.repair.retry) {
            if now >= until {
                continue;
            }
            if let Err(e) = self.apply_content(c, object, &content)
                && e.code == "private-pending"
            {
                self.scatter.repair.retry.push((c, object, content, until));
            }
        }
    }

    /// Pointers for a community: through a private group's mailboxes, or as pointer records of
    /// an open community (crate::open).
    fn scattered_pointers(&mut self, c: Id, pointers: Vec<Pointer>, records: Vec<Id>, media: Vec<(Id, usize)>) {
        if self.is_private_community(&c) { self.send_pointers(c, pointers, records, media) } else { self.open_pointers(c, pointers) }
    }

    /// The drop key a group's mailbox list names, or an open community's ledger: what its
    /// pieces are stored with now.
    pub(crate) fn community_drop_key(&self, c: Id) -> Option<[u8; 32]> {
        if self.is_private_community(&c) { self.private_epoch(c).and_then(|e| e.drop_key) } else { self.communities.get(&c)?.drop_key().map(|d| d.grant.key.0) }
    }

    /// This device's place in the line that drops group `c`'s removed posts: the same as for
    /// its commits, the owner's device alone in a group that does not follow
    /// its records. None when it is not in it.
    pub(crate) fn drop_rank(&self, c: Id) -> Option<usize> {
        if !self.is_private_community(&c) {
            let me = self.device_kp.as_ref()?.public();
            return self.open_line(c).iter().position(|(d, _)| *d == me);
        }
        let ep = self.private_epoch(c).filter(|e| e.active)?;
        if self.follows_records(c) { ep.rank } else { ep.owner.then_some(0) }
    }

    /// The secrets of the drop keys this device holds for group `c`, the listed one first.
    pub(crate) fn held_drop_keys(&self, c: Id) -> Vec<[u8; 32]> {
        let listed = self.community_drop_key(c);
        let mut keys: Vec<[u8; 32]> = self.scatter.saved.held_keys.get(&c).into_iter().flatten().filter_map(|k| secret_of(&k.secret)).collect();
        keys.sort_by_key(|s| Some(hoi_crypto::Keypair::from_secret(s).public().0) != listed);
        keys
    }

    /// The secret of group `c`'s drop key `public`, if this device holds it.
    pub(crate) fn held_drop_key(&self, c: Id, public: &[u8; 32]) -> Option<[u8; 32]> {
        self.held_drop_keys(c).into_iter().find(|s| hoi_crypto::Keypair::from_secret(s).public().0 == *public)
    }

    /// Keeps a drop key's secret for group `c`: one made here, or one a committer gave.
    pub(crate) fn hold_drop_key(&mut self, c: Id, secret: [u8; 32]) {
        let list = self.scatter.saved.held_keys.entry(c).or_default();
        if list.iter().any(|k| secret_of(&k.secret) == Some(secret)) {
            return;
        }
        list.push(HeldKey { secret: b64(&secret), replaced_at: None });
        // the oldest replaced ones go first
        while list.len() > MAX_HELD {
            let Some(i) = list.iter().enumerate().filter(|(_, k)| k.replaced_at.is_some()).min_by_key(|(_, k)| k.replaced_at).map(|(i, _)| i) else { break };
            list.remove(i).secret.zeroize();
        }
        self.scatter.dirty = true;
    }

    /// Keeps this device's drop keys in step with its groups: the listed key is the
    /// current one; another counts as replaced from when this device first sees it so, and is
    /// forgotten [`KEEP_REPLACED`] later, when the pieces stored with it have expired. A device
    /// that is no longer in a group's line forgets its keys, and the drops it had due there.
    fn tend_drop_keys(&mut self) {
        let now = self.now();
        let groups: Vec<Id> = self.scatter.saved.held_keys.keys().copied().collect();
        for c in groups {
            if self.drop_rank(c).is_none() {
                if let Some(mut list) = self.scatter.saved.held_keys.remove(&c) {
                    list.iter_mut().for_each(|k| k.secret.zeroize());
                }
                self.scatter.saved.due_drops.retain(|d| d.community != c);
                self.scatter.dirty = true;
                continue;
            }
            let listed = self.community_drop_key(c);
            // an open community's pieces live as long as its posts may
            let keep_replaced = if self.is_private_community(&c) {
                KEEP_REPLACED
            } else {
                self.communities.get(&c).map(|st| st.limits.max_retention + DAY).unwrap_or(KEEP_REPLACED)
            };
            let Some(list) = self.scatter.saved.held_keys.get_mut(&c) else { continue };
            for k in list.iter_mut() {
                let current = secret_of(&k.secret).map(|s| hoi_crypto::Keypair::from_secret(&s).public().0) == listed;
                let was = k.replaced_at;
                k.replaced_at = if current { None } else { Some(was.unwrap_or(now)) };
                if k.replaced_at != was {
                    self.scatter.dirty = true;
                }
            }
            let before = list.len();
            list.retain_mut(|k| {
                let keep = k.replaced_at.is_none_or(|at| now < at + keep_replaced);
                if !keep {
                    k.secret.zeroize();
                }
                keep
            });
            if list.len() != before {
                self.scatter.dirty = true;
            }
            if list.is_empty() {
                self.scatter.saved.held_keys.remove(&c);
            }
        }
    }

    /// Drops a generation of group `c` with every drop key this device holds for it: its
    /// pieces may have been stored with a key since replaced, and repairs since with the new
    /// one. False if they could not all be sent now.
    fn drop_with_keys(&mut self, c: Id, m: &Manifest) -> bool {
        let mut all = true;
        for key in self.held_drop_keys(c) {
            match self.scatter_drop(m, &crate::scatter::Ask::DropKey(key)) {
                Ok(op) => {
                    self.scatter.jobs.insert(op, Job::Dropped);
                    if self.op_done(op) {
                        self.job_done(op);
                    }
                }
                Err(_) => all = false,
            }
        }
        all
    }

    /// Drops whose turn has come: a later one in line first asks one index of the generation,
    /// and drops it only if that piece is still kept.
    fn drops_due(&mut self) {
        let now = self.now();
        if !self.scatter.saved.due_drops.iter().any(|d| now >= d.at) {
            return;
        }
        let (due, later): (Vec<DueDrop>, Vec<DueDrop>) = std::mem::take(&mut self.scatter.saved.due_drops).into_iter().partition(|d| now >= d.at);
        self.scatter.saved.due_drops = later;
        self.scatter.dirty = true;
        for (i, d) in due.into_iter().enumerate() {
            let Some(m) = unb64(&d.manifest).ok().and_then(|b| Manifest::decode(&b).ok()) else { continue };
            if m.expires_at <= now || self.held_drop_keys(d.community).is_empty() {
                continue;
            }
            let again = DueDrop { at: now + APPLY_AGAIN, ..d.clone() };
            if i >= PER_TICK {
                self.scatter.saved.due_drops.push(DueDrop { at: now, ..d });
                continue;
            }
            if !d.check {
                if !self.drop_with_keys(d.community, &m) {
                    self.scatter.saved.due_drops.push(DueDrop { check: false, ..again });
                }
                continue;
            }
            let index = uniform(m.hashes.len().max(1)) as u8;
            match self.scatter_probe(&m, &[index], &BTreeMap::new()) {
                Ok(op) => {
                    self.scatter.jobs.insert(op, Job::DropCheck { community: d.community, manifest: d.manifest });
                    if self.op_done(op) {
                        self.job_done(op);
                    }
                }
                Err(_) => self.scatter.saved.due_drops.push(again),
            }
        }
    }

    /// Whether everything an object carries is gone here: each post deleted by its author or
    /// removed by the community, or no longer kept; a media item no live post uses.
    fn carried_gone(&self, k: &Kept) -> bool {
        if k.apply || (k.records.is_empty() && k.media.is_none()) {
            return false;
        }
        let gone = |id: &Id| !self.store.contains(id) || self.state.skeletons.contains_key(id);
        let records = k.records.iter().all(gone);
        // a media object is kept for the posts it was sent for: once they are gone it goes, even
        // if other posts use the same item (theirs is sent again)
        let media = k.media.is_none_or(|h| !k.records.is_empty() || !self.store.media_users(&h).any(|s| !gone(&s.id)));
        records && media
    }

    /// Whether a post the object carries was removed by the community (not only deleted).
    fn removed_by_community(&self, k: &Kept) -> bool {
        let Some(c) = k.community.and_then(|c| self.communities.get(&c)) else { return false };
        k.records.iter().any(|id| c.tombstones.contains_key(id))
    }

    /// Objects whose posts are all gone are let go: their pieces are dropped by this device when
    /// it made a generation of them (its own secret), or, when the community removed a post in
    /// them, by the committers' devices that hold the group's drop keys, in the line's order: the
    /// first at once, each next one after an hour more, and only if the pieces are still there.
    /// Nobody repairs them any more.
    fn let_go_of_gone(&mut self) {
        let gone: Vec<Id> =
            self.scatter.saved.kept.iter().filter(|(id, k)| !self.scatter.repair.busy.contains(id) && self.carried_gone(k)).map(|(id, _)| *id).collect();
        let now = self.now();
        for object in gone {
            let Some(k) = self.scatter.saved.kept.remove(&object) else { continue };
            self.scatter.dirty = true;
            // a media item other posts still use goes again for them, in an object of their own
            if let (Some(c), Some(h)) = (k.community, k.media)
                && self.is_private_community(&c)
                && self.store.media_users(&h).any(|s| self.store.contains(&s.id) && !self.state.skeletons.contains_key(&s.id))
            {
                self.private_resend_media(c, h);
            }
            let removed = self.removed_by_community(&k);
            let line = k.community.filter(|c| removed && !self.held_drop_keys(*c).is_empty()).and_then(|c| Some((c, self.drop_rank(c)?)));
            let secret = self.drop_secret();
            for g in &k.gens {
                let Some(m) = g.read() else { continue };
                if let Some((c, rank)) = line {
                    let at = now + rank as u64 * crate::private::TURN;
                    if rank > 0 || !self.drop_with_keys(c, &m) {
                        if self.scatter.saved.due_drops.len() >= MAX_DUE_DROPS {
                            self.scatter.saved.due_drops.remove(0);
                        }
                        self.scatter.saved.due_drops.push(DueDrop { community: c, manifest: g.manifest.clone(), at, check: rank > 0 });
                    }
                    continue;
                }
                let ask = if let Some(cap) = self.scatter.saved.drop_caps.get(&crate::drop_share::cap_id(&m)) {
                    crate::scatter::Ask::DropTokens(cap.tokens.clone())
                } else if let (None, Some(secret)) = (g.from, secret) {
                    crate::scatter::Ask::DropToken(secret)
                } else {
                    continue;
                };
                if self.drop_at_home(&m, &ask) {
                    continue;
                }
                if let Ok(op) = self.scatter_drop(&m, &ask) {
                    self.scatter.jobs.insert(op, Job::Dropped);
                    if self.op_done(op) {
                        self.job_done(op);
                    }
                }
            }
        }
    }

    /// Whether this device could keep something in pieces now: enough places, and forwarders
    /// in at least two places with room for the pieces.
    pub(crate) fn can_scatter(&self, c: Id) -> bool {
        self.scattering_home(c).is_some() || self.can_scatter_local(c)
    }

    pub(crate) fn can_scatter_local(&self, c: Id) -> bool {
        let plan = self.plan_in(Some(c));
        let level = self.community_level(Some(c));
        size::size(level, 0, plan.places() as u32).is_some_and(|s| plan.forwarder_places() >= 2 && plan.forwarder_places() * (s.k - 1) >= s.n)
    }

    /// The lowest level the newest generations of a community's objects meet, as their own
    /// sizes read (`size::reading`): `None` when none is kept here.
    pub(crate) fn pieces_level(&self, c: Id) -> Option<u8> {
        self.scatter
            .saved
            .kept
            .values()
            .filter(|k| k.community == Some(c))
            .filter_map(|k| k.newest())
            .filter_map(|m| size::reading(m.generation, m.places, size::Size { k: m.k as usize, n: m.n as usize, m: m.m as usize }))
            .min()
    }

    /// Objects this device keeps track of, and how many of them still wait to be fetched.
    pub fn kept_stats(&self) -> (usize, usize) {
        let k = &self.scatter.saved.kept;
        (k.len(), k.values().filter(|k| k.apply).count())
    }
}

/// A generation rebuilt here.
fn confirm(k: &mut Kept, generation: u8) {
    if let Some(g) = k.gens.iter_mut().find(|g| g.read().is_some_and(|m| m.generation == generation)) {
        g.confirmed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_is_lost_only_after_two_places_half_a_day_apart() {
        assert!(!lost_by(&[(0, 1)]));
        assert!(!lost_by(&[(0, 1), (LOSS_APART + 5, 1)]), "the same forwarder place twice");
        assert!(!lost_by(&[(0, 1), (LOSS_APART - 1, 2)]), "too close together");
        assert!(lost_by(&[(0, 1), (LOSS_APART, 2)]));
        assert!(lost_by(&[(LOSS_APART, 2), (0, 1)]), "in any order");
    }

    #[test]
    fn probes_thin_out_with_age() {
        assert_eq!(probe_every(0), PROBE_YOUNG);
        assert_eq!(probe_every(10 * DAY), PROBE_MONTH);
        assert_eq!(probe_every(90 * DAY), PROBE_OLD);
    }
}
