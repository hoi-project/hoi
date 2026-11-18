//! Address privacy, phase S7 (SCATTER.md §10): uploads in three hops, so the link a
//! device sends through never learns the holders; and an injector for a device's own records,
//! so it never offers its own new records to its own links before they come back.
use super::*;
use crate::tests::forward::{Mesh, join, mesh};
use hoi_sync::{Msg, PieceOp};

const DAY: u64 = 86_400;

/// `mesh(3, 14)` with the three forwarders linked to one another, so each vouches for the others.
fn linked_mesh() -> Mesh {
    let mut m = mesh(3, 14);
    for (x, y) in [("f0", "f1"), ("f1", "f2"), ("f2", "f0")] {
        m.dial(x, y);
        m.dial(y, x);
    }
    m.pump();
    m.t.fetch_add(3600, Ordering::SeqCst);
    m.run(5);
    m
}

fn stored(o: crate::scatter::ScatterOutcome) -> hoi_types::scatter::Manifest {
    match o {
        crate::scatter::ScatterOutcome::Stored(m) => m,
        other => panic!("not stored: {other:?}"),
    }
}

#[test]
fn an_upload_goes_in_three_hops_and_the_first_never_learns_the_holders() {
    let mut m = linked_mesh();
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    m.run(5);
    m.log.clear();
    let content = b"three hops: the first one sees only the second".to_vec();
    let op = m.n("a").scatter_store(&content, 10, T0 + 30 * DAY).unwrap();
    let man = stored(m.wait("a", op, 120));
    assert!(m.n("a").scatter.wrapped >= man.n as u64, "every put went wrapped: {}", m.n("a").scatter.wrapped);
    let forwarders: Vec<String> = (0..3).map(|i| m.eps[&format!("f{i}")].clone()).collect();
    let holders: Vec<String> = man.holders.iter().map(|h| h.hex()).collect();
    // what the device sent names forwarders only, never a holder
    let from_a: Vec<&Msg> = m.log.iter().filter(|(f, _, _)| f == "a").map(|(_, _, msg)| msg).collect();
    for msg in &from_a {
        if let Msg::Piece { op, to, .. } = msg {
            assert_eq!(*op, PieceOp::Wrap);
            let to = to.expect("through a forwarder").hex();
            assert!(forwarders.contains(&to) && !holders.contains(&to), "{to}");
        }
    }
    // the entry passed each on to its exit, unread; only exits spoke to holders
    for (from, to, msg) in &m.log {
        if from.starts_with('f') && to.starts_with('h') {
            assert!(matches!(msg, Msg::Piece { op: PieceOp::Put, to: None, .. }), "{from} → {to}");
        }
    }
    let entries: Vec<(&String, &String)> = m
        .log
        .iter()
        .filter(|(f, to, msg)| f.starts_with('f') && to.starts_with('f') && matches!(msg, Msg::Piece { op: PieceOp::Wrap, .. }))
        .map(|(f, t, _)| (f, t))
        .collect();
    assert!(entries.len() >= man.n as usize, "{entries:?}");
    assert!(entries.iter().all(|(f, t)| f != t));
    // and it reads back
    join(&mut m, "r", "192.0.2.20:7843", &[0, 1, 2]);
    let op = m.n("r").scatter_load(&man).unwrap();
    assert_eq!(m.wait("r", op, 120), crate::scatter::ScatterOutcome::Loaded(content));
}

#[test]
fn without_a_second_forwarder_that_reaches_the_first_an_upload_goes_in_two_hops() {
    let mut m = mesh(3, 14);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    let op = m.n("a").scatter_store(b"two hops, as before", 10, T0 + 30 * DAY).unwrap();
    stored(m.wait("a", op, 120));
    assert_eq!(m.n("a").scatter.wrapped, 0);
}

#[test]
fn author_deletion_uses_three_hops_and_fallback_is_disclosed() {
    for wrapped in [true, false] {
        let mut m = if wrapped { linked_mesh() } else { mesh(3, 14) };
        join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
        let op = m.n("a").scatter_store(b"delete this object", 10, T0 + 30 * DAY).unwrap();
        let man = stored(m.wait("a", op, 120));
        m.log.clear();
        let secret = m.n("a").drop_secret().unwrap();
        let op = m.n("a").scatter_drop(&man, &crate::scatter::Ask::DropToken(secret)).unwrap();
        let outcome = m.wait("a", op, 120);
        assert!(
            matches!(outcome, crate::scatter::ScatterOutcome::Dropped(ref rows) if rows.len() == man.hashes.len()
                && rows.iter().all(|r| r.outcome == crate::scatter::DropOutcome::LegacyRemoval)),
            "{outcome:?}"
        );
        let sent: Vec<_> = m.log.iter().filter(|(from, _, msg)| from == "a" && matches!(msg, Msg::Piece { .. })).collect();
        assert!(!sent.is_empty());
        for (_, _, msg) in sent {
            assert!(matches!(msg, Msg::Piece { op, .. } if *op == if wrapped { PieceOp::Wrap } else { PieceOp::Drop }));
        }
        assert_eq!(m.n("a").events.iter().any(|e| matches!(e, crate::dto::Event::Notice {text} if text.contains("shorter route"))), !wrapped);
    }
}

/// The ids of the records a message carries, if it carries any.
fn carried(msg: &Msg) -> Vec<Id> {
    match msg {
        Msg::Recs { recs } => recs.iter().filter_map(|w| hoi_validate::decode(w).ok().map(|d| d.id)).collect(),
        _ => vec![],
    }
}

