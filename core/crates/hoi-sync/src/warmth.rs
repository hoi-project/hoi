//! Warmth: who else holds a community's live records, read the way a torrent reads
//! availability (PROTOCOL.md §20). A community is **cold** when no other durable node holds
//! it (a lone desktop reads cold too: its own copy is the one at risk), **cool** when one
//! other independent place holds a copy, **warm** with two, **hot** with three or more that
//! hold everything this device knows of and caught up recently.
//!
//! Copies are counted by **place**, not by node: holders that share a declared operator or a
//! network (the same IPv4 /24, the same IPv6 /48, this very machine) are one place, because one
//! party or one outage takes them all. Ten nodes in one rack are one copy's worth of safety.
//! How well the links themselves are protected counts too: a copy reached only over a link that
//! is unencrypted or whose other end was never checked reads colder. People see one word and
//! one bar; the parts are named only in the details, in plain words.
//!
//! The same arithmetic reads the device's own connections ([`assess_links`]): how many
//! independent places it is linked through, whether any one of them carries most of its links,
//! and how many links are protected. That reading is shown as the warmth of "your connections".
//!
//! Everything here is a pure function of what one node observed first hand on its own links:
//! what connected peers handed over or took, what others listed that is not here yet, when the
//! last catch-up finished. Nothing is trusted from a peer's say-so beyond the one-hop hint a
//! node may attach to its catalog (`w`), which is taken as a maximum and never added up. A
//! warmth reading is not a delivery receipt and not a promise: it says how many copies this
//! device could see, not that any post reached anyone.

/// A connected peer that holds some of this community.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Holder {
    /// The share of this device's live records the peer showed it holds (handed over or taken
    /// on this link, or listed by a node this device dialed at a route it trusted), 0 to 1.
    /// A peer with records of a community this device holds nothing of yet counts as a full copy.
    pub coverage: f64,
    /// The peer says it keeps records across restarts and can be reached by others
    /// (`hello.fx` lists `keeps`: a desktop or a headless node, not a browser or phone).
    pub durable: bool,
    /// Who runs it, as a key, when its operator notice names them: holders with the same key
    /// are one place. A claim, so it only ever merges places, never adds one.
    pub operator: Option<u64>,
    /// The network its address belongs to, as a key (IPv4 /24, IPv6 /48, this machine):
    /// holders with the same key are one place.
    pub network: Option<u64>,
    /// The link is encrypted and the other end was checked: this device dialed a route it chose,
    /// or the transport authenticated the peer's key.
    pub guarded: bool,
}

impl Holder {
    /// A holder with nothing known about its place: it is a place of its own.
    pub fn new(coverage: f64, durable: bool) -> Holder {
        Holder { coverage, durable, operator: None, network: None, guarded: true }
    }
}

/// One of this device's live connections, for [`assess_links`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Link {
    pub operator: Option<u64>,
    pub network: Option<u64>,
    pub guarded: bool,
}

/// What one node observed about one community. Counts are of live, forwardable records.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Observation {
    /// This device is a desktop or a headless node: its copy outlives the tab.
    pub own_durable: bool,
    /// Live records held here.
    pub records: u64,
    /// Records connected peers listed that have not arrived here yet.
    pub missing: u64,
    /// Media items referenced by live posts here, and how many of them are here.
    pub media_wanted: u64,
    pub media_present: u64,
    pub holders: Vec<Holder>,
    /// When the last catch-up with a peer finished, Unix seconds.
    pub last_sync: Option<u64>,
    pub now: u64,
    /// A connected peer lists this community right now.
    pub live: bool,
    /// The best `w` a connected node attached to its catalog entry for this community.
    pub hinted: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Cold,
    Cool,
    Warm,
    Hot,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Cold => "cold",
            Level::Cool => "cool",
            Level::Warm => "warm",
            Level::Hot => "hot",
        }
    }

    fn down(self) -> Level {
        match self {
            Level::Cold | Level::Cool => Level::Cold,
            Level::Warm => Level::Cool,
            Level::Hot => Level::Warm,
        }
    }
}

