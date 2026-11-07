//! hoi-validate: the pure stages of the pipeline (TECHNICAL.md §6), applied to every record on
//! the device that writes it and on every device that receives it. Stage 6 (moderation state
//! and the trust ladder) lives in hoi-moderation; the node composes them.
//!
//! | # | Stage     | Function            |
//! |---|-----------|---------------------|
//! | 1 | Decode    | [`decode`]          |
//! | 2 | Stamp floor (cheap, before any signature work) | [`stamp`] |
//! | 3 | Signature | [`signature`], [`delegation_consent`], [`delegation`] |
//! | 3b| Identity  | [`identity`]        |
//! | 4 | Time      | [`time`]            |
//! | 5 | Rate      | [`rate_windows`], [`window_ok`] |
//! | 7 | Content   | [`content`] ([`text`], [`media`]) |

pub mod media;
pub mod similar;
pub mod text;

use hoi_types::{Body, Id, Kind, LedgerAction, MAX_MEDIA, Record, Verdict, Wire, is_hide_reason};
use thiserror::Error;

pub use text::MAX_LINKS;

/// Records may be stamped up to this far in the future (PROTOCOL.md §4, TECHNICAL.md §6).
pub const CLOCK_SKEW: u64 = 600;
/// "Never expires" for genesis records and ledger entries kept until superseded: 100 years.
pub const KEEP: u64 = 100 * 365 * 86_400;
/// Longest a device may be blessed for in one go.
pub const MAX_DELEGATION: u64 = 400 * 86_400;

/// Since when this build's rules a key can be struck under have been in effect, Unix seconds:
/// rate windows (PROTOCOL.md §9.1), copy-paste floods (§9.2), media (§17) and text (§18). A
/// record made before then may have passed the rules of its day, so failing these earns its
/// author no strike (§9.3). Raise it with every new or stricter rule, to a date some weeks after
/// the release that carries it, so devices still on the previous release can update first.
pub const RULES_SINCE: u64 = 0;

pub const COLORS: [&str; 6] = ["mint", "sky", "lilac", "peach", "sun", "coral"];

pub const HOUR: u64 = 3600;
pub const DAY: u64 = 86_400;

/// Who to blame when a record fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strike {
    None,
    /// The peer that forwarded it (bad bytes, bad signatures).
    Peer,
    /// The key that signed it (rate, content).
    Key,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum Reject {
    #[error("undecodable: {0}")]
    Decode(String),
    #[error("too big for its kind")]
    TooBig,
    #[error("bad signature")]
    BadSignature,
    #[error("signed by a device key we hold no delegation for")]
    NoDelegation,
    #[error("the device's delegation expired")]
    DelegationExpired,
    #[error("the device was revoked")]
    Revoked,
    #[error("the device may not sign this kind of record")]
    Scope,
    #[error("stamp too weak: {have} of {need} bits")]
    WeakStamp { have: u32, need: u32 },
    #[error("the author has no identity yet")]
    NoIdentity,
    #[error("identity work too weak for this community: {have} of {need} bits")]
    WeakIdentity { have: u32, need: u32 },
    #[error("invalid identity: {0}")]
    BadIdentity(String),
    #[error("that identity was burned by its owner")]
    Burned,
    #[error("from the future")]
    FromTheFuture,
    #[error("too old to arrive live")]
    TooOld,
    #[error("expires later than the community allows")]
    ExpiryTooFar,
    #[error("already expired")]
    Expired,
    #[error("rate limited")]
    RateLimited,
    #[error("content rules: {0}")]
    Content(String),
    #[error("unknown community")]
    UnknownCommunity,
    #[error("author is banned here")]
    Banned,
    #[error("removed")]
    Tombstoned,
    #[error("already have it")]
    Duplicate,
    #[error("not allowed: {0}")]
    NotAllowed(String),
}

impl Reject {
    pub fn strike(&self) -> Strike {
        match self {
            Reject::Decode(_) | Reject::TooBig | Reject::BadSignature | Reject::BadIdentity(_) => Strike::Peer,
            Reject::RateLimited | Reject::Content(_) => Strike::Key,
            _ => Strike::None,
        }
    }

    /// Short machine name for counters and import reports.
    pub fn code(&self) -> &'static str {
        match self {
            Reject::Decode(_) => "decode",
            Reject::TooBig => "too-big",
            Reject::BadSignature => "signature",
            Reject::NoDelegation | Reject::DelegationExpired | Reject::Revoked | Reject::Scope => "delegation",
            Reject::WeakStamp { .. } => "stamp",
            Reject::NoIdentity | Reject::WeakIdentity { .. } | Reject::BadIdentity(_) => "identity",
            Reject::Burned => "burned",
            Reject::FromTheFuture | Reject::TooOld | Reject::ExpiryTooFar | Reject::Expired => "time",
            Reject::RateLimited => "rate",
            Reject::Content(_) => "content",
            Reject::UnknownCommunity => "unknown-community",
            Reject::Banned => "banned",
            Reject::Tombstoned => "removed",
            Reject::Duplicate => "duplicate",
            Reject::NotAllowed(_) => "not-allowed",
        }
    }
}

// ------------------------------------------------------------------ 1 · decode

#[derive(Debug, Clone)]
pub struct Decoded {
    pub wire: Wire,
    pub id: Id,
    pub size: usize,
}

