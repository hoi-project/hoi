//! Only public nodes publish an address: a desktop the person didn't make a public
//! node gives strangers, invites and its devices' invites no internet address of its own and
//! says `unlisted`, which nodes honour by remembering it nowhere; invites lead to public nodes
//! that held the community; and a public node offers what too few hold to other public nodes,
//! a couple a day.
use super::*;
use hoi_sync::Msg;

fn fp(c: char) -> String {
    c.to_string().repeat(43)
}

/// A desktop at home: on the home network as 192.168.1.20, and with a public IPv6 address of
/// its own, as many computers have.
fn desktop(t: &Arc<AtomicU64>) -> (Node, String) {
    let mut n = node("desktop", t, &Mem::default());
    n.create_identity("Lena").unwrap();
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    n.set_carry_policy(crate::CarryPolicy::Any, 64);
    let endpoint = Keypair::from_secret(&n.transport_secret().unwrap()).public().hex();
    n.set_transport_address(Some(endpoint.clone()), vec![format!("{endpoint}@192.168.1.20:7843"), format!("{endpoint}@[2001:db8::20]:7843")], vec![]);
    n.set_rtc_routes(vec![format!("rtc:{}@192.168.1.20:7844", fp('d')), format!("rtc:{}@[2001:db8::20]:7844", fp('d'))]);
    (n, endpoint)
}

/// A headless public node at `ip`, as `hoi-node` runs by default.
fn public(t: &Arc<AtomicU64>, ip: &str, cert: char) -> (Node, String) {
    let mut n = node("node", t, &Mem::default());
    n.set_node_name(Some("public node".into()));
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    n.set_carry_policy(crate::CarryPolicy::Any, 64);
    let endpoint = Keypair::from_secret(&n.transport_secret().unwrap()).public().hex();
    n.set_transport_address(Some(endpoint.clone()), vec![format!("{endpoint}@{ip}:7843")], vec![]);
    n.set_rtc_routes(vec![format!("rtc:{}@{ip}:7844", fp(cert))]);
    (n, endpoint)
}

/// `a` dials `b` over QUIC from `a_addr`: they know each other as "b" and "a".
fn quic(a: &mut Node, b: &mut Node, b_ep: &str, b_addr: &str, a_ep: &str, a_addr: &str) {
    a.net_connect("b", Role::Peer);
    a.set_peer_meta("b", PeerMeta { transport: "quic".into(), endpoint: Some(b_ep.into()), addr: Some(b_addr.into()), ..Default::default() });
    b.net_connect("a", Role::Peer);
    b.set_peer_meta("a", PeerMeta { transport: "quic".into(), incoming: true, endpoint: Some(a_ep.into()), addr: Some(a_addr.into()), ..Default::default() });
}

fn hello_to_stranger(n: &mut Node) -> (Vec<String>, Vec<String>) {
    n.take_outbox();
    n.net_connect("stranger", Role::Peer);
    for (to, m) in n.take_outbox() {
        if let (true, Ok(Msg::Hello { routes, features, .. })) = (to == "stranger", Msg::decode(&m)) {
            n.net_disconnect("stranger");
            return (routes, features);
        }
    }
    panic!("no hello");
}

fn invite_peers(n: &Node, c: &str) -> Vec<String> {
    hoi_types::Invite::parse(&n.invite(c).unwrap().uri).unwrap().peers
}

