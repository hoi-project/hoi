//! The directory grows with every node a device reaches (crate::directory): listings
//! are kept between connections and starts, passed from node to node so every node comes to
//! know what any of them carries, aged so a community nobody holds fades everywhere, and never
//! taken from this device's own devices to strangers. Invites made in a browser lead to the
//! public nodes that keep the community, and making one asks them to.
use super::*;
use hoi_sync::{CatalogEntry, Msg};

fn fp(c: char) -> String {
    c.to_string().repeat(43)
}

/// A public node at `ip`, as every `hoi-node` is by default.
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

/// `a` dials `b` over QUIC: each knows the other's authenticated endpoint and address
/// (`(endpoint, ip)`), and calls it by the name given.
fn quic(a: &mut Node, (a_ep, a_ip): (&str, &str), a_calls_b: &str, b: &mut Node, (b_ep, b_ip): (&str, &str), b_calls_a: &str) {
    a.net_connect(a_calls_b, Role::Peer);
    a.set_peer_meta(a_calls_b, PeerMeta { transport: "quic".into(), endpoint: Some(b_ep.into()), addr: Some(format!("{b_ip}:7843")), ..Default::default() });
    b.net_connect(b_calls_a, Role::Peer);
    b.set_peer_meta(
        b_calls_a,
        PeerMeta { transport: "quic".into(), incoming: true, endpoint: Some(a_ep.into()), addr: Some(format!("{a_ip}:7843")), ..Default::default() },
    );
}

/// A browser dials `node`'s WebRTC route: the node knows it as `node_calls`, it knows the node as `calls`.
fn browser_dials(browser: &mut Node, calls: &str, node: &mut Node, node_calls: &str, route: &str) {
    node.net_connect(node_calls, Role::Peer);
    node.set_peer_meta(node_calls, PeerMeta { transport: "webrtc".into(), incoming: true, addr: Some("198.51.100.9:50000".into()), ..Default::default() });
    browser.net_connect(calls, Role::Peer);
    browser.set_peer_meta(calls, PeerMeta { transport: "webrtc".into(), dialed: vec![route.into()], ..Default::default() });
}

fn rtc(ip: &str, cert: char) -> String {
    format!("rtc:{}@{ip}:7844", fp(cert))
}

/// The next gossip round: the clock moves past it and the host's sweep runs.
fn next_round(t: &Arc<AtomicU64>, nodes: &mut [&mut Node]) {
    t.fetch_add(crate::directory::GOSSIP_EVERY + 1, Ordering::SeqCst);
    for n in nodes {
        n.sweep();
    }
}

fn ids(n: &Node) -> Vec<String> {
    n.discover_communities().into_iter().map(|c| c.id).collect()
}

/// A public node that holds a fresh community a browser founded and handed it.
fn carrier_with(t: &Arc<AtomicU64>, name: &str, ip: &str, cert: char) -> (Node, String, CommunityView) {
    let (mut carrier, ep) = public(t, &Mem::default(), ip, cert);
    let mut founder = node("web", t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let c = founder.create_community(draft(name)).unwrap();
    post(&mut founder, &c.id, "First post");
    browser_dials(&mut founder, "n", &mut carrier, "f", &rtc(ip, cert));
    pump_named(&mut founder, "n", &mut carrier, "f");
    assert!(carrier.communities().iter().any(|x| x.id == c.id), "the founder's browser had it carried");
    carrier.net_disconnect("f");
    (carrier, ep, c)
}

#[test]
fn a_browser_keeps_what_a_node_listed_and_joins_from_it_later() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut carrier, _, garden) = carrier_with(&t, "garden", "203.0.113.1", 'a');
    let memory = Mem::default();
    let mut web = node("web", &t, &memory);
    web.create_identity("Lena").unwrap();
    browser_dials(&mut web, "a", &mut carrier, "w", &rtc("203.0.113.1", 'a'));
    pump_named(&mut web, "a", &mut carrier, "w");
    let found = web.discover_communities();
    assert_eq!(found.len(), 1);
    assert!(found[0].live && !found[0].joined && found[0].seen > 0);
    assert!(hoi_types::Invite::parse(&found[0].invite).unwrap().genesis.is_some(), "a link copied from here works elsewhere too");

    // The link closes, the tab closes: what the node listed is still there, not live.
    web.net_disconnect("a");
    carrier.net_disconnect("w");
    let kept = web.discover_communities();
    assert_eq!(kept.iter().map(|c| (c.id.as_str(), c.live)).collect::<Vec<_>>(), vec![(garden.id.as_str(), false)]);
    web.commit().unwrap();
    drop(web);
    let mut web = node("web", &t, &memory);
    let again = web.discover_communities();
    assert_eq!(again.len(), 1, "kept between starts");
    assert_eq!(web.net_status().directory.listings, 1);

    // An invite naming only the key is completed from the directory.
    let bare = hoi_types::Invite { community: Id::from_hex(&garden.id).unwrap(), genesis: None, ..Default::default() };
    assert!(web.preview_invite(&bare.to_uri()).unwrap().verified);
    // Joined from the remembered listing, it syncs once the carrier is reached again.
    web.join(&again[0].invite).unwrap();
    assert!(web.preview_invite(&again[0].invite).unwrap().verified);
    let invite = hoi_types::Invite::parse(&again[0].invite).unwrap();
    assert!(invite.peers.contains(&rtc("203.0.113.1", 'a')), "the carrier's route came with the listing: {:?}", invite.peers);
    browser_dials(&mut web, "a", &mut carrier, "w", &rtc("203.0.113.1", 'a'));
    pump_named(&mut web, "a", &mut carrier, "w");
    assert_eq!(web.feed(Some(&garden.id), "fresh", None).unwrap().len(), 1, "the first post arrived");
}

