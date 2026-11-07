use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use hoi_core::Node;
use hoi_store::sqlite::Sqlite;
use hoi_store::{Change, Loaded, Persist};

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
