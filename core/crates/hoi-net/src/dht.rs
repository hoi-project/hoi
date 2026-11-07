//! The DHT among public nodes, the BitTorrent way: a Mainline DHT node (`mainline`)
//! that public nodes run beside their other ports, so a community can be found by its key alone
//! once every route anyone holds for it has gone stale.
//!
//! - **No outside bootstrap.** The DHT starts from what hoi's own links say: public nodes tell
//!   each other where their DHT nodes listen (`dht` messages, crate hoi-core `find`). Joining
//!   BitTorrent's worldwide DHT is one operator's choice (`--dht-bootstrap HOST:PORT`); once one
//!   node bridges, the others learn the wider network from each other.
//! - **The key never enters the DHT**: each open community a node carries is announced under
//!   the first 20 bytes of `BLAKE3("hoi dht v1" ‖ key)`, with the node's *answer port*.
//! - **Answers are signed.** A finder sends a small padded query to that port; the node answers
//!   with a statement its endpoint key signed for that community, which the core checks before
//!   any route in it is tried. The answer port answers queries only for communities the node
//!   carries, a few a minute per source, and never more than about twice what it was sent.
//! - Lookups run for communities a connected browser asked about (`find`) or that this node
//!   has joined and nobody serves.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use hoi_core::find::{DhtPorts, dht_key, dht_routes};
use hoi_types::Id;
use mainline::Dht;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use crate::Shared;
use crate::limit::{PerSource, source};

/// The DHT port public nodes listen on by default, and the port beside it that answers hoi
/// queries.
pub const DEFAULT_PORT: u16 = 7846;
pub const DEFAULT_ANSWER_PORT: u16 = 7847;
/// A query: magic, version, the community key, a nonce, zeros up to this length (so an answer,
/// at most [`MAX_ANSWER`], is never much more than twice what was sent).
pub const QUERY_LEN: usize = 512;
const QUERY_MAGIC: &[u8; 4] = b"HOI?";
const ANSWER_MAGIC: &[u8; 4] = b"HOI!";
const VERSION: u8 = 1;
pub const MAX_ANSWER: usize = 1200;
/// Queries one source may send a minute, in a table of at most this many sources.
const QUERIES_PER_MINUTE: u32 = 30;
const SOURCES: usize = 4096;
/// Communities are announced again this often; Mainline forgets peers after about half an hour.
const ANNOUNCE_EVERY: Duration = Duration::from_secs(15 * 60);
const TICK: Duration = Duration::from_secs(10);
/// A lookup gathers at most this many announced nodes, for at most this long, and waits this
/// long for each answer.
const LOOKUP_PEERS: usize = 16;
const LOOKUP_FOR: Duration = Duration::from_secs(15);
const ANSWER_WAIT: Duration = Duration::from_secs(3);
/// While the DHT knows nobody, it is started again from the contacts hoi links gave, at most
/// this often.
const RESTART_EVERY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Default)]
pub struct DhtConfig {
    /// The DHT port (UDP), [`DEFAULT_PORT`] when 0.
    pub port: u16,
    /// The port answering hoi queries (UDP), [`DEFAULT_ANSWER_PORT`] when 0.
    pub answer_port: u16,
    /// Outside DHT nodes to start from, chosen by the operator; none by default.
    pub bootstrap: Vec<String>,
    /// The address to listen on, for a node that must stay on one interface (a VPN).
    pub bind: Option<Ipv4Addr>,
}

pub struct DhtHandle {
    tasks: Vec<JoinHandle<()>>,
    pub ports: DhtPorts,
}

impl Drop for DhtHandle {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// A query for `community` with `nonce`.
pub fn query(community: &Id, nonce: &[u8; 16]) -> Vec<u8> {
    let mut q = Vec::with_capacity(QUERY_LEN);
    q.extend_from_slice(QUERY_MAGIC);
    q.push(VERSION);
    q.extend_from_slice(community.as_bytes());
    q.extend_from_slice(nonce);
    q.resize(QUERY_LEN, 0);
    q
}

/// The community and nonce a query asks with, if it is one.
pub fn parse_query(b: &[u8]) -> Option<(Id, [u8; 16])> {
    if b.len() != QUERY_LEN || &b[..4] != QUERY_MAGIC || b[4] != VERSION || b[53..].iter().any(|x| *x != 0) {
        return None;
    }
    Some((Id::from_slice(&b[5..37]).ok()?, b[37..53].try_into().ok()?))
}

fn answer_bytes(nonce: &[u8; 16], payload: &[u8]) -> Option<Vec<u8>> {
    let mut a = Vec::with_capacity(21 + payload.len());
    a.extend_from_slice(ANSWER_MAGIC);
    a.push(VERSION);
    a.extend_from_slice(nonce);
    a.extend_from_slice(payload);
    (a.len() <= MAX_ANSWER).then_some(a)
}

/// The signed payload of an answer to the query with `nonce`.
pub fn parse_answer<'a>(b: &'a [u8], nonce: &[u8; 16]) -> Option<&'a [u8]> {
    (b.len() > 21 && b.len() <= MAX_ANSWER && &b[..4] == ANSWER_MAGIC && b[4] == VERSION && &b[5..21] == nonce).then(|| &b[21..])
}

