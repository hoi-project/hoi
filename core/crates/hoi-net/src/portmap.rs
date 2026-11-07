//! Asking the router in front of this computer to forward the node's UDP ports, so a public
//! node at home can be reached from the internet (`hoi-node --public-relay`, `--port-mapping`,
//! the desktop's *Be a public node*). Three protocols, in this order: PCP (RFC 6887) and
//! NAT-PMP (RFC 6886), each one UDP request to the default gateway's port 5351, then UPnP IGD
//! (an SSDP search on the local network, then SOAP calls to the router that answered). Only the
//! router is asked, and it says which public address it forwards from: no outside service (no
//! STUN) learns that this node exists, and the project runs nothing for it.
//!
//! Through a VPN ([`Via::Interface`]) only the VPN's own gateway is asked, and only over PCP and
//! NAT-PMP: an SSDP search would find the home router on the local network and publish the
//! home's address, which is what the VPN is there to hide.
//!
//! A forward is asked for two hours and renewed after one, which also notices a new public
//! address; the node removes its forwards when it stops. A router that says no, or whose own
//! address is not public (carrier-grade NAT, or a second router in front of it), leaves nothing
//! to announce, and the host hears why ([`Status`]).
//!
//! What a router sends back is read like anything else from the network: answers must come
//! from the gateway's address and match the request; a UPnP description is fetched only from the
//! gateway that answered the search, over plain HTTP to that same host, and every answer is
//! bounded.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::public_ip;

/// PCP and NAT-PMP servers listen on this port of the gateway.
const PCP_PORT: u16 = 5351;
/// Where SSDP searches go.
const SSDP: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900));
/// Seconds a forward is asked for; it is renewed halfway.
const LIFETIME: u32 = 7200;
/// What the router's port forwarding page shows for these entries.
const DESCRIPTION: &str = "hoi-node";
/// After a round in which a port could not be forwarded, the next round comes this soon.
const RETRY: Duration = Duration::from_secs(5 * 60);
/// Renewals come at least this often, and at most this often, whatever lifetime was granted.
const RENEW_MAX: Duration = Duration::from_secs(3600);
const RENEW_MIN: Duration = Duration::from_secs(60);
/// A UDP request is sent up to three times and waits this long after each (RFC 6886 §3.1,
/// shortened: a router on the local network answers within milliseconds).
const UDP_WAITS_MS: [u64; 3] = [250, 500, 1000];
/// How long SSDP answers are collected.
const SSDP_WAIT: Duration = Duration::from_secs(2);
/// A UPnP request must be answered within this.
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
/// A router's device description is a few kilobytes, a SOAP answer less.
const MAX_HTTP_BYTES: usize = 64 * 1024;
/// Removing the forwards when the node stops gets this long, all of them together.
const REMOVE_TIMEOUT: Duration = Duration::from_secs(3);

/// The protocol a router forwards a port with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Pcp,
    NatPmp,
    Upnp,
}

impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Method::Pcp => "PCP",
            Method::NatPmp => "NAT-PMP",
            Method::Upnp => "UPnP",
        }
    }
}

/// One port the router forwards: from `external` on its public side to `internal` here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Forward {
    pub method: Method,
    pub internal: u16,
    pub external: SocketAddrV4,
    /// Seconds the router keeps it; 0 from a UPnP router that keeps only permanent forwards.
    pub lifetime: u32,
}

impl Forward {
    /// Whether visitors from other networks can use it: the router's own address is public.
    pub fn reachable(&self) -> bool {
        public_ip(IpAddr::V4(*self.external.ip()))
    }
}

/// Which router to ask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    /// The router of the IPv4 default route, over PCP, NAT-PMP or UPnP.
    DefaultRoute,
    /// Only the gateway of this network interface (a VPN's), over PCP or NAT-PMP.
    Interface(String),
}

/// What the mapper last found, for a log line or a settings page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// This computer's own address is public: nothing in front of it to ask.
    OwnAddressPublic(Ipv4Addr),
    /// No router to ask: no IPv4 default route, or the interface is gone or has no IPv4 address.
    NoRouter(Via),
    /// Every port is forwarded, from an address other networks can reach.
    Forwarded(Vec<Forward>),
    /// The router forwards from an address that isn't public: another NAT sits in front of it.
    BehindAnotherNat(Ipv4Addr),
    /// These ports aren't forwarded: the router refused (its reason) or nothing answered.
    NotForwarded { ports: Vec<u16>, refused: Option<String> },
    /// The mapper couldn't start.
    Failed(String),
    /// The forwards were removed: the mapper stopped.
    Removed,
}

impl Status {
    /// One line for a log, as `hoi-node` prints it.
    pub fn line(&self) -> String {
        let ports = |list: &[u16]| list.iter().map(u16::to_string).collect::<Vec<_>>().join(" and ");
        match self {
            Status::OwnAddressPublic(ip) => format!("port mapping: this computer's own address {ip} is public, so there is nothing to forward"),
            Status::NoRouter(Via::DefaultRoute) => "port mapping: no IPv4 default route, so no router to ask; nothing is forwarded".into(),
            Status::NoRouter(Via::Interface(name)) => {
                format!("port mapping: {name} is down or has no IPv4 address, so its gateway can't be asked; nothing is forwarded")
            }
            Status::Forwarded(forwards) => {
                let pairs = forwards.iter().map(|f| format!("{} → {}", f.external, f.internal)).collect::<Vec<_>>().join(", ");
                let method = forwards.first().map_or("", |f| f.method.label());
                format!("port mapping: the router ({method}) forwards UDP {pairs}")
            }
            Status::BehindAnotherNat(ip) => format!(
                "port mapping: the router forwards from {ip}, which is not a public address: another NAT sits in front of it (carrier-grade NAT, or a second router), so visitors can't reach this node this way"
            ),
            Status::NotForwarded { ports: missing, refused } => {
                let by_hand = format!("forward UDP {} to this computer by hand and pass --rtc-advertise PUBLIC-IP:PORT", ports(missing));
                match refused {
                    Some(reason) => format!("port mapping: the router refused ({reason}); {by_hand}"),
                    None => format!("port mapping: the router answered no PCP, NAT-PMP or UPnP request; turn on UPnP or NAT-PMP in its settings, or {by_hand}"),
                }
            }
            Status::Failed(e) => format!("port mapping could not start: {e}"),
            Status::Removed => "port mapping: removed the router's forwards".into(),
        }
    }

