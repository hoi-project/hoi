use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Clone)]
struct Failing {
    memory: Mem,
    fail: Arc<AtomicBool>,
}

impl Persist for Failing {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        if self.fail.load(Ordering::SeqCst) { Err("storage unavailable".into()) } else { self.memory.apply(changes) }
    }
    fn load(&mut self) -> Result<Loaded, String> {
        self.memory.load()
    }
}

#[test]
fn offline_author_deletion_keeps_a_durable_job_and_signed_record_after_restart() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &t, &memory);
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let p = post(&mut n, &c, "remove this");
    n.commit().unwrap();
    n.delete_own(&p.id).unwrap();
    n.commit().unwrap();
    let deletion = n.store.all().find(|s| matches!(s.record.body, Body::Delete)).unwrap().id;
    assert!(memory.0.lock().unwrap().1.contains_key("deletion-journal"), "accepted deletion must persist its retry job");
    drop(n);
    let mut reopened = node("desktop", &t, &memory);
    assert!(reopened.store.contains(&deletion), "the signed deletion remains available to sync after its target was removed");
    assert!(reopened.state.skeletons.contains_key(&Id::from_hex(&p.id).unwrap()));
    reopened.delete_own(&p.id).unwrap();
    assert_eq!(reopened.store.all().filter(|s| matches!(s.record.body, Body::Delete)).count(), 1, "duplicate requests reuse work");
}

#[test]
fn failed_deletion_persistence_keeps_the_job_retryable_and_releases_no_traffic() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let fail = Arc::new(AtomicBool::new(false));
    let clock = t.clone();
    let mut n = Node::open(
        Box::new(Failing { memory: memory.clone(), fail: fail.clone() }),
        "desktop",
        "https://example.com/",
        Box::new(move || clock.load(Ordering::SeqCst)),
    )
    .unwrap();
    n.set_identity_bits(4);
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let p = post(&mut n, &c, "keep retry authority");
    n.commit().unwrap();
    fail.store(true, Ordering::SeqCst);
    assert!(n.call("deleteOwn", serde_json::json!([p.id])).is_err());
    n.net_connect("offline", Role::Peer);
    assert!(n.take_outbox().is_empty(), "no message escapes the failed deletion transaction");
    assert!(n.deletion_progress().is_empty(), "uncommitted work is not exposed as durable progress");
    assert!(memory.0.lock().unwrap().0.contains_key(&Id::from_hex(&p.id).unwrap()), "original durable state survived failure");
    fail.store(false, Ordering::SeqCst);
    n.call("deleteOwn", serde_json::json!([p.id])).unwrap();
    let progress = n.deletion_progress();
    assert_eq!(progress.len(), 1);
    assert!(progress[0].local_removed);
    assert!(!progress[0].record_submitted);
    assert!(progress[0].requires_local_state);
    drop(n);
    let reopened = node("desktop", &t, &memory);
    assert_eq!(reopened.deletion_progress(), progress);
}

#[test]
fn a_deletion_before_its_target_survives_restart_and_checks_ownership() {
    for forged in [false, true] {
        let t = Arc::new(AtomicU64::new(T0));
        let memory = Mem::default();
        let mut n = node("desktop", &t, &memory);
        n.create_identity("Robin").unwrap();
        let c = Id::from_hex(&n.create_community(draft("garden")).unwrap().id).unwrap();
        let author = Actor::new();
        let stranger = Actor::new();
        author.intro(&mut n, T0);
        stranger.intro(&mut n, T0);
        let (post_wire, target) = author.sign(rec(c, None, text_post("later", "arrives after deletion"), T0), 8);
        let deleter = if forged { &stranger } else { &author };
        let (delete_wire, deletion) = deleter.sign(rec(c, Some(target), Body::Delete, T0), 8);
        assert!(n.ingest(&delete_wire, &Source::Import).is_err(), "ownership cannot be established yet");
        n.commit().unwrap();
        drop(n);
        let mut n = node("desktop", &t, &memory);
        n.ingest(&post_wire, &Source::Import).unwrap();
        n.commit().unwrap();
        assert_eq!(n.store.contains(&target), forged, "only the original author can suppress the target");
        assert_eq!(n.store.contains(&deletion), !forged);
        assert_eq!(n.state.skeletons.contains_key(&target), !forged);
    }
}

