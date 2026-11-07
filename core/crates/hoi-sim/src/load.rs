//! Load at scale (SCATTER.md §13, §15; phase S6): one peak hour of a network with 10,000 to
//! 200,000 devices online, sampled, with the core's own sizing (`hoi_crypto::scatter::size`)
//! and size classes (`hoi_types::scatter::Class`).
//!
//! The network: one public node per hundred devices online (2,000 at 200,000), each a holder and
//! a forwarder, in a place of its own, linked to `links` others and vouching (`nodes.h`) for up
//! to `vouches` of them, as node lists do. Browsers and phones (70%) keep three forwarder links,
//! desktops (30%) six. A device reaches the holders its forwarders vouch for, and places a
//! generation among them (5.7); a reader fetches through its own forwarders, so it can rebuild an
//! object only if at least k of its holders are among those its own forwarders vouch for.
//!
//! Each device online writes a text post an hour (a 1.5 KiB record) and a picture every ten
//! hours (300 KB), and opens sixty posts and ten pictures an hour, k + 2 pieces each, two hops
//! each way. Scored per public node: piece operations and bytes a second (mean, and the busiest
//! node), storage a day at a 90-day life, sessions; per object: the bytes stored and moved
//! against its content (what padding and encoding cost); and the share of reads that find k
//! pieces through the reader's own forwarders (reach).
//!
//! Media objects are compared two ways: as the core made them before S6 (up to eleven 256 KiB
//! chunks in one object, base64 in JSON, padded to the next class) and fitted (raw bytes, each
//! object filled to a class it fits, as the core makes them since).

use hoi_crypto::scatter::size::{self, Size};
use hoi_types::scatter::Class;

/// xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

/// Devices online per public node.
const DEVICES_PER_NODE: u32 = 100;
const TEXT: usize = 1536;
const PICTURE: usize = 300_000;
const WRITES_PER_HOUR: f64 = 1.0;
const PICTURES_PER_HOUR: f64 = 0.1;
const READS_PER_HOUR: f64 = 60.0;
const PICTURE_READS_PER_HOUR: f64 = 10.0;
/// Bytes a sealed request adds (`put` header, `get`'s fixed 256 bytes, `got`'s header).
const PUT_OVERHEAD: usize = 177;
const GET_LEN: usize = 256;
const GOT_OVERHEAD: usize = 54;
const LIFE_DAYS: f64 = 90.0;

/// One configuration of the network.
#[derive(Debug, Clone, Copy)]
pub struct Load {
    pub online: u32,
    /// Peers a public node links to, and how many of them its node list vouches for.
    pub links: usize,
    pub vouches: usize,
    /// Media objects fitted to classes (since S6), or as before.
    pub fitted: bool,
    /// A forwarder that does not reach a holder passes the request to a link of its own that
    /// vouches for it (since S6): a reader reaches its forwarders' links' holders too.
    pub relay: bool,
    pub level: u8,
    pub seed: u64,
}

#[derive(Debug, Clone, Default)]
pub struct LoadOutcome {
    pub nodes: u32,
    /// The size a typical device chose: k, n, and the places it saw.
    pub size: Option<Size>,
    pub places_seen: u32,
    /// Writes that could be placed in pieces, and reads that found k pieces through the
    /// reader's own forwarders, as shares.
    pub placed: f64,
    pub reach: f64,
    /// Per public node: piece operations a second (mean, busiest), megabytes a second (mean),
    /// gigabytes stored after 90 days, sessions.
    pub ops: f64,
    pub ops_max: f64,
    pub mbps: f64,
    pub stored_gb: f64,
    pub sessions: f64,
    /// Bytes stored per byte of a picture, and bytes a reader downloads per byte of it.
    pub picture_stored: f64,
    pub picture_read: f64,
}

/// The content bytes of the objects a picture of `len` bytes goes in, and each object's class.
pub fn picture_objects(len: usize, fitted: bool) -> Vec<Class> {
    if fitted {
        // raw slices, each object as full as a class it fits, as the core cuts them since S6
        hoi_types::scatter::fit(len, 160)
    } else {
        // before S6: eleven 256 KiB chunks an object, base64 inside JSON
        let chunks = len.div_ceil(256 * 1024);
        (0..chunks.div_ceil(11))
            .map(|i| {
                let bytes = (len - i * 11 * 256 * 1024).min(11 * 256 * 1024);
                let json = bytes.div_ceil(3) * 4 + 200 * (bytes.div_ceil(256 * 1024));
                Class::for_len(json).expect("under 4 MiB")
            })
            .collect()
    }
}

