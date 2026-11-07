//! Trust statements (M3): whom a person trusts, published in a community only when
//! they choose to. Each device works out its own ring from them (`Node::refresh_ring`): the
//! people marked as friends here, then the people those friends' statements name. Ranking
//! weighs friends' votes fully and friends of friends' by half (PROTOCOL §8). The statement
//! names only friends known in that community, so sharing there says nothing about the rest.

use hoi_types::{Body, Id, Kind, Record};

use crate::api::{R, parse_id};
use crate::dto::TrustSharing;
use crate::error::ApiError;
use crate::node::Node;

/// A statement is made again when it would expire within this.
const RENEW_BEFORE: u64 = 2 * 86_400;

impl Node {
    /// Whether this person shares whom they trust in a community, and how many friends it names.
    pub fn trust_sharing(&self, community: &str) -> R<TrustSharing> {
        let c = parse_id(community)?;
        self.community_of(&c)?;
        Ok(TrustSharing {
            community: c.hex(),
            shared: self.state.trust_shared.contains(&c),
            named: self.trust_keys(&c).len() as u32,
            friends: self.state.friends.len() as u32,
        })
    }

    /// Shares whom this person trusts in a community, or takes it back: everyone there can see
    /// the friends it names. Taking it back publishes an empty statement in its place.
    pub fn share_trust(&mut self, community: &str, on: bool) -> R<TrustSharing> {
        let c = parse_id(community)?;
        self.need_me()?;
        if !self.state.joined.contains_key(&c) {
            return Err(ApiError::not_allowed("Join the community first."));
        }
        if self.is_private_community(&c) {
            return Err(ApiError::not_allowed("Private groups keep who trusts whom to themselves: there is nothing to share."));
        }
        self.community_of(&c)?;
        let was = self.state.trust_shared.contains(&c);
        if on {
            self.state.trust_shared.insert(c);
        } else {
            self.state.trust_shared.remove(&c);
        }
        self.mark_state();
        if (on || was)
            && let Err(e) = self.publish_trust(c)
        {
            // nothing changed until the statement is out
            if on && !was {
                self.state.trust_shared.remove(&c);
            } else if !on && was {
                self.state.trust_shared.insert(c);
            }
            return Err(e);
        }
        self.trust_sharing(community)
    }

    /// The friends a statement in `c` names: those known there (admitted, or with records of
    /// their own there), sorted, at most 256.
    pub(crate) fn trust_keys(&self, c: &Id) -> Vec<Id> {
        let Some(st) = self.communities.get(c) else { return Vec::new() };
        self.state.friends.keys().filter(|f| st.is_admitted(f) || self.store.by_author(c, f).next().is_some()).take(hoi_types::MAX_TRUST).copied().collect()
    }

    fn own_statement(&self, c: &Id) -> Option<(u64, Vec<Id>)> {
        let me = self.me_root()?;
        self.store.by_author(c, &me).filter(|s| s.record.kind() == Kind::Trust).max_by_key(|s| (s.record.created_at, s.id)).and_then(|s| match &s.record.body {
            Body::Trust { keys } => Some((s.record.expires_at, keys.clone())),
            _ => None,
        })
    }

    /// Publishes the statement for `c`: the friends known there while sharing, none after.
    pub(crate) fn publish_trust(&mut self, c: Id) -> R<Id> {
        let keys = if self.state.trust_shared.contains(&c) { self.trust_keys(&c) } else { Vec::new() };
        let st = self.community_of(&c)?;
        let now = self.now();
        let r = Record {
            v: 1,
            community: Some(c),
            author: self.device_key()?,
            parent: None,
            created_at: now,
            expires_at: now + st.limits.max_retention,
            body: Body::Trust { keys },
            nonce: 0,
        };
        self.publish(r)
    }

    /// Keeps each shared statement current: made again when the friends it should name changed
    /// or it would expire soon. Called after friends change and at each sweep.
    pub(crate) fn renew_trust(&mut self) {
        if self.me_root().is_none() {
            return;
        }
        let now = self.now();
        let shared: Vec<Id> = self.state.trust_shared.iter().copied().collect();
        for c in shared {
            if !self.communities.contains_key(&c) {
                continue;
            }
            let due = match self.own_statement(&c) {
                None => true,
                Some((expires_at, keys)) => keys != self.trust_keys(&c) || expires_at < now + RENEW_BEFORE,
            };
            if due {
                // a rate limit or a missing connection: the next sweep tries again
                let _ = self.publish_trust(c);
            }
        }
    }
}