    /// What the app shows; `None` once the forwards are gone.
    pub fn report(&self, via_vpn: bool) -> Option<hoi_core::dto::PortMapping> {
        let (state, public, detail) = match self {
            Status::OwnAddressPublic(ip) => ("own-address", Vec::new(), Some(ip.to_string())),
            Status::NoRouter(_) => ("no-router", Vec::new(), None),
            Status::Forwarded(forwards) => ("forwarded", forwards.iter().map(|f| f.external.to_string()).collect(), None),
            Status::BehindAnotherNat(ip) => ("behind-nat", Vec::new(), Some(ip.to_string())),
            Status::NotForwarded { refused: Some(reason), .. } => ("refused", Vec::new(), Some(reason.clone())),
            Status::NotForwarded { refused: None, .. } => ("silent", Vec::new(), None),
            Status::Failed(e) => ("failed", Vec::new(), Some(e.clone())),
            Status::Removed => return None,
        };
        Some(hoi_core::dto::PortMapping { state: state.into(), public, detail, via_vpn })
    }
}

/// The router this computer reaches the internet through, and where to ask it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Router {
    /// This computer's address on the router's network: what the router forwards to.
    pub local: Ipv4Addr,
    /// Where PCP and NAT-PMP requests go: the gateway's port 5351.
    pub pcp: SocketAddr,
    /// Where SSDP searches go; `None` asks over PCP and NAT-PMP only.
    pub ssdp: Option<SocketAddr>,
}

impl Router {
    /// The router to ask `via`, or `None` without one.
    pub fn find(via: &Via) -> Option<Router> {
        match via {
            Via::DefaultRoute => {
                let gateway = netdev::get_default_gateway().ok()?.ipv4.first().copied()?;
                let local = local_toward(gateway)?;
                Some(Router { local, pcp: SocketAddr::new(IpAddr::V4(gateway), PCP_PORT), ssdp: Some(SSDP) })
            }
            Via::Interface(name) => {
                let i = netdev::get_interfaces().into_iter().find(|i| &i.name == name && i.is_up())?;
                let net = i.ipv4.first()?;
                let gateway = vpn_gateway(net.addr(), i.gateway.as_ref().and_then(|g| g.ipv4.first().copied()));
                Some(Router { local: net.addr(), pcp: SocketAddr::new(IpAddr::V4(gateway), PCP_PORT), ssdp: None })
            }
        }
    }
}

/// The gateway of a VPN interface: the one the system names, else the first address of the
/// interface's /24, where VPNs that forward ports over NAT-PMP answer (10.2.0.1 for 10.2.0.2).
fn vpn_gateway(local: Ipv4Addr, named: Option<Ipv4Addr>) -> Ipv4Addr {
    named.filter(|g| !g.is_unspecified() && *g != local).unwrap_or_else(|| {
        let [a, b, c, _] = local.octets();
        Ipv4Addr::new(a, b, c, 1)
    })
}

/// A client for the router's UPnP service on this computer's own network. Requests there are
/// plain HTTP; no system proxy, no redirects, a short timeout.
fn lan_client(timeout: Duration) -> Result<reqwest::Client, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("hoi/{}", hoi_core::VERSION))
        .build()
        .map_err(|e| e.to_string())
}

/// The address this computer sends from toward `gateway`. Connecting a UDP socket sends nothing.
fn local_toward(gateway: Ipv4Addr) -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((gateway, PCP_PORT)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Fail {
    /// Nothing answered: the router doesn't speak this protocol, or has it turned off.
    Silent,
    /// The router speaks it and said no, or answered something unusable.
    Refused(String),
}

/// A router's WAN connection service, as its UPnP description names it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Upnp {
    control: url::Url,
    service: String,
}

/// What the mapper holds for one port, to renew it and remove it.
struct Held {
    forward: Forward,
    nonce: [u8; 12],
    upnp: Option<Upnp>,
}

/// Keeps this node's UDP ports forwarded while it lives. Stop it with [`PortMapper::shutdown`],
/// which removes the forwards.
pub struct PortMapper {
    task: JoinHandle<()>,
    stop: watch::Sender<bool>,
}

impl PortMapper {
    /// Forwards `ports` from the router `via`. `changed` hears the forwards visitors can use
    /// each time they change, and an empty list when there are none; `said` hears each new
    /// [`Status`].
    pub fn spawn(ports: Vec<u16>, via: Via, changed: impl Fn(&[Forward]) + Send + 'static, said: impl FnMut(&Status) + Send + 'static) -> PortMapper {
        let asked = via.clone();
        PortMapper::spawn_with(ports, move || Router::find(&asked), via, changed, said)
    }

    fn spawn_with(
        ports: Vec<u16>,
        find: impl Fn() -> Option<Router> + Send + 'static,
        via: Via,
        changed: impl Fn(&[Forward]) + Send + 'static,
        said: impl FnMut(&Status) + Send + 'static,
    ) -> PortMapper {
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(run(ports, find, via, changed, said, stopped));
        PortMapper { task, stop }
    }

    /// The public addresses the router forwards to `port` here: what to advertise for it.
    pub fn external_for(forwards: &[Forward], port: Option<u16>) -> Vec<SocketAddr> {
        forwards.iter().filter(|f| Some(f.internal) == port).map(|f| SocketAddr::V4(f.external)).collect()
    }

    /// Removes the forwards (best effort, a few seconds at most) and stops.
    pub async fn shutdown(mut self) {
        let _ = self.stop.send(true);
        if tokio::time::timeout(REMOVE_TIMEOUT + Duration::from_secs(1), &mut self.task).await.is_err() {
            self.task.abort();
        }
    }
}

