//! Linking a new device without copying codes, through the home node both devices share
//! (PROTOCOL.md §14, `link`). The new device pairs with the home node (one QR scan), then asks;
//! every device paired with that home node hears the ask; a device holding your root key shows
//! it with four check digits, the new device shows the same four, and once you say yes the root
//! device answers with the same approval the manual codes carry. Messages are sealed with the
//! home node's link key, which it gives each device it pairs (`hello.lk`). The home node also
//! holds that key, so this envelope does not hide linking metadata from it. Signed device
//! consent and root approval establish authority. Private community geneses and MLS keys are
//! excluded from ordinary linking.
//!
//! The digits are a commit-then-reveal comparison, like Bluetooth's numeric comparison: the new
//! device commits to a nonce in its request, each root device answers with a nonce of its own,
//! the new device reveals, and both sides derive the digits from the device key and both
//! nonces. Neither side can steer the result, so another device paired with the same home can't
//! make its own request show the digits your phone shows, nor make your phone show the digits
//! of its request.
//!
//! Nothing here can bless a device by itself: the root device still blesses, and the new device
//! still signs its own consent.

use std::collections::BTreeSet;

use hoi_sync::{FEATURE_LINK, Msg};
use hoi_types::Id;
use hoi_types::cbor::{self, Value};

use crate::api::R;
use crate::dto::{Event, LinkAsk, LinkStarted};
use crate::error::ApiError;
use crate::node::Node;

const MAILBOX_CONTEXT: &str = "hoi 2026 link mailbox v1";
const BOX_CONTEXT: &str = "hoi 2026 link box v1";
const COMMIT_CONTEXT: &str = "hoi 2026 link commit v1";
const CHECK_CONTEXT: &str = "hoi 2026 link check v1";
/// An ask nobody answered within this is dropped.
const ASK_TTL: u64 = 600;
/// Asks kept at once. When more come, the oldest whose digits aren't on screen yet makes room.
const MAX_ASKS: usize = 4;
/// Challenges a new device answers, one per root device: more would mean someone's guessing.
const MAX_CHALLENGES: usize = 3;
/// `link` messages a device may send per minute before they are ignored. Messages from our own
/// home node aren't counted: it passes on what all our devices send, each already limited there.
const LINKS_PER_MINUTE: u32 = 12;
/// What an approval may carry in community geneses, so it stays well under the message cap.
const NETWORK_GENESES: usize = 64;
const NETWORK_GENESIS_BYTES: usize = 256 * 1024;

pub(crate) struct PendingAsk {
    request: String,
    device: Id,
    label: String,
    secret: Vec<u8>,
    /// The new device's commitment, and the nonce we answered it with.
    commit: [u8; 32],
    ours: [u8; 16],
    /// The four digits, once the new device revealed its nonce.
    check: Option<String>,
    /// The connection it came on, when that is one of our own clients; `None` when our home
    /// node passed it on, from whichever of its devices.
    from: Option<String>,
    at: u64,
}

pub(crate) struct Waiting {
    request: String,
    device: Id,
    /// Committed to in the request, revealed to each root device that answers.
    nonce: [u8; 32],
    /// Connections the request went out on; a reconnect sends it again.
    sent: BTreeSet<String>,
    /// Root devices' nonces answered so far, with the digits each gives.
    checks: Vec<([u8; 16], String)>,
}

pub(crate) fn mailbox(secret: &[u8]) -> [u8; 16] {
    let k = blake3::derive_key(MAILBOX_CONTEXT, secret);
    k[..16].try_into().expect("16 of 32 bytes")
}

fn commitment(nonce: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key(COMMIT_CONTEXT, nonce)
}

/// Four digits from the device key and both sides' nonces.
pub(crate) fn check_code(device: &Id, theirs: &[u8; 32], ours: &[u8; 16]) -> String {
    let mut input = Vec::with_capacity(80);
    input.extend_from_slice(&device.0);
    input.extend_from_slice(theirs);
    input.extend_from_slice(ours);
    let h = blake3::derive_key(CHECK_CONTEXT, &input);
    format!("{:04}", u32::from_le_bytes(h[..4].try_into().expect("4 bytes")) % 10_000)
}

fn body(fields: Vec<(&str, Value)>) -> Vec<u8> {
    Value::map(fields).encode()
}

