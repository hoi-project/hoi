use super::*;
use hoi_core::dto::SettingsPatch;
use hoi_store::{Change, Loaded, Persist};

struct FailingStore {
    fail: Arc<AtomicBool>,
    batches: Arc<Mutex<Vec<usize>>>,
}

impl Persist for FailingStore {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        self.batches.lock().unwrap().push(changes.len());
        if self.fail.load(Ordering::Relaxed) { Err("disk unavailable".into()) } else { Ok(()) }
    }

    fn load(&mut self) -> Result<Loaded, String> {
        Ok(Loaded::default())
    }
}

#[test]
fn storage_failure_cancels_network_and_recovers_without_stale_messages() {
    let fail = Arc::new(AtomicBool::new(false));
    let store = FailingStore { fail: fail.clone(), batches: Arc::default() };
    let node = Node::open(Box::new(store), "desktop", "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
    let heard: Arc<Mutex<Vec<Event>>> = Arc::default();
    let sink = heard.clone();
    let shared = Shared::new(node, Box::new(move |events| sink.lock().unwrap().extend(events)));
    let (mut rx, cancelled) = shared.register("peer");
    fail.store(true, Ordering::Relaxed);
    shared.with(|n| {
        n.set_settings(&SettingsPatch { relay: Some(false), ..Default::default() }).unwrap();
        n.net_connect("peer", Role::Peer);
    });
    assert!(rx.try_recv().is_err());
    assert!(*cancelled.borrow());
    assert_eq!(heard.lock().unwrap().len(), 1);
    assert!(matches!(&heard.lock().unwrap()[0], Event::Notice { text } if text.contains("could not save")));
    // Repeated failures keep the journal and do not flood the UI with the same warning.
    shared.with(|_| ());
    assert!(rx.try_recv().is_err());
    assert_eq!(heard.lock().unwrap().len(), 1);

    fail.store(false, Ordering::Relaxed);
    shared.with(|_| ());
    assert!(rx.try_recv().is_err(), "recovery must not flush messages for the cancelled connection");
    assert!(heard.lock().unwrap().iter().any(|e| matches!(e, Event::Notice { text } if text.contains("working again"))));
    shared.unregister("peer");
    let (mut fresh, _) = shared.register("fresh");
    assert!(shared.with_network(|n| n.net_connect("fresh", Role::Peer)).is_some());
    assert!(matches!(hoi_sync::Msg::decode(&fresh.try_recv().unwrap().bytes), Ok(hoi_sync::Msg::Hello { .. })));
}

#[test]
fn persistent_storage_failure_rejects_incoming_traffic_and_reconnects_without_journal_growth() {
    let fail = Arc::new(AtomicBool::new(false));
    let batches = Arc::new(Mutex::new(Vec::new()));
    let store = FailingStore { fail: fail.clone(), batches: batches.clone() };
    let node = Node::open(Box::new(store), "desktop", "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
    let shared = Shared::new(node, Box::new(|_| {}));
    batches.lock().unwrap().clear();
    fail.store(true, Ordering::Relaxed);
    shared.with(|n| n.set_settings(&SettingsPatch { relay: Some(false), ..Default::default() }).unwrap());
    let original = batches.lock().unwrap()[0];
    let incoming = hoi_sync::Msg::Bye { reason: "still sending".into() }.encode();
    for i in 0..1000 {
        let mut accepted = false;
        assert!(
            shared
                .with_network(|n| {
                    accepted = true;
                    n.net_receive("old", &incoming)
                })
                .is_none()
        );
        assert!(!accepted, "pending traffic must not enter the core during a storage outage");
        let peer = format!("retry-{i}");
        let (mut rx, _) = shared.register(&peer);
        assert!(shared.with_network(|n| n.net_connect(&peer, Role::Client)).is_none());
        assert!(rx.try_recv().is_err());
        shared.unregister(&peer);
    }
    assert!(shared.links.lock().unwrap().is_empty());
    assert!(shared.node.lock().unwrap().take_outbox().is_empty());
    assert!(batches.lock().unwrap().iter().all(|size| *size == original), "retries must keep exactly the original failed journal");
}
