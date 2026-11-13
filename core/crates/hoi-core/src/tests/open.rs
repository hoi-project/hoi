//! Open communities in pieces, phase S5 (SCATTER.md §5.1, §5.10, §5.11): posts,
//! comments and edits are kept in pieces and a sealed pointer record syncs in their place; nodes
//! that only carry the community never hold them whole; owners set a drop key in the ledger,
//! sealed to their and their moderators' devices.
use super::*;
use crate::tests::forward::{Mesh, join, kept_pieces, mesh};
use hoi_types::Kind;
use hoi_types::scatter::Manifest;

const DAY: u64 = 86_400;

/// Runs half a minute at a time until `done`, at most `secs`.
pub(crate) fn until(m: &mut Mesh, secs: u64, done: impl Fn(&mut Mesh) -> bool) -> bool {
    for _ in 0..secs.div_ceil(30) {
        if done(m) {
            return true;
        }
        m.run(30);
    }
    done(m)
}

/// `a` makes an open community (in pieces by default) and every named device joins it by
/// invite; all reach holders through the same forwarders, which carry the community.
pub(crate) fn open_community(m: &mut Mesh, names: &[(&str, &str)]) -> String {
    join(m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    let c = m.n("a").create_community(draft("an open garden")).unwrap().id;
    let uri = m.n("a").invite(&c).unwrap().uri;
    for (name, addr) in names {
        join(m, name, addr, &[0, 1, 2]);
        m.n(name).join(&uri).unwrap();
    }
    m.run(60);
    c
}

/// The newest generation of the object holding `record`, as `name` keeps it.
pub(crate) fn generation_of(m: &mut Mesh, name: &str, record: &str) -> Option<Manifest> {
    let id = Id::from_hex(record).unwrap();
    m.n(name).scatter.saved.kept.values().find(|k| k.records == vec![id] && k.media.is_none()).and_then(|k| k.newest())
}

#[test]
fn an_open_community_s_posts_go_in_pieces_and_carriers_hold_only_pointers() {
    let mut m = mesh(3, 14);
    let c = open_community(&mut m, &[("b", "192.0.2.20:7843")]);
    let ca = Id::from_hex(&c).unwrap();
    assert!(m.n("a").communities[&ca].limits.scatter.open, "new communities keep posts in pieces");
    let p = post(m.n("a"), &c, "kept in pieces");
    let pid = Id::from_hex(&p.id).unwrap();
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()), "b rebuilt the post");
    assert_eq!(m.n("b").post(&p.id).unwrap().summary.title, "kept in pieces");
    let man = generation_of(&mut m, "b", &p.id).expect("b keeps track of its object");
    assert_eq!(kept_pieces(&mut m, &man), man.hashes.len());
    // the forwarders carry the community: they hold its pointer record, never the post
    for f in ["f0", "f1", "f2"] {
        let n = m.n(f);
        assert!(n.store.get(&pid).is_none(), "{f} never got the post whole");
        assert!(n.store.of_kind(&ca, Kind::Pointer).any(|s| matches!(&s.record.body, Body::Pointer { record, .. } if *record == pid)), "{f} holds its pointer");
    }
    // nobody offers it whole: not its author, not its reader
    for name in ["a", "b"] {
        let n = m.n(name);
        let s = n.store.get(&pid).unwrap();
        assert!(!n.forwardable(s), "{name} keeps it to itself");
    }
    // a vote and a reply on it travel through nodes that never had it
    m.n("b").vote(&p.id, 1).unwrap();
    let reply = m.n("b").reply(&p.id, "and the reply too").unwrap();
    assert!(until(&mut m, 600, |m| m.n("a").store.of_kind(&ca, Kind::Vote).count() > 0), "the vote reached a");
    assert!(until(&mut m, 600, |m| m.n("a").store.contains(&Id::from_hex(&reply.id).unwrap())), "the reply, in pieces too, reached a");
    assert!(m.n("f0").store.of_kind(&ca, Kind::Vote).count() > 0, "a carrier took the vote on a post it holds only a pointer to");
}

