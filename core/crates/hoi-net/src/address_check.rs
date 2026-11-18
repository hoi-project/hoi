//! One address observation from an explicitly selected destination. No sync hello,
//! saved endpoint, community, pairing credential, discovery or default service is used.
use crate::vpn::{NetPolicy, VpnState};
use futures_util::{SinkExt, StreamExt};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
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
    let Some(interface) = policy.vpn.as_deref().filter(|_| !to.ip().is_loopback() && !(policy.home_network && crate::vpn::home_network(to.ip()))) else {
        return Ok((SocketAddr::new(any, 0), None));
    };
    Ok((tunnel_source(to, policy.state())?, Some(interface)))
}

fn tunnel_source(to: SocketAddr, state: VpnState) -> Result<SocketAddr, &'static str> {
    let VpnState::Up(ips) = state else { return Err("The selected VPN is unavailable for this check.") };
    let ip = ips.into_iter().find(|ip| ip.is_ipv4() == to.is_ipv4()).ok_or("The VPN has no address for this route.")?;
    Ok(SocketAddr::new(ip, 0))
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
    mapped_ip(&buf[..n], &request[8..]).ok_or(UNAVAILABLE)
}

/// RFC 8489 binding success from the connected destination. Plain STUN omits the ICE
/// MESSAGE-INTEGRITY attribute, so use the transaction-bound XOR-MAPPED-ADDRESS directly.
fn mapped_ip(reply: &[u8], transaction: &[u8]) -> Option<IpAddr> {
    let header = reply.get(..20)?;
    if header[..2] != [1, 1]
        || header[4..8] != [0x21, 0x12, 0xa4, 0x42]
        || &header[8..20] != transaction
        || usize::from(u16::from_be_bytes([header[2], header[3]])) != reply.len() - 20
    {
        return None;
    }
    let mut attributes = &reply[20..];
    let mut mapped = None;
    while !attributes.is_empty() {
        let tag = attributes.get(..4)?;
        let len = usize::from(u16::from_be_bytes([tag[2], tag[3]]));
        let value = attributes.get(4..4 + len)?;
        attributes = attributes.get(4 + len.div_ceil(4) * 4..)?;
        if tag[..2] != [0, 0x20] {
            continue;
        }
        if mapped.is_some() || value.first() != Some(&0) {
            return None;
        }
        let ip = match (value.get(1), len) {
            (Some(1), 8) => IpAddr::V4(Ipv4Addr::from(std::array::from_fn::<_, 4, _>(|i| value[4 + i] ^ header[4 + i]))),
            (Some(2), 20) => IpAddr::V6(Ipv6Addr::from(std::array::from_fn::<_, 16, _>(|i| value[4 + i] ^ header[4 + i]))),
            _ => return None,
        };
        mapped = Some(ip.to_canonical());
    }
    mapped.filter(|ip| !ip.is_unspecified() && !ip.is_multicast())
}

const NEED_NUMERIC: &str = "This device has no DNS answer scoped to the selected VPN. Choose the node’s numeric RTC or WebSocket route.";

/// systemd-resolved marks each returned address with its interface. Take only that interface,
/// numeric unicast answers, and a family the selected tunnel can carry.
#[cfg(any(test, target_os = "linux"))]
fn scoped_answer(output: &str, interface: &str, sources: &[IpAddr]) -> Option<IpAddr> {
    output.lines().find_map(|line| {
        let (value, link) = line.split_once("-- link:")?;
        if link.trim() != interface {
            return None;
        }
        let ip = value.split_whitespace().last()?.parse::<IpAddr>().ok()?.to_canonical();
        (!ip.is_multicast() && !ip.is_unspecified() && !ip.is_loopback() && sources.iter().any(|local| local.is_ipv4() == ip.is_ipv4())).then_some(ip)
    })
}

