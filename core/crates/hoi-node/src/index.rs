//! Mirroring a public index (`--index URL`). Three jobs, all against the registry's plain HTTPS
//! API: announce this node's public routes so apps can find it, carry every compatible open
//! community the index lists, and take a community its founder's browser offers as soon as the
//! index confirms its key. The core decides what to carry and signs what is announced; this
//! module only talks HTTP, on a schedule, and prints its own status, never a peer's address.
//! With `--index` given more than once, one of these loops runs per registry.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hoi_core::dto::NodeAnnouncement;
use hoi_net::Shared;
use serde::Deserialize;
use tokio::task::JoinHandle;

use crate::http;
use crate::routes::{announceable, loopback_last};

/// Routes and carried communities are re-announced at least this often; the registry forgets a
/// node after thirty minutes of silence.
const ANNOUNCE_EVERY: Duration = Duration::from_secs(5 * 60);
/// The whole index is paged through this often; newly published communities usually arrive
/// sooner, through the browser that offers them.
const MIRROR_EVERY: Duration = Duration::from_secs(5 * 60);
const TICK: Duration = Duration::from_secs(5);
/// A community a browser offers that the index does not list yet is asked about again after this.
const LOOKUP_RETRY: Duration = Duration::from_secs(15);
/// A key the index did not list is not asked about again for this long: a peer listing
/// thousands of unknown keys in its hello must not turn into thousands of registry requests.
const LOOKUP_MISS_RETRY: Duration = Duration::from_secs(10 * 60);
/// Lookups per tick, whatever peers offer; the rest wait their turn.
const LOOKUPS_PER_TICK: usize = 4;
/// Names per page the registry returns; fewer means the last page.
const PAGE: usize = 50;
/// Pages per mirror pass: 2,000 names.
const MAX_PAGES: usize = 40;

#[derive(Clone, Debug)]
pub struct IndexConfig {
    /// The registry base URL, e.g. `https://registry.example.org`.
    pub url: String,
    /// Communities this node carries at most, those still waiting for a member included.
    pub carry_limit: usize,
    /// Carry what the index lists and what browsers offer under a listed key. Off: only announce
    /// this node's routes (`--carry off`).
    pub mirror: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listing {
    name: String,
    community: String,
    #[serde(default)]
    protocol_version: Option<String>,
    #[serde(default)]
    index_version: Option<u32>,
    #[serde(default)]
    compatible: bool,
    #[serde(default)]
    indexed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    registry: MetaRegistry,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetaRegistry {
    api_version: u64,
}

struct Index {
    client: reqwest::Client,
    base: String,
    protocol: String,
    index_version: u32,
}

impl Index {
    fn new(base: String) -> Result<Index, String> {
        let (protocol, index_version) = hoi_core::release_versions();
        Ok(Index { client: http::client()?, base, protocol, index_version })
    }

    async fn check(&self) -> Result<(), String> {
        let meta: Meta =
            http::get_json(&self.client, &self.base, "/v1/meta").await?.ok_or_else(|| "the public index has no service information".to_string())?;
        if meta.registry.api_version != 2 {
            return Err(format!("the public index speaks API version {}, this node version 2", meta.registry.api_version));
        }
        Ok(())
    }

    /// A listing this build can carry: indexed for the same protocol family and index edition.
    fn carryable(&self, listing: &Listing) -> bool {
        listing.indexed
            && listing.compatible
            && listing.protocol_version.as_deref() == Some(self.protocol.as_str())
            && listing.index_version == Some(self.index_version)
            && listing.community.len() == 64
            && listing.community.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }

    /// Every carryable community the index lists, name included, up to [`MAX_PAGES`] pages.
    async fn listings(&self) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let path = match &after {
                Some(name) => format!("/v2/communities?prefix=&after={name}"),
                None => "/v2/communities?prefix=".to_string(),
            };
            let page: Vec<Listing> = http::get_json(&self.client, &self.base, &path).await?.unwrap_or_default();
            let count = page.len();
            after = page.last().map(|l| l.name.clone());
            out.extend(page.into_iter().filter(|l| self.carryable(l)).map(|l| (l.community, l.name)));
            if count < PAGE || after.is_none() {
                break;
            }
        }
        Ok(out)
    }

