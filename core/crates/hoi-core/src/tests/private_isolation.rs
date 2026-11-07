use super::*;

use hoi_sync::Msg;

fn hello(community: Id) -> Msg {
    Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([91; 32]),
        name: None,
        token: None,
        communities: vec![community],
        serves: false,
        hints: Vec::new(),
        routes: Vec::new(),
        features: vec![hoi_sync::FEATURE_CATALOG.into()],
        salt: None,
        hashed: vec![],
        pairing: None,
        link_key: None,
        notice_hash: None,
        notice: None,
        holder: None,
        forwards: false,
        rebuilds: false,
    }
}

// Deliberately mark a stored group without installing MLS keys. Missing keys must never
// turn privacy off. Production creates the marker before publishing a new group's genesis;
// this test fixture is not a supported conversion of previously public history.
fn mark_private(n: &mut Node, community: Id) {
    n.state.private_communities.insert(community);
    n.mark_state();
    n.take_outbox();
}

#[test]
fn private_marker_hides_catalog_hello_and_routes_even_without_keys() {
    let (_time, mut n, private, name) = owner();
    let public = n.create_community(draft("public-neighbour")).unwrap();
    let public_id = Id::from_hex(&public.id).unwrap();
    let route = Id([81; 32]).hex();
    let rtc = format!("rtc:{}@203.0.113.10:7844", "q".repeat(43));
    n.remember_hints(private, &[route, rtc], &["https://relay.example".into()]);
    mark_private(&mut n, private);

    assert!(n.bootstrap_hints().is_empty());
    assert!(n.rtc_candidates().is_empty());
    assert_eq!(n.community_hints(private), (Vec::new(), Vec::new()));
    assert_eq!(n.discover_communities().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec![public.id.as_str()]);
    assert!(n.community_registration(&name).is_err());
    assert!(n.community_publication(&name).is_err());
    assert!(n.public_join(&name, &[], None).is_err());

    for (peer, role) in [("peer", Role::Peer), ("home", Role::Home)] {
        n.net_connect(peer, role);
        let messages = n.take_outbox();
        let (communities, hints, salt, hashed) = messages
            .iter()
            .find_map(|(_, bytes)| match Msg::decode(bytes).unwrap() {
                Msg::Hello { communities, hints, salt, hashed, .. } => Some((communities, hints, salt, hashed)),
                _ => None,
            })
            .expect("a connection starts with hello");
        // a stranger finds the public neighbour only by its salted hash; neither hears of the group
        let named: Vec<Id> = communities
            .into_iter()
            .chain(hashed.into_iter().filter(|h| salt.is_some_and(|s| *h == hoi_sync::community_hash(&s, &public_id))).map(|_| public_id))
            .collect();
        assert_eq!(named, vec![public_id]);
        assert!(salt.is_none_or(|s| !hashed_contains(&messages, &hoi_sync::community_hash(&s, &private))));
        assert!(hints.iter().all(|h| h.community != private));
    }
    // a device that hasn't shown its credential yet is told nothing at all
    n.net_connect("client", Role::Client);
    assert!(n.take_outbox().is_empty());
}

fn hashed_contains(messages: &[(String, Vec<u8>)], hash: &Id) -> bool {
    messages.iter().any(|(_, bytes)| matches!(Msg::decode(bytes), Ok(Msg::Hello { hashed, .. }) if hashed.contains(hash)))
}

