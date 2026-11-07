//! Mirroring another registry (`--registry-mirror URL`): every five minutes this registry pages
//! through the other one's current listings and imports every version-bound proof it can verify
//! itself, when the name and the key are both still free here. A name already taken here by
//! another key is left alone: the local first claim stands, and both communities exist under
//! their distinct keys. This is how a second registry holds the names when the first stops.
//! Announced nodes are not mirrored; a public node announces to each registry it is given.
//!
//! A mirror is also the other registry's auditor. Each pass it checks the other's signed
//! log head, reads every new entry of its log, keeps every leaf, recomputes the root, and checks
//! that no name ever moved to another key and no key took a second name. Only listings that are
//! in that log are imported. A registry whose log changed its key, shrank, does not match its
//! head, or moved a name is no longer mirrored, and the operator is told once; `hoi-node admin
//! registry unwitness URL` starts the audit over.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use tokio::task::JoinHandle;

use super::Registry;
use super::log::{MAX_LOG_ENTRIES, entry_text, head_statement, leaf_hash, tree_root};
use super::protocol::verified_claim;
use crate::http;

const MIRROR_EVERY: Duration = Duration::from_secs(5 * 60);
const FIRST_PASS: Duration = Duration::from_secs(5);
/// Names per page the registry returns; fewer means the last page.
const PAGE: usize = 50;
/// Pages per pass: 2,000 names.
const MAX_PAGES: usize = 40;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listing {
    name: String,
    community: String,
    #[serde(default)]
    protocol_version: Option<String>,
    #[serde(default)]
    index_version: Option<u64>,
    #[serde(default)]
    proof: Option<Proof>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Proof {
    version: u64,
    signature: String,
}

/// Log pages read per pass: 10,240 entries; a longer log is caught up over several passes.
const MAX_LOG_PAGES: u64 = 40;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Head {
    log_key: String,
    size: u64,
    root: String,
    timestamp: u64,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct Entries {
    entries: Vec<Option<String>>,
    leaves: Vec<String>,
}

/// The name and key an entry's text holds, when its claim checks out.
fn entry_claim(text: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = text.split('\n').collect();
    let ["hoi name log v1", version, name, community, protocol, index, signature] = lines.as_slice() else { return None };
    let body = match *version {
        "1" => json!({ "name": name, "community": community, "signature": signature }),
        "2" => json!({ "name": name, "community": community, "protocolVersion": protocol, "indexVersion": index.parse::<u64>().ok()?, "signature": signature }),
        _ => return None,
    };
    let claim = verified_claim(&body, if *version == "2" { 2 } else { 1 })?;
    // The text must be exactly what this claim logs as, nothing else.
    let index = claim.protocol_version.as_deref().zip(claim.index_version.map(i64::from));
    (entry_text(if *version == "2" { 2 } else { 1 }, &claim.name, &claim.community, index, &claim.signature) == text).then_some((claim.name, claim.community))
}

fn head_checks(head: &Head) -> bool {
    let Ok(key) = hoi_types::Id::from_hex(&head.log_key) else { return false };
    let Ok(sig) = hoi_types::hex_decode(&head.signature) else { return false };
    let Ok(sig) = <[u8; 64]>::try_from(sig.as_slice()) else { return false };
    head.root.len() == 64 && hoi_crypto::verify(&key, head_statement(&head.log_key, head.size, &head.root, head.timestamp).as_bytes(), &hoi_types::Sig(sig))
}

/// One audit pass over `source`'s log: the leaves of every entry it has shown, once the log
/// checks out, or why it does not (which also stops the mirror until the operator says so).
async fn audit(client: &reqwest::Client, registry: &Registry, source: &str) -> Result<HashSet<String>, String> {
    {
        let store = registry.store();
        store.witness_tables().map_err(|e| e.to_string())?;
        if let Some(fault) = store.witnessed(source).map_err(|e| e.to_string())?.and_then(|w| w.fault) {
            return Err(fault);
        }
    }
    let head: Head = http::get_json(client, source, "/v2/log/head").await?.ok_or("it keeps no log of names")?;
    if !head_checks(&head) {
        return Err("its log head is not signed by the key it names".into());
    }
    let fault = |registry: &Registry, why: String| -> String {
        let _ = registry.store().witness_head(source, &head.log_key, head.size, &head.root, Some(&why));
        why
    };
    let (saved, mut leaves) = {
        let store = registry.store();
        (store.witnessed(source).map_err(|e| e.to_string())?, store.witnessed_leaves(source).map_err(|e| e.to_string())?)
    };
    if let Some(saved) = &saved {
        if saved.log_key != head.log_key {
            return Err(fault(registry, "its log changed its key".into()));
        }
        if head.size < saved.size {
            return Err(fault(registry, format!("its log shrank from {} to {} entries", saved.size, head.size)));
        }
    }
    let mut pages = 0;
    while (leaves.len() as u64) < head.size && pages < MAX_LOG_PAGES {
        let start = leaves.len() as u64;
        let end = head.size.min(start + MAX_LOG_ENTRIES);
        let page: Entries = http::get_json(client, source, &format!("/v2/log/entries?start={start}&end={end}")).await?.ok_or("its log has no such entries")?;
        if page.leaves.len() as u64 != end - start || page.entries.len() != page.leaves.len() {
            return Err("its log answered with the wrong number of entries".into());
        }
        let mut store = registry.store();
        for (i, (entry, leaf)) in page.entries.iter().zip(&page.leaves).enumerate() {
            let named = match entry {
                Some(text) => {
                    if hoi_types::hex_encode(&leaf_hash(text)) != *leaf {
                        drop(store);
                        return Err(fault(registry, "an entry of its log does not hash to its leaf".into()));
                    }
                    let Some((name, community)) = entry_claim(text) else {
                        drop(store);
                        return Err(fault(registry, "its log holds an entry no community key signed".into()));
                    };
                    let (by_name, by_key) = store.witnessed_name(source, &name, &community).map_err(|e| e.to_string())?;
                    if by_name.as_ref().is_some_and(|c| *c != community) || by_key.as_ref().is_some_and(|n| *n != name) {
                        drop(store);
                        return Err(fault(registry, format!("its log moved c/{name} to another key, or gave a key a second name")));
                    }
                    Some((name, community))
                }
                None => None, // withheld by its operator: the leaf alone
            };
            store.witness_leaf(source, start + i as u64, leaf, named.as_ref().map(|(n, c)| (n.as_str(), c.as_str()))).map_err(|e| e.to_string())?;
            leaves.push(leaf.clone());
        }
        pages += 1;
    }
    if leaves.len() as u64 == head.size {
        let parsed: Option<Vec<super::log::Hash>> = leaves.iter().map(|l| hoi_types::hex_decode(l).ok().and_then(|b| b.try_into().ok())).collect();
        let Some(parsed) = parsed else { return Err(fault(registry, "its log's leaves are damaged here".into())) };
        if hoi_types::hex_encode(&tree_root(&parsed)) != head.root {
            return Err(fault(registry, "its log does not match the head it signed".into()));
        }
        registry.store().witness_head(source, &head.log_key, head.size, &head.root, None).map_err(|e| e.to_string())?;
    }
    Ok(leaves.into_iter().collect())
}

/// The leaf a listing's publication has in its registry's log.
fn listing_leaf(listing: &Listing) -> Option<String> {
    let proof = listing.proof.as_ref()?;
    let index = listing.protocol_version.as_deref().zip(listing.index_version.map(|i| i as i64));
    Some(hoi_types::hex_encode(&leaf_hash(&entry_text(proof.version as u8, &listing.name, &listing.community, index, &proof.signature))))
}

/// Imports what `listings` prove, returns how many names were new here.
fn import(registry: &Registry, listings: Vec<Listing>) -> usize {
    let Ok(release) = registry.release.load() else { return 0 };
    let mut imported = 0;
    let mut store = registry.store();
    for listing in listings {
        let (Some(proof), Some(protocol), Some(index)) = (listing.proof, listing.protocol_version, listing.index_version) else { continue };
        if proof.version != 2 {
            continue;
        }
        let body =
            json!({ "name": listing.name, "community": listing.community, "protocolVersion": protocol, "indexVersion": index, "signature": proof.signature });
        let Some(claim) = verified_claim(&body, 2) else { continue };
        if matches!(store.find(&claim.name), Ok(Some(_))) {
            continue;
        }
        if store.publish(&claim, &release).is_ok_and(|outcome| outcome.status == 200) {
            imported += 1;
        }
    }
    imported
}

async fn pass(client: &reqwest::Client, base: &str) -> Result<Vec<Listing>, String> {
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let path = match &after {
            Some(name) => format!("/v2/communities?prefix=&after={name}"),
            None => "/v2/communities?prefix=".to_string(),
        };
        let page: Vec<Listing> = http::get_json(client, base, &path).await?.unwrap_or_default();
        let count = page.len();
        after = page.last().map(|l| l.name.clone());
        out.extend(page);
        if count < PAGE || after.is_none() {
            break;
        }
    }
    Ok(out)
}

/// Runs the mirror loop for one other registry until dropped.
pub fn spawn(registry: Arc<Registry>, url: String) -> JoinHandle<()> {
    tokio::spawn(async move {
        let client = match http::client() {
            Ok(client) => client,
            Err(e) => {
                eprintln!("hoi-node: the registry mirror client could not start: {e}");
                return;
            }
        };
        let mut last_error: Option<String> = None;
        tokio::time::sleep(FIRST_PASS).await;
        loop {
            let audited = audit(&client, &registry, &url).await;
            let listings = match audited {
                Ok(logged) => pass(&client, &url)
                    .await
                    .map(|listings| listings.into_iter().filter(|l| listing_leaf(l).is_some_and(|leaf| logged.contains(&leaf))).collect()),
                Err(e) => Err(format!("not mirrored: {e}")),
            };
            match listings {
                Ok(listings) => {
                    last_error = None;
                    let imported = import(&registry, listings);
                    if imported > 0 {
                        println!("registry: mirrored {imported} new name{} from {url}", if imported == 1 { "" } else { "s" });
                    }
                }
                Err(e) => {
                    if last_error.as_deref() != Some(e.as_str()) {
                        println!("registry: mirror of {url}: {e}; retrying");
                        last_error = Some(e);
                    }
                }
            }
            tokio::time::sleep(MIRROR_EVERY).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::RegistryConfig;
    use crate::registry::release::ReleaseSource;

    fn listing(name: &str, seed: u8, protocol: &str, index: u64, version: u64) -> Listing {
        let kp = hoi_crypto::Keypair::from_secret(&[seed; 32]);
        let statement = if version == 2 {
            hoi_core::community_publication_statement(name, &kp.public(), protocol, index as u32)
        } else {
            hoi_core::community_registration_statement(name, &kp.public())
        };
        let signature = hoi_types::hex_encode(&kp.sign(statement.as_bytes()).0);
        Listing {
            name: name.into(),
            community: kp.public().hex(),
            protocol_version: Some(protocol.into()),
            index_version: Some(index),
            proof: Some(Proof { version, signature }),
        }
    }

    #[test]
    fn only_verified_version_bound_proofs_are_imported_and_local_first_claims_stand() {
        let dir = std::env::temp_dir().join(format!("hoi-mirror-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<8>().unwrap())));
        std::fs::create_dir(&dir).unwrap();
        let config = RegistryConfig::new(ReleaseSource::compiled().unwrap());
        let registry = Registry::open(&dir, &config).unwrap();
        let (protocol, index) = hoi_core::release_versions();
        // A name this registry already arbitrated for another key.
        let local = listing("garden", 1, &protocol, index as u64, 2);
        assert_eq!(import(&registry, vec![local]), 1);
        let mut forged = listing("orchard", 2, &protocol, index as u64, 2);
        forged.name = "orchard2".into();
        let listings = vec![
            listing("garden", 3, &protocol, index as u64, 2), // the same name under another key elsewhere: left alone
            listing("meadow", 4, &protocol, index as u64, 2), // new: imported
            listing("legacy", 5, &protocol, index as u64, 1), // a v1 proof is not a publication
            forged,                                           // a listing whose signature does not match
            listing("meadow", 4, &protocol, index as u64, 2), // the same again: nothing new
        ];
        assert_eq!(import(&registry, listings), 1);
        let store = registry.store();
        assert_eq!(store.find("garden").unwrap().unwrap()["community"], json!(hoi_crypto::Keypair::from_secret(&[1; 32]).public().hex()));
        assert!(store.find("meadow").unwrap().is_some());
        assert!(store.find("legacy").unwrap().is_none());
        assert!(store.find("orchard2").unwrap().is_none());
        drop(store);
        // Close the registry before deleting its files: Windows refuses to remove open files, and
        // the directory would not be empty.
        drop(registry);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hoi-{tag}-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<8>().unwrap())));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    fn publish(registry: &Registry, name: &str, seed: u8) {
        let (protocol, index) = hoi_core::release_versions();
        let l = listing(name, seed, &protocol, index as u64, 2);
        let body =
            json!({ "name": l.name, "community": l.community, "protocolVersion": protocol, "indexVersion": index, "signature": l.proof.unwrap().signature });
        let claim = verified_claim(&body, 2).unwrap();
        assert_eq!(registry.store().publish(&claim, &registry.release.load().unwrap()).unwrap().status, 200);
    }

    async fn serve(dir: &std::path::Path, at: &str) -> (Arc<Registry>, tokio::task::JoinHandle<()>, String) {
        let registry = Registry::open(dir, &RegistryConfig::new(ReleaseSource::compiled().unwrap())).unwrap();
        let (task, addr) = crate::registry::serve(registry.clone(), at.parse().unwrap()).await.unwrap();
        (registry, task, format!("http://{addr}"))
    }

    #[tokio::test]
    async fn a_mirror_audits_the_log_and_stops_at_a_rewritten_one() {
        let (source_dir, mirror_dir) = (temp("source"), temp("mirror"));
        let client = http::client().unwrap();
        let mirror = Registry::open(&mirror_dir, &RegistryConfig::new(ReleaseSource::compiled().unwrap())).unwrap();
        let (source, task, url) = serve(&source_dir, "127.0.0.1:0").await;
        publish(&source, "garden", 1);
        publish(&source, "meadow", 2);
        let logged = audit(&client, &mirror, &url).await.unwrap();
        assert_eq!(logged.len(), 2);
        let listings = pass(&client, &url).await.unwrap();
        assert!(listings.iter().all(|l| listing_leaf(l).is_some_and(|leaf| logged.contains(&leaf))), "every listing is in the log");
        publish(&source, "orchard", 3);
        assert_eq!(audit(&client, &mirror, &url).await.unwrap().len(), 3, "the log grew; the audit goes on from where it was");

        // Its operator rewrites the log, the same entries in another order, and starts it again.
        task.abort();
        let _ = task.await;
        drop(source);
        rusqlite::Connection::open(source_dir.join("registry.db"))
            .unwrap()
            .execute_batch("UPDATE name_log SET idx = idx + 100; UPDATE name_log SET idx = 102 - idx;")
            .unwrap();
        let (_source, task, again) = serve(&source_dir, url.trim_start_matches("http://")).await;
        assert_eq!(again, url);
        let caught = audit(&client, &mirror, &url).await.unwrap_err();
        assert!(caught.contains("does not match"), "{caught}");
        assert_eq!(audit(&client, &mirror, &url).await.unwrap_err(), caught, "and stays stopped");
        assert!(mirror.store().unwitness(&url).unwrap());
        assert_eq!(audit(&client, &mirror, &url).await.unwrap().len(), 3, "until the operator starts the audit over");
        task.abort();
        for dir in [source_dir, mirror_dir] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn a_name_that_moves_to_another_key_stops_the_mirror() {
        let (source_dir, mirror_dir) = (temp("source"), temp("mirror"));
        let client = http::client().unwrap();
        let mirror = Registry::open(&mirror_dir, &RegistryConfig::new(ReleaseSource::compiled().unwrap())).unwrap();
        let (source, task, url) = serve(&source_dir, "127.0.0.1:0").await;
        publish(&source, "garden", 1);
        assert_eq!(audit(&client, &mirror, &url).await.unwrap().len(), 1);
        // A registry that tells some people c/garden is another key has to log that too.
        let (protocol, index) = hoi_core::release_versions();
        let other = listing("garden", 9, &protocol, index as u64, 2);
        let text = entry_text(2, "garden", &other.community, Some((&protocol, index as i64)), &other.proof.unwrap().signature);
        {
            let conn = rusqlite::Connection::open(source_dir.join("registry.db")).unwrap();
            conn.execute("INSERT INTO name_log(idx, name, community, entry) VALUES (1, 'garden', ?1, ?2)", rusqlite::params![other.community, text]).unwrap();
        }
        let moved = audit(&client, &mirror, &url).await.unwrap_err();
        assert!(moved.contains("moved c/garden"), "{moved}");
        task.abort();
        for dir in [source_dir, mirror_dir] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
