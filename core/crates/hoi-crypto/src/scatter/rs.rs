//! Systematic Reed–Solomon over GF(2⁸) (SCATTER.md §5.3, PROTOCOL.md §21).
//!
//! The field is GF(2)[x] / (x⁸ + x⁴ + x³ + x² + 1) (0x11D) with generator 2. The code is the one
//! Backblaze published and many libraries share: the m × k Vandermonde matrix with row r =
//! `[r⁰, r¹, …, r^(k−1)]` (0⁰ = 1), multiplied on the right by the inverse of its top k × k
//! block, so rows 0 … k−1 are the identity (pieces 0 … k−1 are the data itself) and any k
//! distinct rows are invertible. Row r depends only on r and k, never on how many pieces exist,
//! which is what lets repair make fresh indexes later that combine with the old ones.
//!
//! Arithmetic only, no secrets and no `unsafe`: everything here runs on ciphertext.

/// The most pieces one codeword can have: one per field element.
pub const MAX_INDEXES: usize = 256;

const POLY: u16 = 0x11D;

struct Tables {
    exp: [u8; 512],
    log: [u8; 256],
}

const TABLES: Tables = {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= POLY;
        }
        i += 1;
    }
    while i < 512 {
        exp[i] = exp[i - 255];
        i += 1;
    }
    Tables { exp, log }
};

pub(crate) fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    TABLES.exp[TABLES.log[a as usize] as usize + TABLES.log[b as usize] as usize]
}

fn inv(a: u8) -> u8 {
    debug_assert!(a != 0, "zero has no inverse");
    TABLES.exp[255 - TABLES.log[a as usize] as usize]
}

/// `a^n`, with 0⁰ = 1.
fn pow(a: u8, n: usize) -> u8 {
    if n == 0 {
        return 1;
    }
    if a == 0 {
        return 0;
    }
    TABLES.exp[(TABLES.log[a as usize] as usize * n) % 255]
}

/// A k × k matrix, row-major.
#[derive(Clone)]
struct Square {
    k: usize,
    a: Vec<u8>,
}

impl Square {
    fn vandermonde_rows(rows: &[usize], k: usize) -> Square {
        let mut a = vec![0u8; rows.len() * k];
        for (i, &r) in rows.iter().enumerate() {
            for c in 0..k {
                a[i * k + c] = pow(r as u8, c);
            }
        }
        Square { k, a }
    }

    /// Gauss–Jordan inversion. Returns `None` for a singular matrix, which distinct
    /// Vandermonde rows never are.
    fn invert(&self) -> Option<Square> {
        let k = self.k;
        let mut m = self.a.clone();
        let mut out = vec![0u8; k * k];
        for i in 0..k {
            out[i * k + i] = 1;
        }
        for col in 0..k {
            let pivot = (col..k).find(|&r| m[r * k + col] != 0)?;
            if pivot != col {
                for c in 0..k {
                    m.swap(pivot * k + c, col * k + c);
                    out.swap(pivot * k + c, col * k + c);
                }
            }
            let p = inv(m[col * k + col]);
            for c in 0..k {
                m[col * k + c] = mul(m[col * k + c], p);
                out[col * k + c] = mul(out[col * k + c], p);
            }
            for r in 0..k {
                let f = m[r * k + col];
                if r == col || f == 0 {
                    continue;
                }
                for c in 0..k {
                    m[r * k + c] ^= mul(f, m[col * k + c]);
                    out[r * k + c] ^= mul(f, out[col * k + c]);
                }
            }
        }
        Some(Square { k, a: out })
    }
}

/// The code for one k: the inverse of the top Vandermonde block, from which any row follows.
pub struct Code {
    k: usize,
    top_inv: Square,
}

impl Code {
    /// `k` from 1 to 256.
    pub fn new(k: usize) -> Code {
        assert!((1..=MAX_INDEXES).contains(&k), "k out of range");
        let rows: Vec<usize> = (0..k).collect();
        let top_inv = Square::vandermonde_rows(&rows, k).invert().expect("Vandermonde rows are invertible");
        Code { k, top_inv }
    }

