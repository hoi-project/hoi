//! One address observation from an explicitly selected destination. No sync hello,
//! saved endpoint, community, pairing credential, discovery or default service is used.
use crate::vpn::{NetPolicy, VpnState};
use futures_util::{SinkExt, StreamExt};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use str0m::ice::StunMessage;
use tokio::net::{TcpSocket, UdpSocket};
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

const UNAVAILABLE: &str = "This node did not report an address on the selected route.";

pub async fn check(route: &str, policy: &NetPolicy) -> Result<(IpAddr, &'static str), &'static str> {
    if route.len() > 1024 {
        return Err(UNAVAILABLE);
    }
    tokio::time::timeout(Duration::from_secs(7), async {
        if let Some(hint) = route.strip_prefix("rtc:") {
            let (fingerprint, socket) = hint.split_once('@').ok_or(UNAVAILABLE)?;
            if hoi_types::uri::unb64(fingerprint).map_or(true, |fp| fp.len() != 32) {
                return Err(UNAVAILABLE);
            }
            let to: SocketAddr = socket.parse().map_err(|_| UNAVAILABLE)?;
            // The existing STUN service on the *same* RTC port tells us the UDP mapping.
            // It proves reachability on this route; it does not authenticate a node identity.
            Ok((udp_observation(to, policy).await?, "udp"))
        } else {
            Ok((socket_observation(route, policy).await?, "websocket"))
        }
    })
    .await
    .map_err(|_| UNAVAILABLE)?
}

/// Select a source from the VPN, or the OS route when permitted. An empty matching address
/// family never turns a VPN-bound check into an unrestricted connection.
fn source(to: SocketAddr, policy: &NetPolicy) -> Result<(SocketAddr, Option<&str>), &'static str> {
    let any = if to.is_ipv4() { IpAddr::V4(Ipv4Addr::UNSPECIFIED) } else { IpAddr::V6(Ipv6Addr::UNSPECIFIED) };
    if to.port() == 0 || to.ip().is_unspecified() || to.ip().is_multicast() {
        return Err(UNAVAILABLE);
    }
    if policy.vpn.is_none() || to.ip().is_loopback() || (policy.home_network && crate::vpn::home_network(to.ip())) {
        return Ok((SocketAddr::new(any, 0), None));
    }
    let VpnState::Up(ips) = policy.state() else {
        return Err("The selected VPN is unavailable for this check.");
    };
    let ip = ips.into_iter().find(|ip| ip.is_ipv4() == to.is_ipv4()).ok_or("The VPN has no address for this route.")?;
    Ok((SocketAddr::new(ip, 0), policy.vpn.as_deref()))
}

async fn udp_observation(to: SocketAddr, policy: &NetPolicy) -> Result<IpAddr, &'static str> {
    let (local, interface) = source(to, policy)?;
    let sock = UdpSocket::from_std(crate::vpn::udp_socket(local, interface).map_err(|_| UNAVAILABLE)?).map_err(|_| UNAVAILABLE)?;
    sock.connect(to).await.map_err(|_| UNAVAILABLE)?;
    let mut request = [0u8; 20];
    request[..8].copy_from_slice(&[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
    request[8..].copy_from_slice(&hoi_crypto::random_bytes::<12>().map_err(|_| UNAVAILABLE)?);
    sock.send(&request).await.map_err(|_| UNAVAILABLE)?;
    let mut buf = [0u8; 512];
    let n = sock.recv(&mut buf).await.map_err(|_| UNAVAILABLE)?;
    if n < 20 || buf[8..20] != request[8..] {
        return Err(UNAVAILABLE);
    }
    let msg = StunMessage::parse(&buf[..n]).map_err(|_| UNAVAILABLE)?;
    if !msg.is_successful_binding_response() {
        return Err(UNAVAILABLE);
    }
    msg.mapped_address().map(|a| a.ip().to_canonical()).filter(|ip| !ip.is_unspecified() && !ip.is_multicast()).ok_or(UNAVAILABLE)
}

async fn socket_observation(route: &str, policy: &NetPolicy) -> Result<IpAddr, &'static str> {
    let url = url::Url::parse(route).map_err(|_| UNAVAILABLE)?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() || url.query().is_some() {
        return Err(UNAVAILABLE);
    }
    let host = url.host_str().ok_or(UNAVAILABLE)?.trim_matches(['[', ']']);
    let loopback = host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if url.scheme() != "wss" && !(url.scheme() == "ws" && loopback) {
        return Err(UNAVAILABLE);
    }
    let port = url.port_or_known_default().ok_or(UNAVAILABLE)?;
    let to = if let Ok(ip) = host.parse::<IpAddr>() {
        SocketAddr::new(ip, port)
    } else if host == "localhost" {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    } else {
        if policy.vpn.is_some() && !matches!(policy.state(), VpnState::Up(_)) {
            return Err("The selected VPN is unavailable for this check.");
        }
        // Name resolution follows the same OS resolver policy as hoi's native HTTPS client.
        tokio::net::lookup_host((host, port)).await.map_err(|_| UNAVAILABLE)?.next().ok_or(UNAVAILABLE)?
    };
    let (local, interface) = source(to, policy)?;
    let sock = if to.is_ipv4() { TcpSocket::new_v4() } else { TcpSocket::new_v6() }.map_err(|_| UNAVAILABLE)?;
    if let Some(name) = interface {
        crate::vpn::pin(&socket2::SockRef::from(&sock), name, local);
    }
    sock.bind(local).map_err(|_| UNAVAILABLE)?;
    let tcp = sock.connect(to).await.map_err(|_| UNAVAILABLE)?;
    let config = WebSocketConfig::default().max_message_size(Some(64)).max_frame_size(Some(64));
    let (mut ws, _) = tokio_tungstenite::client_async_tls_with_config(route, tcp, Some(config), None).await.map_err(|_| UNAVAILABLE)?;
    ws.send(Message::Binary(crate::ADDRESS_CHECK.into())).await.map_err(|_| UNAVAILABLE)?;
    let Some(Ok(Message::Binary(bytes))) = ws.next().await else {
        return Err(UNAVAILABLE);
    };
    let ip = std::str::from_utf8(&bytes).ok().and_then(|s| s.strip_prefix(crate::ADDRESS_REPLY)).and_then(|s| s.parse::<IpAddr>().ok());
    ip.filter(|ip| !ip.is_unspecified() && !ip.is_multicast()).ok_or(UNAVAILABLE)
}
