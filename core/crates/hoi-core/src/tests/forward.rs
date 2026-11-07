//! Scattered delivery, phase S3 (SCATTER.md §5.8–5.10): devices keep objects in pieces on
//! holders they reach through forwarders, two hops, so no holder sees who stored or fetched;
//! members' durable devices probe and repair, within a generation's budget and then by a new
//! generation; pointers and their updates are checked; private groups send posts in pieces.
use super::*;
use crate::scatter::{ANSWER_TIMEOUT, ScatterOutcome};
use hoi_crypto::envelope::{self, Request, XKey};
use hoi_crypto::scatter as sc;
use hoi_store::pieces::MemPieces;
use hoi_sync::{Msg, PieceOp};
use hoi_types::scatter::{Manifest, Pointer};
use std::collections::BTreeSet;

const DAY: u64 = 86_400;

/// Nodes that know each other by their names here.
pub(crate) struct Mesh {
    pub(crate) t: Arc<AtomicU64>,
    pub(crate) nodes: BTreeMap<String, Node>,
    pub(crate) eps: BTreeMap<String, String>,
    pub(crate) addrs: BTreeMap<String, String>,
    /// Nodes whose messages go nowhere, in or out.
    pub(crate) cut: BTreeSet<String>,
    /// Every message delivered: (from, to, decoded).
    pub(crate) log: Vec<(String, String, Msg)>,
}

impl Mesh {
    pub(crate) fn new() -> Mesh {
        Mesh { t: Arc::new(AtomicU64::new(T0)), nodes: BTreeMap::new(), eps: BTreeMap::new(), addrs: BTreeMap::new(), cut: BTreeSet::new(), log: Vec::new() }
    }

    pub(crate) fn add(&mut self, name: &str, mut n: Node, addr: &str) {
        let endpoint = Keypair::from_secret(&n.transport_secret().unwrap()).public().hex();
        n.set_transport_address(Some(endpoint.clone()), vec![format!("{endpoint}@{addr}")], vec![]);
        n.scatter.bar = crate::scatter::HolderBar::none();
        self.eps.insert(name.into(), endpoint);
        self.addrs.insert(name.into(), addr.into());
        self.nodes.insert(name.into(), n);
    }

    /// A public node that holds pieces and forwards, in a network of its own.
    pub(crate) fn public(&mut self, name: &str, addr: &str) {
        let mut n = node("node", &self.t, &Mem::default());
        n.set_node_name(Some(name.into()));
        n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
        n.set_carry_policy(crate::CarryPolicy::Any, 64);
        n.set_piece_store(Some(Box::new(MemPieces::new(1 << 30))));
        self.add(name, n, addr);
        assert!(self.nodes[name].forwards_pieces());
    }

    /// A person's desktop.
    pub(crate) fn device(&mut self, name: &str, addr: &str) {
        let mut n = node("desktop", &self.t, &Mem::default());
        n.create_identity(name).unwrap();
        self.add(name, n, addr);
    }

    /// `from` dials `to` over QUIC.
    pub(crate) fn dial(&mut self, from: &str, to: &str) {
        let (fep, faddr, tep, taddr) = (self.eps[from].clone(), self.addrs[from].clone(), self.eps[to].clone(), self.addrs[to].clone());
        let f = self.nodes.get_mut(from).unwrap();
        f.net_connect(to, Role::Peer);
        f.set_peer_meta(to, PeerMeta { transport: "quic".into(), endpoint: Some(tep), addr: Some(taddr), ..Default::default() });
        let t = self.nodes.get_mut(to).unwrap();
        t.net_connect(from, Role::Peer);
        t.set_peer_meta(from, PeerMeta { transport: "quic".into(), incoming: true, endpoint: Some(fep), addr: Some(faddr), ..Default::default() });
    }

    pub(crate) fn hang_up(&mut self, a: &str, b: &str) {
        self.nodes.get_mut(a).unwrap().net_disconnect(b);
        self.nodes.get_mut(b).unwrap().net_disconnect(a);
    }

    pub(crate) fn pump(&mut self) {
        for _ in 0..400 {
            let mut quiet = true;
            let names: Vec<String> = self.nodes.keys().cloned().collect();
            for from in &names {
                let out = self.nodes.get_mut(from).unwrap().take_outbox();
                for (to, m) in out {
                    if self.cut.contains(from) || self.cut.contains(&to) || !self.nodes.contains_key(&to) {
                        continue;
                    }
                    quiet = false;
                    if let Ok(msg) = Msg::decode(&m) {
                        self.log.push((from.clone(), to.clone(), msg));
                    }
                    self.nodes.get_mut(&to).unwrap().net_receive(from, &m);
                }
            }
            if quiet {
                break;
            }
        }
    }

    /// Every node's round each five seconds for `secs`, everything delivered.
    pub(crate) fn run(&mut self, secs: u64) {
        self.run_every(secs, 5);
    }

    /// Every node's round each `step` seconds for `secs`.
    pub(crate) fn run_every(&mut self, secs: u64, step: u64) {
        for _ in 0..secs.div_ceil(step) {
            self.t.fetch_add(step, Ordering::SeqCst);
            for n in self.nodes.values_mut() {
                n.scatter_tick();
            }
            self.pump();
        }
    }

    pub(crate) fn n(&mut self, name: &str) -> &mut Node {
        self.nodes.get_mut(name).unwrap()
    }

    /// Runs until `op` finishes on `name`, at most `secs`.
    pub(crate) fn wait(&mut self, name: &str, op: u64, secs: u64) -> ScatterOutcome {
        self.pump();
        for _ in 0..secs.div_ceil(5) {
            match self.n(name).scatter_take(op) {
                ScatterOutcome::Pending => self.run(5),
                done => return done,
            }
        }
        self.n(name).scatter_take(op)
    }
}

