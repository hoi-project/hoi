//! Scattered delivery in the core (SCATTER.md §5, PROTOCOL.md §21): a node that holds pieces
//! answers sealed requests, a public node passes requests on to holders (crate::forward), and
//! any device keeps an object in pieces on holders it reaches through its forwarders and fetches
//! it back the same way: two hops, so a holder never sees who stores or fetches (phase S3).
//!
//! **Holding.** A host turns the role on with [`Node::set_piece_store`] (hoi-node
//! `--hold-pieces`). The node then publishes an X25519 key in its hellos, signed by its endpoint
//! key and derived from it for each 30-day period, so nothing new is stored and the key changes
//! by itself; envelopes to last period's key are still opened for 7 days. Requests are rate
//! limited per link; a put must carry a stamp bound to this holder, the locator, the length and
//! the expiry day; nothing about who sent a piece is kept.
//!
//! **Storing and loading.** [`Node::scatter_store`] seals content, sizes a generation for the
//! level from the independent places among holders its forwarders reach (exact arithmetic,
//! `size`), leaving out the forwarders' own places, places one piece per place under the caps
//! of `hoi_sync::place`, and sends each holder its piece sealed to it through a forwarder in
//! another place: at most k−1 indexes of a generation through one forwarder place, at least two
//! forwarder places. When every holder confirmed, [`Node::scatter_take`] hands back the
//! manifest. [`Node::scatter_load`] asks for k+2 pieces the same way, checks each against the
//! manifest, rebuilds from the first k and opens the object. A holder that refuses is replaced
//! by another place for the same index; one whose answer never came is tried through another
//! forwarder, then replaced by a fresh index, so no index is ever placed twice.
//!
//! Holders a device can reach are those its forwarders vouch for in their node lists, with the
//! holder's signed key (crate::nodes); a holder counts for placement when this device saw it
//! long and steadily enough itself, or its own devices or two forwarders in different places
//! vouch for it (SCATTER.md §5.7). Mailboxes (crate::mailbox) still reach their holders one hop.

use std::collections::{BTreeMap, BTreeSet};

use hoi_crypto::envelope::{self, DropAuth, Dropped, Kind, Reply, Request, Stored, XKey};
use hoi_crypto::scatter::{self as sc, Shards, size};
use hoi_store::pieces::{PieceMeta, PieceStats, PieceStore, PutOutcome};
use hoi_sync::place::{self, Candidate};
use hoi_sync::{HolderKey, Msg, PieceOp, RtcBinding, Vouch};
use hoi_types::scatter::Manifest;
use hoi_types::{Id, Sig};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::api::R;
use crate::net::{PeerMeta, Role};
use crate::{ApiError, Node};

/// A holder key's period, and how long after it ends envelopes to it are still opened.
pub const KEY_PERIOD: u64 = 30 * 86_400;
pub const KEY_GRACE: u64 = 7 * 86_400;
/// The longest a piece is kept; posts that live longer are made into a new generation first.
pub const MAX_TTL_DAYS: u32 = 400;
/// Requests a linked peer may send a holder per minute, and put bytes it may send.
pub const REQUESTS_PER_MINUTE: u32 = 600;
pub const PUT_BYTES_PER_MINUTE: u64 = 16 << 20;
/// Seconds an answer may take before the request counts as unanswered.
pub const ANSWER_TIMEOUT: u64 = 60;
/// Holders one index of a store, or one load, may try before giving up.
const MAX_TRIES: u32 = 4;
/// Spare pieces a load asks for beyond k ("Use less data" asks for one).
pub const SPARES: usize = 2;
/// Spare pieces a load asks for with "Use less data" and no home node.
pub const LESS_SPARES: usize = 1;
/// Holder keys remembered from vouches and hellos, at most.
const MAX_KEYS: usize = 4096;
/// How long a holder two forwarder places could not reach is left out of new placements.
const UNREACHED_FOR: u64 = 6 * 3600;
/// Operations kept at once, finished ones not yet taken included.
const MAX_OPS: usize = 256;
const DAY: u64 = 86_400;
/// The weight of one hourly sighting: a week of samples makes up most of a holder's record.
const SAMPLE_WEIGHT: f64 = 1.0 / 168.0;
/// Holders remembered at most, and how long one not seen since is remembered.
const MAX_SEEN: usize = 4096;
const FORGET_AFTER: u64 = 30 * DAY;
/// The blob sightings and mailbox progress are kept in: its own, so older releases, which
/// never read it, still open the store.
pub(crate) const BLOB: &str = "scatter";

/// When a holder counts for a post's pieces (SCATTER.md §5.7): this device has known it for
/// `known` seconds, sampled it in about `samples` hours of its own running, and found it linked
/// in at least `up` of them. A node that just appeared, or comes and goes, holds nothing yet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HolderBar {
    pub known: u64,
    pub samples: f64,
    pub up: f64,
}

impl Default for HolderBar {
    fn default() -> Self {
        HolderBar { known: 2 * DAY, samples: 11.0, up: 0.8 }
    }
}

impl HolderBar {
    /// Every linked holder counts at once (tests of other things).
    #[cfg(test)]
    pub(crate) fn none() -> HolderBar {
        HolderBar { known: 0, samples: 0.0, up: 0.0 }
    }
}

/// What this device saw of one holder, first hand.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub(crate) struct Seen {
    pub first: u64,
    /// When it was last linked, at an hourly sample.
    pub last: u64,
    /// Decaying counts of hourly samples taken while this device ran, and of those it was linked in.
    pub checked: f64,
    pub present: f64,
    /// The routes it gave for itself when last linked: how to find it again, and how a group
    /// that still needs mailboxes reaches holders before it is linked to enough.
    pub routes: Vec<String>,
}

/// The introducer every holder that counts only thanks to a community's rosters is counted
/// against: together they take at most ⌈n/4⌉ of a generation's pieces.
pub(crate) const ROSTER_INTRODUCER: u64 = u64::MAX - 1;

/// What scattered delivery keeps between starts.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ScatterSaved {
    pub holders: BTreeMap<Id, Seen>,
    pub drop_caps: BTreeMap<Id, crate::drop_share::Capability>,
    pub home_delivery: BTreeMap<Id, String>,
    /// Per private group, the drop keys this committer's device holds: the one its mailbox list
    /// names, and those it replaced while pieces stored with them may live (crate::keep).
    pub held_keys: BTreeMap<Id, Vec<crate::keep::HeldKey>>,
    /// Removed posts' generations this device drops with those keys once its turn comes.
    pub due_drops: Vec<crate::keep::DueDrop>,
    pub mailboxes: BTreeMap<Id, crate::mailbox::Progress>,
    /// Invitations this committer's device stores for newcomers to pick up (crate::pickup).
    pub pickups: Vec<crate::pickup::PickupOut>,
    /// This device's own posts, comments and edits of open communities that went whole: too few
    /// places were around to keep them in pieces (crate::open).
    pub whole: BTreeSet<Id>,
    /// Open communities' media items being rebuilt from their objects' chunks.
    pub open_partial: BTreeMap<Id, crate::private::PartialMedia>,
    /// Objects this device keeps track of: their generations, probes and repairs (crate::keep).
    pub kept: BTreeMap<Id, crate::keep::Kept>,
    /// This device's own new records sent to an injector, and until when it waits for them to
    /// come back from elsewhere before it offers them itself (crate::inject). Saved, so a restart
    /// inside the wait keeps the hold.
    pub injecting: BTreeMap<Id, u64>,
    /// Own records that came back from elsewhere while they were still held: an injector took
    /// them, and this device never offered them first (crate::inject).
    pub injected: BTreeSet<Id>,
    /// This device's own posts, comments and edits of open communities kept in pieces, and the
    /// pointer records that went out for them (crate::open).
    pub own_pointers: BTreeMap<Id, BTreeSet<Id>>,
    /// Objects a put of went in two hops, not three, and when: the forwarder it went through saw
    /// the holder (crate::inject).
    pub two_hop: BTreeMap<Id, u64>,
    /// When this device first offered one of its person's own records to a link that is not one
    /// of the person's devices before it came back from elsewhere: from then on that link could
    /// tell this address's records apart (crate::inject).
    pub exposed_at: Option<u64>,
    /// On a home node: the people whose devices paired with it, learned from the identities and
    /// blessings those devices handed it. Their new records it injects as its own (crate::inject).
    pub client_roots: BTreeSet<Id>,
}

/// What a linked peer said and did about scattered delivery.
#[derive(Debug, Default, Clone)]
pub(crate) struct PeerScatter {
    /// Its holder key, kept only when its signature checked out against the endpoint the
    /// transport authenticated (or, over WebRTC, the endpoint its pinned certificate is bound to).
    pub holder: Option<HolderKey>,
    /// The holders its last node list vouched for, kept only from the person's own devices and
    /// links this device dialed (crate::nodes), never its vouch for itself.
    pub vouches: Vec<Vouch>,
    /// It passes `piece` requests on to holders (`fw` in its hello).
    pub forwards: bool,
    /// Our home node rebuilds for us (`rb` in its hello, crate::rebuild).
    pub rebuilds: bool,
    minute: u64,
    requests: u32,
    bytes: u64,
}

/// How an operation ended, or that it has not yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScatterOutcome {
    Pending,
    /// The object is kept in pieces; the manifest says where and how to open it.
    Stored(Manifest),
    /// The object, rebuilt and opened.
    Loaded(Vec<u8>),
    /// What each index probed answered (`has`, crate::keep).
    Probed(Vec<ProbeResult>),
    /// Individual holder observations, never a claim of global erasure (RFC-0001).
    Dropped(Vec<DropResult>),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropOutcome {
    /// No request left this attempt: unavailable route/key or its per-place capacity.
    Queued,
    Acknowledged,
    LegacyRemoval,
    Unknown,
    Refused,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropResult {
    pub index: u8,
    pub outcome: DropOutcome,
}

/// One index's answer to a probe: kept, not kept (the holder said so), or no answer (none in
/// time, or the forwarder said `gone`); and the place key of the forwarder it went through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeResult {
    pub index: u8,
    pub kept: Option<bool>,
    pub place: u64,
}

#[derive(Default)]
pub(crate) struct ScatterState {
    /// Present while this node holds pieces.
    pub pieces: Option<Box<dyn PieceStore>>,
    next_rid: u32,
    next_op: u64,
    pub(crate) pending: BTreeMap<(String, u32), Pending>,
    ops: BTreeMap<u64, Op>,
    /// Completed placements remain available until their outcome is consumed, so deletion
    /// captures even a failed placement's possibly stored pieces before discarding it.
    placements: BTreeMap<u64, Manifest>,
    /// Sightings and mailbox progress, saved in [`BLOB`].
    pub saved: ScatterSaved,
    pub dirty: bool,
    sampled_hour: u64,
    pub bar: HolderBar,
    /// Mailbox polling and retries, in memory only (crate::mailbox).
    pub mail: crate::mailbox::Runtime,
    /// For holders others vouched for: the introducer this device first heard it from, which
    /// the per-introducer cap of a generation counts it against (SCATTER.md §5.7).
    introduced: BTreeMap<Id, u64>,
    /// Holder keys whose signatures checked out, from hellos and vouches: what envelopes to a
    /// holder this device is not linked to are sealed to.
    pub(crate) keys: BTreeMap<Id, HolderKey>,
    /// Holders requests went nowhere for lately, and through which forwarder places, until when:
    /// once two places say so, new pieces go elsewhere for a while (a claim of one forwarder
    /// alone never takes a holder away).
    unreached: BTreeMap<Id, (u64, BTreeSet<u64>)>,
    /// This node as a forwarder (crate::forward).
    pub forward: crate::forward::Forwarding,
    /// What finished operations are for (crate::keep).
    pub jobs: BTreeMap<u64, crate::keep::Job>,
    /// Repair and probing in memory (crate::keep).
    pub repair: crate::keep::Runtime,
    /// The drop key the next store's pieces are kept with (its id), set by whoever starts it
    /// for a community that has one (crate::keep).
    pub(crate) next_drop_key: Option<[u8; 16]>,
    /// The community the next placement is for, set by whoever starts it: its rosters count.
    pub(crate) placing: Option<Id>,
    /// Rebuilds asked of our home node, or run for our devices (crate::rebuild).
    pub(crate) rebuilds: crate::rebuild::Rebuilds,
    pub(crate) home_scatter: crate::home_scatter::Runtime,
    pub(crate) drop_share: crate::drop_share::Runtime,
    /// Records of open communities kept in pieces here: never offered whole (crate::open).
    pub(crate) in_pieces: BTreeSet<Id>,
    /// What an open community's pointer records say they hold, by the record: its kind, its
    /// community and its author's person, so votes and replies to it are taken before it is
    /// rebuilt, and by nodes that never rebuild it (crate::open).
    pub(crate) standins: BTreeMap<Id, crate::open::Standin>,
    /// When this device last made its blessing known to an open community's line (crate::open).
    pub(crate) announced: BTreeMap<Id, u64>,
    /// Puts sent in three hops (S7), for tests and `admin pieces`.
    pub(crate) wrapped: u64,
    /// Throttle the user-facing notice when a deletion has no three-hop path.
    last_drop_notice: u64,
    /// Records this node injects for others, and when (crate::inject).
    pub(crate) injections: Vec<(u64, Vec<Vec<u8>>)>,
    /// The key period this holder's links last heard its key for: a new period's key goes to
    /// every link at once, not only to links made since (S6).
    hello_period: u64,
}

