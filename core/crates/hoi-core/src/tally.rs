//! Community tally aggregators (M5), after the feed-generator pattern: a durable node
//! that holds more of a community than a browser does counts its posts' votes and publishes the
//! counts, signed (`tally`). People follow an aggregator as they follow a labeler; its counts show
//! next to a post's own, and the feed can be ordered by them when the person picks that. They
//! never go into Hot or any other ranking on their own: an aggregator's word is its word.

use std::collections::BTreeMap;

use hoi_types::{Body, Id, Kind, Record};

use crate::api::{R, parse_id, parse_person_key};
use crate::dto::{AggregatorView, TallyView};
use crate::error::ApiError;
use crate::node::Node;

/// An aggregating device counts again after this.
const EVERY: u64 = 3600;

/// A post's counts in a tally: (post, up, down).
type Row = (Id, u64, u64);

impl Node {
    /// The aggregators this person follows.
    pub fn aggregators(&self) -> Vec<AggregatorView> {
        self.state
            .aggregators
            .iter()
            .map(|root| AggregatorView {
                person: self.person(root, None),
                communities: self.state.joined.keys().filter(|c| self.latest_tally(c, root).is_some()).count() as u32,
            })
            .collect()
    }

    /// Follows an aggregator's counts, or stops. Following reconciles every connection again, so
    /// their counts arrive with the next sync.
    pub fn follow_aggregator(&mut self, key: &str, on: bool) -> R<Vec<AggregatorView>> {
        let k = parse_person_key(key)?;
        if Some(k) == self.me_root() {
            return Err(ApiError::invalid("Your own counts always show here."));
        }
        let changed = if on { self.state.aggregators.insert(k) } else { self.state.aggregators.remove(&k) };
        if changed {
            self.mark_state();
            self.changed_everywhere();
            if on {
                self.reconcile_again();
            }
        }
        Ok(self.aggregators())
    }

    /// Whether this device counts for a community.
    pub fn aggregating(&self, community: &str) -> R<bool> {
        Ok(self.state.aggregating.contains(&parse_id(community)?))
    }

    /// Counts a community's votes and publishes them, now and every hour, or stops.
    pub fn set_aggregating(&mut self, community: &str, on: bool) -> R<bool> {
        let c = parse_id(community)?;
        self.need_me()?;
        if !self.state.joined.contains_key(&c) || self.is_private_community(&c) {
            return Err(ApiError::not_allowed("Counts are for public communities this device keeps."));
        }
        if on {
            self.state.aggregating.insert(c);
            self.mark_state();
            self.publish_tally(c)?;
        } else if self.state.aggregating.remove(&c) {
            self.mark_state();
        }
        Ok(on)
    }

    /// The latest tally of `root` in `c`.
    fn latest_tally(&self, c: &Id, root: &Id) -> Option<(u64, &[Row])> {
        self.store.by_author(c, root).filter(|s| s.record.kind() == Kind::Tally).max_by_key(|s| (s.record.created_at, s.id)).and_then(|s| {
            match &s.record.body {
                Body::Tally { counts } => Some((s.record.created_at, counts.as_slice())),
                _ => None,
            }
        })
    }

    /// The counts this device holds for the posts of `c` with the most votes, at most 500: each
    /// voter's latest vote once, unweighted.
    fn count_votes(&self, c: &Id) -> Vec<(Id, u64, u64)> {
        let Some(st) = self.communities.get(c) else { return Vec::new() };
        let mut rows: Vec<(Id, u64, u64)> = Vec::new();
        for s in self.store.of_kind(c, Kind::Post).filter(|s| self.shown(s, st)) {
            let mut latest: BTreeMap<Id, (u64, i64)> = BTreeMap::new();
            for v in self.store.children(&s.id) {
                if let Body::Vote { value, seq } = v.record.body {
                    let e = latest.entry(v.author_root).or_insert((0, 0));
                    if seq >= e.0 {
                        *e = (seq, value);
                    }
                }
            }
            let up = latest.values().filter(|(_, v)| *v > 0).count() as u64;
            let down = latest.values().filter(|(_, v)| *v < 0).count() as u64;
            if up + down > 0 {
                rows.push((s.id, up, down));
            }
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.1 + r.2));
        rows.truncate(hoi_types::MAX_TALLY);
        rows.sort_by_key(|r| r.0);
        rows
    }

    pub(crate) fn publish_tally(&mut self, c: Id) -> R<Id> {
        let st = self.community_of(&c)?;
        let now = self.now();
        let r = Record {
            v: 1,
            community: Some(c),
            author: self.device_key()?,
            parent: None,
            created_at: now,
            expires_at: now + st.limits.max_retention.min(7 * 86_400),
            body: Body::Tally { counts: self.count_votes(&c) },
            nonce: 0,
        };
        self.publish(r)
    }

    /// At each sweep: the communities this device counts for, once an hour.
    pub(crate) fn renew_tallies(&mut self) {
        let Some(me) = self.me_root() else { return };
        let now = self.now();
        let due: Vec<Id> = self.state.aggregating.iter().filter(|c| self.latest_tally(c, &me).is_none_or(|(at, _)| at + EVERY <= now)).copied().collect();
        for c in due {
            let _ = self.publish_tally(c);
        }
    }

    /// The counts the aggregators this person follows give for a post.
    pub(crate) fn tallies_for(&self, c: &Id, post: &Id) -> Vec<TallyView> {
        self.state
            .aggregators
            .iter()
            .filter_map(|a| {
                let (at, counts) = self.latest_tally(c, a)?;
                let i = counts.binary_search_by_key(post, |r| r.0).ok()?;
                Some(TallyView { by: self.display_name(a), key: a.hex(), up: counts[i].1, down: counts[i].2, at })
            })
            .collect()
    }
}
