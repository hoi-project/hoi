use super::*;

fn routes() -> Vec<String> {
    vec![format!("{}@203.0.113.10:7843", Id([7; 32]).hex()), format!("rtc:{}@203.0.113.10:7844", "q".repeat(43))]
}

#[test]
fn public_key_join_catches_up_from_restarted_carrier_with_founder_offline() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut carrier = node("desktop", &t, &memory);
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let pairing = carrier.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    founder.pair(&pairing).unwrap();
    founder.net_connect("home", Role::Home);
    carrier.net_connect("phone", Role::Client);
    pump_star(&mut carrier, &mut [("phone", &mut founder)]);
    let community = founder.create_community(draft("Garden")).unwrap();
    let message = post(&mut founder, &community.id, "Saved on the relay while Robin is offline");
    pump_star(&mut carrier, &mut [("phone", &mut founder)]);
    drop(founder);
    drop(carrier);

    // This new person has only a key and public routes: no invite, home pairing, registry
    // connection, copied identity, or copy of the founder's signed genesis.
    let mut carrier = node("desktop", &t, &memory);
    let mut reader = node("web", &t, &Mem::default());
    reader.create_identity("Lena").unwrap();
    let pending = reader.public_join(&community.id, &routes(), Some("garden")).unwrap();
    assert_eq!(pending.status, "pending");
    assert!(reader.communities().is_empty());
    assert_eq!(reader.bootstrap_hints()[0].0, routes()[0]);
    assert_eq!(reader.rtc_candidates(), vec![routes()[1].clone()]);
    carrier.net_connect("b", Role::Peer);
    reader.net_connect("a", Role::Peer);
    carrier.set_peer_meta("b", PeerMeta { transport: "webrtc".into(), incoming: true, ..Default::default() });
    reader.set_peer_meta("a", PeerMeta { transport: "webrtc".into(), ..Default::default() });
    pump(&mut carrier, &mut reader);
    assert_eq!(reader.public_join_status(&community.id).unwrap().status, "joined");
    assert_eq!(reader.feed(Some(&community.id), "fresh", None).unwrap()[0].id, message.id);
    assert!(reader.state.home.is_none());
    assert!(reader.state.public_joins.is_empty());
}

#[test]
fn pending_public_join_survives_restart_and_cancel_stops_late_data_and_routes() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("desktop", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let community = founder.create_community(draft("garden")).unwrap();
    let id = Id::from_hex(&community.id).unwrap();
    let genesis = founder.store.genesis_of(&id).unwrap().wire.clone();
    let memory = Mem::default();
    let mut reader = node("web", &t, &memory);
    reader.create_identity("Lena").unwrap();
    reader.public_join(&community.id, &routes(), Some("garden")).unwrap();
    reader.commit().unwrap();
    drop(reader);
    let mut reader = node("web", &t, &memory);
    assert_eq!(reader.public_join_status(&community.id).unwrap().status, "pending");
    // Routes from a registry were only on trial: they don't outlive the attempt.
    assert!(reader.rtc_candidates().is_empty() && reader.bootstrap_hints().is_empty());
    assert!(!String::from_utf8(memory.0.lock().unwrap().1["state"].clone()).unwrap().contains("203.0.113.10"));
    assert_eq!(reader.public_join(&community.id, &routes(), Some("garden")).unwrap().status, "pending");
    assert_eq!(reader.rtc_candidates().len(), 1);
    assert_eq!(reader.bootstrap_hints().len(), 1);
    reader.leave(&community.id).unwrap();
    assert_eq!(reader.public_join_status(&community.id).unwrap().status, "unknown");
    assert!(reader.bootstrap_hints().is_empty() && reader.rtc_candidates().is_empty());
    assert!(reader.ingest(&genesis, &Source::Peer { peer: "late".into(), live: false }).is_err());
    reader.commit().unwrap();
    let restarted = node("web", &t, &memory);
    assert_eq!(restarted.public_join_status(&community.id).unwrap().status, "unknown");
    assert!(restarted.communities().is_empty());
}

#[test]
fn public_join_rejects_signed_name_mismatch_and_retries_same_connected_carrier() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut carrier = node("desktop", &t, &Mem::default());
    carrier.create_identity("Robin").unwrap();
    let community = carrier.create_community(draft("garden")).unwrap();
    let message = post(&mut carrier, &community.id, "A real garden post");
    let memory = Mem::default();
    let mut reader = node("web", &t, &memory);
    reader.create_identity("Lena").unwrap();
    reader.public_join(&community.id, &routes(), Some("wrong-name")).unwrap();
    carrier.net_connect("b", Role::Peer);
    reader.net_connect("a", Role::Peer);
    pump(&mut carrier, &mut reader);
    let rejected = reader.public_join_status(&community.id).unwrap();
    assert_eq!(rejected.status, "rejected");
    assert!(rejected.reason.unwrap().contains("signed community name"));
    assert!(reader.communities().is_empty() && reader.bootstrap_hints().is_empty() && reader.rtc_candidates().is_empty());
    assert_eq!(node("web", &t, &memory).public_join_status(&community.id).unwrap().status, "rejected");

    reader.public_join(&community.id, &routes(), Some("Garden")).unwrap();
    pump(&mut carrier, &mut reader);
    assert_eq!(reader.public_join_status(&community.id).unwrap().status, "joined");
    assert_eq!(reader.feed(Some(&community.id), "fresh", None).unwrap()[0].id, message.id);
}