/// `forwarders` public nodes and `holders` more, each in a network of its own, every forwarder
/// linked to every holder, the forwarders' node lists vouching for the holders.
pub(crate) fn mesh(forwarders: usize, holders: usize) -> Mesh {
    let mut m = Mesh::new();
    for i in 0..forwarders {
        m.public(&format!("f{i}"), &format!("44.{}.1.1:7843", 10 + i));
    }
    for i in 0..holders {
        m.public(&format!("h{i}"), &format!("45.{}.7.1:7843", 10 + i));
    }
    for f in 0..forwarders {
        for h in 0..holders {
            m.dial(&format!("f{f}"), &format!("h{h}"));
        }
    }
    m.pump();
    // an hourly sample: each forwarder vouches for the holders it is linked to, with their keys
    m.t.fetch_add(3600, Ordering::SeqCst);
    m.run(5);
    m
}

/// Links a device to the given forwarders and lets their node lists arrive.
pub(crate) fn join(m: &mut Mesh, device: &str, addr: &str, forwarders: &[usize]) {
    m.device(device, addr);
    for f in forwarders {
        m.dial(device, &format!("f{f}"));
    }
    m.pump();
}

fn stored(o: ScatterOutcome) -> Manifest {
    match o {
        ScatterOutcome::Stored(m) => m,
        other => panic!("not stored: {other:?}"),
    }
}

/// Distinct (generation, index) pairs of an object that went through each forwarder, as the
/// requests it passed show: the holder named and the order are all a forwarder sees.
fn passed_through(m: &Mesh, op: PieceOp) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for (from, to, msg) in &m.log {
        if let Msg::Piece { op: o, to: Some(_), .. } = msg
            && *o == op
            && to.starts_with('f')
            && !from.starts_with('f')
        {
            *out.entry(to.clone()).or_default() += 1;
        }
    }
    out
}

#[test]
fn a_post_goes_through_forwarders_and_no_holder_sees_who_stored_or_fetched_it() {
    let mut m = mesh(3, 14);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    join(&mut m, "r", "192.0.2.20:7843", &[0, 1, 2]);
    assert_eq!(m.n("a").scatter_paths(), 3, "three forwarders in three places");
    assert_eq!(m.n("a").scatter_places(), 14, "fourteen holders reached through them, the forwarders' own places left out");

    let content = b"hoi: two hops, and no node holds it whole".to_vec();
    let op = m.n("a").scatter_store(&content, 10, T0 + 30 * DAY).unwrap();
    let man = stored(m.wait("a", op, 60));
    assert_eq!((man.k, man.n, man.places), (5, 12, 14));
    // the device spoke only to its forwarders; holders heard only from forwarders
    assert!(m.log.iter().filter(|(f, _, _)| f == "a").all(|(_, to, _)| to.starts_with('f')));
    assert!(m.log.iter().filter(|(_, to, msg)| to.starts_with('h') && matches!(msg, Msg::Piece { .. })).all(|(f, _, _)| f.starts_with('f')));
    // no forwarder carried k pieces, and at least two carried some
    let through = passed_through(&m, PieceOp::Put);
    assert!(through.len() >= 2, "{through:?}");
    assert!(through.values().all(|&c| c < man.k as usize), "{through:?}");
    assert_eq!(through.values().sum::<usize>(), 12);
    // one piece per holder
    let counts: Vec<u64> = (0..14).map(|i| m.n(&format!("h{i}")).piece_stats().unwrap().count).collect();
    assert_eq!(counts.iter().sum::<u64>(), 12);
    assert!(counts.iter().all(|&c| c <= 1));

    // another device rebuilds it, k+2 pieces asked, never k through one forwarder
    m.log.clear();
    let op = m.n("r").scatter_load(&man).unwrap();
    assert_eq!(m.wait("r", op, 60), ScatterOutcome::Loaded(content.clone()));
    let asked = passed_through(&m, PieceOp::Get);
    assert_eq!(asked.values().sum::<usize>(), man.k as usize + 2);
    assert!(asked.len() >= 2 && asked.values().all(|&c| c < man.k as usize), "{asked:?}");
    assert!(m.log.iter().filter(|(f, _, _)| f == "r").all(|(_, to, _)| to.starts_with('f')));

    // seven of the twelve holders gone: the other five still rebuild it, the forwarders saying
    // `gone` for those they cannot reach
    let held: Vec<usize> = (0..14).filter(|&i| man.holders.iter().any(|e| e.hex() == m.eps[&format!("h{i}")])).collect();
    for &i in &held[..7] {
        for f in 0..3 {
            m.hang_up(&format!("f{f}"), &format!("h{i}"));
        }
    }
    m.log.clear();
    let op = m.n("r").scatter_load(&man).unwrap();
    assert_eq!(m.wait("r", op, 120), ScatterOutcome::Loaded(content));
    // at least two of the seven asked were gone: their forwarders said so within a while
    m.run(30);
    assert!(m.log.iter().any(|(_, to, msg)| to == "r" && matches!(msg, Msg::Piece { op: PieceOp::Gone, .. })));
}

#[test]
fn too_few_forwarders_or_places_keep_it_whole() {
    let mut m = mesh(1, 14);
    join(&mut m, "a", "198.51.100.10:7843", &[0]);
    let e = m.n("a").scatter_store(b"one way out", 10, T0 + DAY).unwrap_err();
    assert_eq!(e.code, "scatter-few-paths", "{}", e.message);

    let mut m = mesh(3, 11);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    let e = m.n("a").scatter_store(b"too few", 10, T0 + DAY).unwrap_err();
    assert_eq!(e.code, "scatter-few-places");
    assert!(e.message.contains("Only 11"), "{}", e.message);
}