#[test]
fn a_device_s_own_record_reaches_others_through_an_injector_and_never_from_it_first() {
    let mut m = linked_mesh();
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    join(&mut m, "b", "192.0.2.20:7843", &[0, 1, 2]);
    let mut d = draft("an open garden");
    d.limits.as_mut().unwrap().scatter_open = Some(false);
    let c = m.n("a").create_community(d).unwrap().id;
    let uri = m.n("a").invite(&c).unwrap().uri;
    m.n("b").join(&uri).unwrap();
    m.run(60);
    m.log.clear();
    let p = post(m.n("a"), &c, "whose post is this?");
    let pid = Id::from_hex(&p.id).unwrap();
    assert!(m.n("a").scatter.saved.injecting.contains_key(&pid), "sent to an injector, kept from a's own links");
    let mut came = false;
    for _ in 0..12 {
        m.run(5);
        if m.n("b").post(&p.id).is_ok() {
            came = true;
            break;
        }
    }
    assert!(came, "b has it, from the injector");
    // a never offered it to its links: no record message of a's carried it
    assert!(!m.log.iter().any(|(f, _, msg)| f == "a" && carried(msg).contains(&pid)), "a sent its own post to nobody");
    // what a sent was one sealed injection, through a link that is not the injector
    let injected: Vec<(&String, Option<Id>)> = m
        .log
        .iter()
        .filter_map(|(f, t, msg)| match msg {
            Msg::Piece { op: PieceOp::Inject, to, .. } if f == "a" => Some((t, *to)),
            _ => None,
        })
        .collect();
    assert_eq!(injected.len(), 1, "{injected:?}");
    let (entry, injector) = injected[0];
    assert_ne!(Some(m.eps[entry].clone()), injector.map(|i| i.hex()), "through another link than the injector");
    // it came back to a, which now treats it as any other record
    assert!(until_back(&mut m, &pid));
    // with no injector around, a device's record goes as before
    let mut lone = mesh(1, 0);
    join(&mut lone, "a", "198.51.100.10:7843", &[0]);
    let c = lone.n("a").create_community(draft("a lone garden")).unwrap().id;
    let q = post(lone.n("a"), &c, "straight out");
    assert!(lone.n("a").scatter.saved.injecting.is_empty());
    lone.run(5);
    assert!(lone.log.iter().any(|(f, _, msg)| f == "a" && carried(msg).contains(&Id::from_hex(&q.id).unwrap())));
}

fn until_back(m: &mut Mesh, id: &Id) -> bool {
    for _ in 0..30 {
        if !m.n("a").scatter.saved.injecting.contains_key(id) {
            return true;
        }
        m.run(5);
    }
    false
}

/// Runs half a minute at a time until `done`, at most `secs`.
fn until(m: &mut Mesh, secs: u64, done: impl Fn(&mut Mesh) -> bool) -> bool {
    for _ in 0..secs.div_ceil(30) {
        if done(m) {
            return true;
        }
        m.run(30);
    }
    done(m)
}

/// `a` makes an open community, in pieces by default, and `b` joins it; the forwarders link to
/// one another, so a has an injector and an entry.
fn open_on_linked_mesh() -> (Mesh, String) {
    let mut m = linked_mesh();
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    join(&mut m, "b", "192.0.2.20:7843", &[0, 1, 2]);
    let c = m.n("a").create_community(draft("an open garden")).unwrap().id;
    let uri = m.n("a").invite(&c).unwrap().uri;
    m.n("b").join(&uri).unwrap();
    m.run(60);
    (m, c)
}

#[test]
fn an_open_community_s_post_reaches_nobody_whole_through_an_injector() {
    let (mut m, c) = open_on_linked_mesh();
    m.log.clear();
    let p = post(m.n("a"), &c, "in pieces, and through an injector");
    let pid = Id::from_hex(&p.id).unwrap();
    assert!(!m.n("a").scatter.saved.injecting.contains_key(&pid), "the post itself is never sealed whole to an injector");
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()), "b rebuilt the post");
    for f in ["f0", "f1", "f2"] {
        assert!(m.n(f).store.get(&pid).is_none(), "{f} never got the post whole");
    }
    assert!(!m.log.iter().any(|(_, _, msg)| carried(msg).contains(&pid)), "no record message carried the post whole");
    // its pointer records went through the injector instead
    assert!(m.log.iter().any(|(f, _, msg)| f == "a" && matches!(msg, Msg::Piece { op: PieceOp::Inject, .. })));
}

#[test]
fn a_post_that_goes_whole_goes_through_the_injector() {
    let (mut m, c) = open_on_linked_mesh();
    m.log.clear();
    let p = post(m.n("a"), &c, "whole, for once");
    let pid = Id::from_hex(&p.id).unwrap();
    // the device gives up on pieces for this one: it goes whole, but still not from a first
    let ca = Id::from_hex(&c).unwrap();
    m.n("a").open_whole(ca, vec![pid]);
    assert!(m.n("a").scatter.saved.injecting.contains_key(&pid), "held, and sent to an injector");
    m.run(5);
    assert!(!m.log.iter().any(|(f, _, msg)| f == "a" && carried(msg).contains(&pid)), "a offered it to nobody");
    assert!(until(&mut m, 300, |m| m.n("b").post(&p.id).is_ok()), "b has it, from the injector");
}

#[test]
fn a_held_record_stays_held_over_a_restart() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut a = node("web", &t, &mem);
    a.create_identity("mira").unwrap();
    let c = a.create_community(draft("a garden")).unwrap().id;
    let p = post(&mut a, &c, "held a moment");
    let pid = Id::from_hex(&p.id).unwrap();
    a.scatter.saved.whole.insert(pid);
    a.scatter.saved.injecting.insert(pid, T0 + 120);
    a.scatter.dirty = true;
    a.commit().unwrap();
    drop(a);
    let b = node("web", &t, &mem);
    assert_eq!(b.scatter.saved.injecting.get(&pid), Some(&(T0 + 120)), "the hold came back with the device");
    assert!(!b.forwardable(b.store.get(&pid).unwrap()), "and it still keeps the record from its links");
}

#[test]
fn a_new_identity_goes_to_its_own_devices_only() {
    let mut m = linked_mesh();
    let n = node("desktop", &m.t, &Mem::default());
    m.add("a", n, "198.51.100.10:7843");
    for f in 0..3 {
        m.dial("a", &format!("f{f}"));
    }
    m.pump();
    m.log.clear();
    m.n("a").create_identity("nadia").unwrap();
    m.run(30);
    let a = m.n("a");
    let root = a.me_root().unwrap();
    let mut mine: Vec<Id> = a.store.delegations_of(&root).iter().map(|(_, d)| d.record).collect();
    mine.extend(a.store.identity(&root).map(|i| i.record));
    assert!(!mine.is_empty());
    assert!(
        !m.log.iter().any(|(f, _, msg)| f == "a" && carried(msg).iter().any(|id| mine.contains(id))),
        "a offered its identity and blessing to none of its links"
    );
    assert!(m.n("a").scatter.saved.exposed_at.is_none());
}

