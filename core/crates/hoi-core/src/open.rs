//! Open communities in pieces (SCATTER.md S5).
//!
//! In an open community whose limits say `scatter.open`, a post, comment or edit is kept in
//! pieces on holders, as a private group's are, and what the community syncs in its place is a
//! small signed **pointer record**: the record it holds by id, its kind, and the pointer
//! (`scatter::Pointer`, the sealed manifest) sealed under the community's read key
//!
//! ```text
//! read key = BLAKE3-derive-key("hoi read v1", community key)
//! ```
//!
//! so nodes that keep pieces but never joined the community cannot open it. Every device that
//! joined with a person's identity fetches each pointer's object and rebuilds the record on its
//! own device, where it stays: a record rebuilt from pieces is never offered whole to anyone, and
//! nor is a device's own while it is kept in pieces. Nodes that only carry the community keep and
//! pass on pointer records, and take votes, replies, edits and removals of what a pointer holds
//! by its id, as if the record itself were there.
//!
//! A post's media items go in pieces as objects of their own, each named by a pointer record
//! with the post's id and the item's hash, one set per post: a removed post's media is dropped
//! even if other posts use the same bytes, since theirs are objects of their own.
//!
//! Updates (repairs, new generations) come as further pointer records for the same object. They
//! count only from the pointer's first author or an established member of the community, and
//! must name what the first named.
//!
//! **The community's drop key** is set by owners in the ledger (`drop-key`): its public part,
//! and its secret sealed (HPKE) to the seal key each owner's and moderator's device carries in
//! its delegation. Owners' devices make one when there is none or when a device it was sealed to
//! left the line (a moderator removed, banned, or a device revoked), and seal the same key again
//! when a device in line lacks it; one after another, in the line's order and turns:
//! owners' devices first, then moderators', by how long their person has
//! held the role. Devices keep replaced keys while pieces stored with them may live.

use std::collections::BTreeSet;

use hoi_moderation::Level;
use hoi_types::scatter::Pointer;
use hoi_types::{Body, DropKeyGrant, Id, Kind, LedgerAction, Record};
use zeroize::Zeroize;

use crate::api::R;
use crate::error::ApiError;
use crate::keep::OutPart;
use crate::{Node, Source};

/// What a pointer record is sealed for.
const CONTEXT: &str = "hoi open pointer v1";
/// Parts one round starts at most.
const PER_ROUND: usize = 16;
const CHUNK: usize = 256 * 1024;
/// How often a device in line the drop key was not sealed to makes its blessing known again.
const ANNOUNCE_EVERY: u64 = 3600;
/// Pieces live at most this long (SCATTER.md §5.11).
const MAX_LIFE: u64 = 400 * 86_400;

/// An open community's read key: anyone who knows the community can derive it, holders that
/// never heard of it cannot.
pub(crate) fn read_key(c: &Id) -> [u8; 32] {
    blake3::derive_key("hoi read v1", c.as_bytes())
}

/// A pointer sealed for a pointer record of community `c`.
pub(crate) fn seal_pointer(c: &Id, p: &Pointer) -> Option<Vec<u8>> {
    let mut key = read_key(c);
    let sealed = hoi_crypto::sealed::seal(CONTEXT, &key, &p.encode()).ok();
    key.zeroize();
    sealed
}

/// The pointer a pointer record of community `c` carries.
pub(crate) fn open_pointer(c: &Id, sealed: &[u8]) -> Option<Pointer> {
    let mut key = read_key(c);
    let plain = hoi_crypto::sealed::open(CONTEXT, &key, sealed).ok();
    key.zeroize();
    Pointer::decode(&plain?).ok()
}

/// What a pointer record says it holds, for checking what refers to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Standin {
    pub kind: Kind,
    pub community: Id,
    pub root: Id,
}

impl Node {
    /// Whether community `c` keeps posts in pieces, by the limits of time `t`: open (not
    /// private), with `scatter.open`.
    pub(crate) fn open_scattered_at(&self, c: Id, t: u64) -> bool {
        !self.is_private_community(&c) && self.communities.get(&c).is_some_and(|st| st.limits_at(t).scatter.open)
    }

