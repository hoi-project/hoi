//! Holder failures against retrieval, repair and deletion (RFC-0005): a deterministic fault
//! matrix on the [`Mesh`] of `forward.rs`. The faults are test-only and sit where they would in
//! the field: a holder's piece store that says it keeps nothing or returns other bytes, a holder
//! that never answers, a forwarder that swallows what it is given. Everything else is the
//! production path: sealing, forwarding, piece hashes, answer timeouts, probes, repair
//! scheduling, placement caps and deletion.
//!
//! | Fault or state | Observed here |
//! | --- | --- |
//! | Timeout, false "not found", corrupt bytes, silent forwarder | Other indexes are asked; the object loads while k valid pieces and paths to them remain, and never otherwise |
//! | Fewer than k valid pieces, no copy that could repair it | Unavailable and tried again; no deletion, removal or forgotten generation; it loads once holders recover |
//! | False "not found" at a durable member's probes | Lost at once; fresh indexes in fresh places within the repair wait restore n |
//! | Author deletion during that repair, puts placed and unacknowledged | Nothing republished; every placed piece is dropped |
//! | Moderator removal while holders are away or lie | The removal holds; every holder that answers drops its piece; nothing returns with the others |
//!
//! Corrupt cases keep at least one valid piece answering. When every piece that answers fails
//! its hash, SCATTER.md §5.10 still refuses the generation as not of these pieces; that rule is
//! the spec's and is not exercised here.
//!
//! Silent holders and timeouts during repair are `forward.rs`'s
//! `a_durable_device_repairs_with_fresh_indexes_then_a_new_generation`; a matching or mismatching
//! own copy is `open.rs`'s `a_member_s_own_copy_brings_back_a_post_whose_pieces_fell_below_k` and
//! `an_own_copy_that_does_not_match_is_never_published`.
//!
//! What each case assumes is recorded with it in a [`Fixture`]: `k`, `n` and the budget come
//! from the manifest the case stores at level 10, the places from the mesh as placement counts
//! them. Every holder and forwarder has an address of its own, but distinct endpoints and places
//! are not independent operators: one operator could run them all. Time is simulated; nothing
//! leaves the process. The results are guarantees of these fixtures, not a failure-tolerance
//! percentage.
use super::*;
use crate::keep::{APPLY_AGAIN, Job, PROBE_YOUNG, REPAIR_WAIT};
use crate::scatter::{ANSWER_TIMEOUT, SPARES, ScatterOutcome, TOO_FEW_ONLINE};
use crate::tests::forward::{Mesh, group, holders_of, join, kept, kept_pieces, mesh, passed_through, stored};
use crate::tests::open::{generation_of, open_community, until};
use hoi_crypto::scatter::size;
use hoi_store::pieces::{MemPieces, PieceMeta, PieceStats, PieceStore, PutOutcome};
use hoi_sync::{Msg, PieceOp};
use hoi_types::scatter::Manifest;
use hoi_types::{Kind, LedgerAction};
use std::collections::BTreeSet;

const HOUR: u64 = 3600;

/// What a holder does with requests for the pieces it keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    Healthy,
    /// Takes requests and never answers: whoever asked times out. Puts and drops do not reach it.
    Silent,
    /// Says it keeps nothing, when asked for a piece or whether it keeps one, though it does.
    /// Puts and authorized drops work as they should.
    Absent,
    /// Returns other bytes of the same length for a piece it keeps; probes still say it is kept.
    Corrupt,
}

/// A holder's piece store with a fault switch over the real one.
struct Faulty {
    inner: MemPieces,
    fault: Arc<Mutex<Fault>>,
}

impl Faulty {
    fn fault(&self) -> Fault {
        *self.fault.lock().unwrap()
    }
}

