//! WebRTC data channels from browsers and phones to native nodes, with no signaling server.
//!
//! A web page can't accept connections, and the only encrypted ones it can open are TLS (which
//! needs a certificate from an authority, so a domain) and WebRTC. WebRTC needs no authority: the
//! browser pins the node's DTLS certificate by its fingerprint. So a native node listens on UDP as
//! an ICE-lite peer, and a route `rtc:<fingerprint>@<addr>` (in pairing links and invites) is all
//! a browser needs: it writes the node's half of the session description itself, the same trick
//! as libp2p's WebRTC Direct. Its own certificate fingerprint travels in its ICE username, so this
//! side checks the browser's certificate too. Nothing central is involved: no STUN, no TURN, no
//! signaling server.
//!
//! The browser picks the ICE username fragment, used for both sides, and the same string as the
//! password: `h1` + mode (`c` one of your paired devices, `p` a member's browser) + the browser's
//! SHA-256 certificate fingerprint in standard base64 without padding (43 characters) + at least
//! eight random characters. One negotiated data channel (id 0) carries the same framed hoi-sync
//! messages as the iroh stream: a four-byte big-endian length, then the message.
//!
//! Every binding request carries the username fragment, so the password, in the clear: ICE checks
//! prove nothing about where a packet came from, and anyone can write a binding request with any
//! source address. A first binding request therefore costs this side one stateless binding
//! request back (a probe, no larger than what came in) and nothing else. The browser, a full ICE
//! agent, answers it; its answer proves it receives at that address, and only then does its next
//! binding request start a session. A probe's transaction id is a MAC of where it went and when,
//! so nothing is kept per probe. DTLS asks for a cookie as well (rtc_crypto).
//!
//! Paired devices still prove themselves with the pairing token in their first `hello`; members'
//! browsers are served only while the relay switch is on (`Node::accepts_members`).

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use hoi_core::{PeerMeta, Role};
use sha2::Sha256;
use str0m::channel::{ChannelConfig, ChannelId};
use str0m::config::{CryptoProvider, DtlsCert, DtlsVersion, Fingerprint};
use str0m::ice::{StunMessage, TransId};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, IceCreds, Input, Output, Rtc};
use tokio::net::UdpSocket;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::task::JoinHandle;

use crate::limit::{self, PerSource};
use crate::vpn::VpnState;
use crate::{HELLO, LinkGuard, Queued, Shared};

/// The port browsers are pointed at unless it's taken.
pub const DEFAULT_PORT: u16 = 7844;
/// Sessions at once, your devices and members' browsers together.
const MAX_SESSIONS: usize = 32;
/// Sessions still setting up (ICE, DTLS, SCTP) at once.
const MAX_OPENING: usize = 8;
/// A session must open its data channel within this.
const OPEN_TIMEOUT: Duration = Duration::from_secs(15);
/// New sessions one source (an IPv4 address, an IPv6 /64) may start per minute.
const NEW_PER_SOURCE: u32 = 10;
/// Probes one source may answer per minute: a browser answers a few per session.
const ANSWERS_PER_SOURCE: u32 = 30;
/// Sources counted per minute; past this, newcomers wait for the next minute.
const SOURCES: usize = 4096;
/// Plain STUN answers: per source and in all, per minute. An answer is at most 52
/// bytes and can't be steered anywhere but the request's source, so spoofed requests make this
/// port send at most about 5 KB a second to anyone.
const STUN_PER_SOURCE: u32 = 20;
const STUN_PER_MINUTE: u32 = 6000;
/// How long an address that answered a probe may start its session. Its next binding request
/// comes within a second.
const PROVEN_FOR: Duration = Duration::from_secs(10);
/// Addresses that answered and haven't started their session yet. Only an address that receives
/// what is sent to it gets in, so spoofed packets can't fill this.
const MAX_PROVEN: usize = 1024;
/// A probe's transaction id is good for this long, and one more.
const PROBE_EPOCH: Duration = Duration::from_secs(5);
/// The priority a probe asks for this address should the browser not know it (RFC 8445 §7.1.1).
const PROBE_PRIORITY: u32 = (110 << 24) | (65535 << 8) | 255;
/// Data channel messages we write; every browser takes at least this much in one message.
const CHUNK: usize = 16 * 1024;
/// What a session may have waiting for the node, in frames and bytes. Frames can be tiny, so
/// the byte budget is what bounds memory; the frame count only stops a flood of empty ones.
const INBOUND_FRAMES: usize = 4096;
const INBOUND_BYTES: usize = 32 * 1024 * 1024;
/// How often network interfaces are looked at again (a laptop changes networks).
const RESCAN: Duration = Duration::from_secs(30);
/// Bytes SCTP may hold unacknowledged per session.
const SCTP_BUFFER: usize = 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct RtcConfig {
    /// UDP port; 0 means [`DEFAULT_PORT`], or any free port if that's taken.
    pub port: u16,
    /// Addresses to advertise as well: a port forwarded on the router, a public address.
    pub advertise: Vec<SocketAddr>,
    /// Bind only these addresses instead of every interface's (tests, containers).
    pub bind: Vec<IpAddr>,
    /// Which networks may be used: every interface, or only a VPN's (and the home network).
    pub policy: crate::vpn::NetPolicy,
}

/// Owns the sockets and sessions. Keep it alive for the node's lifetime.
pub struct RtcHandle {
    task: JoinHandle<()>,
    shared: Arc<Shared>,
    port: u16,
    found: Arc<Found>,
}

/// Addresses learned while running, such as the public side of a port the router forwards:
/// advertised after the operator's own, and replaced whenever they change.
#[derive(Default)]
struct Found {
    addrs: Mutex<Vec<SocketAddr>>,
    changed: Notify,
    /// Look at the interfaces again now: the VPN came or went.
    rescan: Notify,
}

impl RtcHandle {
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Advertises these addresses as well as the configured ones, in place of those set before.
    pub fn set_advertised(&self, addrs: Vec<SocketAddr>) {
        *self.found.addrs.lock().unwrap_or_else(|e| e.into_inner()) = addrs;
        self.found.changed.notify_one();
    }

    /// Looks at the interfaces again now instead of at the next rescan: the host saw the VPN
    /// come or go, and sockets on it open or close at once (the kill switch).
    pub fn rescan(&self) {
        self.found.rescan.notify_one();
    }

    pub async fn shutdown(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
        self.stop();
    }

    fn stop(&self) {
        self.shared.close_all("rtc-");
        self.shared.close_all("client-rtc-");
        self.shared.with(|n| {
            n.set_rtc_routes(Vec::new());
            n.set_signal_relay(false);
        });
    }
}

impl Drop for RtcHandle {
    fn drop(&mut self) {
        self.task.abort();
        self.stop();
    }
}

/// `rtc:<base64url sha-256>@<addr>`, the route browsers use.
pub fn route(fingerprint: &[u8], addr: SocketAddr) -> String {
    format!("rtc:{}@{addr}", hoi_types::uri::b64(fingerprint))
}

struct Datagram {
    local: SocketAddr,
    source: SocketAddr,
    data: Vec<u8>,
}