#[test]
fn nodes_pass_listings_on_so_a_browser_finds_what_a_node_two_hops_away_carries() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut far, far_ep, garden) = carrier_with(&t, "garden", "203.0.113.1", 'a');
    let (mut mid, mid_ep) = public(&t, &Mem::default(), "203.0.113.2", 'b');
    let (mut near, near_ep) = public(&t, &Mem::default(), "203.0.113.3", 'c');
    // mid relays and passes listings on but carries nothing, so nobody offers it garden: what
    // reaches near is the listing alone.
    mid.set_carry_policy(crate::CarryPolicy::Off, 64);
    // mid reaches far; near reaches mid. near never meets far.
    quic(&mut mid, (&mid_ep, "203.0.113.2"), "far", &mut far, (&far_ep, "203.0.113.1"), "mid");
    pump_named(&mut mid, "far", &mut far, "mid");
    assert!(ids(&mid).contains(&garden.id), "mid holds far's listing as far's own");
    quic(&mut near, (&near_ep, "203.0.113.3"), "mid", &mut mid, (&mid_ep, "203.0.113.2"), "near");
    pump_named(&mut near, "mid", &mut mid, "near");
    assert!(ids(&near).contains(&garden.id), "mid relays what far listed in its first catalog to near");
    let listing = near.directory.listings.get(&Id::from_hex(&garden.id).unwrap()).unwrap();
    assert!(listing.p.iter().any(|r| r.contains("203.0.113.1")), "with far's routes, not mid's: {:?}", listing.p);
    assert!(mid.carried_on_request().is_empty() && near.carried_on_request().is_empty(), "hearing of a community carries nothing");

    // A browser that only ever reaches `near` finds garden and joins it through far's route.
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Kim").unwrap();
    browser_dials(&mut web, "near", &mut near, "w", &rtc("203.0.113.3", 'c'));
    pump_named(&mut web, "near", &mut near, "w");
    let found = web.discover_communities();
    let g = found.iter().find(|c| c.id == garden.id).expect("two hops away and still found");
    assert!(!g.live, "near lists it as heard of, not as held");
    web.join(&g.invite).unwrap();
    assert!(web.rtc_candidates().contains(&rtc("203.0.113.1", 'a')), "the join leads to far: {:?}", web.rtc_candidates());
}

