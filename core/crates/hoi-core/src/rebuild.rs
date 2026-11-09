//! Home-node rebuild (SCATTER.md §5.12, phase S3): a paired home node is the person's own
//! device, so it may fetch and rebuild objects kept in pieces for its phones and browsers and
//! hand them the content over the pairing link. A phone then needs one link instead of three
//! forwarders, and fetches no pieces itself.
//!
//! The home node says it does this (`rb` in its hello) only to its own devices, and answers
//! `rebuild` only from them; it never serves rebuilt objects to anyone else. A device asks only a
//! home node that said `rb`, and falls back to fetching through its own forwarders when the home
//! node fails or does not answer.

use std::collections::{BTreeMap, BTreeSet};

use hoi_sync::Msg;
use hoi_types::{Id, scatter::Manifest};

use crate::Node;
use crate::api::R;
use crate::net::Role;
use crate::scatter::ScatterOutcome;

/// Rebuilds a home node runs for one device at a time.
const PER_DEVICE: usize = 16;
/// How long a device waits for its home node before fetching itself.
pub(crate) const WAIT: u64 = 120;
/// After a home node failed, this long before it is asked again.
const PAUSE: u64 = 600;
/// How a rebuild ends that the home node never answered: it says nothing about the generation,
/// so the next try asks for the same one again (crate::keep).
pub(crate) const HOME_AWAY: &str = "Your home node did not answer.";

/// Rebuilds in memory: on a device, what it asked its home node; on a home node, what it runs
/// for its devices.
#[derive(Default)]
pub(crate) struct Rebuilds {
    /// (home link, request id) → (operation, when asked).
    pub(crate) asked: BTreeMap<(String, u32), (u64, u64)>,
    /// Rebuilt replies still queued locally, so a later deletion can suppress their content.
    ready: BTreeMap<(String, u32), BTreeSet<Id>>,
    /// When the home node last failed a rebuild.
    failed_at: u64,
}

impl Node {
    /// Whether this node rebuilds for `peer`: it is one of our own devices.
    pub(crate) fn rebuilds_for(&self, peer: &str) -> bool {
        self.peers.get(peer).is_some_and(|p| p.client)
    }

    /// Our home node, when it said it rebuilds and has not failed lately; with "Use less data",
    /// whenever it is linked and said it rebuilds.
    fn rebuilding_home(&self) -> Option<String> {
        if !self.state.settings.less_data && self.now() < self.scatter.rebuilds.failed_at + PAUSE {
            return None;
        }
        self.peers.iter().find(|(_, p)| p.said_hello && p.role() == Role::Home && p.scatter.rebuilds).map(|(k, _)| k.clone())
    }

    /// Starts fetching an object for this device: through its home node when one rebuilds for
    /// it, else through its own forwarders. With "Use less data" a paired device waits
    /// for its home node instead, and one with no home node asks one spare piece fewer.
    pub(crate) fn load_kept(&mut self, manifest: &Manifest) -> R<u64> {
        let less = self.state.settings.less_data;
        let Some(home) = self.rebuilding_home() else {
            return match (less, self.state.home.is_some()) {
                (true, true) => Err(crate::ApiError::new("unavailable", HOME_AWAY)),
                (true, false) => self.scatter_load_with(manifest, crate::scatter::LESS_SPARES),
                _ => self.scatter_load(manifest),
            };
        };
        let op = self.scatter_wait_op();
        let rid = self.next_rid();
        let now = self.now();
        self.scatter.rebuilds.asked.insert((home.clone(), rid), (op, now));
        self.send_msg(&home, Msg::Rebuild { rid, manifest: manifest.encode() });
        Ok(op)
    }

    /// A `rebuild` from a peer: only one of our own devices is answered, a few at a time.
    pub(crate) fn on_rebuild(&mut self, peer: &str, rid: u32, manifest: &[u8]) {
        if !self.rebuilds_for(peer) {
            return;
        }
        let running = self.scatter.jobs.values().filter(|j| matches!(j, crate::keep::Job::ForDevice { peer: p, .. } if p == peer)).count();
        let op = match Manifest::decode(manifest) {
            Ok(m) if running < PER_DEVICE && !self.home_rebuild_blocked(Id(m.object)) => self.scatter_load(&m).map(|op| (op, Id(m.object))),
            _ => Err(crate::ApiError::invalid("not now")),
        };
        match op {
            Ok((op, object)) => {
                self.scatter.jobs.insert(op, crate::keep::Job::ForDevice { peer: peer.to_string(), rid, object });
                if self.op_done(op) {
                    self.job_done(op);
                }
            }
            Err(_) => self.send_msg(peer, Msg::Rebuilt { rid, content: None }),
        }
    }