    /// Whether a record is kept in pieces here and so never offered whole: rebuilt from pieces,
    /// or this device's own in a community that keeps them so, until it went whole.
    pub(crate) fn kept_in_pieces(&self, s: &hoi_store::Stored) -> bool {
        if !matches!(s.record.kind(), Kind::Post | Kind::Comment | Kind::Edit) {
            return false;
        }
        let Some(c) = s.record.community else { return false };
        if self.scatter.in_pieces.contains(&s.id) {
            return true;
        }
        self.device_kp.as_ref().is_some_and(|d| d.public() == s.record.author)
            && !self.scatter.saved.whole.contains(&s.id)
            && self.open_scattered_at(c, s.record.created_at)
    }

    /// The devices that hold an open community's drop key and drop its removed posts, first in
    /// line first: owners' and moderators' devices whose blessing covers moderation, by how long
    /// their person has held the role (the owners since the genesis, a moderator since the
    /// `mod-add` that made them one), then by the blessing's age. Banned, burned and revoked ones
    /// are left out.
    pub(crate) fn open_line(&self, c: Id) -> Vec<(Id, Id)> {
        let Some(st) = self.communities.get(&c) else { return vec![] };
        let now = self.now();
        let required = [format!("moderate:{}", c.hex())];
        let mut roles: Vec<(u64, Id)> = st.genesis.owners.iter().map(|o| (st.created_at, *o)).collect();
        roles.extend(st.mods.iter().map(|(m, at)| (*at, *m)));
        let mut line = Vec::new();
        for (since, root) in roles {
            if st.is_banned(&root) || self.store.burned_at(&root).is_some() {
                continue;
            }
            for (device, d) in self.store.delegations_of(&root) {
                if d.info.revoked_at.is_some() || d.info.expires_at <= now || !hoi_validate::scope_covers(&d.info.scope, &required) {
                    continue;
                }
                let made = self.store.get(&d.record).map(|s| s.record.created_at).unwrap_or(0);
                line.push(((since, root, made, device), (device, root)));
            }
        }
        line.sort_by_key(|x| x.0);
        line.dedup_by_key(|x| x.1.0);
        line.into_iter().map(|(_, x)| x).collect()
    }

    /// One round for every open community that keeps posts in pieces: this device's own posts,
    /// comments and edits that went neither in pieces nor whole yet go now, with their posts'
    /// media; and its owners' devices keep the drop key in step (crate::keep holds the keys).
    pub(crate) fn open_tick(&mut self) {
        let Some(me) = self.device_kp.as_ref().map(|k| k.public()) else { return };
        let now = self.now();
        let communities: Vec<Id> = self.state.joined.keys().copied().filter(|c| self.open_scattered_at(*c, now)).collect();
        for c in communities {
            self.take_drop_keys(c);
            self.announce_in_line(c, me);
            self.tend_open_drop_key(c);
            self.send_own(c, me);
        }
        let live: BTreeSet<Id> = self.scatter.saved.whole.iter().filter(|id| self.store.contains(id)).copied().collect();
        if live.len() != self.scatter.saved.whole.len() {
            self.scatter.saved.whole = live;
            self.scatter.dirty = true;
        }
        self.prune_inject_marks();
    }

    /// A moderator's device the listed drop key is not sealed to makes its blessing known (it
    /// carries the seal key), now and then: owners' devices may never have seen this device,
    /// which may not have posted anything yet.
    fn announce_in_line(&mut self, c: Id, me: Id) {
        let now = self.now();
        let sealed_to_me = self.communities.get(&c).and_then(|st| st.drop_key()).is_some_and(|d| d.grant.sealed.iter().any(|(x, _)| *x == me));
        if sealed_to_me || self.drop_rank(c).is_none() || self.scatter.announced.get(&c).is_some_and(|t| now < t + ANNOUNCE_EVERY) {
            return;
        }
        self.scatter.announced.insert(c, now);
        // through an injector of this community where one is linked, which knows the people in
        // its line; otherwise first hand, as before
        if let Some(d) = self.store.delegation(&me).map(|d| d.record)
            && !self.inject_in(d, c)
        {
            self.gossip_to_all(&d);
        }
    }

