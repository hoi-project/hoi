//! Warmth: how many copies of a community this device can see besides its own, read from its
//! own links (PROTOCOL.md §20). A browser alone is cold; a home node or a public node makes it
//! cool; members' nodes in places of their own make it warm and hot; nodes in one network or
//! under one operator are one place; browsers count as copies but not as durable ones; a node's
//! own copy never warms its own reading.
use super::*;

/// The three documentation networks: nodes in different ones are different places.
const NETS: [&str; 3] = ["203.0.113", "198.51.100", "192.0.2"];

/// A WebRTC route with its own certificate fingerprint per node, in a network of its own for
/// consecutive `n` (three networks, then they repeat).
fn rtc(n: u8) -> String {
    format!("rtc:{}@{}.{n}:7844", ((b'a' + n % 26) as char).to_string().repeat(43), NETS[usize::from(n) % 3])
}

/// The same, all in one network: one rack, one place.
fn rtc_in_one_network(n: u8) -> String {
    format!("rtc:{}@203.0.113.{n}:7844", ((b'a' + n % 26) as char).to_string().repeat(43))
}

/// A browser dials a native node over WebRTC: the node sees it come in, the browser dialed out,
/// by the node's routes (the certificate it pinned).
fn dial(native: &mut Node, native_peer: &str, browser: &mut Node, browser_peer: &str) {
    native.net_connect(native_peer, Role::Peer);
    native.set_peer_meta(native_peer, PeerMeta { transport: "webrtc".into(), incoming: true, ..Default::default() });
    browser.net_connect(browser_peer, Role::Peer);
    browser.set_peer_meta(browser_peer, PeerMeta { transport: "webrtc".into(), dialed: native.rtc_routes.clone(), ..Default::default() });
}

/// Two native nodes over QUIC: `a` dialed `b`, whose transport authenticated `a`'s endpoint.
fn quic(a: &mut Node, a_peer: &str, b: &mut Node, b_peer: &str) {
    let a_endpoint = Keypair::from_secret(&a.transport_secret().unwrap()).public().hex();
    a.net_connect(a_peer, Role::Peer);
    a.set_peer_meta(a_peer, PeerMeta { transport: "quic".into(), ..Default::default() });
    b.net_connect(b_peer, Role::Peer);
    b.set_peer_meta(b_peer, PeerMeta { transport: "quic".into(), incoming: true, endpoint: Some(a_endpoint), ..Default::default() });
}

fn relaying(n: &mut Node) {
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
}

#[test]
fn a_browser_alone_is_cold_and_its_home_node_makes_it_cool() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Robin").unwrap();
    let c = web.create_community(draft("garden")).unwrap();
    post(&mut web, &c.id, "First");
    let alone = web.community_warmth(&c.id).unwrap();
    assert_eq!((alone.level.as_str(), alone.copies, alone.durable_copies, alone.score), ("cold", 1.0, 0.0, 12), "{alone:?}");
    assert!(!alone.own_durable && !alone.live && !alone.fresh, "no connection and no catch-up yet");
    assert_eq!(alone.complete, 1.0, "nothing is known to be missing");
    assert_eq!(web.warmth().communities.len(), 1);

    let pairing = home.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    web.pair(&pairing).unwrap();
    web.net_connect("home", Role::Home);
    web.set_peer_meta("home", PeerMeta { transport: "websocket".into(), local: true, ..Default::default() });
    home.net_connect("phone", Role::Client);
    home.set_peer_meta("phone", PeerMeta { transport: "websocket".into(), local: true, incoming: true, ..Default::default() });
    pump_star(&mut home, &mut [("phone", &mut web)]);
    let paired = web.community_warmth(&c.id).unwrap();
    assert_eq!((paired.level.as_str(), paired.copies, paired.durable_copies), ("cool", 2.0, 1.0), "{paired:?}");
    assert_eq!((paired.holders_durable, paired.holders_transient), (1, 0), "a home node outlives the tab");
    assert!(paired.live && paired.fresh);
    // The computer sees the same community: its own copy is durable, the browser's is not,
    // and its own copy does not warm its own reading: it is still the only durable one.
    let from_home = home.community_warmth(&c.id).unwrap();
    assert_eq!((from_home.level.as_str(), from_home.copies, from_home.durable_copies), ("cold", 2.0, 1.0), "{from_home:?}");
    assert_eq!((from_home.holders_durable, from_home.holders_transient), (0, 1));
    assert!(from_home.own_durable);
}

