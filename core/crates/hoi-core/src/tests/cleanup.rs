//! Clearing local data without stranding deletion work (RFC-0003). Local clearing sends nothing
//! and abandons work; clearing after a network deletion waits until the core says no saved
//! deletion still needs this device, and refuses when anything changed since the person looked.
use super::*;
use std::sync::atomic::AtomicBool;

fn id(hex: &str) -> Id {
    Id::from_hex(hex).unwrap()
}

fn reasons(r: &CleanupReadiness) -> BTreeMap<String, u32> {
    r.blocked.iter().map(|b| (b.reason.clone(), b.count)).collect()
}

fn deletion_records(n: &Node) -> usize {
    n.store.all().filter(|s| matches!(s.record.body, Body::Delete)).count()
}

/// Rounds of the scatter tick, which signs a few batch targets each time.
fn rounds(n: &mut Node, count: usize) {
    for _ in 0..count {
        n.scatter_tick();
        n.commit().unwrap();
    }
}

/// Storage that refuses writes while `fail` is set.
#[derive(Clone)]
struct Flaky(Mem, Arc<AtomicBool>);

impl Persist for Flaky {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        if self.1.load(Ordering::SeqCst) {
            return Err("disk unavailable".into());
        }
        self.0.apply(changes)
    }
    fn load(&mut self) -> Result<Loaded, String> {
        self.0.load()
    }
}

#[test]
fn clearing_local_data_sends_no_deletion_and_leaves_other_copies() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    let memory = Mem::default();
    let mut web = node("web", &t, &memory);
    home.create_identity("mira").unwrap();
    web.create_identity("lena").unwrap();
    let garden = home.create_community(draft("garden")).unwrap().id;
    pair(&mut home, &mut web, &garden);
    let theirs = post(&mut home, &garden, "Mira's post");
    pump(&mut home, &mut web);
    let mine = post(&mut web, &garden, "Lena's post");
    let reply = web.reply(&theirs.id, "Lena replies to Mira").unwrap();
    pump(&mut home, &mut web);
    let before = web.cleanup_readiness();
    assert!(before.ready && before.blocked.is_empty() && before.until.is_none(), "nothing to strand yet");
    assert_eq!(before.unlisted, 2, "both of Lena's records are known and covered by no deletion");

    web.wipe().unwrap();
    web.commit().unwrap();
    assert!(memory.0.lock().unwrap().0.is_empty() && web.me().is_none(), "the identity and records are gone here");
    let keys: serde_json::Value = memory.0.lock().unwrap().1.get("keys").map(|k| serde_json::from_slice(k).unwrap()).unwrap_or_default();
    assert!(keys.get("root").is_none_or(|v| v.is_null()) && keys.get("device").is_none_or(|v| v.is_null()), "no signing key left: {keys}");
    assert!(web.take_outbox().is_empty(), "a local clear sends nothing");
    pump(&mut home, &mut web);
    for kept in [&theirs.id, &mine.id, &reply.id] {
        assert!(home.store.contains(&id(kept)), "the other device keeps every copy, the cleared author's included");
        assert!(!home.state.skeletons.contains_key(&id(kept)));
    }
    assert_eq!(deletion_records(&home), 0, "no deletion was signed or sent");
    let after = web.cleanup_readiness();
    assert!(after.ready && after.unlisted == 0);
}

