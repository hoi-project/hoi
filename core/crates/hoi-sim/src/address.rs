//! Who can link a device's address to its records (SCATTER.md §10, phase S7's acceptance).
//!
//! A device sends through forwarders it links to; a party runs a share of the public nodes. What
//! links an address to a record:
//!
//! - **its own records** (a post's pointer, a vote, a reply): before S7 the device offered each
//!   new one to its own links, so the first link to get it had both; since S7 it goes sealed
//!   through an entry link to an injector, another of its links, which passes it on as if a peer
//!   had sent it: linking needs the entry (the address) and the injector (the record) together;
//! - **its uploads** (pieces): with two hops the first forwarder saw the device's address and each
//!   holder, which anyone who read the post can match to its manifest; with three hops the entry
//!   sees the address and the exit, the exit the holders and the entry: again two nodes together.
//!
//! Scored over sampled devices and records: the share a single node links by itself, and the
//! share the party links with all its nodes together.

/// xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Linked {
    /// Records a single node could link to the device's address by itself.
    pub single: f64,
    /// Records the party could link with every node it runs.
    pub party: f64,
}

/// `records` of a device's own and `uploads` of its pieces, each through nodes the party runs
/// with probability `share`, before S7 (`s7: false`) and since.
pub fn linked(share: f64, s7: bool, trials: u32, seed: u64) -> (Linked, Linked) {
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let (mut rs, mut rp, mut us, mut up) = (0u32, 0u32, 0u32, 0u32);
    for _ in 0..trials {
        // its own record
        if s7 {
            let (entry, injector) = (rng.chance(share), rng.chance(share));
            rp += u32::from(entry && injector);
        } else {
            // the first link it offers it to has both
            rs += 1;
            rp += u32::from(rng.chance(share));
        }
        // one of its uploads
        if s7 {
            let (entry, exit) = (rng.chance(share), rng.chance(share));
            up += u32::from(entry && exit);
        } else {
            let first = rng.chance(share);
            us += 1;
            up += u32::from(first);
        }
    }
    let t = trials.max(1) as f64;
    (Linked { single: rs as f64 / t, party: rp as f64 / t }, Linked { single: us as f64 / t, party: up as f64 / t })
}

/// The report `hoi-sim scatter --address` prints.
pub fn report(trials: u32, seed: u64) -> Vec<String> {
    let mut out = vec!["party   when      own records: one node  the party   uploads: one node  the party".to_string()];
    for share in [0.05, 0.10, 0.20] {
        for s7 in [false, true] {
            let (r, u) = linked(share, s7, trials, seed);
            out.push(format!(
                "{:>4.0}%   {:<8}  {:>21.4}  {:>9.4}  {:>17.4}  {:>9.4}",
                share * 100.0,
                if s7 { "since S7" } else { "before" },
                r.single,
                r.party,
                u.single,
                u.party
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_s7_no_single_node_links_an_address_to_a_record() {
        for share in [0.05, 0.1, 0.2] {
            let (before_r, before_u) = linked(share, false, 20_000, 3);
            let (r, u) = linked(share, true, 20_000, 3);
            assert_eq!((before_r.single, before_u.single), (1.0, 1.0), "before, the first link always could");
            assert_eq!((r.single, u.single), (0.0, 0.0), "since, no node alone");
            // the party needs two of its nodes on one path: about the square of its share
            assert!((r.party - share * share).abs() < 0.01 && (u.party - share * share).abs() < 0.01, "{share}: {} {}", r.party, u.party);
        }
    }
}