/// A `link` message body, for crate::remote.
pub(crate) fn body_of(fields: Vec<(&str, Value)>) -> Vec<u8> {
    body(fields)
}

/// A root device's answer to a request: its nonce for that device.
fn challenge(device: &Id, ours: &[u8; 16]) -> Vec<u8> {
    body(vec![("t", Value::T("chk".into())), ("device", Value::B(device.0.to_vec())), ("n", Value::B(ours.to_vec()))])
}

fn bytes<const N: usize>(v: Option<&Value>) -> Option<[u8; N]> {
    match v {
        Some(Value::B(b)) => b.as_slice().try_into().ok(),
        _ => None,
    }
}

impl Node {
    fn serve_secret(&self) -> Option<Vec<u8>> {
        self.serving.as_ref()?;
        self.link_key().map(|k| k.to_vec())
    }

    fn home_secret(&self) -> Option<Vec<u8>> {
        self.state.home.as_ref()?.link_key.as_deref().and_then(|k| hoi_types::uri::unb64(k).ok()).filter(|k| k.len() == 32)
    }

    /// The secret a connection's `link` messages are sealed with: our link key for our own
    /// devices, the one our home node gave us on the link to it. Each device's credential is its
    /// own, so it can't be this secret.
    fn link_secret_for(&self, peer: &str) -> Option<Vec<u8>> {
        let p = self.peers.get(peer)?;
        if !p.said_hello || !p.features.contains(FEATURE_LINK) {
            return None;
        }
        if p.client {
            self.serve_secret()
        } else if p.home {
            self.home_secret()
        } else {
            None
        }
    }

    fn send_link(&mut self, peer: &str, secret: &[u8], plain: &[u8]) {
        if let Ok(sealed) = hoi_crypto::sealed::seal(BOX_CONTEXT, secret, plain) {
            self.send_msg(peer, Msg::Link { mailbox: mailbox(secret), sealed });
        }
    }

    /// [`Node::send_link`] for crate::remote: an answer on the connection the ask came on.
    pub(crate) fn send_link_to(&mut self, peer: &str, secret: &[u8], plain: &[u8]) {
        self.send_link(peer, secret, plain);
    }

    /// Sends to our home node, which passes it on to its other devices. Returns how many links
    /// took it.
    pub(crate) fn send_link_home(&mut self, plain: &[u8]) -> usize {
        let targets: Vec<(String, Vec<u8>)> =
            self.peers.iter().filter(|(_, p)| p.home).filter_map(|(k, _)| Some((k.clone(), self.link_secret_for(k)?))).collect();
        for (p, secret) in &targets {
            self.send_link(p, secret, plain);
        }
        targets.len()
    }

    /// Sends to every device that may carry it under `secret`.
    fn send_link_all(&mut self, secret: &[u8], plain: &[u8]) {
        let targets: Vec<String> = self.peers.keys().filter(|p| self.link_secret_for(p).as_deref() == Some(secret)).cloned().collect();
        for p in targets {
            self.send_link(&p, secret, plain);
        }
    }

    pub(crate) fn send_own_link(&mut self, plain: &[u8]) {
        let targets: Vec<_> = self.peers.keys().filter_map(|p| Some((p.clone(), self.link_secret_for(p)?))).collect();
        for (peer, secret) in targets {
            self.send_link(&peer, &secret, plain);
        }
    }

    /// On a new device paired with your home node: ask your other devices to link this one. The
    /// four digits to compare arrive with a `linkCheck` event once a device with your key answers.
    pub fn link_home(&mut self, label: &str) -> R<LinkStarted> {
        self.link_home_as(label, false)
    }

    /// Like [`Node::link_home`], for a browser that keeps no signing key: the device with your
    /// root makes this browser's key, keeps it and signs for it (crate::remote).
    pub fn link_home_remote(&mut self, label: &str) -> R<LinkStarted> {
        self.link_home_as(label, true)
    }

