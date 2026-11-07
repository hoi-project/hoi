//! Operator notices (PROTOCOL.md §14.5): informed before connected.
//!
//! A public node's operator tells people what it processes and keeps and, when they choose to,
//! who runs it, where, and how to reach them (`hoi_types::NodeNotice`). On the node, [`Node::set_notice`] puts the notice
//! into every hello to a stranger (`nh`, `nt`) and, signed by the endpoint key, into the node
//! link, so a link's reader sees it before dialing anything. On a person's device, the consent
//! gate ([`Node::set_consent_gate`], on in browsers, the desktop and phones, never on a headless
//! node) makes a stranger this device dialed hear nothing until its hello was read: no notice in
//! it, an older node or a member's device, and our hello follows at once; a notice the person
//! accepted before, the same; otherwise the link ends there, the words are kept to be shown, and
//! the node is dialed again once the person accepted. A declined node is left alone until the
//! person forgets it. A link the person opened is refused until its notice, checked against the
//! node it names, was accepted.
//!
//! The node has seen this device's address by then, as any connection shows it; the app's own
//! Network & Safety notice says so before the first connection is ever made. What the device
//! says of itself (its communities, hashed or named) waits for the person's answer.

use hoi_types::notice::{self, MAX_NOTICE_WIRE};
use hoi_types::{Id, NodeLink, NodeNotice, Sig};

use crate::Node;
use crate::api::R;
use crate::dto::{Event, NodeLinkPreview, NoticeSection, NoticeView};
use crate::error::ApiError;
use crate::net::{Reach, Role, reach};
use crate::state::NoticeRecord;

/// Notices a device keeps, decided or not, at most; the oldest undecided go first.
pub(crate) const MAX_NOTICES: usize = 64;

/// This node's own notice, as its operator wrote it, with what hellos and the link carry.
pub(crate) struct OperatorNotice {
    pub notice: NodeNotice,
    pub bytes: Vec<u8>,
    pub hash: Id,
    /// Over [`notice::statement`] by the endpoint key; none while a node has no transport key.
    pub signature: Option<Sig>,
}

/// What a stranger's hello means for the link under the consent gate.
pub(crate) enum Consent {
    Proceed,
    /// Close the link, quietly: the notice waits for the person, or was declined.
    Close,
    /// The hello lied about its notice: the peer's fault.
    Penalize,
}

/// Bytes as base64url in the saved state (a notice as JSON numbers would be four times the size).
pub(crate) mod b64bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hoi_types::uri::b64(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        hoi_types::uri::unb64(&text).map_err(serde::de::Error::custom)
    }
}

fn state_word(r: &NoticeRecord) -> &'static str {
    if r.accepted.is_some() {
        "accepted"
    } else if r.declined.is_some() {
        "declined"
    } else {
        "pending"
    }
}

fn view_of(node: &Id, r: &NoticeRecord) -> NoticeView {
    let n = NodeNotice::from_canonical(&r.notice).unwrap_or_default();
    NoticeView {
        node: node.hex(),
        hash: r.hash.hex(),
        state: state_word(r).into(),
        title: n.title(),
        sections: n.sections().into_iter().map(|s| NoticeSection { title: s.title, text: s.text }).collect(),
        footer: format!("Notice version {}, last updated {}.", &r.hash.hex()[..12], n.updated.trim()),
        name: n.node.clone(),
        operator: n.operator.clone(),
        contact: n.contact.clone(),
        country: n.country.clone(),
        provider: n.provider.clone(),
        ip_logging: n.ip_logging,
        retention_days: u32::from(n.retention_days),
        updated: n.updated.clone(),
        host: r.host.clone(),
        seen: r.seen,
        decided_at: r.accepted.or(r.declined),
    }
}

/// The certificate fingerprints among `routes`.
fn certs_of(routes: &[String]) -> Vec<String> {
    routes.iter().filter_map(|r| r.strip_prefix("rtc:")).filter_map(|r| r.split_once('@')).map(|(fp, _)| fp.to_string()).collect()
}

impl Node {
    /// A person's device reads strangers' notices before it says anything, and a link's before it
    /// connects. Browsers, the desktop and phones turn this on; a headless node never does.
    pub fn set_consent_gate(&mut self, on: bool) {
        self.consent_gate = on;
    }

    pub fn consent_gate(&self) -> bool {
        self.consent_gate
    }