enum Cmd {
    Out(u64, Queued),
    Close(u64),
}

struct Inbound {
    bytes: Vec<u8>,
    _permit: OwnedSemaphorePermit,
}

struct Session {
    id: u64,
    rtc: Rtc,
    channel: ChannelId,
    /// Both certificates and the username fragment, for the binding (hoi_core::bind).
    binding: hoi_core::bind::RtcChannel,
    role: Role,
    peer: String,
    /// Where the browser's first packet came from: in memory only, so the node knows how far
    /// the routes it tells this browser about may reach (hoi-core `nodes`).
    source: SocketAddr,
    created: Instant,
    open: bool,
    /// Bytes of a frame still arriving.
    rbuf: Vec<u8>,
    outq: VecDeque<Queued>,
    /// How much of the front frame (length prefix included) is written.
    out_off: usize,
    in_tx: Option<mpsc::Sender<Inbound>>,
    in_budget: Arc<Semaphore>,
}

struct Bound {
    socket: Arc<UdpSocket>,
    task: JoinHandle<()>,
}

/// One socket per local address, so every packet's destination address is known (ICE needs it).
struct Sockets {
    by_addr: BTreeMap<SocketAddr, Bound>,
    port: u16,
    tx: mpsc::Sender<Datagram>,
    policy: crate::vpn::NetPolicy,
    /// The VPN as last seen: sockets on its addresses are pinned to it, and no other socket
    /// sends past the home network (crate::vpn::may_send).
    vpn: VpnState,
}

impl Drop for Sockets {
    fn drop(&mut self) {
        for b in self.by_addr.values() {
            b.task.abort();
        }
    }
}

/// The addresses to listen on, as the policy allows them, and the VPN's state they follow.
fn local_ips(config: &RtcConfig) -> (Vec<IpAddr>, VpnState) {
    if !config.bind.is_empty() {
        return (config.bind.clone(), VpnState::Off);
    }
    let state = config.policy.state();
    let mut local: Vec<(IpAddr, String)> = vec![(IpAddr::from([127, 0, 0, 1]), String::new())];
    for i in netdev::get_interfaces() {
        if !i.is_up() || i.is_loopback() {
            continue;
        }
        for v4 in &i.ipv4 {
            let a = v4.addr();
            if !a.is_link_local() && !a.is_unspecified() {
                local.push((IpAddr::V4(a), i.name.clone()));
            }
        }
        for v6 in &i.ipv6 {
            let a = v6.addr();
            // Link-local addresses need a scope id that a route can't carry.
            if (a.segments()[0] & 0xffc0) != 0xfe80 && !a.is_unspecified() && !a.is_loopback() {
                local.push((IpAddr::V6(a), i.name.clone()));
            }
        }
    }
    let ips: BTreeSet<IpAddr> = config.policy.allowed(&state, &local).into_iter().collect();
    (ips.into_iter().collect(), state)
}

/// The routes for these local addresses and what is advertised (the operator's addresses, a
/// port the router forwards): the best local one, then up to two advertised ones, then the rest
/// of the local ones. The few routes a hello or an invite has room for then carry both a way in
/// on the local network and one from the internet, however many interfaces this computer has.
/// A browser offers them all as ICE candidates of one session; the order is their priority.
fn ordered_routes(fingerprint: &[u8], local: impl Iterator<Item = SocketAddr>, advertise: &[SocketAddr]) -> Vec<String> {
    let max = hoi_types::uri::MAX_PAIR_ROUTES;
    let mut addrs: Vec<SocketAddr> = local.filter(|a| !a.ip().is_loopback() && !advertise.contains(a)).collect();
    addrs.sort_by_key(|a| (rank(&a.ip()), *a));
    let rest = addrs.split_off(addrs.len().min(1));
    let mut out: Vec<String> = addrs.into_iter().chain(advertise.iter().take(2).copied()).chain(rest).map(|a| route(fingerprint, a)).collect();
    let mut seen = BTreeSet::new();
    out.retain(|r| seen.insert(r.clone()));
    out.truncate(max);
    out
}

/// Order browsers should try addresses in: the local network first, then public ones.
fn rank(ip: &IpAddr) -> u8 {
    match ip {
        IpAddr::V4(a) if a.is_private() || (a.octets()[0] == 100 && (a.octets()[1] & 0xc0) == 64) => 0,
        IpAddr::V4(_) => 1,
        IpAddr::V6(a) if (a.segments()[0] & 0xfe00) == 0xfc00 => 2,
        IpAddr::V6(_) => 3,
    }
}

impl Sockets {
    /// A socket on `ip`, pinned to `interface` when it is the VPN's.
    fn bind(ip: IpAddr, port: u16, interface: Option<&str>) -> io::Result<UdpSocket> {
        UdpSocket::from_std(crate::vpn::udp_socket(SocketAddr::new(ip, port), interface)?)
    }

    /// Binds what's new, drops what's gone, as the VPN's state now says. Returns whether
    /// anything changed.
    async fn sync(&mut self, ips: &[IpAddr], vpn: VpnState) -> bool {
        self.vpn = vpn;
        let want: BTreeSet<IpAddr> = ips.iter().copied().collect();
        let gone: Vec<SocketAddr> = self.by_addr.keys().filter(|a| !want.contains(&a.ip())).copied().collect();
        let mut changed = !gone.is_empty();
        for a in gone {
            if let Some(b) = self.by_addr.remove(&a) {
                b.task.abort();
            }
        }
        for ip in want {
            if self.by_addr.keys().any(|a| a.ip() == ip) {
                continue;
            }
            let pin = self.policy.vpn.as_deref().filter(|_| self.vpn.addresses().contains(&ip));
            let Ok(sock) = Sockets::bind(ip, self.port, pin) else { continue };
            let Ok(local) = sock.local_addr() else { continue };
            let socket = Arc::new(sock);
            let tx = self.tx.clone();
            let s = socket.clone();
            let task = tokio::spawn(async move {
                let mut buf = vec![0u8; 2048];
                loop {
                    let Ok((n, source)) = s.recv_from(&mut buf).await else {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    };
                    // A full queue drops the packet, as a busy network would.
                    let _ = tx.try_send(Datagram { local, source, data: buf[..n].to_vec() });
                }
            });
            self.by_addr.insert(local, Bound { socket, task });
            changed = true;
        }
        changed
    }

    fn routes(&self, fingerprint: &[u8], advertise: &[SocketAddr]) -> Vec<String> {
        ordered_routes(fingerprint, self.by_addr.keys().copied(), advertise)
    }

    fn send(&self, from: SocketAddr, to: SocketAddr, bytes: &[u8]) {
        if !crate::vpn::may_send(&self.policy, &self.vpn, from.ip(), to.ip()) {
            return;
        }
        if let Some(b) = self.by_addr.get(&from) {
            // UDP: a full send buffer drops the packet; ICE, DTLS and SCTP retransmit.
            let _ = b.socket.try_send_to(bytes, to);
        }
    }
}

