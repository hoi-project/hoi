//! Configuration declarations, blocks and the assurance policy (RFC-0006, crate::assurance): the
//! validation cases the RFC asks for, on deterministic fixtures. A public node declares how it
//! runs beside its notice; a person's device checks the declaration, binds its answer to the
//! node, the notice and the declaration, blocks a node whose declaration fails for good, and asks
//! again whenever the configuration changes. Under `verified` it relies on no node, since this
//! version verifies none, and says so.
//!
//! What these cases cannot show: that a node runs the software it declares. Its own key signs the
//! declaration, so a modified node passes every case here by declaring what it is expected to;
//! only an attestation profile (RFC-0006 §2), tested on real hardware, could tell.

use super::notice::{browser, dial, fp, hellos_from, public, pump_kept};
use super::*;
use hoi_sync::{Msg, SignedDeclaration};
use hoi_types::NodeDeclaration;
use hoi_types::declaration::{self, NOTHING_DECLARED};

const DIGEST: &str = "1a2b3c4d5e6f1a2b3c4d5e6f1a2b3c4d5e6f1a2b3c4d5e6f1a2b3c4d5e6f1a2b";

/// A public node at 203.0.113.1 with a notice and a declaration, as hoi-node starts one.
fn declaring(t: &Arc<AtomicU64>) -> (Node, String, String) {
    let (mut a, a_ep) = public(t, &Mem::default(), "203.0.113.1", 'a', Some("Lena"));
    a.set_software("hoi-node", "1.0.0", Some(DIGEST)).unwrap();
    (a, a_ep, format!("rtc:{}@203.0.113.1:7844", fp('a')))
}

/// The node's next hello to the browser's link ("w").
fn hello_to_browser(a: &mut Node) -> Msg {
    hellos_from(a, "w").into_iter().last().expect("a hello for the browser")
}

fn with_declaration(mut hello: Msg, d: Option<SignedDeclaration>) -> Msg {
    if let Msg::Hello { declaration, .. } = &mut hello {
        *declaration = d;
    }
    hello
}

/// Signs `d` as the node with transport secret `secret` would.
fn signed(d: &NodeDeclaration, secret: &[u8; 32]) -> SignedDeclaration {
    let kp = Keypair::from_secret(secret);
    let bytes = d.canonical();
    let hash = hoi_types::notice::hash_of(&bytes);
    SignedDeclaration { bytes, sig: kp.sign(declaration::statement(&d.endpoint, &hash).as_bytes()).0 }
}

/// The person accepts what the browser holds for `node`: the hash its view names.
fn accept(b: &mut Node, node: &str) -> NoticeView {
    let view = b.node_notices().into_iter().find(|v| v.node == node).expect("a notice for the node");
    b.accept_notice(node, &view.hash).unwrap()
}