async fn run(
    ports: Vec<u16>,
    find: impl Fn() -> Option<Router>,
    via: Via,
    changed: impl Fn(&[Forward]),
    mut said: impl FnMut(&Status),
    mut stop: watch::Receiver<bool>,
) {
    let client = match lan_client(HTTP_TIMEOUT) {
        Ok(client) => client,
        Err(e) => {
            said(&Status::Failed(e));
            return;
        }
    };
    let mut router: Option<Router> = None;
    let mut held: BTreeMap<u16, Held> = BTreeMap::new();
    let mut usable: Vec<Forward> = Vec::new();
    let mut last: Option<Status> = None;
    loop {
        let found = find();
        if found != router {
            // Another network: forwards on the old router can be neither renewed nor removed.
            held.clear();
            router = found;
        }
        let (wait, status) = match &router {
            // A server with a public address of its own: nothing in front of it to ask.
            Some(r) if public_ip(IpAddr::V4(r.local)) => (RETRY, Status::OwnAddressPublic(r.local)),
            Some(r) => round(r, &ports, &mut held, &client).await,
            None => (RETRY, Status::NoRouter(via.clone())),
        };
        if last.as_ref() != Some(&status) {
            said(&status);
            last = Some(status);
        }
        let now: Vec<Forward> = held.values().map(|h| h.forward).filter(Forward::reachable).collect();
        if now != usable {
            usable = now;
            changed(&usable);
        }
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = stop.changed() => break,
        }
    }
    if let Some(r) = &router
        && !held.is_empty()
    {
        let removals = async {
            for h in held.values() {
                remove(r, h, &client).await;
            }
        };
        if tokio::time::timeout(REMOVE_TIMEOUT, removals).await.is_ok() {
            said(&Status::Removed);
        }
    }
}

/// Forwards or renews every port; returns how long until the next round, and what came of it.
async fn round(router: &Router, ports: &[u16], held: &mut BTreeMap<u16, Held>, client: &reqwest::Client) -> (Duration, Status) {
    let mut renew = RENEW_MAX;
    let mut refused: Option<String> = None;
    let mut missing = Vec::new();
    let mut upnp: Option<Option<Upnp>> = None;
    for &port in ports {
        let previous = held.remove(&port);
        match forward(router, port, previous, client, &mut upnp).await {
            Ok(h) => {
                if h.forward.lifetime > 0 {
                    renew = renew.min(Duration::from_secs(u64::from(h.forward.lifetime) / 2));
                }
                held.insert(port, h);
            }
            Err(e) => {
                missing.push(port);
                if let Fail::Refused(reason) = e {
                    refused.get_or_insert(reason);
                }
            }
        }
    }
    let status = describe(held, missing, refused);
    let renew = renew.max(RENEW_MIN);
    (if matches!(status, Status::NotForwarded { .. }) { renew.min(RETRY) } else { renew }, status)
}

/// What came of a round: what is forwarded, what is not and why.
fn describe(held: &BTreeMap<u16, Held>, missing: Vec<u16>, refused: Option<String>) -> Status {
    if !missing.is_empty() {
        return Status::NotForwarded { ports: missing, refused };
    }
    match held.values().next() {
        Some(first) if !first.forward.reachable() => Status::BehindAnotherNat(*first.forward.external.ip()),
        _ => Status::Forwarded(held.values().map(|h| h.forward).collect()),
    }
}

/// Renews what is held for `port` the way it was made, or asks for a new forward with each
/// protocol in turn.
async fn forward(router: &Router, port: u16, previous: Option<Held>, client: &reqwest::Client, upnp: &mut Option<Option<Upnp>>) -> Result<Held, Fail> {
    if let Some(h) = previous {
        let external = h.forward.external.port();
        let renewed = match (h.forward.method, &h.upnp) {
            (Method::Pcp, _) => pcp_map(router, port, external, LIFETIME, &h.nonce).await,
            (Method::NatPmp, _) => natpmp_map(router, port, external, LIFETIME).await,
            (Method::Upnp, Some(u)) => upnp_map(client, u, router, port, external, LIFETIME).await,
            (Method::Upnp, None) => Err(Fail::Silent),
        };
        if let Ok(forward) = renewed {
            return Ok(Held { forward, ..h });
        }
    }
    let nonce = hoi_crypto::random_bytes::<12>().map_err(|_| Fail::Refused("no randomness".into()))?;
    let mut refused = None;
    match pcp_map(router, port, port, LIFETIME, &nonce).await {
        Ok(forward) => return Ok(Held { forward, nonce, upnp: None }),
        Err(Fail::Refused(r)) => refused = Some(r),
        Err(Fail::Silent) => {}
    }
    match natpmp_map(router, port, port, LIFETIME).await {
        Ok(forward) => return Ok(Held { forward, nonce, upnp: None }),
        Err(Fail::Refused(r)) => refused = refused.or(Some(r)),
        Err(Fail::Silent) => {}
    }
    if upnp.is_none() && router.ssdp.is_some() {
        *upnp = Some(upnp_find(router, client).await.ok());
    }
    if let Some(Some(u)) = upnp.as_ref() {
        match upnp_map(client, u, router, port, port, LIFETIME).await {
            Ok(forward) => return Ok(Held { forward, nonce, upnp: Some(u.clone()) }),
            Err(Fail::Refused(r)) => refused = refused.or(Some(r)),
            Err(Fail::Silent) => {}
        }
    }
    Err(refused.map_or(Fail::Silent, Fail::Refused))
}

/// Asks the router to drop a forward: lifetime 0 for PCP and NAT-PMP, a delete for UPnP.
async fn remove(router: &Router, h: &Held, client: &reqwest::Client) {
    let port = h.forward.internal;
    match (h.forward.method, &h.upnp) {
        (Method::Pcp, _) => {
            let _ = pcp_map(router, port, 0, 0, &h.nonce).await;
        }
        (Method::NatPmp, _) => {
            let _ = natpmp_map(router, port, 0, 0).await;
        }
        (Method::Upnp, Some(u)) => {
            let args = [("NewRemoteHost", String::new()), ("NewExternalPort", h.forward.external.port().to_string()), ("NewProtocol", "UDP".into())];
            let _ = soap(client, u, "DeletePortMapping", &args).await;
        }
        (Method::Upnp, None) => {}
    }
}

