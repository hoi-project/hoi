//! Direct links between browsers (M5): a node passes an offer and an answer between two
//! browsers that share a community, the lower handle offers, and records then flow between the
//! browsers directly.
use super::*;
use crate::signal::DirectEvent;

fn browser_link(n: &mut Node, to: &str) {
    n.net_connect(to, Role::Peer);
    n.set_peer_meta(
        to,
        PeerMeta { transport: "webrtc".into(), dialed: vec!["rtc:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA@203.0.113.9:7844".into()], ..Default::default() },
    );
}

fn node_side(hub: &mut Node, from: &str) {
    hub.net_connect(from, Role::Peer);
    hub.set_peer_meta(from, PeerMeta { transport: "webrtc".into(), incoming: true, addr: Some("198.51.100.4:50000".into()), ..Default::default() });
}

/// Delivers everything queued among a hub and two browsers until nothing is left; returns what
/// the browsers' hosts were asked to do.
fn pump3(hub: &mut Node, a: &mut Node, b: &mut Node) -> (Vec<DirectEvent>, Vec<DirectEvent>) {
    let (mut ea, mut eb) = (Vec::new(), Vec::new());
    for _ in 0..60 {
        let from_hub = hub.take_outbox();
        let from_a = a.take_outbox();
        let from_b = b.take_outbox();
        ea.extend(a.take_direct());
        eb.extend(b.take_direct());
        if from_hub.is_empty() && from_a.is_empty() && from_b.is_empty() {
            break;
        }
        for (to, m) in from_hub {
            match to.as_str() {
                "a" => assert!(a.net_receive("hub", &m)),
                "b" => assert!(b.net_receive("hub", &m)),
                _ => {}
            }
        }
        for (to, m) in from_a {
            if to == "hub" {
                assert!(hub.net_receive("a", &m));
            }
        }
        for (to, m) in from_b {
            if to == "hub" {
                assert!(hub.net_receive("b", &m));
            }
        }
    }
    (ea, eb)
}

#[test]
fn two_browsers_link_directly_through_a_nodes_signals() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut hub = node("node", &t, &Mem::default());
    let mut a = node("web", &t, &Mem::default());
    let mut b = node("web", &t, &Mem::default());
    hub.create_identity("the pi").unwrap();
    a.create_identity("mira").unwrap();
    b.create_identity("lena").unwrap();
    let c = a.create_community(draft("privacy")).unwrap();
    let invite = a.invite(&c.id).unwrap().web;
    hub.join(&invite).unwrap();
    b.join(&invite).unwrap();
    hub.set_settings(&SettingsPatch { relay: Some(true), ..Default::default() }).unwrap();
    hub.set_signal_relay(true);
    for n in [&mut a, &mut b] {
        n.set_settings(&SettingsPatch { direct_links: Some(true), ..Default::default() }).unwrap();
    }
    browser_link(&mut a, "hub");
    node_side(&mut hub, "a");
    browser_link(&mut b, "hub");
    node_side(&mut hub, "b");
    let (ea, eb) = pump3(&mut hub, &mut a, &mut b);
    assert!(a.peers["hub"].features.contains(hoi_sync::FEATURE_SIGNAL));
    assert!(hub.peers["a"].features.contains(hoi_sync::FEATURE_DIRECT));
    // exactly one side offers: the lower handle
    let offers: Vec<(&str, &DirectEvent)> =
        ea.iter().map(|e| ("a", e)).chain(eb.iter().map(|e| ("b", e))).filter(|(_, e)| matches!(e, DirectEvent::Offer { .. })).collect();
    assert_eq!(offers.len(), 1, "{ea:?} {eb:?}");
    let (offerer, DirectEvent::Offer { via, to, stun }) = offers[0] else { unreachable!() };
    assert_eq!((via.as_str(), stun.as_slice()), ("hub", ["stun:203.0.113.9:7844".to_string()].as_slice()));
    let (off, ans) = if offerer == "a" { (&mut a, &mut b) } else { (&mut b, &mut a) };
    off.signal("hub", to, b"{\"type\":\"offer\"}").unwrap();
    let (ea, eb) = if offerer == "a" {
        pump3(&mut hub, off, ans)
    } else {
        let (x, y) = pump3(&mut hub, ans, off);
        (y, x)
    };
    // the answering side hears the offer, from the offerer's handle
    let Some(DirectEvent::Signal { via, from, data, .. }) = eb.first() else { panic!("{eb:?}") };
    assert_eq!((via.as_str(), data.as_slice()), ("hub", b"{\"type\":\"offer\"}".as_slice()));
    assert!(ea.is_empty());
    let back = from.clone();
    ans.signal("hub", &back, b"{\"type\":\"answer\"}").unwrap();
    let (ea, _) = if offerer == "a" {
        pump3(&mut hub, off, ans)
    } else {
        let (x, y) = pump3(&mut hub, ans, off);
        (y, x)
    };
    let Some(DirectEvent::Signal { from: answered_by, data, .. }) = ea.first() else { panic!("{ea:?}") };
    assert_eq!((answered_by.as_str(), data.as_slice()), (to.as_str(), b"{\"type\":\"answer\"}".as_slice()));

    // the channel opened: the two browsers sync directly, as peers
    let to_bytes = |h: &str| <[u8; 16]>::try_from(hoi_types::hex_decode(h).unwrap()).unwrap();
    off.net_connect("direct-1", Role::Peer);
    off.set_peer_meta("direct-1", PeerMeta { transport: "webrtc".into(), direct: Some(("hub".into(), to_bytes(to))), ..Default::default() });
    ans.net_connect("direct-1", Role::Peer);
    ans.set_peer_meta("direct-1", PeerMeta { transport: "webrtc".into(), incoming: true, direct: Some(("hub".into(), to_bytes(&back))), ..Default::default() });
    let p = post(off, &c.id, "straight across");
    pump_named(off, "direct-1", ans, "direct-1");
    assert!(ans.post(&p.id).is_ok(), "the post came over the direct link");
    // no second offer while the link is open
    off.sweep();
    assert!(off.take_direct().is_empty());
}