#[test]
fn hostile_peer_cannot_fetch_or_offer_private_records_by_known_ids() {
    let (_time, mut n, private, name) = owner();
    let p = post(&mut n, &name, "Private history");
    let post_id = Id::from_hex(&p.id).unwrap();
    let genesis_id = n.store.genesis_of(&private).unwrap().id;
    mark_private(&mut n, private);
    n.net_connect("attacker", Role::Peer);
    n.take_outbox();
    assert!(n.net_receive("attacker", &hello(private).encode()));
    assert!(!n.peers["attacker"].communities.contains(&private));
    for (_, bytes) in n.take_outbox() {
        match Msg::decode(&bytes).unwrap() {
            Msg::Have { community, .. } | Msg::Range { community, .. } => assert_ne!(community, private),
            Msg::Catalog { entries } => assert!(entries.is_empty()),
            Msg::Recs { .. } => panic!("private records leaked during hello"),
            _ => {}
        }
    }

    // Even stale/inconsistent subscription state must not bypass the independent egress guard.
    n.peers.get_mut("attacker").unwrap().communities.insert(private);
    assert!(n.net_receive("attacker", &Msg::Want { ids: vec![genesis_id, post_id] }.encode()));
    assert!(n.take_outbox().is_empty());
    assert!(n.net_receive("attacker", &Msg::Have { community: private, ids: vec![Id([99; 32])] }.encode()));
    assert!(n.take_outbox().is_empty(), "private inventory must not cause a plaintext request");
    assert!(!n.peers["attacker"].pending_wants.contains_key(&private));
    // nor does a range message, from either side of a session: no answer, no fingerprints
    for starter in [true, false] {
        let entries = vec![hoi_sync::range::Entry { upper: hoi_sync::range::Bound::END, range: hoi_sync::range::Range::Ids(vec![Id([99; 32])]) }];
        assert!(n.net_receive("attacker", &Msg::Range { community: private, count: 1, starter, entries }.encode()));
        assert!(n.take_outbox().is_empty(), "private communities are never reconciled in plaintext");
    }
    assert!(!n.peers["attacker"].pending_wants.contains_key(&private));
}

#[test]
fn raw_private_records_are_rejected_but_authenticated_decryption_keeps_validation() {
    let (_time, mut n, private, _name) = owner();
    let actor = Actor::new();
    actor.intro(&mut n, T0);
    mark_private(&mut n, private);
    let (wire, id) = actor.sign(rec(private, None, text_post("Encrypted member", "Only the group transport may deliver this."), T0), 8);
    for source in [Source::Peer { peer: "attacker".into(), live: true }, Source::Import] {
        assert!(matches!(n.ingest(&wire, &source), Err(hoi_validate::Reject::NotAllowed(_))));
    }
    assert!(!n.store.contains(&id));
    assert!(matches!(n.ingest(&wire, &Source::Local), Err(hoi_validate::Reject::NotAllowed(_))), "missing keys never allow new local private writes");
    n.ingest(&wire, &Source::PrivateRelay).unwrap();
    assert!(n.store.contains(&id));

    let (mut forged, _) = actor.sign(rec(private, None, text_post("Forged", "A relay cannot change this."), T0), 8);
    *forged.last_mut().unwrap() ^= 1;
    assert!(n.ingest(&forged, &Source::PrivateRelay).is_err(), "decryption does not waive record validation");
    // The same local record cannot be replayed through a plaintext import path either.
    assert!(matches!(n.ingest(&wire, &Source::Import), Err(hoi_validate::Reject::NotAllowed(_))));
}

#[test]
fn private_media_never_uses_raw_blob_or_fetch_messages() {
    let (_time, mut n, private, name) = owner();
    let p = n
        .create_post(&PostDraft {
            community: name,
            title: "Private photograph".into(),
            text: String::new(),
            link: None,
            media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&picture(32, image::ImageFormat::Png)), alt: None }]),
        })
        .unwrap();
    let post_id = Id::from_hex(&p.id).unwrap();
    let Body::Post { media, .. } = &n.store.get(&post_id).unwrap().record.body else { panic!("post expected") };
    let hash = media[0].hash;
    let bytes = n.store.blob(&hash).unwrap().clone();
    mark_private(&mut n, private);
    n.net_connect("attacker", Role::Peer);
    n.net_receive("attacker", &hello(private).encode());
    n.peers.get_mut("attacker").unwrap().communities.insert(private);
    n.take_outbox();

    assert!(n.net_receive("attacker", &Msg::BWant { ids: vec![hash] }.encode()));
    assert!(n.take_outbox().is_empty());
    n.store.remove_blob(&hash);
    n.request_missing_media();
    assert!(n.take_outbox().is_empty());
    assert!(!n.media_wanted.contains_key(&hash));
    assert!(n.net_receive("attacker", &Msg::Blob { id: hash, data: bytes.clone() }.encode()));
    assert!(!n.store.has_blob(&hash), "raw blobs cannot become an alternate private ingestion path");
    assert!(n.on_blob(hash, bytes), "the authenticated private transport can still validate media");
    assert!(n.store.has_blob(&hash));
}