/// Sends `request` to the router's port 5351 and returns the first answer from it that `fits`,
/// sending again after each wait.
async fn exchange(router: &Router, request: &[u8], fits: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    let socket = UdpSocket::bind((router.local, 0)).await.ok()?;
    let mut buf = [0u8; 1100];
    for wait in UDP_WAITS_MS {
        socket.send_to(request, router.pcp).await.ok()?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
        while let Ok(Ok((n, from))) = tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await {
            if from == router.pcp && fits(&buf[..n]) {
                return Some(buf[..n].to_vec());
            }
        }
    }
    None
}

/// A PCP MAP request for UDP (RFC 6887 §11.1): any external IPv4 address, `external` as the
/// port wished for (0: any), `lifetime` 0 to delete.
fn pcp_request(local: Ipv4Addr, internal: u16, external: u16, lifetime: u32, nonce: &[u8; 12]) -> [u8; 60] {
    let mut r = [0u8; 60];
    r[0] = 2; // version
    r[1] = 1; // a request, MAP
    r[4..8].copy_from_slice(&lifetime.to_be_bytes());
    r[8..24].copy_from_slice(&local.to_ipv6_mapped().octets());
    r[24..36].copy_from_slice(nonce);
    r[36] = 17; // UDP
    r[40..42].copy_from_slice(&internal.to_be_bytes());
    r[42..44].copy_from_slice(&external.to_be_bytes());
    r[44..60].copy_from_slice(&Ipv4Addr::UNSPECIFIED.to_ipv6_mapped().octets());
    r
}

fn pcp_result(code: u8) -> String {
    let name = match code {
        2 => "not authorized",
        7 => "network failure",
        8 => "out of resources",
        10 => "over this computer's quota",
        11 => "cannot provide an external port",
        12 => "address mismatch: another NAT between here and the router",
        _ => return format!("PCP result {code}"),
    };
    format!("PCP: {name}")
}

async fn pcp_map(router: &Router, internal: u16, external: u16, lifetime: u32, nonce: &[u8; 12]) -> Result<Forward, Fail> {
    let request = pcp_request(router.local, internal, external, lifetime, nonce);
    // A NAT-PMP server answers a version it doesn't know in its own version 0 (RFC 6887 §9).
    let fits = |a: &[u8]| (a.len() >= 60 && a[0] == 2 && a[1] == 0x81 && a[24..36] == nonce[..]) || (a.len() >= 4 && a[0] == 0);
    let a = exchange(router, &request, fits).await.ok_or(Fail::Silent)?;
    if a[0] == 0 || a[3] == 1 {
        return Err(Fail::Silent);
    }
    if a[3] != 0 {
        return Err(Fail::Refused(pcp_result(a[3])));
    }
    if a[36] != 17 || u16::from_be_bytes([a[40], a[41]]) != internal {
        return Err(Fail::Refused("PCP: an answer about another mapping".into()));
    }
    let granted = u32::from_be_bytes([a[4], a[5], a[6], a[7]]);
    let port = u16::from_be_bytes([a[42], a[43]]);
    let octets: [u8; 16] = a[44..60].try_into().map_err(|_| Fail::Silent)?;
    let ip = Ipv6Addr::from(octets).to_ipv4_mapped();
    match ip {
        _ if lifetime == 0 => Ok(Forward { method: Method::Pcp, internal, external: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), lifetime: 0 }),
        Some(ip) if port != 0 && !ip.is_unspecified() => {
            Ok(Forward { method: Method::Pcp, internal, external: SocketAddrV4::new(ip, port), lifetime: granted })
        }
        _ => Err(Fail::Refused("PCP: no external address in the answer".into())),
    }
}

fn natpmp_result(code: u16) -> String {
    let name = match code {
        2 => "not authorized",
        3 => "network failure",
        4 => "out of resources",
        _ => return format!("NAT-PMP result {code}"),
    };
    format!("NAT-PMP: {name}")
}

/// The router's public address over NAT-PMP (RFC 6886 §3.2).
async fn natpmp_external(router: &Router) -> Result<Ipv4Addr, Fail> {
    let a = exchange(router, &[0, 0], |a| a.len() >= 4 && a[0] == 0 && a[1] == 128).await.ok_or(Fail::Silent)?;
    let result = u16::from_be_bytes([a[2], a[3]]);
    if result != 0 {
        return Err(Fail::Refused(natpmp_result(result)));
    }
    if a.len() < 12 {
        return Err(Fail::Refused("NAT-PMP: no external address in the answer".into()));
    }
    Ok(Ipv4Addr::new(a[8], a[9], a[10], a[11]))
}

/// A UDP forward over NAT-PMP (RFC 6886 §3.3); `lifetime` 0 and `external` 0 delete it.
async fn natpmp_map(router: &Router, internal: u16, external: u16, lifetime: u32) -> Result<Forward, Fail> {
    let mut request = [0u8; 12];
    request[1] = 1; // map UDP
    request[4..6].copy_from_slice(&internal.to_be_bytes());
    request[6..8].copy_from_slice(&external.to_be_bytes());
    request[8..12].copy_from_slice(&lifetime.to_be_bytes());
    let a = exchange(router, &request, |a| a.len() >= 4 && a[0] == 0 && a[1] == 129).await.ok_or(Fail::Silent)?;
    let result = u16::from_be_bytes([a[2], a[3]]);
    if result != 0 {
        return Err(Fail::Refused(natpmp_result(result)));
    }
    if a.len() < 16 || u16::from_be_bytes([a[8], a[9]]) != internal {
        return Err(Fail::Refused("NAT-PMP: an answer about another mapping".into()));
    }
    let port = u16::from_be_bytes([a[10], a[11]]);
    let granted = u32::from_be_bytes([a[12], a[13], a[14], a[15]]);
    if lifetime == 0 {
        return Ok(Forward { method: Method::NatPmp, internal, external: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port), lifetime: 0 });
    }
    if port == 0 {
        return Err(Fail::Refused("NAT-PMP: no external port in the answer".into()));
    }
    let ip = natpmp_external(router).await?;
    Ok(Forward { method: Method::NatPmp, internal, external: SocketAddrV4::new(ip, port), lifetime: granted })
}