#[test]
fn a_holder_the_forwarders_disagree_about_is_left_out() {
    let mut m = mesh(2, 13);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1]);
    assert_eq!(m.n("a").scatter_places(), 13);
    // a forwarder that claims another address for holder 0 takes it away, never adds a place
    let h0 = Id::from_hex(&m.eps["h0"]).unwrap();
    let a = m.n("a");
    let p = a.peers.get_mut("f1").unwrap();
    let v = p.scatter.vouches.iter_mut().find(|v| v.endpoint == h0).unwrap();
    v.routes = vec![format!("{}@203.0.113.9:7843", h0.hex())];
    assert_eq!(a.scatter_places(), 12);
}

#[test]
fn a_forwarder_passes_on_only_to_holders_it_can_reach() {
    let mut m = mesh(1, 2);
    join(&mut m, "a", "198.51.100.10:7843", &[0]);
    let send = |m: &mut Mesh, holder: Id, rid: u32| {
        m.log.clear();
        m.n("f0").net_receive("a", &Msg::Piece { op: PieceOp::Has, rid, to: Some(holder), env: vec![7; 256] }.encode());
        m.pump();
    };
    // a stranger's endpoint: `gone` at once, nothing dialed
    send(&mut m, Id([9; 32]), 1);
    assert!(m.log.iter().any(|(f, to, msg)| f == "f0" && to == "a" && matches!(msg, Msg::Piece { op: PieceOp::Gone, rid: 1, .. })));
    assert!(m.n("f0").forward_dial_routes().is_empty());
    // a holder it saw itself but is not linked to now: dialed, the request waits for the link
    let h1 = Id::from_hex(&m.eps["h1"]).unwrap();
    m.hang_up("f0", "h1");
    send(&mut m, h1, 2);
    assert!(!m.log.iter().any(|(_, to, _)| to == "a"), "it waits");
    assert_eq!(m.n("f0").forward_dial_routes(), vec![format!("{}@{}", m.eps["h1"], m.addrs["h1"])]);
    m.dial("f0", "h1");
    m.pump();
    assert!(m.log.iter().any(|(f, to, msg)| f == "f0" && to == "h1" && matches!(msg, Msg::Piece { op: PieceOp::Has, to: None, .. })), "passed on once linked");
    // unlinked again, and nobody comes: after a while the asker hears `gone`
    m.hang_up("f0", "h1");
    send(&mut m, h1, 3);
    m.run(25);
    assert!(m.log.iter().any(|(f, to, msg)| f == "f0" && to == "a" && matches!(msg, Msg::Piece { op: PieceOp::Gone, rid: 3, .. })));
    assert!(m.n("f0").forward_dial_routes().is_empty());
}

#[test]
fn a_forwarder_reads_nothing_and_the_answer_opens_only_for_the_asker() {
    let mut m = mesh(1, 1);
    join(&mut m, "a", "198.51.100.10:7843", &[0]);
    let h0 = Id::from_hex(&m.eps["h0"]).unwrap();
    let xk = m.n("h0").holder_hello().unwrap().xk;
    let reply = XKey::generate().unwrap();
    let loc = [3u8; 32];
    let env = envelope::seal_request(&h0.0, &xk, &Request::Has { loc, reply: reply.public() }).unwrap();
    m.log.clear();
    m.n("f0").net_receive("a", &Msg::Piece { op: PieceOp::Has, rid: 77, to: Some(h0), env: env.clone() }.encode());
    m.pump();
    // the holder got the very envelope, under the forwarder's own request id
    let to_holder = m.log.iter().find_map(|(f, to, msg)| match msg {
        Msg::Piece { op: PieceOp::Has, rid, env: e, to: None } if f == "f0" && to == "h0" => Some((*rid, e.clone())),
        _ => None,
    });
    let (rid, passed) = to_holder.unwrap();
    assert_ne!(rid, 77, "the asker's id never reaches the holder");
    assert_eq!(passed, env);
    let back = m.log.iter().find_map(|(f, to, msg)| match msg {
        Msg::Piece { op: PieceOp::Had, rid: 77, env, .. } if f == "f0" && to == "a" => Some(env.clone()),
        _ => None,
    });
    let answer = envelope::open_reply(envelope::Kind::Had, &reply, &loc, 0, &back.unwrap()).unwrap();
    assert_eq!(answer, envelope::Reply::Had(None));
    assert_eq!(m.n("f0").forward_stats().0, 1);
}

#[test]
fn a_put_that_went_nowhere_tries_another_forwarder_before_a_fresh_index() {
    let mut m = mesh(4, 14);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2, 3]);
    // forwarder 3 swallows everything
    m.cut.insert("f3".into());
    let op = m.n("a").scatter_store(b"one forwarder is silent", 10, T0 + 7 * DAY).unwrap();
    m.pump();
    assert_eq!(m.n("a").scatter_take(op), ScatterOutcome::Pending);
    m.t.fetch_add(ANSWER_TIMEOUT + 1, Ordering::SeqCst);
    let man = stored(m.wait("a", op, 120));
    // the same indexes went to the same holders through the others: nothing placed twice,
    // no fresh index spent
    assert_eq!(man.hashes.len(), man.n as usize, "no index lost to a silent forwarder");
    let distinct: BTreeSet<_> = man.holders.iter().collect();
    assert_eq!(distinct.len(), man.holders.len());

    // a holder that never answers, through any forwarder: its index may be kept there, so a
    // fresh index goes elsewhere (a fourth forwarder leaves room for it: three carry exactly
    // twelve pieces of five)
    let mut m = mesh(4, 16);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2, 3]);
    let op = m.n("a").scatter_store(b"one holder never answers", 10, T0 + 7 * DAY).unwrap();
    // the first holder a forwarder is asked to pass a put to
    let first = m.nodes["a"].outbox.iter().find_map(|(_, b)| match Msg::decode(b) {
        Ok(Msg::Piece { to: Some(h), .. }) => Some(h.hex()),
        _ => None,
    });
    let first = first.unwrap();
    let silent = m.eps.iter().find(|(_, e)| **e == first).map(|(n, _)| n.clone()).unwrap();
    m.cut.insert(silent.clone());
    m.pump();
    for _ in 0..4 {
        m.t.fetch_add(ANSWER_TIMEOUT + 1, Ordering::SeqCst);
        m.run(5);
    }
    let man = stored(m.wait("a", op, 60));
    assert!(man.hashes.len() > man.n as usize && man.hashes.len() <= man.m as usize, "{} of {}", man.hashes.len(), man.m);
    let distinct: BTreeSet<_> = man.holders.iter().collect();
    assert_eq!(distinct.len(), man.holders.len(), "no holder twice in a generation");
    // and it still rebuilds without the silent holder
    join(&mut m, "r", "192.0.2.20:7843", &[0, 1, 2, 3]);
    let op = m.n("r").scatter_load(&man).unwrap();
    for _ in 0..3 {
        m.t.fetch_add(ANSWER_TIMEOUT + 1, Ordering::SeqCst);
        m.run(5);
    }
    assert_eq!(m.wait("r", op, 60), ScatterOutcome::Loaded(b"one holder never answers".to_vec()));
}