#[test]
fn decrypted_global_identity_records_do_not_gossip_to_public_connections() {
    let (_time, mut n, public, _name) = owner();
    n.net_connect("public-peer", Role::Peer);
    assert!(n.net_receive("public-peer", &hello(public).encode()));
    n.take_outbox();
    let member = Actor::new();
    let identity = member.identity(T0);
    let identity_id = n.ingest(&identity, &Source::PrivateRelay).unwrap().id;
    let delegation_id = n.ingest(&member.delegation(T0), &Source::PrivateRelay).unwrap().id;
    assert!(n.take_outbox().is_empty(), "private identity carriers cannot be auto-gossiped globally");
    assert!(n.net_receive("public-peer", &Msg::Want { ids: vec![identity_id, delegation_id] }.encode()));
    assert!(n.take_outbox().is_empty(), "knowing private carrier IDs must not disclose their records");

    // A record independently published in full is public, even if first learned privately.
    assert!(matches!(n.ingest(&identity, &Source::Peer { peer: "public-peer".into(), live: true }), Err(hoi_validate::Reject::Duplicate)));
    assert!(n.net_receive("public-peer", &Msg::Want { ids: vec![identity_id] }.encode()));
    assert!(n.take_outbox().iter().any(|(_, bytes)| matches!(Msg::decode(bytes), Ok(Msg::Recs { .. }))));
}

#[test]
fn missing_mls_blob_does_not_erase_persisted_private_boundaries() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("web", &t, &memory);
    n.create_identity("Lena").unwrap();
    let group = n.create_community(draft("closed-room")).unwrap();
    let c = Id::from_hex(&group.id).unwrap();
    let genesis = n.store.genesis_of(&c).unwrap().wire.clone();
    mark_private(&mut n, c);
    let member = Actor::new();
    let carrier = n.ingest(&member.identity(T0), &Source::PrivateRelay).unwrap().id;
    n.commit().unwrap();
    drop(n);
    memory.0.lock().unwrap().1.remove("mls");

    let mut reopened = node("web", &t, &memory);
    assert!(reopened.is_private_community(&c));
    assert!(!reopened.private_writable(c));
    assert!(reopened.state.private_carriers.contains(&carrier));
    assert!(reopened.discover_communities().is_empty());
    assert!(matches!(reopened.ingest(&genesis, &Source::Import), Err(hoi_validate::Reject::NotAllowed(_))));
    assert!(post_error(&mut reopened, &group.id).is_err());
}

fn post_error(n: &mut Node, community: &str) -> Result<PostSummary, crate::ApiError> {
    n.create_post(&PostDraft {
        community: community.into(),
        title: "Keys are unavailable".into(),
        text: "Do not silently publish this.".into(),
        link: None,
        media: None,
    })
}

#[test]
fn ordinary_device_link_and_identity_backup_exclude_private_group_geneses() {
    let (time, mut source, private, _name) = owner();
    let public = source.create_community(draft("public-room")).unwrap();
    mark_private(&mut source, private);

    let mut linked = node("web", &time, &Mem::default());
    let request = linked.link_request("Lena's new browser").unwrap();
    let approval = source.link_approve(&request).unwrap();
    linked.link_complete(&approval).unwrap();
    assert!(linked.community(&public.id).is_ok());
    assert!(!linked.state.joined.contains_key(&private));
    assert!(linked.store.genesis_of(&private).is_none());

    let backup = source.export_identity("a long test passphrase").unwrap();
    let mut restored = node("web", &time, &Mem::default());
    restored.import_identity(&backup, "a long test passphrase").unwrap();
    assert!(restored.community(&public.id).is_ok());
    assert!(!restored.state.joined.contains_key(&private));
    assert!(restored.store.genesis_of(&private).is_none());
}