#[test]
fn a_desktop_that_is_not_a_public_node_publishes_no_internet_address() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut d, _) = desktop(&t);
    let c = d.create_community(draft("garden")).unwrap().id;
    d.set_serving(Some("ws://127.0.0.1:7842")).unwrap();

    let (routes, features) = hello_to_stranger(&mut d);
    assert!(routes.iter().all(|r| !r.contains("2001:db8")), "no internet address to a stranger: {routes:?}");
    assert!(routes.iter().any(|r| r.contains("192.168.1.20")), "the home network still hears of it");
    assert!(features.contains(&hoi_sync::FEATURE_UNLISTED.to_string()) && !features.contains(&hoi_sync::FEATURE_CARRY.to_string()));
    assert!(invite_peers(&d, &c).iter().all(|r| !r.contains("2001:db8")));
    let inv = d.invite(&c).unwrap();
    assert!(!inv.outside, "nobody on another network can use it");
    assert!(inv.addresses.iter().all(|a| a.scope != "internet" && a.whose == "yours"), "{:?}", inv.addresses);
    assert!(d.net_status().node_link.is_none());
    // The person's own devices still get every route: a phone away from home pairs over IPv6.
    let pair = hoi_types::uri::Pair::parse(&d.net_status().serving.unwrap().pairing).unwrap();
    assert!(pair.rtc.iter().any(|route| route.contains("2001:db8")), "the pairing link keeps them");
    // And the app says exactly that, address by address.
    let told = |n: &Node, addr: &str| n.net_status().exposure.into_iter().find(|e| e.address == addr).map(|e| (e.scope, e.told)).unwrap();
    assert_eq!(told(&d, "[2001:db8::20]:7844"), ("internet".into(), vec!["devices".into()]));
    assert_eq!(told(&d, "192.168.1.20:7844"), ("network".into(), vec!["devices".into(), "strangers".into()]));

    // Made a public node, it publishes them: in its hello, its invites and its node link.
    d.set_settings(&SettingsPatch { public_node: Some(true), ..Default::default() }).unwrap();
    let (routes, features) = hello_to_stranger(&mut d);
    assert!(routes.iter().any(|r| r.contains("2001:db8")));
    assert!(!features.contains(&hoi_sync::FEATURE_UNLISTED.to_string()) && features.contains(&hoi_sync::FEATURE_CARRY.to_string()));
    let inv = d.invite(&c).unwrap();
    assert!(inv.outside && inv.addresses.iter().any(|a| a.address == "[2001:db8::20]:7844" && a.scope == "internet" && a.whose == "yours"));
    let routes = |l: String| hoi_types::NodeLink::parse(&l).map(|n| n.rtc.into_iter().chain(n.peers).collect::<Vec<_>>()).unwrap_or_default();
    assert!(d.net_status().node_link.map(routes).is_some_and(|r| r.iter().any(|r| r.contains("2001"))));
    assert_eq!(told(&d, "[2001:db8::20]:7844"), ("internet".into(), vec!["devices".into(), "strangers".into(), "node-link".into()]));
    // What the router forwards is named as such.
    d.set_port_mapping(Some(crate::dto::PortMapping { state: "forwarded".into(), public: vec!["203.0.113.7:7844".into()], ..Default::default() }));
    d.set_rtc_routes(vec![format!("rtc:{}@203.0.113.7:7844", fp('d')), format!("rtc:{}@192.168.1.20:7844", fp('d'))]);
    let forwarded = d.net_status().exposure.into_iter().find(|e| e.address == "203.0.113.7:7844").unwrap();
    assert!(forwarded.from_router && forwarded.scope == "internet");
}