/// A pointer for an object stored by `a` (taken there, as its author).
fn kept(m: &mut Mesh, content: &[u8]) -> Manifest {
    let op = m.n("a").scatter_store(content, 10, T0 + 300 * DAY).unwrap();
    let man = stored(m.wait("a", op, 60));
    let now = m.n("a").now();
    assert!(m.n("a").take_pointer(None, Pointer { made_at: now, manifest: man.clone() }, None, false).unwrap());
    man
}

/// Holders of `man` among the mesh's nodes, by name.
fn holders_of(m: &Mesh, man: &Manifest) -> Vec<String> {
    man.holders.iter().filter_map(|e| m.eps.iter().find(|(_, x)| **x == e.hex()).map(|(n, _)| n.clone())).collect()
}

#[test]
fn a_durable_device_repairs_with_fresh_indexes_then_a_new_generation() {
    let mut m = mesh(4, 40);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2, 3]);
    let content = b"kept for a long time".to_vec();
    let man = kept(&mut m, &content);
    let (k, n) = (man.k as usize, man.n as usize);
    let object = Id(man.object);
    // probes find every piece: nothing to do
    m.run_every(7 * 3600, 60);
    let kept_now = m.n("a").scatter.saved.kept[&object].clone();
    assert!(kept_now.lost.is_empty() && kept_now.repair_at.is_none());

    // enough holders go that the reachable pieces fall to k + ⌈k/3⌉: silence is not loss at once
    let names = holders_of(&m, &man);
    let gone = n - (k + k.div_ceil(3));
    for h in &names[..gone] {
        m.cut.insert(h.clone());
    }
    m.run_every(5 * 3600, 60);
    let after_one = m.n("a").scatter.saved.kept[&object].clone();
    assert!(after_one.lost.is_empty(), "one round of silence is not a loss: {:?}", after_one.lost);
    // later rounds, half a day apart and through forwarders in other places: lost, a repair is
    // scheduled within six hours, then made with fresh indexes in places that held nothing
    m.run_every(36 * 3600, 60);
    let after = m.n("a").scatter.saved.kept[&object].clone();
    assert_eq!(after.lost.len(), gone, "{:?}", after.lost);
    let repaired = m.n("a").scatter.saved.kept[&object].newest().unwrap();
    assert_eq!(repaired.generation, 0);
    assert_eq!(repaired.hashes.len(), n + gone.min(man.m as usize - n), "fresh indexes within the budget");
    assert_eq!(&repaired.holders[..n], &man.holders[..], "earlier indexes stay where they were");
    let fresh: BTreeSet<_> = repaired.holders[n..].iter().collect();
    assert!(fresh.iter().all(|h| !man.holders.contains(h)), "fresh places");

    // more go, the budget is spent: the next generation, fresh secrets, sized for generation 1
    let names = holders_of(&m, &repaired);
    let alive: Vec<String> = names.iter().filter(|h| !m.cut.contains(*h)).cloned().collect();
    for h in &alive[..alive.len() - (k + 1)] {
        m.cut.insert(h.clone());
    }
    // at most a day after the last repair, once the losses are known
    for _ in 0..10 {
        m.run_every(DAY / 2, 60);
        if m.n("a").scatter.saved.kept[&object].newest().unwrap().generation > 0 {
            break;
        }
    }
    let next = m.n("a").scatter.saved.kept[&object].newest().unwrap();
    assert_eq!(next.generation, 1, "a new generation");
    assert_ne!(next.loc_secret, man.loc_secret);
    assert_eq!(next.sealed, man.sealed);
    let s = sc::size::Size { k: next.k as usize, n: next.n as usize, m: next.m as usize };
    assert!(sc::size::meets(10, 1, next.places, s));
    // and it rebuilds to the same object
    join(&mut m, "r", "192.0.2.20:7843", &[0, 1, 2, 3]);
    let op = m.n("r").scatter_load(&next).unwrap();
    assert_eq!(m.wait("r", op, 60), ScatterOutcome::Loaded(content));
}

