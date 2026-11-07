//! Public nodes pass each other on (crate::nodes): a device that reached one public node finds
//! the others, with nothing configured; lists from strangers make nobody dial anyone; routes on
//! a private network stay on it.
use super::*;
use hoi_sync::Msg;

fn fp(c: char) -> String {
    c.to_string().repeat(43)
}

/// A native node that serves strangers (relaying, taking carry requests: what every home node
/// does by default), with an endpoint and routes at `ip`.
fn public(t: &Arc<AtomicU64>, memory: &Mem, ip: &str, cert: char) -> (Node, String) {
    let mut n = node("node", t, memory);
    n.set_node_name(Some("public node".into()));
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    n.set_carry_policy(crate::CarryPolicy::Any, 64);
    let endpoint = Keypair::from_secret(&n.transport_secret().unwrap()).public().hex();
    n.set_transport_address(Some(endpoint.clone()), vec![format!("{endpoint}@{ip}:7843")], vec![]);
    n.set_rtc_routes(vec![format!("rtc:{}@{ip}:7844", fp(cert))]);
    (n, endpoint)
}

/// `a` dials `b` over QUIC: each knows the other's authenticated endpoint and address.
fn quic(a: &mut Node, a_ep: &str, a_ip: &str, b: &mut Node, b_ep: &str, b_ip: &str) {
    a.net_connect("b", Role::Peer);
    a.set_peer_meta("b", PeerMeta { transport: "quic".into(), endpoint: Some(b_ep.into()), addr: Some(format!("{b_ip}:7843")), ..Default::default() });
    b.net_connect("a", Role::Peer);
    b.set_peer_meta(
        "a",
        PeerMeta { transport: "quic".into(), incoming: true, endpoint: Some(a_ep.into()), addr: Some(format!("{a_ip}:7843")), ..Default::default() },
    );
}

/// A browser dials `node`'s WebRTC route from `from`: the node knows it as "w", it knows the node as "a".
fn browser_dials(browser: &mut Node, node: &mut Node, route: &str, from: &str) {
    node.net_connect("w", Role::Peer);
    node.set_peer_meta("w", PeerMeta { transport: "webrtc".into(), incoming: true, addr: Some(format!("{from}:50000")), ..Default::default() });
    browser.net_connect("a", Role::Peer);
    browser.set_peer_meta("a", PeerMeta { transport: "webrtc".into(), dialed: vec![route.into()], ..Default::default() });
}

fn lists(n: &mut Node, to: &str) -> Vec<Vec<String>> {
    n.take_outbox()
        .into_iter()
        .filter(|(p, _)| p == to)
        .filter_map(|(_, m)| match Msg::decode(&m) {
            Ok(Msg::Nodes { routes, .. }) => Some(routes),
            _ => None,
        })
        .collect()
}

#[test]
fn a_browser_that_reached_one_public_node_finds_the_others_with_nothing_configured() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let (mut a, a_ep) = public(&t, &memory, "203.0.113.1", 'a');
    let (mut b, b_ep) = public(&t, &Mem::default(), "203.0.113.2", 'b');
    // A dials B (from an invite, say): B answers as a public node, so A keeps it.
    quic(&mut a, &a_ep, "203.0.113.1", &mut b, &b_ep, "203.0.113.2");
    pump_named(&mut a, "b", &mut b, "a");
    let known = &a.state.nodes[0];
    assert!(known.seen > 0);
    assert!(known.routes.contains(&format!("{b_ep}@203.0.113.2:7843")), "the address its authenticated connection came from");
    assert!(known.routes.contains(&format!("rtc:{}@203.0.113.2:7844", fp('b'))), "its WebRTC route on that same address");
    // B was dialed into, so it learned nothing about A from A's list, only A's own address to try.
    assert!(b.state.nodes.is_empty(), "a node that dialed in proves nothing");
    assert_eq!(b.node_candidates.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(), vec![format!("{a_ep}@203.0.113.1:7843")]);
    assert!(a.net_status().directory.connected == 1 && a.net_status().directory.known == 1);

    // A browser, knowing nothing, opens a link to A. It connects, and hears of B.
    let link = a.net_status().node_link.expect("a public node has a link to share");
    assert!(link.starts_with("https://hoi.sh/app/#/node?code="), "{link}");
    assert!(!link.contains("203.0.113.1"), "its addresses are not readable from the link: {link}");
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Robin").unwrap();
    assert_eq!(web.directory_status(), DirectoryStatus::default(), "nothing in a browser names a node");
    assert_eq!(web.open_node_link(&link).unwrap().waiting, 1);
    let a_rtc = format!("rtc:{}@203.0.113.1:7844", fp('a'));
    assert_eq!(web.discovery_candidates(), vec![a_rtc.clone()], "the link is tried first");
    browser_dials(&mut web, &mut a, &a_rtc, "198.51.100.9");
    pump_named(&mut web, "a", &mut a, "w");
    let status = web.directory_status();
    assert_eq!((status.known, status.connected), (1, 1), "A answered as a public node: the link's node is kept");
    assert_eq!(web.discovery_candidates(), vec![format!("rtc:{}@203.0.113.2:7844", fp('b'))], "and B is next to try");
    assert!(web.net_status().node_link.is_none(), "a browser serves nobody and has no link to share");

    // Kept across restarts: the next start has somewhere to look.
    web.commit().unwrap();
    a.commit().unwrap();
    let again = node("node", &t, &memory);
    assert_eq!(again.directory_status().known, 1);
}

