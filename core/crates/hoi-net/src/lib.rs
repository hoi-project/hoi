//! hoi-net: moving sync messages between nodes. The node decides what to say ([`hoi_core::Node`]
//! `net_*` calls); this crate owns sockets.
//!
//! - [`Shared`]: one node behind a lock, with an outbox router and an event callback. Every host
//!   (Tauri commands, WebSocket handlers, the sweeper) goes through [`Shared::with`]; messages
//!   from peers go through [`Shared::receive`], which checks identity work before taking the lock.
//! - [`serve`]: the home-node WebSocket. Browsers and phones paired with a `hoi://pair` link
//!   connect here; the node checks their credential in the first `hello`. Browsers may open it
//!   only from the node's own web app and the origins the host allows ([`origins_from_env`]).
//! - [`dial`]: connect to a home node (desktop paired with another node, phones, tests).
//!
//! [`start_p2p`] runs authenticated iroh connections over `hoi/sync/1`, with explicit
//! peer hints and user-selected relays. [`start_rtc`] lets browsers and phones reach this node
//! over WebRTC data channels with no certificate authority and no signaling server. The same
//! validation and sync engine serves all three; see PROTOCOL.md §14.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hoi_core::dto::Event;
use hoi_core::{IdentityWork, Node, PeerMeta, Role};
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch};

mod blobs;
pub mod dht;
pub mod handle;
pub mod lan;
mod limit;
mod p2p;
pub mod portmap;
pub mod preview;
mod rtc;
mod rtc_crypto;
#[cfg(test)]
mod storage_tests;
pub mod vpn;
pub use p2p::{P2pConfig, P2pHandle, start_p2p};
pub use rtc::{DEFAULT_PORT as RTC_PORT, RtcConfig, RtcHandle, route as rtc_route, start_rtc};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Callback, ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// Paired devices served at once. A home node serves one person's browsers and phones.
pub const MAX_CLIENTS: usize = 32;
/// A WebSocket upgrade must finish within this.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// The other side's `hello` must arrive within this, or the link is dropped.
const HELLO: Duration = Duration::from_secs(15);

/// Message size cap: one media item plus framing (hoi-sync's own cap); nothing bigger is read.
fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(hoi_sync::MAX_MESSAGE)).max_frame_size(Some(hoi_sync::MAX_MESSAGE))
}

/// What a browser sends as `Origin` for a page at `url`: `https://Hoi.sh:443/app/` is
/// `https://hoi.sh`. Only http(s) addresses have one worth allowing; `null` and the rest don't.
pub fn web_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "https" => ":443",
        "http" => ":80",
        _ => return None,
    };
    let host = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    if host.is_empty() || host.contains('@') || host.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()) {
        return None;
    }
    Some(format!("{scheme}://{}", host.strip_suffix(default_port).unwrap_or(&host)))
}

/// Refuses a WebSocket upgrade from a web page outside the allowed origins. A request
/// without `Origin` comes from a native app, not a browser, and passes.
struct OriginCheck(Arc<[String]>);

impl Callback for OriginCheck {
    fn on_request(self, req: &Request, res: Response) -> Result<Response, ErrorResponse> {
        match req.headers().get("origin") {
            None => Ok(res),
            Some(o) if o.to_str().ok().and_then(web_origin).is_some_and(|o| self.0.contains(&o)) => Ok(res),
            Some(_) => {
                let mut refused = ErrorResponse::new(None);
                *refused.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
                Err(refused)
            }
        }
    }
}

/// Origins a host allows besides its node's own web app, from `HOI_ALLOWED_ORIGINS`
/// (comma separated): a self-hosted copy of the app, or a development server.
pub fn origins_from_env() -> Vec<String> {
    std::env::var("HOI_ALLOWED_ORIGINS").map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()).unwrap_or_default()
}

/// An address other networks can reach: no loopback, private, link-local, carrier-grade NAT,
/// documentation or multicast ranges. Reachability itself is only proven by a visitor.
pub fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            let shared = a == 100 && (64..=127).contains(&b);
            let benchmarking = a == 198 && (b == 18 || b == 19);
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || shared
                || benchmarking
                || a == 0
                || a >= 240)
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_ip(std::net::IpAddr::V4(v4));
            }
            let documentation = v6.segments()[0] == 0x2001 && v6.segments()[1] == 0xdb8;
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || v6.is_unique_local() || v6.is_unicast_link_local() || documentation)
        }
    }
}

