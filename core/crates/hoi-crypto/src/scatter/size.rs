//! How many pieces: sizing a generation from the community's level (SCATTER.md §3, §5.4).
//!
//! A party running a share ℓ of the N places a device chooses holders from is assumed to hold
//! ⌊ℓ·N/100⌋ of them. Generation g may give it k or more of everything it ever places (enough to
//! rebuild), or n−k+1 or more of the live pieces (enough to withhold), with probability at most
//! `1 / (LIFE · 2^(g+1))`, so all generations of an object together stay under `1 / LIFE`. The
//! object must stay readable with probability ≥ 0.999 when holders are up 80% of the time, and a
//! browser's three links, each carrying at most k−1 pieces, must be able to carry an upload.
//!
//! Everything is exact integer arithmetic, never floats, so every host agrees on which
//! generations meet a level. `core/vectors/scatter.json` fixes the answers; a separate,
//! deliberately naive BigInt implementation of the same definition (the maintainers' reference
//! calculator) agrees with every case there. Tails here are summed once per n, from the top,
//! which keeps a call to milliseconds even in a browser.

use std::cmp::Ordering;

pub use hoi_types::scatter::{LEVELS, budget};

/// The level communities have unless they decide otherwise, and the floor.
pub const DEFAULT_LEVEL: u8 = LEVELS[0];
/// The lifetime risk target: one object in this many.
pub const LIFE: u64 = 1_000_000;
/// Up to this many places the risk is hypergeometric (exact for the places seen); above it,
/// binomial with the level as the share, which is slightly stricter.
pub const EXACT_UP_TO: u32 = 1000;
/// A browser's links: each carries at most k−1 pieces of a generation, so n ≤ LINKS·(k−1).
pub const LINKS: usize = 3;
/// The uptime bar for holders, as a fraction, used for the availability bound.
const UP: (u32, u32) = (4, 5);
/// Readable while ≥ k of n are up, per repair interval, with at least this probability.
const AVAIL: (u32, u32) = (999, 1000);

/// A generation's size: any `k` of the pieces rebuild it, `n` are placed at first, and at most
/// `m` indexes may ever be placed (the budget repair spends before a new generation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub k: usize,
    pub n: usize,
    pub m: usize,
}

/// Spare pieces above k at which a generation is repaired: two thirds of its `n − k` spares
/// (SCATTER.md §5.10). The plan's first figure, `⌈k/3⌉`, lost about one post in fifty over a
/// month at 10% churn a day in the lifetime simulation (`hoi-sim scatter --life`); this one
/// keeps every post readable there, at about twice the repairs when churn is low.
pub fn repair_margin(k: usize, n: usize) -> usize {
    (2 * n.saturating_sub(k)).div_ceil(3)
}

/// The places a party at `level` percent of `places` is assumed to hold.
pub fn party(level: u8, places: u32) -> u64 {
    level as u64 * places as u64 / 100
}

/// Whether a party at `level` could never hold k places at all: the hard bound of small
/// networks (only counted where the risk is exact).
pub fn hard(level: u8, places: u32, k: usize) -> bool {
    places <= EXACT_UP_TO && party(level, places) < k as u64
}

/// The smallest generation that meets `level` for generation `g` among `places`, or `None`
/// when there are too few places (the object is then not scattered).
pub fn size(level: u8, generation: u8, places: u32) -> Option<Size> {
    size_at_least(level, generation, places, 0)
}

/// [`size`] with at least `min_n` pieces placed at first: a hot object's larger generation
/// (SCATTER.md §5.10), whose k grows with n so the bounds hold at the same risk budget. The
/// first n considered is `max(5, min_n)`; `None` when no n from there fits.
pub fn size_at_least(level: u8, generation: u8, places: u32, min_n: usize) -> Option<Size> {
    let dist = Dist::new(level, places);
    let eps = eps_den(generation);
    let top = if places > EXACT_UP_TO { super::MAX_N } else { (places as usize).min(super::MAX_N) };
    for n in min_n.max(5)..=top {
        let m = dist.cap(budget(n));
        let k_lo = 3.max(n.div_ceil(LINKS) + 1);
        if k_lo > n - 2 {
            continue;
        }
        let rebuild = dist.tails(m, k_lo);
        for k in k_lo..=n - 2 {
            if !available(n, k) {
                break;
            }
            if !rebuild.at_most(k, &eps) {
                continue;
            }
            // the first k that rebuilds safely is also the best for withholding: larger k only
            // makes withholding easier
            if dist.tails(n, n - k + 1).at_most(n - k + 1, &eps) {
                return Some(Size { k, n, m });
            }
            break;
        }
    }
    None
}