impl PieceStore for Faulty {
    fn put(&mut self, loc: &[u8; 32], meta: PieceMeta, bytes: &[u8], today: u32) -> Result<PutOutcome, String> {
        self.inner.put(loc, meta, bytes, today)
    }
    fn get(&mut self, loc: &[u8; 32], today: u32) -> Result<Option<Vec<u8>>, String> {
        let piece = self.inner.get(loc, today)?;
        Ok(match self.fault() {
            Fault::Absent => None,
            Fault::Corrupt => piece.map(|p| p.into_iter().map(|b| b ^ 0x5a).collect()),
            Fault::Healthy | Fault::Silent => piece,
        })
    }
    fn has(&mut self, loc: &[u8; 32], today: u32) -> Result<Option<u32>, String> {
        let kept = self.inner.has(loc, today)?;
        Ok(if self.fault() == Fault::Absent { None } else { kept })
    }
    fn meta(&self, loc: &[u8; 32]) -> Result<Option<PieceMeta>, String> {
        self.inner.meta(loc)
    }
    fn remove(&mut self, loc: &[u8; 32]) -> Result<bool, String> {
        self.inner.remove(loc)
    }
    fn drop_meta(&self, loc: &[u8; 32], today: u32) -> Result<Option<PieceMeta>, String> {
        self.inner.drop_meta(loc, today)
    }
    fn mark_dropped(&mut self, loc: &[u8; 32], today: u32) -> Result<bool, String> {
        self.inner.mark_dropped(loc, today)
    }
    fn durable_drops(&self) -> bool {
        self.inner.durable_drops()
    }
    fn sweep(&mut self, today: u32) -> Result<u64, String> {
        self.inner.sweep(today)
    }
    fn stats(&self) -> PieceStats {
        self.inner.stats()
    }
}

/// Every holder's fault switch, by name.
struct Faults(BTreeMap<String, Arc<Mutex<Fault>>>);

impl Faults {
    /// Gives every holder of the mesh a store with a switch, before anything is stored.
    fn install(m: &mut Mesh) -> Faults {
        let mut switches = BTreeMap::new();
        let names: Vec<String> = m.nodes.keys().filter(|n| n.starts_with('h')).cloned().collect();
        for name in names {
            assert_eq!(m.n(&name).piece_stats().unwrap().count, 0, "faults go in before pieces");
            let fault = Arc::new(Mutex::new(Fault::Healthy));
            m.n(&name).set_piece_store(Some(Box::new(Faulty { inner: MemPieces::new(1 << 30), fault: fault.clone() })));
            switches.insert(name, fault);
        }
        m.pump();
        Faults(switches)
    }

    fn set(&self, m: &mut Mesh, holder: &str, fault: Fault) {
        *self.0[holder].lock().unwrap() = fault;
        if fault == Fault::Silent {
            m.cut.insert(holder.into());
        } else {
            m.cut.remove(holder);
        }
    }

    fn heal(&self, m: &mut Mesh) {
        let names: Vec<String> = self.0.keys().cloned().collect();
        for name in names {
            self.set(m, &name, Fault::Healthy);
        }
    }

    /// How many of a generation's pieces its holders really keep, whatever they say.
    fn truly_kept(&self, m: &mut Mesh, man: &Manifest) -> usize {
        let was: Vec<(Arc<Mutex<Fault>>, Fault)> = self.0.values().map(|f| (f.clone(), *f.lock().unwrap())).collect();
        for (f, _) in &was {
            *f.lock().unwrap() = Fault::Healthy;
        }
        let kept = kept_pieces(m, man);
        for (f, fault) in was {
            *f.lock().unwrap() = fault;
        }
        kept
    }
}

/// What a case assumes, recorded with it and checked against what it stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fixture {
    /// Pieces that rebuild the object, pieces placed at first, and indexes the generation may
    /// ever have (repairs place fresh ones up to it).
    k: usize,
    n: usize,
    budget: usize,
    /// Places (networks) of the holders the generation was sized from.
    holder_places: usize,
    /// Places of the forwarders the device reaches holders through; one place carries at most
    /// `per_place` of a generation's indexes in one operation.
    forwarder_places: usize,
    per_place: usize,
    /// Pieces a load asks at first; each answer is waited for this long before another index is
    /// asked in its place. A load asks each index once.
    first_asks: usize,
    answer_timeout: u64,
}

fn fixture(m: &mut Mesh, device: &str, man: &Manifest) -> Fixture {
    Fixture {
        k: man.k as usize,
        n: man.n as usize,
        budget: man.m as usize,
        holder_places: man.places as usize,
        forwarder_places: m.n(device).scatter_paths(),
        per_place: man.k as usize - 1,
        first_asks: man.k as usize + SPARES,
        answer_timeout: ANSWER_TIMEOUT,
    }
}