#[test]
fn a_list_from_a_peer_that_dialed_in_makes_nobody_dial_anyone() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let (mut s, s_ep) = public(&t, &Mem::default(), "203.0.113.9", 's');
    quic(&mut s, &s_ep, "203.0.113.9", &mut a, &a_ep, "203.0.113.1");
    pump_named(&mut s, "b", &mut a, "a");
    let victim = format!("rtc:{}@203.0.113.66:7844", fp('v'));
    let other = format!("{}@203.0.113.67:7843", Id([7; 32]).hex());
    assert!(a.net_receive("a", &Msg::Nodes { routes: vec![victim.clone(), other.clone()], holders: vec![] }.encode()));
    let candidates: Vec<String> = a.node_candidates.iter().map(|(r, _)| r.clone()).collect();
    assert_eq!(candidates, vec![format!("{s_ep}@203.0.113.9:7843")], "only the caller's own address, never what it listed");
    assert!(a.state.nodes.is_empty());
    // Six lists a minute from one peer are read; the rest are ignored, not obeyed.
    for _ in 0..10 {
        assert!(s.net_receive("b", &Msg::Nodes { routes: vec![victim.clone()], holders: vec![] }.encode()));
    }
    assert_eq!(s.node_candidates.iter().filter(|(r, _)| *r == victim).count(), 1);
    // A list from a peer that never said it understands one ends the link, like any unadvertised message.
    let mut quiet = node("node", &t, &Mem::default());
    quiet.net_connect("x", Role::Peer);
    let hello = Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([1; 32]),
        name: None,
        token: None,
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
    };
    assert!(quiet.net_receive("x", &hello.encode()));
    assert!(!quiet.net_receive("x", &Msg::Nodes { routes: vec![], holders: vec![] }.encode()));
}

#[test]
fn routes_on_a_private_network_or_this_machine_stay_there() {
    let t = Arc::new(AtomicU64::new(T0));
    let lan = format!("rtc:{}@192.168.1.20:7844", fp('l'));
    let wide = format!("rtc:{}@203.0.113.5:7844", fp('i'));
    let here = format!("rtc:{}@127.0.0.1:7844", fp('m'));
    // The browser dials the node on the network it shares with it, as it would.
    let heard = |from: &str, node_ip: &str| {
        let a_rtc = format!("rtc:{}@{node_ip}:7844", fp('a'));
        let (mut a, _) = public(&t, &Mem::default(), "203.0.113.1", 'a');
        for r in [&here, &lan, &wide] {
            a.state.nodes.push(crate::state::KnownNode { routes: vec![r.clone()], seen: T0 });
        }
        let mut web = node("web", &t, &Mem::default());
        browser_dials(&mut web, &mut a, &a_rtc, from);
        pump_named(&mut web, "a", &mut a, "w");
        let mut got = web.discovery_candidates();
        got.sort();
        got
    };
    assert_eq!(heard("198.51.100.9", "203.0.113.1"), vec![wide.clone()], "a browser on the internet hears of nodes on the internet");
    let mut both = vec![lan.clone(), wide.clone()];
    both.sort();
    assert_eq!(heard("192.168.1.30", "192.168.1.10"), both, "one on a local network hears of that network's nodes too");
    let mut all = vec![here.clone(), lan.clone(), wide.clone()];
    all.sort();
    assert_eq!(heard("127.0.0.1", "127.0.0.1"), all, "one on this machine hears of everything");

    // Taking works the same way: a node on the internet can't point us into our own network.
    let (mut b, b_ep) = public(&t, &Mem::default(), "203.0.113.2", 'b');
    let (mut c, c_ep) = public(&t, &Mem::default(), "203.0.113.3", 'c');
    quic(&mut b, &b_ep, "203.0.113.2", &mut c, &c_ep, "203.0.113.3");
    pump_named(&mut b, "b", &mut c, "a");
    assert!(b.net_receive("b", &Msg::Nodes { routes: vec![lan.clone(), here.clone(), wide.clone()], holders: vec![] }.encode()));
    let taken: Vec<String> = b.node_candidates.iter().map(|(r, _)| r.clone()).collect();
    assert_eq!(taken, vec![wide], "routes into a private network from the internet are dropped");
}