    /// A rebuild this home node ran for a device finished.
    pub(crate) fn rebuilt_for(&mut self, peer: &str, rid: u32, object: Id, outcome: ScatterOutcome) {
        let content = match outcome {
            ScatterOutcome::Loaded(c) if !self.home_rebuild_blocked(object) => Some(c),
            _ => None,
        };
        if self.rebuilds_for(peer) {
            if content.is_some() {
                self.scatter.rebuilds.ready.entry((peer.into(), rid)).or_default().insert(object);
            }
            self.send_msg(peer, Msg::Rebuilt { rid, content });
        }
    }

    pub(crate) fn home_rebuild_blocked(&self, object: Id) -> bool {
        if self.scatter_object_deleted(object) {
            return true;
        }
        match self.scatter.saved.kept.get(&object).and_then(|kept| kept.community) {
            Some(community) => self.deletion_restore_quarantined(Some(community)),
            // A manifest has no community. Without an established association, a stale
            // restored node cannot know whether catch-up will suppress this object.
            None => !self.deletions.saved.restored.is_empty(),
        }
    }

    pub(crate) fn cancel_unservable_home_rebuilds(&mut self) {
        let blocked: Vec<_> = self
            .scatter
            .jobs
            .iter()
            .filter_map(|(op, job)| match job {
                crate::keep::Job::ForDevice { peer, rid, object } if self.home_rebuild_blocked(*object) => Some((*op, peer.clone(), *rid, *object)),
                _ => None,
            })
            .collect();
        for (op, peer, rid, object) in blocked {
            self.cancel_scatter_operation(op);
            self.rebuilt_for(&peer, rid, object, ScatterOutcome::Failed("This object is unavailable.".into()));
        }
    }

    pub(crate) fn filter_rebuilt_outbox(&mut self) {
        let blocked: BTreeSet<_> = std::mem::take(&mut self.scatter.rebuilds.ready)
            .into_iter()
            .filter(|(_, objects)| objects.iter().any(|object| self.home_rebuild_blocked(*object)))
            .map(|(key, _)| key)
            .collect();
        if blocked.is_empty() {
            return;
        }
        for (peer, bytes) in &mut self.outbox {
            if let Ok(Msg::Rebuilt { rid, content: Some(_) }) = Msg::decode(bytes)
                && blocked.contains(&(peer.clone(), rid))
            {
                *bytes = Msg::Rebuilt { rid, content: None }.encode();
            }
        }
    }

    /// The home node's answer to a `rebuild` this device sent.
    pub(crate) fn on_rebuilt(&mut self, peer: &str, rid: u32, content: Option<Vec<u8>>) {
        let Some((op, _)) = self.scatter.rebuilds.asked.remove(&(peer.to_string(), rid)) else { return };
        let outcome = match content {
            Some(c) => ScatterOutcome::Loaded(c),
            None => {
                self.scatter.rebuilds.failed_at = self.now();
                ScatterOutcome::Failed("Your home node could not rebuild this post right now.".into())
            }
        };
        self.end_wait_op(op, outcome);
    }

    /// Rebuilds asked of a home node that never answered fail, and the home node is left alone a
    /// while; so are those of a home link that went away.
    pub(crate) fn rebuild_tick(&mut self, gone: Option<&str>) {
        let now = self.now();
        let late: Vec<(String, u32)> =
            self.scatter.rebuilds.asked.iter().filter(|((p, _), (_, at))| now >= at + WAIT || Some(p.as_str()) == gone).map(|(k, _)| k.clone()).collect();
        for key in late {
            let Some((op, _)) = self.scatter.rebuilds.asked.remove(&key) else { continue };
            self.scatter.rebuilds.failed_at = now;
            self.end_wait_op(op, ScatterOutcome::Failed(HOME_AWAY.into()));
        }
    }
}