pub async fn start_rtc(shared: Arc<Shared>, config: RtcConfig) -> io::Result<RtcHandle> {
    let provider = Arc::new(crate::rtc_crypto::provider());
    let cert = match shared.with(|n| n.rtc_cert()) {
        Some((certificate, private_key)) => DtlsCert { certificate, private_key },
        None => {
            let c = crate::rtc_crypto::certificate().ok_or_else(|| io::Error::other("could not make a WebRTC certificate"))?;
            shared.with(|n| n.set_rtc_cert(c.certificate.clone(), c.private_key.clone())).map_err(|e| io::Error::other(e.to_string()))?;
            c
        }
    };
    let fingerprint = provider.sha256_provider.sha256(&cert.certificate).to_vec();
    let (tx, rx) = mpsc::channel(4096);
    let (ips, vpn) = local_ips(&config);
    let first = *ips.iter().find(|ip| !ip.is_loopback()).or(ips.first()).ok_or_else(|| io::Error::other("no address to listen on"))?;
    let wanted = if config.port == 0 { DEFAULT_PORT } else { config.port };
    let port = match Sockets::bind(first, wanted, None) {
        Ok(s) => s.local_addr()?.port(),
        Err(_) if config.port == 0 => Sockets::bind(first, 0, None)?.local_addr()?.port(),
        Err(e) => return Err(e),
    };
    let mut sockets = Sockets { by_addr: BTreeMap::new(), port, tx, policy: config.policy.clone(), vpn: VpnState::Off };
    sockets.sync(&ips, vpn).await;
    if sockets.by_addr.is_empty() {
        return Err(io::Error::other("could not bind the WebRTC port"));
    }
    shared.with(|n| {
        n.set_rtc_routes(sockets.routes(&fingerprint, &config.advertise));
        n.set_signal_relay(true);
    });
    let found = Arc::new(Found::default());
    let lp = Loop::new(shared.clone(), sockets, cert, provider, fingerprint, config, found.clone())?;
    let task = tokio::spawn(lp.run(rx));
    Ok(RtcHandle { task, shared, port, found })
}

