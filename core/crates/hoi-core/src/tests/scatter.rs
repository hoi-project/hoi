//! Scattered delivery, phase S2 (SCATTER.md): public nodes hold pieces and answer sealed
//! requests, holder keys, vouches and mailboxes. Storing and loading objects through forwarders
//! (S3) is tests/forward.rs.
use super::*;
use crate::scatter::{KEY_GRACE, KEY_PERIOD};
use hoi_crypto::envelope::{self, DropAuth, Dropped, Kind, Reply, Request, Stored, XKey};
use hoi_crypto::scatter as sc;
use hoi_store::pieces::MemPieces;
use hoi_sync::{Msg, PieceOp};

const DAY: u64 = 86_400;

/// A public node at its own address in a network of its own, holding pieces.
fn holder(t: &Arc<AtomicU64>, i: usize) -> (Node, String, String) {
    let mut n = node("node", t, &Mem::default());
    n.set_node_name(Some(format!("holder {i}")));
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    n.set_carry_policy(crate::CarryPolicy::Any, 64);
    let endpoint = Keypair::from_secret(&n.transport_secret().unwrap()).public().hex();
    let addr = format!("45.{}.7.1:7843", 10 + i);
    n.set_transport_address(Some(endpoint.clone()), vec![format!("{endpoint}@{addr}")], vec![]);
    n.set_piece_store(Some(Box::new(MemPieces::new(1 << 30))));
    (n, endpoint, addr)
}

/// A person's desktop that keeps things in pieces or fetches them.
fn device(t: &Arc<AtomicU64>, addr: &str) -> (Node, String) {
    let mut n = node("desktop", t, &Mem::default());
    n.create_identity("Lena").unwrap();
    let endpoint = Keypair::from_secret(&n.transport_secret().unwrap()).public().hex();
    n.set_transport_address(Some(endpoint.clone()), vec![format!("{endpoint}@{addr}")], vec![]);
    n.scatter.bar = crate::scatter::HolderBar::none();
    (n, endpoint)
}

struct Holders {
    nodes: Vec<Node>,
    endpoints: Vec<String>,
    addrs: Vec<String>,
}

fn holders(t: &Arc<AtomicU64>, count: usize) -> Holders {
    let mut h = Holders { nodes: vec![], endpoints: vec![], addrs: vec![] };
    for i in 0..count {
        let (n, e, a) = holder(t, i);
        h.nodes.push(n);
        h.endpoints.push(e);
        h.addrs.push(a);
    }
    h
}

/// `dev` dials holder `i` over QUIC: it knows the holder as "h{i}", the holder knows it as `name`.
fn link(dev: &mut Node, name: &str, dev_ep: &str, dev_addr: &str, h: &mut Holders, i: usize) {
    let hn = format!("h{i}");
    dev.net_connect(&hn, Role::Peer);
    dev.set_peer_meta(&hn, PeerMeta { transport: "quic".into(), endpoint: Some(h.endpoints[i].clone()), addr: Some(h.addrs[i].clone()), ..Default::default() });
    h.nodes[i].net_connect(name, Role::Peer);
    h.nodes[i].set_peer_meta(
        name,
        PeerMeta { transport: "quic".into(), incoming: true, endpoint: Some(dev_ep.into()), addr: Some(dev_addr.into()), ..Default::default() },
    );
}

/// Delivers everything between the devices and the holders until nobody has more to say.
/// `cut` holders get nothing (as if their link were dead).
fn pump(devs: &mut [(&str, &mut Node)], h: &mut Holders, cut: &[usize]) {
    for _ in 0..200 {
        let mut quiet = true;
        for (name, d) in devs.iter_mut() {
            for (to, m) in d.take_outbox() {
                let Some(i) = to.strip_prefix('h').and_then(|i| i.parse::<usize>().ok()) else { continue };
                quiet = false;
                if !cut.contains(&i) {
                    h.nodes[i].net_receive(name, &m);
                }
            }
        }
        for (i, hn) in h.nodes.iter_mut().enumerate() {
            for (to, m) in hn.take_outbox() {
                if cut.contains(&i) {
                    continue;
                }
                if let Some((_, d)) = devs.iter_mut().find(|(n, _)| *n == to) {
                    quiet = false;
                    d.net_receive(&format!("h{i}"), &m);
                }
            }
        }
        if quiet {
            break;
        }
    }
}

/// Sends `request` sealed to holder 0 and returns its answer, opened.
fn ask(a: &mut Node, h: &mut Holders, request: &Request, reply: &XKey, rid: u32) -> Option<Reply> {
    let key = h.nodes[0].holder_hello().unwrap();
    let endpoint = Id::from_hex(&h.endpoints[0]).unwrap();
    let env = envelope::seal_request(&endpoint.0, &key.xk, request).unwrap();
    let op = match request.kind() {
        Kind::Put => PieceOp::Put,
        Kind::Get => PieceOp::Get,
        Kind::Has => PieceOp::Has,
        _ => PieceOp::Drop,
    };
    let _ = a;
    h.nodes[0].net_receive("a", &Msg::Piece { op, rid, to: None, env }.encode());
    let want = match request {
        Request::Get { want_len, .. } => *want_len as usize,
        _ => 0,
    };
    h.nodes[0].take_outbox().into_iter().find_map(|(_, m)| match Msg::decode(&m) {
        Ok(Msg::Piece { op, rid: r, env, .. }) if r == rid => {
            let kind = match op {
                PieceOp::Stored => Kind::Stored,
                PieceOp::Got => Kind::Got,
                PieceOp::Had => Kind::Had,
                _ => Kind::Dropped,
            };
            envelope::open_reply(kind, reply, request.loc(), want, &env).ok()
        }
        _ => None,
    })
}

