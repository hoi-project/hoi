//! Authenticated native peers. No default discovery service or relay is contacted.
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey, TransportAddr,
    endpoint::{Connection, Incoming, IncomingAddr, QuicTransportConfig, RecvStream, SendStream, presets},
};
use tokio::{
    sync::Semaphore,
    task::{JoinHandle, JoinSet},
};

use hoi_core::PeerMeta;

use crate::limit::{self, PerSource};
use crate::{HELLO, LinkGuard, Shared, WRITE_TIMEOUT};

pub const SYNC_ALPN: &[u8] = b"hoi/sync/1";
const MAX_PEERS: usize = 32;
/// Temporary holder dials cannot occupy every regular sync slot.
const MAX_FORWARD_PEERS: usize = 8;
const FORWARD_IDLE: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const RESCAN: Duration = Duration::from_secs(5);
/// Connections one source may open to this node per minute, as for WebRTC (rtc.rs).
const NEW_PER_SOURCE: u32 = 10;
/// iroh-blobs connections served at once, and fetches under way at once (crate::blobs).
const BLOBS_SERVED: usize = 8;
const BLOBS_FETCHED: usize = 2;
const BLOBS_POLL: Duration = Duration::from_millis(250);
/// Sources counted per minute; past this, newcomers wait for the next minute.
const SOURCES: usize = 4096;

/// Where an incoming connection is counted: an IPv4 address or IPv6 /64, or, through a relay,
/// the endpoint the relay vouches for (the relay's own address says nothing about the sender).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    Ip(IpAddr),
    Relayed(EndpointId),
}

/// Whether to take an incoming connection. A direct one first answers a stateless QUIC retry,
/// so its address is one that receives: a spoofed source never reaches the count, and can
/// neither hold a slot for the handshake timeout nor use up someone else's allowance.
fn admit(incoming: Incoming, recent: &mut PerSource<Source>, now: Instant) -> Option<Incoming> {
    let source = match incoming.remote_addr() {
        IncomingAddr::Ip(_) if !incoming.remote_addr_validated() => {
            let _ = incoming.retry();
            return None;
        }
        IncomingAddr::Ip(a) => Source::Ip(limit::source(a.ip())),
        IncomingAddr::Relay { endpoint_id, .. } => Source::Relayed(endpoint_id),
        _ => {
            incoming.refuse();
            return None;
        }
    };
    if !recent.take(source, now) {
        incoming.refuse();
        return None;
    }
    Some(incoming)
}

/// Relays are supplied by the operator or an invite, never by hoi or iroh defaults.
#[derive(Clone, Debug, Default)]
pub struct P2pConfig {
    pub bind: Option<SocketAddr>,
    pub relays: Vec<String>,
}

/// Owns the listener and dialer. Keep this alive for the native node's lifetime.
pub struct P2pHandle {
    endpoint: Endpoint,
    task: JoinHandle<()>,
    shared: Arc<Shared>,
    found: Found,
}

/// Addresses learned while running, such as the public side of a port the router forwards.
type Found = Arc<Mutex<Vec<SocketAddr>>>;

impl P2pHandle {
    /// The UDP ports the transport listens on for IPv4, which a router may forward.
    pub fn ipv4_ports(&self) -> Vec<u16> {
        let mut ports: Vec<u16> = self.endpoint.bound_sockets().iter().filter(|a| a.is_ipv4()).map(SocketAddr::port).collect();
        ports.dedup();
        ports
    }

    /// Advertises these addresses as well as the transport's own, in place of those set before.
    pub fn set_advertised(&self, addrs: Vec<SocketAddr>) {
        *self.found.lock().unwrap_or_else(|e| e.into_inner()) = addrs;
        publish_address(&self.shared, &self.endpoint, &self.found);
    }

