//! Only through a VPN: a node can be told to reach the internet through one network
//! interface only, a VPN's, so the peers it talks to see the VPN's address and never the
//! home's. Off by default; for people who want it as added security.
//!
//! Three things hold while it is on:
//! - **Bound to the VPN.** Every socket hoi opens toward the internet is bound to the VPN
//!   interface's own address, and pinned to the interface where the system allows it
//!   (`SO_BINDTODEVICE` on Linux 5.7 and later, `IP_BOUND_IF` on macOS and iOS; on Windows the
//!   strong host model sends from an address only over its own interface). Addresses on the home
//!   network may still be used directly if the person allows it, so a phone on the Wi-Fi keeps
//!   working; an internet address outside the VPN never is, and a home-network socket never
//!   sends to an internet address.
//! - **Routed through it.** The system itself must send internet traffic through the VPN: the
//!   source address it picks toward the internet has to be the VPN's. This is asked the way
//!   `connect` on a UDP socket asks it, which sends nothing, and it follows policy routing (the
//!   way WireGuard and most VPN apps route), not just the main table's default route.
//! - **A kill switch.** While the VPN is down, or the system routes around it (split
//!   tunnelling), nothing goes to the internet: the sockets on it close, and the host stops the
//!   native transport until the VPN is back ([`VpnState::Down`]).
//!
//! Port mapping through a VPN asks only the VPN's own gateway (portmap `Via::Interface`), never
//! the home router.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Which networks this node may use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetPolicy {
    /// The VPN's network interface (`wg0`, `tun0`, `utun3`, …). `None`: every interface.
    pub vpn: Option<String>,
    /// With a VPN: addresses on the home network may still be used directly (a phone on the
    /// Wi-Fi). Never an internet address outside the VPN.
    pub home_network: bool,
}

/// What the VPN is doing right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VpnState {
    /// No VPN asked for: every interface may be used.
    Off,
    /// Up, and the system sends internet traffic through it; these are its addresses.
    Up(Vec<IpAddr>),
    /// Asked for but not usable: nothing goes to the internet until it is.
    Down(VpnDown),
}

/// Why the VPN can't be used right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VpnDown {
    /// No interface by that name is up.
    Missing,
    /// It is up but has no address yet.
    NoAddress,
    /// The system sends internet traffic around it (the VPN isn't connected, or splits traffic).
    NotRouted,
}

impl VpnDown {
    pub fn words(self) -> &'static str {
        match self {
            VpnDown::Missing => "is down",
            VpnDown::NoAddress => "has no address yet",
            VpnDown::NotRouted => "doesn't carry this computer's internet traffic (not connected, or split tunnelling)",
        }
    }
}

impl VpnState {
    /// The VPN's addresses while it is up.
    pub fn addresses(&self) -> &[IpAddr] {
        match self {
            VpnState::Up(a) => a,
            _ => &[],
        }
    }

    /// The VPN's IPv4 address while it is up: what the native transport binds to.
    pub fn ipv4(&self) -> Option<Ipv4Addr> {
        self.addresses().iter().find_map(|a| match a {
            IpAddr::V4(v4) => Some(*v4),
            IpAddr::V6(_) => None,
        })
    }

    /// What the app shows: the interface, whether it is up, its address, and why not.
    pub fn report(&self, interface: &str) -> Option<hoi_core::dto::VpnStatus> {
        match self {
            VpnState::Off => None,
            VpnState::Up(a) => {
                Some(hoi_core::dto::VpnStatus { interface: interface.into(), up: true, addresses: a.iter().map(IpAddr::to_string).collect(), detail: None })
            }
            VpnState::Down(why) => {
                Some(hoi_core::dto::VpnStatus { interface: interface.into(), up: false, addresses: Vec::new(), detail: Some(why.words().into()) })
            }
        }
    }
}

/// Where the route check asks the way to: documentation addresses, never contacted (connecting
/// a UDP socket sends nothing), routed like any address on the internet.
const PROBE_V4: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 1);

impl NetPolicy {
    /// Looks at the interfaces and the system's routes now.
    pub fn state(&self) -> VpnState {
        let Some(name) = &self.vpn else { return VpnState::Off };
        let found = netdev::get_interfaces().into_iter().find(|i| &i.name == name && i.is_up());
        let up = found.is_some();
        let addresses = found.map(|i| interface_addresses(&i)).unwrap_or_default();
        vpn_state(up, addresses, route_source(IpAddr::V4(PROBE_V4)))
    }

    /// Which of this computer's addresses, with the interface each is on, hoi may listen and
    /// send on now.
    pub fn allowed(&self, state: &VpnState, local: &[(IpAddr, String)]) -> Vec<IpAddr> {
        allowed(self, state, local)
    }
}

