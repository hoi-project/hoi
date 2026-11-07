//! Where a generation's pieces go (SCATTER.md §5.7): one piece per independent place, never a
//! place that held a sibling of the same generation, and no network block or introducer with
//! more than its share. Pure: the caller brings the candidates and the randomness, so the core
//! and the simulator place pieces with the same rules.
//!
//! The sizing that decides how many pieces (`hoi_crypto::scatter::size`) assumes a party's
//! share among these candidates is its share of real places. The rules here are what keep it
//! so: places merge everything one network or one named operator runs, every place weighs the
//! same however many nodes it has or however long they have been up, and caps keep one network
//! block or one source of candidates from filling a generation.

use std::collections::{BTreeMap, BTreeSet};

/// A node pieces may go to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    /// Who runs it, as its notice names them: a claim, so it only merges places.
    pub operator: Option<u64>,
    /// Its network: IPv4 /24, IPv6 /48, or this machine. One network is one place.
    pub network: Option<u64>,
    /// Its wider block: IPv4 /16, IPv6 /32. At most ⌈n/8⌉ pieces of a generation per block.
    pub block: Option<u64>,
    /// Where this device first learned of it (itself, for a node it reached first hand). At
    /// most ⌈n/4⌉ pieces of a generation per introducer.
    pub introducer: u64,
}

/// Every candidate's place, numbered from 0: candidates sharing an operator or a network,
/// directly or through others, are one place (as Warmth counts them).
pub fn places(candidates: &[Candidate]) -> Vec<usize> {
    let keys: Vec<(Option<u64>, Option<u64>)> = candidates.iter().map(|c| (c.operator, c.network)).collect();
    crate::warmth::places_of(&keys)
}

/// How many distinct places the candidates make, leaving out the places of `exclude` (the
/// candidates this object's forwarders, or earlier holders of the generation, are in).
pub fn count_places(candidates: &[Candidate], exclude: &[usize]) -> usize {
    let place = places(candidates);
    let out: BTreeSet<usize> = exclude.iter().filter_map(|&i| place.get(i).copied()).collect();
    place.iter().filter(|p| !out.contains(p)).collect::<BTreeSet<_>>().len()
}