#[test]
fn a_node_declares_how_it_runs_and_the_person_accepts_notice_and_configuration_together() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep, route) = declaring(&t);
    let mut b = browser(&t, &Mem::default());
    dial(&mut b, &mut a, &route);
    let hello = hello_to_browser(&mut a);
    let Msg::Hello { declaration: Some(d), notice_hash: Some(nh), .. } = &hello else { panic!("a declaration beside the notice: {hello:?}") };
    let declared = NodeDeclaration::from_canonical(&d.bytes).unwrap();
    assert_eq!((declared.endpoint.hex(), declared.notice), (a_ep.clone(), *nh));
    assert_eq!((declared.software.name.as_str(), declared.software.version.as_str()), ("hoi-node", "1.0.0"));
    assert_eq!(declared.software.digest.unwrap().hex(), DIGEST);
    assert!(declared.relay.is_some() && declared.carry.as_ref().is_some_and(|c| c.policy == "any" && c.communities == 64) && declared.forwards);
    assert_eq!(a.operator_declaration_hash(), Some(hoi_types::notice::hash_of(&d.bytes).hex()));
    // Nothing of the browser's own goes out before the person answered.
    assert!(b.take_outbox().is_empty());
    assert!(!b.net_receive("a", &hello.encode()), "the link ends until the person has read it all");
    assert!(b.take_outbox().is_empty(), "zero application traffic before approval");
    let view = b.net_status().pending_notices.remove(0);
    assert_eq!((view.assurance.as_str(), view.notice_hash.as_str()), ("declared", nh.hex().as_str()));
    assert_eq!(view.declaration.as_deref(), Some(view.hash.as_str()), "the answer names the declaration, which names the notice");
    let titles: Vec<&str> = view.sections.iter().map(|s| s.title.as_str()).collect();
    assert!(titles.ends_with(&["Software", "Roles and limits", "What was checked"]), "{titles:?}");
    let checked = view.sections.iter().find(|s| s.title == "What was checked").unwrap();
    assert_eq!(checked.text, declaration::DECLARED_ONLY, "a signature by the node's own key is no verification, and the person reads so");
    assert!(view.footer.contains(&format!("Configuration {}.", &view.hash[..12])));
    // Accepting the notice alone does not accept the configuration.
    assert!(b.accept_notice(&a_ep, &nh.hex()).unwrap_err().message.contains("changed since"));
    accept(&mut b, &a_ep);
    b.net_disconnect("a");
    a.net_disconnect("w");
    dial(&mut b, &mut a, &route);
    assert!(pump_kept(&mut b, &mut a));
    assert!(b.peers["a"].said_hello && a.peers["w"].said_hello);
    // The answer is saved with the declaration it named.
    let saved = serde_json::to_string(&b.state).unwrap();
    let back: crate::state::LocalState = serde_json::from_str(&saved).unwrap();
    assert_eq!(back.notices[&Id::from_hex(&a_ep).unwrap()].declaration.as_deref(), Some(d.bytes.as_slice()));
}

#[test]
fn a_changed_configuration_with_an_unchanged_notice_stops_the_link_and_asks_again() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep, route) = declaring(&t);
    let mut b = browser(&t, &Mem::default());
    dial(&mut b, &mut a, &route);
    assert!(!pump_kept(&mut b, &mut a));
    let first = accept(&mut b, &a_ep);
    dial(&mut b, &mut a, &route);
    assert!(pump_kept(&mut b, &mut a));
    let notice_before = a.operator_notice_hash().unwrap();
    // A rebuilt program: same notice, new measurement of the code. Connected strangers hear it at
    // once, and the browser stops the link before anything else goes to the node.
    a.set_software("hoi-node", "1.0.1", Some(&"f".repeat(64))).unwrap();
    assert_eq!(a.operator_notice_hash().unwrap(), notice_before, "the operator's words did not change");
    assert!(!pump_kept(&mut b, &mut a), "a changed declaration ends the link until it was read");
    let pending = b.net_status().pending_notices;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].notice_hash, first.notice_hash);
    assert_ne!(pending[0].hash, first.hash);
    assert!(pending[0].sections.iter().any(|s| s.text.contains("hoi-node 1.0.1, build ffffffffffff")));
    // A changed limit is a change too, and the person accepts the new configuration: traffic resumes.
    accept(&mut b, &a_ep);
    dial(&mut b, &mut a, &route);
    assert!(pump_kept(&mut b, &mut a));
    a.set_carry_bytes(1 << 30);
    assert!(!pump_kept(&mut b, &mut a), "a new limit is a new question as well");
    assert!(b.net_status().pending_notices[0].sections.iter().any(|s| s.text.contains("64 communities and 1 GB together")));
    // An earlier configuration shown again is not quietly taken for the accepted one.
    accept(&mut b, &a_ep);
    dial(&mut b, &mut a, &route);
    let stale = with_declaration(hello_to_browser(&mut a), {
        let mut d = a.operator_declaration().unwrap();
        d.carry.as_mut().unwrap().bytes = 2 << 30;
        Some(signed(&d, &a.transport_secret().unwrap()))
    });
    assert!(!b.net_receive("a", &stale.encode()));
    assert_eq!(b.net_status().pending_notices.len(), 1, "asked again, not matched");
}

