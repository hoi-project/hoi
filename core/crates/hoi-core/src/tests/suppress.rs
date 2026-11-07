//! Operators can stop serving, nobody can forge: a suppressed community, record or
//! media blob is deleted, refused on every path that could bring it back (sync, carry requests,
//! an index, an invite), and stays refused across restarts; nothing is signed on anyone's behalf.
use super::*;
use crate::CarryPolicy;
use crate::suppress::Target;

fn public_node(t: &Arc<AtomicU64>, memory: &Mem) -> Node {
    let mut n = node("node", t, memory);
    n.set_node_name(Some("public node".into()));
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    n.set_carry_policy(CarryPolicy::Any, 8);
    n
}

fn dial(native: &mut Node, browser: &mut Node) {
    native.net_connect("b", Role::Peer);
    native.set_peer_meta("b", PeerMeta { transport: "webrtc".into(), incoming: true, ..Default::default() });
    browser.net_connect("a", Role::Peer);
    browser.set_peer_meta("a", PeerMeta { transport: "webrtc".into(), ..Default::default() });
}

fn state_json(memory: &Mem) -> String {
    String::from_utf8(memory.0.lock().unwrap().1.get("state").cloned().unwrap_or_default()).unwrap()
}

#[test]
fn a_suppressed_community_is_deleted_refused_everywhere_and_stays_refused_after_a_restart() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let c = founder.create_community(draft("Unwanted")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    post(&mut founder, &c.id, "Something");
    let invite = founder.invite(&c.id).unwrap().uri;
    let memory = Mem::default();
    let mut carrier = public_node(&t, &memory);
    carrier.join(&invite).unwrap();
    dial(&mut carrier, &mut founder);
    pump(&mut carrier, &mut founder);
    assert!(carrier.store.in_community(&cid).count() >= 2, "the operator joined it on purpose and holds its records");
    assert!(!state_json(&memory).contains("suppressed"), "an unused list is not even written down");

    // An evidence copy first: the signed records as they arrived, each verifiable on its own.
    let material = carrier.suppression_material(Target::Community, &c.id).unwrap();
    assert_eq!(material.records.len(), carrier.store.in_community(&cid).count());
    for r in &material.records {
        let d = hoi_validate::decode(&r.wire).unwrap();
        assert_eq!(d.id, r.id);
        hoi_validate::signature(&d).unwrap();
        assert_eq!(r.community, Some(cid));
    }
    assert!(material.records.iter().any(|r| r.kind == "community"));

    let purged = carrier.suppress_community(&c.id, Some("case 2026-14")).unwrap();
    assert_eq!(purged.records, material.records.len());
    carrier.commit().unwrap();
    assert!(carrier.communities().is_empty());
    assert_eq!(carrier.store.in_community(&cid).count(), 0);
    assert!(material.records.iter().all(|r| !memory.0.lock().unwrap().0.contains_key(&r.id)), "nothing of it is left on disk");
    assert!(state_json(&memory).contains("suppressed"));
    let listed = carrier.suppressions();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].target, listed[0].id.as_str(), listed[0].note.as_deref()), (Target::Community, c.id.as_str(), Some("case 2026-14")));

    // Every way back is closed: a request, an index, an offer, an invite, a public join, sync.
    let genesis = founder.store.genesis_of(&cid).unwrap().wire.clone();
    assert!(carrier.net_receive("b", &hoi_sync::Msg::Carry { genesis: genesis.clone() }.encode()));
    let answers: Vec<_> = carrier
        .take_outbox()
        .into_iter()
        .filter_map(|(_, m)| match hoi_sync::Msg::decode(&m) {
            Ok(hoi_sync::Msg::Carried { ok, reason, .. }) => Some((ok, reason)),
            _ => None,
        })
        .collect();
    assert_eq!(answers, vec![(false, Some("refused".to_string()))]);
    assert!(carrier.carry(&c.id, Some("unwanted")).is_err());
    assert!(carrier.offered_unknown().is_empty());
    assert!(carrier.join(&invite).is_err());
    assert!(carrier.public_join(&c.id, &[], None).is_err());
    assert_eq!(carrier.ingest(&genesis, &Source::Peer { peer: "b".into(), live: true }).unwrap_err(), hoi_validate::Reject::Tombstoned);
    pump(&mut carrier, &mut founder);
    assert!(carrier.communities().is_empty());

    // A restart does not bring it back; neither does `--refuse` being dropped from the command line.
    drop(carrier);
    let mut carrier = public_node(&t, &memory);
    carrier.set_refused(&[]).unwrap();
    assert!(carrier.communities().is_empty());
    assert!(carrier.carry(&c.id, None).is_err());
    assert!(carrier.join(&invite).is_err());

    // Lifted, it may be carried again; nothing came back by itself.
    assert_eq!(carrier.unsuppress(&c.id).unwrap(), Target::Community);
    assert!(carrier.suppressions().is_empty());
    assert!(carrier.communities().is_empty());
    assert_eq!(carrier.carry(&c.id, Some("unwanted")).unwrap().status, "pending");
    assert!(carrier.unsuppress(&c.id).is_err(), "nothing left to lift");
}