/// Whether a generation of size `s` among `places` meets `level` for generation `g`: the
/// rebuild and withhold bounds alone, as a reader checks a manifest.
pub fn meets(level: u8, generation: u8, places: u32, s: Size) -> bool {
    if s.k < 1 || s.k > s.n || s.n > s.m || (places <= EXACT_UP_TO && s.m as u64 > places as u64) {
        return false;
    }
    let dist = Dist::new(level, places);
    let eps = eps_den(generation);
    dist.tails(s.m, s.k).at_most(s.k, &eps) && dist.tails(s.n, s.n - s.k + 1).at_most(s.n - s.k + 1, &eps)
}

/// A generation's reading: the highest level whose bounds it meets, or `None` if it meets not
/// even the floor.
pub fn reading(generation: u8, places: u32, s: Size) -> Option<u8> {
    LEVELS.iter().rev().copied().find(|&l| meets(l, generation, places, s))
}

/// `LIFE · 2^(g+1)`: the risk budget of generation g is its inverse.
fn eps_den(generation: u8) -> Big {
    let mut b = Big::from(LIFE);
    b.shl(generation as usize + 1);
    b
}

/// P(Bin(n, 4/5) ≥ k) ≥ 999/1000.
fn available(n: usize, k: usize) -> bool {
    let t = binomial_tails(n, UP.0, UP.1, k);
    let mut lhs = t.cum[0].clone();
    lhs.mul_small(AVAIL.1);
    let mut rhs = t.den.clone();
    rhs.mul_small(AVAIL.0);
    lhs.cmp(&rhs) != Ordering::Less
}

/// The party's draws: hypergeometric over the places seen, or binomial with the level's share.
enum Dist {
    Hyper { places: usize, party: usize },
    Binomial { level: u32 },
}

impl Dist {
    fn new(level: u8, places: u32) -> Dist {
        if places > EXACT_UP_TO {
            Dist::Binomial { level: level as u32 }
        } else {
            Dist::Hyper { places: places as usize, party: party(level, places) as usize }
        }
    }

    /// Indexes cannot outnumber places: one per place per generation.
    fn cap(&self, m: usize) -> usize {
        match *self {
            Dist::Hyper { places, .. } => m.min(places),
            Dist::Binomial { .. } => m,
        }
    }

    /// P(X ≥ i) for every i from `lo` to `draws`, X the party's share of `draws`.
    fn tails(&self, draws: usize, lo: usize) -> Tails {
        match *self {
            Dist::Binomial { level } => binomial_tails(draws, level, 100, lo),
            Dist::Hyper { places, party } => hyper_tails(places, party, draws, lo),
        }
    }
}

/// Tail probabilities as exact fractions over one denominator: `cum[j]` is the numerator of
/// P(X ≥ lo + j).
struct Tails {
    lo: usize,
    cum: Vec<Big>,
    den: Big,
}

impl Tails {
    /// Whether P(X ≥ i) ≤ 1 / eps_den.
    fn at_most(&self, i: usize, eps_den: &Big) -> bool {
        let num = match i.checked_sub(self.lo).and_then(|j| self.cum.get(j)) {
            Some(n) => n,
            None if i > self.lo => return true, // beyond every draw: impossible
            None => unreachable!("tail asked below its lower bound"),
        };
        num.mul(eps_den).cmp(&self.den) != Ordering::Greater
    }
}

/// C(n, i) for i in 0..=upto.
fn binomial_row(n: usize, upto: usize) -> Vec<Big> {
    let mut row = Vec::with_capacity(upto + 1);
    let mut c = Big::from(1);
    row.push(c.clone());
    for i in 0..upto.min(n) {
        c.mul_small((n - i) as u32);
        c.div_small_exact((i + 1) as u32);
        row.push(c.clone());
    }
    row.resize(upto + 1, Big::zero());
    row
}

fn binomial_tails(m: usize, a: u32, b: u32, lo: usize) -> Tails {
    let c = b - a;
    let mut pa = vec![Big::from(1)];
    let mut pc = vec![Big::from(1)];
    for i in 1..=m {
        let mut x = pa[i - 1].clone();
        x.mul_small(a);
        pa.push(x);
        let mut y = pc[i - 1].clone();
        y.mul_small(c);
        pc.push(y);
    }
    let row = binomial_row(m, m);
    let mut den = Big::from(1);
    for _ in 0..m {
        den.mul_small(b);
    }
    let lo = lo.min(m + 1);
    let mut cum = vec![Big::zero(); m + 1 - lo + usize::from(lo > m)];
    let mut acc = Big::zero();
    for i in (lo..=m).rev() {
        acc.add(&row[i].mul(&pa[i]).mul(&pc[m - i]));
        cum[i - lo] = acc.clone();
    }
    Tails { lo, cum, den }
}