    pub fn k(&self) -> usize {
        self.k
    }

    /// Row `r` of the systematic matrix: the coefficients of piece `r` over the k data pieces.
    pub fn row(&self, r: usize) -> Vec<u8> {
        assert!(r < MAX_INDEXES, "index out of range");
        let k = self.k;
        if r < k {
            let mut e = vec![0u8; k];
            e[r] = 1;
            return e;
        }
        let v = Square::vandermonde_rows(&[r], k).a;
        (0..k).map(|c| (0..k).fold(0u8, |acc, j| acc ^ mul(v[j], self.top_inv.a[j * k + c]))).collect()
    }

    /// Piece `index` from the k data pieces (all the same length).
    pub fn encode(&self, data: &[&[u8]], index: usize) -> Vec<u8> {
        assert_eq!(data.len(), self.k, "k data pieces");
        let len = data[0].len();
        assert!(data.iter().all(|d| d.len() == len), "data pieces of one length");
        if index < self.k {
            return data[index].to_vec();
        }
        let mut out = vec![0u8; len];
        for (coef, d) in self.row(index).into_iter().zip(data) {
            add_scaled(&mut out, d, coef);
        }
        out
    }

    /// The k data pieces from any k pieces with distinct indexes (all the same length).
    /// Returns `None` if the indexes are not k distinct values below 256 or lengths differ.
    pub fn decode(&self, pieces: &[(usize, &[u8])]) -> Option<Vec<Vec<u8>>> {
        let k = self.k;
        if pieces.len() != k {
            return None;
        }
        let len = pieces[0].1.len();
        let mut seen = [false; MAX_INDEXES];
        for &(i, p) in pieces {
            if i >= MAX_INDEXES || seen[i] || p.len() != len {
                return None;
            }
            seen[i] = true;
        }
        // All data pieces present: nothing to solve.
        if (0..k).all(|i| seen[i]) {
            let mut out = vec![Vec::new(); k];
            for &(i, p) in pieces {
                out[i] = p.to_vec();
            }
            return Some(out);
        }
        let mut sub = Vec::with_capacity(k * k);
        for &(i, _) in pieces {
            sub.extend(self.row(i));
        }
        let solve = Square { k, a: sub }.invert()?;
        let mut out = vec![vec![0u8; len]; k];
        for (r, o) in out.iter_mut().enumerate() {
            for (c, &(_, p)) in pieces.iter().enumerate() {
                add_scaled(o, p, solve.a[r * k + c]);
            }
        }
        Some(out)
    }
}