#[test]
fn a_node_that_publishes_no_address_is_remembered_nowhere_and_invites_lead_to_public_nodes() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut carrier, carrier_ep) = public(&t, "203.0.113.1", 'p');
    // A browser founds the community and posts through the public node.
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let c = founder.create_community(draft("garden")).unwrap().id;
    let cid = Id::from_hex(&c).unwrap();
    post(&mut founder, &c, "hello from the browser");
    carrier.join(&founder.invite(&c).unwrap().uri).unwrap();
    carrier.net_connect("w", Role::Peer);
    carrier.set_peer_meta("w", PeerMeta { transport: "webrtc".into(), incoming: true, addr: Some("198.51.100.9:50000".into()), ..Default::default() });
    founder.net_connect("a", Role::Peer);
    founder.set_peer_meta("a", PeerMeta { transport: "webrtc".into(), dialed: vec![format!("rtc:{}@203.0.113.1:7844", fp('p'))], ..Default::default() });
    pump_named(&mut founder, "a", &mut carrier, "w");

    // A desktop that isn't a public node joins, posts, and catches up through the public node,
    // dialing it from its home's public address.
    let (mut d, d_ep) = desktop(&t);
    d.join(&founder.invite(&c).unwrap().uri).unwrap();
    post(&mut d, &c, "hello from the desktop");
    quic(&mut d, &mut carrier, &carrier_ep, "203.0.113.1:7843", &d_ep, "198.51.100.7:40000");
    pump_named(&mut d, "b", &mut carrier, "a");
    assert_eq!(d.store.in_community(&cid).count(), carrier.store.in_community(&cid).count(), "both have everything");

    // The public node learned nothing to pass on about the desktop, nor the browser.
    let theirs = invite_peers(&carrier, &c);
    assert!(theirs.iter().all(|r| !r.contains("198.51.100.7") && !r.contains("198.51.100.9")), "{theirs:?}");
    assert!(carrier.state.learned.get(&cid).is_none_or(|l| l.is_empty()));

    // The desktop's invite leads to the public node first, its own home address last.
    let ours = invite_peers(&d, &c);
    let first_address = ours.iter().find(|r| r.contains('@')).unwrap();
    assert!(first_address.contains("203.0.113.1"), "{ours:?}");
    let inv = d.invite(&c).unwrap();
    assert!(inv.outside);
    assert!(inv.addresses.iter().any(|a| a.address == "203.0.113.1:7843" && a.whose == "node"));
    assert!(inv.addresses.iter().all(|a| a.scope != "internet" || a.whose == "node"), "{:?}", inv.addresses);

    // A headless node publishes its address, and is remembered as before once it delivers.
    post(&mut founder, &c, "late news");
    let (mut other, other_ep) = public(&t, "198.51.100.8", 'o');
    other.join(&founder.invite(&c).unwrap().uri).unwrap();
    other.import_bundle(&founder.export_bundle(&c).unwrap()).unwrap();
    quic(&mut other, &mut carrier, &carrier_ep, "203.0.113.1:7843", &other_ep, "198.51.100.8:7843");
    pump_named(&mut other, "b", &mut carrier, "a");
    assert!(invite_peers(&carrier, &c).iter().any(|r| r.contains("198.51.100.8")), "{:?}", invite_peers(&carrier, &c));
}

#[test]
fn a_public_node_offers_what_too_few_hold_to_a_couple_of_others_a_day() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let c = founder.create_community(draft("garden")).unwrap().id;
    let cid = Id::from_hex(&c).unwrap();
    let (mut a, a_ep) = public(&t, "203.0.113.1", 'a');
    a.join(&founder.invite(&c).unwrap().uri).unwrap();

    let mut carriers = 0;
    for (i, ip) in ["203.0.113.2", "203.0.113.3", "203.0.113.4"].into_iter().enumerate() {
        let (mut other, other_ep) = public(&t, ip, char::from(b'b' + i as u8));
        quic(&mut a, &mut other, &other_ep, &format!("{ip}:7843"), &a_ep, "203.0.113.1:7843");
        pump_named(&mut a, "b", &mut other, "a");
        if other.state.carried.contains_key(&cid) {
            carriers += 1;
        }
        // Gone again: this node sees no more copies than before.
        a.net_disconnect("b");
    }
    assert_eq!(carriers, crate::carry::SPREAD_PER_DAY as usize, "two other public nodes a day were asked, the third was not");

    t.fetch_add(86_400, Ordering::SeqCst);
    let (mut next, next_ep) = public(&t, "203.0.113.5", 'e');
    quic(&mut a, &mut next, &next_ep, "203.0.113.5:7843", &a_ep, "203.0.113.1:7843");
    pump_named(&mut a, "b", &mut next, "a");
    assert!(next.state.carried.contains_key(&cid), "a new day, a new offer");

    // A desktop that isn't a public node asks for nothing it didn't found.
    let (mut d, d_ep) = desktop(&t);
    d.join(&founder.invite(&c).unwrap().uri).unwrap();
    let (mut far, far_ep) = public(&t, "203.0.113.6", 'f');
    quic(&mut d, &mut far, &far_ep, "203.0.113.6:7843", &d_ep, "198.51.100.7:40000");
    pump_named(&mut d, "b", &mut far, "a");
    assert!(!far.state.carried.contains_key(&cid));
}

