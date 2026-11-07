//! Labelers (M3): people whose labels a person chooses to follow. A label (`spam`,
//! `nsfw`, `off-topic`, …, or a note) from a labeler someone here follows, or one the community
//! names as a default in its limits, puts its target to sleep or adds a note on this device; one
//! from anybody else is refused, without penalty. Following a labeler reconciles every
//! connection afresh, so labels this device refused before arrive with the next sync.

use std::collections::BTreeMap;

use hoi_types::{Body, Id, Kind, Record};

use crate::api::{R, parse_id, parse_person_key};
use crate::dto::LabelerView;
use crate::error::ApiError;
use crate::node::Node;

impl Node {
    /// The labelers this person follows, and the ones their communities name by default.
    pub fn labelers(&self) -> Vec<LabelerView> {
        let mut out: BTreeMap<Id, LabelerView> = BTreeMap::new();
        let count = |root: &Id| -> u32 {
            self.state.joined.keys().map(|c| self.store.by_author(c, root).filter(|s| s.record.kind() == Kind::Label).count() as u32).sum()
        };
        for root in &self.state.labelers {
            out.insert(*root, LabelerView { person: self.person(root, None), followed: true, default_in: Vec::new(), labels: count(root) });
        }
        for c in self.state.joined.keys() {
            let Some(st) = self.communities.get(c) else { continue };
            for root in &st.limits.labelers {
                let v = out.entry(*root).or_insert_with(|| LabelerView {
                    person: self.person(root, None),
                    followed: false,
                    default_in: Vec::new(),
                    labels: count(root),
                });
                v.default_in.push(self.community_ref(st));
            }
        }
        out.into_values().collect()
    }

    /// Follows a labeler (their labels count on this device) or stops following them.
    pub fn follow_labeler(&mut self, key: &str, on: bool) -> R<Vec<LabelerView>> {
        let k = parse_person_key(key)?;
        if Some(k) == self.me_root() {
            return Err(ApiError::invalid("Your own labels always count here."));
        }
        let changed = if on { self.state.labelers.insert(k) } else { self.state.labelers.remove(&k) };
        if changed {
            self.mark_state();
            self.changed_everywhere();
            if on {
                self.reconcile_again();
            }
        }
        Ok(self.labelers())
    }

    /// Labels a post or comment, as a labeler does: everyone who follows this person, or whose
    /// community names them, sees it. A hiding label (`spam`, `nsfw`, …) puts it to sleep there.
    pub fn publish_label(&mut self, target: &str, label: &str, note: Option<&str>) -> R<()> {
        self.need_me()?;
        let t = parse_id(target)?;
        let s = self.store.get(&t).ok_or_else(|| ApiError::not_found("post or comment"))?;
        if !matches!(s.record.kind(), Kind::Post | Kind::Comment) {
            return Err(ApiError::invalid("Labels go on posts and comments."));
        }
        let c = s.record.community.ok_or_else(|| ApiError::not_found("community"))?;
        if self.is_private_community(&c) {
            return Err(ApiError::not_allowed("Private groups moderate among themselves: labels are for public communities."));
        }
        let label = label.trim();
        if label.is_empty() {
            return Err(ApiError::invalid("Say what the label is: spam, nsfw, off-topic, or a short note."));
        }
        let st = self.community_of(&c)?;
        let now = self.now();
        let r = Record {
            v: 1,
            community: Some(c),
            author: self.device_key()?,
            parent: Some(t),
            created_at: now,
            expires_at: now + st.limits.max_retention,
            body: Body::Label { label: label.to_string(), note: note.map(str::trim).filter(|n| !n.is_empty()).map(String::from) },
            nonce: 0,
        };
        hoi_validate::content(&r)?;
        self.publish(r)?;
        Ok(())
    }

    /// Starts every connection's sync over: what this device refused before and takes now
    /// comes with it (crate::net).
    pub(crate) fn reconcile_again(&mut self) {
        let peers: Vec<String> = self.peers.iter().filter(|(_, p)| p.said_hello).map(|(k, _)| k.clone()).collect();
        for peer in peers {
            if let Some(p) = self.peers.get_mut(&peer) {
                p.sent_have.clear();
                p.ranging.clear();
            }
            self.send_haves(&peer);
        }
    }
}
