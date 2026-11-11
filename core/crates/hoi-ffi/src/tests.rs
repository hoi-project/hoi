//! The FFI object as an app sees it: keys in the keystore, records in SQLite, events as JSON,
//! and a sync burst through a home node.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hoi_core::dto::{CommunityDraft, LimitsPatch, PostDraft};
use hoi_store::Persist;

use crate::keys::{KEYS, VAULT_KEY};
use crate::{EventListener, HoiError, HoiNode, KeyStore};

#[derive(Default)]
struct MemKeys(Mutex<HashMap<String, Vec<u8>>>);

impl KeyStore for MemKeys {
    fn get(&self, key: String) -> Option<Vec<u8>> {
        self.0.lock().unwrap().get(&key).cloned()
    }
    fn set(&self, key: String, value: Option<Vec<u8>>) {
        let mut m = self.0.lock().unwrap();
        match value {
            Some(v) => m.insert(key, v),
            None => m.remove(&key),
        };
    }
}

impl MemKeys {
    fn has_keys(&self) -> bool {
        self.0.lock().unwrap().contains_key(VAULT_KEY)
    }
}

#[derive(Default)]
struct Seen(Mutex<Vec<serde_json::Value>>);

impl EventListener for Seen {
    fn on_event(&self, json: String) {
        self.0.lock().unwrap().push(serde_json::from_str(&json).expect("events are JSON"));
    }
}

impl Seen {
    fn of_type(&self, t: &str) -> Vec<serde_json::Value> {
        self.0.lock().unwrap().iter().filter(|e| e["type"] == t).cloned().collect()
    }
}