#[test]
fn members_nodes_make_a_community_warm_and_three_complete_copies_hot() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Robin").unwrap();
    let c = web.create_community(draft("garden")).unwrap();
    post(&mut web, &c.id, "First");
    let invite = web.invite(&c.id).unwrap().uri;
    let mut nodes: Vec<Node> = (0..3)
        .map(|i| {
            let mut n = node("node", &t, &Mem::default());
            relaying(&mut n);
            n.set_rtc_routes(vec![rtc(10 + i)]);
            n.create_identity(&format!("member {i}")).unwrap();
            n.join(&invite).unwrap();
            n
        })
        .collect();
    for (i, n) in nodes.iter_mut().enumerate() {
        let (a, b) = (format!("browser{i}"), format!("node{i}"));
        dial(n, &a, &mut web, &b);
        pump_named(n, &a, &mut web, &b);
        let w = web.community_warmth(&c.id).unwrap();
        let expected = ["cool", "warm", "hot"][i];
        assert_eq!(w.level, expected, "after {} member node(s): {w:?}", i + 1);
        assert_eq!(w.durable_copies, (i + 1) as f64);
        assert_eq!(w.holders_durable as usize, i + 1);
    }
    let hot = web.community_warmth(&c.id).unwrap();
    assert_eq!((hot.copies, hot.complete, hot.score), (4.0, 1.0, 85), "{hot:?}");
    assert_eq!((hot.kept, hot.places, hot.spread, hot.guarded), (3.0, 3, 1.0, 1.0), "three places, every link checked: {hot:?}");
    let links = web.warmth().links;
    assert_eq!((links.level.as_str(), links.links, links.places), ("hot", 3, 3), "{links:?}");
    assert_eq!(hot.remembered, 0, "a route is learned once a node proves it holds records, on a later exchange");
    for (i, n) in nodes.iter_mut().enumerate() {
        web.net_disconnect(&format!("node{i}"));
        n.net_disconnect(&format!("browser{i}"));
    }
    for (i, n) in nodes.iter_mut().enumerate() {
        let (a, b) = (format!("browser{i}"), format!("node{i}"));
        dial(n, &a, &mut web, &b);
        pump_named(n, &a, &mut web, &b);
    }
    assert_eq!(web.community_warmth(&c.id).unwrap().remembered, 3, "each member's route, learned from an exchange that proved its copy");
    // Stale: no connection and no catch-up for a week reads one level colder.
    for i in 0..3 {
        web.net_disconnect(&format!("node{i}"));
    }
    t.fetch_add(8 * 86_400, Ordering::SeqCst);
    let later = web.community_warmth(&c.id).unwrap();
    assert_eq!((later.level.as_str(), later.durable_copies, later.fresh), ("cold", 0.0, false), "{later:?}");
}

#[test]
fn a_public_node_reads_browsers_as_passing_copies_and_a_half_finished_catch_up_as_incomplete() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut carrier = node("node", &t, &Mem::default());
    relaying(&mut carrier);
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let c = founder.create_community(draft("garden")).unwrap();
    for i in 0..4 {
        post(&mut founder, &c.id, &format!("Post {i}"));
    }
    let invite = founder.invite(&c.id).unwrap().uri;
    carrier.create_identity("carrier").unwrap();
    carrier.join(&invite).unwrap();
    dial(&mut carrier, "b", &mut founder, "a");
    // Hellos and the founder's `have` arrive; the carrier has asked for its first page but
    // nothing has come back yet.
    for _ in 0..2 {
        for (_, m) in founder.take_outbox() {
            assert!(carrier.net_receive("b", &m));
        }
        for (_, m) in carrier.take_outbox() {
            assert!(founder.net_receive("a", &m));
        }
    }
    let half = carrier.community_warmth(&c.id).unwrap();
    assert!(half.missing > 0 && half.complete < 1.0, "{half:?}");
    assert_eq!((half.holders_durable, half.holders_transient), (0, 1), "a browser that dialed in is a passing copy");
    assert_eq!(half.level, "cold", "the node's own copy is incomplete: {half:?}");
    pump(&mut carrier, &mut founder);
    let full = carrier.community_warmth(&c.id).unwrap();
    assert_eq!((full.missing, full.complete, full.level.as_str()), (0, 1.0, "cold"), "the node holds the only durable copy: {full:?}");
    assert_eq!(full.copies, 2.0);
    assert_eq!(full.durable_copies, 1.0, "the browser never counts as durable");
    // A post made here is gossiped to the browser: it still holds a full copy.
    post(&mut carrier, &c.id, "From the node");
    pump(&mut carrier, &mut founder);
    assert_eq!(carrier.community_warmth(&c.id).unwrap().copies, 2.0);
    assert_eq!(founder.community_warmth(&c.id).unwrap().complete, 1.0);
}

