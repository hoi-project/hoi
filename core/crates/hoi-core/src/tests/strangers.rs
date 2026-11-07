//! Strangers learn only what they need: a hello names in the clear only the
//! communities whose routes led to a connection and salts the rest; routes a registry suggested
//! are tried, never kept or passed on until they delivered records; WebRTC routes are learned
//! only for the certificate that was dialed; each device gets its own pairing credential.
use super::*;

fn route(fp: char, addr: &str) -> String {
    format!("rtc:{}@{addr}", fp.to_string().repeat(43))
}

fn relaying_node(t: &Arc<AtomicU64>, name: &str) -> Node {
    let mut n = node("node", t, &Mem::default());
    n.create_identity(name).unwrap();
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    n
}

/// `browser` dials `native` over WebRTC by `dialed`; they know each other as "a" and "b".
fn dial(native: &mut Node, browser: &mut Node, dialed: Vec<String>) {
    native.net_connect("b", Role::Peer);
    native.set_peer_meta("b", PeerMeta { transport: "webrtc".into(), incoming: true, ..Default::default() });
    browser.net_connect("a", Role::Peer);
    browser.set_peer_meta("a", PeerMeta { transport: "webrtc".into(), dialed, ..Default::default() });
}

#[test]
fn a_stranger_finds_only_shared_communities_and_a_carrier_only_what_a_route_led_to_it_for() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut carrier = relaying_node(&t, "carrier");
    let shared = Id::from_hex(&carrier.create_community(draft("garden")).unwrap().id).unwrap();
    let theirs = Id::from_hex(&carrier.create_community(draft("orchard")).unwrap().id).unwrap();
    // An invite with no routes in it: the browser reaches the carrier only by a registry's route.
    let invite = carrier.invite(&shared.hex()).unwrap().uri;
    let at = route('q', "203.0.113.10:7844");
    carrier.set_rtc_routes(vec![at.clone()]);
    let mut browser = node("web", &t, &Mem::default());
    browser.create_identity("lena").unwrap();
    browser.join(&invite).unwrap();
    let mine = Id::from_hex(&browser.create_community(draft("kitchen")).unwrap().id).unwrap();
    let asked = Id([0xd0; 32]);
    browser.public_join(&asked.hex(), std::slice::from_ref(&at), None).unwrap();

    dial(&mut carrier, &mut browser, vec![at.clone()]);
    let out = browser.take_outbox();
    assert_eq!(out.len(), 1, "one hello, rewritten in place once the host said which route it dialed");
    let Ok(hoi_sync::Msg::Hello { communities: named, salt, hashed, name, .. }) = hoi_sync::Msg::decode(&out[0].1) else { panic!("a hello") };
    assert_eq!(named, vec![asked], "only the community a route led here for is named");
    assert!(name.is_none());
    let salt = salt.expect("salted");
    let hashes: BTreeMap<Id, Id> = [shared, mine].iter().map(|c| (hoi_sync::community_hash(&salt, c), *c)).collect();
    assert_eq!(hashed.len(), 2);
    assert!(hashed.iter().all(|h| hashes.contains_key(h)), "the rest only as salted hashes");
    assert!(carrier.net_receive("b", &out[0].1));
    pump(&mut carrier, &mut browser);
    let seen_by_carrier = &carrier.peers["b"].communities;
    assert!(seen_by_carrier.contains(&shared) && seen_by_carrier.contains(&asked));
    assert!(!seen_by_carrier.contains(&mine), "a community the carrier doesn't hold stays unknown to it");
    assert_eq!(carrier.offered_unknown(), vec![asked.hex()], "a carrier can still take what a member came to it for");
    let seen_by_browser = &browser.peers["a"].communities;
    assert!(seen_by_browser.contains(&shared) && !seen_by_browser.contains(&theirs), "the browser finds only what it holds too");
    assert!(browser.feed(Some(&shared.hex()), "fresh", None).is_ok());
}

#[test]
fn a_carrier_finds_members_already_connected_when_it_takes_a_community_on() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut carrier = node("node", &t, &Mem::default());
    carrier.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    carrier.set_carry_policy(crate::CarryPolicy::Any, 8);
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let c = founder.create_community(draft("garden")).unwrap();
    let news = post(&mut founder, &c.id, "Kept by a public node");
    let mut member = node("web", &t, &Mem::default());
    member.create_identity("Lena").unwrap();
    member.join(&founder.invite(&c.id).unwrap().uri).unwrap();
    let connect = |carrier: &mut Node, peer: &str, device: &mut Node| {
        carrier.net_connect(peer, Role::Peer);
        carrier.set_peer_meta(peer, PeerMeta { transport: "webrtc".into(), incoming: true, ..Default::default() });
        device.net_connect("home", Role::Peer);
        device.set_peer_meta("home", PeerMeta { transport: "webrtc".into(), ..Default::default() });
    };
    connect(&mut carrier, "m", &mut member);
    pump_star(&mut carrier, &mut [("m", &mut member)]);
    assert!(carrier.peers["m"].communities.is_empty(), "the member's community was only a hash the carrier couldn't read");
    // The founder asks, unasked, for its own public-named community (`auto_request_carry`).
    connect(&mut carrier, "f", &mut founder);
    pump_star(&mut carrier, &mut [("m", &mut member), ("f", &mut founder)]);
    assert_eq!(carrier.carried_on_request(), vec![c.id.clone()]);
    assert!(carrier.peers["m"].communities.contains(&Id::from_hex(&c.id).unwrap()), "the member's earlier hash matches now");
    assert!(member.store.contains(&Id::from_hex(&news.id).unwrap()), "and it got the founder's post through the carrier");
}