#[test]
fn network_deletion_blocks_the_combined_clear_until_its_work_ends() {
    let (t, mut n, cid, c) = owner();
    let x = Actor::new();
    x.intro(&mut n, T0);
    let theirs = x.send(&mut n, rec(cid, None, text_post("x writes", "a post by somebody else"), T0)).unwrap();
    let mine: Vec<PostSummary> = (0..3).map(|i| post(&mut n, &c, &format!("mira's post {i}"))).collect();
    let root = n.me().unwrap().root_key;
    let ready = n.cleanup_readiness();
    assert!(ready.ready);
    assert_eq!(ready.unlisted, 3);

    let preview = n.preview_bulk_deletion().unwrap();
    n.confirm_bulk_deletion(&preview.snapshot).unwrap();
    let confirmed = n.cleanup_readiness();
    assert!(!confirmed.ready);
    assert_eq!(reasons(&confirmed), BTreeMap::from([("unsigned".into(), 3)]), "confirmed, not signed yet");
    assert_eq!(confirmed.unlisted, 0, "listed in a deletion");
    assert_ne!(confirmed.inventory, ready.inventory, "the confirmed inventory moved on");
    let err = n.clear_after_deletion(&confirmed.inventory, &confirmed.deletions).unwrap_err();
    assert_eq!(err.code, "cleanup-blocked");

    rounds(&mut n, 1);
    assert_eq!(n.bulk_deletions()[0].signed, 3);
    let signed = n.cleanup_readiness();
    assert_eq!(reasons(&signed), BTreeMap::from([("in-effect".into(), 3)]), "signed deletions still need this device");
    let horizon = n.deletion_progress().iter().map(|p| p.expires_at).max().unwrap();
    assert_eq!(signed.until, Some(horizon), "known work ends at the deletions' horizon, if nothing new arrives");
    assert!(n.deletion_progress().iter().all(|p| p.requires_local_state), "the same signal RFC-0001 progress reports");
    assert_eq!(n.clear_after_deletion(&signed.inventory, &signed.deletions).unwrap_err().code, "cleanup-blocked");
    // network deletion keeps the identity and never touches other authors
    let me = n.me().unwrap();
    assert!(!me.burned && me.root_key == root, "the identity stays, unburned");
    assert!(n.store.contains(&theirs), "another author's post stays");
    assert_eq!(deletion_records(&n), 3);
    for p in &mine {
        assert!(n.state.skeletons.contains_key(&id(&p.id)));
    }

    // the horizon passes: nothing is left to strand
    t.store(horizon + 1, Ordering::SeqCst);
    let done = n.cleanup_readiness();
    assert!(done.ready && done.blocked.is_empty(), "{:?}", done.blocked);
    // a stale look is refused, and a new post moves the inventory on
    let late = post(&mut n, &c, "posted after the check");
    let moved = n.cleanup_readiness();
    assert!(moved.ready && moved.unlisted == 1);
    assert_eq!(n.clear_after_deletion(&done.inventory, &done.deletions).unwrap_err().code, "cleanup-changed");
    // a deletion racing the clear refuses it before any key goes
    n.delete_own(&late.id).unwrap();
    assert_eq!(n.clear_after_deletion(&moved.inventory, &moved.deletions).unwrap_err().code, "cleanup-blocked");
    assert_eq!(n.me().unwrap().root_key, root, "a refused clear changes nothing");
    assert_eq!(n.deletion_progress().len(), 4);
    for bad in ["", "not a revision"] {
        assert_eq!(n.clear_after_deletion(bad, &moved.deletions).unwrap_err().code, "invalid");
    }

    let expires = n.deletion_progress().iter().map(|p| p.expires_at).max().unwrap();
    t.store(expires + 1, Ordering::SeqCst);
    let fresh = n.cleanup_readiness();
    assert!(fresh.ready);
    n.clear_after_deletion(&fresh.inventory, &fresh.deletions).unwrap();
    n.commit().unwrap();
    assert!(n.me().is_none(), "cleared like a wipe");
    assert!(n.deletion_progress().is_empty() && n.bulk_deletions().is_empty());
}

