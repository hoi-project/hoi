//! Configuration declarations, the assurance policy and durable blocks (RFC-0006,
//! PROTOCOL.md §14.7).
//!
//! **Declaring.** A public node with an operator notice also declares how it runs
//! (`hoi_types::NodeDeclaration`): the software its host named ([`Node::set_software`]), the
//! roles it serves with their configured limits, its endpoint key and the notice's hash. The node
//! builds it from its effective configuration, signs it with its endpoint key and sends it beside
//! the notice in every hello to a stranger. A changed setting or build is a new declaration, and
//! connected strangers hear it at once, even while the notice stays the same.
//!
//! **Checking.** Under the consent gate (crate::notice) a person's device checks a declaration
//! against the node that sent it before anything else: it must decode, name the endpoint key the
//! hello names (and the key the transport proved, where it proved one) and the notice beside it,
//! and carry that key's signature. On a WebRTC link the device then says nothing until the node
//! bound the channel to that key (crate::bind). A declaration that fails, or a node that cannot
//! bind, stops the link and is blocked for good ([`Node::block_peer`]): kept across restarts,
//! left out of every dial list, answered with silence, until the person reconsiders. A person's
//! answer holds for one node, one notice and one declaration: a changed declaration, or one that
//! disappeared, is asked about again, and the link stops until then.
//!
//! **What it proves.** The node's own key signed it: it says what the node's software claims,
//! not what runs. A changed program signs whatever it likes, reads what it is sent and can keep
//! it; nothing here detects that. The sentences people read say so.
//!
//! **The policy.** `declared` (the default) uses nodes on their declarations, as above.
//! `verified` uses only participants whose execution was verified under an approved attestation
//! profile (RFC-0006 §2). This version implements none, so under `verified` no public node,
//! relay, holder or home node is eligible: links this device dialed end before it says anything,
//! no route is dialed on its own initiative, and the status says why it is offline. It never
//! falls back to declarations by itself. Strangers that dial in are served as ever: the policy is
//! about whom this device relies on. A headless node takes the policy from its operator's flags
//! (`hoi-node --assurance`) instead of prompts.

use hoi_sync::{Msg, SignedDeclaration};
use hoi_types::declaration::{self, Carry, DECLARATION_VERSION, MAX_SOFTWARE_NAME, MAX_SOFTWARE_VERSION, NodeDeclaration, Software};
use hoi_types::{Id, Sig};

use crate::Node;
use crate::api::R;
use crate::carry::CarryPolicy;
use crate::dto::{AssuranceStatus, Event, NodeBlockView};
use crate::error::ApiError;
use crate::net::{Reach, Role, reach};
use crate::state::NodeBlock;

/// Nodes are used on what they declare, signed by their own key.
pub const DECLARED: &str = "declared";
/// Only verified participants are used; none can be in this version.
pub const VERIFIED: &str = "verified";
/// Blocks a device keeps at most; the oldest go first.
pub(crate) const MAX_BLOCKS: usize = 256;

/// Why a device under `verified` uses no node.
pub const NO_VERIFIER: &str = "This device uses only nodes whose software was verified, and no node offers a verification this version of hoi accepts. Switch to declared configuration to connect again.";
/// Why a device has no node left to use.
pub const NONE_LEFT: &str = "Every node this device knows is blocked, declined or waiting for your answer.";

/// This node's own declaration, as its hellos carry it.
pub(crate) struct OperatorDeclaration {
    pub bytes: Vec<u8>,
    pub hash: Id,
    pub sig: Sig,
}

impl OperatorDeclaration {
    pub(crate) fn signed(&self) -> SignedDeclaration {
        SignedDeclaration { bytes: self.bytes.clone(), sig: self.sig.0 }
    }
}

/// Checks a hello's declaration against the node that sent it: the endpoint key its hello
/// names, the key the transport proved (when it proved one) and the notice beside it. Returns
/// the declaration's hash, which a person's answer names.
pub(crate) fn check(node: &Id, proven: Option<&Id>, notice: &Id, d: &SignedDeclaration) -> Result<Id, &'static str> {
    let parsed = NodeDeclaration::from_canonical(&d.bytes).map_err(|_| "Its configuration declaration is damaged.")?;
    if parsed.endpoint != *node || proven.is_some_and(|p| p != node) {
        return Err("Its configuration declaration names another node.");
    }
    if parsed.notice != *notice {
        return Err("Its configuration declaration is for another notice.");
    }
    let hash = hoi_types::notice::hash_of(&d.bytes);
    if !hoi_crypto::verify(node, declaration::statement(node, &hash).as_bytes(), &Sig(d.sig)) {
        return Err("Its configuration declaration is not signed by the node.");
    }
    Ok(hash)
}

