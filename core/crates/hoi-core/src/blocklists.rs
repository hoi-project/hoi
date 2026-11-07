//! Open blocklists (M5): a list keeper (a group that tracks abuse imagery, a
//! community's own moderators) publishes the exact media hashes and picture fingerprints it
//! refuses, signed (`blocklist`). A person follows keepers as they follow labelers; every device
//! of theirs then refuses those hashes, and pictures close to those fingerprints, in every
//! community, besides its own list. A keeper nobody here follows is refused without penalty.

use std::collections::BTreeSet;

use hoi_types::{Body, Id, Kind, Record};

use crate::api::{R, parse_id, parse_person_key};
use crate::dto::BlocklistKeeper;
use crate::error::ApiError;
use crate::node::Node;
use crate::state::PdqList;

impl Node {
    /// Builds again what the followed keepers' latest lists refuse together, and lets go of what
    /// this device holds that they refuse.
    pub(crate) fn refresh_lists(&mut self) {
        self.lists_dirty = false;
        let mut exact = BTreeSet::new();
        let mut pdq = PdqList::default();
        for keeper in &self.state.blocklists {
            for c in self.communities.keys() {
                let latest = self.store.by_author(c, keeper).filter(|s| s.record.kind() == Kind::Blocklist).max_by_key(|s| (s.record.created_at, s.id));
                if let Some(Body::Blocklist { exact: e, pdq: p }) = latest.map(|s| &s.record.body) {
                    exact.extend(e.iter().copied());
                    for h in p {
                        pdq.insert(h.0);
                    }
                }
            }
        }
        let changed = exact != self.lists_exact || pdq != self.lists_pdq;
        self.lists_exact = exact;
        self.lists_pdq = pdq;
        if changed {
            let refused: Vec<Id> = self.store.blobs().map(|(h, _)| *h).filter(|h| self.lists_exact.contains(h)).collect();
            for h in refused {
                self.drop_stored_media(h);
            }
            if !self.lists_pdq.is_empty() {
                let pictures: Vec<(Id, String)> = self
                    .store
                    .blobs()
                    .map(|(h, _)| *h)
                    .filter_map(|h| self.mime_of_blob(&h).filter(|m| hoi_validate::media::is_image(m)).map(|m| (h, m)))
                    .collect();
                for (h, mime) in pictures {
                    if let Some(look) = self.look_at(h, &mime, None, crate::look::Why::Recheck)
                        && self.blocked_look(&look)
                    {
                        self.drop_stored_media(h);
                    }
                }
            }
            self.purge_blocked_profiles();
            self.changed_everywhere();
        }
    }

    /// The keepers this person follows, and what their lists hold here.
    pub fn blocklist_keepers(&self) -> Vec<BlocklistKeeper> {
        self.state
            .blocklists
            .iter()
            .map(|k| {
                let (mut exact, mut pdq) = (0u32, 0u32);
                for c in self.communities.keys() {
                    if let Some(Body::Blocklist { exact: e, pdq: p }) = self
                        .store
                        .by_author(c, k)
                        .filter(|s| s.record.kind() == Kind::Blocklist)
                        .max_by_key(|s| (s.record.created_at, s.id))
                        .map(|s| &s.record.body)
                    {
                        exact += e.len() as u32;
                        pdq += p.len() as u32;
                    }
                }
                BlocklistKeeper { person: self.person(k, None), exact, pdq }
            })
            .collect()
    }

    /// Follows a keeper's blocklist, or stops. Following reconciles every connection again, so
    /// their lists arrive with the next sync.
    pub fn follow_blocklist(&mut self, key: &str, on: bool) -> R<Vec<BlocklistKeeper>> {
        let k = parse_person_key(key)?;
        if Some(k) == self.me_root() {
            return Err(ApiError::invalid("Your own list always counts here."));
        }
        let changed = if on { self.state.blocklists.insert(k) } else { self.state.blocklists.remove(&k) };
        if changed {
            self.mark_state();
            self.lists_dirty = true;
            self.settle_dirty();
            if on {
                self.reconcile_again();
            }
        }
        Ok(self.blocklist_keepers())
    }

    /// Publishes a list in a community, for whoever follows this person's lists: one hash a line,
    /// a 64-character PDQ fingerprint, or `blake3:` and an exact media hash. Lines starting with
    /// `#` are notes.
    pub fn publish_blocklist(&mut self, community: &str, text: &str) -> R<u32> {
        let c = parse_id(community)?;
        self.need_me()?;
        if self.is_private_community(&c) {
            return Err(ApiError::not_allowed("Lists are for public communities."));
        }
        let (mut exact, mut pdq) = (BTreeSet::new(), BTreeSet::new());
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let token = line.split(|ch: char| ch == ',' || ch.is_whitespace()).next().unwrap_or("").to_ascii_lowercase();
            if let Some(h) = token.strip_prefix("blake3:") {
                exact.insert(Id::from_hex(h).map_err(|_| ApiError::invalid(format!("{line} isn't a media hash.")))?);
            } else if let Some(h) = hoi_validate::similar::from_hex(&token) {
                pdq.insert(Id(h));
            } else {
                return Err(ApiError::invalid(format!("{line} isn't a fingerprint (64 hex) or blake3:<hash>.")));
            }
        }
        if exact.len() > hoi_types::MAX_BLOCKLIST || pdq.len() > hoi_types::MAX_BLOCKLIST {
            return Err(ApiError::invalid("A list holds 1000 hashes of each kind at most."));
        }
        let n = (exact.len() + pdq.len()) as u32;
        let st = self.community_of(&c)?;
        let now = self.now();
        let r = Record {
            v: 1,
            community: Some(c),
            author: self.device_key()?,
            parent: None,
            created_at: now,
            expires_at: now + st.limits.max_retention,
            body: Body::Blocklist { exact: exact.into_iter().collect(), pdq: pdq.into_iter().collect() },
            nonce: 0,
        };
        self.publish(r)?;
        Ok(n)
    }
}
