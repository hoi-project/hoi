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
