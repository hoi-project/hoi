//! Identity work (PROTOCOL.md §4.1): what it costs to be born on hoi.
//!
//! A root key is only an identity once it carries memory-hard proof of work:
//!
//! ```text
//! w    = Argon2id(password = root ‖ u64be(work), salt = "hoi/identity/v1\0",
//!                 m = 8 MiB, t = 1, p = 1, out = 32 bytes)
//! bits = leading_zero_bits(w)
//! ```
//!
//! The work depends on the key alone, so it is done once and stays with the key for life. It
//! does not transfer: a new key starts from nothing, in every community. Argon2id
//! makes each try cost memory, so graphics cards and server farms gain far less over a phone
//! than they would with a plain hash, and minting thousands of keys gets expensive.

use argon2::{Algorithm, Argon2, Block, Params, Version};
use hoi_types::Id;

use crate::pow::leading_zero_bits;

/// Memory per try, KiB. Fixed by the protocol so nobody can claim cheaper work.
pub const M_KIB: u32 = 8192;
pub const T_COST: u32 = 1;
pub const SALT: &[u8; 16] = b"hoi/identity/v1\0";
/// No identity carrying less than this is stored at all.
pub const MIN_BITS: u32 = 4;
/// What a new identity is mined to by default: well above the communities' default of 8.
pub const DEFAULT_BITS: u32 = 10;
/// Communities may ask for at most this much.
pub const MAX_BITS: u32 = 20;