#[test]
fn public_join_never_admits_private_or_tampered_genesis() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("desktop", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let mut private = draft("private");
    private.limits.as_mut().unwrap().admission = Some("invite".into());
    let community = founder.create_community(private).unwrap();
    let id = Id::from_hex(&community.id).unwrap();
    let genesis = founder.store.genesis_of(&id).unwrap().wire.clone();
    let mut reader = node("web", &t, &Mem::default());
    reader.create_identity("Lena").unwrap();
    reader.public_join(&community.id, &routes(), None).unwrap();
    let mut tampered = genesis.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(reader.ingest(&tampered, &Source::Peer { peer: "untrusted".into(), live: false }).is_err());
    assert_eq!(reader.public_join_status(&community.id).unwrap().status, "pending", "unverified data cannot reject a request");
    assert!(reader.ingest(&genesis, &Source::Peer { peer: "carrier".into(), live: false }).is_err());
    assert_eq!(reader.public_join_status(&community.id).unwrap().status, "rejected");
    assert!(reader.public_join_status(&community.id).unwrap().reason.unwrap().contains("not open"));
    assert!(reader.communities().is_empty());
    // The public path does not alter the existing invite-based admission mechanism.
    reader.leave(&community.id).unwrap();
    reader.join(&founder.invite(&community.id).unwrap().uri).unwrap();
    assert_eq!(reader.public_join(&community.id, &[], None).unwrap_err().code, "not-allowed");
}

#[test]
fn public_join_retry_recovers_an_interrupted_inventory_without_reconnecting() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut carrier = node("desktop", &t, &Mem::default());
    carrier.create_identity("Robin").unwrap();
    let community = carrier.create_community(draft("garden")).unwrap();
    let mut reader = node("web", &t, &Mem::default());
    reader.create_identity("Lena").unwrap();
    reader.public_join(&community.id, &routes(), Some("garden")).unwrap();
    carrier.net_connect("b", Role::Peer);
    reader.net_connect("a", Role::Peer);
    // Deliver handshake/inventory, but interrupt the actual record response.
    for _ in 0..10 {
        for (_, bytes) in carrier.take_outbox() {
            if !matches!(hoi_sync::Msg::decode(&bytes), Ok(hoi_sync::Msg::Recs { .. })) {
                assert!(reader.net_receive("a", &bytes));
            }
        }
        for (_, bytes) in reader.take_outbox() {
            assert!(carrier.net_receive("b", &bytes));
        }
    }
    assert_eq!(reader.public_join_status(&community.id).unwrap().status, "pending");
    assert!(!reader.peers["a"].wants.is_empty());
    reader.public_join(&community.id, &routes(), Some("garden")).unwrap();
    pump(&mut carrier, &mut reader);
    assert_eq!(reader.public_join_status(&community.id).unwrap().status, "joined");
}

#[test]
fn public_join_validates_routes_identity_and_bounds_before_mutating_state() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut reader = node("web", &t, &Mem::default());
    let id = Id([1; 32]).hex();
    assert!(reader.public_join(&id, &[], None).is_err());
    reader.create_identity("Lena").unwrap();
    for bad in [
        "https://registry.test/".to_string(),
        format!("{}@0.0.0.0:7843", Id([7; 32]).hex()),
        format!("rtc:{}@203.0.113.10:0", "q".repeat(43)),
        "rtc:bad@203.0.113.10:7844".into(),
    ] {
        assert_eq!(reader.public_join(&id, &[bad], None).unwrap_err().code, "invalid");
    }
    assert!(reader.public_join(&id, &vec![routes()[0].clone(); 17], None).is_err());
    assert!(reader.public_join(&id, &[], Some("not a public name")).is_err());
    assert!(reader.state.public_joins.is_empty() && reader.state.joined.is_empty() && reader.state.bootstrap.is_empty());
    for i in 1..=16 {
        reader.public_join(&Id([i; 32]).hex(), &[], None).unwrap();
    }
    assert_eq!(reader.public_join(&Id([17; 32]).hex(), &[], None).unwrap_err().code, "not-allowed");
    reader.state.joined.insert(Id([18; 32]), crate::state::Joined { at: T0, pinning: false });
    assert_eq!(
        reader.public_join(&Id([18; 32]).hex(), &[], None).unwrap_err().code,
        "not-allowed",
        "an unverified subscription cannot bypass the pending limit"
    );
    reader.public_join(&id, &routes(), None).unwrap();
    reader.leave(&id).unwrap();
    reader.public_join(&Id([17; 32]).hex(), &[], None).unwrap();
    assert_eq!(reader.state.public_joins.len(), 16);
}
