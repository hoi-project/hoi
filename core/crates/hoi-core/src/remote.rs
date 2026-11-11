//! Remote signing: a browser that holds no signing key at all. When it links in
//! band with the `remote` flag, the device with the person's root makes the browser's device
//! key itself, blesses it, signs its consent and keeps it. The browser learns only the public
//! key and the delegation. From then on it builds and stamps its records as before and asks over
//! the home link (`link` messages, sealed with the home node's link key, passed on by the home
//! node to its other devices) for the signature: `sign { device, rec }` → `signed { device, id,
//! sig }` or `unsigned { device, id, reason }`. The computer signs posts, comments, votes and the
//! like without asking; ledger entries (moderation, rules, owners) wait until the person says yes
//! on the computer. Signing covers the record's id, which already holds its stamp.
//!
//! The browser's vault then holds no key a thief could use; turning the computer off turns its
//! browsers' signing off, and revoking the device there ends it for good.

use std::collections::BTreeMap;

use hoi_crypto::Keypair;
use hoi_types::cbor::Value;
use hoi_types::{Id, Kind, Record};
use serde::Serialize;

use crate::api::R;
use crate::dto::Event;
use crate::error::ApiError;
use crate::node::{Node, Source};

/// Records a browser waits for a signature on at once.
const MAX_PENDING: usize = 64;
/// A request nobody answered within this is dropped.
const PENDING_TTL: u64 = 600;
/// Asks waiting on the computer for a yes at once.
const MAX_ASKS: usize = 16;

/// What the computer keeps for one browser it signs for.
pub(crate) struct RemoteKey {
    pub kp: Keypair,
    pub label: String,
}

/// A record a browser asked the computer to sign that waits for the person's yes.
pub(crate) struct SignWaiting {
    record: Record,
    peer: String,
    secret: Vec<u8>,
    at: u64,
}

#[derive(Default)]
pub(crate) struct Remote {
    /// On the computer: device key → the key it signs with for a browser.
    pub keys: BTreeMap<Id, RemoteKey>,
    /// On the computer: id → a record waiting for a yes.
    pub asks: BTreeMap<Id, SignWaiting>,
    /// On the browser: id → the record waiting for its signature, and when it was sent.
    pub pending: BTreeMap<Id, (Record, u64)>,
}

/// A record a browser asked this computer to sign, waiting for a yes (`signAsks`).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignAsk {
    pub id: String,
    /// The browser it came from, as this computer calls it.
    pub device: String,
    pub kind: String,
    pub community: Option<String>,
    pub at: u64,
}

/// Kinds a computer signs for its browsers without asking. Ledger entries (moderation, rules,
/// limits, owners) wait for the person.
fn signs_unasked(kind: Kind) -> bool {
    !matches!(kind, Kind::Ledger | Kind::Community | Kind::Identity | Kind::Delegation | Kind::Revoke | Kind::Rotate)
}

impl Node {
    /// This device signs nothing itself: its device key lives on the person's computer.
    pub(crate) fn signs_remotely(&self) -> bool {
        self.device_kp.is_none() && self.state.me.as_ref().is_some_and(|m| m.remote)
    }

    /// On a browser in remote mode: send `r` (stamped) to be signed, keep it until the answer.
    pub(crate) fn publish_remote(&mut self, r: Record) -> R<Id> {
        let me = self.need_me()?;
        if r.author != me.device {
            return Err(ApiError::new("internal", "author does not match this device"));
        }
        let now = self.now();
        self.remote.pending.retain(|_, (_, at)| *at + PENDING_TTL > now);
        if self.remote.pending.len() >= MAX_PENDING {
            return Err(ApiError::new("rate-limited", "Your computer hasn't signed what this browser already sent. Check that it's on."));
        }
        let id = r.id();
        let plain = crate::link::body_of(vec![("t", Value::T("sign".into())), ("device", Value::B(me.device.0.to_vec())), ("rec", Value::B(r.encode()))]);
        if self.send_link_home(&plain) == 0 {
            return Err(ApiError::not_allowed("This browser signs through your computer, and isn't connected to it now."));
        }
        self.remote.pending.insert(id, (r, now));
        Ok(id)
    }

    /// Whether `id` waits for its signature on this browser.
    pub(crate) fn signing(&self, id: &Id) -> bool {
        self.remote.pending.contains_key(id)
    }

    /// Whether `id` still waits for its signature: asked within the last [`PENDING_TTL`].
    pub(crate) fn signing_pending(&self, id: &Id, now: u64) -> bool {
        self.remote.pending.get(id).is_some_and(|(_, at)| *at + PENDING_TTL > now)
    }

    /// Records this browser asked to have signed within the last [`PENDING_TTL`].
    pub(crate) fn signing_outstanding(&self, now: u64) -> usize {
        self.remote.pending.values().filter(|(_, at)| *at + PENDING_TTL > now).count()
    }

