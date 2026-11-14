//! What passes through this device: amounts and times, per connection and per minute, never
//! content. Kept in memory only; it starts over when the app does. The relay cap is the one
//! number that outlives a restart (`LocalState::relay_day`), so a cap means a day.

use std::collections::VecDeque;

use crate::dto::{RelayStatus, TrafficConnection, TrafficMinute, TrafficView};
use crate::node::Node;

/// Minutes kept for the chart.
const MINUTES: usize = 60;
/// The relay counter is written to storage at most once per this many bytes.
const RELAY_PERSIST_STEP: u64 = 1 << 20;

#[derive(Debug, Default)]
pub(crate) struct Traffic {
    pub started: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub records_in: u64,
    pub records_out: u64,
    pub media_in: u64,
    pub media_out: u64,
    pub refused: u64,
    /// (minute start, bytes in, bytes out), oldest first, at most [`MINUTES`].
    minutes: VecDeque<(u64, u64, u64)>,
}

impl Traffic {
    fn minute(&mut self, now: u64) -> &mut (u64, u64, u64) {
        let at = now - now % 60;
        if self.minutes.back().is_none_or(|m| m.0 != at) {
            self.minutes.push_back((at, 0, 0));
            while self.minutes.len() > MINUTES {
                self.minutes.pop_front();
            }
        }
        self.minutes.back_mut().expect("just pushed")
    }

    /// The last hour ending with the current minute, quiet minutes included.
    fn hour(&self, now: u64) -> Vec<TrafficMinute> {
        let current = now - now % 60;
        (0..MINUTES as u64)
            .rev()
            .map(|back| {
                let at = current.saturating_sub(back * 60);
                let (bytes_in, bytes_out) = self.minutes.iter().find(|m| m.0 == at).map_or((0, 0), |m| (m.1, m.2));
                TrafficMinute { at, bytes_in, bytes_out }
            })
            .collect()
    }
}

/// One connection's share, shown next to it.
#[derive(Debug, Clone, Default)]
pub(crate) struct PeerTraffic {
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub records_in: u64,
    pub records_out: u64,
    pub since: u64,
    pub last_at: u64,
}

impl Node {
    /// Someone else's device that dialed us: what the relay switch and the daily cap are about.
    pub(crate) fn is_member_in(&self, peer: &str) -> bool {
        self.peers.get(peer).is_some_and(|p| !p.client && !p.home && p.meta.incoming)
    }

    pub(crate) fn count_in(&mut self, peer: &str, bytes: usize, now: u64) {
        let n = bytes as u64;
        self.traffic.bytes_in += n;
        self.traffic.minute(now).1 += n;
        if let Some(p) = self.peers.get_mut(peer) {
            p.traffic.bytes_in += n;
            p.traffic.last_at = now;
        }
        if self.is_member_in(peer) {
            self.count_relay(n, now);
        }
    }

    /// `records` and `media` say what the bytes carried, for the totals.
    pub(crate) fn count_out(&mut self, peer: &str, bytes: usize, records: u64, media: u64, now: u64) {
        let n = bytes as u64;
        self.traffic.bytes_out += n;
        self.traffic.records_out += records;
        self.traffic.media_out += media;
        self.traffic.minute(now).2 += n;
        if let Some(p) = self.peers.get_mut(peer) {
            p.traffic.bytes_out += n;
            p.traffic.records_out += records;
            p.traffic.last_at = now;
        }
        if self.is_member_in(peer) {
            self.count_relay(n, now);
        }
    }

    pub(crate) fn count_records_in(&mut self, peer: &str, accepted: u64, refused: u64) {
        self.traffic.records_in += accepted;
        self.traffic.refused += refused;
        if let Some(p) = self.peers.get_mut(peer) {
            p.traffic.records_in += accepted;
        }
    }

    fn count_relay(&mut self, n: u64, now: u64) {
        let day = now / 86_400;
        let (d, before) = self.state.relay_day;
        let before = if d == day { before } else { 0 };
        let after = before + n;
        self.state.relay_day = (day, after);
        if d != day || before / RELAY_PERSIST_STEP != after / RELAY_PERSIST_STEP {
            self.mark_state();
        }
    }

    pub(crate) fn relay_cap_bytes(&self) -> u64 {
        (self.state.settings.daily_cap_gb * 1e9) as u64
    }

    fn relay_today(&self) -> u64 {
        let (d, bytes) = self.state.relay_day;
        if d == self.now() / 86_400 { bytes } else { 0 }
    }

    /// Native nodes can take connections; browsers and phones can't.
    pub(crate) fn can_relay(&self) -> bool {
        matches!(self.platform.as_str(), "desktop" | "node")
    }

    /// Whether other people's devices may connect here now: the relay switch is on and today's
    /// cap isn't used up. Hosts ask before accepting a connection; links already open are closed
    /// when this turns false (see `net_receive`).
    pub fn accepts_members(&self) -> bool {
        self.can_relay() && self.state.settings.relay && self.relay_today() < self.relay_cap_bytes()
    }

    pub fn traffic(&self) -> TrafficView {
        let now = self.now();
        let t = &self.traffic;
        let mut connections: Vec<TrafficConnection> = self
            .peers
            .iter()
            .filter(|(_, p)| p.said_hello)
            .map(|(id, p)| TrafficConnection {
                peer: id.clone(),
                role: if p.home {
                    "home"
                } else if p.client {
                    "client"
                } else {
                    "member"
                }
                .into(),
                // Names are self-chosen; only your own devices' are shown.
                name: if p.home || p.client { p.name.clone() } else { None },
                transport: p.meta.transport.clone(),
                encryption: crate::net::encryption_words(&p.meta),
                bytes_in: p.traffic.bytes_in,
                bytes_out: p.traffic.bytes_out,
                records_in: p.traffic.records_in,
                records_out: p.traffic.records_out,
                since: p.traffic.since,
                last_at: p.traffic.last_at,
            })
            .collect();
        connections.sort_by(|a, b| b.last_at.cmp(&a.last_at).then(a.peer.cmp(&b.peer)));
        let cap = self.relay_cap_bytes();
        let today = self.relay_today();
        TrafficView {
            since: t.started,
            now,
            bytes_in: t.bytes_in,
            bytes_out: t.bytes_out,
            records_in: t.records_in,
            records_out: t.records_out,
            media_in: t.media_in,
            media_out: t.media_out,
            refused: t.refused,
            minutes: t.hour(now),
            connections,
            relay: RelayStatus {
                on: self.can_relay() && self.state.settings.relay,
                possible: self.can_relay(),
                cap_bytes: cap,
                today_bytes: today,
                napping: self.can_relay() && self.state.settings.relay && today >= cap,
            },
        }
    }
}