fn argon() -> Argon2<'static> {
    let params = Params::new(M_KIB, T_COST, 1, Some(32)).expect("fixed parameters are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// One evaluation, reusing `mem` (allocate it with [`memory`]).
fn eval(a: &Argon2, root: &Id, work: u64, mem: &mut [Block]) -> [u8; 32] {
    let mut pwd = [0u8; 40];
    pwd[..32].copy_from_slice(root.as_bytes());
    pwd[32..].copy_from_slice(&work.to_be_bytes());
    let mut out = [0u8; 32];
    a.hash_password_into_with_memory(&pwd, SALT, &mut out, mem).expect("fixed sizes are valid");
    out
}

fn memory() -> Vec<Block> {
    vec![Block::default(); (M_KIB as usize).max(8)]
}

/// Tries nonces one at a time and hands back each output, so a host can show the work as it
/// happens (the web app's first run). Keeps one 8 MiB buffer for all its tries.
pub struct Miner {
    a: Argon2<'static>,
    root: Id,
    mem: Vec<Block>,
}

impl Miner {
    pub fn new(root: Id) -> Miner {
        Miner { a: argon(), root, mem: memory() }
    }

    /// `w` for this nonce; its leading zero bits are the work the nonce carries.
    pub fn hash(&mut self, work: u64) -> [u8; 32] {
        eval(&self.a, &self.root, work, &mut self.mem)
    }
}

/// The identity work a key carries with this nonce.
pub fn work_bits(root: &Id, work: u64) -> u32 {
    leading_zero_bits(&Miner::new(*root).hash(work))
}

/// Tries `start, start+step, …` for at most `tries` nonces.
pub fn search(root: &Id, bits: u32, start: u64, step: u64, tries: u64) -> Option<u64> {
    let mut m = Miner::new(*root);
    let mut n = start;
    for _ in 0..tries {
        if leading_zero_bits(&m.hash(n)) >= bits {
            return Some(n);
        }
        n = n.checked_add(step)?;
    }
    None
}

/// Mines identity work on every core (native) or one (wasm). Returns the nonce.
#[cfg(not(target_arch = "wasm32"))]
pub fn mine(root: &Id, bits: u32) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16) as u64;
    if bits <= 4 || threads == 1 {
        return search(root, bits, 0, 1, u64::MAX).expect("nonce space is large enough");
    }
    let found = AtomicU64::new(u64::MAX);
    std::thread::scope(|s| {
        for t in 0..threads {
            let found = &found;
            s.spawn(move || {
                let mut start = t;
                const CHUNK: u64 = 8;
                loop {
                    if found.load(Ordering::Relaxed) != u64::MAX {
                        return;
                    }
                    if let Some(n) = search(root, bits, start, threads, CHUNK) {
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

/// What a host watches while [`mine_watched`] runs, and how it calls the work off.
#[derive(Debug, Default)]
pub struct Watch {
    /// Set it and the miners stop after their current try.
    pub stop: std::sync::atomic::AtomicBool,
    /// Tries so far, across every core.
    pub tries: std::sync::atomic::AtomicU64,
    /// The most leading zero bits any try reached so far.
    pub best: std::sync::atomic::AtomicU32,
}

/// Like [`mine`], but every try is counted in `watch` and setting `watch.stop` ends it early
/// (then it returns `None`): for more work on a key that has some, which can take minutes.
#[cfg(not(target_arch = "wasm32"))]
pub fn mine_watched(root: &Id, bits: u32, watch: &Watch) -> Option<u64> {
    use std::sync::atomic::{AtomicU64, Ordering};
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16) as u64;
    let found = AtomicU64::new(u64::MAX);
    std::thread::scope(|s| {
        for t in 0..threads {
            let found = &found;
            s.spawn(move || {
                let mut m = Miner::new(*root);
                let mut n = t;
                while found.load(Ordering::Relaxed) == u64::MAX && !watch.stop.load(Ordering::Relaxed) {
                    let got = leading_zero_bits(&m.hash(n));
                    watch.tries.fetch_add(1, Ordering::Relaxed);
                    watch.best.fetch_max(got, Ordering::Relaxed);
                    if got >= bits {
                        found.fetch_min(n, Ordering::Relaxed);
                        return;
                    }
                    let Some(next) = n.checked_add(threads) else { return };
                    n = next;
                }
            });
        }
    });
    Some(found.load(Ordering::Relaxed)).filter(|&n| n != u64::MAX)
}

#[cfg(target_arch = "wasm32")]
pub fn mine(root: &Id, bits: u32) -> u64 {
    search(root, bits, 0, 1, u64::MAX).expect("nonce space is large enough")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mined_work_verifies_and_is_bound_to_the_key() {
        let k = Id([3; 32]);
        let n = mine(&k, 5);
        assert!(work_bits(&k, n) >= 5);
        // the same nonce means nothing for another key (with overwhelming odds)
        let other = Id([4; 32]);
        let mut hits = 0;
        for w in 0..8 {
            if work_bits(&other, n + w) >= 5 {
                hits += 1;
            }
        }
        assert!(hits < 8);
        assert_eq!(search(&k, 5, n, 1, 1), Some(n));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))] // the threaded miners are native only
    fn watched_work_is_counted_and_can_be_stopped() {
        use std::sync::atomic::Ordering;
        let k = Id([5; 32]);
        let w = Watch::default();
        let n = mine_watched(&k, 4, &w).unwrap();
        assert!(work_bits(&k, n) >= 4);
        assert!(w.tries.load(Ordering::Relaxed) >= 1);
        assert!(w.best.load(Ordering::Relaxed) >= 4);
        let stopped = Watch::default();
        stopped.stop.store(true, Ordering::Relaxed);
        assert_eq!(mine_watched(&k, 30, &stopped), None, "called off before it began");
        assert_eq!(stopped.tries.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn one_try_at_a_time_matches_the_search() {
        let k = Id([7; 32]);
        let mut m = Miner::new(k);
        let n = (0..).find(|&w| leading_zero_bits(&m.hash(w)) >= 4).unwrap();
        assert_eq!(search(&k, 4, 0, 1, n + 1), Some(n));
        assert!(work_bits(&k, n) >= 4);
        // the buffer is reused, the output isn't: trying again gives the same hash
        assert_eq!(m.hash(n), Miner::new(k).hash(n));
        assert_ne!(m.hash(n), m.hash(n + 1));
    }
}
