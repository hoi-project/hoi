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
enum Packet {
    Store { community: Id, content: String, level: u8, expires: u64 },
    Stored { capability: Option<Capability> },
    Status { records: Vec<Id> },
    Receipt { states: BTreeMap<Id, String> },
    Drop { capability: Capability },
}
enum Waiting {
    Store { op: u64, content: Vec<u8>, level: u8, expires: u64 },
    Status(Vec<Id>),
}
#[derive(Default)]
pub(crate) struct Runtime {
    asked: BTreeMap<(String, u32), (u64, Waiting)>,
    rate: BTreeMap<String, (u64, u32)>,
    poll_at: u64,
    failed_at: Option<u64>,
}
impl Node {
    pub(crate) fn scattering_home(&self, _c: Id) -> Option<String> {
        // RFC-0001 requires deletion authority to survive an origin restart. Legacy home
        // placement has no durable request identity or result recovery: a lost Stored reply
        // loses every locator and token. Until that protocol exists, originate locally or
        // use whole-record delivery. Keep the legacy receiver for existing clients.
        None
    }
    pub(crate) fn deletion_home_placement_pending(&self, target: Id) -> bool {
        self.scatter.home_scatter.asked.values().any(|(_, waiting)| {
            let Waiting::Store { op, .. } = waiting else { return false };
            let Some(crate::keep::Job::Outgoing { batch }) = self.scatter.jobs.get(op) else { return false };
            self.scatter.repair.outgoing.get(batch).and_then(|out| out.carries.get(op)).is_some_and(|(records, _)| records.contains(&target))
        })
    }
    fn home_packet(&mut self, peer: &str, rid: u32, reply: bool, packet: Packet) {
        if let Ok(payload) = serde_json::to_vec(&packet) {
            self.send_msg(peer, Msg::HomeScatter { rid, reply, payload });
        }
    }
    pub(crate) fn store_at_home(&mut self, content: &[u8], level: u8, expires: u64) -> R<u64> {
        let c = self.scatter.placing.ok_or_else(|| ApiError::invalid("Missing community."))?;
        let home = self.scattering_home(c).ok_or_else(|| ApiError::invalid("Home unavailable."))?;
        self.start_home_store(&home, c, content, level, expires)
    }
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
        if !p.features.contains(hoi_sync::FEATURE_HOME_SCATTER) {
            return;
        }
        if reply {
            if !self.scatter.home_scatter.asked.contains_key(&(peer.into(), rid)) {
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
        if reply {
            let Some((_, waiting)) = self.scatter.home_scatter.asked.remove(&(peer.into(), rid)) else { return };
            match (waiting, packet) {
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
                        self.scatter.home_scatter.failed_at = Some(self.now());
                        ScatterOutcome::Failed("Your home could not store this post in pieces; delivery will use the whole post.".into())
                    };
                    self.end_wait_op(op, outcome);
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
                (Waiting::Store { op, .. }, _) => {
                    self.scatter.home_scatter.failed_at = Some(self.now());
                    self.end_wait_op(op, ScatterOutcome::Failed("Your home returned an invalid placement response.".into()));
                }
                _ => {}
            }
            return;
        }
        match packet {
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
        let now = self.now();
        self.scatter.home_scatter.rate.retain(|peer, _| self.peers.contains_key(peer));
        let late: Vec<_> =
            self.scatter.home_scatter.asked.iter().filter(|((p, _), (at, _))| now >= at + 120 || gone == Some(p.as_str())).map(|(k, _)| k.clone()).collect();
        for key in late {
            if let Some((_, Waiting::Store { op, .. })) = self.scatter.home_scatter.asked.remove(&key) {
                self.scatter.home_scatter.failed_at = Some(now);
                self.end_wait_op(op, ScatterOutcome::Failed("Your home did not answer; delivery will use the whole post.".into()));
            }
        }
        if gone.is_some() || now < self.scatter.home_scatter.poll_at {
            return;
        }
        self.scatter.home_scatter.poll_at = now + 15;
        let Some(home) = self.peers.iter().find(|(_, p)| p.home && p.said_hello && p.features.contains(hoi_sync::FEATURE_HOME_SCATTER)).map(|(p, _)| p.clone())
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