/// Holder faults never become a deletion or a moderator's removal on any node.
fn no_deletion_or_removal(m: &Mesh) {
    for (name, n) in &m.nodes {
        for s in n.store.all() {
            assert_ne!(s.record.kind(), Kind::Delete, "{name} holds a deletion");
            assert!(!matches!(&s.record.body, Body::Ledger(l) if l.action == LedgerAction::Remove), "{name} holds a removal");
        }
        assert!(n.deletion_progress().is_empty(), "{name} tracks a deletion");
    }
}

/// Piece requests `device` sent, delivered or swallowed: one per index a load asked.
fn gets_from(m: &Mesh, device: &str) -> usize {
    m.log.iter().chain(&m.lost).filter(|(from, _, msg)| from == device && matches!(msg, Msg::Piece { op: PieceOp::Get, .. })).count()
}

/// A minute at a time until `done`, at most `secs`; how long it took.
fn step_until(m: &mut Mesh, secs: u64, done: impl Fn(&mut Mesh) -> bool) -> Option<u64> {
    let start = m.t.load(Ordering::SeqCst);
    for _ in 0..secs.div_ceil(60) {
        if done(m) {
            return Some(m.t.load(Ordering::SeqCst) - start);
        }
        m.run_every(60, 60);
    }
    done(m).then(|| m.t.load(Ordering::SeqCst) - start)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Loaded,
    Unavailable,
}

/// One row: the generation's first holders, in index order, fail as listed; `silent_forwarders`
/// of the reader's three forwarders swallow everything.
struct Row {
    case: &'static str,
    holders: &'static [Fault],
    silent_forwarders: usize,
    expect: Expect,
}

use Fault::{Absent, Corrupt, Silent};

const RETRIEVAL: Fixture = Fixture { k: 5, n: 12, budget: 14, holder_places: 14, forwarder_places: 3, per_place: 4, first_asks: 7, answer_timeout: 60 };

/// n − k faulty holders leave exactly k valid pieces; one more leaves k − 1. Three forwarder
/// places carry at most k − 1 = 4 indexes each: one silent place leaves 8 askable indexes, two
/// leave 4.
const ROWS: &[Row] = &[
    Row { case: "timeouts, k valid left", holders: &[Silent; 7], silent_forwarders: 0, expect: Expect::Loaded },
    Row { case: "timeouts, k-1 valid left", holders: &[Silent; 8], silent_forwarders: 0, expect: Expect::Unavailable },
    Row { case: "false not found, k valid left", holders: &[Absent; 7], silent_forwarders: 0, expect: Expect::Loaded },
    Row { case: "false not found, k-1 valid left", holders: &[Absent; 8], silent_forwarders: 0, expect: Expect::Unavailable },
    Row { case: "corrupt bytes, k valid left", holders: &[Corrupt; 7], silent_forwarders: 0, expect: Expect::Loaded },
    Row { case: "corrupt bytes, k-1 valid left", holders: &[Corrupt; 8], silent_forwarders: 0, expect: Expect::Unavailable },
    Row { case: "mixed, k valid left", holders: &[Silent, Silent, Absent, Absent, Corrupt, Corrupt, Corrupt], silent_forwarders: 0, expect: Expect::Loaded },
    Row {
        case: "mixed, k-1 valid left",
        holders: &[Silent, Silent, Absent, Absent, Absent, Corrupt, Corrupt, Corrupt],
        silent_forwarders: 0,
        expect: Expect::Unavailable,
    },
    Row { case: "one forwarder place silent", holders: &[], silent_forwarders: 1, expect: Expect::Loaded },
    Row { case: "two forwarder places silent", holders: &[], silent_forwarders: 2, expect: Expect::Unavailable },
];

