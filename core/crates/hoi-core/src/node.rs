//! The node: holds the store, community states and local state, runs every record through the
//! pipeline (TECHNICAL.md §6), applies its effects, persists, and queues gossip.
//!
//! The order of checks is part of the security design: the cheapest checks run first so junk
//! costs an attacker more than it costs us (a hash before a signature, a signature before an
//! Argon2id run), and nothing is stored, shown or passed on before it passed all of them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use hoi_crypto::Keypair;
use hoi_moderation::Community;
use hoi_moderation::ladder::{Contribution, Inputs, ReviewIn, VoteIn, VouchIn};
use hoi_spam::{Offence, PeerScores, Strikes};
use hoi_store::{Change, Persist, Store, Stored};
use hoi_types::{Body, Id, Kind, Limits, Record, Verdict, Wire, hex_decode, hex_encode};
use hoi_validate::{DAY, Reject};

use crate::dto::{Event, NetStatus};
use crate::error::ApiError;
use crate::state::{Bounce, LocalState, STORAGE_FORMAT_VERSION, Secrets, Skeleton, StorageFormat};

pub type Clock = Box<dyn Fn() -> u64 + Send>;

/// Media this device keeps at most, bytes. Beyond it new media isn't stored (it can be fetched
/// again later); expired posts free their media on the next sweep.
pub const MEDIA_CAP: u64 = 512 * 1024 * 1024;

/// Media a peer asked for that isn't here yet, remembered per peer to send once it arrives.
const MAX_OWED: usize = 256;
/// Seconds before media still missing is asked for again.
const MEDIA_RETRY: u64 = 120;

/// A vouch, review or ledger entry that turns up this long after it says it was made, although
/// this device had caught up since, counts from when it turned up (PROTOCOL.md §15.5).
pub const LATE_GRACE: u64 = 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Made on this device.
    Local,
    /// Read back from this device's storage at startup.
    Load,
    /// From an invite, a bundle file or a device link code.
    Import,
    /// Decrypted and authenticated by the private-group transport, never raw peer bytes.
    PrivateRelay,
    /// Rebuilt from pieces on this device (an open community's, crate::open): never offered
    /// whole to anyone.
    Pieces,
    /// From a peer. `live` records arrived on gossip and must be fresh.
    Peer { peer: String, live: bool },
}

#[derive(Debug, Clone)]
pub struct Accepted {
    pub id: Id,
    pub community: Option<Id>,
    pub kind: Kind,
}

#[derive(Debug, Clone)]
pub(crate) struct PeerState {
    pub communities: BTreeSet<Id>,
    pub name: Option<String>,
    /// The peer serves us (our home node).
    pub serves: bool,
    /// The peer is a browser or phone we serve (paired with our token).
    pub client: bool,
    pub home: bool,
    pub route: String,
    pub said_hello: bool,
    pub sent_have: BTreeSet<Id>,
    /// Catch-ups in progress: community → (when their `have` came, ids we still wait for).
    pub wants: BTreeMap<Id, (u64, BTreeSet<Id>)>,
    /// Remaining advertised records, requested in bounded pages after the current page arrives.
    pub pending_wants: BTreeMap<Id, BTreeSet<Id>>,
    /// Range sessions this node started with the peer and that are still open:
    /// community → (when it last moved, whether the peer answered yet).
    pub ranging: BTreeMap<Id, (u64, bool)>,
    /// Last progress on an active catch-up page, independent of the security snapshot time.
    pub sync_progress: Option<u64>,
    /// Media they asked for while it was still on its way here: sent when it arrives.
    pub owed: BTreeSet<Id>,
    /// Messages in the current minute, and when that minute started.
    pub msgs: u32,
    pub minute: u64,
    /// What the host told us about the connection (transport, endpoint, direction).
    pub meta: crate::net::PeerMeta,
    /// The sender's own routes and optional messages, from its hello.
    pub routes: Vec<String>,
    pub features: BTreeSet<String>,
    /// Verified public listings from this connection; never persisted or automatically joined.
    pub catalog: BTreeMap<Id, hoi_sync::CatalogEntry>,
    /// Suppress unchanged directory snapshots, including empty ones.
    pub sent_catalog: Option<Vec<hoi_sync::CatalogEntry>>,
    pub catalog_msgs: u32,
    pub catalog_minute: u64,
    /// Where the next relayed window to this peer starts, and the window it gets until the next
    /// round: listings by id, rebuilt only on the round so a catalog resent for other reasons
    /// (a community settled, a hello) is the same catalog (crate::directory).
    pub gossip_cursor: Option<Id>,
    pub gossip_ids: Vec<Id>,
    /// When its window was last picked.
    pub gossip_at: u64,
    /// New listings this connection brought this hour: (hour, how many).
    pub listed_new: (u64, u32),
    pub traffic: crate::traffic::PeerTraffic,
    /// `link` messages in the current minute (crate::link).
    pub link_msgs: u32,
    pub link_minute: u64,
    /// Per community: what this peer is known to hold of it (crate::warmth).
    pub offers: BTreeMap<Id, Offer>,
    /// Communities this node asked the peer to carry and has no answer for yet.
    pub carry_asked: BTreeSet<Id>,
    /// `carry` requests received from this peer in the current minute (crate::carry).
    pub carry_msgs: u32,
    pub carry_minute: u64,
    /// `carry` requests sent to this peer in the current minute: a requester paces itself.
    pub carry_sent: u32,
    pub carry_sent_minute: u64,
    /// Salts the community hashes in our hellos to a stranger, fresh for each connection.
    pub salt: Option<[u8; 16]>,
    /// Communities our last hello named in the clear.
    pub named: BTreeSet<Id>,
    /// The salted community hashes in the peer's last hello, with their salt: a community we
    /// take on later is matched against them, so a member already connected is found.
    pub their_hashes: Option<([u8; 16], BTreeSet<Id>)>,
    /// A device that just spent the pairing link: the credential it gets in our hello.
    pub issued: Option<String>,
    /// Hints from a home node that has not delivered records yet, taken once it has.
    pub pending_hints: Vec<hoi_sync::Hints>,
    /// The node list (and the holders it vouched for) last sent to this peer, and `nodes`
    /// messages from it this minute (crate::nodes).
    pub sent_nodes: Option<(Vec<String>, Vec<hoi_sync::Vouch>)>,
    pub nodes_msgs: u32,
    pub nodes_minute: u64,
    /// A stranger this device dialed under the consent gate: our hello waits for its hello, so
    /// an operator's notice is read before this device says anything (crate::notice).
    pub awaiting_hello: bool,
    /// Who runs the peer, as its operator notice names them, as a key: nodes with the same
    /// operator count as one place in Warmth (crate::warmth). Its own word, so it only merges.
    pub operator: Option<u64>,
    /// Its holder key and its requests' pace (crate::scatter).
    pub scatter: crate::scatter::PeerScatter,
    /// The key its hello named (crate::bind).
    pub node_key: Option<Id>,
    /// On a WebRTC link: we sent our binding, or the node's checked out (crate::bind). A
    /// binding goes only after our hello.
    pub hello_sent: bool,
    pub bind_sent: bool,
    pub bound: bool,
    /// Communities whose media this peer didn't send over iroh-blobs on this link: asked with
    /// `bwant` from then on (crate::blobs).
    pub blobs_refused: BTreeSet<Id>,
}

impl PeerState {
    pub(crate) fn role(&self) -> crate::net::Role {
        if self.home {
            crate::net::Role::Home
        } else if self.client {
            crate::net::Role::Client
        } else {
            crate::net::Role::Peer
        }
    }
}

/// What a connected peer is known to hold of one community, for Warmth (PROTOCOL.md §20).
/// Counts restart with each `have` the peer sends. The genesis is kept apart: a peer that
/// subscribes to a community and listed, delivered or received it holds it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Offer {
    /// Live ids in its last `have`, the genesis aside. A listing is a claim: it counts for a
    /// peer this node dialed by a route it already trusted, not for one that merely dialed in.
    pub listed: u64,
    /// Records this node sent it since, the genesis aside.
    pub given: u64,
    /// Valid records it delivered since, the genesis aside: possession shown, not claimed.
    pub delivered: u64,
    pub genesis: bool,
}

/// Sanitized media bytes by hash.
pub(crate) type Blobs = Vec<(Id, Vec<u8>)>;

/// What a batch ingest reports for each record.
pub(crate) type BatchReport<'a> = &'a mut dyn FnMut(&mut Node, &[u8], Result<Accepted, Reject>);

pub(crate) struct Draft {
    pub record: Record,
    pub bits: u32,
    pub made: u64,
    /// Sanitized media bytes to keep once the record is published.
    pub media: Blobs,
}

/// A key made by `draft_identity`, waiting for the host to bring its identity work.
pub(crate) struct PendingIdentity {
    pub name: String,
    /// The recovery line's secret the root is derived from.
    pub master: zeroize::Zeroizing<[u8; 32]>,
    pub root: Keypair,
    pub device: Keypair,
    pub bits: u32,
}

pub struct Node {
    pub(crate) private: crate::private::PrivateState,
    /// Scattered delivery: pieces held for others, and operations in flight (crate::scatter).
    pub(crate) scatter: crate::scatter::ScatterState,
    pub(crate) store: Store,
    pub(crate) communities: HashMap<Id, Community>,
    pub(crate) state: LocalState,
    pub(crate) root_kp: Option<Keypair>,
    /// The recovery line's secret while this device keeps it (crate::recover).
    pub(crate) master: Option<zeroize::Zeroizing<[u8; 32]>>,
    /// A recovery request this device sent to friends, and the answers in (crate::recover).
    pub(crate) asking: crate::recover::Asking,
    /// Remote signing for browsers, on either side (crate::remote).
    pub(crate) remote: crate::remote::Remote,
    /// Autobans that fired on a moderator's device, to write to the ledger after the batch, and
    /// when this device wrote the last ones, per community.
    pub(crate) autobans_due: Vec<(Id, Id)>,
    /// LAN beacons (crate::lan).
    pub(crate) lan: crate::lan::Lan,
    pub(crate) autobans_written: BTreeMap<Id, Vec<u64>>,
    /// Friends of friends: person → the friends whose trust statements name them.
    /// Worked out here from the statements this device holds; never sent anywhere.
    pub(crate) fof: BTreeMap<Id, BTreeSet<Id>>,
    pub(crate) ring_dirty: bool,
    /// What the list keepers this person follows refuse, together (crate::blocklists).
    pub(crate) lists_exact: BTreeSet<Id>,
    pub(crate) lists_pdq: crate::state::PdqList,
    pub(crate) lists_dirty: bool,
    /// Link previews kept on this device, and the fetches under way (crate::preview).
    pub(crate) previews: crate::preview::Previews,
    /// Handle checks under way (crate::handle).
    pub(crate) handles: crate::handle::Handles,
    /// Media over iroh-blobs (crate::blobs).
    pub(crate) blobs: crate::blobs::Blobs,
    /// Direct links between browsers (crate::signal).
    pub(crate) signals: crate::signal::Signals,
    pub(crate) device_kp: Option<Keypair>,
    pub(crate) pending_kp: Option<Keypair>,
    pub(crate) transport_kp: Option<Keypair>,
    pub(crate) community_kps: BTreeMap<Id, Keypair>,
    pub(crate) strikes: Strikes,
    pub(crate) peer_scores: PeerScores,
    /// Ids recently refused per key, kept as evidence for reports.
    pub(crate) evidence: HashMap<(Id, Id), Vec<Id>>,
    persist: Box<dyn Persist>,
    pub(crate) changes: Vec<Change>,
    state_dirty: bool,
    keys_dirty: bool,
    pub(crate) profile_heads: crate::profile::ProfileHeads,
    pub(crate) profile_color_heads: crate::profile::ProfileHeads,
    pub(crate) profile_heads_dirty: bool,
    clock: Clock,
    pub(crate) platform: String,
    pub(crate) web_base: String,
    pub(crate) events: Vec<Event>,
    changed: BTreeSet<Id>,
    /// Communities whose ledger or ladder need settling again.
    pub(crate) dirty: BTreeSet<Id>,
    /// When each community was last settled: the ladder depends on the clock too.
    settled_at: HashMap<Id, u64>,
    pub(crate) drafts: HashMap<u64, Draft>,
    pub(crate) next_draft: u64,
    pub(crate) pending_identity: Option<PendingIdentity>,
    pub(crate) peers: BTreeMap<String, PeerState>,
    pub(crate) outbox: Vec<(String, Vec<u8>)>,
    pub(crate) serving: Option<String>,
    pub(crate) net_error: Option<String>,
    /// What the router said about a public node's ports, while the host asks it (hoi-net portmap).
    pub(crate) port_mapping: Option<crate::dto::PortMapping>,
    /// The VPN as the host found it, while only a VPN may be used.
    pub(crate) vpn_status: Option<crate::dto::VpnStatus>,
    pub(crate) transport: Option<crate::dto::TransportStatus>,
    /// The network status last reported in a `net` event.
    last_net: Option<NetStatus>,
    /// Media hashes asked for, and when.
    pub(crate) media_wanted: HashMap<Id, u64>,
    /// Identity work a new identity is mined to.
    pub(crate) identity_bits: u32,
    pub(crate) loading: bool,
    /// While a batch from a peer is being taken in: the roots that batch carries community
    /// records of, so their identity, device and revocation records may come along.
    batch_roots: Option<BTreeSet<Id>>,
    /// Undecodable records retained on disk for compatible software, never indexed or served.
    pub(crate) preserved_records: u64,
    /// This node's WebRTC routes, set by the native host once its sockets are bound.
    pub(crate) rtc_routes: Vec<String>,
    /// Routes a registry or an announcement suggested for a public join: dialed for this attempt,
    /// never saved, never put in invites or catalogs until they delivered records.
    pub(crate) trial_routes: BTreeMap<Id, Vec<String>>,
    /// Routes of public nodes that other nodes passed on, with when: only tried, never saved or
    /// passed on before the node behind them answered as a public node itself (crate::nodes).
    pub(crate) node_candidates: Vec<(String, u64)>,
    /// Public nodes a dial failed to reach lately, by who they are, with when (crate::nodes).
    pub(crate) node_failed: BTreeMap<crate::net::Reach, u64>,
    /// The WebRTC DTLS certificate and key (DER), kept with the other secrets.
    pub(crate) rtc_cert: Option<(Vec<u8>, Vec<u8>)>,
    pub(crate) traffic: crate::traffic::Traffic,
    /// Devices asking to be linked through the home node, waiting for the person's answer.
    pub(crate) link_asks: BTreeMap<u64, crate::link::PendingAsk>,
    pub(crate) next_ask: u64,
    /// This device asked to be linked and waits for an approval.
    pub(crate) link_waiting: Option<crate::link::Waiting>,
    /// What this node calls itself while it holds no key (set by the host).
    pub(crate) node_name: Option<String>,
    /// Whether this node takes communities from indexes and connected peers (crate::carry).
    pub(crate) carry_policy: crate::carry::CarryPolicy,
    pub(crate) carry_limit: usize,
    pub(crate) carry_bytes: u64,
    /// A person's device: strangers' operator notices are read before it says anything, and a
    /// node link's notice before it connects (crate::notice). Never on a headless node.
    pub(crate) consent_gate: bool,
    /// Media is decoded by the host, in a worker of its own (crate::look). Off on native hosts.
    pub(crate) look_elsewhere: bool,
    pub(crate) looks: crate::look::Looks,
    /// Finding who carries a community, and the DHT among public nodes (crate::find).
    pub(crate) finds: crate::find::Finds,
    /// This node's own operator notice, when the operator gave one (crate::notice).
    pub(crate) operator_notice: Option<crate::notice::OperatorNotice>,
    /// Links this device closed for a notice the person has not read yet: no refusal, no backoff.
    pub(crate) consent_closed: BTreeSet<String>,
    /// Requests accepted in the current minute and day, node-wide (crate::carry).
    pub(crate) carry_accepted: (u64, u32),
    pub(crate) carry_accepted_day: (u64, u32),
    /// Communities the operator refuses to carry, whoever asks (crate::carry).
    pub(crate) refused: BTreeSet<Id>,
    /// The `w` this node last attached to its catalog entries, with when it was read.
    pub(crate) warmth_hints: BTreeMap<Id, (u64, u8)>,
    /// Communities this public node offered other public nodes today: (UTC day, how many).
    pub(crate) spread: BTreeMap<Id, (u64, u8)>,
    /// Open communities heard of from other nodes, kept between starts (crate::directory).
    pub(crate) directory: crate::directory::Directory,
    /// When relayed listings last went out to every peer (crate::directory).
    pub(crate) gossiped_at: u64,
    /// Open communities this device's person made an invite for in this run: the public nodes
    /// it reaches are asked to keep them, as for the communities the person founded (crate::carry).
    pub(crate) invite_carry: BTreeSet<Id>,
    /// Records made before this earn no strikes for breaking the protocol's rules
    /// (`hoi_validate::RULES_SINCE`; tests move it).
    pub(crate) rules_since: u64,
    /// Identity work a native host checked before taking the node lock, for the message being
    /// taken in (crate::work); the pipeline looks answers up here instead of running Argon2id.
    pub(crate) checked_work: HashMap<(Id, u64), Result<u32, Reject>>,
}