/// The reading. `copies` and `durable_copies` carry one decimal, like a torrent's availability.
#[derive(Debug, Clone, PartialEq)]
pub struct Warmth {
    /// Copies on other durable nodes, counted once per independent place: what the level is
    /// read from. Never more than `other_durable`.
    pub kept: f64,
    /// Independent places that hold a durable copy (any part of one).
    pub places: u32,
    /// `kept / other_durable`: 1 when every copy is in a place of its own, lower when copies
    /// share an operator or a network; 0 when no other durable node holds any.
    pub spread: f64,
    /// Share of the other durable copies reached over a guarded link, 0 to 1.
    pub guarded: f64,
    pub level: Level,
    /// 0 to 100, for a bar.
    pub score: u32,
    /// Copies this device can see, its own included: 1.0 is just this device.
    pub copies: f64,
    /// Copies on durable nodes, this device's own if it is one.
    pub durable_copies: f64,
    /// Copies on durable nodes other than this device: what the level is read from.
    pub other_durable: f64,
    /// Copies on other durable nodes according to the best connected node's hint, or this
    /// device's own reading when that is higher. One hop, never summed.
    pub network_copies: f64,
    /// Share of the live window known to exist that is here, 0 to 1.
    pub complete: f64,
    /// Share of referenced media that is here, 0 to 1 (1 when nothing is referenced).
    pub media: f64,
    /// A connected peer lists it, or a catch-up finished within the last seven days.
    pub fresh: bool,
    pub holders_durable: u32,
    pub holders_transient: u32,
}

/// A catch-up older than this, with no live connection, makes a community stale.
pub const FRESH_FOR: u64 = 7 * 86_400;
/// Copies on other durable nodes that make a community hot (with a complete window).
pub const HOT_COPIES: f64 = 3.0;
/// Copies on other durable nodes that fill the bar.
pub const FULL_BAR: f64 = 4.0;
/// Below this share of guarded copies, a reading is one level colder.
pub const GUARDED_ENOUGH: f64 = 0.5;

/// Groups items into places: two share a place when they share an operator key or a network
/// key, directly or through a third. Returns each item's place, numbered from 0 in order of
/// first appearance; items with neither key are places of their own.
pub fn places_of(keys: &[(Option<u64>, Option<u64>)]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..keys.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut by_operator: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    let mut by_network: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    for (i, (operator, network)) in keys.iter().enumerate() {
        for (key, seen) in [(operator, &mut by_operator), (network, &mut by_network)] {
            let Some(key) = key else { continue };
            match seen.get(key) {
                Some(&j) => {
                    let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                    if a != b {
                        parent[a.max(b)] = a.min(b);
                    }
                }
                None => {
                    seen.insert(*key, i);
                }
            }
        }
    }
    let mut number: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    (0..keys.len())
        .map(|i| {
            let r = root(&mut parent, i);
            let next = number.len();
            *number.entry(r).or_insert(next)
        })
        .collect()
}