/// The addresses an interface has that a route can carry: IPv4, and IPv6 that isn't link-local.
fn interface_addresses(i: &netdev::Interface) -> Vec<IpAddr> {
    i.ipv4
        .iter()
        .map(|n| IpAddr::V4(n.addr()))
        .chain(i.ipv6.iter().map(|n| n.addr()).filter(|a| (a.segments()[0] & 0xffc0) != 0xfe80).map(IpAddr::V6))
        .filter(|a| !a.is_unspecified() && !a.is_loopback())
        .collect()
}

/// The VPN's state from what was found: its addresses, and the source address the system
/// picks toward the internet.
fn vpn_state(up: bool, addresses: Vec<IpAddr>, routed_from: Option<IpAddr>) -> VpnState {
    if !up {
        return VpnState::Down(VpnDown::Missing);
    }
    if addresses.is_empty() {
        return VpnState::Down(VpnDown::NoAddress);
    }
    if !routed_from.is_some_and(|s| addresses.contains(&s)) {
        return VpnState::Down(VpnDown::NotRouted);
    }
    VpnState::Up(addresses)
}

/// The local address the system would send from toward `dest`, with every routing rule
/// applied. Connecting a UDP socket picks a route and sends nothing.
fn route_source(dest: IpAddr) -> Option<IpAddr> {
    let any: IpAddr = if dest.is_ipv4() { Ipv4Addr::UNSPECIFIED.into() } else { Ipv6Addr::UNSPECIFIED.into() };
    let socket = std::net::UdpSocket::bind(SocketAddr::new(any, 0)).ok()?;
    socket.connect(SocketAddr::new(dest, 9)).ok()?;
    Some(socket.local_addr().ok()?.ip()).filter(|a| !a.is_unspecified())
}

/// A home-network address: IPv4 private or link-local, IPv6 unique local. Carrier-grade NAT
/// space (100.64/10) is not: overlay VPNs use it.
pub fn home_network(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

fn allowed(policy: &NetPolicy, state: &VpnState, local: &[(IpAddr, String)]) -> Vec<IpAddr> {
    let Some(vpn) = &policy.vpn else { return local.iter().map(|(a, _)| *a).collect() };
    local
        .iter()
        .filter(|(a, on)| a.is_loopback() || (on == vpn && state.addresses().contains(a)) || (on != vpn && policy.home_network && home_network(*a)))
        .map(|(a, _)| *a)
        .collect()
}

/// Whether a packet may go from `from` (one of this computer's addresses) to `to`: always
/// without a VPN; with one, from the VPN's addresses anywhere, from any other address only to
/// this computer or the home network.
pub fn may_send(policy: &NetPolicy, state: &VpnState, from: IpAddr, to: IpAddr) -> bool {
    policy.vpn.is_none() || state.addresses().contains(&from) || to.is_loopback() || (policy.home_network && home_network(from) && home_network(to))
}

/// How the person picks the VPN: interfaces that are up, tunnels first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    /// What the system calls it, when that differs ("ProtonVPN", "Mullvad").
    pub label: Option<String>,
    /// A tunnel or point-to-point link: what VPNs look like.
    pub tunnel: bool,
    pub addresses: Vec<String>,
}

pub fn interfaces() -> Vec<Interface> {
    let mut out: Vec<Interface> = netdev::get_interfaces()
        .into_iter()
        .filter(|i| i.is_up() && !i.is_loopback())
        .map(|i| {
            let addresses = interface_addresses(&i).iter().map(IpAddr::to_string).collect();
            let label = i.friendly_name.clone().filter(|f| f != &i.name);
            Interface {
                tunnel: i.is_tun() || i.is_point_to_point() || i.if_type == netdev::interface::types::InterfaceType::Tunnel,
                name: i.name,
                label,
                addresses,
            }
        })
        .collect();
    out.sort_by_key(|i| (!i.tunnel, i.name.clone()));
    out
}