fn build(port: u16, bootstrap: &[SocketAddrV4], bind: Option<Ipv4Addr>) -> std::io::Result<mainline::async_dht::AsyncDht> {
    let mut b = Dht::builder();
    b.server_mode().port(port);
    if bootstrap.is_empty() {
        b.no_bootstrap();
    } else {
        b.bootstrap(bootstrap);
    }
    if let Some(ip) = bind {
        b.bind_address(ip);
    }
    Ok(b.build()?.as_async())
}

/// Starts the DHT node and the answer port, and tells the core it runs one.
pub async fn start_dht(shared: Arc<Shared>, config: DhtConfig) -> std::io::Result<DhtHandle> {
    let ports = DhtPorts {
        dht: if config.port == 0 { DEFAULT_PORT } else { config.port },
        answer: if config.answer_port == 0 { DEFAULT_ANSWER_PORT } else { config.answer_port },
    };
    let bind = config.bind.unwrap_or(Ipv4Addr::UNSPECIFIED);
    let answer = Arc::new(UdpSocket::bind(SocketAddrV4::new(bind, ports.answer)).await?);
    let outside: Vec<SocketAddrV4> = config.bootstrap.iter().filter_map(|s| resolve(s)).collect();
    let dht = build(ports.dht, &outside, config.bind)?;
    shared.with(|n| n.set_dht(Some(ports)));
    let mut tasks = vec![tokio::spawn(answer_loop(shared.clone(), answer))];
    tasks.push(tokio::spawn(dht_loop(shared, dht, ports, outside, config.bind)));
    Ok(DhtHandle { tasks, ports })
}

fn resolve(s: &str) -> Option<SocketAddrV4> {
    use std::net::ToSocketAddrs;
    s.to_socket_addrs().ok()?.find_map(|a| match a {
        SocketAddr::V4(v4) => Some(v4),
        _ => None,
    })
}

/// Answers hoi queries for communities this node carries.
async fn answer_loop(shared: Arc<Shared>, sock: Arc<UdpSocket>) {
    let mut limit = PerSource::new(QUERIES_PER_MINUTE, SOURCES);
    let mut buf = vec![0u8; 2048];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf).await else { continue };
        let Some((community, nonce)) = parse_query(&buf[..n]) else { continue };
        if !limit.take(source(from.ip()), Instant::now()) {
            continue;
        }
        let Some(payload) = shared.with(|node| node.dht_answer(&community)) else { continue };
        if let Some(a) = answer_bytes(&nonce, &payload) {
            let _ = sock.send_to(&a, from).await;
        }
    }
}

/// Keeps the DHT going: started again from hoi's contacts while it knows nobody, announcing
/// what this node carries, and running the lookups the core asks for.
async fn dht_loop(shared: Arc<Shared>, mut dht: mainline::async_dht::AsyncDht, ports: DhtPorts, outside: Vec<SocketAddrV4>, bind: Option<Ipv4Addr>) {
    let mut announced: Option<(Instant, Vec<Id>)> = None;
    let mut restarted = Instant::now();
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let (contacts, communities, lookups) = shared.with(|n| (n.dht_contacts(), n.dht_communities(), n.dht_lookups()));
        // nobody known yet: start over from what hoi's links said
        if dht.to_bootstrap().await.is_empty() && !contacts.is_empty() && restarted.elapsed() >= RESTART_EVERY {
            restarted = Instant::now();
            let from: Vec<SocketAddrV4> = contacts.iter().chain(&outside).copied().collect::<BTreeSet<_>>().into_iter().collect();
            drop(dht);
            // the old node lets go of its port as its actor stops
            tokio::time::sleep(Duration::from_millis(500)).await;
            dht = match build(ports.dht, &from, bind) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("dht: could not start again: {e}");
                    return;
                }
            };
            announced = None;
        }
        let due = announced.as_ref().is_none_or(|(at, list)| at.elapsed() >= ANNOUNCE_EVERY || *list != communities);
        if due && !communities.is_empty() {
            for c in &communities {
                let Ok(id) = mainline::Id::from_bytes(dht_key(c)) else { continue };
                let _ = tokio::time::timeout(Duration::from_secs(20), dht.announce_peer(id, Some(ports.answer))).await;
            }
            announced = Some((Instant::now(), communities.clone()));
        }
        for c in lookups {
            let dht = dht.clone();
            let shared = shared.clone();
            tokio::spawn(async move {
                let routes = lookup(&dht, &c).await;
                if !routes.is_empty() {
                    shared.with(|n| n.dht_found(c, routes));
                }
            });
        }
    }
}

