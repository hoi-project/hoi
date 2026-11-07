//! hoi-trust: how much a key counts on *this* device. Nothing here is ever sent anywhere:
//! two devices legitimately rank the same community differently.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Me,
    Friend,
    /// Friend of a friend (degree 2). Needs published trust statements (M3).
    Fof,
    /// Seen for more than 90 days.
    Member,
    Newcomer,
}

pub const MEMBER_AGE: u64 = 90 * 86_400;

impl Level {
    pub fn of(is_me: bool, is_friend: bool, is_fof: bool, key_age_secs: u64) -> Level {
        if is_me {
            Level::Me
        } else if is_friend {
            Level::Friend
        } else if is_fof {
            Level::Fof
        } else if key_age_secs > MEMBER_AGE {
            Level::Member
        } else {
            Level::Newcomer
        }
    }

    /// PROTOCOL.md §8 default weights. Banned keys weigh 0 and never reach this.
    pub fn weight(&self) -> f64 {
        match self {
            Level::Me | Level::Friend => 1.0,
            Level::Fof => 0.5,
            Level::Member => 0.25,
            Level::Newcomer => 0.05,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Me => "me",
            Level::Friend => "friend",
            Level::Fof => "fof",
            Level::Member => "member",
            Level::Newcomer => "newcomer",
        }
    }
}

/// Lower bound of the Wilson score interval (95%) on weighted votes.
pub fn wilson(up: f64, down: f64) -> f64 {
    let n = up + down;
    if n <= 0.0 {
        return 0.0;
    }
    let z = 1.96_f64;
    let p = up / n;
    (p + z * z / (2.0 * n) - z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt()) / (1.0 + z * z / n)
}

/// `hot = wilson(up, down) / (age_hours + 2)^1.5` (TECHNICAL.md §13), with a small floor so a
/// brand-new post with no votes yet still surfaces by freshness.
pub fn hot(up_w: f64, down_w: f64, age_secs: u64) -> f64 {
    let age_h = age_secs as f64 / 3600.0;
    (wilson(up_w, down_w) + 0.02) / (age_h + 2.0).powf(1.5)
}

pub fn loudest(up_w: f64, down_w: f64) -> f64 {
    up_w - down_w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking() {
        assert!(wilson(10.0, 0.0) > wilson(1.0, 0.0));
        assert!(hot(5.0, 0.0, 3600) > hot(5.0, 0.0, 86_400));
        assert!(hot(0.0, 0.0, 60) > hot(0.0, 0.0, 7200), "fresh posts surface without votes");
        assert_eq!(Level::of(false, false, false, 100 * 86_400), Level::Member);
        assert_eq!(Level::of(false, true, false, 0).weight(), 1.0);
    }
}