/// A UDP socket bound to `addr`, pinned to `interface` where the system allows it. Pinning that
/// the system refuses (an older Linux kernel without the privilege) leaves the address binding,
/// and the route check, in place.
pub(crate) fn udp_socket(addr: SocketAddr, interface: Option<&str>) -> io::Result<std::net::UdpSocket> {
    let socket = socket2::Socket::new(socket2::Domain::for_address(addr), socket2::Type::DGRAM, Some(socket2::Protocol::UDP))?;
    if let Some(name) = interface {
        pin(&socket, name, addr);
    }
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    Ok(socket.into())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn pin(socket: &socket2::Socket, name: &str, _addr: SocketAddr) {
    let _ = socket.bind_device(Some(name.as_bytes()));
}

#[cfg(target_vendor = "apple")]
fn pin(socket: &socket2::Socket, name: &str, addr: SocketAddr) {
    let Some(index) = netdev::get_interfaces().into_iter().find(|i| i.name == name).and_then(|i| std::num::NonZeroU32::new(i.index)) else { return };
    let _ = if addr.is_ipv4() { socket.bind_device_by_index_v4(Some(index)) } else { socket.bind_device_by_index_v6(Some(index)) };
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn pin(_socket: &socket2::Socket, _name: &str, _addr: SocketAddr) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn vpn(home: bool) -> NetPolicy {
        NetPolicy { vpn: Some("wg0".into()), home_network: home }
    }

    #[test]
    fn the_vpn_counts_only_when_the_system_sends_through_it() {
        let a = vec![ip("10.2.0.2")];
        assert_eq!(vpn_state(true, a.clone(), Some(ip("10.2.0.2"))), VpnState::Up(a.clone()));
        assert_eq!(vpn_state(true, a.clone(), Some(ip("192.168.1.20"))), VpnState::Down(VpnDown::NotRouted), "split tunnelling, or not connected");
        assert_eq!(vpn_state(true, a, None), VpnState::Down(VpnDown::NotRouted));
        assert_eq!(vpn_state(true, Vec::new(), None), VpnState::Down(VpnDown::NoAddress));
        assert_eq!(vpn_state(false, Vec::new(), None), VpnState::Down(VpnDown::Missing));
    }

    #[test]
    fn only_the_vpn_the_home_network_if_allowed_and_this_computer_are_used() {
        let local = vec![
            (ip("127.0.0.1"), "lo".to_string()),
            (ip("192.168.1.20"), "eth0".to_string()),
            (ip("2001:db8::20"), "eth0".to_string()),
            (ip("fd00::20"), "eth0".to_string()),
            (ip("10.2.0.2"), "wg0".to_string()),
            (ip("100.101.1.2"), "tailscale0".to_string()),
        ];
        let up = VpnState::Up(vec![ip("10.2.0.2")]);
        assert_eq!(vpn(true).allowed(&up, &local), vec![ip("127.0.0.1"), ip("192.168.1.20"), ip("fd00::20"), ip("10.2.0.2")]);
        assert_eq!(vpn(false).allowed(&up, &local), vec![ip("127.0.0.1"), ip("10.2.0.2")]);
        // VPN down: nothing toward the internet, the home network still.
        let down = VpnState::Down(VpnDown::NotRouted);
        assert_eq!(vpn(true).allowed(&down, &local), vec![ip("127.0.0.1"), ip("192.168.1.20"), ip("fd00::20")]);
        assert_eq!(vpn(false).allowed(&down, &local), vec![ip("127.0.0.1")]);
        // No VPN asked for: everything, as before.
        assert_eq!(NetPolicy::default().allowed(&VpnState::Off, &local).len(), local.len());
    }

    #[test]
    fn a_socket_off_the_vpn_never_sends_to_the_internet() {
        let up = VpnState::Up(vec![ip("10.2.0.2")]);
        let p = vpn(true);
        assert!(may_send(&p, &up, ip("10.2.0.2"), ip("203.0.113.9")));
        assert!(may_send(&p, &up, ip("192.168.1.20"), ip("192.168.1.30")), "a phone on the Wi-Fi");
        assert!(!may_send(&p, &up, ip("192.168.1.20"), ip("203.0.113.9")), "never past the VPN");
        assert!(!may_send(&p, &VpnState::Down(VpnDown::Missing), ip("10.2.0.2"), ip("203.0.113.9")), "the kill switch");
        assert!(!may_send(&vpn(false), &up, ip("192.168.1.20"), ip("192.168.1.30")), "the home network only if allowed");
        assert!(may_send(&NetPolicy::default(), &VpnState::Off, ip("192.168.1.20"), ip("203.0.113.9")));
    }

    #[test]
    fn the_route_check_reads_the_systems_choice_without_sending() {
        // Loopback is always routable: the system picks loopback as the source.
        assert_eq!(route_source(ip("127.0.0.1")), Some(ip("127.0.0.1")));
    }

    #[test]
    fn a_pinned_socket_binds_to_its_address() {
        let s = udp_socket(SocketAddr::new(ip("127.0.0.1"), 0), Some("lo")).unwrap();
        assert_eq!(s.local_addr().unwrap().ip(), ip("127.0.0.1"));
    }
}