/// Searches the local network for an internet gateway device and reads its description.
async fn upnp_find(router: &Router, client: &reqwest::Client) -> Result<Upnp, Fail> {
    let ssdp = router.ssdp.ok_or(Fail::Silent)?;
    let socket = UdpSocket::bind((router.local, 0)).await.map_err(|_| Fail::Silent)?;
    for device in ["urn:schemas-upnp-org:device:InternetGatewayDevice:1", "urn:schemas-upnp-org:device:InternetGatewayDevice:2"] {
        let search = format!("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: {device}\r\n\r\n");
        let _ = socket.send_to(search.as_bytes(), ssdp).await;
    }
    let deadline = tokio::time::Instant::now() + SSDP_WAIT;
    let mut buf = [0u8; 2048];
    let mut tried = BTreeSet::new();
    while let Ok(Ok((n, from))) = tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await {
        // Only the gateway itself: another device on the network must not pose as the router
        // and have its own idea of the public address announced.
        if from.ip() != router.pcp.ip() {
            continue;
        }
        let Some(location) = ssdp_location(&buf[..n], from.ip()) else { continue };
        if tried.len() >= 4 || !tried.insert(location.to_string()) {
            continue;
        }
        if let Ok(found) = upnp_describe(client, &location).await {
            return Ok(found);
        }
    }
    Err(Fail::Silent)
}

/// The description URL an SSDP answer names, if it may be fetched: plain HTTP to the very
/// address the answer came from, which must be on this network.
fn ssdp_location(packet: &[u8], from: IpAddr) -> Option<url::Url> {
    let text = std::str::from_utf8(packet).ok()?;
    let mut lines = text.lines();
    if !lines.next()?.starts_with("HTTP/1.1 200") {
        return None;
    }
    let value = lines.find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("location")).map(|(_, v)| v.trim()))?;
    let url = url::Url::parse(value).ok()?;
    let IpAddr::V4(source) = from else { return None };
    let local = source.is_private() || source.is_link_local() || source.is_loopback();
    let same = matches!(url.host(), Some(url::Host::Ipv4(host)) if host == source);
    (url.scheme() == "http" && local && same && url.username().is_empty() && url.password().is_none()).then_some(url)
}

async fn upnp_describe(client: &reqwest::Client, location: &url::Url) -> Result<Upnp, Fail> {
    let response = client.get(location.clone()).send().await.map_err(|_| Fail::Silent)?;
    if response.status().as_u16() != 200 {
        return Err(Fail::Silent);
    }
    let description = bounded(response).await?;
    let (service, control) = wan_service(&description).ok_or(Fail::Silent)?;
    let control = location.join(control).map_err(|_| Fail::Silent)?;
    // The control URL stays on the device that answered the search.
    if control.scheme() != "http" || control.host() != location.host() || control.port_or_known_default() != location.port_or_known_default() {
        return Err(Fail::Silent);
    }
    Ok(Upnp { control, service })
}

/// The WAN connection service a device description offers, best first, with its control URL.
fn wan_service(description: &str) -> Option<(String, &str)> {
    const WANTED: [&str; 3] =
        ["urn:schemas-upnp-org:service:WANIPConnection:2", "urn:schemas-upnp-org:service:WANIPConnection:1", "urn:schemas-upnp-org:service:WANPPPConnection:1"];
    let services: Vec<(&str, &str)> = description
        .split("<service>")
        .skip(1)
        .filter_map(|block| {
            let block = block.split("</service>").next()?;
            Some((xml_text(block, "serviceType")?.trim(), xml_text(block, "controlURL")?.trim()))
        })
        .collect();
    WANTED.iter().find_map(|w| services.iter().find(|(t, _)| t == w).map(|(t, c)| (t.to_string(), *c)))
}

/// The text of the first element named `name`, with or without a namespace prefix.
fn xml_text<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = xml;
    loop {
        let open = rest.find('<')?;
        rest = &rest[open + 1..];
        let end = rest.find('>')?;
        let tag = rest[..end].split_whitespace().next().unwrap_or("");
        let local = tag.rsplit(':').next().unwrap_or(tag);
        if local == name {
            let body = &rest[end + 1..];
            return Some(&body[..body.find("</")?]);
        }
    }
}

/// A SOAP error a router answered with: the UPnP error code when it named one.
enum Soap {
    Unreachable,
    Error(Option<u32>, u16),
}

impl From<Soap> for Fail {
    fn from(e: Soap) -> Fail {
        match e {
            Soap::Unreachable => Fail::Silent,
            Soap::Error(Some(code), _) => Fail::Refused(format!("UPnP error {code}")),
            Soap::Error(None, status) => Fail::Refused(format!("UPnP: HTTP {status}")),
        }
    }
}

async fn soap(client: &reqwest::Client, upnp: &Upnp, action: &str, args: &[(&str, String)]) -> Result<String, Soap> {
    let service = &upnp.service;
    let mut body = format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{service}\">"
    );
    // Every value is a number, an address, `UDP` or this program's name: nothing to escape.
    for (name, value) in args {
        body.push_str(&format!("<{name}>{value}</{name}>"));
    }
    body.push_str(&format!("</u:{action}></s:Body></s:Envelope>"));
    let response = client
        .post(upnp.control.clone())
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header("SOAPAction", format!("\"{service}#{action}\""))
        .body(body)
        .send()
        .await
        .map_err(|_| Soap::Unreachable)?;
    let status = response.status().as_u16();
    let text = bounded(response).await.map_err(|_| Soap::Unreachable)?;
    if status == 200 { Ok(text) } else { Err(Soap::Error(xml_text(&text, "errorCode").and_then(|c| c.trim().parse().ok()), status)) }
}