    async fn lookup(&self, community: &str) -> Result<Option<(String, String)>, String> {
        let listing: Option<Listing> = http::get_json(&self.client, &self.base, &format!("/v2/communities?community={community}")).await?;
        Ok(listing.filter(|l| self.carryable(l)).map(|l| (l.community, l.name)))
    }

    /// Whether the registry kept the announcement. A full registry answers 200 with
    /// `retained: false`: the node is not listed yet and should say so rather than claim it is.
    async fn announce(&self, announcement: &NodeAnnouncement) -> Result<bool, String> {
        let response =
            self.client.put(format!("{}/v2/nodes", self.base)).json(announcement).send().await.map_err(|_| "the public index is unreachable".to_string())?;
        let status = response.status().as_u16();
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default = "yes")]
            retained: bool,
            #[serde(default)]
            error: String,
        }
        fn yes() -> bool {
            true
        }
        // The registry's own error code, never its free text: it is printed, and it is not ours.
        let answer = http::bounded(response).await.ok().and_then(|body| serde_json::from_slice::<Answer>(&body).ok());
        match (status, answer.as_ref().map(|a| a.error.as_str()).unwrap_or("")) {
            (200, _) => Ok(answer.is_none_or(|a| a.retained)),
            (403, "announcer-not-allowed") => {
                Err("the public index lists only nodes its operator allowed; give them this node's endpoint key (hoi-node admin endpoint)".into())
            }
            (403, "announcer-refused") => Err("the public index's operator does not list this node".into()),
            (403, _) => Err("the public index does not take node announcements; ask its operator to list this node's routes".into()),
            (400, "wrong-registry") => {
                Err("the public index answers to another address than the one this node was given; use the URL its operator publishes".into())
            }
            (status, _) => Err(format!("the public index refused the announcement ({status})")),
        }
    }
}

/// Says something once per change, so a flapping connection doesn't fill the journal.
pub(crate) struct Once(pub(crate) Option<String>);
impl Once {
    pub(crate) fn say(&mut self, line: String) {
        if self.0.as_deref() != Some(line.as_str()) {
            println!("{line}");
            self.0 = Some(line);
        }
    }
}

