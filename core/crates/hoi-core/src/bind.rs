//! Channel binding on WebRTC links. A browser dials a node by a route that pins the
//! node's DTLS certificate (`rtc:<fingerprint>@addr`), and the node pins the browser's from the
//! ICE username fragment, so the channel is encrypted between those two certificates. What was
//! missing is the link between the channel and the node's identity: a hello names the node's
//! endpoint key, and nothing showed that the key's holder is the one at the other end of the
//! channel, rather than someone who forwards another node's hello.
//!
//! So right after its hello on a WebRTC link, a native node sends `bind`: its endpoint key's
//! signature over `hoi/rtc-binding/1`, both certificate fingerprints, the username fragment and
//! the key itself. The browser takes nothing but the hello, the `bind` and a goodbye from that
//! link until the signature checks out against the key the hello named and the channel it
//! actually opened; anything else, or a bad signature, ends the link. Browsers name no lasting
//! key to strangers (a fresh random id per connection), so nothing binds in the other
//! direction: paired devices prove themselves with their credential, as before.

use hoi_sync::Msg;
use hoi_types::Id;
use serde::{Deserialize, Serialize};

use crate::node::Node;

/// What the host knows about a WebRTC channel: the node's and the browser's certificate
/// fingerprints (SHA-256) and the ICE username fragment the browser chose.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtcChannel {
    pub node: Vec<u8>,
    pub browser: Vec<u8>,
    pub ufrag: String,
}

/// The statement a node's endpoint key signs for one channel.
pub fn statement(c: &RtcChannel, endpoint: &Id) -> String {
    format!("hoi/rtc-binding/1\n{}\n{}\n{}\n{}", hoi_types::hex_encode(&c.node), hoi_types::hex_encode(&c.browser), c.ufrag, endpoint.hex())
}

impl Node {
    /// This side answers a WebRTC link it accepted (a native node) and must bind it.
    fn binds(&self, peer: &str) -> bool {
        self.peers.get(peer).is_some_and(|p| p.meta.transport == "webrtc" && p.meta.incoming && p.meta.rtc.is_some()) && self.transport.is_some()
    }

    /// This side dialed a WebRTC link (a browser) and waits for the node to bind it.
    pub(crate) fn awaits_bind(&self, peer: &str) -> bool {
        self.peers.get(peer).is_some_and(|p| p.meta.transport == "webrtc" && !p.meta.incoming && p.meta.rtc.is_some() && !p.bound)
    }

    /// After our hello to `peer`: the binding, once, on a WebRTC link we accepted.
    pub(crate) fn send_bind(&mut self, peer: &str) {
        if !self.binds(peer) || self.peers.get(peer).is_some_and(|p| p.bind_sent) {
            return;
        }
        let Some(kp) = self.transport_kp.as_ref() else { return };
        let Some(channel) = self.peers.get(peer).and_then(|p| p.meta.rtc.clone()) else { return };
        let sig = kp.sign(statement(&channel, &kp.public()).as_bytes());
        if let Some(p) = self.peers.get_mut(peer) {
            p.bind_sent = true;
        }
        self.send_msg(peer, Msg::Bind { sig: sig.0 });
    }

    /// The node's binding: whether the link may go on.
    pub(crate) fn on_bind(&mut self, peer: &str, sig: [u8; 64]) -> bool {
        let Some(p) = self.peers.get(peer) else { return false };
        if p.meta.transport != "webrtc" || p.meta.incoming {
            // only a node we dialed over WebRTC binds; anywhere else it means nothing
            return true;
        }
        // a host that told us nothing about the channel checks nothing (the native test dialer)
        let Some(channel) = p.meta.rtc.clone() else { return true };
        let Some(key) = p.node_key else { return false };
        if !hoi_crypto::verify(&key, statement(&channel, &key).as_bytes(), &hoi_types::Sig(sig)) {
            return false;
        }
        if let Some(p) = self.peers.get_mut(peer) {
            p.bound = true;
        }
        true
    }
}
