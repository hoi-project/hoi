use super::*;
use hoi_types::{
    Kind,
    uri::{b64, unb64},
};

fn avatar() -> Vec<u8> {
    hoi_validate::media::sanitize(&picture(96, image::ImageFormat::Jpeg)).unwrap().0
}

#[test]
fn profile_syncs_persists_and_removal_updates_existing_contributions() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut a = node("desktop", &t, &mem);
    let me = a.create_identity("alice").unwrap();
    let c = a.create_community(draft("pictures")).unwrap();
    let mut b = node("web", &t, &Mem::default());
    b.create_identity("bob").unwrap();
    pair(&mut a, &mut b, &c.id);
    let jpeg = avatar();
    assert!(jpeg.len() <= 4096);
    assert_eq!(a.set_avatar(&jpeg).unwrap().avatar, Some(b64(&jpeg)));
    let p = post(&mut a, &c.id, "First portrait");
    pump(&mut a, &mut b);
    assert_eq!(b.post(&p.id).unwrap().summary.author.avatar, Some(b64(&jpeg)));
    assert_eq!(b.person_view(&me.root_key, Some(&c.id)).unwrap().person.avatar, Some(b64(&jpeg)));
    b.set_settings(&SettingsPatch { auto_media: Some(false), ..Default::default() }).unwrap();
    assert!(b.post(&p.id).unwrap().summary.author.avatar.is_none());
    b.set_settings(&SettingsPatch { auto_media: Some(true), ..Default::default() }).unwrap();
    post(&mut a, &c.id, "Same portrait");
    assert_eq!(a.store.of_kind(&Id::from_hex(&c.id).unwrap(), Kind::Profile).count(), 1);
    a.commit().unwrap();
    let reopened = node("desktop", &t, &mem);
    assert_eq!(reopened.me().unwrap().avatar, Some(b64(&jpeg)));
    let backup = a.export_identity("a long backup passphrase").unwrap();
    let mut restored = node("web", &t, &Mem::default());
    assert_eq!(restored.import_identity(&backup, "a long backup passphrase").unwrap().avatar, Some(b64(&jpeg)));
    a.set_avatar(&[]).unwrap();
    a.reply(&p.id, "Back to my key picture").unwrap();
    pump(&mut a, &mut b);
    assert!(b.post(&p.id).unwrap().summary.author.avatar.is_none());
}

#[test]
fn profile_rejects_malicious_images_and_respects_media_standing() {
    let (_t, mut a, cid, c) = owner();
    assert!(a.set_avatar(&vec![1; 4097]).is_err());
    assert!(a.set_avatar(b"<svg onload=alert(1)></svg>").is_err());
    let wide = hoi_validate::media::sanitize(&picture(129, image::ImageFormat::Jpeg)).unwrap().0;
    assert!(a.set_avatar(&wide).is_err());
    let png = hoi_validate::media::sanitize(&picture(64, image::ImageFormat::Png)).unwrap().0;
    assert!(a.set_avatar(&png).is_err());
    let actor = Actor::new();
    actor.intro(&mut a, T0);
    let (bad, _) = actor.sign(rec(cid, None, Body::Profile { image: b"junk".to_vec(), seq: 1 }, T0), 8);
    assert!(a.ingest(&bad, &Source::Import).is_err());
    let image = avatar();
    let (signed, id) = actor.sign(rec(cid, None, Body::Profile { image: image.clone(), seq: 2 }, T0), 8);
    a.ingest(&signed, &Source::Import).unwrap();
    assert!(a.held(a.store.get(&id).unwrap()).is_some());
    assert!(a.person_view(&actor.key(), Some(&c)).unwrap().person.avatar.is_none());
    a.add_mod(&c, &actor.key()).unwrap();
    assert_eq!(a.person_view(&actor.key(), Some(&c)).unwrap().person.avatar, Some(b64(&image)));
    assert!(a.me().unwrap().avatar.is_none(), "another key cannot change my profile");
    let mut tampered = signed;
    let i = tampered.len() / 2;
    tampered[i] ^= 1;
    assert!(a.ingest(&tampered, &Source::Import).is_err());
}