/// A transaction id with these bytes. `TransId` has no constructor for them, but parsing a bare
/// binding indication (a header and nothing else) gives one.
fn trans_id(bytes: &[u8; 12]) -> Option<TransId> {
    let mut b = [0u8; 20];
    b[..8].copy_from_slice(&[0x00, 0x11, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
    b[8..].copy_from_slice(bytes);
    StunMessage::parse(&b).ok().map(|m| m.trans_id())
}

/// The transaction id of a STUN binding request with no USERNAME (RFC 8489): what a browser
/// sends the STUN server it was given, never part of an ICE check.
fn plain_stun_request(b: &[u8]) -> Option<TransId> {
    if b.len() < 20 || b.len() > 512 || b[..2] != [0x00, 0x01] || b[4..8] != [0x21, 0x12, 0xa4, 0x42] {
        return None;
    }
    let len = u16::from_be_bytes([b[2], b[3]]) as usize;
    if len + 20 != b.len() || !len.is_multiple_of(4) {
        return None;
    }
    let mut at = 20;
    while at + 4 <= b.len() {
        let kind = u16::from_be_bytes([b[at], b[at + 1]]);
        let n = u16::from_be_bytes([b[at + 2], b[at + 3]]) as usize;
        // USERNAME or MESSAGE-INTEGRITY: an ICE check, handled as one
        if kind == 0x0006 || kind == 0x0008 {
            return None;
        }
        at += 4 + n.div_ceil(4) * 4;
    }
    trans_id(b[8..20].try_into().ok()?)
}

/// The browser's certificate fingerprint and mode from its ICE username fragment.
fn parse_ufrag(ufrag: &str) -> Option<(Role, Vec<u8>)> {
    if ufrag.len() < 3 + 43 + 8 || ufrag.len() > 256 || !ufrag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/') {
        return None;
    }
    let role = match ufrag.get(..3)? {
        "h1c" => Role::Client,
        "h1p" => Role::Peer,
        _ => return None,
    };
    let fp = base64::engine::general_purpose::STANDARD_NO_PAD.decode(ufrag.get(3..46)?).ok()?;
    (fp.len() == 32).then_some((role, fp))
}

struct Loop {
    shared: Arc<Shared>,
    sockets: Sockets,
    cert: DtlsCert,
    provider: Arc<CryptoProvider>,
    fingerprint: Vec<u8>,
    config: RtcConfig,
    found: Arc<Found>,
    sessions: Vec<Session>,
    next_id: u64,
    /// New sessions per source.
    recent: PerSource<IpAddr>,
    /// Probes answered per source.
    answers: PerSource<IpAddr>,
    /// Plain STUN answered per source, and in all.
    stun: PerSource<IpAddr>,
    stun_all: PerSource<()>,
    /// Keys the transaction ids of probes, so an answer is checked with nothing kept per probe.
    probe_key: [u8; 32],
    /// The ICE tie-breaker probes carry.
    tie_breaker: u64,
    started: Instant,
    /// Addresses that answered a probe, until they start their session or `PROVEN_FOR` passes.
    proven: HashMap<SocketAddr, Instant>,
}

impl Loop {
    fn new(
        shared: Arc<Shared>,
        sockets: Sockets,
        cert: DtlsCert,
        provider: Arc<CryptoProvider>,
        fingerprint: Vec<u8>,
        config: RtcConfig,
        found: Arc<Found>,
    ) -> io::Result<Loop> {
        let probe_key = hoi_crypto::random_bytes().map_err(|e| io::Error::other(e.to_string()))?;
        let tie_breaker = u64::from_le_bytes(hoi_crypto::random_bytes().map_err(|e| io::Error::other(e.to_string()))?);
        Ok(Loop {
            shared,
            sockets,
            cert,
            provider,
            fingerprint,
            config,
            found,
            sessions: Vec::new(),
            next_id: 1,
            recent: PerSource::new(NEW_PER_SOURCE, SOURCES),
            answers: PerSource::new(ANSWERS_PER_SOURCE, SOURCES),
            stun: PerSource::new(STUN_PER_SOURCE, SOURCES),
            stun_all: PerSource::new(STUN_PER_MINUTE, 1),
            probe_key,
            tie_breaker,
            started: Instant::now(),
            proven: HashMap::new(),
        })
    }

    /// The routes for the bound sockets, then the operator's addresses, then those found.
    fn publish_routes(&self) {
        let mut advertise = self.config.advertise.clone();
        for a in self.found.addrs.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            if !advertise.contains(a) {
                advertise.push(*a);
            }
        }
        let routes = self.sockets.routes(&self.fingerprint, &advertise);
        self.shared.with(|n| n.set_rtc_routes(routes));
    }

    async fn run(mut self, mut rx: mpsc::Receiver<Datagram>) {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Cmd>(1024);
        let found = self.found.clone();
        let mut rescan = tokio::time::interval(RESCAN);
        rescan.tick().await;
        loop {
            let next = self.drive(&cmd_tx);
            tokio::select! {
                d = rx.recv() => {
                    let Some(d) = d else { break };
                    self.datagram(d);
                }
                c = cmd_rx.recv() => {
                    let Some(c) = c else { break };
                    self.command(c);
                }
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(next)) => {
                    let now = Instant::now();
                    for s in &mut self.sessions {
                        let _ = s.rtc.handle_input(Input::Timeout(now));
                    }
                }
                _ = rescan.tick() => self.rescan().await,
                _ = found.rescan.notified() => self.rescan().await,
                _ = found.changed.notified() => self.publish_routes(),
            }
        }
    }

    async fn rescan(&mut self) {
        let (ips, vpn) = local_ips(&self.config);
        if self.sockets.sync(&ips, vpn).await {
            self.publish_routes();
        }
    }

    fn datagram(&mut self, d: Datagram) {
        let now = Instant::now();
        // plain STUN: a browser asks where it is seen from, to link with another
        if let Some(id) = plain_stun_request(&d.data) {
            self.answer_stun(d.local, d.source, id, limit::source(d.source.ip()), now);
            return;
        }
        let Ok(rx) = Receive::new(Protocol::Udp, d.source, d.local, &d.data) else { return };
        let input = Input::Receive(now, rx);
        if let Some(s) = self.sessions.iter_mut().find(|s| s.rtc.accepts(&input)) {
            if s.rtc.handle_input(input).is_err() {
                s.rtc.disconnect();
            }
            return;
        }
        let Ok(stun) = StunMessage::parse(&d.data) else { return };
        let source = limit::source(d.source.ip());
        if stun.is_successful_binding_response() {
            if self.answers_probe(d.local, d.source, &d.data[8..20], now) && self.answers.take(source, now) {
                self.prove(d.source, now);
            }
            return;
        }
        // Only a binding request with a well-formed username starts a session.
        if !stun.is_binding_request() {
            return;
        }
        let Some((ours, theirs)) = stun.split_username() else { return };
        if ours != theirs {
            return;
        }
        let Some((role, fp)) = parse_ufrag(ours) else { return };
        if role == Role::Peer && !self.shared.with(|n| n.accepts_members()) {
            return;
        }
        let opening = self.sessions.iter().filter(|s| !s.open).count();
        if self.sessions.len() >= MAX_SESSIONS || opening >= MAX_OPENING {
            return;
        }
        // Until this address has answered a probe, it gets a probe and holds nothing here.
        if self.proven.get(&d.source).is_none_or(|t| now.duration_since(*t) > PROVEN_FOR) {
            if self.recent.left(&source, now) {
                self.probe(d.local, d.source, ours, now);
            }
            return;
        }
        if !self.recent.take(source, now) {
            return;
        }
        self.proven.remove(&d.source);
        let Some(mut session) = self.session(role, ours.to_string(), fp, d.source, now) else { return };
        if session.rtc.handle_input(input).is_ok() {
            self.sessions.push(session);
        }
    }

    /// Tells `to` the address its request came from (a STUN binding response, RFC 8489), while
    /// this node passes signals between browsers and within the limits above.
    fn answer_stun(&mut self, local: SocketAddr, to: SocketAddr, id: TransId, source: IpAddr, now: Instant) {
        if !self.shared.with(|n| n.passes_signals()) || !self.stun_all.left(&(), now) || !self.stun.take(source, now) {
            return;
        }
        self.stun_all.take((), now);
        let mut buf = [0u8; 128];
        let hmac = |key: &[u8], parts: &[&[u8]]| self.provider.sha1_hmac_provider.sha1_hmac(key, parts);
        if let Ok(n) = StunMessage::binding_reply(id, to).to_bytes(None, &mut buf, hmac) {
            self.sockets.send(local, to, &buf[..n]);
        }
    }

    /// The probe's transaction id for `local` → `to` in time step `epoch`.
    fn probe_mac(&self, local: SocketAddr, to: SocketAddr, epoch: u64) -> Hmac<Sha256> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.probe_key).expect("HMAC takes a key of any size");
        mac.update(&epoch.to_be_bytes());
        for a in [local, to] {
            let ip = match a.ip() {
                IpAddr::V4(v4) => v4.to_ipv6_mapped(),
                IpAddr::V6(v6) => v6,
            };
            mac.update(&ip.octets());
            mac.update(&a.port().to_be_bytes());
        }
        mac
    }

    fn epoch(&self, now: Instant) -> u64 {
        (now.saturating_duration_since(self.started).as_millis() / PROBE_EPOCH.as_millis()) as u64
    }

    /// Whether `id` is the transaction id of a probe sent lately from `local` to `from`.
    fn answers_probe(&self, local: SocketAddr, from: SocketAddr, id: &[u8], now: Instant) -> bool {
        let epoch = self.epoch(now);
        (0..=epoch.min(1)).any(|age| self.probe_mac(local, from, epoch - age).verify_truncated_left(id).is_ok())
    }

    /// Sends the browser at `to` a binding request of our own, from the address it wrote to. The
    /// browser checks it with the password both sides use (the username fragment) and answers.
    fn probe(&self, local: SocketAddr, to: SocketAddr, ufrag: &str, now: Instant) {
        let id: [u8; 12] = self.probe_mac(local, to, self.epoch(now)).finalize().into_bytes()[..12].try_into().expect("12 of 32 bytes");
        let Some(id) = trans_id(&id) else { return };
        let username = format!("{ufrag}:{ufrag}");
        let request = StunMessage::binding_request(&username, id, false, self.tie_breaker, PROBE_PRIORITY, false);
        let mut buf = [0u8; 1024];
        let hmac = |key: &[u8], parts: &[&[u8]]| self.provider.sha1_hmac_provider.sha1_hmac(key, parts);
        if let Ok(n) = request.to_bytes(Some(ufrag.as_bytes()), &mut buf, hmac) {
            self.sockets.send(local, to, &buf[..n]);
        }
    }

    /// Remembers that `addr` answered. When the table is full, the oldest answer gives way: to
    /// push out an answer before its session starts, a flood needs that many real addresses.
    fn prove(&mut self, addr: SocketAddr, now: Instant) {
        if self.proven.len() >= MAX_PROVEN && !self.proven.contains_key(&addr) {
            self.proven.retain(|_, t| now.duration_since(*t) <= PROVEN_FOR);
            if self.proven.len() >= MAX_PROVEN
                && let Some(oldest) = self.proven.iter().min_by_key(|(_, t)| **t).map(|(a, _)| *a)
            {
                self.proven.remove(&oldest);
            }
        }
        self.proven.insert(addr, now);
    }

    fn session(&mut self, role: Role, ufrag: String, fp: Vec<u8>, source: SocketAddr, now: Instant) -> Option<Session> {
        let mut rtc = Rtc::builder()
            .set_ice_lite(true)
            .set_dtls_cert(self.cert.clone())
            .set_crypto_provider(self.provider.clone())
            .set_dtls_version(DtlsVersion::Auto)
            .set_sctp_max_buffered_amount(SCTP_BUFFER)
            .build(now);
        for addr in self.sockets.by_addr.keys() {
            rtc.add_local_candidate(Candidate::host(*addr, "udp").ok()?)?;
        }
        let channel;
        {
            let mut api = rtc.direct_api();
            api.set_ice_controlling(false);
            api.set_local_ice_credentials(IceCreds { ufrag: ufrag.clone(), pass: ufrag.clone() });
            api.set_remote_ice_credentials(IceCreds { ufrag: ufrag.clone(), pass: ufrag.clone() });
            api.set_remote_fingerprint(Fingerprint { hash_func: "sha-256".into(), bytes: fp.clone() });
            api.start_dtls(false).ok()?;
            api.start_sctp(false);
            channel = api.create_data_channel(ChannelConfig { label: "hoi".into(), negotiated: Some(0), ..Default::default() });
        }
        let id = self.next_id;
        self.next_id += 1;
        let peer = match role {
            Role::Client => self.shared.peer_id("client-rtc"),
            _ => self.shared.peer_id("rtc"),
        };
        let binding = hoi_core::bind::RtcChannel { node: self.fingerprint.clone(), browser: fp, ufrag };
        Some(Session {
            id,
            rtc,
            channel,
            binding,
            role,
            peer,
            source,
            created: now,
            open: false,
            rbuf: Vec::new(),
            outq: VecDeque::new(),
            out_off: 0,
            in_tx: None,
            in_budget: Arc::new(Semaphore::new(INBOUND_BYTES)),
        })
    }

    fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Out(id, q) => {
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
                    s.outq.push_back(q);
                    if flush(s).is_err() {
                        s.rtc.disconnect();
                    }
                }
            }
            Cmd::Close(id) => {
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
                    s.rtc.disconnect();
                }
            }
        }
    }

    /// Runs every session until it waits, sends what it produced, and returns when the next
    /// timer is due. Sessions that died or never opened are dropped.
    fn drive(&mut self, cmd_tx: &mpsc::Sender<Cmd>) -> Instant {
        let now = Instant::now();
        let mut next = now + Duration::from_secs(1);
        for s in &mut self.sessions {
            loop {
                if !s.rtc.is_alive() {
                    break;
                }
                match s.rtc.poll_output() {
                    Ok(Output::Timeout(t)) => {
                        next = next.min(t);
                        break;
                    }
                    Ok(Output::Transmit(t)) => self.sockets.send(t.source, t.destination, &t.contents),
                    Ok(Output::Event(e)) => {
                        if event(s, e, &self.shared, cmd_tx).is_err() {
                            s.rtc.disconnect();
                        }
                    }
                    Err(_) => s.rtc.disconnect(),
                }
            }
            if !s.open && now.duration_since(s.created) > OPEN_TIMEOUT {
                s.rtc.disconnect();
            }
        }
        // Dropping a session drops its inbound queue, which ends its link task.
        self.sessions.retain(|s| s.rtc.is_alive());
        next.max(now + Duration::from_millis(1))
    }
}

