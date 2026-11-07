use super::*;

#[test]
fn separate_people_discover_join_and_receive_posts_through_a_persistent_relay() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut home = node("desktop", &t, &memory);
    let mut phone = node("web", &t, &Mem::default());
    let mut lena = node("web", &t, &Mem::default());
    phone.create_identity("Robin").unwrap();
    lena.create_identity("Lena").unwrap();
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    home.set_rtc_routes(vec![format!("rtc:{}@192.168.1.20:7844", "q".repeat(43))]);
    for (device, peer) in [(&mut phone, "phone"), (&mut lena, "lena")] {
        pair_now(&mut home, device, "home", peer);
    }
    pump_star(&mut home, &mut [("phone", &mut phone), ("lena", &mut lena)]);
    assert!(lena.discover_communities().is_empty());

    // No community page has to be opened on any receiving device.
    let community = phone.create_community(draft("garden")).unwrap();
    let first = post(&mut phone, &community.id, "A post from Robin's phone");
    pump_star(&mut home, &mut [("phone", &mut phone), ("lena", &mut lena)]);
    let found = lena.discover_communities();
    assert_eq!(found.len(), 1, "an existing browser sees the new phone community without reconnecting");
    assert_eq!(found[0].id, community.id);
    assert!(!found[0].joined);
    assert!(lena.communities().is_empty(), "discovery doesn't subscribe or download posts");
    assert!(lena.store.get(&Id::from_hex(&first.id).unwrap()).is_none());
    assert!(!found[0].invite.contains(home.state.serve_token.as_ref().unwrap()));
    assert!(
        hoi_types::Invite::parse(&found[0].invite).unwrap().peers.iter().any(|p| p.starts_with("rtc:")),
        "a public join route is carried instead of the pairing secret"
    );

    lena.join(&found[0].invite).unwrap();
    pump_star(&mut home, &mut [("phone", &mut phone), ("lena", &mut lena)]);
    assert_eq!(lena.feed(None, "fresh", None).unwrap()[0].id, first.id);
    let live = post(&mut phone, &community.id, "A second post arrives live");
    pump_star(&mut home, &mut [("phone", &mut phone), ("lena", &mut lena)]);
    assert!(lena.store.contains(&Id::from_hex(&live.id).unwrap()));
    assert_ne!(phone.me().unwrap().root_key, lena.me().unwrap().root_key);

    // The writer and original reader close. The native node restarts from its durable store.
    home.net_disconnect("phone");
    home.net_disconnect("lena");
    home.commit().unwrap();
    drop(home);
    let mut relay = node("desktop", &t, &memory);
    let mut later = node("web", &t, &Mem::default());
    later.create_identity("Another browser").unwrap();
    later.join(&found[0].invite).unwrap();
    relay.net_connect("b", Role::Peer);
    relay.set_peer_meta("b", PeerMeta { transport: "webrtc".into(), incoming: true, ..Default::default() });
    later.net_connect("a", Role::Peer);
    later.set_peer_meta("a", PeerMeta { transport: "webrtc".into(), ..Default::default() });
    pump(&mut relay, &mut later);
    let posts = later.feed(Some(&community.id), "fresh", None).unwrap();
    assert_eq!(posts.len(), 2, "an unpaired reader catches up after the author is offline");
    assert!(posts.iter().any(|p| p.id == first.id) && posts.iter().any(|p| p.id == live.id));
}

#[test]
fn discovery_keeps_same_names_separate_and_remembers_disconnected_sources() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    let mut b = node("web", &t, &Mem::default());
    a.create_identity("Robin").unwrap();
    b.create_identity("Lena").unwrap();
    let ca = a.create_community(draft("garden")).unwrap();
    let cb = b.create_community(draft("garden")).unwrap();
    assert_ne!(ca.id, cb.id);
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    pump(&mut a, &mut b);
    let listings = b.discover_communities();
    assert_eq!(listings.len(), 2);
    assert!(listings.iter().all(|c| c.name == "garden"));
    assert_eq!(listings.iter().filter(|c| c.joined).count(), 1);
    b.net_disconnect("a");
    // The source is gone; its listing stays in the directory, no longer live.
    let after = b.discover_communities();
    assert_eq!(after.len(), 2);
    assert_eq!(after.iter().filter(|c| c.live).count(), 0);
    assert_eq!(after.iter().find(|c| c.id == ca.id).map(|c| c.joined), Some(false));
}

