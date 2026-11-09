use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use hoi_core::dto::{CommunityDraft, LimitsPatch, PostDraft};
use hoi_core::{Node, Role};
use hoi_store::sqlite::Sqlite;
use hoi_store::{Change, Loaded, Persist};
use hoi_types::{Body, Id, Wire};

const NOW: u64 = 1_790_000_000;

struct DatabaseFile(PathBuf);

impl DatabaseFile {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!("hoi-deletion-persistence-{}-{}.sqlite", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test database: {error}"),
            }
        }
    }
}

impl Drop for DatabaseFile {
    fn drop(&mut self) {
        // Every connection is closed before this fixture, including SQLite's WAL sidecars.
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.0.as_os_str().to_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(path));
        }
    }
}

fn open_file(file: &DatabaseFile) -> Node {
    Node::open(Box::new(Sqlite::open(&file.0).unwrap()), "desktop", "https://example.com/", Box::new(|| NOW)).unwrap()
}

fn saved_post(node: &mut Node) -> String {
    node.set_identity_bits(8);
    node.create_identity("Robin").unwrap();
    let community = node
        .create_community(CommunityDraft {
            name: "garden".into(),
            description: "A community for persistence tests.".into(),
            color: "mint".into(),
            limits: Some(LimitsPatch { pow_bits: Some(8), ..Default::default() }),
            rules: None,
            co_owners: None,
            threshold: None,
        })
        .unwrap();
    let post = node
        .create_post(&PostDraft { community: community.id, title: "remove this".into(), text: "A saved test post.".into(), link: None, media: None })
        .unwrap();
    node.commit().unwrap();
    post.id
}

fn deletion_ids(snapshot: &Loaded, target: Id) -> Vec<Id> {
    snapshot
        .records
        .iter()
        .filter_map(|bytes| {
            let (wire, id) = Wire::decode(bytes).unwrap();
            (wire.record.parent == Some(target) && matches!(wire.record.body, Body::Delete)).then_some(id)
        })
        .collect()
}

#[derive(Clone)]
struct FailingStore {
    disk: Arc<Mutex<Sqlite>>,
    fail: Arc<AtomicBool>,
}

impl Persist for FailingStore {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        if self.fail.load(Ordering::Relaxed) { Err("disk unavailable".into()) } else { self.disk.lock().unwrap().apply(changes) }
    }

    fn load(&mut self) -> Result<Loaded, String> {
        self.disk.lock().unwrap().load()
    }
}

#[test]
fn failed_commit_retries_records_keys_and_latest_state_after_reopening() {
    let store = FailingStore { disk: Arc::new(Mutex::new(Sqlite::memory().unwrap())), fail: Arc::new(AtomicBool::new(false)) };
    let open = || Node::open(Box::new(store.clone()), "desktop", "https://hoi.sh/app/", Box::new(|| 1_790_000_000)).unwrap();
    let mut node = open();
    node.set_identity_bits(hoi_crypto::identity::MIN_BITS);
    let me = node.create_identity("Robin").unwrap();
    store.fail.store(true, Ordering::Relaxed);
    assert!(node.commit().is_err());
    assert!(node.commit().is_err());
    // Newer state must follow, not replace or precede, the failed record/key writes.
    node.set_name("Lena").unwrap();
    let records = node.stats().records;
    store.fail.store(false, Ordering::Relaxed);
    node.commit().unwrap();
    drop(node);

    let mut restored = open();
    let restored_me = restored.me().unwrap();
    assert_eq!(restored_me.root_key, me.root_key);
    assert_eq!(restored_me.name, "Lena");
    assert_eq!(restored.stats().records, records);
    assert!(restored_me.has_root);
    // Signing still works: the private keys survived the failed initial save too.
    restored.set_name("Robin again").unwrap();
    restored.commit().unwrap();
}