#[test]
fn a_renewed_blessing_reaches_others_only_through_an_injector() {
    let (mut m, c) = open_on_linked_mesh();
    let p = post(m.n("b"), &c, "the first, through an injector");
    assert!(until(&mut m, 600, |m| m.n("a").post(&p.id).is_ok()), "a rebuilt b's post");
    m.log.clear();
    // b's device blesses itself again, as it does before its blessing lapses
    let b = m.n("b");
    let (root, dev) = (b.root_kp.clone().unwrap(), b.device_kp.clone().unwrap());
    let mut r = Node::delegation_record(&root, dev.public(), b.now() + 1);
    if let Body::Delegation { seal, .. } = &mut r.body {
        *seal = crate::private::seal_public(&dev);
    }
    let d = b.publish(r).unwrap();
    m.run(30);
    assert!(!m.log.iter().any(|(f, _, msg)| f == "b" && carried(msg).contains(&d)), "b offered its new blessing to none of its links");
    // others get it with b's next record, through an injector
    let q = post(m.n("b"), &c, "the second");
    assert!(until(&mut m, 600, |m| m.n("a").post(&q.id).is_ok() && m.n("a").store.contains(&d)), "a has the new blessing");
    assert!(!m.log.iter().any(|(f, _, msg)| f == "b" && carried(msg).contains(&d)));
    assert!(m.n("b").scatter.saved.exposed_at.is_none(), "b never offered its person's records first");
}

fn home_phone() -> (Mesh, String) {
    let (mut m, c) = open_on_linked_mesh();
    let uri = m.n("a").invite(&Id::from_hex(&c).unwrap().hex()).unwrap().uri;
    join(&mut m, "home", "203.0.113.50:7843", &[0, 1, 2]);
    m.n("home").join(&uri).unwrap();
    let pairing = m.n("home").set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    let mut phone = node("mobile", &m.t, &Mem::default());
    phone.create_identity("lena").unwrap();
    phone.pair(&pairing).unwrap();
    phone.net_connect("home", Role::Home);
    m.nodes.insert("phone".into(), phone);
    m.n("home").net_connect("phone", Role::Client);
    m.pump();
    m.n("phone").join(&uri).unwrap();
    m.run(60);
    m.log.clear();
    (m, c)
}

#[test]
fn a_home_node_places_phone_posts_with_recoverable_deletion_authority() {
    let (mut m, c) = home_phone();
    // Fully featured homes can omit the legacy feature to fit the 16-feature hello limit.
    // The v2 capability must still support delivery-status polling and completion.
    m.n("phone").peers.get_mut("home").unwrap().features.remove(hoi_sync::FEATURE_HOME_SCATTER);
    let p = post(m.n("phone"), &c, "from the phone, through the home node");
    let pid = Id::from_hex(&p.id).unwrap();
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()), "b has the phone's post");
    m.run(60);
    assert!(m.n("phone").scatter.saved.own_pointers.contains_key(&pid));
    assert!(!m.n("phone").scatter.saved.whole.contains(&pid));
    assert!(m.log.iter().any(|(f, _, msg)| f == "phone"
        && matches!(msg, Msg::HomeScatter { reply:false,payload,.. }
            if serde_json::from_slice::<serde_json::Value>(payload).unwrap()["type"] == "JournalStore")));
    assert!(!m.n("phone").post(&p.id).unwrap().checks.address_hidden, "the home knows its sender");
    assert_eq!(m.n("phone").post(&p.id).unwrap().checks.home_delivery.as_deref(), Some("pieces-injected"));
    assert!(!m.log.iter().any(|(f, _, msg)| f == "phone" && carried(msg).contains(&pid)), "the phone sends pointers instead of whole records");
}

#[test]
fn the_post_view_says_the_node_you_connect_through_cannot_tell() {
    let (mut m, c) = open_on_linked_mesh();
    let p = post(m.n("b"), &c, "whose is this?");
    assert!(until(&mut m, 600, |m| m.n("a").post(&p.id).is_ok()), "a rebuilt b's post");
    assert!(until(&mut m, 300, |m| m.n("b").post(&p.id).unwrap().checks.address_hidden), "the injector took b's pointers and they came back");
    assert!(!m.n("a").post(&p.id).unwrap().checks.address_hidden, "only on the device's own posts");
    // once this device offers its person's records first hand, it says it no more
    m.n("b").note_exposed();
    assert!(!m.n("b").post(&p.id).unwrap().checks.address_hidden);
    // with no injector around, never
    let mut lone = mesh(3, 14);
    let c = {
        join(&mut lone, "a", "198.51.100.10:7843", &[0, 1, 2]);
        lone.n("a").create_community(draft("a garden")).unwrap().id
    };
    let q = post(lone.n("a"), &c, "straight out");
    lone.run(300);
    assert!(!lone.n("a").post(&q.id).unwrap().checks.address_hidden);
}

#[test]
fn another_authorized_device_can_delete_pieces_after_restart_without_the_original() {
    let (mut m, c) = open_on_linked_mesh();
    let disk = Mem::default();
    let mut second = node("desktop", &m.t, &disk);
    let request = second.link_request("second device").unwrap();
    let approval = m.n("a").link_approve(&request).unwrap();
    second.link_complete(&approval).unwrap();
    let pairing = m.n("a").set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    second.pair(&pairing).unwrap();
    second.net_connect("a", Role::Home);
    m.add("second", second, "192.0.2.99:7843");
    m.n("a").net_connect("second", Role::Client);
    for f in ["f0", "f1", "f2"] {
        m.dial("second", f);
    }
    m.run(60);
    let p = post(m.n("a"), &c, "delete from my other device");
    assert!(until(&mut m, 600, |m| m.n("second").post(&p.id).is_ok()));
    m.run(120);
    let manifest = m.n("a").scatter.saved.kept.values().find(|k| k.records.contains(&Id::from_hex(&p.id).unwrap())).unwrap().newest().unwrap();
    let cap = crate::drop_share::cap_id(&manifest);
    assert!(
        m.n("second").scatter.saved.drop_caps.contains_key(&cap),
        "sender caps {}, second blessing {:?}, links {}",
        m.nodes["a"].scatter.saved.drop_caps.len(),
        m.nodes["a"].store.delegations_of(&m.nodes["second"].me_root().unwrap()),
        m.log.iter().filter(|(f, _, msg)| f == "a" && matches!(msg, Msg::Link { .. })).count()
    );
    assert_ne!(m.n("second").drop_secret(), m.n("a").drop_secret(), "no device-wide secret was copied");
    m.n("second").commit().unwrap();
    for f in ["f0", "f1", "f2", "a"] {
        m.hang_up("second", f);
    }
    m.nodes.remove("a");
    let restarted = node("desktop", &m.t, &disk);
    m.add("second", restarted, "192.0.2.99:7843");
    assert!(m.n("second").scatter.saved.drop_caps.contains_key(&cap));
    for f in ["f0", "f1", "f2"] {
        m.dial("second", f);
    }
    m.run(30);
    m.n("second").delete_own(&p.id).unwrap();
    m.run(120);
    let indexes = (0..manifest.hashes.len() as u8).collect::<Vec<_>>();
    let op = m.n("b").scatter_probe(&manifest, &indexes, &Default::default()).unwrap();
    assert!(matches!(m.wait("b", op, 120), crate::scatter::ScatterOutcome::Probed(rows) if rows.iter().all(|r| r.kept == Some(false))));
}

