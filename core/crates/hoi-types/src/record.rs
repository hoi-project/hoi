//! The record envelope and kinds (PROTOCOL.md §2, §3, §5, §6).
//!
//! Wire field names are the protocol's snake_case names. Optional fields are omitted when absent.

use std::collections::BTreeMap;

use crate::cbor::{self, Value};
use crate::{Id, Sig, TypesError};

pub const PROTOCOL_VERSION: u64 = 1;

/// Difficulty for records that belong to no community (genesis, delegation, revoke, identity).
/// Not in PROTOCOL.md 0.1.
pub const GLOBAL_POW_BITS: u32 = 16;

/// Media items a post may carry (PROTOCOL.md §17).
pub const MAX_MEDIA: usize = 4;
/// Small, signed per-community profile pictures. Empty bytes restore the identicon.
pub const MAX_AVATAR_BYTES: usize = 4096;
pub const MAX_AVATAR_SIDE: u32 = 128;

/// Empty restores the key-derived color. Values are palette names, never CSS or URLs.
pub fn is_avatar_color(color: &str) -> bool {
    matches!(color, "" | "mint" | "sky" | "lilac" | "peach" | "sun")
}

/// Reasons a member may give for hiding a record or objecting to a person (PROTOCOL.md §16).
/// Disagreement is not one of them: that is what a downvote is for.
pub fn is_hide_reason(r: &str) -> bool {
    matches!(r, "spam" | "abuse" | "illegal" | "off-topic")
        || r.strip_prefix("rule:").is_some_and(|n| !n.is_empty() && n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Profile,
    ProfileColor,
    Post,
    Comment,
    Vote,
    Delete,
    Delegation,
    Revoke,
    Ledger,
    Label,
    Report,
    Community,
    Edit,
    Identity,
    Vouch,
    Review,
    /// A post, comment or edit kept in pieces in an open community (SCATTER.md S5): what syncs
    /// in its place.
    Pointer,
    /// The holders of pieces a member's durable device saw up for days, published by choice so
    /// the community's other devices can count them (SCATTER.md §5.7).
    Roster,
    /// A recovery key replacing a person's root with a new one.
    Rotate,
    /// One share of a person's recovery line, sealed to a friend's devices.
    Share,
    /// Whom a person trusts, published in a community by choice (M3).
    Trust,
    /// One edit to a page of a community's wiki: a Loro update (M3).
    Wiki,
    /// An aggregator's vote counts for a community's posts (M5).
    Tally,
    /// A list keeper's media hashes and picture fingerprints others may follow.
    Blocklist,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Profile => "profile",
            Kind::ProfileColor => "profile-color",
            Kind::Post => "post",
            Kind::Comment => "comment",
            Kind::Vote => "vote",
            Kind::Delete => "delete",
            Kind::Delegation => "delegation",
            Kind::Revoke => "revoke",
            Kind::Ledger => "ledger",
            Kind::Label => "label",
            Kind::Report => "report",
            Kind::Community => "community",
            Kind::Edit => "edit",
            Kind::Identity => "identity",
            Kind::Vouch => "vouch",
            Kind::Review => "review",
            Kind::Pointer => "pointer",
            Kind::Roster => "roster",
            Kind::Rotate => "rotate",
            Kind::Share => "share",
            Kind::Trust => "trust",
            Kind::Wiki => "wiki",
            Kind::Tally => "tally",
            Kind::Blocklist => "blocklist",
        }
    }

    pub fn parse(s: &str) -> Result<Kind, TypesError> {
        Ok(match s {
            "profile" => Kind::Profile,
            "profile-color" => Kind::ProfileColor,
            "post" => Kind::Post,
            "comment" => Kind::Comment,
            "vote" => Kind::Vote,
            "delete" => Kind::Delete,
            "delegation" => Kind::Delegation,
            "revoke" => Kind::Revoke,
            "ledger" => Kind::Ledger,
            "label" => Kind::Label,
            "report" => Kind::Report,
            "community" => Kind::Community,
            "edit" => Kind::Edit,
            "identity" => Kind::Identity,
            "vouch" => Kind::Vouch,
            "review" => Kind::Review,
            "pointer" => Kind::Pointer,
            "roster" => Kind::Roster,
            "rotate" => Kind::Rotate,
            "share" => Kind::Share,
            "trust" => Kind::Trust,
            "wiki" => Kind::Wiki,
            "tally" => Kind::Tally,
            "blocklist" => Kind::Blocklist,
            other => return Err(TypesError::Field(format!("unknown kind {other:?}"))),
        })
    }

    /// Max encoded wire size (PROTOCOL.md §2).
    pub fn max_size(&self) -> usize {
        match self {
            Kind::Post | Kind::Ledger | Kind::Community | Kind::Edit => 64 * 1024,
            Kind::Profile => 5 * 1024,
            Kind::Comment => 16 * 1024,
            Kind::ProfileColor | Kind::Vote | Kind::Label | Kind::Delete | Kind::Revoke | Kind::Identity | Kind::Vouch | Kind::Rotate => 512,
            Kind::Review => 1024,
            Kind::Delegation | Kind::Report | Kind::Roster => 2 * 1024,
            Kind::Share => 4 * 1024,
            Kind::Trust => 10 * 1024,
            Kind::Wiki => MAX_WIKI_UPDATE + 1024,
            Kind::Tally => 32 * 1024,
            Kind::Blocklist => 72 * 1024,
            Kind::Pointer => 16 * 1024,
        }
    }

    /// Stamp discount relative to the community's `pow_bits` (PROTOCOL.md §4: votes and labels
    /// use pow_bits - 8; we extend that to other small signed actions).
    pub fn pow_discount(&self) -> u32 {
        match self {
            Kind::Profile
            | Kind::ProfileColor
            | Kind::Vote
            | Kind::Label
            | Kind::Ledger
            | Kind::Report
            | Kind::Delete
            | Kind::Vouch
            | Kind::Review
            | Kind::Roster
            | Kind::Share
            | Kind::Trust
            | Kind::Wiki
            | Kind::Tally
            | Kind::Blocklist => 8,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Admission {
    Open,
    Invite,
    AgeProof,
    Approve,
}

impl Admission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Admission::Open => "open",
            Admission::Invite => "invite",
            Admission::AgeProof => "age-proof",
            Admission::Approve => "approve",
        }
    }
    pub fn parse(s: &str) -> Result<Admission, TypesError> {
        Ok(match s {
            "open" => Admission::Open,
            "invite" => Admission::Invite,
            "age-proof" => Admission::AgeProof,
            "approve" => Admission::Approve,
            o => return Err(TypesError::Field(format!("unknown admission {o:?}"))),
        })
    }
}

/// The trust ladder (PROTOCOL.md §15, §16): how newcomers earn their way up, and how the
/// community, not a moderator alone, decides what gets hidden and who gets blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ladder {
    /// Memory-hard identity work every author must carry (PROTOCOL.md §4.1), in bits.
    pub identity_bits: u32,
    /// Vouches from established members that turn a newcomer into a member.
    pub admit_vouches: u32,
    /// Seconds a member waits before they count as established (and may vouch, review, veto).
    pub established_age: u64,
    /// Live contributions in the last 90 days an established member needs to be trusted (media).
    pub trusted_posts: u32,
    /// Response score over those contributions needed to be trusted (media).
    pub media_score: u32,
    /// Established members whose hides put a record to sleep for everyone.
    pub hide_quorum: u32,
    /// Distinct established hiders that block a member from the community.
    pub block_quorum: u32,
    /// Established members whose vetoes overturn a moderator's or owner's action.
    pub veto_quorum: u32,
    /// Seconds a removal waits (napping, contestable) before its body is deleted.
    pub contest_window: u64,
}

