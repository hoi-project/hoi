use super::*;
use crate::private::CipherObject;

/// Delivers what the browser transport would: the objects of the room this device reads now.
fn deliver(node: &mut Node, c: &str, objects: &[CipherObject]) {
    for object in objects {
        if node.private_group(c).unwrap().room == object.room {
            node.private_receive(c, &object.body).unwrap();
        }
    }
}
fn accept(a: &mut Node, b: &mut Node, c: &str) -> Vec<CipherObject> {
    let objects = a.private_outbox(c).unwrap();
    deliver(b, c, &objects);
    a.private_ack(c, &objects.iter().map(|o| o.id.clone()).collect::<Vec<_>>()).unwrap();
    objects
}
fn clear(a: &mut Node, c: &str) {
    let objects = a.private_outbox(c).unwrap();
    a.private_ack(c, &objects.iter().map(|o| o.id.clone()).collect::<Vec<_>>()).unwrap();
}
fn setup() -> (Arc<AtomicU64>, Node, Node, String) {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("web", &t, &Mem::default());
    let mut b = node("web", &t, &Mem::default());
    a.create_identity("alice").unwrap();
    b.create_identity("bob").unwrap();
    let c = a.create_private_community(draft("secret garden")).unwrap().id;
    let request = b.private_join_request().unwrap();
    let who = a.private_preview_request(&request).unwrap();
    assert_eq!(who.root, b.me_root().unwrap().hex());
    let invitation = a.private_invite(&c, &request).unwrap();
    assert!(!invitation.contains("secret garden"));
    b.private_join(&invitation).unwrap();
    accept(&mut a, &mut b, &c);
    (t, a, b, c)
}