#[test]
fn a_pointer_only_copy_persists_author_deletion_before_the_body_arrives() {
    let mut m = mesh(3, 14);
    let c = open_community(&mut m, &[]);
    let community = Id::from_hex(&c).unwrap();
    m.n("a").commit().unwrap();
    let loaded = m.n("a").persist_mut().load().unwrap();
    let disk = Mem::default();
    {
        let mut saved = disk.0.lock().unwrap();
        for wire in loaded.records {
            let (_, id) = hoi_types::Wire::decode(&wire).unwrap();
            saved.0.insert(id, wire);
        }
        saved.1.extend(loaded.blobs);
    }
    // This device has the same identity and community, but receives only the pointer.
    let mut reader = node("desktop", &m.t, &disk);
    let p = post(m.n("a"), &c, "deleted before its body arrives");
    let target = Id::from_hex(&p.id).unwrap();
    let whole = m.n("a").store.get(&target).unwrap().wire.clone();
    let expires_at = m.n("a").store.get(&target).unwrap().record.expires_at;
    assert!(until(&mut m, 600, |m| {
        m.n("a").store.of_kind(&community, Kind::Pointer).any(|s| matches!(&s.record.body, Body::Pointer { record, media: None, .. } if *record == target))
    }));
    let pointer = m
        .n("a")
        .store
        .of_kind(&community, Kind::Pointer)
        .find(|s| matches!(&s.record.body, Body::Pointer { record, media: None, .. } if *record == target))
        .unwrap()
        .wire
        .clone();
    reader.ingest(&pointer, &Source::Import).unwrap();
    assert!(reader.scatter.standins.contains_key(&target));
    assert!(!reader.store.contains(&target));

    m.n("a").delete_own(&p.id).unwrap();
    let deletion = m.n("a").store.of_kind(&community, Kind::Delete).find(|s| s.record.parent == Some(target)).unwrap().wire.clone();
    let accepted = reader.ingest(&deletion, &Source::Import).unwrap();
    reader.commit().unwrap();
    assert_eq!(reader.state.skeletons[&target].expires_at, expires_at, "suppression lasts through the pointer's target horizon");
    assert!(reader.deletion_progress().iter().any(|p| p.target == target.hex() && p.local_removed));
    assert!(matches!(reader.ingest(&whole, &Source::Import), Err(hoi_validate::Reject::Tombstoned)));
    drop(reader);

    let mut reopened = node("desktop", &m.t, &disk);
    assert!(reopened.store.contains(&accepted.id), "the signed deletion remains valid without its target body");
    assert!(reopened.deletion_progress().iter().any(|p| p.target == target.hex() && p.local_removed));
    for source in [Source::Import, Source::Pieces] {
        assert!(matches!(reopened.ingest(&whole, &source), Err(hoi_validate::Reject::Tombstoned)));
    }
    assert!(!reopened.store.contains(&target), "neither a late whole copy nor a delayed rebuild resurrects the post");
}

#[test]
fn with_too_few_places_a_post_goes_whole() {
    let mut m = mesh(1, 0);
    join(&mut m, "a", "198.51.100.10:7843", &[0]);
    join(&mut m, "b", "192.0.2.20:7843", &[0]);
    let c = m.n("a").create_community(draft("a small garden")).unwrap().id;
    let uri = m.n("a").invite(&c).unwrap().uri;
    m.n("b").join(&uri).unwrap();
    m.run(30);
    let p = post(m.n("a"), &c, "whole, for now");
    assert!(until(&mut m, 300, |m| m.n("b").post(&p.id).is_ok()));
    let pid = Id::from_hex(&p.id).unwrap();
    assert!(m.n("f0").store.get(&pid).is_some(), "carried whole, as before");
}