pub(crate) struct Pending {
    op: u64,
    index: u8,
    /// The forwarder it went through, if any.
    pub(crate) via: Option<String>,
    pub(crate) kind: Kind,
    pub(crate) reply: XKey,
    pub(crate) loc: [u8; 32],
    pub(crate) want: usize,
    pub(crate) sent_at: u64,
    /// A mailbox request rather than a piece of a store or load (crate::mailbox).
    pub(crate) mail: Option<crate::mailbox::MailRequest>,
}

impl Pending {
    pub(crate) fn mail(kind: Kind, reply: XKey, loc: [u8; 32], want: usize, sent_at: u64, request: crate::mailbox::MailRequest) -> Pending {
        Pending { op: 0, index: 0, via: None, kind, reply, loc, want, sent_at, mail: Some(request) }
    }
}

enum Op {
    Store(Box<StoreOp>),
    Load(Box<LoadOp>),
    Probe(Box<ProbeOp>),
    /// Done elsewhere: a rebuild the home node runs for this device (crate::rebuild).
    Wait,
    Done(Box<ScatterOutcome>),
}

struct ProbeOp {
    asked: BTreeSet<u8>,
    holders: BTreeMap<u8, Id>,
    /// Per index: kept, not kept, or no answer; and the place key of the forwarder asked.
    answers: BTreeMap<u8, (Option<bool>, u64)>,
    fkey: BTreeMap<String, u64>,
    drops: Option<BTreeMap<u8, DropOutcome>>,
    /// Marker capability verified against each holder's signed key when the request was sent.
    durable: BTreeSet<u8>,
}

/// What a probe asks each holder: whether it keeps its piece, or to drop it with the creator's
/// secret or the community drop key's secret.
pub(crate) enum Ask {
    Has,
    DropToken([u8; 32]),
    DropTokens(BTreeMap<Id, [u8; 32]>),
    DropKey([u8; 32]),
}

struct AskTarget {
    index: u8,
    holder: Id,
    loc: [u8; 32],
    auth: Option<DropAuth>,
}

/// What a forwarder's place is called across operations: its network, else its operator, else
/// the link itself. Two probes count as "from different places" when these differ.
pub(crate) fn place_key(c: &Candidate) -> u64 {
    c.network.or(c.operator).unwrap_or(c.introducer)
}

/// A node's network and block, as keys.
type Spot = (Option<u64>, Option<u64>);

/// Where a route says a node is: its network and block.
pub(crate) fn route_place(route: &str) -> Spot {
    let meta = PeerMeta { dialed: vec![route.to_string()], ..Default::default() };
    (crate::warmth::network_key(&meta), crate::warmth::block_key(&meta))
}

pub(crate) const MISMATCH: &str = "These pieces do not make the post they were said to be.";
const FEW_PATHS: &str = "Too few independent places to send pieces through are connected right now. It stays as it is for now.";
const TOO_FEW_ONLINE: &str = "Too few places are online for this post right now. It is still kept; try again later.";

/// What an object can use now (`Node::plan`).
pub(crate) struct Plan {
    /// Forwarder link → its place.
    fplace: BTreeMap<String, usize>,
    /// Holders that may take pieces, outside the forwarders' places.
    holders: Vec<Reachable>,
}

impl Plan {
    /// The places the holders make: what a generation is sized from.
    pub(crate) fn places(&self) -> usize {
        let candidates: Vec<Candidate> = self.holders.iter().map(|h| h.candidate).collect();
        place::count_places(&candidates, &[])
    }

    pub(crate) fn forwarder_places(&self) -> usize {
        self.fplace.values().collect::<BTreeSet<_>>().len()
    }

    /// At least two forwarder places, and room through them for `n` pieces at k−1 a place.
    fn check_paths(&self, k: usize, n: usize) -> R<()> {
        let places = self.forwarder_places();
        if places < 2 || places * (k - 1) < n {
            return Err(ApiError::new(
                "scatter-few-paths",
                format!(
                    "Only {places} independent places to send pieces through are connected: too few to keep this in pieces safely. It stays as it is for now."
                ),
            ));
        }
        Ok(())
    }

    fn paths(&self, k: usize) -> Paths {
        Paths {
            place: self.fplace.clone(),
            carried: BTreeMap::new(),
            holders: self.holders.iter().map(|h| (h.endpoint, (h.xk, h.via.iter().filter(|f| self.fplace.contains_key(*f)).cloned().collect()))).collect(),
            cap: k - 1,
        }
    }
}

/// The forwarders an operation may use, fixed when it starts: each link's place, and the
/// distinct indexes of the generation it carried, so no forwarder place ever carries k.
#[derive(Default)]
struct Paths {
    /// Forwarder link → its place, numbered as the operation numbered them.
    place: BTreeMap<String, usize>,
    /// Forwarder place → indexes of this generation sent through it.
    carried: BTreeMap<usize, BTreeSet<u8>>,
    /// Holder → its signed key and the forwarders that reach it.
    holders: BTreeMap<Id, (/* xk */ [u8; 32], Vec<String>)>,
    /// At most this many indexes through one forwarder place: k−1.
    cap: usize,
}

impl Paths {
    /// A forwarder for index `index` to `holder`, not in `avoid` and not over its place's cap,
    /// the least loaded first (and at random among equals).
    fn choose(&self, holder: &Id, index: u8, avoid: &BTreeSet<String>, live: &BTreeSet<String>) -> Option<String> {
        let (_, via) = self.holders.get(holder)?;
        let load = |f: &String| -> Option<usize> {
            let place = self.place.get(f)?;
            let carried = self.carried.get(place).map(|c| (c.len(), c.contains(&index))).unwrap_or((0, false));
            let after = if carried.1 { carried.0 } else { carried.0 + 1 };
            (after <= self.cap).then_some(carried.0)
        };
        let mut options: Vec<(usize, &String)> =
            via.iter().filter(|f| !avoid.contains(*f) && live.contains(*f)).filter_map(|f| load(f).map(|l| (l, f))).collect();
        let least = options.iter().map(|(l, _)| *l).min()?;
        options.retain(|(l, _)| *l == least);
        Some(options[uniform(options.len())].1.clone())
    }

    /// Whether any live forwarder's place may still carry `index`.
    fn has_room(&self, index: u8, live: &BTreeSet<String>) -> bool {
        self.place.iter().filter(|(f, _)| live.contains(*f)).any(|(_, place)| {
            let c = self.carried.get(place);
            c.is_some_and(|c| c.contains(&index)) || c.map_or(0, |c| c.len()) < self.cap
        })
    }

    /// Holders of `manifest` no forwarder of this device vouches for: reached with the key the
    /// manifest lists for them, through any forwarder, which passes the request to one of its
    /// own links that vouches for them (S6). A forwarder that is the holder itself answers
    /// `gone`, and the request goes through another.
    fn relay_unvouched(&mut self, manifest: &Manifest, forwarders: &[Forwarder]) {
        for (i, h) in manifest.holders.iter().enumerate() {
            let Some(xk) = manifest.keys.get(i) else { break };
            if self.holders.contains_key(h) {
                continue;
            }
            let via: Vec<String> = forwarders.iter().map(|f| f.peer.clone()).collect();
            if !via.is_empty() {
                self.holders.insert(*h, (*xk, via));
            }
        }
    }

    /// Whether two forwarders are in different places.
    fn apart(&self, a: &str, b: &str) -> bool {
        matches!((self.place.get(a), self.place.get(b)), (Some(x), Some(y)) if x != y)
    }

    fn carry(&mut self, forwarder: &str, index: u8) {
        if let Some(&place) = self.place.get(forwarder) {
            self.carried.entry(place).or_default().insert(index);
        }
    }
}

struct StoreOp {
    manifest: Manifest,
    shards: Shards,
    expiry_day: u32,
    /// The community drop key the pieces are kept with, or zeros.
    drop_key_id: [u8; 16],
    /// Per index placed: its holder's endpoint, whether it confirmed, how often it was tried.
    slots: Vec<Slot>,
    /// Endpoints that held, or were asked to hold, any index of this generation.
    used: BTreeSet<Id>,
    paths: Paths,
    /// Candidates of this operation, for replacements: the reachable holders that may take
    /// pieces, outside the forwarders' places.
    candidates: Vec<Reachable>,
}

struct Slot {
    holder: Id,
    stored: bool,
    /// Lost for good (unanswered, replaced by a fresh index): never stored here again.
    lost: bool,
    tries: u32,
    /// Forwarders this index already went through to this holder.
    via: BTreeSet<String>,
}

struct LoadOp {
    manifest: Manifest,
    got: BTreeMap<u8, Vec<u8>>,
    asked: BTreeSet<u8>,
    tries: u32,
    paths: Paths,
    /// Answers that said a holder was full or slow: how hot the object is (crate::keep).
    trouble: u32,
    /// Pieces that came but did not match their hash.
    bad: usize,
}

/// A link this device sends pieces through.
pub(crate) struct Forwarder {
    pub peer: String,
    pub candidate: Candidate,
}

/// A forwarder link as a three-hop upload sees it (S7).
struct Hop {
    peer: String,
    endpoint: Id,
    xk: Option<[u8; 32]>,
    reaches: BTreeSet<Id>,
}

/// A holder this device reaches through its forwarders (S3).
#[derive(Clone)]
pub(crate) struct Reachable {
    pub endpoint: Id,
    pub xk: [u8; 32],
    pub candidate: Candidate,
    /// The forwarder links that vouch for it, so can pass envelopes to it.
    pub via: Vec<String>,
    /// It may take pieces: seen long and steadily enough first hand, or vouched for by the
    /// person's own devices or two forwarders in different places.
    pub qualified: bool,
    /// Its routes, as the first voucher that gave any gave them.
    pub routes: Vec<String>,
}

/// A linked holder this device could put pieces on.
pub(crate) struct LinkedHolder {
    pub peer: String,
    pub endpoint: Id,
    pub xk: [u8; 32],
    pub candidate: Candidate,
    /// Its published routes, as its hello gave them.
    pub routes: Vec<String>,
    /// It cleared the bar of first-hand sightings ([`HolderBar`]), or those who vouch for it
    /// make up for them ([`Node::vouched_by`]).
    pub qualified: bool,
}

/// A short key for an introducer: its authenticated endpoint, or the certificate this device
/// pinned when it dialed it.
fn introducer_key(p: &crate::node::PeerState) -> u64 {
    if let Some(e) = p.meta.endpoint.as_deref().and_then(|e| Id::from_hex(e).ok()) {
        return u64::from_le_bytes(e.0[..8].try_into().expect("eight bytes"));
    }
    let pinned = p.meta.dialed.iter().find_map(|r| crate::net::rtc_parts(r).map(|(fp, _)| fp.to_string())).unwrap_or_else(|| p.route.clone());
    u64::from_le_bytes(blake3::hash(pinned.as_bytes()).as_bytes()[..8].try_into().expect("eight bytes"))
}

/// The certificates this device pinned when it dialed a WebRTC link (one node's routes).
fn pinned_certs(meta: &crate::net::PeerMeta) -> Vec<[u8; 32]> {
    if meta.transport != "webrtc" {
        return Vec::new();
    }
    meta.dialed
        .iter()
        .filter_map(|r| crate::net::rtc_parts(r))
        .filter_map(|(fp, _)| hoi_types::uri::unb64(fp).ok())
        .filter_map(|b| <[u8; 32]>::try_from(b).ok())
        .collect()
}

/// A uniform index below `len` from the system's randomness (rejection sampling).
pub(crate) fn uniform(len: usize) -> usize {
    let len = len as u64;
    let zone = u64::MAX - u64::MAX % len;
    loop {
        let x = u64::from_le_bytes(hoi_crypto::random_bytes::<8>().unwrap_or([0; 8]));
        if x < zone {
            return (x % len) as usize;
        }
    }
}

