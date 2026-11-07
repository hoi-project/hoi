//! The scatter scenario (SCATTER.md §3, §15; phase S2's acceptance): generations placed with the
//! core's own rules (`hoi_sync::place`) among places a party runs a share of, sized with the
//! core's own function (`hoi_crypto::scatter::size`). Each trial spends a generation's whole
//! index budget, the worst case repair can reach, and counts whether the party ended up with
//! enough to rebuild (k of all m placed) or to withhold (n−k+1 of the n live). The counts are
//! set against the exact hypergeometric tails: in the hard regime they must be zero, elsewhere
//! within chance of the formula. A poisoned variant brings the party's places in through one
//! introducer, to show what the per-introducer cap holds back.

use hoi_crypto::scatter::size::{self, Size};
use hoi_sync::place::{self, Candidate};

/// How the party's places reach the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Party {
    /// Like everyone else: each place reached first hand.
    Fair,
    /// All through one introducer (a poisoned node list), which placement caps at ⌈n/4⌉.
    OneIntroducer,
    /// Established Sybils with enough voice list every party place in the community's rosters,
    /// and only rosters make them count: like a fifth of the honest places, which count only
    /// thanks to honest rosters, they share the rosters' one introducer.
    Roster,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub level: u8,
    pub places: u32,
    /// Places the party runs.
    pub party: u32,
    pub size: Size,
    pub trials: u64,
    pub rebuilt: u64,
    pub withheld: u64,
    /// The exact chance per generation the arithmetic gives, for a fair party.
    pub rebuild_chance: f64,
    pub withhold_chance: f64,
    /// The party can never hold k places at all.
    pub hard: bool,
    /// The most pieces of one generation the party ever held.
    pub most: usize,
}

/// xorshift64*: deterministic, so a run with the same seed gives the same report.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, len: usize) -> usize {
        (self.next() % len as u64) as usize
    }
}

/// P(X ≥ k) for X ~ Hypergeometric(N, A, d), in floating point (reports only; the core decides
/// with exact arithmetic).
pub fn hyper_tail(places: u64, party: u64, draws: u64, k: u64) -> f64 {
    if party < k || draws < k {
        return 0.0;
    }
    // ln C(a, b)
    let ln_c = |a: u64, b: u64| -> f64 { (0..b).map(|i| ((a - i) as f64).ln() - ((i + 1) as f64).ln()).sum() };
    let total = ln_c(places, draws);
    (k..=draws.min(party)).filter(|&i| draws - i <= places - party).map(|i| (ln_c(party, i) + ln_c(places - party, draws - i) - total).exp()).sum()
}

/// One configuration: `trials` generations among `places` places, `party_percent` of which the
/// party runs, sized for `level`.
pub fn run(level: u8, places: u32, party_percent: u32, party_mode: Party, trials: u64, seed: u64) -> Option<Outcome> {
    let s = size::size(level, 0, places)?;
    let party = places * party_percent / 100;
    // Place p is the party's when p < party. Every place a node of its own, in a network and a
    // block of its own: blocks are no constraint here, so a fair party's draws are exactly the
    // hypergeometric ones.
    let candidates: Vec<Candidate> = (0..places as u64)
        .map(|p| Candidate {
            operator: None,
            network: Some(p),
            block: None,
            introducer: match party_mode {
                Party::OneIntroducer if p < party as u64 => u64::MAX,
                // the party's places, and every fifth honest one, count only thanks to rosters
                Party::Roster if p < party as u64 || p % 5 == 0 => u64::MAX - 1,
                _ => p,
            },
        })
        .collect();
    let mut rng = Rng(seed.max(1));
    let (mut rebuilt, mut withheld, mut most) = (0, 0, 0);
    for _ in 0..trials {
        let mut pick = |len: usize| rng.below(len);
        let first = place::place(&candidates, s.n, s.n, &[], &mut pick)?;
        let live = first.iter().filter(|&&i| (i as u32) < party).count();
        // Repair spends the rest of the budget on places that never held a sibling: the party
        // keeps everything it was ever given.
        let more = place::place(&candidates, s.n, s.m - s.n, &first, &mut pick).unwrap_or_default();
        let ever = live + more.iter().filter(|&&i| (i as u32) < party).count();
        most = most.max(ever);
        if ever >= s.k {
            rebuilt += 1;
        }
        if live > s.n - s.k {
            withheld += 1;
        }
    }
    Some(Outcome {
        level,
        places,
        party,
        size: s,
        trials,
        rebuilt,
        withheld,
        rebuild_chance: hyper_tail(places as u64, party as u64, s.m as u64, s.k as u64),
        withhold_chance: hyper_tail(places as u64, party as u64, s.n as u64, (s.n - s.k + 1) as u64),
        hard: (party as usize) < s.k,
        most,
    })
}

/// Whether an observed count is within chance of `p` per trial: five standard deviations and a
/// little slack for tiny counts.
pub fn within_chance(observed: u64, trials: u64, p: f64) -> bool {
    let mean = trials as f64 * p;
    let sd = (trials as f64 * p * (1.0 - p)).sqrt();
    (observed as f64 - mean).abs() <= 5.0 * sd + 3.0
}