#[test]
fn holders_keep_only_stamped_pieces_and_drop_only_for_their_creator() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut h = holders(&t, 1);
    let (mut a, a_ep) = device(&t, "198.51.100.10:7843");
    link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, 0);
    pump(&mut [("a", &mut a)], &mut h, &[]);
    let endpoint = Id::from_hex(&h.endpoints[0]).unwrap();
    let reply = XKey::generate().unwrap();
    let loc = [7u8; 32];
    let piece = vec![9u8; 300];
    let day = (T0 / DAY) as u32 + 10;
    let token = sc::drop_token(&[5; 32], &loc);
    let put = |stamp: u64, expiry_day: u32| Request::Put {
        loc,
        reply: reply.public(),
        expiry_day,
        drop_hash: sc::drop_hash(&token),
        drop_key_id: [0; 16],
        stamp,
        piece: piece.clone(),
    };
    // a stamp made for another holder is worth nothing here
    let elsewhere = sc::mine_stamp(&[1; 32], &loc, piece.len(), day);
    if !sc::stamp_ok(&endpoint.0, &loc, piece.len(), day, elsewhere) {
        assert_eq!(ask(&mut a, &mut h, &put(elsewhere, day), &reply, 1), Some(Reply::Stored(Stored::BadStamp)));
    }
    // a day in the past, or more than 400 days ahead, is refused
    let past = (T0 / DAY) as u32 - 1;
    assert_eq!(ask(&mut a, &mut h, &put(sc::mine_stamp(&endpoint.0, &loc, piece.len(), past), past), &reply, 2), Some(Reply::Stored(Stored::Refused)));
    let far = (T0 / DAY) as u32 + 401;
    assert_eq!(ask(&mut a, &mut h, &put(sc::mine_stamp(&endpoint.0, &loc, piece.len(), far), far), &reply, 3), Some(Reply::Stored(Stored::Refused)));
    // a good put is kept, and found by its exact locator only
    let stamp = sc::mine_stamp(&endpoint.0, &loc, piece.len(), day);
    assert_eq!(ask(&mut a, &mut h, &put(stamp, day), &reply, 4), Some(Reply::Stored(Stored::Ok)));
    assert_eq!(h.nodes[0].piece_stats().unwrap().count, 1);
    let get = |loc: [u8; 32]| Request::Get { loc, reply: reply.public(), want_len: 300 };
    assert_eq!(ask(&mut a, &mut h, &get(loc), &reply, 5), Some(Reply::Got(Some(piece.clone()))));
    assert_eq!(ask(&mut a, &mut h, &get([8; 32]), &reply, 6), Some(Reply::Got(None)));
    // a drop needs the creator's token: anyone else's is refused, the right one drops it
    let drop = |auth: DropAuth| Request::Drop { loc, reply: reply.public(), auth };
    assert_eq!(ask(&mut a, &mut h, &drop(DropAuth::Token(sc::drop_token(&[6; 32], &loc))), &reply, 7), Some(Reply::Dropped(Dropped::Refused)));
    let k = Keypair::from_secret(&[3; 32]);
    let signed = DropAuth::Signed { key: k.public().0, sig: k.sign(&sc::drop_statement(&loc)).0 };
    assert_eq!(ask(&mut a, &mut h, &drop(signed), &reply, 8), Some(Reply::Dropped(Dropped::Refused)), "a drop key the piece wasn't stored with");
    assert_eq!(ask(&mut a, &mut h, &drop(DropAuth::Token(token)), &reply, 9), Some(Reply::Dropped(Dropped::Dropped)));
    assert_eq!(ask(&mut a, &mut h, &get(loc), &reply, 10), Some(Reply::Got(None)));
    assert_eq!(ask(&mut a, &mut h, &drop(DropAuth::Token(token)), &reply, 11), Some(Reply::Dropped(Dropped::Unknown)));
    // an envelope sealed to another key gets no answer at all
    let other = XKey::generate().unwrap();
    let env = envelope::seal_request(&endpoint.0, &other.public(), &get(loc)).unwrap();
    h.nodes[0].net_receive("a", &Msg::Piece { op: PieceOp::Get, rid: 12, to: None, env }.encode());
    assert!(h.nodes[0].take_outbox().is_empty());
}