pub fn decode(bytes: &[u8]) -> Result<Decoded, Reject> {
    if bytes.len() > 64 * 1024 + 256 {
        return Err(Reject::TooBig);
    }
    let (wire, id) = Wire::decode(bytes).map_err(|e| Reject::Decode(e.to_string()))?;
    if bytes.len() > wire.record.kind().max_size() {
        return Err(Reject::TooBig);
    }
    Ok(Decoded { wire, id, size: bytes.len() })
}

// ------------------------------------------------------------------ 3 · signature & delegation

pub fn signature(d: &Decoded) -> Result<(), Reject> {
    if hoi_crypto::verify(&d.wire.record.author, d.id.as_bytes(), &d.wire.sig) { Ok(()) } else { Err(Reject::BadSignature) }
}

/// A delegation carries two signatures: the record's, by the device (its consent, so
/// nobody can claim someone else's device), checked by [`signature`]; and `root_sig`, the root's
/// blessing, checked here. Neither key may be weak, and no blessing lasts more than 400 days.
pub fn delegation_consent(r: &Record) -> Result<Id, Reject> {
    let Body::Delegation { root, scope, expires_at, root_sig, .. } = &r.body else { return Err(Reject::NotAllowed("not a delegation".into())) };
    if !hoi_crypto::key_is_strong(root) || !hoi_crypto::key_is_strong(&r.author) {
        return Err(Reject::BadSignature);
    }
    if !hoi_crypto::verify(root, hoi_types::delegation_statement(root, &r.author, scope, *expires_at).as_bytes(), root_sig) {
        return Err(Reject::BadSignature);
    }
    if *expires_at > r.created_at.saturating_add(MAX_DELEGATION) {
        return Err(Reject::Content("a device is blessed for at most 400 days at a time".into()));
    }
    Ok(*root)
}

/// What a node knows about a device key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationInfo {
    pub root: Id,
    pub scope: Vec<String>,
    pub expires_at: u64,
    pub revoked_at: Option<u64>,
}

/// The scope string a device needs to sign `kind` (PROTOCOL.md §3, delegation).
/// `None` means only a root (or community) key may sign it.
pub fn scope_needed(kind: Kind, community: Option<&Id>) -> Option<Vec<String>> {
    match kind {
        Kind::Profile | Kind::ProfileColor => Some(vec!["post".into(), "comment".into()]),
        Kind::Post => Some(vec!["post".into()]),
        Kind::Comment => Some(vec!["comment".into()]),
        Kind::Edit | Kind::Delete => Some(vec!["post".into(), "comment".into()]),
        Kind::Vote | Kind::Label | Kind::Report | Kind::Vouch | Kind::Review => Some(vec!["vote".into()]),
        Kind::Ledger => community.map(|c| vec![format!("moderate:{}", c.hex())]),
        Kind::Pointer => Some(vec!["post".into(), "comment".into()]),
        Kind::Roster | Kind::Share | Kind::Trust | Kind::Tally | Kind::Blocklist => Some(vec!["vote".into()]),
        Kind::Wiki => Some(vec!["post".into()]),
        Kind::Community | Kind::Delegation | Kind::Revoke | Kind::Identity | Kind::Rotate => None,
    }
}

pub fn scope_covers(scope: &[String], needed: &[String]) -> bool {
    scope.iter().any(|s| s == "*" || needed.contains(s))
}

/// Stage 3 for records signed by a device key. `deleg` is `None` when the author is not a known
/// device; call this only when the author isn't a root key itself.
pub fn delegation(record: &Record, deleg: Option<&DelegationInfo>) -> Result<Id, Reject> {
    let d = deleg.ok_or(Reject::NoDelegation)?;
    if record.created_at > d.expires_at {
        return Err(Reject::DelegationExpired);
    }
    if let Some(rv) = d.revoked_at
        && record.created_at >= rv
    {
        return Err(Reject::Revoked);
    }
    let needed = scope_needed(record.kind(), record.community.as_ref()).ok_or(Reject::Scope)?;
    if !scope_covers(&d.scope, &needed) {
        return Err(Reject::Scope);
    }
    Ok(d.root)
}

// ------------------------------------------------------------------ 3b · identity

/// An `identity` record's memory-hard work (PROTOCOL.md §4.1). Returns the bits it carries.
/// Costs one Argon2id evaluation (8 MiB), so the node runs it only after the record's cheap
/// stamp and signature passed, and caches the answer.
pub fn identity(r: &Record) -> Result<u32, Reject> {
    let Body::Identity { work, .. } = r.body else { return Err(Reject::NotAllowed("not an identity".into())) };
    if !hoi_crypto::key_is_strong(&r.author) {
        return Err(Reject::BadIdentity("weak key".into()));
    }
    let bits = hoi_crypto::identity::work_bits(&r.author, work);
    if bits < hoi_crypto::identity::MIN_BITS {
        return Err(Reject::BadIdentity(format!("{bits} bits of work, {} needed", hoi_crypto::identity::MIN_BITS)));
    }
    Ok(bits)
}

// ------------------------------------------------------------------ 2 · stamp

pub fn stamp(id: &Id, nonce: u64, need: u32) -> Result<u32, Reject> {
    let have = hoi_crypto::pow::stamp_bits(id, nonce);
    if have >= need { Ok(have) } else { Err(Reject::WeakStamp { have, need }) }
}