impl Load {
    pub fn run(&self) -> LoadOutcome {
        let mut rng = Rng(self.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let nodes = (self.online / DEVICES_PER_NODE).max(12) as usize;
        // each public node's vouches: holders among its links
        let vouched: Vec<Vec<usize>> = (0..nodes)
            .map(|me| {
                let mut v = Vec::new();
                let want = self.links.min(nodes - 1).min(self.vouches);
                while v.len() < want {
                    let h = rng.below(nodes);
                    if h != me && !v.contains(&h) {
                        v.push(h);
                    }
                }
                v
            })
            .collect();
        // a sample of devices, each with its forwarders and the holders they reach
        let sample = 400usize;
        let devices: Vec<(Vec<usize>, Vec<usize>, Vec<usize>)> = (0..sample)
            .map(|_| {
                let links = if rng.chance(0.3) { 6 } else { 3 };
                let mut fw: Vec<usize> = Vec::new();
                while fw.len() < links.min(nodes) {
                    let f = rng.below(nodes);
                    if !fw.contains(&f) {
                        fw.push(f);
                    }
                }
                let mut reach: Vec<usize> = fw.iter().flat_map(|f| vouched[*f].iter().copied()).filter(|h| !fw.contains(h)).collect();
                reach.sort_unstable();
                reach.dedup();
                // what reading reaches: with the relay, the holders its forwarders' links vouch for
                let mut read: Vec<usize> = if self.relay {
                    fw.iter().flat_map(|f| vouched[*f].iter().flat_map(|l| vouched[*l].iter().copied()).chain(vouched[*f].iter().copied())).collect()
                } else {
                    reach.clone()
                };
                read.sort_unstable();
                read.dedup();
                (fw, reach, read)
            })
            .collect();
        let mut out = LoadOutcome { nodes: nodes as u32, ..Default::default() };
        // writes: each sampled device places a text object
        let mut objects: Vec<(Vec<usize>, Size)> = Vec::new();
        let mut placed = 0;
        let mut per_node_pieces = vec![0u64; nodes];
        for (_, reach, _) in &devices {
            let Some(s) = size::size(self.level, 0, reach.len() as u32) else { continue };
            let mut holders = reach.clone();
            for i in (1..holders.len()).rev() {
                holders.swap(i, rng.below(i + 1));
            }
            holders.truncate(s.n);
            for h in &holders {
                per_node_pieces[*h] += 1;
            }
            placed += 1;
            out.size.get_or_insert(s);
            out.places_seen = out.places_seen.max(reach.len() as u32);
            objects.push((holders, s));
        }
        out.placed = placed as f64 / devices.len() as f64;
        // reads: a device opens an object someone else placed
        let (mut tries, mut found) = (0u64, 0u64);
        for i in 0..devices.len() * 4 {
            if objects.is_empty() {
                break;
            }
            let (_, _, read) = &devices[i % devices.len()];
            let (holders, s) = &objects[rng.below(objects.len())];
            let reachable = holders.iter().filter(|h| read.binary_search(h).is_ok()).count();
            tries += 1;
            if reachable >= s.k {
                found += 1;
            }
        }
        out.reach = if tries == 0 { 0.0 } else { found as f64 / tries as f64 };
        // load, scaled from the sample to the whole hour
        let s = out.size.unwrap_or(Size { k: 15, n: 28, m: 35 });
        let (k, n) = (s.k, s.n);
        let online = self.online as f64;
        let text = Class::for_len(TEXT).expect("small");
        let pictures = picture_objects(PICTURE, self.fitted);
        let piece_ops_text = (WRITES_PER_HOUR * n as f64 + READS_PER_HOUR * (k + 2) as f64) * online / 3600.0;
        let piece_ops_pictures = pictures.len() as f64 * (PICTURES_PER_HOUR * n as f64 + PICTURE_READS_PER_HOUR * (k + 2) as f64) * online / 3600.0;
        // each piece operation passes a forwarder and reaches a holder: two hops
        out.ops = 2.0 * (piece_ops_text + piece_ops_pictures) / nodes as f64;
        let busiest = *per_node_pieces.iter().max().unwrap_or(&0) as f64;
        let mean = per_node_pieces.iter().sum::<u64>() as f64 / nodes as f64;
        out.ops_max = if mean > 0.0 { out.ops * busiest / mean } else { out.ops };
        let bytes = |c: &Class, writes: f64, reads: f64| {
            let piece = c.piece_len(k);
            writes * (n * (piece + PUT_OVERHEAD)) as f64 + reads * ((k + 2) * (piece + GOT_OVERHEAD + GET_LEN)) as f64
        };
        let per_hour = online
            * (bytes(&text, WRITES_PER_HOUR, READS_PER_HOUR) + pictures.iter().map(|c| bytes(c, PICTURES_PER_HOUR, PICTURE_READS_PER_HOUR)).sum::<f64>());
        out.mbps = 2.0 * per_hour / 3600.0 / nodes as f64 / 1e6;
        let stored_per_hour =
            online * (WRITES_PER_HOUR * (n * text.piece_len(k)) as f64 + PICTURES_PER_HOUR * pictures.iter().map(|c| (n * c.piece_len(k)) as f64).sum::<f64>());
        out.stored_gb = stored_per_hour * 24.0 * LIFE_DAYS / nodes as f64 / 1e9;
        out.sessions = (online * (0.7 * 3.0 + 0.3 * 6.0)) / nodes as f64;
        out.picture_stored = pictures.iter().map(|c| (n * c.piece_len(k)) as f64).sum::<f64>() / PICTURE as f64;
        out.picture_read = pictures.iter().map(|c| ((k + 2) * (c.piece_len(k) + GOT_OVERHEAD)) as f64).sum::<f64>() / PICTURE as f64;
        out
    }
}

/// The report `hoi-sim scatter --load` prints.
pub fn report(seed: u64) -> Vec<String> {
    let mut out =
        vec!["  online  nodes  vouches  relay  media    k/n     placed  reach   ops/s (busiest)  MB/s  GB stored  sessions  picture stored/read".to_string()];
    let mut row = |online: u32, vouches: usize, fitted: bool, relay: bool| {
        let l = Load { online, links: 64, vouches, fitted, relay, level: 10, seed };
        let o = l.run();
        let s = o.size.map(|s| format!("{}/{}", s.k, s.n)).unwrap_or_else(|| "-".into());
        out.push(format!(
            "{:>8}  {:>5}  {:>7}  {:>5}  {:>6}  {:>6}  {:>7.3}  {:>5.3}  {:>6.0} ({:>6.0})  {:>4.2}  {:>9.1}  {:>8.0}  {:>6.2}× / {:.2}×",
            online,
            o.nodes,
            vouches,
            if relay { "yes" } else { "" },
            if fitted { "fitted" } else { "before" },
            s,
            o.placed,
            o.reach,
            o.ops,
            o.ops_max,
            o.mbps,
            o.stored_gb,
            o.sessions,
            o.picture_stored,
            o.picture_read
        ));
    };
    // before S6
    for online in [10_000, 20_000, 50_000, 100_000, 200_000] {
        row(online, 32, false, false);
    }
    // wider vouch lists alone
    row(200_000, 64, true, false);
    // since S6: media fitted to classes, and one hop more where a reader's forwarders do not reach
    for online in [10_000, 20_000, 50_000, 100_000, 200_000] {
        row(online, 32, true, true);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_pictures_waste_little_and_cost_no_more_objects_than_needed() {
        let before = picture_objects(PICTURE, false);
        let fitted = picture_objects(PICTURE, true);
        let padded = |v: &[Class]| v.iter().map(|c| c.capacity()).sum::<usize>() as f64 / PICTURE as f64;
        assert_eq!(before.len(), 1);
        assert!(padded(&before) > 3.0, "base64 in JSON, padded to 1 MiB: {}", padded(&before));
        assert!(padded(&fitted) < 1.2, "fitted to classes: {}", padded(&fitted));
        assert!(fitted.len() <= 3);
        for len in [1, 4000, 70_000, 1 << 20, 3 << 20, 16 << 20] {
            let held: usize = picture_objects(len, true).iter().map(|c| c.capacity() - 160).sum();
            assert!(held >= len, "{len}");
        }
    }

    #[test]
    fn a_large_network_is_reached_only_with_one_hop_more() {
        let small = Load { online: 10_000, links: 64, vouches: 32, fitted: true, relay: false, level: 10, seed: 1 }.run();
        assert!(small.placed > 0.99);
        let large = Load { online: 200_000, links: 64, vouches: 32, fitted: true, relay: false, level: 10, seed: 1 }.run();
        assert!(large.reach < 0.05, "{}", large.reach);
        let relayed = Load { online: 200_000, links: 64, vouches: 32, fitted: true, relay: true, level: 10, seed: 1 }.run();
        assert!(relayed.reach > 0.99, "one hop more reaches nearly every object: {}", relayed.reach);
    }
}
