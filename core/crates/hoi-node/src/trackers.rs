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

/// Whether `url` names this machine. A community's signed limits are chosen by its owners, so a tracker
/// there that points at loopback would make a public node send requests to services on its own host.
fn names_this_machine(url: &str) -> bool {
    let Some(host) = url::Url::parse(url).ok().and_then(|u| u.host().map(|h| h.to_owned())) else { return true };
    match host {
        url::Host::Domain(d) => d.eq_ignore_ascii_case("localhost") || d.to_ascii_lowercase().ends_with(".localhost"),
        url::Host::Ipv4(ip) => ip.is_loopback() || ip.is_unspecified(),
        url::Host::Ipv6(ip) => ip.is_loopback() || ip.is_unspecified(),
    }
}

/// The trackers to announce to: what the carried communities name, minus `skip` and minus any
/// that names this machine.
fn announce_targets(named: Vec<String>, skip: &[String]) -> BTreeSet<String> {
    named.into_iter().filter(|t| !skip.contains(t) && !names_this_machine(t)).collect()
}

/// Watches what the carried communities name; `skip`: registries already announced to
/// (`--index`).
pub fn spawn(shared: Arc<Shared>, skip: Vec<String>, carry_limit: usize) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut running: HashMap<String, JoinHandle<()>> = HashMap::new();
        let mut tick = tokio::time::interval(EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let want = announce_targets(shared.with(|n| n.tracker_targets(MAX_TRACKERS)), &skip);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tracker_that_names_this_machine_is_never_announced_to() {
        let named = [
            "http://127.0.0.1:4100",
            "http://localhost:4100/internal-admin",
            "http://[::1]:4100",
            "https://127.0.0.2",
            "https://LOCALHOST",
            "https://0.0.0.0",
            "https://tracker.example",
            "https://registry.example:8443/registry",
        ];
        let want = announce_targets(named.iter().map(|t| t.to_string()).collect(), &[]);
        assert_eq!(want, BTreeSet::from(["https://tracker.example".to_string(), "https://registry.example:8443/registry".to_string()]));
        let skipped = announce_targets(vec!["https://tracker.example".into()], &["https://tracker.example".to_string()]);
        assert!(skipped.is_empty(), "registries already given with --index are left to that loop");
    }
}