/// The least work any record of this kind could ever need: community difficulty may be as low
/// as 8 bits, minus the kind and the largest age discount. Checked before the signature, so a
/// flood of unstamped junk costs one hash each instead of a signature verification.
pub fn stamp_floor(kind: Kind) -> u32 {
    match kind {
        Kind::Community | Kind::Delegation | Kind::Revoke | Kind::Identity | Kind::Rotate => hoi_types::GLOBAL_POW_BITS,
        k => 8u32.saturating_sub(k.pow_discount()).saturating_sub(6),
    }
}

// ------------------------------------------------------------------ 4 · time

/// `live`: arrived on gossip, so it must be fresh (±10 min). Catch-up sync and local imports
/// may be older but never from the future or expired. `max_retention` is `None` for records
/// outside any community's window (genesis, delegations, entries kept until superseded).
pub fn time(record: &Record, now: u64, live: bool, max_retention: Option<u64>) -> Result<(), Reject> {
    if record.created_at > now + CLOCK_SKEW {
        return Err(Reject::FromTheFuture);
    }
    if live && record.created_at + CLOCK_SKEW < now {
        return Err(Reject::TooOld);
    }
    if record.expires_at <= now {
        return Err(Reject::Expired);
    }
    let cap = max_retention.unwrap_or(KEEP);
    if record.expires_at > record.created_at.saturating_add(cap) {
        return Err(Reject::ExpiryTooFar);
    }
    Ok(())
}

/// Whether a record lives outside the community's retention window (kept for up to 100 years
/// or until superseded): genesis, delegations, revocations, identities, vouches, and ledger
/// entries for rules, limits and moderators.
pub fn exempt_from_retention(record: &Record) -> bool {
    match &record.body {
        Body::Community(_)
        | Body::Delegation { .. }
        | Body::Revoke { .. }
        | Body::Identity { .. }
        | Body::Rotate { .. }
        | Body::Vouch { .. }
        | Body::Share { .. }
        // a page is its whole history: each edit builds on the ones before
        | Body::Wiki { .. } => true,
        Body::Ledger(l) => l.action.kept_until_superseded(),
        _ => false,
    }
}

// ------------------------------------------------------------------ 5 · rate

/// A token bucket per key per community. Pure: the caller stores it. (Superseded for records by
/// [`rate_windows`]; still handy for per-connection budgets.)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateBucket {
    pub tokens: f64,
    pub updated: u64,
}

impl RateBucket {
    pub fn full(capacity: f64, now: u64) -> RateBucket {
        RateBucket { tokens: capacity, updated: now }
    }

