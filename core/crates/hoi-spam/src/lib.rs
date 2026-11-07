//! hoi-spam: every device is the bouncer. Keys collect decaying strikes for rate
//! violations and bad records; past the threshold the key is bounced on this device for a while
//! and a signed report goes out with record ids as evidence. Peers that forward junk lose score
//! and get disconnected.

use std::collections::HashMap;

use hoi_types::Id;

pub const HALF_LIFE_SECS: f64 = 3600.0;
pub const BAN_AT: f64 = 10.0;

/// How long a key's first bounce lasts. A bounce is one device's verdict, and the rules it judged
/// by may differ from the author's client, so it lifts on its own.
pub const BOUNCE_FOR: u64 = 86_400;
/// The longest a bounce lasts, and how long a lifted one is remembered: each bounce of a key
/// while its last one is remembered lasts twice as long as that one.
pub const BOUNCE_MAX: u64 = 30 * 86_400;

/// How long a key's `times`-th bounce in a row lasts.
pub fn bounce_lasts(times: u32) -> u64 {
    BOUNCE_FOR.saturating_mul(1 << times.saturating_sub(1).min(16)).min(BOUNCE_MAX)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Offence {
    /// Hit the rate limit.
    Rate,
    /// Broke the content rules.
    Content,
    /// Sent the same thing again and again.
    Duplicate,
}

impl Offence {
    fn weight(&self) -> f64 {
        match self {
            Offence::Rate => 1.0,
            Offence::Content => 3.0,
            Offence::Duplicate => 0.25,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Strikes {
    scores: HashMap<(Id, Id), (f64, u64)>,
}

fn decay(score: f64, from: u64, to: u64) -> f64 {
    let dt = to.saturating_sub(from) as f64;
    score * 0.5_f64.powf(dt / HALF_LIFE_SECS)
}

impl Strikes {
    /// Records an offence by `key` in `community`. Returns true when the key just crossed the line.
    pub fn add(&mut self, key: Id, community: Id, o: Offence, now: u64) -> bool {
        let e = self.scores.entry((key, community)).or_insert((0.0, now));
        let before = decay(e.0, e.1, now);
        let after = before + o.weight();
        *e = (after, now);
        before < BAN_AT && after >= BAN_AT
    }

    pub fn score(&self, key: &Id, community: &Id, now: u64) -> f64 {
        self.scores.get(&(*key, *community)).map_or(0.0, |(s, t)| decay(*s, *t, now))
    }

    pub fn forget_quiet(&mut self, now: u64) {
        self.scores.retain(|_, (s, t)| decay(*s, *t, now) > 0.05);
    }
}

/// Gossipsub-style peer scores, per connection.
#[derive(Debug, Default, Clone)]
pub struct PeerScores {
    scores: HashMap<String, f64>,
}

pub const DISCONNECT_AT: f64 = -20.0;
/// The best score good behaviour earns. A penalty of `MAX_SCORE - DISCONNECT_AT` drops any peer.
pub const MAX_SCORE: f64 = 10.0;

impl PeerScores {
    /// Returns true when the peer should be dropped.
    pub fn penalize(&mut self, peer: &str, amount: f64) -> bool {
        let s = self.scores.entry(peer.to_string()).or_insert(0.0);
        *s -= amount;
        *s <= DISCONNECT_AT
    }

    pub fn reward(&mut self, peer: &str) {
        let s = self.scores.entry(peer.to_string()).or_insert(0.0);
        *s = (*s + 0.1).min(MAX_SCORE);
    }

    pub fn forget(&mut self, peer: &str) {
        self.scores.remove(peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strikes_cross_once_and_decay() {
        let mut s = Strikes::default();
        let (k, c) = (Id([1; 32]), Id([2; 32]));
        let mut crossed = 0;
        for i in 0..12 {
            if s.add(k, c, Offence::Rate, 1000 + i) {
                crossed += 1;
            }
        }
        assert_eq!(crossed, 1);
        assert!(s.score(&k, &c, 1000 + 10 * 3600) < 0.1);
    }

    #[test]
    fn bounces_double_and_stay_bounded() {
        assert_eq!(bounce_lasts(0), BOUNCE_FOR);
        assert_eq!(bounce_lasts(1), BOUNCE_FOR);
        assert_eq!(bounce_lasts(2), 2 * BOUNCE_FOR);
        assert_eq!(bounce_lasts(3), 4 * BOUNCE_FOR);
        assert_eq!(bounce_lasts(6), BOUNCE_MAX);
        assert_eq!(bounce_lasts(u32::MAX), BOUNCE_MAX);
    }

    #[test]
    fn bad_peers_get_dropped() {
        let mut p = PeerScores::default();
        assert!(!p.penalize("a", 5.0));
        assert!(p.penalize("a", 15.0));
        for _ in 0..1000 {
            p.reward("b");
        }
        assert!(p.penalize("b", MAX_SCORE - DISCONNECT_AT), "however well it behaved before");
    }
}
