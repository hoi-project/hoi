//! Objects over their whole life (SCATTER.md §5.5, §5.10, §15; phase S3's acceptance): a network
//! of places that come and go, objects placed with the core's rules (`hoi_sync::place`) and
//! sized with the core's function (`hoi_crypto::scatter::size`), repaired the way members'
//! durable devices repair them, every six hours:
//!
//! - an index counts as lost when its holder's place answered "not kept", or stayed silent at
//!   two probes half a day apart (a holder only briefly away is not lost);
//! - once a third of the spare pieces are gone (`size::repair_margin`), fresh indexes go to places that never held one
//!   of the generation, within the budget; with the budget spent, the next generation, sized for
//!   its smaller share of the risk, at most once a day unless fewer than k are left;
//! - every placement is carried by three forwarder places, at most k−1 indexes of a generation
//!   each, and the most any one carried is recorded.
//!
//! A party runs a share of the places. It keeps every piece it is ever given. It may **force
//! repairs** (its holders answer "not kept" for everything they hold, so devices re-place those
//! indexes elsewhere and the party gets more draws), and a member of it may **publish bad
//! updates** (a new generation onto its own places, as often as readers take one, which it then
//! withholds). Scored: how often objects could be read at the repair intervals, how many objects
//! the party could ever rebuild (k pieces of one generation) or withhold (n−k+1 of the live
//! ones), against the exact per-generation tails the sizing trusts, and the most indexes of a
//! generation any forwarder place carried.

use hoi_crypto::scatter::size::{self, Size};
use hoi_sync::place::{self, Candidate};

use crate::scatter::hyper_tail;

/// Repair rounds a day.
const ROUNDS: u32 = 4;
/// Silent rounds in a row past which an index is lost: the first and the last of three are half a
/// day apart (and, in the core, through forwarders in different places).
const SILENT_ROUNDS: u32 = 2;
/// Forwarder places a device sends through.
const FORWARDERS: usize = 3;

/// What happens to objects over `days`.
#[derive(Debug, Clone, Copy)]
pub struct Life {
    pub places: u32,
    pub party_percent: u32,
    pub level: u8,
    /// Percent of places that leave for good each day (and are replaced by new ones).
    pub churn: f64,
    /// Percent of the time an honest holder is reachable at a probe.
    pub uptime: f64,
    pub days: u32,
    pub objects: u32,
    /// The party's holders answer "not kept" for everything: every one of its pieces is
    /// re-placed elsewhere, so it gets as many draws as the budget allows.
    pub forced: bool,
    /// A member of the party makes a new generation onto its own places as often as readers
    /// take one (once a day) and withholds it.
    pub bad_updates: bool,
    /// Spare pieces above k at which a repair starts (`size::repair_at`).
    pub margin: fn(usize, usize) -> usize,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LifeOutcome {
    /// Object-rounds checked, and those the object could be read in (from its newest generation,
    /// or the one before, which readers keep as a fallback).
    pub rounds: u64,
    pub readable: u64,
    /// Objects some generation of which the party could rebuild, or ever withhold.
    pub rebuilt: u64,
    pub withheld: u64,
    /// Objects that could not be read at the end.
    pub lost: u64,
    /// The sum over every generation made of its exact rebuild chance for a fair party of this
    /// share: what the observed `rebuilt` must stay within.
    pub expected_rebuilt: f64,
    pub generations: u64,
    pub repairs: u64,
    /// The most indexes of one generation any forwarder place carried, the smallest k, and
    /// placements where a forwarder place would have carried k or more (none may).
    pub max_path: usize,
    pub min_k: usize,
    pub over: u64,
    /// Placements that found too few places or paths.
    pub failed: u64,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, len: usize) -> usize {
        (self.next() % len.max(1) as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 * 100.0 < p
    }
}

/// A place in the network: a slot whose occupant changes when one leaves.
#[derive(Clone, Copy)]
struct Place {
    party: bool,
    /// Bumped when the place's node leaves and a new one takes the slot: pieces placed with an
    /// older epoch are gone.
    epoch: u32,
}

#[derive(Clone)]
struct Index {
    place: usize,
    epoch: u32,
    party: bool,
    /// Rounds in a row its probe went unanswered.
    silent: u32,
    lost: bool,
}

#[derive(Clone)]
struct Generation {
    g: u8,
    size: Size,
    indexes: Vec<Index>,
    made_round: u32,
    /// Party pieces ever placed of it, and whether the party made it (a bad update).
    party_ever: usize,
    bad: bool,
}

struct Object {
    gens: Vec<Generation>,
    rebuilt: bool,
    withheld: bool,
}

struct Net {
    places: Vec<Place>,
    candidates: Vec<Candidate>,
}

impl Net {
    fn new(n: u32, party: u32) -> Net {
        let places = (0..n).map(|i| Place { party: i < party, epoch: 0 }).collect();
        let candidates = (0..n as u64).map(|p| Candidate { operator: None, network: Some(p), block: None, introducer: p }).collect();
        Net { places, candidates }
    }