/// The report `hoi-sim scatter` prints.
pub fn report(trials: u64, seed: u64) -> Vec<String> {
    let mut out = vec!["places  level  party   k/n (m)    rebuilt (exact)            withheld (exact)           most  hard".to_string()];
    for &places in &[30u32, 80, 100, 400, 1000] {
        for &level in &size::LEVELS {
            for &share in &[5u32, 10, 15, 20, 30] {
                let Some(o) = run(level, places, share, Party::Fair, trials, seed) else { continue };
                out.push(format!(
                    "{:>6}  {:>4}%  {:>4}%  {:>3}/{:<3} ({:>3})  {:>7} ({:>10.3e})  {:>7} ({:>10.3e})  {:>4}  {}",
                    places,
                    level,
                    share,
                    o.size.k,
                    o.size.n,
                    o.size.m,
                    o.rebuilt,
                    o.rebuild_chance * trials as f64,
                    o.withheld,
                    o.withhold_chance * trials as f64,
                    o.most,
                    if o.hard { "yes" } else { "" }
                ));
            }
        }
    }
    for &places in &[400u32, 1000] {
        if let Some(o) = run(10, places, 50, Party::OneIntroducer, trials, seed) {
            out.push(format!(
                "poisoned: {} places, half of them the party's through one node list: k/n {}/{}, the party held at most {} of a generation, rebuilt {} times",
                places, o.size.k, o.size.n, o.most, o.rebuilt
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exact_tail_in_floats_agrees_with_known_values() {
        // 15 of 28 with a budget of 35 in a large network: ≈ 4.6e-7 at 10% (SCATTER.md §3)
        let big = hyper_tail(100_000, 10_000, 35, 15);
        assert!((big / 4.6e-7 - 1.0).abs() < 0.05, "{big}");
        assert_eq!(hyper_tail(80, 8, 23, 9), 0.0, "hard: fewer places than k");
    }

    #[test]
    fn in_small_networks_a_tenth_of_the_places_never_rebuilds_or_withholds() {
        for places in [12u32, 30, 50, 80, 90] {
            let o = run(10, places, 10, Party::Fair, 3000, 7).unwrap();
            assert!(o.hard, "{places}");
            assert_eq!((o.rebuilt, o.withheld), (0, 0), "{o:?}");
            assert!(o.most < o.size.k);
        }
    }

    #[test]
    fn placement_draws_match_the_arithmetic() {
        // A party far above the level, so the chances are large enough to measure: what it
        // observes must be what the formula the sizing trusts says.
        for (places, share) in [(1000u32, 30u32), (400, 35), (100, 40)] {
            let o = run(10, places, share, Party::Fair, 4000, 11).unwrap();
            assert!(o.rebuild_chance > 0.01, "{o:?}");
            assert!(
                within_chance(o.rebuilt, o.trials, o.rebuild_chance),
                "rebuilt {} of {}, expected {:.1}",
                o.rebuilt,
                o.trials,
                o.rebuild_chance * o.trials as f64
            );
            assert!(
                within_chance(o.withheld, o.trials, o.withhold_chance),
                "withheld {} of {}, expected {:.1}",
                o.withheld,
                o.trials,
                o.withhold_chance * o.trials as f64
            );
        }
    }

    #[test]
    fn at_the_level_nothing_measurable_happens() {
        // at the level's own share the chance is under a millionth: none in a few thousand
        let o = run(10, 1000, 10, Party::Fair, 3000, 3).unwrap();
        assert!(o.rebuild_chance < 1e-6, "{o:?}");
        assert_eq!(o.rebuilt, 0);
    }

    #[test]
    fn a_poisoned_roster_gets_no_more_than_the_rosters_cap() {
        // half the candidate places are the party's, all counting only thanks to rosters its
        // established Sybils publish: with honest places that need rosters too, they share ⌈n/4⌉
        let poisoned = run(10, 400, 50, Party::Roster, 2000, 7).unwrap();
        assert!(poisoned.most <= poisoned.size.n.div_ceil(4), "{poisoned:?}");
        assert_eq!((poisoned.rebuilt, poisoned.withheld), (0, 0), "{poisoned:?}");
        // at the level's own share, rosters make it no likelier than a fair party
        let at_level = run(10, 400, 10, Party::Roster, 2000, 7).unwrap();
        assert_eq!(at_level.rebuilt, 0, "{at_level:?}");
    }

    #[test]
    fn a_poisoned_node_list_gets_no_more_than_its_cap() {
        // half the candidate places are the party's, all through one introducer
        let poisoned = run(10, 400, 50, Party::OneIntroducer, 2000, 5).unwrap();
        assert!(poisoned.most <= poisoned.size.n.div_ceil(4), "{poisoned:?}");
        assert_eq!(poisoned.rebuilt, 0, "{poisoned:?}");
        // the same places reached fairly are half the network: the cap is what held it back
        let fair = run(10, 400, 50, Party::Fair, 2000, 5).unwrap();
        assert!(fair.rebuilt > 1000, "{fair:?}");
    }
}