#[test]
fn discovery_excludes_private_communities_and_withdraws_changed_admission() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    let mut b = node("web", &t, &Mem::default());
    a.create_identity("Robin").unwrap();
    let open = a.create_community(draft("garden")).unwrap();
    let mut private = draft("private");
    private.limits.as_mut().unwrap().admission = Some("invite".into());
    a.create_community(private).unwrap();
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    pump(&mut a, &mut b);
    assert_eq!(b.discover_communities().len(), 1);
    assert_eq!(b.discover_communities()[0].id, open.id);
    let mut limits = a.community(&open.id).unwrap().limits;
    limits.admission = "invite".into();
    a.set_limits(&open.id, &limits).unwrap();
    pump(&mut a, &mut b);
    assert!(b.discover_communities().is_empty(), "a policy change withdraws the directory listing live");
}

#[test]
fn directory_validates_genesis_and_does_not_dial_unaccepted_routes() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut owner = node("desktop", &t, &Mem::default());
    let mut web = node("web", &t, &Mem::default());
    owner.create_identity("Robin").unwrap();
    let c = owner.create_community(draft("garden")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    owner.net_connect("b", Role::Peer);
    web.net_connect("a", Role::Peer);
    pump(&mut owner, &mut web);
    let genesis = owner.store.genesis_of(&cid).unwrap().wire.clone();
    let mut forged = genesis.clone();
    let last = forged.len() - 1;
    forged[last] ^= 1;
    let entry = hoi_sync::CatalogEntry {
        genesis: forged,
        peers: vec![format!("rtc:{}@203.0.113.10:7844", "q".repeat(43))],
        relays: vec!["https://relay.example/".into()],
        warmth: None,
        relayed: false,
        age: None,
    };
    assert!(web.net_receive("a", &hoi_sync::Msg::Catalog { entries: vec![entry.clone()] }.encode()));
    assert!(web.discover_communities().is_empty(), "a tampered community definition is discarded");
    let valid = hoi_sync::CatalogEntry { genesis, ..entry };
    assert!(web.net_receive("a", &hoi_sync::Msg::Catalog { entries: vec![valid] }.encode()));
    assert_eq!(web.discover_communities().len(), 1);
    assert!(web.communities().is_empty() && web.bootstrap_hints().is_empty() && web.rtc_candidates().is_empty());
    web.commit().unwrap();
    assert!(web.take_events().iter().any(|e| matches!(e, Event::Changed { .. })), "the UI is notified when discovery changes");
}

#[test]
fn legacy_peers_do_not_receive_catalog_messages() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("Robin").unwrap();
    n.create_community(draft("garden")).unwrap();
    n.net_connect("old", Role::Peer);
    n.take_outbox();
    let hello = hoi_sync::Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([6; 32]),
        name: None,
        token: None,
        communities: vec![],
        serves: false,
        hints: vec![],
        routes: vec![],
        features: vec![],
        salt: None,
        hashed: vec![],
        pairing: None,
        link_key: None,
        notice_hash: None,
        notice: None,
        holder: None,
        forwards: false,
        rebuilds: false,
    };
    assert!(n.net_receive("old", &hello.encode()));
    assert!(!n.take_outbox().iter().any(|(_, bytes)| matches!(hoi_sync::Msg::decode(bytes), Ok(hoi_sync::Msg::Catalog { .. }))));
}

#[test]
fn connection_status_is_scoped_to_the_community_shared_with_the_peer() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    let mut b = node("web", &t, &Mem::default());
    a.create_identity("Robin").unwrap();
    b.create_identity("Lena").unwrap();
    let first = a.create_community(draft("garden")).unwrap();
    let second = a.create_community(draft("bikes")).unwrap();
    b.join(&a.invite(&first.id).unwrap().uri).unwrap();
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    pump(&mut a, &mut b);
    let net = a.net_status();
    assert_eq!(net.peers, 1);
    assert_eq!(net.communities.iter().find(|c| c.community == first.id).unwrap().connected_peers, 1);
    assert_eq!(net.communities.iter().find(|c| c.community == second.id).unwrap().connected_peers, 0);
    a.net_disconnect("b");
    assert!(a.net_status().communities.iter().all(|c| c.connected_peers == 0));
}

#[test]
fn registration_signature_vector() {
    let key = Keypair::from_secret(&[42; 32]);
    let name = "garden_1.v2-test";
    let community = key.public();
    let statement = crate::community_registration_statement(name, &community);
    let signature = key.sign(statement.as_bytes());
    let vector = serde_json::json!({
        "seed": hoi_types::hex_encode(&[42; 32]),
        "name": name,
        "community": community.hex(),
        "statement": statement,
        "signature": hoi_types::hex_encode(&signature.0)
    });
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../vectors/community-registration.json")).unwrap();
    assert_eq!(vector, fixture, "the Rust signer and registry share an exact signature statement");
}