#[test]
fn updates_are_checked_before_they_count() {
    let mut m = mesh(3, 30);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    join(&mut m, "r", "192.0.2.20:7843", &[0, 1, 2]);
    let man = kept(&mut m, b"checked updates");
    let now = m.n("r").now();
    let alice = Some(Id([1; 32]));
    let mallory = Some(Id([2; 32]));
    let r = m.n("r");
    assert!(r.take_pointer(None, Pointer { made_at: now, manifest: man.clone() }, alice, true).unwrap(), "new to r");
    let object = Id(man.object);
    // a repair that moves an earlier index is refused
    let mut moved = man.clone();
    moved.hashes.push([1; 16]);
    moved.holders.push(Id([77; 32]));
    moved.holders.swap(0, 1);
    assert!(r.take_pointer(None, Pointer { made_at: now, manifest: moved }, mallory, true).is_err());
    // a new generation within a day of the last is refused; a day later it counts
    let mut regen = man.clone();
    regen.generation = 1;
    regen.loc_secret = [9; 32];
    let soon = Pointer { made_at: now + 3600, manifest: regen.clone() };
    assert!(r.take_pointer(None, soon, alice, true).is_err(), "one generation a day");
    m.t.fetch_add(DAY + 7200, Ordering::SeqCst);
    let r = m.n("r");
    let later = Pointer { made_at: now + DAY + 3600, manifest: regen.clone() };
    assert!(!r.take_pointer(None, later, alice, true).unwrap());
    assert_eq!(r.scatter.saved.kept[&object].newest().unwrap().generation, 1);
    // a competing generation 1 made earlier wins, so every member ends with the same
    let mut rival = regen.clone();
    rival.loc_secret = [8; 32];
    assert!(!r.take_pointer(None, Pointer { made_at: now + DAY + 1800, manifest: rival.clone() }, mallory, true).unwrap());
    assert_eq!(r.scatter.saved.kept[&object].newest().unwrap().loc_secret, [8; 32]);
    let mut late_rival = regen.clone();
    late_rival.loc_secret = [6; 32];
    assert!(!r.take_pointer(None, Pointer { made_at: now + DAY + 5000, manifest: late_rival }, alice, true).unwrap());
    assert_eq!(r.scatter.saved.kept[&object].newest().unwrap().loc_secret, [8; 32], "the later one is ignored");
    // the generation before stays as the fallback while the new one does not rebuild here
    assert_eq!(r.scatter.saved.kept[&object].gens.len(), 2);
}

#[test]
fn a_generation_that_rebuilds_into_something_else_is_dropped_and_its_sender_refused() {
    let mut m = mesh(3, 30);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    join(&mut m, "r", "192.0.2.20:7843", &[0, 1, 2]);
    let good = kept(&mut m, b"the real post");
    // mallory, a member, knows the object's key: she seals other words under the same object
    // and places them as its next generation, with honest piece hashes and the real sealed hash
    let (_, other_sealed) = sc::seal_object(b"a forged post", &good.object, &good.key).unwrap();
    let op = m.n("a").start_generation(&good, &other_sealed, 1, 10, 0).unwrap();
    let forged = stored(m.wait("a", op, 60));
    assert_eq!(forged.sealed, good.sealed, "it claims the real object");
    let now = m.n("r").now();
    let alice = Id([1; 32]);
    let mallory = Id([2; 32]);
    let r = m.n("r");
    assert!(r.take_pointer(None, Pointer { made_at: now, manifest: good.clone() }, Some(alice), true).unwrap());
    m.t.fetch_add(DAY + 60, Ordering::SeqCst);
    let made = m.n("r").now();
    m.n("r").take_pointer(None, Pointer { made_at: made, manifest: forged }, Some(mallory), true).unwrap();
    // r fetches: the forged generation rebuilds into other words, it is dropped and mallory is
    // not heard again for this object; the real one is still there and rebuilds
    m.run(600);
    let k = m.n("r").scatter.saved.kept[&Id(good.object)].clone();
    assert!(k.refused.contains(&mallory), "{k:?}");
    assert_eq!(k.newest().unwrap().generation, 0);
    assert!(!k.apply, "the real generation was fetched and applied");
}

#[test]
fn a_private_group_sends_its_posts_in_pieces_through_forwarders() {
    let mut m = mesh(3, 14);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    join(&mut m, "b", "192.0.2.20:7843", &[0, 1, 2]);
    let c = m.n("a").create_private_community(draft("scattered garden")).unwrap().id;
    m.run(10);
    assert_eq!(m.n("a").private_group(&c).unwrap().transport, "mailboxes");
    let request = m.n("b").private_join_request().unwrap();
    let invitation = m.n("a").private_invite(&c, &request).unwrap();
    m.n("b").private_join(&invitation).unwrap();
    m.run(60);

    m.log.clear();
    let first = post(m.n("a"), &c, "hello in pieces");
    m.run(120);
    assert_eq!(m.n("b").post(&first.id).unwrap().summary.title, "hello in pieces");
    // the post itself went out in pieces through forwarders, and its pointer through the mailboxes
    let puts = passed_through(&m, PieceOp::Put);
    assert!(puts.values().sum::<usize>() >= 12, "{puts:?}");
    let ca = Id::from_hex(&c).unwrap();
    assert!(m.n("a").scatter.saved.kept.values().any(|k| k.community == Some(ca) && k.mine));
    assert!(m.n("b").scatter.saved.kept.values().any(|k| k.community == Some(ca) && !k.apply), "b fetched and applied it");
    // no holder holds a whole post: every stored piece is shorter than the post's object
    let reply = post(m.n("b"), &c, "and back, in pieces too");
    m.run(120);
    assert_eq!(m.n("a").post(&reply.id).unwrap().summary.title, "and back, in pieces too");
}

