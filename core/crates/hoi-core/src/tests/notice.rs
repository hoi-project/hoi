//! Informed before connected (crate::notice): a public node tells strangers who runs it
//! in its first hello and, signed, in its link; a person's device says nothing of itself until
//! the notice was read and accepted, and dials a node it declined no more.
use super::*;
use hoi_sync::Msg;
use hoi_types::NodeNotice;

pub(super) fn fp(c: char) -> String {
    c.to_string().repeat(43)
}

pub(super) fn notice(operator: &str) -> NodeNotice {
    NodeNotice {
        operator: operator.into(),
        contact: "lena@example.org".into(),
        country: "XX".into(),
        updated: "2026-11-03".into(),
        ..NodeNotice::template("sleepy-walrus")
    }
}

/// A native public node at `ip`, with an operator notice when `operator` is given.
pub(super) fn public(t: &Arc<AtomicU64>, memory: &Mem, ip: &str, cert: char, operator: Option<&str>) -> (Node, String) {
    let mut n = node("node", t, memory);
    n.set_node_name(Some("sleepy-walrus".into()));
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    n.set_carry_policy(crate::CarryPolicy::Any, 64);
    let endpoint = Keypair::from_secret(&n.transport_secret().unwrap()).public().hex();
    n.set_transport_address(Some(endpoint.clone()), vec![format!("{endpoint}@{ip}:7843")], vec![]);
    n.set_rtc_routes(vec![format!("rtc:{}@{ip}:7844", fp(cert))]);
    if let Some(operator) = operator {
        n.set_notice(Some(notice(operator))).unwrap();
    }
    (n, endpoint)
}

/// A browser with the consent gate on, as hoi-wasm starts one.
pub(super) fn browser(t: &Arc<AtomicU64>, memory: &Mem) -> Node {
    let mut b = node("web", t, memory);
    b.set_consent_gate(true);
    b.create_identity("mira").unwrap();
    b
}

/// A browser dials `node`'s WebRTC route: the node knows it as "w", the browser knows the node as "a".
pub(super) fn dial(browser: &mut Node, node: &mut Node, route: &str) {
    node.net_connect("w", Role::Peer);
    node.set_peer_meta("w", PeerMeta { transport: "webrtc".into(), incoming: true, addr: Some("198.51.100.4:50000".into()), ..Default::default() });
    browser.net_connect("a", Role::Peer);
    browser.set_peer_meta("a", PeerMeta { transport: "webrtc".into(), dialed: vec![route.into()], ..Default::default() });
}

/// Pumps until nothing moves; false when the browser ended the link (then both sides forget it).
pub(super) fn pump_kept(browser: &mut Node, node: &mut Node) -> bool {
    for _ in 0..20 {
        let to_node: Vec<Vec<u8>> = browser.take_outbox().into_iter().filter(|(p, _)| p == "a").map(|(_, m)| m).collect();
        let to_browser: Vec<Vec<u8>> = node.take_outbox().into_iter().filter(|(p, _)| p == "w").map(|(_, m)| m).collect();
        if to_node.is_empty() && to_browser.is_empty() {
            break;
        }
        for m in to_browser {
            if !browser.net_receive("a", &m) {
                browser.net_disconnect("a");
                node.net_disconnect("w");
                return false;
            }
        }
        for m in to_node {
            assert!(node.net_receive("w", &m), "the node keeps what the browser sends");
        }
    }
    browser.commit().unwrap();
    node.commit().unwrap();
    true
}

pub(super) fn hellos_from(n: &mut Node, to: &str) -> Vec<Msg> {
    n.take_outbox().into_iter().filter(|(p, _)| p == to).filter_map(|(_, m)| Msg::decode(&m).ok()).filter(|m| matches!(m, Msg::Hello { .. })).collect()
}