#[test]
fn hints_from_a_connected_node_raise_the_network_reading_but_never_the_level() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Robin").unwrap();
    let c = web.create_community(draft("garden")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    let genesis = web.store.genesis_of(&cid).unwrap().wire.clone();
    let mut carrier = node("node", &t, &Mem::default());
    relaying(&mut carrier);
    carrier.create_identity("carrier").unwrap();
    carrier.join(&web.invite(&c.id).unwrap().uri).unwrap();
    dial(&mut carrier, "b", &mut web, "a");
    pump(&mut carrier, &mut web);
    assert_eq!(web.community_warmth(&c.id).unwrap().network_copies, 1.0, "the carrier saw only itself: {:?}", web.community_warmth(&c.id));
    // The carrier's catalog says it saw three durable copies.
    let entry = hoi_sync::CatalogEntry { genesis, peers: vec![], relays: vec![], warmth: Some(3), relayed: false, age: None };
    assert!(web.net_receive("a", &hoi_sync::Msg::Catalog { entries: vec![entry] }.encode()));
    let w = web.community_warmth(&c.id).unwrap();
    assert_eq!((w.level.as_str(), w.durable_copies, w.network_copies), ("cool", 1.0, 3.0), "{w:?}");
    web.net_disconnect("a");
    assert_eq!(web.community_warmth(&c.id).unwrap().network_copies, 0.0, "hints live with the connection");
}

#[test]
fn a_nodes_catalog_carries_its_own_quantised_warmth() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut hub = node("node", &t, &Mem::default());
    relaying(&mut hub);
    hub.create_identity("hub").unwrap();
    let c = hub.create_community(draft("garden")).unwrap();
    post(&mut hub, &c.id, "First");
    let invite = hub.invite(&c.id).unwrap().uri;
    let mut other = node("node", &t, &Mem::default());
    other.create_identity("other").unwrap();
    other.join(&invite).unwrap();
    quic(&mut other, "hub", &mut hub, "other");
    pump_named(&mut other, "hub", &mut hub, "other");
    assert_eq!(hub.community_warmth(&c.id).unwrap().durable_copies, 2.0);
    // A minute later (hints are read at most once a minute) a browser that knows nothing
    // connects: the hub's catalog lists the community with w = 1, the one copy besides its own.
    t.fetch_add(60, Ordering::SeqCst);
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Lena").unwrap();
    dial(&mut hub, "web", &mut web, "hub");
    let mut seen = None;
    for _ in 0..10 {
        for (_, m) in web.take_outbox() {
            assert!(hub.net_receive("web", &m));
        }
        for (_, m) in hub.take_outbox() {
            if let Ok(hoi_sync::Msg::Catalog { entries }) = hoi_sync::Msg::decode(&m) {
                seen = entries.first().and_then(|e| e.warmth);
            }
            assert!(web.net_receive("hub", &m));
        }
    }
    assert_eq!(seen, Some(1), "the hub tells what it saw besides itself, quantised");
    assert_eq!(web.discover_communities().len(), 1, "the listing is still just a listing");
}

#[test]
fn warmth_lists_public_communities_coldest_first_with_the_directory_reading() {
    let t = Arc::new(AtomicU64::new(T0));
    // Founded by someone else, so this browser is a member and offers nothing to carriers unasked.
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let cold = founder.create_community(draft("zebra")).unwrap();
    let cool = founder.create_community(draft("apple")).unwrap();
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Lena").unwrap();
    web.join(&founder.invite(&cold.id).unwrap().uri).unwrap();
    web.join(&founder.invite(&cool.id).unwrap().uri).unwrap();
    let mut carrier = node("node", &t, &Mem::default());
    relaying(&mut carrier);
    carrier.set_carry_policy(crate::CarryPolicy::Any, 8);
    carrier.create_identity("carrier").unwrap();
    carrier.join(&founder.invite(&cool.id).unwrap().uri).unwrap();
    let mut private = draft("secret");
    private.limits.as_mut().unwrap().admission = Some("invite".into());
    carrier.create_community(private).unwrap();
    dial(&mut carrier, "b", &mut web, "a");
    pump(&mut carrier, &mut web);
    let view = web.warmth();
    assert_eq!(view.communities.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), vec!["zebra", "apple"], "coldest first, then by name");
    assert_eq!(view.communities[0].community, cold.id);
    assert_eq!(view.communities[1].level, "cool");
    assert!(!view.own_durable);
    assert_eq!(view.directory.carry_nodes, 1, "the carrier takes requests");
    assert_eq!(view.directory.catalog_nodes, 1, "the carrier shares a catalog");
    assert_eq!(view.directory.discoverable, 1, "its open community the browser is in; the invite-only one is not listed");
    // Through the JSON dispatcher, as the apps call it.
    let json = web.call("warmth", serde_json::json!([])).unwrap();
    assert_eq!(json["communities"].as_array().unwrap().len(), 2);
    let one = web.call("communityWarmth", serde_json::json!([cool.id])).unwrap();
    assert_eq!(one["level"], "cool");
    assert!(web.call("communityWarmth", serde_json::json!([Id([9; 32]).hex()])).is_err());
}