#[test]
fn signals_pass_only_between_browsers_that_share_a_community_and_asked_for_it() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut hub = node("node", &t, &Mem::default());
    let mut a = node("web", &t, &Mem::default());
    let mut b = node("web", &t, &Mem::default());
    hub.create_identity("the pi").unwrap();
    a.create_identity("mira").unwrap();
    b.create_identity("lena").unwrap();
    let c = a.create_community(draft("privacy")).unwrap();
    let other = b.create_community(draft("elsewhere")).unwrap();
    hub.join(&a.invite(&c.id).unwrap().web).unwrap();
    hub.join(&b.invite(&other.id).unwrap().web).unwrap();
    hub.set_settings(&SettingsPatch { relay: Some(true), ..Default::default() }).unwrap();
    hub.set_signal_relay(true);
    a.set_settings(&SettingsPatch { direct_links: Some(true), ..Default::default() }).unwrap();
    b.set_settings(&SettingsPatch { direct_links: Some(true), ..Default::default() }).unwrap();
    browser_link(&mut a, "hub");
    node_side(&mut hub, "a");
    browser_link(&mut b, "hub");
    node_side(&mut hub, "b");
    let (ea, eb) = pump3(&mut hub, &mut a, &mut b);
    assert!(ea.is_empty() && eb.is_empty(), "nothing in common, nothing offered");
    // a guessed handle goes nowhere
    let b_handle = hub.signals.handles["b"];
    a.signal("hub", &hoi_types::hex_encode(&b_handle), b"{}").unwrap();
    let (_, eb) = pump3(&mut hub, &mut a, &mut b);
    assert!(eb.is_empty(), "not passed between browsers with nothing in common");
    // a browser with direct links off says nothing, and hears nothing
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("kim").unwrap();
    assert!(!web.takes_direct());
    // a desktop never takes them
    let mut desk = node("desktop", &t, &Mem::default());
    desk.set_settings(&SettingsPatch { direct_links: Some(true), ..Default::default() }).unwrap();
    assert!(!desk.state.settings.direct_links);
}

#[test]
fn two_links_to_the_same_browser_become_one() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("web", &t, &Mem::default());
    a.create_identity("mira").unwrap();
    a.set_settings(&SettingsPatch { direct_links: Some(true), ..Default::default() }).unwrap();
    // the same browser, reached by two handles at two nodes: its key is in both offers
    for hub in ["hub1", "hub2"] {
        a.net_connect(hub, Role::Peer);
        a.peers.get_mut(hub).unwrap().features.insert(hoi_sync::FEATURE_SIGNAL.into());
    }
    let kim = [9u8; 16];
    let offer = |n: &[u8]| [kim.as_slice(), n].concat();
    a.on_signal_msg("hub1", hoi_sync::Msg::Sig { peer: [1; 16], data: offer(b"{\"type\":\"offer\"}") });
    assert_eq!(a.take_direct().len(), 1, "the first offer is answered");
    a.on_signal_msg("hub2", hoi_sync::Msg::Sig { peer: [2; 16], data: offer(b"{\"type\":\"offer\"}") });
    assert!(a.take_direct().is_empty(), "a second one from the same browser isn't");
    // an offer of its own, back from a node, is never taken
    let me = a.signals.me.unwrap();
    a.on_signal_msg("hub2", hoi_sync::Msg::Sig { peer: [3; 16], data: [me.as_slice(), b"{}"].concat() });
    assert!(a.take_direct().is_empty());
    // if two open anyway (both offered at once), the same one closes on both sides
    a.signals.keys.insert(("hub2".into(), [2; 16]), kim);
    for (peer, via, h) in [("direct-1", "hub1", [1u8; 16]), ("direct-2", "hub2", [2u8; 16])] {
        a.net_connect(peer, Role::Peer);
        a.set_peer_meta(peer, PeerMeta { transport: "webrtc".into(), incoming: true, direct: Some((via.into(), h)), ..Default::default() });
    }
    let closed: Vec<DirectEvent> = a.take_direct();
    assert_eq!(closed.len(), 1, "{closed:?}");
    assert!(matches!(&closed[0], DirectEvent::Close { .. }));
}