impl Default for Ladder {
    fn default() -> Self {
        Ladder {
            identity_bits: 8,
            admit_vouches: 2,
            established_age: 14 * 86_400,
            trusted_posts: 5,
            media_score: 10,
            hide_quorum: 3,
            block_quorum: 5,
            veto_quorum: 5,
            contest_window: 72 * 3600,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub pow_bits: u32,
    pub max_retention: u64,
    pub newcomer_posts_per_day: u32,
    pub newcomer_age: u64,
    pub admission: Admission,
    pub autoban_threshold: u32,
    pub labelers: Vec<Id>,
    pub ladder: Ladder,
    pub scatter: Scatter,
    /// Trackers the community names: https addresses of registries where public nodes
    /// carrying it announce themselves, asked by devices when no route they hold answers. At
    /// most [`crate::uri::MAX_TRACKERS`]; chosen by the community, never by an app or a site.
    pub trackers: Vec<String>,
    /// Age-proof admission: the public keys of the credential issuers the community
    /// trusts, raw: 32 bytes Ed25519 (EdDSA) or 33 or 65 bytes P-256 SEC1 (ES256). At most 8.
    pub age_issuers: Vec<Vec<u8>>,
}

/// Credential issuers one community names at most.
pub const MAX_AGE_ISSUERS: usize = 8;

/// A credential issuer's key as `limits.age_issuers` holds it.
pub fn age_issuer_ok(k: &[u8]) -> bool {
    matches!(k.len(), 32 | 33 | 65)
}

/// Scattered delivery (SCATTER.md §6): how much of the places a post's pieces go to a party may
/// run without being able to rebuild or withhold it, and whether an open community's posts are
/// scattered at all. Shaped like `ladder.identity_bits`: the protocol keeps a floor, communities
/// may ask for more by consent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scatter {
    /// Percent of places: one of `scatter::LEVELS` (10, 15, 20).
    pub resist: u8,
    /// Open communities only: whether post bodies travel in pieces (phase S5).
    pub open: bool,
}

impl Default for Scatter {
    fn default() -> Self {
        Scatter { resist: crate::scatter::LEVELS[0], open: false }
    }
}

impl Scatter {
    pub fn to_value(&self) -> Value {
        Value::map([("resist", Value::U(self.resist as u64)), ("open", Value::Bool(self.open))])
    }

    pub fn from_value(v: Value) -> Result<Scatter, TypesError> {
        let mut f = Fields::new(v, "scatter")?;
        let resist = f.u32("resist")?;
        let open = match f.take("open")? {
            Value::Bool(b) => b,
            _ => return Err(TypesError::Field("scatter: open must be true or false".into())),
        };
        f.done()?;
        let resist = u8::try_from(resist).map_err(|_| TypesError::Field("scatter: resist out of range".into()))?;
        Ok(Scatter { resist, open })
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            pow_bits: 22,
            max_retention: 15_552_000,
            newcomer_posts_per_day: 2,
            newcomer_age: 2_592_000,
            admission: Admission::Open,
            autoban_threshold: 12,
            labelers: Vec::new(),
            ladder: Ladder::default(),
            scatter: Scatter::default(),
            trackers: Vec::new(),
            age_issuers: Vec::new(),
        }
    }
}

/// A media item a post carries (PROTOCOL.md §17). The bytes travel separately, by hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
    /// BLAKE3 of the (already sanitized) bytes.
    pub hash: Id,
    pub mime: String,
    pub size: u64,
    pub w: Option<u32>,
    pub h: Option<u32>,
    pub alt: Option<String>,
}

/// A member's judgement of a record (PROTOCOL.md §16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// Read it, it belongs here (posts, comments) or supports a ledger entry.
    Ok,
    /// Hide it from the community, with a reason.
    Hide,
    /// Overturn a ledger entry, with a reason.
    Veto,
    /// Takes back an earlier verdict.
    None,
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Hide => "hide",
            Verdict::Veto => "veto",
            Verdict::None => "none",
        }
    }
    pub fn parse(s: &str) -> Result<Verdict, TypesError> {
        Ok(match s {
            "ok" => Verdict::Ok,
            "hide" => Verdict::Hide,
            "veto" => Verdict::Veto,
            "none" => Verdict::None,
            o => return Err(TypesError::Field(format!("unknown verdict {o:?}"))),
        })
    }
}

/// Holders a roster names at most.
pub const MAX_ROSTER: usize = 32;
/// People one trust statement names at most.
pub const MAX_TRUST: usize = 256;
/// One wiki edit's Loro update at most, and a page name's length.
pub const MAX_WIKI_UPDATE: usize = 32 * 1024;
pub const MAX_WIKI_PAGE: usize = 64;
/// Hashes of each kind one blocklist holds at most.
pub const MAX_BLOCKLIST: usize = 1000;
/// Posts one tally counts at most.
pub const MAX_TALLY: usize = 500;
/// A rules entry's Loro patch at most.
pub const MAX_RULES_PATCH: usize = 64 * 1024;