pub type EventSink = Box<dyn Fn(Vec<Event>) + Send + Sync>;

/// Argon2id runs for incoming identity work at once, 8 MiB each (hoi_core::IdentityWork).
const WORK_AT_ONCE: usize = 2;

/// A node shared between UI calls and connections.
pub struct Shared {
    node: Mutex<Node>,
    links: Mutex<HashMap<String, Link>>,
    on_events: EventSink,
    next: AtomicU64,
    storage_failed: AtomicBool,
    work: Arc<Semaphore>,
}

const QUEUED_MESSAGES: usize = 128;
const QUEUED_BYTES: usize = 32 * 1024 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

struct Queued {
    bytes: Vec<u8>,
    _permit: OwnedSemaphorePermit,
}
struct Link {
    tx: mpsc::Sender<Queued>,
    cancel: watch::Sender<bool>,
    budget: Arc<Semaphore>,
}

impl Shared {
    pub fn new(node: Node, on_events: EventSink) -> Arc<Shared> {
        Arc::new(Shared {
            node: Mutex::new(node),
            links: Mutex::new(HashMap::new()),
            on_events,
            next: AtomicU64::new(1),
            storage_failed: AtomicBool::new(false),
            work: Arc::new(Semaphore::new(WORK_AT_ONCE)),
        })
    }

    /// Runs `f` on the node, then persists, routes queued messages and reports events.
    pub fn with<T>(&self, f: impl FnOnce(&mut Node) -> T) -> T {
        let (out, msgs, events) = {
            let mut n = self.node.lock().unwrap_or_else(|e| e.into_inner());
            let out = f(&mut n);
            match n.commit() {
                Ok(()) => {
                    let mut events = n.take_events();
                    if self.storage_failed.swap(false, Ordering::Relaxed) {
                        events.push(Event::Notice { text: "Storage is working again. Pending changes are saved and syncing can resume.".into() });
                    }
                    (out, n.take_outbox(), events)
                }
                Err(e) => {
                    // Neither advertise new records nor report successful changes before they
                    // are durable. Stop network mutations and discard connection-specific
                    // messages: after recovery, fresh links catch up from the saved records.
                    // Keeping an outbox for dead links would grow throughout a storage outage.
                    n.take_outbox();
                    self.close_all("");
                    let events = if self.storage_failed.swap(true, Ordering::Relaxed) {
                        Vec::new()
                    } else {
                        eprintln!("hoi: storage error: {e}");
                        vec![Event::Notice {
                            text: "This node could not save changes. Sync is paused. Keep it running, check free disk space and storage access; saving will retry.".into(),
                        }]
                    };
                    (out, Vec::new(), events)
                }
            }
        };
        if !msgs.is_empty() {
            let links = self.links.lock().unwrap_or_else(|e| e.into_inner());
            for (peer, bytes) in msgs {
                if let Some(link) = links.get(&peer) {
                    let permit = link.budget.clone().try_acquire_many_owned(bytes.len() as u32);
                    if let Ok(permit) = permit {
                        if link.tx.try_send(Queued { bytes, _permit: permit }).is_err() {
                            let _ = link.cancel.send(true);
                        }
                    } else {
                        let _ = link.cancel.send(true);
                    }
                }
            }
        }
        if !events.is_empty() {
            (self.on_events)(events);
        }
        out
    }

    /// A storage outage rejects incoming records and new sync sessions under the node lock.
    /// The unchanged journal still gets a retry, but remote traffic cannot keep extending it.
    fn with_network<T>(&self, f: impl FnOnce(&mut Node) -> T) -> Option<T> {
        let out = self.with(|n| (!self.storage_failed.load(Ordering::Relaxed)).then(|| f(n)));
        if self.storage_failed.load(Ordering::Relaxed) { None } else { out }
    }