    fn link_home_as(&mut self, label: &str, remote: bool) -> R<LinkStarted> {
        if self.state.me.is_some() {
            return Err(ApiError::not_allowed("This device already has a key."));
        }
        if self.state.home.is_none() && self.serving.is_none() {
            return Err(ApiError::not_allowed("Pair with your home node first: scan its code or open its pairing link."));
        }
        let request = self.link_request_as(label, remote)?;
        let device = self.pending_kp.as_ref().map(|k| k.public()).ok_or_else(|| ApiError::new("internal", "no pending key"))?;
        let nonce = hoi_crypto::random_bytes::<32>()?;
        self.link_waiting = Some(Waiting { request, device, nonce, sent: BTreeSet::new(), checks: Vec::new() });
        let sent = self.send_link_request();
        Ok(LinkStarted { sent })
    }

    /// Sends a waiting request on every connection that can carry it and hasn't had it.
    pub(crate) fn send_link_request(&mut self) -> u64 {
        let Some(w) = &self.link_waiting else { return 0 };
        let plain = body(vec![("t", Value::T("req".into())), ("code", Value::T(w.request.clone())), ("c", Value::B(commitment(&w.nonce).to_vec()))]);
        let targets: Vec<(String, Vec<u8>)> =
            self.peers.keys().filter(|p| !w.sent.contains(*p)).filter_map(|p| Some((p.clone(), self.link_secret_for(p)?))).collect();
        for (p, secret) in &targets {
            self.send_link(p, secret, &plain);
        }
        if let Some(w) = &mut self.link_waiting {
            w.sent.extend(targets.iter().map(|(p, _)| p.clone()));
        }
        targets.len() as u64
    }

    pub(crate) fn link_peer_gone(&mut self, peer: &str) {
        if let Some(w) = &mut self.link_waiting {
            w.sent.remove(peer);
        }
    }

    pub fn link_cancel(&mut self) {
        self.link_waiting = None;
    }

    /// Asks ready for an answer here (their digits are known), oldest first. Asks for devices
    /// that got linked meanwhile (their blessing arrived from another of your devices) are gone.
    pub fn link_asks(&self) -> Vec<LinkAsk> {
        let now = self.now();
        self.link_asks
            .iter()
            .filter(|(_, a)| self.ask_shown(a, now))
            .filter_map(|(id, a)| Some(LinkAsk { id: *id, label: a.label.clone(), check: a.check.clone()?, device: a.device.hex(), at: a.at }))
            .collect()
    }

    /// Whether an ask's digits are on the person's screen: revealed, not timed out, and its
    /// device not linked meanwhile.
    fn ask_shown(&self, a: &PendingAsk, now: u64) -> bool {
        a.check.is_some() && now.saturating_sub(a.at) < ASK_TTL && self.store.delegation(&a.device).is_none()
    }

    /// On a device holding your root key: say yes or no to an ask.
    pub fn link_answer(&mut self, id: u64, yes: bool) -> R<()> {
        let ready = self.link_asks.get(&id).is_some_and(|a| a.check.is_some());
        let ask = self.link_asks.remove(&id).filter(|_| ready).ok_or_else(|| ApiError::not_found("link request (it may have timed out)"))?;
        if self.now().saturating_sub(ask.at) >= ASK_TTL {
            return Err(ApiError::new("expired", "That request timed out. Ask again from the new device."));
        }
        if yes {
            let approval = self.link_approve_with(&ask.request, NETWORK_GENESES, NETWORK_GENESIS_BYTES)?;
            self.send_link_all(&ask.secret, &body(vec![("t", Value::T("ok".into())), ("code", Value::T(approval))]));
        } else {
            self.send_link_all(&ask.secret, &body(vec![("t", Value::T("no".into())), ("device", Value::B(ask.device.0.to_vec()))]));
        }
        Ok(())
    }

