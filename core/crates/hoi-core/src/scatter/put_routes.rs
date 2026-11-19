//! Assign distinct indexes to vouched forwarder places before a batch of PUTs leaves.
//! Only unexposed assignments can move. An already dispatched request stays charged to
//! its original place by `Paths::carried`.
use std::collections::{BTreeMap, VecDeque};

/// A capacitated bipartite matching. Rows are sampled holders during dispatch, or distinct holder
/// places when testing whether the pool has enough independent routes.
/// Adjacency order breaks equal-load ties; the sending path randomizes it.
pub(super) struct Matching {
    choices: Vec<Vec<usize>>,
    cap: usize,
    assigned: Vec<Option<usize>>,
    occupants: BTreeMap<usize, Vec<usize>>,
}

impl Matching {
    pub(super) fn new(choices: Vec<Vec<usize>>, cap: usize) -> Self {
        let assigned = vec![None; choices.len()];
        Self { choices, cap, assigned, occupants: BTreeMap::new() }
    }

    pub(super) fn place(&self, holder: usize) -> Option<usize> {
        self.assigned[holder]
    }

    /// Give one more holder a forwarder without dropping any previous holder. Search for
    /// spare capacity directly first, then along alternating paths through existing assignments.
    pub(super) fn add(&mut self, start: usize) -> bool {
        let mut queue = VecDeque::from([start]);
        // Each visited forwarder place remembers the holder that could move there.
        let mut predecessor = BTreeMap::new();
        while let Some(holder) = queue.pop_front() {
            let free = self.choices[holder]
                .iter()
                .filter(|p| !predecessor.contains_key(*p))
                .map(|p| (*p, self.occupants.get(p).map_or(0, Vec::len)))
                .filter(|(_, load)| *load < self.cap)
                .min_by_key(|(_, load)| *load);
            if let Some((mut place, _)) = free {
                predecessor.insert(place, holder);
                loop {
                    let moving = predecessor[&place];
                    self.occupants.entry(place).or_default().push(moving);
                    let Some(old) = self.assigned[moving].replace(place) else { break };
                    self.occupants.get_mut(&old).expect("assigned holder has a forwarder").retain(|h| *h != moving);
                    place = old;
                }
                return true;
            }
            for place in &self.choices[holder] {
                if let std::collections::btree_map::Entry::Vacant(entry) = predecessor.entry(*place) {
                    entry.insert(holder);
                    if let Some(previous) = self.occupants.get(place) {
                        queue.extend(previous);
                    }
                }
            }
        }
        false
    }
}