/// Chooses `want` more candidates for a generation of `n` pieces: each in a place of its own,
/// outside the places of `used` (the candidates that hold, or ever held, an index of this
/// generation, or this object's forwarders), with at most ⌈n/8⌉ of the generation in one block
/// and ⌈n/4⌉ through one introducer over its whole life, `used` counting toward both. `pick(len)`
/// returns a uniform index below `len` (the caller's CSPRNG, or a seeded one in simulations).
/// Places are drawn uniformly, each weighing one; a place's node is drawn uniformly among its
/// candidates that keep the caps. `None` when the caps or the places leave fewer than `want`.
pub fn place(candidates: &[Candidate], n: usize, want: usize, used: &[usize], pick: &mut dyn FnMut(usize) -> usize) -> Option<Vec<usize>> {
    if want == 0 {
        return Some(Vec::new());
    }
    let place = places(candidates);
    let out: BTreeSet<usize> = used.iter().filter_map(|&i| place.get(i).copied()).collect();
    let mut by_place: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, &p) in place.iter().enumerate() {
        if !out.contains(&p) {
            by_place.entry(p).or_default().push(i);
        }
    }
    let block_cap = n.div_ceil(8);
    let introducer_cap = n.div_ceil(4);
    let mut blocks: BTreeMap<u64, usize> = BTreeMap::new();
    let mut introducers: BTreeMap<u64, usize> = BTreeMap::new();
    for c in used.iter().filter_map(|&i| candidates.get(i)) {
        if let Some(b) = c.block {
            *blocks.entry(b).or_default() += 1;
        }
        *introducers.entry(c.introducer).or_default() += 1;
    }
    let mut open: Vec<usize> = by_place.keys().copied().collect();
    let mut chosen = Vec::with_capacity(want);
    // Places are filtered again only when a cap may have filled (and once at the start, for the
    // generation's earlier holders): nothing else changes which places still have a node
    // within the caps.
    let mut capped = !used.is_empty();
    while chosen.len() < want {
        let within = |c: &Candidate, blocks: &BTreeMap<u64, usize>, introducers: &BTreeMap<u64, usize>| {
            c.block.is_none_or(|b| blocks.get(&b).copied().unwrap_or(0) < block_cap) && introducers.get(&c.introducer).copied().unwrap_or(0) < introducer_cap
        };
        if capped {
            open.retain(|p| by_place[p].iter().any(|&i| within(&candidates[i], &blocks, &introducers)));
            capped = false;
        }
        if open.is_empty() {
            return None;
        }
        let p = open.swap_remove(pick(open.len()));
        let fits: Vec<usize> = by_place[&p].iter().copied().filter(|&i| within(&candidates[i], &blocks, &introducers)).collect();
        let i = fits[pick(fits.len())];
        let c = &candidates[i];
        if let Some(b) = c.block {
            let count = blocks.entry(b).or_default();
            *count += 1;
            capped |= *count == block_cap;
        }
        let count = introducers.entry(c.introducer).or_default();
        *count += 1;
        capped |= *count == introducer_cap;
        chosen.push(i);
    }
    Some(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator for tests (xorshift).
    fn rng(mut s: u64) -> impl FnMut(usize) -> usize {
        move |len| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % len as u64) as usize
        }
    }

    fn own(i: u64) -> Candidate {
        Candidate { operator: None, network: Some(i), block: Some(i), introducer: i }
    }

    #[test]
    fn one_piece_per_place() {
        // ten nodes in one network and an operator who runs nodes in three networks are two places
        let mut c: Vec<Candidate> = (0..10).map(|i| Candidate { operator: None, network: Some(1), block: Some(1), introducer: 100 + i }).collect();
        c.extend((0..3).map(|i| Candidate { operator: Some(9), network: Some(10 + i), block: Some(2 + i), introducer: 200 + i }));
        assert_eq!(count_places(&c, &[]), 2);
        assert_eq!(place(&c, 3, 3, &[], &mut rng(1)), None);
        let two = place(&c, 2, 2, &[], &mut rng(1)).unwrap();
        let p = places(&c);
        assert_ne!(p[two[0]], p[two[1]]);
    }

    #[test]
    fn caps_per_block_and_introducer() {
        // 32 places, all in one /16: at most ⌈n/8⌉ of a generation may go there
        let same_block: Vec<Candidate> = (0..32).map(|i| Candidate { block: Some(7), ..own(i) }).collect();
        assert_eq!(place(&same_block, 8, 8, &[], &mut rng(2)), None, "8 pieces, one block, cap 1");
        let mut mixed = same_block.clone();
        mixed.extend((100..120).map(own));
        let chosen = place(&mixed, 16, 16, &[], &mut rng(3)).unwrap();
        assert!(chosen.iter().filter(|&&i| mixed[i].block == Some(7)).count() <= 2);
        // one introducer that brought 30 places may supply at most ⌈n/4⌉
        let poisoned: Vec<Candidate> = (0..30).map(|i| Candidate { introducer: 999, ..own(i) }).chain((100..120).map(own)).collect();
        for seed in 0..50 {
            let chosen = place(&poisoned, 16, 16, &[], &mut rng(seed + 10)).unwrap();
            assert!(chosen.iter().filter(|&&i| poisoned[i].introducer == 999).count() <= 4);
        }
    }

    #[test]
    fn excluded_places_and_reuse() {
        let c: Vec<Candidate> = (0..12).map(own).collect();
        // the forwarders' (or earlier holders') places are left out
        let chosen = place(&c, 9, 9, &[0, 1, 2], &mut rng(4)).unwrap();
        assert!(chosen.iter().all(|&i| i > 2));
        assert_eq!(count_places(&c, &[0, 1, 2]), 9);
        assert_eq!(place(&c, 10, 10, &[0, 1, 2], &mut rng(4)), None);
        // and nothing is chosen twice
        let all = place(&c, 12, 12, &[], &mut rng(5)).unwrap();
        assert_eq!(all.iter().collect::<BTreeSet<_>>().len(), 12);
    }

    #[test]
    fn caps_count_over_the_generation_s_whole_life() {
        // a poisoned introducer brought 30 places; the generation (n = 16) already holds 4 of its
        let c: Vec<Candidate> = (0..30).map(|i| Candidate { introducer: 999, ..own(i) }).chain((100..140).map(own)).collect();
        let first: Vec<usize> = vec![0, 1, 2, 3];
        for seed in 0..50 {
            // one piece at a time, as repair places them: never another through that introducer
            let more = place(&c, 16, 1, &first, &mut rng(seed + 100)).unwrap();
            assert!(more.iter().all(|&i| c[i].introducer != 999), "seed {seed}");
        }
    }

    #[test]
    fn places_weigh_the_same_whatever_their_size() {
        // place A has 50 nodes, places B..K one each: A is chosen no more often than any other
        let mut c: Vec<Candidate> = (0..50).map(|i| Candidate { operator: Some(1), network: Some(1000 + i), block: None, introducer: 5000 + i }).collect();
        c.extend((0..10).map(|i| Candidate { block: None, ..own(i) }));
        let p = places(&c);
        let mut hits_a = 0;
        let trials = 4000;
        let mut r = rng(7);
        for _ in 0..trials {
            let chosen = place(&c, 1, 1, &[], &mut r).unwrap();
            if p[chosen[0]] == p[0] {
                hits_a += 1;
            }
        }
        // one of 11 places: expect ~364; a node-weighted draw would give ~3300
        assert!((250..500).contains(&hits_a), "{hits_a}");
    }
}