#[test]
fn revoked_devices_receive_no_new_deletion_capability() {
    let (mut m, c) = open_on_linked_mesh();
    let mut second = node("desktop", &m.t, &Mem::default());
    let req = second.link_request("revoked device").unwrap();
    let approval = m.n("a").link_approve(&req).unwrap();
    second.link_complete(&approval).unwrap();
    let key = second.device_kp.as_ref().unwrap().public();
    let pairing = m.n("a").set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    second.pair(&pairing).unwrap();
    second.net_connect("a", Role::Home);
    m.add("second", second, "192.0.2.99:7843");
    m.n("a").net_connect("second", Role::Client);
    m.run(60);
    m.n("a").revoke_device(&key.hex()).unwrap();
    let p = post(m.n("a"), &c, "created after revocation");
    m.run(300);
    let manifest = m.n("a").scatter.saved.kept.values().find(|k| k.records.contains(&Id::from_hex(&p.id).unwrap())).unwrap().newest().unwrap();
    assert!(!m.n("second").scatter.saved.drop_caps.contains_key(&crate::drop_share::cap_id(&manifest)));
}

#[test]
fn home_delivery_waits_for_reconnection_and_receipts_never_claim_privacy_from_home() {
    let (mut m, c) = home_phone();
    m.hang_up("phone", "home");
    let p = post(m.n("phone"), &c, "home goes away during delivery");
    m.t.fetch_add(5, Ordering::SeqCst);
    m.n("phone").scatter_tick();
    assert_eq!(m.n("phone").post(&p.id).unwrap().checks.home_delivery.as_deref(), Some("whole-pending"));
    m.run(130);
    assert!(m.n("phone").scatter.saved.whole.contains(&Id::from_hex(&p.id).unwrap()));
    assert!(!m.n("phone").post(&p.id).unwrap().checks.address_hidden);
    m.n("phone").net_connect("home", Role::Home);
    m.n("home").net_connect("phone", Role::Client);
    m.pump();
    m.run(180);
    assert!(m.n("b").post(&p.id).is_ok());
    assert_eq!(m.n("phone").post(&p.id).unwrap().checks.home_delivery.as_deref(), Some("whole-injected"));
    assert!(!m.n("phone").post(&p.id).unwrap().checks.address_hidden);
}

#[test]
fn legacy_home_placement_ignores_strangers_and_releases_a_wait_after_an_invalid_reply() {
    let (mut m, c) = home_phone();
    let op = m.n("phone").legacy_home_store("home", Id::from_hex(&c).unwrap(), b"authenticated response", 10, T0 + 30 * DAY).unwrap();
    let rid = m
        .n("phone")
        .take_outbox()
        .into_iter()
        .find_map(|(_, bytes)| match Msg::decode(&bytes).ok()? {
            Msg::HomeScatter { rid, reply: false, .. } => Some(rid),
            _ => None,
        })
        .unwrap();
    let bad = br#"{"type":"Receipt","states":{}}"#;
    m.n("phone").on_home_scatter("f0", rid, true, bad);
    assert!(!m.n("phone").op_done(op));
    m.n("phone").on_home_scatter("home", rid, true, bad);
    assert!(matches!(m.wait("phone", op, 1), crate::scatter::ScatterOutcome::Failed(_)));
}

#[test]
fn legacy_home_placement_still_returns_a_verified_manifest() {
    let (mut m, c) = home_phone();
    let content = b"legacy placement response";
    let op = m.n("phone").legacy_home_store("home", Id::from_hex(&c).unwrap(), content, 10, T0 + 30 * DAY).unwrap();
    let manifest = match m.wait("phone", op, 120) {
        crate::scatter::ScatterOutcome::Stored(manifest) => manifest,
        other => panic!("legacy placement failed: {other:?}"),
    };
    assert!(m.n("phone").scatter.saved.drop_caps.contains_key(&crate::drop_share::cap_id(&manifest)));
    let op = m.n("b").scatter_load(&manifest).unwrap();
    assert!(matches!(m.wait("b", op, 120), crate::scatter::ScatterOutcome::Loaded(bytes) if bytes == content));
}

#[test]
fn deleting_during_a_legacy_home_placement_is_retryable_before_any_mutation() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "awaiting legacy placement authority");
    let target = Id::from_hex(&p.id).unwrap();
    let community = Id::from_hex(&c).unwrap();
    let phone = m.n("phone");
    let content = b"[]".to_vec();
    let op = phone.legacy_home_store("home", community, &content, 10, T0 + 30 * DAY).unwrap();
    let batch = phone.scatter.repair.outgoing.keys().next_back().map_or(0, |batch| batch + 1);
    phone.scatter.jobs.insert(op, crate::keep::Job::Outgoing { batch });
    phone.scatter.repair.outgoing.insert(
        batch,
        crate::keep::Outgoing {
            community,
            parts: BTreeMap::from([(op, content)]),
            carries: BTreeMap::from([(op, (vec![target], None))]),
            stored: BTreeMap::new(),
            cancelled: Default::default(),
            failed: false,
            records: vec![target],
            media: Vec::new(),
        },
    );
    let before = phone.deletions.saved.jobs.len();
    assert_eq!(phone.delete_own(&p.id).unwrap_err().code, "deletion-pending");
    assert!(phone.store.contains(&target));
    assert!(!phone.state.skeletons.contains_key(&target));
    assert_eq!(phone.deletions.saved.jobs.len(), before);
    assert!(phone.deletion_for_target(target).is_none());
    assert!(phone.scatter.repair.outgoing[&batch].cancelled.is_empty());
    assert!(phone.deletion_home_placement_pending(target));
    assert!(!phone.op_done(op));
}

/// Snapshot the accepted persistence transaction, rather than serializing volatile runtime.
fn home_checkpoint(n: &mut Node) -> Mem {
    n.commit().unwrap();
    let loaded = n.persist_mut().load().unwrap();
    let disk = Mem::default();
    let mut data = disk.0.lock().unwrap();
    for wire in loaded.records {
        let id = hoi_types::Wire::decode(&wire).unwrap().1;
        data.0.insert(id, wire);
    }
    data.1.extend(loaded.blobs);
    drop(data);
    disk
}