/// A UDP forward over UPnP. Routers that keep only permanent forwards (error 725) get one, and
/// it is removed when the node stops; a port another computer holds (718) makes way for a
/// random one.
async fn upnp_map(client: &reqwest::Client, upnp: &Upnp, router: &Router, internal: u16, external: u16, lifetime: u32) -> Result<Forward, Fail> {
    let mut lease = lifetime;
    let mut port = external;
    let mut moves = 0;
    loop {
        let args = [
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", port.to_string()),
            ("NewProtocol", "UDP".to_string()),
            ("NewInternalPort", internal.to_string()),
            ("NewInternalClient", router.local.to_string()),
            ("NewEnabled", "1".to_string()),
            ("NewPortMappingDescription", DESCRIPTION.to_string()),
            ("NewLeaseDuration", lease.to_string()),
        ];
        match soap(client, upnp, "AddPortMapping", &args).await {
            Ok(_) => break,
            Err(Soap::Error(Some(725), _)) if lease != 0 => lease = 0,
            Err(Soap::Error(Some(718), _)) if moves < 2 => {
                let [a, b] = hoi_crypto::random_bytes::<2>().map_err(|_| Fail::Refused("no randomness".into()))?;
                port = 20_000 + u16::from_be_bytes([a, b]) % 40_000;
                moves += 1;
            }
            Err(e) => return Err(e.into()),
        }
    }
    let answer = soap(client, upnp, "GetExternalIPAddress", &[]).await?;
    let ip = xml_text(&answer, "NewExternalIPAddress")
        .and_then(|s| s.trim().parse::<Ipv4Addr>().ok())
        .filter(|ip| !ip.is_unspecified())
        .ok_or_else(|| Fail::Refused("UPnP: the router named no public address".into()))?;
    Ok(Forward { method: Method::Upnp, internal, external: SocketAddrV4::new(ip, port), lifetime: lease })
}