fn event(s: &mut Session, e: Event, shared: &Arc<Shared>, cmd_tx: &mpsc::Sender<Cmd>) -> Result<(), ()> {
    match e {
        Event::ChannelOpen(id, _) if id == s.channel && !s.open => {
            s.open = true;
            if let Some(mut ch) = s.rtc.channel(s.channel) {
                ch.set_buffered_amount_low_threshold(SCTP_BUFFER / 4);
            }
            let (in_tx, in_rx) = mpsc::channel(INBOUND_FRAMES);
            s.in_tx = Some(in_tx);
            tokio::spawn(link(shared.clone(), s.peer.clone(), s.role, s.id, s.source, s.binding.clone(), in_rx, cmd_tx.clone()));
        }
        Event::ChannelData(d) if d.id == s.channel => {
            s.rbuf.extend_from_slice(&d.data);
            for frame in frames(&mut s.rbuf)? {
                let permit = s.in_budget.clone().try_acquire_many_owned(frame.len() as u32).map_err(|_| ())?;
                let tx = s.in_tx.as_ref().ok_or(())?;
                tx.try_send(Inbound { bytes: frame, _permit: permit }).map_err(|_| ())?;
            }
        }
        Event::ChannelBufferedAmountLow(id) if id == s.channel => flush(s)?,
        Event::ChannelClose(id) if id == s.channel => return Err(()),
        Event::IceConnectionStateChange(IceConnectionState::Disconnected) => return Err(()),
        _ => {}
    }
    Ok(())
}

/// Complete frames from the front of `buf`. A length over the message cap ends the session
/// before any of it is buffered.
fn frames(buf: &mut Vec<u8>) -> Result<Vec<Vec<u8>>, ()> {
    let mut out = Vec::new();
    let mut pos = 0;
    while buf.len() - pos >= 4 {
        let len = u32::from_be_bytes(buf[pos..pos + 4].try_into().expect("4 bytes")) as usize;
        if len == 0 || len > hoi_sync::MAX_MESSAGE {
            return Err(());
        }
        if buf.len() - pos - 4 < len {
            break;
        }
        out.push(buf[pos + 4..pos + 4 + len].to_vec());
        pos += 4 + len;
    }
    buf.drain(..pos);
    Ok(out)
}

/// Writes queued frames while SCTP takes them; the rest waits for "buffered amount low".
fn flush(s: &mut Session) -> Result<(), ()> {
    loop {
        let Some(q) = s.outq.front() else { return Ok(()) };
        let total = 4 + q.bytes.len();
        let end = (s.out_off + CHUNK).min(total);
        let prefix = (q.bytes.len() as u32).to_be_bytes();
        let mut chunk = Vec::with_capacity(end - s.out_off);
        if s.out_off < 4 {
            chunk.extend_from_slice(&prefix[s.out_off..end.min(4)]);
        }
        if end > 4 {
            chunk.extend_from_slice(&q.bytes[s.out_off.max(4) - 4..end - 4]);
        }
        let mut ch = s.rtc.channel(s.channel).ok_or(())?;
        match ch.write(true, &chunk) {
            Ok(true) => {
                s.out_off = end;
                if end == total {
                    s.outq.pop_front();
                    s.out_off = 0;
                }
            }
            Ok(false) => return Ok(()),
            Err(_) => return Err(()),
        }
    }
}

/// One open session as a link of the node: hello first, then everything both ways.
#[allow(clippy::too_many_arguments)]
async fn link(
    shared: Arc<Shared>,
    peer: String,
    role: Role,
    id: u64,
    source: SocketAddr,
    binding: hoi_core::bind::RtcChannel,
    mut inbound: mpsc::Receiver<Inbound>,
    cmd: mpsc::Sender<Cmd>,
) {
    let (mut rx, mut cancelled) = shared.register(&peer);
    let guard = LinkGuard { shared: shared.clone(), peer: peer.clone() };
    if shared
        .with_network(|n| {
            n.net_connect(&peer, role);
            n.set_peer_meta(
                &peer,
                PeerMeta { transport: "webrtc".into(), incoming: true, addr: Some(source.to_string()), rtc: Some(binding), ..Default::default() },
            );
        })
        .is_none()
    {
        let _ = cmd.send(Cmd::Close(id)).await;
        return;
    }
    let mut first = true;
    let hello_by = tokio::time::Instant::now() + HELLO;
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            out = rx.recv() => {
                let Some(q) = out else { break };
                if cmd.send(Cmd::Out(id, q)).await.is_err() { break; }
            }
            got = inbound.recv() => {
                let Some(m) = got else { break };
                if first && !matches!(hoi_sync::Msg::decode(&m.bytes), Ok(hoi_sync::Msg::Hello { .. })) { break; }
                first = false;
                if !shared.receive(&peer, &m.bytes).await.unwrap_or(false) { break; }
            }
            _ = tokio::time::sleep_until(hello_by), if first => break,
            _ = tick.tick() => {
                if shared.with(|n| n.sync_stalled(&peer)) { break; }
            }
            _ = cancelled.changed() => break,
        }
    }
    // Let the last messages (a `bye`) go out before the session closes.
    while let Ok(q) = rx.try_recv() {
        let _ = cmd.try_send(Cmd::Out(id, q));
    }
    let _ = cmd.send(Cmd::Close(id)).await;
    drop(guard);
}

