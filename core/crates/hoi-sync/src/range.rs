//! Range-based set reconciliation, after Aljoscha Meyer's and negentropy's: two
//! peers learn which records the other holds that they lack without either listing every id.
//!
//! Records are ordered by `(created_at, id)`. A message is a partition of that whole order into
//! consecutive ranges, each ending at an exclusive upper bound, each either settled (`Skip`), a
//! fingerprint of the sender's records in it, or the sender's ids in it. The fingerprint is
//! `BLAKE3(Σ ids mod 2²⁵⁶ ‖ u64le(count))[..16]`: the same set gives the same fingerprint
//! whatever order it was summed in, and it can be summed over any range.
//!
//! One side starts (`initiate`) and learns what the other holds. The responder answers a
//! fingerprint that differs by splitting its range in sixteen, or with its ids when few; it
//! answers ids with its own ids, and always replies, so the starter knows when it is done. The
//! starter takes the ids it lacks from the responder's ids and stops once nothing is open. Each
//! side starts its own session, so each learns what to ask for, and asks with `want` as before.
//! A set of 64 ids or fewer goes as one list in the first message.

use std::collections::BTreeSet;

use hoi_types::Id;

/// Ids a range is sent as rather than split, at most.
pub const IDS_THRESHOLD: usize = 64;
/// Sub-ranges a differing range is split into.
pub const BRANCHES: usize = 16;
/// Ranges one message carries, at most. A side that would need more answers the rest of the
/// differing ranges with its ids, which always settles them.
pub const MAX_RANGES: usize = 8192;

/// A record's place in the order.
pub type Item = (u64, Id);

/// An exclusive upper bound; `Bound::END` is past every record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bound {
    pub at: u64,
    pub id: Id,
}

impl Bound {
    pub const END: Bound = Bound { at: u64::MAX, id: Id([0xff; 32]) };

    /// The shortest bound above `prev` and at or below `next`: no id at all when their times
    /// differ, else just enough of `next`'s id. Shorter bounds are fewer bytes on the wire.
    fn between(prev: &Item, next: &Item) -> Bound {
        if prev.0 < next.0 {
            return Bound { at: next.0, id: Id([0; 32]) };
        }
        let mut id = [0u8; 32];
        for (i, b) in next.1.0.iter().enumerate() {
            id[i] = *b;
            if *b != prev.1.0[i] {
                break;
            }
        }
        Bound { at: next.0, id: Id(id) }
    }

    fn above(&self, item: &Item) -> bool {
        (item.0, item.1) < (self.at, self.id)
    }

    /// The id's length without its trailing zero bytes, as it goes on the wire.
    pub fn id_len(&self) -> usize {
        self.id.0.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Range {
    /// Settled: nothing to do here.
    Skip,
    /// The sender's records in the range, summed.
    Fingerprint([u8; 16]),
    /// The sender's ids in the range, all of them.
    Ids(Vec<Id>),
}

/// One range: everything from the previous entry's bound (or the start) up to `upper`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub upper: Bound,
    pub range: Range,
}

/// The fingerprint of a set of records.
pub fn fingerprint(items: &[Item]) -> [u8; 16] {
    let mut sum = [0u64; 4];
    for (_, id) in items {
        let mut carry = 0u128;
        for (i, limb) in sum.iter_mut().enumerate() {
            let v = u64::from_le_bytes(id.0[i * 8..i * 8 + 8].try_into().expect("8 bytes"));
            let s = *limb as u128 + v as u128 + carry;
            *limb = s as u64;
            carry = s >> 64;
        }
    }
    let mut h = blake3::Hasher::new_derive_key("hoi 2026 range fingerprint v1");
    for limb in sum {
        h.update(&limb.to_le_bytes());
    }
    h.update(&(items.len() as u64).to_le_bytes());
    h.finalize().as_bytes()[..16].try_into().expect("16 bytes")
}

/// The records of `items` (sorted) from `lower` (inclusive) up to `upper` (exclusive).
fn slice<'a>(items: &'a [Item], lower: &Bound, upper: &Bound) -> &'a [Item] {
    let start = items.partition_point(|i| (i.0, i.1) < (lower.at, lower.id));
    let end = items.partition_point(|i| upper.above(i));
    &items[start.min(end)..end]
}

const START: Bound = Bound { at: 0, id: Id([0; 32]) };