#[test]
fn private_snapshot_restore_discards_old_content_and_outbox_without_forging_deletions() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &t, &memory);
    n.create_identity("Robin").unwrap();
    let private = n.create_private_community(draft("private garden")).unwrap().id;
    let c = Id::from_hex(&private).unwrap();
    let public = n.create_community(draft("public garden")).unwrap().id;
    let visible = post(&mut n, &public, "public snapshot");
    let shared_draft = PostDraft {
        community: public.clone(),
        title: "shared attachment".into(),
        text: String::new(),
        link: None,
        media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&picture(32, image::ImageFormat::Png)), alt: None }]),
    };
    let shared = n.create_post(&shared_draft).unwrap();
    n.create_post(&PostDraft { community: private.clone(), ..shared_draft }).unwrap();
    let shared_hash = Id::from_hex(&shared.media[0].hash).unwrap();
    let removed = post(&mut n, &private, "already deleted");
    n.delete_own(&removed.id).unwrap();
    let deletion = n.store.all().find(|s| matches!(s.record.body, Body::Delete)).unwrap().id;
    let old = n
        .create_post(&PostDraft {
            community: private.clone(),
            title: "private snapshot".into(),
            text: "cached before catch-up".into(),
            link: None,
            media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&png_with_location()), alt: None }]),
        })
        .unwrap();
    let old_id = Id::from_hex(&old.id).unwrap();
    let old_wire = n.store.get(&old_id).unwrap().wire.clone();
    let hash = Id::from_hex(&old.media[0].hash).unwrap();
    let object = Id([91; 32]);
    let manifest = hoi_types::scatter::Manifest {
        object: object.0,
        sealed: [92; 32],
        class: hoi_types::scatter::Class::from_index(0).unwrap(),
        generation: 0,
        level: 10,
        places: 50,
        k: 3,
        n: 5,
        m: 7,
        hashes: vec![[93; 16]; 5],
        holders: (1..=5).map(|i| Id([i; 32])).collect(),
        loc_secret: [94; 32],
        key: [95; 32],
        expires_at: T0 + 86_400,
        keys: Vec::new(),
    };
    let cap_id = crate::drop_share::cap_id(&manifest);
    n.save_drop_cap(n.capability_for(&manifest).unwrap());
    n.scatter.saved.kept.insert(
        object,
        crate::keep::Kept {
            community: Some(c),
            records: vec![old_id],
            expires_at: manifest.expires_at,
            gens: vec![crate::keep::Gen { manifest: hoi_types::uri::b64(&manifest.encode()), made_at: T0, confirmed: true, from: None }],
            ..Default::default()
        },
    );
    n.scatter.dirty = true;
    assert!(!n.private_outbox(&private).unwrap().is_empty(), "snapshot includes queued encrypted content");
    n.commit().unwrap();
    let ordinary = node("desktop", &t, &memory);
    assert!(ordinary.store.contains(&old_id), "ordinary restart keeps its cache");
    drop(ordinary);
    let clock = t.clone();
    let mut restored =
        Node::open_restored(Box::new(memory.clone()), "desktop", "https://example.com/", Box::new(move || clock.load(Ordering::SeqCst))).unwrap();
    assert!(!restored.store.contains(&old_id));
    assert!(!restored.store.has_blob(&hash));
    assert!(restored.store.has_blob(&shared_hash), "a surviving public reference keeps its shared media bytes");
    assert!(!restored.scatter.saved.kept.contains_key(&object));
    assert!(!restored.scatter.saved.drop_caps.contains_key(&cap_id));
    assert!(restored.store.contains(&Id::from_hex(&visible.id).unwrap()), "public cache uses catch-up quarantine");
    assert!(restored.store.contains(&deletion));
    assert!(restored.deletion_for_target(Id::from_hex(&removed.id).unwrap()).is_some());
    assert!(restored.state.skeletons.contains_key(&Id::from_hex(&removed.id).unwrap()));
    assert!(restored.private_outbox(&private).unwrap().is_empty());
    assert!(restored.private_pending(c).is_empty());
    assert!(!restored.store.is_deleted(&old_id), "discarding a stale snapshot must allow normal authenticated recovery");
    restored.ingest(&old_wire, &Source::PrivateRelay).unwrap();
    assert!(restored.post(&old.id).is_err(), "receiving content alone does not release private quarantine");
    restored.commit().unwrap();
    drop(restored);
    let mut reopened = node("desktop", &t, &memory);
    assert!(reopened.deletion_restore_quarantined(Some(c)));
    assert!(reopened.private_outbox(&private).unwrap().is_empty());
}

#[test]
fn private_mls_roundtrip_media_tamper_and_replay() {
    let (_t, mut a, mut b, c) = setup();
    let p = a
        .create_post(&PostDraft {
            community: c.clone(),
            title: "hidden photograph".into(),
            text: "members only".into(),
            link: None,
            media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&png_with_location()), alt: None }]),
        })
        .unwrap();
    let objects = a.private_outbox(&c).unwrap();
    assert!(!objects.is_empty());
    let mut tamper = hoi_types::uri::unb64(&objects[0].body).unwrap();
    let end = tamper.len() - 1;
    tamper[end] ^= 1;
    assert!(b.private_receive(&c, &hoi_types::uri::b64(&tamper)).is_err());
    for object in &objects {
        let bytes = hoi_types::uri::unb64(&object.body).unwrap();
        assert!(!bytes.windows(b"hidden photograph".len()).any(|w| w == b"hidden photograph"));
        b.private_receive(&c, &object.body).unwrap();
        b.private_receive(&c, &object.body).unwrap();
    }
    assert_eq!(b.post(&p.id).unwrap().summary.title, "hidden photograph");
    assert_eq!(a.media(&p.media[0].hash).unwrap().data, b.media(&p.media[0].hash).unwrap().data);
    assert!(a.export_bundle(&c).is_err());
    assert!(a.invite(&c).is_err());
}