#[test]
fn a_home_node_that_keeps_to_itself_is_no_public_node() {
    // Relaying for its communities' members, but taking no carry requests (`--no-public-relay`).
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let (mut b, b_ep) = public(&t, &Mem::default(), "203.0.113.2", 'b');
    b.set_carry_policy(crate::CarryPolicy::Off, 64);
    assert!(b.net_status().relaying && b.net_status().node_link.is_none());
    quic(&mut a, &a_ep, "203.0.113.1", &mut b, &b_ep, "203.0.113.2");
    pump_named(&mut a, "b", &mut b, "a");
    assert!(a.state.nodes.is_empty(), "it sent no list, so it isn't passed on");
    assert!(b.directory_hints().is_empty(), "and it dials no public nodes of its own accord");
}

#[test]
fn a_node_that_serves_no_strangers_sends_no_list_and_is_not_kept() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let (mut b, b_ep) = public(&t, &Mem::default(), "203.0.113.2", 'b');
    b.set_settings(&SettingsPatch { relay: Some(false), ..Default::default() }).unwrap();
    assert!(b.net_status().node_link.is_none(), "no link to share while it serves nobody");
    quic(&mut a, &a_ep, "203.0.113.1", &mut b, &b_ep, "203.0.113.2");
    let to_a = lists(&mut b, "a");
    assert!(to_a.is_empty(), "no list before the peer's hello, and none at all while not relaying");
    a.take_outbox();
    assert!(a.state.nodes.is_empty());
}

#[test]
fn a_failed_dial_drops_what_others_passed_on_and_rests_a_known_node() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, _) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let b_rtc = format!("rtc:{}@203.0.113.2:7844", fp('b'));
    a.state.nodes.push(crate::state::KnownNode { routes: vec![b_rtc.clone()], seen: T0 });
    let a_rtc = format!("rtc:{}@203.0.113.1:7844", fp('a'));
    let mut web = node("web", &t, &Mem::default());
    browser_dials(&mut web, &mut a, &a_rtc, "198.51.100.9");
    pump_named(&mut web, "a", &mut a, "w");
    assert_eq!(web.discovery_candidates(), vec![b_rtc.clone()]);
    web.dial_failed(&b_rtc);
    assert!(web.discovery_candidates().is_empty(), "a route another node passed on is dropped once it failed");
    // A node this device reached itself rests for ten minutes, then is tried again.
    web.net_disconnect("a");
    assert_eq!(web.discovery_candidates(), vec![a_rtc.clone()]);
    web.dial_failed(&a_rtc);
    assert!(web.discovery_candidates().is_empty());
    t.fetch_add(601, Ordering::SeqCst);
    assert_eq!(web.discovery_candidates(), vec![a_rtc]);
}

#[test]
fn a_node_link_is_an_address_and_nothing_more() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut web = node("web", &t, &Mem::default());
    assert!(web.open_node_link("https://hoi.sh/app/#/join?c=abc").is_err(), "an invite is not a node link");
    assert!(web.open_node_link("hoi://node?w=short@203.0.113.1:7844").is_err(), "a link without a usable route");
    let link = hoi_types::NodeLink { rtc: vec![format!("rtc:{}@203.0.113.1:7844", fp('a'))], ..Default::default() }.to_uri();
    assert_eq!(web.open_node_link(&link).unwrap().waiting, 1);
    assert_eq!(web.open_node_link(&link).unwrap().waiting, 1, "opening it again keeps one entry");
    assert_eq!(web.directory_status().known, 0, "it counts as known only once it answered as a public node");
    // A home node's own link opened on it is no route anywhere.
    let (mut a, _) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let own = a.net_status().node_link.unwrap();
    assert!(a.open_node_link(&own).is_err());
}