#[test]
fn profile_blocklists_purge_matching_pixels_and_reject_republication() {
    let (_t, mut a, cid, c) = owner();
    let image = avatar();
    a.set_avatar(&image).unwrap();
    post(&mut a, &c, "Picture before blocklist");
    let fp = hoi_validate::similar::fingerprint("image/jpeg", &image).expect("test image has structure");
    a.import_blocklist(&hoi_validate::similar::to_hex(&fp.hash)).unwrap();
    assert!(a.me().unwrap().avatar.is_none());
    assert_eq!(a.store.of_kind(&cid, Kind::Profile).count(), 0);
    assert!(a.set_avatar(&image).is_err());
    let actor = Actor::new();
    actor.intro(&mut a, T0);
    let (signed, _) = actor.sign(rec(cid, None, Body::Profile { image, seq: 1 }, T0), 8);
    assert!(a.ingest(&signed, &Source::Import).is_err());
}

#[test]
fn private_profile_stays_inside_its_encrypted_community() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("web", &t, &Mem::default());
    let mut b = node("web", &t, &Mem::default());
    let alice = a.create_identity("alice").unwrap();
    b.create_identity("bob").unwrap();
    let c = a.create_private_community(draft("private pictures")).unwrap().id;
    let request = b.private_join_request().unwrap();
    b.private_join(&a.private_invite(&c, &request).unwrap()).unwrap();
    let image = avatar();
    a.set_avatar(&image).unwrap();
    let p = post(&mut a, &c, "A private profile");
    let objects = a.private_outbox(&c).unwrap();
    for object in objects {
        let ciphertext = unb64(&object.body).unwrap();
        assert!(!ciphertext.windows(image.len()).any(|part| part == image));
        // The commit that added bob stays in the room he never reads.
        if object.room == b.private_group(&c).unwrap().room {
            b.private_receive(&c, &object.body).unwrap();
        }
    }
    assert_eq!(b.post(&p.id).unwrap().summary.author.avatar, Some(b64(&image)));
    assert!(b.person_view(&alice.root_key, None).unwrap().person.avatar.is_none(), "a private profile is not a public profile");
    for record in a.store.of_kind(&Id::from_hex(&c).unwrap(), Kind::Profile) {
        assert!(!a.plaintext_forwardable(record));
    }
}

#[test]
fn profile_sidecars_are_not_sent_to_legacy_peers() {
    let (_t, mut a, cid, c) = owner();
    a.set_avatar(&avatar()).unwrap();
    post(&mut a, &c, "Still a regular post");
    let profile = a.store.of_kind(&cid, Kind::Profile).next().unwrap().id;
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
    assert!(a.net_receive("old", &hello.encode()));
    for (_, bytes) in a.take_outbox() {
        if let hoi_sync::Msg::Have { ids, .. } = hoi_sync::Msg::decode(&bytes).unwrap() {
            assert!(!ids.contains(&profile));
        }
    }
    assert!(a.net_receive("old", &hoi_sync::Msg::Want { ids: vec![profile] }.encode()));
    for (_, bytes) in a.take_outbox() {
        if let hoi_sync::Msg::Recs { recs } = hoi_sync::Msg::decode(&bytes).unwrap() {
            assert!(recs.iter().all(|wire| hoi_types::Wire::decode(wire).unwrap().0.record.kind() != Kind::Profile));
        }
    }
}

