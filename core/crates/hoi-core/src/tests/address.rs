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
            matches!(outcome, crate::scatter::ScatterOutcome::Probed(ref rows) if rows.len() == man.hashes.len() && rows.iter().all(|r| r.kept == Some(false))),
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
fn a_home_node_injects_what_its_phone_posts() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "from the phone, through the home node");
    let pid = Id::from_hex(&p.id).unwrap();
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()), "b has the phone's post");
    m.run(60);
    let pointers = m.n("phone").scatter.saved.own_pointers.get(&pid).cloned().expect("the home placed pieces and the phone signed pointers");
    assert!(!pointers.is_empty());
    assert!(!m.n("phone").scatter.saved.whole.contains(&pid));
    assert!(m.log.iter().any(|(f, _, msg)| f == "phone" && matches!(msg, Msg::HomeScatter { reply: false, .. })), "placement used the paired home link");
    assert!(m.log.iter().any(|(f, _, msg)| f == "home" && matches!(msg, Msg::Piece { op: PieceOp::Inject, .. })), "the home injected the pointers");
    assert!(!m.n("phone").post(&p.id).unwrap().checks.address_hidden, "the home knows its sender");
    assert_eq!(m.n("phone").post(&p.id).unwrap().checks.home_delivery.as_deref(), Some("pieces-injected"));
    for f in ["f0", "f1", "f2"] {
        assert!(m.n(f).store.get(&pid).is_none(), "the forwarders never received the whole post");
    }
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
fn home_placement_timeout_falls_back_and_receipts_never_claim_privacy_from_home() {
    let (mut m, c) = home_phone();
    let p = post(m.n("phone"), &c, "home goes away during placement");
    m.t.fetch_add(5, Ordering::SeqCst);
    m.n("phone").scatter_tick();
    assert_eq!(m.n("phone").post(&p.id).unwrap().checks.home_delivery.as_deref(), Some("placing"));
    m.cut.insert("home".into());
    m.run(130);
    assert!(m.n("phone").scatter.saved.whole.contains(&Id::from_hex(&p.id).unwrap()));
    assert!(!m.n("phone").post(&p.id).unwrap().checks.address_hidden);
    m.cut.remove("home");
    m.run(180);
    assert!(!m.n("phone").post(&p.id).unwrap().checks.address_hidden);
}

#[test]
fn home_placement_ignores_strangers_and_releases_a_wait_after_an_invalid_reply() {
    let (mut m, c) = home_phone();
    m.n("phone").scatter.placing = Some(Id::from_hex(&c).unwrap());
    let op = m.n("phone").scatter_store(b"authenticated response", 10, T0 + 30 * DAY).unwrap();
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
