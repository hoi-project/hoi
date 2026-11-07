//! Media between native nodes over iroh-blobs.
//!
//! A desktop app or hoi-node fetches a picture or a video from a peer that says `blobs`, over
//! the iroh-blobs protocol on the same endpoint, in place of a `bwant` on the sync link. The
//! bytes are checked against their hash as they stream (BLAKE3, the hash media already has), so
//! a bad peer is caught at its first wrong chunk, and a large file never waits in front of
//! records on the sync link. What arrives goes through the same checks as a `blob` message.
//!
//! A node serves this way only the media of the communities it pins, and only to a peer it has
//! a sync link with that shares the community: the rule `bwant` already has, narrowed to
//! pinners. A peer that doesn't serve a community this way is asked with `bwant` for the rest of
//! the link. Scattered and private media never go this way.

use std::collections::{BTreeMap, BTreeSet};

use hoi_types::{Body, Id};
use serde::{Deserialize, Serialize};

use crate::node::Node;

/// One fetch for the host: `hash` from the peer at `endpoint` (an iroh endpoint id), at most
/// `size` bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct BlobFetch {
    pub peer: String,
    pub endpoint: String,
    pub hash: Vec<u8>,
    pub size: u64,
}

#[derive(Debug, Default)]
pub(crate) struct Blobs {
    /// The host fetches and serves over iroh-blobs.
    pub on: bool,
    /// Fetches for the host to start.
    pub due: Vec<BlobFetch>,
    /// Fetches under way: hash → the peer and the community it was asked for.
    pub trying: BTreeMap<Id, (String, Id)>,
    /// Blobs that came in this way, and requests served, since the node started.
    pub fetched: u64,
    pub served: u64,
}

impl Node {
    /// The host fetches and serves media over iroh-blobs (hoi-net), or stopped. Only a desktop
    /// app or hoi-node does.
    pub fn set_blobs(&mut self, on: bool) {
        let on = on && self.can_relay();
        if self.blobs.on != on {
            self.blobs.on = on;
            if !on {
                self.blobs.due.clear();
                self.blobs.trying.clear();
            }
            // the hello's feature list changed
            self.announce();
        }
    }

    /// Blobs fetched over iroh-blobs, and requests served that way, since the node started.
    pub fn blobs_counts(&self) -> (u64, u64) {
        (self.blobs.fetched, self.blobs.served)
    }

    pub(crate) fn serves_blobs(&self) -> bool {
        self.blobs.on
    }

    /// For the host: the fetches to start now.
    pub fn take_blob_fetches(&mut self) -> Vec<BlobFetch> {
        std::mem::take(&mut self.blobs.due)
    }

    /// A peer to fetch `hash` from over iroh-blobs, among those sharing `c`: one that said
    /// `blobs`, reached over iroh, and hasn't turned this community down on this link.
    pub(crate) fn blobs_peer(&self, c: &Id, peers: &[String]) -> Option<(String, String)> {
        if !self.blobs.on {
            return None;
        }
        peers.iter().find_map(|k| {
            let p = self.peers.get(k)?;
            let endpoint = p.meta.endpoint.clone().filter(|_| p.meta.transport == "quic")?;
            (p.features.contains(hoi_sync::FEATURE_BLOBS) && !p.blobs_refused.contains(c)).then(|| (k.clone(), endpoint))
        })
    }

    /// Queues a fetch over iroh-blobs. The size is the one the post states, which the bytes
    /// must match anyway.
    pub(crate) fn queue_blob_fetch(&mut self, peer: String, endpoint: String, hash: Id, c: Id) {
        let size = self
            .store
            .media_users(&hash)
            .find_map(|s| match &s.record.body {
                Body::Post { media, .. } => media.iter().find(|m| m.hash == hash).map(|m| m.size),
                _ => None,
            })
            .unwrap_or(hoi_sync::MAX_BLOB as u64)
            .min(hoi_sync::MAX_BLOB as u64);
        self.blobs.trying.insert(hash, (peer.clone(), c));
        self.blobs.due.push(BlobFetch { peer, endpoint, hash: hash.as_bytes().to_vec(), size });
    }

    /// For the host: what a fetch brought. The bytes go through every check a `blob` message
    /// does. On an error the peer is asked with `bwant` instead, and for the rest of the link
    /// this community's media comes from it that way.
    pub fn blob_fetched(&mut self, peer: &str, hash: &[u8], got: Result<Vec<u8>, String>) {
        let Ok(hash) = <[u8; 32]>::try_from(hash).map(Id) else { return };
        // only the peer it was asked of answers for it
        if self.blobs.trying.get(&hash).is_none_or(|(asked, _)| asked != peer) {
            return;
        }
        let Some((_, c)) = self.blobs.trying.remove(&hash) else { return };
        match got {
            // iroh-blobs checked the bytes against the hash as they came; every other check a
            // `blob` message gets still applies
            Ok(data) if self.peers.contains_key(peer) => {
                let had = self.store.has_blob(&hash);
                let _ = self.take_media(peer, hash, data);
                if !had && self.store.has_blob(&hash) {
                    self.blobs.fetched += 1;
                }
            }
            Ok(_) => self.request_missing_media(),
            Err(_) => {
                if let Some(p) = self.peers.get_mut(peer) {
                    p.blobs_refused.insert(c);
                    let msg = hoi_sync::Msg::BWant { ids: vec![hash] }.encode();
                    self.queue_out(peer.to_string(), msg, 0, 0);
                }
            }
        }
    }

    /// For the host, when a peer asks over iroh-blobs: the bytes, if this node serves them to
    /// that endpoint. It must have a sync link with us, share a community whose post uses the
    /// media, and this node must pin that community. What goes out counts like a `blob` message,
    /// against the relay's daily cap too.
    pub fn blob_to_serve(&mut self, endpoint: &str, hash: &[u8]) -> Option<Vec<u8>> {
        if !self.blobs.on {
            return None;
        }
        let hash = Id(<[u8; 32]>::try_from(hash).ok()?);
        if self.state.private_blobs.contains(&hash) {
            return None;
        }
        let links: Vec<(&String, &crate::node::PeerState)> =
            self.peers.iter().filter(|(_, p)| p.said_hello && p.meta.endpoint.as_deref() == Some(endpoint)).collect();
        let peer = links.first().map(|(k, _)| (*k).clone())?;
        let shared: BTreeSet<Id> = links.iter().flat_map(|(_, p)| p.communities.iter().copied()).collect();
        let allowed = self.store.media_users(&hash).any(|s| {
            self.plaintext_forwardable(s) && s.record.community.is_some_and(|c| shared.contains(&c) && self.state.joined.get(&c).is_some_and(|j| j.pinning))
        });
        if !allowed || (self.is_member_in(&peer) && !self.accepts_members()) {
            return None;
        }
        let data = self.store.blob(&hash).cloned()?;
        let now = self.now();
        self.count_out(&peer, data.len(), 0, data.len() as u64, now);
        self.blobs.served += 1;
        Some(data)
    }

    /// For the host: whether `endpoint` has a sync link here, so its iroh-blobs connection is
    /// taken at all.
    pub fn blobs_link(&self, endpoint: &str) -> bool {
        self.blobs.on && self.peers.values().any(|p| p.said_hello && p.meta.endpoint.as_deref() == Some(endpoint))
    }
}