#[test]
fn retrieval_asks_other_holders_and_loads_while_k_valid_pieces_and_paths_remain() {
    let mut m = mesh(3, 14);
    let faults = Faults::install(&mut m);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    let content = b"read back while holders fail".to_vec();
    let op = m.n("a").scatter_store(&content, 10, T0 + 30 * DAY).unwrap();
    let man = stored(m.wait("a", op, 60));
    let holders = holders_of(&m, &man);
    // a load asks each index once: even one at a time, each waiting out its timeout, it ends by then
    let deadline = RETRIEVAL.n as u64 * RETRIEVAL.answer_timeout;

    for (i, row) in ROWS.iter().enumerate() {
        let reader = format!("r{i}");
        join(&mut m, &reader, &format!("192.0.2.{}:7843", 20 + i), &[0, 1, 2]);
        assert_eq!(fixture(&mut m, &reader, &man), RETRIEVAL, "{}", row.case);
        for (h, fault) in holders.iter().zip(row.holders) {
            faults.set(&mut m, h, *fault);
        }
        for f in 0..row.silent_forwarders {
            m.cut.insert(format!("f{f}"));
        }
        m.log.clear();
        m.lost.clear();
        let op = m.n(&reader).scatter_load(&man).unwrap();
        let outcome = m.wait(&reader, op, deadline);
        let asked = gets_from(&m, &reader);
        let swallowed = m.lost.iter().filter(|(from, _, msg)| *from == reader && matches!(msg, Msg::Piece { op: PieceOp::Get, .. })).count();
        match row.expect {
            Expect::Loaded if row.holders.is_empty() => {
                assert_eq!(outcome, ScatterOutcome::Loaded(content.clone()), "{}", row.case);
                // the first asks are spread over the three places: some went into the silent one,
                // and k came through the others
                assert!(swallowed > 0 && asked - swallowed >= RETRIEVAL.k, "{}: {asked} asked, {swallowed} swallowed", row.case);
            }
            Expect::Loaded => {
                assert_eq!(outcome, ScatterOutcome::Loaded(content.clone()), "{}", row.case);
                // only k valid pieces, so at least two of the first asks met a fault: other
                // indexes were asked in their place
                assert!(asked > RETRIEVAL.first_asks, "{}: {asked} asked", row.case);
            }
            Expect::Unavailable => {
                assert_eq!(outcome, ScatterOutcome::Failed(TOO_FEW_ONLINE.into()), "{}", row.case);
                assert_eq!(asked, RETRIEVAL.n, "{}: every index was asked before it gave up", row.case);
            }
        }
        // faults never cost a holder its piece
        faults.heal(&mut m);
        m.cut.clear();
        assert_eq!(kept_pieces(&mut m, &man), RETRIEVAL.n, "{}", row.case);
        // answers to this row's requests are forgotten before the next
        m.run(2 * ANSWER_TIMEOUT);
    }
    no_deletion_or_removal(&m);
}

#[test]
fn with_too_few_valid_pieces_a_post_stays_unavailable_and_retried_never_removed() {
    // once the post is placed, 10 of the 14 holders fail: at most 4 of its 12 pieces stay valid,
    // one fewer than k, and at least 2
    let cases: [(&str, [Fault; 10]); 3] = [
        ("false not found", [Absent; 10]),
        ("corrupt bytes", [Corrupt; 10]),
        ("mixed", [Silent, Silent, Silent, Absent, Absent, Absent, Absent, Corrupt, Corrupt, Corrupt]),
    ];
    for (case, holder_faults) in cases {
        let mut m = mesh(3, 14);
        let faults = Faults::install(&mut m);
        let c = open_community(&mut m, &[]);
        let p = post(m.n("a"), &c, "unavailable for a while");
        let pid = Id::from_hex(&p.id).unwrap();
        let ca = Id::from_hex(&c).unwrap();
        assert!(
            until(&mut m, 600, |m| m
                .n("f0")
                .store
                .of_kind(&ca, Kind::Pointer)
                .any(|s| matches!(&s.record.body, Body::Pointer { record, .. } if *record == pid))),
            "{case}: its pointer is out"
        );
        let man = generation_of(&mut m, "a", &p.id).unwrap();
        let object = Id(man.object);
        // the author's device, the one copy that could make it again, goes away; then the
        // holders fail, and a reader joins through the forwarders, which carry the pointer
        let uri = m.n("a").invite(&c).unwrap().uri;
        m.cut.insert("a".into());
        for (i, fault) in holder_faults.iter().enumerate() {
            faults.set(&mut m, &format!("h{i}"), *fault);
        }
        join(&mut m, "r", "192.0.2.22:7843", &[0, 1, 2]);
        assert_eq!(fixture(&mut m, "r", &man), RETRIEVAL, "{case}");
        m.log.clear();
        m.lost.clear();
        m.n("r").join(&uri).unwrap();
        m.run(3 * APPLY_AGAIN + 120);
        assert!(
            m.n("r").store.of_kind(&ca, Kind::Pointer).any(|s| matches!(&s.record.body, Body::Pointer { record, .. } if *record == pid)),
            "{case}: the reader has its pointer"
        );
        assert!(m.n("r").post(&p.id).is_err(), "{case}: nothing made up");
        let kept = m.n("r").scatter.saved.kept.get(&object).cloned();
        let kept = kept.unwrap_or_else(|| panic!("{case}: the reader forgot the post's object"));
        assert!(kept.apply, "{case}: it is still to be fetched");
        assert_eq!(kept.gens.len(), 1, "{case}: its generation stays");
        assert!(kept.refused.is_empty(), "{case}: nobody is refused for holders' faults");
        assert!(gets_from(&m, "r") >= 2 * RETRIEVAL.n, "{case}: tried again");
        no_deletion_or_removal(&m);
        let device = m.n("r").device_kp.as_ref().unwrap().public();
        assert!(!m.n("r").store.of_kind(&ca, Kind::Pointer).any(|s| s.record.author == device), "{case}: the reader published no generation");

        // the holders recover: the next try reads it
        faults.heal(&mut m);
        assert!(until(&mut m, APPLY_AGAIN + 300, |m| m.n("r").post(&p.id).is_ok()), "{case}: readable once holders recover");
        assert_eq!(m.n("r").post(&p.id).unwrap().summary.title, "unavailable for a while");
    }
}

