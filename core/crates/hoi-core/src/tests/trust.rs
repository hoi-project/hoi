//! Trust statements and rings (M3): friends of friends count by half, two people with
//! different rings see different Hot orders, and the views say why.
use super::*;
use hoi_types::Kind;

/// A community on `a`, five members vouched in, and `b` holding the same records.
fn two_readers() -> (Arc<AtomicU64>, Node, Node, Id, String, Vec<Actor>) {
    let (t, mut a, cid, c) = owner();
    let people = members(&mut a, &c, 5);
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lu").unwrap();
    b.join(&a.invite(&c).unwrap().web).unwrap();
    (t, a, b, cid, c, people)
}

fn connect(a: &mut Node, b: &mut Node) {
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    pump(a, b);
}

fn hot_titles(n: &Node, c: &str) -> Vec<String> {
    n.feed(Some(c), "hot", None).unwrap().into_iter().map(|p| p.title).collect()
}

#[test]
fn two_people_with_different_rings_see_different_hot_orders_and_why() {
    let (t, mut a, mut b, cid, c, people) = two_readers();
    let [poster, f1, f2, v1, v2] = &people[..] else { unreachable!() };
    let key = |x: &Actor| Id::from_hex(&x.key()).unwrap();
    let now = t.load(Ordering::SeqCst) + 60;
    t.store(now, Ordering::SeqCst);
    let (p1, p2) = (rec(cid, None, text_post("Garden tools", "which ones last"), now), rec(cid, None, text_post("Seed swaps", "spring list"), now));
    let p1 = poster.send(&mut a, p1).unwrap();
    let p2 = poster.send(&mut a, p2).unwrap();
    // f1 trusts v1, f2 trusts v2; v1 likes the first post, v2 the second
    let trust = |keys: Vec<Id>, at: u64| rec(cid, None, Body::Trust { keys }, at);
    f1.send(&mut a, trust(vec![key(v1)], now + 1)).unwrap();
    f2.send(&mut a, trust(vec![key(v2)], now + 1)).unwrap();
    v1.send(&mut a, rec(cid, Some(p1), Body::Vote { value: 1, seq: 1 }, now + 2)).unwrap();
    v2.send(&mut a, rec(cid, Some(p2), Body::Vote { value: 1, seq: 1 }, now + 2)).unwrap();
    connect(&mut a, &mut b);
    assert_eq!(b.store.of_kind(&cid, Kind::Trust).count(), 2, "both statements reached the other reader");

    // a's person is f1's friend, b's person f2's
    a.set_trust(&f1.key(), true).unwrap();
    b.set_trust(&f2.key(), true).unwrap();
    assert_eq!(hot_titles(&a, &c), vec!["Garden tools", "Seed swaps"]);
    assert_eq!(hot_titles(&b, &c), vec!["Seed swaps", "Garden tools"], "the same records, another ring, another order");

    // and why: a friend of a friend liked it, and which friend trusts them
    let top = &a.feed(Some(&c), "hot", None).unwrap()[0];
    assert_eq!((top.friends_up, top.fof_up), (0, 1));
    let v1_seen = a.person_view(&v1.key(), Some(&c)).unwrap().person;
    assert_eq!(v1_seen.trust, "fof");
    assert_eq!(v1_seen.trusted_by.len(), 1);
    // v2 is nobody's friend's friend on a's device: a key seen for a minute, a newcomer
    assert_eq!(a.person_view(&v2.key(), Some(&c)).unwrap().person.trust, "newcomer");

    // f1 takes it back: v1 is no friend of a friend any more, here, at the next settle
    f1.send(&mut a, trust(vec![], now + 100)).unwrap();
    a.settle_dirty();
    assert_ne!(a.person_view(&v1.key(), Some(&c)).unwrap().person.trust, "fof");
}