#[test]
fn a_public_node_says_who_runs_it_first_and_a_browser_says_nothing_until_the_person_accepted() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a', Some("Lena"));
    let route = format!("rtc:{}@203.0.113.1:7844", fp('a'));
    let mut b = browser(&t, &Mem::default());
    dial(&mut b, &mut a, &route);
    // The node's hello carries its notice; the browser queued nothing on connect.
    let hellos = hellos_from(&mut a, "w");
    let Some(Msg::Hello { notice_hash: Some(h), notice: Some(bytes), .. }) = hellos.first() else { panic!("a hello with a notice: {hellos:?}") };
    assert_eq!(hoi_types::notice::hash_of(bytes), *h);
    assert!(b.take_outbox().is_empty(), "a gated browser waits for the node's hello");
    assert!(!b.net_receive("a", &hellos[0].encode()), "the link ends until the person has read the notice");
    assert!(b.consent_closed("a"), "the host is told this was no refusal");
    assert!(matches!(b.take_events().last(), Some(Event::NodeNotice { node }) if *node == a_ep));
    b.commit().unwrap();
    let pending = b.net_status().pending_notices;
    assert_eq!(pending.len(), 1);
    assert_eq!((pending[0].node.as_str(), pending[0].state.as_str(), pending[0].operator.as_str()), (a_ep.as_str(), "pending", "Lena"));
    assert!(pending[0].sections.iter().any(|s| s.title == "Operator" && s.text.contains("operated by Lena.")));
    assert!(b.take_outbox().is_empty(), "still nothing of its own went out");
    // While the question is open, the node is not dialed on the device's own initiative.
    b.net_disconnect("a");
    a.net_disconnect("w");
    b.open_node_link(&hoi_types::NodeLink { rtc: vec![route.clone()], ..Default::default() }.to_uri()).unwrap();
    assert!(b.discovery_candidates().is_empty(), "a pending notice keeps its node out of the dial list");
    // The person declines: still nothing; a hello from that node is answered with silence.
    b.decline_notice(&a_ep).unwrap();
    assert_eq!(b.node_notices()[0].state, "declined");
    dial(&mut b, &mut a, &route);
    assert!(!pump_kept(&mut b, &mut a), "a declined node is not talked to");
    assert!(b.discovery_candidates().is_empty());
    // The person accepts after all (the hash names the words accepted): the node is dialed and heard.
    let hash = b.node_notices()[0].hash.clone();
    assert!(b.accept_notice(&a_ep, "0000").is_err(), "a different hash is another notice");
    let accepted = b.accept_notice(&a_ep, &hash).unwrap();
    assert_eq!(accepted.state, "accepted");
    assert_eq!(b.discovery_candidates(), vec![route.clone()], "dialed again at the next chance");
    dial(&mut b, &mut a, &route);
    assert!(pump_kept(&mut b, &mut a));
    assert!(b.peers["a"].said_hello && a.peers["w"].said_hello, "both sides said hello, the browser second");
    assert!(b.net_status().pending_notices.is_empty());
    assert_eq!(b.net_status().directory.connected, 1);
    // The saved state carries the decision, and a store that never met a notice saves no field.
    let saved = serde_json::to_string(&b.state).unwrap();
    assert!(saved.contains("\"notices\"") && saved.contains("\"accepted\""));
    assert!(!serde_json::to_string(&a.state).unwrap().contains("\"notices\""));
    let back: crate::state::LocalState = serde_json::from_str(&saved).unwrap();
    assert_eq!(back.notices.values().next().unwrap().notice, a.operator_notice.as_ref().unwrap().bytes);
}

#[test]
fn a_node_without_a_notice_and_a_headless_node_behave_as_before() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, _) = public(&t, &Mem::default(), "203.0.113.1", 'a', None);
    let route = format!("rtc:{}@203.0.113.1:7844", fp('a'));
    let mut b = browser(&t, &Mem::default());
    dial(&mut b, &mut a, &route);
    assert!(b.take_outbox().is_empty(), "the gate waits for the node's hello");
    assert!(pump_kept(&mut b, &mut a));
    assert!(b.peers["a"].said_hello && a.peers["w"].said_hello, "no notice: our hello follows theirs at once");
    assert!(b.net_status().pending_notices.is_empty());
    // A headless node (no gate) dialing another queues its hello on connect, as ever.
    let (mut c, _) = public(&t, &Mem::default(), "203.0.113.3", 'c', Some("Ola"));
    c.net_connect("d", Role::Peer);
    assert_eq!(hellos_from(&mut c, "d").len(), 1);
    // An incoming stranger on a gated desktop hears a hello the moment the host says it dialed in.
    let mut d = node("desktop", &t, &Mem::default());
    d.set_consent_gate(true);
    d.net_connect("x", Role::Peer);
    assert!(d.take_outbox().is_empty());
    d.set_peer_meta("x", PeerMeta { transport: "quic".into(), incoming: true, ..Default::default() });
    assert_eq!(hellos_from(&mut d, "x").len(), 1);
}

