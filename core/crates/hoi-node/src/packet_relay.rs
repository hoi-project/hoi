//! The embedded packet relay (`--packet-relay`): iroh's own relay server, run beside
//! the node, so two nodes that can't reach each other directly (both behind routers that won't
//! forward) still connect, through a relay their community's node operates instead of anyone's
//! default. The relay forwards QUIC packets it can't read, by destination endpoint, and keeps
//! nothing but an in-memory table of who is connected now.
//!
//! - **Who may use it**: everyone but the endpoints on the operator's block list
//!   (`packet-relay-blocked.txt` in the data directory, one endpoint key a line, read again when
//!   it changes; `hoi-node admin relay block|unblock|list`). With `--packet-relay-members`, only
//!   endpoints this node knows as members of the communities it carries. iroh's clients send no
//!   token of their own, so this stands in for PROTOCOL.md §11's bearer token.
//! - **How much**: each client may send at most 1 MiB a second (4 MiB bursts).
//! - **TLS**: on plain HTTP behind a TLS proxy (Caddy, `install.sh --tls caddy`), or HTTPS from a
//!   certificate and key in PEM files. Its public address (`--packet-relay-url`) goes into the
//!   node's routes, so invites name it as a relay.
//! - **Logs**: none; the node installs no tracing subscriber.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use hoi_net::Shared;
use hoi_types::Id;
use iroh_relay::server::{Access, AccessControl, ClientRateLimit, ClientRequest, Limits, RelayConfig, Server, ServerConfig};

/// The block list's file name in the data directory.
pub const BLOCKED_FILE: &str = "packet-relay-blocked.txt";
const BYTES_PER_SECOND: u32 = 1 << 20;
const BURST_BYTES: u32 = 4 << 20;
const RELOAD_EVERY: Duration = Duration::from_secs(30);

pub struct Options {
    pub listen: SocketAddr,
    pub cert: Option<(PathBuf, PathBuf)>,
    pub members_only: bool,
    pub data: PathBuf,
}

/// Endpoint keys in a block list file: one 64-hex key at the start of each line, `#` comments.
pub fn read_blocked(path: &Path) -> BTreeSet<[u8; 32]> {
    let Ok(text) = std::fs::read_to_string(path) else { return BTreeSet::new() };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
        .filter_map(|k| Id::from_hex(&k.to_ascii_lowercase()).ok())
        .map(|id| id.0)
        .collect()
}

#[derive(Debug)]
struct Gate {
    shared: Arc<SharedRef>,
    blocked: Arc<RwLock<BTreeSet<[u8; 32]>>>,
    members_only: bool,
}

/// `Shared` isn't `Debug`; the gate only needs to ask it one question.
struct SharedRef(Arc<Shared>);
impl std::fmt::Debug for SharedRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("node")
    }
}

impl AccessControl for Gate {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        let id = *request.endpoint_id().as_bytes();
        if self.blocked.read().is_ok_and(|b| b.contains(&id)) {
            return Access::Deny { reason: Some("this relay's operator refuses this endpoint".into()) };
        }
        if self.members_only && !self.shared.0.with(|n| n.relay_member(&Id(id))) {
            return Access::Deny { reason: Some("this relay serves the members of the communities its node carries".into()) };
        }
        Access::Allow
    }
}

fn tls_config(cert: &Path, key: &Path) -> Result<rustls::ServerConfig, String> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert)
        .map_err(|e| format!("{}: {e}", cert.display()))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", cert.display()))?;
    let key = PrivateKeyDer::from_pem_file(key).map_err(|e| format!("{}: {e}", key.display()))?;
    rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("the relay's certificate and key don't fit: {e}"))
}

