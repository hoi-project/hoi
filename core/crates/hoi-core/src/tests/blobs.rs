//! Media over iroh-blobs (M5): a peer that says `blobs` is asked that way, a node serves
//! that way only the media of communities it pins and only to an endpoint it has a link with,
//! and a fetch that fails is asked again with `bwant`.
use super::*;

fn quic(endpoint: &str) -> PeerMeta {
    PeerMeta { transport: "quic".into(), endpoint: Some(endpoint.into()), ..Default::default() }
}

#[test]
fn a_pinner_serves_its_media_over_blobs_and_a_failed_fetch_falls_back() {
    let (t, mut a, cid, c) = owner();
    let png = picture(200, image::ImageFormat::Png);
    let p = a
        .create_post(&PostDraft {
            community: c.clone(),
            title: "picture".into(),
            text: String::new(),
            link: None,
            media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&png), alt: None }]),
        })
        .unwrap();
    let hash = Id::from_hex(&p.media[0].hash).unwrap();
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lu").unwrap();
    b.join(&a.invite(&c).unwrap().web).unwrap();
    a.set_blobs(true);
    b.set_blobs(true);
    a.net_connect("b", Role::Peer);
    a.set_peer_meta("b", quic("ep-b"));
    b.net_connect("a", Role::Peer);
    b.set_peer_meta("a", quic("ep-a"));
    // the records come over the link; the picture is asked for over iroh-blobs, not with `bwant`
    let mut fetches = Vec::new();
    for _ in 0..50 {
        let out_a = a.take_outbox();
        let out_b = b.take_outbox();
        fetches.extend(b.take_blob_fetches());
        if out_a.is_empty() && out_b.is_empty() {
            break;
        }
        for (_, m) in out_a {
            assert!(b.net_receive("a", &m));
        }
        for (_, m) in out_b {
            assert!(!matches!(hoi_sync::Msg::decode(&m), Ok(hoi_sync::Msg::BWant { .. })), "no bwant while iroh-blobs is tried");
            assert!(a.net_receive("b", &m));
        }
    }
    assert!(a.peers["b"].features.contains(hoi_sync::FEATURE_BLOBS) && b.peers["a"].features.contains(hoi_sync::FEATURE_BLOBS));
    assert_eq!(fetches.len(), 1, "{fetches:?}");
    let f = &fetches[0];
    assert_eq!((f.peer.as_str(), f.endpoint.as_str(), f.hash.as_slice(), f.size), ("a", "ep-a", hash.as_bytes().as_slice(), png.len() as u64));

    // a serves it to b's endpoint only, and only while it pins the community
    assert!(a.blobs_link("ep-b") && !a.blobs_link("ep-z"));
    assert_eq!(a.blob_to_serve("ep-z", hash.as_bytes()), None, "no link, nothing");
    assert_eq!(a.blob_to_serve("ep-b", hash.as_bytes()).as_deref(), Some(png.as_slice()));
    a.set_pinning(&c, false).unwrap();
    assert_eq!(a.blob_to_serve("ep-b", hash.as_bytes()), None, "a member that doesn't pin uses bwant");
    a.set_pinning(&c, true).unwrap();
    let before = a.traffic().media_out;
    assert!(a.blob_to_serve("ep-b", hash.as_bytes()).is_some());
    assert_eq!(a.traffic().media_out, before + png.len() as u64, "counted like a blob message");

    // the fetch failed: b asks with `bwant`, and the picture comes over the link
    b.blob_fetched("a", hash.as_bytes(), Err("the peer doesn't serve it".into()));
    assert!(b.peers["a"].blobs_refused.contains(&cid));
    let asked = b.take_outbox();
    assert!(asked.iter().any(|(to, m)| to == "a" && matches!(hoi_sync::Msg::decode(m), Ok(hoi_sync::Msg::BWant { ids }) if ids == vec![hash])));
    for (_, m) in asked {
        assert!(a.net_receive("b", &m));
    }
    pump(&mut a, &mut b);
    assert!(b.store.has_blob(&hash));
    assert_eq!(b.blobs_counts(), (0, 0));
    // later fetches of this community go straight to `bwant` on this link
    assert_eq!(b.blobs_peer(&cid, &["a".to_string()]), None);
}

#[test]
fn bytes_fetched_over_blobs_get_every_check_a_blob_message_does() {
    let (t, mut a, _cid, c) = owner();
    let png = picture(200, image::ImageFormat::Png);
    let p = a
        .create_post(&PostDraft {
            community: c.clone(),
            title: "picture".into(),
            text: String::new(),
            link: None,
            media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&png), alt: None }]),
        })
        .unwrap();
    let hash = Id::from_hex(&p.media[0].hash).unwrap();
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lu").unwrap();
    b.join(&a.invite(&c).unwrap().web).unwrap();
    a.set_blobs(true);
    b.set_blobs(true);
    a.net_connect("b", Role::Peer);
    a.set_peer_meta("b", quic("ep-b"));
    b.net_connect("a", Role::Peer);
    b.set_peer_meta("a", quic("ep-a"));
    for _ in 0..50 {
        let out_a = a.take_outbox();
        let out_b = b.take_outbox();
        if out_a.is_empty() && out_b.is_empty() {
            break;
        }
        for (_, m) in out_a {
            assert!(b.net_receive("a", &m));
        }
        for (_, m) in out_b {
            assert!(a.net_receive("b", &m));
        }
    }
    assert_eq!(b.take_blob_fetches().len(), 1);
    // an answer from another peer than the one asked is ignored
    b.blob_fetched("z", hash.as_bytes(), Ok(png.clone()));
    assert!(!b.store.has_blob(&hash));
    assert!(b.blobs.trying.contains_key(&hash), "still waiting for a");
    // bytes that don't hash right never count
    b.blob_fetched("a", hash.as_bytes(), Ok(vec![0; png.len()]));
    assert!(!b.store.has_blob(&hash) && !b.blobs.trying.contains_key(&hash));
    // a browser never says `blobs`, and never asks that way
    let mut web = node("web", &t, &Mem::default());
    web.set_blobs(true);
    assert!(!web.serves_blobs());
}