#[test]
fn a_node_link_carries_the_notice_signed_and_is_opened_only_once_it_was_accepted() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a', Some("Lena"));
    let link = a.net_status().node_link.unwrap();
    let parsed = hoi_types::NodeLink::parse(&link).unwrap();
    assert!(parsed.notice.is_some() && parsed.signature.is_some(), "{link}");
    assert!(!link.contains("203.0.113.1") && !link.contains("Lena"), "the link reads as a code: {link}");
    let mut b = browser(&t, &Mem::default());
    let preview = b.preview_node_link(&link).unwrap();
    assert!(preview.verified && preview.problem.is_none(), "{preview:?}");
    assert_eq!((preview.host.as_str(), preview.scope.as_str(), preview.endpoint.as_deref()), ("203.0.113.1:7844", "internet", Some(a_ep.as_str())));
    let shown = preview.notice.unwrap();
    assert_eq!((shown.state.as_str(), shown.operator.as_str()), ("pending", "Lena"));
    assert!(b.net_status().pending_notices.is_empty(), "a notice read from a link waits on the link's page, not in the app's dialog");
    assert!(b.open_node_link(&link).unwrap_err().message.contains("accept the operator's notice first"));
    b.accept_notice(&a_ep, &shown.hash).unwrap();
    b.open_node_link(&link).unwrap();
    assert_eq!(b.state.nodes.len(), 1);
    // Someone else's node cannot pass off the notice as its own: the signature names the node.
    let mut parsed = hoi_types::NodeLink::parse(&link).unwrap();
    let other = Keypair::generate().unwrap();
    parsed.peers = vec![format!("{}@203.0.113.9:7843", other.public().hex())];
    let forged = parsed.to_uri();
    let preview = b.preview_node_link(&forged).unwrap();
    assert!(!preview.verified && preview.problem.as_deref().unwrap().contains("not signed by the node it names"));
    assert!(b.open_node_link(&forged).is_err());
    // A link without a notice opens as before (the app shows its own warning for it).
    let plain = hoi_types::NodeLink { rtc: vec![format!("rtc:{}@203.0.113.2:7844", fp('p'))], ..Default::default() }.to_uri();
    let preview = b.preview_node_link(&plain).unwrap();
    assert!(preview.notice.is_none() && !preview.verified && preview.problem.is_none());
    b.open_node_link(&plain).unwrap();
    // Without the gate (a headless node given --node), a link's notice is nobody's question.
    let mut c = node("node", &t, &Mem::default());
    c.open_node_link(&link).unwrap();
    // A notice the node refuses to carry is refused with its problems; none means no link notice.
    let mut bad = notice("");
    bad.country = "xx".into();
    bad.updated = "yesterday".into();
    let e = a.set_notice(Some(bad)).unwrap_err();
    assert!(e.message.contains("country:") && e.message.contains("updated:") && !e.message.contains("operator:"), "{}", e.message);
    a.set_notice(None).unwrap();
    assert!(hoi_types::NodeLink::parse(&a.net_status().node_link.unwrap()).unwrap().notice.is_none());
}

#[test]
fn a_changed_notice_is_a_new_question_even_on_an_open_link() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a', Some("Lena"));
    let route = format!("rtc:{}@203.0.113.1:7844", fp('a'));
    let mut b = browser(&t, &Mem::default());
    let first = a.operator_notice_hash().unwrap();
    // Accept through the link first, so the connection goes through at once.
    b.preview_node_link(&a.net_status().node_link.unwrap()).unwrap();
    b.accept_notice(&a_ep, &first).unwrap();
    dial(&mut b, &mut a, &route);
    assert!(pump_kept(&mut b, &mut a));
    // The operator rewrites the notice: connected strangers hear it again, and the browser stops.
    let mut changed = notice("Lena");
    changed.ip_logging = true;
    changed.retention_days = 14;
    let second = a.set_notice(Some(changed)).unwrap().unwrap();
    assert_ne!(first, second);
    assert!(!pump_kept(&mut b, &mut a), "a new notice ends the link until it was read");
    let pending = b.net_status().pending_notices;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].hash, second);
    assert!(pending[0].sections.iter().any(|s| s.text.contains("logged for 14 days")));
    assert_eq!(b.node_notices().len(), 1, "one record per node");
    b.forget_notice(&a_ep).unwrap();
    assert!(b.node_notices().is_empty() && b.forget_notice(&a_ep).is_err());
}