fn temp_dir() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
    let d = std::env::temp_dir().join(format!("hoi-ffi-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn open(dir: &std::path::Path, keys: &Arc<MemKeys>, seen: &Arc<Seen>) -> Arc<HoiNode> {
    HoiNode::open(dir.to_string_lossy().into_owned(), keys.clone(), seen.clone()).unwrap()
}

fn draft(name: &str) -> CommunityDraft {
    CommunityDraft {
        name: name.into(),
        description: "Practical privacy for normal people.".into(),
        color: "mint".into(),
        limits: Some(LimitsPatch { pow_bits: Some(8), ..Default::default() }),
        rules: Some("1. Be kind.".into()),
        co_owners: None,
        threshold: None,
    }
}

fn post(community: &str, title: &str) -> PostDraft {
    PostDraft { community: community.into(), title: title.into(), text: "Short version: three apps.".into(), link: None, media: None }
}

#[test]
fn keys_go_to_the_keystore_and_records_to_sqlite() {
    let dir = temp_dir();
    let keys = Arc::new(MemKeys::default());
    let seen = Arc::new(Seen::default());
    let node = open(&dir, &keys, &seen);
    let boot = node.boot();
    assert_eq!(boot.platform, "mobile");
    assert!(!boot.has_identity);

    let me = node.create_identity("lena".into()).unwrap();
    assert!(keys.has_keys(), "database key went to the keystore");
    assert_eq!(me.devices[0].label, "This phone");
    let c = node.create_community(draft("privacy")).unwrap();
    let p = node.create_post(post(&c.id, "Which messengers survived?")).unwrap();
    let r = node.reply(p.id.clone(), "Signal, mostly.".into()).unwrap();
    node.edit(p.id.clone(), "Short version: two apps.".into(), None).unwrap();
    assert_eq!(node.vote(r.id.clone(), 1).unwrap(), 1);

    // the work bar's events, around each mined record
    let stamps = seen.of_type("stamp");
    assert_eq!(stamps.len(), 6, "start and done for the post, the reply and the edit");
    assert_eq!(stamps[0]["phase"], "start");
    assert_eq!(stamps[0]["bits"], 8);
    assert!(stamps[0]["estimateMs"].is_u64());
    assert_eq!(stamps[1]["phase"], "done");
    assert!(stamps[1]["tookMs"].is_u64());
    assert!(seen.of_type("changed").iter().any(|e| e["communities"][0] == c.id.as_str()));
    drop(node);

    // SQLite holds encrypted records and state, never plaintext identity or MLS keys
    let mut db = hoi_store::sqlite::Sqlite::open(&dir.join("hoi.db")).unwrap();
    let loaded = db.load().unwrap();
    assert!(loaded.records.len() >= 5);
    assert!(loaded.blobs.iter().any(|(k, _)| k == "v:state"));
    assert!(!loaded.blobs.iter().any(|(k, _)| k == KEYS));
    drop(db);

    // reopen: keys come back from the keystore, records from SQLite
    let node = open(&dir, &keys, &seen);
    let again = node.me().unwrap();
    assert_eq!(again.root_key, me.root_key);
    assert!(again.has_root);
    assert_eq!(node.communities().len(), 1);
    let view = node.post(p.id.clone()).unwrap();
    assert_eq!(view.text, "Short version: two apps.");
    assert_eq!(view.comments.len(), 1);
    assert_eq!(node.feed(None, Some("fresh".into()), None).unwrap().len(), 1);
    node.create_post(post(&c.id, "Still signing")).unwrap();
    drop(node);

    // Missing platform keys fail closed before replay, signing or database changes.
    let locked = Arc::new(MemKeys::default());
    let db = hoi_store::sqlite::Sqlite::open(&dir.join("hoi.db")).unwrap();
    let before = db.snapshot().unwrap();
    assert!(HoiNode::open(dir.to_string_lossy().into_owned(), locked.clone(), seen.clone()).is_err());
    assert_eq!(db.snapshot().unwrap(), before);
    assert!(locked.get(VAULT_KEY.into()).is_none());
    drop(db);
    // Once the existing key becomes readable, reopen the unchanged database.
    locked.set(VAULT_KEY.into(), keys.get(VAULT_KEY.into()));
    let restored = open(&dir, &locked, &seen);
    assert_eq!(restored.me().unwrap().root_key, me.root_key);
    drop(restored);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_report_json_keeps_its_order() {
    let mut r = hoi_core::dto::ImportReport { accepted: 1, duplicates: 0, rejected: 3, reasons: HashMap::new() };
    for k in ["stamp", "decode", "rate-limited", "banned"] {
        r.reasons.insert(k.into(), 1);
    }
    let s = serde_json::to_string(&r).unwrap();
    assert!(s.ends_with(r#""reasons":{"banned":1,"decode":1,"rate-limited":1,"stamp":1}}"#), "{s}");
}

#[test]
fn a_fresh_database_never_inherits_identity_and_wipe_keeps_storage_encrypted() {
    let keys = Arc::new(MemKeys::default());
    let seen = Arc::new(Seen::default());
    let first = temp_dir();
    let node = open(&first, &keys, &seen);
    node.create_identity("mira".into()).unwrap();
    drop(node);
    assert!(keys.has_keys());

    // the app was deleted and installed again: the Keychain kept the keys, the files are gone
    let old_vault = keys.get(VAULT_KEY.into()).unwrap();
    let second = temp_dir();
    let node = open(&second, &keys, &seen);
    assert_ne!(keys.get(VAULT_KEY.into()).unwrap(), old_vault, "fresh storage has an independent data key");
    assert!(node.me().is_none());

    node.create_identity("mira".into()).unwrap();
    assert!(keys.has_keys());
    node.wipe().unwrap();
    assert!(keys.has_keys(), "encryption stays enabled after deleting the identity");
    assert!(node.me().is_none());
    drop(node);
    for d in [first, second] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn the_guarded_clear_keeps_unfinished_deletions_and_their_encrypted_storage() {
    let dir = temp_dir();
    let keys = Arc::new(MemKeys::default());
    let seen = Arc::new(Seen::default());
    let node = open(&dir, &keys, &seen);
    node.create_identity("mira".into()).unwrap();
    let c = node.create_community(draft("garden")).unwrap();
    let p = node.create_post(post(&c.id, "deleted from the network")).unwrap();
    let before = node.cleanup_readiness().unwrap();
    assert!(before.ready && before.unlisted == 1);
    node.delete_own(p.id).unwrap();
    let after = node.cleanup_readiness().unwrap();
    assert!(!after.ready && after.blocked.iter().any(|b| b.reason == "in-effect"));
    for (inventory, deletions) in [(after.inventory.clone(), after.deletions.clone()), (before.inventory, before.deletions)] {
        assert!(matches!(node.clear_after_deletion(inventory, deletions), Err(HoiError::Api { code, .. }) if code == "cleanup-blocked"));
    }
    assert!(node.me().is_some(), "a refused clear keeps the identity");
    drop(node);
    let node = open(&dir, &keys, &seen);
    assert!(node.me().is_some());
    assert_eq!(node.cleanup_readiness().unwrap().deletions, after.deletions, "the deletion work survives a restart");
    // a local clear abandons it, and storage stays encrypted
    node.wipe().unwrap();
    assert!(keys.has_keys());
    assert!(node.me().is_none() && node.deletion_progress().unwrap().is_empty());
    drop(node);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn json_calls_and_errors() {
    let dir = temp_dir();
    let keys = Arc::new(MemKeys::default());
    let seen = Arc::new(Seen::default());
    let node = open(&dir, &keys, &seen);
    node.create_identity("tobi".into()).unwrap();
    let me: serde_json::Value = serde_json::from_str(&node.call("me".into(), "[]".into()).unwrap()).unwrap();
    assert_eq!(me["name"], "tobi");
    assert_eq!(node.call("setName".into(), r#"["tobi_k"]"#.into()).map(|s| s.contains("tobi_k")), Ok(true));
    assert!(matches!(node.call("nope".into(), "[]".into()), Err(HoiError::Api { code, .. }) if code == "invalid"));
    assert!(matches!(node.call("me".into(), "{".into()), Err(HoiError::Api { code, .. }) if code == "invalid"));
    let e = node.community("zz".into()).unwrap_err();
    assert_eq!(e.code(), "invalid");
    assert_eq!(e.to_string(), "That doesn't look like an id.");
    assert!(!node.start_home_link(), "nothing paired");
    assert!(!node.sync_burst(1));
    assert_eq!(node.net_status().label, "only on this phone");
    drop(node);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn syncs_in_bursts_through_a_home_node() {
    // the home node: a desktop app or hoi-node, serving paired devices on a WebSocket
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let mut home = hoi_core::Node::open(Box::new(hoi_store::Ephemeral), "desktop", "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
    home.create_identity("mira").unwrap();
    let c = home.create_community(draft("privacy")).unwrap();
    let p = home.create_post(&post(&c.id, "Start here")).unwrap();
    let invite = home.invite(&c.id).unwrap().uri;
    let home = hoi_net::Shared::new(home, Box::new(|_| {}));
    let _server = rt.block_on(hoi_net::serve(home.clone(), "127.0.0.1:0".parse().unwrap(), &[])).unwrap();
    let pairing = home.with(|n| n.net_status()).serving.unwrap().pairing;

    let dir = temp_dir();
    let keys = Arc::new(MemKeys::default());
    let seen = Arc::new(Seen::default());
    let phone = open(&dir, &keys, &seen);
    phone.create_identity("lena".into()).unwrap();
    phone.join(invite).unwrap();
    let st = phone.pair(pairing).unwrap();
    assert_eq!(st.label, "looking for your home node");
    assert!(phone.last_synced_at().is_none());

    // background refresh: one burst, then the link is down again
    assert!(phone.sync_burst(15), "burst status: {:?}; notices: {:?}", phone.net_status(), seen.of_type("notice"));
    assert_eq!(phone.feed(None, None, None).unwrap().len(), 1, "the post came over");
    assert!(phone.last_synced_at().is_some());
    assert!(seen.of_type("net").iter().any(|e| e["status"]["mode"] == "home-node"));
    let mut down = false;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if !phone.net_status().home_node.unwrap().connected {
            down = true;
            break;
        }
    }
    assert!(down, "the burst closed its link");

    // foreground: the link stays up and replies go out live
    assert!(phone.start_home_link());
    let mut up = false;
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(50));
        if phone.net_status().mode == "home-node" {
            up = true;
            break;
        }
    }
    assert!(up, "linked to the home node");
    phone.reply(p.id.clone(), "hoi from my phone".into()).unwrap();
    let mut arrived = false;
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(50));
        if home.with(|n| n.post(&p.id).unwrap().comments.len()) == 1 {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "the reply reached the home node");

    // background again; then back to the foreground right away
    phone.stop_home_link();
    assert!(phone.start_home_link());
    phone.stop_home_link();
    let mut closed = false;
    for _ in 0..60 {
        std::thread::sleep(Duration::from_millis(100));
        if !phone.net_status().home_node.unwrap().connected && home.clients() == 0 {
            closed = true;
            break;
        }
    }
    assert!(closed, "nothing stays connected in the background");
    let synced = phone.last_synced_at();
    drop(phone);

    // "last synced" survives the app being killed
    let phone = open(&dir, &keys, &seen);
    assert_eq!(phone.last_synced_at(), synced);
    assert_eq!(phone.unpair().label, "only on this phone");
    assert!(phone.last_synced_at().is_none());
    drop(phone);
    let _ = std::fs::remove_dir_all(&dir);
}