#[test]
fn listings_come_in_windows_each_round_and_a_new_one_goes_out_first() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let (mut b, b_ep) = public(&t, &Mem::default(), "203.0.113.2", 'b');
    // `a` heard of more open communities than one catalog holds, from a node that dialed it.
    let mut maker = node("web", &t, &Mem::default());
    maker.create_identity("Robin").unwrap();
    let mut made = Vec::new();
    for i in 0..70 {
        let c = maker.create_community(draft(&format!("c{i}"))).unwrap();
        made.push(Id::from_hex(&c.id).unwrap());
    }
    let entries: Vec<CatalogEntry> = made
        .iter()
        .map(|c| CatalogEntry {
            genesis: maker.store.genesis_of(c).unwrap().wire.clone(),
            peers: vec![],
            relays: vec![],
            warmth: None,
            relayed: true,
            age: Some(60),
        })
        .collect();
    quic(&mut a, (&a_ep, "203.0.113.1"), "src", &mut b, (&b_ep, "203.0.113.2"), "x");
    pump_named(&mut a, "src", &mut b, "x");
    for chunk in entries.chunks(40) {
        assert!(a.net_receive("src", &Msg::Catalog { entries: chunk.to_vec() }.encode()));
    }
    assert_eq!(a.directory.listings.len(), 70);
    // b is a fresh peer of a: its first window is the newest listings, the next round the rest.
    let (mut c, c_ep) = public(&t, &Mem::default(), "203.0.113.3", 'c');
    quic(&mut c, (&c_ep, "203.0.113.3"), "a", &mut a, (&a_ep, "203.0.113.1"), "c");
    pump_named(&mut c, "a", &mut a, "c");
    let first = c.directory.listings.len();
    assert_eq!(first, hoi_sync::MAX_CATALOG_ENTRIES, "one full window");
    for _ in 0..3 {
        next_round(&t, &mut [&mut a, &mut c]);
        pump_named(&mut c, "a", &mut a, "c");
    }
    assert_eq!(c.directory.listings.len(), 70, "the rotation reaches every listing");
}

#[test]
fn a_listing_nobody_holds_fades_everywhere_instead_of_circling() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let (mut b, b_ep) = public(&t, &Mem::default(), "203.0.113.2", 'b');
    let mut maker = node("web", &t, &Mem::default());
    maker.create_identity("Robin").unwrap();
    let old = maker.create_community(draft("old")).unwrap();
    let fresh = maker.create_community(draft("fresh")).unwrap();
    let ancient = maker.create_community(draft("ancient")).unwrap();
    let entry = |id: &str, age: u64| CatalogEntry {
        genesis: maker.store.genesis_of(&Id::from_hex(id).unwrap()).unwrap().wire.clone(),
        peers: vec![],
        relays: vec![],
        warmth: None,
        relayed: true,
        age: Some(age),
    };
    quic(&mut a, (&a_ep, "203.0.113.1"), "b", &mut b, (&b_ep, "203.0.113.2"), "a");
    pump_named(&mut a, "b", &mut b, "a");
    // A relayed listing told as eight days old is kept but not passed on; one past three weeks is not even kept.
    let catalog = Msg::Catalog { entries: vec![entry(&old.id, 8 * 86_400), entry(&fresh.id, 60), entry(&ancient.id, 30 * 86_400)] };
    let mut src = node("node", &t, &Mem::default());
    src.net_connect("a", Role::Peer);
    a.net_connect("s", Role::Peer);
    pump_named(&mut src, "a", &mut a, "s");
    assert!(a.net_receive("s", &catalog.encode()));
    assert!(ids(&a).contains(&old.id) && ids(&a).contains(&fresh.id) && !ids(&a).contains(&ancient.id));
    next_round(&t, &mut [&mut a, &mut b]);
    pump_named(&mut a, "b", &mut b, "a");
    assert!(ids(&b).contains(&fresh.id) && !ids(&b).contains(&old.id), "too old to pass on");
    // Passing on never makes a listing younger: b tells a its age, and a's stays as old as it was.
    let held = a.directory.listings[&Id::from_hex(&old.id).unwrap()].held;
    next_round(&t, &mut [&mut a, &mut b]);
    pump_named(&mut a, "b", &mut b, "a");
    assert_eq!(a.directory.listings[&Id::from_hex(&old.id).unwrap()].held, held);
    // Three weeks after anyone held it, it's gone.
    t.fetch_add(21 * 86_400, Ordering::SeqCst);
    a.sweep();
    assert!(!ids(&a).contains(&old.id));
}

