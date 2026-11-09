use super::*;
use crate::drop_share::{Capability, cap_id};
use hoi_crypto::scatter as sc;
use hoi_types::scatter::{Class, Manifest};

fn manifest() -> Manifest {
    let m = Manifest {
        object: [1; 32],
        sealed: [2; 32],
        class: Class::from_index(0).unwrap(),
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
    };
    assert!(Manifest::decode(&m.encode()).is_ok(), "fixture is a valid wire manifest");
    m
}

fn remote_cap(m: &Manifest) -> Capability {
    let tokens = (0..m.hashes.len())
        .map(|i| {
            let locator = sc::locator(&m.loc_secret, &m.object, m.generation, i as u8);
            (Id(locator), sc::drop_token(&[123; 32], &locator))
        })
        .collect();
    Capability { manifest: hoi_types::uri::b64(&m.encode()), tokens }
}

fn attach(n: &mut Node, community: &str, target: &str, m: &Manifest) {
    n.scatter.saved.kept.insert(
        Id(m.object),
        crate::keep::Kept {
            community: Some(Id::from_hex(community).unwrap()),
            records: vec![Id::from_hex(target).unwrap()],
            expires_at: m.expires_at,
            gens: vec![crate::keep::Gen { manifest: hoi_types::uri::b64(&m.encode()), made_at: T0, confirmed: true, from: Some(Id([9; 32])) }],
            ..Default::default()
        },
    );
    n.scatter.dirty = true;
}

#[test]
fn deletion_reduces_saved_or_late_capabilities_to_holder_authority() {
    for late in [false, true] {
        let t = Arc::new(AtomicU64::new(T0));
        let memory = Mem::default();
        let mut n = node("desktop", &t, &memory);
        n.create_identity("Robin").unwrap();
        let c = n.create_community(draft("garden")).unwrap().id;
        let p = post(&mut n, &c, "capability cleanup");
        let m = manifest();
        let cap = remote_cap(&m);
        attach(&mut n, &c, &p.id, &m);
        if !late {
            n.save_drop_cap(cap.clone());
            assert!(n.scatter.saved.drop_caps.contains_key(&cap_id(&m)));
        }
        n.delete_own(&p.id).unwrap();
        let job = n.deletion_for_target(Id::from_hex(&p.id).unwrap()).unwrap();
        if late {
            assert!(n.deletions.saved.jobs[&job].targets.values().all(|target| target.token.is_none()));
            n.save_drop_cap(cap.clone());
        }
        assert!(!n.scatter.saved.drop_caps.contains_key(&cap_id(&m)), "captured authority no longer retains the content key");
        assert!(!n.scatter.saved.kept.contains_key(&Id(m.object)));
        assert_eq!(n.deletions.saved.jobs[&job].targets.len(), m.holders.len());
        for target in n.deletions.saved.jobs[&job].targets.values() {
            assert!(target.token == cap.tokens.get(&target.locator).copied(), "late arrival uses the sender's tokens, never this device's secret");
        }
        n.commit().unwrap();
        drop(n);
        let reopened = node("desktop", &t, &memory);
        assert!(!reopened.scatter.saved.drop_caps.contains_key(&cap_id(&m)));
        for target in reopened.deletions.saved.jobs[&job].targets.values() {
            assert!(target.token == cap.tokens.get(&target.locator).copied());
        }
    }
}

#[test]
fn capturing_a_different_placement_keeps_the_saved_capability_until_its_turn() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let p = post(&mut n, &c, "two known placement snapshots");
    let initial = manifest();
    attach(&mut n, &c, &p.id, &initial);
    let mut later = initial.clone();
    later.holders[0] = Id([8; 32]);
    let cap = remote_cap(&later);
    n.save_drop_cap(cap.clone());
    n.delete_own(&p.id).unwrap();
    let job = n.deletion_for_target(Id::from_hex(&p.id).unwrap()).unwrap();
    let targets = &n.deletions.saved.jobs[&job].targets;
    for (i, holder) in later.holders.iter().enumerate() {
        let locator = Id(sc::locator(&later.loc_secret, &later.object, later.generation, i as u8));
        assert!(targets.values().any(|t| t.holder == *holder && t.locator == locator && t.token == cap.tokens.get(&locator).copied()));
    }
    assert!(!n.scatter.saved.drop_caps.contains_key(&cap_id(&later)));
}

#[test]
fn a_late_capability_survives_capacity_pressure_and_is_retried_after_restart() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &t, &memory);
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let p = post(&mut n, &c, "late generation");
    let other = post(&mut n, &c, "other pending placement");
    let mut m = manifest();
    attach(&mut n, &c, &p.id, &m);
    n.delete_own(&p.id).unwrap();
    n.delete_own(&other.id).unwrap();
    let job = n.deletion_for_target(Id::from_hex(&p.id).unwrap()).unwrap();
    let reservation = n.deletion_for_target(Id::from_hex(&other.id).unwrap()).unwrap();
    n.deletions.saved.jobs.get_mut(&reservation).unwrap().reserved_targets = 65_536 - m.holders.len();
    n.deletions.dirty = true;
    m.generation = 1;
    let cap = remote_cap(&m);
    n.save_drop_cap(cap.clone());
    assert!(n.scatter.saved.drop_caps.contains_key(&cap_id(&m)), "a full journal must retain uncaptured authority");
    n.commit().unwrap();
    drop(n);

    let mut n = node("desktop", &t, &memory);
    let mut second = node("desktop", &t, &Mem::default());
    let request = second.link_request("second device").unwrap();
    let approval = n.link_approve(&request).unwrap();
    second.link_complete(&approval).unwrap();
    n.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    pair_now(&mut n, &mut second, "home", "second");
    pump_named(&mut n, "second", &mut second, "home");
    n.take_outbox();
    // Reopening already ran a sharing tick without peers; let its announcement become due.
    t.fetch_add(60, Ordering::SeqCst);
    n.drop_share_tick();
    assert!(n.take_outbox().iter().any(|(_, wire)| matches!(hoi_sync::Msg::decode(wire), Ok(hoi_sync::Msg::Link { .. }))), "the own-device route works");
    assert!(n.scatter.saved.drop_caps.contains_key(&cap_id(&m)));
    t.fetch_add(20, Ordering::SeqCst);
    n.drop_share_tick();
    assert!(
        !n.take_outbox().iter().any(|(_, wire)| matches!(hoi_sync::Msg::decode(wire), Ok(hoi_sync::Msg::Link { .. }))),
        "uncaptured deleted caps are not shared"
    );
    n.deletions.saved.jobs.get_mut(&reservation).unwrap().reserved_targets = 0;
    n.deletions.dirty = true;
    t.fetch_add(20, Ordering::SeqCst);
    n.drop_share_tick();
    assert!(!n.scatter.saved.drop_caps.contains_key(&cap_id(&m)), "tick retries retained caps without another pointer or packet");
    let targets = &n.deletions.saved.jobs[&job].targets;
    assert_eq!(targets.len(), 2 * m.holders.len());
    for (locator, token) in cap.tokens {
        assert!(targets.values().any(|target| target.locator == locator && target.token == Some(token)));
    }
}