    /// Takes in one message from `peer`. Returns false when the connection should be dropped,
    /// `None` while storage is failing. Identity work the message carries costs an Argon2id run
    /// each; it is verified first, on a blocking thread and without the node lock, so the UI and
    /// other connections don't wait on it. Under the lock the pipeline only
    /// looks the answers up.
    pub async fn receive(&self, peer: &str, bytes: &[u8]) -> Option<bool> {
        self.receive_with(peer, bytes, |work| async move { tokio::task::spawn_blocking(move || work.verify()).await.unwrap_or_default() }).await
    }

    async fn receive_with<F, Fut>(&self, peer: &str, bytes: &[u8], verify: F) -> Option<bool>
    where
        F: FnOnce(IdentityWork) -> Fut,
        Fut: std::future::Future<Output = IdentityWork>,
    {
        let mut work = IdentityWork::scan(bytes);
        if !work.is_empty() && !self.storage_failed.load(Ordering::Relaxed) {
            work = self.node.lock().unwrap_or_else(|e| e.into_inner()).identity_work_wanted(peer, work);
            if !work.is_empty() {
                let _permit = self.work.acquire().await.ok()?;
                work = verify(work).await;
            }
        }
        self.with_network(|n| n.net_receive_checked(peer, bytes, work))
    }

    fn peer_id(&self, prefix: &str) -> String {
        format!("{prefix}-{}", self.next.fetch_add(1, Ordering::Relaxed))
    }

    fn register(&self, peer: &str) -> (mpsc::Receiver<Queued>, watch::Receiver<bool>) {
        let (tx, rx) = mpsc::channel(QUEUED_MESSAGES);
        let (cancel, cancelled) = watch::channel(false);
        self.links.lock().unwrap_or_else(|e| e.into_inner()).insert(peer.to_string(), Link { tx, cancel, budget: Arc::new(Semaphore::new(QUEUED_BYTES)) });
        (rx, cancelled)
    }

    fn unregister(&self, peer: &str) {
        self.links.lock().unwrap_or_else(|e| e.into_inner()).remove(peer);
        self.with(|n| n.net_disconnect(peer));
    }

    /// Closes every link of a role (e.g. when unpairing).
    pub fn close_all(&self, prefix: &str) {
        let links = self.links.lock().unwrap_or_else(|e| e.into_inner());
        for (k, link) in links.iter() {
            if k.starts_with(prefix) {
                let _ = link.cancel.send(true);
            }
        }
    }

    pub fn clients(&self) -> usize {
        self.links.lock().map(|l| l.keys().filter(|k| k.starts_with("client-")).count()).unwrap_or(0)
    }
}

/// Pumps one WebSocket: node messages out, socket messages in.
async fn run_link<S>(shared: Arc<Shared>, peer: String, role: Role, meta: PeerMeta, ws: tokio_tungstenite::WebSocketStream<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut rx, mut cancelled) = shared.register(&peer);
    let guard = LinkGuard { shared: shared.clone(), peer: peer.clone() };
    if shared
        .with_network(|n| {
            n.net_connect(&peer, role);
            n.set_peer_meta(&peer, meta);
        })
        .is_none()
    {
        return;
    }
    let (mut sink, mut stream) = ws.split();
    // Keep the reader alive across outgoing messages; partial reads must never be discarded.
    let read = async {
        let mut first = true;
        loop {
            let timeout = if first { HELLO } else { Duration::from_secs(180) };
            let inc = match tokio::time::timeout(timeout, stream.next()).await {
                Ok(Some(Ok(inc))) => inc,
                _ => break,
            };
            match inc {
                Message::Binary(b) => {
                    if first && !matches!(hoi_sync::Msg::decode(&b), Ok(hoi_sync::Msg::Hello { .. })) {
                        break;
                    }
                    first = false;
                    if !shared.receive(&peer, &b).await.unwrap_or(false) {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ if first => break,
                // tungstenite queues pong replies automatically.
                _ => {}
            }
        }
    };
    let write = async {
        let heartbeat = Duration::from_secs(30);
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + heartbeat, heartbeat);
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    let Some(msg) = msg else { break };
                    if !matches!(tokio::time::timeout(WRITE_TIMEOUT, sink.send(Message::Binary(msg.bytes.into()))).await, Ok(Ok(()))) { break; }
                }
                _ = tick.tick() => {
                    if shared.with(|n| n.sync_stalled(&peer)) { break; }
                    if !matches!(tokio::time::timeout(WRITE_TIMEOUT, sink.send(Message::Ping(Vec::new().into()))).await, Ok(Ok(()))) { break; }
                }
            }
        }
    };
    tokio::select! {
        _ = read => {},
        _ = write => {},
        _ = cancelled.changed() => {},
    }
    drop(guard);
}