    /// A `link` message arrived. Returns false when the connection should be dropped.
    pub(crate) fn on_link(&mut self, peer: &str, m: [u8; 16], sealed: Vec<u8>) -> bool {
        let now = self.now();
        let Some(p) = self.peers.get_mut(peer) else { return false };
        if !p.home {
            if now / 60 != p.link_minute {
                p.link_minute = now / 60;
                p.link_msgs = 0;
            }
            p.link_msgs += 1;
            if p.link_msgs > LINKS_PER_MINUTE {
                return true;
            }
        }
        let Some(secret) = self.link_secret_for(peer).filter(|s| mailbox(s) == m) else {
            // Not for this node's devices: ignore it, it may be from a newer version.
            return true;
        };
        // A home node passes asks and answers on to its other devices, unread.
        if self.peers.get(peer).is_some_and(|p| p.client) {
            let others: Vec<String> =
                self.peers.keys().filter(|o| o.as_str() != peer && self.link_secret_for(o).as_deref() == Some(secret.as_slice())).cloned().collect();
            for o in others {
                self.send_msg(&o, Msg::Link { mailbox: m, sealed: sealed.clone() });
            }
        }
        let Ok(plain) = hoi_crypto::sealed::open(BOX_CONTEXT, &secret, &sealed) else {
            return !self.peer_scores.penalize(peer, 1.0);
        };
        let Ok(Value::M(entries)) = cbor::decode(&plain) else { return true };
        let msg: std::collections::BTreeMap<String, Value> = entries.into_iter().collect();
        let Some(Value::T(t)) = msg.get("t") else { return true };
        let device = bytes::<32>(msg.get("device")).map(Id);
        match (t.as_str(), msg.get("code")) {
            ("req", Some(Value::T(code))) => {
                if let Some(commit) = bytes::<32>(msg.get("c")) {
                    self.on_link_ask(peer, code, commit, secret, now);
                }
            }
            ("chk", _) => {
                if let (Some(device), Some(ours)) = (device, bytes::<16>(msg.get("n"))) {
                    self.on_link_challenge(device, ours, &secret);
                }
            }
            ("rev", _) => {
                if let (Some(device), Some(theirs), Some(ours)) = (device, bytes::<32>(msg.get("n")), bytes::<16>(msg.get("to"))) {
                    self.on_link_reveal(device, theirs, ours);
                }
            }
            ("ok", Some(Value::T(code))) => self.on_link_approval(code),
            // remote signing (crate::remote)
            ("sign", _) => {
                if let (Some(device), Some(Value::B(rec))) = (device, msg.get("rec")) {
                    self.on_sign_request(peer, &secret, device, rec);
                }
            }
            ("signed", _) => {
                if let (Some(device), Some(id), Some(sig)) = (device, bytes::<32>(msg.get("id")), bytes::<64>(msg.get("sig"))) {
                    self.on_signed(device, Id(id), Some(sig), None);
                }
            }
            ("unsigned", _) => {
                if let (Some(device), Some(id)) = (device, bytes::<32>(msg.get("id"))) {
                    let reason = match msg.get("reason") {
                        Some(Value::T(r)) => Some(r.chars().take(256).collect()),
                        _ => None,
                    };
                    self.on_signed(device, Id(id), None, reason);
                }
            }
            // link previews (crate::preview): a device asks its home node, which answers
            ("pv", _) => {
                if let Some(Value::T(url)) = msg.get("u") {
                    let url = url.clone();
                    self.on_preview_ask(peer, &secret, &url);
                }
            }
            ("drop-device", _) => {
                if let Some(Value::B(wire)) = msg.get("rec") {
                    self.on_drop_device(peer, wire);
                }
            }
            ("drop-share", _) => self.on_drop_share(&msg),
            ("pvd", _) => self.on_preview_answer(peer, &msg),
            ("no", _) if device.is_some() && self.link_waiting.as_ref().map(|w| w.device) == device => {
                self.link_waiting = None;
                self.pending_kp = None;
                self.mark_keys();
                self.notice("Your other device said no. Nothing was linked.");
            }
            _ => {}
        }
        true
    }