    fn send_own(&mut self, c: Id, me: Id) {
        let now = self.now();
        let busy = self.scatter.repair.inflight_records.get(&c).cloned().unwrap_or_default();
        let busy_media = self.scatter.repair.inflight_media.get(&c).cloned().unwrap_or_default();
        let mut parts: Vec<OutPart> = Vec::new();
        let mut records = Vec::new();
        let mut media = Vec::new();
        let mut expires_at = 0;
        let own: Vec<(Id, Vec<u8>, u64, Vec<Id>)> = self
            .store
            .in_community(&c)
            .filter(|s| s.record.author == me && s.record.expires_at > now && self.kept_in_pieces(s))
            .filter(|s| !self.scatter.in_pieces.contains(&s.id) && !busy.contains(&s.id) && !self.state.skeletons.contains_key(&s.id))
            .map(|s| {
                let items = match &s.record.body {
                    Body::Post { media, .. } => media.iter().map(|m| m.hash).collect(),
                    _ => vec![],
                };
                (s.id, s.wire.clone(), s.record.expires_at, items)
            })
            .take(PER_ROUND)
            .collect();
        for (id, wire, expires, items) in own {
            parts.push(OutPart { content: crate::private::records_content(c, &wire), records: vec![id], media: None });
            records.push(id);
            expires_at = expires_at.max(expires);
            for hash in items {
                if busy_media.contains(&hash) {
                    continue;
                }
                let Some(bytes) = self.store.blob(&hash) else { continue };
                let total = bytes.len().div_ceil(CHUNK);
                for content in crate::private::media_contents(c, hash, bytes, &[id]) {
                    parts.push(OutPart { content, records: vec![id], media: Some(hash) });
                }
                media.push((hash, total));
            }
        }
        if parts.is_empty() {
            return;
        }
        let expires_at = expires_at.min(now + MAX_LIFE);
        if !self.can_scatter(c) || !self.scatter_outgoing(c, parts, records.clone(), media, expires_at) {
            self.open_whole(c, records);
        }
    }

    /// Records that could not be kept in pieces go whole, as they always did.
    pub(crate) fn open_whole(&mut self, c: Id, records: Vec<Id>) {
        for id in records {
            if self.state.home.is_some() {
                self.scatter.saved.home_delivery.insert(id, "pending".into());
            }
            if self.scatter.saved.whole.insert(id) {
                self.scatter.dirty = true;
            }
            self.forget_inflight_open(c, id);
            // whole now, but still through an injector where one is linked (S7)
            if !self.inject_own(id) {
                self.gossip(&id, None);
            }
        }
    }

    fn forget_inflight_open(&mut self, c: Id, id: Id) {
        if let Some(set) = self.scatter.repair.inflight_records.get_mut(&c) {
            set.remove(&id);
        }
    }

    /// Pointer records for objects this device stored or repaired: each names what its object
    /// holds, as the object registry knows it.
    pub(crate) fn open_pointers(&mut self, c: Id, pointers: Vec<Pointer>) {
        let now = self.now();
        let Some(device) = self.device_kp.as_ref().map(|k| k.public()) else { return };
        for pointer in pointers {
            let object = Id(pointer.manifest.object);
            let Some((record, media)) = self.scatter.saved.kept.get(&object).and_then(|k| k.claim) else { continue };
            let of = if media.is_some() { Kind::Post } else { self.store.get(&record).map(|s| s.record.kind()).unwrap_or(Kind::Post) };
            let Some(sealed) = seal_pointer(&c, &pointer) else { continue };
            let r = Record {
                v: 1,
                community: Some(c),
                author: device,
                parent: None,
                created_at: now,
                expires_at: pointer.manifest.expires_at.max(now + 1),
                body: Body::Pointer { record, of, media, sealed },
                nonce: 0,
            };
            if let Ok(pointer) = self.publish(r) {
                self.scatter.saved.own_pointers.entry(record).or_default().insert(pointer);
                if self.scattering_home(c).is_some() {
                    self.scatter.saved.home_delivery.insert(pointer, "pending".into());
                }
                self.scatter.dirty = true;
            }
        }
        for id in self.scatter.repair.inflight_records.get(&c).cloned().unwrap_or_default() {
            if self.scatter.in_pieces.contains(&id) {
                self.forget_inflight_open(c, id);
            }
        }
    }