#[test]
fn private_mls_invite_is_bound_to_request_and_membership_revokes() {
    let (t, mut a, mut b, c) = setup();
    let mut stranger = node("web", &t, &Mem::default());
    stranger.create_identity("stranger").unwrap();
    let request = stranger.private_join_request().unwrap();
    clear(&mut a, &c);
    let invitation = a.private_invite(&c, &request).unwrap();
    assert!(b.private_join(&invitation).is_err());
    stranger.private_join(&invitation).unwrap();
    let addition = a.private_outbox(&c).unwrap();
    deliver(&mut b, &c, &addition);
    deliver(&mut stranger, &c, &addition);
    a.private_ack(&c, &addition.iter().map(|o| o.id.clone()).collect::<Vec<_>>()).unwrap();
    assert_eq!(b.private_group(&c).unwrap().members.len(), 3);
    assert!(b.private_remove(&c, &a.me().unwrap().device).is_err());
    let removed = b.me().unwrap().device;
    a.private_remove(&c, &removed).unwrap();
    let removal = a.private_outbox(&c).unwrap();
    for o in &removal {
        stranger.private_receive(&c, &o.body).unwrap();
        let _ = b.private_receive(&c, &o.body);
    }
    assert!(!b.private_group(&c).unwrap().active);
    a.private_ack(&c, &removal.iter().map(|o| o.id.clone()).collect::<Vec<_>>()).unwrap();
    let secret = post(&mut a, &c, "after removing bob");
    let future = a.private_outbox(&c).unwrap();
    for o in &future {
        stranger.private_receive(&c, &o.body).unwrap();
        assert!(b.private_receive(&c, &o.body).is_err());
    }
    assert!(stranger.post(&secret.id).is_ok());
    assert!(b.post(&secret.id).is_err());
    assert!(b.create_post(&PostDraft { community: c, title: "removed".into(), text: "no".into(), link: None, media: None }).is_err());
}

#[test]
fn private_mls_state_and_identical_outbox_survive_restart() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut a = node("web", &t, &mem);
    a.create_identity("alice").unwrap();
    let c = a.create_private_community(draft("persistent secret")).unwrap().id;
    let first = a.call("privateOutbox", serde_json::json!([c])).unwrap();
    let mut restored = node("web", &t, &mem);
    let second = restored.call("privateOutbox", serde_json::json!([c])).unwrap();
    assert_eq!(first, second);
    assert_eq!(restored.private_group(&c).unwrap().members.len(), 1);
    assert!(restored.state.private_communities.contains(&Id::from_hex(&c).unwrap()));
    let mut b = node("web", &t, &Mem::default());
    b.create_identity("bob").unwrap();
    clear(&mut restored, &c);
    let request = b.private_join_request().unwrap();
    let invitation = restored.private_invite(&c, &request).unwrap();
    b.private_join(&invitation).unwrap();
    accept(&mut restored, &mut b, &c);
}

#[test]
fn private_mls_removed_member_can_rejoin_and_private_labels_survive_local_leave() {
    let (_t, mut a, mut b, c) = setup();
    assert!(a.leave(&c).is_err(), "the sole coordinator must not silently lose the namespace key");
    let device = b.me().unwrap().device;
    a.private_remove(&c, &device).unwrap();
    let out = a.private_outbox(&c).unwrap();
    b.private_receive(&c, &out[0].body).unwrap();
    a.private_ack(&c, &out.iter().map(|o| o.id.clone()).collect::<Vec<_>>()).unwrap();
    assert!(!b.private_group(&c).unwrap().active);
    let request = b.private_join_request().unwrap();
    let invite = a.private_invite(&c, &request).unwrap();
    b.private_join(&invite).unwrap();
    assert!(b.private_group(&c).unwrap().active);
    b.leave(&c).unwrap();
    assert!(!b.private_group(&c).unwrap().active);
    assert!(b.private_groups().unwrap().iter().any(|g| g.community == c));
    assert!(b.invite(&c).is_err());
}