fn block_key(b: &NodeBlock) -> String {
    match (&b.endpoint, b.certs.first()) {
        (Some(e), _) => e.hex(),
        (None, Some(c)) => format!("cert:{c}"),
        (None, None) => String::new(),
    }
}

fn view_of(b: &NodeBlock) -> NodeBlockView {
    NodeBlockView { key: block_key(b), endpoint: b.endpoint.map(|e| e.hex()), certs: b.certs.clone(), reason: b.reason.clone(), at: b.at, host: b.host.clone() }
}

impl Node {
    /// The host names the software this node runs: what its declaration says. `digest` is the
    /// BLAKE3 of the program the host started (hex), when it could read it.
    pub fn set_software(&mut self, name: &str, version: &str, digest: Option<&str>) -> R<()> {
        let (name, version) = (name.trim(), version.trim());
        if name.is_empty() || name.chars().count() > MAX_SOFTWARE_NAME || version.is_empty() || version.chars().count() > MAX_SOFTWARE_VERSION {
            return Err(ApiError::invalid("Name the software and its version, each in at most 32 characters."));
        }
        if name.chars().chain(version.chars()).any(char::is_control) {
            return Err(ApiError::invalid("No control characters in the software's name or version."));
        }
        let digest = digest.map(|d| Id::from_hex(d).map_err(|_| ApiError::invalid("A software digest is 64 hex characters."))).transpose()?;
        self.software = Some(Software { name: name.into(), version: version.into(), digest });
        self.redeclare();
        Ok(())
    }

    /// What this node declares now: nothing without a notice, a software name and an endpoint
    /// key. Configured limits, not today's counters: a relay cap used up is no new configuration.
    fn declaration_now(&self) -> Option<NodeDeclaration> {
        let notice = self.operator_notice.as_ref()?.hash;
        let software = self.software.clone()?;
        let endpoint = self.transport_kp.as_ref()?.public();
        let relays = self.can_relay() && self.state.settings.relay;
        let carry = match self.carry_policy {
            CarryPolicy::Any if self.accepts_carry() => Some("any"),
            CarryPolicy::Index if self.can_relay() => Some("index"),
            _ => None,
        }
        .map(|policy| Carry { policy: policy.into(), communities: self.carry_limit as u64, bytes: self.carry_bytes });
        Some(NodeDeclaration {
            v: DECLARATION_VERSION,
            endpoint,
            notice,
            software,
            relay: relays.then(|| self.relay_cap_bytes()),
            carry,
            pieces: self.piece_stats().map(|s| s.budget),
            // as `forwards_pieces`, by configuration
            forwards: relays && self.accepts_carry(),
            home: self.serving_url().is_some(),
            unknown: false,
        })
    }

    /// Builds and signs the declaration again; true when it changed.
    pub(crate) fn refresh_declaration(&mut self) -> bool {
        let next = self.declaration_now().filter(|d| d.validate().is_empty());
        let hash = next.as_ref().map(NodeDeclaration::hash);
        if hash == self.operator_declaration.as_ref().map(|d| d.hash) {
            return false;
        }
        self.operator_declaration = match (next, self.transport_kp.as_ref()) {
            (Some(d), Some(kp)) => {
                let bytes = d.canonical();
                let hash = hoi_types::notice::hash_of(&bytes);
                let sig = kp.sign(declaration::statement(&kp.public(), &hash).as_bytes());
                Some(OperatorDeclaration { bytes, hash, sig })
            }
            _ => None,
        };
        true
    }

    /// After a change to what this node declares: connected strangers hear it in a new hello.
    pub(crate) fn redeclare(&mut self) {
        if self.refresh_declaration() {
            self.rehello_strangers();
        }
    }

    /// This node's own declaration, as people who connect read it.
    pub fn operator_declaration(&self) -> Option<NodeDeclaration> {
        self.operator_declaration.as_ref().and_then(|d| NodeDeclaration::from_canonical(&d.bytes).ok())
    }

    /// The hash (hex) of this node's own declaration, when it sends one.
    pub fn operator_declaration_hash(&self) -> Option<String> {
        self.operator_declaration.as_ref().map(|d| d.hash.hex())
    }

    /// The person (or a headless node's operator) chose `verified`: only verified participants.
    pub(crate) fn strict(&self) -> bool {
        self.state.settings.assurance == VERIFIED
    }

    /// Under `verified`, whether this link leads to a participant this device would rely on and
    /// that is not verified: a stranger it dialed, or its home node. None is verified in this
    /// version. Its own devices, and strangers that dialed in, are not relied on.
    pub(crate) fn unverified_relied_on(&self, peer: &str) -> bool {
        self.strict() && self.peers.get(peer).is_some_and(|p| p.home || (p.role() == Role::Peer && !p.meta.incoming))
    }