#[test]
fn a_hello_that_lies_about_its_notice_is_the_peers_fault() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut b = browser(&t, &Mem::default());
    b.net_connect("a", Role::Peer);
    b.set_peer_meta("a", PeerMeta { transport: "webrtc".into(), dialed: vec![format!("rtc:{}@203.0.113.1:7844", fp('a'))], ..Default::default() });
    let lying = Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([4; 32]),
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
        notice_hash: Some(Id([1; 32])),
        notice: Some(notice("Ola").canonical()),
        declaration: None,
        holder: None,
        forwards: false,
        rebuilds: false,
    };
    let mut kept = true;
    for _ in 0..8 {
        kept = b.net_receive("a", &lying.encode());
        if !kept {
            break;
        }
    }
    assert!(!kept, "a hash that doesn't match the words costs score until the link goes");
    assert!(b.net_status().pending_notices.is_empty() && !b.consent_closed("a"));
}

#[test]
fn a_desktop_does_not_redial_a_node_whose_notice_waits_or_was_declined() {
    // The native dial list (bootstrap_hints) follows the same rule as a browser's candidates:
    // a node whose notice is pending or declined is not dialed on the device's own initiative.
    let t = Arc::new(AtomicU64::new(T0));
    let mut owner = node("desktop", &t, &Mem::default());
    owner.create_identity("mira").unwrap();
    let c = owner.create_community(draft("gated")).unwrap();
    let (a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a', Some("Lena"));
    // A desktop that is not a public node puts only addresses on its own network in invites.
    let route = format!("{a_ep}@127.0.0.1:7843");
    owner.set_transport_address(Some(a_ep.clone()), vec![route.clone()], vec![]);
    let invite = owner.invite(&c.id).unwrap().uri;
    let mut desk = node("desktop", &t, &Mem::default());
    desk.set_consent_gate(true);
    desk.create_identity("lena").unwrap();
    desk.join(&invite).unwrap();
    let dialable = |d: &Node| d.bootstrap_hints().iter().any(|(p, _)| p == &route);
    assert!(dialable(&desk), "an invite's native route is dialed");
    let own = a.operator_notice.as_ref().unwrap();
    let node_id = Id::from_hex(&a_ep).unwrap();
    desk.state.notices.insert(
        node_id,
        crate::state::NoticeRecord {
            hash: own.hash,
            notice: own.bytes.clone(),
            certs: vec![],
            host: None,
            seen: T0,
            asked: true,
            accepted: None,
            declined: None,
            declaration: None,
        },
    );
    assert!(!dialable(&desk), "a pending notice keeps its node out of the native dial list");
    desk.decline_notice(&a_ep).unwrap();
    assert!(!dialable(&desk), "a declined node is not redialed");
    desk.accept_notice(&a_ep, &own.hash.hex()).unwrap();
    assert!(dialable(&desk), "accepted, it is dialed again");
    // Without the gate (a headless node), notices never keep a route out.
    desk.set_consent_gate(false);
    desk.decline_notice(&a_ep).unwrap();
    assert!(dialable(&desk));
}

#[test]
fn a_node_that_keeps_pieces_says_so_and_browsers_read_the_sentence_before_anything_else() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep) = public(&t, &Mem::default(), "203.0.113.1", 'a', Some("Lena"));
    assert!(!a.operator_notice().unwrap().pieces);
    let before = a.operator_notice_hash().unwrap();
    // turning on --hold-pieces changes the notice by itself: a new hash, so people see it again
    a.set_piece_store(Some(Box::new(hoi_store::pieces::MemPieces::new(1 << 20))));
    assert!(a.operator_notice().unwrap().pieces);
    assert_ne!(a.operator_notice_hash().unwrap(), before);
    // and a file that claims pieces on a node that keeps none does not make it say so
    a.set_piece_store(None);
    a.set_notice(Some(NodeNotice { pieces: true, ..notice("Lena") })).unwrap();
    assert!(!a.operator_notice().unwrap().pieces);
    assert_eq!(a.operator_notice_hash().unwrap(), before);
    a.set_piece_store(Some(Box::new(hoi_store::pieces::MemPieces::new(1 << 20))));
    let route = format!("rtc:{}@203.0.113.1:7844", fp('a'));
    let mut b = browser(&t, &Mem::default());
    dial(&mut b, &mut a, &route);
    let hellos = hellos_from(&mut a, "w");
    assert!(!b.net_receive("a", &hellos[0].encode()));
    b.commit().unwrap();
    let pending = b.net_status().pending_notices;
    assert_eq!(pending[0].node, a_ep);
    let s = pending[0].sections.iter().find(|s| s.title == "Encrypted pieces").expect("the sentence is shown");
    assert_eq!(s.text, hoi_types::notice::PIECES);
}