/// Ranges for the records `here` up to `upper`: their ids when few (or when `split` is false),
/// else sixteen fingerprints.
fn describe(here: &[Item], upper: &Bound, split: bool, out: &mut Vec<Entry>) {
    if here.len() <= IDS_THRESHOLD || !split {
        out.push(Entry { upper: *upper, range: Range::Ids(here.iter().map(|i| i.1).collect()) });
        return;
    }
    let per = here.len().div_ceil(BRANCHES);
    let chunks = here.len().div_ceil(per);
    for (n, chunk) in here.chunks(per).enumerate() {
        let bound = if n == chunks - 1 { *upper } else { Bound::between(&chunk[chunk.len() - 1], &here[(n + 1) * per]) };
        out.push(Entry { upper: bound, range: Range::Fingerprint(fingerprint(chunk)) });
    }
}

/// The first message of a session over `items`, sorted by `(created_at, id)`.
pub fn initiate(items: &[Item]) -> Vec<Entry> {
    let mut out = Vec::new();
    describe(items, &Bound::END, true, &mut out);
    out
}

/// What one side does with a message.
#[derive(Debug, Default)]
pub struct Step {
    /// The answer. The responder always sends it; the starter only while `open`.
    pub reply: Vec<Entry>,
    /// Something is still to settle.
    pub open: bool,
    /// Ids the other side holds that this one lacks (meaningful to the starter).
    pub lack: Vec<Id>,
    /// This side's records in ranges both hold alike: the other side knows their ids.
    pub shared: usize,
}