#[test]
fn registration_proof_binds_canonical_name_and_key_and_survives_restart() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &t, &memory);
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("Garden")).unwrap();
    let id = Id::from_hex(&c.id).unwrap();
    let proof = n.community_registration(&c.id).unwrap();
    assert_eq!(proof.name, "garden");
    assert_eq!(proof.community, c.id);
    let signature = hoi_types::Sig::from_slice(&hoi_types::hex_decode(&proof.signature).unwrap()).unwrap();
    assert!(hoi_crypto::verify(&id, crate::community_registration_statement(&proof.name, &id).as_bytes(), &signature));
    assert!(!hoi_crypto::verify(&id, crate::community_registration_statement("other", &id).as_bytes(), &signature));
    assert!(n.community_kps.is_empty(), "a public proof does not retain the community signing key");
    n.commit().unwrap();
    let reopened = node("desktop", &t, &memory);
    assert_eq!(reopened.community_registration(&c.id).unwrap().signature, proof.signature);
    let mut other = node("web", &t, &Mem::default());
    other.create_identity("Lena").unwrap();
    other.join(&n.invite(&c.id).unwrap().uri).unwrap();
    assert_eq!(other.community_registration(&c.id).unwrap_err().code, "not-allowed");
    let unicode = n.create_community(draft("jardin partagé")).unwrap();
    assert_eq!(n.community_registration(&unicode.id).unwrap_err().code, "invalid");
    for invalid in ["", "-garden", "garden party", "gar\nden", "gárden", "123456789012345678901234567890123"] {
        assert!(crate::community_registration_name(invalid).is_none());
    }
    assert_eq!(crate::community_registration_name(" Garden_1.v2-test ").as_deref(), Some("garden_1.v2-test"));
}

#[test]
fn publication_signature_vector() {
    let key = Keypair::from_secret(&[42; 32]);
    let name = "garden_1.v2-test";
    let community = key.public();
    let statement = crate::community_publication_statement(name, &community, "0.3", 1);
    let vector = serde_json::json!({
        "seed": hoi_types::hex_encode(&[42; 32]), "name": name, "community": community.hex(),
        "protocolVersion": "0.3", "indexVersion": 1, "statement": statement,
        "signature": hoi_types::hex_encode(&key.sign(statement.as_bytes()).0),
    });
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../vectors/community-publication.json")).unwrap();
    assert_eq!(vector, fixture, "Rust and the registry must bind the same versioned statement");
}

#[test]
fn publication_is_version_bound_persistent_and_only_available_for_open_communities() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &t, &memory);
    n.create_identity("Robin").unwrap();
    let c = n.create_community(draft("Garden")).unwrap();
    let id = Id::from_hex(&c.id).unwrap();
    let proof = n.community_publication(&c.id).unwrap();
    let signature = hoi_types::Sig::from_slice(&hoi_types::hex_decode(&proof.signature).unwrap()).unwrap();
    assert_eq!((proof.protocol_version.clone(), proof.index_version), crate::registration::publication_version());
    assert!(hoi_crypto::verify(
        &id,
        crate::community_publication_statement(&proof.name, &id, &proof.protocol_version, proof.index_version).as_bytes(),
        &signature
    ));
    assert!(!hoi_crypto::verify(&id, crate::community_publication_statement(&proof.name, &id, "other", proof.index_version).as_bytes(), &signature));
    assert!(!hoi_crypto::verify(
        &id,
        crate::community_publication_statement(&proof.name, &id, &proof.protocol_version, proof.index_version + 1).as_bytes(),
        &signature
    ));
    assert!(n.community_kps.is_empty());
    n.commit().unwrap();
    let reopened = node("desktop", &t, &memory);
    assert_eq!(reopened.community_publication(&c.id).unwrap().signature, proof.signature);
    let mut other = node("web", &t, &Mem::default());
    other.create_identity("Lena").unwrap();
    other.join(&n.invite(&c.id).unwrap().uri).unwrap();
    assert_eq!(other.community_publication(&c.id).unwrap_err().code, "not-allowed");
    n.state.publication_proofs.remove(&id);
    assert_eq!(n.community_publication(&c.id).unwrap_err().code, "not-allowed", "legacy local state never invents a versioned proof");
    let mut private = draft("private");
    private.limits.as_mut().unwrap().admission = Some("invite".into());
    let private = n.create_community(private).unwrap();
    assert_eq!(n.community_publication(&private.id).unwrap_err().code, "not-allowed");
}
