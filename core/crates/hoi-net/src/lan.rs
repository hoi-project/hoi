//! LAN beacons (`hoi_core::lan`): one UDP socket on the multicast group, sending this
//! node's beacon every half minute and handing what it hears to the node, which puts the routes
//! of members it shares a community with on trial. The dial loop then reaches them directly.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use hoi_core::lan::{LAN_EVERY_SECS, LAN_GROUP, LAN_PORT};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use crate::Shared;

/// The beacon tasks; dropping it stops them.
pub struct LanHandle {
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for LanHandle {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// The socket every hoi node on a machine shares (address reuse), joined to the group.
fn socket(port: u16) -> std::io::Result<std::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(unix)]
    s.set_reuse_port(true)?;
    s.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into())?;
    let group = Ipv4Addr::from(LAN_GROUP);
    s.join_multicast_v4(&group, &Ipv4Addr::UNSPECIFIED)?;
    s.set_multicast_loop_v4(true)?;
    s.set_multicast_ttl_v4(1)?;
    s.set_nonblocking(true)?;
    Ok(s.into())
}

/// Starts sending and hearing beacons for a node whose endpoint listens on `peer_port`.
/// `port`: the beacon port, `LAN_PORT` unless a test picks another.
pub fn start_lan(shared: Arc<Shared>, peer_port: u16, port: Option<u16>) -> std::io::Result<LanHandle> {
    let port = port.unwrap_or(LAN_PORT);
    let sock = Arc::new(UdpSocket::from_std(socket(port)?)?);
    let group = SocketAddrV4::new(Ipv4Addr::from(LAN_GROUP), port);
    let send = {
        let (shared, sock) = (shared.clone(), sock.clone());
        tokio::spawn(async move {
            loop {
                if let Some(b) = shared.with(|n| n.lan_beacon(peer_port)) {
                    let _ = sock.send_to(&b, group).await;
                }
                tokio::time::sleep(Duration::from_secs(LAN_EVERY_SECS)).await;
            }
        })
    };
    let hear = tokio::spawn(async move {
        let mut buf = vec![0u8; 1024];
        loop {
            let Ok((n, from)) = sock.recv_from(&mut buf).await else { continue };
            let beacon = buf[..n].to_vec();
            shared.with(|node| node.lan_heard(&beacon, from.ip()));
        }
    });
    Ok(LanHandle { tasks: vec![send, hear] })
}