async fn vpn_hostname(host: &str, port: u16, interface: &str, policy: &NetPolicy) -> Result<SocketAddr, &'static str> {
    let VpnState::Up(ips) = policy.state() else {
        return Err("The selected VPN is unavailable for this check.");
    };
    #[cfg(target_os = "linux")]
    {
        let index = netdev::get_interfaces().into_iter().find(|i| i.name == interface).map(|i| i.index).ok_or(NEED_NUMERIC)?;
        // The D-Bus lookup is explicitly scoped to this link. Disallow synthetic local answers,
        // search suffixes, multicast protocols and cached answers from earlier routes.
        let result = tokio::process::Command::new("/usr/bin/resolvectl")
            .args(["--interface", &index.to_string(), "--protocol=dns", "--cache=no", "--synthesize=no", "--search=no", "--legend=no", "query", "--", host])
            .env("LC_ALL", "C")
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|_| NEED_NUMERIC)?;
        if !result.status.success() || result.stdout.len() > 8192 {
            return Err(NEED_NUMERIC);
        }
        let answer = scoped_answer(std::str::from_utf8(&result.stdout).map_err(|_| NEED_NUMERIC)?, interface, &ips).ok_or(NEED_NUMERIC)?;
        return Ok(SocketAddr::new(answer, port));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (host, port, interface, ips);
        Err(NEED_NUMERIC)
    }
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
    } else if let Some(interface) = policy.vpn.as_deref() {
        vpn_hostname(host, port, interface, policy).await?
    } else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_stun_uses_the_reply_transaction_and_both_address_families() {
        let transaction = [7; 12];
        for ip in ["198.51.100.9".parse::<IpAddr>().unwrap(), "2001:db8::7".parse::<IpAddr>().unwrap()] {
            let mut reply = vec![1, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42];
            reply.extend_from_slice(&transaction);
            let (family, bytes) = match ip {
                IpAddr::V4(v) => (1, v.octets().to_vec()),
                IpAddr::V6(v) => (2, v.octets().to_vec()),
            };
            let encoded: Vec<u8> = bytes.iter().enumerate().map(|(i, b)| b ^ reply[4 + i]).collect();
            reply.extend_from_slice(&[0, 0x20, 0, 4 + bytes.len() as u8, 0, family, 0, 1]);
            reply.extend_from_slice(&encoded);
            reply[3] = (reply.len() - 20) as u8;
            assert_eq!(mapped_ip(&reply, &transaction), Some(ip));
            assert_eq!(mapped_ip(&reply, &[8; 12]), None);
            reply.pop();
            assert_eq!(mapped_ip(&reply, &transaction), None);
        }
    }

    #[test]
    fn only_a_fresh_answer_scoped_to_the_required_tunnel_can_supply_a_hostname() {
        let answer = "node.example: 127.0.0.53 -- link: wg0\n  203.0.113.3 -- link: eth0\n  2001:db8::1 -- link: wg0\n  203.0.113.7 -- link: wg0";
        assert_eq!(scoped_answer(answer, "wg0", &["10.0.0.2".parse().unwrap()]), Some("203.0.113.7".parse().unwrap()));
        assert!(scoped_answer("node.example: 203.0.113.7", "wg0", &["10.0.0.2".parse().unwrap()]).is_none());
    }

    #[test]
    fn binds_the_vpn_source_for_the_destinations_address_family() {
        let addresses = || VpnState::Up(vec!["fd00::8".parse().unwrap(), "10.2.0.8".parse().unwrap()]);
        let peer = "203.0.113.11:3344".parse().unwrap();
        let local = tunnel_source(peer, addresses()).unwrap();
        assert_eq!(local, "10.2.0.8:0".parse().unwrap());
        assert_eq!(tunnel_source("[2001:db8::4]:3344".parse().unwrap(), addresses()).unwrap(), "[fd00::8]:0".parse().unwrap());
        assert!(tunnel_source(peer, VpnState::Up(vec!["fd00::8".parse().unwrap()])).is_err());
    }

    #[tokio::test]
    async fn vpn_check_never_looks_up_a_name_or_falls_back_to_an_unbound_public_socket() {
        let policy = NetPolicy { vpn: Some("nonexistent-hoi-check".into()), home_network: false };
        assert!(socket_observation("wss://node.example/", &policy).await.unwrap_err().contains("VPN"));
        assert!(source("203.0.113.11:3344".parse().unwrap(), &policy).is_err());
        assert!(source("127.0.0.1:3344".parse().unwrap(), &policy).is_ok());
    }
}