#[test]
fn a_hot_object_is_regenerated_once_with_twice_the_pieces() {
    let mut m = mesh(4, 60);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2, 3]);
    let content = b"everyone opens this at once".to_vec();
    let man = kept(&mut m, &content);
    let object = Id(man.object);
    // a day on, fetches of it keep failing or stalling: holders are swamped
    m.t.fetch_add(DAY + 60, Ordering::SeqCst);
    let hour = m.n("a").now() / 3600;
    m.n("a").scatter.saved.kept.get_mut(&object).unwrap().trouble = (hour, crate::keep::HOT);
    m.run_every(3600, 30);
    let hot = m.n("a").scatter.saved.kept[&object].newest().unwrap();
    assert_eq!(hot.generation, 1, "made anew");
    assert!(hot.n >= 2 * man.n && hot.k > man.k, "more pieces, and k rises with them: {hot:?}");
    let s = sc::size::Size { k: hot.k as usize, n: hot.n as usize, m: hot.m as usize };
    assert!(sc::size::meets(10, 1, hot.places, s), "the bound tightens rather than loosens");
    // once: the trouble was counted, and nothing more happens within a day
    m.run_every(3600, 60);
    assert_eq!(m.n("a").scatter.saved.kept[&object].newest().unwrap().generation, 1);
    join(&mut m, "r", "192.0.2.20:7843", &[0, 1, 2, 3]);
    let op = m.n("r").scatter_load(&hot).unwrap();
    assert_eq!(m.wait("r", op, 60), ScatterOutcome::Loaded(content));
}