    /// A device of yours asks (root devices only): answer with a nonce of our own and wait for
    /// the reveal before showing anything.
    fn on_link_ask(&mut self, peer: &str, code: &str, commit: [u8; 32], secret: Vec<u8>, now: u64) {
        if self.root_kp.is_none() || self.state.me.as_ref().is_none_or(|m| m.burned) {
            return; // another of your devices holds the key
        }
        if self.platform == "node" {
            return; // hoi-node has no screen to ask on; its key is its own
        }
        let Ok(m) = Self::decode_code(code, "hoi:link:") else { return };
        if m.get("t") != Some(&Value::T("link-req".into())) {
            return;
        }
        let Some(Value::B(d)) = m.get("device") else { return };
        let Ok(device) = Id::from_slice(d) else { return };
        if self.store.delegation(&device).is_some() {
            return;
        }
        let label = match m.get("label") {
            Some(Value::T(t)) => crate::api::clean_name(t).unwrap_or_else(|_| "Another device".into()),
            _ => "Another device".into(),
        };
        // The same request again (the new device reconnected): the same answer, so it shows the
        // same digits.
        if let Some(a) = self.link_asks.values().find(|a| a.device == device && a.commit == commit && now.saturating_sub(a.at) < ASK_TTL) {
            let chk = challenge(&device, &a.ours);
            let secret = a.secret.clone();
            self.send_link_all(&secret, &chk);
            return;
        }
        // Anyone holding the pairing token can ask, so one device holds one ask: a new ask
        // replaces the last one from the same device key, or from the same connection when it
        // came straight from one of our clients. Digits on the person's screen are never pushed
        // out: a device key on screen asked again from elsewhere may be someone replaying it,
        // and with digits in every slot a new ask waits until the person answers one.
        let from = self.peers.get(peer).filter(|p| p.client).map(|_| peer.to_string());
        let mine = |a: &PendingAsk| from.is_some() && a.from == from;
        if self.link_asks.values().any(|a| a.device == device && !mine(a) && self.ask_shown(a, now)) {
            return;
        }
        self.link_asks.retain(|_, a| now.saturating_sub(a.at) < ASK_TTL && a.device != device && !mine(a));
        if self.link_asks.len() >= MAX_ASKS {
            let Some(oldest) = self.link_asks.iter().find(|(_, a)| !self.ask_shown(a, now)).map(|(id, _)| *id) else { return };
            self.link_asks.remove(&oldest);
        }
        let Ok(ours) = hoi_crypto::random_bytes::<16>() else { return };
        let id = self.next_ask;
        self.next_ask += 1;
        self.send_link_all(&secret, &challenge(&device, &ours));
        self.link_asks.insert(id, PendingAsk { request: code.to_string(), device, label, secret, commit, ours, check: None, from, at: now });
    }

    /// On the new device: a device with your key answered. Reveal our nonce to it and show the
    /// digits this pair of nonces gives.
    fn on_link_challenge(&mut self, device: Id, ours: [u8; 16], secret: &[u8]) {
        let Some(w) = &mut self.link_waiting else { return };
        if w.device != device {
            return;
        }
        // A challenge seen before (asked again after a reconnect) gets the reveal again, in
        // case the first was lost, and nothing new to show.
        let known = w.checks.iter().any(|(n, _)| *n == ours);
        if !known {
            if w.checks.len() >= MAX_CHALLENGES {
                return;
            }
            let check = check_code(&device, &w.nonce, &ours);
            w.checks.push((ours, check));
        }
        let reveal = body(vec![
            ("t", Value::T("rev".into())),
            ("device", Value::B(device.0.to_vec())),
            ("n", Value::B(w.nonce.to_vec())),
            ("to", Value::B(ours.to_vec())),
        ]);
        let checks = (!known).then(|| w.checks.iter().map(|(_, c)| c.clone()).collect());
        self.send_link_all(secret, &reveal);
        if let Some(checks) = checks {
            self.events.push(Event::LinkCheck { checks });
        }
    }

    /// On a root device: the new device revealed the nonce it committed to; now the ask shows.
    fn on_link_reveal(&mut self, device: Id, theirs: [u8; 32], ours: [u8; 16]) {
        let Some((id, ask)) = self.link_asks.iter_mut().find(|(_, a)| a.device == device && a.ours == ours && a.check.is_none()) else { return };
        if commitment(&theirs) != ask.commit {
            return;
        }
        let check = check_code(&device, &theirs, &ours);
        ask.check = Some(check.clone());
        let shown = LinkAsk { id: *id, label: ask.label.clone(), check, device: device.hex(), at: ask.at };
        self.events.push(Event::LinkAsk { ask: shown });
    }

    fn on_link_approval(&mut self, code: &str) {
        if self.link_waiting.is_none() {
            return;
        }
        // Approvals for other devices fail the device check and are ignored.
        if let Ok(me) = self.link_complete(code) {
            self.link_waiting = None;
            let name = me.name.clone();
            self.events.push(Event::Linked { me: Box::new(me) });
            self.notice(format!("Linked. This device signs as {name} now."));
        }
    }
}