/// Ingest order when many records arrive at once: what others depend on first.
pub(crate) fn priority(k: Kind) -> u8 {
    match k {
        Kind::Community => 0,
        Kind::Identity => 1,
        // after the roots it names, before anything they sign
        Kind::Rotate => 2,
        Kind::Delegation => 3,
        Kind::Revoke => 4,
        Kind::Ledger => 5,
        Kind::Vouch => 6,
        Kind::Post => 7,
        Kind::Comment => 8,
        Kind::Edit => 9,
        Kind::Review => 10,
        Kind::Vote => 11,
        Kind::Label => 12,
        Kind::Report => 13,
        Kind::Delete => 14,
        Kind::Profile => 15,
        Kind::ProfileColor => 16,
        Kind::Pointer => 17,
        // after the ladder settled: only an established member's counts
        Kind::Roster => 18,
        Kind::Share => 19,
        Kind::Trust => 20,
        Kind::Wiki => 21,
        Kind::Tally => 22,
        Kind::Blocklist => 23,
    }
}

/// Devices one root may have blessed and live at once (PROTOCOL.md §3); more need a revocation first.
pub(crate) const MAX_LIVE_DEVICES: usize = 16;

/// In a batch, the ladder is settled again before records of this priority and later, so
/// people admitted by vouches in the same batch post with their new standing.
pub(crate) const SETTLE_BEFORE: u8 = 7;

/// The stamp, retention, identity work and newcomer rate a record answers to: the limits in
/// effect when it says it was made, or today's where they are more lenient (PROTOCOL.md §4).
/// Members took it under the limits of its day, so a device that joins after the community
/// tightened them takes it too; nothing made since needs more than today's.
pub(crate) fn limits_for(c: &Community, created_at: u64) -> Limits {
    let then = c.limits_at(created_at);
    let mut l = c.limits.clone();
    l.pow_bits = l.pow_bits.min(then.pow_bits);
    l.max_retention = l.max_retention.max(then.max_retention);
    l.newcomer_posts_per_day = l.newcomer_posts_per_day.max(then.newcomer_posts_per_day);
    l.ladder.identity_bits = l.ladder.identity_bits.min(then.ladder.identity_bits);
    l
}

pub(crate) fn keypair_from_hex(h: &str) -> Option<Keypair> {
    let b = hex_decode(h).ok()?;
    let arr: [u8; 32] = b.try_into().ok()?;
    Some(Keypair::from_secret(&arr))
}

pub(crate) fn media_key(h: &Id) -> String {
    format!("m:{}", h.hex())
}

impl Node {
    /// Opens a node over `persist`, replaying everything stored there.
    pub fn open(mut persist: Box<dyn Persist>, platform: &str, web_base: &str, clock: Clock) -> Result<Node, ApiError> {
        let loaded = persist.load().map_err(|e| ApiError::new("internal", format!("storage: {e}")))?;
        let incompatible = |part: &str| {
            ApiError::new(
                "storage-incompatible",
                format!(
                    "Saved {part} is damaged or uses an unsupported format. No stored data was changed. Back up this storage and open it with compatible software."
                ),
            )
        };
        let format = loaded.blobs.iter().find(|(key, _)| key == "format");
        let format_version = if let Some((_, bytes)) = format {
            let format: StorageFormat = serde_json::from_slice(bytes).map_err(|_| incompatible("storage metadata"))?;
            if !(1..=STORAGE_FORMAT_VERSION).contains(&format.version) {
                return Err(incompatible("storage metadata"));
            }
            format.version
        } else {
            1
        };
        let mut node = Node {
            private: crate::private::PrivateState::default(),
            scatter: Default::default(),
            store: Store::new(),
            communities: HashMap::new(),
            state: LocalState::default(),
            root_kp: None,
            master: None,
            asking: Default::default(),
            remote: Default::default(),
            autobans_due: Vec::new(),
            lan: Default::default(),
            autobans_written: BTreeMap::new(),
            fof: BTreeMap::new(),
            ring_dirty: true,
            lists_exact: BTreeSet::new(),
            lists_pdq: Default::default(),
            lists_dirty: true,
            previews: Default::default(),
            handles: Default::default(),
            blobs: Default::default(),
            signals: Default::default(),
            device_kp: None,
            pending_kp: None,
            transport_kp: None,
            community_kps: BTreeMap::new(),
            strikes: Strikes::default(),
            peer_scores: PeerScores::default(),
            evidence: HashMap::new(),
            persist,
            changes: Vec::new(),
            state_dirty: false,
            keys_dirty: false,
            profile_heads: BTreeMap::new(),
            profile_color_heads: BTreeMap::new(),
            profile_heads_dirty: false,
            clock,
            platform: platform.to_string(),
            web_base: web_base.to_string(),
            events: Vec::new(),
            changed: BTreeSet::new(),
            dirty: BTreeSet::new(),
            settled_at: HashMap::new(),
            drafts: HashMap::new(),
            pending_identity: None,
            next_draft: 1,
            peers: BTreeMap::new(),
            outbox: Vec::new(),
            serving: None,
            net_error: None,
            port_mapping: None,
            vpn_status: None,
            transport: None,
            last_net: None,
            media_wanted: HashMap::new(),
            identity_bits: hoi_crypto::identity::DEFAULT_BITS,
            loading: false,
            batch_roots: None,
            preserved_records: 0,
            rtc_routes: Vec::new(),
            trial_routes: BTreeMap::new(),
            node_candidates: Vec::new(),
            node_failed: BTreeMap::new(),
            rtc_cert: None,
            traffic: crate::traffic::Traffic::default(),
            link_asks: BTreeMap::new(),
            next_ask: 1,
            link_waiting: None,
            node_name: None,
            carry_policy: crate::carry::CarryPolicy::Off,
            carry_limit: crate::carry::DEFAULT_CARRY_LIMIT,
            carry_bytes: crate::carry::DEFAULT_CARRY_BYTES,
            consent_gate: false,
            look_elsewhere: false,
            looks: Default::default(),
            finds: Default::default(),
            operator_notice: None,
            consent_closed: BTreeSet::new(),
            carry_accepted: (0, 0),
            carry_accepted_day: (0, 0),
            refused: BTreeSet::new(),
            warmth_hints: BTreeMap::new(),
            spread: BTreeMap::new(),
            directory: Default::default(),
            gossiped_at: 0,
            invite_carry: BTreeSet::new(),
            rules_since: hoi_validate::RULES_SINCE,
            checked_work: HashMap::new(),
        };
        node.traffic.started = node.now();
        let mut blobs = Vec::new();
        let mut preview_index: Option<Vec<u8>> = None;
        let mut preview_images: Vec<(Id, Vec<u8>)> = Vec::new();
        for (k, v) in &loaded.blobs {
            match k.as_str() {
                "mls" => node.private = serde_json::from_slice(v).map_err(|_| incompatible("private community keys"))?,
                "state" => node.state = serde_json::from_slice(v).map_err(|_| incompatible("local state"))?,
                "profiles" => node.profile_heads = serde_json::from_slice(v).map_err(|_| incompatible("profile revisions"))?,
                "profile-colors" => node.profile_color_heads = serde_json::from_slice(v).map_err(|_| incompatible("profile color revisions"))?,
                // A directory that can't be read is only a cache of what other nodes listed: it
                // starts over rather than keeping the device from opening.
                crate::directory::BLOB => node.directory = serde_json::from_slice(v).unwrap_or_default(),
                // Sightings start over if unreadable; a mailbox writer that lost its place is
                // told `taken` by the holders and moves on, so nothing is overwritten.
                crate::scatter::BLOB => {
                    node.scatter.saved = serde_json::from_slice(v).unwrap_or_default();
                    node.scatter.in_pieces = node.scatter.saved.kept.values().filter(|k| k.open).flat_map(|k| k.records.iter().copied()).collect();
                }
                "keys" => {
                    let mut s: Secrets = serde_json::from_slice(v).map_err(|_| incompatible("keys"))?;
                    let checked = (|| {
                        let key = |value: Option<&str>| value.map(|v| keypair_from_hex(v).ok_or_else(|| incompatible("keys"))).transpose();
                        node.transport_kp = key(s.transport.as_deref())?;
                        node.root_kp = key(s.root.as_deref())?;
                        for (device, (secret, label)) in &s.remote {
                            let kp = keypair_from_hex(secret).filter(|k| k.public() == *device).ok_or_else(|| incompatible("keys"))?;
                            node.remote.keys.insert(*device, crate::remote::RemoteKey { kp, label: label.clone() });
                        }
                        node.master = match s.master.as_deref() {
                            Some(h) => Some(zeroize::Zeroizing::new(
                                hoi_types::hex_decode(h).ok().and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()).ok_or_else(|| incompatible("keys"))?,
                            )),
                            None => None,
                        };
                        node.device_kp = key(s.device.as_deref())?;
                        node.pending_kp = key(s.pending_device.as_deref())?;
                        node.community_kps = s
                            .communities
                            .iter()
                            .map(|(c, h)| keypair_from_hex(h).map(|key| (*c, key)).ok_or_else(|| incompatible("keys")))
                            .collect::<Result<_, _>>()?;
                        node.rtc_cert = s
                            .rtc_cert
                            .as_ref()
                            .map(|(c, k)| {
                                let cert = hoi_types::uri::unb64(c).map_err(|_| incompatible("transport keys"))?;
                                let key = hoi_types::uri::unb64(k).map_err(|_| incompatible("transport keys"))?;
                                if cert.is_empty() || key.is_empty() {
                                    return Err(incompatible("transport keys"));
                                }
                                Ok((cert, key))
                            })
                            .transpose()?;
                        Ok::<(), ApiError>(())
                    })();
                    s.wipe();
                    checked?;
                }
                m if m.starts_with("m:") => {
                    if let Ok(h) = Id::from_hex(&m[2..]) {
                        blobs.push((h, v.clone()));
                    }
                }
                crate::preview::BLOB => preview_index = Some(v.clone()),
                m if m.starts_with("pv:") => {
                    if let Ok(h) = Id::from_hex(&m[3..]) {
                        preview_images.push((h, v.clone()));
                    }
                }
                _ => {}
            }
        }
        node.load_previews(preview_index.as_deref(), preview_images);
        // Legacy stores migrate after validation. The current guard prevents older clients
        // from bypassing private record and blob provenance on plaintext sync paths.
        if format.is_none() || format_version < STORAGE_FORMAT_VERSION {
            node.changes.push(Change::Blob { key: "format".into(), value: Some(StorageFormat::encoded()) });
        }
        if node.state.node_id.is_none() {
            node.state.node_id = Some(Id(hoi_crypto::random_bytes::<32>()?));
            node.state_dirty = true;
        }
        node.native_defaults();
        node.seed_suppressions();