/// Starts the relay. Keep the server alive for as long as it should run.
pub async fn start(shared: Arc<Shared>, o: Options) -> Result<Server, String> {
    let blocked_path = o.data.join(BLOCKED_FILE);
    let blocked = Arc::new(RwLock::new(read_blocked(&blocked_path)));
    {
        // the operator's list is read again whenever it changes
        let blocked = blocked.clone();
        tokio::spawn(async move {
            let mut seen: Option<SystemTime> = None;
            loop {
                tokio::time::sleep(RELOAD_EVERY).await;
                let modified = std::fs::metadata(&blocked_path).and_then(|m| m.modified()).ok();
                if modified != seen {
                    seen = modified;
                    if let Ok(mut b) = blocked.write() {
                        *b = read_blocked(&blocked_path);
                    }
                }
            }
        });
    }
    let mut relay = RelayConfig::new(o.listen);
    let mut rate = ClientRateLimit::new(NonZeroU32::new(BYTES_PER_SECOND).expect("non-zero"));
    rate.max_burst_bytes = NonZeroU32::new(BURST_BYTES);
    let mut limits = Limits::default();
    limits.client_rx = Some(rate);
    relay.limits = limits;
    relay.access = Arc::new(Gate { shared: Arc::new(SharedRef(shared)), blocked, members_only: o.members_only });
    if let Some((cert, key)) = &o.cert {
        let cert = iroh_relay::server::CertConfig::Manual { server_config: tls_config(cert, key)? };
        relay.tls = Some(iroh_relay::server::TlsConfig::new(o.listen, cert));
        // the plain HTTP side (captive-portal checks) moves one port down
        relay.http_bind_addr = SocketAddr::new(o.listen.ip(), o.listen.port().saturating_sub(1).max(1));
    }
    let mut config = ServerConfig::default();
    config.relay = Some(relay);
    Server::spawn(config).await.map_err(|e| format!("the packet relay could not start: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_core::Node;
    use hoi_core::dto::{CommunityDraft, LimitsPatch, PostDraft};

    fn node() -> Arc<Shared> {
        let mut n = Node::open(Box::new(hoi_store::Ephemeral), "desktop", "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
        n.set_identity_bits(8);
        Shared::new(n, Box::new(|_| {}))
    }

    /// Two nodes that know each other only by endpoint key and this relay still sync.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nodes_that_know_each_other_only_through_the_relay_sync() {
        let dir = std::env::temp_dir().join(format!("hoi-relay-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<6>().unwrap())));
        std::fs::create_dir_all(&dir).unwrap();
        let a = node();
        let b = node();
        let server = start(a.clone(), Options { listen: "127.0.0.1:0".parse().unwrap(), cert: None, members_only: false, data: dir.clone() }).await.unwrap();
        let url = format!("http://{}", server.http_addr().unwrap());
        let cid = a.with(|n| {
            n.create_identity("mira").unwrap();
            let limits = Some(LimitsPatch { pow_bits: Some(8), ..Default::default() });
            n.create_community(CommunityDraft {
                name: "relay".into(),
                description: "".into(),
                color: "mint".into(),
                limits,
                rules: None,
                co_owners: None,
                threshold: None,
            })
            .unwrap()
            .id
        });
        let cfg = hoi_net::P2pConfig { bind: Some("127.0.0.1:0".parse().unwrap()), relays: vec![url.clone()] };
        let _first = hoi_net::start_p2p(a.clone(), cfg.clone()).await.unwrap();
        let endpoint = a.with(|n| hoi_crypto::Keypair::from_secret(&n.transport_secret().unwrap()).public().hex());
        // an invite naming only the endpoint key and the relay: no address to dial
        let mut inv = hoi_types::Invite::parse(&a.with(|n| n.invite(&cid).unwrap().uri)).unwrap();
        inv.peers = vec![endpoint];
        inv.relays = vec![url];
        b.with(|n| {
            n.create_identity("lena").unwrap();
            n.join(&inv.to_uri()).unwrap();
        });
        let _second = hoi_net::start_p2p(b.clone(), cfg).await.unwrap();
        let pid = a.with(|n| {
            n.create_post(&PostDraft { community: cid.clone(), title: "Through the relay".into(), text: "hi".into(), link: None, media: None }).unwrap().id
        });
        tokio::time::timeout(Duration::from_secs(40), async {
            while !b.with(|n| n.post(&pid).is_ok()) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("the post came through the relay");

        // refused on the block list: a new endpoint is turned away
        let c = node();
        let refused = c.with(|n| hoi_crypto::Keypair::from_secret(&n.transport_secret().unwrap()).public().hex());
        std::fs::write(dir.join(BLOCKED_FILE), format!("{refused}\n")).unwrap();
        assert!(read_blocked(&dir.join(BLOCKED_FILE)).contains(&Id::from_hex(&refused).unwrap().0));
        drop(server);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// M0's acceptance, scripted: two nodes that reach each other only through
    /// the embedded packet relay; a post shows on the other within 5 seconds once they are
    /// linked; at the end of the community's retention it is gone from both. Real QUIC over
    /// loopback, the nodes' clocks under the test's hand. `release-major.yml` runs it (`--ignored`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "real network and up to a minute: the major-release suites run it"]
    async fn m0_acceptance_two_nodes_relay_only_five_seconds_and_expiry() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let dir = std::env::temp_dir().join(format!("hoi-m0-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<6>().unwrap())));
        std::fs::create_dir_all(&dir).unwrap();
        let skew = Arc::new(AtomicU64::new(0));
        let node = |skew: Arc<AtomicU64>| {
            let clock = Box::new(move || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + skew.load(Ordering::SeqCst));
            let mut n = Node::open(Box::new(hoi_store::Ephemeral), "desktop", "https://hoi.sh/app/", clock).unwrap();
            n.set_identity_bits(8);
            Shared::new(n, Box::new(|_| {}))
        };
        let (a, b) = (node(skew.clone()), node(skew.clone()));
        let server = start(a.clone(), Options { listen: "127.0.0.1:0".parse().unwrap(), cert: None, members_only: false, data: dir.clone() }).await.unwrap();
        let url = format!("http://{}", server.http_addr().unwrap());
        let cid = a.with(|n| {
            n.create_identity("mira").unwrap();
            let limits = Some(LimitsPatch { pow_bits: Some(8), max_retention: Some(3600), ..Default::default() });
            n.create_community(CommunityDraft {
                name: "m0".into(),
                description: "".into(),
                color: "mint".into(),
                limits,
                rules: None,
                co_owners: None,
                threshold: None,
            })
            .unwrap()
            .id
        });
        let cfg = hoi_net::P2pConfig { bind: Some("127.0.0.1:0".parse().unwrap()), relays: vec![url.clone()] };
        let _first = hoi_net::start_p2p(a.clone(), cfg.clone()).await.unwrap();
        let endpoint = a.with(|n| hoi_crypto::Keypair::from_secret(&n.transport_secret().unwrap()).public().hex());
        let mut inv = hoi_types::Invite::parse(&a.with(|n| n.invite(&cid).unwrap().uri)).unwrap();
        inv.peers = vec![endpoint];
        inv.relays = vec![url];
        b.with(|n| {
            n.create_identity("lena").unwrap();
            n.join(&inv.to_uri()).unwrap();
        });
        let _second = hoi_net::start_p2p(b.clone(), cfg).await.unwrap();
        let post = |n: &Arc<Shared>, title: &str| {
            let title = title.to_string();
            let cid = cid.clone();
            n.with(move |n| n.create_post(&PostDraft { community: cid, title, text: "hi".into(), link: None, media: None }).unwrap().id)
        };
        let seen = |n: &Arc<Shared>, id: &str| {
            let id = id.to_string();
            n.with(move |n| n.post(&id).is_ok())
        };
        // linked: the first post comes through once the nodes found each other on the relay
        let first = post(&a, "Linking up");
        tokio::time::timeout(Duration::from_secs(40), async {
            while !seen(&b, &first) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the nodes linked through the relay");
        // then a post shows on the other within five seconds, both ways
        for (from, to, title) in [(&a, &b, "Five seconds there"), (&b, &a, "Five seconds back")] {
            let id = post(from, title);
            let t0 = std::time::Instant::now();
            while !seen(to, &id) {
                assert!(t0.elapsed() < Duration::from_secs(5), "{title}: not there within five seconds");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            println!("m0: {title} in {} ms", t0.elapsed().as_millis());
        }
        // at the end of the retention, gone from both
        skew.store(3601, Ordering::SeqCst);
        for n in [&a, &b] {
            n.with(|n| {
                n.sweep();
                n.commit().unwrap();
            });
        }
        assert!(!seen(&a, &first) && !seen(&b, &first), "expired records are deleted on both nodes");
        drop(server);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn request(endpoint: [u8; 32]) -> ClientRequest {
        let id = iroh_base::EndpointId::from_bytes(&endpoint).unwrap();
        ClientRequest::new(id, iroh_relay::http::ProtocolVersion::default(), hyper::Request::new(()).into_parts().0)
    }

    #[tokio::test]
    async fn the_gate_refuses_the_block_list_and_in_members_mode_strangers() {
        let shared = node();
        let member = hoi_crypto::Keypair::generate().unwrap().public();
        let stranger = hoi_crypto::Keypair::generate().unwrap().public();
        let blocked = hoi_crypto::Keypair::generate().unwrap().public();
        // the node joined a community with an invite naming the member's endpoint
        shared.with(|n| {
            n.create_identity("mira").unwrap();
            let limits = Some(LimitsPatch { pow_bits: Some(8), ..Default::default() });
            let c = n
                .create_community(CommunityDraft {
                    name: "g".into(),
                    description: "".into(),
                    color: "mint".into(),
                    limits,
                    rules: None,
                    co_owners: None,
                    threshold: None,
                })
                .unwrap();
            let mut inv = hoi_types::Invite::parse(&n.invite(&c.id).unwrap().uri).unwrap();
            inv.peers = vec![format!("{}@203.0.113.5:7843", member.hex())];
            n.join(&inv.to_uri()).unwrap();
        });
        let gate = |members_only| Gate { shared: Arc::new(SharedRef(shared.clone())), blocked: Arc::new(RwLock::new([blocked.0].into())), members_only };
        let allowed = |a: Access| matches!(a, Access::Allow);
        let open = gate(false);
        assert!(allowed(open.on_connect(&request(stranger.0)).await));
        assert!(!allowed(open.on_connect(&request(blocked.0)).await));
        let members = gate(true);
        assert!(allowed(members.on_connect(&request(member.0)).await));
        assert!(!allowed(members.on_connect(&request(stranger.0)).await));
    }

    #[test]
    fn the_block_list_reads_keys_and_skips_the_rest() {
        let dir = std::env::temp_dir().join(format!("hoi-relay-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<6>().unwrap())));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(BLOCKED_FILE);
        assert!(read_blocked(&path).is_empty(), "no file: nobody blocked");
        let key = "ab".repeat(32);
        std::fs::write(&path, format!("# refused\n{key} spammed the relay\nnot a key\n\n{}\n", "CD".repeat(32))).unwrap();
        let b = read_blocked(&path);
        assert_eq!(b.len(), 2);
        assert!(b.contains(&[0xab; 32]) && b.contains(&[0xcd; 32]));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