struct LinkGuard {
    shared: Arc<Shared>,
    peer: String,
}
impl Drop for LinkGuard {
    fn drop(&mut self) {
        self.shared.unregister(&self.peer);
    }
}

/// Serves paired devices on `addr` until the task is dropped. Tells the node where it serves.
///
/// Any web page can try to open a WebSocket to a loopback port, so a browser may do it only
/// from the web app this node's pairing links open (its web base) or from one of `origins`;
/// any other page is refused at the handshake. A request without `Origin` comes from a
/// native app and goes on to the credential check like any other. A device without a valid
/// credential hears nothing at all: the link just ends.
pub async fn serve(shared: Arc<Shared>, addr: SocketAddr, origins: &[String]) -> std::io::Result<tokio::task::JoinHandle<()>> {
    let mut allowed = Vec::new();
    for o in origins {
        allowed.push(web_origin(o).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{o:?} is not a web origin")))?);
    }
    allowed.extend(shared.with(|n| web_origin(n.web_base())));
    let allowed: Arc<[String]> = allowed.into();
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    let url = format!("ws://{local}");
    shared.with(|n| n.set_serving(Some(&url))).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(tokio::spawn(async move {
        let slots = Arc::new(Semaphore::new(MAX_CLIENTS));
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            let accepted = tokio::select! {
                pair = listener.accept() => pair,
                _ = tasks.join_next(), if !tasks.is_empty() => continue,
            };
            let Ok((tcp, from)) = accepted else { continue };
            let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
            let shared = shared.clone();
            let allowed = allowed.clone();
            tasks.spawn(async move {
                let _permit = permit;
                let accept = tokio_tungstenite::accept_hdr_async_with_config(tcp, OriginCheck(allowed), Some(ws_config()));
                let Ok(Ok(ws)) = tokio::time::timeout(HANDSHAKE, accept).await else { return };
                let peer = shared.peer_id("client");
                // Behind a TLS proxy (a `wss://` public address) loopback is the proxy, not you.
                let proxied = shared.with(|n| n.serving_url().is_some_and(|u| u.starts_with("wss://")));
                let local = from.ip().is_loopback() && !proxied;
                let meta = PeerMeta { transport: "websocket".into(), local, tls: proxied, incoming: true, ..Default::default() };
                run_link(shared, peer, Role::Client, meta, ws).await;
            });
        }
    }))
}

/// Dials a home node and keeps the link up, retrying with backoff while `keep_going` says so.
pub async fn dial(shared: Arc<Shared>, url: String, keep_going: Arc<dyn Fn() -> bool + Send + Sync>) {
    let mut wait = Duration::from_secs(1);
    while keep_going() {
        match tokio::time::timeout(HANDSHAKE, tokio_tungstenite::connect_async_with_config(url.as_str(), Some(ws_config()), false))
            .await
            .unwrap_or_else(|_| Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed))
        {
            Ok((ws, _)) => {
                wait = Duration::from_secs(1);
                let peer = shared.peer_id("home");
                let tls = url.starts_with("wss://");
                let local = url.split_once("://").and_then(|(_, rest)| rest.split(['/', '?']).next()).is_some_and(|h| {
                    let host = h.rsplit_once(':').map_or(h, |(host, _)| host).trim_matches(['[', ']']);
                    host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
                });
                let meta = PeerMeta { transport: "websocket".into(), local, tls, ..Default::default() };
                run_link(shared.clone(), peer, Role::Home, meta, ws).await;
            }
            Err(e) => {
                let msg = e.to_string();
                shared.with(|n| n.set_net_error(Some(msg)));
            }
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(60));
    }
}