#[test]
fn holder_keys_change_each_period_and_old_ones_open_for_a_week() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut h = holders(&t, 1);
    let (mut a, a_ep) = device(&t, "198.51.100.10:7843");
    link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, 0);
    pump(&mut [("a", &mut a)], &mut h, &[]);
    let old = h.nodes[0].holder_hello().unwrap();
    // the device took the key from the hello: signed by the endpoint the transport authenticated
    assert_eq!(a.peers["h0"].scatter.holder, Some(old));
    let endpoint = Id::from_hex(&h.endpoints[0]).unwrap();
    let reply = XKey::generate().unwrap();
    let has = Request::Has { loc: [1; 32], reply: reply.public() };
    let send = |h: &mut Holders, xk: &[u8; 32], rid: u32| {
        let env = envelope::seal_request(&endpoint.0, xk, &has).unwrap();
        h.nodes[0].net_receive("a", &Msg::Piece { op: PieceOp::Has, rid, to: None, env }.encode());
        h.nodes[0].take_outbox().into_iter().any(|(_, m)| matches!(Msg::decode(&m), Ok(Msg::Piece { rid: r, .. }) if r == rid))
    };
    assert!(send(&mut h, &old.xk, 1));
    // into the next period: a new key, and the old one still opens during the grace week
    let next_period = (T0 / KEY_PERIOD + 1) * KEY_PERIOD;
    t.store(next_period + 1, Ordering::SeqCst);
    let new = h.nodes[0].holder_hello().unwrap();
    assert_ne!(new.xk, old.xk);
    assert!(new.expires_at > old.expires_at);
    assert!(send(&mut h, &new.xk, 2));
    assert!(send(&mut h, &old.xk, 3));
    // after the grace week the old key still opens asks: a manifest lists the key a holder had
    // when it took its piece, and readers seal to it (S6)
    t.store(next_period + KEY_GRACE + 1, Ordering::SeqCst);
    assert!(send(&mut h, &old.xk, 4));
    // but a put sealed to it is not opened: pieces go to fresh keys only
    let day = (next_period / 86_400) as u32 + 2;
    let loc = [3u8; 32];
    let piece = vec![7u8; 100];
    let put = Request::Put {
        loc,
        reply: XKey::generate().unwrap().public(),
        expiry_day: day,
        drop_hash: [0; 32],
        drop_key_id: [0; 16],
        stamp: hoi_crypto::scatter::mine_stamp(&endpoint.0, &loc, piece.len(), day),
        piece,
    };
    let env = envelope::seal_request(&endpoint.0, &old.xk, &put).unwrap();
    h.nodes[0].net_receive("a", &Msg::Piece { op: PieceOp::Put, rid: 5, to: None, env }.encode());
    assert!(
        !h.nodes[0].take_outbox().into_iter().any(|(_, m)| matches!(Msg::decode(&m), Ok(Msg::Piece { rid: 5, .. }))),
        "a put to an old key goes unanswered"
    );
}

#[test]
fn a_holder_key_counts_only_from_the_endpoint_that_signed_it() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut h = holders(&t, 1);
    let (mut a, a_ep) = device(&t, "198.51.100.10:7843");
    // the transport authenticated someone else: the holder key in the hello is not theirs
    let real = h.endpoints[0].clone();
    h.endpoints[0] = Keypair::from_secret(&[44; 32]).public().hex();
    link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, 0);
    h.endpoints[0] = real;
    pump(&mut [("a", &mut a)], &mut h, &[]);
    assert!(a.peers["h0"].scatter.holder.is_none());
    assert_eq!(a.scatter_places(), 0);
    // a node that stops holding says so in a fresh hello
    let t = Arc::new(AtomicU64::new(T0));
    let mut h = holders(&t, 1);
    let (mut a, a_ep) = device(&t, "198.51.100.10:7843");
    link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, 0);
    pump(&mut [("a", &mut a)], &mut h, &[]);
    assert!(a.peers["h0"].scatter.holder.is_some());
    h.nodes[0].set_piece_store(None);
    pump(&mut [("a", &mut a)], &mut h, &[]);
    assert!(a.peers["h0"].scatter.holder.is_none());
}

#[test]
fn a_community_asks_for_more_than_a_tenth_by_the_usual_proposal() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("Lena").unwrap();
    let c = n.create_community(draft("scattered")).unwrap();
    let limits = n.community(&c.id).unwrap().limits;
    assert_eq!((limits.scatter_resist, limits.scatter_open), (10, true), "the floor, and posts in pieces: a new community's default");
    // a founder alone decides at once
    match n.set_limits(&c.id, &LimitsDto { scatter_resist: 15, ..limits.clone() }).unwrap() {
        LedgerResult::Published { entry } => assert_eq!(entry.action, "limits"),
        _ => panic!("threshold 1 publishes at once"),
    }
    n.commit().unwrap();
    assert_eq!(n.community(&c.id).unwrap().limits.scatter_resist, 15);
    // below the floor, or between levels, is refused before anything is signed
    for bad in [5u8, 12, 25] {
        assert!(n.set_limits(&c.id, &LimitsDto { scatter_resist: bad, ..n.community(&c.id).unwrap().limits }).is_err(), "{bad}");
    }
}