/// A private group of `a` (owner) and `b` on a mesh, both reaching holders through f0–f2.
fn group(m: &mut Mesh) -> String {
    join(m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    join(m, "b", "192.0.2.20:7843", &[0, 1, 2]);
    let c = m.n("a").create_private_community(draft("dropping garden")).unwrap().id;
    m.run(10);
    let request = m.n("b").private_join_request().unwrap();
    let invitation = m.n("a").private_invite(&c, &request).unwrap();
    m.n("b").private_join(&invitation).unwrap();
    m.run(60);
    c
}

/// Pieces every holder keeps, in all.
fn pieces_held(m: &mut Mesh) -> u64 {
    let names: Vec<String> = m.nodes.keys().filter(|n| n.starts_with('h')).cloned().collect();
    names.iter().map(|n| m.n(n).piece_stats().unwrap().count).sum()
}

#[test]
fn a_deleted_post_is_let_go_and_its_author_drops_its_pieces() {
    let mut m = mesh(3, 14);
    let c = group(&mut m);
    let ca = Id::from_hex(&c).unwrap();
    // the group's drop key went out with its mailboxes; posts are kept with it
    let key = m.n("b").community_drop_key(ca);
    assert!(key.is_some(), "members know the group's drop key");
    assert_eq!(m.n("a").held_drop_keys(ca).len(), 1, "only the owner's device holds its secret");
    assert!(m.n("b").held_drop_keys(ca).is_empty());
    let before = pieces_held(&mut m);
    let p = post(m.n("b"), &c, "soon gone");
    m.run(120);
    assert_eq!(m.n("a").post(&p.id).unwrap().summary.title, "soon gone");
    let after_post = pieces_held(&mut m);
    assert!(after_post >= before + 12, "{before} → {after_post}");
    let pid = Id::from_hex(&p.id).unwrap();
    assert!(m.n("b").scatter.saved.kept.values().any(|k| k.records == vec![pid] && k.mine), "one object for the post");
    assert!(m.n("a").scatter.saved.kept.values().any(|k| k.records == vec![pid] && !k.mine));

    let man = m.n("b").scatter.saved.kept.values().find(|k| k.records == vec![pid]).unwrap().newest().unwrap();
    assert_eq!(kept_pieces(&mut m, &man), man.hashes.len());

    // its author deletes it: both devices let the object go, and the author's device drops its pieces
    m.n("b").delete_own(&p.id).unwrap();
    m.run(120);
    assert!(!m.n("b").scatter.saved.kept.values().any(|k| k.records == vec![pid]));
    assert!(!m.n("a").scatter.saved.kept.values().any(|k| k.records == vec![pid]), "nobody repairs a deleted post");
    assert_eq!(kept_pieces(&mut m, &man), 0, "every piece is dropped");
}

/// How many of a generation's pieces its holders still keep.
pub(crate) fn kept_pieces(m: &mut Mesh, man: &Manifest) -> usize {
    let today = (m.t.load(Ordering::SeqCst) / DAY) as u32;
    let names = holders_of(m, man);
    let mut kept = 0;
    for (i, h) in man.holders.iter().enumerate() {
        let Some(name) = names.iter().find(|n| m.eps[*n] == h.hex()) else { continue };
        let loc = sc::locator(&man.loc_secret, &man.object, man.generation, i as u8);
        if m.n(name).scatter.pieces.as_mut().unwrap().has(&loc, today).ok().flatten().is_some() {
            kept += 1;
        }
    }
    kept
}

#[test]
fn the_owner_drops_a_removed_post_with_the_group_drop_key() {
    let mut m = mesh(3, 14);
    let c = group(&mut m);
    let p = post(m.n("b"), &c, "against the rules");
    m.run(120);
    let pid = Id::from_hex(&p.id).unwrap();
    assert_eq!(m.n("a").post(&p.id).unwrap().summary.title, "against the rules");
    let man = m.n("a").scatter.saved.kept.values().find(|k| k.records == vec![pid]).unwrap().newest().unwrap();
    assert_eq!(kept_pieces(&mut m, &man), man.hashes.len());
    // the author's device is away for good; the owner removes the post, and once the removal
    // holds, the owner's device drops the pieces with the drop key they were stored with
    m.cut.insert("b".into());
    m.n("a").remove(&p.id, "spam").unwrap();
    m.run(60);
    m.t.fetch_add(2 * 3600, Ordering::SeqCst);
    m.run(120);
    assert!(!m.n("a").scatter.saved.kept.values().any(|k| k.records == vec![pid]));
    assert_eq!(kept_pieces(&mut m, &man), 0, "dropped with the drop key");
}

#[test]
fn mailboxes_are_reached_through_forwarders_and_never_see_the_members() {
    let mut m = mesh(3, 14);
    let c = group(&mut m);
    let ca = Id::from_hex(&c).unwrap();
    let ep = m.n("a").private_epoch(ca).unwrap();
    let holders: Vec<Id> = (0..14).map(|i| Id::from_hex(&m.eps[&format!("h{i}")]).unwrap()).collect();
    let behind: Vec<Id> = ep.mailboxes.iter().map(|b| b.endpoint).filter(|e| holders.contains(e)).collect();
    assert!(!behind.is_empty(), "some mailboxes are holders the members are not linked to");
    // nobody dials them: forwarders reach them
    let dial = m.n("a").mailbox_routes();
    for e in &behind {
        assert!(!dial.iter().any(|r| r.starts_with(&e.hex())), "a mailbox a forwarder reaches is not dialed");
    }
    assert!(m.n("a").private_group(&c).unwrap().mailboxes.iter().all(|b| b.linked), "all reachable");
    m.log.clear();
    let p = post(m.n("a"), &c, "through the mailboxes, two hops");
    m.run(120);
    assert_eq!(m.n("b").post(&p.id).unwrap().summary.title, "through the mailboxes, two hops");
    // the holders heard only from forwarders, and the members' requests to them went through one
    assert!(m.log.iter().filter(|(_, to, _)| to.starts_with('h')).all(|(f, _, _)| f.starts_with('f')));
    let names: BTreeSet<String> = behind.iter().filter_map(|e| m.eps.iter().find(|(_, x)| **x == e.hex()).map(|(n, _)| n.clone())).collect();
    assert!(m.log.iter().any(|(f, to, msg)| (f == "a" || f == "b")
        && to.starts_with('f')
        && matches!(msg, Msg::Piece { to: Some(h), .. } if names.contains(m.eps.iter().find(|(_, x)| **x == h.hex()).map(|(n, _)| n).unwrap()))));
}

#[test]
fn a_phone_fetches_through_its_home_node_which_rebuilds_for_it_alone() {
    let mut m = mesh(3, 14);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    let content = b"rebuilt at home, handed over the pairing link".to_vec();
    let op = m.n("a").scatter_store(&content, 10, T0 + 30 * DAY).unwrap();
    let man = stored(m.wait("a", op, 60));
    // a home node that reaches the forwarders, and a phone paired with it and nothing else
    join(&mut m, "home", "203.0.113.50:7843", &[0, 1, 2]);
    let pairing = m.n("home").set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    let mut phone = node("browser", &m.t, &Mem::default());
    phone.create_identity("lena").unwrap();
    phone.pair(&pairing).unwrap();
    phone.net_connect("home", Role::Home);
    m.nodes.insert("phone".into(), phone);
    m.n("home").net_connect("phone", Role::Client);
    m.pump();
    assert!(m.n("phone").peers["home"].scatter.rebuilds, "the home node said it rebuilds for its devices");
    assert!(!m.n("f0").peers.get("a").is_some_and(|p| p.scatter.rebuilds), "and nobody else hears it");

    m.log.clear();
    let op = m.n("phone").load_kept(&man).unwrap();
    assert_eq!(m.wait("phone", op, 60), ScatterOutcome::Loaded(content));
    // the phone sent one request to its home node, and fetched no piece itself
    assert!(
        m.log.iter().filter(|(f, _, _)| f == "phone").all(|(_, to, msg)| to == "home" && matches!(msg, Msg::Rebuild { .. })),
        "{:?}",
        m.log.iter().filter(|(f, _, _)| f == "phone").count()
    );
    assert!(m.log.iter().any(|(f, to, msg)| f == "home" && to.starts_with('f') && matches!(msg, Msg::Piece { op: PieceOp::Get, .. })), "the home node fetched");

    // a stranger asking a home node to rebuild gets nothing
    m.log.clear();
    m.n("home").net_receive("f0", &Msg::Rebuild { rid: 9, manifest: man.encode() }.encode());
    m.pump();
    assert!(!m.log.iter().any(|(f, to, _)| f == "home" && to == "f0"));
}

/// A browser paired with `home`, which reaches the forwarders, and linked to the forwarders
/// itself, with "Use less data" on.
fn browser_with_home(m: &mut Mesh) {
    join(m, "home", "203.0.113.50:7843", &[0, 1, 2]);
    let pairing = m.n("home").set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    let mut phone = node("web", &m.t, &Mem::default());
    phone.create_identity("lena").unwrap();
    phone.pair(&pairing).unwrap();
    phone.set_settings(&SettingsPatch { less_data: Some(true), ..Default::default() }).unwrap();
    m.add("phone", phone, "192.0.2.77:7843");
    m.n("phone").net_connect("home", Role::Home);
    m.n("home").net_connect("phone", Role::Client);
    for f in 0..3 {
        m.dial("phone", &format!("f{f}"));
    }
    m.pump();
}

#[test]
fn with_less_data_a_device_waits_for_its_home_node_and_fetches_nothing_itself() {
    let mut m = mesh(3, 14);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    let content = b"rebuilt at home, or not at all".to_vec();
    let op = m.n("a").scatter_store(&content, 10, T0 + 30 * DAY).unwrap();
    let man = stored(m.wait("a", op, 60));
    browser_with_home(&mut m);
    assert!(m.n("phone").settings().less_data);
    // the home node is away: the browser asks it, and fetches no piece itself meanwhile
    m.cut.insert("home".into());
    m.log.clear();
    let op = m.n("phone").load_kept(&man).unwrap();
    assert_eq!(m.wait("phone", op, 200), ScatterOutcome::Failed(crate::rebuild::HOME_AWAY.into()));
    let op = m.n("phone").load_kept(&man).unwrap();
    assert_eq!(m.wait("phone", op, 200), ScatterOutcome::Failed(crate::rebuild::HOME_AWAY.into()), "it asks again at once, with no pause");
    assert!(!m.log.iter().any(|(f, _, msg)| f == "phone" && matches!(msg, Msg::Piece { .. })), "no piece fetched by the browser itself");
    // with the home link gone, a paired device waits rather than fetching
    m.n("phone").net_disconnect("home");
    assert!(m.n("phone").load_kept(&man).is_err());
    // the home node back: it rebuilds for the browser
    m.cut.remove("home");
    m.n("phone").net_connect("home", Role::Home);
    m.n("home").net_connect("phone", Role::Client);
    m.pump();
    let op = m.n("phone").load_kept(&man).unwrap();
    assert_eq!(m.wait("phone", op, 60), ScatterOutcome::Loaded(content));
    assert!(!m.log.iter().any(|(f, _, msg)| f == "phone" && matches!(msg, Msg::Piece { .. })));
}

#[test]
fn with_less_data_and_no_home_node_a_load_asks_one_spare_piece() {
    let mut m = mesh(3, 14);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    let content = b"one spare piece fewer".to_vec();
    let op = m.n("a").scatter_store(&content, 10, T0 + 30 * DAY).unwrap();
    let man = stored(m.wait("a", op, 60));
    let mut r = node("web", &m.t, &Mem::default());
    r.create_identity("ines").unwrap();
    m.add("r", r, "192.0.2.20:7843");
    for f in 0..3 {
        m.dial("r", &format!("f{f}"));
    }
    m.pump();
    for (less, spares) in [(false, 2), (true, 1)] {
        m.n("r").set_settings(&SettingsPatch { less_data: Some(less), ..Default::default() }).unwrap();
        m.log.clear();
        let op = m.n("r").load_kept(&man).unwrap();
        assert_eq!(m.wait("r", op, 60), ScatterOutcome::Loaded(content.clone()));
        let asked = passed_through(&m, PieceOp::Get);
        assert_eq!(asked.values().sum::<usize>(), man.k as usize + spares, "less data: {less}");
    }
}

#[test]
fn a_home_node_that_did_not_answer_says_nothing_about_the_generation() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("web", &t, &Mem::default());
    let object = Id([7; 32]);
    n.scatter.saved.kept.insert(object, crate::keep::Kept::default());
    n.applied(object, 0, ScatterOutcome::Failed(crate::rebuild::HOME_AWAY.into()));
    assert!(!n.scatter.saved.kept[&object].unreadable, "no answer is not a failed generation");
    n.applied(object, 0, ScatterOutcome::Failed("Too few places are online for this post right now.".into()));
    assert!(n.scatter.saved.kept[&object].unreadable, "a failed fetch is");
}