/// Keeps mailboxes and pieces for others as the person's setting says (`hold_pieces_gb`, the
/// desktop's switch under *Be a public node*): a store of its own at `path`, beside the database,
/// opened with that budget and reopened when it changes. Off, the store is closed and its file
/// deleted, so what this computer kept for others goes with it. Returns whether it holds now.
/// `hoi-node` uses `--hold-pieces` instead and never calls this.
pub fn sync_piece_store(shared: &Shared, path: &std::path::Path) -> Result<bool, String> {
    shared.with(|n| {
        // only while a public node: nobody could reach a holder that isn't one (the setting
        // itself is on by default since S6)
        let s = n.settings();
        let want = if s.public_node { (s.hold_pieces_gb * 1e9) as u64 } else { 0 };
        let have = n.piece_stats().map(|s| s.budget);
        if want > 0 && have == Some(want) {
            return Ok(true);
        }
        if have.is_some() {
            // the old store lets go of its file before a new one opens it
            n.set_piece_store(None);
        }
        if want == 0 {
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let mut file = path.as_os_str().to_owned();
                file.push(suffix);
                match std::fs::remove_file(&file) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("Could not delete what this computer kept for others: {e}")),
                }
            }
            return Ok(false);
        }
        let store = hoi_store::pieces::SqlitePieces::open(path, want)?;
        n.set_piece_store(Some(Box::new(store)));
        Ok(true)
    })
}