#[test]
fn a_holder_counts_only_after_staying_online_for_days() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut h = holders(&t, 2);
    let (mut a, a_ep) = device(&t, "198.51.100.10:7843");
    a.scatter.bar = crate::scatter::HolderBar::default();
    link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, 0);
    link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, 1);
    pump(&mut [("a", &mut a)], &mut h, &[]);
    let qualified = |a: &Node| a.linked_holders(None).iter().filter(|x| x.qualified).count();
    a.scatter_tick();
    assert_eq!(a.linked_holders(None).len(), 2, "both are linked");
    assert_eq!(qualified(&a), 0, "a holder just met holds nothing yet");
    // three days, sampled each hour this device runs: holder 0 is always there, holder 1 only
    // one hour in two
    for hour in 0..72u64 {
        t.fetch_add(3600, Ordering::SeqCst);
        if hour % 2 == 0 {
            a.net_disconnect("h1");
        } else {
            link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, 1);
            pump(&mut [("a", &mut a)], &mut h, &[]);
        }
        a.scatter_tick();
        pump(&mut [("a", &mut a)], &mut h, &[]);
        if hour == 24 {
            assert_eq!(qualified(&a), 0, "a day is not enough");
        }
    }
    link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, 1);
    pump(&mut [("a", &mut a)], &mut h, &[]);
    let linked = a.linked_holders(None);
    let by_ep = |i: usize| linked.iter().find(|x| x.endpoint.hex() == h.endpoints[i]).map(|x| x.qualified);
    assert_eq!(by_ep(0), Some(true), "steady for three days");
    assert_eq!(by_ep(1), Some(false), "online half the time is not enough");
    // sightings survive a restart: they are kept in their own blob
    a.commit().unwrap();
    assert!(a.scatter.saved.holders.len() == 2);
}

/// Runs every device's scatter round each `step` seconds for `secs`, delivering everything.
fn run(t: &Arc<AtomicU64>, devs: &mut [(&str, &mut Node)], h: &mut Holders, secs: u64) {
    let step = 5;
    for _ in 0..secs.div_ceil(step) {
        t.fetch_add(step, Ordering::SeqCst);
        for (_, d) in devs.iter_mut() {
            d.scatter_tick();
        }
        pump(devs, h, &[]);
    }
}

#[test]
fn a_private_group_syncs_through_its_mailboxes_without_a_relay() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut h = holders(&t, 6);
    let (mut a, a_ep) = device(&t, "198.51.100.10:7843");
    let (mut b, b_ep) = device(&t, "192.0.2.20:7843");
    for i in 0..6 {
        link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, i);
        link(&mut b, "b", &b_ep, "192.0.2.20:7843", &mut h, i);
    }
    pump(&mut [("a", &mut a), ("b", &mut b)], &mut h, &[]);
    let c = a.create_private_community(draft("secret garden")).unwrap().id;
    assert_eq!(a.private_group(&c).unwrap().transport, "waiting");
    // the owner's device picks the mailboxes by itself, one per place
    run(&t, &mut [("a", &mut a)], &mut h, 10);
    let group = a.private_group(&c).unwrap();
    assert_eq!(group.transport, "mailboxes");
    assert_eq!(group.mailboxes.len(), crate::mailbox::MAILBOXES);
    assert!(group.mailboxes.iter().all(|m| m.linked));
    assert_eq!(group.pending, 0, "a group of one has nobody to tell");

    // an invitation carries the list: the newcomer reads the same mailboxes at once
    let request = b.private_join_request().unwrap();
    let invitation = a.private_invite(&c, &request).unwrap();
    b.private_join(&invitation).unwrap();
    assert_eq!(b.private_group(&c).unwrap().transport, "mailboxes");
    assert_eq!(b.private_group(&c).unwrap().mailboxes.len(), crate::mailbox::MAILBOXES);
    run(&t, &mut [("a", &mut a), ("b", &mut b)], &mut h, 30);
    assert_eq!(a.private_group(&c).unwrap().pending, 0, "the addition is in the mailboxes");

    // posts go both ways, through slots only members can find
    let first = post(&mut a, &c, "hello from the owner");
    run(&t, &mut [("a", &mut a), ("b", &mut b)], &mut h, 60);
    assert_eq!(b.post(&first.id).unwrap().summary.title, "hello from the owner");
    assert_eq!(a.private_group(&c).unwrap().pending, 0);
    let reply = post(&mut b, &c, "hello back");
    run(&t, &mut [("a", &mut a), ("b", &mut b)], &mut h, 60);
    assert_eq!(a.post(&reply.id).unwrap().summary.title, "hello back");

    // a quiet group is checked less often, and a new post still arrives within two minutes
    run(&t, &mut [("a", &mut a), ("b", &mut b)], &mut h, 600);
    let later = post(&mut a, &c, "after a quiet while");
    run(&t, &mut [("a", &mut a), ("b", &mut b)], &mut h, 150);
    assert_eq!(b.post(&later.id).unwrap().summary.title, "after a quiet while");

    // a removal goes to the slots of the epoch it ends; what follows is out of reach
    let removed = b.me().unwrap().device;
    a.private_remove(&c, &removed).unwrap();
    // b had been quiet a while, so it checks within two minutes, not at once
    run(&t, &mut [("a", &mut a), ("b", &mut b)], &mut h, 150);
    assert!(!b.private_group(&c).unwrap().active, "the removed member read its removal");
    let secret = post(&mut a, &c, "after removing bob");
    run(&t, &mut [("a", &mut a), ("b", &mut b)], &mut h, 60);
    assert!(b.post(&secret.id).is_err());
    assert_eq!(a.private_group(&c).unwrap().pending, 0, "a group of one again acknowledges at once");
}