#[test]
fn damaged_deletion_journal_is_rejected_without_overwriting_storage() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &t, &memory);
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let p = post(&mut n, &c, "remove this");
    n.delete_own(&p.id).unwrap();
    n.commit().unwrap();
    drop(n);
    let original = memory.0.lock().unwrap().1[crate::deletion::BLOB].clone();
    for field in ["version", "reserved_targets"] {
        let mut saved: serde_json::Value = serde_json::from_slice(&original).unwrap();
        if field == "version" {
            saved[field] = serde_json::json!(u32::MAX);
        } else {
            saved["jobs"].as_object_mut().unwrap().values_mut().next().unwrap()[field] = serde_json::json!(u64::MAX);
        }
        memory.0.lock().unwrap().1.insert(crate::deletion::BLOB.into(), serde_json::to_vec(&saved).unwrap());
        let before = memory.0.lock().unwrap().clone();
        let clock = t.clone();
        let error = match Node::open(Box::new(memory.clone()), "desktop", "https://example.com/", Box::new(move || clock.load(Ordering::SeqCst))) {
            Ok(_) => panic!("damaged journal must fail closed"),
            Err(error) => error,
        };
        assert_eq!(error.code, "storage-incompatible");
        assert_eq!(*memory.0.lock().unwrap(), before);
    }
}

#[test]
fn supported_snapshot_restore_quarantines_content_until_available_deletions_are_applied() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut current = node("desktop", &t, &memory);
    current.create_identity("Robin").unwrap();
    let c = current.create_community(draft("garden")).unwrap().id;
    let p = post(&mut current, &c, "stale snapshot");
    current.commit().unwrap();
    let snapshot = Mem(Arc::new(Mutex::new(memory.0.lock().unwrap().clone())));
    current.delete_own(&p.id).unwrap();
    current.commit().unwrap();
    let clock = t.clone();
    let mut restored =
        Node::open_restored(Box::new(snapshot.clone()), "desktop", "https://example.com/", Box::new(move || clock.load(Ordering::SeqCst))).unwrap();
    let target = Id::from_hex(&p.id).unwrap();
    assert!(!restored.forwardable(restored.store.get(&target).unwrap()));
    assert!(restored.post(&p.id).is_err(), "snapshot content remains quarantined locally too");
    current.net_connect("restored", Role::Peer);
    restored.net_connect("current", Role::Peer);
    pump_named(&mut current, "restored", &mut restored, "current");
    restored.scatter_tick();
    restored.commit().unwrap();
    assert!(!restored.store.contains(&target), "catch-up applied the available authorized deletion");
    assert!(restored.post(&p.id).is_err());
    t.store(T0 + 8 * 86_400, Ordering::SeqCst);
    let reopened = node("desktop", &t, &snapshot);
    assert!(!reopened.store.contains(&target), "expired payload never returns after suppression is collected");
}

#[test]
fn deletion_removes_edits_and_preserves_other_posts() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &t, &memory);
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let deleted = post(&mut n, &c, "old words");
    let retained = post(&mut n, &c, "keep these words");
    n.edit(&deleted.id, "changed words", None).unwrap();
    let target = Id::from_hex(&deleted.id).unwrap();
    let edits: Vec<Id> = n.store.children(&target).filter(|s| matches!(s.record.body, Body::Edit { .. })).map(|s| s.id).collect();
    assert!(!edits.is_empty());
    n.delete_own(&deleted.id).unwrap();
    n.commit().unwrap();
    for edit in edits {
        assert!(!n.store.contains(&edit));
    }
    assert!(n.post(&retained.id).is_ok());
    let n = node("desktop", &t, &memory);
    assert!(!n.store.contains(&target));
    assert!(n.store.contains(&Id::from_hex(&retained.id).unwrap()));
}

#[test]
fn unavailable_early_jobs_cannot_starve_later_deletions() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    for i in 0..5 {
        let p = post(&mut n, &c, &format!("offline deletion {i}"));
        n.delete_own(&p.id).unwrap();
    }
    for job in n.deletions.saved.jobs.values_mut() {
        let holder = Id([1; 32]);
        let locator = Id([2; 32]);
        let key = Id::hash(&[holder.0.as_slice(), locator.0.as_slice()].concat());
        job.targets.insert(
            key,
            crate::deletion::Target {
                holder,
                locator,
                expiry_day: (T0 / 86_400 + 7) as u32,
                token: Some([3; 32]),
                outcome: crate::deletion::Outcome::Queued,
                attempts: 0,
                last_attempt: None,
                next_retry: T0,
            },
        );
    }
    n.deletions.dirty = true;
    n.deletion_tick();
    assert_eq!(n.deletions.saved.jobs.values().filter(|j| j.targets.values().any(|t| t.next_retry > T0)).count(), 4);
    t.fetch_add(5, Ordering::SeqCst);
    n.deletion_tick();
    assert!(
        n.deletions.saved.jobs.values().all(|j| j.targets.values().all(|t| t.next_retry > T0)),
        "every job gets a turn even when the first four have no route"
    );
    assert!(n.deletions.saved.jobs.values().all(|j| j.targets.values().all(|t| t.attempts == 0)), "unsent work is not counted as submitted");
}