#[cfg(test)]
mod tests {
    //! A str0m client plays the browser: same username trick, same SDP-free setup.
    use super::*;
    use hoi_core::Node;
    use hoi_core::dto::{CommunityDraft, LimitsPatch, MediaDraft, PostDraft};

    fn node(platform: &str) -> Arc<Shared> {
        let mut n = Node::open(Box::new(hoi_store::Ephemeral), platform, "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
        n.set_identity_bits(8);
        Shared::new(n, Box::new(|_| {}))
    }

    #[tokio::test]
    async fn the_port_tells_a_browser_where_it_is_seen_from_while_it_relays() {
        let home = node("desktop");
        home.with(|n| n.set_settings(&hoi_core::dto::SettingsPatch { relay: Some(true), ..Default::default() }).unwrap());
        let rtc = start_rtc(home.clone(), RtcConfig { port: 0, bind: vec!["127.0.0.1".parse().unwrap()], ..Default::default() }).await.unwrap();
        let server = SocketAddr::from(([127, 0, 0, 1], rtc.port()));
        assert!(home.with(|n| n.passes_signals()));
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        // a plain binding request, as a browser sends its STUN server: a header and nothing else
        let mut request = vec![0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xa4, 0x42];
        request.extend_from_slice(&[7; 12]);
        let ask = || async {
            sock.send_to(&request, server).await.unwrap();
            let mut buf = [0u8; 256];
            let Ok(Ok((n, _))) = tokio::time::timeout(Duration::from_millis(500), sock.recv_from(&mut buf)).await else { return None };
            let b = &buf[..n];
            // a binding success response with the same transaction id, and XOR-MAPPED-ADDRESS
            assert_eq!((&b[..2], &b[8..20]), (&[0x01, 0x01][..], &[7u8; 12][..]));
            assert!(n <= 52, "{n} bytes");
            let mut at = 20;
            while at + 4 <= n {
                let (kind, len) = (u16::from_be_bytes([b[at], b[at + 1]]), u16::from_be_bytes([b[at + 2], b[at + 3]]) as usize);
                if kind == 0x0020 && b[at + 5] == 1 {
                    let port = u16::from_be_bytes([b[at + 6], b[at + 7]]) ^ 0x2112;
                    let ip: [u8; 4] = std::array::from_fn(|i| b[at + 8 + i] ^ [0x21, 0x12, 0xa4, 0x42][i]);
                    return Some(SocketAddr::from((ip, port)));
                }
                at += 4 + len.div_ceil(4) * 4;
            }
            None
        };
        assert_eq!(ask().await, Some(sock.local_addr().unwrap()));
        // not relaying: no answer
        home.with(|n| n.set_settings(&hoi_core::dto::SettingsPatch { relay: Some(false), ..Default::default() }).unwrap());
        assert_eq!(ask().await, None);
        rtc.shutdown().await;
        assert!(!home.with(|n| n.passes_signals()));
    }

    #[tokio::test]
    async fn without_its_vpn_the_listener_offers_no_address_at_all() {
        // Only through a VPN that isn't there, the home network not allowed: the kill switch.
        let home = node("desktop");
        let policy = crate::vpn::NetPolicy { vpn: Some("hoi-test-missing0".into()), home_network: false };
        let rtc = start_rtc(home.clone(), RtcConfig { port: 0, policy, ..Default::default() }).await.unwrap();
        assert!(home.with(|n| n.net_status().rtc).is_empty(), "no route on any network: only this computer's loopback is bound");
        rtc.rescan();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(home.with(|n| n.net_status().rtc).is_empty());
        rtc.shutdown().await;
        // The home network allowed: its addresses only, never an internet one outside the VPN.
        let policy = crate::vpn::NetPolicy { vpn: Some("hoi-test-missing0".into()), home_network: true };
        let rtc = start_rtc(home.clone(), RtcConfig { port: 0, policy, ..Default::default() }).await.unwrap();
        for route in home.with(|n| n.net_status().rtc) {
            let ip = route.rsplit_once('@').unwrap().1.parse::<SocketAddr>().unwrap().ip();
            assert!(crate::vpn::home_network(ip), "{route}");
        }
        rtc.shutdown().await;
    }

    /// Dials `route` like a browser would and runs the session as `role` for `shared`, until
    /// the returned handle is dropped.
    async fn browser(shared: Arc<Shared>, route: &str, mode: char, peer: &str) -> JoinHandle<()> {
        let (fp_b64, addr) = route.strip_prefix("rtc:").unwrap().split_once('@').unwrap();
        let server: SocketAddr = addr.parse().unwrap();
        let server_fp = hoi_types::uri::unb64(fp_b64).unwrap();
        let provider = Arc::new(crate::rtc_crypto::provider());
        let cert = crate::rtc_crypto::certificate().unwrap();
        let mine = provider.sha256_provider.sha256(&cert.certificate);
        let ufrag = format!("h1{mode}{}{}", base64::engine::general_purpose::STANDARD_NO_PAD.encode(mine), "abcdefgh");
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local = sock.local_addr().unwrap();
        let now = Instant::now();
        let mut rtc = Rtc::builder().set_dtls_cert(cert).set_crypto_provider(provider).build(now);
        rtc.add_local_candidate(Candidate::host(local, "udp").unwrap()).unwrap();
        rtc.add_remote_candidate(Candidate::host(server, "udp").unwrap());
        let channel;
        {
            let mut api = rtc.direct_api();
            api.set_ice_controlling(true);
            api.set_local_ice_credentials(IceCreds { ufrag: ufrag.clone(), pass: ufrag.clone() });
            api.set_remote_ice_credentials(IceCreds { ufrag: ufrag.clone(), pass: ufrag.clone() });
            api.set_remote_fingerprint(Fingerprint { hash_func: "sha-256".into(), bytes: server_fp });
            api.start_dtls(true).unwrap();
            api.start_sctp(true);
            channel = api.create_data_channel(ChannelConfig { label: "hoi".into(), negotiated: Some(0), ..Default::default() });
        }
        let peer = peer.to_string();
        let role = if mode == 'c' { Role::Home } else { Role::Peer };
        tokio::spawn(async move {
            let (mut rx, _cancel) = shared.register(&peer);
            let mut buf = vec![0u8; 2048];
            let mut rbuf = Vec::new();
            let mut outq: VecDeque<Queued> = VecDeque::new();
            let mut off = 0usize;
            let mut open = false;
            loop {
                let mut next = Instant::now() + Duration::from_millis(50);
                loop {
                    match rtc.poll_output().unwrap() {
                        Output::Timeout(t) => {
                            next = next.min(t);
                            break;
                        }
                        Output::Transmit(t) => {
                            let _ = sock.send_to(&t.contents, t.destination).await;
                        }
                        Output::Event(Event::ChannelOpen(..)) => {
                            open = true;
                            shared.with(|n| n.net_connect(&peer, role));
                        }
                        Output::Event(Event::ChannelData(d)) => {
                            rbuf.extend_from_slice(&d.data);
                            for f in frames(&mut rbuf).unwrap() {
                                shared.with(|n| n.net_receive(&peer, &f));
                            }
                        }
                        Output::Event(_) => {}
                    }
                }
                if open {
                    while let Ok(q) = rx.try_recv() {
                        outq.push_back(q);
                    }
                    // the same chunking as the server
                    while let Some(q) = outq.front() {
                        let total = 4 + q.bytes.len();
                        let end = (off + CHUNK).min(total);
                        let prefix = (q.bytes.len() as u32).to_be_bytes();
                        let mut chunk = Vec::new();
                        if off < 4 {
                            chunk.extend_from_slice(&prefix[off..end.min(4)]);
                        }
                        if end > 4 {
                            chunk.extend_from_slice(&q.bytes[off.max(4) - 4..end - 4]);
                        }
                        if !rtc.channel(channel).unwrap().write(true, &chunk).unwrap() {
                            break;
                        }
                        off = end;
                        if end == total {
                            outq.pop_front();
                            off = 0;
                        }
                    }
                }
                let wait = next.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
                match tokio::time::timeout(wait, sock.recv_from(&mut buf)).await {
                    Ok(Ok((n, from))) => {
                        let rx = Receive::new(Protocol::Udp, from, local, &buf[..n]).unwrap();
                        rtc.handle_input(Input::Receive(Instant::now(), rx)).unwrap();
                    }
                    _ => rtc.handle_input(Input::Timeout(Instant::now())).unwrap(),
                }
            }
        })
    }

    fn community(n: &mut Node, name: &str) -> String {
        n.create_community(CommunityDraft {
            name: name.into(),
            description: "".into(),
            color: "mint".into(),
            limits: Some(LimitsPatch { pow_bits: Some(8), ..Default::default() }),
            rules: None,
            co_owners: None,
            threshold: None,
        })
        .unwrap()
        .id
    }

    async fn until(what: &str, mut f: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while !f() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out: {what}"));
    }

    #[test]
    fn usernames_carry_the_mode_and_the_browser_fingerprint() {
        let fp = base64::engine::general_purpose::STANDARD_NO_PAD.encode([7u8; 32]);
        assert_eq!(parse_ufrag(&format!("h1c{fp}12345678")).map(|(r, f)| (r, f.len())), Some((Role::Client, 32)));
        assert_eq!(parse_ufrag(&format!("h1p{fp}12345678")).map(|(r, _)| r), Some(Role::Peer));
        assert!(parse_ufrag(&format!("h1x{fp}12345678")).is_none());
        assert!(parse_ufrag(&format!("h1c{fp}1234")).is_none(), "too little randomness");
        assert!(parse_ufrag(&format!("h1c{fp}1234-678")).is_none(), "not an ICE character");
        let mut buf = vec![0, 0, 0, 2, 9, 9, 0, 0];
        assert_eq!(frames(&mut buf).unwrap(), vec![vec![9, 9]]);
        assert_eq!(buf, vec![0, 0]);
        let mut huge = ((hoi_sync::MAX_MESSAGE + 1) as u32).to_be_bytes().to_vec();
        assert!(frames(&mut huge).is_err());
    }

    #[test]
    fn advertised_addresses_come_right_after_the_best_local_one() {
        let a = |s: &str| s.parse::<SocketAddr>().unwrap();
        let local = [a("192.168.1.20:7844"), a("10.0.0.5:7844"), a("[2001:db8::20]:7844"), a("127.0.0.1:7844"), a("172.17.0.1:7844")];
        let routes = ordered_routes(&[1; 32], local.into_iter(), &[a("93.184.216.34:7844")]);
        let addrs: Vec<&str> = routes.iter().map(|r| r.split_once('@').unwrap().1).collect();
        assert_eq!(addrs.len(), hoi_types::uri::MAX_PAIR_ROUTES);
        assert_eq!(addrs[1], "93.184.216.34:7844", "{addrs:?}");
        assert!(!addrs.contains(&"127.0.0.1:7844"));
        assert_eq!(ordered_routes(&[1; 32], std::iter::empty(), &[a("93.184.216.34:7844")]).len(), 1);
    }

    #[tokio::test]
    async fn a_forwarded_port_joins_the_routes_and_leaves_them() {
        let home = node("node");
        let rtc = start_rtc(home.clone(), RtcConfig { port: 0, bind: vec!["127.0.0.1".parse().unwrap()], ..Default::default() }).await.unwrap();
        assert!(home.with(|n| n.net_status().rtc.is_empty()), "loopback isn't advertised");
        rtc.set_advertised(vec![SocketAddr::from(([93, 184, 216, 34], 7844))]);
        until("the router's address is a route", || home.with(|n| n.net_status().rtc.iter().any(|r| r.ends_with("@93.184.216.34:7844")))).await;
        rtc.set_advertised(Vec::new());
        until("and gone again", || home.with(|n| n.net_status().rtc.is_empty())).await;
        rtc.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn a_phone_pairs_and_a_members_browser_syncs_over_webrtc() {
        let home = node("desktop");
        let cid = home.with(|n| {
            n.create_identity("mira").unwrap();
            community(n, "rtc")
        });
        // Noise doesn't compress: a picture of about a megabyte, many data channel messages.
        let picture = {
            let mut img = image::RgbImage::new(600, 600);
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
        };
        let pid = home.with(|n| {
            n.create_post(&PostDraft {
                community: cid.clone(),
                title: "Over WebRTC".into(),
                text: "no certificate authority involved".into(),
                link: None,
                media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&picture), alt: None }]),
            })
            .unwrap()
            .id
        });
        let _ws = crate::serve(home.clone(), "127.0.0.1:0".parse().unwrap(), &[]).await.unwrap();
        let rtc = start_rtc(home.clone(), RtcConfig { port: 0, bind: vec!["127.0.0.1".parse().unwrap()], ..Default::default() }).await.unwrap();
        let st = home.with(|n| n.net_status());
        assert_eq!(st.rtc.len(), 0, "loopback isn't advertised");
        // Tests bind loopback only, so take the route straight from the port.
        let fp = home.with(|n| n.rtc_cert()).map(|(c, _)| crate::rtc_crypto::provider().sha256_provider.sha256(&c).to_vec()).unwrap();
        let route = route(&fp, SocketAddr::from(([127, 0, 0, 1], rtc.port())));
        let pairing = st.serving.unwrap().pairing;

        // Your phone: paired with the token, over WebRTC only.
        let phone = node("web");
        phone.with(|n| {
            n.create_identity("lena").unwrap();
            n.join(&n_invite(&home, &cid)).unwrap();
            n.pair(&pairing).unwrap();
        });
        let ph = browser(phone.clone(), &route, 'c', "home").await;
        until("the phone caught up with the picture", || phone.with(|n| n.post(&pid).is_ok_and(|p| !p.media_missing()))).await;
        assert_eq!(home.with(|n| n.net_status().serving.unwrap().clients), 1);
        let t = home.with(|n| n.traffic());
        let c = t.connections.iter().find(|c| c.role == "client").expect("the phone's connection");
        assert_eq!(c.transport, "webrtc");
        assert!(c.encryption.contains("DTLS"));
        assert!(t.media_out > 500_000, "{t:?}");
        // A reply goes back live.
        phone.with(|n| n.reply(&pid, "through the data channel").unwrap());
        until("the reply arrived", || home.with(|n| n.post(&pid).unwrap().comments.len() == 1)).await;
        drop(ph);

        // A member's browser, unpaired: served while relaying is on.
        let member = node("web");
        member.with(|n| {
            n.create_identity("kim").unwrap();
            n.join(&n_invite(&home, &cid)).unwrap();
        });
        let mb = browser(member.clone(), &route, 'p', "rtc-home").await;
        until("the member's browser caught up", || member.with(|n| n.post(&pid).is_ok())).await;
        drop(mb);

        // Relaying off: members' browsers can't start a session; your phone still can.
        home.with(|n| n.set_settings(&hoi_core::dto::SettingsPatch { relay: Some(false), ..Default::default() }).unwrap());
        let late = node("web");
        late.with(|n| {
            n.create_identity("noa").unwrap();
            n.join(&n_invite(&home, &cid)).unwrap();
        });
        let lb = browser(late.clone(), &route, 'p', "rtc-home").await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(late.with(|n| n.post(&pid).is_err()), "turned away");
        drop(lb);
        rtc.shutdown().await;
    }

    fn n_invite(home: &Arc<Shared>, cid: &str) -> String {
        home.with(|n| n.invite(cid).unwrap().uri)
    }

    /// A binding request as a browser sends it, nominating, with its username fragment `ufrag`.
    fn binding_request(ufrag: &str, provider: &CryptoProvider) -> Vec<u8> {
        let username = format!("{ufrag}:{ufrag}");
        let mut buf = [0u8; 1024];
        let msg = StunMessage::binding_request(&username, TransId::new(), true, 7, PROBE_PRIORITY, true);
        let n = msg.to_bytes(Some(ufrag.as_bytes()), &mut buf, |k, p| provider.sha1_hmac_provider.sha1_hmac(k, p)).unwrap();
        buf[..n].to_vec()
    }

    /// A different well-formed username fragment for each `i`.
    fn some_ufrag(i: usize) -> String {
        format!("h1c{}{i:08}", base64::engine::general_purpose::STANDARD_NO_PAD.encode([i as u8; 32]))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn spoofed_binding_requests_hold_nothing_and_a_browser_still_gets_in() {
        let home = node("desktop");
        let cid = home.with(|n| {
            n.create_identity("mira").unwrap();
            community(n, "spoofed")
        });
        let pid = home.with(|n| {
            n.create_post(&PostDraft { community: cid.clone(), title: "Still here".into(), text: "under a flood".into(), link: None, media: None }).unwrap().id
        });
        let provider = Arc::new(crate::rtc_crypto::provider());

        // In the listener's own loop: a thousand first packets from addresses that never answer,
        // and forged answers, leave no session, no proof and no count behind.
        let (tx, _rx) = mpsc::channel(16);
        let mut sockets = Sockets { by_addr: BTreeMap::new(), port: 0, tx, policy: Default::default(), vpn: VpnState::Off };
        sockets.sync(&["127.0.0.1".parse().unwrap()], VpnState::Off).await;
        let local = *sockets.by_addr.keys().next().unwrap();
        let cert = crate::rtc_crypto::certificate().unwrap();
        let mut lp = Loop::new(home.clone(), sockets, cert, provider.clone(), vec![0; 32], RtcConfig::default(), Arc::default()).unwrap();
        let mut buf = [0u8; 256];
        for i in 0..1000 {
            let source = SocketAddr::from(([203, 0, 113, (i % 250) as u8], 1024 + i as u16));
            lp.datagram(Datagram { local, source, data: binding_request(&some_ufrag(i), &provider) });
            let n = StunMessage::binding_reply(TransId::new(), source)
                .to_bytes(Some(b"guess"), &mut buf, |k, p| provider.sha1_hmac_provider.sha1_hmac(k, p))
                .unwrap();
            lp.datagram(Datagram { local, source, data: buf[..n].to_vec() });
        }
        assert!(lp.sessions.is_empty(), "no session for an address that never answered");
        assert!(lp.proven.is_empty(), "a guessed transaction id proves nothing");
        assert!(lp.recent.sources() == 0 && lp.answers.sources() == 0, "spoofed sources aren't counted");
        drop(lp);

        // Over the network: sockets that keep sending first packets and never answer, while a
        // browser connects. Before, ten of them filled the opening slots and used up this
        // address's allowance for a minute.
        let rtc = start_rtc(home.clone(), RtcConfig { port: 0, bind: vec!["127.0.0.1".parse().unwrap()], ..Default::default() }).await.unwrap();
        let server = SocketAddr::from(([127, 0, 0, 1], rtc.port()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut flood = Vec::new();
        for i in 0..32 {
            let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let packet = binding_request(&some_ufrag(i), &provider);
            let stop = stop.clone();
            flood.push(tokio::spawn(async move {
                let (mut probes, mut answers) = (0, 0);
                let mut buf = vec![0u8; 2048];
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = sock.send_to(&packet, server).await;
                    while let Ok((n, _)) = sock.try_recv_from(&mut buf) {
                        match StunMessage::parse(&buf[..n]) {
                            Ok(m) if m.is_binding_request() => probes += 1,
                            _ => answers += 1,
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                (probes, answers)
            }));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let fp = home.with(|n| n.rtc_cert()).map(|(c, _)| provider.sha256_provider.sha256(&c).to_vec()).unwrap();
        let member = node("web");
        member.with(|n| {
            n.create_identity("kim").unwrap();
            n.join(&n_invite(&home, &cid)).unwrap();
        });
        let mb = browser(member.clone(), &route(&fp, server), 'p', "rtc-home").await;
        until("the browser caught up during the flood", || member.with(|n| n.post(&pid).is_ok())).await;
        mb.abort();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for f in flood {
            let (probes, answers) = f.await.unwrap();
            assert!(probes > 0 && answers == 0, "a probe each, never an answer: {probes} {answers}");
        }
        rtc.shutdown().await;
    }

    trait MediaMissing {
        fn media_missing(&self) -> bool;
    }
    impl MediaMissing for hoi_core::dto::PostView {
        fn media_missing(&self) -> bool {
            self.summary.media.iter().any(|m| !m.available)
        }
    }
}