#[test]
fn an_owner_still_choosing_mailboxes_dials_holders_it_met_before() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut h = holders(&t, 3);
    let (mut a, a_ep) = device(&t, "198.51.100.10:7843");
    for i in 0..3 {
        link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, i);
    }
    pump(&mut [("a", &mut a)], &mut h, &[]);
    a.scatter_tick();
    for i in 0..3 {
        a.net_disconnect(&format!("h{i}"));
    }
    assert_eq!(a.mailbox_links(), 0, "no private group, nothing to dial for");
    assert!(!a.bootstrap_hints().iter().any(|(r, _)| r.ends_with(&h.addrs[0])));
    let c = a.create_private_community(draft("quiet garden")).unwrap().id;
    t.fetch_add(60, Ordering::SeqCst);
    a.scatter_tick();
    assert_eq!(a.private_group(&c).unwrap().transport, "waiting", "no holder linked, nothing chosen yet");
    assert_eq!(a.mailbox_links(), crate::mailbox::MAILBOXES);
    let hints = a.bootstrap_hints();
    for i in 0..3 {
        assert!(hints.iter().any(|(r, _)| r == &format!("{}@{}", h.endpoints[i], h.addrs[i])), "holder {i} is dialed again");
    }
    // linked again, the owner's device chooses at once
    for i in 0..3 {
        link(&mut a, "a", &a_ep, "198.51.100.10:7843", &mut h, i);
    }
    pump(&mut [("a", &mut a)], &mut h, &[]);
    run(&t, &mut [("a", &mut a)], &mut h, 40);
    assert_eq!(a.private_group(&c).unwrap().transport, "mailboxes");
    assert_eq!(a.private_group(&c).unwrap().mailboxes.len(), 3, "as many as there are places");
    assert_eq!(a.mailbox_links(), 3);
}

/// Delivers everything between `nodes` until nobody has more to say: each node knows every other
/// by the other's name here, so a message to "h1" goes to the node named "h1", from its sender.
fn deliver(nodes: &mut [(&str, &mut Node)]) {
    for _ in 0..200 {
        let mut quiet = true;
        for i in 0..nodes.len() {
            let from = nodes[i].0.to_string();
            for (to, m) in nodes[i].1.take_outbox() {
                if let Some(j) = nodes.iter().position(|(n, _)| *n == to) {
                    quiet = false;
                    nodes[j].1.net_receive(&from, &m);
                }
            }
        }
        if quiet {
            break;
        }
    }
}

/// `from` dials `to` over QUIC; each knows the other by its name in [`deliver`].
fn dial(from: (&str, &mut Node, &str, &str), to: (&str, &mut Node, &str, &str)) {
    let (fname, f, fep, faddr) = from;
    let (tname, t, tep, taddr) = to;
    f.net_connect(tname, Role::Peer);
    f.set_peer_meta(tname, PeerMeta { transport: "quic".into(), endpoint: Some(tep.into()), addr: Some(taddr.into()), ..Default::default() });
    t.net_connect(fname, Role::Peer);
    t.set_peer_meta(fname, PeerMeta { transport: "quic".into(), incoming: true, endpoint: Some(fep.into()), addr: Some(faddr.into()), ..Default::default() });
}

