//! Public discovery selects a key, not a bearer invite. Only peers can supply its signed
//! definition; the registry is never consulted by record validation or existing membership.
use std::collections::BTreeSet;

use hoi_types::{Admission, Body, Id, Record};
use hoi_validate::Reject;

use crate::api::{R, parse_id};
use crate::dto::PublicJoinStatus;
use crate::net::valid_peer_hint;
use crate::state::{Joined, PendingPublicJoin};
use crate::{ApiError, Node};

/// Public joins a person may have waiting at once.
const MAX_PENDING_PUBLIC_JOINS: usize = 16;
/// A carrier mirroring a public index asks for many communities whose founders are not around
/// yet; each waits until a member connects.
const MAX_CARRIED: usize = 1024;

impl Node {
    pub fn public_join(&mut self, id: &str, peers: &[String], expected_name: Option<&str>) -> R<PublicJoinStatus> {
        self.join_public_key(id, peers, expected_name, MAX_PENDING_PUBLIC_JOINS, false)
    }

    /// A native node keeping a public community available for everyone: like [`Node::public_join`]
    /// without routes (members connect to the carrier, not the other way round), pinning the
    /// records, and with room for a whole index of communities still waiting for their founder.
    /// Browsers and phones can't accept connections, so they can't carry.
    pub fn carry(&mut self, id: &str, expected_name: Option<&str>) -> R<PublicJoinStatus> {
        if !self.can_relay() {
            return Err(ApiError::not_allowed("Only a desktop app or hoi-node can carry public communities."));
        }
        if self.is_refused(&parse_id(id)?) {
            return Err(ApiError::not_allowed("The operator of this node refuses to carry that community."));
        }
        let status = self.join_public_key(id, &[], expected_name, MAX_CARRIED, true)?;
        if status.status != "rejected" {
            self.note_carried(parse_id(id)?, crate::carry::ORIGIN_INDEX);
        }
        Ok(status)
    }

    fn join_public_key(&mut self, id: &str, peers: &[String], expected_name: Option<&str>, max_pending: usize, pinning: bool) -> R<PublicJoinStatus> {
        // A person joins with their key. A native carrier (a public node) has none: it stores
        // and serves records, never signs any, so it needs no identity to carry.
        if self.me().is_none() && !self.can_relay() {
            return Err(ApiError::no_identity());
        }
        let c = parse_id(id)?;
        if self.is_private_community(&c) {
            return Err(ApiError::not_allowed("Private communities require an encrypted group invitation."));
        }
        if self.suppressed_community(&c) {
            return Err(ApiError::not_allowed("This node's operator suppressed that community; it is not held here."));
        }
        if peers.len() > 16 || peers.iter().any(|p| !valid_peer_hint(p)) {
            return Err(ApiError::invalid("Public nodes must be valid network routes (at most 16)."));
        }
        let name =
            expected_name.map(|n| crate::community_registration_name(n).ok_or_else(|| ApiError::invalid("Invalid public community name."))).transpose()?;
        if let Some(known) = self.communities.get(&c) {
            if known.limits.admission != Admission::Open {
                return Err(ApiError::not_allowed("This community is not open for public joining. Ask a member about its admission rules."));
            }
            if name.as_ref().is_some_and(|n| crate::community_registration_name(&known.genesis.name).as_ref() != Some(n)) {
                return Err(ApiError::not_allowed("The signed community name does not match this directory result."));
            }
        }
        if !self.communities.contains_key(&c) && !self.state.public_joins.contains_key(&c) && self.state.public_joins.len() >= max_pending {
            return Err(ApiError::not_allowed("Finish or cancel an earlier public join before starting another."));
        }
        if !self.communities.contains_key(&c) {
            if self.state.public_joins.contains_key(&c) {
                // A retry must ask the same connected carrier for a fresh inventory. Its
                // previous answer may have been exhausted or its response interrupted.
                self.state.joined.remove(&c);
                self.clear_public_join_sync(c);
                self.announce();
            }
            self.state.public_joins.insert(c, PendingPublicJoin { name, error: None });
        }
        let at = self.now();
        let joined = self.state.joined.entry(c).or_insert(Joined { at, pinning });
        if pinning && !joined.pinning {
            joined.pinning = true;
        }
        self.mark_state();
        // Routes from a registry or an announcement are tried, not trusted: kept for this attempt
        // only, until one delivers the community's records.
        self.try_routes(c, peers);
        self.announce();
        self.public_join_status(id)
    }

    pub fn public_join_status(&self, id: &str) -> R<PublicJoinStatus> {
        let c = parse_id(id)?;
        let pending = self.state.public_joins.get(&c);
        let reason = pending.and_then(|p| p.error.clone());
        let status = if reason.is_some() {
            "rejected"
        } else if self.communities.contains_key(&c) && self.state.joined.contains_key(&c) {
            "joined"
        } else if pending.is_some() {
            "pending"
        } else {
            "unknown"
        };
        Ok(PublicJoinStatus { id: c.hex(), status: status.into(), reason })
    }

    /// Public communities that connected members' devices offer and this node neither holds nor
    /// has asked for. A carrier checks them against its index before deciding to carry them;
    /// nothing is joined by merely being offered. A device names a community in the clear only
    /// when its routes led it here (a founder's browser publishing through this carrier); the
    /// rest of its communities arrive salted, and stay unknown to a node that doesn't hold them.
    pub fn offered_unknown(&self) -> Vec<String> {
        let mut out = BTreeSet::new();
        for p in self.peers.values().filter(|p| p.said_hello && !p.client && !p.home) {
            for c in &p.communities {
                if !self.state.joined.contains_key(c) && !self.state.public_joins.contains_key(c) && !self.is_private_community(c) && !self.is_refused(c) {
                    out.insert(c.hex());
                }
            }
        }
        out.into_iter().collect()
    }

    /// Called after signature/content/time validation, before accepting a genesis. A bare
    /// public key never bypasses admission or allows the directory to choose another key.
    pub(crate) fn check_public_join(&mut self, r: &Record) -> Result<(), Reject> {
        let Body::Community(g) = &r.body else { return Ok(()) };
        let c = r.author;
        let Some(pending) = self.state.public_joins.get(&c) else { return Ok(()) };
        let error = if g.limits.admission != Admission::Open {
            Some("This community is not open for public joining. Ask a member about its admission rules.")
        } else if pending.name.as_ref().is_some_and(|name| crate::community_registration_name(&g.name).as_ref() != Some(name)) {
            Some("The signed community name does not match the directory result. No community was joined.")
        } else {
            None
        };
        if let Some(error) = error {
            self.state.public_joins.get_mut(&c).expect("pending join").error = Some(error.into());
            self.state.joined.remove(&c);
            self.state.bootstrap.remove(&c);
            self.trial_routes.remove(&c);
            self.clear_public_join_sync(c);
            self.mark_state();
            self.mark_changed(c);
            self.notice(error);
            self.announce();
            return Err(Reject::NotAllowed(error.into()));
        }
        Ok(())
    }

    pub(crate) fn complete_public_join(&mut self, c: Id) {
        if self.state.public_joins.remove(&c).is_some() {
            self.mark_state();
        }
    }

    pub(crate) fn clear_public_join_sync(&mut self, c: Id) {
        for peer in self.peers.values_mut() {
            peer.wants.remove(&c);
            peer.pending_wants.remove(&c);
            // and reconcile afresh when it comes back (a retry, or leaving and joining again):
            // with `rng` this side asks, so no hello of the peer's resets it
            peer.sent_have.remove(&c);
            peer.ranging.remove(&c);
        }
    }
}