    pub async fn shutdown(mut self) {
        self.endpoint.close().await;
        self.task.abort();
        let _ = (&mut self.task).await;
        self.shared.with(|n| {
            n.set_blobs(false);
            n.set_transport_address(None, Vec::new(), Vec::new());
        });
    }
}
impl Drop for P2pHandle {
    fn drop(&mut self) {
        self.task.abort();
        self.shared.close_all("peer-");
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn relay_url(raw: &str) -> io::Result<RelayUrl> {
    // Credentials and query strings must never enter invites/status exports. Plain HTTP only
    // for a relay on this machine (tests and development).
    let loopback = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().any(|p| raw.starts_with(p));
    if !(raw.starts_with("https://") || loopback) || raw.contains('@') || raw.contains('?') || raw.contains('#') || raw.len() > 2048 {
        return Err(invalid("relays must be HTTPS URLs without credentials, query or fragment"));
    }
    raw.parse().map_err(|_| invalid("invalid relay URL"))
}
fn peer_address(hint: &str, relays: &[String]) -> io::Result<EndpointAddr> {
    let (key, direct) = match hint.split_once('@') {
        Some((k, a)) => (k, Some(a)),
        None => (hint, None),
    };
    let id: EndpointId = key.parse().map_err(|_| invalid("invalid peer endpoint ID"))?;
    let mut addrs = Vec::new();
    if let Some(raw) = direct {
        let addr: SocketAddr = raw.parse().map_err(|_| invalid("invalid peer socket address"))?;
        if addr.ip().is_unspecified() || addr.ip().is_multicast() || addr.port() == 0 {
            return Err(invalid("peer socket must be reachable"));
        }
        addrs.push(TransportAddr::Ip(addr));
    }
    for raw in relays {
        addrs.push(TransportAddr::Relay(relay_url(raw)?));
    }
    if addrs.is_empty() {
        return Err(invalid("peer has no direct address or relay"));
    }
    Ok(EndpointAddr::from_parts(id, addrs))
}

pub async fn start_p2p(shared: Arc<Shared>, config: P2pConfig) -> io::Result<P2pHandle> {
    if config.relays.len() > 16 {
        return Err(invalid("at most 16 relays may be configured"));
    }
    let relays = config.relays.iter().map(|r| relay_url(r)).collect::<io::Result<Vec<_>>>()?;
    let secret = shared.with(|n| n.transport_secret()).map_err(|e| io::Error::other(e.to_string()))?;
    // Minimal installs only the TLS provider. An empty custom map enables later explicit
    // relay insertion without opting into N0's DNS, pkarr or relay infrastructure.
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::from_bytes(&secret))
        .alpns(vec![SYNC_ALPN.to_vec(), iroh_blobs::ALPN.to_vec()])
        .transport_config(
            QuicTransportConfig::builder()
                .max_concurrent_bidi_streams(1u32.into())
                .max_concurrent_uni_streams(0u32.into())
                .receive_window((4 * 1024 * 1024u32).into())
                .send_window(4 * 1024 * 1024)
                .build(),
        )
        .relay_mode(RelayMode::Custom(RelayMap::from_iter(relays.clone())));
    if let Some(bind) = config.bind {
        builder = builder.clear_ip_transports().bind_addr(bind).map_err(|_| invalid("invalid UDP bind address"))?;
    }
    let endpoint = builder.bind().await.map_err(|_| io::Error::other("could not bind peer transport"))?;
    let found = Found::default();
    publish_address(&shared, &endpoint, &found);
    let ep = endpoint.clone();
    let sh = shared.clone();
    let fd = found.clone();
    let serving = Arc::new(crate::blobs::Serving::new(shared.clone()));
    shared.with(|n| n.set_blobs(true));
    let task = tokio::spawn(async move {
        supervise(sh, ep, relays, fd, serving).await;
    });
    Ok(P2pHandle { endpoint, task, shared, found })
}

/// Those found (a forwarded port's public side) first, so the few routes a hello or an invite
/// has room for include them, then the endpoint's own direct addresses. A dialer tries them all.
fn publish_address(shared: &Shared, endpoint: &Endpoint, found: &Found) {
    let addr = endpoint.addr();
    let id = endpoint.id().to_string();
    let found: Vec<SocketAddr> = found.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut peers: Vec<String> = Vec::new();
    for a in found.into_iter().chain(addr.ip_addrs().filter(|a| !a.ip().is_unspecified()).take(8).copied()) {
        let route = format!("{id}@{a}");
        if !peers.contains(&route) {
            peers.push(route);
        }
    }
    let relays: Vec<String> = addr.relay_urls().map(ToString::to_string).collect();
    if !relays.is_empty() {
        peers.insert(0, id.clone());
    }
    shared.with(|n| n.set_transport_address(Some(id), peers, relays));
}

// Active reservations cover attempts as well as established links. Both listeners and
// dialers share a semaphore; a hostile handshake cannot create unbounded work.
async fn supervise(shared: Arc<Shared>, endpoint: Endpoint, initial_relays: Vec<RelayUrl>, found: Found, serving: Arc<crate::blobs::Serving>) {
    let slots = Arc::new(Semaphore::new(MAX_PEERS));
    let forward_slots = Arc::new(Semaphore::new(MAX_FORWARD_PEERS));
    let served = Arc::new(Semaphore::new(BLOBS_SERVED));
    let fetches = Arc::new(Semaphore::new(BLOBS_FETCHED));
    let mut blob_tick = tokio::time::interval(BLOBS_POLL);
    let active = Arc::new(Mutex::new(BTreeSet::<EndpointId>::new()));
    let connections = Arc::new(Mutex::new(BTreeMap::<EndpointId, (Connection, bool)>::new()));
    let mut attempts: BTreeMap<EndpointId, (u32, tokio::time::Instant)> = BTreeMap::new();
    let mut known_relays: BTreeSet<RelayUrl> = initial_relays.into_iter().collect();
    let mut recent = PerSource::new(NEW_PER_SOURCE, SOURCES);
    let mut jobs = JoinSet::new();
    let mut tick = tokio::time::interval(RESCAN);
    loop {
        tokio::select! {
            _ = endpoint.closed() => break,
            _ = jobs.join_next(), if !jobs.is_empty() => {},
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                // Other people's nodes dialing in are what the relay switch and daily cap govern.
                if !shared.with(|n| n.accepts_members()) { incoming.refuse(); continue; }
                let Some(incoming) = admit(incoming, &mut recent, Instant::now()) else { continue };
                let Ok(slot) = slots.clone().try_acquire_owned() else { incoming.refuse(); continue };
                let sh = shared.clone(); let sessions = connections.clone(); let local = endpoint.id();
                let (serving, served) = (serving.clone(), served.clone());
                jobs.spawn(async move {
                    let slot = slot;
                    let Ok(Ok(conn)) = tokio::time::timeout(CONNECT_TIMEOUT, incoming).await else { return };
                    if conn.alpn() == iroh_blobs::ALPN {
                        // media only, for an endpoint with a sync link here (crate::blobs)
                        let Ok(_serving) = served.try_acquire_owned() else { conn.close(0u32.into(), b"busy"); return };
                        drop(slot);
                        serving.serve(conn).await;
                        return;
                    }
                    run_peer(sh, conn, false, None, local, sessions, FORWARD_IDLE).await;
                });
            },
            _ = blob_tick.tick() => {
                for f in shared.with(|n| n.take_blob_fetches()) {
                    let Ok(id) = f.endpoint.parse::<EndpointId>() else { continue };
                    let Ok(hash) = <[u8; 32]>::try_from(f.hash.as_slice()) else { continue };
                    // where the sync link to it goes, so the media takes the same path
                    let paths: Vec<TransportAddr> = connections.lock().unwrap_or_else(|e| e.into_inner()).get(&id).map(|(c, _)| c.paths().iter().map(|p| p.remote_addr().clone()).collect()).unwrap_or_default();
                    let (ep, sh, fetches) = (endpoint.clone(), shared.clone(), fetches.clone());
                    jobs.spawn(async move {
                        let got = match fetches.acquire_owned().await {
                            Ok(_permit) if !paths.is_empty() => crate::blobs::fetch(&ep, EndpointAddr::from_parts(id, paths), hash, f.size).await,
                            Ok(_) => Err("no link to that peer".into()),
                            Err(_) => return,
                        };
                        sh.with(|n| n.blob_fetched(&f.peer, &hash, got));
                    });
                }
            },
            _ = tick.tick() => {
                let hints = shared.with(|n| n.bootstrap_hints());
                let persistent: BTreeSet<EndpointId> = shared.with(|n| n.persistent_bootstrap_hints()).iter()
                    .filter_map(|(route, relays)| peer_address(route, relays).ok().map(|a| a.id)).collect();
                let mut addresses: BTreeMap<EndpointId, EndpointAddr> = BTreeMap::new();
                for (hint, relays) in hints {
                    let Ok(addr) = peer_address(&hint, &relays) else { continue };
                    if addr.id == endpoint.id() { continue; }
                    for relay in addr.relay_urls() {
                        if known_relays.len() < 16 && known_relays.insert(relay.clone()) {
                            endpoint.insert_relay(relay.clone(), Arc::new(RelayConfig::from(relay.clone()))).await;
                        }
                    }
                    addresses.entry(addr.id).and_modify(|a| a.addrs.extend(addr.addrs.clone())).or_insert(addr);
                }
                publish_address(&shared, &endpoint, &found);
                attempts.retain(|id, _| addresses.contains_key(id));
                for (id, addr) in addresses {
                    if connections.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&id) { attempts.remove(&id); continue; }
                    if attempts.get(&id).is_some_and(|(_, retry)| *retry > tokio::time::Instant::now()) { continue; }
                    let temporary = !persistent.contains(&id);
                    let forward_slot = if temporary { match forward_slots.clone().try_acquire_owned() { Ok(slot) => Some(slot), Err(_) => continue } } else { None };
                    let Ok(slot) = slots.clone().try_acquire_owned() else { break };
                    if !active.lock().unwrap_or_else(|e| e.into_inner()).insert(id) { continue; }
                    let (count, retry) = attempts.entry(id).or_insert((0, tokio::time::Instant::now()));
                    *count = (*count + 1).min(6);
                    *retry = tokio::time::Instant::now() + Duration::from_secs((1 << *count).min(60));
                    let ep = endpoint.clone(); let sh = shared.clone(); let ids = active.clone(); let sessions = connections.clone();
                    jobs.spawn(async move {
                        let _slot = slot;

                        let _reservation = Reservation { active: ids, id };
                        match tokio::time::timeout(CONNECT_TIMEOUT, ep.connect(addr, SYNC_ALPN)).await {
                            Ok(Ok(conn)) => {
                                sh.with(|n| n.set_transport_error(None));
                                run_peer(sh, conn, true, forward_slot, ep.id(), sessions, FORWARD_IDLE).await;
                            }
                            _ => sh.with(|n| {
                                n.set_transport_error(Some("A peer is unreachable. Check its address or relay and try again.".into()));
                                n.dial_failed(&id.to_string());
                            }),
                        }
                    });
                }
            }
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
}
struct Reservation {
    active: Arc<Mutex<BTreeSet<EndpointId>>>,
    id: EndpointId,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.active.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

async fn read_frame(recv: &mut RecvStream) -> io::Result<Vec<u8>> {
    let mut size = [0; 4];
    // Quiet communities stay connected. iroh owns connection keepalives and loss
    // detection; only an unfinished application frame needs a separate deadline.
    recv.read_exact(&mut size[..1]).await.map_err(io::Error::other)?;
    tokio::time::timeout(WRITE_TIMEOUT, async {
        recv.read_exact(&mut size[1..]).await.map_err(io::Error::other)?;
        let len = u32::from_be_bytes(size) as usize;
        if len == 0 || len > hoi_sync::MAX_MESSAGE {
            return Err(invalid("invalid sync frame size"));
        }
        let mut bytes = vec![0; len];
        recv.read_exact(&mut bytes).await.map_err(io::Error::other)?;
        Ok(bytes)
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "unfinished sync frame"))?
}
async fn write_frame(send: &mut SendStream, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > hoi_sync::MAX_MESSAGE {
        return Err(invalid("invalid sync frame size"));
    }
    send.write_all(&(bytes.len() as u32).to_be_bytes()).await.map_err(io::Error::other)?;
    send.write_all(bytes).await.map_err(io::Error::other)
}

/// The UDP address a direct path to the peer uses, if the selected path is direct.
fn direct_addr(conn: &Connection) -> Option<SocketAddr> {
    conn.paths().iter().find(|p| p.is_selected()).and_then(|p| match p.remote_addr() {
        TransportAddr::Ip(a) => Some(*a),
        _ => None,
    })
}

type Connections = Arc<Mutex<BTreeMap<EndpointId, (Connection, bool)>>>;
struct ConnectionGuard {
    connections: Connections,
    id: EndpointId,
    stable_id: usize,
}
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let mut sessions = self.connections.lock().unwrap_or_else(|e| e.into_inner());
        if sessions.get(&self.id).is_some_and(|(conn, _)| conn.stable_id() == self.stable_id) {
            sessions.remove(&self.id);
        }
    }
}
async fn run_peer(
    shared: Arc<Shared>,
    conn: Connection,
    outgoing: bool,
    forward_slot: Option<tokio::sync::OwnedSemaphorePermit>,
    local: EndpointId,
    connections: Connections,
    idle: Duration,
) {
    let id = conn.remote_id();
    // Simultaneous dials converge on the connection initiated by the lower ID.
    // A sole incoming or outgoing connection is always usable, regardless of ordering.
    {
        let mut sessions = connections.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((old, old_outgoing)) = sessions.get(&id) {
            let preferred = local < id;
            if *old_outgoing == preferred || outgoing != preferred {
                conn.close(0u32.into(), b"duplicate");
                return;
            }
            old.close(0u32.into(), b"duplicate");
        }
        sessions.insert(id, (conn.clone(), outgoing));
    }
    let _connection = ConnectionGuard { connections, id, stable_id: conn.stable_id() };
    conn.set_max_concurrent_bi_streams(1u32.into());
    conn.set_max_concurrent_uni_streams(0u32.into());
    let streams = tokio::time::timeout(HELLO, async { if outgoing { conn.open_bi().await } else { conn.accept_bi().await } }).await;
    let Ok(Ok((mut send, mut recv))) = streams else {
        conn.close(0u32.into(), b"no sync stream");
        return;
    };
    let peer = format!("peer-{}-{}", conn.remote_id(), conn.stable_id());
    let (mut rx, mut cancelled) = shared.register(&peer);
    let _guard = LinkGuard { shared: shared.clone(), peer: peer.clone() };
    let meta = |addr: Option<SocketAddr>| PeerMeta {
        transport: "quic".into(),
        incoming: !outgoing,
        endpoint: Some(id.to_string()),
        addr: addr.map(|a| a.to_string()),
        ..Default::default()
    };
    if shared
        .with_network(|n| {
            n.net_connect(&peer, hoi_core::Role::Peer);
            n.set_peer_meta(&peer, meta(direct_addr(&conn)));
        })
        .is_none()
    {
        conn.close(0u32.into(), b"storage unavailable");
        return;
    }
    let read = async {
        let mut first = true;
        loop {
            let received = if first {
                match tokio::time::timeout(HELLO, read_frame(&mut recv)).await {
                    Ok(result) => result,
                    Err(_) => break,
                }
            } else {
                read_frame(&mut recv).await
            };
            let Ok(bytes) = received else { break };
            if first && !matches!(hoi_sync::Msg::decode(&bytes), Ok(hoi_sync::Msg::Hello { .. })) {
                break;
            }
            first = false;
            if !shared.receive(&peer, &bytes).await.unwrap_or(false) {
                break;
            }
        }
    };
    let write = async {
        while let Some(msg) = rx.recv().await {
            if !matches!(tokio::time::timeout(WRITE_TIMEOUT, write_frame(&mut send, &msg.bytes)).await, Ok(Ok(()))) {
                break;
            }
        }
    };
    let routes = async {
        let opened = tokio::time::Instant::now();
        let mut forward_slot = forward_slot;
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tick.tick().await;
            if shared.with(|n| n.sync_stalled(&peer)) {
                break;
            }
            if forward_slot.is_some() {
                let (hints, busy) = shared.with(|n| (n.persistent_bootstrap_hints(), n.forwarding_active(&peer, idle.as_secs())));
                if hints.iter().filter_map(|(r, relays)| peer_address(r, relays).ok()).any(|a| a.id == id) {
                    drop(forward_slot.take()); // It became a regular community/discovery link.
                } else if opened.elapsed() >= idle && !busy {
                    break;
                }
            }
            let route = conn
                .paths()
                .iter()
                .find(|p| p.is_selected())
                .map(|p| {
                    if p.is_relay() {
                        "relay"
                    } else if p.is_ip() {
                        "direct"
                    } else {
                        "unknown"
                    }
                })
                .unwrap_or("unknown");
            let addr = direct_addr(&conn);
            shared.with(|n| {
                n.set_peer_route(&peer, route);
                n.set_peer_meta(&peer, meta(addr));
            });
        }
    };
    tokio::select! { _ = read => {}, _ = write => {}, _ = routes => {}, _ = cancelled.changed() => {} }
    conn.close(0u32.into(), b"sync closed");
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_core::{
        Node,
        dto::{CommunityDraft, LimitsPatch, PostDraft},
    };
    fn node() -> Arc<Shared> {
        let mut n = Node::open(Box::new(hoi_store::Ephemeral), "desktop", "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
        n.set_identity_bits(8);
        Shared::new(n, Box::new(|_| {}))
    }
    #[test]
    fn hints_are_explicit_and_reject_unsafe_addresses() {
        let key = SecretKey::from_bytes(&[42; 32]).public().to_string();
        assert!(peer_address(&key, &[]).is_err());
        assert!(peer_address(&format!("{key}@0.0.0.0:4"), &[]).is_err());
        assert!(peer_address(&format!("{key}@[::1]:1234"), &[]).is_ok());
        assert!(peer_address(&key, &["http://relay.example".into()]).is_err());
        assert!(peer_address(&key, &["https://relay.example".into()]).is_ok());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_idle_temporary_connection_closes_and_releases_its_slot() {
        let a = node();
        let b = node();
        let cfg = P2pConfig { bind: Some("127.0.0.1:0".parse().unwrap()), relays: Vec::new() };
        let first = start_p2p(a.clone(), cfg.clone()).await.unwrap();
        let second = start_p2p(b.clone(), cfg).await.unwrap();
        let hint = b.with(|n| n.net_status().transport.unwrap().peers[0].clone());
        let conn = first.endpoint.connect(peer_address(&hint, &[]).unwrap(), SYNC_ALPN).await.unwrap();
        let slots = Arc::new(Semaphore::new(1));
        let permit = slots.clone().try_acquire_owned().unwrap();
        let task = tokio::spawn(run_peer(a.clone(), conn, true, Some(permit), first.endpoint.id(), Default::default(), Duration::from_millis(200)));
        until("temporary sync connection opened", || a.with(|n| n.net_status().peers > 0)).await;
        assert_eq!(slots.available_permits(), 0);
        tokio::time::timeout(Duration::from_secs(10), task).await.unwrap().unwrap();
        assert_eq!(slots.available_permits(), 1);
        assert_eq!(a.with(|n| n.net_status().peers), 0);
        first.shutdown().await;
        second.shutdown().await;
    }

    #[tokio::test]
    async fn a_forwarded_port_joins_the_routes_and_leaves_them() {
        let a = node();
        let handle = start_p2p(a.clone(), P2pConfig { bind: Some("127.0.0.1:0".parse().unwrap()), relays: Vec::new() }).await.unwrap();
        let ports = handle.ipv4_ports();
        assert_eq!(ports.len(), 1);
        let peers = || a.with(|n| n.net_status().transport.unwrap().peers);
        assert!(peers()[0].ends_with(&format!("@127.0.0.1:{}", ports[0])));
        handle.set_advertised(vec![SocketAddr::from(([93, 184, 216, 34], 7843))]);
        assert!(peers()[0].ends_with("@93.184.216.34:7843"), "{:?}", peers());
        handle.set_advertised(Vec::new());
        assert!(!peers().iter().any(|p| p.contains("93.184.216.34")));
        handle.shutdown().await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_source_opens_ten_connections_a_minute_after_a_retry() {
        let a = node();
        let handle = start_p2p(a.clone(), P2pConfig { bind: Some("127.0.0.1:0".parse().unwrap()), relays: Vec::new() }).await.unwrap();
        let hint = a.with(|n| n.net_status().transport.unwrap().peers[0].clone());
        let addr = peer_address(&hint, &[]).unwrap();
        // Fresh endpoints, so only the source address is what they share. Each answers the
        // listener's retry (iroh does that by itself); the eleventh in a minute is refused.
        let mut held = Vec::new();
        for i in 0..=NEW_PER_SOURCE {
            let ep = Endpoint::builder(presets::Minimal)
                .secret_key(SecretKey::from_bytes(&[i as u8 + 1; 32]))
                .clear_ip_transports()
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap()
                .bind()
                .await
                .unwrap();
            let conn = tokio::time::timeout(CONNECT_TIMEOUT, ep.connect(addr.clone(), SYNC_ALPN)).await.expect("an answer, not a timeout");
            assert_eq!(conn.is_ok(), i < NEW_PER_SOURCE, "connection {i}");
            held.push((ep, conn));
        }
        for (ep, _) in held {
            ep.close().await;
        }
        handle.shutdown().await;
    }

    /// A picture of about `side`² × 3 bytes that doesn't compress.
    fn noise(side: u32) -> Vec<u8> {
        let mut img = image::RgbImage::new(side, side);
        let mut seed = 0x2545_f491_u32;
        for p in img.pixels_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *p = image::Rgb([seed as u8, (seed >> 8) as u8, (seed >> 16) as u8]);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    async fn until(what: &str, check: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(30), async {
            while !check() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{what}"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn a_pinners_media_comes_over_iroh_blobs_and_the_rest_over_the_sync_link() {
        let a = node();
        let b = node();
        let cid = a.with(|n| {
            n.create_identity("mira").unwrap();
            n.create_community(CommunityDraft {
                name: "blobs".into(),
                description: "".into(),
                color: "mint".into(),
                limits: Some(LimitsPatch { pow_bits: Some(8), ..Default::default() }),
                rules: None,
                co_owners: None,
                threshold: None,
            })
            .unwrap()
            .id
        });
        let cfg = P2pConfig { bind: Some("127.0.0.1:0".parse().unwrap()), relays: Vec::new() };
        let first = start_p2p(a.clone(), cfg.clone()).await.unwrap();
        let invite = a.with(|n| n.invite(&cid).unwrap().uri);
        b.with(|n| {
            n.create_identity("lena").unwrap();
            n.join(&invite).unwrap();
        });
        let second = start_p2p(b.clone(), cfg).await.unwrap();
        let picture = |n: &mut Node, title: &str, side| {
            n.create_post(&PostDraft {
                community: cid.clone(),
                title: title.into(),
                text: String::new(),
                link: None,
                media: Some(vec![hoi_core::dto::MediaDraft { data: hoi_types::uri::b64(&noise(side)), alt: None }]),
            })
            .unwrap()
            .id
        };
        // a made the community, so a pins it: its picture comes to b over iroh-blobs
        let pid = a.with(|n| picture(n, "pinned", 400));
        until("the pinner's picture arrived", || b.with(|n| n.post(&pid).is_ok_and(|p| p.summary.media.iter().all(|m| m.available)))).await;
        assert_eq!(b.with(|n| n.blobs_counts()).0, 1, "fetched over iroh-blobs");
        assert_eq!(a.with(|n| n.blobs_counts()).1, 1, "served over iroh-blobs");
        assert!(a.with(|n| n.traffic().media_out) > 400_000);
        // no longer pinned there: a turns the ask down, and b asks with `bwant` instead
        a.with(|n| n.set_pinning(&cid, false).unwrap());
        let qid = a.with(|n| picture(n, "not pinned", 300));
        until("the second picture arrived", || b.with(|n| n.post(&qid).is_ok_and(|p| p.summary.media.iter().all(|m| m.available)))).await;
        assert_eq!(b.with(|n| n.blobs_counts()).0, 1, "not over iroh-blobs");
        assert_eq!(a.with(|n| n.blobs_counts()).1, 1, "not over iroh-blobs");
        second.shutdown().await;
        first.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn independent_nodes_sync_over_authenticated_quic() {
        let a = node();
        let b = node();
        let cid = a.with(|n| {
            n.create_identity("mira").unwrap();
            n.create_community(CommunityDraft {
                name: "quic".into(),
                description: "".into(),
                color: "mint".into(),
                limits: Some(LimitsPatch { pow_bits: Some(8), ..Default::default() }),
                rules: None,
                co_owners: None,
                threshold: None,
            })
            .unwrap()
            .id
        });
        let cfg = P2pConfig { bind: Some("127.0.0.1:0".parse().unwrap()), relays: Vec::new() };
        let first = start_p2p(a.clone(), cfg.clone()).await.unwrap();
        let invite = a.with(|n| n.invite(&cid).unwrap().uri);
        b.with(|n| {
            n.create_identity("lena").unwrap();
            n.join(&invite).unwrap();
        });
        let second = start_p2p(b.clone(), cfg).await.unwrap();
        let reverse = b.with(|n| n.invite(&cid).unwrap().uri);
        a.with(|n| n.join(&reverse).unwrap());
        let pid = a.with(|n| {
            n.create_post(&PostDraft { community: cid.clone(), title: "Across QUIC".into(), text: "hello".into(), link: None, media: None }).unwrap().id
        });
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if b.with(|n| n.post(&pid).is_ok()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("post arrived over QUIC");
        b.with(|n| n.reply(&pid, "back through QUIC").unwrap());
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if a.with(|n| n.post(&pid).unwrap().comments.len()) == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("reply arrived live");
        assert!(a.with(|n| n.net_status().peers) > 0);
        // A browser attached to the other home node receives the same QUIC-synced data.
        let server = crate::serve(b.clone(), "127.0.0.1:0".parse().unwrap(), &[]).await.unwrap();
        let pairing = b.with(|n| n.net_status().serving.unwrap().pairing);
        let web = node();
        let url = web.with(|n| {
            n.create_identity("browser").unwrap();
            n.join(&invite).unwrap();
            n.pair(&pairing).unwrap();
            n.home().unwrap().url
        });
        let gateway = tokio::spawn(crate::dial(web.clone(), url, Arc::new(|| true)));
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if web.with(|n| n.post(&pid).is_ok()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("browser received the record through the remote home node");
        gateway.abort();
        server.abort();
        // Force both ends down: two simultaneous redials must converge, and queued
        // local data must be recovered by the next have/want exchange.
        a.close_all("peer-");
        b.close_all("peer-");
        let later = a.with(|n| {
            n.create_post(&PostDraft { community: cid.clone(), title: "After reconnect".into(), text: "still here".into(), link: None, media: None })
                .unwrap()
                .id
        });
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if b.with(|n| n.post(&later).is_ok()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("catch-up recovered the post after simultaneous reconnects");
        second.shutdown().await;
        first.shutdown().await;
    }
}