#[test]
fn a_holder_two_places_vouch_for_counts_at_once_on_a_new_device() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut h = holders(&t, 4);
    let (e, ad) = (h.endpoints.clone(), h.addrs.clone());
    let [h0, h1, h2, h3] = &mut h.nodes[..] else { unreachable!() };
    // h1 and h2 keep links to h0 for three days, and see it steady at their hourly samples
    dial(("h1", h1, &e[1], &ad[1]), ("h0", h0, &e[0], &ad[0]));
    dial(("h2", h2, &e[2], &ad[2]), ("h0", h0, &e[0], &ad[0]));
    dial(("h1", h1, &e[1], &ad[1]), ("h3", h3, &e[3], &ad[3]));
    let mut all = [("h0", &mut *h0), ("h1", &mut *h1), ("h2", &mut *h2), ("h3", &mut *h3)];
    deliver(&mut all);
    for _ in 0..72 {
        t.fetch_add(3600, Ordering::SeqCst);
        for (_, n) in all.iter_mut() {
            n.scatter_tick();
        }
        deliver(&mut all);
    }
    let vouched = |n: &Node| n.vouches().iter().map(|v| v.endpoint.hex()).collect::<Vec<_>>();
    assert!(vouched(h1).contains(&e[0]) && vouched(h2).contains(&e[0]));
    assert!(!vouched(h2).contains(&e[3]), "only holders it saw itself");
    assert!(h1.vouches().iter().all(|v| v.up >= 80 && v.up % 5 == 0 && v.days == 2), "{:?}", h1.vouches());

    // a device that never saw any of them dials h0, h1 and h2
    let (mut a, a_ep) = device(&t, "198.51.100.10:7843");
    a.scatter.bar = crate::scatter::HolderBar::default();
    let a_addr = "198.51.100.10:7843";
    for (i, n) in [(0, &mut *h0), (1, &mut *h1), (2, &mut *h2)] {
        dial(("a", &mut a, &a_ep, a_addr), (["h0", "h1", "h2"][i], n, &e[i], &ad[i]));
    }
    deliver(&mut [("a", &mut a), ("h0", &mut *h0), ("h1", &mut *h1), ("h2", &mut *h2)]);
    let qualified = |a: &Node| a.linked_holders(None).into_iter().filter(|x| x.qualified).map(|x| x.endpoint.hex()).collect::<Vec<_>>();
    // h0: vouched for by h1 and h2, two places of their own, both away from h0's
    assert_eq!(qualified(&a), vec![e[0].clone()], "and only h0: h1 and h2 have one voucher each (h0)");
    let h0_cand = a.linked_holders(None).into_iter().find(|x| x.endpoint.hex() == e[0]).unwrap().candidate;
    let first_introducer = |ep: &str| u64::from_le_bytes(Id::from_hex(ep).unwrap().0[..8].try_into().unwrap());
    assert!([first_introducer(&e[1]), first_introducer(&e[2])].contains(&h0_cand.introducer), "counted against an introducer, not itself");

    // seen from here, h2 is in h1's network: one place vouching is not enough. A real public
    // address: places are counted by network, and documentation ranges are not public.
    a.set_peer_meta("h2", PeerMeta { transport: "quic".into(), endpoint: Some(e[2].clone()), addr: Some("45.11.7.99:7843".into()), ..Default::default() });
    assert!(qualified(&a).is_empty());
    a.set_peer_meta("h2", PeerMeta { transport: "quic".into(), endpoint: Some(e[2].clone()), addr: Some(ad[2].clone()), ..Default::default() });
    assert_eq!(qualified(&a), vec![e[0].clone()]);

    // a vouch from a peer that dialed in counts for nothing, and neither does a node's vouch for
    // itself or one that reports less than this device's own bar
    a.net_disconnect("h2");
    assert!(qualified(&a).is_empty());
    let mut s = node("node", &t, &Mem::default());
    let s_ep = Keypair::from_secret(&s.transport_secret().unwrap()).public().hex();
    dial(("s", &mut s, &s_ep, "203.0.113.9:7843"), ("a", &mut a, &a_ep, a_addr));
    deliver(&mut [("a", &mut a), ("s", &mut s)]);
    let strong = hoi_sync::Vouch::bare(Id::from_hex(&e[0]).unwrap(), 100, 30);
    assert!(a.net_receive("s", &Msg::Nodes { routes: vec![], holders: vec![strong.clone()] }.encode()));
    assert!(qualified(&a).is_empty(), "a stranger that dialed in vouches for nobody");
    let weak = hoi_sync::Vouch::bare(Id::from_hex(&e[0]).unwrap(), 75, 30);
    dial(("a", &mut a, &a_ep, a_addr), ("h3", &mut *h3, &e[3], &ad[3]));
    deliver(&mut [("a", &mut a), ("h3", &mut *h3)]);
    assert!(a.net_receive("h3", &Msg::Nodes { routes: vec![], holders: vec![weak] }.encode()));
    assert!(qualified(&a).is_empty(), "online three hours in four is below the bar");
    assert!(a.net_receive("h3", &Msg::Nodes { routes: vec![], holders: vec![strong.clone()] }.encode()));
    assert_eq!(qualified(&a), vec![e[0].clone()], "h1 and h3: two places again");
}

#[test]
fn a_browser_checks_a_holder_key_by_the_certificate_it_pinned() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut hn, ep, addr) = holder(&t, 0);
    let cert = [9u8; 32];
    let route = format!("rtc:{}@45.10.7.1:7844", hoi_types::uri::b64(&cert));
    hn.set_rtc_routes(vec![route.clone()]);
    let key = hn.holder_hello().unwrap();
    assert_eq!(key.rtc.map(|r| (r.endpoint.hex(), r.cert)), Some((ep.clone(), cert)));
    let browser = |dialed: &str| {
        let mut b = node("web", &t, &Mem::default());
        b.net_connect("h0", Role::Peer);
        b.set_peer_meta("h0", PeerMeta { transport: "webrtc".into(), dialed: vec![dialed.to_string()], ..Default::default() });
        b
    };
    let link = |b: &mut Node, hn: &mut Node| {
        hn.net_connect("w", Role::Peer);
        hn.set_peer_meta("w", PeerMeta { transport: "webrtc".into(), incoming: true, addr: Some("192.0.2.30:50000".into()), ..Default::default() });
        deliver(&mut [("w", b), ("h0", hn)]);
    };
    // it pinned the holder's certificate: the binding names the endpoint, the key checks out
    let mut b = browser(&route);
    link(&mut b, &mut hn);
    let linked = b.linked_holders(None);
    assert_eq!(linked.iter().map(|x| x.endpoint.hex()).collect::<Vec<_>>(), vec![ep.clone()]);
    assert_eq!(b.peers["h0"].meta.endpoint.as_deref(), Some(ep.as_str()));
    let _ = addr;
    // a binding for a certificate it didn't pin (someone passing on another holder's hello)
    // makes no endpoint and no holder
    let mut b = browser(&format!("rtc:{}@45.10.7.1:7844", hoi_types::uri::b64(&[8u8; 32])));
    link(&mut b, &mut hn);
    assert!(b.linked_holders(None).is_empty());
    assert!(b.peers["h0"].meta.endpoint.is_none());
}