    fn up(&self, i: &Index) -> bool {
        self.places[i.place].epoch == i.epoch
    }
}

/// Notes a placement of `count` indexes through the forwarder places.
fn carried(out: &mut LifeOutcome, count: usize, k: usize) {
    let mut c = [0; FORWARDERS];
    match paths(&mut c, count, k) {
        Some(most) => out.max_path = out.max_path.max(most),
        None => out.over += 1,
    }
}

/// Assigns indexes to forwarder places, at most k−1 each over the generation's life; returns
/// the most any carried, or `None` when they do not fit.
fn paths(carried: &mut [usize; FORWARDERS], count: usize, k: usize) -> Option<usize> {
    for _ in 0..count {
        let (i, &least) = carried.iter().enumerate().min_by_key(|(_, c)| **c)?;
        if least + 1 > k - 1 {
            return None;
        }
        carried[i] += 1;
    }
    carried.iter().copied().max()
}

impl Life {
    fn party(&self) -> u32 {
        self.places * self.party_percent / 100
    }

    /// Places a generation of `size`: `want` fresh indexes, never in a place that held one of
    /// it. The party's places are taken by the bad updater when `bad`.
    fn place(&self, net: &Net, gn: &mut Generation, want: usize, rng: &mut Rng, bad: bool) -> bool {
        let used: Vec<usize> = gn.indexes.iter().map(|i| i.place).collect();
        let chosen = if bad {
            let mut party: Vec<usize> = (0..net.places.len()).filter(|&p| net.places[p].party && !used.contains(&p)).collect();
            if party.len() < want {
                return false;
            }
            (0..want).map(|_| party.swap_remove(rng.below(party.len()))).collect()
        } else {
            match place::place(&net.candidates, gn.size.n, want, &used, &mut |len| rng.below(len)) {
                Some(c) => c,
                None => return false,
            }
        };
        for p in chosen {
            let party = net.places[p].party;
            gn.party_ever += usize::from(party);
            gn.indexes.push(Index { place: p, epoch: net.places[p].epoch, party, silent: 0, lost: false });
        }
        true
    }

    /// Whether a generation can be read now: k of its pieces on holders that are there, up and
    /// not the party's when it withholds.
    fn readable(&self, net: &Net, gn: &Generation, rng: &mut Rng) -> bool {
        let withholding = self.forced || gn.bad;
        let up = gn.indexes.iter().filter(|i| net.up(i) && !(i.party && withholding) && (i.party || rng.chance(self.uptime))).count();
        up >= gn.size.k
    }

    pub fn run(&self) -> LifeOutcome {
        let mut rng = Rng(self.seed.max(1));
        let mut net = Net::new(self.places, self.party());
        let mut out = LifeOutcome { min_k: usize::MAX, ..Default::default() };
        let mut objects: Vec<Object> = Vec::new();
        // every object starts as generation 0, placed on the first day
        for _ in 0..self.objects {
            let Some(s) = size::size(self.level, 0, self.places) else { return out };
            let mut gn = Generation { g: 0, size: s, indexes: Vec::new(), made_round: 0, party_ever: 0, bad: false };
            carried(&mut out, s.n, s.k);
            if !self.place(&net, &mut gn, s.n, &mut rng, false) {
                out.failed += 1;
                continue;
            }
            out.generations += 1;
            out.expected_rebuilt += hyper_tail(self.places as u64, self.party() as u64, s.m as u64, s.k as u64);
            out.min_k = out.min_k.min(s.k);
            objects.push(Object { gens: vec![gn], rebuilt: false, withheld: false });
        }
        let leave = self.churn / ROUNDS as f64;
        for round in 1..=self.days * ROUNDS {
            // places that leave for good: a new node takes the slot (the party keeps its share)
            for p in net.places.iter_mut() {
                if rng.chance(leave) {
                    p.epoch += 1;
                }
            }
            for o in objects.iter_mut() {
                self.round(&net, o, round, &mut rng, &mut out);
            }
        }
        for o in &objects {
            out.rebuilt += u64::from(o.rebuilt);
            out.withheld += u64::from(o.withheld);
            let readable =
                o.gens.iter().rev().take(2).filter(|g| !g.bad).any(|g| g.indexes.iter().filter(|i| net.up(i) && !(i.party && self.forced)).count() >= g.size.k);
            out.lost += u64::from(!readable);
        }
        if out.min_k == usize::MAX {
            out.min_k = 0;
        }
        out
    }