#[test]
fn a_node_cannot_fall_back_to_saying_less_than_the_person_accepted() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep, route) = declaring(&t);
    let mut b = browser(&t, &Mem::default());
    dial(&mut b, &mut a, &route);
    assert!(!pump_kept(&mut b, &mut a));
    accept(&mut b, &a_ep);
    dial(&mut b, &mut a, &route);
    // the same notice, its declaration left out
    let bare = with_declaration(hello_to_browser(&mut a), None);
    assert!(!b.net_receive("a", &bare.encode()), "missing evidence is no match for accepted evidence");
    let view = b.net_status().pending_notices.remove(0);
    assert_eq!((view.assurance.as_str(), view.declaration.as_deref()), ("notice", None));
    assert_eq!(view.hash, view.notice_hash);
    assert_eq!(view.sections.last().unwrap().text, NOTHING_DECLARED, "the person reads that nothing was declared");
    assert!(b.take_outbox().is_empty());
}

#[test]
fn a_forged_misbound_or_damaged_declaration_blocks_what_the_connection_proved_for_good() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep, route) = declaring(&t);
    let secret = a.transport_secret().unwrap();
    let genuine = a.operator_declaration().unwrap();
    let other = Keypair::generate().unwrap();
    let mut for_other_notice = genuine.clone();
    for_other_notice.notice = Id([7; 32]);
    let mut naming_another = genuine.clone();
    naming_another.endpoint = other.public();
    let cases: Vec<(&str, SignedDeclaration)> = vec![
        ("not signed by the node", SignedDeclaration { sig: signed(&genuine, &other.secret()).sig, ..signed(&genuine, &secret) }),
        ("for another notice", signed(&for_other_notice, &secret)),
        ("names another node", signed(&naming_another, &other.secret())),
        ("damaged", SignedDeclaration { bytes: vec![0xa1, 0x61], sig: [0; 64] }),
    ];
    for (reason, bad) in cases {
        let memory = Mem::default();
        let mut b = browser(&t, &memory);
        b.open_node_link(&hoi_types::NodeLink { rtc: vec![route.clone()], ..Default::default() }.to_uri()).unwrap();
        assert_eq!(b.discovery_candidates(), vec![route.clone()]);
        dial(&mut b, &mut a, &route);
        let hello = with_declaration(hello_to_browser(&mut a), Some(bad));
        assert!(!b.net_receive("a", &hello.encode()), "{reason}: the link stops");
        assert!(b.take_outbox().is_empty(), "{reason}: nothing went out");
        assert!(!b.consent_closed("a"), "{reason}: a failed check, not a question for the person");
        assert!(b.net_status().pending_notices.is_empty(), "{reason}: nothing to accept");
        assert!(matches!(b.take_events().last(), Some(Event::NodeBlocked { key }) if *key == format!("cert:{}", fp('a'))));
        let blocks = b.node_blocks();
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].reason.contains(reason), "{reason}: {}", blocks[0].reason);
        assert_eq!(
            (blocks[0].endpoint.as_deref(), blocks[0].certs.as_slice()),
            (None, [fp('a')].as_slice()),
            "the certificate the route pinned, never the key a hello merely names"
        );
        // Left out of the dial list, and silent on the next connection, even with a good hello.
        assert!(b.discovery_candidates().is_empty(), "{reason}");
        b.net_disconnect("a");
        a.net_disconnect("w");
        dial(&mut b, &mut a, &route);
        assert!(!pump_kept(&mut b, &mut a), "{reason}: a blocked node is not talked to");
        // Durable: the block outlives a restart.
        b.commit().unwrap();
        let mut again = node("web", &t, &memory);
        again.set_consent_gate(true);
        assert_eq!(again.node_blocks().len(), 1, "{reason}");
        assert!(again.discovery_candidates().is_empty());
        assert!(again.net_status().assurance.offline, "{reason}: no other node to use, and the status says so");
        assert_eq!(again.net_status().assurance.reason.as_deref(), Some(crate::assurance::NONE_LEFT));
        // The person reconsiders: the node may be dialed, and is asked about anew.
        again.unblock_node(&blocks[0].key).unwrap();
        assert!(again.unblock_node(&blocks[0].key).is_err());
        assert_eq!(again.discovery_candidates(), vec![route.clone()]);
        a.net_disconnect("w");
    }
    // A node that shows another's genuine declaration over a connection that proved its own key
    // is blocked by its own key; the node it copied stays usable.
    let mut desk = node("desktop", &t, &Mem::default());
    desk.set_consent_gate(true);
    desk.create_identity("ola").unwrap();
    let copier = Keypair::generate().unwrap().public();
    desk.net_connect("q", Role::Peer);
    desk.set_peer_meta("q", PeerMeta { transport: "quic".into(), endpoint: Some(copier.hex()), ..Default::default() });
    a.net_connect("d", Role::Peer);
    let copied = hellos_from(&mut a, "d").remove(0);
    assert!(!desk.net_receive("q", &copied.encode()));
    let blocks = desk.node_blocks();
    assert_eq!((blocks[0].endpoint.as_deref(), blocks[0].key.as_str()), (Some(copier.hex().as_str()), copier.hex().as_str()));
    assert!(blocks[0].reason.contains("names another node"));
    assert!(desk.consent_blocks(&format!("{copier}@203.0.113.5:7843")) && !desk.consent_blocks(&format!("{a_ep}@203.0.113.1:7843")));
}

