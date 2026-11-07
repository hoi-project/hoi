//! Media between native nodes over iroh-blobs, on the endpoint [`crate::start_p2p`]
//! runs. The core decides; this module moves bytes.
//!
//! - Serving: an iroh-blobs connection is taken only from an endpoint with a sync link here, and
//!   each request only for media the core says this node serves that endpoint
//!   ([`hoi_core::Node::blob_to_serve`]). Those bytes are copied into a small in-memory store
//!   just before they are sent, and the oldest leave it past [`SERVE_CACHE`]. Requests for many
//!   blobs, pushes and observing are refused.
//! - Fetching: what the core asks for ([`hoi_core::Node::take_blob_fetches`]) is fetched over a
//!   connection of its own to the peer, checked as it streams, never longer than the post says,
//!   and handed back ([`hoi_core::Node::blob_fetched`]).

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::protocol::ProtocolHandler;
use iroh::{Endpoint, EndpointAddr};
use iroh_blobs::get::fsm;
use iroh_blobs::protocol::GetRequest;
use iroh_blobs::provider::events::{AbortReason, ConnectMode, EventMask, EventSender, ObserveMode, ProviderMessage, RequestMode, ThrottleMode};
use iroh_blobs::store::GcConfig;
use iroh_blobs::store::mem::{MemStore, Options};
use iroh_blobs::{BlobsProtocol, Hash};
use tokio::task::JoinHandle;

use crate::Shared;

/// Bytes kept ready to serve at most; the blobs asked for longest ago leave first.
pub const SERVE_CACHE: usize = 64 * 1024 * 1024;
/// A fetch, from dialing to the last byte, at most.
pub const FETCH_SECS: u64 = 60;
const GC_EVERY: Duration = Duration::from_secs(30);
const TAG: &str = "hoi-serve-";

/// The serving half: the protocol handler and the task answering its questions.
pub(crate) struct Serving {
    protocol: BlobsProtocol,
    events: JoinHandle<()>,
}

impl Drop for Serving {
    fn drop(&mut self) {
        self.events.abort();
    }
}

impl Serving {
    pub(crate) fn new(shared: Arc<Shared>) -> Serving {
        let store = MemStore::new_with_opts(Options { gc_config: Some(GcConfig { interval: GC_EVERY, add_protected: None }) });
        let mask = EventMask {
            connected: ConnectMode::Intercept,
            get: RequestMode::Intercept,
            get_many: RequestMode::Disabled,
            push: RequestMode::Disabled,
            observe: ObserveMode::Intercept,
            throttle: ThrottleMode::None,
        };
        let (events, rx) = EventSender::channel(32, mask);
        let protocol = BlobsProtocol::new(&store, Some(events));
        let events = tokio::spawn(answer(shared, store.clone(), rx));
        Serving { protocol, events }
    }

    /// Serves one iroh-blobs connection until the other side is done with it.
    pub(crate) async fn serve(&self, conn: Connection) {
        let _ = self.protocol.accept(conn).await;
    }
}

/// Answers the provider's questions: who may connect, and which blobs they may have.
async fn answer(shared: Arc<Shared>, store: MemStore, mut rx: tokio::sync::mpsc::Receiver<ProviderMessage>) {
    let mut who: BTreeMap<u64, String> = BTreeMap::new();
    let mut kept: VecDeque<(Hash, usize)> = VecDeque::new();
    let mut held = 0usize;
    while let Some(msg) = rx.recv().await {
        match msg {
            ProviderMessage::ClientConnected(m) => {
                let endpoint = m.inner.endpoint_id.map(|e| e.to_string());
                let ok = endpoint.as_deref().is_some_and(|e| shared.with(|n| n.blobs_link(e)));
                if ok && who.len() < 1024 {
                    who.insert(m.inner.connection_id, endpoint.unwrap_or_default());
                    m.tx.send(Ok(())).await.ok();
                } else {
                    m.tx.send(Err(AbortReason::Permission)).await.ok();
                }
            }
            ProviderMessage::ConnectionClosed(m) => {
                who.remove(&m.inner.connection_id);
            }
            ProviderMessage::GetRequestReceived(m) => {
                let hash = m.inner.request.hash;
                let bytes = who.get(&m.inner.connection_id).and_then(|e| shared.with(|n| n.blob_to_serve(e, hash.as_bytes())));
                let Some(bytes) = bytes else {
                    m.tx.send(Err(AbortReason::Permission)).await.ok();
                    continue;
                };
                if !kept.iter().any(|(h, _)| *h == hash) {
                    let len = bytes.len();
                    if store.add_bytes(bytes).with_named_tag(tag(&hash)).await.is_err() {
                        m.tx.send(Err(AbortReason::RateLimited)).await.ok();
                        continue;
                    }
                    kept.push_back((hash, len));
                    held += len;
                    while held > SERVE_CACHE && kept.len() > 1 {
                        let Some((old, n)) = kept.pop_front() else { break };
                        held -= n;
                        let _ = store.tags().delete(tag(&old)).await;
                    }
                }
                m.tx.send(Ok(())).await.ok();
            }
            ProviderMessage::ObserveRequestReceived(m) => {
                m.tx.send(Err(AbortReason::Permission)).await.ok();
            }
            ProviderMessage::GetManyRequestReceived(m) => {
                m.tx.send(Err(AbortReason::Permission)).await.ok();
            }
            ProviderMessage::PushRequestReceived(m) => {
                m.tx.send(Err(AbortReason::Permission)).await.ok();
            }
            ProviderMessage::Throttle(m) => {
                m.tx.send(Ok(())).await.ok();
            }
            _ => {}
        }
    }
}

fn tag(hash: &Hash) -> String {
    format!("{TAG}{hash}")
}

/// One blob from `addr`: at most `size` bytes, checked against `hash` as they stream.
pub(crate) async fn fetch(endpoint: &Endpoint, addr: EndpointAddr, hash: [u8; 32], size: u64) -> Result<Vec<u8>, String> {
    let work = async {
        let conn = endpoint.connect(addr, iroh_blobs::ALPN).await.map_err(|_| "the peer didn't answer".to_string())?;
        let got = get(&conn, Hash::from_bytes(hash), size).await;
        conn.close(0u32.into(), b"done");
        got
    };
    tokio::time::timeout(Duration::from_secs(FETCH_SECS), work).await.map_err(|_| "the peer took too long".to_string())?
}

async fn get(conn: &Connection, hash: Hash, size: u64) -> Result<Vec<u8>, String> {
    fn refused<E>(_: E) -> String {
        "the peer doesn't serve it".to_string()
    }
    let connected = fsm::start(conn.clone(), GetRequest::blob(hash), Default::default()).next().await.map_err(refused)?;
    let fsm::ConnectedNext::StartRoot(start) = connected.next().await.map_err(refused)? else {
        return Err("the peer doesn't serve it".into());
    };
    let (content, claimed) = start.next().next().await.map_err(refused)?;
    // the stream holds exactly the size it starts with, which the hash proves at the end
    if claimed != size {
        return Err("not the size its post says".into());
    }
    let (end, bytes) = content.concatenate_into_vec().await.map_err(|_| "bytes that don't match their hash".to_string())?;
    if let fsm::EndBlobNext::Closing(closing) = end.next() {
        let _ = closing.next().await;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_store_hashes_media_the_way_posts_do() {
        let store = MemStore::new();
        let data = vec![7u8; 200_000];
        let tag = store.add_bytes(data.clone()).with_named_tag("t").await.unwrap();
        assert_eq!(tag.hash.as_bytes(), hoi_types::Id::hash(&data).as_bytes());
    }
}