    fn round(&self, net: &Net, o: &mut Object, round: u32, rng: &mut Rng, out: &mut LifeOutcome) {
        out.rounds += 1;
        // readers read the newest generation, or the one before while the newest does not rebuild
        let readable = o.gens.iter().rev().take(2).any(|g| self.readable(net, g, rng));
        out.readable += u64::from(readable);
        let newest = o.gens.len() - 1;
        // a bad update: a new generation onto the party's places, once a day as readers allow
        if self.bad_updates && round >= o.gens[newest].made_round + ROUNDS && o.gens[newest].g < 255 {
            let g = o.gens[newest].g + 1;
            if let Some(s) = size::size(self.level, g, self.places) {
                let mut gn = Generation { g, size: s, indexes: Vec::new(), made_round: round, party_ever: 0, bad: true };
                if self.place(net, &mut gn, s.n, rng, true) {
                    out.generations += 1;
                    o.gens.push(gn);
                }
            }
        }
        let newest = o.gens.len() - 1;
        let gn = &mut o.gens[newest];
        // probes: the party answers "not kept" when it forces repairs or withholds a bad generation
        let withholding = self.forced || gn.bad;
        for i in gn.indexes.iter_mut().filter(|i| !i.lost) {
            if i.party && withholding {
                i.lost = true;
            } else if !net.up(i) || (!i.party && !rng.chance(self.uptime)) {
                i.silent += 1;
                i.lost = i.silent > SILENT_ROUNDS;
            } else {
                i.silent = 0;
            }
        }
        let live_party = gn.indexes.iter().filter(|i| !i.lost && i.party).count();
        if !gn.bad && live_party > gn.size.n - gn.size.k {
            o.withheld = true;
        }
        let s = gn.size;
        let reachable = gn.indexes.iter().filter(|i| !i.lost).count();
        if reachable > s.k + (self.margin)(s.k, s.n) && !gn.bad {
            return;
        }
        // repair: a member's device rebuilds from k (the newest, or the fallback) and places
        // fresh indexes within the budget, else the next generation once a day
        let source_readable =
            o.gens.iter().rev().take(2).any(|g| !g.bad && g.indexes.iter().filter(|i| net.up(i) && !(i.party && self.forced)).count() >= g.size.k);
        if !source_readable {
            return;
        }
        out.repairs += 1;
        let gn = &mut o.gens[newest];
        let left = s.m.saturating_sub(gn.indexes.len());
        let want = s.n.saturating_sub(reachable);
        if !gn.bad && left > 0 && want > 0 {
            let more = want.min(left);
            carried(out, more, s.k);
            if !self.place(net, gn, more, rng, false) {
                out.failed += 1;
            }
            if gn.party_ever >= s.k {
                o.rebuilt = true;
            }
            return;
        }
        let made = gn.made_round;
        if round < made + ROUNDS && reachable >= s.k && !gn.bad {
            return;
        }
        let Some(g) = gn.g.checked_add(1) else { return };
        let Some(next) = size::size(self.level, g, self.places) else {
            out.failed += 1;
            return;
        };
        let mut fresh = Generation { g, size: next, indexes: Vec::new(), made_round: round, party_ever: 0, bad: false };
        carried(out, next.n, next.k);
        if !self.place(net, &mut fresh, next.n, rng, false) {
            out.failed += 1;
            return;
        }
        out.generations += 1;
        out.min_k = out.min_k.min(next.k);
        out.expected_rebuilt += hyper_tail(self.places as u64, self.party() as u64, next.m as u64, next.k as u64);
        // a bad generation is dropped by readers once an honest one replaces it
        if o.gens[newest].bad {
            o.gens.pop();
        }
        o.gens.push(fresh);
        if o.gens.len() > 2 {
            o.gens.remove(0);
        }
        for gn in o.gens.iter().filter(|g| !g.bad) {
            if gn.party_ever >= gn.size.k {
                o.rebuilt = true;
            }
        }
    }
}

/// The lines `hoi-sim scatter --life` prints.
pub fn report(objects: u32, days: u32, seed: u64) -> Vec<String> {
    let mut out = vec!["places  party  churn/day  forced  bad-updates  readable     lost  rebuilt (expected)   withheld  gens/object  over k".to_string()];
    let cases: &[(u32, u32, f64, bool, bool)] = &[
        (1000, 10, 1.0, false, false),
        (1000, 10, 5.0, false, false),
        (1000, 10, 10.0, false, false),
        (1000, 10, 20.0, false, false),
        (1000, 10, 10.0, true, false),
        (1000, 10, 10.0, false, true),
        (80, 10, 10.0, true, false),
        (1000, 30, 10.0, true, false),
    ];
    for &(places, party, churn, forced, bad) in cases {
        let life =
            Life { places, party_percent: party, level: 10, churn, uptime: 90.0, days, objects, forced, bad_updates: bad, margin: size::repair_margin, seed };
        let o = life.run();
        out.push(format!(
            "{:>6}  {:>4}%  {:>8}%  {:>6}  {:>11}  {:>8.5}  {:>7}  {:>7} ({:>8.2e})  {:>8}  {:>11.1}  {:>6}",
            places,
            party,
            churn,
            if forced { "yes" } else { "" },
            if bad { "yes" } else { "" },
            o.readable as f64 / o.rounds.max(1) as f64,
            o.lost,
            o.rebuilt,
            o.expected_rebuilt,
            o.withheld,
            o.generations as f64 / objects.max(1) as f64,
            o.over
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scatter::within_chance;

    fn life(places: u32, party: u32, churn: f64) -> Life {
        Life {
            places,
            party_percent: party,
            level: 10,
            churn,
            uptime: 90.0,
            days: 30,
            objects: 150,
            forced: false,
            bad_updates: false,
            margin: size::repair_margin,
            seed: 9,
        }
    }

    #[test]
    fn at_ten_percent_churn_a_day_posts_stay_readable() {
        // SCATTER.md §10, S3: 99.9% readable at 10% churn a day
        for places in [80u32, 1000] {
            let o = life(places, 10, 10.0).run();
            let share = o.readable as f64 / o.rounds as f64;
            assert!(share >= 0.999, "{places} places: {share} {o:?}");
            assert_eq!(o.lost, 0, "{o:?}");
            assert_eq!(o.failed, 0, "{o:?}");
            assert_eq!(o.over, 0, "no path carries k pieces: {o:?}");
            assert_eq!((o.rebuilt, o.withheld), (0, 0), "{o:?}");
        }
    }

    #[test]
    fn forced_repairs_cost_the_party_its_pieces_and_buy_no_more_than_the_budgets() {
        // At the level's share: nothing, in the hard regime and in a large network.
        for places in [80u32, 1000] {
            let o = Life { forced: true, ..life(places, 10, 10.0) }.run();
            assert_eq!(o.rebuilt, 0, "{o:?}");
            assert!(o.readable as f64 / o.rounds as f64 >= 0.999, "{o:?}");
        }
        // Far above the level, where chances are measurable: forcing repairs gets the party no
        // more objects than the generations' exact tails add up to.
        let o = Life { forced: true, objects: 300, ..life(1000, 30, 10.0) }.run();
        assert!(o.expected_rebuilt > 1.0, "{o:?}");
        assert!(
            (o.rebuilt as f64) <= o.expected_rebuilt + 5.0 * o.expected_rebuilt.sqrt() + 3.0,
            "rebuilt {} against {:.1} expected: {o:?}",
            o.rebuilt,
            o.expected_rebuilt
        );
        let _ = within_chance;
    }

    #[test]
    fn bad_updates_delay_a_post_but_leave_it_readable_and_the_bound_intact() {
        let o = Life { bad_updates: true, days: 20, ..life(1000, 10, 10.0) }.run();
        assert!(o.readable as f64 / o.rounds as f64 >= 0.999, "{o:?}");
        assert_eq!(o.lost, 0, "{o:?}");
        assert_eq!(o.rebuilt, 0, "honest generations stay out of reach: {o:?}");
        assert_eq!(o.over, 0, "{o:?}");
    }

    /// The evidence for `size::repair_margin`: run with `--ignored --nocapture` (release).
    #[test]
    #[ignore]
    fn compare_repair_triggers() {
        let third: fn(usize, usize) -> usize = |k, _| k.div_ceil(3);
        let half: fn(usize, usize) -> usize = |k, n| (n - k).div_ceil(2);
        let two_thirds: fn(usize, usize) -> usize = |k, n| (2 * (n - k)).div_ceil(3);
        for (name, f) in [("third", third), ("half", half), ("two thirds", two_thirds)] {
            for places in [80u32, 1000] {
                for churn in [2.0, 10.0] {
                    let o = Life { margin: f, objects: 300, days: 60, ..life(places, 10, churn) }.run();
                    eprintln!(
                        "{name:>10} {places:>5} {churn:>4}%: readable {:.5} lost {} gens/obj {:.2} repairs/obj {:.2} failed {}",
                        o.readable as f64 / o.rounds as f64,
                        o.lost,
                        o.generations as f64 / 300.0,
                        o.repairs as f64 / 300.0,
                        o.failed
                    );
                }
            }
        }
    }

    #[test]
    fn paths_never_carry_k() {
        let mut c = [0; FORWARDERS];
        assert_eq!(paths(&mut c, 12, 5), Some(4));
        assert_eq!(paths(&mut c, 1, 5), None, "three places of four are full");
        let mut c = [0; FORWARDERS];
        assert_eq!(paths(&mut c, 28, 15), Some(10));
    }
}