    /// On the computer: a browser asks for a signature.
    pub(crate) fn on_sign_request(&mut self, peer: &str, secret: &[u8], device: Id, rec: &[u8]) {
        if !self.remote.keys.contains_key(&device) {
            return; // another of the person's devices keeps that key, or nobody does
        }
        let refuse = |n: &mut Node, id: Id, reason: &str| {
            let plain = crate::link::body_of(vec![
                ("t", Value::T("unsigned".into())),
                ("device", Value::B(device.0.to_vec())),
                ("id", Value::B(id.0.to_vec())),
                ("reason", Value::T(reason.into())),
            ]);
            n.send_link_to(peer, secret, &plain);
        };
        let Ok(record) = hoi_types::cbor::decode(rec).map_err(|_| ()).and_then(|v| Record::from_value(v).map_err(|_| ())) else { return };
        let id = record.id();
        if record.encode() != rec || record.author != device {
            return refuse(self, id, "That record isn't this browser's.");
        }
        let revoked = self.store.delegation(&device).is_none_or(|d| d.info.revoked_at.is_some_and(|t| t <= self.now()));
        if revoked {
            return refuse(self, id, "This computer no longer signs for that browser.");
        }
        if !signs_unasked(record.kind()) {
            let now = self.now();
            self.remote.asks.retain(|_, a| a.at + PENDING_TTL > now);
            if self.remote.asks.len() >= MAX_ASKS {
                return refuse(self, id, "Too many things wait for a yes on your computer.");
            }
            let ask = SignAsk {
                id: id.hex(),
                device: self.remote.keys[&device].label.clone(),
                kind: record.kind().as_str().into(),
                community: record.community.map(|c| c.hex()),
                at: now,
            };
            self.remote.asks.insert(id, SignWaiting { record, peer: peer.into(), secret: secret.to_vec(), at: now });
            self.events.push(Event::SignAsk { ask });
            return;
        }
        self.sign_for(peer, secret, &record);
    }

    fn sign_for(&mut self, peer: &str, secret: &[u8], record: &Record) {
        let Some(key) = self.remote.keys.get(&record.author) else { return };
        let id = record.id();
        let sig = key.kp.sign(id.as_bytes());
        let plain = crate::link::body_of(vec![
            ("t", Value::T("signed".into())),
            ("device", Value::B(record.author.0.to_vec())),
            ("id", Value::B(id.0.to_vec())),
            ("sig", Value::B(sig.0.to_vec())),
        ]);
        self.send_link_to(peer, secret, &plain);
    }

    /// On the computer: what its browsers asked to sign and wait for a yes.
    pub fn sign_asks(&self) -> Vec<SignAsk> {
        self.remote
            .asks
            .iter()
            .map(|(id, a)| SignAsk {
                id: id.hex(),
                device: self.remote.keys.get(&a.record.author).map(|k| k.label.clone()).unwrap_or_default(),
                kind: a.record.kind().as_str().into(),
                community: a.record.community.map(|c| c.hex()),
                at: a.at,
            })
            .collect()
    }

    /// On the computer: yes signs what the browser asked, no refuses it.
    pub fn sign_answer(&mut self, id: &str, yes: bool) -> R<()> {
        let id = crate::api::parse_id(id)?;
        let a = self.remote.asks.remove(&id).ok_or_else(|| ApiError::not_found("signature ask"))?;
        if yes {
            self.sign_for(&a.peer, &a.secret, &a.record);
        } else {
            let plain = crate::link::body_of(vec![
                ("t", Value::T("unsigned".into())),
                ("device", Value::B(a.record.author.0.to_vec())),
                ("id", Value::B(id.0.to_vec())),
                ("reason", Value::T("You said no on your computer.".into())),
            ]);
            self.send_link_to(&a.peer, &a.secret, &plain);
        }
        Ok(())
    }

    /// On the browser: the computer's answer.
    pub(crate) fn on_signed(&mut self, device: Id, id: Id, sig: Option<[u8; 64]>, reason: Option<String>) {
        if self.state.me.as_ref().is_none_or(|m| m.device != device) {
            return;
        }
        let Some((record, _)) = self.remote.pending.remove(&id) else { return };
        let Some(sig) = sig else {
            let message = reason.unwrap_or_else(|| "Your computer didn't sign it.".into());
            self.events.push(Event::Signed { id: id.hex(), ok: false, message: Some(message) });
            return;
        };
        let wire = hoi_types::Wire { record, sig: hoi_types::Sig(sig) };
        match self.ingest(&wire.encode(), &Source::Local) {
            Ok(_) => {
                self.settle_dirty();
                self.events.push(Event::Signed { id: id.hex(), ok: true, message: None });
            }
            Err(e) => self.events.push(Event::Signed { id: id.hex(), ok: false, message: Some(ApiError::from(e).message) }),
        }
    }

    /// On the computer: the device keys it signs with for browsers, by device.
    pub(crate) fn remote_label(&self, device: &Id) -> Option<&str> {
        self.remote.keys.get(device).map(|k| k.label.as_str())
    }
}
