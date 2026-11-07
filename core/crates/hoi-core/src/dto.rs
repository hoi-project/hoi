//! Data handed to UIs. Mirrors `packages/core-api/src/types.ts` field for field (camelCase).
//! Keys and ids are lowercase hex. Change both files together.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

/// The trust ladder's steps and quorums (PROTOCOL.md §15, §16). Seconds for durations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LadderDto {
    pub identity_bits: u32,
    pub admit_vouches: u32,
    pub established_age: u64,
    pub trusted_posts: u32,
    pub media_score: u32,
    pub hide_quorum: u32,
    pub block_quorum: u32,
    pub veto_quorum: u32,
    pub contest_window: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LadderPatch {
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub identity_bits: Option<u32>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub admit_vouches: Option<u32>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub established_age: Option<u64>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub trusted_posts: Option<u32>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub media_score: Option<u32>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub hide_quorum: Option<u32>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub block_quorum: Option<u32>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub veto_quorum: Option<u32>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub contest_window: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LimitsDto {
    pub pow_bits: u32,
    pub max_retention: u64,
    pub newcomer_posts_per_day: u32,
    pub newcomer_age: u64,
    pub admission: String,
    pub autoban_threshold: u32,
    pub labelers: Vec<String>,
    pub ladder: LadderDto,
    /// Scattered delivery: the percent of places a party may run without rebuilding or
    /// withholding a post (10, 15 or 20), and whether an open community scatters at all.
    #[serde(default = "default_scatter_resist")]
    pub scatter_resist: u8,
    #[serde(default)]
    pub scatter_open: bool,
    /// Trackers the community names: https registries asked when no route answers.
    #[serde(default)]
    pub trackers: Vec<String>,
    /// Age-proof admission: the credential issuers' keys, hex.
    #[serde(default)]
    pub age_issuers: Vec<String>,
}

fn default_scatter_resist() -> u8 {
    hoi_types::scatter::LEVELS[0]
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LimitsPatch {
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub pow_bits: Option<u32>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub max_retention: Option<u64>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub newcomer_posts_per_day: Option<u32>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub newcomer_age: Option<u64>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub admission: Option<String>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub autoban_threshold: Option<u32>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub labelers: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub ladder: Option<LadderPatch>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub scatter_resist: Option<u8>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub scatter_open: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub trackers: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub age_issuers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Person {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    pub key: String,
    pub short: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub petname: Option<String>,
    pub trust: String,
    pub muted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banned: Option<bool>,
    pub key_age_days: u64,
    /// Where they stand on the community's ladder (community context only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standing: Option<String>,
    /// A friend of a friend: the friends whose trust statements name them (at most 3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_by: Vec<String>,
    /// Their name is a domain: whether it names their key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<HandleView>,
}

/// A name that is a domain, and whether the domain says it is this person's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct HandleView {
    pub domain: String,
    pub verified: bool,
    /// Not checked on this device yet.
    pub checking: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DeviceInfo {
    pub key: String,
    pub short: String,
    pub label: String,
    pub this_device: bool,
    pub expires_at: u64,
    pub revoked: bool,
    /// This computer keeps that browser's key and signs for it.
    pub signed_here: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Me {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    pub root: String,
    pub root_key: String,
    /// The root more identity work is mined for: the one this device signs with now. The same
    /// as `root_key` until a recovery replaces the root; then the person keeps
    /// `root_key` and the stronger identity record is signed by this one.
    pub work_key: String,
    pub fingerprint: String,
    pub short: String,
    pub name: String,
    pub created_at: u64,
    pub has_root: bool,
    pub device: String,
    pub devices: Vec<DeviceInfo>,
    /// Memory-hard work this identity carries (PROTOCOL.md §4.1).
    pub identity_bits: u32,
    pub burned: bool,
    /// This device still keeps the recovery line.
    pub holds_line: bool,
    /// Brought back from the line, and not yet sure it signs with the person's current root.
    pub restoring: bool,
    /// Another device replaced the root this one signs with: nothing it signs counts anymore.
    pub replaced: bool,
    /// This device holds no signing key: the person's computer signs for it.
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CommunityRef {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PublicJoinStatus {
    pub id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CommunitySummary {
    pub id: String,
    pub name: String,
    pub color: String,
    pub description: String,
    pub created_at: u64,
    pub joined: bool,
    pub pinning: bool,
    pub role: String,
    pub posts: u64,
    pub last_activity: u64,
}

/// An open community available here or advertised by a currently connected node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DiscoveredCommunity {
    pub id: String,
    pub name: String,
    pub color: String,
    pub description: String,
    pub invite: String,
    pub joined: bool,
    /// A node connected now lists it; false for a listing remembered from earlier.
    pub live: bool,
    /// When a node last listed it as its own, as far as this device knows (0: never heard).
    pub seen: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CommunityRegistration {
    pub name: String,
    pub community: String,
    pub signature: String,
}

/// A version-bound public directory claim; it contains no owner or member identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CommunityPublication {
    pub name: String,
    pub community: String,
    pub protocol_version: String,
    pub index_version: u32,
    pub signature: String,
}

/// How safe one community's live records are, read from this device's own links
/// (PROTOCOL.md §20). Not a delivery receipt: copies this device could see, nothing more.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CommunityWarmth {
    pub community: String,
    pub name: String,
    /// "cold", "cool", "warm" or "hot".
    pub level: String,
    /// 0 to 100.
    pub score: u32,
    /// Copies on other durable nodes, once per independent place (one operator, one network):
    /// what the level is read from; one decimal.
    pub kept: f64,
    /// Independent places holding a durable copy.
    pub places: u32,
    /// 1 when every other durable copy is in a place of its own, lower when they share one.
    pub spread: f64,
    /// Share of those copies reached over a guarded link (encrypted, other end checked).
    pub guarded: f64,
    /// Copies this device can see, its own included; one decimal.
    pub copies: f64,
    /// Of those, copies on nodes that outlive a browser tab (desktops, headless nodes).
    pub durable_copies: f64,
    /// The best a connected node reports, or this device's own count when higher; one hop.
    pub network_copies: f64,
    /// Share of the live window known to exist that is here, 0 to 1.
    pub complete: f64,
    /// Share of referenced media that is here, 0 to 1.
    pub media: f64,
    /// A connected peer lists it, or a catch-up finished within the last seven days.
    pub fresh: bool,
    /// A connected peer lists it right now.
    pub live: bool,
    pub records: u64,
    pub missing: u64,
    pub holders_durable: u32,
    pub holders_transient: u32,
    /// Members' nodes met within 30 days, by route: leads, not copies.
    pub remembered: u32,
    pub own_durable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<u64>,
    /// "index" or "request" when this node carries it for others (native nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carried: Option<String>,
    /// A public join still waiting for the signed definition.
    pub pending: bool,
}

/// What this device knows of the directory beyond any registry: connected nodes sharing their
/// catalogs, open communities they list, nodes that take carry requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DirectoryWarmth {
    pub catalog_nodes: u32,
    pub discoverable: u32,
    pub carry_nodes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct WarmthView {
    /// Public communities this device is in, coldest first.
    pub communities: Vec<CommunityWarmth>,
    pub directory: DirectoryWarmth,
    pub own_durable: bool,
    /// The warmth of this device's own connections.
    pub links: LinkWarmthView,
}

/// The warmth of this device's own connections (PROTOCOL.md §20): how many independent places
/// it is linked through, whether one carries most of its links, how many are guarded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LinkWarmthView {
    /// "cold", "cool", "warm" or "hot".
    pub level: String,
    /// 0 to 100.
    pub score: u32,
    pub links: u32,
    /// Independent places among them (one operator, one network).
    pub places: u32,
    /// The effective number of places, two decimals.
    pub effective: f64,
    /// Share of links in the place that carries the most.
    pub largest: f64,
    /// Share of guarded links (encrypted, other end checked).
    pub guarded: f64,
}

/// The outcome of asking connected public nodes to keep a community available.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CarryRequest {
    pub community: String,
    /// Nodes asked just now; their answers arrive as notices.
    pub asked: u32,
    /// Connected nodes that take requests and already list this community.
    pub carrying: u32,
}

/// A node telling the public registry where it can be reached and which open communities it
/// carries, signed by its transport key. It names infrastructure, never a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NodeAnnouncement {
    /// The origin of the registry it is for, signed: another registry refuses it.
    pub registry: String,
    /// The transport public key (iroh endpoint id), lowercase hex.
    pub endpoint: String,
    /// Public routes, in the order others should try them (`rtc:` and native).
    pub routes: Vec<String>,
    /// Open public communities this node holds, sorted, at most 64.
    pub communities: Vec<String>,
    /// Unix seconds; the registry refuses announcements more than ten minutes off its clock.
    pub issued_at: u64,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RulesView {
    pub text: String,
    pub version: u32,
    pub at: u64,
    pub signed_by: Vec<Person>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct BanView {
    pub person: Person,
    pub reason: String,
    pub at: u64,
    pub auto: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CommunityStats {
    pub posts: u64,
    pub comments: u64,
    pub people: u64,
    pub stored_bytes: u64,
    pub ledger_entries: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum LedgerTarget {
    #[serde(rename = "record", rename_all = "camelCase")]
    Record {
        id: String,
        short: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        author: Option<Person>,
        #[serde(skip_serializing_if = "Option::is_none")]
        record_kind: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        post: Option<String>,
    },
    #[serde(rename = "key")]
    Key { person: Person },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FlagView {
    pub target: LedgerTarget,
    pub reasons: Vec<String>,
    pub count: u64,
    pub weight: f64,
    pub last_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CommunityView {
    pub id: String,
    pub name: String,
    pub color: String,
    pub description: String,
    pub created_at: u64,
    pub joined: bool,
    pub pinning: bool,
    pub role: String,
    pub posts: u64,
    pub last_activity: u64,
    pub owners: Vec<Person>,
    pub threshold: u32,
    pub mods: Vec<Person>,
    pub limits: LimitsDto,
    pub rules: Option<RulesView>,
    pub stats: CommunityStats,
    pub pending: Vec<Person>,
    pub banned: Vec<BanView>,
    pub flags: Vec<FlagView>,
    /// Established members (owners and moderators included).
    pub established: u64,
    /// Their voice together, in voices (PROTOCOL.md §15.4).
    pub established_voice: f64,
    /// Your place on the ladder, when you have an identity.
    pub me: Option<StandingView>,
    /// Newcomer posts waiting for your review (established members only).
    pub review_queue: u64,
    /// Proposed rules and limits still being decided, or decided in the last 30 days.
    pub proposals: Vec<ProposalView>,
    /// You own this members-only community, whose records travel in the clear, and could make
    /// a private copy of it (`private_copy`): an encrypted group whose membership follows its
    /// own records.
    #[serde(default)]
    pub private_copy: bool,
    /// An open community whose posts go in pieces (SCATTER.md S5): the level the posts kept in
    /// pieces here meet, in percent of places. Absent while posts here travel whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pieces: Option<u8>,
    /// Trackers to ask when no route answers: the community's, then those an invite
    /// named before its limits arrived.
    #[serde(default)]
    pub trackers: Vec<String>,
}

/// A field a limits proposal changes. `field` is the snake_case name in `Limits`/`Ladder`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ChangeView {
    pub field: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Objection {
    pub person: Person,
    pub reason: String,
}

/// A proposed change to the rules or limits and where the community stands on it
/// (PROTOCOL.md §6.1). Voices are in whole voices.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProposalView {
    pub id: String,
    /// "limits" or "rules"
    pub kind: String,
    pub at: u64,
    /// Answers close; silence counts as consent from then on.
    pub deadline: u64,
    /// open, in-effect, passed, lapsed
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<u64>,
    /// Everyone agreed (rather than nobody objecting in time).
    pub unanimous: bool,
    pub by: Vec<Person>,
    pub changes: Vec<ChangeView>,
    /// The proposed rules text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The people who decide: established when it was proposed.
    pub electors: u32,
    pub agreed: Vec<Person>,
    pub objecting: Vec<Objection>,
    /// Around and haven't answered: until they do, it waits for them.
    pub waiting: Vec<Person>,
    /// Away (nothing of theirs left here) and haven't answered: they don't hold it up.
    pub away: u32,
    /// Retired by the owners after leaving for good: their silence counts for nothing.
    pub retired: u32,
    /// For owners: away electors who have signed nothing here for 30 days and may be retired.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub retirable: Vec<Person>,
    pub agreed_voice: f64,
    pub objecting_voice: f64,
    pub waiting_voice: f64,
    /// "agree", "object" or none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_answer: Option<String>,
    /// You're one of the people who decide it, and answers are still open.
    pub can_answer: bool,
}

/// One rung missing on the way up.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NextStep {
    pub level: String,
    pub missing: Vec<String>,
}

/// Where someone stands in a community, and why (PROTOCOL.md §15).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct StandingView {
    pub community: String,
    pub person: Person,
    /// newcomer, member, established, trusted
    pub level: String,
    /// text, links, vote, review, vouch, veto, media
    pub capabilities: Vec<String>,
    pub admitted_at: Option<u64>,
    pub established_at: Option<u64>,
    pub vouched_by: Vec<Person>,
    pub vouches_needed: u32,
    pub objections: u32,
    pub contributions: u32,
    pub contributions_needed: u32,
    pub score: i64,
    pub score_needed: i64,
    pub blocked: bool,
    /// Vouched for people the community later blocked: no voice in reviews and vetoes.
    pub struck: bool,
    /// Your own vouch for them: 1, -1 or 0.
    pub my_vouch: i64,
    /// Their weight in hides, blocks, vetoes and objections, 0 to 1 (PROTOCOL.md §15.4).
    pub voice: f64,
    /// People they vouched for this quarter: each of those vouches carries `voice / n`.
    pub vouched_this_quarter: u32,
    pub identity_bits: u32,
    pub identity_needed: u32,
    pub next: Option<NextStep>,
}

/// A media item a post carries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct MediaView {
    pub hash: String,
    pub mime: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    /// The bytes are on this device (checked); otherwise they are on their way.
    pub available: bool,
}

/// A file to attach to a post. `data` is base64url; the core sniffs the type and strips
/// metadata before anything is hashed or shared.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct MediaDraft {
    pub data: String,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub alt: Option<String>,
}

/// Media bytes for display. `data` is base64url.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct MediaBlob {
    pub hash: String,
    pub mime: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CommunityDraft {
    pub name: String,
    pub description: String,
    pub color: String,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub limits: Option<LimitsPatch>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub rules: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub co_owners: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub threshold: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Invite {
    pub uri: String,
    pub web: String,
    /// The addresses inside, for the invite screen to name before it is shared.
    #[serde(default)]
    pub addresses: Vec<InviteAddress>,
    /// Someone on another network can use it: at least one address inside is on the internet.
    #[serde(default)]
    pub outside: bool,
}

/// One address an invite carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct InviteAddress {
    /// `ip:port`, without the key or certificate fingerprint that comes with it.
    pub address: String,
    /// `device`, `network` (this home or office) or `internet`.
    pub scope: String,
    /// `yours`: this device's or its home node's; `node`: a node that holds the community or was
    /// asked to keep it; `entry`: a public node this device reached, a way into the network.
    pub whose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct InviteCommunity {
    pub id: String,
    pub name: String,
    pub color: String,
    pub description: String,
    pub limits: LimitsDto,
    pub owners: Vec<Person>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct InvitePreview {
    pub community: InviteCommunity,
    pub relays: Vec<String>,
    pub peers: Vec<String>,
    /// Trackers the invite names: asked only when no route answers.
    #[serde(default)]
    pub trackers: Vec<String>,
    pub already_joined: bool,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Napping {
    pub label: String,
    pub by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Removal {
    pub by: Option<Person>,
    pub reason: String,
    pub at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PostSummary {
    pub id: String,
    pub short: String,
    pub community: CommunityRef,
    pub author: Person,
    pub title: String,
    pub excerpt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    pub created_at: u64,
    pub expires_at: u64,
    pub score: i64,
    pub my_vote: i64,
    /// Up-votes from your friends and from friends of friends: why Hot puts it where it is.
    pub friends_up: u32,
    pub fof_up: u32,
    /// What the aggregators you follow count for it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tallies: Vec<TallyView>,
    pub replies: u64,
    pub pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub napping: Option<Napping>,
    pub labels: Vec<String>,
    pub edited: bool,
    pub mine: bool,
    pub saved: bool,
    pub waiting: bool,
    /// The author is a newcomer the community hasn't validated yet.
    pub unvalidated: bool,
    pub media: Vec<MediaView>,
    /// Only on your own records: why other devices hold it back for now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Checks {
    pub signature: String,
    pub signed_by: String,
    pub stamp_bits: u32,
    pub required_bits: u32,
    pub received_from: u32,
    pub created_at: u64,
    pub fades_at: u64,
    /// The author's identity work, bits.
    pub identity_bits: u32,
    /// The author's standing in this community.
    pub standing: String,
    /// Established members whose vouches validated the author.
    pub vouched_by: u32,
    /// Only on this device's own posts in open communities: the node it connects through can't
    /// tell the post is its person's (an injector took it, its pieces went in three hops, and
    /// this device never offered its person's records first).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub address_hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_delivery: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CommentView {
    pub id: String,
    pub short: String,
    pub parent: String,
    pub author: Person,
    pub text: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub score: i64,
    pub my_vote: i64,
    pub mine: bool,
    pub edited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub napping: Option<Napping>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<Removal>,
    pub waiting: bool,
    pub unvalidated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
    pub children: Vec<CommentView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PostView {
    #[serde(flatten)]
    pub summary: PostSummary,
    pub text: String,
    pub comments: Vec<CommentView>,
    pub checks: Checks,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<Removal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PostDraft {
    pub community: String,
    pub title: String,
    pub text: String,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub link: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub media: Option<Vec<MediaDraft>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LedgerEntryView {
    pub id: String,
    pub seq: u64,
    pub at: u64,
    pub action: String,
    pub by: Vec<Person>,
    pub auto: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<LedgerTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<LimitsDto>,
    pub evidence: u64,
    pub expires_at: Option<u64>,
    /// Established members vetoing it, their voice, and the voice it takes (PROTOCOL.md §16.3).
    pub vetoes: u32,
    pub vetoes_voice: f64,
    pub vetoes_needed: f64,
    pub support: u32,
    pub overturned: bool,
    pub vetoable: bool,
    /// Your own review of it: "veto", "ok" or none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_review: Option<String>,
    /// Removals: the body is deleted at this time unless vetoed first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_until: Option<u64>,
    /// Rules and limits: open, in-effect, passed or lapsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status")]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[allow(clippy::large_enum_variant)] // one per owner action; boxing would only complicate the bindings
pub enum LedgerResult {
    #[serde(rename = "published")]
    Published { entry: LedgerEntryView },
    #[serde(rename = "needs-cosign")]
    NeedsCosign { proposal: String, have: u32, need: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NodeStats {
    pub records: u64,
    pub stored_bytes: u64,
    pub communities: u64,
    pub bounced_keys: u64,
    pub bounced_records: u64,
    pub peers: u64,
    pub passed_along_bytes: u64,
    /// Checked media bytes kept here.
    pub media_bytes: u64,
    /// Posts you saved (kept on this device only, never passed on).
    pub saved: u64,
    /// Newcomers whose posts you read, remembered for up to 30 days so the app can vouch for
    /// them. Never sent anywhere.
    pub reading_notes: u64,
    /// When the last post or comment stored here fades.
    pub fades_by: u64,
    /// Pictures this device refuses by fingerprint, near copies included.
    pub blocked_pictures: u64,
    /// While this node keeps mailboxes and pieces for others (`hoi-node --hold-pieces`, or the
    /// desktop's switch): how many, how many bytes, and of how many at most. None readable here.
    #[serde(default)]
    pub held_pieces: u64,
    #[serde(default)]
    pub held_bytes: u64,
    #[serde(default)]
    pub held_budget: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Settings {
    pub relay: bool,
    pub daily_cap_gb: f64,
    pub keep_moderated: bool,
    pub link_previews: bool,
    pub tor: bool,
    pub theme: String,
    pub motion: String,
    /// Vouch for newcomers whose posts you read and didn't hide (established members).
    pub auto_vouch: bool,
    /// Fetch media of trusted members' posts by itself.
    pub auto_media: bool,
    /// The person saw what relaying means (TECHNICAL.md §4); native nodes relay by default.
    pub relay_notice_seen: bool,
    /// Desktop: open as a small companion (status, traffic, pairing) instead of the full app.
    pub companion: bool,
    /// A browser or phone without a home node: the person saw, once, that nobody can read what
    /// it posts until a home node carrying the community has it.
    pub posting_notice_seen: bool,
    /// Serve strangers as a public node: carry open communities whoever asks, pass on the
    /// public nodes this one reached and share a node link. On the desktop the person turns it
    /// on and the router is asked to forward its ports; `hoi-node` sets it from its flags.
    /// Native only, and only while relaying.
    pub public_node: bool,
    /// Only through this VPN: the network interface (`wg0`, `utun3`, …) every connection to
    /// the internet must use; empty for every interface. Native only.
    pub vpn: String,
    /// With a VPN: the home network may still be used directly, so a phone on the Wi-Fi keeps
    /// working. Never an internet address outside the VPN.
    pub home_network: bool,
    /// Desktop: keep mailboxes and pieces for others, up to this many gigabytes; 0 is off.
    /// Only while a public node (others must be able to reach a holder); the host opens the
    /// store and turns the role on (SCATTER.md §5.7). `hoi-node` uses `--hold-pieces` instead.
    pub hold_pieces_gb: f64,
    /// Communities marked as favorites in this app's list, in the order shown (community IDs).
    /// A view of this device only: never sent, never in a backup.
    pub favorites: Vec<String>,
    /// The order the person put the other communities in (community IDs), this device only.
    pub community_order: Vec<String>,
    /// Browsers and phones: "Use less data". Posts kept in pieces are rebuilt by the home node
    /// alone, waiting for it rather than fetching through this device's own forwarders, or,
    /// with no home node, fetched with one spare piece fewer (SCATTER.md §5.12).
    pub less_data: bool,
    /// Desktops and nodes: "Share the holders I see". In communities where this person is
    /// established, publish a roster of the holders this device saw up for days, which others
    /// count as one more vouch. Off at first: it ties this device's address to its
    /// person, for the links it sends its node list to.
    pub share_holders: bool,
    /// Browsers and phones: "Direct links with members' browsers". Off at first: the
    /// browsers linked learn each other's addresses.
    pub direct_links: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            relay: false,
            daily_cap_gb: 5.0,
            keep_moderated: true,
            link_previews: false,
            tor: false,
            theme: "system".into(),
            motion: "system".into(),
            auto_vouch: true,
            auto_media: true,
            relay_notice_seen: false,
            companion: false,
            posting_notice_seen: false,
            public_node: false,
            vpn: String::new(),
            home_network: true,
            // on while the desktop is a public node, as on every node that serves strangers (S6)
            hold_pieces_gb: 2.0,
            favorites: Vec::new(),
            community_order: Vec::new(),
            less_data: false,
            share_holders: false,
            direct_links: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SettingsPatch {
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub relay: Option<bool>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub daily_cap_gb: Option<f64>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub keep_moderated: Option<bool>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub link_previews: Option<bool>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub tor: Option<bool>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub theme: Option<String>,
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub motion: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub auto_vouch: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub auto_media: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub relay_notice_seen: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub companion: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub posting_notice_seen: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub public_node: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub vpn: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub home_network: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub hold_pieces_gb: Option<f64>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub favorites: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub community_order: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub less_data: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub share_holders: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "ffi", uniffi(default = None))]
    pub direct_links: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct HomeNode {
    pub url: String,
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The pairing link as a web address (with the home node's WebRTC routes): a phone's camera
    /// opens it, pairs with the same home node and can ask to be linked.
    #[serde(default)]
    pub pairing_web: String,
    /// The pairing link names a route a phone can use: a WebRTC route or a wss:// address (the
    /// link itself is masked, so the app can't read that from it).
    #[serde(default)]
    pub phone_can_reach: bool,
    /// How this device reaches it: "websocket" or "webrtc", once connected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Serving {
    pub url: String,
    pub pairing: String,
    /// The same link as a web address, for QR codes: a phone's camera opens the app with it.
    #[serde(default)]
    pub pairing_web: String,
    /// The pairing link names a route a phone can use (see `HomeNode::phone_can_reach`).
    #[serde(default)]
    pub phone_can_reach: bool,
    pub clients: u64,
}

/// What a pairing link says, before pairing: the links are masked, so the app asks the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PairPreview {
    /// The name the link gives; whatever the link says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// WebSocket address; empty when the link names only WebRTC routes.
    pub url: String,
    /// WebRTC routes, `<fp>@<addr>`.
    pub routes: Vec<String>,
    /// This device is already paired with the home node through this link (or its next one).
    pub paired_here: bool,
}

/// The local native endpoint. A bound endpoint does not imply a reachable peer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TransportStatus {
    pub endpoint_id: String,
    pub peers: Vec<String>,
    pub relays: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ConnectionStatus {
    pub peer: String,
    pub role: String,
    pub route: String,
    pub connected: bool,
    /// "quic", "webrtc" or "websocket".
    #[serde(default)]
    pub transport: String,
    /// How the bytes travel, in words: "QUIC · TLS 1.3", "WebRTC · DTLS", "stays on this device".
    #[serde(default)]
    pub encryption: String,
    /// What is on the other end, for the network map: `home` (this device's home node),
    /// `device` (one of the person's own devices it serves), `public` (a public node: it takes
    /// carry requests), `node` (another node that keeps its copy: a member's computer), or
    /// `browser` (a browser or phone, whose copy closes with it). From the peer's own hello.
    #[serde(default)]
    pub kind: String,
    /// The peer keeps records after the link closes (`keeps` in its hello, PROTOCOL.md §20).
    #[serde(default)]
    pub keeps: bool,
    /// The peer dialed this device.
    #[serde(default)]
    pub incoming: bool,
    /// A direct link with another member's browser.
    #[serde(default)]
    pub direct: bool,
    /// The public communities this device is in that this link syncs: where a post in one of
    /// them goes from here. Never a private community.
    #[serde(default)]
    pub communities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CommunitySyncStatus {
    pub community: String,
    /// Completed peer greetings advertising this exact community; not delivery receipts.
    pub connected_peers: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NetStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<TransportStatus>,
    #[serde(default)]
    pub connections: Vec<ConnectionStatus>,
    #[serde(default)]
    pub communities: Vec<CommunitySyncStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<u64>,
    pub mode: String,
    pub label: String,
    pub peers: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_node: Option<HomeNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serving: Option<Serving>,
    /// This node's own WebRTC routes (`rtc:<fingerprint>@<addr>`), for browsers and phones.
    #[serde(default)]
    pub rtc: Vec<String>,
    /// Members' browsers may connect and catch up through this node (the relay switch).
    #[serde(default)]
    pub relaying: bool,
    /// Public nodes this device knows of besides its communities' routes (PROTOCOL.md §14.2).
    #[serde(default)]
    pub directory: DirectoryStatus,
    /// A link anyone can open to reach this node, while it serves strangers (`#/node?…`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_link: Option<String>,
    /// What the router said when asked to forward this node's ports, while it is asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_mapping: Option<PortMapping>,
    /// Every address this node gives out, and who hears it. Empty in a browser.
    #[serde(default)]
    pub exposure: Vec<Exposure>,
    /// While only a VPN may be used: whether it is up, as the host found it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vpn: Option<VpnStatus>,
    /// Operators' notices waiting for the person's answer (crate::notice): nodes this
    /// device met and will not talk to until they are read.
    #[serde(default)]
    pub pending_notices: Vec<NoticeView>,
}

/// One part of an operator's notice as people read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NoticeSection {
    pub title: String,
    pub text: String,
}

/// An operator's notice as this device keeps it: the node it belongs to, what the person
/// decided, and the words (the fixed sentences with the operator's fields in them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NoticeView {
    /// The node's endpoint key, hex: what the decision is kept under.
    pub node: String,
    /// BLAKE3 of the notice's canonical bytes, hex: a changed notice is a new question.
    pub hash: String,
    /// `pending`, `accepted` or `declined`.
    pub state: String,
    pub title: String,
    pub sections: Vec<NoticeSection>,
    pub footer: String,
    /// The operator's own fields, for a list or a search.
    pub name: String,
    pub operator: String,
    pub contact: String,
    pub country: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub ip_logging: bool,
    pub retention_days: u32,
    pub updated: String,
    /// Where the node was met (an address), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub seen: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<u64>,
}

/// What a node link says before this device dials it (crate::notice::preview_node_link).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NodeLinkPreview {
    /// The first address the link dials, and how far it reaches: `device`, `network` or `internet`.
    pub host: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// The operator's notice in the link, when it carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<NoticeView>,
    /// The notice is signed by the node the link names.
    pub verified: bool,
    /// Why the link cannot be opened, in a sentence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// The VPN every internet connection must go through, as the host sees it now.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct VpnStatus {
    pub interface: String,
    /// Up, with an address, and the system sends internet traffic through it. While it isn't,
    /// nothing goes to the internet.
    pub up: bool,
    /// Its addresses on the VPN's own network.
    #[serde(default)]
    pub addresses: Vec<String>,
    /// Why it can't be used right now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One address of this node's own, and who is told it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Exposure {
    /// `ip:port`.
    pub address: String,
    /// `device`, `network` (this home or office) or `internet`.
    pub scope: String,
    /// The router forwards it here (port mapping): this computer doesn't hold it itself.
    pub from_router: bool,
    /// Who hears it: `devices` (your own, and their pairing links), `strangers` (whoever
    /// connects, and invites), `node-link` (the link anyone can open).
    pub told: Vec<String>,
}

/// The router's answer for a public node's ports (hoi-net `portmap`), in a form the app shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PortMapping {
    /// `asking`, `forwarded`, `own-address` (this computer's address is public already),
    /// `no-router`, `behind-nat` (carrier-grade NAT or a second router), `refused`, `silent`
    /// (no PCP, NAT-PMP or UPnP answer) or `failed`.
    pub state: String,
    /// The public addresses visitors reach this node at, as the router named them.
    #[serde(default)]
    pub public: Vec<String>,
    /// The router's reason for a refusal, or the address that isn't public.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Asked through a VPN's gateway rather than the home router.
    #[serde(default)]
    pub via_vpn: bool,
}

/// Public nodes this device knows of: those that answered it as public nodes, those it was
/// told of or given a link to and hasn't reached yet, and how many of the first are connected.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DirectoryStatus {
    pub known: u64,
    pub waiting: u64,
    pub connected: u64,
    /// Open communities this device heard of from nodes and keeps (crate::directory).
    #[serde(default)]
    pub listings: u64,
}

/// What passed through this device since it started: amounts and times, never content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TrafficView {
    pub since: u64,
    pub now: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Signed records that arrived and passed every check here.
    pub records_in: u64,
    /// Records this device sent on (to your devices and to members).
    pub records_out: u64,
    pub media_in: u64,
    pub media_out: u64,
    /// Records refused here: bad bytes, signatures, stamps or rules.
    pub refused: u64,
    /// The last hour, one entry a minute, oldest first.
    pub minutes: Vec<TrafficMinute>,
    pub connections: Vec<TrafficConnection>,
    pub relay: RelayStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TrafficMinute {
    pub at: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TrafficConnection {
    pub peer: String,
    /// "home" (your home node), "client" (your device), "member" (someone else's device).
    pub role: String,
    /// What it calls itself, for your own devices only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub transport: String,
    pub encryption: String,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub records_in: u64,
    pub records_out: u64,
    pub since: u64,
    pub last_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RelayStatus {
    /// The switch: members' devices may connect here and catch up.
    pub on: bool,
    /// This device can relay at all (native nodes; browsers can't accept connections).
    pub possible: bool,
    pub cap_bytes: u64,
    pub today_bytes: u64,
    /// Over today's cap: members are turned away until tomorrow (UTC).
    pub napping: bool,
}

/// Another of your devices asks to be linked to your key, through your home node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LinkAsk {
    pub id: u64,
    pub label: String,
    /// Four digits the asking device shows too: the person compares them before saying yes.
    pub check: String,
    pub device: String,
    pub at: u64,
}

/// The new device's side: the request is out. The digits to compare arrive with `linkCheck`
/// once a device holding your key answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LinkStarted {
    /// Devices the request went to right away; 0 means it waits for the home node.
    pub sent: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Boot {
    pub platform: String,
    pub version: String,
    pub protocol: u64,
    pub has_identity: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_by_other_tab: Option<bool>,
    /// Legacy API counter. Unsupported records are now preserved instead of deleted.
    pub cleaned: u64,
    /// Unsupported records retained on disk but not indexed or served by this build.
    pub preserved_records: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ImportReport {
    pub accepted: u64,
    pub duplicates: u64,
    pub rejected: u64,
    #[serde(serialize_with = "sorted")]
    pub reasons: HashMap<String, u64>,
}

/// Writes a map with its keys in order, as a `BTreeMap` would (UniFFI only carries `HashMap`).
fn sorted<S: serde::Serializer>(m: &HashMap<String, u64>, s: S) -> Result<S::Ok, S::Error> {
    s.collect_map(m.iter().collect::<BTreeMap<_, _>>())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PersonView {
    pub person: Person,
    pub communities: Vec<CommunityRef>,
    pub posts: u64,
    pub first_seen: u64,
}

/// A key made and waiting for its identity work (draft → mine → publish, like stamps). Hosts
/// that can show the work as it happens (the web app's first run) mine it themselves.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityDraft {
    /// did:key
    pub root: String,
    pub root_key: String,
    pub short: String,
    pub fingerprint: String,
    /// Identity work to mine, bits.
    pub bits: u32,
}

/// A record waiting for its proof-of-work (draft/publish flow). Hosts that can mine in
/// parallel (web workers, native threads) take the puzzle, mine, then publish.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftDto {
    pub token: u64,
    /// base64url
    pub prefix: String,
    /// base64url
    pub suffix: String,
    pub bits: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Event {
    #[serde(rename = "changed")]
    Changed { communities: Vec<String> },
    #[serde(rename = "notice")]
    Notice { text: String },
    /// The network as the UI shows it changed: a device paired or left, the home node came or went.
    #[serde(rename = "net")]
    Net { status: Box<NetStatus> },
    /// A new device of yours asks to be linked (root devices only). Answer with `linkAnswer`.
    #[serde(rename = "linkAsk")]
    LinkAsk { ask: LinkAsk },
    /// On a computer: a browser it signs for asks for a signature that waits for a yes
    /// (crate::remote). Answer with `signAnswer`.
    #[serde(rename = "signAsk")]
    SignAsk { ask: crate::remote::SignAsk },
    /// On a browser whose computer signs for it: the answer to a record it sent.
    #[serde(rename = "signed")]
    Signed { id: String, ok: bool, message: Option<String> },
    /// A link preview arrived (crate::preview): ask for it again.
    #[serde(rename = "preview")]
    Preview { url: String },
    /// On the new device: the digits to compare with your other device, one per device holding
    /// your key that answered (usually one).
    #[serde(rename = "linkCheck")]
    LinkCheck { checks: Vec<String> },
    /// This device was linked to your key through the home node.
    #[serde(rename = "linked")]
    Linked { me: Box<Me> },
    /// A public node this device met asks the person to read its operator's notice before it
    /// serves them; `nodeNotices` and `net.pendingNotices` hold the words. Answer with
    /// `acceptNotice` or `declineNotice`.
    #[serde(rename = "nodeNotice")]
    NodeNotice { node: String },
}

/// Whether this person shares whom they trust in a community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TrustSharing {
    pub community: String,
    pub shared: bool,
    /// Friends the statement names: those known in the community.
    pub named: u32,
    /// Friends in all.
    pub friends: u32,
}

/// A labeler: someone whose labels count on this device, because this person follows them or
/// a community they are in names them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LabelerView {
    pub person: Person,
    pub followed: bool,
    /// Communities that name them as a default labeler.
    pub default_in: Vec<CommunityRef>,
    /// Their labels held here.
    pub labels: u32,
}

/// A link's preview, fetched by this device or its home node, kept here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LinkPreview {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub site: String,
    /// The picture, sanitized, as a `data:` URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// A page of a community's wiki, in a list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct WikiPageSummary {
    pub page: String,
    pub chars: u64,
    pub updated_at: u64,
    pub editors: u32,
}

/// A page of a community's wiki, as its edits made it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct WikiPageView {
    pub page: String,
    pub text: String,
    /// 0 for a page nobody wrote yet.
    pub updated_at: u64,
    pub edits: u32,
    /// The most recent editors first, at most 8.
    pub editors: Vec<Person>,
    pub can_edit: bool,
}

/// An aggregator this person follows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AggregatorView {
    pub person: Person,
    /// Communities of yours it counts for.
    pub communities: u32,
}

/// What an aggregator this person follows counts for a post.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TallyView {
    pub by: String,
    pub key: String,
    pub up: u64,
    pub down: u64,
    pub at: u64,
}

/// A list keeper whose blocklist this person follows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct BlocklistKeeper {
    pub person: Person,
    /// Exact media hashes and picture fingerprints their lists hold here.
    pub exact: u32,
    pub pdq: u32,
}