/// The body as text, or an error once it grows past what any router answer could need.
async fn bounded(mut response: reqwest::Response) -> Result<String, Fail> {
    if response.content_length().is_some_and(|n| n > MAX_HTTP_BYTES as u64) {
        return Err(Fail::Silent);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Fail::Silent)? {
        if body.len() + chunk.len() > MAX_HTTP_BYTES {
            return Err(Fail::Silent);
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| Fail::Silent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const PUBLIC: Ipv4Addr = Ipv4Addr::new(93, 184, 216, 34);
    const LOCALHOST: Ipv4Addr = Ipv4Addr::LOCALHOST;

    fn router(pcp: SocketAddr, ssdp: SocketAddr) -> Router {
        Router { local: LOCALHOST, pcp, ssdp: Some(ssdp) }
    }

    /// A UDP port nobody answers on: what a router without PCP or NAT-PMP looks like.
    async fn silent() -> (UdpSocket, SocketAddr) {
        let s = UdpSocket::bind((LOCALHOST, 0)).await.unwrap();
        let a = s.local_addr().unwrap();
        (s, a)
    }

    /// A NAT-PMP router (it answers PCP's version 2 with "unsupported version", as RFC 6887
    /// asks). Counts the forwards it holds by internal port.
    async fn natpmp_router(external: Ipv4Addr) -> (SocketAddr, Arc<Mutex<BTreeMap<u16, u32>>>) {
        let s = UdpSocket::bind((LOCALHOST, 0)).await.unwrap();
        let addr = s.local_addr().unwrap();
        let held = Arc::new(Mutex::new(BTreeMap::new()));
        let h = held.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 1100];
            loop {
                let (n, from) = s.recv_from(&mut buf).await.unwrap();
                let r = &buf[..n];
                let answer: Vec<u8> = match (r[0], r[1]) {
                    (0, 0) => [&[0, 128, 0, 0, 0, 0, 0, 1][..], &external.octets()].concat(),
                    (0, 1) => {
                        let internal = u16::from_be_bytes([r[4], r[5]]);
                        let lifetime = u32::from_be_bytes([r[8], r[9], r[10], r[11]]);
                        let external = if lifetime == 0 { 0 } else { internal };
                        if lifetime == 0 {
                            h.lock().unwrap().remove(&internal);
                        } else {
                            h.lock().unwrap().insert(internal, lifetime);
                        }
                        [&[0, 129, 0, 0, 0, 0, 0, 1][..], &internal.to_be_bytes(), &external.to_be_bytes(), &lifetime.to_be_bytes()].concat()
                    }
                    (2, _) => vec![0, 128 + r[1], 0, 1, 0, 0, 0, 1],
                    _ => continue,
                };
                s.send_to(&answer, from).await.unwrap();
            }
        });
        (addr, held)
    }

    /// A PCP router that grants every MAP request for `lifetime` seconds.
    async fn pcp_router(external: Ipv4Addr, lifetime: u32) -> SocketAddr {
        let s = UdpSocket::bind((LOCALHOST, 0)).await.unwrap();
        let addr = s.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 1100];
            loop {
                let (n, from) = s.recv_from(&mut buf).await.unwrap();
                let r = &buf[..n];
                if n < 60 || r[0] != 2 {
                    continue;
                }
                let mut a = [0u8; 60];
                a[0] = 2;
                a[1] = 0x81;
                let asked = u32::from_be_bytes([r[4], r[5], r[6], r[7]]);
                a[4..8].copy_from_slice(&if asked == 0 { 0 } else { lifetime }.to_be_bytes());
                a[24..44].copy_from_slice(&r[24..44]);
                a[44..60].copy_from_slice(&external.to_ipv6_mapped().octets());
                s.send_to(&a, from).await.unwrap();
            }
        });
        addr
    }

    /// A UPnP router: answers SSDP searches with its description's address, serves the
    /// description, and takes AddPortMapping, GetExternalIPAddress and DeletePortMapping.
    /// `permanent_only` answers a lease with error 725, as some routers do.
    async fn upnp_router(external: Ipv4Addr, permanent_only: bool) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
        let http = tokio::net::TcpListener::bind((LOCALHOST, 0)).await.unwrap();
        let port = http.local_addr().unwrap().port();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let c = calls.clone();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = http.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 4096];
                // Headers, then as much body as Content-Length says.
                loop {
                    let n = stream.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text[..end]
                            .lines()
                            .find_map(|l| {
                                l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("content-length")).map(|(_, v)| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&request).to_string();
                let (status, body) = if text.starts_with("GET /desc.xml") {
                    (200, "<?xml version=\"1.0\"?><root><device><serviceList><service><serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType><controlURL>/l3f</controlURL></service><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl/IPConn</controlURL></service></serviceList></device></root>".to_string())
                } else if text.contains("#AddPortMapping") {
                    c.lock().unwrap().push(xml_text(&text, "NewLeaseDuration").unwrap().to_string());
                    if permanent_only && xml_text(&text, "NewLeaseDuration") != Some("0") {
                        (
                            500,
                            "<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>725</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>"
                                .to_string(),
                        )
                    } else {
                        (200, "<s:Envelope><s:Body><u:AddPortMappingResponse/></s:Body></s:Envelope>".to_string())
                    }
                } else if text.contains("#GetExternalIPAddress") {
                    (
                        200,
                        format!(
                            "<s:Envelope><s:Body><u:GetExternalIPAddressResponse><NewExternalIPAddress>{external}</NewExternalIPAddress></u:GetExternalIPAddressResponse></s:Body></s:Envelope>"
                        ),
                    )
                } else if text.contains("#DeletePortMapping") {
                    c.lock().unwrap().push("delete".into());
                    (200, "<s:Envelope/>".to_string())
                } else {
                    (404, String::new())
                };
                let response = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let ssdp = UdpSocket::bind((LOCALHOST, 0)).await.unwrap();
        let ssdp_addr = ssdp.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            loop {
                let (n, from) = ssdp.recv_from(&mut buf).await.unwrap();
                if !buf[..n].starts_with(b"M-SEARCH") {
                    continue;
                }
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nLocation: http://127.0.0.1:{port}/desc.xml\r\n\r\n"
                );
                ssdp.send_to(answer.as_bytes(), from).await.unwrap();
            }
        });
        (ssdp_addr, calls)
    }

    #[test]
    fn a_pcp_request_is_laid_out_as_rfc_6887_says() {
        let r = pcp_request(Ipv4Addr::new(192, 168, 1, 20), 7844, 7844, 7200, &[9; 12]);
        assert_eq!(&r[..4], &[2, 1, 0, 0]);
        assert_eq!(&r[4..8], &7200u32.to_be_bytes());
        assert_eq!(&r[8..24], &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 192, 168, 1, 20]);
        assert_eq!(&r[24..36], &[9; 12]);
        assert_eq!(r[36], 17);
        assert_eq!(&r[40..44], &[0x1e, 0xa4, 0x1e, 0xa4]);
        assert_eq!(&r[44..60], &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0]);
    }

    #[test]
    fn ssdp_locations_are_fetched_only_from_the_device_that_answered_on_this_network() {
        let answer = |location: &str| format!("HTTP/1.1 200 OK\r\nST: x\r\nLOCATION: {location}\r\n\r\n").into_bytes();
        let router = IpAddr::from([192, 168, 1, 1]);
        assert!(ssdp_location(&answer("http://192.168.1.1:5000/rootDesc.xml"), router).is_some());
        // Another host, another scheme, a host name, credentials, a public sender: refused.
        assert!(ssdp_location(&answer("http://192.168.1.7:5000/rootDesc.xml"), router).is_none());
        assert!(ssdp_location(&answer("https://192.168.1.1/rootDesc.xml"), router).is_none());
        assert!(ssdp_location(&answer("http://router.local/rootDesc.xml"), router).is_none());
        assert!(ssdp_location(&answer("http://admin:pw@192.168.1.1/rootDesc.xml"), router).is_none());
        assert!(ssdp_location(&answer("http://93.184.216.34/rootDesc.xml"), IpAddr::V4(PUBLIC)).is_none());
        assert!(ssdp_location(b"NOTIFY * HTTP/1.1\r\nLOCATION: http://192.168.1.1/d.xml\r\n\r\n", router).is_none());
    }

    #[test]
    fn the_best_wan_service_is_chosen_and_prefixed_tags_are_read() {
        let description = "<root><service><serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</serviceType><controlURL>/ppp</controlURL></service>\
            <service><serviceType> urn:schemas-upnp-org:service:WANIPConnection:1 </serviceType><controlURL>/ip</controlURL></service></root>";
        assert_eq!(wan_service(description), Some(("urn:schemas-upnp-org:service:WANIPConnection:1".into(), "/ip")));
        assert_eq!(wan_service("<root><service><serviceType>urn:other</serviceType><controlURL>/x</controlURL></service></root>"), None);
        assert_eq!(xml_text("<s:Body><m:NewExternalIPAddress>203.0.113.4</m:NewExternalIPAddress></s:Body>", "NewExternalIPAddress"), Some("203.0.113.4"));
        assert_eq!(xml_text("<errorCode>718</errorCode>", "errorCode"), Some("718"));
        assert_eq!(xml_text("<a>b</a>", "c"), None);
    }

    #[tokio::test]
    async fn pcp_forwards_and_renews_with_the_same_nonce() {
        let pcp = pcp_router(PUBLIC, 3600).await;
        let r = router(pcp, SSDP);
        let f = pcp_map(&r, 7844, 7844, LIFETIME, &[3; 12]).await.unwrap();
        assert_eq!(f, Forward { method: Method::Pcp, internal: 7844, external: SocketAddrV4::new(PUBLIC, 7844), lifetime: 3600 });
        assert!(f.reachable());
        let client = lan_client(HTTP_TIMEOUT).unwrap();
        let mut held = BTreeMap::new();
        let (wait, _) = round(&r, &[7844, 7843], &mut held, &client).await;
        assert_eq!(wait, Duration::from_secs(1800), "renewed halfway through what the router granted");
        assert_eq!(held.len(), 2);
        let nonce = held[&7844].nonce;
        round(&r, &[7844, 7843], &mut held, &client).await;
        assert_eq!(held[&7844].nonce, nonce, "a renewal is the same mapping");
    }

    #[tokio::test]
    async fn a_nat_pmp_router_is_asked_in_its_own_protocol_after_pcp() {
        let (pcp, held_there) = natpmp_router(PUBLIC).await;
        let r = router(pcp, SSDP);
        assert_eq!(pcp_map(&r, 7844, 7844, LIFETIME, &[1; 12]).await, Err(Fail::Silent));
        let client = lan_client(HTTP_TIMEOUT).unwrap();
        let mut held = BTreeMap::new();
        round(&r, &[7844], &mut held, &client).await;
        assert_eq!(held[&7844].forward, Forward { method: Method::NatPmp, internal: 7844, external: SocketAddrV4::new(PUBLIC, 7844), lifetime: LIFETIME });
        assert_eq!(held_there.lock().unwrap().get(&7844), Some(&LIFETIME));
        remove(&r, &held[&7844], &client).await;
        assert!(held_there.lock().unwrap().is_empty(), "removed when the node stops");
    }

    #[tokio::test]
    async fn upnp_is_found_by_ssdp_and_keeps_a_permanent_forward_when_that_is_all_it_offers() {
        let (_quiet, pcp) = silent().await;
        let (ssdp, calls) = upnp_router(PUBLIC, true).await;
        let r = router(pcp, ssdp);
        let client = lan_client(HTTP_TIMEOUT).unwrap();
        let mut held = BTreeMap::new();
        let (wait, status) = round(&r, &[7844], &mut held, &client).await;
        let f = held[&7844].forward;
        assert_eq!(f, Forward { method: Method::Upnp, internal: 7844, external: SocketAddrV4::new(PUBLIC, 7844), lifetime: 0 });
        assert_eq!(wait, RENEW_MAX, "a permanent forward is still checked hourly");
        assert_eq!(*calls.lock().unwrap(), vec!["7200".to_string(), "0".to_string()]);
        assert_eq!(status.line(), "port mapping: the router (UPnP) forwards UDP 93.184.216.34:7844 → 7844");
        remove(&r, &held[&7844], &client).await;
        assert_eq!(calls.lock().unwrap().last().map(String::as_str), Some("delete"));
    }

    #[tokio::test]
    async fn only_the_gateway_is_believed_when_it_says_it_is_a_upnp_router() {
        // The device answering the search is not the gateway (127.0.0.2 here): ignored.
        let (ssdp, calls) = upnp_router(PUBLIC, false).await;
        let r = Router { local: LOCALHOST, pcp: SocketAddr::from(([127, 0, 0, 2], PCP_PORT)), ssdp: Some(ssdp) };
        let client = lan_client(HTTP_TIMEOUT).unwrap();
        assert_eq!(upnp_find(&r, &client).await, Err(Fail::Silent));
        assert!(calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_router_behind_another_nat_forwards_nothing_visitors_can_use() {
        let (pcp, held_there) = natpmp_router(Ipv4Addr::new(100, 64, 1, 2)).await;
        let seen: Arc<Mutex<Vec<Vec<Forward>>>> = Arc::default();
        let s = seen.clone();
        let mapper =
            PortMapper::spawn_with(vec![7844], move || Some(router(pcp, SSDP)), Via::DefaultRoute, move |f| s.lock().unwrap().push(f.to_vec()), |_| {});
        for _ in 0..50 {
            if !held_there.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(held_there.lock().unwrap().len(), 1, "the router forwards the port");
        mapper.shutdown().await;
        assert!(seen.lock().unwrap().is_empty(), "a carrier-grade NAT address is never announced");
        assert!(held_there.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_computer_with_a_public_address_asks_no_router() {
        let (pcp, held_there) = natpmp_router(PUBLIC).await;
        let seen: Arc<Mutex<Vec<Vec<Forward>>>> = Arc::default();
        let s = seen.clone();
        let mapper = PortMapper::spawn_with(
            vec![7844],
            move || Some(Router { local: PUBLIC, pcp, ssdp: Some(SSDP) }),
            Via::DefaultRoute,
            move |f| s.lock().unwrap().push(f.to_vec()),
            |_| {},
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        mapper.shutdown().await;
        assert!(held_there.lock().unwrap().is_empty(), "the router was never asked");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_mapper_reports_usable_forwards_and_removes_them_when_stopped() {
        let (pcp, held_there) = natpmp_router(PUBLIC).await;
        let seen: Arc<Mutex<Vec<Vec<Forward>>>> = Arc::default();
        let s = seen.clone();
        let mapper =
            PortMapper::spawn_with(vec![7844, 7843], move || Some(router(pcp, SSDP)), Via::DefaultRoute, move |f| s.lock().unwrap().push(f.to_vec()), |_| {});
        for _ in 0..50 {
            if !seen.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let reported = seen.lock().unwrap().clone();
        assert_eq!(reported.len(), 1);
        assert_eq!(
            reported[0].iter().map(|f| (f.internal, f.external)).collect::<Vec<_>>(),
            vec![(7843, SocketAddrV4::new(PUBLIC, 7843)), (7844, SocketAddrV4::new(PUBLIC, 7844))]
        );
        assert_eq!(held_there.lock().unwrap().len(), 2);
        mapper.shutdown().await;
        assert!(held_there.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_silent_network_says_how_to_forward_by_hand() {
        let (_quiet, pcp) = silent().await;
        let (_quiet_ssdp, ssdp) = silent().await;
        let client = lan_client(HTTP_TIMEOUT).unwrap();
        let mut held = BTreeMap::new();
        let (wait, status) = round(&router(pcp, ssdp), &[7844], &mut held, &client).await;
        assert!(held.is_empty());
        assert_eq!(wait, RETRY);
        let line = status.line();
        assert!(line.contains("forward UDP 7844 to this computer by hand") && line.contains("--rtc-advertise"), "{line}");
    }
}
