//! The native side of handle checks: for each domain the core asks about, the TXT
//! records of `_hoi.<domain>` from the system's own resolver (`/etc/resolv.conf`; no fixed
//! provider, none where there is no such file), then `https://<domain>/.well-known/hoi` by the
//! rules of link previews. With "Only through my VPN" the DNS question leaves from the VPN's
//! address, and nothing leaves while it is down.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use crate::Shared;
use crate::preview::{Policy, Want, fetch_as};
use crate::vpn::{NetPolicy, VpnState};

const POLL: Duration = Duration::from_secs(1);
const DNS_WAIT: Duration = Duration::from_secs(3);

pub struct HandleChecks {
    task: JoinHandle<()>,
}

impl Drop for HandleChecks {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Checks handles for the node until the value is dropped.
pub fn start_handle_checks(shared: Arc<Shared>, policy: Policy) -> HandleChecks {
    let task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            for c in shared.with(|n| n.take_handle_checks()) {
                let (shared, policy) = (shared.clone(), policy.clone());
                tokio::spawn(async move {
                    let answers = check(&c.domain, &policy()).await;
                    shared.with(|n| n.handle_checked(&c.domain, answers));
                });
            }
        }
    });
    HandleChecks { task }
}

/// Everything the domain says: its `_hoi` TXT strings, then its well-known file's lines.
pub async fn check(domain: &str, policy: &NetPolicy) -> Vec<String> {
    if hoi_core::handle::as_domain(domain).as_deref() != Some(domain) {
        return Vec::new();
    }
    let mut out = txt(&format!("_hoi.{domain}"), policy).await;
    if let Ok(body) = fetch_as(&format!("https://{domain}/.well-known/hoi"), Want::Text, policy).await {
        out.extend(String::from_utf8_lossy(&body[..body.len().min(4096)]).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).take(8));
    }
    out
}

/// The system's resolvers, as `/etc/resolv.conf` names them.
fn resolvers() -> Vec<IpAddr> {
    let Ok(conf) = std::fs::read_to_string("/etc/resolv.conf") else { return Vec::new() };
    conf.lines().filter_map(|l| l.trim().strip_prefix("nameserver")).filter_map(|a| a.trim().parse().ok()).take(3).collect()
}

/// The TXT strings of `name`, asked of each system resolver in turn.
async fn txt(name: &str, policy: &NetPolicy) -> Vec<String> {
    let from: IpAddr = match policy.state() {
        VpnState::Off => Ipv4Addr::UNSPECIFIED.into(),
        VpnState::Up(_) => match policy.state().ipv4() {
            Some(v4) => v4.into(),
            None => return Vec::new(),
        },
        VpnState::Down(_) => return Vec::new(),
    };
    let id = hoi_crypto::random_bytes::<2>().map(u16::from_le_bytes).unwrap_or(0);
    let mut q = simple_dns::Packet::new_query(id);
    q.set_flags(simple_dns::PacketFlag::RECURSION_DESIRED);
    let Ok(qname) = simple_dns::Name::new(name) else { return Vec::new() };
    q.questions.push(simple_dns::Question::new(qname, simple_dns::TYPE::TXT.into(), simple_dns::CLASS::IN.into(), false));
    let Ok(bytes) = q.build_bytes_vec() else { return Vec::new() };
    for server in resolvers() {
        if server.is_ipv6() != from.is_ipv6() && !from.is_unspecified() {
            continue;
        }
        let bind: SocketAddr = if server.is_ipv6() && from.is_unspecified() { (std::net::Ipv6Addr::UNSPECIFIED, 0).into() } else { (from, 0).into() };
        let Ok(sock) = UdpSocket::bind(bind).await else { continue };
        if sock.send_to(&bytes, (server, 53)).await.is_err() {
            continue;
        }
        let mut buf = vec![0u8; 4096];
        let Ok(Ok((n, peer))) = tokio::time::timeout(DNS_WAIT, sock.recv_from(&mut buf)).await else { continue };
        if peer.ip() != server {
            continue;
        }
        let Ok(answer) = simple_dns::Packet::parse(&buf[..n]) else { continue };
        if answer.id() != id {
            continue;
        }
        return answer
            .answers
            .into_iter()
            .filter_map(|rr| match rr.rdata {
                simple_dns::rdata::RData::TXT(t) => String::try_from(t).ok(),
                _ => None,
            })
            .take(16)
            .collect();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_domains_are_checked_and_nothing_leaves_while_the_vpn_is_down() {
        assert!(check("not a domain", &NetPolicy::default()).await.is_empty());
        let vpn = NetPolicy { vpn: Some("hoi-test-no-such-interface".into()), home_network: false };
        assert!(check("example.org", &vpn).await.is_empty());
    }
}