#[test]
fn another_device_can_change_and_remove_a_picture_without_stale_republication() {
    let (a_time, mut a, cid, c) = owner();
    let initial = avatar();
    a.set_avatar(&initial).unwrap();
    let original = post(&mut a, &c, "My original portrait");
    let backup = a.export_identity("linked device backup phrase").unwrap();
    let time = Arc::new(AtomicU64::new(T0));
    let mut b = node("web", &time, &Mem::default());
    b.import_identity(&backup, "linked device backup phrase").unwrap();
    pair(&mut a, &mut b, &c);
    let replacement = hoi_validate::media::sanitize(&picture(80, image::ImageFormat::Jpeg)).unwrap().0;
    b.set_avatar(&replacement).unwrap();
    b.reply(&original.id, "Updated from my other device").unwrap();
    pump(&mut a, &mut b);
    assert_eq!(a.me().unwrap().avatar, Some(b64(&replacement)));
    assert_eq!(a.post(&original.id).unwrap().summary.author.avatar, Some(b64(&replacement)));
    a.reply(&original.id, "Still the new picture").unwrap();
    pump(&mut a, &mut b);
    assert_eq!(a.store.of_kind(&cid, Kind::Profile).count(), 2, "an old local selection must not be republished");
    b.set_avatar(&[]).unwrap();
    // Profile rate limits apply to ordinary members; the owner is exempt.
    b.reply(&original.id, "Removed on the second device").unwrap();
    pump(&mut a, &mut b);
    assert!(a.me().unwrap().avatar.is_none());
    a.reply(&original.id, "The old picture must stay removed").unwrap();
    pump(&mut a, &mut b);
    assert!(a.post(&original.id).unwrap().summary.author.avatar.is_none());
    assert!(b.post(&original.id).unwrap().summary.author.avatar.is_none());
    assert_eq!(a.store.of_kind(&cid, Kind::Profile).count(), 3);
    let elsewhere = a.create_community(draft("another public space")).unwrap();
    let later = post(&mut a, &elsewhere.id, "The removal also applies here");
    assert!(later.author.avatar.is_none());
    let sidecar = a.store.of_kind(&Id::from_hex(&elsewhere.id).unwrap(), Kind::Profile).next().unwrap();
    assert!(matches!(&sidecar.record.body, Body::Profile { image, .. } if image.is_empty()));
    let cleared_backup = a.export_identity("a cleared profile backup").unwrap();
    let memory = Mem::default();
    let mut recovered = node("web", &a_time, &memory);
    assert!(recovered.import_identity(&cleared_backup, "a cleared profile backup").unwrap().avatar.is_none());
    recovered.commit().unwrap();
    a_time.store(T0 + 8 * 86_400, Ordering::SeqCst);
    a.sweep();
    assert!(a.me().unwrap().avatar.is_none(), "expiry must not resurrect the other device's older local picture");
    let reopened = node("web", &a_time, &memory);
    assert!(reopened.me().unwrap().avatar.is_none());
    time.store(T0 + 8 * 86_400, Ordering::SeqCst);
    b.sweep();
    assert!(b.me().unwrap().avatar.is_none(), "expiry must not resurrect an older local picture");
}

#[test]
fn clearing_a_blocklisted_own_picture_advances_its_shared_revision() {
    let (time, mut a, _cid, c) = owner();
    let image = avatar();
    a.set_avatar(&image).unwrap();
    let original = post(&mut a, &c, "Picture before local blocking");
    let mut b = node("web", &time, &Mem::default());
    b.create_identity("reader").unwrap();
    pair(&mut a, &mut b, &c);
    assert!(b.post(&original.id).unwrap().summary.author.avatar.is_some());
    let fp = hoi_validate::similar::fingerprint("image/jpeg", &image).unwrap();
    a.import_blocklist(&hoi_validate::similar::to_hex(&fp.hash)).unwrap();
    a.reply(&original.id, "The picture has been cleared").unwrap();
    pump(&mut a, &mut b);
    assert!(b.post(&original.id).unwrap().summary.author.avatar.is_none(), "same-second removal must beat the old image on peers");
}

#[test]
fn short_lived_profile_removal_never_reveals_an_older_long_lived_picture() {
    let time = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut a = node("desktop", &time, &memory);
    a.create_identity("reader").unwrap();
    let c = a.create_community(draft("profile expiry")).unwrap().id;
    let cid = Id::from_hex(&c).unwrap();
    let author = Actor::new();
    author.intro(&mut a, T0);
    a.add_mod(&c, &author.key()).unwrap();
    let mut old = rec(cid, None, Body::Profile { image: avatar(), seq: 1 }, T0);
    old.expires_at = T0 + 7 * 86_400;
    let old_id = author.send(&mut a, old).unwrap();
    author.send(&mut a, rec(cid, None, Body::Profile { image: vec![], seq: 2 }, T0)).unwrap();
    time.store(T0 + 2 * 86_400, Ordering::SeqCst);
    a.sweep();
    a.commit().unwrap();
    assert!(a.store.contains(&old_id), "keep rate accounting history until its original expiry");
    assert!(!a.forwardable(a.store.get(&old_id).unwrap()), "do not reintroduce a superseded picture to new peers");
    assert!(a.person_view(&author.key(), Some(&c)).unwrap().person.avatar.is_none());
    let reopened = node("desktop", &time, &memory);
    assert!(reopened.person_view(&author.key(), Some(&c)).unwrap().person.avatar.is_none());
    assert!(!reopened.forwardable(reopened.store.get(&old_id).unwrap()));
}