/// A browser that joined with an invite whose routes are all gone asks the public node it is
/// linked to; the node knows a carrier and says so; the carrier's routes go on trial.
#[test]
fn a_device_finds_carriers_through_a_public_node_that_answers_find() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut owner = node("desktop", &t, &Mem::default());
    owner.create_identity("mira").unwrap();
    let c = owner.create_community(draft("garden")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    let (mut carrier, carrier_ep) = public(&t, "203.0.113.7", 'c');
    carrier.join(&owner.invite(&c.id).unwrap().uri).unwrap();
    let (mut hub, hub_ep) = public(&t, "198.51.100.9", 'h');
    // a public node whose operator won't carry this one: it still answers `find`
    hub.set_refused(std::slice::from_ref(&c.id)).unwrap();
    // the hub dials the carrier and hears what it holds
    quic(&mut hub, &mut carrier, &carrier_ep, "203.0.113.7:7843", &hub_ep, "198.51.100.9:50000");
    pump_named(&mut hub, "b", &mut carrier, "a");
    assert!(!hub.state.joined.contains_key(&cid), "the hub doesn't carry it");

    let mut web = node("web", &t, &Mem::default());
    web.create_identity("kim").unwrap();
    let mut inv = hoi_types::Invite::parse(&owner.invite(&c.id).unwrap().uri).unwrap();
    inv.peers.clear();
    inv.relays.clear();
    web.join(&inv.to_uri()).unwrap();
    web.net_connect("h", Role::Peer);
    web.set_peer_meta("h", PeerMeta { transport: "webrtc".into(), ..Default::default() });
    hub.net_connect("w", Role::Peer);
    hub.set_peer_meta("w", PeerMeta { transport: "webrtc".into(), incoming: true, ..Default::default() });
    pump_named(&mut web, "h", &mut hub, "w");
    let trial = web.trial_routes.get(&cid).cloned().unwrap_or_default();
    assert!(trial.iter().any(|r| r.contains("203.0.113.7")), "the carrier's routes are on trial: {trial:?}");
    assert!(!web.state.bootstrap.get(&cid).is_some_and(|h| h.peers.iter().any(|r| r.contains("203.0.113.7"))), "only tried, never kept");

    // asking again so soon sends nothing
    web.take_outbox();
    web.find_missing();
    assert!(!web.take_outbox().iter().any(|(_, m)| matches!(Msg::decode(m), Ok(Msg::Find { .. }))));
}

/// What a public node's DHT answer port says for a community it carries is signed by its
/// endpoint key and checked before any route is tried.
#[test]
fn dht_answers_are_signed_for_one_community_and_expire() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut owner = node("desktop", &t, &Mem::default());
    owner.create_identity("mira").unwrap();
    let c = owner.create_community(draft("garden")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    let (mut carrier, carrier_ep) = public(&t, "203.0.113.7", 'c');
    carrier.join(&owner.invite(&c.id).unwrap().uri).unwrap();
    assert!(carrier.dht_answer(&cid).is_none(), "no DHT running: no answer");
    carrier.set_dht(Some(crate::find::DhtPorts { dht: 7845, answer: 7846 }));
    assert_eq!(carrier.dht_communities(), vec![cid]);
    let answer = carrier.dht_answer(&cid).unwrap();
    let routes = crate::find::dht_routes(&answer, &cid, T0).unwrap();
    assert!(routes.contains(&format!("{carrier_ep}@203.0.113.7:7843")));
    assert!(routes.iter().any(|r| r.starts_with("rtc:")));
    // for another community, tampered, or old: nothing
    assert!(crate::find::dht_routes(&answer, &Id([9; 32]), T0).is_none());
    let mut bad = answer.clone();
    let i = bad.len() - 3;
    bad[i] ^= 1;
    assert!(crate::find::dht_routes(&bad, &cid, T0).is_none());
    assert!(crate::find::dht_routes(&answer, &cid, T0 + crate::find::DHT_ANSWER_TTL + 1).is_none());
    assert!(carrier.dht_answer(&Id([9; 32])).is_none(), "a community it doesn't carry");
    // and a DHT contact it gives others: its public IPv4 at the DHT port
    assert_eq!(carrier.own_dht_contacts(), vec!["203.0.113.7:7845"]);
    assert_eq!(crate::find::dht_key(&cid).len(), 20);
}