fn hyper_tails(places: usize, party: usize, draws: usize, lo: usize) -> Tails {
    let honest = places - party;
    let ca = binomial_row(party, draws);
    let ch = binomial_row(honest, draws);
    let den = binomial_row(places, draws)[draws].clone();
    let lo = lo.min(draws + 1);
    let mut cum = vec![Big::zero(); draws + 1 - lo + usize::from(lo > draws)];
    let mut acc = Big::zero();
    for i in (lo..=draws).rev() {
        if i <= party && draws - i <= honest {
            acc.add(&ca[i].mul(&ch[draws - i]));
        }
        cum[i - lo] = acc.clone();
    }
    Tails { lo, cum, den }
}

/// Just enough unsigned big integer for exact tails: little-endian 32-bit limbs, no trailing
/// zero limbs.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Big(Vec<u32>);

impl Big {
    fn zero() -> Big {
        Big(Vec::new())
    }

    fn from(x: u64) -> Big {
        let mut b = Big(vec![x as u32, (x >> 32) as u32]);
        b.trim();
        b
    }

    fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    fn mul_small(&mut self, x: u32) {
        let mut carry = 0u64;
        for l in &mut self.0 {
            let v = *l as u64 * x as u64 + carry;
            *l = v as u32;
            carry = v >> 32;
        }
        if carry > 0 {
            self.0.push(carry as u32);
        }
        self.trim();
    }

    /// Divides by `x`, which must divide it exactly (binomial rows only ever do).
    fn div_small_exact(&mut self, x: u32) {
        let mut rem = 0u64;
        for l in self.0.iter_mut().rev() {
            let v = (rem << 32) | *l as u64;
            *l = (v / x as u64) as u32;
            rem = v % x as u64;
        }
        debug_assert_eq!(rem, 0, "inexact division");
        self.trim();
    }

    fn shl(&mut self, bits: usize) {
        let (limbs, bits) = (bits / 32, bits % 32);
        if bits > 0 {
            let mut carry = 0u32;
            for l in &mut self.0 {
                let v = (*l as u64) << bits;
                *l = v as u32 | carry;
                carry = (v >> 32) as u32;
            }
            if carry > 0 {
                self.0.push(carry);
            }
        }
        if !self.0.is_empty() {
            self.0.splice(0..0, std::iter::repeat_n(0, limbs));
        }
    }

    fn add(&mut self, o: &Big) {
        if self.0.len() < o.0.len() {
            self.0.resize(o.0.len(), 0);
        }
        let mut carry = 0u64;
        for (i, l) in self.0.iter_mut().enumerate() {
            let v = *l as u64 + *o.0.get(i).unwrap_or(&0) as u64 + carry;
            *l = v as u32;
            carry = v >> 32;
        }
        if carry > 0 {
            self.0.push(carry as u32);
        }
    }

