//! Community tally aggregators (M5): counts a durable node publishes show where people
//! follow it, and order the feed only when they pick that.
use super::*;
use hoi_types::Kind;

#[test]
fn an_aggregators_counts_show_where_it_is_followed_and_order_only_on_request() {
    let (t, mut a, cid, c) = owner();
    let voters = members(&mut a, &c, 3);
    let now = T0 + DAY + 10;
    t.store(now, Ordering::SeqCst);
    let quiet = post(&mut a, &c, "A quiet post");
    let loved = post(&mut a, &c, "A loved post");
    for (i, v) in voters.iter().enumerate() {
        let target = if i < 2 { &loved } else { &quiet };
        v.send(&mut a, rec(cid, Some(Id::from_hex(&target.id).unwrap()), Body::Vote { value: 1, seq: 1 }, now + i as u64)).unwrap();
    }
    assert!(a.set_aggregating(&c, true).unwrap());
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lu").unwrap();
    b.join(&a.invite(&c).unwrap().web).unwrap();
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    pump(&mut a, &mut b);
    // b doesn't follow a's counts: refused on the way, nothing shown
    assert_eq!(b.store.of_kind(&cid, Kind::Tally).count(), 0);
    assert!(b.feed(Some(&c), "fresh", None).unwrap().iter().all(|p| p.tallies.is_empty()));
    // b follows them: one sync later the counts show
    let akey = a.me().unwrap().root_key;
    b.follow_aggregator(&akey, true).unwrap();
    pump(&mut a, &mut b);
    let feed = b.feed(Some(&c), "counted", None).unwrap();
    assert_eq!(feed.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), vec!["A loved post", "A quiet post"], "ordered by the counts, on request");
    let top = &feed[0];
    assert_eq!((top.tallies.len(), top.tallies[0].up, top.tallies[0].down), (1, 2, 0));
    assert_eq!(b.aggregators()[0].communities, 1);
    // Hot is b's own: the counts don't go into it
    let hot = b.feed(Some(&c), "hot", None).unwrap();
    assert!(hot.iter().all(|p| p.tallies.len() == 1), "shown everywhere, as counts");
    // stop following: gone from the views
    b.follow_aggregator(&akey, false).unwrap();
    assert!(b.feed(Some(&c), "fresh", None).unwrap().iter().all(|p| p.tallies.is_empty()));
    // an aggregator counts again on its own, after an hour
    t.store(now + 2 * 3600, Ordering::SeqCst);
    a.sweep();
    let me = a.me_root().unwrap();
    assert_eq!(a.store.by_author(&cid, &me).filter(|s| s.record.kind() == Kind::Tally).count(), 1, "the newer replaces the older");
}
