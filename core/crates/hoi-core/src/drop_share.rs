//! Per-generation deletion capabilities, encrypted to active devices of the same person.
//! The home relays these opaque link packets; it never receives the author's global secret.
use crate::Node;
use hoi_crypto::{envelope, scatter as sc};
use hoi_types::{
    Id, Sig,
    cbor::Value,
    scatter::Manifest,
    uri::{b64, unb64},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
const CONTEXT: &[u8] = b"hoi own-device scatter deletion v1";
const MAX_CAPS: usize = 4096;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Capability {
    pub manifest: String,
    pub tokens: BTreeMap<Id, [u8; 32]>,
}
impl Capability {
    pub(crate) fn read(&self) -> Option<Manifest> {
        if self.manifest.len() > 40_000 || self.tokens.len() > hoi_types::scatter::MAX_M {
            return None;
        }
        let m = Manifest::decode(&unb64(&self.manifest).ok()?).ok()?;
        if !(0..m.hashes.len()).all(|i| self.tokens.contains_key(&Id(sc::locator(&m.loc_secret, &m.object, m.generation, i as u8)))) {
            return None;
        }
        Some(m)
    }
}
pub(crate) fn cap_id(m: &Manifest) -> Id {
    let mut bytes = m.object.to_vec();
    bytes.push(m.generation);
    Id::hash(&bytes)
}
#[derive(Default)]
pub(crate) struct Runtime {
    next: u64,
    cursor: usize,
    announce: u64,
}
impl Node {
    fn active_own_device(&self, device: &Id) -> bool {
        self.me_root().is_some_and(|root| {
            self.store.burned_at(&root).is_none()
                && self.store.delegation(device).is_some_and(|d| d.info.root == root && d.info.revoked_at.is_none() && d.info.expires_at > self.now())
        })
    }
    pub(crate) fn capability_for(&self, m: &Manifest) -> Option<Capability> {
        let id = cap_id(m);
        let known = self.scatter.saved.drop_caps.get(&id).into_iter().chain(self.home_journal.saved.origin.values().filter_map(|r| r.capability.as_ref()));
        let mut tokens = BTreeMap::new();
        // An extension can combine home-issued old pieces and locally placed fresh indexes.
        // Per-locator authority must survive; the origin never knows the home's drop secret.
        for cap in known {
            let Some(old) = cap.read() else { continue };
            if old.object != m.object
                || old.generation != m.generation
                || old.loc_secret != m.loc_secret
                || old.key != m.key
                || old.sealed != m.sealed
                || old.hashes.len() > m.hashes.len()
                || !m.hashes.starts_with(&old.hashes)
                || !m.holders.starts_with(&old.holders)
            {
                continue;
            }
            for i in 0..old.hashes.len() {
                let loc = Id(sc::locator(&old.loc_secret, &old.object, old.generation, i as u8));
                if let Some(token) = cap.tokens.get(&loc) {
                    tokens.insert(loc, *token);
                }
            }
        }
        let secret = self.drop_secret();
        for i in 0..m.hashes.len() {
            let loc = Id(sc::locator(&m.loc_secret, &m.object, m.generation, i as u8));
            if let std::collections::btree_map::Entry::Vacant(entry) = tokens.entry(loc) {
                entry.insert(sc::drop_token(&secret?, &loc.0));
            }
        }
        Some(Capability { manifest: b64(&m.encode()), tokens })
    }
    pub(crate) fn save_drop_cap(&mut self, mut cap: Capability) {
        let Some(mut m) = cap.read().filter(|m| self.drop_cap_needed(m)) else { return };
        let id = cap_id(&m);
        if let Some(existing) = self.scatter.saved.drop_caps.get(&id).cloned().and_then(|cap| Some((cap.read()?, cap))) {
            let (old, known) = existing;
            if old == m || matches!(m.update_to(&old), Ok(hoi_types::scatter::Update::Extend)) {
                // Replaying an old receipt must not discard a later device's tail tokens.
                cap = known;
                m = old;
            } else if matches!(old.update_to(&m), Ok(hoi_types::scatter::Update::Extend)) {
                for i in 0..old.hashes.len() {
                    let loc = Id(sc::locator(&old.loc_secret, &old.object, old.generation, i as u8));
                    if let Some(token) = known.tokens.get(&loc) {
                        cap.tokens.insert(loc, *token);
                    }
                }
            } else {
                return;
            }
        }
        if self.scatter.saved.drop_caps.len() >= MAX_CAPS && !self.scatter.saved.drop_caps.contains_key(&id) {
            return;
        }
        self.scatter.saved.drop_caps.insert(id, cap);
        self.scatter.dirty = true;
        self.capture_deleted_cap(&m);
    }
    fn drop_cap_needed(&self, m: &Manifest) -> bool {
        // Holder days are inclusive and outlive content expiry. Keep each generation's
        // locator authority through that boundary, including tokens learned from other devices.
        self.now() / 86_400 <= m.expires_at.div_ceil(86_400)
    }
    fn capture_deleted_cap(&mut self, m: &Manifest) {
        let communities: BTreeSet<_> = self.deletions.saved.jobs.values().filter(|j| j.objects.contains(&Id(m.object))).map(|j| j.community).collect();
        let id = cap_id(m);
        let Some(cap) = self.scatter.saved.drop_caps.get(&id).cloned() else { return };
        let mut complete = true;
        for community in communities {
            self.scatter.saved.drop_caps.insert(id, cap.clone());
            // A saved capability supplies its own tokens. Never derive replacement tokens
            // from this device's secret, even when its original sender is no longer known.
            complete &= matches!(self.queue_late_deletion_generation(community, m, Some(Id::ZERO)), Ok(true));
        }
        if !complete {
            self.scatter.saved.drop_caps.insert(id, cap);
            self.scatter.dirty = true;
        }
    }
    pub(crate) fn drop_share_tick(&mut self) {
        let now = self.now();
        if now < self.scatter.drop_share.next {
            return;
        }
        self.scatter.drop_share.next = now + 20;
        if now >= self.scatter.drop_share.announce {
            self.scatter.drop_share.announce = now + 60;
            if let Some(record) = self.device_kp.as_ref().and_then(|k| self.store.delegation(&k.public())).and_then(|d| self.store.get(&d.record)) {
                let body = crate::link::body_of(vec![("t", Value::T("drop-device".into())), ("rec", Value::B(record.wire.clone()))]);
                self.send_own_link(&body);
            }
        }
        let before = self.scatter.saved.drop_caps.len();
        let expired: Vec<_> =
            self.scatter.saved.drop_caps.iter().filter(|(_, cap)| cap.read().is_none_or(|m| !self.drop_cap_needed(&m))).map(|(id, _)| *id).collect();
        for id in expired {
            self.scatter.saved.drop_caps.remove(&id);
        }
        self.scatter.dirty |= before != self.scatter.saved.drop_caps.len();
        let pending: Vec<_> =
            self.scatter.saved.drop_caps.values().filter_map(|cap| cap.read()).filter(|m| self.deletion_object_suppressed(Id(m.object))).collect();
        for m in pending {
            self.capture_deleted_cap(&m);
        }
        let local: Vec<_> = self
            .scatter
            .saved
            .kept
            .values()
            .flat_map(|k| &k.gens)
            .filter(|g| g.from.is_none())
            .filter_map(|g| g.read())
            .filter(|m| !self.deletion_object_suppressed(Id(m.object)))
            .filter(|m| !self.scatter.saved.drop_caps.contains_key(&cap_id(m)))
            .take(8)
            .collect();
        for m in local {
            if let Some(cap) = self.capability_for(&m) {
                self.save_drop_cap(cap);
            }
        }
        let Some(kp) = self.device_kp.as_ref() else { return };
        let sender = kp.public();
        if !self.active_own_device(&sender) {
            return;
        }
        let Some(root) = self.me_root() else { return };
        let devices: Vec<_> = self
            .store
            .delegations_of(&root)
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| *id != sender && self.active_own_device(id))
            .filter_map(|id| Some((id, self.delegation_seal(&id)?)))
            .collect();
        let caps: Vec<_> =
            self.scatter.saved.drop_caps.values().filter(|cap| cap.read().is_some_and(|m| !self.deletion_object_suppressed(Id(m.object)))).collect();
        if caps.is_empty() || devices.is_empty() {
            return;
        }
        let cursor = self.scatter.drop_share.cursor;
        self.scatter.drop_share.cursor = cursor.wrapping_add(1);
        let (to, key) = devices[cursor % devices.len()];
        let cap = caps[(cursor / devices.len()) % caps.len()];
        let Ok(plain) = serde_json::to_vec(cap) else { return };
        let mut aad = sender.0.to_vec();
        aad.extend_from_slice(&to.0);
        let Ok(sealed) = envelope::seal_to(&key, CONTEXT, &aad, &plain) else { return };
        let mut signed = CONTEXT.to_vec();
        signed.extend_from_slice(&aad);
        signed.extend_from_slice(&sealed);
        let sig = kp.sign(&signed);
        let body = crate::link::body_of(vec![
            ("t", Value::T("drop-share".into())),
            ("from", Value::B(sender.0.to_vec())),
            ("to", Value::B(to.0.to_vec())),
            ("sealed", Value::B(sealed)),
            ("sig", Value::B(sig.0.to_vec())),
        ]);
        self.send_own_link(&body);
    }
    pub(crate) fn on_drop_device(&mut self, peer: &str, wire: &[u8]) {
        if wire.len() > 4096 {
            return;
        }
        let Ok((record, _)) = hoi_types::Wire::decode(wire) else { return };
        if !matches!(record.record.body, hoi_types::Body::Delegation { .. }) {
            return;
        }
        // Normal signature, root authorization, expiry and revocation validation applies.
        let _ = self.ingest(wire, &crate::Source::Peer { peer: peer.into(), live: false });
    }
    pub(crate) fn on_drop_share(&mut self, msg: &BTreeMap<String, Value>) {
        let bytes = |key: &str| match msg.get(key) {
            Some(Value::B(b)) => Some(b.as_slice()),
            _ => None,
        };
        let Some((from, to, sealed, sig)) =
            (|| Some((Id::from_slice(bytes("from")?).ok()?, Id::from_slice(bytes("to")?).ok()?, bytes("sealed")?, Sig(bytes("sig")?.try_into().ok()?))))()
        else {
            return;
        };
        if sealed.len() > 128 * 1024
            || self.device_kp.as_ref().map(|k| k.public()) != Some(to)
            || !self.active_own_device(&to)
            || !self.active_own_device(&from)
        {
            return;
        }
        let mut aad = from.0.to_vec();
        aad.extend_from_slice(&to.0);
        let mut signed = CONTEXT.to_vec();
        signed.extend_from_slice(&aad);
        signed.extend_from_slice(sealed);
        if !hoi_crypto::verify(&from, &signed, &sig) {
            return;
        }
        let Some(key) = self.seal_key() else { return };
        let Ok(plain) = envelope::open_sealed(&key, CONTEXT, &aad, sealed) else { return };
        if let Ok(cap) = serde_json::from_slice::<Capability>(&plain) {
            self.save_drop_cap(cap);
        }
    }
}