fn pump_losing_home_results(m: &mut Mesh) {
    for _ in 0..400 {
        let mut quiet = true;
        let names: Vec<_> = m.nodes.keys().cloned().collect();
        for from in names {
            for (to, bytes) in m.n(&from).take_outbox() {
                if from == "home"
                    && to == "phone"
                    && matches!(Msg::decode(&bytes),Ok(Msg::HomeScatter{reply:true,payload,..}) if serde_json::from_slice::<serde_json::Value>(&payload).unwrap()["type"]=="JournalResult")
                {
                    continue;
                }
                if m.nodes.contains_key(&to) {
                    quiet = false;
                    m.n(&to).net_receive(&from, &bytes);
                }
            }
        }
        if quiet {
            break;
        }
    }
}

#[test]
fn a_lost_placement_reply_survives_both_restarts_without_duplicate_posts_and_still_deletes() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "retry my same request after both restarts");
    let pid = Id::from_hex(&p.id).unwrap();
    for _ in 0..60 {
        m.t.fetch_add(5, Ordering::SeqCst);
        for n in m.nodes.values_mut() {
            n.scatter_tick();
        }
        pump_losing_home_results(&mut m);
        if m.n("home").home_journal.saved.placements.values().any(|p| p.result == Some(true)) {
            break;
        }
    }
    assert_eq!(m.n("home").home_journal.saved.placements.len(), 1);
    let original = m.n("home").home_journal.saved.placements.values().next().unwrap().capability.as_ref().unwrap().read().unwrap();
    assert_eq!(m.n("home").home_journal.saved.placements.values().next().unwrap().result, Some(true));
    assert!(m.n("phone").journal_deletion_pending(pid));
    assert_eq!(m.n("phone").delete_own(&p.id).unwrap_err().code, "deletion-pending");
    let phone_disk = home_checkpoint(m.n("phone"));
    let home_disk = home_checkpoint(m.n("home"));
    m.hang_up("phone", "home");
    for f in ["f0", "f1", "f2"] {
        m.hang_up("home", f);
    }
    let phone = node("mobile", &m.t, &phone_disk);
    let mut home = node("desktop", &m.t, &home_disk);
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    m.add("phone", phone, "192.0.2.60:7843");
    m.add("home", home, "203.0.113.50:7843");
    for f in ["f0", "f1", "f2"] {
        m.dial("home", f);
    }
    m.n("phone").net_connect("home", Role::Home);
    m.n("home").net_connect("phone", Role::Client);
    m.pump();
    // An existing receipt is recoverable even before a restarted home can advertise fresh
    // placement capacity. Neither reconnect nor cleanup depends on adding another generation.
    m.n("phone").peers.get_mut("home").unwrap().features.remove("scatter-store");
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()));
    assert_eq!(m.n("home").home_journal.saved.placements.len(), 1, "retry never creates a second placement");
    let recovered = m.n("phone").scatter.saved.kept.values().find(|k| k.records.contains(&pid)).unwrap().newest().unwrap();
    assert_eq!(original, recovered);
    assert!(m.n("phone").scatter.saved.drop_caps.contains_key(&crate::drop_share::cap_id(&recovered)));
    assert_eq!(m.n("b").store.in_community(&Id::from_hex(&c).unwrap()).filter(|s| s.id == pid).count(), 1);
    m.n("phone").delete_own(&p.id).unwrap();
    // The deletion itself also survives the originating device's restart with only locator
    // capabilities left: neither the manifest nor runtime holder-key cache is required.
    let deleting = home_checkpoint(m.n("phone"));
    m.hang_up("phone", "home");
    let phone = node("mobile", &m.t, &deleting);
    m.add("phone", phone, "192.0.2.60:7843");
    m.n("phone").net_connect("home", Role::Home);
    m.n("home").net_connect("phone", Role::Client);
    m.pump();
    m.run(180);
    let progress = m.n("phone").deletion_progress().into_iter().find(|row| row.target == p.id).unwrap();
    assert_eq!(progress.acknowledged, 0, "volatile test holders cannot assert durable removal");
    assert!(progress.legacy_removal > 0, "real holder replies, rather than home acceptance");
    let indexes = (0..recovered.hashes.len() as u8).collect::<Vec<_>>();
    let op = m.n("b").scatter_probe(&recovered, &indexes, &Default::default()).unwrap();
    assert!(matches!(m.wait("b",op,120),crate::scatter::ScatterOutcome::Probed(rows) if rows.iter().all(|r|r.kept==Some(false))));
}

#[test]
fn duplicate_home_requests_reuse_the_receipt_and_conflicts_do_not_change_it() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "same request");
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()));
    let packet = m
        .log
        .iter()
        .find_map(|(from, _, msg)| match msg {
            Msg::HomeScatter { payload, reply: false, .. }
                if from == "phone" && serde_json::from_slice::<serde_json::Value>(payload).unwrap()["type"] == "JournalStore" =>
            {
                Some(payload.clone())
            }
            _ => None,
        })
        .unwrap();
    let before = serde_json::to_vec(&m.n("home").home_journal.saved).unwrap();
    m.n("home").on_home_scatter("phone", 900, false, &packet);
    let mut altered: serde_json::Value = serde_json::from_slice(&packet).unwrap();
    altered["expires"] = serde_json::json!(T0 + 20 * DAY);
    m.n("home").on_home_scatter("phone", 901, false, &serde_json::to_vec(&altered).unwrap());
    assert_eq!(serde_json::to_vec(&m.n("home").home_journal.saved).unwrap(), before);
    assert_eq!(m.n("home").home_journal.saved.placements.len(), 1);
}

#[test]
fn corrupt_home_authority_fails_closed_and_previous_format_upgrades_without_changing_keys() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("web", &t, &memory);
    n.create_identity("Robin").unwrap();
    n.commit().unwrap();
    let keys = memory.0.lock().unwrap().1["keys"].clone();
    memory.0.lock().unwrap().1.insert("format".into(), br#"{"version":5}"#.to_vec());
    let reopened = node("web", &t, &memory);
    assert_eq!(memory.0.lock().unwrap().1["keys"], keys);
    assert!(reopened.home_journal.saved.origin.is_empty());
    memory.0.lock().unwrap().1.insert(crate::home_journal::BLOB.into(), b"{".to_vec());
    let before = memory.0.lock().unwrap().clone();
    let clock = t.clone();
    assert!(
        matches!(Node::open(Box::new(memory.clone()),"web","https://example.com/",Box::new(move||clock.load(Ordering::SeqCst))),Err(e) if e.code=="storage-incompatible")
    );
    assert_eq!(*memory.0.lock().unwrap(), before);
}