        // Replay supported records, dependencies first. Unrecognized wire formats remain on
        // disk for compatible software; they are neither indexed nor relayed by this build.
        let mut recs: Vec<(u8, u64, Vec<u8>)> = Vec::new();
        for w in loaded.records {
            match Wire::decode(&w) {
                Ok((wire, _)) => recs.push((priority(wire.record.kind()), wire.record.created_at, w)),
                Err(_) => {
                    node.preserved_records += 1;
                }
            }
        }
        recs.sort_by_key(|a| (a.0, a.1));
        node.loading = true;
        for (_, _, w) in recs {
            if let Err(e) = node.ingest(&w, &Source::Load)
                && e != Reject::Duplicate
                && let Ok((_, id)) = Wire::decode(&w)
            {
                node.changes.push(Change::Delete { id });
            }
        }
        node.loading = false;
        for (h, b) in blobs {
            if blake3::hash(&b).as_bytes() == h.as_bytes() && !node.state.suppressed.media.contains_key(&h) {
                node.store.put_blob(h, b);
            } else {
                node.changes.push(Change::Blob { key: media_key(&h), value: None });
            }
        }
        if format_version < 3 {
            // Interim private stores did not persist blob provenance. Default their private
            // attachments to private even if an unrelated public post references the hash.
            let private_blobs: Vec<Id> = node
                .store
                .all()
                .filter(|s| s.record.community.is_some_and(|c| node.is_private_community(&c)))
                .flat_map(|s| match &s.record.body {
                    Body::Post { media, .. } => media.iter().map(|m| m.hash).collect(),
                    _ => Vec::new(),
                })
                .collect();
            if !private_blobs.is_empty() {
                node.state.private_blobs.extend(private_blobs);
                node.mark_state();
            }
        }
        let cs: Vec<Id> = node.communities.keys().copied().collect();
        for c in cs {
            node.rebuild(&c);
        }
        node.sweep();
        node.ensure_identity();
        node.renew_delegation();
        node.changed.clear();
        node.events.clear();
        node.commit()?;
        Ok(node)
    }

    /// Identity work new identities on this node are mined to (tests and slow devices may use
    /// less, never below the protocol floor).
    pub fn set_identity_bits(&mut self, bits: u32) {
        self.identity_bits = bits.clamp(hoi_crypto::identity::MIN_BITS, hoi_crypto::identity::MAX_BITS);
    }

    pub fn now(&self) -> u64 {
        (self.clock)()
    }

    pub(crate) fn me_root(&self) -> Option<Id> {
        self.state.me.as_ref().map(|m| m.root)
    }

    pub(crate) fn mark_state(&mut self) {
        self.state_dirty = true;
    }

    pub(crate) fn mark_keys(&mut self) {
        self.keys_dirty = true;
    }

    pub(crate) fn push_change(&mut self, c: Change) {
        self.changes.push(c);
    }

    pub(crate) fn mark_changed(&mut self, c: Id) {
        self.changed.insert(c);
    }

    pub(crate) fn notice(&mut self, text: impl Into<String>) {
        self.events.push(Event::Notice { text: text.into() });
    }

    /// Writes pending changes to storage and turns changed communities into an event.
    pub fn commit(&mut self) -> Result<(), ApiError> {
        self.settle_dirty();
        // autobans that fired since: to the ledger, on a moderator's device
        if !self.autobans_due.is_empty() {
            self.write_autobans();
            self.settle_dirty();
        }
        if self.profile_heads_dirty {
            self.changes.push(Change::Blob {
                key: "profile-colors".into(),
                value: Some(serde_json::to_vec(&self.profile_color_heads).map_err(|e| ApiError::new("internal", e.to_string()))?),
            });
            self.changes.push(Change::Blob {
                key: "profiles".into(),
                value: Some(serde_json::to_vec(&self.profile_heads).map_err(|e| ApiError::new("internal", e.to_string()))?),
            });
            self.profile_heads_dirty = false;
        }
        if self.private.dirty {
            self.changes.push(Change::Blob {
                key: "mls".into(),
                value: Some(serde_json::to_vec(&self.private).map_err(|_| ApiError::new("internal", "Private state could not be saved."))?),
            });
            self.private.dirty = false;
        }
        if let Some(v) = self.directory.take_save(self.now()) {
            self.changes.push(Change::Blob { key: crate::directory::BLOB.into(), value: Some(v) });
        }
        if self.scatter.dirty {
            let v = serde_json::to_vec(&self.scatter.saved).map_err(|e| ApiError::new("internal", e.to_string()))?;
            self.changes.push(Change::Blob { key: crate::scatter::BLOB.into(), value: Some(v) });
            self.scatter.dirty = false;
        }
        if self.state_dirty {
            let v = serde_json::to_vec(&self.state).map_err(|e| ApiError::new("internal", e.to_string()))?;
            self.changes.push(Change::Blob { key: "state".into(), value: Some(v) });
            self.state_dirty = false;
        }
        if self.keys_dirty {
            let mut s = Secrets {
                transport: self.transport_kp.as_ref().map(|k| hex_encode(&k.secret())),
                root: self.root_kp.as_ref().map(|k| hex_encode(&k.secret())),
                device: self.device_kp.as_ref().map(|k| hex_encode(&k.secret())),
                pending_device: self.pending_kp.as_ref().map(|k| hex_encode(&k.secret())),
                communities: self.community_kps.iter().map(|(c, k)| (*c, hex_encode(&k.secret()))).collect(),
                rtc_cert: self.rtc_cert.as_ref().map(|(c, k)| (hoi_types::uri::b64(c), hoi_types::uri::b64(k))),
                master: self.master.as_ref().map(|m| hex_encode(m.as_slice())),
                remote: self.remote.keys.iter().map(|(d, k)| (*d, (hex_encode(&k.kp.secret()), k.label.clone()))).collect(),
            };
            let v = serde_json::to_vec(&s).map_err(|e| ApiError::new("internal", e.to_string()));
            s.wipe();
            self.changes.push(Change::Blob { key: "keys".into(), value: Some(v?) });
            self.keys_dirty = false;
        }
        if !self.changes.is_empty() {
            // Keep the ordered journal until storage accepts it. On failure, a later commit
            // must retry records, keys and state together, including any newer changes.
            self.persist.apply(&self.changes).map_err(|e| ApiError::new("internal", format!("storage: {e}")))?;
            self.changes.clear();
        }
        if !self.changed.is_empty() {
            let cs = std::mem::take(&mut self.changed).into_iter().map(|c| c.hex()).collect();
            self.events.push(Event::Changed { communities: cs });
        }
        let net = self.net_status();
        if self.last_net.as_ref() != Some(&net) {
            self.last_net = Some(net.clone());
            self.events.push(Event::Net { status: Box::new(net) });
        }
        Ok(())
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Drains the persistence journal when the host owns storage (browser).
    pub fn persist_mut(&mut self) -> &mut dyn Persist {
        self.persist.as_mut()
    }

    pub(crate) fn wipe_all(&mut self) -> Result<(), ApiError> {
        self.private.wipe();
        self.store.clear();
        self.communities.clear();
        let node_id = Some(Id(hoi_crypto::random_bytes::<32>()?));
        self.state = LocalState { node_id, ..LocalState::default() };
        self.profile_heads.clear();
        self.profile_color_heads.clear();
        self.profile_heads_dirty = true;
        self.root_kp = None;
        self.master = None;
        self.asking = Default::default();
        self.remote = Default::default();
        self.device_kp = None;
        self.pending_kp = None;
        self.transport_kp = None;
        self.transport = None;
        self.peers.clear();
        self.outbox.clear();
        self.community_kps.clear();
        self.drafts.clear();
        self.pending_identity = None;
        self.dirty.clear();
        self.settled_at.clear();
        self.media_wanted.clear();
        self.changes.clear();
        self.changes.push(Change::Wipe);
        self.changes.push(Change::Blob { key: "format".into(), value: Some(StorageFormat::encoded()) });
        self.preserved_records = 0;
        self.rtc_cert = None;
        self.rtc_routes.clear();
        self.trial_routes.clear();
        self.node_candidates.clear();
        self.node_failed.clear();
        self.directory = Default::default();
        self.invite_carry.clear();
        self.link_asks.clear();
        self.link_waiting = None;
        self.native_defaults();
        self.state_dirty = true;
        self.keys_dirty = true;
        Ok(())
    }

    /// Native nodes relay by default (opt-out, TECHNICAL.md §4); the app tells people once. The
    /// desktop opens light, as the companion, until someone chooses the full app.
    fn native_defaults(&mut self) {
        if !self.state.relay_defaulted && matches!(self.platform.as_str(), "desktop" | "node") {
            self.state.settings.relay = true;
            self.state.relay_defaulted = true;
            self.state_dirty = true;
        }
        if !self.state.companion_defaulted && self.platform == "desktop" {
            self.state.settings.companion = true;
            self.state.companion_defaulted = true;
            self.state_dirty = true;
        }
    }

    // ------------------------------------------------------------------ the pipeline

    fn key_age(&self, root: &Id, now: u64) -> u64 {
        self.state.first_seen.get(root).map_or(0, |t| now.saturating_sub(*t))
    }

    /// When a record counts from: when it says it was made, or when this device first saw it if
    /// it turned up late (PROTOCOL.md §15.5).
    pub(crate) fn effective_at(&self, s: &Stored) -> u64 {
        self.state.late.get(&s.id).map_or(s.record.created_at, |t| (*t).max(s.record.created_at))
    }

    /// The proposal a review answers, if its parent is one.
    pub(crate) fn proposal_of(&self, r: &Record) -> Option<&hoi_moderation::Proposal> {
        let (c, p) = (r.community?, r.parent?);
        self.communities.get(&c)?.proposals.get(&p)
    }

    pub(crate) fn bits_for(&self, r: &Record, key_age: u64) -> u32 {
        match r.community.and_then(|c| self.communities.get(&c)) {
            Some(st) if r.kind() != Kind::Community => hoi_crypto::pow::required_bits(limits_for(st, r.created_at).pow_bits, r.kind().pow_discount(), key_age),
            _ => hoi_types::GLOBAL_POW_BITS,
        }
    }

    /// Stage 3: the root behind a record, with the rules that keep identities apart: a device
    /// belongs to the first root that blessed it with its consent, devices can't be identities
    /// and identities can't be devices, only a device's own root revokes it.
    fn speaker(&self, r: &Record) -> Result<Id, Reject> {
        // A root replaced under a recovery key signs nothing from then on, nor do its devices.
        // What it signed before stays.
        let replaced = |root: &Id| self.store.replaced_at(root).is_some_and(|t| r.created_at >= t);
        match &r.body {
            Body::Community(_) => Ok(r.author),
            Body::Identity { of, .. } => {
                if self.store.delegation(&r.author).is_some() {
                    return Err(Reject::NotAllowed("a device key can't be an identity".into()));
                }
                if *of == Some(r.author) {
                    return Err(Reject::NotAllowed("a root continues someone else".into()));
                }
                if replaced(&r.author) {
                    return Err(Reject::Revoked);
                }
                Ok(r.author)
            }
            Body::Rotate { person, to } => self.rotation(r, person, to),
            Body::Delegation { .. } => {
                let root = hoi_validate::delegation_consent(r)?;
                if self.store.identity(&r.author).is_some() {
                    return Err(Reject::NotAllowed("an identity can't be someone's device".into()));
                }
                if self.store.delegation(&r.author).is_some_and(|d| d.info.root != root) {
                    return Err(Reject::NotAllowed("that device already belongs to another identity".into()));
                }
                if replaced(&root) {
                    return Err(Reject::Revoked);
                }
                Ok(root)
            }
            Body::Revoke { device } => {
                if self.store.delegation(&r.author).is_some() {
                    return Err(Reject::NotAllowed("only a root revokes".into()));
                }
                if *device != r.author
                    && let Some(dl) = self.store.delegation(device)
                    && dl.info.root != r.author
                {
                    return Err(Reject::NotAllowed("only the root that blessed a device can revoke it".into()));
                }
                if replaced(&r.author) {
                    return Err(Reject::Revoked);
                }
                Ok(r.author)
            }
            _ => {
                let root = match self.store.delegation(&r.author) {
                    Some(dl) => hoi_validate::delegation(r, Some(&dl.info))?,
                    None => r.author,
                };
                if replaced(&root) {
                    return Err(Reject::Revoked);
                }
                // A root that continues someone speaks for them once its rotation is here; until
                // then it is no one (and its record comes again with the rotation).
                if self.store.identity(&root).is_some_and(|i| i.of.is_some()) && self.store.person_of(&root) == root {
                    return Err(Reject::NoIdentity);
                }
                Ok(self.store.person_of(&root))
            }
        }
    }

    /// A `rotate`: signed by the recovery key a root of `person` committed to, naming
    /// a root whose identity continues `person`. The earliest valid rotation from a root counts.
    fn rotation(&self, r: &Record, person: &Id, to: &Id) -> Result<Id, Reject> {
        if !hoi_crypto::key_is_strong(&r.author) {
            return Err(Reject::NotAllowed("a weak recovery key".into()));
        }
        let Some(first) = self.store.identity(person) else { return Err(Reject::NoIdentity) };
        if first.of.is_some() {
            return Err(Reject::NotAllowed("a person's first root continues no one".into()));
        }
        let commitment = hoi_crypto::recovery::commitment(&r.author);
        let chain = self.store.chain(person);
        let Some(from) = chain.iter().copied().find(|root| self.store.identity(root).and_then(|i| i.rc) == Some(commitment)) else {
            return Err(Reject::NotAllowed("not the recovery key this identity committed to".into()));
        };
        match self.store.identity(to) {
            None => return Err(Reject::NoIdentity),
            Some(i) if i.of != Some(*person) => return Err(Reject::NotAllowed("the new root doesn't say whom it continues".into())),
            Some(_) => {}
        }
        if self.store.delegation(to).is_some() || chain.contains(to) || self.store.person_of(to) != *to {
            return Err(Reject::NotAllowed("that root is already someone's".into()));
        }
        // the root it replaces had taken over before this was signed
        if let Some(into) = self.store.rotations_of(person).into_iter().find(|x| x.to == from)
            && into.at > r.created_at
        {
            return Err(Reject::NotAllowed("a rotation from before its root took over".into()));
        }
        if self.store.replaced_at(&from).is_some_and(|t| t <= r.created_at) {
            return Err(Reject::NotAllowed("that root was already replaced".into()));
        }
        Ok(*person)
    }

    /// The identity work an `identity` record carries, from cache, from the host's check before
    /// it took the node lock (crate::work), or one Argon2id run.
    fn identity_work(&mut self, r: &Record) -> Result<u32, Reject> {
        let Body::Identity { work, .. } = r.body else { return Ok(0) };
        if let Some((w, bits)) = self.state.identity_work.get(&r.author)
            && *w == work
        {
            return Ok(*bits);
        }
        let bits = match self.checked_work.remove(&(r.author, work)) {
            Some(checked) => checked?,
            None => hoi_validate::identity(r)?,
        };
        self.state.identity_work.insert(r.author, (work, bits));
        if !self.loading {
            self.mark_state();
        }
        Ok(bits)
    }

    /// Runs one record through every stage. Effects are applied, persisted and gossiped.
    pub fn ingest(&mut self, bytes: &[u8], src: &Source) -> Result<Accepted, Reject> {
        let now = self.now();
        let d = hoi_validate::decode(bytes)?;
        if d.wire.record.community.is_some_and(|c| self.is_private_community(&c)) && matches!(src, Source::Peer { .. } | Source::Import) {
            return Err(Reject::NotAllowed("Private community records require the encrypted group connection.".into()));
        }
        if matches!(src, Source::Local)
            && let Some(c) = d.wire.record.community.filter(|c| self.is_private_community(c))
            && !self.private_writable(c)
            && !(d.wire.record.kind() == Kind::Community && self.community_kps.contains_key(&c))
        {
            return Err(Reject::NotAllowed("This device no longer has active encrypted membership. Ask the owner for a new invitation.".into()));
        }
        let id = d.id;
        if self.store.contains(&id) {
            // Observing the complete, valid record on a public path is an explicit disclosure;
            // guessing its hash in a Want is not. Do not promote records merely on local load.
            if matches!(src, Source::Local | Source::Import | Source::Peer { .. })
                && self.store.get(&id).is_some_and(|s| s.wire == bytes)
                && self.state.private_carriers.remove(&id)
            {
                self.mark_state();
            }
            if let Source::Peer { .. } = src
                && let Some(s) = self.store.get_mut(&id)
            {
                s.from_peers += 1;
            }
            // one of this device's own, back from elsewhere: offered as any other from now on
            if matches!(src, Source::Peer { .. }) {
                self.came_back(&id);
            }
            return Err(Reject::Duplicate);
        }
        // What the operator suppressed stays gone, however it arrives (crate::suppress); at load
        // this also deletes any copy still on disk.
        if self.store.is_deleted(&id) || self.state.skeletons.contains_key(&id) || self.suppressed(&id, d.wire.record.community) {
            return Err(Reject::Tombstoned);
        }
        let loading = matches!(src, Source::Load);
        let kind = d.wire.record.kind();
        // which community, and do we carry it: before any signature work, so a stranger's
        // records for communities we don't hold cost one hash and nothing more
        let community = d.wire.record.community;
        if let Some(c) = community {
            if !self.state.joined.contains_key(&c) {
                return Err(Reject::NotAllowed("not subscribed to this community".into()));
            }
            if kind != Kind::Community && !self.communities.contains_key(&c) {
                return Err(Reject::UnknownCommunity);
            }
        }
        // stage 2: the cheapest check first, before any signature work. For a community we
        // hold, the floor is the least any key could need there (its stamp minus the kind and
        // the largest age discount), not the protocol-wide minimum.
        if !loading {
            let floor = match community.and_then(|c| self.communities.get(&c)) {
                Some(st) if kind != Kind::Community => hoi_validate::stamp_floor(kind).max(hoi_crypto::pow::required_bits(
                    limits_for(st, d.wire.record.created_at).pow_bits,
                    kind.pow_discount(),
                    366 * 86_400,
                )),
                _ => hoi_validate::stamp_floor(kind),
            };
            hoi_validate::stamp(&id, d.wire.record.nonce, floor)?;
        }
        // stage 3: signatures and who is really speaking
        hoi_validate::signature(&d)?;
        let r = &d.wire.record;
        let root = self.speaker(r)?;
        let is_burn = matches!(&r.body, Body::Revoke { device } if *device == r.author);
        if self.store.burned_at(&root).is_some() && !is_burn {
            return Err(Reject::Burned);
        }
        if kind == Kind::Community && self.store.genesis_of(&r.author).is_some() {
            return Err(Reject::NotAllowed("a community's genesis never changes".into()));
        }
        // Identities, devices and revocations have no community and never expire soon. From a
        // stranger (not one of our own devices, not our home node) they are taken only for roots
        // we already know (they posted in a community we hold, or we hold their identity or a
        // device of theirs) or that the same batch carries a community record of, as the
        // protocol says they travel (PROTOCOL.md §7). Otherwise anyone could fill every node
        // with keys nobody uses, for good.
        if community.is_none()
            && let Source::Peer { peer, .. } = src
            && !self.peers.get(peer).is_some_and(|p| p.client || p.home)
            && Some(root) != self.me_root()
            && !self.root_known(&root)
            && !self.batch_roots.as_ref().is_some_and(|b| b.contains(&root))
            // a new root that continues someone known here
            && !matches!(&r.body, Body::Identity { of: Some(p), .. } if self.root_known(p) || self.batch_roots.as_ref().is_some_and(|b| b.contains(p)))
        {
            return Err(Reject::NotAllowed("an identity, device or revocation of a key nobody here has heard from".into()));
        }
        // A root blesses only so many devices at once; the rest can be revoked first.
        if let Body::Delegation { .. } = &r.body
            && !loading
            && Some(root) != self.me_root()
            && self.store.delegation(&r.author).is_none()
            && self.store.live_devices(&root, now) >= MAX_LIVE_DEVICES
        {
            return Err(Reject::NotAllowed("that key already has as many devices as a key may have".into()));
        }

        // stage 2, in full: the stamp the community asks for
        let age = self.key_age(&root, now);
        let need = if loading { 0 } else { self.bits_for(r, age) };
        let bits = hoi_validate::stamp(&id, r.nonce, need)?;

        // stage 3b: every author in a community is an identity with enough work behind it
        let work_bits = self.identity_work(r)?;
        // (stored records passed this when they arrived; at startup the limits that applied
        // then aren't replayed yet, so checking again could only wrongly drop them)
        if let Some(st) = community.filter(|_| kind != Kind::Community && !loading).and_then(|c| self.communities.get(&c)) {
            let need = limits_for(st, r.created_at).ladder.identity_bits;
            match self.store.best_identity(&root) {
                None => return Err(Reject::NoIdentity),
                Some(i) if i.bits < need => return Err(Reject::WeakIdentity { have: i.bits, need }),
                _ => {}
            }
        }

        // stage 4: time
        let threshold_proposal = matches!(&r.body, Body::Ledger(l) if l.cosign.as_ref().is_some_and(|c| !c.is_empty()));
        let live = matches!(src, Source::Peer { live: true, .. }) && !threshold_proposal;
        // answers to a proposal are kept as long as the proposal: they decide it
        let answer = matches!(r.body, Body::Review { .. }) && self.proposal_of(r).is_some();
        let max_ret = if hoi_validate::exempt_from_retention(r) || answer {
            None
        } else {
            community.and_then(|c| self.communities.get(&c)).map(|st| limits_for(st, r.created_at).max_retention)
        };
        hoi_validate::time(r, now, live, max_ret)?;
        if threshold_proposal && matches!(src, Source::Local | Source::Peer { live: true, .. }) && r.created_at + crate::api::PROPOSAL_MAX_AGE < now {
            return Err(Reject::TooOld);
        }

        let is_me = Some(root) == self.me_root();
        // stage 6: moderation state
        if let Some(st) = community.and_then(|c| self.communities.get(&c)) {
            if let Some(b) = st.bans.get(&root)
                && r.created_at >= b.at
            {
                return Err(Reject::Banned);
            }
            if let Some(rm) = st.tombstones.get(&id)
                && rm.effective_at <= now
            {
                self.store.mark_deleted(id, r.expires_at);
                return Err(Reject::Tombstoned);
            }
        }
        // A bounce lifts on its own, so what was stored before it stays stored (and hidden) over a
        // restart: deleting it would outlast the bounce.
        if !is_me && !loading && self.state.bounced.get(&root).is_some_and(|b| b.active(now)) {
            self.state.bounced_records += 1;
            self.mark_state();
            return Err(Reject::Banned);
        }

        // stage 5: rate, by the records' own time, whatever path they came by
        if !loading
            && let Some(c) = community
            && let Err(e) = self.check_rate(r, root, c)
        {
            if matches!(src, Source::Peer { live: true, .. }) {
                self.offence(root, c, (id, r.created_at), Offence::Rate, now);
            }
            return Err(e);
        }

        // stage 7: content rules, and copy-paste floods
        if let Err(e) = hoi_validate::content(r) {
            if let (Source::Peer { .. }, Some(c)) = (src, community) {
                self.offence(root, c, (id, r.created_at), Offence::Content, now);
            }
            return Err(e);
        }
        if !loading
            && let Some(c) = community
            && let Err(e) = self.check_duplicate(r, root, c)
        {
            if let Source::Peer { .. } = src {
                self.offence(root, c, (id, r.created_at), Offence::Duplicate, now);
            }
            return Err(e);
        }

        self.semantic(r, &id, root, src)?;
        self.check_public_join(r)?;
        let superseded = match self.superseded(r, &id, root) {
            Ok(v) => v,
            Err(e) => {
                // remember it, so peers offering it again aren't asked for it again
                self.store.mark_deleted(id, r.expires_at);
                return Err(e);
            }
        };

        // stage 8: accept
        let from_peers = matches!(src, Source::Peer { .. }) as u32;
        let stored = Stored {
            id,
            record: r.clone(),
            sig: d.wire.sig,
            wire: bytes.to_vec(),
            author_root: root,
            received_at: now,
            from_peers,
            stamp_bits: bits,
            work_bits,
        };
        let expires_at = r.expires_at;
        let record = r.clone();
        if matches!(kind, Kind::Vouch | Kind::Review | Kind::Ledger)
            && matches!(src, Source::Peer { .. } | Source::Import | Source::PrivateRelay | Source::Pieces)
            && let Some(c) = community
            && self.state.synced.get(&c).is_some_and(|t| r.created_at.saturating_add(LATE_GRACE) < *t)
        {
            self.state.late.insert(id, now);
            self.mark_state();
        }
        if community.is_none() && matches!(src, Source::PrivateRelay) {
            self.state.private_carriers.insert(id);
            self.mark_state();
        }
        if !loading && let Body::Post { media, .. } = &stored.record.body {
            for item in media {
                let private = community.is_some_and(|c| self.is_private_community(&c));
                if private {
                    // Existing public bytes remain public. A public reference without bytes
                    // is not proof of disclosure and cannot cancel private provenance.
                    let previously_public = self.store.has_blob(&item.hash)
                        && !self.state.private_blobs.contains(&item.hash)
                        && self.store.media_users(&item.hash).any(|s| self.plaintext_forwardable(s));
                    if !previously_public && self.state.private_blobs.insert(item.hash) {
                        self.mark_state();
                    }
                } else if matches!(src, Source::Local) && self.store.has_blob(&item.hash) && self.state.private_blobs.remove(&item.hash) {
                    // The local publishing API takes and validates the actual attachment bytes.
                    self.mark_state();
                }
            }
        }
        self.store.insert(stored);
        if kind == Kind::Trust {
            self.ring_dirty = true;
        }
        if kind == Kind::Blocklist {
            self.lists_dirty = true;
        }
        if !loading {
            self.changes.push(Change::Put { id, community, expires_at, wire: bytes.to_vec() });
        }
        for old in superseded {
            self.drop_record(&old);
        }
        self.note_author(root, &record, now);
        // this device's own post in a community that keeps posts in pieces goes whole at once
        // when no pieces can be placed now (crate::open)
        if matches!(src, Source::Local)
            && matches!(kind, Kind::Post | Kind::Comment | Kind::Edit)
            && let Some(c) = community
            && self.open_scattered_at(c, record.created_at)
            && !self.can_scatter(c)
        {
            self.scatter.saved.whole.insert(id);
            self.scatter.dirty = true;
        }
        self.after_accept(&record, id, root, src);
        if kind == Kind::Community {
            self.complete_public_join(record.author);
        }
        if let Some(c) = community {
            self.dirty.insert(c);
            self.mark_changed(c);
        }
        // a home node learns whose devices paired with it from the identities and blessings they
        // hand it: their new records it injects as its own
        let from_client = matches!(src, Source::Peer { peer, .. } if self.peers.get(peer).is_some_and(|p| p.client));
        if from_client && matches!(kind, Kind::Identity | Kind::Delegation) && self.scatter.saved.client_roots.insert(root) {
            self.scatter.dirty = true;
        }
        // this device's own new record goes through an injector where one is linked (S7), and is
        // kept from its own links until it comes back from elsewhere
        if !loading && (matches!(src, Source::Local) || (from_client && self.own_root(&root))) {
            self.inject_own(id);
        }
        if !loading && !matches!(src, Source::PrivateRelay | Source::Pieces) {
            let except = match src {
                Source::Peer { peer, .. } => Some(peer.clone()),
                _ => None,
            };
            self.gossip(&id, except.as_deref());
        }
        Ok(Accepted { id, community, kind })
    }

    /// Ingests many records at once (sync, bundles): dependencies first, settling the ladder
    /// before contributions so their authors' new standing applies. `f` reports each result.
    pub(crate) fn ingest_batch(&mut self, wires: Vec<Vec<u8>>, src: &dyn Fn(u64) -> Source, f: BatchReport) {
        let mut items: Vec<(u8, u64, Vec<u8>)> = Vec::with_capacity(wires.len());
        // Which roots this batch carries community records of (device keys resolved through the
        // delegations in the batch or already known), so their carriers may come along.
        let mut devices: BTreeMap<Id, Id> = BTreeMap::new();
        let mut authors: Vec<Id> = Vec::new();
        for w in wires {
            let Ok((wire, _)) = Wire::decode(&w) else { continue };
            match &wire.record.body {
                Body::Delegation { root, .. } => {
                    devices.insert(wire.record.author, *root);
                }
                _ if wire.record.community.is_some() => authors.push(wire.record.author),
                _ => {}
            }
            items.push((priority(wire.record.kind()), wire.record.created_at, w));
        }
        self.batch_roots = Some(self.roots_of(&devices, authors));
        items.sort_by_key(|a| (a.0, a.1));
        let mut settled = false;
        for (p, created, w) in items {
            if p >= SETTLE_BEFORE && !settled {
                self.settle_dirty();
                settled = true;
            }
            let source = src(created);
            let res = self.ingest(&w, &source);
            let forged = matches!(res, Err(Reject::BadIdentity(_))) && matches!(source, Source::Peer { .. });
            f(self, &w, res);
            // Forged identity work drops the peer (crate::net): nothing more of its batch is
            // worth another Argon2id run.
            if forged {
                break;
            }
        }
        self.batch_roots = None;
        self.settle_dirty();
        // a device restoring from a recovery line takes over from whichever root the person's
        // chain reached (crate::recover)
        if self.state.me.as_ref().is_some_and(|m| m.restoring) {
            let _ = self.catch_up_rotation();
        }
        self.write_autobans();
    }

    /// Autobans that fired on this moderator's device, as `ban` ledger entries carrying the reports
    /// behind them: once for a person, at most 200 a community an hour. Reports count
    /// only from established and trusted members (or friends), which is the real safeguard; the
    /// cap bounds a device gone wrong without stopping it answering a flood of fresh keys.
    pub(crate) fn write_autobans(&mut self) {
        let due = std::mem::take(&mut self.autobans_due);
        let now = self.now();
        for (c, target) in due {
            let Some(st) = self.communities.get(&c) else { continue };
            if st.is_banned(&target) || st.is_owner(&target) || st.is_mod(&target) {
                continue;
            }
            let written = self.autobans_written.entry(c).or_default();
            written.retain(|t| *t + 3600 > now);
            if written.len() >= 200 {
                continue;
            }
            let evidence: Vec<Id> = self.store.of_kind(&c, Kind::Report).filter(|s| s.record.parent == Some(target)).map(|s| s.id).take(64).collect();
            let Ok(mut r) = self.ledger_record(&c, hoi_types::LedgerAction::Ban, Some(target), Some("Flagged by enough trusted members".into()), None, None)
            else {
                continue;
            };
            if let Body::Ledger(l) = &mut r.body {
                l.evidence = Some(evidence);
            }
            if self.publish(r).is_ok() {
                self.autobans_written.entry(c).or_default().push(now);
            }
        }
    }

    /// The roots behind a batch's community records, given the devices it blesses and its
    /// community records' authors: whose identities, devices and revocations may come along.
    pub(crate) fn roots_of(&self, devices: &BTreeMap<Id, Id>, authors: Vec<Id>) -> BTreeSet<Id> {
        authors.into_iter().map(|a| devices.get(&a).copied().unwrap_or_else(|| self.store.root_of(&a))).collect()
    }

    /// Whether this device has heard from `root` before: it holds records of theirs in a
    /// community it carries, or their identity, or one of their devices.
    pub(crate) fn root_known(&self, root: &Id) -> bool {
        let person = self.store.person_of(root);
        if person != *root {
            return self.root_known(&person);
        }
        self.store.identity(root).is_some()
            || !self.store.delegations_of(root).is_empty()
            || self.state.joined.keys().any(|c| self.store.by_author(c, root).next().is_some())
            // an owner or moderator of a community held here: their devices take its drop key
            // before they post anything (crate::open)
            || self.communities.values().any(|st| st.is_owner(root) || st.is_mod(root))
    }

    fn check_rate(&self, r: &Record, root: Id, c: Id) -> Result<(), Reject> {
        let Some(st) = self.communities.get(&c) else { return Ok(()) };
        if st.is_owner(&root) || st.is_mod(&root) {
            return Ok(());
        }
        let newcomer = !st.is_admitted(&root);
        let windows = hoi_validate::rate_windows(r.kind(), newcomer, limits_for(st, r.created_at).newcomer_posts_per_day, hoi_validate::has_media(r));
        if windows.is_empty() {
            return Ok(());
        }
        let times = self.store.author_times(&c, &root, hoi_validate::rate_group(r.kind()));
        for (w, max) in windows {
            if !hoi_validate::window_ok(&times, r.created_at, w, max) {
                return Err(Reject::RateLimited);
            }
        }
        Ok(())
    }

    /// The same words twice from one author within a week, or from three newcomers within a day.
    fn check_duplicate(&self, r: &Record, root: Id, c: Id) -> Result<(), Reject> {
        let Some(f) = hoi_validate::text_fingerprint(r) else { return Ok(()) };
        let same = self.store.same_text(&c, &f);
        if same.iter().any(|(a, t, _)| *a == root && t.abs_diff(r.created_at) < 7 * DAY) {
            return Err(Reject::Content("the same words again".into()));
        }
        if let Some(st) = self.communities.get(&c)
            && !st.is_admitted(&root)
        {
            let others: BTreeSet<Id> =
                same.iter().filter(|(a, t, _)| *a != root && t.abs_diff(r.created_at) < DAY && !st.is_admitted(a)).map(|x| x.0).collect();
            if others.len() >= 2 {
                return Err(Reject::Content("the same words from several newcomers: a copy-paste flood".into()));
            }
        }
        Ok(())
    }

    /// Since when the rules behind an offence have been in effect: the protocol's, and for rate
    /// windows the community's current limits too.
    fn strikes_from(&self, c: &Id, o: Offence) -> u64 {
        let limits = match o {
            Offence::Rate => self.communities.get(c).and_then(|st| st.limits_since.last()).map_or(0, |(at, _)| *at),
            Offence::Content | Offence::Duplicate => 0,
        };
        self.rules_since.max(limits)
    }

    /// Records a strike for record `id`, made at `made`; past the line the key is bounced on this
    /// device and a signed report goes out. A record made before the rules it broke took effect
    /// is refused and nothing more: its author's client may have judged it by the rules of its
    /// day, and a device that reads the rules differently must not bounce honest people.
    fn offence(&mut self, root: Id, c: Id, (id, made): (Id, u64), o: Offence, now: u64) {
        if made < self.strikes_from(&c, o) {
            return;
        }
        let ev = self.evidence.entry((root, c)).or_default();
        if ev.len() < 16 {
            ev.push(id);
        }
        if self.strikes.add(root, c, o, now) {
            let evidence = self.evidence.remove(&(root, c)).unwrap_or_default();
            self.bounce(root, c, "too many bad records", now);
            let reason = match o {
                Offence::Rate => "rate",
                Offence::Duplicate => "dup",
                Offence::Content => "spam",
            };
            let _ = self.publish_report(c, root, reason, evidence);
        }
    }

    /// Bounces `root` on this device for a while (PROTOCOL.md §9.3): a day the first time, twice
    /// as long each time again while the last bounce is remembered, never more than a month.
    pub(crate) fn bounce(&mut self, root: Id, c: Id, reason: &str, now: u64) {
        if Some(root) == self.me_root() || self.state.bounced.get(&root).is_some_and(|b| b.active(now)) {
            return;
        }
        let name = self.display_name(&root);
        let times = self.state.bounced.get(&root).map_or(0, |b| b.times.max(1)).saturating_add(1);
        let lasts = hoi_spam::bounce_lasts(times);
        self.state.bounced.insert(root, Bounce { community: c, at: now, reason: reason.into(), until: now.saturating_add(lasts), times });
        self.mark_state();
        self.mark_changed(c);
        let days = lasts.div_ceil(DAY);
        let span = if days == 1 { "a day".to_string() } else { format!("{days} days") };
        self.notice(format!("Bounced {name} for {span}: {reason}. Until then your device won't store or pass on anything it signs."));
    }

    /// The bounce `root` is under on this device, unless it has lifted.
    pub(crate) fn bounced(&self, root: &Id) -> Option<&Bounce> {
        let now = self.now();
        self.state.bounced.get(root).filter(|b| b.active(now))
    }

    /// Checks that need other records: parents exist in the same community, edits and deletes
    /// come from the author, reviews judge what they may judge, ledger entries come from
    /// someone with standing, labels from labelers someone listens to.
    fn semantic(&self, r: &Record, _id: &Id, root: Id, src: &Source) -> Result<(), Reject> {
        let parent = r.parent.and_then(|p| self.store.get(&p));
        // what a parent is, from the record or, in an open community that keeps posts in pieces,
        // from a pointer record that holds it (crate::open)
        let standin = r.parent.filter(|_| parent.is_none()).and_then(|p| self.scatter.standins.get(&p).copied());
        let known: Option<(Kind, Option<Id>, Id)> =
            parent.map(|p| (p.record.kind(), p.record.community, p.author_root)).or(standin.map(|s| (s.kind, Some(s.community), s.root)));
        let same_community = |p: &Stored| p.record.community == r.community;
        match &r.body {
            Body::Profile { image, .. } if self.avatar_blocked(image) => {
                return Err(Reject::NotAllowed("profile picture is blocked on this device".into()));
            }
            Body::Comment { .. } => {
                // A reply under something that was removed since stays, under the Poof box.
                let (kind, community) = match (known, r.parent.and_then(|p| self.state.skeletons.get(&p))) {
                    (Some((k, c, _)), _) => (k.as_str().to_string(), c),
                    (None, Some(sk)) => (sk.kind.clone(), Some(sk.community)),
                    _ => return Err(Reject::NotAllowed("reply to something we don't have".into())),
                };
                if !matches!(kind.as_str(), "post" | "comment") || community != r.community {
                    return Err(Reject::NotAllowed("replies go under posts and comments of the same community".into()));
                }
            }
            Body::Vote { .. } => {
                let (kind, community, _) = known.ok_or_else(|| Reject::NotAllowed("vote on something we don't have".into()))?;
                if !matches!(kind, Kind::Post | Kind::Comment) || community != r.community {
                    return Err(Reject::NotAllowed("only posts and comments of the same community take votes".into()));
                }
            }
            Body::Edit { .. } | Body::Delete => {
                let (kind, community, author) = known.ok_or_else(|| Reject::NotAllowed("change to something we don't have".into()))?;
                if author != root || !matches!(kind, Kind::Post | Kind::Comment) || community != r.community {
                    return Err(Reject::NotAllowed("only the author can edit or delete".into()));
                }
            }
            Body::Vouch { .. } => {
                if r.parent == Some(root) {
                    return Err(Reject::NotAllowed("nobody vouches for themselves".into()));
                }
            }
            Body::Review { verdict, .. } if parent.is_none() && standin.is_some() => {
                // a review of a post or comment held by a pointer record
                let s = standin.expect("checked");
                if Some(s.community) != r.community {
                    return Err(Reject::NotAllowed("reviews stay in their community".into()));
                }
                if s.root == root {
                    return Err(Reject::NotAllowed("nobody reviews their own words".into()));
                }
                if *verdict == Verdict::Veto {
                    return Err(Reject::NotAllowed("vetoes are for ledger entries".into()));
                }
            }
            Body::Review { verdict, .. } => {
                let p = parent.ok_or_else(|| Reject::NotAllowed("review of something we don't have".into()))?;
                if !same_community(p) {
                    return Err(Reject::NotAllowed("reviews stay in their community".into()));
                }
                match &p.record.body {
                    Body::Post { .. } | Body::Comment { .. } => {
                        if p.author_root == root {
                            return Err(Reject::NotAllowed("nobody reviews their own words".into()));
                        }
                        if *verdict == Verdict::Veto {
                            return Err(Reject::NotAllowed("vetoes are for ledger entries".into()));
                        }
                    }
                    Body::Ledger(l) => {
                        let proposal = self.proposal_of(r);
                        if !l.action.vetoable() && proposal.is_none() {
                            return Err(Reject::NotAllowed("that ledger entry can't be vetoed".into()));
                        }
                        if p.author_root == root || l.cosign.iter().flatten().any(|c| self.store.root_of(&c.key) == root) {
                            return Err(Reject::NotAllowed("nobody vetoes their own action".into()));
                        }
                        if *verdict == Verdict::Hide {
                            return Err(Reject::NotAllowed("ledger entries are vetoed, not hidden".into()));
                        }
                        if proposal.is_some_and(|x| r.created_at >= x.deadline) {
                            return Err(Reject::NotAllowed("answers to that proposal closed".into()));
                        }
                    }
                    _ => return Err(Reject::NotAllowed("only posts, comments and ledger entries are reviewed".into())),
                }
            }
            Body::Ledger(l) => {
                let c = r.community.expect("shape");
                let st = self.communities.get(&c).ok_or(Reject::UnknownCommunity)?;
                let standing = if l.action.needs_threshold() { st.is_owner(&root) } else { st.is_owner(&root) || st.is_mod(&root) };
                if !standing && !matches!(src, Source::Load) {
                    return Err(Reject::NotAllowed("only owners and moderators write the ledger".into()));
                }
            }
            Body::Wiki { .. } => self.check_wiki(r, root, src)?,
            // an aggregator's counts come from aggregators someone here follows
            Body::Tally { .. } => {
                if !matches!(src, Source::Load) && Some(root) != self.me_root() && !self.state.aggregators.contains(&root) {
                    return Err(Reject::NotAllowed("counts from an aggregator nobody here follows".into()));
                }
            }
            // a list keeper's list from keepers someone here follows
            Body::Blocklist { .. } => {
                if !matches!(src, Source::Load) && Some(root) != self.me_root() && !self.state.blocklists.contains(&root) {
                    return Err(Reject::NotAllowed("a blocklist from a keeper nobody here follows".into()));
                }
            }
            Body::Label { .. } => {
                let allowed = Some(root) == self.me_root()
                    || self.state.labelers.contains(&root)
                    || r.community.and_then(|c| self.communities.get(&c)).is_some_and(|st| st.limits.labelers.contains(&root));
                if !allowed {
                    return Err(Reject::NotAllowed("labels from a labeler nobody here follows".into()));
                }
            }
            // only an established member's roster counts, so only theirs is carried
            Body::Roster { .. } => {
                let c = r.community.expect("shape");
                let st = self.communities.get(&c).ok_or(Reject::UnknownCommunity)?;
                if !matches!(src, Source::Load) && (st.is_banned(&root) || st.level(&root) < hoi_moderation::Level::Established) {
                    return Err(Reject::NotAllowed("rosters come from established members' devices".into()));
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Vouches and reviews: only the latest per (author, target) is kept. Returns the older
    /// ones to drop, or `Duplicate` when this record is itself the older one.
    fn superseded(&self, r: &Record, id: &Id, root: Id) -> Result<Vec<Id>, Reject> {
        // a person's latest trust statement, or tally, in a community replaces the earlier ones
        if let (Body::Trust { .. } | Body::Tally { .. } | Body::Blocklist { .. }, Some(c)) = (&r.body, r.community) {
            let mine = (r.created_at, *id);
            let kind = r.kind();
            let mut older = Vec::new();
            for s in self.store.by_author(&c, &root).filter(|s| s.record.kind() == kind) {
                if (s.record.created_at, s.id) > mine {
                    return Err(Reject::Duplicate);
                }
                older.push(s.id);
            }
            return Ok(older);
        }
        // a device's latest roster replaces its earlier ones
        if let (Body::Roster { .. }, Some(c)) = (&r.body, r.community) {
            let mine = (r.created_at, *id);
            let mut older = Vec::new();
            for s in self.store.by_author(&c, &root).filter(|s| s.record.kind() == Kind::Roster && s.record.author == r.author) {
                if (s.record.created_at, s.id) > mine {
                    return Err(Reject::Duplicate);
                }
                older.push(s.id);
            }
            return Ok(older);
        }
        let seq = match &r.body {
            Body::Vouch { seq, .. } | Body::Review { seq, .. } => *seq,
            _ => return Ok(Vec::new()),
        };
        let (Some(c), Some(target)) = (r.community, r.parent) else { return Ok(Vec::new()) };
        if matches!(r.body, Body::Review { .. }) && self.proposal_of(r).is_some() {
            // every answer to a proposal is kept (when people agreed decides it), up to a few each
            let earlier = self
                .store
                .by_author(&c, &root)
                .filter(|s| s.record.kind() == Kind::Review && s.record.parent == Some(target) && (s.record.created_at, s.id) < (r.created_at, *id))
                .count();
            if earlier >= hoi_moderation::ladder::MAX_ANSWERS {
                return Err(Reject::NotAllowed("enough answers to that proposal from this person".into()));
            }
            return Ok(Vec::new());
        }
        let mine = (seq, r.created_at, *id);
        let mut older = Vec::new();
        for s in self.store.by_author(&c, &root) {
            if s.record.kind() != r.kind() || s.record.parent != Some(target) {
                continue;
            }
            let theirs = match &s.record.body {
                Body::Vouch { seq, .. } | Body::Review { seq, .. } => (*seq, s.record.created_at, s.id),
                _ => continue,
            };
            if theirs > mine {
                return Err(Reject::Duplicate);
            }
            older.push(s.id);
        }
        Ok(older)
    }

    /// Deletes a record for good, without leaving a Poof box (superseded, burned).
    pub(crate) fn drop_record(&mut self, id: &Id) {
        if let Some(s) = self.store.remove(id) {
            self.store.mark_deleted(*id, s.record.expires_at);
            self.changes.push(Change::Delete { id: *id });
            if let Some(c) = s.record.community {
                self.dirty.insert(c);
            }
        }
    }

    fn note_author(&mut self, root: Id, r: &Record, now: u64) {
        // Key age counts from when this device first saw the key, not from what the record
        // claims, so nobody can backdate their way to a discount.
        self.state.first_seen.entry(root).or_insert(now);
        self.remember_profile_head(root, r);
        if r.kind() == Kind::Profile && r.community.is_some_and(|c| !self.is_private_community(&c)) {
            self.remember_public_avatar(root);
        }
        if r.kind() == Kind::ProfileColor && r.community.is_some_and(|c| !self.is_private_community(&c)) {
            self.remember_public_avatar_color(root);
        }
        let name = match &r.body {
            Body::Post { name: Some(n), .. } | Body::Comment { name: Some(n), .. } => Some(n.clone()),
            _ => None,
        };
        if let Some(n) = name {
            let newer = self.state.names.get(&root).is_none_or(|(_, at)| *at <= r.created_at);
            if newer {
                if !self.loading {
                    self.want_handle_check(&n);
                }
                self.state.names.insert(root, (n, r.created_at));
            }
        }
        if !self.loading {
            self.mark_state();
        }
    }

    fn after_accept(&mut self, r: &Record, _id: Id, root: Id, src: &Source) {
        match &r.body {
            Body::Pointer { .. } => self.on_pointer_record(r, root),
            Body::Community(g) => {
                let c = r.author;
                self.communities.insert(c, Community::new(c, _id, g.clone(), r.created_at));
                self.dirty.insert(c);
                if !self.loading {
                    self.rebuild(&c);
                }
            }
            Body::Delete => {
                if let Some(p) = r.parent {
                    self.skeletonize(&p, None, "deleted by the author".into(), r.created_at, None);
                }
            }
            Body::Ledger(l) if matches!(l.action, hoi_types::LedgerAction::Rules | hoi_types::LedgerAction::Limits) && matches!(src, Source::Peer { .. }) => {
                // a change this person gets a say in: tell them, once, as it arrives
                if let (Some(me), Some(c)) = (self.me_root(), r.community)
                    && root != me
                    && let Some(st) = self.communities.get(&c)
                    && st.level(&me) >= hoi_moderation::Level::Established
                {
                    let what = if l.action == hoi_types::LedgerAction::Rules { "new rules" } else { "new limits" };
                    let text = format!(
                        "c/{}: {} proposes {what}. They apply only if everyone established agrees. Have your say on the Rules tab.",
                        st.genesis.name,
                        self.display_name(&root)
                    );
                    self.notice(text);
                }
            }
            Body::Report { .. } => {
                if !matches!(src, Source::Load)
                    && let (Some(c), Some(target)) = (r.community, r.parent)
                {
                    self.check_autoban(c, target);
                }
            }
            Body::Revoke { device } if *device == r.author => self.purge_burned(root),
            _ => {}
        }
    }

    /// A burned identity takes everything it signed with it, on every device.
    fn purge_burned(&mut self, root: Id) {
        let ids: Vec<Id> =
            self.store.all().filter(|s| s.author_root == root && !matches!(&s.record.body, Body::Revoke { device } if *device == root)).map(|s| s.id).collect();
        for id in ids {
            self.drop_record(&id);
        }
        if let Some(me) = self.state.me.as_mut()
            && me.root == root
        {
            me.burned = true;
        }
        self.mark_state();
    }

    /// Settles every community whose records changed since, or that wasn't settled for a
    /// minute (people become established, contributions leave the window, removals ripen).
    pub(crate) fn settle_dirty(&mut self) {
        if self.loading {
            return;
        }
        if self.ring_dirty {
            self.refresh_ring();
        }
        if self.lists_dirty {
            self.refresh_lists();
        }
        let now = self.now();
        for c in self.communities.keys() {
            if self.settled_at.get(c).is_none_or(|t| t + 60 <= now) {
                self.dirty.insert(*c);
            }
        }
        let cs: Vec<Id> = std::mem::take(&mut self.dirty).into_iter().collect();
        let me = self.me_root();
        for c in cs {
            // what this person wrote that waited for their standing (an age proof, a vouch)
            // goes out as soon as it may: the next sync could be hours away
            let held: Vec<Id> = match (me, self.communities.get(&c)) {
                (Some(m), Some(_)) => self.store.by_author(&c, &m).filter(|s| self.held(s).is_some()).map(|s| s.id).collect(),
                _ => Vec::new(),
            };
            self.rebuild(&c);
            for id in held {
                if self.store.get(&id).is_some_and(|s| self.held(s).is_none()) {
                    self.gossip(&id, None);
                }
            }
        }
    }

    /// What the ladder reads for a community (PROTOCOL.md §15).
    /// The latest effective time of anything `root` signed in community `c`.
    pub(crate) fn last_seen_in(&self, c: &Id, root: &Id) -> Option<u64> {
        self.store.in_community(c).filter(|s| s.author_root == *root).map(|s| self.effective_at(s)).max()
    }

    /// Whether owners may retire `root` from community `c` now: not an owner, and
    /// silent here, and a member, for at least [`hoi_moderation::ladder::RETIRE_AWAY`].
    pub(crate) fn retirable(&self, st: &Community, root: &Id) -> bool {
        let admitted = st.standing.person(root).and_then(|p| p.admitted_at);
        let since = self.last_seen_in(&st.key, root).into_iter().chain(admitted).max().unwrap_or(0);
        !st.is_owner(root) && since.saturating_add(hoi_moderation::ladder::RETIRE_AWAY) <= self.now()
    }

    fn ladder_inputs(&self, c: &Id, now: u64) -> Inputs {
        let mut inp = Inputs { now, ..Default::default() };
        let mut votes: BTreeMap<(Id, Id), (u64, i64)> = BTreeMap::new();
        for s in self.store.in_community(c) {
            let r = &s.record;
            let at = self.effective_at(s);
            let seen = inp.last_seen.entry(s.author_root).or_insert(at);
            *seen = (*seen).max(at);
            match &r.body {
                Body::Post { .. } | Body::Comment { .. } => {
                    inp.contributions.push(Contribution { id: s.id, author: s.author_root, created_at: r.created_at });
                    inp.authors.insert(s.id, s.author_root);
                }
                Body::Vouch { value, seq, .. } => {
                    if let Some(subject) = r.parent {
                        inp.vouches.push(VouchIn { id: s.id, voucher: s.author_root, subject, value: *value, seq: *seq, created_at: self.effective_at(s) });
                    }
                }
                Body::Review { verdict, seq, reason } => {
                    if let Some(target) = r.parent {
                        inp.reviews.push(ReviewIn {
                            id: s.id,
                            reviewer: s.author_root,
                            target,
                            verdict: *verdict,
                            reason: reason.clone(),
                            seq: *seq,
                            created_at: self.effective_at(s),
                        });
                    }
                }
                Body::Vote { value, seq } => {
                    if let Some(target) = r.parent {
                        let e = votes.entry((s.author_root, target)).or_insert((0, 0));
                        if *seq >= e.0 {
                            *e = (*seq, *value);
                        }
                    }
                }
                _ => {}
            }
        }
        inp.votes = votes.into_iter().map(|((voter, target), (_, value))| VoteIn { voter, target, value }).collect();
        for (id, sk) in self.state.skeletons.iter().filter(|(_, sk)| sk.community == *c) {
            inp.authors.insert(*id, sk.author);
        }
        inp.burned = self.store.burned().copied().collect();
        inp
    }

    /// Replays a community's ledger, settles its ladder, and applies the outcome: removals
    /// whose contest window passed lose their bodies, overturned removals let records back in,
    /// media the community hid as illegal is deleted here and never fetched again.
    pub(crate) fn rebuild(&mut self, c: &Id) {
        let now = self.now();
        let entries: Vec<hoi_moderation::Entry> = self
            .store
            .of_kind(c, Kind::Ledger)
            .filter_map(|s| match &s.record.body {
                Body::Ledger(l) => Some(hoi_moderation::Entry {
                    id: s.id,
                    created_at: s.record.created_at,
                    at: self.effective_at(s),
                    expires_at: s.record.expires_at,
                    signer: s.record.author,
                    signer_root: s.author_root,
                    body: l.clone(),
                    proposal: s.record.proposal_id(),
                }),
                _ => None,
            })
            .collect();
        let inputs = self.ladder_inputs(c, now);
        let store = &self.store;
        let root_of = |k: &Id, at: u64| store.root_of_at(k, at);
        let Some(st) = self.communities.get_mut(c) else { return };
        st.settle(&entries, &root_of, &inputs);
        self.dirty.remove(c);
        self.settled_at.insert(*c, now);

        let st = &self.communities[c];
        let effective: Vec<(Id, hoi_moderation::Removal)> =
            st.tombstones.iter().filter(|(t, rm)| rm.effective_at <= now && self.store.contains(t)).map(|(k, v)| (*k, v.clone())).collect();
        let back: Vec<Id> = self
            .state
            .skeletons
            .iter()
            .filter(|(id, sk)| sk.community == *c && sk.ledger.is_some() && st.tombstones.get(*id).is_none_or(|rm| Some(rm.entry) != sk.ledger))
            .map(|(id, _)| *id)
            .collect();
        let illegal: Vec<Id> = st
            .standing
            .hidden
            .iter()
            .filter(|(_, h)| h.reason == "illegal")
            .map(|(id, _)| *id)
            .chain(
                st.tombstones
                    .iter()
                    .filter(|(_, rm)| hoi_moderation::removal_is_urgent(&rm.reason) && rm.reason.to_ascii_lowercase().starts_with("illegal"))
                    .map(|(id, _)| *id),
            )
            .collect();
        // media first: once the body is gone there's nothing left to blocklist
        for id in illegal {
            self.forget_media_of(&id);
        }
        for (target, rm) in effective {
            self.skeletonize(&target, Some(rm.by), rm.reason, rm.at, Some(rm.entry));
        }
        for id in back {
            self.state.skeletons.remove(&id);
            self.store.forget_deleted(&id);
            self.mark_state();
        }
        self.mark_changed(*c);
        self.broadcast_catalogs();
    }

    /// Whether a picture, any way round, is close to one this device refuses.
    pub(crate) fn similar_blocked(&self, v: &hoi_validate::similar::Variants) -> bool {
        self.state.similar_blocklist.blocks(v) || self.lists_pdq.blocks(v)
    }

    /// Whether this device refuses media by its exact hash: its own list, and the lists of the
    /// keepers this person follows.
    pub(crate) fn media_refused(&self, h: &Id) -> bool {
        self.state.media_blocklist.contains(h) || self.lists_exact.contains(h)
    }

    /// Deletes a post's media bytes and refuses them from now on, near copies included.
    pub(crate) fn forget_media_of(&mut self, id: &Id) {
        let hashes: Vec<(Id, String)> = match self.store.get(id).map(|s| &s.record.body) {
            Some(Body::Post { media, .. }) => media.iter().map(|m| (m.hash, m.mime.clone())).collect(),
            _ => return,
        };
        for (h, mime) in hashes {
            if self.state.media_blocklist.insert(h) {
                self.mark_state();
            }
            // remember what the picture looks like, so altered copies are refused too
            if crate::look::wants_look(&mime)
                && let Some(data) = self.store.blob(&h).map(|b| b.to_vec())
                && let Some(look) = self.look_at(h, &mime, Some(data), crate::look::Why::Remember)
            {
                self.remember_look(&look);
            }
            if self.store.remove_blob(&h) {
                self.changes.push(Change::Blob { key: media_key(&h), value: None });
            }
        }
        self.purge_blocked_profiles();
    }

    /// Deletes a record's body for good, keeping where it was and why (the Poof box).
    pub(crate) fn skeletonize(&mut self, id: &Id, by: Option<Id>, reason: String, at: u64, ledger: Option<Id>) {
        let Some(s) = self.store.remove(id) else { return };
        let r = &s.record;
        if let Some(c) = r.community {
            self.state.skeletons.insert(
                *id,
                Skeleton {
                    community: c,
                    parent: r.parent,
                    author: s.author_root,
                    kind: r.kind().as_str().into(),
                    created_at: r.created_at,
                    expires_at: r.expires_at,
                    by,
                    reason,
                    at,
                    ledger,
                },
            );
            self.mark_changed(c);
            self.dirty.insert(c);
        }
        self.store.mark_deleted(*id, r.expires_at);
        self.state.saved.remove(id);
        self.changes.push(Change::Delete { id: *id });
        self.mark_state();
    }

    /// Weighted reports against `target` in `c` (PROTOCOL.md §9). A reporter counts by the
    /// larger of their weight in your trust ring and their standing in the community.
    pub(crate) fn report_weight(&self, c: &Id, target: &Id) -> f64 {
        let st = self.communities.get(c);
        let mut by: BTreeMap<Id, f64> = BTreeMap::new();
        for s in self.store.of_kind(c, Kind::Report) {
            if s.record.parent == Some(*target) {
                let local = self.trust_level(&s.author_root).weight();
                let ladder = st.map_or(0.0, |st| match st.level(&s.author_root) {
                    hoi_moderation::Level::Trusted => 1.0,
                    hoi_moderation::Level::Established => 0.5,
                    _ => 0.0,
                });
                by.insert(s.author_root, local.max(ladder));
            }
        }
        by.values().sum()
    }

    fn check_autoban(&mut self, c: Id, target: Id) {
        let Some(st) = self.communities.get(&c) else { return };
        if st.is_owner(&target) || st.is_mod(&target) {
            return;
        }
        let threshold = st.limits.autoban_threshold as f64;
        let moderates = self.me_root().is_some_and(|me| st.is_owner(&me) || st.is_mod(&me));
        if self.report_weight(&c, &target) >= threshold {
            let now = self.now();
            self.bounce(target, c, "enough trusted members flagged it", now);
            // on an owner's or moderator's device the ban goes to the ledger, for everyone
            if moderates && !self.autobans_due.contains(&(c, target)) {
                self.autobans_due.push((c, target));
            }
        }
    }

    pub(crate) fn trust_level(&self, root: &Id) -> hoi_trust::Level {
        let now = self.now();
        hoi_trust::Level::of(Some(*root) == self.me_root(), self.state.friends.contains_key(root), self.fof.contains_key(root), self.key_age(root, now))
    }

    /// Works out the ring again (PROTOCOL §8): degree 1 is the people marked as friends
    /// here; degree 2 the people their latest trust statements name, in any community this
    /// device holds, besides this person and their friends.
    pub(crate) fn refresh_ring(&mut self) {
        self.ring_dirty = false;
        let me = self.me_root();
        let mut fof: BTreeMap<Id, BTreeSet<Id>> = BTreeMap::new();
        for friend in self.state.friends.keys() {
            for c in self.communities.keys() {
                let latest =
                    self.store.by_author(c, friend).filter(|s| s.record.kind() == Kind::Trust && self.usable(s)).max_by_key(|s| (s.record.created_at, s.id));
                let Some(Body::Trust { keys }) = latest.map(|s| &s.record.body) else { continue };
                for k in keys {
                    if Some(*k) != me && !self.state.friends.contains_key(k) {
                        fof.entry(*k).or_default().insert(*friend);
                    }
                }
            }
        }
        if fof != self.fof {
            self.fof = fof;
            self.changed_everywhere();
        }
    }

    pub(crate) fn display_name(&self, root: &Id) -> String {
        self.state.petnames.get(root).cloned().or_else(|| self.state.names.get(root).map(|(n, _)| n.clone())).unwrap_or_else(|| root.short_key())
    }

    // ------------------------------------------------------------------ what's shown and passed on

    /// Why a record is held back: shown to nobody but its author, passed on to nobody, until
    /// its author's standing allows it (PROTOCOL.md §15.3). `None` when it may go out.
    pub(crate) fn held(&self, s: &Stored) -> Option<&'static str> {
        let c = s.record.community?;
        let st = self.communities.get(&c)?;
        let root = &s.author_root;
        if !self.profile_is_current(s) {
            return Some("a newer profile is known");
        }
        if let Body::Post { media, .. } = &s.record.body
            && media.iter().any(|m| self.media_refused(&m.hash) || self.state.bad_media.contains(&m.hash))
        {
            return Some("its media failed the checks");
        }
        if let Body::Profile { image, .. } = &s.record.body
            && self.avatar_blocked(image)
        {
            return Some("its profile picture is blocked");
        }
        if st.is_owner(root) || st.is_mod(root) || !matches!(s.record.kind(), Kind::Profile | Kind::ProfileColor | Kind::Post | Kind::Comment | Kind::Edit) {
            return None;
        }
        let level = st.level(root);
        if st.limits.admission == hoi_types::Admission::Invite && level == hoi_moderation::Level::Newcomer && st.standing.counted_vouches(root) == 0 {
            return Some("this community needs a member to vouch for you first");
        }
        if st.limits.admission == hoi_types::Admission::AgeProof && !st.is_admitted(root) {
            return Some("this community lets people in once a moderator saw a proof of their age");
        }
        if level < hoi_moderation::Level::Member && hoi_validate::has_links(&s.record) {
            return Some("links unlock once the community has vouched for you");
        }
        if level < hoi_moderation::Level::Trusted && hoi_validate::has_media(&s.record) {
            return Some("media unlocks for trusted members");
        }
        None
    }

    /// Whether this device passes a record on: not held, and not by someone the community blocked.
    pub(crate) fn forwardable(&self, s: &Stored) -> bool {
        self.usable(s) && !self.kept_in_pieces(s) && !self.scatter.saved.injecting.contains_key(&s.id)
    }

    /// Whether a record counts here: not held back, its author not blocked. A record kept in
    /// pieces counts, but is never offered whole ([`Node::forwardable`]).
    pub(crate) fn usable(&self, s: &Stored) -> bool {
        self.held(s).is_none() && !s.record.community.and_then(|c| self.communities.get(&c)).is_some_and(|st| st.standing.blocked(&s.author_root))
    }

    /// Private markers remain authoritative even while group keys are unavailable.
    pub(crate) fn is_private_community(&self, community: &Id) -> bool {
        self.state.private_communities.contains(community) || self.private.contains_group(community)
    }

    /// Raw sync never carries private content; encrypted transport still uses `forwardable`.
    pub(crate) fn plaintext_forwardable(&self, s: &Stored) -> bool {
        !self.state.private_carriers.contains(&s.id) && !s.record.community.is_some_and(|c| self.is_private_community(&c)) && self.forwardable(s)
    }

    /// The records a peer needs before this one: the author's identity and device blessing.
    pub(crate) fn carriers(&self, s: &Stored) -> Vec<&Stored> {
        let mut out = Vec::new();
        if let Some(i) = self.store.identity(&s.author_root)
            && i.record != s.id
            && let Some(is) = self.store.get(&i.record)
        {
            out.push(is);
        }
        // a person whose root was replaced: each later root's identity and each rotation,
        // before the delegation they bless
        for id in self.store.chain_records(&s.author_root) {
            if id != s.id
                && let Some(x) = self.store.get(&id)
            {
                out.push(x);
            }
        }
        if s.record.author != s.author_root
            && let Some(d) = self.store.delegation(&s.record.author)
            && d.record != s.id
            && let Some(ds) = self.store.get(&d.record)
        {
            out.push(ds);
        }
        out
    }

    // ------------------------------------------------------------------ expiry

    /// Deletes everything past its expiry (TECHNICAL.md §8), lets contested removals take
    /// effect once their window passed, frees unused media. Hosts call this every few minutes.
    pub fn sweep(&mut self) -> u64 {
        // communities nobody here serves: ask the public nodes that answer `find` (crate::find)
        self.find_missing();
        self.sweep_ranging();
        // browsers whose shared communities changed hear of each other (crate::signal)
        self.refresh_here();
        self.offer_direct();
        // statements that expired or were removed leave the ring
        self.ring_dirty = true;
        if self.state.me.as_ref().is_some_and(|m| m.restoring) {
            let _ = self.catch_up_rotation();
        }
        let now = self.now();
        let expired = self.store.expired(now);
        self.sweep_profile_heads();
        for id in &expired {
            if let Some(s) = self.store.remove(id) {
                if let Some(c) = s.record.community {
                    if s.record.kind() == Kind::Community {
                        self.communities.remove(&c);
                        self.dirty.remove(&c);
                    } else {
                        self.dirty.insert(c);
                    }
                    self.mark_changed(c);
                }
                self.state.saved.remove(id);
                self.changes.push(Change::Delete { id: *id });
            }
        }
        // removals whose contest window just passed
        for (c, st) in &self.communities {
            if st.tombstones.values().any(|rm| rm.effective_at <= now) {
                self.dirty.insert(*c);
            }
        }
        if !self.loading {
            self.sweep_carried();
            self.gossip_directory();
            self.scatter_sweep();
        }
        self.settle_dirty();
        let before = self.state.skeletons.len();
        self.state.skeletons.retain(|_, s| s.expires_at > now);
        self.state.reads.retain(|r| r.first + 30 * DAY > now);
        let late = self.state.late.len();
        let store = &self.store;
        self.state.late.retain(|id, _| store.contains(id));
        if before != self.state.skeletons.len() || !expired.is_empty() || late != self.state.late.len() {
            self.mark_state();
        }
        for h in self.store.orphan_blobs() {
            self.store.remove_blob(&h);
            self.changes.push(Change::Blob { key: media_key(&h), value: None });
        }
        self.media_wanted.retain(|_, t| *t + 3600 > now);
        self.store.forget_deleted_before(now);
        self.strikes.forget_quiet(now);
        // A lifted bounce is remembered for a month, so a repeat lasts longer; then forgotten.
        let bounced = self.state.bounced.len();
        self.state.bounced.retain(|_, b| b.lifts_at().saturating_add(hoi_spam::BOUNCE_MAX) > now);
        if bounced != self.state.bounced.len() {
            self.mark_state();
        }
        self.drafts.retain(|_, d| d.made + 3600 > now);
        if !self.loading {
            self.auto_vouch();
            self.renew_trust();
            self.renew_tallies();
            // names that are domains, checked again now and then
            let names: Vec<String> = self.state.names.values().map(|(n, _)| n.clone()).collect();
            for n in names {
                self.want_handle_check(&n);
            }
            self.request_missing_media();
        }
        expired.len() as u64
    }

    // ------------------------------------------------------------------ signing

    /// Picks the key that must sign a record of this kind.
    fn signer_for(&self, r: &Record) -> Result<&Keypair, ApiError> {
        match r.kind() {
            Kind::Community => self.community_kps.get(&r.author).ok_or_else(|| ApiError::not_allowed("This device doesn't hold that community's key.")),
            Kind::Identity | Kind::Revoke => self.root_kp.as_ref().ok_or_else(|| ApiError::not_allowed("Only the device with your root key can do that.")),
            // a delegation is signed by the device it blesses (its consent)
            Kind::Delegation => [self.device_kp.as_ref(), self.pending_kp.as_ref()]
                .into_iter()
                .flatten()
                .find(|k| k.public() == r.author)
                .ok_or_else(|| ApiError::not_allowed("Only the device itself can accept a blessing.")),
            _ => self.device_kp.as_ref().ok_or_else(ApiError::no_identity),
        }
    }

    /// Bits an author stamps for: the community's full difficulty for the kind (no age discount).
    pub(crate) fn bits_to_stamp(&self, r: &Record) -> u32 {
        self.bits_for(r, 0)
    }

    /// Mines, signs and ingests a record made here.
    pub(crate) fn publish(&mut self, mut r: Record) -> Result<Id, ApiError> {
        let bits = self.bits_to_stamp(&r);
        r.nonce = mine(&r, bits);
        self.publish_mined(r)
    }

    /// [`Node::publish`] signed by `kp` rather than the key the kind calls for: the first root
    /// made again from a recovery line, and a rotation under a recovery key (crate::recover).
    pub(crate) fn publish_as(&mut self, mut r: Record, kp: &Keypair) -> Result<Id, ApiError> {
        if r.author != kp.public() {
            return Err(ApiError::new("internal", "author does not match the signing key"));
        }
        let bits = self.bits_to_stamp(&r);
        r.nonce = mine(&r, bits);
        let (wire, _) = kp.sign_record(r);
        let acc = self.ingest(&wire.encode(), &Source::Local)?;
        self.settle_dirty();
        Ok(acc.id)
    }

    pub(crate) fn publish_mined(&mut self, r: Record) -> Result<Id, ApiError> {
        // a browser whose computer keeps its key asks there (crate::remote)
        if self.signs_remotely() && !matches!(r.kind(), Kind::Community | Kind::Identity | Kind::Revoke | Kind::Rotate) {
            return self.publish_remote(r);
        }
        let kp = self.signer_for(&r)?.clone();
        if r.author != kp.public() {
            return Err(ApiError::new("internal", "author does not match the signing key"));
        }
        let (wire, _) = kp.sign_record(r);
        let acc = self.ingest(&wire.encode(), &Source::Local)?;
        self.settle_dirty();
        if matches!(acc.kind, Kind::Post | Kind::Comment)
            && let Some(c) = acc.community
        {
            if let Err(e) = self.publish_profile(c) {
                self.notice(format!("Your contribution was sent; the profile picture could not be shared yet: {}", e.message));
            }
            if let Err(e) = self.publish_profile_color(c) {
                self.notice(format!("Your contribution was sent; the identicon color could not be shared yet: {}", e.message));
            }
        }
        Ok(acc.id)
    }

    pub(crate) fn publish_report(&mut self, c: Id, target: Id, reason: &str, evidence: Vec<Id>) -> Result<Id, ApiError> {
        let device = self.device_kp.as_ref().ok_or_else(ApiError::no_identity)?.public();
        let st = self.communities.get(&c).ok_or_else(|| ApiError::not_found("community"))?;
        let now = self.now();
        let r = Record {
            v: 1,
            community: Some(c),
            author: device,
            parent: Some(target),
            created_at: now,
            expires_at: now + st.limits.max_retention.min(30 * 86_400),
            body: Body::Report { reason: reason.into(), evidence },
            nonce: 0,
        };
        self.publish(r)
    }

    /// Publishes this root's identity record if no copy of it is on this device (new
    /// identities, and installs from before identities existed). Mines the work once.
    pub(crate) fn ensure_identity(&mut self) {
        let (Some(me), Some(root)) = (self.state.me.clone(), self.root_kp.clone()) else { return };
        if me.burned || self.store.identity(&root.public()).is_some() {
            return;
        }
        let work = match me.work {
            Some(w) if hoi_crypto::identity::work_bits(&root.public(), w) >= hoi_crypto::identity::MIN_BITS => w,
            _ => hoi_crypto::identity::mine(&root.public(), if me.generation == 0 { self.identity_bits } else { hoi_crypto::identity::MIN_BITS }),
        };
        // the commitment and the person it continues are fixed for the root
        let (rc, of) = match self.master.as_ref() {
            Some(m) => match Self::identity_body(m, me.generation, work) {
                Body::Identity { rc, of, .. } => (rc, of),
                _ => (None, None),
            },
            None => (me.rc, me.of),
        };
        if let Some(m) = self.state.me.as_mut() {
            m.work = Some(work);
            m.rc = rc;
            m.of = of;
        }
        self.mark_state();
        let now = self.now();
        let r = Record {
            v: 1,
            community: None,
            author: root.public(),
            parent: None,
            created_at: now,
            expires_at: now + hoi_validate::KEEP,
            body: Body::Identity { work, rc, of },
            nonce: 0,
        };
        let _ = self.publish(r);
    }

    /// A delegation record for `device`, blessed by `root`, to be signed by the device.
    pub(crate) fn delegation_record(root: &Keypair, device: Id, now: u64) -> Record {
        let exp = now + crate::api::DELEGATION_LIFETIME;
        let scope = vec!["*".to_string()];
        let root_sig = hoi_crypto::bless(root, &device, &scope, exp);
        Record {
            v: 1,
            community: None,
            author: device,
            parent: None,
            created_at: now,
            expires_at: exp,
            body: Body::Delegation { root: root.public(), scope, expires_at: exp, root_sig, seal: None },
            nonce: 0,
        }
    }

    /// Keeps this device's delegation from lapsing while the root key is here.
    pub(crate) fn renew_delegation(&mut self) {
        let (Some(me), Some(root), Some(dev)) = (self.state.me.clone(), self.root_kp.clone(), self.device_kp.clone()) else { return };
        if me.burned {
            return;
        }
        let now = self.now();
        let current = self.store.delegation(&dev.public()).map(|d| d.info.expires_at).unwrap_or(0);
        // a blessing without the seal key open communities' drop keys are sealed to is renewed
        // at once (SCATTER.md S5)
        let sealed = self.delegation_seal(&dev.public()).is_some();
        if current > now + 30 * 86_400 && sealed {
            return;
        }
        let mut r = Self::delegation_record(&root, dev.public(), now);
        if let Body::Delegation { seal, .. } = &mut r.body {
            *seal = crate::private::seal_public(&dev);
        }
        let _ = self.publish(r);
        self.renew_remote_delegations();
    }

    /// The browsers this computer signs for (crate::remote): their blessings are renewed here,
    /// with the keys it keeps for them.
    pub(crate) fn renew_remote_delegations(&mut self) {
        let Some(root) = self.root_kp.clone() else { return };
        let now = self.now();
        let due: Vec<Keypair> = self
            .remote
            .keys
            .iter()
            .filter(|(d, _)| self.store.delegation(d).is_some_and(|x| x.info.revoked_at.is_none() && x.info.expires_at <= now + 30 * 86_400))
            .map(|(_, k)| k.kp.clone())
            .collect();
        for kp in due {
            let mut r = Self::delegation_record(&root, kp.public(), now);
            r.nonce = mine(&r, self.bits_to_stamp(&r));
            let _ = self.ingest(&kp.sign_record(r).0.encode(), &Source::Local);
        }
    }

    /// The seal key `device`'s current delegation carries, if any.
    pub(crate) fn delegation_seal(&self, device: &Id) -> Option<[u8; 32]> {
        let d = self.store.delegation(device)?;
        match &self.store.get(&d.record)?.record.body {
            Body::Delegation { seal, .. } => seal.map(|k| k.0),
            _ => None,
        }
    }

    // ------------------------------------------------------------------ gossip

    /// Queues a record (with what it depends on) for peers in its community.
    pub(crate) fn gossip(&mut self, id: &Id, except: Option<&str>) {
        self.gossip_inner(id, except, false);
    }

    /// [`Node::gossip`] to every peer, a person's own blessing included: for the blessing a
    /// community's line must see when no injector is linked (crate::open).
    pub(crate) fn gossip_to_all(&mut self, id: &Id) {
        self.gossip_inner(id, None, true);
    }

    fn gossip_inner(&mut self, id: &Id, except: Option<&str>, to_all: bool) {
        let Some(s) = self.store.get(id) else { return };
        if !self.plaintext_forwardable(s) {
            return;
        }
        let mut wires: Vec<Vec<u8>> = self.carriers(s).into_iter().map(|x| x.wire.clone()).collect();
        wires.push(s.wire.clone());
        let community = s.record.community;
        let is_genesis = s.record.kind() == Kind::Community;
        let size: usize = wires.iter().map(|w| w.len()).sum();
        let records = wires.len() as u64;
        let msg = hoi_sync::Msg::Recs { recs: wires }.encode();
        // a person's own identity and blessings go first hand only to their own devices; others
        // get them with the records they come with
        let own_only = !to_all && self.own_blessing(s);
        let first_hand = self.first_hand(s);
        let targets: Vec<String> = self
            .peers
            .iter()
            .filter(|(p, st)| {
                Some(p.as_str()) != except
                    && st.said_hello
                    && (!own_only || st.client || st.home)
                    && community.is_none_or(|c| st.communities.contains(&c))
                    && (s.record.kind() != Kind::Profile || st.features.contains(hoi_sync::FEATURE_PROFILE))
                    && (s.record.kind() != Kind::ProfileColor || st.features.contains(hoi_sync::FEATURE_PROFILE_COLOR))
            })
            .map(|(p, _)| p.clone())
            .collect();
        if first_hand && targets.iter().any(|p| self.peers.get(p).is_some_and(|st| !st.client && !st.home)) {
            self.note_exposed();
        }
        if except.is_some() && !targets.is_empty() {
            self.state.passed_along += (size * targets.len()) as u64;
            self.mark_state();
        }
        for p in targets {
            // A peer subscribed to this community holds one more of our records now.
            if let (Some(c), Some(st)) = (community, self.peers.get_mut(&p))
                && !is_genesis
            {
                st.offers.entry(c).or_default().given += 1;
            }
            self.queue_out(p, msg.clone(), records, 0);
        }
    }

    // ------------------------------------------------------------------ media

    /// Asks peers for media of visible posts by members allowed to share it.
    pub(crate) fn request_missing_media(&mut self) {
        if !self.state.settings.auto_media {
            return;
        }
        let now = self.now();
        // Keep attempts after timeout so an unavailable first image cannot starve every
        // later image. Forget only completed, blocked or no-longer-referenced requests.
        let (store, state, lists) = (&self.store, &self.state, &self.lists_exact);
        self.media_wanted.retain(|h, _| {
            !store.has_blob(h)
                && !state.private_blobs.contains(h)
                && !(state.media_blocklist.contains(h) || lists.contains(h))
                && !state.bad_media.contains(h)
                && store.media_users(h).next().is_some()
        });
        // One outstanding blob keeps valid catch-up below the transport's byte budget.
        if self.media_wanted.values().any(|t| *t + MEDIA_RETRY > now) {
            return;
        }
        let mut by_community: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
        for c in self.state.joined.keys() {
            if self.is_private_community(c) {
                continue;
            }
            for s in self.store.of_kind(c, Kind::Post) {
                let Body::Post { media, .. } = &s.record.body else { continue };
                if media.is_empty() || !self.forwardable(s) {
                    continue;
                }
                for m in media {
                    if !self.store.has_blob(&m.hash)
                        && !self.state.private_blobs.contains(&m.hash)
                        && !self.media_refused(&m.hash)
                        && !self.state.bad_media.contains(&m.hash)
                        && !self.media_wanted.get(&m.hash).is_some_and(|t| t + MEDIA_RETRY > now)
                    {
                        by_community.entry(*c).or_default().push(m.hash);
                    }
                }
            }
        }
        if self.store.blob_bytes() >= MEDIA_CAP {
            return;
        }
        let mut next: Option<(u64, Id, Id, Vec<String>)> = None;
        for (c, hashes) in by_community {
            let peers: Vec<String> = self.peers.iter().filter(|(_, p)| p.said_hello && p.communities.contains(&c)).map(|(k, _)| k.clone()).collect();
            if peers.is_empty() {
                continue;
            }
            for hash in hashes {
                let attempted = self.media_wanted.get(&hash).copied().unwrap_or(0);
                if next.as_ref().is_none_or(|(at, id, _, _)| (attempted, hash) < (*at, *id)) {
                    next = Some((attempted, hash, c, peers.clone()));
                }
            }
        }
        if let Some((_, hash, c, peers)) = next {
            self.media_wanted.insert(hash, now);
            // over iroh-blobs from one peer that serves it so, else `bwant` to all (crate::blobs)
            if let Some((peer, endpoint)) = self.blobs_peer(&c, &peers) {
                self.queue_blob_fetch(peer, endpoint, hash, c);
                return;
            }
            let msg = hoi_sync::Msg::BWant { ids: vec![hash] }.encode();
            for peer in peers {
                self.queue_out(peer, msg.clone(), 0, 0);
            }
        }
    }

    /// Media bytes from a peer. Returns false when the peer sent something that doesn't hash
    /// to what it says (its fault); bytes that hash right but fail the checks are the
    /// author's fault: their post stays held and their key takes a strike.
    pub(crate) fn on_blob(&mut self, id: Id, data: Vec<u8>) -> bool {
        if blake3::hash(&data).as_bytes() != id.as_bytes() {
            return false;
        }
        if self.store.has_blob(&id) || self.media_refused(&id) || self.state.bad_media.contains(&id) {
            return true;
        }
        let Some((post, made, c, author, mref)) = self.store.media_users(&id).filter(|s| self.usable(s)).find_map(|s| match &s.record.body {
            Body::Post { media, .. } => media.iter().find(|m| m.hash == id).map(|m| (s.id, s.record.created_at, s.record.community, s.author_root, m.clone())),
            _ => None,
        }) else {
            // nobody here asked for it
            return true;
        };
        let c = c.expect("posts have communities");
        let ok = hoi_validate::media::check(&mref.mime, &data)
            .ok()
            .is_some_and(|info| data.len() as u64 == mref.size && (!hoi_validate::media::is_image(&mref.mime) || (info.w == mref.w && info.h == mref.h)));
        if !ok {
            self.state.bad_media.insert(id);
            self.mark_state();
            let now = self.now();
            self.offence(author, c, (post, made), Offence::Content, now);
            self.mark_changed(c);
            return true;
        }
        if !crate::look::wants_look(&mref.mime) {
            self.take_blob(id, data, crate::look::Look::Nothing);
            return true;
        }
        // a picture this device can't fingerprint would be a way around the blocklist; a host
        // that decodes elsewhere answers later (crate::look)
        if let Some(look) = self.look_at(id, &mref.mime.clone(), Some(data.clone()), crate::look::Why::Arrived) {
            self.take_blob(id, data, look);
        }
        true
    }

    /// Bytes that passed the checks, once looked at: refused when they look like something this
    /// device refuses, struck when a picture can't be decoded, kept otherwise.
    pub(crate) fn take_blob(&mut self, id: Id, data: Vec<u8>, look: crate::look::Look) {
        if self.store.has_blob(&id) || self.media_refused(&id) || self.state.bad_media.contains(&id) {
            return;
        }
        let Some((post, made, c, author)) = self.store.media_users(&id).filter(|s| self.usable(s)).find_map(|s| match &s.record.body {
            Body::Post { media, .. } => media.iter().any(|m| m.hash == id).then_some((s.id, s.record.created_at, s.record.community, s.author_root)),
            _ => None,
        }) else {
            return;
        };
        let c = c.expect("posts have communities");
        if self.blocked_look(&look) {
            // the same picture as one this device refuses: never kept, never shown
            self.state.media_blocklist.insert(id);
            self.mark_state();
            self.media_wanted.remove(&id);
            self.mark_changed(c);
            return;
        }
        if look == crate::look::Look::Unreadable {
            self.state.bad_media.insert(id);
            self.mark_state();
            let now = self.now();
            self.offence(author, c, (post, made), Offence::Content, now);
            self.mark_changed(c);
            return;
        }
        if self.store.blob_bytes() + data.len() as u64 > MEDIA_CAP {
            return;
        }
        self.changes.push(Change::Blob { key: media_key(&id), value: Some(data.clone()) });
        self.store.put_blob(id, data);
        self.media_wanted.remove(&id);
        self.mark_changed(c);
    }

    /// Media a peer asks for: only bytes of visible posts in communities they share. Bytes still
    /// on their way here (a home node relaying a post that just came from its author's device)
    /// are sent once they arrive.
    pub(crate) fn on_bwant(&mut self, peer: &str, ids: &[Id]) {
        let Some(shared) = self.peers.get(peer).map(|p| p.communities.clone()) else { return };
        let mut owed = Vec::new();
        let mut sent_bytes = 0usize;
        for h in ids.iter().take(hoi_sync::MAX_BWANT) {
            if self.state.private_blobs.contains(h) {
                continue;
            }
            let allowed = self.store.media_users(h).any(|s| self.plaintext_forwardable(s) && s.record.community.is_some_and(|c| shared.contains(&c)));
            if !allowed {
                continue;
            }
            match self.store.blob(h) {
                Some(data) => {
                    // Older clients may ask for 32 large blobs at once. Send a bounded prefix;
                    // absent blobs remain eligible for their normal retry requests.
                    if sent_bytes + data.len() > hoi_sync::MAX_BLOB {
                        break;
                    }
                    sent_bytes += data.len();
                    let media = data.len() as u64;
                    let msg = hoi_sync::Msg::Blob { id: *h, data: data.clone() }.encode();
                    self.queue_out(peer.to_string(), msg, 0, media);
                }
                None => owed.push(*h),
            }
        }
        if let Some(p) = self.peers.get_mut(peer) {
            for h in owed {
                if p.owed.len() < MAX_OWED {
                    p.owed.insert(h);
                }
            }
        }
    }

    /// Media that just arrived from `from`: sent on to the other peers that asked for it, if
    /// this device kept it.
    pub(crate) fn pay_owed(&mut self, id: Id, from: &str) {
        let waiting: Vec<String> = self.peers.iter_mut().filter_map(|(k, p)| (p.owed.remove(&id) && k != from).then(|| k.clone())).collect();
        if !self.store.has_blob(&id) {
            return;
        }
        for p in waiting {
            self.on_bwant(&p, &[id]);
        }
    }

    /// Keeps sanitized media bytes made on this device.
    pub(crate) fn keep_media(&mut self, items: Blobs) {
        for (h, b) in items {
            if !self.store.has_blob(&h) {
                // Staged attachments are private until a valid local public Post explicitly
                // publishes them. This also protects bytes left by a failed private draft,
                // and public references planted before the local file was selected.
                if self.state.private_blobs.insert(h) {
                    self.mark_state();
                }
                self.changes.push(Change::Blob { key: media_key(&h), value: Some(b.clone()) });
                self.store.put_blob(h, b);
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn mine(r: &Record, bits: u32) -> u64 {
    hoi_crypto::pow::mine(r, bits)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn mine(r: &Record, bits: u32) -> u64 {
    hoi_crypto::pow::mine_single(r, bits)
}