#[test]
fn a_suppressed_post_goes_with_its_edits_and_is_never_taken_again() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut home = node("desktop", &t, &memory);
    home.create_identity("mira").unwrap();
    let c = home.create_community(draft("privacy")).unwrap();
    let keep = post(&mut home, &c.id, "Stays");
    let gone = post(&mut home, &c.id, "Goes");
    home.edit(&gone.id, "Still goes, edited", None).unwrap();
    let reply = home.reply(&gone.id, "A reply someone else could have written").unwrap();
    home.commit().unwrap();
    let gone_id = Id::from_hex(&gone.id).unwrap();
    let wire = home.store.get(&gone_id).unwrap().wire.clone();
    let edits: Vec<Id> = home.store.children(&gone_id).filter(|s| s.record.kind() == hoi_types::Kind::Edit).map(|s| s.id).collect();
    assert_eq!(edits.len(), 1);
    assert_eq!(home.suppression_material(Target::Record, &gone.id).unwrap().records.len(), 2, "the post and its edit");

    // Records that decide who may speak or what is hidden cannot be suppressed one by one.
    let genesis = home.store.genesis_of(&Id::from_hex(&c.id).unwrap()).unwrap().id;
    assert_eq!(home.suppress_record(&genesis.hex(), None).unwrap_err().code, "not-allowed");
    assert!(home.suppress_record(&gone.id, Some(&"x".repeat(201))).is_err(), "a note is a short reference");
    assert!(home.suppress_record(&gone.id, Some("line\nbreak")).is_err());

    assert_eq!(home.suppress_record(&gone.id, None).unwrap(), crate::suppress::Purged { records: 2, media: 0 });
    home.commit().unwrap();
    assert!(home.post(&gone.id).is_err());
    assert!(home.post(&keep.id).is_ok());
    assert!(home.store.contains(&Id::from_hex(&reply.id).unwrap()), "someone else's reply is their own item");
    assert_eq!(home.ingest(&wire, &Source::Peer { peer: "p".into(), live: true }).unwrap_err(), hoi_validate::Reject::Tombstoned);
    assert!(home.store.is_deleted(&gone_id), "sync never asks for it");

    // After a restart: still gone from disk, still refused, still never asked for.
    drop(home);
    assert!(!memory.0.lock().unwrap().0.contains_key(&gone_id));
    let mut home = node("desktop", &t, &memory);
    assert!(home.post(&gone.id).is_err());
    assert!(home.store.is_deleted(&gone_id) && home.store.is_deleted(&edits[0]));
    assert_eq!(home.ingest(&wire, &Source::Import).unwrap_err(), hoi_validate::Reject::Tombstoned);
    assert_eq!(home.suppressions().iter().filter(|s| s.target == Target::Record).count(), 2);
    assert_eq!(home.unsuppress(&gone.id).unwrap(), Target::Record);
    assert!(home.ingest(&wire, &Source::Import).is_ok(), "lifted, the same signed record is welcome again");
}

#[test]
fn suppressed_media_is_deleted_refused_and_its_post_served_to_nobody() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut home = node("desktop", &t, &memory);
    home.create_identity("mira").unwrap();
    let c = home.create_community(draft("privacy")).unwrap();
    let p = home
        .create_post(&PostDraft {
            community: c.id.clone(),
            title: "a dot".into(),
            text: "".into(),
            link: None,
            media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&png_with_location()), alt: None }]),
        })
        .unwrap();
    let hash = Id::from_hex(&p.media[0].hash).unwrap();
    let bytes = home.store.blob(&hash).unwrap().clone();
    let material = home.suppression_material(Target::Media, &hash.hex()).unwrap();
    assert_eq!(material.media, vec![(hash, bytes.clone())]);
    assert_eq!(material.records.len(), 1, "the post that shows it comes along as context");

    assert_eq!(home.suppress_media(&hash.hex(), None).unwrap(), crate::suppress::Purged { records: 0, media: 1 });
    home.commit().unwrap();
    assert!(!home.store.has_blob(&hash));
    assert!(home.media(&hash.hex()).is_err());
    let post_id = Id::from_hex(&p.id).unwrap();
    assert!(!home.forwardable(home.store.get(&post_id).unwrap()), "the post that shows it is served to nobody");
    assert!(home.on_blob(hash, bytes.clone()), "the bytes are not the sender's fault");
    assert!(!home.store.has_blob(&hash), "and they are not kept");

    drop(home);
    assert!(!memory.0.lock().unwrap().1.contains_key(&format!("m:{}", hash.hex())));
    let mut home = node("desktop", &t, &memory);
    assert!(!home.store.has_blob(&hash));
    assert!(home.state.media_blocklist.contains(&hash));
    assert_eq!(home.unsuppress(&hash.hex()).unwrap(), Target::Media);
    assert!(!home.state.media_blocklist.contains(&hash));
}