#[test]
fn an_idle_member_listens_for_the_bell_instead_of_checking_everyone() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut h = holders(&t, 4);
    let names = ["a", "m1", "m2", "m3", "m4", "m5", "m6", "m7"];
    let mut devs: Vec<Node> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let addr = format!("198.51.{}.10:7843", 100 + i);
        let (mut d, ep) = device(&t, &addr);
        for j in 0..4 {
            link(&mut d, name, &ep, &addr, &mut h, j);
        }
        devs.push(d);
    }
    let mut all: Vec<(&str, &mut Node)> = names.iter().copied().zip(devs.iter_mut()).collect();
    pump(&mut all, &mut h, &[]);
    let c = all[0].1.create_private_community(draft("big garden")).unwrap().id;
    run(&t, &mut all[..1], &mut h, 40);
    for i in 1..names.len() {
        let request = all[i].1.private_join_request().unwrap();
        let invitation = all[0].1.private_invite(&c, &request).unwrap();
        all[i].1.private_join(&invitation).unwrap();
        run(&t, &mut all, &mut h, 30);
    }
    run(&t, &mut all, &mut h, 300);
    assert!(all.iter().all(|(_, d)| d.private_group(&c).unwrap().members.len() == 8), "everyone read every addition");

    // ten quiet minutes: a member looks at the bell, and at everyone once, in case a ring was lost
    for (_, d) in all.iter_mut() {
        d.scatter.mail.sent.clear();
    }
    run(&t, &mut all, &mut h, 600);
    let sent = |d: &Node, what: &str| d.scatter.mail.sent.get(what).copied().unwrap_or(0);
    let m = &all[3].1;
    assert!(sent(m, "bell") <= 12, "the bell, backing off to every two minutes: {:?}", m.scatter.mail.sent);
    assert!(sent(m, "probe") <= 2 * 7, "each of the seven others once or twice: {:?}", m.scatter.mail.sent);
    // probing each member's next slot on its own would have taken about nine probes each
    assert!(sent(m, "bell") + sent(m, "probe") < 7 * 9 / 2);

    // a post in the quiet group rings the bell, and every member has it within two minutes
    let quiet = post(all[5].1, &c, "after ten quiet minutes");
    for (_, d) in all.iter_mut() {
        d.scatter.mail.sent.clear();
    }
    run(&t, &mut all, &mut h, 150);
    for (name, d) in all.iter_mut() {
        assert_eq!(d.post(&quiet.id).unwrap().summary.title, "after ten quiet minutes", "{name}");
    }
    assert!(sent(all[5].1, "ring") >= 2, "the writer rang on its mailboxes: {:?}", all[5].1.scatter.mail.sent);

    // a message whose rings never went out (an older app, a mailbox that lost them) is found at
    // the next look at every member, within ten minutes
    let unrung = post(all[6].1, &c, "nobody rang for this");
    for _ in 0..(FULL_SWEEP_SECS / 5 + 4) {
        t.fetch_add(5, Ordering::SeqCst);
        for (_, d) in all.iter_mut() {
            d.scatter_tick();
            d.scatter.mail.forget_rings();
        }
        pump(&mut all, &mut h, &[]);
    }
    for (name, d) in all.iter_mut() {
        assert_eq!(d.post(&unrung.id).unwrap().summary.title, "nobody rang for this", "{name}");
    }
}

const FULL_SWEEP_SECS: u64 = 600;

/// h1 keeps a link to h0 for three days and vouches for it; `a`, new and with the real bar, links
/// to both and owns a community where `n` members were vouched in a day ago.
fn rostered(n: usize) -> (Arc<AtomicU64>, Node, Holders, String, Vec<Actor>) {
    let t = Arc::new(AtomicU64::new(T0));
    let mut h = holders(&t, 2);
    let (e, ad) = (h.endpoints.clone(), h.addrs.clone());
    {
        let [h0, h1] = &mut h.nodes[..] else { unreachable!() };
        dial(("h1", h1, &e[1], &ad[1]), ("h0", h0, &e[0], &ad[0]));
        let mut all = [("h0", &mut *h0), ("h1", &mut *h1)];
        deliver(&mut all);
        for _ in 0..72 {
            t.fetch_add(3600, Ordering::SeqCst);
            for (_, n) in all.iter_mut() {
                n.scatter_tick();
            }
            deliver(&mut all);
        }
    }
    let (mut a, a_ep) = device(&t, "198.51.100.10:7843");
    a.scatter.bar = crate::scatter::HolderBar::default();
    let c = a.create_community(draft("a garden")).unwrap().id;
    let people = members(&mut a, &c, n);
    t.fetch_add(2 * 86_400, Ordering::SeqCst);
    a.settle_dirty();
    {
        let [h0, h1] = &mut h.nodes[..] else { unreachable!() };
        for (i, hn) in [(0, &mut *h0), (1, &mut *h1)] {
            dial(("a", &mut a, &a_ep, "198.51.100.10:7843"), (["h0", "h1"][i], hn, &e[i], &ad[i]));
        }
        deliver(&mut [("a", &mut a), ("h0", &mut *h0), ("h1", &mut *h1)]);
    }
    (t, a, h, c, people)
}