    /// A pointer record taken in, on any node: what it holds counts for what refers to it; a
    /// device with a person's identity fetches its object, if it may.
    pub(crate) fn on_pointer_record(&mut self, r: &Record, root: Id) {
        let (Some(c), Body::Pointer { record, of, media, sealed }) = (r.community, &r.body) else { return };
        if matches!(of, Kind::Post | Kind::Comment) {
            self.scatter.standins.entry(*record).or_insert(Standin { kind: *of, community: c, root });
        }
        if self.loading || self.state.me.is_none() || self.device_kp.as_ref().is_some_and(|d| d.public() == r.author) {
            return;
        }
        let Some(pointer) = open_pointer(&c, sealed) else { return };
        let object = Id(pointer.manifest.object);
        let claim = (*record, *media);
        if let Some(k) = self.scatter.saved.kept.get(&object) {
            // an update: what the first named, from its author or an established member
            if k.claim != Some(claim) {
                return;
            }
            let established = self.communities.get(&c).is_some_and(|st| st.level(&root) >= Level::Established);
            if k.author.is_some_and(|a| a != root) && !established {
                return;
            }
        }
        if let Ok(true) = self.take_pointer(Some(c), pointer, Some(r.author), true)
            && let Some(k) = self.scatter.saved.kept.get_mut(&object)
        {
            k.open = true;
            k.claim = Some(claim);
            k.author = Some(root);
            k.records = vec![*record];
            k.media = *media;
            self.scatter.in_pieces.insert(*record);
        }
    }