#[test]
fn on_a_webrtc_link_the_device_says_nothing_until_the_node_bound_the_channel_to_its_declared_key() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep, route) = declaring(&t);
    let channel = crate::bind::RtcChannel { node: vec![1; 32], browser: vec![2; 32], ufrag: format!("h1p{}abcdefgh", "A".repeat(43)) };
    let mut b = browser(&t, &Mem::default());
    let link = |b: &mut Node, a: &mut Node, as_peer: &str, route: &str, channel: crate::bind::RtcChannel| {
        b.net_connect(as_peer, Role::Peer);
        b.set_peer_meta(as_peer, PeerMeta { transport: "webrtc".into(), dialed: vec![route.into()], rtc: Some(channel.clone()), ..Default::default() });
        a.net_connect("w", Role::Peer);
        a.set_peer_meta("w", PeerMeta { transport: "webrtc".into(), incoming: true, rtc: Some(channel), ..Default::default() });
        a.take_outbox().into_iter().filter(|(p, _)| p == "w").map(|(_, m)| m).collect::<Vec<_>>()
    };
    // The person accepts the node's notice and configuration first (the link ends on first contact).
    let first = link(&mut b, &mut a, "a", &route, channel.clone());
    assert!(!b.net_receive("a", &first[0]));
    accept(&mut b, &a_ep);
    b.net_disconnect("a");
    a.net_disconnect("w");
    // Accepted: the hello is read, and kept until the binding checked out; then the browser speaks.
    let out = link(&mut b, &mut a, "a", &route, channel.clone());
    assert!(matches!(Msg::decode(&out[1]), Ok(Msg::Bind { .. })));
    assert!(b.net_receive("a", &out[0]));
    assert!(b.take_outbox().is_empty(), "nothing before the binding");
    assert!(!b.peers["a"].said_hello);
    assert!(b.net_receive("a", &out[1]));
    assert!(b.peers["a"].said_hello && b.peers["a"].bound);
    let said: Vec<Msg> = b.take_outbox().into_iter().filter(|(p, _)| p == "a").filter_map(|(_, m)| Msg::decode(&m).ok()).collect();
    assert!(matches!(said.first(), Some(Msg::Hello { .. })), "the browser's hello comes first, once bound");
    a.net_disconnect("w");
    // Someone else's channel replays the node's hello, notice and declaration: the hello reads as
    // accepted, but the binding cannot name the key, so the browser says nothing and blocks that
    // channel's certificate. The node it copied stays usable.
    let (mut c, _) = public(&t, &Mem::default(), "203.0.113.3", 'c', None);
    let c_route = format!("rtc:{}@203.0.113.3:7844", fp('c'));
    let replay = link(&mut b, &mut a, "c", &c_route, channel.clone());
    assert!(b.net_receive("c", &replay[0]));
    c.net_connect("w", Role::Peer);
    c.set_peer_meta("w", PeerMeta { transport: "webrtc".into(), incoming: true, rtc: Some(channel.clone()), ..Default::default() });
    let own_bind = c.take_outbox().into_iter().filter(|(p, _)| p == "w").map(|(_, m)| m).find(|m| matches!(Msg::decode(m), Ok(Msg::Bind { .. }))).unwrap();
    assert!(!b.net_receive("c", &own_bind));
    assert!(b.take_outbox().iter().all(|(p, _)| p != "c"), "wrong-session evidence: nothing went out");
    let blocks = b.node_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].certs, vec![fp('c')]);
    assert!(b.consent_blocks(&c_route) && !b.consent_blocks(&route));
}