#[derive(Clone)]
struct HomeFailing {
    memory: Mem,
    fail: Arc<std::sync::atomic::AtomicBool>,
}
impl Persist for HomeFailing {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        if self.fail.load(Ordering::SeqCst) { Err("storage unavailable".into()) } else { self.memory.apply(changes) }
    }
    fn load(&mut self) -> Result<Loaded, String> {
        self.memory.load()
    }
}

#[test]
fn failed_home_authority_persistence_releases_no_request_and_retries_the_same_transaction() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let clock = t.clone();
    let mut phone = Node::open(
        Box::new(HomeFailing { memory: memory.clone(), fail: fail.clone() }),
        "mobile",
        "https://example.com/",
        Box::new(move || clock.load(Ordering::SeqCst)),
    )
    .unwrap();
    phone.set_identity_bits(4);
    phone.create_identity("Robin").unwrap();
    let c = phone.create_community(draft("garden")).unwrap().id;
    let mut home = node("desktop", &t, &Mem::default());
    home.create_identity("Home").unwrap();
    let pairing = home.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    phone.pair(&pairing).unwrap();
    phone.commit().unwrap();
    phone.take_outbox();
    fail.store(true, Ordering::SeqCst);
    phone.journal_store("home", Id::from_hex(&c).unwrap(), b"request whose authority must be durable", 10, T0 + DAY).unwrap();
    assert!(phone.take_outbox().is_empty());
    assert!(!memory.0.lock().unwrap().1.contains_key(crate::home_journal::BLOB));
    assert!(phone.home_journal.dirty);
    fail.store(false, Ordering::SeqCst);
    assert!(!phone.take_outbox().is_empty());
    assert!(!phone.home_journal.dirty);
    let reopened = node("mobile", &t, &memory);
    assert_eq!(reopened.home_journal.saved.origin.len(), 1);
}

#[test]
fn home_journal_never_evicts_live_requests_and_keeps_the_inclusive_expiry_day() {
    let (mut m, c) = home_phone();
    let community = Id::from_hex(&c).unwrap();
    for i in 0..256u32 {
        let op = m.n("phone").journal_store("home", community, &i.to_le_bytes(), 10, T0 + DAY).unwrap();
        m.n("phone").drop_op(op);
    }
    assert!(m.n("phone").journal_store("home", community, b"over the bound", 10, T0 + DAY).is_err());
    assert_eq!(m.n("phone").home_journal.saved.origin.len(), 256);
    m.t.store((T0 + DAY).div_ceil(DAY) * DAY, Ordering::SeqCst);
    m.n("phone").journal_tick();
    assert_eq!(m.n("phone").home_journal.saved.origin.len(), 256, "holder storage days are inclusive");
    m.t.fetch_add(DAY, Ordering::SeqCst);
    m.n("phone").journal_tick();
    assert!(m.n("phone").home_journal.saved.origin.is_empty());
}

#[test]
fn an_uncertain_home_restart_returns_partial_cleanup_authority_and_whole_fallback() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "keep cleanup authority even when the home loses its in-flight operation");
    let pid = Id::from_hex(&p.id).unwrap();
    m.n("phone").scatter_tick();
    for (to, bytes) in m.n("phone").take_outbox() {
        if to == "home" {
            m.n("home").net_receive("phone", &bytes);
        }
    }
    assert_eq!(m.n("home").home_journal.saved.placements.len(), 1);
    let partial = m.n("home").home_journal.saved.placements.values().next().unwrap().capability.as_ref().unwrap().read().unwrap();
    assert_eq!(m.n("home").home_journal.saved.placements.values().next().unwrap().result, None);
    // Dispatch the actual PUTs, but lose their replies before the home can record completion.
    for (to, bytes) in m.n("home").take_outbox() {
        if m.nodes.contains_key(&to) {
            m.n(&to).net_receive("home", &bytes);
        }
    }
    m.cut.insert("home".into());
    m.pump();
    m.cut.remove("home");
    let disk = home_checkpoint(m.n("home"));
    m.hang_up("phone", "home");
    for f in ["f0", "f1", "f2"] {
        m.hang_up("home", f);
    }
    let mut restarted = node("desktop", &m.t, &disk);
    restarted.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    m.add("home", restarted, "203.0.113.50:7843");
    for f in ["f0", "f1", "f2"] {
        m.dial("home", f);
    }
    m.n("phone").net_connect("home", Role::Home);
    m.n("home").net_connect("phone", Role::Client);
    m.pump();
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()));
    assert!(m.n("phone").scatter.saved.whole.contains(&pid));
    assert!(m.n("phone").scatter.saved.drop_caps.contains_key(&crate::drop_share::cap_id(&partial)));
    assert_eq!(m.n("home").home_journal.saved.placements.len(), 1);
    m.n("phone").delete_own(&p.id).unwrap();
    m.run(180);
    let indexes = (0..partial.hashes.len() as u8).collect::<Vec<_>>();
    let op = m.n("b").scatter_probe(&partial, &indexes, &Default::default()).unwrap();
    assert!(matches!(m.wait("b",op,120),crate::scatter::ScatterOutcome::Probed(rows) if rows.iter().all(|r|r.kept==Some(false))));
    assert!(m.n("phone").home_journal.saved.origin.values().all(|r| r.released && r.content.is_empty() && r.capability.is_none()));
    assert!(m.n("home").home_journal.saved.placements.values().all(|p| p.capability.is_none()), "authenticated release clears home result keys");
}

#[test]
fn re_pairing_never_sends_an_old_queued_or_retried_placement_to_the_new_home() {
    let (mut m, c) = home_phone();
    post(m.n("phone"), &c, "keep this old pending content away from a new operator");
    m.n("phone").scatter_tick();
    assert_eq!(m.n("phone").home_journal.saved.origin.len(), 1);
    let scope = m.n("phone").journal_home();
    let holder = Id::from_hex(&m.eps["h0"]).unwrap();
    let op = m.n("phone").scatter_drop_targets(&[(holder, Id([3; 32]), [4; 32], scope)]).unwrap();
    m.n("phone").drop_op(op);
    let mut other = node("desktop", &m.t, &Mem::default());
    other.create_identity("Other operator").unwrap();
    let pairing = other.set_serving(Some("ws://127.0.0.1:7845")).unwrap().serving.unwrap().pairing;
    m.n("phone").pair(&pairing).unwrap();
    for _ in 0..3 {
        m.t.fetch_add(30, Ordering::SeqCst);
        m.n("phone").journal_tick();
        assert!(!m.n("phone").take_outbox().into_iter().any(|(_, bytes)| match Msg::decode(&bytes) {
            Ok(Msg::HomeScatter { reply: false, payload, .. }) =>
                ["JournalStore", "CleanupKeys"].iter().any(|kind| serde_json::from_slice::<serde_json::Value>(&payload).unwrap()["type"] == *kind),
            Ok(Msg::Piece { op: hoi_sync::PieceOp::Drop, .. }) => true,
            _ => false,
        }));
    }
}