#[test]
fn deletion_does_not_drop_a_shared_object_before_its_other_record_arrives() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let p = post(&mut n, &c, "first in the batch");
    let target = Id::from_hex(&p.id).unwrap();
    n.delete_own(&p.id).unwrap();
    let kept = crate::keep::Kept { community: Some(Id::from_hex(&c).unwrap()), records: vec![target, Id([9; 32])], ..Default::default() };
    let manifest = deletion_manifest();
    assert!(n.queue_deletion_generation(&kept, &manifest, true).is_err(), "a missing carried record is not evidence of its deletion");
    assert!(n.deletions.saved.jobs.values().all(|j| j.targets.is_empty()));
}

fn deletion_manifest() -> hoi_types::scatter::Manifest {
    hoi_types::scatter::Manifest {
        object: [1; 32],
        sealed: [2; 32],
        class: hoi_types::scatter::Class::from_index(0).unwrap(),
        generation: 0,
        level: 10,
        places: 10,
        k: 3,
        n: 5,
        m: 7,
        hashes: vec![[1; 16]; 5],
        holders: (3..8).map(|i| Id([i; 32])).collect(),
        loc_secret: [6; 32],
        key: [7; 32],
        expires_at: T0 + 86_400,
        keys: Vec::new(),
    }
}

#[test]
fn unavailable_early_targets_cannot_starve_a_later_batch() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let p = post(&mut n, &c, "many offline holders");
    n.delete_own(&p.id).unwrap();
    let job = n.deletions.saved.jobs.values_mut().next().unwrap();
    for i in 1..=33 {
        let holder = Id([i; 32]);
        let locator = Id([2; 32]);
        let key = Id::hash(&[holder.0.as_slice(), locator.0.as_slice()].concat());
        job.targets.insert(
            key,
            crate::deletion::Target {
                holder,
                locator,
                expiry_day: (T0 / 86_400 + 7) as u32,
                token: Some([3; 32]),
                outcome: crate::deletion::Outcome::Queued,
                attempts: 0,
                last_attempt: None,
                next_retry: T0,
            },
        );
    }
    n.deletions.dirty = true;
    n.deletion_tick();
    t.fetch_add(5, Ordering::SeqCst);
    n.deletion_tick();
    assert!(n.deletions.saved.jobs.values().all(|j| j.targets.values().all(|t| t.next_retry > T0)));
}

#[test]
fn late_or_duplicate_deletion_results_cannot_rewrite_terminal_evidence() {
    use crate::deletion::{Outcome, Target};
    let mut target = Target {
        holder: Id([1; 32]),
        locator: Id([2; 32]),
        expiry_day: 10,
        token: Some([3; 32]),
        outcome: Outcome::Submitted,
        attempts: 1,
        last_attempt: Some(9 * 86_400),
        next_retry: 10 * 86_400,
    };
    target.observe(Outcome::Unknown, 10 * 86_400);
    assert!(target.outcome == Outcome::Unknown && target.token.is_some());
    target.observe(Outcome::LegacyRemoval, 10 * 86_400);
    assert!(target.outcome == Outcome::LegacyRemoval && target.token.is_some());
    let mut expired = target.clone();
    expired.observe(Outcome::Acknowledged, 11 * 86_400);
    assert!(expired.outcome == Outcome::Expired && expired.token.is_none());
    expired.observe(Outcome::Acknowledged, 11 * 86_400);
    assert!(expired.outcome == Outcome::Expired);
    target.observe(Outcome::Acknowledged, 11 * 86_400 - 1);
    assert!(target.outcome == Outcome::Acknowledged && target.token.is_none());
    target.observe(Outcome::Refused, 11 * 86_400);
    assert!(target.outcome == Outcome::Acknowledged && target.attempts == 1);
}

#[test]
fn full_deletion_journal_preserves_existing_authority_and_rejects_before_suppression() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let first = post(&mut n, &c, "existing cleanup");
    let second = post(&mut n, &c, "new cleanup");
    n.delete_own(&first.id).unwrap();
    let job = n.deletions.saved.jobs.values_mut().next().unwrap();
    job.reserved_targets = 65_536;
    let signed = job.record.clone();
    let manifest = deletion_manifest();
    n.scatter.saved.kept.insert(
        Id(manifest.object),
        crate::keep::Kept {
            community: Some(Id::from_hex(&c).unwrap()),
            records: vec![Id::from_hex(&second.id).unwrap()],
            gens: vec![crate::keep::Gen { manifest: hoi_types::uri::b64(&manifest.encode()), made_at: T0, confirmed: true, from: None }],
            ..Default::default()
        },
    );
    let error = n.delete_own(&second.id).unwrap_err();
    assert_eq!(error.code, "deletion-full");
    assert_eq!(n.deletions.saved.jobs.len(), 1);
    assert_eq!(n.deletions.saved.jobs.values().next().unwrap().record, signed);
    assert!(n.store.contains(&Id::from_hex(&second.id).unwrap()));
}