#[test]
fn offline_deletion_journal_and_suppression_survive_reopening_a_sqlite_file() {
    let file = DatabaseFile::new();
    let mut node = open_file(&file);
    let post = saved_post(&mut node);
    node.call("deleteOwn", serde_json::json!([post])).unwrap();
    let progress = node.deletion_progress();
    assert_eq!(progress.len(), 1);
    assert!(progress[0].local_removed && progress[0].requires_local_state);
    assert!(!progress[0].record_submitted, "an offline deletion is retained for later dissemination");
    assert_eq!(progress[0].acknowledged, 0);
    assert!(!progress[0].inventory_complete);
    let snapshot = node.persist_mut().load().unwrap();
    let deletes = deletion_ids(&snapshot, Id::from_hex(&post).unwrap());
    assert_eq!(deletes, vec![Id::from_hex(&progress[0].id).unwrap()]);
    assert!(snapshot.blobs.iter().any(|(key, _)| key == "deletion-journal"));
    drop(node);

    // This creates a new SQLite connection, not only a new Node over an in-memory store.
    let mut restored = open_file(&file);
    assert!(restored.post(&post).is_err());
    assert_eq!(restored.deletion_progress(), progress);
    restored.call("deleteOwn", serde_json::json!([post])).unwrap();
    assert_eq!(restored.deletion_progress(), progress, "duplicate requests keep the same durable work");
    let snapshot = restored.persist_mut().load().unwrap();
    assert_eq!(deletion_ids(&snapshot, Id::from_hex(&post).unwrap()), deletes);
}

#[test]
fn sqlite_deletion_write_failure_retries_authority_and_suppression_together() {
    for restart_before_retry in [false, true] {
        let file = DatabaseFile::new();
        let store = FailingStore { disk: Arc::new(Mutex::new(Sqlite::open(&file.0).unwrap())), fail: Arc::new(AtomicBool::new(false)) };
        let mut node = Node::open(Box::new(store.clone()), "desktop", "https://example.com/", Box::new(|| NOW)).unwrap();
        let post = saved_post(&mut node);
        let root = node.me().unwrap().root_key;
        let before = store.disk.lock().unwrap().load().unwrap();
        store.fail.store(true, Ordering::Relaxed);
        assert!(node.call("deleteOwn", serde_json::json!([post])).is_err());
        assert!(node.call("deleteOwn", serde_json::json!([post])).is_err());
        node.net_connect("offline", Role::Peer);
        assert!(node.take_outbox().is_empty(), "a failed deletion transaction releases no messages");
        assert!(node.deletion_progress().is_empty(), "failed writes are never reported as durable progress");
        let after = store.disk.lock().unwrap().load().unwrap();
        assert!(after.records == before.records && after.blobs == before.blobs, "the last committed SQLite snapshot remains intact");
        assert!(deletion_ids(&after, Id::from_hex(&post).unwrap()).is_empty());

        if restart_before_retry {
            // No write succeeded. A process crash must leave the original post and signing
            // authority available to retry the request on the next start.
            drop(node);
            drop(store);
            node = open_file(&file);
            assert!(node.post(&post).is_ok());
            assert!(node.deletion_progress().is_empty());
        } else {
            store.fail.store(false, Ordering::Relaxed);
            drop(store);
        }
        assert_eq!(node.me().unwrap().root_key, root);
        node.call("deleteOwn", serde_json::json!([post])).unwrap();
        let progress = node.deletion_progress();
        assert_eq!(progress.len(), 1);
        assert!(progress[0].local_removed && progress[0].requires_local_state);
        assert_eq!(progress[0].acknowledged, 0);
        assert!(node.post(&post).is_err());
        let snapshot = node.persist_mut().load().unwrap();
        assert_eq!(deletion_ids(&snapshot, Id::from_hex(&post).unwrap()), vec![Id::from_hex(&progress[0].id).unwrap()]);
        // Signing still works after both retry paths; the failed transaction did not strand
        // the identity while deleting its content and creating its durable deletion record.
        node.set_name("Robin again").unwrap();
        node.commit().unwrap();
        drop(node);

        let mut restored = open_file(&file);
        assert_eq!(restored.me().unwrap().root_key, root);
        assert_eq!(restored.me().unwrap().name, "Robin again");
        assert!(restored.post(&post).is_err());
        assert_eq!(restored.deletion_progress(), progress);
    }
}
