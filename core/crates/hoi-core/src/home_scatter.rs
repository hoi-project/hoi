//! Placement over the existing authenticated home link. The originating device still
//! validates the object and signs its pointers. Its home necessarily knows the sender.
use crate::drop_share::Capability;
use crate::{
    Node,
    api::R,
    error::ApiError,
    scatter::{Ask, ScatterOutcome},
};
use hoi_crypto::scatter as sc;
use hoi_sync::Msg;
use hoi_types::{
    Id,
    scatter::Manifest,
    uri::{b64, unb64},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Packet {
    CleanupKeys { holders: Vec<Id> },
    CleanupKeyList { keys: Vec<CleanupKey> },
    JournalRelease { request: Id },
    JournalStore { request: Id, community: Id, content: String, level: u8, expires: u64 },
    JournalResult { request: Id, stored: Option<bool>, capability: Option<Capability> },
    Store { community: Id, content: String, level: u8, expires: u64 },
    Stored { capability: Option<Capability> },
    Status { records: Vec<Id> },
    Receipt { states: BTreeMap<Id, String> },
    Drop { capability: Capability },
}
/// Fresh signed encryption keys, without historical uptime or reachability claims.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CleanupKey {
    pub holder: Id,
    pub xk: String,
    pub expires: u64,
    pub sig: String,
    pub drop_journal: Option<String>,
}
enum Waiting {
    #[cfg(test)]
    Store {
        op: u64,
        content: Vec<u8>,
        level: u8,
        expires: u64,
    },
    Status(Vec<Id>),
    CleanupKeys {
        home: Id,
        holders: Vec<Id>,
    },
}
#[derive(Default)]
pub(crate) struct Runtime {
    asked: BTreeMap<(String, u32), (u64, Waiting)>,
    rate: BTreeMap<String, (u64, u32)>,
    poll_at: u64,
    cleanup_keys_at: u64,
    pub(crate) cleanup_egress: BTreeMap<(String, u32), Id>,
}
impl Node {
    pub(crate) fn scattering_home(&self, c: Id) -> Option<String> {
        if self.is_private_community(&c) {
            return None;
        }
        self.peers
            .iter()
            .find(|(_, p)| p.home && p.said_hello && p.features.contains(hoi_sync::FEATURE_HOME_SCATTER_JOURNAL) && p.features.contains("scatter-store"))
            .map(|(peer, _)| peer.clone())
    }
    pub(crate) fn deletion_home_placement_pending(&self, target: Id) -> bool {
        if self.journal_deletion_pending(target) {
            return true;
        }
        #[cfg(test)]
        {
            self.scatter.home_scatter.asked.values().any(|(_, waiting)| {
                let Waiting::Store { op, .. } = waiting else { return false };
                let Some(crate::keep::Job::Outgoing { batch }) = self.scatter.jobs.get(op) else { return false };
                self.scatter.repair.outgoing.get(batch).and_then(|out| out.carries.get(op)).is_some_and(|(records, _)| records.contains(&target))
            })
        }
        #[cfg(not(test))]
        {
            false
        }
    }
    pub(crate) fn home_packet(&mut self, peer: &str, rid: u32, reply: bool, packet: Packet) {
        if let Ok(payload) = serde_json::to_vec(&packet) {
            self.send_msg(peer, Msg::HomeScatter { rid, reply, payload });
        }
    }
    pub(crate) fn store_at_home(&mut self, content: &[u8], level: u8, expires: u64) -> R<u64> {
        let c = self.scatter.placing.ok_or_else(|| ApiError::invalid("Missing community."))?;
        let home = self.scattering_home(c).unwrap_or_default();
        self.journal_store(&home, c, content, level, expires)
    }
    #[cfg(test)]
    fn start_home_store(&mut self, home: &str, c: Id, content: &[u8], level: u8, expires: u64) -> R<u64> {
        self.check_room(expires)?;
        if content.len() > hoi_sync::MAX_REBUILT {
            return Err(ApiError::invalid("This object is too large."));
        }
        let op = self.scatter_wait_op();
        let rid = self.next_rid();
        self.scatter.home_scatter.asked.insert((home.into(), rid), (self.now(), Waiting::Store { op, content: content.to_vec(), level, expires }));
        self.home_packet(home, rid, false, Packet::Store { community: c, content: b64(content), level, expires });
        Ok(op)
    }
    /// Exercise compatibility with a request already submitted by a legacy client.
    #[cfg(test)]
    pub(crate) fn legacy_home_store(&mut self, home: &str, c: Id, content: &[u8], level: u8, expires: u64) -> R<u64> {
        self.start_home_store(home, c, content, level, expires)
    }
    pub(crate) fn on_home_scatter(&mut self, peer: &str, rid: u32, reply: bool, bytes: &[u8]) {
        let Some(p) = self.peers.get(peer).filter(|p| p.said_hello && if reply { p.home } else { p.client }) else { return };
        if !p.features.contains(hoi_sync::FEATURE_HOME_SCATTER) && !p.features.contains(hoi_sync::FEATURE_HOME_SCATTER_JOURNAL) {
            return;
        }
        if reply {
            if !self.scatter.home_scatter.asked.contains_key(&(peer.into(), rid)) && !self.home_journal.waits.contains_key(&(peer.into(), rid)) {
                return;
            }
        } else {
            let now = self.now();
            let rate = self.scatter.home_scatter.rate.entry(peer.into()).or_default();
            if rate.0 != now / 60 {
                *rate = (now / 60, 0);
            }
            rate.1 = rate.1.saturating_add(1);
            if rate.1 > 32 {
                return;
            }
        }
        let Ok(packet) = serde_json::from_slice::<Packet>(bytes) else { return };
        if self.journal_packet(peer, rid, reply, &packet) {
            return;
        }
        if reply {
            let Some((_, waiting)) = self.scatter.home_scatter.asked.remove(&(peer.into(), rid)) else { return };
            match (waiting, packet) {
                #[cfg(test)]
                (Waiting::Store { op, content, level, expires }, Packet::Stored { capability }) => {
                    let valid = capability.and_then(|cap| {
                        let m = cap.read()?;
                        let (class, sealed) = sc::seal_object(&content, &m.object, &m.key).ok()?;
                        (m.generation == 0 && m.level == level && m.expires_at == expires && m.class == class && m.sealed == sc::sealed_hash(&sealed))
                            .then_some((cap, m))
                    });
                    let outcome = if let Some((cap, m)) = valid {
                        self.save_drop_cap(cap);
                        ScatterOutcome::Stored(m)
                    } else {
                        ScatterOutcome::Failed("Your home could not store this post in pieces; delivery will use the whole post.".into())
                    };
                    self.end_wait_op(op, outcome);
                }
                (Waiting::CleanupKeys { home, holders }, Packet::CleanupKeyList { keys }) if Some(home) == self.journal_home() && keys.len() <= 32 => {
                    for key in keys.into_iter().filter(|k| holders.contains(&k.holder)) {
                        let xk = unb64(&key.xk).ok().and_then(|v| <[u8; 32]>::try_from(v).ok());
                        let sig = unb64(&key.sig).ok().and_then(|v| <[u8; 64]>::try_from(v).ok());
                        if let (Some(xk), Some(sig)) = (xk, sig)
                            && hoi_crypto::envelope::xk_valid(&key.holder.0, &xk, key.expires, &hoi_types::Sig(sig), self.now())
                        {
                            let drop_journal = key.drop_journal.and_then(|s| unb64(&s).ok()).and_then(|v| <[u8; 64]>::try_from(v).ok());
                            self.remember_key(key.holder, hoi_sync::HolderKey { xk, expires_at: key.expires, sig, rtc: None, drop_journal });
                        }
                    }
                }
                (Waiting::Status(ids), Packet::Receipt { states }) => {
                    for id in ids {
                        if let Some(state) = states.get(&id).filter(|s| ["pending", "injected", "fallback"].contains(&s.as_str())) {
                            self.scatter.saved.home_delivery.insert(id, state.clone());
                            self.scatter.dirty = true;
                            if let Some(c) = self.store.get(&id).and_then(|s| s.record.community) {
                                self.mark_changed(c);
                            }
                        }
                    }
                }
                #[cfg(test)]
                (Waiting::Store { op, .. }, _) => {
                    self.end_wait_op(op, ScatterOutcome::Failed("Your home returned an invalid placement response.".into()));
                }
                _ => {}
            }
            return;
        }
        match packet {
            Packet::CleanupKeys { holders }
                if holders.len() <= 32
                    && self.peers.get(peer).is_some_and(|p| p.client_credential.is_some() && p.features.contains(hoi_sync::FEATURE_HOME_SCATTER_JOURNAL)) =>
            {
                let now = self.now();
                let keys = holders
                    .into_iter()
                    .filter_map(|holder| {
                        let k = self.scatter.keys.get(&holder).filter(|k| k.expires_at > now)?;
                        Some(CleanupKey { holder, xk: b64(&k.xk), expires: k.expires_at, sig: b64(&k.sig), drop_journal: k.drop_journal.map(|s| b64(&s)) })
                    })
                    .collect();
                self.home_packet(peer, rid, true, Packet::CleanupKeyList { keys });
            }
            Packet::Store { community, content, level, expires } => {
                let running = self.scatter.jobs.values().filter(|j| matches!(j, crate::keep::Job::StoreForDevice { peer: p, .. } if p == peer)).count();
                let content = unb64(&content).ok().filter(|b| b.len() <= hoi_sync::MAX_REBUILT);
                let op = if running < 16 && !self.is_private_community(&community) && self.peers.get(peer).is_some_and(|p| p.communities.contains(&community)) {
                    self.scatter.placing = Some(community);
                    self.scatter.next_drop_key = self.community_drop_key(community).map(|k| sc::drop_key_id(&k));
                    let result = content.and_then(|b| self.scatter_store_inner(&b, level, expires).ok());
                    self.scatter.placing = None;
                    self.scatter.next_drop_key = None;
                    result
                } else {
                    None
                };
                if let Some(op) = op {
                    self.scatter.jobs.insert(op, crate::keep::Job::StoreForDevice { peer: peer.into(), rid });
                    if self.op_done(op) {
                        self.job_done(op);
                    }
                } else {
                    self.home_packet(peer, rid, true, Packet::Stored { capability: None });
                }
            }
            Packet::Status { records } if records.len() <= 32 => {
                let states = records
                    .into_iter()
                    .map(|id| {
                        let state = if self.scatter.saved.injected.contains(&id) {
                            "injected"
                        } else if self.scatter.saved.injecting.contains_key(&id) || !self.store.contains(&id) {
                            "pending"
                        } else {
                            "fallback"
                        };
                        (id, state.into())
                    })
                    .collect();
                self.home_packet(peer, rid, true, Packet::Receipt { states });
            }
            Packet::Drop { capability } => {
                if let Some(m) = capability.read()
                    && let Ok(op) = self.scatter_drop(&m, &Ask::DropTokens(capability.tokens))
                {
                    self.scatter.jobs.insert(op, crate::keep::Job::Dropped);
                    if self.op_done(op) {
                        self.job_done(op);
                    }
                }
            }
            _ => {}
        }
    }
    /// Lookup discloses endpoints only to the original home. Tokens remain sealed by the
    /// origin, and this reply never counts as a holder acknowledgement.
    pub(crate) fn home_cleanup_keys(&mut self, home: &str, holders: Vec<Id>) {
        let now = self.now();
        if holders.is_empty() || now < self.scatter.home_scatter.cleanup_keys_at {
            return;
        }
        let Some(scope) = self.journal_home() else { return };
        self.scatter.home_scatter.cleanup_keys_at = now + 5;
        let rid = self.next_rid();
        self.scatter.home_scatter.asked.insert((home.into(), rid), (now, Waiting::CleanupKeys { home: scope, holders: holders.clone() }));
        self.home_packet(home, rid, false, Packet::CleanupKeys { holders });
    }
    pub(crate) fn home_cleanup_scopes(&mut self) -> BTreeMap<(String, u32), Id> {
        let mut scopes = std::mem::take(&mut self.scatter.home_scatter.cleanup_egress);
        for (key, (_, waiting)) in &self.scatter.home_scatter.asked {
            if let Waiting::CleanupKeys { home, .. } = waiting {
                scopes.insert(key.clone(), *home);
            }
        }
        scopes
    }
    pub(crate) fn stored_for_home_device(&mut self, peer: &str, rid: u32, outcome: ScatterOutcome) {
        let capability = match outcome {
            ScatterOutcome::Stored(m) => self.capability_for(&m),
            _ => None,
        };
        self.home_packet(peer, rid, true, Packet::Stored { capability });
    }
    pub(crate) fn drop_at_home(&mut self, m: &Manifest, ask: &Ask) -> bool {
        let Some(home) = self.peers.iter().find(|(_, p)| p.home && p.said_hello && p.features.contains(hoi_sync::FEATURE_HOME_SCATTER)).map(|(p, _)| p.clone())
        else {
            return false;
        };
        let tokens = match ask {
            Ask::DropTokens(tokens) => tokens.clone(),
            Ask::DropToken(secret) => (0..m.hashes.len() as u8)
                .map(|i| {
                    let loc = sc::locator(&m.loc_secret, &m.object, m.generation, i);
                    (Id(loc), sc::drop_token(secret, &loc))
                })
                .collect(),
            _ => return false,
        };
        let rid = self.next_rid();
        self.home_packet(&home, rid, false, Packet::Drop { capability: Capability { manifest: b64(&m.encode()), tokens } });
        true
    }
    pub(crate) fn home_scatter_tick(&mut self, gone: Option<&str>) {
        self.journal_tick();
        let now = self.now();
        self.scatter.home_scatter.rate.retain(|peer, _| self.peers.contains_key(peer));
        let late: Vec<_> =
            self.scatter.home_scatter.asked.iter().filter(|((p, _), (at, _))| now >= at + 120 || gone == Some(p.as_str())).map(|(k, _)| k.clone()).collect();
        for key in late {
            let waiting = self.scatter.home_scatter.asked.remove(&key);
            #[cfg(not(test))]
            let _ = waiting;
            #[cfg(test)]
            if let Some((_, Waiting::Store { op, .. })) = waiting {
                self.end_wait_op(op, ScatterOutcome::Failed("Your home did not answer; delivery will use the whole post.".into()));
            }
        }
        if gone.is_some() || now < self.scatter.home_scatter.poll_at {
            return;
        }
        self.scatter.home_scatter.poll_at = now + 15;
        let Some(home) = self
            .peers
            .iter()
            .find(|(_, p)| {
                p.home && p.said_hello && (p.features.contains(hoi_sync::FEATURE_HOME_SCATTER) || p.features.contains(hoi_sync::FEATURE_HOME_SCATTER_JOURNAL))
            })
            .map(|(p, _)| p.clone())
        else {
            return;
        };
        let records: Vec<Id> = self.scatter.saved.home_delivery.iter().filter(|(_, state)| state.as_str() == "pending").map(|(id, _)| *id).take(32).collect();
        if records.is_empty() {
            return;
        }
        let rid = self.next_rid();
        self.scatter.home_scatter.asked.insert((home.clone(), rid), (now, Waiting::Status(records.clone())));
        self.home_packet(&home, rid, false, Packet::Status { records });
    }
}