#[test]
fn nodes_in_one_network_are_one_place_and_one_operator_is_one_place() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Robin").unwrap();
    let c = web.create_community(draft("garden")).unwrap();
    post(&mut web, &c.id, "First");
    let invite = web.invite(&c.id).unwrap().uri;
    // Three member nodes in one rack: three copies, one place.
    let mut rack: Vec<Node> = (0..3)
        .map(|i| {
            let mut n = node("node", &t, &Mem::default());
            relaying(&mut n);
            n.set_rtc_routes(vec![rtc_in_one_network(20 + i)]);
            n.create_identity(&format!("rack {i}")).unwrap();
            n.join(&invite).unwrap();
            n
        })
        .collect();
    for (i, n) in rack.iter_mut().enumerate() {
        let (a, b) = (format!("browser{i}"), format!("rack{i}"));
        dial(n, &a, &mut web, &b);
        pump_named(n, &a, &mut web, &b);
    }
    let w = web.community_warmth(&c.id).unwrap();
    assert_eq!((w.durable_copies, w.kept, w.places, w.spread), (3.0, 1.0, 1, 0.33), "{w:?}");
    assert_eq!(w.level, "cool", "one outage or one party takes all three");
    let links = web.warmth().links;
    assert_eq!((links.level.as_str(), links.places, links.largest), ("cool", 1, 1.0), "every link goes into one network: {links:?}");
    for (i, r) in rack.iter_mut().enumerate().take(3) {
        web.net_disconnect(&format!("rack{i}"));
        r.net_disconnect(&format!("browser{i}"));
    }
    // Two public nodes in different networks whose notices name the same operator are one place.
    let lena: Vec<(Node, String)> = (0..2)
        .map(|i| {
            let mut n = node("node", &t, &Mem::default());
            relaying(&mut n);
            n.set_rtc_routes(vec![rtc(30 + i)]);
            // A public node: only those say who runs them in their hello.
            n.set_node_name(Some(format!("lena-{i}")));
            n.set_carry_policy(crate::CarryPolicy::Any, 64);
            n.create_identity(&format!("lena {i}")).unwrap();
            n.join(&invite).unwrap();
            n.set_notice(Some(hoi_types::NodeNotice {
                operator: "Lena".into(),
                contact: "lena@example.org".into(),
                updated: "2026-11-03".into(),
                ..hoi_types::NodeNotice::template(&format!("lena-{i}"))
            }))
            .unwrap();
            let route = n.rtc_routes[0].clone();
            (n, route)
        })
        .collect();
    for (i, (mut n, _)) in lena.into_iter().enumerate() {
        let (a, b) = (format!("browser-l{i}"), format!("lena{i}"));
        dial(&mut n, &a, &mut web, &b);
        pump_named(&mut n, &a, &mut web, &b);
        rack.push(n);
    }
    let w = web.community_warmth(&c.id).unwrap();
    assert_eq!((w.durable_copies, w.kept, w.places), (2.0, 1.0, 1), "one operator in two networks: {w:?}");
    assert_eq!(w.level, "cool");
}

#[test]
fn a_copy_reached_only_over_an_unchecked_link_reads_colder() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut hub = node("node", &t, &Mem::default());
    relaying(&mut hub);
    hub.create_identity("hub").unwrap();
    let c = hub.create_community(draft("garden")).unwrap();
    post(&mut hub, &c.id, "First");
    let invite = hub.invite(&c.id).unwrap().uri;
    let mut other = node("node", &t, &Mem::default());
    other.create_identity("other").unwrap();
    other.join(&invite).unwrap();
    // `other` dialed in, and its transport did not say who it is: a copy, but not a guarded one.
    other.net_connect("hub", Role::Peer);
    other.set_peer_meta("hub", PeerMeta { transport: "quic".into(), ..Default::default() });
    hub.net_connect("other", Role::Peer);
    hub.set_peer_meta("other", PeerMeta { transport: "quic".into(), incoming: true, addr: Some("198.51.100.7:4433".into()), ..Default::default() });
    pump_named(&mut other, "hub", &mut hub, "other");
    let w = hub.community_warmth(&c.id).unwrap();
    assert_eq!((w.kept, w.guarded), (1.0, 0.0), "{w:?}");
    assert_eq!(w.level, "cold", "one copy elsewhere, over an unchecked link: one level colder than cool");
    assert_eq!(hub.warmth().links.guarded, 0.0);
}
