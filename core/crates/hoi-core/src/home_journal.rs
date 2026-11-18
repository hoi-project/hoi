//! Restart-safe public-object placement over a paired home link (proposed RFC-0008).
//! This is authority, not a cache: an unreadable journal must never silently start over.
use crate::{Node, api::R, drop_share::Capability, error::ApiError, home_scatter::Packet, scatter::ScatterOutcome};
use hoi_crypto::scatter as sc;
use hoi_types::{
    Id,
    scatter::Manifest,
    uri::{b64, unb64},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const BLOB: &str = "home-scatter-journal";
const MAX_ENTRIES: usize = 256;
const MAX_ORIGIN_BYTES: usize = 8 << 20;
const DAY: u64 = 86_400;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Origin {
    pub home: Id,
    pub community: Id,
    pub content: Vec<u8>,
    pub level: u8,
    pub expires: u64,
    pub records: Vec<Id>,
    pub released: bool,
    pub restored: bool,
    pub unknown: bool,
    pub result: Option<bool>,
    pub capability: Option<Capability>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Placement {
    pub request: Id,
    pub owner: Id,
    pub community: Id,
    pub level: u8,
    pub expires: u64,
    pub result: Option<bool>,
    pub capability: Option<Capability>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Journal {
    version: u32,
    pub origin: BTreeMap<Id, Origin>,
    pub placements: BTreeMap<Id, Placement>,
}
impl Default for Journal {
    fn default() -> Self {
        Self { version: 1, origin: BTreeMap::new(), placements: BTreeMap::new() }
    }
}
impl Journal {
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let j: Self = serde_json::from_slice(bytes).ok()?;
        if j.version != 1 || j.origin.len() + j.placements.len() > MAX_ENTRIES || j.origin.values().map(|r| r.content.len()).sum::<usize>() > MAX_ORIGIN_BYTES {
            return None;
        }
        for (id, r) in &j.origin {
            if (!r.released && !r.restored && request_id(r.community, &r.content, r.level) != *id)
                || (r.released && (!r.content.is_empty() || r.capability.is_some() || r.result.is_none()))
                || (r.restored && !r.content.is_empty())
                || (r.unknown && (!r.restored || !r.released || r.capability.is_some()))
                || r.records.len() > 256
                || (r.result == Some(true) && r.capability.is_none())
                || r.capability.as_ref().is_some_and(|c| {
                    c.read().is_none_or(|m| {
                        m.expires_at != r.expires
                            || m.level != r.level
                            || m.generation != 0
                            || (!r.restored
                                && sc::seal_object(&r.content, &m.object, &m.key)
                                    .ok()
                                    .is_none_or(|(class, sealed)| class != m.class || sc::sealed_hash(&sealed) != m.sealed))
                    })
                })
            {
                return None;
            }
        }
        for (key, p) in &j.placements {
            if placement_id(p.owner, p.request) != *key
                || p.capability.as_ref().is_some_and(|c| c.read().is_none_or(|m| m.expires_at != p.expires || m.level != p.level || m.generation != 0))
            {
                return None;
            }
        }
        Some(j)
    }
}
#[derive(Default)]
pub(crate) struct State {
    pub saved: Journal,
    pub dirty: bool,
    pub capturing: Option<Id>,
    pub operations: BTreeMap<u64, Id>,
    pub waits: BTreeMap<(String, u32), (u64, u64, Id)>,
    background: BTreeSet<u64>,
    release_at: u64,
    release_cursor: usize,
}
fn request_id(c: Id, content: &[u8], level: u8) -> Id {
    let mut h = blake3::Hasher::new_derive_key("hoi home placement request v1");
    h.update(c.as_bytes());
    h.update(&[level]);
    h.update(content);
    Id(*h.finalize().as_bytes())
}
fn placement_id(owner: Id, request: Id) -> Id {
    let mut h = blake3::Hasher::new_derive_key("hoi home placement owner v1");
    h.update(owner.as_bytes());
    h.update(request.as_bytes());
    Id(*h.finalize().as_bytes())
}
fn live(expires: u64, now: u64) -> bool {
    now / DAY <= expires.div_ceil(DAY)
}
impl Node {
    pub(crate) fn journal_home(&self) -> Option<Id> {
        self.state.home.as_ref().map(|h| Id::hash(h.token.as_bytes()))
    }
    pub(crate) fn journal_home_peer(&self) -> Option<String> {
        self.peers.iter().find(|(_, p)| p.home && p.said_hello && p.features.contains(hoi_sync::FEATURE_HOME_SCATTER_JOURNAL)).map(|(peer, _)| peer.clone())
    }
    pub(crate) fn journal_store(&mut self, home: &str, c: Id, content: &[u8], level: u8, expires: u64) -> R<u64> {
        let home_id = self.journal_home().ok_or_else(|| ApiError::invalid("Home unavailable."))?;
        let request = request_id(c, content, level);
        self.check_room(if self.home_journal.saved.origin.contains_key(&request) { self.now() + 60 } else { expires })?;
        if !sc::size::LEVELS.contains(&level) || content.len() > hoi_sync::MAX_REBUILT {
            return Err(ApiError::invalid("Invalid home placement."));
        }
        if !self.home_journal.saved.origin.contains_key(&request) {
            if home.is_empty() {
                return Err(ApiError::invalid("Home unavailable."));
            }
            let bytes: usize = self.home_journal.saved.origin.values().map(|r| r.content.len()).sum();
            if self.home_journal.saved.origin.len() + self.home_journal.saved.placements.len() >= MAX_ENTRIES
                || bytes.saturating_add(content.len()) > MAX_ORIGIN_BYTES
                || self.scatter.saved.drop_caps.len() >= 4096
            {
                return Err(ApiError::new("busy", "Recoverable home placement storage is full."));
            }
            self.home_journal.saved.origin.insert(
                request,
                Origin {
                    home: home_id,
                    community: c,
                    content: content.to_vec(),
                    level,
                    expires,
                    records: vec![],
                    released: false,
                    restored: false,
                    unknown: false,
                    result: None,
                    capability: None,
                },
            );
            self.home_journal.dirty = true;
        }
        let r = self.home_journal.saved.origin.get(&request).expect("origin inserted").clone();
        if r.released || r.restored {
            return Err(ApiError::invalid("This placement was deleted."));
        }
        if r.home != home_id {
            return Err(ApiError::invalid("This placement still belongs to your previous home."));
        }
        let op = self.scatter_wait_op();
        self.home_journal.operations.insert(op, request);
        if r.result.is_some() {
            self.complete_home_origin(op, &r);
        } else {
            self.send_journal_request(home, op, request);
        }
        Ok(op)
    }
    pub(crate) fn attach_home_records(&mut self, op: u64, records: &[Id]) {
        if let Some(r) = self.home_journal.operations.get(&op).and_then(|id| self.home_journal.saved.origin.get_mut(id)) {
            r.records = records.to_vec();
            self.home_journal.dirty = true;
        }
    }
    pub(crate) fn journal_deletion_pending(&self, target: Id) -> bool {
        self.home_journal.saved.origin.values().any(|r| r.result.is_none() && live(r.expires, self.now()) && r.records.contains(&target))
    }
    fn send_journal_request(&mut self, home: &str, op: u64, request: Id) {
        let Some(r) = self.home_journal.saved.origin.get(&request).cloned() else { return };
        let rid = self.next_rid();
        self.home_journal.waits.insert((home.into(), rid), (self.now(), op, request));
        if home.is_empty() || r.restored || Some(r.home) != self.journal_home() || self.deletion_restore_quarantined(Some(r.community)) {
            return;
        }
        self.home_packet(
            home,
            rid,
            false,
            Packet::JournalStore { request, community: r.community, content: b64(&r.content), level: r.level, expires: r.expires },
        );
    }
    fn complete_home_origin(&mut self, op: u64, r: &Origin) {
        let manifest = r.capability.as_ref().and_then(Capability::read);
        if let Some(cap) = r.capability.clone() {
            self.recover_home_cap(r.community, &r.records, cap);
        }
        self.end_home_wait_op(
            op,
            manifest.clone(),
            if r.result == Some(true) && manifest.as_ref().is_some_and(|m| m.expires_at > self.now()) {
                manifest.map(ScatterOutcome::Stored).unwrap_or_else(|| ScatterOutcome::Failed("Invalid home placement result.".into()))
            } else {
                ScatterOutcome::Failed("Your home could not finish placement; delivery will use the whole post.".into())
            },
        );
    }
    pub(crate) fn journal_packet(&mut self, peer: &str, rid: u32, reply: bool, packet: &Packet) -> bool {
        match packet {
            Packet::JournalStore { request, community, content, level, expires } if !reply => {
                let Some(owner) = self.peers.get(peer).and_then(|p| p.client_credential) else { return true };
                if !self.peers.get(peer).is_some_and(|p| p.features.contains(hoi_sync::FEATURE_HOME_SCATTER_JOURNAL)) {
                    return true;
                }
                if self.deletion_restore_quarantined(Some(*community)) {
                    self.home_packet(peer, rid, true, Packet::JournalResult { request: *request, stored: None, capability: None });
                    return true;
                }
                let key = placement_id(owner, *request);
                let Some(content) = unb64(content).ok().filter(|c| c.len() <= hoi_sync::MAX_REBUILT && request_id(*community, c, *level) == *request) else {
                    return true;
                };
                if let Some(p) = self.home_journal.saved.placements.get(&key) {
                    if p.community != *community || p.level != *level || p.expires != *expires {
                        return true;
                    }
                    let running = self.home_journal.operations.iter().any(|(op, k)| *k == key && self.scatter.jobs.contains_key(op));
                    if p.result.is_none() && !running {
                        // Unknown in-flight puts are never repeated after a home restart.
                        self.home_journal.saved.placements.get_mut(&key).expect("placement exists").result = Some(false);
                        self.home_journal.dirty = true;
                    }
                    self.reply_journal(peer, rid, key);
                    return true;
                }
                if self.home_journal.saved.origin.len() + self.home_journal.saved.placements.len() >= MAX_ENTRIES
                    || self.is_private_community(community)
                    || !self.peers.get(peer).is_some_and(|p| p.communities.contains(community))
                    || self.check_room(*expires).is_err()
                    || !sc::size::LEVELS.contains(level)
                    || self.drop_secret().is_none()
                    || self.home_journal.saved.placements.values().filter(|p| p.owner == owner && p.result.is_none()).count() >= 16
                {
                    self.home_packet(peer, rid, true, Packet::JournalResult { request: *request, stored: Some(false), capability: None });
                    return true;
                }
                self.home_journal.saved.placements.insert(
                    key,
                    Placement { request: *request, owner, community: *community, level: *level, expires: *expires, result: None, capability: None },
                );
                self.home_journal.dirty = true;
                self.home_journal.capturing = Some(key);
                self.scatter.placing = Some(*community);
                self.scatter.next_drop_key = self.community_drop_key(*community).map(|k| sc::drop_key_id(&k));
                let result = self.scatter_store_inner(&content, *level, *expires);
                self.scatter.placing = None;
                self.scatter.next_drop_key = None;
                self.home_journal.capturing = None;
                if let Ok(op) = result {
                    self.home_journal.operations.insert(op, key);
                    self.scatter.jobs.insert(op, crate::keep::Job::StoreJournalled { key, peer: peer.into(), rid });
                    if self.op_done(op) {
                        self.job_done(op);
                    }
                } else {
                    self.home_journal.saved.placements.get_mut(&key).expect("placement inserted").result = Some(false);
                    self.reply_journal(peer, rid, key);
                }
                true
            }
            Packet::JournalResult { request, stored, capability } if reply => {
                let Some((_, op, wanted)) = self.home_journal.waits.get(&(peer.into(), rid)).copied() else { return true };
                if wanted != *request || stored.is_none() {
                    return true;
                }
                let Some(mut r) = self.home_journal.saved.origin.get(request).cloned() else { return true };
                if self.deletion_restore_quarantined(Some(r.community)) {
                    return true;
                }
                if Some(r.home) != self.journal_home() {
                    return true;
                }
                let valid = capability
                    .as_ref()
                    .map(|cap| {
                        cap.read().is_some_and(|m| {
                            sc::seal_object(&r.content, &m.object, &m.key).ok().is_some_and(|(class, sealed)| {
                                m.generation == 0 && m.level == r.level && m.expires_at == r.expires && m.class == class && m.sealed == sc::sealed_hash(&sealed)
                            })
                        })
                    })
                    .unwrap_or(*stored == Some(false));
                if !valid || (capability.is_some() && self.scatter.saved.drop_caps.len() >= 4096) {
                    return true;
                }
                r.result = *stored;
                r.capability = capability.clone();
                self.home_journal.saved.origin.insert(*request, r.clone());
                self.home_journal.dirty = true;
                self.home_journal.waits.remove(&(peer.into(), rid));
                self.complete_home_origin(op, &r);
                if self.home_journal.background.remove(&op) {
                    self.drop_op(op);
                    self.home_journal.operations.remove(&op);
                }
                true
            }
            Packet::JournalRelease { request } if !reply => {
                if let Some(owner) = self.peers.get(peer).and_then(|p| p.client_credential) {
                    let key = placement_id(owner, *request);
                    if let Some(p) = self.home_journal.saved.placements.get_mut(&key).filter(|p| p.result.is_some()) {
                        p.capability = None;
                        p.result = Some(false);
                        self.home_journal.dirty = true;
                    }
                }
                true
            }
            Packet::JournalStore { .. } | Packet::JournalResult { .. } | Packet::JournalRelease { .. } => true,
            _ => false,
        }
    }
    fn reply_journal(&mut self, peer: &str, rid: u32, key: Id) {
        if let Some(p) = self.home_journal.saved.placements.get(&key).cloned() {
            self.home_packet(peer, rid, true, Packet::JournalResult { request: p.request, stored: p.result, capability: p.capability });
        }
    }
    /// Called before every PUT, including replacement indexes. Egress waits for this journal.
    pub(crate) fn journal_before_put(&mut self, op: u64) -> bool {
        let Some(key) = self.home_journal.capturing.or_else(|| self.home_journal.operations.get(&op).copied()) else { return true };
        if !self.home_journal.saved.placements.contains_key(&key) {
            return true;
        }
        let Some(cap) = self.scatter_placement(op).and_then(|m| self.capability_for(&m)).filter(|c| c.read().is_some()) else { return false };
        self.home_journal.saved.placements.get_mut(&key).expect("placement exists").capability = Some(cap);
        self.home_journal.dirty = true;
        true
    }
    pub(crate) fn journal_stored(&mut self, key: Id, peer: &str, rid: u32, outcome: ScatterOutcome, placement: Option<Manifest>) {
        let capability = placement.as_ref().and_then(|m| self.capability_for(m));
        if let Some(p) = self.home_journal.saved.placements.get_mut(&key) {
            p.result = Some(matches!(outcome, ScatterOutcome::Stored(_)));
            if capability.is_some() {
                p.capability = capability;
            }
            self.home_journal.dirty = true;
        }
        self.reply_journal(peer, rid, key);
    }
    /// A member archive is a stale snapshot, never permission to reconnect another operator
    /// or serve that operator's clients. Known capabilities remain deletion inventory;
    /// missing results become explicit unknown coverage rather than fresh placements.
    pub(crate) fn journal_restore_snapshot(&mut self) {
        self.invalidate_home_delivery_status();
        self.journal_recover_authority();
        self.home_journal.saved.placements.clear();
        // An archive is a new installation. Retain locator authority, but never restore
        // delegation to an archived operator, even if its old pairing token is reused later.
        for target in self.deletions.saved.jobs.values_mut().flat_map(|j| j.targets.values_mut()) {
            if target.home.take().is_some() {
                self.deletions.dirty = true;
            }
        }
        let mut unknown_records = Vec::new();
        for r in self.home_journal.saved.origin.values_mut() {
            r.restored = true;
            r.content.clear();
            if r.result.is_none() {
                unknown_records.extend(r.records.iter().copied());
                r.unknown = true;
                r.result = Some(false);
                r.released = true;
            }
        }
        for record in unknown_records {
            self.scatter.saved.home_delivery.insert(record, "unknown".into());
            self.scatter.dirty = true;
        }
        self.home_journal.operations.clear();
        self.home_journal.waits.clear();
        self.home_journal.background.clear();
        self.home_journal.dirty = true;
    }
    pub(crate) fn journal_recover_authority(&mut self) {
        let known: Vec<_> = self
            .home_journal
            .saved
            .origin
            .values()
            .filter_map(|r| r.capability.clone().and_then(|cap| Some((r.community, r.records.clone(), cap.read()?, cap))))
            .collect();
        for (c, records, _, cap) in known {
            self.recover_home_cap(c, &records, cap);
        }
    }
    fn recover_home_cap(&mut self, c: Id, records: &[Id], cap: Capability) {
        let Some(original) = cap.read() else { return };
        self.save_drop_cap(cap);
        let m = self.scatter.saved.drop_caps.get(&crate::drop_share::cap_id(&original)).and_then(Capability::read).unwrap_or(original);
        let deleted = self.deletion_object_suppressed(Id(m.object)) || (!records.is_empty() && records.iter().all(|r| self.record_deleted(r)));
        if deleted {
            let kept = crate::keep::Kept { community: Some(c), records: records.to_vec(), ..Default::default() };
            if matches!(self.queue_deletion_generation(&kept, &m, false), Ok(true)) {
                return;
            }
        }
        self.remember_outgoing_placement(c, records, None, m);
    }
    pub(crate) fn journal_capability(&self, m: &Manifest) -> Option<Capability> {
        self.home_journal.saved.origin.values().filter_map(|r| r.capability.as_ref()).find(|cap| cap.read().is_some_and(|old| old == *m)).cloned()
    }
    pub(crate) fn journal_unknown_until(&self, target: Id) -> u64 {
        self.home_journal
            .saved
            .origin
            .values()
            .filter(|r| r.unknown && r.records.contains(&target))
            .map(|r| (r.expires.div_ceil(DAY) + 1).saturating_mul(DAY))
            .max()
            .unwrap_or(0)
    }
    /// The host can replace the connection named `home` after re-pairing. Never let an
    /// already queued old request leak its body to the newly selected operator.
    pub(crate) fn filter_home_journal_outbox(&mut self) {
        let current = self.journal_home();
        let scopes = self.home_request_scopes();
        self.outbox.retain(|(peer, bytes)| {
            let rid = match hoi_sync::Msg::decode(bytes) {
                Ok(hoi_sync::Msg::Piece { rid, .. } | hoi_sync::Msg::HomeScatter { rid, .. }) => rid,
                _ => return true,
            };
            scopes.get(&(peer.clone(), rid)).is_none_or(|scope| Some(*scope) == current)
        });
        let allowed: std::collections::BTreeSet<_> = self
            .home_journal
            .saved
            .origin
            .iter()
            .filter(|(_, r)| Some(r.home) == current && !r.restored && !self.deletion_restore_quarantined(Some(r.community)))
            .map(|(id, _)| *id)
            .collect();
        let origin = &self.home_journal.saved.origin;
        self.outbox.retain(|(_, bytes)| match hoi_sync::Msg::decode(bytes) {
            Ok(hoi_sync::Msg::HomeScatter { reply: false, payload, .. }) => match serde_json::from_slice::<Packet>(&payload) {
                Ok(Packet::JournalStore { request, .. }) => {
                    allowed.contains(&request) && origin.get(&request).is_some_and(|r| r.result.is_none() && !r.released)
                }
                Ok(Packet::JournalRelease { request }) => allowed.contains(&request) && origin.get(&request).is_some_and(|r| r.released),
                _ => true,
            },
            _ => true,
        });
    }
    pub(crate) fn journal_operation_finished(&mut self, op: u64) {
        self.home_journal.operations.remove(&op);
        self.home_journal.waits.retain(|_, (_, pending, _)| *pending != op);
        self.home_journal.background.remove(&op);
    }
    pub(crate) fn journal_has_content(&self, c: Id, content: &[u8], level: u8) -> bool {
        self.home_journal.saved.origin.contains_key(&request_id(c, content, level))
    }
    pub(crate) fn journal_community_work(&self, c: Id) -> bool {
        self.home_journal.saved.origin.values().any(|r| r.community == c && !r.released && !r.restored && live(r.expires, self.now()))
    }
    /// The deletion journal has atomically captured these locator-level tokens. Retain only
    /// the request tombstone; do not keep deleted record bodies or decryption keys.
    pub(crate) fn journal_release_empty_target(&mut self, target: Id) {
        for r in self.home_journal.saved.origin.values_mut().filter(|r| r.records.contains(&target) && r.result.is_some() && r.capability.is_none()) {
            r.content.clear();
            r.released = true;
            r.result = Some(false);
            self.home_journal.dirty = true;
        }
    }
    pub(crate) fn journal_locator_home(&self, m: &Manifest, index: usize) -> Option<Id> {
        self.home_journal.saved.origin.values().find_map(|r| {
            (!r.restored
                && r.capability.as_ref().is_some_and(|c| {
                    c.read().is_some_and(|old| {
                        index < old.holders.len()
                            && old.holders.get(index) == m.holders.get(index)
                            && (old == *m || matches!(old.update_to(m), Ok(hoi_types::scatter::Update::Extend)))
                    })
                }))
            .then_some(r.home)
        })
    }
    pub(crate) fn journal_release_generation(&mut self, m: &Manifest) {
        for r in self.home_journal.saved.origin.values_mut().filter(|r| {
            r.capability.as_ref().is_some_and(|c| c.read().is_some_and(|old| old == *m || matches!(old.update_to(m), Ok(hoi_types::scatter::Update::Extend))))
        }) {
            r.capability = None;
            r.content.clear();
            r.released = true;
            r.result = Some(false);
            self.home_journal.dirty = true;
        }
    }
    pub(crate) fn journal_tick(&mut self) {
        let now = self.now();
        let before = self.home_journal.saved.origin.len() + self.home_journal.saved.placements.len();
        self.home_journal.saved.origin.retain(|_, r| live(r.expires, now));
        self.home_journal.saved.placements.retain(|_, r| live(r.expires, now));
        if before != self.home_journal.saved.origin.len() + self.home_journal.saved.placements.len() {
            self.home_journal.dirty = true;
        }
        let orphaned: Vec<_> = self
            .home_journal
            .saved
            .origin
            .iter()
            .filter(|(id, r)| !r.released && r.result.is_none() && !self.home_journal.operations.values().any(|r| r == *id))
            .map(|(id, _)| *id)
            .take(4)
            .collect();
        for request in orphaned {
            if self.check_room(now + 60).is_err() {
                continue;
            }
            let op = self.scatter_wait_op();
            self.home_journal.operations.insert(op, request);
            self.home_journal.background.insert(op);
            let home = self.journal_home_peer().unwrap_or_default();
            self.send_journal_request(&home, op, request);
        }
        if now >= self.home_journal.release_at {
            self.home_journal.release_at = now + 30;
            let releases: Vec<_> =
                self.home_journal.saved.origin.iter().filter(|(_, r)| r.released && Some(r.home) == self.journal_home()).map(|(id, _)| *id).collect();
            let count = releases.len().min(8);
            let cursor = self.home_journal.release_cursor;
            self.home_journal.release_cursor = cursor.saturating_add(count);
            for request in (0..count).map(|i| releases[(cursor + i) % releases.len()]) {
                if let Some(home) = self.journal_home_peer() {
                    let rid = self.next_rid();
                    self.home_packet(&home, rid, false, Packet::JournalRelease { request });
                }
            }
        }
        let retries: Vec<_> = self
            .home_journal
            .waits
            .iter()
            .filter(|((p, _), (at, _, _))| now >= at + 30 || !self.peers.contains_key(p))
            .map(|(k, (_, op, r))| (k.clone(), *op, *r))
            .collect();
        for (key, op, request) in retries {
            self.home_journal.waits.remove(&key);
            let Some(_) = self.home_journal.saved.origin.get(&request) else {
                self.end_wait_op(op, ScatterOutcome::Failed("The original piece storage interval has ended.".into()));
                continue;
            };
            if let Some(home) = self.journal_home_peer() {
                self.send_journal_request(&home, op, request);
            } else {
                self.home_journal.waits.insert(key, (now, op, request));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_identity_and_packets_match_the_protocol_vectors() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../../vectors/home-scatter.json")).unwrap();
        let c = Id::from_hex(v["community"].as_str().unwrap()).unwrap();
        let owner = Id::from_hex(v["owner"].as_str().unwrap()).unwrap();
        let content = hoi_types::hex_decode(v["content_hex"].as_str().unwrap()).unwrap();
        let request = request_id(c, &content, v["level"].as_u64().unwrap() as u8);
        assert_eq!(request.hex(), v["request"]);
        assert_eq!(placement_id(owner, request).hex(), v["owned_request"]);
        assert_ne!(request_id(c, b"changed", 10), request);
        assert_ne!(request_id(Id::ZERO, &content, 10), request);
        assert_ne!(request_id(c, &content, 15), request);
        assert_ne!(placement_id(Id::ZERO, request), placement_id(owner, request));
        for name in ["store", "pending", "fallback", "release", "cleanup_keys", "cleanup_key_list"] {
            let packet: Packet = serde_json::from_value(v[name].clone()).unwrap();
            assert_eq!(serde_json::to_value(packet).unwrap(), v[name]);
        }
        assert!(hoi_sync::FEATURE_HOME_SCATTER_JOURNAL.len() <= 16);
    }
}