    fn mul(&self, o: &Big) -> Big {
        if self.0.is_empty() || o.0.is_empty() {
            return Big::zero();
        }
        let mut out = vec![0u32; self.0.len() + o.0.len()];
        for (i, &a) in self.0.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in o.0.iter().enumerate() {
                let v = out[i + j] as u64 + a as u64 * b as u64 + carry;
                out[i + j] = v as u32;
                carry = v >> 32;
            }
            let mut k = i + o.0.len();
            while carry > 0 {
                let v = out[k] as u64 + carry;
                out[k] = v as u32;
                carry = v >> 32;
                k += 1;
            }
        }
        let mut b = Big(out);
        b.trim();
        b
    }

    fn cmp(&self, o: &Big) -> Ordering {
        self.0.len().cmp(&o.0.len()).then_with(|| self.0.iter().rev().cmp(o.0.iter().rev()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MORE: u32 = u32::MAX;

    fn s(k: usize, n: usize, m: usize) -> Option<Size> {
        Some(Size { k, n, m })
    }

    #[test]
    fn big_arithmetic() {
        let mut a = Big::from(u64::MAX);
        a.mul_small(u32::MAX);
        let mut b = Big::from(u64::MAX);
        b.shl(32);
        assert_eq!(a.cmp(&b), Ordering::Less);
        let c = Big::from(1 << 40).mul(&Big::from(1 << 40));
        let mut d = Big::from(1);
        d.shl(80);
        assert_eq!(c, d);
        let mut e = d.clone();
        e.div_small_exact(1 << 16);
        let mut f = Big::from(1);
        f.shl(64);
        assert_eq!(e, f);
        let mut g = Big::from(u32::MAX as u64);
        g.add(&Big::from(1));
        assert_eq!(g, Big::from(1 << 32));
        // C(100, 50) = 100891344545564193334812497256, and Pascal's rule holds along a row
        let row = binomial_row(100, 50);
        let want = 100891344545564193334812497256u128;
        assert_eq!(row[50], Big(vec![want as u32, (want >> 32) as u32, (want >> 64) as u32, (want >> 96) as u32]));
        let prev = binomial_row(99, 50);
        let mut pascal = prev[49].clone();
        pascal.add(&prev[50]);
        assert_eq!(row[50], pascal);
        assert_eq!(binomial_row(5, 8)[6], Big::zero());
    }

    #[test]
    fn the_tables_of_the_plan() {
        // SCATTER.md §5.4, large networks
        assert_eq!(size(10, 0, MORE), s(15, 28, 35));
        assert_eq!(size(15, 0, MORE), s(24, 44, 55));
        assert_eq!(size(20, 0, MORE), s(39, 71, 89));
        assert_eq!(size(10, 1, MORE), s(16, 29, 37));
        assert_eq!(size(10, 8, MORE), s(22, 40, 50));
        // and by places seen
        assert_eq!(size(10, 0, 11), None);
        assert_eq!(size(10, 0, 12), s(5, 12, 12));
        assert_eq!(size(20, 0, 30), s(7, 15, 19));
        assert_eq!(size(10, 0, 90), s(10, 20, 25));
        assert_eq!(size(10, 0, 100), s(10, 20, 25));
        assert_eq!(size(15, 0, 1000), s(23, 42, 53));
    }

    #[test]
    fn hot_objects_grow_k_with_n() {
        // twice the pieces at the default: k rises with n, and the bounds still hold
        let d = size(10, 1, MORE).unwrap();
        let hot = size_at_least(10, 1, MORE, 2 * d.n).unwrap();
        assert!(hot.n >= 2 * d.n && hot.k > d.k, "{hot:?}");
        assert!(meets(10, 1, MORE, hot));
        assert!(hot.n <= LINKS * (hot.k - 1));
        // nothing below the plain size; nothing at all where n cannot fit the places
        assert_eq!(size_at_least(10, 0, MORE, 0), size(10, 0, MORE));
        assert_eq!(size_at_least(10, 0, 30, 31), None);
    }

    #[test]
    fn hard_bounds_in_small_networks() {
        let small = size(10, 0, 80).unwrap();
        assert!(hard(10, 80, small.k));
        let big = size(10, 0, 100).unwrap();
        assert!(!hard(10, 100, big.k));
        assert!(!hard(10, MORE, 15));
    }

    #[test]
    fn readings() {
        let d = size(10, 0, MORE).unwrap();
        assert_eq!(reading(0, MORE, d), Some(10));
        assert_eq!(reading(0, MORE, size(20, 0, MORE).unwrap()), Some(20));
        // the first draft's 3 of 6 meets no level
        assert_eq!(reading(0, MORE, Size { k: 3, n: 6, m: 6 }), None);
        // a size that meets generation 0 may not meet a later generation's tighter budget
        assert!(meets(10, 0, MORE, d));
        assert!(!meets(10, 8, MORE, d));
        // nonsense sizes meet nothing
        assert!(!meets(10, 0, 50, Size { k: 6, n: 14, m: 60 }));
        assert!(!meets(10, 0, MORE, Size { k: 9, n: 8, m: 9 }));
    }

    #[test]
    fn sizes_obey_their_own_rules() {
        for &level in &LEVELS {
            for places in [12u32, 20, 40, 77, 100, 333, 1000, MORE] {
                for g in [0u8, 3] {
                    let Some(x) = size(level, g, places) else { continue };
                    assert!(x.k >= 3 && x.k + 2 <= x.n && x.n <= x.m && x.m <= budget(x.n));
                    assert!(x.n <= LINKS * (x.k - 1), "three links carry it");
                    assert!(available(x.n, x.k));
                    assert!(meets(level, g, places, x));
                    assert!(places > EXACT_UP_TO || x.m <= places as usize);
                }
            }
        }
    }
}