/// Runs the expiry sweeper every few minutes (TECHNICAL.md §8), and the scattered-delivery
/// round every few seconds: answers that never came, holder sightings, and the mailboxes of
/// private groups (SCATTER.md §5.13).
pub fn spawn_sweeper(shared: Arc<Shared>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut rounds: u64 = 0;
        loop {
            tick.tick().await;
            shared.with(|n| {
                if rounds.is_multiple_of(36) {
                    n.sweep();
                }
                n.scatter_tick();
            });
            rounds = rounds.wrapping_add(1);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_core::dto::{CommunityDraft, LimitsPatch, PostDraft};

    #[test]
    fn a_desktop_keeps_mailboxes_for_others_only_while_its_owner_says_so() {
        let dir = std::env::temp_dir().join(format!("hoi-pieces-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<8>().unwrap())));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("pieces.db");
        let shared = Shared::new(node("desktop"), Box::new(|_| {}));
        let set = |p: hoi_core::dto::SettingsPatch| shared.with(|n| n.set_settings(&p));
        // off by default: nothing opened, nothing on disk
        assert!(!sync_piece_store(&shared, &path).unwrap());
        assert!(!path.exists());
        // only a public node can be reached by others: without one, the switch stays off
        set(hoi_core::dto::SettingsPatch { hold_pieces_gb: Some(2.0), ..Default::default() }).unwrap();
        assert_eq!(shared.with(|n| n.settings().hold_pieces_gb), 0.0);
        assert!(!sync_piece_store(&shared, &path).unwrap());
        // once one, it holds 2 GB unless told otherwise (S6)
        set(hoi_core::dto::SettingsPatch { public_node: Some(true), ..Default::default() }).unwrap();
        assert_eq!(shared.with(|n| n.settings().hold_pieces_gb), 0.0, "turned off above, it stays off");
        set(hoi_core::dto::SettingsPatch { public_node: Some(true), hold_pieces_gb: Some(2.0), ..Default::default() }).unwrap();
        assert!(sync_piece_store(&shared, &path).unwrap());
        assert!(path.exists());
        assert_eq!(shared.with(|n| n.stats().held_budget), 2_000_000_000);
        // a new budget reopens the same store
        set(hoi_core::dto::SettingsPatch { hold_pieces_gb: Some(5.0), ..Default::default() }).unwrap();
        assert!(sync_piece_store(&shared, &path).unwrap());
        assert_eq!(shared.with(|n| n.stats().held_budget), 5_000_000_000);
        // out of range is refused, and a browser or phone can't hold at all
        assert!(set(hoi_core::dto::SettingsPatch { hold_pieces_gb: Some(0.1), ..Default::default() }).is_err());
        let web = Shared::new(node("web"), Box::new(|_| {}));
        assert!(web.with(|n| n.set_settings(&hoi_core::dto::SettingsPatch { hold_pieces_gb: Some(2.0), ..Default::default() })).is_err());
        // no longer a public node: off, and what it kept is deleted
        set(hoi_core::dto::SettingsPatch { public_node: Some(false), ..Default::default() }).unwrap();
        assert!(!sync_piece_store(&shared, &path).unwrap());
        assert!(!shared.with(|n| n.holds_pieces()));
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn node(platform: &str) -> Node {
        let mut n = Node::open(Box::new(hoi_store::Ephemeral), platform, "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
        n.set_identity_bits(8);
        n
    }

    /// An identity record of a fresh key with `work` (valid or not), stamped and signed.
    fn identity_record(root: &hoi_crypto::Keypair, work: u64) -> Vec<u8> {
        let now = hoi_core::system_clock()();
        let mut r = hoi_types::Record {
            v: 1,
            community: None,
            author: root.public(),
            parent: None,
            created_at: now,
            expires_at: now + 365 * 86_400,
            body: hoi_types::Body::Identity { work, rc: None, of: None },
            nonce: 0,
        };
        r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
        root.sign_record(r).0.encode()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identity_work_is_verified_without_the_node_lock() {
        let home = Shared::new(node("desktop"), Box::new(|_| {}));
        let pairing = home.with(|n| {
            n.create_identity("mira").unwrap();
            n.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing_web
        });
        // A paired browser says hello; its identities need no company (PROTOCOL.md §7).
        let mut web = node("web");
        web.pair(&pairing).unwrap();
        web.net_connect("home", Role::Home);
        let hello = web.take_outbox().pop().unwrap().1;
        home.with(|n| n.net_connect("c", Role::Client));
        assert_eq!(home.receive("c", &hello).await, Some(true));

        let key = hoi_crypto::Keypair::generate().unwrap();
        let good = hoi_sync::Msg::Recs { recs: vec![identity_record(&key, hoi_crypto::identity::mine(&key.public(), 4))] }.encode();
        let records = home.with(|n| n.stats().records);
        let checked = home
            .receive_with("c", &good, |work| async {
                assert_eq!(work.len(), 1);
                assert!(home.node.try_lock().is_ok(), "the node is free while Argon2id runs");
                assert_eq!(home.with(|n| n.stats().records), records, "and other callers get through");
                work.verify()
            })
            .await;
        assert_eq!(checked, Some(true));
        assert_eq!(home.with(|n| n.stats().records), records + 1, "the verified identity was taken");

        // Forged work is still refused, and still drops the peer.
        let forger = hoi_crypto::Keypair::generate().unwrap();
        let w = (0..).find(|w| hoi_crypto::identity::work_bits(&forger.public(), *w) < hoi_crypto::identity::MIN_BITS).unwrap();
        let forged = hoi_sync::Msg::Recs { recs: vec![identity_record(&forger, w)] }.encode();
        assert_eq!(home.receive("c", &forged).await, Some(false));
        assert_eq!(home.with(|n| n.stats().records), records + 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn browser_syncs_through_websocket_home_node() {
        let heard: Arc<Mutex<Vec<Event>>> = Arc::default();
        let h2 = heard.clone();
        let home = Shared::new(node("desktop"), Box::new(move |evs| h2.lock().unwrap().extend(evs)));
        let (cid, pid) = home.with(|n| {
            n.create_identity("mira").unwrap();
            let c = n
                .create_community(CommunityDraft {
                    name: "privacy".into(),
                    description: "".into(),
                    color: "mint".into(),
                    limits: Some(LimitsPatch { pow_bits: Some(8), ..Default::default() }),
                    rules: None,
                    co_owners: None,
                    threshold: None,
                })
                .unwrap();
            let p = n.create_post(&PostDraft { community: c.id.clone(), title: "Start here".into(), text: "hi".into(), link: None, media: None }).unwrap();
            (c.id, p.id)
        });
        let _server = serve(home.clone(), "127.0.0.1:0".parse().unwrap(), &[]).await.unwrap();
        let st = home.with(|n| n.net_status());
        let pairing = st.serving.unwrap().pairing;
        let invite = home.with(|n| n.invite(&cid).unwrap().uri);

        let web = Shared::new(node("web"), Box::new(|_| {}));
        let url = web.with(|n| {
            n.create_identity("lena").unwrap();
            n.join(&invite).unwrap();
            n.pair(&pairing).unwrap();
            n.home().unwrap().url
        });
        let w2 = web.clone();
        tokio::spawn(dial(w2, url, Arc::new(|| true)));

        let mut ok = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if web.with(|n| n.feed(None, "fresh", None).unwrap().len()) == 1 {
                ok = true;
                break;
            }
        }
        assert!(ok, "post arrived over the websocket");
        web.with(|n| n.reply(&pid, "hoi back").unwrap());
        let mut back = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if home.with(|n| n.post(&pid).unwrap().comments.len()) == 1 {
                back = true;
                break;
            }
        }
        assert!(back, "reply came back live");
        assert_eq!(home.clients(), 1);
        assert_eq!(web.with(|n| n.net_status().mode), "home-node");
        let clients = heard.lock().unwrap().iter().rev().find_map(|e| match e {
            Event::Net { status } => status.serving.as_ref().map(|s| s.clients),
            _ => None,
        });
        assert_eq!(clients, Some(1), "the desktop's window heard the browser arrive");
    }

    #[test]
    fn origins_are_what_browsers_send() {
        assert_eq!(web_origin("https://hoi.sh/app/").as_deref(), Some("https://hoi.sh"));
        assert_eq!(web_origin("HTTPS://Hoi.SH:443/app/#/pair").as_deref(), Some("https://hoi.sh"));
        assert_eq!(web_origin("http://localhost:5173").as_deref(), Some("http://localhost:5173"));
        assert_eq!(web_origin("http://[::1]:80/").as_deref(), Some("http://[::1]"));
        assert_eq!(web_origin("https://hoi.sh:4430").as_deref(), Some("https://hoi.sh:4430"));
        for bad in ["null", "file:///app/index.html", "ws://127.0.0.1:7842", "https://user@hoi.sh", "https://", "hoi.sh"] {
            assert_eq!(web_origin(bad), None, "{bad}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_the_app_origin_reaches_the_listener_and_a_wrong_token_hears_nothing() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let home = Shared::new(node("desktop"), Box::new(|_| {}));
        assert!(serve(home.clone(), "127.0.0.1:0".parse().unwrap(), &["not an origin".into()]).await.is_err());
        let _server = serve(home.clone(), "127.0.0.1:0".parse().unwrap(), &["http://localhost:5173".into()]).await.unwrap();
        let url = home.with(|n| n.serving_url().unwrap().to_string());
        let open = |origin: Option<&'static str>| {
            let mut req = url.as_str().into_client_request().unwrap();
            if let Some(o) = origin {
                req.headers_mut().insert("origin", o.parse().unwrap());
            }
            tokio_tungstenite::connect_async(req)
        };
        // A page elsewhere can't even open the socket; the app, a configured origin and a
        // native app (no Origin) can.
        assert!(open(Some("https://evil.example")).await.is_err());
        assert!(open(Some("null")).await.is_err());
        for ok in [Some("https://hoi.sh"), Some("http://localhost:5173"), None] {
            assert!(open(ok).await.is_ok(), "{ok:?}");
        }
        // Connected without the credential: no hello, no version, no reason; the link just ends.
        let (mut ws, _) = open(None).await.unwrap();
        let hello = hoi_sync::Msg::Hello {
            v: hoi_sync::SYNC_VERSION,
            node: hoi_types::Id::ZERO,
            name: None,
            token: Some("guess".into()),
            communities: vec![],
            salt: None,
            hashed: vec![],
            serves: false,
            hints: vec![],
            routes: vec![],
            features: vec![],
            pairing: None,
            link_key: None,
            notice_hash: None,
            notice: None,
            holder: None,
            forwards: false,
            rebuilds: false,
        };
        ws.send(Message::Binary(hello.encode().into())).await.unwrap();
        let mut heard = Vec::new();
        while let Ok(Some(Ok(m))) = tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            if let Message::Binary(b) = m {
                heard.push(b);
            }
        }
        assert!(heard.is_empty(), "a wrong token hears nothing");
    }
}
