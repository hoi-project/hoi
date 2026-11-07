//! Per-source limits for the listeners strangers reach (QUIC and WebRTC). Only addresses that
//! proved they receive what is sent to them are counted, so a spoofed flood can neither use up
//! someone else's allowance nor fill the table. Nothing here is logged or kept past a minute.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);

/// How many connections each source may start per minute, in a table of at most `cap` sources.
pub(crate) struct PerSource<K> {
    per_minute: u32,
    cap: usize,
    since: Option<Instant>,
    counts: HashMap<K, u32>,
}

impl<K: Hash + Eq> PerSource<K> {
    pub(crate) fn new(per_minute: u32, cap: usize) -> Self {
        PerSource { per_minute, cap, since: None, counts: HashMap::new() }
    }

    fn roll(&mut self, now: Instant) {
        if self.since.is_none_or(|s| now.saturating_duration_since(s) >= WINDOW) {
            self.since = Some(now);
            self.counts.clear();
        }
    }

    /// Whether `key` could start one more now.
    pub(crate) fn left(&mut self, key: &K, now: Instant) -> bool {
        self.roll(now);
        match self.counts.get(key) {
            Some(n) => *n < self.per_minute,
            None => self.counts.len() < self.cap,
        }
    }

    /// Counts one start for `key`, or says no. A full table turns newcomers away until the
    /// minute is over rather than forgetting anyone's count early.
    pub(crate) fn take(&mut self, key: K, now: Instant) -> bool {
        if !self.left(&key, now) {
            return false;
        }
        *self.counts.entry(key).or_insert(0) += 1;
        true
    }

    #[cfg(test)]
    pub(crate) fn sources(&self) -> usize {
        self.counts.len()
    }
}

/// The source an address counts as: an IPv4 address, or the /64 an IPv6 address is in (what one
/// host or household gets, so fresh addresses inside it are free to whoever holds it).
pub(crate) fn source(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::from([s[0], s[1], s[2], s[3], 0, 0, 0, 0])
            }
        },
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_counted_per_minute_in_a_bounded_table() {
        let t = Instant::now();
        let mut l = PerSource::new(2, 2);
        assert!(l.take(1, t) && l.take(1, t));
        assert!(!l.left(&1, t) && !l.take(1, t), "two a minute");
        assert!(l.take(2, t));
        assert!(!l.take(3, t), "a full table turns newcomers away");
        assert!(l.take(1, t + WINDOW) && l.take(3, t + WINDOW), "a new minute starts over");
    }

    #[test]
    fn ipv6_counts_per_slash_64() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:bbbb::7".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(source(a), source(b));
        assert_ne!(source(a), source(c));
        assert_eq!(source("::ffff:192.0.2.1".parse().unwrap()), "192.0.2.1".parse::<IpAddr>().unwrap());
        assert_eq!(source("192.0.2.1".parse().unwrap()), "192.0.2.1".parse::<IpAddr>().unwrap());
    }
}