#[test]
fn notice_less_peers_go_on_as_before_and_a_headless_node_follows_its_operators_policy() {
    let t = Arc::new(AtomicU64::new(T0));
    // A node without a notice declares nothing and is used under `declared`, with the app's own
    // warning, as before; a declaration without a notice names none and changes nothing.
    let (mut plain, _) = public(&t, &Mem::default(), "203.0.113.2", 'p', None);
    plain.set_software("hoi-node", "1.0.0", None).unwrap();
    assert!(plain.operator_declaration_hash().is_none(), "no notice, no declaration");
    let mut b = browser(&t, &Mem::default());
    dial(&mut b, &mut plain, &format!("rtc:{}@203.0.113.2:7844", fp('p')));
    assert!(pump_kept(&mut b, &mut plain));
    // A headless node (no gate) under `declared` asks nobody and checks no declaration: its
    // operator chose to use nodes on what they say.
    let (mut a, _, _) = declaring(&t);
    let (mut h, _) = public(&t, &Mem::default(), "203.0.113.4", 'h', None);
    h.net_connect("a", Role::Peer);
    a.net_connect("h", Role::Peer);
    let forged = with_declaration(hellos_from(&mut a, "h").remove(0), Some(SignedDeclaration { bytes: vec![0xa0], sig: [0; 64] }));
    assert!(h.net_receive("a", &forged.encode()));
    assert!(h.node_blocks().is_empty());
    // Under `verified`, the operator's explicit policy: the node relies on nobody it cannot verify,
    // which in this version is nobody. It dials and greets no stranger, and serves those that dial it.
    h.set_settings(&SettingsPatch { assurance: Some("verified".into()), ..Default::default() }).unwrap();
    assert!(h.take_outbox().iter().any(|(p, m)| p == "a" && matches!(Msg::decode(m), Ok(Msg::Bye { .. }))), "a link it relied on ends");
    h.net_disconnect("a");
    h.net_connect("d", Role::Peer);
    h.set_peer_meta("d", PeerMeta { transport: "quic".into(), ..Default::default() });
    assert!(hellos_from(&mut h, "d").is_empty(), "a node it dialed hears nothing");
    a.net_disconnect("h");
    a.net_connect("h", Role::Peer);
    assert!(!h.net_receive("d", &hellos_from(&mut a, "h").remove(0).encode()));
    h.net_connect("x", Role::Peer);
    h.set_peer_meta("x", PeerMeta { transport: "quic".into(), incoming: true, ..Default::default() });
    assert_eq!(hellos_from(&mut h, "x").len(), 1, "a stranger that dialed in is served");
    let status = h.net_status().assurance;
    assert_eq!((status.policy.as_str(), status.offline), ("verified", true));
    assert_eq!(status.reason.as_deref(), Some(crate::assurance::NO_VERIFIER));
    assert!(h.set_settings(&SettingsPatch { assurance: Some("trusted".into()), ..Default::default() }).is_err());
}

#[test]
fn under_verified_nothing_falls_back_to_declarations_until_the_person_switches_back() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep, route) = declaring(&t);
    let mut b = browser(&t, &Mem::default());
    b.open_node_link(&hoi_types::NodeLink { rtc: vec![route.clone()], ..Default::default() }.to_uri()).unwrap();
    dial(&mut b, &mut a, &route);
    assert!(!pump_kept(&mut b, &mut a));
    accept(&mut b, &a_ep);
    dial(&mut b, &mut a, &route);
    assert!(pump_kept(&mut b, &mut a), "accepted under declared");
    // The person turns on `verified` while connected: the link it relied on stops at once.
    b.take_outbox();
    b.set_settings(&SettingsPatch { assurance: Some("verified".into()), ..Default::default() }).unwrap();
    let out: Vec<Msg> = b.take_outbox().into_iter().filter(|(p, _)| p == "a").filter_map(|(_, m)| Msg::decode(&m).ok()).collect();
    assert!(matches!(out.as_slice(), [Msg::Bye { .. }]), "queued traffic dropped, a goodbye and nothing else: {out:?}");
    b.net_disconnect("a");
    a.net_disconnect("w");
    // Its accepted declaration is no verification: no dial on its own, and a link ends unspoken.
    assert!(b.discovery_candidates().is_empty());
    dial(&mut b, &mut a, &route);
    assert!(!pump_kept(&mut b, &mut a));
    assert!(!b.consent_closed("a"), "no question for the person: nothing to accept under verified");
    // Nor does its home node hear anything.
    b.net_connect("home", Role::Home);
    assert!(b.take_outbox().is_empty());
    let status = b.net_status().assurance;
    assert!(status.offline && status.reason.as_deref() == Some(crate::assurance::NO_VERIFIER));
    // Declared use again is the person's explicit choice: then the earlier answer holds.
    b.net_disconnect("home");
    b.set_settings(&SettingsPatch { assurance: Some("declared".into()), ..Default::default() }).unwrap();
    assert_eq!(b.discovery_candidates(), vec![route.clone()]);
    dial(&mut b, &mut a, &route);
    assert!(pump_kept(&mut b, &mut a));
    assert!(!b.net_status().assurance.offline);
}