#[test]
fn false_absence_loses_indexes_at_once_and_a_durable_device_restores_redundancy() {
    let mut m = mesh(4, 40);
    let faults = Faults::install(&mut m);
    join(&mut m, "a", "198.51.100.10:7843", &[0, 1, 2, 3]);
    let content = b"kept while holders say it is gone".to_vec();
    let man = kept(&mut m, &content);
    let fx = fixture(&mut m, "a", &man);
    assert_eq!(fx, Fixture { k: 5, n: 12, budget: 15, holder_places: 32, forwarder_places: 4, per_place: 4, first_asks: 7, answer_timeout: 60 });
    let object = Id(man.object);
    // enough holders say "not found" that the reachable pieces fall to k + the repair margin
    let gone = fx.n - (fx.k + size::repair_margin(fx.k, fx.n));
    let names = holders_of(&m, &man);
    for h in &names[..gone] {
        faults.set(&mut m, h, Fault::Absent);
    }
    let absent: BTreeSet<u8> = (0..gone as u8).collect();

    // one round of probes, the first within PROBE_YOUNG and its jitter: a holder's own "no" is a
    // loss at once, with no second round twelve hours later
    let lost = |m: &mut Mesh| m.n("a").scatter.saved.kept[&object].lost.len() >= gone;
    let took = step_until(&mut m, PROBE_YOUNG + PROBE_YOUNG / 6 + 60, lost).expect("lost on the first probe round");
    assert!(took <= PROBE_YOUNG + PROBE_YOUNG / 6 + 60, "{took}");
    let k = m.n("a").scatter.saved.kept[&object].clone();
    assert_eq!(k.lost, absent);
    assert!(absent.iter().all(|i| !k.fails.contains_key(i)), "lost on the holders' word, not by silence");

    // within the repair wait (the rebuild and placement take a few minutes more): fresh indexes
    // in places that never held the generation, through forwarder places carrying fewer than k
    m.log.clear();
    let repaired = |m: &mut Mesh| m.n("a").scatter.saved.kept[&object].newest().unwrap().hashes.len() > fx.n;
    step_until(&mut m, REPAIR_WAIT + 600, repaired).expect("repaired within the repair wait");
    let after = m.n("a").scatter.saved.kept[&object].newest().unwrap();
    assert_eq!(after.generation, 0, "within the budget, not a new generation");
    assert_eq!(after.hashes.len(), fx.n + gone);
    assert!(after.hashes.len() <= fx.budget);
    assert_eq!(&after.holders[..fx.n], &man.holders[..], "earlier indexes stay where they were");
    assert!(after.holders[fx.n..].iter().all(|h| !man.holders.contains(h)), "fresh places");
    assert_eq!(after.holders.iter().collect::<BTreeSet<_>>().len(), after.holders.len(), "no holder twice in a generation");
    let puts = passed_through(&m, PieceOp::Put);
    assert!(puts.values().sum::<usize>() >= gone && puts.values().all(|&c| c <= fx.per_place), "{puts:?}");
    // n valid pieces again, while the two keep saying "not found"
    assert_eq!(after.hashes.len() - m.n("a").scatter.saved.kept[&object].lost.len(), fx.n);
    join(&mut m, "r", "192.0.2.20:7843", &[0, 1, 2, 3]);
    let op = m.n("r").scatter_load(&after).unwrap();
    assert_eq!(m.wait("r", op, 300), ScatterOutcome::Loaded(content));
}

