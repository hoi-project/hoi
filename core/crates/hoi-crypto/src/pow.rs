//! BLAKE3 hashcash stamps (PROTOCOL.md §4).
//!
//! `valid iff leading_zero_bits(BLAKE3(id ‖ u64be(nonce))) >= bits_required`, where `id` is the
//! record id, which itself covers the nonce. Mining hashes the canonical prefix once and only
//! re-hashes `nonce ‖ suffix` per try.

use hoi_types::{Id, Record, cbor};

pub fn leading_zero_bits(h: &[u8; 32]) -> u32 {
    let mut n = 0;
    for b in h {
        if *b == 0 {
            n += 8;
        } else {
            n += b.leading_zeros();
            break;
        }
    }
    n
}

pub fn stamp_hash(id: &Id, nonce: u64) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(id.as_bytes());
    h.update(&nonce.to_be_bytes());
    *h.finalize().as_bytes()
}

/// Bits of work a record actually carries.
pub fn stamp_bits(id: &Id, nonce: u64) -> u32 {
    leading_zero_bits(&stamp_hash(id, nonce))
}

/// The bytes a miner needs. `encode(record with nonce n) == prefix ‖ cbor_uint(n) ‖ suffix`.
#[derive(Debug, Clone)]
pub struct Puzzle {
    pub prefix: Vec<u8>,
    pub suffix: Vec<u8>,
    pub bits: u32,
}

impl Puzzle {
    pub fn new(record: &Record, bits: u32) -> Puzzle {
        let (prefix, suffix) = record.split_for_nonce();
        Puzzle { prefix, suffix, bits }
    }

    /// Tries `start, start+step, …` for at most `tries` nonces. Returns the first nonce that works.
    pub fn search(&self, start: u64, step: u64, tries: u64) -> Option<u64> {
        let mut base = blake3::Hasher::new();
        base.update(&self.prefix);
        let mut nonce = start;
        let mut buf = Vec::with_capacity(9);
        for _ in 0..tries {
            buf.clear();
            cbor::head(0, nonce, &mut buf);
            let mut h = base.clone();
            h.update(&buf);
            h.update(&self.suffix);
            let id = Id(*h.finalize().as_bytes());
            if stamp_bits(&id, nonce) >= self.bits {
                return Some(nonce);
            }
            nonce = nonce.checked_add(step)?;
        }
        None
    }
}

/// Mines on every core (native only). Returns the nonce.
#[cfg(not(target_arch = "wasm32"))]
pub fn mine(record: &Record, bits: u32) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    let p = Puzzle::new(record, bits);
    if bits <= 12 {
        return p.search(0, 1, u64::MAX).expect("nonce space is large enough");
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16) as u64;
    let found = AtomicU64::new(u64::MAX);
    std::thread::scope(|s| {
        for t in 0..threads {
            let p = &p;
            let found = &found;
            s.spawn(move || {
                let mut start = t;
                const CHUNK: u64 = 4096;
                loop {
                    if found.load(Ordering::Relaxed) != u64::MAX {
                        return;
                    }
                    if let Some(n) = p.search(start, threads, CHUNK) {
                        found.fetch_min(n, Ordering::Relaxed);
                        return;
                    }
                    start += threads * CHUNK;
                }
            });
        }
    });
    found.load(Ordering::Relaxed)
}

/// Single-threaded mining for targets without threads.
pub fn mine_single(record: &Record, bits: u32) -> u64 {
    Puzzle::new(record, bits).search(0, 1, u64::MAX).expect("nonce space is large enough")
}

/// Hashes per second on this machine for one thread (used for "about a second of work").
#[cfg(not(target_arch = "wasm32"))]
pub fn benchmark(tries: u64) -> f64 {
    let p = Puzzle { prefix: vec![0xa9; 180], suffix: vec![0x01; 60], bits: 255 };
    let t = std::time::Instant::now();
    let _ = p.search(0, 1, tries);
    tries as f64 / t.elapsed().as_secs_f64().max(1e-9)
}

/// Required bits for a record (PROTOCOL.md §4): community difficulty minus the kind discount
/// minus the key-age discount.
pub fn required_bits(pow_bits: u32, kind_discount: u32, key_age_secs: u64) -> u32 {
    let d = key_age_secs / 86_400;
    let age = if d < 30 {
        0
    } else if d < 90 {
        2
    } else if d < 365 {
        4
    } else {
        6
    };
    pow_bits.saturating_sub(kind_discount).saturating_sub(age)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_types::{Body, Record};

    fn rec() -> Record {
        Record {
            v: 1,
            community: Some(Id([1; 32])),
            author: Id([2; 32]),
            parent: None,
            created_at: 1_790_000_000,
            expires_at: 1_790_086_400,
            body: Body::Post { title: "t".into(), text: "x".repeat(500), link: None, name: None, media: Vec::new() },
            nonce: 0,
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))] // the threaded miners are native only
    fn mined_stamp_verifies() {
        let mut r = rec();
        let n = mine(&r, 12);
        r.nonce = n;
        assert!(stamp_bits(&r.id(), n) >= 12);
        let mut r2 = rec();
        r2.nonce = mine_single(&r2, 10);
        assert!(stamp_bits(&r2.id(), r2.nonce) >= 10);
    }

    #[test]
    fn age_discount() {
        assert_eq!(required_bits(22, 0, 0), 22);
        assert_eq!(required_bits(22, 8, 40 * 86_400), 12);
        assert_eq!(required_bits(22, 0, 400 * 86_400), 16);
        assert_eq!(required_bits(4, 8, 0), 0);
    }

    #[test]
    fn lzb() {
        let mut h = [0u8; 32];
        assert_eq!(leading_zero_bits(&h), 256);
        h[1] = 0b0001_0000;
        assert_eq!(leading_zero_bits(&h), 11);
    }
}