#[test]
fn another_node_is_used_only_after_its_own_approval() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, _, route) = declaring(&t);
    let mut b = browser(&t, &Mem::default());
    b.open_node_link(&hoi_types::NodeLink { rtc: vec![route.clone()], ..Default::default() }.to_uri()).unwrap();
    dial(&mut b, &mut a, &route);
    let forged = with_declaration(hello_to_browser(&mut a), Some(SignedDeclaration { bytes: vec![0xa0], sig: [0; 64] }));
    assert!(!b.net_receive("a", &forged.encode()));
    b.net_disconnect("a");
    // The only node it knew is blocked: offline, and it says why.
    assert!(b.net_status().assurance.offline);
    // Another node is a new question of its own; until answered, still nothing to use.
    let (mut e, e_ep) = public(&t, &Mem::default(), "203.0.113.6", 'e', Some("Ola"));
    e.set_software("hoi-node", "1.0.0", None).unwrap();
    let e_route = format!("rtc:{}@203.0.113.6:7844", fp('e'));
    dial(&mut b, &mut e, &e_route);
    assert!(!pump_kept(&mut b, &mut e), "a replacement needs its own approval");
    let pending = b.net_status().pending_notices;
    assert_eq!((pending.len(), pending[0].node.as_str()), (1, e_ep.as_str()));
    assert!(pending[0].sections.iter().any(|s| s.text.ends_with("It gave no build fingerprint.")));
    assert!(b.net_status().assurance.offline);
    accept(&mut b, &e_ep);
    dial(&mut b, &mut e, &e_route);
    assert!(pump_kept(&mut b, &mut e));
    assert!(!b.net_status().assurance.offline);
    assert_eq!(b.node_blocks().len(), 1, "the blocked node stays blocked");
}

#[test]
fn answers_saved_before_declarations_still_open_and_a_declaring_node_is_asked_again() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut a, a_ep, route) = declaring(&t);
    let memory = Mem::default();
    let mut b = browser(&t, &memory);
    // Accepted from the node's link, as before declarations: the record names the notice alone.
    b.preview_node_link(&a.net_status().node_link.unwrap()).unwrap();
    b.accept_notice(&a_ep, &a.operator_notice_hash().unwrap()).unwrap();
    b.commit().unwrap();
    // The saved state is what older software wrote, and still reads: no `declaration` in the
    // record, no `node_blocks` at all.
    let saved = serde_json::to_value(&b.state).unwrap();
    assert!(saved.get("node_blocks").is_none());
    assert!(saved["notices"].as_object().unwrap().values().all(|r| r.get("declaration").is_none()));
    let mut b = node("web", &t, &memory);
    b.set_consent_gate(true);
    assert_eq!(b.node_notices()[0].state, "accepted");
    // The node declares its configuration: the old answer did not cover it, so it is asked again.
    dial(&mut b, &mut a, &route);
    assert!(!pump_kept(&mut b, &mut a));
    let pending = b.net_status().pending_notices;
    assert_eq!((pending.len(), pending[0].assurance.as_str()), (1, "declared"));
    accept(&mut b, &a_ep);
    dial(&mut b, &mut a, &route);
    assert!(pump_kept(&mut b, &mut a));
}