    /// Refills at `per_day` tokens a day up to `capacity`, then takes one.
    pub fn take(&mut self, now: u64, per_day: f64, capacity: f64) -> bool {
        let dt = now.saturating_sub(self.updated) as f64;
        self.tokens = (self.tokens + dt * per_day / 86_400.0).min(capacity);
        self.updated = now.max(self.updated);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Which rate group a kind counts in.
pub fn rate_group(k: Kind) -> u8 {
    match k {
        Kind::Profile => 7,
        Kind::ProfileColor => 8,
        Kind::Post => 0,
        Kind::Comment => 1,
        Kind::Edit | Kind::Delete => 2,
        Kind::Vote => 3,
        Kind::Vouch | Kind::Review => 4,
        Kind::Label | Kind::Report => 5,
        Kind::Pointer => 9,
        Kind::Roster => 10,
        Kind::Share => 11,
        Kind::Trust => 12,
        Kind::Wiki => 13,
        Kind::Tally => 14,
        Kind::Blocklist => 15,
        _ => 6,
    }
}

/// How many records of a kind one author may have in a community within each window, counted
/// by `created_at` (PROTOCOL.md §9.1). Counting by the records' own time rather than arrival
/// makes the limit the same on every device and closes the old gap where backdated records,
/// arriving as "catch-up", skipped the limit. `newcomer` means not yet admitted by the ladder.
pub fn rate_windows(kind: Kind, newcomer: bool, newcomer_posts_per_day: u32, with_media: bool) -> Vec<(u64, u32)> {
    match (kind, newcomer) {
        (Kind::Profile | Kind::ProfileColor, _) => vec![(HOUR, 2), (DAY, 6)],
        (Kind::Post, true) => vec![(DAY, newcomer_posts_per_day)],
        (Kind::Post, false) if with_media => vec![(HOUR, 4), (DAY, 6)],
        (Kind::Post, false) => vec![(HOUR, 12), (DAY, 48)],
        (Kind::Comment, true) => vec![(HOUR, 10), (DAY, 30)],
        (Kind::Comment, false) => vec![(HOUR, 60), (DAY, 400)],
        (Kind::Edit | Kind::Delete, _) => vec![(HOUR, 30), (DAY, 100)],
        (Kind::Vote, _) => vec![(HOUR, 300), (DAY, 2000)],
        (Kind::Vouch | Kind::Review, true) => vec![(DAY, 20)],
        (Kind::Vouch | Kind::Review, false) => vec![(HOUR, 60), (DAY, 300)],
        (Kind::Label | Kind::Report, true) => vec![(DAY, 20)],
        (Kind::Label | Kind::Report, false) => vec![(DAY, 200)],
        // a post's or comment's own pointer, and repairs' updates of others' (SCATTER.md S5)
        (Kind::Pointer, true) => vec![(HOUR, 20), (DAY, 60)],
        (Kind::Pointer, false) => vec![(HOUR, 120), (DAY, 1000)],
        // one every twelve hours from each of a person's durable devices
        (Kind::Roster, _) => vec![(DAY, 8)],
        // five friends, now and then
        (Kind::Share, _) => vec![(DAY, 20)],
        // a person's friends change now and then; the newest statement replaces the last
        (Kind::Trust, _) => vec![(HOUR, 4), (DAY, 12)],
        // a page is written in sittings: many small saves, then nothing for a while
        (Kind::Wiki, _) => vec![(HOUR, 60), (DAY, 200)],
        // a durable node counts once an hour or so
        (Kind::Tally, _) => vec![(HOUR, 4), (DAY, 48)],
        (Kind::Blocklist, _) => vec![(HOUR, 4), (DAY, 12)],
        _ => Vec::new(),
    }
}

/// Whether a record at `t` keeps every `w`-second span around it at `max` records or fewer,
/// given the author's other records' times (`times`, any order).
pub fn window_ok(times: &[u64], t: u64, w: u64, max: u32) -> bool {
    if max == 0 {
        return false;
    }
    let mut near: Vec<u64> = times.iter().copied().filter(|x| x.abs_diff(t) < w).collect();
    near.push(t);
    near.sort_unstable();
    // every span [s, s + w) that contains t starts at a record time in (t - w, t]
    near.iter().filter(|s| **s <= t).all(|s| near.iter().filter(|x| **x >= *s && **x < s.saturating_add(w)).count() <= max as usize)
}

// ------------------------------------------------------------------ 7 · content

fn bad(msg: impl Into<String>) -> Reject {
    Reject::Content(msg.into())
}

fn c(r: Result<(), String>) -> Result<(), Reject> {
    r.map_err(Reject::Content)
}

fn name(n: &Option<String>) -> Result<(), Reject> {
    match n {
        Some(n) => c(text::name(n)),
        None => Ok(()),
    }
}

pub fn limits(l: &hoi_types::Limits) -> Result<(), Reject> {
    if !(8..=30).contains(&l.pow_bits) {
        return Err(bad("stamp difficulty must be 8–30 bits"));
    }
    if !(3600..=10 * 365 * 86_400).contains(&l.max_retention) {
        return Err(bad("retention must be between an hour and ten years"));
    }
    if l.newcomer_posts_per_day > 1000 || l.newcomer_age > 365 * 86_400 {
        return Err(bad("newcomer limits out of range"));
    }
    if l.autoban_threshold == 0 || l.autoban_threshold > 10_000 || l.labelers.len() > 32 {
        return Err(bad("autoban or labelers out of range"));
    }
    if l.admission == hoi_types::Admission::AgeProof && l.age_issuers.is_empty() {
        return Err(bad("an age-proof community names the credential issuers it trusts"));
    }
    let d = &l.ladder;
    if d.identity_bits > hoi_crypto::identity::MAX_BITS {
        return Err(bad(format!("identity work is at most {} bits", hoi_crypto::identity::MAX_BITS)));
    }
    if !(1..=16).contains(&d.admit_vouches) || d.established_age > 365 * DAY || d.trusted_posts > 1000 || d.media_score > 100_000 {
        return Err(bad("ladder steps out of range"));
    }
    // Quorums below two would let one person hide, block or overturn alone.
    if [d.hide_quorum, d.block_quorum, d.veto_quorum].iter().any(|q| !(2..=100).contains(q)) {
        return Err(bad("quorums are 2 to 100"));
    }
    if d.contest_window > 14 * DAY {
        return Err(bad("the contest window is at most 14 days"));
    }
    // The floor is part of the protocol: no community asks for less (SCATTER.md §6).
    if !hoi_types::scatter::LEVELS.contains(&l.scatter.resist) {
        return Err(bad("the scatter level is 10, 15 or 20 percent of places"));
    }
    Ok(())
}

fn media_refs(media: &[hoi_types::MediaRef]) -> Result<(), Reject> {
    if media.len() > MAX_MEDIA {
        return Err(bad(format!("a post carries at most {MAX_MEDIA} media items")));
    }
    let mut total = 0u64;
    for (i, m) in media.iter().enumerate() {
        let cap = media::max_size(&m.mime).ok_or_else(|| bad("unknown media type"))?;
        if m.size == 0 || m.size > cap {
            return Err(bad("media size out of range"));
        }
        total += m.size;
        if media.iter().take(i).any(|o| o.hash == m.hash) {
            return Err(bad("the same media twice"));
        }
        if media::is_image(&m.mime) {
            let (w, h) = (m.w.unwrap_or(0), m.h.unwrap_or(0));
            if w == 0 || h == 0 || w > media::MAX_SIDE || h > media::MAX_SIDE || w as u64 * h as u64 > media::MAX_PIXELS {
                return Err(bad("image dimensions out of range"));
            }
        } else if m.w.is_some() || m.h.is_some() {
            return Err(bad("only images state dimensions"));
        }
        if let Some(a) = &m.alt {
            c(text::line(a, "alt text", 500, false))?;
        }
    }
    if total > 24 * 1024 * 1024 {
        return Err(bad("media too big in total"));
    }
    Ok(())
}

/// Validate profile pixels before they can be stored, rendered or fingerprinted.
pub fn avatar(image: &[u8]) -> Result<(), Reject> {
    if image.is_empty() {
        return Ok(());
    }
    if image.len() > hoi_types::MAX_AVATAR_BYTES {
        return Err(bad("profile picture exceeds 4 KiB"));
    }
    let info = media::check(media::JPEG, image).map_err(bad)?;
    if !matches!((info.w, info.h), (Some(w), Some(h)) if w == h && w > 0 && w <= hoi_types::MAX_AVATAR_SIDE) {
        return Err(bad("profile pictures must be square and at most 128 pixels"));
    }
    similar::examine(media::JPEG, image).map_err(|_| bad("profile picture cannot be decoded"))?;
    Ok(())
}

/// Devices a `drop-key` entry seals its secret to at most.
pub const MAX_DROP_KEY_DEVICES: usize = 64;
/// How long a roster counts at most.
pub const MAX_ROSTER_LIFE: u64 = 3 * DAY;

/// A share of a recovery line (`x ‖ y[32]`) sealed to one device: 32 + 33 + 16 bytes.
pub const SEALED_SHARE: usize = 32 + 33 + 16;

/// A pointer record's sealed manifest at most.
pub const MAX_POINTER_SEALED: usize = 12 * 1024;

pub fn content(r: &Record) -> Result<(), Reject> {
    match &r.body {
        Body::Profile { image, .. } => avatar(image),
        Body::ProfileColor { color, .. } => {
            if hoi_types::is_avatar_color(color) {
                Ok(())
            } else {
                Err(bad("unknown profile color"))
            }
        }
        Body::Post { title, text: t, link, name: n, media } => {
            c(text::line(title, "title", 300, true))?;
            c(text::markdown(t, "text", 40_000, 2000))?;
            if let Some(l) = link {
                c(text::url(l).map_err(|e| format!("link: {e}")))?;
            }
            media_refs(media)?;
            name(n)
        }
        Body::Comment { text: t, name: n } => {
            if t.trim().is_empty() {
                return Err(bad("empty comment"));
            }
            c(text::markdown(t, "text", 10_000, 500))?;
            name(n)
        }
        Body::Edit { text: t, title } => {
            c(text::markdown(t, "text", 40_000, 2000))?;
            if let Some(ti) = title {
                c(text::line(ti, "title", 300, true))?;
            }
            Ok(())
        }
        Body::Vote { value, .. } => {
            if !(-1..=1).contains(value) {
                return Err(bad("a vote is -1, 0 or 1"));
            }
            Ok(())
        }
        Body::Delete | Body::Revoke { .. } | Body::Identity { .. } => Ok(()),
        Body::Rotate { person, to } => {
            if person == to || *to == r.author || *person == r.author {
                return Err(bad("a rotation names the person and a new root, both other than the recovery key"));
            }
            Ok(())
        }
        // whom it names is its author's to say; only a person who trusts the author reads it
        Body::Trust { .. } => Ok(()),
        // the update's shape is checked where it is read (hoi-core, crate::wiki)
        Body::Wiki { .. } => Ok(()),
        // counts are their author's word; only people who follow the author read them
        Body::Tally { .. } => Ok(()),
        // what a keeper lists is theirs to say; only people who follow them apply it
        Body::Blocklist { .. } => Ok(()),
        Body::Share { sealed, .. } => {
            // a share is 33 bytes, sealed: 32 bytes of encapsulated key, 16 of tag
            if sealed.iter().any(|(_, s)| s.len() != SEALED_SHARE) {
                return Err(bad("a sealed share is 81 bytes"));
            }
            if sealed.iter().enumerate().any(|(i, (d, _))| sealed[..i].iter().any(|(e, _)| e == d)) {
                return Err(bad("one sealed share per device"));
            }
            Ok(())
        }
        Body::Delegation { scope, .. } => {
            if scope.is_empty() || scope.len() > 16 {
                return Err(bad("bad delegation scope"));
            }
            for s in scope {
                let ok = matches!(s.as_str(), "*" | "post" | "comment" | "vote") || s.strip_prefix("moderate:").is_some_and(|h| Id::from_hex(h).is_ok());
                if !ok {
                    return Err(bad("unknown delegation scope"));
                }
            }
            Ok(())
        }
        Body::Vouch { value, reason, .. } => match (value, reason) {
            (1 | 0, None) => Ok(()),
            (-1, Some(why)) if is_hide_reason(why) => Ok(()),
            (-1, _) => Err(bad("an objection needs a reason: spam, abuse, illegal, off-topic or rule:<n>")),
            (1 | 0, Some(_)) => Err(bad("only objections carry a reason")),
            _ => Err(bad("a vouch is -1, 0 or 1")),
        },
        Body::Review { verdict, reason, .. } => match (verdict, reason) {
            (Verdict::Ok | Verdict::None, None) => Ok(()),
            (Verdict::Hide, Some(why)) if is_hide_reason(why) => Ok(()),
            (Verdict::Hide, _) => Err(bad("hiding needs a reason: spam, abuse, illegal, off-topic or rule:<n>. Disagreeing is what downvotes are for")),
            (Verdict::Veto, Some(why)) => c(text::line(why, "veto reason", 200, true)),
            (Verdict::Veto, None) => Err(bad("a veto needs a reason")),
            (Verdict::Ok | Verdict::None, Some(_)) => Err(bad("only hides and vetoes carry a reason")),
        },
        Body::Community(g) => {
            c(text::line(&g.name, "community name", 40, true))?;
            if g.description.chars().count() > 300 {
                return Err(bad("description is too long"));
            }
            c(text::chars(&g.description, text::Field::Body, "description"))?;
            if !COLORS.contains(&g.color.as_str()) {
                return Err(bad("unknown community colour"));
            }
            if g.owners.is_empty() || g.owners.len() > 16 {
                return Err(bad("a community needs 1–16 owners"));
            }
            if g.threshold == 0 || g.threshold as usize > g.owners.len() {
                return Err(bad("threshold must be between 1 and the number of owners"));
            }
            limits(&g.limits)
        }
        Body::Ledger(l) => {
            if let Some(x) = &l.reason {
                c(text::line(x, "reason", 500, false))?;
            }
            if let Some(t) = &l.text {
                c(text::markdown(t, "rules", 20_000, 1000))?;
            }
            if l.evidence.as_ref().is_some_and(|e| e.len() > 256) || l.cosign.as_ref().is_some_and(|c| c.len() > 16) {
                return Err(bad("too much evidence or too many cosigners"));
            }
            let needs_target = matches!(
                l.action,
                LedgerAction::ModAdd
                    | LedgerAction::ModRemove
                    | LedgerAction::Remove
                    | LedgerAction::Ban
                    | LedgerAction::Unban
                    | LedgerAction::Label
                    | LedgerAction::Pin
                    | LedgerAction::Unpin
                    | LedgerAction::Admit
                    | LedgerAction::Retire
            );
            if needs_target && l.target.is_none() {
                return Err(bad("ledger action needs a target"));
            }
            if (l.action == LedgerAction::DropKey) != l.drop_key.is_some() {
                return Err(bad("only a drop-key entry carries a drop key, and it must"));
            }
            if let Some(g) = &l.drop_key {
                let devices: std::collections::BTreeSet<&Id> = g.sealed.iter().map(|(d, _)| d).collect();
                if l.target.is_some()
                    || g.sealed.len() > MAX_DROP_KEY_DEVICES
                    || devices.len() != g.sealed.len()
                    || g.sealed.iter().any(|(_, x)| x.len() != hoi_crypto::envelope::DROP_KEY_LEN)
                    || !hoi_crypto::key_is_strong(&g.key)
                {
                    return Err(bad("a drop key is sealed once to each of at most 64 devices"));
                }
            }
            match l.action {
                LedgerAction::Rules if l.text.is_none() => Err(bad("rules entry without text")),
                LedgerAction::Rules if l.patch.as_ref().is_none_or(|p| p.is_empty() || p.len() > hoi_types::MAX_RULES_PATCH) => {
                    Err(bad("rules entry without its change (a patch of up to 64 KiB)"))
                }
                _ if l.action != LedgerAction::Rules && l.patch.is_some() => Err(bad("only a rules entry carries a patch")),
                LedgerAction::Limits => match &l.limits {
                    Some(x) => limits(x),
                    None => Err(bad("limits entry without limits")),
                },
                LedgerAction::Label if l.reason.as_deref().is_none_or(|s| s.trim().is_empty()) => Err(bad("label entry without a label")),
                _ => Ok(()),
            }
        }
        Body::Roster { .. } => {
            if r.expires_at > r.created_at + MAX_ROSTER_LIFE {
                return Err(bad("a roster lasts three days at most"));
            }
            Ok(())
        }
        Body::Pointer { of, media, sealed, .. } => {
            if !matches!(of, Kind::Post | Kind::Comment | Kind::Edit) || (media.is_some() && *of != Kind::Post) {
                return Err(bad("a pointer holds a post, a comment, an edit or a post's media"));
            }
            if sealed.is_empty() || sealed.len() > MAX_POINTER_SEALED {
                return Err(bad("a pointer's sealed manifest is too large"));
            }
            Ok(())
        }
        Body::Label { label, note } => {
            c(text::line(label, "label", 40, true))?;
            if let Some(n) = note {
                c(text::line(n, "note", 300, false))?;
            }
            Ok(())
        }
        Body::Report { reason, evidence } => {
            c(text::line(reason, "reason", 40, true))?;
            let ok = matches!(reason.as_str(), "spam" | "rate" | "dup" | "illegal") || reason.strip_prefix("rule:").is_some_and(|n| n.parse::<u32>().is_ok());
            if !ok || evidence.len() > 64 {
                return Err(bad("unknown report reason"));
            }
            Ok(())
        }
    }
}

/// Whether a record carries anything a newcomer may not share yet: links (PROTOCOL.md §15.3).
pub fn has_links(r: &Record) -> bool {
    match &r.body {
        Body::Post { link, text: t, title, .. } => link.is_some() || text::has_links(t) || text::has_links(title),
        Body::Comment { text: t, .. } => text::has_links(t),
        Body::Edit { text: t, title } => text::has_links(t) || title.as_deref().is_some_and(text::has_links),
        _ => false,
    }
}

pub fn has_media(r: &Record) -> bool {
    matches!(&r.body, Body::Post { media, .. } if !media.is_empty()) || matches!(&r.body, Body::Profile { image, .. } if !image.is_empty())
}

/// A fingerprint of a record's words for spotting copy-paste floods: lowercase, whitespace
/// collapsed, BLAKE3, first 16 bytes. `None` for kinds without free text.
pub fn text_fingerprint(r: &Record) -> Option<[u8; 16]> {
    let words = match &r.body {
        Body::Post { title, text: t, link, .. } => format!("{title}\n{t}\n{}", link.as_deref().unwrap_or("")),
        Body::Comment { text: t, .. } => t.clone(),
        _ => return None,
    };
    let norm: String = words.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    if norm.chars().count() < 12 {
        return None;
    }
    let h = blake3::hash(norm.as_bytes());
    Some(h.as_bytes()[..16].try_into().expect("16"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_crypto::Keypair;
    use hoi_types::{Body, MediaRef, Sig};
    use proptest::prelude::*;

    fn post(author: Id, title: &str, text: &str, now: u64) -> Record {
        Record {
            v: 1,
            community: Some(Id([1; 32])),
            author,
            parent: None,
            created_at: now,
            expires_at: now + 86_400,
            body: Body::Post { title: title.into(), text: text.into(), link: None, name: Some("lena".into()), media: Vec::new() },
            nonce: 0,
        }
    }

    #[test]
    fn full_pipeline_accepts_a_good_post() {
        let k = Keypair::generate().unwrap();
        let now = 1_790_000_000;
        let mut r = post(k.public(), "hoi", "say hi quietly", now);
        r.nonce = hoi_crypto::pow::mine(&r, 10);
        let (w, id) = k.sign_record(r);
        let d = decode(&w.encode()).unwrap();
        assert_eq!(d.id, id);
        signature(&d).unwrap();
        assert!(stamp(&d.id, d.wire.record.nonce, 10).unwrap() >= 10);
        time(&d.wire.record, now + 5, true, Some(86_400)).unwrap();
        content(&d.wire.record).unwrap();
    }

    #[test]
    fn content_rules() {
        let a = Id([3; 32]);
        assert!(content(&post(a, "x", "<script>alert(1)</script>", 0)).is_err());
        assert!(content(&post(a, "x", "look ![cat](https://x.example)", 0)).is_err());
        assert!(content(&post(a, "x", "2 < 3 and 5 > 4", 0)).is_ok());
        assert!(content(&post(a, " ", "x", 0)).is_err());
        let many = "https://a.example ".repeat(9);
        assert!(content(&post(a, "x", &many, 0)).is_err());
        assert!(content(&post(a, "x", "[win](javascript:alert(1))", 0)).is_err());
        assert!(content(&post(a, "evil\u{202E}gpj.exe", "x", 0)).is_err());
        let mut named = post(a, "x", "y", 0);
        named.body = Body::Post { title: "x".into(), text: "y".into(), link: Some("https://user@bank.example".into()), name: None, media: Vec::new() };
        assert!(content(&named).is_err());
    }

    #[test]
    fn new_kinds_content() {
        let a = Id([3; 32]);
        let mut r = post(a, "x", "y", 0);
        r.parent = Some(Id([4; 32]));
        let mut check = |b: Body| {
            r.body = b;
            content(&r)
        };
        assert!(check(Body::Vouch { value: 1, seq: 1, reason: None }).is_ok());
        assert!(check(Body::Vouch { value: -1, seq: 1, reason: Some("spam".into()) }).is_ok());
        assert!(check(Body::Vouch { value: -1, seq: 1, reason: Some("i disagree".into()) }).is_err());
        assert!(check(Body::Vouch { value: -1, seq: 1, reason: None }).is_err());
        assert!(check(Body::Vouch { value: 2, seq: 1, reason: None }).is_err());
        assert!(check(Body::Review { verdict: Verdict::Hide, seq: 1, reason: Some("rule:2".into()) }).is_ok());
        assert!(check(Body::Review { verdict: Verdict::Hide, seq: 1, reason: Some("wrong opinion".into()) }).is_err());
        assert!(check(Body::Review { verdict: Verdict::Veto, seq: 1, reason: Some("the post broke no rule".into()) }).is_ok());
        assert!(check(Body::Review { verdict: Verdict::Veto, seq: 1, reason: None }).is_err());
        assert!(check(Body::Review { verdict: Verdict::Ok, seq: 1, reason: None }).is_ok());
        let img = |w, h, mime: &str| MediaRef { hash: Id([9; 32]), mime: mime.into(), size: 1000, w, h, alt: None };
        let with = |m: Vec<MediaRef>| Body::Post { title: "t".into(), text: "".into(), link: None, name: None, media: m };
        assert!(check(with(vec![img(Some(10), Some(10), "image/png")])).is_ok());
        assert!(check(with(vec![img(None, None, "image/png")])).is_err(), "images state dimensions");
        assert!(check(with(vec![img(Some(10), Some(10), "image/svg+xml")])).is_err());
        assert!(check(with(vec![img(None, None, "video/mp4")])).is_ok());
        assert!(check(with(vec![img(Some(1), Some(1), "image/png"), img(Some(1), Some(1), "image/png")])).is_err(), "duplicate");
    }

    #[test]
    fn delegation_needs_both_keys() {
        let root = Keypair::generate().unwrap();
        let device = Keypair::generate().unwrap();
        let now = 1_790_000_000;
        let exp = now + 180 * DAY;
        let scope = vec!["*".to_string()];
        let mk = |sig: Sig, exp: u64| Record {
            v: 1,
            community: None,
            author: device.public(),
            parent: None,
            created_at: now,
            expires_at: exp,
            body: Body::Delegation { root: root.public(), scope: scope.clone(), expires_at: exp, root_sig: sig, seal: None },
            nonce: 0,
        };
        assert_eq!(delegation_consent(&mk(hoi_crypto::bless(&root, &device.public(), &scope, exp), exp)), Ok(root.public()));
        let thief = Keypair::generate().unwrap();
        assert_eq!(
            delegation_consent(&mk(hoi_crypto::bless(&thief, &device.public(), &scope, exp), exp)),
            Err(Reject::BadSignature),
            "someone else's blessing"
        );
        let far = now + 500 * DAY;
        assert!(delegation_consent(&mk(hoi_crypto::bless(&root, &device.public(), &scope, far), far)).is_err(), "too long");
    }

    #[test]
    fn identity_work_is_checked() {
        let k = Keypair::generate().unwrap();
        let mk = |work| Record {
            v: 1,
            community: None,
            author: k.public(),
            parent: None,
            created_at: 0,
            expires_at: KEEP,
            body: Body::Identity { work, rc: None, of: None },
            nonce: 0,
        };
        let w = hoi_crypto::identity::mine(&k.public(), 5);
        assert!(identity(&mk(w)).unwrap() >= 5);
        let weak = (0..200u64).find(|n| hoi_crypto::identity::work_bits(&k.public(), *n) < hoi_crypto::identity::MIN_BITS).unwrap();
        assert!(matches!(identity(&mk(weak)), Err(Reject::BadIdentity(_))));
    }

    #[test]
    fn time_rules() {
        let a = Id([3; 32]);
        let now = 1_790_000_000;
        assert_eq!(time(&post(a, "x", "y", now + 601), now, false, None), Err(Reject::FromTheFuture));
        assert_eq!(time(&post(a, "x", "y", now - 601), now, true, None), Err(Reject::TooOld));
        assert!(time(&post(a, "x", "y", now - 601), now, false, Some(86_400)).is_ok());
        assert_eq!(time(&post(a, "x", "y", now - 86_400), now, false, None), Err(Reject::Expired));
        assert_eq!(time(&post(a, "x", "y", now), now, false, Some(3600)), Err(Reject::ExpiryTooFar));
    }

    #[test]
    fn delegation_rules() {
        let root = Id([7; 32]);
        let r = post(Id([8; 32]), "x", "y", 1000);
        let d = DelegationInfo { root, scope: vec!["post".into()], expires_at: 5000, revoked_at: None };
        assert_eq!(delegation(&r, Some(&d)), Ok(root));
        assert_eq!(delegation(&r, None), Err(Reject::NoDelegation));
        let revoked = DelegationInfo { revoked_at: Some(900), ..d.clone() };
        assert_eq!(delegation(&r, Some(&revoked)), Err(Reject::Revoked));
        let voter = DelegationInfo { scope: vec!["vote".into()], ..d };
        assert_eq!(delegation(&r, Some(&voter)), Err(Reject::Scope));
    }

    #[test]
    fn windows_count_by_created_at() {
        // two a day: a third inside the same day fails wherever it lands in time
        let times = [1000, 5000];
        assert!(!window_ok(&times, 3000, DAY, 2));
        assert!(!window_ok(&times, 900, DAY, 2), "backdating doesn't dodge it");
        assert!(window_ok(&times, 5000 + DAY, DAY, 2));
        assert!(window_ok(&[], 0, DAY, 1));
        assert!(!window_ok(&[], 0, DAY, 0), "zero means none");
        assert_eq!(rate_windows(Kind::Post, true, 2, false), vec![(DAY, 2)]);
    }

    #[test]
    fn rate_bucket() {
        let mut b = RateBucket::full(2.0, 0);
        assert!(b.take(0, 2.0, 2.0));
        assert!(b.take(1, 2.0, 2.0));
        assert!(!b.take(2, 2.0, 2.0));
        assert!(b.take(43_300, 2.0, 2.0), "refills half a day later");
    }

    #[test]
    fn fingerprints_ignore_case_and_spacing() {
        let a = Id([3; 32]);
        let f1 = text_fingerprint(&post(a, "Free crypto", "click   here now", 0));
        let f2 = text_fingerprint(&post(a, "free CRYPTO", "click here\nnow", 5));
        assert!(f1.is_some() && f1 == f2);
        assert!(text_fingerprint(&post(a, "hi", "", 0)).is_none(), "too short to mean anything");
    }

    proptest! {
        #[test]
        fn tampering_is_caught(byte in 0usize..200, bit in 0u8..8) {
            let k = Keypair::from_secret(&[42; 32]);
            let r = post(k.public(), "hoi", "a signed note", 1_790_000_000);
            let (w, _) = k.sign_record(r);
            let mut b = w.encode();
            let i = byte % b.len();
            b[i] ^= 1 << bit;
            match decode(&b) {
                Err(_) => {}
                Ok(d) => prop_assert!(signature(&d).is_err() || d.wire.encode() == w.encode()),
            }
        }

        #[test]
        fn decode_never_panics(b in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = decode(&b);
        }

        #[test]
        fn window_never_exceeds(ts in proptest::collection::vec(0u64..200_000, 0..30), max in 1u32..5) {
            // greedily accept what fits, then no day-long span holds more than max
            let mut kept: Vec<u64> = Vec::new();
            for t in ts {
                if window_ok(&kept, t, DAY, max) {
                    kept.push(t);
                }
            }
            kept.sort_unstable();
            for (i, s) in kept.iter().enumerate() {
                prop_assert!(kept[i..].iter().take_while(|x| **x < s + DAY).count() <= max as usize);
            }
        }
    }

    #[test]
    fn scatter_levels_are_the_three_the_protocol_knows() {
        for resist in [10, 15, 20] {
            let l = hoi_types::Limits { scatter: hoi_types::Scatter { resist, open: false }, ..hoi_types::Limits::default() };
            assert!(limits(&l).is_ok(), "{resist}");
        }
        for resist in [0, 5, 9, 12, 25, 100] {
            let l = hoi_types::Limits { scatter: hoi_types::Scatter { resist, open: false }, ..hoi_types::Limits::default() };
            assert!(limits(&l).is_err(), "{resist} is below the floor or not a level");
        }
    }
}