/// One side's answer to a message: `starter` says this side began the session (it takes ids in;
/// the other side answers ids with its own). `items` sorted by `(created_at, id)`.
pub fn reconcile(items: &[Item], theirs: &[Entry], starter: bool) -> Step {
    let theirs = &theirs[..theirs.len().min(MAX_RANGES)];
    let mut step = Step::default();
    let mut reply: Vec<Entry> = Vec::with_capacity(theirs.len());
    let mut lower = START;
    for (n, e) in theirs.iter().enumerate() {
        let here = slice(items, &lower, &e.upper);
        match &e.range {
            Range::Skip => reply.push(Entry { upper: e.upper, range: Range::Skip }),
            Range::Fingerprint(fp) if fingerprint(here) == *fp => {
                step.shared += here.len();
                reply.push(Entry { upper: e.upper, range: Range::Skip });
            }
            Range::Fingerprint(_) => {
                // Splitting adds up to fifteen ranges; the ranges still to answer need one each.
                let split = reply.len() + BRANCHES + (theirs.len() - n - 1) <= MAX_RANGES;
                describe(here, &e.upper, split, &mut reply);
                step.open = true;
            }
            Range::Ids(ids) => {
                let mine: BTreeSet<Id> = here.iter().map(|i| i.1).collect();
                let given: BTreeSet<Id> = ids.iter().copied().collect();
                step.lack.extend(given.difference(&mine));
                step.shared += mine.intersection(&given).count();
                if starter || mine == given {
                    reply.push(Entry { upper: e.upper, range: Range::Skip });
                } else {
                    reply.push(Entry { upper: e.upper, range: Range::Ids(mine.into_iter().collect()) });
                    step.open = true;
                }
            }
        }
        lower = e.upper;
    }
    // neighbouring skips are one
    for e in reply {
        if let (Some(last), Range::Skip) = (step.reply.last_mut(), &e.range)
            && last.range == Range::Skip
        {
            last.upper = e.upper;
            continue;
        }
        step.reply.push(e);
    }
    step
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(range: std::ops::Range<u64>, salt: u8) -> Vec<Item> {
        range.map(|i| (1_000 + i, Id::hash(&[salt, (i >> 16) as u8, (i >> 8) as u8, i as u8]))).collect()
    }

    fn bytes(m: &[Entry]) -> usize {
        m.iter()
            .map(|e| {
                10 + e.upper.id_len()
                    + match &e.range {
                        Range::Ids(v) => v.len() * 33,
                        Range::Fingerprint(_) => 17,
                        Range::Skip => 0,
                    }
            })
            .sum()
    }

    /// Runs both sessions to the end; returns what each learned it lacks, the bytes moved and
    /// the most rounds a session took.
    fn sync(a: &[Item], b: &[Item]) -> (BTreeSet<Id>, BTreeSet<Id>, usize, usize) {
        let mut moved = 0;
        let mut most = 0;
        let mut run = |starter: &[Item], other: &[Item]| {
            let mut msg = initiate(starter);
            let mut lack = BTreeSet::new();
            let mut rounds = 0;
            loop {
                assert!(msg.len() <= MAX_RANGES);
                moved += bytes(&msg);
                let answer = reconcile(other, &msg, false);
                moved += bytes(&answer.reply);
                let step = reconcile(starter, &answer.reply, true);
                lack.extend(step.lack);
                rounds += 1;
                assert!(rounds < 20, "a session ends");
                if !step.open {
                    break;
                }
                msg = step.reply;
            }
            most = most.max(rounds);
            lack
        };
        let a_lacks = run(a, b);
        let b_lacks = run(b, a);
        (a_lacks, b_lacks, moved, most)
    }

    fn sorted(mut v: Vec<Item>) -> Vec<Item> {
        v.sort();
        v
    }

    fn lacking(of: &[Item], have: &[Item]) -> BTreeSet<Id> {
        let h: BTreeSet<Id> = have.iter().map(|i| i.1).collect();
        of.iter().map(|i| i.1).filter(|i| !h.contains(i)).collect()
    }

    #[test]
    fn two_sides_of_a_thousand_learn_exactly_what_they_lack() {
        let common = items(0..1000, 1);
        let a = sorted([common.clone(), items(0..7, 2)].concat());
        let b = sorted([common.clone(), items(0..5, 3)].concat());
        let (a_lacks, b_lacks, moved, _) = sync(&a, &b);
        assert_eq!(a_lacks, lacking(&b, &a));
        assert_eq!(b_lacks, lacking(&a, &b));
        // far less than both full lists (2 × 1,000 × 32 bytes)
        assert!(moved < 2 * 1000 * 32 / 4, "moved {moved} bytes");
    }

    #[test]
    fn the_same_set_settles_at_once_and_an_empty_side_learns_everything() {
        let a = sorted(items(0..1000, 1));
        let (x, y, _, rounds) = sync(&a, &a);
        assert!(x.is_empty() && y.is_empty());
        assert_eq!(rounds, 1);
        let (lacks, nothing, _, _) = sync(&[], &a);
        assert_eq!(lacks.len(), 1000);
        assert!(nothing.is_empty());
    }

    #[test]
    fn many_differences_and_shared_times_still_settle() {
        // 20,000 records, every third missing on one side, ten records a second
        let all: Vec<Item> = items(0..20_000, 5).into_iter().map(|(at, id)| (at / 10, id)).collect();
        let a = sorted(all.clone());
        let b = sorted(all.iter().enumerate().filter(|(n, _)| n % 3 != 0).map(|(_, i)| *i).collect());
        let (a_lacks, b_lacks, _, rounds) = sync(&a, &b);
        assert!(a_lacks.is_empty());
        assert_eq!(b_lacks, lacking(&a, &b));
        assert!(rounds <= 5, "{rounds} rounds");
    }

    #[test]
    fn a_reply_stays_within_the_range_cap() {
        // their message has the most ranges there may be, every one of them differing
        let mine = sorted(items(0..1000, 6));
        let mut theirs: Vec<Entry> =
            (0..MAX_RANGES as u64 - 1).map(|n| Entry { upper: Bound { at: 2_000 + n, id: Id([0; 32]) }, range: Range::Fingerprint([1; 16]) }).collect();
        theirs.push(Entry { upper: Bound::END, range: Range::Fingerprint([1; 16]) });
        let step = reconcile(&mine, &theirs, false);
        assert!(step.reply.len() <= MAX_RANGES);
        // with no room to split, the 1,000 records go as ids, which settles them
        assert_eq!(step.reply[0].range, Range::Ids(mine.iter().map(|i| i.1).collect()));
    }

    #[test]
    fn short_bounds_split_the_same_way() {
        let a = (5, Id([1; 32]));
        let b = (5, Id([[1; 31].as_slice(), &[2]].concat().try_into().unwrap()));
        let bound = Bound::between(&a, &b);
        assert!(bound.above(&a) && !bound.above(&b));
        assert_eq!(bound.id_len(), 32);
        let c = (6, Id([9; 32]));
        let bound = Bound::between(&b, &c);
        assert!(bound.above(&b) && !bound.above(&c));
        assert_eq!(bound.id_len(), 0);
    }

    #[test]
    fn fingerprints_do_not_depend_on_order() {
        let a = items(0..100, 4);
        let mut b = a.clone();
        b.reverse();
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_ne!(fingerprint(&a), fingerprint(&a[1..]));
    }
}