#[test]
fn what_my_own_home_node_lists_never_goes_to_strangers() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    home.create_identity("Robin").unwrap();
    let mine = home.create_community(draft("family")).unwrap();
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Robin's browser").unwrap();
    pair_now(&mut home, &mut web, "home", "web");
    pump_named(&mut web, "home", &mut home, "web");
    assert!(ids(&web).contains(&mine.id), "my own home node tells me what it holds");
    // Lena's carrier, reached by the same browser: it hears what public nodes listed, not this.
    let (mut carrier, carrier_ep, garden) = carrier_with(&t, "garden", "203.0.113.1", 'a');
    let (mut other, other_ep) = public(&t, &Mem::default(), "203.0.113.2", 'b');
    quic(&mut other, (&other_ep, "203.0.113.2"), "c", &mut carrier, (&carrier_ep, "203.0.113.1"), "o");
    pump_named(&mut other, "c", &mut carrier, "o");
    browser_dials(&mut web, "c", &mut carrier, "w", &rtc("203.0.113.1", 'a'));
    pump_named(&mut web, "c", &mut carrier, "w");
    assert!(ids(&web).contains(&garden.id));
    // The browser passes what it heard from public nodes on to a public node it dials...
    browser_dials(&mut web, "o", &mut other, "w", &rtc("203.0.113.2", 'b'));
    pump_named(&mut web, "o", &mut other, "w");
    let mine_id = Id::from_hex(&mine.id).unwrap();
    assert!(!other.directory.listings.contains_key(&mine_id), "...never what only its own home node listed");
    assert!(!carrier.directory.listings.contains_key(&mine_id));
}

#[test]
fn an_invite_made_in_a_browser_leads_to_the_public_node_that_keeps_it() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut carrier, _) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let route = rtc("203.0.113.1", 'a');
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Robin").unwrap();
    web.open_node_link(&hoi_types::NodeLink { rtc: vec![route.clone()], ..Default::default() }.to_web("https://hoi.sh/app/")).unwrap();
    browser_dials(&mut web, "a", &mut carrier, "w", &route);
    pump_named(&mut web, "a", &mut carrier, "w");
    // Made a minute ago, nothing posted: the carrier has given nothing back yet.
    let c = web.create_community(draft("garden")).unwrap();
    web.announce();
    pump_named(&mut web, "a", &mut carrier, "w");
    assert!(carrier.communities().iter().any(|x| x.id == c.id), "the founder's browser had it carried");
    let invite = web.invite(&c.id).unwrap();
    let parsed = hoi_types::Invite::parse(&invite.uri).unwrap();
    assert!(parsed.peers.contains(&route), "the invite names the node that keeps it: {:?}", parsed.peers);
    assert!(invite.outside, "someone on another network can use it");

    // Hours later, with the founder's tab long closed, a newcomer still finds it there.
    web.net_disconnect("a");
    carrier.net_disconnect("w");
    t.fetch_add(3 * 3600, Ordering::SeqCst);
    carrier.sweep();
    assert!(carrier.communities().iter().any(|x| x.id == c.id), "a community that is all definition so far is not let go while its founder listed it");
    let mut friend = node("web", &t, &Mem::default());
    friend.create_identity("Lena").unwrap();
    friend.join(&invite.uri).unwrap();
    browser_dials(&mut friend, "a", &mut carrier, "f", &route);
    pump_named(&mut friend, "a", &mut carrier, "f");
    assert_eq!(friend.warmth().communities.iter().find(|w| w.community == c.id).map(|w| w.live), Some(true), "the carrier lists it to the newcomer");
    assert!(friend.discover_communities().iter().any(|x| x.id == c.id && x.joined));
}

#[test]
fn inviting_to_someone_elses_community_asks_the_nodes_in_reach_to_keep_it() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let c = founder.create_community(draft("garden")).unwrap();
    let mut member = node("web", &t, &Mem::default());
    member.create_identity("Lena").unwrap();
    member.join(&founder.invite(&c.id).unwrap().uri).unwrap();
    let (mut carrier, _) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let route = rtc("203.0.113.1", 'a');
    browser_dials(&mut member, "a", &mut carrier, "m", &route);
    pump_named(&mut member, "a", &mut carrier, "m");
    assert!(carrier.communities().is_empty(), "a member's communities wait for the member to ask");
    // Making an invite is asking: the dispatcher does both, as the app calls it.
    let invite: crate::dto::Invite = serde_json::from_value(member.call("invite", serde_json::json!([c.id])).unwrap()).unwrap();
    assert!(hoi_types::Invite::parse(&invite.uri).unwrap().peers.contains(&route), "the asked node is in the invite already");
    pump_named(&mut member, "a", &mut carrier, "m");
    assert!(carrier.communities().iter().any(|x| x.id == c.id), "and it took the community");

    // An invite-only community is never offered to a public node.
    let mut closed = draft("closed");
    closed.limits.as_mut().unwrap().admission = Some("invite".into());
    let closed = member.create_community(closed).unwrap();
    member.call("invite", serde_json::json!([closed.id])).unwrap();
    pump_named(&mut member, "a", &mut carrier, "m");
    assert!(!carrier.communities().iter().any(|x| x.id == closed.id));
}