#[test]
fn the_owner_s_device_seals_the_drop_key_to_moderators_and_a_moderator_drops_a_removed_post() {
    let mut m = mesh(3, 14);
    let c = open_community(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843")]);
    let ca = Id::from_hex(&c).unwrap();
    // the owner's device made a drop key in the ledger, sealed to its own seal key
    assert!(until(&mut m, 300, |m| m.n("b").community_drop_key(ca).is_some()), "a drop-key entry is in the ledger");
    let first = m.n("b").community_drop_key(ca).unwrap();
    assert!(m.n("a").held_drop_key(ca, &first).is_some());
    // a moderator's device gets it sealed to the key its delegation carries
    let md_root = m.n("md").me().unwrap().root;
    m.n("a").add_mod(&c, &md_root).unwrap();
    assert!(until(&mut m, 1800, |m| m.n("md").held_drop_key(ca, &first).is_some()), "the moderator's device holds the drop key");
    assert!(m.n("b").held_drop_keys(ca).is_empty(), "a member's device holds none");
    let p = post(m.n("b"), &c, "against the rules");
    assert!(until(&mut m, 600, |m| m.n("md").post(&p.id).is_ok()));
    let man = generation_of(&mut m, "md", &p.id).unwrap();
    assert_eq!(kept_pieces(&mut m, &man), man.hashes.len());
    // the owner's and the author's devices are away; the moderator removes it
    m.cut.insert("a".into());
    m.cut.insert("b".into());
    m.n("md").remove(&p.id, "spam").unwrap();
    m.run(60);
    m.t.fetch_add(2 * 3600, Ordering::SeqCst);
    m.run(120);
    m.t.fetch_add(crate::private::TURN, Ordering::SeqCst);
    m.run(120);
    assert_eq!(kept_pieces(&mut m, &man), 0, "dropped with the community's drop key, in its turn");
    // once the moderator is removed, the owner's device makes a new key without them
    m.cut.remove("a");
    m.cut.remove("b");
    m.n("a").remove_mod(&c, &md_root).unwrap();
    assert!(until(&mut m, 1800, |m| m.n("b").community_drop_key(ca).is_some_and(|k| k != first)), "a new key, on mod-remove");
    let new = m.n("b").community_drop_key(ca).unwrap();
    assert!(m.n("md").held_drop_key(ca, &new).is_none());
    assert!(until(&mut m, 120, |m| m.n("md").held_drop_keys(ca).is_empty()), "a device out of the line keeps none");
    assert!(m.n("a").held_drop_key(ca, &first).is_some(), "the owner's device keeps the old one while its pieces may live");
}

#[test]
fn a_removed_post_s_media_is_dropped_though_another_post_uses_it() {
    let mut m = mesh(3, 14);
    let c = open_community(&mut m, &[("b", "192.0.2.20:7843")]);
    let ca = Id::from_hex(&c).unwrap();
    let rgb = image::RgbImage::from_pixel(64, 64, image::Rgb([40, 140, 200]));
    let mut jpeg = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80).encode_image(&rgb).unwrap();
    let with = |title: &str| PostDraft {
        community: c.clone(),
        title: title.into(),
        text: String::new(),
        link: None,
        media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&jpeg), alt: None }]),
    };
    // b's standing lets it post media once established
    m.t.fetch_add(2 * DAY, Ordering::SeqCst);
    m.run(30);
    let first = m.n("a").create_post(&with("first with the picture")).unwrap();
    let second = m.n("a").create_post(&with("second with the same picture")).unwrap();
    // the post keeps the picture as hoi re-encoded it
    let hash = match &m.n("a").store.get(&Id::from_hex(&first.id).unwrap()).unwrap().record.body {
        Body::Post { media, .. } => media[0].hash,
        _ => unreachable!(),
    };
    assert!(until(&mut m, 900, |m| m.n("b").store.has_blob(&hash) && m.n("b").post(&second.id).is_ok()), "b rebuilt the picture");
    let media_of = |m: &mut Mesh, post: &str| {
        let id = Id::from_hex(post).unwrap();
        m.n("a").scatter.saved.kept.values().find(|k| k.media == Some(hash) && k.records == vec![id]).and_then(|k| k.newest())
    };
    let one = media_of(&mut m, &first.id).expect("the first post's media is an object of its own");
    let two = media_of(&mut m, &second.id).expect("and the second's");
    assert_ne!(one.object, two.object);
    m.n("a").remove(&first.id, "spam").unwrap();
    m.run(60);
    m.t.fetch_add(2 * 3600, Ordering::SeqCst);
    m.run(120);
    assert_eq!(kept_pieces(&mut m, &one), 0, "the removed post's media is dropped");
    assert_eq!(kept_pieces(&mut m, &two), two.hashes.len(), "the other post's stays");
    let _ = ca;
}