fn kind_of(op: PieceOp) -> Kind {
    match op {
        PieceOp::Put => Kind::Put,
        PieceOp::Get => Kind::Get,
        PieceOp::Has => Kind::Has,
        PieceOp::Drop => Kind::Drop,
        PieceOp::Stored => Kind::Stored,
        PieceOp::Got => Kind::Got,
        PieceOp::Had => Kind::Had,
        PieceOp::Dropped => Kind::Dropped,
        // never opened as requests or answers: a forwarder's word that the request went nowhere,
        // and the three-hop layers, opened by their own functions (S7)
        PieceOp::Gone | PieceOp::Wrap | PieceOp::Inject => Kind::Dropped,
    }
}

fn op_of(kind: Kind) -> PieceOp {
    match kind {
        Kind::Put => PieceOp::Put,
        Kind::Get => PieceOp::Get,
        Kind::Has => PieceOp::Has,
        Kind::Drop => PieceOp::Drop,
        Kind::Stored => PieceOp::Stored,
        Kind::Got => PieceOp::Got,
        Kind::Had => PieceOp::Had,
        Kind::Dropped => PieceOp::Dropped,
    }
}

fn busy() -> ApiError {
    ApiError::new("scatter-busy", "Too many pieces are on their way right now; try again in a moment.")
}

fn few_places(places: usize) -> ApiError {
    ApiError::new(
        "scatter-few-places",
        format!("Only {places} independent places that keep pieces are connected: too few to keep this in pieces safely. It stays as it is for now."),
    )
}

impl Node {
    // ------------------------------------------------------------------ holding

    /// Turns the holder role on (with where pieces are kept) or off. Linked peers hear of it in
    /// a fresh hello.
    pub fn set_piece_store(&mut self, store: Option<Box<dyn PieceStore>>) {
        self.scatter.pieces = store;
        // the notice says whether this node keeps pieces (hoi_types::notice::PIECES)
        if let Some(n) = self.operator_notice().filter(|n| n.pieces != self.holds_pieces()).cloned() {
            let _ = self.set_notice(Some(n));
        }
        let peers: Vec<String> = self.peers.iter().filter(|(_, p)| p.said_hello).map(|(k, _)| k.clone()).collect();
        for p in peers {
            self.rehello(&p);
        }
    }

    /// Whether this node keeps pieces for others now.
    pub fn holds_pieces(&self) -> bool {
        self.scatter.pieces.is_some()
    }

    /// What this node holds, while it holds pieces.
    pub fn piece_stats(&self) -> Option<PieceStats> {
        self.scatter.pieces.as_ref().map(|p| p.stats())
    }

    fn own_endpoint(&self) -> Option<Id> {
        self.transport_kp.as_ref().map(|k| k.public())
    }

    #[cfg(test)]
    pub(crate) fn own_endpoint_id(&self) -> Option<Id> {
        self.own_endpoint()
    }

    /// The holder key of a period: derived from the endpoint key, so it needs no storage of
    /// its own and changes by itself.
    fn holder_key(&self, period: u64) -> Option<XKey> {
        let t = self.transport_kp.as_ref()?;
        let mut material = t.secret().to_vec();
        material.extend_from_slice(&period.to_be_bytes());
        let mut seed = blake3::derive_key("hoi scatter xk seed v1", &material);
        material.zeroize();
        let key = XKey::from_seed(&seed).ok();
        seed.zeroize();
        key
    }

    /// A holder whose key period turned tells its links its new key, by a hello that replaces
    /// the one they had: forwarders vouch with it, and devices seal to it. Links that stay up for
    /// months would otherwise keep a key that has expired.
    fn announce_new_key(&mut self) {
        if self.scatter.pieces.is_none() {
            return;
        }
        let period = self.now() / KEY_PERIOD;
        if self.scatter.hello_period == period {
            return;
        }
        let first = self.scatter.hello_period == 0;
        self.scatter.hello_period = period;
        if first {
            return;
        }
        let peers: Vec<String> = self.peers.iter().filter(|(_, p)| p.said_hello && !p.awaiting_hello).map(|(k, _)| k.clone()).collect();
        for peer in peers {
            self.rehello(&peer);
        }
    }