#[test]
fn webrtc_routes_are_learned_only_for_the_certificate_that_was_dialed() {
    let t = Arc::new(AtomicU64::new(T0));
    for dialed in [true, false] {
        let mut member = relaying_node(&t, "member");
        let c = member.create_community(draft("garden")).unwrap();
        let cid = Id::from_hex(&c.id).unwrap();
        let invite = member.invite(&c.id).unwrap().uri;
        let at = route('q', "203.0.113.7:7844");
        member.set_rtc_routes(vec![at.clone()]);
        post(&mut member, &c.id, "Something to deliver");
        let mut browser = node("web", &t, &Mem::default());
        browser.create_identity("lena").unwrap();
        browser.join(&invite).unwrap();
        dial(&mut member, &mut browser, if dialed { vec![at.clone()] } else { vec![] });
        // The member's hello also claims a new address for its own certificate, and someone
        // else's route: only the first may stick.
        let moved = route('q', "198.51.100.9:7844");
        let elsewhere = route('x', "192.0.2.1:7844");
        let out = member.take_outbox();
        for (_, bytes) in out {
            let mut m = hoi_sync::Msg::decode(&bytes).unwrap();
            if let hoi_sync::Msg::Hello { routes, .. } = &mut m {
                *routes = vec![elsewhere.clone(), moved.clone(), at.clone()];
            }
            assert!(browser.net_receive("a", &m.encode()));
        }
        pump(&mut member, &mut browser);
        let learned: Vec<String> = browser.state.learned.get(&cid).map(|l| l.iter().map(|l| l.hint.clone()).collect()).unwrap_or_default();
        if dialed {
            assert_eq!(learned, vec![at.clone(), moved.clone()], "the dialed route, then the same certificate's other address");
            assert!(hoi_types::Invite::parse(&browser.invite(&c.id).unwrap().uri).unwrap().peers.contains(&at));
        } else {
            assert!(learned.is_empty(), "nothing ties these routes to the connection: none is learned");
        }
        assert!(!learned.contains(&elsewhere));
    }
}

#[test]
fn registry_routes_are_tried_and_kept_only_once_they_deliver() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut carrier = relaying_node(&t, "carrier");
    let c = carrier.create_community(draft("garden")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    post(&mut carrier, &c.id, "On the carrier");
    let at = route('q', "203.0.113.10:7844");
    carrier.set_rtc_routes(vec![at.clone()]);
    // A poisoned registry lists another node's route next to the carrier's.
    let junk = route('z', "198.51.100.66:7844");
    let memory = Mem::default();
    let mut reader = node("web", &t, &memory);
    reader.create_identity("lena").unwrap();
    reader.public_join(&c.id, &[junk.clone(), at.clone()], Some("garden")).unwrap();
    reader.commit().unwrap();
    assert!(reader.state.bootstrap.is_empty(), "registry routes are never saved");
    assert!(!String::from_utf8(memory.0.lock().unwrap().1["state"].clone()).unwrap().contains("198.51.100.66"));
    assert_eq!(reader.rtc_candidates(), vec![junk.clone(), at.clone()], "but they are tried");
    assert!(reader.community_hints(cid).0.is_empty(), "and never passed on");

    dial(&mut carrier, &mut reader, vec![at.clone()]);
    pump(&mut carrier, &mut reader);
    assert_eq!(reader.public_join_status(&c.id).unwrap().status, "joined");
    let peers = hoi_types::Invite::parse(&reader.invite(&c.id).unwrap().uri).unwrap().peers;
    assert!(peers.contains(&at), "the route that delivered records is kept and passed on: {peers:?}");
    assert!(!peers.contains(&junk), "the one that didn't is not: {peers:?}");
    let listed = reader.discover_communities();
    assert!(listed.iter().any(|d| d.id == c.id));
    assert!(listed.iter().all(|d| !hoi_types::Invite::parse(&d.invite).unwrap().peers.contains(&junk)), "nor in a catalog listing");
    reader.commit().unwrap();
    drop(reader);
    let reopened = node("web", &t, &memory);
    assert_eq!(reopened.rtc_candidates(), vec![at], "after a restart only the proven route is left");
}

#[test]
fn a_home_node_keeps_a_bounded_set_of_device_credentials() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("node", &t, &Mem::default());
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let hello = |token: &str| {
        hoi_sync::Msg::Hello {
            v: hoi_sync::SYNC_VERSION,
            node: Id::ZERO,
            name: None,
            token: Some(token.into()),
            communities: vec![],
            salt: None,
            hashed: vec![],
            serves: false,
            hints: vec![],
            routes: vec![],
            features: vec![],
            pairing: None,
            link_key: None,
            notice_hash: None,
            notice: None,
            holder: None,
            forwards: false,
            rebuilds: false,
        }
        .encode()
    };
    let mut credentials = Vec::new();
    for i in 0..33 {
        t.store(T0 + i, Ordering::SeqCst);
        let peer = format!("d{i}");
        let link = home.state.serve_token.clone().unwrap();
        home.net_connect(&peer, Role::Client);
        assert!(home.net_receive(&peer, &hello(&link)));
        let credential = home.take_outbox().into_iter().find_map(|(_, b)| match hoi_sync::Msg::decode(&b) {
            Ok(hoi_sync::Msg::Hello { token, .. }) => token,
            _ => None,
        });
        credentials.push(credential.expect("a credential of its own"));
        home.net_disconnect(&peer);
    }
    assert_eq!(home.state.paired.len(), 32);
    home.net_connect("oldest", Role::Client);
    assert!(!home.net_receive("oldest", &hello(&credentials[0])), "the device seen least recently made room");
    home.net_connect("newest", Role::Client);
    assert!(home.net_receive("newest", &hello(&credentials[32])));
}