/// The operation placing a repair of `object` on `name`, if one is running.
fn placing(m: &Mesh, name: &str, object: Id) -> Option<u64> {
    m.nodes[name].scatter.jobs.iter().find_map(|(op, job)| matches!(job, Job::Place { object: o } if *o == object).then_some(*op))
}

/// Messages held back: (from, to, bytes).
type Held = Vec<(String, String, Vec<u8>)>;

/// A minute at a time until `name` places a repair of `object`, at most `secs`. Its puts reach
/// their holders; the holders' acknowledgements to `name` come back held, undelivered.
fn repair_in_flight(m: &mut Mesh, name: &str, object: Id, secs: u64) -> Option<(u64, Held)> {
    for _ in 0..secs.div_ceil(60) {
        m.t.fetch_add(60, Ordering::SeqCst);
        for n in m.nodes.values_mut() {
            n.scatter_tick();
        }
        let mut held = Vec::new();
        for _ in 0..400 {
            let mut quiet = true;
            let names: Vec<String> = m.nodes.keys().cloned().collect();
            for from in &names {
                for (to, bytes) in m.n(from).take_outbox() {
                    if m.cut.contains(from) || m.cut.contains(&to) || !m.nodes.contains_key(&to) {
                        continue;
                    }
                    let msg = Msg::decode(&bytes);
                    if to == name && placing(m, name, object).is_some() && matches!(msg, Ok(Msg::Piece { op: PieceOp::Stored, .. })) {
                        held.push((from.clone(), to, bytes));
                        continue;
                    }
                    quiet = false;
                    if let Ok(msg) = msg {
                        m.log.push((from.clone(), to.clone(), msg));
                    }
                    m.n(&to).net_receive(from, &bytes);
                }
            }
            if quiet {
                break;
            }
        }
        if let Some(op) = placing(m, name, object)
            && !held.is_empty()
        {
            return Some((op, held));
        }
        for (from, to, bytes) in held {
            m.n(&to).net_receive(&from, &bytes);
        }
    }
    None
}