    /// This node's operator notice, for its hellos to strangers and its link; `None` takes it
    /// away. Returns the notice's hash (hex). A notice with problems is refused with them.
    pub fn set_notice(&mut self, notice: Option<NodeNotice>) -> R<Option<String>> {
        let Some(mut notice) = notice else {
            if self.operator_notice.take().is_some() {
                self.rehello_strangers();
            }
            return Ok(None);
        };
        // Whether this node keeps pieces for others is how it runs, not what the file says: the
        // fixed sentence about it is never missing and never claimed by a node that doesn't.
        notice.pieces = self.holds_pieces();
        let problems = notice.validate();
        if !problems.is_empty() {
            return Err(ApiError::invalid(problems.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")));
        }
        let bytes = notice.canonical();
        let hash = notice::hash_of(&bytes);
        if self.operator_notice.as_ref().is_some_and(|n| n.hash == hash) {
            return Ok(Some(hash.hex()));
        }
        let kp = hoi_crypto::Keypair::from_secret(&self.transport_secret()?);
        let signature = Some(kp.sign(notice::statement(&kp.public(), &hash).as_bytes()));
        self.operator_notice = Some(OperatorNotice { notice, bytes, hash, signature });
        self.rehello_strangers();
        self.settle_dirty();
        Ok(Some(hash.hex()))
    }

    /// The hash (hex) of this node's own notice, when it has one.
    pub fn operator_notice_hash(&self) -> Option<String> {
        self.operator_notice.as_ref().map(|n| n.hash.hex())
    }

    /// This node's own notice, as its operator wrote it.
    pub fn operator_notice(&self) -> Option<&NodeNotice> {
        self.operator_notice.as_ref().map(|n| &n.notice)
    }

    /// Connected strangers hear the notice as it is now: a hello replacing the last one.
    fn rehello_strangers(&mut self) {
        let strangers: Vec<String> =
            self.peers.iter().filter(|(_, p)| p.role() == Role::Peer && p.said_hello && !p.awaiting_hello).map(|(k, _)| k.clone()).collect();
        for peer in strangers {
            self.rehello(&peer);
        }
    }

    /// A stranger's hello under the consent gate: what its notice means for this link.
    pub(crate) fn consent_check(&mut self, peer: &str, node: &Id, hash: Option<&Id>, bytes: Option<&[u8]>) -> Consent {
        let Some(hash) = hash else { return Consent::Proceed };
        let Some(bytes) = bytes else { return Consent::Penalize };
        if bytes.len() > MAX_NOTICE_WIRE || notice::hash_of(bytes) != *hash || NodeNotice::from_canonical(bytes).is_err() {
            return Consent::Penalize;
        }
        let now = self.now();
        match self.state.notices.get_mut(node) {
            Some(r) if r.hash == *hash && r.accepted.is_some() => {
                r.seen = now;
                self.mark_state();
                Consent::Proceed
            }
            Some(r) if r.hash == *hash && r.declined.is_some() => Consent::Close,
            _ => {
                let (certs, host) = match self.peers.get(peer) {
                    Some(p) => {
                        let mut certs = certs_of(&p.meta.dialed);
                        certs.extend(p.meta.reaches().into_iter().filter_map(|x| match x {
                            Reach::Cert(fp) => Some(fp),
                            Reach::Endpoint(_) => None,
                        }));
                        certs.sort();
                        certs.dedup();
                        let host = p.meta.addr.clone().or_else(|| p.meta.dialed.first().and_then(|r| r.split_once('@')).map(|(_, a)| a.to_string()));
                        (certs, host)
                    }
                    None => (Vec::new(), None),
                };
                self.remember_notice(
                    *node,
                    NoticeRecord { hash: *hash, notice: bytes.to_vec(), certs, host, seen: now, asked: true, accepted: None, declined: None },
                );
                self.events.push(Event::NodeNotice { node: node.hex() });
                self.consent_closed.insert(peer.to_string());
                Consent::Close
            }
        }
    }

    /// A pending record for a node whose notice the person has not answered: a new notice from
    /// a node known before replaces the old answer, since the words changed.
    fn remember_notice(&mut self, node: Id, record: NoticeRecord) {
        let asked = record.asked;
        if let Some(r) = self.state.notices.get_mut(&node) {
            let mut certs = std::mem::take(&mut r.certs);
            certs.extend(record.certs.iter().cloned());
            certs.sort();
            certs.dedup();
            *r = NoticeRecord { certs, host: record.host.or_else(|| r.host.take()), asked: asked || r.asked, ..record };
        } else {
            while self.state.notices.len() >= MAX_NOTICES {
                let oldest =
                    self.state.notices.iter().min_by_key(|(_, r)| (r.accepted.is_some(), r.seen)).map(|(k, _)| *k).expect("a full map has an oldest entry");
                self.state.notices.remove(&oldest);
            }
            self.state.notices.insert(node, record);
        }
        self.mark_state();
    }

    /// Whether the host closed `peer` for a notice the person has yet to read: no refusal to
    /// count, no backoff; the node is dialed again once the person answered.
    pub fn consent_closed(&self, peer: &str) -> bool {
        self.consent_closed.contains(peer)
    }

    /// Whether a route leads to a node whose notice waits for an answer or was declined: such a
    /// node is not dialed on this device's own initiative.
    pub(crate) fn consent_blocks(&self, route: &str) -> bool {
        if !self.consent_gate || self.state.notices.is_empty() {
            return false;
        }
        match reach(route) {
            Some(Reach::Endpoint(id)) => self.state.notices.get(&id).is_some_and(|r| r.accepted.is_none()),
            Some(Reach::Cert(fp)) => self.state.notices.values().any(|r| r.accepted.is_none() && r.certs.contains(&fp)),
            None => false,
        }
    }

    /// Every notice this device keeps, newest first: pending ones first among equals.
    pub fn node_notices(&self) -> Vec<NoticeView> {
        let mut out: Vec<NoticeView> = self.state.notices.iter().map(|(k, r)| view_of(k, r)).collect();
        out.sort_by_key(|v| (v.state != "pending", std::cmp::Reverse(v.seen)));
        out
    }

    /// Notices nodes asked about on a connection and the person has not answered, oldest first:
    /// what the app shows unasked. One read from a link waits on the link's page.
    pub(crate) fn pending_notices(&self) -> Vec<NoticeView> {
        let mut out: Vec<NoticeView> =
            self.state.notices.iter().filter(|(_, r)| r.asked && r.accepted.is_none() && r.declined.is_none()).map(|(k, r)| view_of(k, r)).collect();
        out.sort_by_key(|v| v.seen);
        out
    }

    fn notice_key(node: &str) -> R<Id> {
        Id::from_hex(node).map_err(|_| ApiError::invalid("A notice is accepted for a node by its endpoint key."))
    }

    /// The person read the notice and accepts it: the node is dialed again, and its hellos with
    /// this notice go through. The hash says which words were accepted.
    pub fn accept_notice(&mut self, node: &str, hash: &str) -> R<NoticeView> {
        let key = Self::notice_key(node)?;
        let now = self.now();
        let Some(r) = self.state.notices.get_mut(&key) else { return Err(ApiError::not_found("notice")) };
        if r.hash.hex() != hash {
            return Err(ApiError::invalid("The operator changed this notice since; read the new one."));
        }
        r.accepted = Some(now);
        r.declined = None;
        let certs = r.certs.clone();
        let view = view_of(&key, r);
        self.mark_state();
        // Dial it at the next chance, whatever a dial before the answer looked like.
        self.node_failed.retain(|x, _| *x != Reach::Endpoint(key) && !matches!(x, Reach::Cert(fp) if certs.contains(fp)));
        self.settle_dirty();
        Ok(view)
    }

    /// The person declines: the node is not dialed again, and a link to it is answered with a
    /// goodbye, until the notice is forgotten.
    pub fn decline_notice(&mut self, node: &str) -> R<NoticeView> {
        let key = Self::notice_key(node)?;
        let now = self.now();
        let Some(r) = self.state.notices.get_mut(&key) else { return Err(ApiError::not_found("notice")) };
        r.declined = Some(now);
        r.accepted = None;
        let certs = r.certs.clone();
        let view = view_of(&key, r);
        self.mark_state();
        let connected: Vec<String> = self
            .peers
            .iter()
            .filter(|(_, p)| {
                p.role() == Role::Peer && p.meta.reaches().iter().any(|x| *x == Reach::Endpoint(key) || matches!(x, Reach::Cert(fp) if certs.contains(fp)))
            })
            .map(|(k, _)| k.clone())
            .collect();
        for peer in connected {
            self.send_msg(&peer, hoi_sync::Msg::Bye { reason: "declined".into() });
        }
        self.settle_dirty();
        Ok(view)
    }

    /// Forgets a notice and the answer to it: the next contact asks again.
    pub fn forget_notice(&mut self, node: &str) -> R<()> {
        let key = Self::notice_key(node)?;
        if self.state.notices.remove(&key).is_none() {
            return Err(ApiError::not_found("notice"));
        }
        self.mark_state();
        self.settle_dirty();
        Ok(())
    }

    /// Checks a link's notice against the node it names: the hash and the endpoint, when the
    /// signature verifies.
    fn verified_link_notice(parsed: &NodeLink) -> Result<Option<(Id, Id)>, &'static str> {
        let Some(bytes) = &parsed.notice else { return Ok(None) };
        let Some(sig) = &parsed.signature else { return Err("The notice in this link is not signed.") };
        if bytes.len() > MAX_NOTICE_WIRE || NodeNotice::from_canonical(bytes).is_err() {
            return Err("The notice in this link is damaged.");
        }
        let Some(endpoint) = parsed.endpoint() else { return Err("This link carries a notice but names no node to check it against.") };
        let hash = notice::hash_of(bytes);
        if !hoi_crypto::verify(&endpoint, notice::statement(&endpoint, &hash).as_bytes(), sig) {
            return Err("The notice in this link is not signed by the node it names.");
        }
        Ok(Some((endpoint, hash)))
    }

    /// Under the consent gate, a link whose notice the person has not accepted is not opened.
    pub(crate) fn link_consent(&self, parsed: &NodeLink) -> R<()> {
        if !self.consent_gate {
            return Ok(());
        }
        match Self::verified_link_notice(parsed) {
            Ok(None) => Ok(()),
            Ok(Some((endpoint, hash))) => match self.state.notices.get(&endpoint) {
                Some(r) if r.hash == hash && r.accepted.is_some() => Ok(()),
                _ => Err(ApiError::invalid("Read and accept the operator's notice first.")),
            },
            Err(problem) => Err(ApiError::invalid(problem)),
        }
    }

    /// What a node link says, before anything is dialed: where it points and, when it carries
    /// the operator's notice, the notice (kept as pending, to be accepted by its hash) and
    /// whether the node the link names signed it.
    pub fn preview_node_link(&mut self, link: &str) -> R<NodeLinkPreview> {
        let parsed =
            NodeLink::parse(link).map_err(|_| ApiError::invalid("That isn't a node link. A node link starts with hoi://node? or has #/node? in it."))?;
        let first = parsed
            .rtc
            .iter()
            .chain(&parsed.peers)
            .find_map(|r| crate::nodes::route_addr(r))
            .or_else(|| parsed.peers.first().and_then(|p| crate::nodes::route_addr(p)));
        let host = first.map(|a| a.to_string()).unwrap_or_default();
        let scope = first.map(|a| crate::nodes::ip_scope(a.ip()).word()).unwrap_or("internet").to_string();
        let endpoint = parsed.endpoint();
        let mut preview = NodeLinkPreview { host, scope, endpoint: endpoint.map(|e| e.hex()), notice: None, verified: false, problem: None };
        match Self::verified_link_notice(&parsed) {
            Ok(None) => {}
            Ok(Some((endpoint, hash))) => {
                preview.verified = true;
                let known = self.state.notices.get(&endpoint).is_some_and(|r| r.hash == hash);
                if !known {
                    let bytes = parsed.notice.clone().unwrap_or_default();
                    let certs = certs_of(&parsed.rtc);
                    let host = Some(preview.host.clone()).filter(|h| !h.is_empty());
                    let now = self.now();
                    self.remember_notice(endpoint, NoticeRecord { hash, notice: bytes, certs, host, seen: now, asked: false, accepted: None, declined: None });
                    self.settle_dirty();
                }
                preview.notice = self.state.notices.get(&endpoint).map(|r| view_of(&endpoint, r));
            }
            Err(problem) => preview.problem = Some(problem.into()),
        }
        Ok(preview)
    }
}

impl Default for NoticeView {
    fn default() -> Self {
        NoticeView {
            node: String::new(),
            hash: String::new(),
            state: "pending".into(),
            title: String::new(),
            sections: Vec::new(),
            footer: String::new(),
            name: String::new(),
            operator: String::new(),
            contact: String::new(),
            country: String::new(),
            provider: None,
            ip_logging: false,
            retention_days: 0,
            updated: String::new(),
            host: None,
            seen: 0,
            decided_at: None,
        }
    }
}