#[test]
fn an_invite_to_an_open_community_also_names_public_nodes_this_device_reached() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, _) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let route = rtc("203.0.113.1", 'a');
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Robin").unwrap();
    browser_dials(&mut web, "a", &mut a, "w", &route);
    pump_named(&mut web, "a", &mut a, "w");
    web.net_disconnect("a");
    // A community made later, while no node is in reach: its invite still leads into the network.
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Lena").unwrap();
    let c = founder.create_community(draft("garden")).unwrap();
    web.join(&founder.invite(&c.id).unwrap().uri).unwrap();
    let peers = hoi_types::Invite::parse(&web.invite(&c.id).unwrap().uri).unwrap().peers;
    assert!(peers.contains(&route), "{peers:?}");
    let mut closed = draft("closed");
    closed.limits.as_mut().unwrap().admission = Some("invite".into());
    let closed = web.create_community(closed).unwrap();
    assert!(!hoi_types::Invite::parse(&web.invite(&closed.id).unwrap().uri).unwrap().peers.contains(&route), "an invite-only community keeps to its members");
}

#[test]
fn the_network_map_says_what_each_link_is_and_which_communities_it_carries_from_here() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut carrier, _, garden) = carrier_with(&t, "garden", "203.0.113.1", 'a');
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Lena").unwrap();
    browser_dials(&mut web, "a", &mut carrier, "w", &rtc("203.0.113.1", 'a'));
    pump_named(&mut web, "a", &mut carrier, "w");
    web.join(&web.discover_communities()[0].invite.clone()).unwrap();
    let private = web.create_private_community(draft("family")).ok();
    pump_named(&mut web, "a", &mut carrier, "w");
    let link = web.net_status().connections.into_iter().find(|c| c.peer == "a").unwrap();
    assert_eq!((link.kind.as_str(), link.keeps, link.incoming), ("public", true, false));
    assert_eq!(link.communities, vec![garden.id.clone()], "a post in garden goes to this node from here");
    if let Some(p) = private {
        assert!(!link.communities.contains(&p.id), "never a private community");
    }
    // The node sees a browser that dialed in, whose copy closes with it.
    let back = carrier.net_status().connections.into_iter().find(|c| c.peer == "w").unwrap();
    assert_eq!((back.kind.as_str(), back.keeps, back.incoming), ("browser", false, true));
}

#[test]
fn browsers_get_a_new_window_every_half_hour_nodes_every_five_minutes() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a');
    let mut maker = node("web", &t, &Mem::default());
    maker.create_identity("Robin").unwrap();
    let entries: Vec<CatalogEntry> = (0..80)
        .map(|i| {
            let c = Id::from_hex(&maker.create_community(draft(&format!("c{i}"))).unwrap().id).unwrap();
            CatalogEntry {
                genesis: maker.store.genesis_of(&c).unwrap().wire.clone(),
                peers: vec![],
                relays: vec![],
                warmth: None,
                relayed: true,
                age: Some(7200),
            }
        })
        .collect();
    let (mut src, src_ep) = public(&t, &Mem::default(), "203.0.113.9", 's');
    quic(&mut a, (&a_ep, "203.0.113.1"), "src", &mut src, (&src_ep, "203.0.113.9"), "a");
    pump_named(&mut a, "src", &mut src, "a");
    for chunk in entries.chunks(40) {
        assert!(a.net_receive("src", &Msg::Catalog { entries: chunk.to_vec() }.encode()));
    }
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("Kim").unwrap();
    browser_dials(&mut web, "a", &mut a, "w", &rtc("203.0.113.1", 'a'));
    pump_named(&mut web, "a", &mut a, "w");
    let first = web.directory.listings.len();
    assert_eq!(first, hoi_sync::MAX_CATALOG_ENTRIES, "a browser gets a full window with its hello");
    next_round(&t, &mut [&mut a]);
    pump_named(&mut web, "a", &mut a, "w");
    assert_eq!(web.directory.listings.len(), first, "five minutes later, the same window");
    for _ in 0..6 {
        next_round(&t, &mut [&mut a]);
    }
    pump_named(&mut web, "a", &mut a, "w");
    assert_eq!(web.directory.listings.len(), 80, "half an hour later, the next one");
}