#[test]
fn failed_storage_keeps_the_batch_retryable_and_refuses_the_clear() {
    let t = Arc::new(AtomicU64::new(T0));
    let fail = Arc::new(AtomicBool::new(false));
    let memory = Mem::default();
    let clock = t.clone();
    let mut n =
        Node::open(Box::new(Flaky(memory.clone(), fail.clone())), "web", "https://hoi.sh/app/", Box::new(move || clock.load(Ordering::SeqCst))).unwrap();
    n.set_identity_bits(4);
    n.create_identity("robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    post(&mut n, &c, "to be deleted");
    n.commit().unwrap();
    let ready = n.cleanup_readiness();
    assert!(ready.ready);

    fail.store(true, Ordering::SeqCst);
    let preview = n.preview_bulk_deletion().unwrap();
    assert!(n.confirm_bulk_deletion(&preview.snapshot).is_err(), "not reported as saved");
    assert!(n.bulk_deletions().is_empty(), "status never runs ahead of storage");
    assert!(!n.cleanup_readiness().ready, "the unsaved batch still counts as work");
    assert!(n.call("cleanupReadiness", serde_json::json!([])).is_err(), "hosts get no readiness while storage fails");
    assert!(n.clear_after_deletion(&ready.inventory, &ready.deletions).is_err());
    assert!(n.me().is_some(), "nothing cleared");

    fail.store(false, Ordering::SeqCst);
    let status = n.confirm_bulk_deletion(&preview.snapshot).unwrap();
    assert_eq!(status.count, 1, "the same snapshot is confirmed once storage works");
    drop(n);
    let reopened = node("web", &t, &memory);
    assert_eq!(reopened.bulk_deletions().len(), 1, "saved, and resumed after restart");
    assert_eq!(reasons(&reopened.cleanup_readiness()).len(), 1);
}

#[test]
fn a_deletion_waiting_for_the_computers_signature_blocks_the_clear() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("node", &t, &Mem::default());
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let mut desk = node("desktop", &t, &Mem::default());
    let mut browser = node("web", &t, &Mem::default());
    desk.create_identity("lena").unwrap();
    let c = desk.create_community(draft("privacy")).unwrap();
    let p = post(&mut desk, &c.id, "Start here");
    desk.pair(&home.net_status().serving.unwrap().pairing).unwrap();
    desk.net_connect("home", Role::Home);
    home.net_connect("d", Role::Client);
    pump_star(&mut home, &mut [("d", &mut desk)]);
    browser.pair(&desk.net_status().home_node.unwrap().pairing_web).unwrap();
    browser.net_connect("home", Role::Home);
    home.net_connect("b", Role::Client);
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    browser.link_home_remote("Work browser").unwrap();
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    let asks = link_asks(&mut desk);
    desk.link_answer(asks[0].id, true).unwrap();
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    assert!(browser.me().unwrap().remote);

    browser.delete_own(&p.id).unwrap();
    let waiting = browser.cleanup_readiness();
    assert_eq!(reasons(&waiting), BTreeMap::from([("unsigned".into(), 1)]), "the request exists nowhere else yet");
    assert_eq!(browser.clear_after_deletion(&waiting.inventory, &waiting.deletions).unwrap_err().code, "cleanup-blocked");
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    let signed = browser.cleanup_readiness();
    assert_eq!(reasons(&signed), BTreeMap::from([("in-effect".into(), 1)]), "signed by the computer, now a job here");
    assert_ne!(signed.deletions, waiting.deletions);
}

#[test]
fn someone_elses_deletion_blocks_only_while_this_device_holds_their_drops() {
    let (_t, mut n, cid, _) = owner();
    let x = Actor::new();
    x.intro(&mut n, T0);
    let theirs = x.send(&mut n, rec(cid, None, text_post("x writes", "soon deleted by x"), T0)).unwrap();
    let mut gone = rec(cid, Some(theirs), Body::Delete, T0 + 1);
    gone.expires_at = T0 + 86_400;
    let deletion = x.send(&mut n, gone).unwrap();
    assert!(n.state.skeletons.contains_key(&theirs));
    let job = n.deletions.saved.jobs.get(&deletion).expect("their deletion is in the journal");
    assert!(job.needs_local_state(n.now()) && job.author != id(&n.me().unwrap().root_key));
    let ready = n.cleanup_readiness();
    assert!(ready.ready, "their deletion travels without this device: {:?}", ready.blocked);
    assert!(n.deletion_progress().is_empty(), "progress lists the person's own deletions only");

    // this device repaired x's post once, so it alone holds drop authority for that generation
    let holder = Id([5; 32]);
    let locator = Id([6; 32]);
    let day = ((T0 + 86_400) / 86_400) as u32 + 1;
    n.deletions.saved.jobs.get_mut(&deletion).unwrap().targets.insert(
        Id([7; 32]),
        crate::deletion::Target {
            holder,
            locator,
            expiry_day: day,
            token: Some([9; 32]),
            outcome: crate::deletion::Outcome::Submitted,
            attempts: 1,
            last_attempt: Some(T0),
            next_retry: T0 + 60,
        },
    );
    let holding = n.cleanup_readiness();
    assert_eq!(reasons(&holding), BTreeMap::from([("holder-drops".into(), 1)]));
    assert_eq!(holding.until, Some((day as u64 + 1) * 86_400));
    assert_ne!(holding.deletions, ready.deletions);

    let now = n.now();
    n.deletions.saved.jobs.get_mut(&deletion).unwrap().targets.get_mut(&Id([7; 32])).unwrap().observe(crate::deletion::Outcome::Acknowledged, now);
    assert!(n.cleanup_readiness().ready, "acknowledged: nothing of theirs is left only here");
}
