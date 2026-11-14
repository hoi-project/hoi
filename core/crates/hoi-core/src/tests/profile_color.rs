use super::*;
use hoi_types::Kind;

#[test]
fn identicon_colors_sync_persist_and_restore_without_photo_permissions() {
    let time = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut a = node("desktop", &time, &memory);
    a.create_identity("alice").unwrap();
    let c = a.create_community(draft("colors")).unwrap();
    let mut b = node("web", &time, &Mem::default());
    b.create_identity("bob").unwrap();
    pair(&mut a, &mut b, &c.id);
    assert_eq!(a.set_avatar_color(Some("lilac")).unwrap().avatar_color.as_deref(), Some("lilac"));
    let p = post(&mut a, &c.id, "Chosen color");
    pump(&mut a, &mut b);
    assert_eq!(b.post(&p.id).unwrap().summary.author.avatar_color.as_deref(), Some("lilac"));
    b.set_settings(&SettingsPatch { auto_media: Some(false), ..Default::default() }).unwrap();
    assert_eq!(b.post(&p.id).unwrap().summary.author.avatar_color.as_deref(), Some("lilac"));
    post(&mut a, &c.id, "Unchanged color");
    assert_eq!(a.store.of_kind(&Id::from_hex(&c.id).unwrap(), Kind::ProfileColor).count(), 1);
    a.commit().unwrap();
    assert_eq!(node("desktop", &time, &memory).me().unwrap().avatar_color.as_deref(), Some("lilac"));
    let backup = a.export_identity("a long color backup passphrase").unwrap();
    let mut restored = node("web", &time, &Mem::default());
    assert_eq!(restored.import_identity(&backup, "a long color backup passphrase").unwrap().avatar_color.as_deref(), Some("lilac"));
    a.set_avatar_color(None).unwrap();
    a.reply(&p.id, "Use the color from my key again").unwrap();
    pump(&mut a, &mut b);
    assert!(b.post(&p.id).unwrap().summary.author.avatar_color.is_none());
}

#[test]
fn only_signed_palette_values_are_accepted_and_color_does_not_require_media_standing() {
    let (_time, mut a, cid, c) = owner();
    for invalid in ["coral", "#fff", "url(https://example.com)", "MINT"] {
        assert!(a.set_avatar_color(Some(invalid)).is_err());
    }
    let actor = Actor::new();
    actor.intro(&mut a, T0);
    let (bad, _) = actor.sign(rec(cid, None, Body::ProfileColor { color: "expression(alert(1))".into(), seq: 1 }, T0), 8);
    assert!(a.ingest(&bad, &Source::Import).is_err());
    let (signed, id) = actor.sign(rec(cid, None, Body::ProfileColor { color: "mint".into(), seq: 2 }, T0), 8);
    a.ingest(&signed, &Source::Import).unwrap();
    assert!(a.held(a.store.get(&id).unwrap()).is_none());
    let person = a.person_view(&actor.key(), Some(&c)).unwrap().person;
    assert_eq!(person.standing.as_deref(), Some("newcomer"));
    assert_eq!(person.avatar_color.as_deref(), Some("mint"));
    assert!(a.me().unwrap().avatar_color.is_none(), "another root cannot change my local selection");
    let mut tampered = signed;
    let i = tampered.len() / 2;
    tampered[i] ^= 1;
    assert!(a.ingest(&tampered, &Source::Import).is_err());
    // Two accepted palette edits per hour; publishing palette text is still rate limited.
    actor.send(&mut a, rec(cid, None, Body::ProfileColor { color: "sky".into(), seq: 3 }, T0)).unwrap();
    assert!(actor.send(&mut a, rec(cid, None, Body::ProfileColor { color: "sun".into(), seq: 4 }, T0)).is_err());
}

#[test]
fn color_resets_have_independent_persistent_heads_and_do_not_resurrect_after_expiry() {
    let time = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut a = node("desktop", &time, &memory);
    a.create_identity("reader").unwrap();
    let c = a.create_community(draft("color expiry")).unwrap().id;
    let cid = Id::from_hex(&c).unwrap();
    let actor = Actor::new();
    actor.intro(&mut a, T0);
    a.add_mod(&c, &actor.key()).unwrap();
    let mut old = rec(cid, None, Body::ProfileColor { color: "sky".into(), seq: 1 }, T0);
    old.expires_at = T0 + 7 * 86_400;
    let old_id = actor.send(&mut a, old).unwrap();
    // A photo reset with a larger sequence must not supersede the independent color.
    actor.send(&mut a, rec(cid, None, Body::Profile { image: vec![], seq: 99 }, T0)).unwrap();
    assert_eq!(a.person_view(&actor.key(), Some(&c)).unwrap().person.avatar_color.as_deref(), Some("sky"));
    actor.send(&mut a, rec(cid, None, Body::ProfileColor { color: String::new(), seq: 2 }, T0)).unwrap();
    time.store(T0 + 2 * 86_400, Ordering::SeqCst);
    a.sweep();
    a.commit().unwrap();
    assert!(a.store.contains(&old_id));
    assert!(!a.forwardable(a.store.get(&old_id).unwrap()));
    let reopened = node("desktop", &time, &memory);
    assert!(reopened.person_view(&actor.key(), Some(&c)).unwrap().person.avatar_color.is_none());
    assert!(!reopened.forwardable(reopened.store.get(&old_id).unwrap()));
}