    /// The policy changed: under `verified`, links to participants this device relies on stop now
    /// (queued messages dropped, a goodbye that names the policy and nothing else).
    pub(crate) fn apply_assurance(&mut self) {
        let relied: Vec<String> = self.peers.keys().filter(|k| self.unverified_relied_on(k)).cloned().collect();
        for peer in relied {
            self.stop_peer(&peer);
            self.send_msg(&peer, Msg::Bye { reason: "this device uses only verified nodes".into() });
        }
    }

    /// Nothing queued for `peer` goes out any more.
    pub(crate) fn stop_peer(&mut self, peer: &str) {
        self.outbox.retain(|(to, _)| to != peer);
    }

    /// The policy, and why this device uses no node when it uses none for that reason.
    pub(crate) fn assurance_status(&self) -> AssuranceStatus {
        let policy = self.state.settings.assurance.clone();
        let blocked = self.state.node_blocks.len() as u64;
        if self.strict() {
            return AssuranceStatus { policy, offline: true, reason: Some(NO_VERIFIER.into()), blocked };
        }
        let held_back = blocked > 0 || self.state.notices.values().any(|r| r.accepted.is_none());
        let linked = self.peers.values().any(|p| p.said_hello && !p.client && !p.awaiting_hello);
        let none_left = held_back && !linked && self.discovery_candidates().is_empty() && self.bootstrap_hints().is_empty();
        AssuranceStatus { policy, offline: none_left, reason: none_left.then(|| NONE_LEFT.into()), blocked }
    }

    /// The link `peer` failed a check (RFC-0006 §4): nothing more goes to it, and what its
    /// connection proved (the endpoint key the transport or a binding showed, the certificates
    /// its routes pin) is blocked for good. Never the key a hello merely names: someone else's
    /// claim must not shut a node out. True when something was blocked.
    pub(crate) fn block_peer(&mut self, peer: &str, reason: &str) -> bool {
        self.stop_peer(peer);
        let Some(p) = self.peers.get(peer) else { return false };
        let endpoint = p.meta.endpoint.as_deref().and_then(|e| Id::from_hex(e).ok()).or(if p.bound { p.node_key } else { None });
        let mut certs: Vec<String> = p.meta.reaches().into_iter().filter_map(|r| if let Reach::Cert(fp) = r { Some(fp) } else { None }).collect();
        certs.sort();
        certs.dedup();
        if endpoint.is_none() && certs.is_empty() {
            return false;
        }
        let host = p.meta.addr.clone().or_else(|| p.meta.dialed.first().and_then(|r| r.split_once('@')).map(|(_, a)| a.to_string()));
        let now = self.now();
        // one entry per node: a block met again is renewed with the latest reason
        self.state.node_blocks.retain(|b| !(endpoint.is_some() && b.endpoint == endpoint) && !b.certs.iter().any(|c| certs.contains(c)));
        while self.state.node_blocks.len() >= MAX_BLOCKS {
            self.state.node_blocks.remove(0);
        }
        let block = NodeBlock { endpoint, certs, reason: reason.into(), at: now, host };
        let key = block_key(&block);
        self.state.node_blocks.push(block);
        self.mark_state();
        self.events.push(Event::NodeBlocked { key });
        self.settle_dirty();
        true
    }

    /// Whether a route leads to a blocked node.
    pub(crate) fn route_blocked(&self, route: &str) -> bool {
        match reach(route) {
            Some(Reach::Endpoint(id)) => self.state.node_blocks.iter().any(|b| b.endpoint == Some(id)),
            Some(Reach::Cert(fp)) => self.state.node_blocks.iter().any(|b| b.certs.contains(&fp)),
            None => false,
        }
    }

    /// Whether the connection `peer` proved itself to be a blocked node.
    pub(crate) fn peer_blocked(&self, peer: &str) -> bool {
        if self.state.node_blocks.is_empty() {
            return false;
        }
        let Some(p) = self.peers.get(peer) else { return false };
        p.meta.reaches().into_iter().any(|r| match r {
            Reach::Endpoint(id) => self.state.node_blocks.iter().any(|b| b.endpoint == Some(id)),
            Reach::Cert(fp) => self.state.node_blocks.iter().any(|b| b.certs.contains(&fp)),
        })
    }

    /// Nodes this device blocked, newest first.
    pub fn node_blocks(&self) -> Vec<NodeBlockView> {
        self.state.node_blocks.iter().rev().map(view_of).collect()
    }

    /// The person reconsiders a block: the node may be dialed again, and is asked about anew.
    pub fn unblock_node(&mut self, key: &str) -> R<()> {
        let before = self.state.node_blocks.len();
        self.state.node_blocks.retain(|b| block_key(b) != key);
        if self.state.node_blocks.len() == before {
            return Err(ApiError::not_found("block"));
        }
        self.mark_state();
        self.settle_dirty();
        Ok(())
    }
}