    /// An open community's object rebuilt here: the record its pointer named, taken in as
    /// rebuilt on this device, or a part of the media item it named.
    pub(crate) fn open_apply_content(&mut self, c: Id, object: Id, content: &[u8]) -> R<()> {
        let Some((record, media)) = self.scatter.saved.kept.get(&object).and_then(|k| k.claim) else { return Err(ApiError::not_found("object")) };
        let author = self.scatter.saved.kept.get(&object).and_then(|k| k.author);
        let wrong = || ApiError::new("scatter-update", "This object holds something else than its pointer said.");
        for item in crate::private::content_items(c, content)? {
            match item {
                crate::private::Item::Records(wires) => {
                    for wire in wires {
                        let d = hoi_validate::decode(&wire)?;
                        let by = self.store.root_of(&d.wire.record.author);
                        if d.id != record || media.is_some() || author.is_some_and(|a| a != by) {
                            self.scatter.saved.kept.remove(&object);
                            return Err(wrong());
                        }
                        match self.ingest(&wire, &Source::Pieces) {
                            Ok(_) | Err(hoi_validate::Reject::Duplicate) => {}
                            Err(hoi_validate::Reject::NotAllowed(why)) if why.contains("we don't have") || why.contains("not subscribed") => {
                                return Err(ApiError::new("private-pending", "What this is about is still on its way."));
                            }
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
                crate::private::Item::Media { hash, total, offset, bytes } => {
                    if media != Some(hash) {
                        self.scatter.saved.kept.remove(&object);
                        return Err(wrong());
                    }
                    if self.store.has_blob(&hash) {
                        continue;
                    }
                    if !self.store.media_users(&hash).any(|s| s.record.community == Some(c)) {
                        return Err(ApiError::new("private-pending", "The post for this attachment is still on its way."));
                    }
                    let done = crate::private::take_chunk(&mut self.scatter.saved.open_partial, hash, total, offset, bytes)?;
                    self.scatter.dirty = true;
                    if let Some(all) = done
                        && !self.on_blob(hash, all)
                    {
                        return Err(wrong());
                    }
                }
            }
        }
        Ok(())
    }

    /// Takes the drop keys of `c` the ledger sealed to this device, the listed one and those
    /// it replaced.
    fn take_drop_keys(&mut self, c: Id) {
        let Some(me) = self.device_kp.as_ref().map(|k| k.public()) else { return };
        if self.drop_rank(c).is_none() {
            return;
        }
        let Some(mine) = self.seal_key() else { return };
        let grants: Vec<DropKeyGrant> = self.communities.get(&c).map(|st| st.drop_keys.iter().map(|d| d.grant.clone()).collect()).unwrap_or_default();
        for g in grants {
            if self.held_drop_key(c, &g.key.0).is_some() {
                continue;
            }
            let Some((_, sealed)) = g.sealed.iter().find(|(d, _)| *d == me) else { continue };
            let Ok(mut secret) = hoi_crypto::envelope::open_drop_key(&mine, &crate::private::drop_key_aad(c, me, &g.key.0), sealed) else { continue };
            if hoi_crypto::Keypair::from_secret(&secret).public() == g.key {
                self.hold_drop_key(c, secret);
            }
            secret.zeroize();
        }
    }

    /// On an owner's device: a drop key for a community without one, a new one once a device it
    /// was sealed to has left the line, and the same one sealed again when a device in line
    /// lacks it. Each in this device's turn among the line.
    fn tend_open_drop_key(&mut self, c: Id) {
        let Some(me) = self.device_kp.as_ref().map(|k| k.public()) else { return };
        let line = self.open_line(c);
        let Some(rank) = line.iter().position(|(d, _)| *d == me) else { return };
        let Some(st) = self.communities.get(&c) else { return };
        if !st.is_owner(&line[rank].1) {
            return;
        }
        // no key is needed before anything here can go in pieces
        if st.drop_key().is_none() && !self.can_scatter(c) && !self.scatter.saved.kept.values().any(|k| k.community == Some(c)) {
            return;
        }
        let in_line: BTreeSet<Id> = line.iter().map(|(d, _)| *d).collect();
        let sealable: Vec<(Id, [u8; 32])> = line.iter().filter_map(|(d, _)| Some((*d, self.delegation_seal(d)?))).collect();
        let listed = st.drop_key().map(|d| d.grant.clone());
        let work = match &listed {
            None => Some(None),
            Some(g) if g.sealed.iter().any(|(d, _)| !in_line.contains(d)) => Some(None),
            Some(g) => match self.held_drop_key(c, &g.key.0) {
                None => Some(None),
                Some(secret) if sealable.iter().any(|(d, _)| !g.sealed.iter().any(|(x, _)| x == d)) => Some(Some((g.key, secret))),
                Some(_) => None,
            },
        };
        let Some(work) = work else {
            self.not_due(c, crate::private::DROP_KEY_DUE);
            return;
        };
        // one that lacks the listed key waits a turn past those that hold it
        let lacks = listed.as_ref().is_some_and(|g| self.held_drop_key(c, &g.key.0).is_none());
        let turns = if lacks { rank + 1 } else { rank };
        if !self.waited_turn(c, crate::private::DROP_KEY_DUE, turns) || sealable.is_empty() {
            return;
        }
        let (key, mut secret, made) = match work {
            Some((key, secret)) => (key, secret, false),
            None => {
                let Ok(kp) = hoi_crypto::Keypair::generate() else { return };
                (kp.public(), kp.secret(), true)
            }
        };
        let sealed: Vec<(Id, Vec<u8>)> = sealable
            .iter()
            .take(hoi_validate::MAX_DROP_KEY_DEVICES)
            .filter_map(|(d, xk)| Some((*d, hoi_crypto::envelope::seal_drop_key(xk, &crate::private::drop_key_aad(c, *d, &key.0), &secret).ok()?)))
            .collect();
        let entry = self.ledger_record(&c, LedgerAction::DropKey, None, None, None, None).map(|mut r| {
            if let Body::Ledger(l) = &mut r.body {
                l.drop_key = Some(DropKeyGrant { key, sealed });
            }
            r
        });
        if entry.and_then(|r| self.publish(r)).is_ok() {
            if made {
                self.hold_drop_key(c, secret);
            }
            self.not_due(c, crate::private::DROP_KEY_DUE);
        }
        secret.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointers_open_only_with_their_community_s_read_key() {
        let manifest = hoi_types::scatter::Manifest {
            object: [1; 32],
            sealed: [2; 32],
            class: hoi_types::scatter::Class::from_index(0).unwrap(),
            generation: 0,
            level: 10,
            places: 5000,
            k: 15,
            n: 28,
            m: 35,
            hashes: (0..28u8).map(|i| [i; 16]).collect(),
            holders: (0..28u8).map(|i| Id([i; 32])).collect(),
            loc_secret: [3; 32],
            key: [4; 32],
            expires_at: 1_790_000_000,
            keys: Vec::new(),
        };
        let p = Pointer { made_at: 7, manifest };
        let sealed = seal_pointer(&Id([1; 32]), &p).unwrap();
        assert_eq!(open_pointer(&Id([1; 32]), &sealed).map(|x| x.encode()), Some(p.encode()));
        assert!(open_pointer(&Id([2; 32]), &sealed).is_none(), "another community's key opens nothing");
        assert_ne!(read_key(&Id([1; 32])), *Id([1; 32]).as_bytes());
    }
}