/// Runs the index loop until dropped.
pub fn spawn(shared: Arc<Shared>, config: IndexConfig) -> JoinHandle<()> {
    tokio::spawn(async move {
        let dev = http::loopback_index(&config.url);
        let index = match Index::new(config.url.clone()) {
            Ok(index) => index,
            Err(e) => {
                eprintln!("hoi-node: the public index client could not start: {e}");
                return;
            }
        };
        // Each announcement is signed for this registry's origin, so it is no good anywhere else.
        let origin = match crate::registry::protocol::registry_origin(&config.url) {
            Ok(origin) => origin,
            Err(e) => {
                eprintln!("hoi-node: {e}");
                return;
            }
        };
        let mut status = Once(None);
        let mut announced: Option<(Instant, Vec<String>, usize)> = None;
        let mut mirrored: Option<Instant> = None;
        let mut lookups: HashMap<String, (Instant, bool)> = HashMap::new();
        let mut checked = false;
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if !checked {
                match index.check().await {
                    Ok(()) => checked = true,
                    Err(e) => {
                        status.say(format!("public index: {e}; retrying"));
                        continue;
                    }
                }
            }
            let (routes, joined) = shared.with(|n| {
                let st = n.net_status();
                let mut routes: Vec<String> = st.rtc.iter().filter(|r| announceable(r, dev)).cloned().collect();
                routes.extend(st.transport.iter().flat_map(|t| t.peers.iter()).filter(|r| announceable(r, dev)).cloned());
                (loopback_last(routes), st.communities.len())
            });
            let carried = joined;

            // 1. Announce when the routes or the carried set changed, and every five minutes anyway.
            let due = announced.as_ref().is_none_or(|(at, r, j)| at.elapsed() >= ANNOUNCE_EVERY || *r != routes || *j != joined);
            if due {
                if routes.is_empty() {
                    status.say("public index: no public route to announce yet (a public address or --rtc-advertise is needed)".into());
                } else {
                    match shared.with(|n| n.node_announcement(&origin, &routes)) {
                        Ok(announcement) => match index.announce(&announcement).await {
                            Ok(true) => {
                                status.say(format!(
                                    "public index: announced {} route{} and {} communit{} to {}",
                                    announcement.routes.len(),
                                    if announcement.routes.len() == 1 { "" } else { "s" },
                                    announcement.communities.len(),
                                    if announcement.communities.len() == 1 { "y" } else { "ies" },
                                    config.url
                                ));
                                announced = Some((Instant::now(), routes.clone(), joined));
                            }
                            Ok(false) => {
                                status.say(format!(
                                    "public index: {} has no room for another node right now; this node is not listed yet and keeps trying",
                                    config.url
                                ));
                                announced = Some((Instant::now(), routes.clone(), joined));
                            }
                            Err(e) => status.say(format!("public index: {e}")),
                        },
                        Err(e) => status.say(format!("public index: nothing announced: {e}")),
                    }
                }
            }
            if !config.mirror {
                continue;
            }

            // 2. Mirror: carry every compatible community the index lists, within the limit.
            if mirrored.is_none_or(|at| at.elapsed() >= MIRROR_EVERY) {
                mirrored = Some(Instant::now());
                match index.listings().await {
                    Ok(listings) => {
                        let names = carry_all(&shared, listings, config.carry_limit).await;
                        for name in names {
                            println!("public index: now carrying c/{name}");
                        }
                    }
                    Err(e) => status.say(format!("public index: {e}")),
                }
            }

            // 3. A community a browser offers that this node doesn't hold: carry it once the
            // index confirms its key, so a founder's first post lands here without pairing.
            let offered = shared.with(|n| n.offered_unknown());
            // Misses are remembered a while even after the peer is gone; a peer could list
            // thousands of keys, and each lookup is a request against the registry's budget.
            lookups.retain(|c, (at, miss)| offered.contains(c) || (*miss && at.elapsed() < LOOKUP_MISS_RETRY));
            let mut asked = 0;
            for community in offered {
                if carried >= config.carry_limit || asked >= LOOKUPS_PER_TICK {
                    break;
                }
                if lookups.get(&community).is_some_and(|(at, miss)| at.elapsed() < if *miss { LOOKUP_MISS_RETRY } else { LOOKUP_RETRY }) {
                    continue;
                }
                asked += 1;
                match index.lookup(&community).await {
                    Ok(Some((key, name))) => {
                        lookups.insert(community.clone(), (Instant::now(), false));
                        if shared.with(|n| n.carry(&key, Some(&name)).is_ok()) {
                            println!("public index: now carrying c/{name}");
                        }
                    }
                    Ok(None) => {
                        lookups.insert(community.clone(), (Instant::now(), true));
                    }
                    Err(e) => {
                        lookups.insert(community.clone(), (Instant::now(), false));
                        status.say(format!("public index: {e}"));
                    }
                }
            }
        }
    })
}

/// Listings handled per hold of the node's lock: an index pass can list 2,000 communities, and
/// peers, browsers and the sweeper must not wait for all of them.
const CARRY_CHUNK: usize = 32;

/// Carries listings not yet asked for, a chunk at a time, letting go of the node between chunks;
/// returns the names newly taken on.
async fn carry_all(shared: &Shared, listings: Vec<(String, String)>, limit: usize) -> Vec<String> {
    let mut taken = Vec::new();
    for chunk in listings.chunks(CARRY_CHUNK) {
        let (names, room) = shared.with(|n| carry_some(n, chunk, limit));
        taken.extend(names);
        if !room {
            break;
        }
        tokio::task::yield_now().await;
    }
    taken
}

