//! Open blocklists (M5): a keeper's signed list, followed, refuses its hashes and near
//! copies of its pictures on every device of the follower; nobody else's list counts.
use super::*;

fn with_picture(n: &mut Node, c: &str, side: u32) -> (PostSummary, Vec<u8>) {
    let png = picture(side, image::ImageFormat::Png);
    let p = n
        .create_post(&PostDraft {
            community: c.into(),
            title: format!("picture {side}"),
            text: String::new(),
            link: None,
            media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&png), alt: None }]),
        })
        .unwrap();
    (p, png)
}

#[test]
fn a_followed_keepers_list_refuses_its_hashes_and_near_copies() {
    let (t, mut a, cid, c) = owner();
    let (exact_post, _) = with_picture(&mut a, &c, 240);
    let (near_post, _) = with_picture(&mut a, &c, 260);
    let exact = Id::from_hex(&exact_post.media[0].hash).unwrap();
    let near = Id::from_hex(&near_post.media[0].hash).unwrap();
    assert!(a.store.has_blob(&exact) && a.store.has_blob(&near));
    // the keeper lists the first picture exactly, and the second by its fingerprint (from a copy
    // of a slightly different size: near, not equal)
    let keeper = members(&mut a, &c, 1).remove(0);
    let copy = picture(250, image::ImageFormat::Png);
    let pdq = hoi_validate::similar::examine_variants(hoi_validate::media::PNG, &copy).unwrap().unwrap().hashes[0];
    let list = |at| rec(cid, None, Body::Blocklist { exact: vec![exact], pdq: vec![Id(pdq)] }, at);
    let now = t.load(Ordering::SeqCst);
    let e = keeper.send(&mut a, list(now)).unwrap_err();
    assert!(matches!(e, hoi_validate::Reject::NotAllowed(_)), "nobody here follows them: {e:?}");
    assert!(a.store.has_blob(&exact));
    // followed: both go
    a.follow_blocklist(&keeper.key(), true).unwrap();
    keeper.send(&mut a, list(now + 1)).unwrap();
    a.settle_dirty();
    assert!(!a.store.has_blob(&exact), "the exact hash is refused");
    assert!(!a.store.has_blob(&near), "and the picture close to the fingerprint");
    let k = &a.blocklist_keepers()[0];
    assert_eq!((k.exact, k.pdq), (1, 1));
    assert!(!a.state.media_blocklist.contains(&exact), "the keeper's list, not this person's own");
    // unfollowed: refused no more
    a.follow_blocklist(&keeper.key(), false).unwrap();
    assert!(!a.media_refused(&exact));
}

#[test]
fn a_keeper_publishes_a_list_from_text() {
    let (_t, mut a, _cid, c) = owner();
    let pdq = "f".repeat(64);
    let blake = format!("blake3:{}", "a".repeat(64));
    assert_eq!(a.publish_blocklist(&c, &format!("# my list\n{pdq}\n{blake}, an exact one\n")).unwrap(), 2);
    assert_eq!(a.publish_blocklist(&c, "not a hash").unwrap_err().code, "invalid");
}
