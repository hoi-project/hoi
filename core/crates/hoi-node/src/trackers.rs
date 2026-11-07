//! Announcing to the trackers communities name. An open community may name up to two
//! trackers in its limits: registries where public nodes that carry it say so, so a device whose
//! invite routes have all gone stale can still find a carrier. A public node announces itself to
//! the trackers of the communities it carries, with the same signed announcement and the same
//! loop as `--index` (announcing only, never mirroring), and stops when no community it carries
//! names a tracker any more. Which trackers is the communities' choice, never this node's.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use hoi_net::Shared;
use tokio::task::JoinHandle;

use crate::index::{self, IndexConfig};

/// Trackers one node announces to at most, whatever its communities name.
pub const MAX_TRACKERS: usize = 16;
const EVERY: Duration = Duration::from_secs(60);

/// Watches what the carried communities name; `skip`: registries already announced to
/// (`--index`).
pub fn spawn(shared: Arc<Shared>, skip: Vec<String>, carry_limit: usize) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut running: HashMap<String, JoinHandle<()>> = HashMap::new();
        let mut tick = tokio::time::interval(EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let want: BTreeSet<String> = shared.with(|n| n.tracker_targets(MAX_TRACKERS)).into_iter().filter(|t| !skip.contains(t)).collect();
            running.retain(|url, task| {
                let keep = want.contains(url);
                if !keep {
                    task.abort();
                    println!("tracker {url}: no community this node carries names it any more; stopped announcing there");
                }
                keep
            });
            for url in want {
                if running.contains_key(&url) {
                    continue;
                }
                println!("tracker {url}: a community this node carries names it; announcing this node there");
                running.insert(url.clone(), index::spawn(shared.clone(), IndexConfig { url, carry_limit, mirror: false }));
            }
        }
    })
}