/// Channel binding: a node binds the WebRTC channel to the endpoint key its hello names; the
/// browser takes nothing else from the link until that checks out, and ends a link whose binding
/// is for another channel.
#[test]
fn a_node_binds_its_webrtc_channel_to_its_key() {
    let t = Arc::new(AtomicU64::new(T0));
    let channel = crate::bind::RtcChannel { node: vec![1; 32], browser: vec![2; 32], ufrag: format!("h1p{}abcdefgh", "A".repeat(43)) };
    let link = |browser_sees: crate::bind::RtcChannel| {
        let (mut node, _) = public(&t, "203.0.113.9", 'n');
        let mut web = node_("web", &t);
        web.net_connect("n", Role::Peer);
        web.set_peer_meta("n", PeerMeta { transport: "webrtc".into(), rtc: Some(browser_sees), ..Default::default() });
        node.net_connect("w", Role::Peer);
        node.set_peer_meta("w", PeerMeta { transport: "webrtc".into(), incoming: true, rtc: Some(channel.clone()), ..Default::default() });
        // the node's hello, then its binding, before anything else it sends
        let out: Vec<Msg> = node.take_outbox().into_iter().filter(|(p, _)| p == "w").map(|(_, m)| Msg::decode(&m).unwrap()).collect();
        assert!(matches!(out.first(), Some(Msg::Hello { .. })));
        assert!(matches!(out.get(1), Some(Msg::Bind { .. })), "the binding follows the hello");
        let kept: Vec<bool> = out.iter().map(|m| web.net_receive("n", &m.encode())).collect();
        (web, kept)
    };
    let (web, kept) = link(channel.clone());
    assert!(kept.iter().all(|k| *k));
    assert!(web.peers["n"].bound);
    // a binding for another channel (a node forwarding someone else's hello, say) ends the link
    let (_, kept) = link(crate::bind::RtcChannel { node: vec![9; 32], ..channel.clone() });
    assert_eq!(kept.get(1), Some(&false));
    // and before the binding, nothing but a hello, a bind or a goodbye is taken
    let mut web = node_("web", &t);
    web.net_connect("n", Role::Peer);
    web.set_peer_meta("n", PeerMeta { transport: "webrtc".into(), rtc: Some(channel), ..Default::default() });
    let hello = Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([5; 32]),
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
    assert!(web.net_receive("n", &hello.encode()));
    assert!(!web.net_receive("n", &Msg::Nodes { routes: vec![], holders: vec![] }.encode()), "nothing before the binding");
}

/// A paired device on WebRTC: the node holds its hello for the device's credential, and the
/// binding follows that hello, never before it.
#[test]
fn a_home_node_binds_after_the_hello_it_holds_for_a_device() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node_("desktop", &t);
    home.create_identity("mira").unwrap();
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let endpoint = Keypair::from_secret(&home.transport_secret().unwrap()).public().hex();
    home.set_transport_address(Some(endpoint), vec![], vec![]);
    let mut phone = node_("web", &t);
    phone.pair(&home.net_status().serving.unwrap().pairing).unwrap();
    let channel = crate::bind::RtcChannel { node: vec![3; 32], browser: vec![4; 32], ufrag: format!("h1c{}abcdefgh", "B".repeat(43)) };
    home.net_connect("p", Role::Client);
    home.set_peer_meta("p", PeerMeta { transport: "webrtc".into(), incoming: true, rtc: Some(channel.clone()), ..Default::default() });
    assert!(home.take_outbox().is_empty(), "nothing before the device's credential");
    phone.net_connect("h", Role::Home);
    phone.set_peer_meta("h", PeerMeta { transport: "webrtc".into(), rtc: Some(channel), ..Default::default() });
    pump_named(&mut phone, "h", &mut home, "p");
    assert!(phone.peers["h"].bound);
    assert!(phone.net_status().home_node.is_some_and(|h| h.connected));
}

fn node_(platform: &str, t: &Arc<AtomicU64>) -> Node {
    node(platform, t, &Mem::default())
}