#[test]
fn sharing_whom_you_trust_is_a_choice_and_names_only_friends_known_there() {
    let (t, mut a, mut b, cid, c, people) = two_readers();
    let stranger = Actor::new();
    a.set_trust(&people[0].key(), true).unwrap();
    a.set_trust(&people[1].key(), true).unwrap();
    a.set_trust(&stranger.key(), true).unwrap();
    // nothing is published until the person says so
    assert_eq!(a.store.of_kind(&cid, Kind::Trust).count(), 0);
    assert!(!a.trust_sharing(&c).unwrap().shared);
    t.store(T0 + 120, Ordering::SeqCst);
    // only friends known here are named: people[0] and people[1] were vouched in; the stranger
    // has nothing in this community
    let mine = |n: &Node| {
        let me = n.me_root().unwrap();
        n.store.by_author(&cid, &me).filter(|s| s.record.kind() == Kind::Trust).map(|s| s.record.body.clone()).collect::<Vec<_>>()
    };
    let sharing = a.share_trust(&c, true).unwrap();
    assert!(sharing.shared);
    assert_eq!(sharing.friends, 3);
    let Body::Trust { keys } = &mine(&a)[0] else { unreachable!() };
    assert!(!keys.contains(&Id::from_hex(&stranger.key()).unwrap()));
    // a friend dropped: the statement follows at the next sweep (or at once)
    t.store(T0 + 240, Ordering::SeqCst);
    a.set_trust(&people[1].key(), false).unwrap();
    let after = mine(&a);
    assert_eq!(after.len(), 1, "the newer statement replaces the older one");
    let Body::Trust { keys: now_named } = &after[0] else { unreachable!() };
    assert!(!now_named.contains(&Id::from_hex(&people[1].key()).unwrap()));
    // taking it back leaves an empty statement in its place, which reaches others like any
    t.store(T0 + 360, Ordering::SeqCst);
    assert!(!a.share_trust(&c, false).unwrap().shared);
    assert_eq!(mine(&a), vec![Body::Trust { keys: vec![] }]);
    connect(&mut a, &mut b);
    let me = a.me_root().unwrap();
    assert_eq!(
        b.store.by_author(&cid, &me).filter(|s| s.record.kind() == Kind::Trust).map(|s| s.record.body.clone()).collect::<Vec<_>>(),
        vec![Body::Trust { keys: vec![] }]
    );
}

#[test]
fn following_a_labeler_hides_what_they_labeled_within_one_sync() {
    let (t, mut a, mut b, cid, c, people) = two_readers();
    let labeler = &people[0];
    let p = post(&mut a, &c, "Cheap watches, today only");
    let pid = Id::from_hex(&p.id).unwrap();
    let napping = |n: &Node| n.feed(Some(&c), "fresh", None).unwrap().into_iter().find(|x| x.id == p.id).map(|x| x.napping.is_some());
    // a follows the labeler, so a holds their label and passes it on
    a.follow_labeler(&labeler.key(), true).unwrap();
    let now = t.load(Ordering::SeqCst);
    labeler.send(&mut a, rec(cid, Some(pid), Body::Label { label: "spam".into(), note: None }, now)).unwrap();
    assert_eq!(napping(&a), Some(true));
    // b doesn't: the label is refused on the way, without penalty, and the post shows
    connect(&mut a, &mut b);
    assert_eq!(napping(&b), Some(false));
    assert_eq!(b.store.of_kind(&cid, Kind::Label).count(), 0);
    // b follows the labeler: one sync later the post sleeps there too
    let listed = b.follow_labeler(&labeler.key(), true).unwrap();
    assert!(listed.iter().any(|l| l.person.key == labeler.key() && l.followed));
    pump(&mut a, &mut b);
    assert_eq!(napping(&b), Some(true), "the label arrived with the next sync");
    // and wakes when b stops following
    b.follow_labeler(&labeler.key(), false).unwrap();
    assert_eq!(napping(&b), Some(false));
}

#[test]
fn a_labeler_labels_from_the_app_and_a_community_default_counts_for_everyone() {
    let (_t, mut a, mut b, _cid, c, _people) = two_readers();
    let p = post(&mut a, &c, "Holiday photos");
    // b is a labeler; the community names b as a default labeler
    let bkey = b.me().unwrap().root_key;
    let mut l = a.community(&c).unwrap().limits;
    l.labelers = vec![bkey.clone()];
    a.set_limits(&c, &l).unwrap();
    connect(&mut a, &mut b);
    b.publish_label(&p.id, "nsfw", Some("beach")).unwrap();
    assert!(b.publish_label(&p.id, " ", None).is_err());
    pump(&mut a, &mut b);
    let seen = a.feed(Some(&c), "fresh", None).unwrap().into_iter().find(|x| x.id == p.id).unwrap();
    assert_eq!(seen.napping.map(|n| n.label), Some("nsfw".to_string()), "a follows nobody, but the community names b");
    let listed = a.labelers();
    assert!(listed.iter().any(|x| x.person.key == bkey && !x.followed && x.default_in.len() == 1 && x.labels == 1));
}