#[test]
fn a_restored_unanswered_home_request_becomes_a_visible_unknown_deletion_gap() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "snapshot before receiving placement authority");
    let target = Id::from_hex(&p.id).unwrap();
    m.n("phone").scatter_tick();
    assert!(m.n("phone").journal_deletion_pending(target));
    let disk = home_checkpoint(m.n("phone"));
    let t = m.t.clone();
    let mut restored = Node::open_restored(Box::new(disk.clone()), "mobile", "https://example.com/", Box::new(move || t.load(Ordering::SeqCst))).unwrap();
    restored.set_identity_bits(4);
    assert!(!restored.journal_deletion_pending(target));
    assert!(restored.home_journal.saved.origin.values().all(|r| r.restored && r.unknown && r.content.is_empty()));
    restored.delete_own(&p.id).unwrap();
    restored.commit().unwrap();
    let job = restored.deletion_for_target(target).unwrap();
    assert!(restored.deletions.saved.jobs[&job].coverage_gaps.contains(&target));
    assert!(restored.deletions.saved.jobs[&job].unknown_until > restored.now());
    assert!(!restored.cleanup_readiness().ready);
    restored.scatter_tick();
    assert!(!restored.take_outbox().iter().any(|(_, bytes)| matches!(Msg::decode(bytes), Ok(Msg::HomeScatter { reply: false, .. }))));
    let reopened = node("mobile", &m.t, &disk);
    assert!(reopened.deletions.saved.jobs[&job].coverage_gaps.contains(&target));
}

#[test]
fn delegated_tokens_survive_content_expiry_until_the_inclusive_holder_day_ends() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "retain the home-issued tokens through the holder storage boundary");
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()));
    let manifest = m.n("phone").home_journal.saved.origin.values().next().unwrap().capability.as_ref().unwrap().read().unwrap();
    m.t.store(manifest.expires_at + 1, Ordering::SeqCst);
    m.n("phone").drop_share_tick();
    assert!(
        m.n("phone").scatter.saved.drop_caps.contains_key(&crate::drop_share::cap_id(&manifest)),
        "content expiry is earlier than the inclusive holder cleanup boundary"
    );
}

#[test]
fn home_deletion_authority_is_recovered_when_the_non_authority_scatter_cache_is_damaged() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "the durable authority journal outlives a damaged sightings cache");
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()));
    let original = m.n("phone").home_journal.saved.origin.values().next().unwrap().capability.clone().unwrap();
    let manifest = original.read().unwrap();
    let disk = home_checkpoint(m.n("phone"));
    disk.0.lock().unwrap().1.insert(crate::scatter::BLOB.into(), b"{".to_vec());
    let reopened = node("mobile", &m.t, &disk);
    assert_eq!(reopened.capability_for(&manifest).unwrap().tokens, original.tokens, "do not derive replacement tokens from this phone's unrelated secret");
    assert!(reopened.scatter.saved.kept.values().any(|k| k.records.contains(&Id::from_hex(&p.id).unwrap())));
}

#[test]
fn extending_a_delegated_generation_preserves_home_tokens_for_its_original_indexes() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "old home-issued pieces and a locally placed fresh index");
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()));
    let old = m.n("phone").home_journal.saved.origin.values().next().unwrap().capability.clone().unwrap();
    let mut extended = old.read().unwrap();
    assert!(extended.hashes.len() < extended.m as usize);
    let fresh = extended.hashes.len() as u8;
    extended.hashes.push([0x55; 16]);
    let holder = m.eps.values().filter_map(|h| Id::from_hex(h).ok()).find(|h| !extended.holders.contains(h)).unwrap();
    extended.holders.push(holder);
    extended.keys.push(extended.keys[0]);
    let cap = m.n("phone").capability_for(&extended).unwrap();
    assert_eq!(m.n("phone").journal_locator_home(&extended, 0), m.n("phone").journal_home());
    assert_eq!(m.n("phone").journal_locator_home(&extended, fresh as usize), None, "an extension cannot disclose a new holder to the original home");
    for (loc, token) in old.tokens {
        assert_eq!(cap.tokens[&loc], token, "an origin restart or repair cannot substitute its own secret for the home's authority");
    }
    let loc = hoi_crypto::scatter::locator(&extended.loc_secret, &extended.object, extended.generation, fresh);
    let secret = m.n("phone").drop_secret().unwrap();
    assert_eq!(cap.tokens[&Id(loc)], hoi_crypto::scatter::drop_token(&secret, &loc));
    assert_eq!(cap.read(), Some(extended));
}