fn roster(by: &Actor, a: &mut Node, c: &str, holders: Vec<Id>) -> Result<Id, hoi_validate::Reject> {
    let now = a.now();
    let mut r = rec(Id::from_hex(c).unwrap(), None, Body::Roster { holders }, now);
    r.expires_at = now + 2 * 86_400;
    by.send(a, r)
}

fn counts(a: &Node, holder: &str, c: Option<&str>) -> bool {
    a.linked_holders(c.map(|c| Id::from_hex(c).unwrap())).into_iter().any(|x| x.endpoint.hex() == holder && x.qualified)
}

#[test]
fn a_holder_one_link_and_the_community_s_rosters_vouch_for_counts_for_that_community_only() {
    let (_t, mut a, h, c, people) = rostered(2);
    let h0 = Id::from_hex(&h.endpoints[0]).unwrap();
    assert!(!counts(&a, &h.endpoints[0], None), "one place vouching is not enough");
    assert!(!counts(&a, &h.endpoints[0], Some(&c)));
    for p in &people {
        roster(p, &mut a, &c, vec![h0]).unwrap();
    }
    assert!(a.roster_holders(Id::from_hex(&c).unwrap()).contains(&h0));
    assert!(counts(&a, &h.endpoints[0], Some(&c)), "h1 and the community's rosters: two places");
    let cand = a.linked_holders(Some(Id::from_hex(&c).unwrap())).into_iter().find(|x| x.endpoint == h0).unwrap().candidate;
    assert_eq!(cand.introducer, crate::scatter::ROSTER_INTRODUCER, "counted against the rosters, which take ⌈n/4⌉ at most");
    assert!(!counts(&a, &h.endpoints[0], None), "only for that community's objects");
    let other = a.create_community(draft("another garden")).unwrap().id;
    assert!(!counts(&a, &h.endpoints[0], Some(&other)));
    // a roster alone never makes a holder count: a link must vouch for it too
    a.net_disconnect("h1");
    assert!(!counts(&a, &h.endpoints[0], Some(&c)));
}

#[test]
fn rosters_from_one_person_or_without_quorum_voice_count_for_nothing() {
    // one person, however many devices
    let (_t, mut a, h, c, people) = rostered(2);
    let h0 = Id::from_hex(&h.endpoints[0]).unwrap();
    roster(&people[0], &mut a, &c, vec![h0]).unwrap();
    assert!(!counts(&a, &h.endpoints[0], Some(&c)), "one person's roster is one voice, not two places");
    // four members share the owner's voice: two of them reach no quorum
    let (_t, mut a, h, c, people) = rostered(4);
    let h0 = Id::from_hex(&h.endpoints[0]).unwrap();
    for p in &people[..2] {
        roster(p, &mut a, &c, vec![h0]).unwrap();
    }
    assert!(!counts(&a, &h.endpoints[0], Some(&c)), "two quarter voices are below the quorum");
    for p in &people[2..] {
        roster(p, &mut a, &c, vec![h0]).unwrap();
    }
    assert!(counts(&a, &h.endpoints[0], Some(&c)), "all four together reach it");
}

#[test]
fn a_newcomer_s_roster_is_refused_and_a_newer_one_replaces_the_older() {
    let (t, mut a, h, c, people) = rostered(2);
    let h0 = Id::from_hex(&h.endpoints[0]).unwrap();
    let h1 = Id::from_hex(&h.endpoints[1]).unwrap();
    // someone vouched in just now is not established yet
    let late = members(&mut a, &c, 1).remove(0);
    assert!(matches!(roster(&late, &mut a, &c, vec![h0]), Err(hoi_validate::Reject::NotAllowed(_))));
    // a roster names its holders sorted, each once
    let (lo, hi) = if h0 < h1 { (h0, h1) } else { (h1, h0) };
    assert!(roster(&people[0], &mut a, &c, vec![hi, lo]).is_err());
    let first = roster(&people[0], &mut a, &c, vec![h0]).unwrap();
    t.fetch_add(60, Ordering::SeqCst);
    let second = roster(&people[0], &mut a, &c, vec![lo, hi]).unwrap();
    assert!(!a.store.contains(&first) && a.store.contains(&second), "a device's newer roster replaces its older one");
}