#[test]
fn linked_color_edits_and_resets_override_stale_local_choices_and_backups() {
    let (time, mut a, cid, c) = owner();
    a.set_avatar_color(Some("peach")).unwrap();
    let p = post(&mut a, &c, "Initial color");
    let backup = a.export_identity("linked color backup phrase").unwrap();
    let mut b = node("web", &time, &Mem::default());
    b.import_identity(&backup, "linked color backup phrase").unwrap();
    pair(&mut a, &mut b, &c);
    b.set_avatar_color(Some("sun")).unwrap();
    b.reply(&p.id, "Changed on another device").unwrap();
    pump(&mut a, &mut b);
    assert_eq!(a.me().unwrap().avatar_color.as_deref(), Some("sun"));
    a.reply(&p.id, "Keep the changed color").unwrap();
    assert_eq!(a.store.of_kind(&cid, Kind::ProfileColor).count(), 2);
    b.set_avatar_color(None).unwrap();
    b.reply(&p.id, "Reset to my key's color").unwrap();
    pump(&mut a, &mut b);
    a.reply(&p.id, "The old color stays removed").unwrap();
    assert_eq!(a.store.of_kind(&cid, Kind::ProfileColor).count(), 3);
    assert!(a.post(&p.id).unwrap().summary.author.avatar_color.is_none());
    let another = a.create_community(draft("another color scope")).unwrap();
    assert!(post(&mut a, &another.id, "Same reset in a future scope").author.avatar_color.is_none());
    let reset_backup = a.export_identity("color reset backup phrase").unwrap();
    let memory = Mem::default();
    let mut restored = node("web", &time, &memory);
    assert!(restored.import_identity(&reset_backup, "color reset backup phrase").unwrap().avatar_color.is_none());
    restored.commit().unwrap();
    time.store(T0 + 8 * 86_400, Ordering::SeqCst);
    a.sweep();
    assert!(a.me().unwrap().avatar_color.is_none());
    assert!(node("web", &time, &memory).me().unwrap().avatar_color.is_none());
}

#[test]
fn photo_capable_legacy_peers_receive_photos_and_posts_but_no_color_records() {
    let (_time, mut a, cid, c) = owner();
    a.set_avatar(&[]).unwrap();
    a.set_avatar_color(Some("mint")).unwrap();
    let post = post(&mut a, &c, "A regular contribution");
    let profile = a.store.of_kind(&cid, Kind::Profile).next().unwrap().id;
    let color = a.store.of_kind(&cid, Kind::ProfileColor).next().unwrap().id;
    a.net_connect("old", Role::Peer);
    a.take_outbox();
    let hello = hoi_sync::Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([77; 32]),
        name: None,
        token: None,
        communities: vec![cid],
        serves: false,
        hints: vec![],
        routes: vec![],
        features: vec![hoi_sync::FEATURE_PROFILE.into()],
        salt: None,
        hashed: vec![],
        pairing: None,
        link_key: None,
        notice_hash: None,
        notice: None,
        declaration: None,
        holder: None,
        forwards: false,
        rebuilds: false,
    };
    assert!(a.net_receive("old", &hello.encode()));
    for (_, bytes) in a.take_outbox() {
        if let hoi_sync::Msg::Have { ids, .. } = hoi_sync::Msg::decode(&bytes).unwrap() {
            assert!(!ids.contains(&color));
        }
    }
    assert!(a.net_receive("old", &hoi_sync::Msg::Want { ids: vec![profile, color, Id::from_hex(&post.id).unwrap()] }.encode()));
    let mut received = vec![];
    for (_, bytes) in a.take_outbox() {
        if let hoi_sync::Msg::Recs { recs } = hoi_sync::Msg::decode(&bytes).unwrap() {
            received.extend(recs.iter().map(|wire| hoi_types::Wire::decode(wire).unwrap().0.record.kind()));
        }
    }
    assert!(received.contains(&Kind::Post));
    assert!(received.contains(&Kind::Profile));
    assert!(!received.contains(&Kind::ProfileColor));
    a.set_avatar_color(Some("sky")).unwrap();
    a.reply(&post.id, "A changed palette").unwrap();
    for (_, bytes) in a.take_outbox() {
        if let hoi_sync::Msg::Recs { recs } = hoi_sync::Msg::decode(&bytes).unwrap() {
            assert!(recs.iter().all(|wire| hoi_types::Wire::decode(wire).unwrap().0.record.kind() != Kind::ProfileColor));
        }
    }
}

#[test]
fn private_color_never_becomes_a_public_profile() {
    let time = Arc::new(AtomicU64::new(T0));
    let mut a = node("web", &time, &Mem::default());
    let mut b = node("web", &time, &Mem::default());
    let me = a.create_identity("alice").unwrap();
    b.create_identity("bob").unwrap();
    let c = a.create_private_community(draft("private colors")).unwrap().id;
    let request = b.private_join_request().unwrap();
    b.private_join(&a.private_invite(&c, &request).unwrap()).unwrap();
    a.set_avatar_color(Some("sun")).unwrap();
    let p = post(&mut a, &c, "Private palette");
    for object in a.private_outbox(&c).unwrap() {
        if object.room == b.private_group(&c).unwrap().room {
            b.private_receive(&c, &object.body).unwrap();
        }
    }
    assert_eq!(b.post(&p.id).unwrap().summary.author.avatar_color.as_deref(), Some("sun"));
    assert!(b.person_view(&me.root_key, None).unwrap().person.avatar_color.is_none());
    for record in a.store.of_kind(&Id::from_hex(&c).unwrap(), Kind::ProfileColor) {
        assert!(!a.plaintext_forwardable(record));
    }
}