#[test]
fn a_reader_whose_forwarders_vouch_for_no_holder_still_rebuilds_through_one_hop_more() {
    let mut m = mesh(3, 14);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    // two forwarders linked to the first three, and to no holder
    m.public("f3", "44.20.1.1:7843");
    m.public("f4", "44.21.1.1:7843");
    for (x, y) in [("f3", "f0"), ("f3", "f1"), ("f4", "f1"), ("f4", "f2")] {
        m.dial(x, y);
    }
    m.pump();
    m.t.fetch_add(3600, Ordering::SeqCst);
    m.run(5);
    join(&mut m, "r", "192.0.2.20:7843", &[3, 4]);
    let content = b"read through a forwarder's forwarder".to_vec();
    let op = m.n("a").scatter_store(&content, 10, T0 + 90 * DAY).unwrap();
    let man = stored(m.wait("a", op, 60));
    assert_eq!(man.keys.len(), man.holders.len(), "the manifest lists each holder's key");
    let fs: Vec<String> = (0..3).map(|i| m.eps[&format!("f{i}")].clone()).collect();
    assert!(man.holders.iter().all(|h| !fs.contains(&h.hex())), "r's forwarders vouch for none of its holders");
    m.log.clear();
    let op = m.n("r").scatter_load(&man).unwrap();
    assert_eq!(m.wait("r", op, 120), ScatterOutcome::Loaded(content));
    // r spoke only to its own forwarders, which passed requests to f0–f2, which reached the holders
    assert!(m.log.iter().filter(|(f, _, _)| f == "r").all(|(_, to, _)| to == "f3" || to == "f4"));
    assert!(
        m.log.iter().any(|(f, to, msg)| (f == "f3" || f == "f4") && ["f0", "f1", "f2"].contains(&to.as_str()) && matches!(msg, Msg::Piece { to: Some(_), .. }))
    );
    assert!(!m.log.iter().any(|(f, to, _)| (f == "f3" || f == "f4") && to.starts_with('h')), "never to a holder they did not know");
    let relayed = m.n("f3").scatter.forward.relayed + m.n("f4").scatter.forward.relayed;
    assert!(relayed >= man.k as u64, "{relayed}");
    // and a month later, when the holders sealed with have new keys, still
    m.t.fetch_add(40 * DAY, Ordering::SeqCst);
    m.run(5);
    // an hourly round: node lists vouch again, with the holders' new keys
    m.t.fetch_add(3600, Ordering::SeqCst);
    m.run(30);
    let now_keys: Vec<[u8; 32]> =
        man.holders.iter().map(|h| m.nodes.values().find(|n| n.own_endpoint_id() == Some(*h)).unwrap().holder_hello().unwrap().xk).collect();
    assert!(now_keys.iter().zip(&man.keys).all(|(a, b)| a != b), "every holder has a new key since");
    let op = m.n("r").scatter_load(&man).unwrap();
    assert!(matches!(m.wait("r", op, 120), ScatterOutcome::Loaded(_)), "holders open what was sealed to their earlier keys");
}