#[test]
fn public_media_references_cannot_exfiltrate_private_blob_bytes() {
    let time = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &time, &memory);
    n.create_identity("Lena").unwrap();
    let (bytes, info) = hoi_validate::media::sanitize(&picture(32, image::ImageFormat::Png)).unwrap();
    let hash = Id::hash(&bytes);
    let reference = hoi_types::MediaRef { hash, mime: info.mime.into(), size: bytes.len() as u64, w: info.w, h: info.h, alt: None };

    let attacker = Actor::new();
    attacker.intro(&mut n, T0);
    let mut public = draft("public-photos");
    public.co_owners = Some(vec![attacker.key()]);
    let public = n.create_community(public).unwrap();
    let c = Id::from_hex(&public.id).unwrap();
    // A malicious authorized public author knows the private hash and metadata, but supplies
    // no attachment bytes. Planting this reference before the private file is uploaded must
    // not authorize later disclosure from the shared blob store.
    let (forged_reference, id) = attacker
        .sign(rec(c, None, Body::Post { title: "A public hash reference".into(), text: String::new(), link: None, name: None, media: vec![reference] }, T0), 8);
    n.ingest(&forged_reference, &Source::Peer { peer: "attacker".into(), live: true }).unwrap();
    assert!(n.plaintext_forwardable(n.store.get(&id).unwrap()), "the public post itself is valid and visible");
    assert!(!n.store.has_blob(&hash));
    let private = n.create_private_community(draft("private-photos")).unwrap();
    n.create_post(&PostDraft {
        community: private.id,
        title: "A private photograph".into(),
        text: String::new(),
        link: None,
        media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&bytes), alt: None }]),
    })
    .unwrap();
    assert!(n.store.has_blob(&hash));
    assert!(n.state.private_blobs.contains(&hash));
    n.commit().unwrap();
    drop(n);

    // An interim format-2 store did not save this field. Its private blobs must acquire
    // conservative provenance before the upgraded node can answer network requests.
    let legacy = Mem(Arc::new(Mutex::new(memory.0.lock().unwrap().clone())));
    {
        let mut saved = legacy.0.lock().unwrap();
        saved.1.insert("format".into(), br#"{"version":2}"#.to_vec());
        let mut state: serde_json::Value = serde_json::from_slice(&saved.1["state"]).unwrap();
        state.as_object_mut().unwrap().remove("private_blobs");
        saved.1.insert("state".into(), serde_json::to_vec(&state).unwrap());
    }
    let upgraded = node("desktop", &time, &legacy);
    assert!(upgraded.state.private_blobs.contains(&hash));

    let mut n = node("desktop", &time, &memory);
    n.net_connect("attacker", Role::Peer);
    assert!(n.net_receive("attacker", &hello(c).encode()));
    n.take_outbox();
    assert!(n.net_receive("attacker", &Msg::BWant { ids: vec![hash] }.encode()));
    assert!(n.take_outbox().is_empty(), "a public reference cannot exfiltrate the private blob after restart");
    let _ = n.net_receive("attacker", &Msg::Blob { id: hash, data: vec![1, 2, 3] }.encode());
    assert!(n.state.private_blobs.contains(&hash), "invalid bytes cannot declassify a private blob");
    n.take_outbox();
    assert!(n.net_receive("attacker", &Msg::BWant { ids: vec![hash] }.encode()));
    assert!(n.take_outbox().is_empty());

    // An explicit local publication supplies the actual attachment bytes and can disclose it.
    n.create_post(&PostDraft {
        community: public.id,
        title: "Deliberately shared by Lena".into(),
        text: String::new(),
        link: None,
        media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&bytes), alt: None }]),
    })
    .unwrap();
    assert!(!n.state.private_blobs.contains(&hash));
    n.take_outbox();
    assert!(n.net_receive("attacker", &Msg::BWant { ids: vec![hash] }.encode()));
    assert!(n.take_outbox().iter().any(|(_, b)| matches!(Msg::decode(b), Ok(Msg::Blob { id, .. }) if id == hash)));
}