/// Carries `listings` not yet asked for, up to `limit` communities in all; returns the names
/// newly taken on and whether there is room for more.
fn carry_some(n: &mut hoi_core::Node, listings: &[(String, String)], limit: usize) -> (Vec<String>, bool) {
    let mut carried = n.net_status().communities.len();
    let mut taken = Vec::new();
    for (community, name) in listings {
        if carried >= limit {
            break;
        }
        let known = n.public_join_status(community).map(|s| s.status).unwrap_or_default();
        if known != "unknown" {
            continue;
        }
        if n.carry(community, Some(name)).is_ok() {
            carried += 1;
            taken.push(name.clone());
        }
    }
    (taken, carried < limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listings_for_this_build_are_carried() {
        let index = Index::new("https://hoi.sh/registry".into()).unwrap();
        let (protocol, edition) = hoi_core::release_versions();
        let listing = |indexed: bool, protocol: &str, edition: u32| Listing {
            name: "garden".into(),
            community: "a".repeat(64),
            protocol_version: Some(protocol.into()),
            index_version: Some(edition),
            compatible: indexed,
            indexed,
        };
        assert!(index.carryable(&listing(true, &protocol, edition)));
        assert!(!index.carryable(&listing(false, &protocol, edition)));
        assert!(!index.carryable(&listing(true, "99.0", edition)));
        assert!(!index.carryable(&listing(true, &protocol, edition + 1)));
        let mut odd = listing(true, &protocol, edition);
        odd.community = "A".repeat(64);
        assert!(!index.carryable(&odd));
    }

    #[test]
    fn a_public_node_carries_an_index_without_a_persons_key() {
        // A public node has no identity: it stores and serves, never signs. Carrying must not
        // need one, so a fresh `--public` data directory can mirror an index right away.
        let mut node = hoi_core::Node::open(Box::new(hoi_store::Ephemeral), "node", "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
        assert!(node.me().is_none());
        let taken = carry_some(&mut node, &[("a".repeat(64), "garden".into()), ("b".repeat(64), "meadow".into())], 1);
        assert_eq!(taken, (vec!["garden".to_string()], false), "the carry limit holds");
        assert_eq!(node.public_join_status(&"a".repeat(64)).unwrap().status, "pending");
        assert_eq!(node.public_join_status(&"b".repeat(64)).unwrap().status, "unknown");
        assert!(carry_some(&mut node, &[("a".repeat(64), "garden".into())], 8).0.is_empty(), "already asked for");
        let mut browser = hoi_core::Node::open(Box::new(hoi_store::Ephemeral), "web", "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
        assert!(browser.carry(&"c".repeat(64), None).is_err(), "a browser cannot carry, with or without a key");
        assert!(browser.public_join(&"c".repeat(64), &[], None).is_err(), "a person joins with a key");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_whole_index_pass_lets_go_of_the_node_between_chunks() {
        let node = hoi_core::Node::open(Box::new(hoi_store::Ephemeral), "node", "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
        let shared = Shared::new(node, Box::new(|_| {}));
        let listings: Vec<(String, String)> = (0..100u32).map(|i| (format!("{:064x}", i + 1), format!("c{i}"))).collect();
        // Another task on the same thread gets the node while the pass is under way: it sees
        // the pass part done, which it could not if the pass held the lock throughout.
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let watcher = {
            let (shared, seen) = (shared.clone(), seen.clone());
            tokio::spawn(async move {
                for _ in 0..200 {
                    let carried = shared.with(|n| n.carried_count());
                    seen.lock().unwrap().push(carried);
                    tokio::task::yield_now().await;
                }
            })
        };
        tokio::task::yield_now().await;
        let taken = carry_all(&shared, listings, 80).await;
        assert_eq!(taken.len(), 80, "the limit holds across chunks");
        assert_eq!(shared.with(|n| n.carried_count()), 80);
        watcher.abort();
        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|c| *c > 0 && *c < 80), "another task held the node between chunks: {seen:?}");
    }
}