    /// The signed key this node's hellos carry while it holds pieces, with the binding of its
    /// WebRTC certificate to its endpoint when it serves WebRTC (for browsers).
    pub(crate) fn holder_hello(&self) -> Option<HolderKey> {
        let store = self.scatter.pieces.as_ref()?;
        let t = self.transport_kp.as_ref()?;
        let period = self.now() / KEY_PERIOD;
        let xk = self.holder_key(period)?.public();
        let expires_at = (period + 1) * KEY_PERIOD + KEY_GRACE;
        let sig = t.sign(&envelope::xk_statement(&t.public().0, &xk, expires_at));
        let cert = self
            .rtc_routes
            .iter()
            .filter_map(|r| crate::net::rtc_parts(r))
            .find_map(|(fp, _)| hoi_types::uri::unb64(fp).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()));
        let rtc = cert.map(|cert| RtcBinding { endpoint: t.public(), cert, sig: t.sign(&envelope::rtc_statement(&t.public().0, &cert)).0 });
        let drop_journal = store.durable_drops().then(|| t.sign(&envelope::drop_journal_statement(&t.public().0, &xk, expires_at)).0);
        Some(HolderKey { xk, expires_at, sig: sig.0, rtc, drop_journal })
    }

    /// Keeps a peer's holder key if its signature checks out against the endpoint the
    /// transport authenticated, and it has not expired. Over WebRTC no endpoint is
    /// authenticated, only the certificate this device pinned when it dialed: a binding of that
    /// certificate to an endpoint, signed by the endpoint, makes it that endpoint's link.
    pub(crate) fn note_holder_key(&mut self, peer: &str, key: Option<HolderKey>) {
        let now = self.now();
        let Some(p) = self.peers.get_mut(peer) else { return };
        let mut endpoint = p.meta.endpoint.as_deref().and_then(|e| Id::from_hex(e).ok());
        if endpoint.is_none()
            && let Some(b) = key.and_then(|k| k.rtc)
            && pinned_certs(&p.meta).contains(&b.cert)
            && envelope::rtc_valid(&b.endpoint.0, &b.cert, &Sig(b.sig))
        {
            p.meta.endpoint = Some(b.endpoint.hex());
            endpoint = Some(b.endpoint);
        }
        p.scatter.holder = match (key, endpoint) {
            (Some(k), Some(e)) if envelope::xk_valid(&e.0, &k.xk, k.expires_at, &Sig(k.sig), now) => Some(k),
            _ => None,
        };
        if let (Some(k), Some(e)) = (p.scatter.holder, endpoint) {
            self.remember_key(e, HolderKey { rtc: None, ..k });
        }
    }

    /// Keeps a holder key whose signature checked out, the latest per holder.
    fn remember_key(&mut self, endpoint: Id, mut key: HolderKey) {
        let now = self.now();
        key.drop_journal =
            key.drop_journal.filter(|sig| hoi_crypto::verify(&endpoint, &envelope::drop_journal_statement(&endpoint.0, &key.xk, key.expires_at), &Sig(*sig)));
        let keys = &mut self.scatter.keys;
        if keys.get(&endpoint).is_some_and(|k| k.expires_at > key.expires_at || (k.expires_at == key.expires_at && k.drop_journal == key.drop_journal)) {
            return;
        }
        if keys.len() >= MAX_KEYS && !keys.contains_key(&endpoint) {
            keys.retain(|_, k| k.expires_at > now);
            if keys.len() >= MAX_KEYS {
                return;
            }
        }
        keys.insert(endpoint, key);
    }

    /// A holder's key, if one is known and unexpired.
    pub(crate) fn key_of(&self, endpoint: &Id) -> Option<[u8; 32]> {
        let now = self.now();
        self.scatter.keys.get(endpoint).filter(|k| k.expires_at > now).map(|k| k.xk)
    }

    /// The holders this node vouches for in its node lists (SCATTER.md §5.7): those it saw
    /// itself, at its own hourly samples, long and steadily enough to take pieces. Never what it
    /// was told. Steadiest first; the share is rounded down to five percent and the days are
    /// capped, so lists change seldom.
    pub(crate) fn vouches(&self) -> Vec<Vouch> {
        let now = self.now();
        let mut out: Vec<Vouch> = self
            .scatter
            .saved
            .holders
            .iter()
            .filter(|(e, s)| s.checked > 0.0 && self.qualifies(e))
            .map(|(e, s)| {
                let share = (s.present / s.checked).clamp(0.0, 1.0);
                let up = ((share * 100.0 + 1e-9) as u8) / 5 * 5;
                let days = (now.saturating_sub(s.first) / DAY).min(hoi_sync::MAX_VOUCH_DAYS as u64) as u16;
                // its key, as its own hello gave it, so devices this node forwards for can seal to it
                let key = self.scatter.keys.get(e).filter(|k| k.expires_at > now).copied();
                let routes = s.routes.iter().take(hoi_sync::MAX_VOUCH_ROUTES).cloned().collect();
                Vouch { endpoint: *e, up, days, key, routes }
            })
            .collect();
        out.sort_by(|a, b| b.up.cmp(&a.up).then(b.days.cmp(&a.days)).then(a.endpoint.cmp(&b.endpoint)));
        out.truncate(hoi_sync::MAX_VOUCHES);
        out
    }

    /// A node list's vouches, from the person's own device or a link this device dialed
    /// (crate::nodes checks which). A node's vouch for itself is dropped.
    pub(crate) fn note_vouches(&mut self, peer: &str, holders: Vec<Vouch>) {
        let now = self.now();
        for v in &holders {
            if let Some(k) = v.key
                && envelope::xk_valid(&v.endpoint.0, &k.xk, k.expires_at, &Sig(k.sig), now)
            {
                self.remember_key(v.endpoint, k);
            }
        }
        let Some(p) = self.peers.get_mut(peer) else { return };
        let own = p.meta.endpoint.as_deref().and_then(|e| Id::from_hex(e).ok());
        let key = introducer_key(p);
        p.scatter.vouches = holders.into_iter().filter(|v| Some(v.endpoint) != own).collect();
        for v in &p.scatter.vouches {
            if self.scatter.introduced.len() < MAX_SEEN || self.scatter.introduced.contains_key(&v.endpoint) {
                self.scatter.introduced.entry(v.endpoint).or_insert(key);
            }
        }
    }

    /// Whether those who vouch for a holder make up for this device's own sightings
    /// (SCATTER.md §5.7), and the introducer it counts against. The person's own devices (a
    /// paired home node, their desktop) count as this device's eyes; otherwise at least two
    /// introducers in different places, neither in the holder's own place, must vouch, each
    /// reporting the holder known and online at least as long as this device's own bar asks.
    /// One node's list alone never makes a holder count.
    pub(crate) fn vouched_by(&self, holder: &Id, candidate: &Candidate, roster: Option<&BTreeSet<Id>>) -> Option<u64> {
        let bar = self.scatter.bar;
        let enough = |v: &Vouch| (v.days as u64) * DAY >= bar.known && f64::from(v.up) + 1e-9 >= bar.up * 100.0;
        let mut introducers = vec![*candidate];
        let mut keys = Vec::new();
        for p in self.peers.values() {
            let own = p.client || p.home;
            let dialed = p.role() == Role::Peer && !p.meta.incoming;
            if !p.said_hello || !(own || dialed) || !p.scatter.vouches.iter().any(|v| v.endpoint == *holder && enough(v)) {
                continue;
            }
            if own {
                return Some(u64::from_le_bytes(holder.0[..8].try_into().expect("eight bytes")));
            }
            introducers.push(Candidate {
                operator: p.operator,
                network: crate::warmth::network_key(&p.meta),
                block: crate::warmth::block_key(&p.meta),
                introducer: introducer_key(p),
            });
            keys.push(introducer_key(p));
        }
        let place = place::places(&introducers);
        let elsewhere: BTreeSet<usize> = place[1..].iter().copied().filter(|&p| p != place[0]).collect();
        // the community's rosters, where enough established voice lists the holder, are one
        // place more; a link must still vouch for it
        let listed = roster.is_some_and(|r| r.contains(holder));
        if elsewhere.len() + usize::from(listed) < 2 || keys.is_empty() {
            return None;
        }
        // every holder that counts only thanks to the rosters counts against one introducer,
        // the rosters: at most ⌈n/4⌉ of a generation's pieces go to them, however much voice
        // a party behind them gathered
        if elsewhere.len() < 2 {
            return Some(ROSTER_INTRODUCER);
        }
        keys.sort();
        Some(self.scatter.introduced.get(holder).copied().unwrap_or(keys[0]))
    }

    /// The holders community `c`'s rosters list with enough weight: named by the current
    /// rosters of at least two established people (one each, however many devices), neither
    /// banned nor struck, whose voices together reach the community's hiding quorum, as a review
    /// must.
    pub(crate) fn roster_holders(&self, c: Id) -> BTreeSet<Id> {
        let Some(st) = self.communities.get(&c) else { return BTreeSet::new() };
        let now = self.now();
        let mut by: BTreeMap<Id, BTreeSet<Id>> = BTreeMap::new();
        for s in self.store.of_kind(&c, hoi_types::Kind::Roster) {
            let root = s.author_root;
            let struck = st.standing.people.get(&root).is_some_and(|p| p.struck);
            if s.record.expires_at <= now || struck || st.is_banned(&root) || st.level(&root) < hoi_moderation::Level::Established {
                continue;
            }
            if let hoi_types::Body::Roster { holders } = &s.record.body {
                for h in holders {
                    by.entry(*h).or_default().insert(root);
                }
            }
        }
        let quorum = st.standing.quorums.hide;
        by.into_iter()
            .filter(|(_, people)| people.len() >= hoi_moderation::ladder::MIN_HEADS && people.iter().map(|p| st.standing.voice(p)).sum::<u64>() >= quorum)
            .map(|(h, _)| h)
            .collect()
    }

    /// With "Share the holders I see" on, a desktop's or node's roster: in every
    /// community its person is established in, the holders it saw up for days, first hand, at
    /// most every twelve hours, again when they changed or the current one ends within two days.
    pub(crate) fn roster_tick(&mut self) {
        const EVERY: u64 = 12 * 3600;
        const RENEW: u64 = 2 * DAY;
        if !self.state.settings.share_holders || !self.can_relay() {
            return;
        }
        let (Some(me), Some(device)) = (self.me_root(), self.device_kp.as_ref().map(|k| k.public())) else { return };
        let now = self.now();
        let mut holders: Vec<Id> = self.vouches().into_iter().map(|v| v.endpoint).collect();
        holders.sort();
        holders.dedup();
        holders.truncate(hoi_types::MAX_ROSTER);
        if holders.is_empty() {
            return;
        }
        let communities: Vec<Id> = self.state.joined.keys().copied().collect();
        for c in communities {
            let Some(st) = self.communities.get(&c) else { continue };
            if st.is_banned(&me) || st.level(&me) < hoi_moderation::Level::Established {
                continue;
            }
            let current = self
                .store
                .of_kind(&c, hoi_types::Kind::Roster)
                .filter(|s| s.record.author == device)
                .max_by_key(|s| s.record.created_at)
                .map(|s| (s.record.created_at, s.record.expires_at, matches!(&s.record.body, hoi_types::Body::Roster { holders: h } if *h == holders)));
            let due = match current {
                None => true,
                Some((at, expires, same)) => now >= at + EVERY && (!same || expires <= now + RENEW),
            };
            if !due {
                continue;
            }
            let r = hoi_types::Record {
                v: 1,
                community: Some(c),
                author: device,
                parent: None,
                created_at: now,
                expires_at: now + hoi_validate::MAX_ROSTER_LIFE,
                body: hoi_types::Body::Roster { holders: holders.clone() },
                nonce: 0,
            };
            let _ = self.publish(r);
        }
    }

    /// A `piece` message from a linked peer.
    pub(crate) fn on_piece(&mut self, peer: &str, op: PieceOp, rid: u32, to: Option<Id>, env: Vec<u8>) -> bool {
        if op.is_request() || matches!(op, PieceOp::Wrap | PieceOp::Inject) {
            if let Some(over) = self.count_request(peer, op, env.len()) {
                // Over the limit: no answer, and a peer that keeps at it is let go.
                return over;
            }
            match (to, op) {
                (Some(holder), _) => {
                    self.forward_piece(peer, op, rid, holder, env);
                    true
                }
                (None, PieceOp::Wrap) => self.unwrap_piece(peer, rid, env),
                (None, PieceOp::Inject) => self.take_injection(peer, env),
                (None, _) => self.serve_piece(peer, op, rid, env),
            }
        } else {
            if self.forward_answer(peer, op, rid, &env) {
                return true;
            }
            self.on_answer(peer, op, rid, env);
            true
        }
    }

    /// Counts a request against its link's minute: `Some(keep the link)` when over the limit.
    /// A link that forwards carries many devices' requests and gets more.
    fn count_request(&mut self, peer: &str, op: PieceOp, len: usize) -> Option<bool> {
        let now = self.now();
        let p = self.peers.get_mut(peer)?;
        if p.scatter.minute != now / 60 {
            p.scatter.minute = now / 60;
            p.scatter.requests = 0;
            p.scatter.bytes = 0;
        }
        p.scatter.requests += 1;
        if matches!(op, PieceOp::Put | PieceOp::Wrap | PieceOp::Inject) {
            p.scatter.bytes += len as u64;
        }
        let (requests, bytes) = if p.scatter.forwards {
            (crate::forward::FORWARDER_REQUESTS_PER_MINUTE, crate::forward::FORWARDER_PUT_BYTES_PER_MINUTE)
        } else {
            (REQUESTS_PER_MINUTE, PUT_BYTES_PER_MINUTE)
        };
        if p.scatter.requests > requests || p.scatter.bytes > bytes {
            return Some(!self.peer_scores.penalize(peer, 1.0));
        }
        None
    }

    /// This node's holder keys a fresh request may be sealed to: this period's, and for a week
    /// the one before.
    pub(crate) fn fresh_keys(&self) -> Vec<XKey> {
        let now = self.now();
        let period = now / KEY_PERIOD;
        let mut periods = vec![period];
        if period > 0 && now < period * KEY_PERIOD + KEY_GRACE {
            periods.push(period - 1);
        }
        periods.into_iter().filter_map(|p| self.holder_key(p)).collect()
    }

    /// A three-hop request's outer layer, at its exit (S7): opened with this node's key, the
    /// request inside goes on to the holder it names, as any forwarded request does, and the
    /// answer comes back the same way. The link it came through never learns that holder.
    fn unwrap_piece(&mut self, peer: &str, rid: u32, env: Vec<u8>) -> bool {
        let Some(endpoint) = self.own_endpoint() else { return true };
        if !self.forwards_pieces() {
            return true;
        }
        let opened = self.fresh_keys().iter().find_map(|k| envelope::open_wrap(&endpoint.0, k, &env).ok());
        let Some((to, kind, inner)) = opened else {
            let forwarder = self.peers.get(peer).is_some_and(|p| p.scatter.forwards);
            return forwarder || !self.peer_scores.penalize(peer, 2.0);
        };
        self.forward_piece(peer, op_of(kind), rid, Id(to), inner);
        true
    }

    fn serve_piece(&mut self, peer: &str, op: PieceOp, rid: u32, env: Vec<u8>) -> bool {
        let now = self.now();
        if self.scatter.pieces.is_none() {
            return true;
        }
        let Some(endpoint) = self.own_endpoint() else { return true };
        let kind = kind_of(op);
        let period = now / KEY_PERIOD;
        let mut keys = vec![period];
        if period > 0 && now < period * KEY_PERIOD + KEY_GRACE {
            keys.push(period - 1);
        }
        // A reader may seal to the key the manifest listed when the piece was stored (S6): asks
        // for what is kept open with any key of a piece's lifetime. Puts only with fresh ones.
        if kind != Kind::Put {
            let oldest = period.saturating_sub((MAX_TTL_DAYS as u64 * DAY).div_ceil(KEY_PERIOD));
            let older: Vec<u64> = (oldest..period).rev().filter(|p| !keys.contains(p)).collect();
            keys.extend(older);
        }
        let request = keys.into_iter().filter_map(|p| self.holder_key(p)).find_map(|k| envelope::open_request(kind, &endpoint.0, &k, &env).ok());
        let Some(request) = request else {
            // Not sealed to this holder, or damaged: nothing to answer. A forwarder only passes
            // on what devices gave it, so it is not blamed for it.
            let forwarder = self.peers.get(peer).is_some_and(|p| p.scatter.forwards);
            return forwarder || !self.peer_scores.penalize(peer, 2.0);
        };
        let today = (now / DAY) as u32;
        let (answer, want) = self.answer(&endpoint, &request, today);
        let (Some(answer_kind), loc, reply) = (kind.answer(), *request.loc(), *request.reply_key()) else { return true };
        match envelope::seal_reply(&reply, &loc, &answer, want) {
            Ok(sealed) => self.send_msg(peer, Msg::Piece { op: op_of(answer_kind), rid, to: None, env: sealed }),
            Err(_) => return !self.peer_scores.penalize(peer, 2.0),
        }
        true
    }

    fn answer(&mut self, endpoint: &Id, request: &Request, today: u32) -> (Reply, usize) {
        let store = self.scatter.pieces.as_mut().expect("holding");
        match request {
            Request::Put { loc, expiry_day, drop_hash, drop_key_id, stamp, piece, .. } => {
                let outcome = if !sc::stamp_ok(&endpoint.0, loc, piece.len(), *expiry_day, *stamp) {
                    Stored::BadStamp
                } else if *expiry_day < today || *expiry_day > today + MAX_TTL_DAYS {
                    Stored::Refused
                } else {
                    match store.put(loc, PieceMeta { expiry_day: *expiry_day, drop_hash: *drop_hash, drop_key_id: *drop_key_id }, piece, today) {
                        Ok(PutOutcome::Stored) => Stored::Ok,
                        // the same bytes again are fine; different ones never replace them
                        Ok(PutOutcome::Exists) => match store.get(loc, today) {
                            Ok(Some(kept)) if kept == *piece => Stored::Ok,
                            _ => Stored::Taken,
                        },
                        Ok(PutOutcome::Dropped) => Stored::Refused,
                        Ok(PutOutcome::Full) | Err(_) => Stored::Full,
                    }
                };
                (Reply::Stored(outcome), 0)
            }
            Request::Get { loc, want_len, .. } => {
                let want = *want_len as usize;
                let piece = store.get(loc, today).ok().flatten().filter(|p| p.len() <= want);
                (Reply::Got(piece), want)
            }
            Request::Has { loc, .. } => (Reply::Had(store.has(loc, today).ok().flatten()), 0),
            Request::Drop { loc, auth, .. } => {
                let outcome = match store.drop_meta(loc, today) {
                    Ok(None) => Dropped::Unknown,
                    Err(_) => Dropped::Refused,
                    Ok(Some(meta)) => {
                        let allowed = match auth {
                            DropAuth::Token(d) => sc::drop_token_matches(d, &meta.drop_hash),
                            DropAuth::Signed { key, sig } => {
                                sc::drop_key_id(key) == meta.drop_key_id && hoi_crypto::verify(&Id(*key), &sc::drop_statement(loc), &Sig(*sig))
                            }
                        };
                        // A marker is committed before acknowledgement; failed storage leaves
                        // the request unresolved and authenticated duplicates keep its expiry.
                        if allowed && store.mark_dropped(loc, today).unwrap_or(false) { Dropped::Dropped } else { Dropped::Refused }
                    }
                };
                (Reply::Dropped(outcome), 0)
            }
        }
    }

    // ------------------------------------------------------------------ storing and loading

    /// Whether this device saw a holder long and steadily enough for it to hold pieces.
    pub(crate) fn qualifies(&self, endpoint: &Id) -> bool {
        let bar = self.scatter.bar;
        if bar.known == 0 && bar.samples <= 0.0 {
            return true;
        }
        let now = self.now();
        self.scatter
            .saved
            .holders
            .get(endpoint)
            .is_some_and(|s| now >= s.first.saturating_add(bar.known) && s.checked >= bar.samples && s.present >= bar.up * s.checked)
    }

    /// Notes which holders are linked now: a first sighting at once, and once an hour a sample
    /// for every holder this device knows, linked or not. Samples are only taken while this
    /// device runs, so a holder is judged against this device's own time online.
    pub(crate) fn sample_holders(&mut self) {
        let now = self.now();
        let holders = self.linked_holders(None);
        let linked: BTreeSet<Id> = holders.iter().map(|h| h.endpoint).collect();
        for h in &holders {
            let routes: Vec<String> = h.routes.iter().filter(|r| r.len() <= 128).take(4).cloned().collect();
            match self.scatter.saved.holders.get_mut(&h.endpoint) {
                Some(seen) if seen.routes == routes => {}
                Some(seen) => {
                    seen.routes = routes;
                    self.scatter.dirty = true;
                }
                None => {
                    self.scatter.saved.holders.insert(h.endpoint, Seen { first: now, last: now, routes, ..Default::default() });
                    self.scatter.dirty = true;
                }
            }
        }
        let hour = now / 3600;
        if hour <= self.scatter.sampled_hour {
            return;
        }
        self.scatter.sampled_hour = hour;
        for (e, s) in self.scatter.saved.holders.iter_mut() {
            let here = linked.contains(e);
            s.checked = s.checked * (1.0 - SAMPLE_WEIGHT) + 1.0;
            s.present = s.present * (1.0 - SAMPLE_WEIGHT) + if here { 1.0 } else { 0.0 };
            if here {
                s.last = now;
            }
        }
        self.scatter.saved.holders.retain(|_, s| s.last.saturating_add(FORGET_AFTER) > now);
        if self.scatter.saved.holders.len() > MAX_SEEN {
            let mut by_age: Vec<(u64, Id)> = self.scatter.saved.holders.iter().map(|(e, s)| (s.last, *e)).collect();
            by_age.sort();
            for (_, e) in by_age.into_iter().take(self.scatter.saved.holders.len() - MAX_SEEN) {
                self.scatter.saved.holders.remove(&e);
            }
        }
        // Introducers are remembered while the holder is known here or someone still vouches.
        let vouched: BTreeSet<Id> = self.peers.values().flat_map(|p| p.scatter.vouches.iter().map(|v| v.endpoint)).collect();
        let known = &self.scatter.saved.holders;
        self.scatter.introduced.retain(|e, _| known.contains_key(e) || vouched.contains(e));
        self.scatter.dirty = true;
        // What this node vouches for may have changed: lists that differ go out again.
        self.broadcast_nodes();
    }

    /// The holders this device is linked to and could use: strangers' durable nodes whose
    /// holder key checked out, one per authenticated endpoint. Our own devices and home node
    /// are not independent of us and never hold our pieces.
    pub(crate) fn linked_holders(&self, community: Option<Id>) -> Vec<LinkedHolder> {
        let now = self.now();
        let roster = community.map(|c| self.roster_holders(c));
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for (name, p) in &self.peers {
            if !p.said_hello || p.role() != Role::Peer || !p.features.contains(hoi_sync::FEATURE_KEEPS) {
                continue;
            }
            let (Some(k), Some(endpoint)) = (p.scatter.holder, p.meta.endpoint.as_deref().and_then(|e| Id::from_hex(e).ok())) else { continue };
            if k.expires_at <= now || !seen.insert(endpoint) {
                continue;
            }
            let mut candidate = Candidate {
                operator: p.operator,
                network: crate::warmth::network_key(&p.meta),
                block: crate::warmth::block_key(&p.meta),
                // reached and seen first hand: its own introducer
                introducer: u64::from_le_bytes(endpoint.0[..8].try_into().expect("eight bytes")),
            };
            let qualified = self.qualifies(&endpoint)
                || match self.vouched_by(&endpoint, &candidate, roster.as_ref()) {
                    Some(introducer) => {
                        candidate.introducer = introducer;
                        true
                    }
                    None => false,
                };
            out.push(LinkedHolder { peer: name.clone(), endpoint, xk: k.xk, candidate, routes: p.routes.clone(), qualified });
        }
        out
    }

    /// The links this device sends pieces through: its own links that forward (never a browser
    /// or phone it serves).
    pub(crate) fn forwarders(&self) -> Vec<Forwarder> {
        self.peers
            .iter()
            .filter(|(_, p)| p.said_hello && p.scatter.forwards && p.role() != Role::Client)
            .map(|(name, p)| Forwarder {
                peer: name.clone(),
                candidate: Candidate {
                    operator: p.operator,
                    network: crate::warmth::network_key(&p.meta),
                    block: crate::warmth::block_key(&p.meta),
                    introducer: introducer_key(p),
                },
            })
            .collect()
    }

    /// The holders this device reaches through `forwarders`: those they vouch for, with a key
    /// whose signature checked out. A holder is placed by its link when this device is linked to
    /// it, by the routes this device saw for it itself, else by the first route every forwarder
    /// vouching for it gives alike: one whose vouchers disagree, or that nobody gave an address
    /// for, is left out, so a claim can only take a candidate away, never add a place.
    pub(crate) fn reachable_holders(&self, forwarders: &[Forwarder], community: Option<Id>) -> Vec<Reachable> {
        let roster = community.map(|c| self.roster_holders(c));
        let mut via: BTreeMap<Id, Vec<String>> = BTreeMap::new();
        let mut said: BTreeMap<Id, BTreeSet<Spot>> = BTreeMap::new();
        let mut routes: BTreeMap<Id, Vec<String>> = BTreeMap::new();
        for f in forwarders {
            let Some(p) = self.peers.get(&f.peer) else { continue };
            for v in &p.scatter.vouches {
                via.entry(v.endpoint).or_default().push(f.peer.clone());
                if !v.routes.is_empty() {
                    routes.entry(v.endpoint).or_insert_with(|| v.routes.clone());
                }
                said.entry(v.endpoint).or_default().insert(v.routes.first().map(|r| route_place(r)).unwrap_or((None, None)));
            }
        }
        let mut out = Vec::new();
        for (endpoint, via) in via {
            let Some(xk) = self.key_of(&endpoint) else { continue };
            let hex = endpoint.hex();
            let link = self.peers.values().find(|p| p.meta.endpoint.as_deref() == Some(hex.as_str()));
            let own = self.scatter.saved.holders.get(&endpoint).and_then(|s| s.routes.first()).map(|r| route_place(r));
            let (network, block) = match (link, own) {
                (Some(p), _) => (crate::warmth::network_key(&p.meta), crate::warmth::block_key(&p.meta)),
                (None, Some(place)) => place,
                (None, None) => match said.get(&endpoint) {
                    Some(places) if places.len() == 1 => *places.iter().next().expect("one"),
                    _ => continue,
                },
            };
            if network.is_none() {
                continue;
            }
            let mut candidate = Candidate {
                operator: link.and_then(|p| p.operator),
                network,
                block,
                introducer: u64::from_le_bytes(endpoint.0[..8].try_into().expect("eight bytes")),
            };
            let qualified = self.qualifies(&endpoint)
                || match self.vouched_by(&endpoint, &candidate, roster.as_ref()) {
                    Some(introducer) => {
                        candidate.introducer = introducer;
                        true
                    }
                    None => false,
                };
            let routes = routes.remove(&endpoint).unwrap_or_default();
            out.push(Reachable { endpoint, xk, candidate, via, qualified, routes });
        }
        out
    }

    /// What an object can use now: this device's forwarders, each with its place, and the
    /// holders that may take pieces outside every forwarder's place, each with the forwarders
    /// that reach it.
    pub(crate) fn plan(&self) -> Plan {
        self.plan_in(self.scatter.placing)
    }

    /// [`Node::plan`] for an object of community `c`: its rosters count too.
    pub(crate) fn plan_in(&self, c: Option<Id>) -> Plan {
        let forwarders = self.forwarders();
        let holders: Vec<Reachable> = self.reachable_holders(&forwarders, c).into_iter().filter(|h| h.qualified && !self.unreached(&h.endpoint)).collect();
        let mut all: Vec<Candidate> = forwarders.iter().map(|f| f.candidate).collect();
        all.extend(holders.iter().map(|h| h.candidate));
        let place = place::places(&all);
        let fplace: BTreeMap<String, usize> = forwarders.iter().enumerate().map(|(i, f)| (f.peer.clone(), place[i])).collect();
        let taken: BTreeSet<usize> = fplace.values().copied().collect();
        let holders = holders.into_iter().enumerate().filter(|(i, _)| !taken.contains(&place[forwarders.len() + i])).map(|(_, h)| h).collect();
        Plan { fplace, holders }
    }

    /// How many independent places the holders that may take pieces make: reached through this
    /// device's forwarders, outside their places, and seen long and steadily enough. What a
    /// generation is sized from.
    pub fn scatter_places(&self) -> usize {
        self.plan().places()
    }

    /// Independent places among this device's forwarders: at least two carry any object.
    pub fn scatter_paths(&self) -> usize {
        self.plan().forwarder_places()
    }

    /// The secret this device's drop tokens come from: its own, derived from its device key (or
    /// its endpoint key before it has one), so only it can drop what it stored.
    pub(crate) fn drop_secret(&self) -> Option<[u8; 32]> {
        let base = self.device_kp.as_ref().or(self.transport_kp.as_ref())?;
        let mut secret = base.secret();
        let out = blake3::derive_key("hoi scatter drop secret v1", &secret);
        secret.zeroize();
        Some(out)
    }

    /// Starts keeping `content` in pieces on holders this device reaches through its forwarders,
    /// sized for `level` (percent of places), until `expires_at`. Returns the operation's id for
    /// [`Node::scatter_take`].
    pub fn scatter_store(&mut self, content: &[u8], level: u8, expires_at: u64) -> R<u64> {
        let result = if self.scatter.placing.is_some_and(|c| self.scattering_home(c).is_some()) {
            self.store_at_home(content, level, expires_at)
        } else {
            self.scatter_store_inner(content, level, expires_at)
        };
        self.scatter.next_drop_key = None;
        self.scatter.placing = None;
        result
    }

    pub(crate) fn scatter_store_inner(&mut self, content: &[u8], level: u8, expires_at: u64) -> R<u64> {
        if !size::LEVELS.contains(&level) {
            return Err(ApiError::invalid("The scatter level is 10, 15 or 20 percent of places."));
        }
        self.check_room(expires_at)?;
        let object = sc::new_object_id()?;
        let key = sc::new_secret()?;
        let (class, sealed) = sc::seal_object(content, &object, &key).map_err(|_| ApiError::invalid("This is too large to keep in pieces (4 MiB at most)."))?;
        let base = Manifest {
            object,
            sealed: sc::sealed_hash(&sealed),
            class,
            generation: 0,
            level,
            places: 0,
            k: 0,
            n: 0,
            m: 0,
            hashes: Vec::new(),
            holders: Vec::new(),
            loc_secret: [0; 32],
            key,
            expires_at,
            keys: Vec::new(),
        };
        self.start_generation(&base, &sealed, 0, level, 0)
    }

    /// Whether another operation fits, and pieces may be kept until `expires_at`.
    pub(crate) fn check_room(&self, expires_at: u64) -> R<()> {
        let now = self.now();
        if self.scatter.ops.len() >= MAX_OPS {
            return Err(busy());
        }
        if expires_at <= now || expires_at.div_ceil(DAY) > now / DAY + MAX_TTL_DAYS as u64 {
            return Err(ApiError::invalid("Pieces are kept from a day to 400 days."));
        }
        Ok(())
    }

    /// Places a new generation `generation` of the sealed object `base` describes, at `level`,
    /// with at least `min_n` pieces (a hot object's larger generation): fresh transform key,
    /// fresh locator secret, fresh places.
    pub(crate) fn start_generation(&mut self, base: &Manifest, sealed: &[u8], generation: u8, level: u8, min_n: usize) -> R<u64> {
        self.check_room(base.expires_at)?;
        let plan = self.plan();
        let places = plan.places();
        let s = size::size_at_least(level, generation, places as u32, min_n).ok_or_else(|| few_places(places))?;
        plan.check_paths(s.k, s.n)?;
        let candidates: Vec<Candidate> = plan.holders.iter().map(|h| h.candidate).collect();
        let chosen = place::place(&candidates, s.n, s.n, &[], &mut uniform).ok_or_else(|| few_places(places))?;
        let shards = Shards::new(sealed, base.class, s.k)?;
        let object = base.object;
        let manifest = Manifest {
            generation,
            level,
            places: places as u32,
            k: s.k as u16,
            n: s.n as u16,
            m: s.m as u16,
            hashes: (0..s.n as u8).map(|i| sc::piece_hash(&object, generation, i, &shards.piece(i))).collect(),
            holders: chosen.iter().map(|&i| plan.holders[i].endpoint).collect(),
            loc_secret: sc::new_secret()?,
            keys: Vec::new(),
            ..base.clone()
        };
        Ok(self.begin_store(manifest, shards, 0, plan, s.k))
    }

    /// Places `fresh` new indexes of the generation `manifest` describes (a repair), from its
    /// rebuilt `shards`: each in a place that never held an index of the generation, within its
    /// budget. The operation ends with the manifest listing them.
    pub(crate) fn scatter_extend(&mut self, mut manifest: Manifest, shards: Shards, fresh: usize) -> R<u64> {
        self.check_room(manifest.expires_at)?;
        let first = manifest.hashes.len();
        let fresh = fresh.min((manifest.m as usize).saturating_sub(first));
        if fresh == 0 {
            return Err(ApiError::new("scatter-budget", "This generation has used every index it may."));
        }
        let plan = self.plan();
        plan.check_paths(manifest.k as usize, 0)?;
        let candidates: Vec<Candidate> = plan.holders.iter().map(|h| h.candidate).collect();
        let used: Vec<usize> = plan.holders.iter().enumerate().filter(|(_, h)| manifest.holders.contains(&h.endpoint)).map(|(i, _)| i).collect();
        let chosen = place::place(&candidates, manifest.n as usize, fresh, &used, &mut uniform).ok_or_else(|| few_places(plan.places()))?;
        for (j, &c) in chosen.iter().enumerate() {
            let index = (first + j) as u8;
            manifest.hashes.push(sc::piece_hash(&manifest.object, manifest.generation, index, &shards.piece(index)));
            manifest.holders.push(plan.holders[c].endpoint);
        }
        let k = manifest.k as usize;
        Ok(self.begin_store(manifest, shards, first, plan, k))
    }

    fn begin_store(&mut self, manifest: Manifest, shards: Shards, first: usize, plan: Plan, k: usize) -> u64 {
        let expiry_day = manifest.expires_at.div_ceil(DAY) as u32;
        let slots: Vec<Slot> =
            manifest.holders.iter().enumerate().map(|(i, &holder)| Slot { holder, stored: i < first, lost: false, tries: 0, via: BTreeSet::new() }).collect();
        let used = manifest.holders.iter().copied().collect();
        let paths = plan.paths(k);
        let id = self.scatter.next_op;
        self.scatter.next_op += 1;
        let count = slots.len();
        let drop_key_id = self.scatter.next_drop_key.take().unwrap_or([0; 16]);
        self.scatter.ops.insert(id, Op::Store(Box::new(StoreOp { manifest, shards, expiry_day, drop_key_id, slots, used, paths, candidates: plan.holders })));
        for index in first..count {
            self.send_put(id, index as u8, &BTreeSet::new());
        }
        self.check_store(id);
        id
    }

    /// Forwarder links linked now.
    fn live_forwarders(&self) -> BTreeSet<String> {
        self.peers.iter().filter(|(_, p)| p.said_hello && p.scatter.forwards).map(|(k, _)| k.clone()).collect()
    }

    /// Sends index `index` of a store to its holder, through a forwarder not in `avoid`. Without
    /// one, the index goes elsewhere.
    fn send_put(&mut self, op: u64, index: u8, avoid: &BTreeSet<String>) {
        let now = self.now();
        let drop_secret = self.drop_secret();
        let live = self.live_forwarders();
        let hops = self.hops(&live);
        let Some(Op::Store(st)) = self.scatter.ops.get_mut(&op) else { return };
        let i = index as usize;
        let holder = st.slots[i].holder;
        let Some(forwarder) = st.paths.choose(&holder, index, avoid, &live) else {
            if !st.paths.has_room(index, &live) {
                // every forwarder place carried its share of the generation: no path is left
                self.finish(op, ScatterOutcome::Failed(FEW_PATHS.into()));
                return;
            }
            // No forwarder reaches this holder any more, or every one that does carried its share.
            let certain = st.slots[i].via.is_empty();
            self.replace_slot(op, index, certain);
            return;
        };
        let Some(&(xk, _)) = st.paths.holders.get(&holder) else { return };
        st.paths.carry(&forwarder, index);
        st.slots[i].via.insert(forwarder.clone());
        st.slots[i].tries += 1;
        let m = &st.manifest;
        let piece = st.shards.piece(index);
        let loc = sc::locator(&m.loc_secret, &m.object, m.generation, index);
        let stamp = sc::mine_stamp(&holder.0, &loc, piece.len(), st.expiry_day);
        let drop_hash = drop_secret.map(|d| sc::drop_hash(&sc::drop_token(&d, &loc))).unwrap_or([0; 32]);
        let Ok(reply) = XKey::generate() else { return };
        let request = Request::Put { loc, reply: reply.public(), expiry_day: st.expiry_day, drop_hash, drop_key_id: st.drop_key_id, stamp, piece };
        let Ok(env) = envelope::seal_request(&holder.0, &xk, &request) else { return };
        // Three hops where it can (S7): the forwarder chosen is the exit, which reaches the
        // holder; the put goes wrapped through another link, in another place, that reaches the
        // exit, so the link this device sends through never learns the holder.
        let entry = hops.iter().find(|h| h.peer == forwarder).and_then(|x| Some((x, x.xk?))).and_then(|(x, xk)| {
            // an entry learns no holder, so only the exit counts against its place's k−1
            let options: Vec<&Hop> =
                hops.iter().filter(|e| e.peer != forwarder && e.reaches.contains(&x.endpoint) && st.paths.apart(&e.peer, &forwarder)).collect();
            (!options.is_empty()).then(|| (options[uniform(options.len())].peer.clone(), x.endpoint, xk))
        });
        let wrapped = entry.and_then(|(e, x, xk)| envelope::seal_wrap(&x.0, &xk, &holder.0, Kind::Put, &env).ok().map(|w| (e, x, w)));
        let object = Id(m.object);
        let rid = self.next_rid();
        let (via, msg) = match wrapped {
            Some((e, exit, wrapped)) => {
                self.scatter.wrapped += 1;
                (e, Msg::Piece { op: PieceOp::Wrap, rid, to: Some(exit), env: wrapped })
            }
            None => {
                // the forwarder saw this holder next to this device's address (crate::inject)
                self.scatter.saved.two_hop.insert(object, now);
                self.scatter.dirty = true;
                (forwarder.clone(), Msg::Piece { op: PieceOp::Put, rid, to: Some(holder), env })
            }
        };
        self.scatter
            .pending
            .insert((via.clone(), rid), Pending { op, index, via: Some(forwarder.clone()), kind: Kind::Stored, reply, loc, want: 0, sent_at: now, mail: None });
        self.send_msg(&via, msg);
    }

    /// This device's forwarder links as three-hop uploads use them: each one's endpoint, its
    /// holder key (an exit needs one: the wrap is sealed to it), and the holders it vouches for
    /// (an entry must reach its exit).
    fn hops(&self, live: &BTreeSet<String>) -> Vec<Hop> {
        live.iter()
            .filter_map(|peer| {
                let p = self.peers.get(peer)?;
                let endpoint = p.meta.endpoint.as_deref().and_then(|e| Id::from_hex(e).ok())?;
                Some(Hop {
                    peer: peer.clone(),
                    endpoint,
                    xk: p.scatter.holder.as_ref().map(|k| k.xk),
                    reaches: p.scatter.vouches.iter().map(|v| v.endpoint).collect(),
                })
            })
            .collect()
    }

    /// A request to `holder` through `forwarder` went nowhere (no answer in time, `gone`).
    fn note_unreached(&mut self, holder: Id, forwarder: Option<&str>) {
        let now = self.now();
        let Some(place) = forwarder
            .and_then(|f| self.peers.get(f))
            .map(|p| place_key(&Candidate { operator: p.operator, network: crate::warmth::network_key(&p.meta), block: None, introducer: introducer_key(p) }))
        else {
            return;
        };
        let u = &mut self.scatter.unreached;
        if u.len() >= MAX_KEYS && !u.contains_key(&holder) {
            u.retain(|_, (until, _)| *until > now);
            if u.len() >= MAX_KEYS {
                return;
            }
        }
        let e = u.entry(holder).or_insert((0, BTreeSet::new()));
        if e.0 <= now {
            e.1.clear();
        }
        e.0 = now + UNREACHED_FOR;
        e.1.insert(place);
    }

    /// Whether requests to `holder` went nowhere through two forwarder places lately.
    pub(crate) fn unreached(&self, holder: &Id) -> bool {
        self.scatter.unreached.get(holder).is_some_and(|(until, places)| *until > self.now() && places.len() >= 2)
    }

    /// The holder a pending request went to.
    fn pending_holder(&self, p: &Pending) -> Option<Id> {
        match self.scatter.ops.get(&p.op)? {
            Op::Store(st) => st.slots.get(p.index as usize).map(|s| s.holder),
            Op::Load(ld) => ld.manifest.holders.get(p.index as usize).copied(),
            Op::Probe(pr) => pr.holders.get(&p.index).copied(),
            Op::Wait | Op::Done(_) => None,
        }
    }

    pub(crate) fn next_rid(&mut self) -> u32 {
        self.scatter.next_rid = self.scatter.next_rid.wrapping_add(1);
        self.scatter.next_rid
    }

    /// A put that got no answer that says what happened (none in time, a forwarder's `gone`, an
    /// answer that would not open): the holder may or may not keep it. The same index goes to
    /// the same holder through another forwarder, which places nothing new; with none left, a
    /// fresh index goes elsewhere.
    fn retry_slot(&mut self, op: u64, index: u8) {
        let Some(Op::Store(st)) = self.scatter.ops.get(&op) else { return };
        let slot = &st.slots[index as usize];
        let live = self.live_forwarders();
        if slot.tries < MAX_TRIES && st.paths.choose(&slot.holder, index, &slot.via, &live).is_some() {
            let avoid = slot.via.clone();
            self.send_put(op, index, &avoid);
        } else {
            self.replace_slot(op, index, false);
        }
    }

    /// An index whose holder did not keep it. `certain`: the holder said so (or was never
    /// asked), so the same index may go to another place; otherwise it may have been kept, and
    /// a fresh index of the generation replaces it.
    fn replace_slot(&mut self, op: u64, index: u8, certain: bool) {
        let Some(Op::Store(st)) = self.scatter.ops.get_mut(&op) else { return };
        let candidates: Vec<Candidate> = st.candidates.iter().map(|h| h.candidate).collect();
        // Places that held or were asked for any index of this generation are left out, and
        // count toward the generation's caps.
        let used: Vec<usize> = st.candidates.iter().enumerate().filter(|(_, h)| st.used.contains(&h.endpoint)).map(|(i, _)| i).collect();
        let choice = place::place(&candidates, st.manifest.n as usize, 1, &used, &mut uniform).map(|c| st.candidates[c[0]].endpoint);
        let i = index as usize;
        let target = if certain && st.slots[i].tries < MAX_TRIES {
            Some(index)
        } else if !certain && st.slots.len() < st.manifest.m as usize {
            st.slots[i].lost = true;
            let fresh = st.slots.len() as u8;
            st.slots.push(Slot { holder: Id::ZERO, stored: false, lost: false, tries: 0, via: BTreeSet::new() });
            st.manifest.hashes.push(sc::piece_hash(&st.manifest.object, st.manifest.generation, fresh, &st.shards.piece(fresh)));
            st.manifest.holders.push(Id::ZERO);
            Some(fresh)
        } else {
            None
        };
        match (target, choice) {
            (Some(t), Some(holder)) => {
                st.used.insert(holder);
                let slot = &mut st.slots[t as usize];
                slot.holder = holder;
                slot.via.clear();
                st.manifest.holders[t as usize] = holder;
                self.send_put(op, t, &BTreeSet::new());
            }
            _ => {
                let places = st.candidates.len();
                self.finish(op, ScatterOutcome::Failed(few_places(places).message));
            }
        }
    }

    fn finish(&mut self, op: u64, outcome: ScatterOutcome) {
        if let Some(manifest) = self.scatter_placement(op) {
            self.scatter.placements.insert(op, manifest);
        }
        self.discard_pending_scatter(op);
        self.scatter.ops.insert(op, Op::Done(Box::new(outcome)));
        self.job_done(op);
    }

    /// Starts fetching and rebuilding the object a manifest names, through this device's
    /// forwarders, with two spare pieces.
    pub fn scatter_load(&mut self, manifest: &Manifest) -> R<u64> {
        self.scatter_load_with(manifest, SPARES)
    }

    /// [`Node::scatter_load`] with `spares` pieces beyond k ("Use less data" asks for one).
    pub fn scatter_load_with(&mut self, manifest: &Manifest, spares: usize) -> R<u64> {
        manifest.check()?;
        if self.scatter.ops.len() >= MAX_OPS {
            return Err(busy());
        }
        let forwarders = self.forwarders();
        let reach = self.reachable_holders(&forwarders, None);
        let mut all: Vec<Candidate> = forwarders.iter().map(|f| f.candidate).collect();
        all.extend(reach.iter().map(|h| h.candidate));
        let place = place::places(&all);
        let k = manifest.k as usize;
        let mut paths = Paths { cap: k - 1, ..Default::default() };
        for (i, f) in forwarders.iter().enumerate() {
            paths.place.insert(f.peer.clone(), place[i]);
        }
        for (j, h) in reach.iter().enumerate() {
            if !manifest.holders.contains(&h.endpoint) {
                continue;
            }
            // never through a forwarder in the holder's own place
            let own = place[forwarders.len() + j];
            let via: Vec<String> = h.via.iter().filter(|f| paths.place.get(*f) != Some(&own)).cloned().collect();
            if !via.is_empty() {
                paths.holders.insert(h.endpoint, (h.xk, via));
            }
        }
        paths.relay_unvouched(manifest, &forwarders);
        let id = self.scatter.next_op;
        self.scatter.next_op += 1;
        self.scatter.ops.insert(
            id,
            Op::Load(Box::new(LoadOp { manifest: manifest.clone(), got: BTreeMap::new(), asked: BTreeSet::new(), tries: 0, paths, trouble: 0, bad: 0 })),
        );
        for _ in 0..k + spares.min(4) {
            if !self.ask_another(id) {
                break;
            }
        }
        self.check_load(id);
        Ok(id)
    }

    /// Asks for one more index, at random among those not asked yet whose holder a forwarder
    /// with room reaches. False when none is left.
    fn ask_another(&mut self, op: u64) -> bool {
        let now = self.now();
        let live = self.live_forwarders();
        let Some(Op::Load(ld)) = self.scatter.ops.get_mut(&op) else { return false };
        let m = &ld.manifest;
        let mut open: Vec<u8> =
            m.holders.iter().enumerate().filter(|(i, h)| !ld.asked.contains(&(*i as u8)) && ld.paths.holders.contains_key(h)).map(|(i, _)| i as u8).collect();
        while !open.is_empty() {
            let index = open.swap_remove(uniform(open.len()));
            let holder = m.holders[index as usize];
            let Some(forwarder) = ld.paths.choose(&holder, index, &BTreeSet::new(), &live) else { continue };
            let xk = ld.paths.holders[&holder].0;
            ld.paths.carry(&forwarder, index);
            ld.asked.insert(index);
            ld.tries += 1;
            let loc = sc::locator(&m.loc_secret, &m.object, m.generation, index);
            let want = m.class.piece_len(m.k as usize);
            let Ok(reply) = XKey::generate() else { return false };
            let request = Request::Get { loc, reply: reply.public(), want_len: want as u32 };
            let Ok(env) = envelope::seal_request(&holder.0, &xk, &request) else { return false };
            let rid = self.next_rid();
            self.scatter.pending.insert(
                (forwarder.clone(), rid),
                Pending { op, index, via: Some(forwarder.clone()), kind: Kind::Got, reply, loc, want, sent_at: now, mail: None },
            );
            self.send_msg(&forwarder, Msg::Piece { op: PieceOp::Get, rid, to: Some(holder), env });
            return true;
        }
        false
    }

    fn on_answer(&mut self, peer: &str, op: PieceOp, rid: u32, env: Vec<u8>) {
        let Some(p) = self.scatter.pending.remove(&(peer.to_string(), rid)) else { return };
        self.discard_scatter_messages(&BTreeSet::from([(peer.to_string(), rid)]));
        let answer =
            if op == PieceOp::Gone { None } else { envelope::open_reply(kind_of(op), &p.reply, &p.loc, p.want, &env).ok().filter(|_| kind_of(op) == p.kind) };
        if let Some(request) = p.mail {
            self.on_mail_answer(request, answer);
            return;
        }
        if answer.is_none()
            && let Some(h) = self.pending_holder(&p)
        {
            self.note_unreached(h, p.via.as_deref());
        }
        match (self.scatter.ops.get_mut(&p.op), answer) {
            (Some(Op::Store(st)), Some(Reply::Stored(Stored::Ok))) => {
                st.slots[p.index as usize].stored = true;
                self.check_store(p.op);
            }
            (Some(Op::Store(_)), Some(Reply::Stored(_))) => self.replace_slot(p.op, p.index, true),
            (Some(Op::Store(_)), _) => self.retry_slot(p.op, p.index),
            (Some(Op::Load(ld)), Some(Reply::Got(Some(piece)))) => {
                let m = &ld.manifest;
                if m.hashes.get(p.index as usize) != Some(&sc::piece_hash(&m.object, m.generation, p.index, &piece)) {
                    ld.bad += 1;
                    self.ask_another(p.op);
                } else if ld.got.len() < m.k as usize {
                    ld.got.insert(p.index, piece);
                }
                self.check_load(p.op);
            }
            (Some(Op::Load(ld)), _) => {
                ld.trouble += 1;
                self.ask_another(p.op);
                self.check_load(p.op);
            }
            (Some(Op::Probe(pr)), answer) => {
                if let Some(drops) = pr.drops.as_mut() {
                    let result = match answer {
                        Some(Reply::Dropped(Dropped::Dropped)) if pr.durable.contains(&p.index) => DropOutcome::Acknowledged,
                        Some(Reply::Dropped(Dropped::Dropped)) => DropOutcome::LegacyRemoval,
                        Some(Reply::Dropped(Dropped::Unknown)) => DropOutcome::Unknown,
                        Some(Reply::Dropped(Dropped::Refused)) => DropOutcome::Refused,
                        _ => DropOutcome::Unreachable,
                    };
                    drops.insert(p.index, result);
                }
                let result = match answer {
                    Some(Reply::Had(Some(_))) => Some(true),
                    Some(Reply::Had(None)) => Some(false),
                    _ => None,
                };
                pr.answers.insert(p.index, (result, p.via.as_ref().and_then(|v| pr.fkey.get(v)).copied().unwrap_or(0)));
                self.check_probe(p.op);
            }
            _ => {}
        }
    }

    fn check_store(&mut self, op: u64) {
        let Some(Op::Store(st)) = self.scatter.ops.get(&op) else { return };
        if st.slots.iter().all(|s| s.stored || s.lost) {
            // each holder's key, as it was sealed to: readers whose forwarders do not vouch for
            // a holder seal to it (S6)
            // (a repair keeps the keys listed before and adds the new holders')
            let mut manifest = st.manifest.clone();
            let mut keys = manifest.keys.clone();
            for h in manifest.holders.iter().skip(keys.len()) {
                match st.paths.holders.get(h) {
                    Some(x) => keys.push(x.0),
                    None => break,
                }
            }
            manifest.keys = if keys.len() == manifest.holders.len() { keys } else { Vec::new() };
            self.finish(op, ScatterOutcome::Stored(manifest));
        }
    }

    fn check_load(&mut self, op: u64) {
        let Some(Op::Load(ld)) = self.scatter.ops.get(&op) else { return };
        let m = &ld.manifest;
        let k = m.k as usize;
        let wants_shards = matches!(self.scatter.jobs.get(&op), Some(crate::keep::Job::Rebuild { .. }));
        let mut rebuilt = None;
        let outcome = if ld.got.len() >= k {
            if wants_shards {
                rebuilt = self.rebuild_shards(m, &ld.got);
            }
            let pieces: Vec<(u8, &[u8])> = ld.got.iter().take(k).map(|(i, p)| (*i, p.as_slice())).collect();
            let opened = Shards::rebuild(m.class, k, &pieces)
                .and_then(|s| s.sealed())
                .ok()
                .filter(|sealed| sc::sealed_hash(sealed) == m.sealed)
                .and_then(|sealed| sc::open_object(&sealed, &m.object, m.class, &m.key).ok());
            match opened {
                Some(content) => ScatterOutcome::Loaded(content),
                None => ScatterOutcome::Failed(MISMATCH.into()),
            }
        } else if !self.scatter.pending.values().any(|p| p.op == op && p.mail.is_none()) {
            // k pieces whose holders all answered, none matching the manifest: the manifest
            // is not of these pieces
            ScatterOutcome::Failed(if ld.bad >= k { MISMATCH } else { TOO_FEW_ONLINE }.into())
        } else {
            return;
        };
        let trouble = ld.trouble;
        if let Some(r) = rebuilt {
            self.scatter.repair.rebuilt.insert(op, r);
        }
        self.note_trouble(op, trouble);
        self.finish(op, outcome);
    }

    /// Rebuilds the generation a manifest names into shards from k pieces (for a repair): the
    /// sealed object must be the manifest's.
    pub(crate) fn rebuild_shards(&self, m: &Manifest, got: &BTreeMap<u8, Vec<u8>>) -> Option<(Shards, Vec<u8>)> {
        let k = m.k as usize;
        let pieces: Vec<(u8, &[u8])> = got.iter().take(k).map(|(i, p)| (*i, p.as_slice())).collect();
        let shards = Shards::rebuild(m.class, k, &pieces).ok()?;
        let sealed = shards.sealed().ok().filter(|s| sc::sealed_hash(s) == m.sealed)?;
        Some((shards, sealed))
    }

    /// Asks every live index of a generation whether its holder still keeps it (`has`), each
    /// through a forwarder in another place, at most k−1 through one place. Indexes no
    /// forwarder can reach now are left for another round.
    /// `failed`: per index, the places of forwarders a probe of it failed through before; another
    /// place is asked if one can be, so a loss is two places' word.
    pub(crate) fn scatter_probe(&mut self, manifest: &Manifest, indexes: &[u8], failed: &BTreeMap<u8, BTreeSet<u64>>) -> R<u64> {
        self.scatter_ask(manifest, indexes, failed, &Ask::Has)
    }

    /// Drops every index of a generation `with` the creator's secret or the community drop key,
    /// through forwarders as probes go. Holders refuse what the credential does not open.
    pub(crate) fn scatter_drop(&mut self, manifest: &Manifest, with: &Ask) -> R<u64> {
        let indexes: Vec<u8> = (0..manifest.hashes.len() as u8).collect();
        self.scatter_ask(manifest, &indexes, &BTreeMap::new(), with)
    }

    /// A durable deletion needs only holder/locator capabilities, never object decryption
    /// keys. Unknown routes remain unresolved until another retry (RFC-0001).
    pub(crate) fn scatter_drop_targets(&mut self, targets: &[(Id, Id, [u8; 32])]) -> R<u64> {
        if targets.len() > hoi_types::scatter::MAX_M || targets.len() > 256 {
            return Err(ApiError::invalid("Too many deletion targets in one attempt."));
        }
        let holders: Vec<_> = targets.iter().map(|t| t.0).collect();
        // Without retaining a generation's manifest, use the conservative cap of one
        // locator per forwarder place in this attempt, below every supported k.
        let (mut paths, fkey, forwarders) = self.ask_paths(&holders, 1);
        for holder in &holders {
            if !paths.holders.contains_key(holder)
                && let Some(key) = self.key_of(holder)
            {
                let via = forwarders.iter().map(|f| f.peer.clone()).collect();
                paths.holders.insert(*holder, (key, via));
            }
        }
        let targets = targets
            .iter()
            .enumerate()
            .map(|(index, &(holder, loc, token))| AskTarget { index: index as u8, holder, loc: loc.0, auth: Some(DropAuth::Token(token)) })
            .collect();
        self.start_asks(targets, paths, fkey, &BTreeMap::new(), true)
    }

    fn scatter_ask(&mut self, manifest: &Manifest, indexes: &[u8], failed: &BTreeMap<u8, BTreeSet<u64>>, ask: &Ask) -> R<u64> {
        let (mut paths, fkey, forwarders) = self.ask_paths(&manifest.holders, manifest.k.saturating_sub(1) as usize);
        paths.relay_unvouched(manifest, &forwarders);
        let targets = indexes
            .iter()
            .filter_map(|&index| {
                let holder = *manifest.holders.get(index as usize)?;
                let loc = sc::locator(&manifest.loc_secret, &manifest.object, manifest.generation, index);
                let auth = match ask {
                    Ask::Has => None,
                    Ask::DropToken(secret) => Some(DropAuth::Token(sc::drop_token(secret, &loc))),
                    Ask::DropTokens(tokens) => Some(DropAuth::Token(*tokens.get(&Id(loc))?)),
                    Ask::DropKey(secret) => {
                        let kp = hoi_crypto::Keypair::from_secret(secret);
                        Some(DropAuth::Signed { key: kp.public().0, sig: kp.sign(&sc::drop_statement(&loc)).0 })
                    }
                };
                Some(AskTarget { index, holder, loc, auth })
            })
            .collect();
        self.start_asks(targets, paths, fkey, failed, !matches!(ask, Ask::Has))
    }

    fn ask_paths(&self, holders: &[Id], cap: usize) -> (Paths, BTreeMap<String, u64>, Vec<Forwarder>) {
        let forwarders = self.forwarders();
        let fkey: BTreeMap<String, u64> = forwarders.iter().map(|f| (f.peer.clone(), place_key(&f.candidate))).collect();
        let reach = self.reachable_holders(&forwarders, None);
        let mut all: Vec<Candidate> = forwarders.iter().map(|f| f.candidate).collect();
        all.extend(reach.iter().map(|h| h.candidate));
        let place = place::places(&all);
        let mut paths = Paths { cap, ..Default::default() };
        for (i, f) in forwarders.iter().enumerate() {
            paths.place.insert(f.peer.clone(), place[i]);
        }
        for (j, h) in reach.iter().enumerate() {
            let own = place[forwarders.len() + j];
            let via: Vec<String> = h.via.iter().filter(|f| paths.place.get(*f) != Some(&own)).cloned().collect();
            if holders.contains(&h.endpoint) && !via.is_empty() {
                paths.holders.insert(h.endpoint, (h.xk, via));
            }
        }
        (paths, fkey, forwarders)
    }

    fn start_asks(
        &mut self,
        targets: Vec<AskTarget>,
        mut paths: Paths,
        fkey: BTreeMap<String, u64>,
        failed: &BTreeMap<u8, BTreeSet<u64>>,
        dropping: bool,
    ) -> R<u64> {
        if self.scatter.ops.len() >= MAX_OPS {
            return Err(busy());
        }
        let now = self.now();
        let live = self.live_forwarders();
        let hops = self.hops(&live);
        let id = self.scatter.next_op;
        self.scatter.next_op += 1;
        let mut sent = Vec::new();
        let requested: BTreeSet<u8> = targets.iter().map(|t| t.index).collect();
        let durable = targets
            .iter()
            .filter(|t| self.scatter.keys.get(&t.holder).is_some_and(|k| k.expires_at > now && k.drop_journal.is_some()))
            .map(|t| t.index)
            .collect();
        for AskTarget { index, holder, loc, auth } in targets {
            let before = failed.get(&index);
            let avoid: BTreeSet<String> = fkey.iter().filter(|(_, k)| before.is_some_and(|b| b.contains(k))).map(|(f, _)| f.clone()).collect();
            let Some(forwarder) = paths.choose(&holder, index, &avoid, &live).or_else(|| paths.choose(&holder, index, &BTreeSet::new(), &live)) else {
                continue;
            };
            paths.carry(&forwarder, index);
            let xk = paths.holders[&holder].0;
            let Ok(reply) = XKey::generate() else { continue };
            let request = match auth {
                None => Request::Has { loc, reply: reply.public() },
                Some(auth) => Request::Drop { loc, reply: reply.public(), auth },
            };
            let Ok(env) = envelope::seal_request(&holder.0, &xk, &request) else { continue };
            sent.push((index, forwarder, reply, loc, env, holder));
        }
        let sent_indexes: BTreeSet<u8> = sent.iter().map(|s| s.0).collect();
        let asked = if dropping { requested } else { sent_indexes.clone() };
        let holders = sent.iter().map(|s| (s.0, s.5)).collect();
        let (kind, op) = if dropping { (Kind::Dropped, PieceOp::Drop) } else { (Kind::Had, PieceOp::Has) };
        let answers = asked.difference(&sent_indexes).map(|&i| (i, (None, 0))).collect();
        let drops = dropping.then(|| asked.difference(&sent_indexes).map(|&i| (i, DropOutcome::Queued)).collect());
        self.scatter.ops.insert(id, Op::Probe(Box::new(ProbeOp { asked, holders, answers, fkey, drops, durable })));
        let mut shorter = false;
        for (index, forwarder, reply, loc, env, holder) in sent {
            let rid = self.next_rid();
            let wrapped = if !dropping {
                None
            } else {
                hops.iter().find(|h| h.peer == forwarder).and_then(|exit| {
                    let xk = exit.xk?;
                    let entries: Vec<_> = hops
                        .iter()
                        .filter(|entry| entry.peer != forwarder && entry.reaches.contains(&exit.endpoint) && paths.apart(&entry.peer, &forwarder))
                        .collect();
                    if entries.is_empty() {
                        return None;
                    }
                    let entry = entries[uniform(entries.len())];
                    envelope::seal_wrap(&exit.endpoint.0, &xk, &holder.0, Kind::Drop, &env).ok().map(|wrapped| (entry.peer.clone(), exit.endpoint, wrapped))
                })
            };
            let (via, message) = if let Some((entry, exit, env)) = wrapped {
                (entry, Msg::Piece { op: PieceOp::Wrap, rid, to: Some(exit), env })
            } else {
                shorter |= dropping;
                (forwarder.clone(), Msg::Piece { op, rid, to: Some(holder), env })
            };
            self.scatter
                .pending
                .insert((via.clone(), rid), Pending { op: id, index, via: Some(forwarder.clone()), kind, reply, loc, want: 0, sent_at: now, mail: None });
            self.send_msg(&via, message);
        }
        if shorter && now >= self.scatter.last_drop_notice.saturating_add(60) {
            self.scatter.last_drop_notice = now;
            self.events.push(crate::dto::Event::Notice { text: "Deletion requested through a shorter route: a three-hop path was unavailable, so the entry node may associate the request with you. Holders can retain copies.".into() });
        }
        self.check_probe(id);
        Ok(id)
    }

    fn check_probe(&mut self, op: u64) {
        let Some(Op::Probe(pr)) = self.scatter.ops.get(&op) else { return };
        if pr.asked.iter().all(|i| pr.answers.contains_key(i)) {
            let outcome = if let Some(drops) = &pr.drops {
                ScatterOutcome::Dropped(drops.iter().map(|(&index, &outcome)| DropResult { index, outcome }).collect())
            } else {
                ScatterOutcome::Probed(pr.answers.iter().map(|(&index, &(kept, place))| ProbeResult { index, kept, place }).collect())
            };
            self.finish(op, outcome);
        }
    }

    /// Indexes actually handed to the outgoing queue, so durable progress does not report
    /// an unsent target as submitted while other holders' replies are still pending.
    pub(crate) fn scatter_submitted_indexes(&self, op: u64) -> BTreeSet<u8> {
        match self.scatter.ops.get(&op) {
            Some(Op::Probe(probe)) => {
                probe.asked.iter().filter(|index| probe.drops.as_ref().is_some_and(|drops| drops.get(index) != Some(&DropOutcome::Queued))).copied().collect()
            }
            Some(Op::Done(outcome)) => match outcome.as_ref() {
                ScatterOutcome::Dropped(results) => results.iter().filter(|r| r.outcome != DropOutcome::Queued).map(|r| r.index).collect(),
                _ => BTreeSet::new(),
            },
            _ => BTreeSet::new(),
        }
    }

    /// How an operation stands; a finished one is handed over once and forgotten.
    pub fn scatter_take(&mut self, op: u64) -> ScatterOutcome {
        self.scatter_tick();
        self.take_outcome(op)
    }

    /// An operation whose outcome comes from elsewhere (crate::rebuild).
    pub(crate) fn scatter_wait_op(&mut self) -> u64 {
        let id = self.scatter.next_op;
        self.scatter.next_op += 1;
        self.scatter.ops.insert(id, Op::Wait);
        id
    }

    pub(crate) fn end_wait_op(&mut self, op: u64, outcome: ScatterOutcome) {
        if matches!(self.scatter.ops.get(&op), Some(Op::Wait)) {
            self.finish(op, outcome);
        }
    }

    /// Every locator a placement may have stored, including failed and unfinished work.
    pub(crate) fn scatter_placement(&self, op: u64) -> Option<Manifest> {
        match self.scatter.ops.get(&op) {
            Some(Op::Store(st)) => {
                let mut manifest = st.manifest.clone();
                manifest.keys = manifest.holders.iter().map(|h| st.paths.holders.get(h).map(|k| k.0)).collect::<Option<Vec<_>>>().unwrap_or_default();
                Some(manifest)
            }
            Some(Op::Done(outcome)) => match &**outcome {
                ScatterOutcome::Stored(manifest) => Some(manifest.clone()),
                _ => self.scatter.placements.get(&op).cloned(),
            },
            _ => self.scatter.placements.get(&op).cloned(),
        }
    }

    /// Snapshot before cancellation: requests already sent can still put bytes on a holder.
    /// The caller must persist their deletion targets before invoking this (RFC-0001).
    pub(crate) fn cancel_scatter_operation(&mut self, op: u64) -> Option<Manifest> {
        let manifest = self.scatter_placement(op);
        self.drop_op(op);
        manifest
    }

    fn discard_scatter_messages(&mut self, pending: &BTreeSet<(String, u32)>) {
        self.outbox.retain(|(peer, bytes)| !matches!(Msg::decode(bytes), Ok(Msg::Piece { rid, .. }) if pending.contains(&(peer.clone(), rid))));
    }

    fn discard_pending_scatter(&mut self, op: u64) {
        let pending = self.scatter.pending.iter().filter(|(_, p)| p.op == op && p.mail.is_none()).map(|(key, _)| key.clone()).collect();
        self.discard_scatter_messages(&pending);
        self.scatter.pending.retain(|_, p| p.op != op || p.mail.is_some());
    }

    /// Lets an operation go, with whatever of it is still waiting for answers.
    pub(crate) fn drop_op(&mut self, op: u64) {
        self.scatter.ops.remove(&op);
        self.scatter.placements.remove(&op);
        self.scatter.jobs.remove(&op);
        self.scatter.repair.rebuilt.remove(&op);
        self.discard_pending_scatter(op);
    }

    /// Whether an operation finished and waits to be taken.
    pub(crate) fn op_done(&self, op: u64) -> bool {
        matches!(self.scatter.ops.get(&op), Some(Op::Done(_)))
    }

    pub(crate) fn take_outcome(&mut self, op: u64) -> ScatterOutcome {
        if self.op_done(op) {
            self.scatter.placements.remove(&op);
        }
        match self.scatter.ops.get(&op) {
            Some(Op::Done(_)) => match self.scatter.ops.remove(&op) {
                Some(Op::Done(o)) => *o,
                _ => unreachable!(),
            },
            Some(_) => ScatterOutcome::Pending,
            None => ScatterOutcome::Failed("No such operation.".into()),
        }
    }

    /// Requests whose answer is overdue count as unanswered; expired pieces are let go. Hosts
    /// call this every few seconds while operations run (and `sweep` calls it too).
    pub fn scatter_tick(&mut self) {
        let now = self.now();
        self.announce_new_key();
        self.sample_holders();
        self.forward_tick();
        let overdue: Vec<(String, u32)> = self.scatter.pending.iter().filter(|(_, p)| now >= p.sent_at + ANSWER_TIMEOUT).map(|(k, _)| k.clone()).collect();
        for key in overdue {
            let Some(p) = self.scatter.pending.remove(&key) else { continue };
            self.discard_scatter_messages(&BTreeSet::from([key]));
            if let Some(request) = p.mail {
                self.on_mail_answer(request, None);
                continue;
            }
            if let Some(h) = self.pending_holder(&p) {
                self.note_unreached(h, p.via.as_deref());
            }
            match self.scatter.ops.get_mut(&p.op) {
                Some(Op::Store(_)) => self.retry_slot(p.op, p.index),
                Some(Op::Load(ld)) => {
                    ld.trouble += 1;
                    self.ask_another(p.op);
                    self.check_load(p.op);
                }
                Some(Op::Probe(pr)) => {
                    let place = p.via.as_ref().and_then(|v| pr.fkey.get(v)).copied().unwrap_or(0);
                    pr.answers.insert(p.index, (None, place));
                    if let Some(drops) = pr.drops.as_mut() {
                        drops.insert(p.index, DropOutcome::Unreachable);
                    }
                    self.check_probe(p.op);
                }
                _ => {}
            }
        }
        self.mailbox_tick();
        self.keep_tick();
        self.deletion_tick();
        self.open_tick();
        self.roster_tick();
        self.inject_tick();
        self.rebuild_tick(None);
        self.home_scatter_tick(None);
        self.drop_share_tick();
    }

    pub(crate) fn scatter_sweep(&mut self) {
        let today = (self.now() / DAY) as u32;
        if let Some(store) = self.scatter.pieces.as_mut() {
            let _ = store.sweep(today);
        }
        self.scatter_tick();
    }

    /// A link went away: what was waiting on it is unanswered.
    pub(crate) fn scatter_disconnect(&mut self, peer: &str) {
        self.forward_disconnect(peer);
        self.rebuild_tick(Some(peer));
        self.home_scatter_tick(Some(peer));
        let gone: Vec<(String, u32)> = self.scatter.pending.keys().filter(|(p, _)| p == peer).cloned().collect();
        for key in gone {
            if let Some(p) = self.scatter.pending.get_mut(&key) {
                p.sent_at = 0;
            }
        }
        self.scatter_tick();
    }
}