#[test]
fn deletion_wins_over_a_repair_that_holder_faults_started() {
    let mut m = mesh(3, 40);
    let faults = Faults::install(&mut m);
    let c = group(&mut m);
    let p = post(m.n("b"), &c, "deleted while its holders fail");
    let other = post(m.n("a"), &c, "another author's post stays");
    m.run(120);
    let pid = Id::from_hex(&p.id).unwrap();
    let man = m.n("b").scatter.saved.kept.values().find(|k| k.records == vec![pid]).unwrap().newest().unwrap();
    let fx = fixture(&mut m, "b", &man);
    assert_eq!(fx, Fixture { k: 5, n: 12, budget: 15, holder_places: 32, forwarder_places: 3, per_place: 4, first_asks: 7, answer_timeout: 60 });
    let object = Id(man.object);
    assert_eq!(m.n("a").post(&p.id).unwrap().summary.title, "deleted while its holders fail");
    // only the author's device probes this object here, so the repair is its own
    m.n("a").scatter.saved.kept.get_mut(&object).unwrap().next_probe = u64::MAX;

    let gone = fx.n - (fx.k + size::repair_margin(fx.k, fx.n));
    for h in &holders_of(&m, &man)[..gone] {
        faults.set(&mut m, h, Fault::Absent);
    }
    // the first probe round loses them, the repair waits at most REPAIR_WAIT, then rebuilds
    let (op, held) = repair_in_flight(&mut m, "b", object, PROBE_YOUNG + PROBE_YOUNG / 6 + REPAIR_WAIT + 1800).expect("a repair started");
    assert_eq!(m.n("b").scatter.saved.kept[&object].lost, (0..gone as u8).collect::<BTreeSet<_>>(), "started by the holders' false absence");
    let placed = m.n("b").scatter_placement(op).expect("a live placement");
    assert_eq!(placed.generation, 0);
    assert!(placed.hashes.len() > fx.n, "fresh indexes of the same generation");
    assert!(faults.truly_kept(&mut m, &placed) > fx.n, "some fresh pieces reached their holders");

    m.n("b").delete_own(&p.id).unwrap();
    m.n("b").keep_tick();
    assert!(!m.n("b").scatter.saved.kept.contains_key(&object));
    assert!(!m.n("b").scatter.jobs.contains_key(&op));
    for (from, to, bytes) in held {
        m.n(&to).net_receive(&from, &bytes);
    }
    m.run(180);
    // the holders recover, and hours pass: probes and repairs would have run by now
    faults.heal(&mut m);
    m.run_every(PROBE_YOUNG + HOUR, 60);

    for name in ["a", "b"] {
        assert!(!m.n(name).scatter.saved.kept.contains_key(&object), "{name} let it go, and no late acknowledgement republished it");
        assert!(m.n(name).post(&p.id).is_err(), "{name} does not show it");
    }
    assert_eq!(kept_pieces(&mut m, &man), 0, "every original piece is dropped, including those the faulty holders kept");
    assert_eq!(kept_pieces(&mut m, &placed), 0, "and every piece the cancelled repair placed");
    assert_eq!(m.n("b").post(&other.id).unwrap().summary.title, "another author's post stays");
    assert_eq!(m.n("a").post(&other.id).unwrap().summary.title, "another author's post stays");
}

#[test]
fn a_moderator_s_removal_holds_while_holders_fail_and_after_they_return() {
    let mut m = mesh(3, 14);
    let faults = Faults::install(&mut m);
    let c = group(&mut m);
    let p = post(m.n("b"), &c, "against the rules");
    m.run(120);
    let pid = Id::from_hex(&p.id).unwrap();
    assert_eq!(m.n("a").post(&p.id).unwrap().summary.title, "against the rules");
    let man = m.n("a").scatter.saved.kept.values().find(|k| k.records == vec![pid]).unwrap().newest().unwrap();
    assert_eq!(fixture(&mut m, "a", &man), RETRIEVAL);
    let object = Id(man.object);
    // the author's device is away for good; of the post's holders, two never answer and two say
    // they keep nothing
    m.cut.insert("b".into());
    let names = holders_of(&m, &man);
    for h in &names[..2] {
        faults.set(&mut m, h, Fault::Silent);
    }
    for h in &names[2..4] {
        faults.set(&mut m, h, Fault::Absent);
    }
    // the owner removes it; once the removal holds, the owner's device drops the pieces with the
    // group's drop key
    m.n("a").remove(&p.id, "spam").unwrap();
    m.run(60);
    m.t.fetch_add(2 * HOUR, Ordering::SeqCst);
    m.run(120);
    let removed =
        |m: &mut Mesh| m.n("a").store.all().any(|s| matches!(&s.record.body, Body::Ledger(l) if l.action == LedgerAction::Remove && l.target == Some(pid)));
    assert!(removed(&mut m));
    assert!(m.n("a").post(&p.id).is_err(), "not shown");
    assert!(!m.n("a").scatter.saved.kept.contains_key(&object), "let go");
    assert_eq!(faults.truly_kept(&mut m, &man), 2, "every holder that answers dropped its piece, those that lie included");

    // the silent holders come back with their pieces; hours pass: nothing brings the post back
    faults.heal(&mut m);
    m.run_every(PROBE_YOUNG + HOUR, 60);
    assert!(removed(&mut m));
    assert!(m.n("a").post(&p.id).is_err(), "still removed");
    assert!(!m.n("a").scatter.saved.kept.contains_key(&object), "no repair, no new generation");
}