/// Who the DHT says carries `c`, asked for their signed answers, checked.
pub async fn lookup(dht: &mainline::async_dht::AsyncDht, c: &Id) -> Vec<String> {
    let Ok(id) = mainline::Id::from_bytes(dht_key(c)) else { return Vec::new() };
    let mut peers: BTreeSet<SocketAddrV4> = BTreeSet::new();
    let mut stream = dht.get_peers(id);
    let deadline = tokio::time::Instant::now() + LOOKUP_FOR;
    while peers.len() < LOOKUP_PEERS {
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(batch)) => peers.extend(batch),
            _ => break,
        }
    }
    let Ok(sock) = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).await else { return Vec::new() };
    let mut routes = Vec::new();
    for peer in peers.into_iter().take(LOOKUP_PEERS) {
        let Ok(nonce) = hoi_crypto::random_bytes::<16>() else { break };
        if sock.send_to(&query(c, &nonce), peer).await.is_err() {
            continue;
        }
        let mut buf = vec![0u8; MAX_ANSWER + 64];
        let until = tokio::time::Instant::now() + ANSWER_WAIT;
        while let Ok(Ok((n, from))) = tokio::time::timeout_at(until, sock.recv_from(&mut buf)).await {
            if from != SocketAddr::V4(peer) {
                continue;
            }
            if let Some(payload) = parse_answer(&buf[..n], &nonce)
                && let Some(found) = dht_routes(payload, c, now())
            {
                routes.extend(found);
            }
            break;
        }
    }
    let mut seen = BTreeSet::new();
    routes.retain(|r| seen.insert(r.clone()));
    routes
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_core::Node;

    fn free_udp() -> u16 {
        std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    fn node(platform: &str) -> Node {
        let mut n = Node::open(Box::new(hoi_store::Ephemeral), platform, "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
        n.set_identity_bits(8);
        n
    }

    /// A public node that carries an open community announces it in the DHT; a node that only
    /// knows the community's key finds it there, asks the answer port, and checks what comes back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_community_is_found_by_its_key_through_the_dht() {
        let testnet = mainline::Testnet::builder(6).build().unwrap();
        let mut owner = node("desktop");
        owner.create_identity("mira").unwrap();
        let mut draft = hoi_core::dto::CommunityDraft {
            name: "garden".into(),
            description: "".into(),
            color: "mint".into(),
            limits: None,
            rules: None,
            co_owners: None,
            threshold: None,
        };
        draft.limits = Some(hoi_core::dto::LimitsPatch { pow_bits: Some(8), ..Default::default() });
        let c = owner.create_community(draft).unwrap();
        let cid = Id::from_hex(&c.id).unwrap();
        let mut carrier = node("node");
        carrier.set_settings(&hoi_core::dto::SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
        carrier.set_carry_policy(hoi_core::CarryPolicy::Any, 8);
        carrier.join(&owner.invite(&c.id).unwrap().uri).unwrap();
        let endpoint = hoi_crypto::Keypair::from_secret(&carrier.transport_secret().unwrap()).public().hex();
        carrier.set_transport_address(Some(endpoint.clone()), vec![format!("{endpoint}@127.0.0.1:17843")], vec![]);
        let shared = Shared::new(carrier, Box::new(|_| {}));
        let config = DhtConfig { port: free_udp(), answer_port: free_udp(), bootstrap: testnet.bootstrap.clone(), bind: Some(Ipv4Addr::LOCALHOST) };
        let handle = start_dht(shared.clone(), config).await.unwrap();
        assert!(shared.with(|n| n.dht_communities()) == vec![cid]);

        let from: Vec<SocketAddrV4> = testnet.bootstrap.iter().filter_map(|s| resolve(s)).collect();
        let finder = build(free_udp(), &from, Some(Ipv4Addr::LOCALHOST)).unwrap();
        let mut routes = Vec::new();
        for _ in 0..40 {
            routes = lookup(&finder, &cid).await;
            if !routes.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert_eq!(routes, vec![format!("{endpoint}@127.0.0.1:17843")]);
        // a community nobody announced finds nothing
        assert!(lookup(&finder, &Id([9; 32])).await.is_empty());
        drop(handle);
    }

    #[test]
    fn queries_are_padded_and_answers_bounded() {
        let c = Id([7; 32]);
        let nonce = [3u8; 16];
        let q = query(&c, &nonce);
        assert_eq!(q.len(), QUERY_LEN);
        assert_eq!(parse_query(&q), Some((c, nonce)));
        let mut short = q.clone();
        short.pop();
        assert!(parse_query(&short).is_none());
        let mut dirty = q.clone();
        dirty[QUERY_LEN - 1] = 1;
        assert!(parse_query(&dirty).is_none(), "the padding is zeros");
        let a = answer_bytes(&nonce, b"payload").unwrap();
        assert_eq!(parse_answer(&a, &nonce), Some(&b"payload"[..]));
        assert!(parse_answer(&a, &[4u8; 16]).is_none(), "another query's answer");
        assert!(answer_bytes(&nonce, &[0u8; MAX_ANSWER]).is_none());
        const { assert!(MAX_ANSWER <= 3 * QUERY_LEN) };
    }
}