/// `out ^= coef · src`, byte by byte, through a 256-entry table for `coef`.
fn add_scaled(out: &mut [u8], src: &[u8], coef: u8) {
    match coef {
        0 => {}
        1 => out.iter_mut().zip(src).for_each(|(o, s)| *o ^= s),
        _ => {
            let mut t = [0u8; 256];
            for (x, e) in t.iter_mut().enumerate() {
                *e = mul(coef, x as u8);
            }
            out.iter_mut().zip(src).for_each(|(o, &s)| *o ^= t[s as usize]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn field_basics() {
        for a in 1..=255u8 {
            assert_eq!(mul(a, inv(a)), 1, "{a}");
            assert_eq!(mul(a, 1), a);
            assert_eq!(mul(a, 0), 0);
        }
        // 2⁸ = x⁴ + x³ + x² + 1 under 0x11D
        assert_eq!(pow(2, 8), 0x1D);
        assert_eq!(pow(0, 0), 1);
        assert_eq!(pow(0, 3), 0);
        // the generator has order 255
        assert!((1..255).all(|n| pow(2, n) != 1));
        assert_eq!(pow(2, 255), 1);
    }

    #[test]
    fn rows_are_systematic_and_independent_of_count() {
        let code = Code::new(4);
        for r in 0..4 {
            let mut e = vec![0u8; 4];
            e[r] = 1;
            assert_eq!(code.row(r), e);
        }
        // a row never depends on how many pieces are made: it is a function of r and k alone
        assert_eq!(code.row(9), Code::new(4).row(9));
        assert_ne!(code.row(9), code.row(10));
    }

    fn sample(k: usize, len: usize, seed: u8) -> Vec<Vec<u8>> {
        (0..k).map(|i| (0..len).map(|j| (j as u8).wrapping_mul(31).wrapping_add((i as u8).wrapping_mul(7)).wrapping_add(seed)).collect()).collect()
    }

    #[test]
    fn every_k_subset_of_small_codes_rebuilds() {
        for k in 1..=5usize {
            let m = k + 4;
            let data = sample(k, 13, k as u8);
            let refs: Vec<&[u8]> = data.iter().map(|d| d.as_slice()).collect();
            let code = Code::new(k);
            let pieces: Vec<Vec<u8>> = (0..m).map(|i| code.encode(&refs, i)).collect();
            // every subset of size k
            for mask in 0u32..(1 << m) {
                if mask.count_ones() as usize != k {
                    continue;
                }
                let chosen: Vec<(usize, &[u8])> = (0..m).filter(|i| mask & (1 << i) != 0).map(|i| (i, pieces[i].as_slice())).collect();
                assert_eq!(code.decode(&chosen).unwrap(), data, "k {k} mask {mask:b}");
            }
        }
    }

    #[test]
    fn decode_refuses_bad_sets() {
        let code = Code::new(3);
        let p = [1u8, 2, 3];
        assert!(code.decode(&[(0, &p[..]), (1, &p[..])]).is_none(), "too few");
        assert!(code.decode(&[(0, &p[..]), (0, &p[..]), (1, &p[..])]).is_none(), "repeated index");
        assert!(code.decode(&[(0, &p[..]), (1, &p[..2]), (4, &p[..])]).is_none(), "lengths differ");
        assert!(code.decode(&[(0, &p[..]), (1, &p[..]), (256, &p[..])]).is_none(), "index out of range");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]
        #[test]
        fn any_k_of_many_rebuild(k in 1usize..40, extra in 0usize..40, len in 1usize..64, seed in any::<u8>(), pick in any::<u64>()) {
            let m = (k + extra).min(MAX_INDEXES);
            let data = sample(k, len, seed);
            let refs: Vec<&[u8]> = data.iter().map(|d| d.as_slice()).collect();
            let code = Code::new(k);
            // choose k distinct indexes from 0..m, deterministically from `pick`
            let mut idx: Vec<usize> = (0..m).collect();
            let mut s = pick;
            for i in (1..idx.len()).rev() {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                idx.swap(i, (s >> 33) as usize % (i + 1));
            }
            idx.truncate(k);
            let pieces: Vec<(usize, Vec<u8>)> = idx.iter().map(|&i| (i, code.encode(&refs, i))).collect();
            let chosen: Vec<(usize, &[u8])> = pieces.iter().map(|(i, p)| (*i, p.as_slice())).collect();
            prop_assert_eq!(code.decode(&chosen).unwrap(), data);
        }

        #[test]
        fn highest_indexes_rebuild(k in 1usize..60, len in 1usize..32, seed in any::<u8>()) {
            let data = sample(k, len, seed);
            let refs: Vec<&[u8]> = data.iter().map(|d| d.as_slice()).collect();
            let code = Code::new(k);
            let pieces: Vec<(usize, Vec<u8>)> = (MAX_INDEXES - k..MAX_INDEXES).map(|i| (i, code.encode(&refs, i))).collect();
            let chosen: Vec<(usize, &[u8])> = pieces.iter().map(|(i, p)| (*i, p.as_slice())).collect();
            prop_assert_eq!(code.decode(&chosen).unwrap(), data);
        }
    }
}