#[test]
fn an_update_counts_from_the_author_or_an_established_member_only() {
    let mut m = mesh(3, 14);
    let c = open_community(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843"), ("r", "192.0.2.22:7843")]);
    let ca = Id::from_hex(&c).unwrap();
    let md_root = m.n("md").me().unwrap().root;
    m.n("a").add_mod(&c, &md_root).unwrap();
    m.run(60);
    let p = post(m.n("a"), &c, "kept by its author");
    let pid = Id::from_hex(&p.id).unwrap();
    for name in ["b", "md", "r"] {
        assert!(until(&mut m, 600, |m| m.n(name).post(&p.id).is_ok()), "{name}");
    }
    let man = generation_of(&mut m, "r", &p.id).unwrap();
    // an update that places one more index of the same generation, as a repair would
    let extended = |m: &Manifest| {
        let mut x = Manifest::decode(&m.encode()).unwrap();
        x.hashes.push([9; 16]);
        x.holders.push(Id([9; 32]));
        if !x.keys.is_empty() {
            x.keys.push([9; 32]);
        }
        x
    };
    let update = |m: &mut Mesh, by: &str, man: &Manifest| {
        let now = m.n(by).now();
        let pointer = hoi_types::scatter::Pointer { made_at: now, manifest: extended(man) };
        let sealed = crate::open::seal_pointer(&ca, &pointer).unwrap();
        let author = Id::from_hex(&m.n(by).me().unwrap().device).unwrap();
        let r = Record {
            v: 1,
            community: Some(ca),
            author,
            parent: None,
            created_at: now,
            expires_at: man.expires_at,
            body: Body::Pointer { record: pid, of: Kind::Post, media: None, sealed },
            nonce: 0,
        };
        m.n(by).publish(r).unwrap();
        m.run(120);
    };
    // a newcomer's update is ignored by every reader
    update(&mut m, "b", &man);
    assert!(m.n("f0").store.of_kind(&ca, Kind::Pointer).count() >= 2, "carried like any pointer record");
    assert_eq!(generation_of(&mut m, "r", &p.id).unwrap().hashes.len(), man.hashes.len(), "not from a newcomer");
    // a moderator's counts
    update(&mut m, "md", &man);
    assert_eq!(generation_of(&mut m, "r", &p.id).unwrap().hashes.len(), man.hashes.len() + 1, "from an established member (a moderator)");
}

/// `a` posts in an open community that `md`, a moderator's desktop, reads; then `a` goes away
/// and all but k−1 of the holders of the post's pieces with it. What `md` does then, as days
/// pass, decides whether the post can be read again.
fn below_k(m: &mut Mesh) -> (String, String, Manifest) {
    let c = open_community(m, &[("md", "192.0.2.21:7843")]);
    let md_root = m.n("md").me().unwrap().root;
    m.n("a").add_mod(&c, &md_root).unwrap();
    m.run(60);
    let p = post(m.n("a"), &c, "kept in pieces, and by its readers");
    assert!(until(m, 600, |m| m.n("md").post(&p.id).is_ok()), "md rebuilt the post");
    let man = generation_of(m, "md", &p.id).unwrap();
    m.cut.insert("a".into());
    let names: Vec<String> = m.eps.iter().filter(|(_, e)| man.holders.iter().any(|h| h.hex() == **e)).map(|(n, _)| n.clone()).collect();
    // the holders leave for good: their links to the forwarders go, and the forwarders, which
    // vouch with no uptime bar in this mesh, forget them (with the bar they stop vouching once
    // the holders' share up falls under it, a day and a half on)
    for name in names.iter().skip(man.k as usize - 1) {
        let holder = Id::from_hex(&m.eps[name]).unwrap();
        for f in 0..3 {
            let f = format!("f{f}");
            m.hang_up(&f, name);
            m.n(&f).scatter.saved.holders.remove(&holder);
        }
        m.cut.insert(name.clone());
    }
    (c, p.id, man)
}

/// Days pass, a few hours at a time, until `done` or `days`.
fn days_until(m: &mut Mesh, days: u64, done: impl Fn(&mut Mesh) -> bool) -> bool {
    for _ in 0..days * 4 {
        if done(m) {
            return true;
        }
        m.t.fetch_add(6 * 3600, std::sync::atomic::Ordering::SeqCst);
        m.run(60);
    }
    done(m)
}

#[test]
fn a_member_s_own_copy_brings_back_a_post_whose_pieces_fell_below_k() {
    let mut m = mesh(3, 30);
    let (c, p, man) = below_k(&mut m);
    assert!(days_until(&mut m, 6, |m| generation_of(m, "md", &p).is_some_and(|g| g.generation == 1)), "md made the next generation from its own copy");
    let next = generation_of(&mut m, "md", &p).unwrap();
    assert_eq!((next.object, next.sealed), (man.object, man.sealed), "the very same sealed object");
    // someone who joins now reads the post from the new generation's pieces
    let uri = m.n("md").invite(&c).unwrap().uri;
    join(&mut m, "r", "192.0.2.22:7843", &[0, 1, 2]);
    m.n("r").join(&uri).unwrap();
    assert!(until(&mut m, 900, |m| m.n("r").post(&p).is_ok()), "a newcomer rebuilt the post");
}

#[test]
fn an_own_copy_that_does_not_match_is_never_published() {
    let mut m = mesh(3, 30);
    let (c, p, man) = below_k(&mut m);
    // md keeps another post where this one should be: its copy seals to other bytes
    let other = post(m.n("md"), &c, "not the post the pointer names");
    let other = Id::from_hex(&other.id).unwrap();
    let object = Id(man.object);
    m.n("md").scatter.saved.kept.get_mut(&object).unwrap().claim = Some((other, None));
    assert!(days_until(&mut m, 4, |m| m.n("md").scatter.saved.kept[&object].copy_mismatch), "md found its copy does not match");
    assert!(generation_of(&mut m, "md", &p).is_none_or(|g| g.generation == 0) && m.n("md").scatter.saved.kept[&object].gens.len() == 1, "no new generation");
    // and it said nothing about the post: no pointer record of md's names it
    let (ca, pid) = (Id::from_hex(&c).unwrap(), Id::from_hex(&p).unwrap());
    let md = m.n("md");
    let device = md.device_kp.as_ref().unwrap().public();
    assert!(
        !md.store.of_kind(&ca, Kind::Pointer).any(|s| s.record.author == device && matches!(&s.record.body, Body::Pointer { record, .. } if *record == pid))
    );
}

#[test]
fn an_opted_in_established_desktop_publishes_its_roster_and_others_do_not() {
    let mut m = mesh(3, 14);
    let c = open_community(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843")]);
    let ca = Id::from_hex(&c).unwrap();
    let md_root = m.n("md").me().unwrap().root;
    m.n("a").add_mod(&c, &md_root).unwrap();
    m.run(60);
    for name in ["md", "b"] {
        m.n(name).set_settings(&SettingsPatch { share_holders: Some(true), ..Default::default() }).unwrap();
    }
    // an hourly sample: what a device saw itself, which is all a roster names
    m.t.fetch_add(3600, std::sync::atomic::Ordering::SeqCst);
    m.run(60);
    let rosters = |m: &mut Mesh, name: &str| {
        let n = m.n(name);
        let device = n.device_kp.as_ref().unwrap().public();
        n.store.of_kind(&ca, Kind::Roster).filter(|s| s.record.author == device).count()
    };
    assert_eq!(rosters(&mut m, "md"), 1, "md, a moderator, published its roster");
    assert_eq!(rosters(&mut m, "b"), 0, "b is not established here yet");
    assert!(until(&mut m, 300, |m| m.n("a").store.of_kind(&ca, Kind::Roster).count() == 1), "and it reached the owner");
    // not again within twelve hours, and never with the setting off
    m.run(600);
    assert_eq!(rosters(&mut m, "md"), 1);
    m.n("md").set_settings(&SettingsPatch { share_holders: Some(false), ..Default::default() }).unwrap();
    m.t.fetch_add(13 * 3600, std::sync::atomic::Ordering::SeqCst);
    m.run(60);
    assert_eq!(rosters(&mut m, "md"), 1);
    // a device that publishes rosters never says the node it connects through can't tell
    assert!(!m.n("md").settings().share_holders);
}