/// A wiki page's name: 1 to 64 of `a-z`, `0-9` and `-`, not starting or ending with `-`.
pub fn is_wiki_page(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_WIKI_PAGE
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Genesis {
    pub name: String,
    pub description: String,
    pub color: String,
    pub owners: Vec<Id>,
    pub threshold: u32,
    pub rules_text_cid: Option<Id>,
    pub limits: Limits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LedgerAction {
    Rules,
    Limits,
    ModAdd,
    ModRemove,
    Remove,
    Ban,
    Unban,
    Label,
    Pin,
    Unpin,
    Admit,
    /// An open community's drop key (SCATTER.md S5): its public part, and its secret sealed to
    /// the seal key in each owner's and moderator's device's delegation.
    DropKey,
    /// Owners say an elector left for good: after a notice, their silence no longer
    /// holds up decisions. Void once they sign anything again.
    Retire,
}

impl LedgerAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            LedgerAction::Rules => "rules",
            LedgerAction::Limits => "limits",
            LedgerAction::ModAdd => "mod-add",
            LedgerAction::ModRemove => "mod-remove",
            LedgerAction::Remove => "remove",
            LedgerAction::Ban => "ban",
            LedgerAction::Unban => "unban",
            LedgerAction::Label => "label",
            LedgerAction::Pin => "pin",
            LedgerAction::Unpin => "unpin",
            LedgerAction::Admit => "admit",
            LedgerAction::DropKey => "drop-key",
            LedgerAction::Retire => "retire",
        }
    }

    pub fn parse(s: &str) -> Result<LedgerAction, TypesError> {
        Ok(match s {
            "rules" => LedgerAction::Rules,
            "limits" => LedgerAction::Limits,
            "mod-add" => LedgerAction::ModAdd,
            "mod-remove" => LedgerAction::ModRemove,
            "remove" => LedgerAction::Remove,
            "ban" => LedgerAction::Ban,
            "unban" => LedgerAction::Unban,
            "label" => LedgerAction::Label,
            "pin" => LedgerAction::Pin,
            "unpin" => LedgerAction::Unpin,
            "admit" => LedgerAction::Admit,
            "drop-key" => LedgerAction::DropKey,
            "retire" => LedgerAction::Retire,
            o => return Err(TypesError::Field(format!("unknown ledger action {o:?}"))),
        })
    }

    /// Needs owner signatures reaching the community threshold (PROTOCOL.md §6).
    pub fn needs_threshold(&self) -> bool {
        matches!(self, LedgerAction::Rules | LedgerAction::Limits | LedgerAction::ModAdd | LedgerAction::ModRemove | LedgerAction::Retire)
    }

    /// Kept until superseded instead of expiring with the retention window.
    pub fn kept_until_superseded(&self) -> bool {
        self.needs_threshold() || *self == LedgerAction::DropKey
    }

    /// Established members may overturn these by veto (PROTOCOL.md §16.3). Rules and limits
    /// need the owners' threshold and are not vetoable; neither is taking a moderator away.
    pub fn vetoable(&self) -> bool {
        matches!(
            self,
            LedgerAction::Remove
                | LedgerAction::Ban
                | LedgerAction::Unban
                | LedgerAction::Label
                | LedgerAction::Pin
                | LedgerAction::Admit
                | LedgerAction::ModAdd
                | LedgerAction::Retire
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cosig {
    pub key: Id,
    pub sig: Sig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerBody {
    pub seq: u64,
    pub action: LedgerAction,
    pub target: Option<Id>,
    pub reason: Option<String>,
    pub evidence: Option<Vec<Id>>,
    pub cosign: Option<Vec<Cosig>>,
    /// Full rules text for `rules`.
    pub text: Option<String>,
    /// New limits for `limits`.
    pub limits: Option<Limits>,
    /// The key and its sealed secrets for `drop-key` (SCATTER.md S5).
    pub drop_key: Option<DropKeyGrant>,
    /// For `rules`: the change as a Loro update to the rules document.
    pub patch: Option<Vec<u8>>,
}

/// An open community's drop key as a `drop-key` entry carries it: the Ed25519 public part, and
/// its 32-byte secret sealed to the seal key of each device it is for (HPKE, 80 bytes each).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropKeyGrant {
    pub key: Id,
    pub sealed: Vec<(Id, Vec<u8>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// Optional identicon palette, independent of photo permissions and revisions.
    ProfileColor {
        color: String,
        seq: u64,
    },
    /// JPEG bytes scoped to this community; empty restores the public-key identicon.
    Profile {
        image: Vec<u8>,
        seq: u64,
    },
    /// `name` is the author's self-chosen display name.
    Post {
        title: String,
        text: String,
        link: Option<String>,
        name: Option<String>,
        /// Empty means absent on the wire (PROTOCOL.md §17).
        media: Vec<MediaRef>,
    },
    Comment {
        text: String,
        name: Option<String>,
    },
    /// `value` 1 or -1; 0 retracts.
    Vote {
        value: i64,
        seq: u64,
    },
    Edit {
        text: String,
        title: Option<String>,
    },
    Delete,
    /// Signed by the device key (its consent); `root_sig` is the root's blessing over
    /// [`delegation_statement`].
    Delegation {
        root: Id,
        scope: Vec<String>,
        expires_at: u64,
        root_sig: Sig,
        /// An X25519 key this device takes an open community's drop key with (SCATTER.md S5),
        /// derived from its device key for nothing else. Signed by the device, not the root.
        seal: Option<Id>,
    },
    /// Signed by a root. `device` = the root itself burns the identity for good.
    Revoke {
        device: Id,
    },
    /// A root key's birth certificate: `work` is the nonce of its memory-hard identity work.
    /// `rc` commits to the recovery key that may replace this root (see
    /// `hoi_crypto::recovery::commitment`); `of` names the person a root continues, for a root
    /// that replaces an earlier one.
    Identity {
        work: u64,
        rc: Option<Id>,
        of: Option<Id>,
    },
    /// Signed by the recovery key the current root of `person` committed to: `to` (a root whose
    /// identity names `person` in `of`) takes over from it.
    Rotate {
        person: Id,
        to: Id,
    },
    /// `parent` is the friend who keeps one share of the author's recovery line; `sealed` is the
    /// share sealed to each of the friend's devices (device, sealed). Latest `seq` per (author,
    /// friend) wins; an empty `sealed` takes it back.
    Share {
        sealed: Vec<(Id, Vec<u8>)>,
        seq: u64,
    },
    /// `parent` is the person (root key) vouched for. 1 vouches, -1 objects (with a reason),
    /// 0 takes it back. Latest `seq` per (voucher, person) wins.
    Vouch {
        value: i64,
        seq: u64,
        reason: Option<String>,
    },
    /// `parent` is the record judged. Latest `seq` per (reviewer, record) wins.
    /// `record`: the post, comment or edit (`of`) the object holds, or the post whose media
    /// item `media` (a part of) it holds; `sealed`: the pointer (`scatter::Pointer`) sealed
    /// under the community's read key, so nodes that never joined cannot open it.
    Pointer {
        record: Id,
        of: Kind,
        media: Option<Id>,
        sealed: Vec<u8>,
    },
    /// The endpoints of holders the author's device saw up for days, first hand: 1 to 32,
    /// sorted, each once. The latest per device counts.
    Roster {
        holders: Vec<Id>,
    },
    /// The people the author trusts, as person ids: 0 to 256, sorted, each once. The latest per
    /// person and community counts; an empty one takes the last one back.
    Trust {
        keys: Vec<Id>,
    },
    /// An edit to the wiki page `page`: a Loro update to the page's document.
    Wiki {
        page: String,
        update: Vec<u8>,
    },
    /// An aggregator's counts: (post, up-votes, down-votes), sorted by post, each once, at most
    /// 500. The latest per author and community counts.
    Tally {
        counts: Vec<(Id, u64, u64)>,
    },
    /// A list keeper's list: exact media hashes (BLAKE3) and picture fingerprints (PDQ), each
    /// sorted, each once, at most 1000 of each. The latest per author and community counts.
    Blocklist {
        exact: Vec<Id>,
        pdq: Vec<Id>,
    },
    Review {
        verdict: Verdict,
        seq: u64,
        reason: Option<String>,
    },
    Community(Genesis),
    Ledger(LedgerBody),
    Label {
        label: String,
        note: Option<String>,
    },
    Report {
        reason: String,
        evidence: Vec<Id>,
    },
}

impl Body {
    pub fn kind(&self) -> Kind {
        match self {
            Body::Profile { .. } => Kind::Profile,
            Body::ProfileColor { .. } => Kind::ProfileColor,
            Body::Post { .. } => Kind::Post,
            Body::Comment { .. } => Kind::Comment,
            Body::Vote { .. } => Kind::Vote,
            Body::Edit { .. } => Kind::Edit,
            Body::Delete => Kind::Delete,
            Body::Delegation { .. } => Kind::Delegation,
            Body::Revoke { .. } => Kind::Revoke,
            Body::Community(_) => Kind::Community,
            Body::Ledger(_) => Kind::Ledger,
            Body::Label { .. } => Kind::Label,
            Body::Report { .. } => Kind::Report,
            Body::Identity { .. } => Kind::Identity,
            Body::Vouch { .. } => Kind::Vouch,
            Body::Review { .. } => Kind::Review,
            Body::Pointer { .. } => Kind::Pointer,
            Body::Roster { .. } => Kind::Roster,
            Body::Rotate { .. } => Kind::Rotate,
            Body::Share { .. } => Kind::Share,
            Body::Trust { .. } => Kind::Trust,
            Body::Wiki { .. } => Kind::Wiki,
            Body::Tally { .. } => Kind::Tally,
            Body::Blocklist { .. } => Kind::Blocklist,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub v: u64,
    pub community: Option<Id>,
    pub author: Id,
    pub parent: Option<Id>,
    pub created_at: u64,
    pub expires_at: u64,
    pub body: Body,
    pub nonce: u64,
}

// ------------------------------------------------------------------ to Value

fn b32(id: &Id) -> Value {
    Value::B(id.0.to_vec())
}
fn t(s: &str) -> Value {
    Value::T(s.to_string())
}
fn ids(v: &[Id]) -> Value {
    Value::A(v.iter().map(b32).collect())
}

impl Limits {
    pub fn to_value(&self) -> Value {
        let mut e = vec![
            ("pow_bits", Value::U(self.pow_bits as u64)),
            ("max_retention", Value::U(self.max_retention)),
            ("newcomer_posts_per_day", Value::U(self.newcomer_posts_per_day as u64)),
            ("newcomer_age", Value::U(self.newcomer_age)),
            ("admission", t(self.admission.as_str())),
            ("autoban_threshold", Value::U(self.autoban_threshold as u64)),
            ("labelers", ids(&self.labelers)),
            ("ladder", self.ladder.to_value()),
        ];
        // Written only when it differs from the default, so communities on the default stay
        // readable by releases from before it existed (their decoders refuse unknown fields).
        if self.scatter != Scatter::default() {
            e.push(("scatter", self.scatter.to_value()));
        }
        if !self.trackers.is_empty() {
            e.push(("trackers", Value::A(self.trackers.iter().cloned().map(Value::T).collect())));
        }
        if !self.age_issuers.is_empty() {
            e.push(("age_issuers", Value::A(self.age_issuers.iter().cloned().map(Value::B).collect())));
        }
        Value::map(e)
    }

    pub fn from_value(v: Value) -> Result<Limits, TypesError> {
        let mut f = Fields::new(v, "limits")?;
        let l = Limits {
            pow_bits: f.u32("pow_bits")?,
            max_retention: f.u64("max_retention")?,
            newcomer_posts_per_day: f.u32("newcomer_posts_per_day")?,
            newcomer_age: f.u64("newcomer_age")?,
            admission: Admission::parse(&f.text("admission")?)?,
            autoban_threshold: f.u32("autoban_threshold")?,
            labelers: f.ids("labelers")?,
            ladder: Ladder::from_value(f.take("ladder")?)?,
            scatter: match f.opt("scatter") {
                None => Scatter::default(),
                // The default is never written, so it is never accepted written either: one
                // encoding per value keeps ids and signatures stable.
                Some(v) => match Scatter::from_value(v)? {
                    s if s == Scatter::default() => return Err(TypesError::Field("limits: the default scatter setting is left out".into())),
                    s => s,
                },
            },
            trackers: match f.opt("trackers") {
                None => Vec::new(),
                Some(Value::A(v)) if !v.is_empty() && v.len() <= crate::uri::MAX_TRACKERS => v
                    .into_iter()
                    .map(|t| match t {
                        Value::T(s) if crate::uri::tracker_ok(&s) => Ok(s),
                        _ => Err(TypesError::Field("limits: a tracker is an https address".into())),
                    })
                    .collect::<Result<_, _>>()?,
                Some(_) => return Err(TypesError::Field(format!("limits: one to {} trackers, or none written", crate::uri::MAX_TRACKERS))),
            },
            age_issuers: match f.opt("age_issuers") {
                None => Vec::new(),
                Some(Value::A(v)) if !v.is_empty() && v.len() <= MAX_AGE_ISSUERS => v
                    .into_iter()
                    .map(|k| match k {
                        Value::B(b) if age_issuer_ok(&b) => Ok(b),
                        _ => Err(TypesError::Field("limits: an age issuer is a 32, 33 or 65 byte key".into())),
                    })
                    .collect::<Result<_, _>>()?,
                Some(_) => return Err(TypesError::Field(format!("limits: one to {MAX_AGE_ISSUERS} age issuers, or none written"))),
            },
        };
        f.done()?;
        Ok(l)
    }
}

impl Ladder {
    pub fn to_value(&self) -> Value {
        Value::map([
            ("identity_bits", Value::U(self.identity_bits as u64)),
            ("admit_vouches", Value::U(self.admit_vouches as u64)),
            ("established_age", Value::U(self.established_age)),
            ("trusted_posts", Value::U(self.trusted_posts as u64)),
            ("media_score", Value::U(self.media_score as u64)),
            ("hide_quorum", Value::U(self.hide_quorum as u64)),
            ("block_quorum", Value::U(self.block_quorum as u64)),
            ("veto_quorum", Value::U(self.veto_quorum as u64)),
            ("contest_window", Value::U(self.contest_window)),
        ])
    }

    pub fn from_value(v: Value) -> Result<Ladder, TypesError> {
        let mut f = Fields::new(v, "ladder")?;
        let l = Ladder {
            identity_bits: f.u32("identity_bits")?,
            admit_vouches: f.u32("admit_vouches")?,
            established_age: f.u64("established_age")?,
            trusted_posts: f.u32("trusted_posts")?,
            media_score: f.u32("media_score")?,
            hide_quorum: f.u32("hide_quorum")?,
            block_quorum: f.u32("block_quorum")?,
            veto_quorum: f.u32("veto_quorum")?,
            contest_window: f.u64("contest_window")?,
        };
        f.done()?;
        Ok(l)
    }
}

impl MediaRef {
    pub fn to_value(&self) -> Value {
        let mut e = vec![("hash", b32(&self.hash)), ("mime", t(&self.mime)), ("size", Value::U(self.size))];
        if let Some(w) = self.w {
            e.push(("w", Value::U(w as u64)));
        }
        if let Some(h) = self.h {
            e.push(("h", Value::U(h as u64)));
        }
        if let Some(a) = &self.alt {
            e.push(("alt", t(a)));
        }
        Value::map(e)
    }

    pub fn from_value(v: Value) -> Result<MediaRef, TypesError> {
        let mut f = Fields::new(v, "media")?;
        let m = MediaRef { hash: f.id("hash")?, mime: f.text("mime")?, size: f.u64("size")?, w: f.opt_u32("w")?, h: f.opt_u32("h")?, alt: f.opt_text("alt")? };
        f.done()?;
        Ok(m)
    }
}

/// What a root signs to bless a device: the BLAKE3 of
/// `canonical_cbor({t: "hoi/delegation/1", root, device, scope, expires_at})`.
pub fn delegation_statement(root: &Id, device: &Id, scope: &[String], expires_at: u64) -> Id {
    let v = Value::map([
        ("t", t("hoi/delegation/1")),
        ("root", b32(root)),
        ("device", b32(device)),
        ("scope", Value::A(scope.iter().map(|s| t(s)).collect())),
        ("expires_at", Value::U(expires_at)),
    ]);
    Id::hash(&v.encode())
}

impl Body {
    pub fn to_value(&self) -> Value {
        let mut e: Vec<(&str, Value)> = Vec::new();
        match self {
            Body::ProfileColor { color, seq } => {
                e.push(("color", t(color)));
                e.push(("seq", Value::U(*seq)));
            }
            Body::Profile { image, seq } => {
                e.push(("image", Value::B(image.clone())));
                e.push(("seq", Value::U(*seq)));
            }
            Body::Post { title, text, link, name, media } => {
                e.push(("title", t(title)));
                e.push(("text", t(text)));
                if let Some(l) = link {
                    e.push(("link", t(l)));
                }
                if let Some(n) = name {
                    e.push(("name", t(n)));
                }
                if !media.is_empty() {
                    e.push(("media", Value::A(media.iter().map(MediaRef::to_value).collect())));
                }
            }
            Body::Comment { text, name } => {
                e.push(("text", t(text)));
                if let Some(n) = name {
                    e.push(("name", t(n)));
                }
            }
            Body::Vote { value, seq } => {
                e.push(("value", Value::int(*value)));
                e.push(("seq", Value::U(*seq)));
            }
            Body::Edit { text, title } => {
                e.push(("text", t(text)));
                if let Some(ti) = title {
                    e.push(("title", t(ti)));
                }
            }
            Body::Delete => {}
            Body::Delegation { root, scope, expires_at, root_sig, seal } => {
                e.push(("root", b32(root)));
                e.push(("scope", Value::A(scope.iter().map(|s| t(s)).collect())));
                e.push(("expires_at", Value::U(*expires_at)));
                e.push(("root_sig", Value::B(root_sig.0.to_vec())));
                if let Some(k) = seal {
                    e.push(("seal", b32(k)));
                }
            }
            Body::Pointer { record, of, media, sealed } => {
                e.push(("record", b32(record)));
                e.push(("of", t(of.as_str())));
                if let Some(m) = media {
                    e.push(("media", b32(m)));
                }
                e.push(("sealed", Value::B(sealed.clone())));
            }
            Body::Roster { holders } => e.push(("holders", ids(holders))),
            Body::Trust { keys } => e.push(("keys", ids(keys))),
            Body::Wiki { page, update } => {
                e.push(("page", t(page)));
                e.push(("update", Value::B(update.clone())));
            }
            Body::Blocklist { exact, pdq } => {
                e.push(("exact", ids(exact)));
                e.push(("pdq", ids(pdq)));
            }
            Body::Tally { counts } => {
                e.push(("counts", Value::A(counts.iter().map(|(p, u, d)| Value::A(vec![b32(p), Value::U(*u), Value::U(*d)])).collect())));
            }
            Body::Revoke { device } => e.push(("device", b32(device))),
            Body::Identity { work, rc, of } => {
                e.push(("work", Value::U(*work)));
                if let Some(rc) = rc {
                    e.push(("rc", b32(rc)));
                }
                if let Some(of) = of {
                    e.push(("of", b32(of)));
                }
            }
            Body::Rotate { person, to } => {
                e.push(("person", b32(person)));
                e.push(("to", b32(to)));
            }
            Body::Share { sealed, seq } => {
                e.push(("sealed", Value::A(sealed.iter().map(|(d, s)| Value::map([("d", b32(d)), ("s", Value::B(s.clone()))])).collect())));
                e.push(("seq", Value::U(*seq)));
            }
            Body::Vouch { value, seq, reason } => {
                e.push(("value", Value::int(*value)));
                e.push(("seq", Value::U(*seq)));
                if let Some(r) = reason {
                    e.push(("reason", t(r)));
                }
            }
            Body::Review { verdict, seq, reason } => {
                e.push(("verdict", t(verdict.as_str())));
                e.push(("seq", Value::U(*seq)));
                if let Some(r) = reason {
                    e.push(("reason", t(r)));
                }
            }
            Body::Community(g) => {
                e.push(("name", t(&g.name)));
                e.push(("description", t(&g.description)));
                e.push(("color", t(&g.color)));
                e.push(("owners", ids(&g.owners)));
                e.push(("threshold", Value::U(g.threshold as u64)));
                if let Some(c) = &g.rules_text_cid {
                    e.push(("rules_text_cid", b32(c)));
                }
                e.push(("limits", g.limits.to_value()));
            }
            Body::Ledger(l) => {
                e.push(("seq", Value::U(l.seq)));
                e.push(("action", t(l.action.as_str())));
                if let Some(x) = &l.target {
                    e.push(("target", b32(x)));
                }
                if let Some(x) = &l.reason {
                    e.push(("reason", t(x)));
                }
                if let Some(x) = &l.evidence {
                    e.push(("evidence", ids(x)));
                }
                if let Some(x) = &l.cosign {
                    e.push(("cosign", Value::A(x.iter().map(|c| Value::map([("key", b32(&c.key)), ("sig", Value::B(c.sig.0.to_vec()))])).collect())));
                }
                if let Some(x) = &l.text {
                    e.push(("text", t(x)));
                }
                if let Some(x) = &l.limits {
                    e.push(("limits", x.to_value()));
                }
                if let Some(x) = &l.drop_key {
                    let to = x.sealed.iter().map(|(d, s)| Value::map([("d", b32(d)), ("s", Value::B(s.clone()))])).collect();
                    e.push(("drop_key", Value::map([("key", b32(&x.key)), ("to", Value::A(to))])));
                }
                if let Some(x) = &l.patch {
                    e.push(("patch", Value::B(x.clone())));
                }
            }
            Body::Label { label, note } => {
                e.push(("label", t(label)));
                if let Some(n) = note {
                    e.push(("note", t(n)));
                }
            }
            Body::Report { reason, evidence } => {
                e.push(("reason", t(reason)));
                e.push(("evidence", ids(evidence)));
            }
        }
        Value::map(e)
    }

    pub fn from_value(kind: Kind, v: Value) -> Result<Body, TypesError> {
        let mut f = Fields::new(v, kind.as_str())?;
        let b = match kind {
            Kind::Profile => Body::Profile { image: f.bytes("image")?, seq: f.u64("seq")? },
            Kind::ProfileColor => Body::ProfileColor { color: f.text("color")?, seq: f.u64("seq")? },
            Kind::Post => Body::Post {
                title: f.text("title")?,
                text: f.text("text")?,
                link: f.opt_text("link")?,
                name: f.opt_text("name")?,
                media: match f.opt("media") {
                    None => Vec::new(),
                    Some(Value::A(items)) if !items.is_empty() => items.into_iter().map(MediaRef::from_value).collect::<Result<_, _>>()?,
                    Some(_) => return Err(TypesError::Field("media must be a non-empty array".into())),
                },
            },
            Kind::Comment => Body::Comment { text: f.text("text")?, name: f.opt_text("name")? },
            Kind::Vote => Body::Vote { value: f.int("value")?, seq: f.u64("seq")? },
            Kind::Edit => Body::Edit { text: f.text("text")?, title: f.opt_text("title")? },
            Kind::Delete => Body::Delete,
            Kind::Delegation => Body::Delegation {
                root: f.id("root")?,
                scope: f.texts("scope")?,
                expires_at: f.u64("expires_at")?,
                root_sig: Sig::from_slice(&f.bytes("root_sig")?)?,
                seal: f.opt_id("seal")?,
            },
            Kind::Pointer => Body::Pointer { record: f.id("record")?, of: Kind::parse(&f.text("of")?)?, media: f.opt_id("media")?, sealed: f.bytes("sealed")? },
            Kind::Roster => Body::Roster { holders: f.ids("holders")? },
            Kind::Trust => Body::Trust { keys: f.ids("keys")? },
            Kind::Wiki => Body::Wiki { page: f.text("page")?, update: f.bytes("update")? },
            Kind::Blocklist => Body::Blocklist { exact: f.ids("exact")?, pdq: f.ids("pdq")? },
            Kind::Tally => Body::Tally {
                counts: match f.take("counts")? {
                    Value::A(rows) if rows.len() <= MAX_TALLY => rows
                        .into_iter()
                        .map(|r| match r {
                            Value::A(v) => match v.as_slice() {
                                [Value::B(p), Value::U(u), Value::U(d)] => Ok((Id::from_slice(p)?, *u, *d)),
                                _ => Err(TypesError::Field("a tally row is [post, up, down]".into())),
                            },
                            _ => Err(TypesError::Field("a tally row is [post, up, down]".into())),
                        })
                        .collect::<Result<_, _>>()?,
                    _ => return Err(TypesError::Field("a tally counts up to 500 posts".into())),
                },
            },
            Kind::Revoke => Body::Revoke { device: f.id("device")? },
            Kind::Identity => Body::Identity { work: f.u64("work")?, rc: f.opt_id("rc")?, of: f.opt_id("of")? },
            Kind::Rotate => Body::Rotate { person: f.id("person")?, to: f.id("to")? },
            Kind::Share => Body::Share {
                sealed: match f.take("sealed")? {
                    Value::A(items) if items.len() <= 16 => items
                        .into_iter()
                        .map(|it| {
                            let mut x = Fields::new(it, "sealed")?;
                            let d = x.id("d")?;
                            let s = x.bytes("s")?;
                            x.done()?;
                            Ok((d, s))
                        })
                        .collect::<Result<Vec<_>, TypesError>>()?,
                    _ => return Err(TypesError::Field("sealed must be an array of at most 16".into())),
                },
                seq: f.u64("seq")?,
            },
            Kind::Vouch => Body::Vouch { value: f.int("value")?, seq: f.u64("seq")?, reason: f.opt_text("reason")? },
            Kind::Review => Body::Review { verdict: Verdict::parse(&f.text("verdict")?)?, seq: f.u64("seq")?, reason: f.opt_text("reason")? },
            Kind::Community => Body::Community(Genesis {
                name: f.text("name")?,
                description: f.text("description")?,
                color: f.text("color")?,
                owners: f.ids("owners")?,
                threshold: f.u32("threshold")?,
                rules_text_cid: f.opt_id("rules_text_cid")?,
                limits: Limits::from_value(f.take("limits")?)?,
            }),
            Kind::Ledger => Body::Ledger(LedgerBody {
                seq: f.u64("seq")?,
                action: LedgerAction::parse(&f.text("action")?)?,
                target: f.opt_id("target")?,
                reason: f.opt_text("reason")?,
                evidence: match f.opt("evidence") {
                    Some(v) => Some(ids_from(v)?),
                    None => None,
                },
                cosign: match f.opt("cosign") {
                    Some(Value::A(items)) => Some(
                        items
                            .into_iter()
                            .map(|it| {
                                let mut c = Fields::new(it, "cosign")?;
                                let key = c.id("key")?;
                                let sig = Sig::from_slice(&c.bytes("sig")?)?;
                                c.done()?;
                                Ok(Cosig { key, sig })
                            })
                            .collect::<Result<Vec<_>, TypesError>>()?,
                    ),
                    Some(_) => return Err(TypesError::Field("cosign must be an array".into())),
                    None => None,
                },
                text: f.opt_text("text")?,
                limits: match f.opt("limits") {
                    Some(v) => Some(Limits::from_value(v)?),
                    None => None,
                },
                drop_key: match f.opt("drop_key") {
                    Some(v) => {
                        let mut g = Fields::new(v, "drop_key")?;
                        let key = g.id("key")?;
                        let to = match g.take("to")? {
                            Value::A(items) => items
                                .into_iter()
                                .map(|it| {
                                    let mut x = Fields::new(it, "drop_key.to")?;
                                    let d = x.id("d")?;
                                    let s = x.bytes("s")?;
                                    x.done()?;
                                    Ok((d, s))
                                })
                                .collect::<Result<Vec<_>, TypesError>>()?,
                            _ => return Err(TypesError::Field("drop_key.to must be an array".into())),
                        };
                        g.done()?;
                        Some(DropKeyGrant { key, sealed: to })
                    }
                    None => None,
                },
                patch: match f.opt("patch") {
                    None => None,
                    Some(Value::B(b)) => Some(b),
                    Some(_) => return Err(TypesError::Field("patch must be bytes".into())),
                },
            }),
            Kind::Label => Body::Label { label: f.text("label")?, note: f.opt_text("note")? },
            Kind::Report => Body::Report { reason: f.text("reason")?, evidence: f.ids("evidence")? },
        };
        f.done()?;
        Ok(b)
    }
}

impl Record {
    pub fn kind(&self) -> Kind {
        self.body.kind()
    }

    pub fn to_value(&self) -> Value {
        let mut e: Vec<(&str, Value)> = vec![
            ("v", Value::U(self.v)),
            ("kind", t(self.kind().as_str())),
            ("author", b32(&self.author)),
            ("created_at", Value::U(self.created_at)),
            ("expires_at", Value::U(self.expires_at)),
            ("body", self.body.to_value()),
            ("nonce", Value::U(self.nonce)),
        ];
        if let Some(c) = &self.community {
            e.push(("community", b32(c)));
        }
        if let Some(p) = &self.parent {
            e.push(("parent", b32(p)));
        }
        Value::map(e)
    }

    pub fn from_value(v: Value) -> Result<Record, TypesError> {
        let mut f = Fields::new(v, "record")?;
        let ver = f.u64("v")?;
        if ver != PROTOCOL_VERSION {
            return Err(TypesError::Version(ver));
        }
        let kind = Kind::parse(&f.text("kind")?)?;
        let r = Record {
            v: ver,
            community: f.opt_id("community")?,
            author: f.id("author")?,
            parent: f.opt_id("parent")?,
            created_at: f.u64("created_at")?,
            expires_at: f.u64("expires_at")?,
            body: Body::from_value(kind, f.take("body")?)?,
            nonce: f.u64("nonce")?,
        };
        f.done()?;
        r.check_shape()?;
        Ok(r)
    }

    /// Structural rules that don't need any state: which kinds carry a community or a parent.
    pub fn check_shape(&self) -> Result<(), TypesError> {
        let k = self.kind();
        let needs_community = !matches!(k, Kind::Delegation | Kind::Revoke | Kind::Label | Kind::Identity | Kind::Rotate);
        // a pointer names what it holds in its body, never by `parent`
        let no_community = matches!(k, Kind::Delegation | Kind::Revoke | Kind::Identity | Kind::Rotate);
        let needs_parent =
            matches!(k, Kind::Comment | Kind::Vote | Kind::Edit | Kind::Delete | Kind::Label | Kind::Report | Kind::Vouch | Kind::Review | Kind::Share);
        let no_parent = matches!(
            k,
            Kind::Profile
                | Kind::ProfileColor
                | Kind::Post
                | Kind::Delegation
                | Kind::Revoke
                | Kind::Community
                | Kind::Ledger
                | Kind::Identity
                | Kind::Pointer
                | Kind::Roster
                | Kind::Rotate
                | Kind::Trust
                | Kind::Wiki
                | Kind::Tally
                | Kind::Blocklist
        );
        if needs_community && self.community.is_none() {
            return Err(TypesError::Field(format!("{} needs a community", k.as_str())));
        }
        if no_community && self.community.is_some() {
            return Err(TypesError::Field(format!("{} has no community", k.as_str())));
        }
        if needs_parent && self.parent.is_none() {
            return Err(TypesError::Field(format!("{} needs a parent", k.as_str())));
        }
        if no_parent && self.parent.is_some() {
            return Err(TypesError::Field(format!("{} has no parent", k.as_str())));
        }
        if k == Kind::Community && self.community != Some(self.author) {
            return Err(TypesError::Field("genesis must be signed by the community key".into()));
        }
        if let Body::Delegation { root, expires_at, .. } = &self.body {
            if *root == self.author {
                return Err(TypesError::Field("a key can't be its own device".into()));
            }
            if *expires_at != self.expires_at {
                return Err(TypesError::Field("a delegation expires when its record does".into()));
            }
        }
        if let Body::Vouch { .. } = &self.body
            && self.parent == Some(self.author)
        {
            return Err(TypesError::Field("nobody vouches for themselves".into()));
        }
        if let Body::Roster { holders } = &self.body
            && (holders.is_empty() || holders.len() > MAX_ROSTER || holders.windows(2).any(|w| w[0] >= w[1]))
        {
            return Err(TypesError::Field("a roster names 1 to 32 holders, sorted, each once".into()));
        }
        if let Body::Trust { keys } = &self.body
            && (keys.len() > MAX_TRUST || keys.windows(2).any(|w| w[0] >= w[1]))
        {
            return Err(TypesError::Field("a trust statement names up to 256 people, sorted, each once".into()));
        }
        if let Body::Blocklist { exact, pdq } = &self.body
            && [exact, pdq].iter().any(|l| l.len() > MAX_BLOCKLIST || l.windows(2).any(|w| w[0] >= w[1]))
        {
            return Err(TypesError::Field("a blocklist holds up to 1000 hashes of each kind, sorted, each once".into()));
        }
        if let Body::Tally { counts } = &self.body
            && (counts.len() > MAX_TALLY || counts.windows(2).any(|w| w[0].0 >= w[1].0))
        {
            return Err(TypesError::Field("a tally counts up to 500 posts, sorted, each once".into()));
        }
        if let Body::Wiki { page, update } = &self.body
            && (!is_wiki_page(page) || update.is_empty() || update.len() > MAX_WIKI_UPDATE)
        {
            return Err(TypesError::Field("a wiki edit names a page (a-z, 0-9, -) and carries an update of up to 32 KiB".into()));
        }
        if self.expires_at < self.created_at {
            return Err(TypesError::Field("expires before it is created".into()));
        }
        Ok(())
    }

    pub fn encode(&self) -> Vec<u8> {
        self.to_value().encode()
    }

    /// `id = BLAKE3(canonical_cbor(Record))`
    pub fn id(&self) -> Id {
        Id::hash(&self.encode())
    }

    /// The canonical bytes split around the nonce value: `encode() == prefix ‖ cbor_uint(nonce) ‖ suffix`.
    /// Lets a miner hash the prefix once and try nonces cheaply.
    pub fn split_for_nonce(&self) -> (Vec<u8>, Vec<u8>) {
        let Value::M(entries) = self.to_value() else { unreachable!("record is a map") };
        let mut prefix = Vec::new();
        cbor::head(5, entries.len() as u64, &mut prefix);
        let mut suffix = Vec::new();
        let mut seen_nonce = false;
        for (k, v) in &entries {
            if k == "nonce" {
                cbor::encode_text(k, &mut prefix);
                seen_nonce = true;
                continue;
            }
            let out = if seen_nonce { &mut suffix } else { &mut prefix };
            cbor::encode_text(k, out);
            cbor::encode_into(v, out);
        }
        (prefix, suffix)
    }

    /// What cosigners sign for threshold ledger actions: the id of this record with
    /// `body.cosign` left out, `nonce` 0 and `author` zeroed, so whichever owner collects the last
    /// signature can publish it under their own key. The publisher still signs the full id.
    pub fn proposal_id(&self) -> Id {
        let mut r = self.clone();
        if let Body::Ledger(l) = &mut r.body {
            l.cosign = None;
        }
        r.nonce = 0;
        r.author = Id::ZERO;
        r.id()
    }
}

/// `Wire = { r: Record, sig: bytes64 }`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wire {
    pub record: Record,
    pub sig: Sig,
}

impl Wire {
    pub fn encode(&self) -> Vec<u8> {
        Value::map([("r", self.record.to_value()), ("sig", Value::B(self.sig.0.to_vec()))]).encode()
    }

    /// Strict decode. Returns the record id computed from its canonical bytes.
    pub fn decode(bytes: &[u8]) -> Result<(Wire, Id), TypesError> {
        let v = cbor::decode(bytes)?;
        let mut f = Fields::new(v, "wire")?;
        let rv = f.take("r")?;
        let sig = Sig::from_slice(&f.bytes("sig")?)?;
        f.done()?;
        let id = Id::hash(&rv.encode());
        let record = Record::from_value(rv)?;
        Ok((Wire { record, sig }, id))
    }

    /// The id of stored bytes that no longer decode as a record (an older format), so storage
    /// can delete them. `None` when they aren't even a wire map.
    pub fn loose_id(bytes: &[u8]) -> Option<Id> {
        let Value::M(e) = cbor::decode(bytes).ok()? else { return None };
        e.into_iter().find(|(k, _)| k == "r").map(|(_, v)| Id::hash(&v.encode()))
    }
}

// ------------------------------------------------------------------ field reader

/// Takes fields out of a decoded map and complains about missing, mistyped or unknown keys.
pub(crate) struct Fields {
    m: BTreeMap<String, Value>,
    what: &'static str,
}

fn ids_from(v: Value) -> Result<Vec<Id>, TypesError> {
    match v {
        Value::A(items) => items
            .into_iter()
            .map(|i| match i {
                Value::B(b) => Id::from_slice(&b),
                _ => Err(TypesError::Field("expected bytes32".into())),
            })
            .collect(),
        _ => Err(TypesError::Field("expected an array of bytes32".into())),
    }
}

impl Fields {
    pub(crate) fn new(v: Value, what: &'static str) -> Result<Fields, TypesError> {
        match v {
            Value::M(e) => Ok(Fields { m: e.into_iter().collect(), what }),
            _ => Err(TypesError::Field(format!("{what} must be a map"))),
        }
    }
    fn missing(&self, k: &str) -> TypesError {
        TypesError::Field(format!("{}: missing or mistyped {k:?}", self.what))
    }
    pub(crate) fn opt(&mut self, k: &str) -> Option<Value> {
        self.m.remove(k)
    }
    pub(crate) fn take(&mut self, k: &str) -> Result<Value, TypesError> {
        self.m.remove(k).ok_or_else(|| self.missing(k))
    }
    pub(crate) fn u64(&mut self, k: &str) -> Result<u64, TypesError> {
        match self.take(k)? {
            Value::U(n) => Ok(n),
            _ => Err(self.missing(k)),
        }
    }
    pub(crate) fn u32(&mut self, k: &str) -> Result<u32, TypesError> {
        u32::try_from(self.u64(k)?).map_err(|_| self.missing(k))
    }
    pub(crate) fn opt_u32(&mut self, k: &str) -> Result<Option<u32>, TypesError> {
        match self.opt(k) {
            None => Ok(None),
            Some(Value::U(n)) => u32::try_from(n).map(Some).map_err(|_| self.missing(k)),
            Some(_) => Err(self.missing(k)),
        }
    }
    pub(crate) fn int(&mut self, k: &str) -> Result<i64, TypesError> {
        match self.take(k)? {
            Value::U(n) if n <= i64::MAX as u64 => Ok(n as i64),
            Value::N(n) if n < i64::MAX as u64 => Ok(-1 - n as i64),
            _ => Err(self.missing(k)),
        }
    }
    pub(crate) fn text(&mut self, k: &str) -> Result<String, TypesError> {
        match self.take(k)? {
            Value::T(s) => Ok(s),
            _ => Err(self.missing(k)),
        }
    }
    pub(crate) fn opt_text(&mut self, k: &str) -> Result<Option<String>, TypesError> {
        match self.opt(k) {
            None => Ok(None),
            Some(Value::T(s)) => Ok(Some(s)),
            Some(_) => Err(self.missing(k)),
        }
    }
    pub(crate) fn bytes(&mut self, k: &str) -> Result<Vec<u8>, TypesError> {
        match self.take(k)? {
            Value::B(b) => Ok(b),
            _ => Err(self.missing(k)),
        }
    }
    pub(crate) fn id(&mut self, k: &str) -> Result<Id, TypesError> {
        Id::from_slice(&self.bytes(k)?)
    }
    pub(crate) fn opt_id(&mut self, k: &str) -> Result<Option<Id>, TypesError> {
        match self.opt(k) {
            None => Ok(None),
            Some(Value::B(b)) => Ok(Some(Id::from_slice(&b)?)),
            Some(_) => Err(self.missing(k)),
        }
    }
    pub(crate) fn ids(&mut self, k: &str) -> Result<Vec<Id>, TypesError> {
        ids_from(self.take(k)?)
    }
    pub(crate) fn texts(&mut self, k: &str) -> Result<Vec<String>, TypesError> {
        match self.take(k)? {
            Value::A(items) => items
                .into_iter()
                .map(|i| match i {
                    Value::T(s) => Ok(s),
                    _ => Err(TypesError::Field(format!("{k}: expected text"))),
                })
                .collect(),
            _ => Err(self.missing(k)),
        }
    }
    pub(crate) fn done(self) -> Result<(), TypesError> {
        match self.m.keys().next() {
            Some(k) => Err(TypesError::Field(format!("{}: unknown field {k:?}", self.what))),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scatter_limits_are_written_only_when_asked_for() {
        // The default is left out: limits from before the setting existed encode the same.
        let d = Limits::default();
        let bytes = d.to_value().encode();
        assert!(!String::from_utf8_lossy(&bytes).contains("scatter"));
        assert_eq!(Limits::from_value(crate::cbor::decode(&bytes).unwrap()).unwrap(), d);
        // A community that asks for more carries it, and reads back the same.
        let more = Limits { scatter: Scatter { resist: 20, open: true }, ..Limits::default() };
        let bytes = more.to_value().encode();
        assert_eq!(Limits::from_value(crate::cbor::decode(&bytes).unwrap()).unwrap(), more);
        // The default written out is refused: one encoding per value.
        let Value::M(mut e) = d.to_value() else { unreachable!() };
        e.push(("scatter".into(), Scatter::default().to_value()));
        assert!(Limits::from_value(Value::map(e)).is_err());
        // and so are unknown fields inside it
        let Value::M(mut e) = d.to_value() else { unreachable!() };
        e.push(("scatter".into(), Value::map([("resist", Value::U(15)), ("open", Value::Bool(false)), ("x", Value::U(1))])));
        assert!(Limits::from_value(Value::map(e)).is_err());
    }

    #[test]
    fn trackers_are_written_only_when_named_and_must_be_https() {
        let d = Limits::default();
        assert!(!String::from_utf8_lossy(&d.to_value().encode()).contains("trackers"));
        let named = Limits { trackers: vec!["https://tracker.example".into()], ..Limits::default() };
        let bytes = named.to_value().encode();
        assert_eq!(Limits::from_value(crate::cbor::decode(&bytes).unwrap()).unwrap(), named);
        let with = |t: Vec<Value>| {
            let Value::M(mut e) = d.to_value() else { unreachable!() };
            e.push(("trackers".into(), Value::A(t)));
            Limits::from_value(Value::map(e))
        };
        assert!(with(vec![]).is_err(), "none is left out, never written empty");
        assert!(with(vec![Value::T("http://tracker.example".into())]).is_err());
        assert!(with((0..3).map(|i| Value::T(format!("https://t{i}.example"))).collect()).is_err());
    }

    pub(crate) fn sample_post() -> Record {
        Record {
            v: 1,
            community: Some(Id([1; 32])),
            author: Id([2; 32]),
            parent: None,
            created_at: 1_790_000_000,
            expires_at: 1_790_000_000 + 86_400,
            body: Body::Post { title: "hoi".into(), text: "say hi quietly".into(), link: None, name: Some("lena".into()), media: Vec::new() },
            nonce: 300,
        }
    }

    #[test]
    fn record_roundtrip_and_split() {
        let r = sample_post();
        let wire = Wire { record: r.clone(), sig: Sig([9; 64]) };
        let bytes = wire.encode();
        let (back, id) = Wire::decode(&bytes).unwrap();
        assert_eq!(back.record, r);
        assert_eq!(id, r.id());

        let (prefix, suffix) = r.split_for_nonce();
        let mut joined = prefix.clone();
        crate::cbor::head(0, r.nonce, &mut joined);
        joined.extend_from_slice(&suffix);
        assert_eq!(joined, r.encode());
    }

    #[test]
    fn shape_rules() {
        let mut r = sample_post();
        r.community = None;
        assert!(r.check_shape().is_err());
        let mut c = sample_post();
        c.body = Body::Comment { text: "x".into(), name: None };
        assert!(c.check_shape().is_err(), "comment without parent");
    }

    #[test]
    fn new_kinds_roundtrip() {
        let mut p = sample_post();
        p.body = Body::Post {
            title: "a photo".into(),
            text: "".into(),
            link: None,
            name: None,
            media: vec![MediaRef { hash: Id([4; 32]), mime: "image/png".into(), size: 120, w: Some(1), h: Some(1), alt: Some("a dot".into()) }],
        };
        let mut vouch = sample_post();
        vouch.parent = Some(Id([5; 32]));
        vouch.body = Body::Vouch { value: -1, seq: 2, reason: Some("spam".into()) };
        let mut review = sample_post();
        review.parent = Some(Id([6; 32]));
        review.body = Body::Review { verdict: Verdict::Veto, seq: 1, reason: Some("rule 3 doesn't say that".into()) };
        let mut ident = sample_post();
        ident.community = None;
        ident.body = Body::Identity { work: 77, rc: None, of: None };
        let mut deleg = sample_post();
        deleg.community = None;
        deleg.body = Body::Delegation { root: Id([8; 32]), scope: vec!["*".into()], expires_at: deleg.expires_at, root_sig: Sig([1; 64]), seal: None };
        for r in [p, vouch, review, ident, deleg] {
            r.check_shape().unwrap();
            let w = Wire { record: r.clone(), sig: Sig([9; 64]) };
            let (back, id) = Wire::decode(&w.encode()).unwrap();
            assert_eq!(back.record, r);
            assert_eq!(id, r.id());
        }
    }

    #[test]
    fn shapes_of_new_kinds() {
        let mut v = sample_post();
        v.parent = Some(v.author);
        v.body = Body::Vouch { value: 1, seq: 1, reason: None };
        assert!(v.check_shape().is_err(), "self-vouch");
        let mut d = sample_post();
        d.community = None;
        d.body = Body::Delegation { root: d.author, scope: vec!["*".into()], expires_at: d.expires_at, root_sig: Sig([1; 64]), seal: None };
        assert!(d.check_shape().is_err(), "own device");
        d.body = Body::Delegation { root: Id([3; 32]), scope: vec!["*".into()], expires_at: d.expires_at + 1, root_sig: Sig([1; 64]), seal: None };
        assert!(d.check_shape().is_err(), "expiry mismatch");
        // an empty media array is not canonical: absent means none
        let Value::M(mut e) = sample_post().to_value() else { unreachable!() };
        for (k, v) in e.iter_mut() {
            if k == "body"
                && let Value::M(b) = v
            {
                b.push(("media".into(), Value::A(vec![])));
                b.sort_by(|x, y| crate::cbor::key_cmp(&x.0, &y.0));
            }
        }
        assert!(Record::from_value(Value::M(e)).is_err());
        assert!(is_hide_reason("rule:3") && is_hide_reason("spam") && !is_hide_reason("disagree") && !is_hide_reason("rule:"));
    }

    #[test]
    fn unknown_fields_rejected() {
        let r = sample_post();
        let Value::M(mut e) = r.to_value() else { unreachable!() };
        e.push(("zzzzzzzzzzzzzz".into(), Value::U(1)));
        let v = Value::map(e);
        assert!(Record::from_value(v).is_err());
    }

    #[test]
    fn palette_records_keep_photo_wire_separate_and_have_strict_shapes() {
        for color in ["", "mint", "sky", "lilac", "peach", "sun"] {
            let mut r = sample_post();
            r.body = Body::ProfileColor { color: color.into(), seq: 17 };
            let wire = Wire { record: r.clone(), sig: Sig([9; 64]) };
            assert!(wire.encode().len() <= Kind::ProfileColor.max_size());
            assert_eq!(Wire::decode(&wire.encode()).unwrap().0.record, r);
            r.parent = Some(Id([5; 32]));
            assert!(r.check_shape().is_err());
            r.parent = None;
            r.community = None;
            assert!(r.check_shape().is_err());
        }
        let profile = Body::Profile { image: vec![], seq: 1 }.to_value();
        assert_eq!(Body::from_value(Kind::Profile, profile.clone()).unwrap().kind(), Kind::Profile);
        let Value::M(mut fields) = profile else { unreachable!() };
        fields.push(("color".into(), Value::T("mint".into())));
        assert!(Body::from_value(Kind::Profile, Value::map(fields)).is_err(), "the original strict profile schema is unchanged");
    }
}