#[test]
fn old_home_receipts_cannot_replace_extended_authority_or_restore_deleted_manifest_keys() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "keep foreign tail tokens while replaying the original home receipt");
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()));
    let (request, origin) = m.n("phone").home_journal.saved.origin.iter().next().map(|(id, r)| (*id, r.clone())).unwrap();
    let original = origin.capability.clone().unwrap();
    let mut extended = original.read().unwrap();
    let object = Id(extended.object);
    let index = extended.hashes.len() as u8;
    extended.hashes.push([0x55; 16]);
    let holder = m.eps.values().filter_map(|h| Id::from_hex(h).ok()).find(|h| !extended.holders.contains(h)).unwrap();
    extended.holders.push(holder);
    extended.keys.push(extended.keys[0]);
    let loc = Id(hoi_crypto::scatter::locator(&extended.loc_secret, &extended.object, extended.generation, index));
    let mut stronger = m.n("phone").capability_for(&extended).unwrap();
    // The tail was placed by another authorized device; this phone cannot derive its token.
    let foreign_token = [0x91; 32];
    stronger.tokens.insert(loc, foreign_token);
    m.n("phone").save_drop_cap(stronger);
    m.n("phone").save_drop_cap(original);
    let disk = home_checkpoint(m.n("phone"));
    let mut reopened = node("mobile", &m.t, &disk);
    let saved = &reopened.scatter.saved.drop_caps[&crate::drop_share::cap_id(&extended)];
    assert_eq!(saved.read(), Some(extended.clone()));
    assert_eq!(saved.tokens[&loc], foreign_token);
    reopened.delete_own(&p.id).unwrap();
    let job = reopened.deletion_for_target(Id::from_hex(&p.id).unwrap()).unwrap();
    assert!(reopened.deletions.saved.jobs[&job].targets.values().any(|t| t.home.is_some()));
    assert!(
        reopened.deletions.saved.jobs[&job].targets.values().filter(|t| t.locator == loc).all(|t| t.home.is_none()),
        "extension locators retain ordinary forwarding only"
    );
    assert!(
        reopened.deletions.saved.jobs[&job].targets.values().any(|t| t.locator == loc && t.token == Some(foreign_token)),
        "a shorter generation cannot consume the stronger cap before its tail is captured"
    );
    reopened.home_journal.saved.origin.insert(request, origin);
    reopened.home_journal.dirty = true;
    reopened.journal_recover_authority();
    assert!(!reopened.scatter.saved.kept.contains_key(&object), "late receipt replay never re-indexes a deleted object's decryption manifest");
    assert!(reopened.home_journal.saved.origin[&request].capability.is_none());
    reopened.commit().unwrap();
    let mut reopened = node("mobile", &m.t, &disk);
    assert!(!reopened.scatter.saved.kept.contains_key(&object));
    assert!(reopened.deletions.saved.jobs[&job].targets.values().any(|t| t.locator == loc && t.token == Some(foreign_token)));
    let authority: Vec<_> = reopened.deletions.saved.jobs[&job].targets.values().map(|t| (t.holder, t.locator, t.token)).collect();
    reopened.journal_restore_snapshot();
    assert!(reopened.deletions.saved.jobs[&job].targets.values().all(|t| t.home.is_none()), "an archive never revives an old operator's cleanup delegation");
    assert_eq!(
        reopened.deletions.saved.jobs[&job].targets.values().map(|t| (t.holder, t.locator, t.token)).collect::<Vec<_>>(),
        authority,
        "restore retains locator authority for ordinary routes"
    );
}

#[test]
fn paired_cleanup_key_lookup_verifies_holder_signatures_without_uptime_or_completion_claims() {
    let (mut m, _) = home_phone();
    let holder = Id::from_hex(&m.eps["h0"]).unwrap();
    m.n("phone").scatter.keys.remove(&holder);
    let ask = |m: &mut Mesh| {
        m.n("phone").home_cleanup_keys("home", vec![holder]);
        m.n("phone")
            .take_outbox()
            .into_iter()
            .find_map(|(_, bytes)| match Msg::decode(&bytes).unwrap() {
                Msg::HomeScatter { rid, payload, reply: false } if serde_json::from_slice::<serde_json::Value>(&payload).unwrap()["type"] == "CleanupKeys" => {
                    Some((rid, payload))
                }
                _ => None,
            })
            .unwrap()
    };
    let answer = |m: &mut Mesh, rid, request: &[u8]| {
        m.n("home").on_home_scatter("phone", rid, false, request);
        m.n("home")
            .take_outbox()
            .into_iter()
            .find_map(|(_, bytes)| match Msg::decode(&bytes).unwrap() {
                Msg::HomeScatter { rid: found, payload, reply: true } if found == rid => Some(payload),
                _ => None,
            })
            .unwrap()
    };
    let (rid, request) = ask(&mut m);
    let mut forged: serde_json::Value = serde_json::from_slice(&answer(&mut m, rid, &request)).unwrap();
    assert_eq!(forged["keys"].as_array().unwrap().len(), 1);
    forged["keys"][0]["sig"] = serde_json::json!(hoi_types::uri::b64(&[0; 64]));
    m.n("phone").on_home_scatter("home", rid, true, &serde_json::to_vec(&forged).unwrap());
    assert!(m.n("phone").key_of(&holder).is_none(), "a trusted home cannot forge a holder key");
    m.t.fetch_add(5, Ordering::SeqCst);
    let (rid, request) = ask(&mut m);
    let payload = answer(&mut m, rid, &request);
    m.n("phone").on_home_scatter("f0", rid, true, &payload);
    assert!(m.n("phone").key_of(&holder).is_none(), "only the authenticated home can answer its request");
    let mut forged_durability: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    forged_durability["keys"][0]["drop_journal"] = forged_durability["keys"][0]["sig"].clone();
    m.n("phone").on_home_scatter("home", rid, true, &serde_json::to_vec(&forged_durability).unwrap());
    assert!(m.n("phone").key_of(&holder).is_some());
    assert!(m.n("phone").scatter.keys[&holder].drop_journal.is_none(), "a holder-key signature cannot assert durable deletion");
    assert!(m.n("phone").deletion_progress().is_empty(), "key lookup never creates completion observations");
}

#[test]
fn scoped_cleanup_can_address_an_older_index_on_the_home_holder_itself() {
    use crate::scatter::{DropOutcome, DropResult, ScatterOutcome};
    use hoi_store::pieces::{MemPieces, PieceMeta};
    let (mut m, _) = home_phone();
    let endpoint = Id::from_hex(&m.eps["home"]).unwrap();
    m.n("phone").set_peer_meta(
        "home",
        PeerMeta { transport: "quic".into(), endpoint: Some(endpoint.hex()), addr: Some("203.0.113.50:7843".into()), ..Default::default() },
    );
    m.n("home").set_piece_store(Some(Box::new(MemPieces::new(1 << 20))));
    m.pump();
    let today = (m.t.load(Ordering::SeqCst) / DAY) as u32;
    let locator = Id([0x71; 32]);
    let token = [0x72; 32];
    let meta = PieceMeta { expiry_day: today + 1, drop_hash: hoi_crypto::scatter::drop_hash(&token), drop_key_id: [0; 16] };
    m.n("home").scatter.pieces.as_mut().unwrap().put(locator.as_bytes(), meta, b"older self-held index", today).unwrap();
    let scope = m.n("phone").journal_home();
    for (original_home, auth, expected) in
        [(Some(Id::ZERO), token, DropOutcome::Queued), (scope, [0; 32], DropOutcome::Refused), (scope, token, DropOutcome::LegacyRemoval)]
    {
        let op = m.n("phone").scatter_drop_targets(&[(endpoint, locator, auth, original_home)]).unwrap();
        assert_eq!(m.wait("phone", op, 120), ScatterOutcome::Dropped(vec![DropResult { index: 0, outcome: expected }]));
    }
    assert!(m.n("home").scatter.pieces.as_mut().unwrap().has(locator.as_bytes(), today).unwrap().is_none());
    assert!(
        m.log.iter().any(|(from, to, msg)| from == "phone" && to == "home" && matches!(msg, Msg::Piece { op: hoi_sync::PieceOp::Drop, to: None, .. })),
        "address the exact holder without a proxy loop"
    );
}