fn tenths(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn hundredths(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Reads a community's warmth from what this node observed. Deterministic: the same
/// observation gives the same reading on every host (core/vectors/warmth.json).
pub fn assess(o: &Observation) -> Warmth {
    let coverage = |h: &Holder| h.coverage.clamp(0.0, 1.0);
    let own = 1.0;
    let copies = own + o.holders.iter().map(coverage).sum::<f64>();
    let durable: Vec<&Holder> = o.holders.iter().filter(|h| h.durable).collect();
    // `+ 0.0`: an empty sum is -0.0, which would print as such.
    let other_durable = durable.iter().map(|h| coverage(h)).sum::<f64>() + 0.0;
    // One copy's worth per place: the most any holder there showed.
    let place = places_of(&durable.iter().map(|h| (h.operator, h.network)).collect::<Vec<_>>());
    let mut per_place = vec![0.0f64; place.iter().max().map_or(0, |m| m + 1)];
    for (h, p) in durable.iter().zip(&place) {
        per_place[*p] = per_place[*p].max(coverage(h));
    }
    let kept = per_place.iter().sum::<f64>() + 0.0;
    let places = per_place.iter().filter(|c| **c > 0.0).count() as u32;
    let spread = if other_durable > 0.0 { kept / other_durable } else { 0.0 };
    let guarded = if other_durable > 0.0 { durable.iter().filter(|h| h.guarded).map(|h| coverage(h)).sum::<f64>() / other_durable } else { 0.0 };
    let durable_copies = if o.own_durable { 1.0 } else { 0.0 } + other_durable;
    let known = o.records + o.missing;
    let complete = if known == 0 { 0.0 } else { o.records as f64 / known as f64 };
    let media = if o.media_wanted == 0 { 1.0 } else { (o.media_present.min(o.media_wanted)) as f64 / o.media_wanted as f64 };
    let fresh = o.live || o.last_sync.is_some_and(|at| o.now.saturating_sub(at) < FRESH_FOR);
    // The level counts the others, once per place: this device's own copy is the one the
    // reading is about, and copies one party or one outage could take together are one.
    let mut level = if kept < 1.0 {
        Level::Cold
    } else if kept < 2.0 {
        Level::Cool
    } else if kept < HOT_COPIES || complete < 0.95 {
        Level::Warm
    } else {
        Level::Hot
    };
    if complete < 0.5 || !fresh || (other_durable > 0.0 && guarded < GUARDED_ENOUGH) {
        level = level.down();
    }
    // Four independent copies fill most of the bar; copies in places of their own, guarded
    // links, a complete window and a recent catch-up add the rest. A copy alone reads 12 to 20;
    // one other place 55, two 70, three 85, four 100, when every link is guarded.
    let score = (100.0 * (0.6 * (kept / FULL_BAR).min(1.0) + 0.1 * spread + 0.1 * guarded + 0.12 * complete + if fresh { 0.08 } else { 0.0 }))
        .round()
        .clamp(0.0, 100.0) as u32;
    let hinted = (o.hinted.min(super::MAX_WARMTH_HINT)) as f64;
    Warmth {
        kept: tenths(kept),
        places,
        spread: hundredths(spread),
        guarded: hundredths(guarded),
        level,
        score,
        copies: tenths(copies),
        durable_copies: tenths(durable_copies),
        other_durable: tenths(other_durable),
        network_copies: tenths(other_durable.max(hinted)),
        complete: tenths(complete * 100.0) / 100.0,
        media: tenths(media * 100.0) / 100.0,
        fresh,
        holders_durable: o.holders.iter().filter(|h| h.durable).count() as u32,
        holders_transient: o.holders.iter().filter(|h| !h.durable).count() as u32,
    }
}

/// What a node tells its peers about a community in its catalog: the durable copies it saw on
/// other nodes, once per place, quantised to 0..=[`MAX_WARMTH_HINT`](super::MAX_WARMTH_HINT), so
/// catalogs change rarely. A receiver reads it as "that node saw this many besides itself".
pub fn hint(w: &Warmth) -> u8 {
    (w.kept.floor() as u64).min(super::MAX_WARMTH_HINT as u64) as u8
}

/// The warmth of this device's own connections: how many independent places it is linked
/// through, whether one of them carries most of its links, how many links are guarded.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkWarmth {
    pub level: Level,
    /// 0 to 100, for a bar.
    pub score: u32,
    pub links: u32,
    /// Independent places among the links.
    pub places: u32,
    /// The effective number of places (inverse Simpson over links per place, two decimals):
    /// 3 for three links in three places, 1.6 for four links of which three share one place.
    pub effective: f64,
    /// Share of links in the place that carries the most, 0 to 1.
    pub largest: f64,
    /// Share of guarded links, 0 to 1.
    pub guarded: f64,
}

/// Places that make this device's connections hot, with no place over half the links.
pub const HOT_PLACES: f64 = 3.0;

/// Reads the device's connections. Cold: no links. Cool: every link goes through one place, or
/// fewer than half are guarded. Hot: three effective places or more, none with more than half
/// of the links, nine in ten guarded. Warm in between. The score is
/// `0.6 × min(1, effective / 4) + 0.25 × guarded + 0.15 × (1 − largest)`, kept inside the
/// level's band (cool at most 49, warm 50 to 79, hot from 80).
pub fn assess_links(links: &[Link]) -> LinkWarmth {
    let total = links.len();
    if total == 0 {
        return LinkWarmth { level: Level::Cold, score: 0, links: 0, places: 0, effective: 0.0, largest: 0.0, guarded: 0.0 };
    }
    let place = places_of(&links.iter().map(|l| (l.operator, l.network)).collect::<Vec<_>>());
    let mut per_place = vec![0usize; place.iter().max().map_or(0, |m| m + 1)];
    for p in &place {
        per_place[*p] += 1;
    }
    let n = total as f64;
    let effective = 1.0 / per_place.iter().map(|c| (*c as f64 / n).powi(2)).sum::<f64>();
    let largest = *per_place.iter().max().unwrap_or(&0) as f64 / n;
    let guarded = links.iter().filter(|l| l.guarded).count() as f64 / n;
    let level = if per_place.len() < 2 || guarded < GUARDED_ENOUGH {
        Level::Cool
    } else if effective >= HOT_PLACES - 1e-9 && largest <= 0.5 && guarded >= 0.9 {
        Level::Hot
    } else {
        Level::Warm
    };
    // The bar stays inside its word's band, so it never says more than the word does.
    let raw = (100.0 * (0.6 * (effective / FULL_BAR).min(1.0) + 0.25 * guarded + 0.15 * (1.0 - largest))).round().clamp(0.0, 100.0) as u32;
    let score = match level {
        Level::Cold => 0,
        Level::Cool => raw.min(49),
        Level::Warm => raw.clamp(50, 79),
        Level::Hot => raw.max(80),
    };
    LinkWarmth {
        level,
        score,
        links: total as u32,
        places: per_place.len() as u32,
        effective: hundredths(effective),
        largest: hundredths(largest),
        guarded: hundredths(guarded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_790_000_000;

    fn browser(holders: Vec<Holder>) -> Observation {
        Observation { records: 40, holders, last_sync: Some(NOW - 60), now: NOW, live: true, ..Default::default() }
    }

    #[test]
    fn a_copy_on_this_browser_alone_is_cold() {
        let w = assess(&browser(vec![]));
        assert_eq!((w.level, w.copies, w.durable_copies), (Level::Cold, 1.0, 0.0));
        assert_eq!(w.score, 20);
        assert_eq!(w.complete, 1.0);
    }

    #[test]
    fn one_durable_holder_is_cool_two_are_warm_three_complete_are_hot() {
        let node = Holder::new(1.0, true);
        assert_eq!(assess(&browser(vec![node])).level, Level::Cool);
        assert_eq!(assess(&browser(vec![node, node])).level, Level::Warm);
        let hot = assess(&browser(vec![node, node, node]));
        assert_eq!(hot.level, Level::Hot);
        assert_eq!(hot.copies, 4.0);
        assert_eq!(hot.durable_copies, 3.0);
        assert_eq!([assess(&browser(vec![node])).score, assess(&browser(vec![node, node])).score, hot.score], [55, 70, 85]);
        assert_eq!((hot.kept, hot.places, hot.spread, hot.guarded), (3.0, 3, 1.0, 1.0));
        assert_eq!(assess(&browser(vec![node, node, node, node, node])).score, 100);
    }

    #[test]
    fn a_computer_alone_is_cold_too_and_browsers_count_as_copies_only() {
        let mut o = browser(vec![Holder::new(1.0, false)]);
        o.own_durable = true;
        let w = assess(&o);
        assert_eq!((w.level, w.copies, w.durable_copies, w.other_durable), (Level::Cold, 2.0, 1.0, 0.0), "its own copy is the one at risk: {w:?}");
        assert_eq!((w.holders_durable, w.holders_transient), (0, 1));
        assert_eq!(w.score, 20);
        o.holders.push(Holder::new(1.0, true));
        let w = assess(&o);
        assert_eq!((w.level, w.durable_copies, w.other_durable), (Level::Cool, 2.0, 1.0));
    }

    #[test]
    fn partial_copies_add_up_fractionally() {
        let o = browser(vec![Holder::new(0.5, true), Holder::new(0.75, true)]);
        let w = assess(&o);
        assert_eq!(w.other_durable, 1.3);
        assert_eq!(w.level, Level::Cool);
    }

    #[test]
    fn an_incomplete_or_stale_window_reads_one_level_colder() {
        let node = Holder::new(1.0, true);
        let mut o = browser(vec![node, node]);
        o.missing = 60;
        let w = assess(&o);
        assert_eq!(w.complete, 0.4);
        assert_eq!(w.level, Level::Cool, "40% here: warm reads cool");
        let mut o = browser(vec![node, node, node]);
        o.missing = 3;
        assert_eq!(assess(&o).level, Level::Warm, "93% here is not hot yet");
        let mut stale = browser(vec![node, node]);
        stale.live = false;
        stale.last_sync = Some(NOW - FRESH_FOR - 1);
        let w = assess(&stale);
        assert!(!w.fresh);
        assert_eq!(w.level, Level::Cool);
        stale.last_sync = Some(NOW - FRESH_FOR + 1);
        assert!(assess(&stale).fresh);
    }

    #[test]
    fn nothing_here_yet_is_cold_until_records_arrive() {
        let pending = Observation { holders: vec![Holder::new(1.0, true)], missing: 12, now: NOW, live: true, ..Default::default() };
        let w = assess(&pending);
        assert_eq!(w.complete, 0.0);
        assert_eq!(w.level, Level::Cold);
        let nobody = Observation { now: NOW, ..Default::default() };
        assert_eq!(assess(&nobody).level, Level::Cold);
        assert_eq!(assess(&nobody).complete, 0.0);
    }

    #[test]
    fn media_is_read_separately_and_coverage_is_clamped() {
        let mut o = browser(vec![Holder::new(7.0, true), Holder::new(-1.0, true)]);
        o.media_wanted = 4;
        o.media_present = 1;
        let w = assess(&o);
        assert_eq!(w.durable_copies, 1.0);
        assert_eq!(w.media, 0.25);
        assert_eq!(assess(&browser(vec![])).media, 1.0, "no media referenced: nothing is missing");
    }

    #[test]
    fn hints_raise_the_network_reading_without_adding_up() {
        let mut o = browser(vec![Holder::new(1.0, true)]);
        o.hinted = 3;
        let w = assess(&o);
        assert_eq!(w.other_durable, 1.0, "what this device saw itself");
        assert_eq!(w.network_copies, 3.0, "what the connected node reports, as a maximum");
        assert_eq!(w.level, Level::Cool, "the level follows first-hand copies");
        o.hinted = 200;
        assert_eq!(assess(&o).network_copies, 3.0, "hints are capped on reading too");
        let node = Holder::new(1.0, true);
        assert_eq!(hint(&assess(&browser(vec![node, node, node, node]))), 3);
        assert_eq!(hint(&assess(&browser(vec![Holder::new(0.9, true)]))), 0);
        let mut mine = browser(vec![node, node]);
        mine.own_durable = true;
        assert_eq!(hint(&assess(&mine)), 2, "a node counts the others, not itself");
    }

    fn at(network: u64) -> Holder {
        Holder { network: Some(network), ..Holder::new(1.0, true) }
    }

    #[test]
    fn copies_in_one_network_are_one_place() {
        let rack = assess(&browser(vec![at(7), at(7), at(7)]));
        assert_eq!((rack.other_durable, rack.kept, rack.places), (3.0, 1.0, 1), "three nodes in one network: {rack:?}");
        assert_eq!(rack.level, Level::Cool, "one outage takes them all");
        assert_eq!(rack.spread, 0.33);
        let spread = assess(&browser(vec![at(1), at(2), at(3)]));
        assert_eq!((spread.kept, spread.places, spread.level), (3.0, 3, Level::Hot));
        assert!(spread.score > rack.score + 30, "{} vs {}", spread.score, rack.score);
        assert_eq!(hint(&rack), 1, "a hint counts places too");
    }

    #[test]
    fn one_operator_is_one_place_even_across_networks_and_a_claim_only_merges() {
        let lena = |network: u64| Holder { operator: Some(42), ..at(network) };
        let w = assess(&browser(vec![lena(1), lena(2), at(3)]));
        assert_eq!((w.kept, w.places, w.level), (2.0, 2, Level::Warm), "{w:?}");
        // Places join through a shared key: an operator in two networks ties them together.
        assert_eq!(
            places_of(&[(Some(1), Some(10)), (None, Some(10)), (Some(1), Some(20)), (None, Some(30)), (None, None), (None, None)]),
            vec![0, 0, 0, 1, 2, 3]
        );
        // Nobody can add places by claiming: an unknown holder is one place, a claim can only merge.
        assert_eq!(assess(&browser(vec![Holder::new(1.0, true)])).places, 1);
    }

    #[test]
    fn copies_reached_over_unguarded_links_read_colder() {
        let open = |network: u64| Holder { guarded: false, ..at(network) };
        let w = assess(&browser(vec![open(1), open(2), at(3)]));
        assert_eq!((w.kept, w.guarded), (3.0, 0.33));
        assert_eq!(w.level, Level::Warm, "two of three copies only over unchecked links: one level colder");
        let guarded = assess(&browser(vec![at(1), at(2), at(3)]));
        assert_eq!(guarded.level, Level::Hot);
        assert!(guarded.score > w.score);
    }

    #[test]
    fn a_devices_connections_read_by_place_and_protection() {
        let link = |network: u64| Link { operator: None, network: Some(network), guarded: true };
        let none = assess_links(&[]);
        assert_eq!((none.level, none.score), (Level::Cold, 0));
        let one = assess_links(&[link(1)]);
        assert_eq!((one.level, one.places, one.effective, one.largest, one.score), (Level::Cool, 1, 1.0, 1.0, 40));
        let same = assess_links(&[link(1), link(1), link(1)]);
        assert_eq!((same.level, same.places), (Level::Cool, 1), "three links into one place are one way in");
        let three = assess_links(&[link(1), link(2), link(3)]);
        assert_eq!((three.level, three.effective, three.score), (Level::Hot, 3.0, 80));
        let lopsided = assess_links(&[link(1), link(1), link(1), link(2)]);
        assert_eq!((lopsided.level, lopsided.effective, lopsided.largest), (Level::Warm, 1.6, 0.75));
        let open = assess_links(&[Link { guarded: false, ..link(1) }, Link { guarded: false, ..link(2) }, link(3)]);
        assert_eq!(open.level, Level::Cool, "most links unguarded");
        assert_eq!(open.score, 49, "the bar never reads warmer than the word");
        assert_eq!(assess(&browser(vec![])).kept.to_string(), "0", "no negative zero");
        assert_eq!(assess_links(&[link(1), link(2), link(3), link(4)]).score, 96);
    }
}
