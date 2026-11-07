//! What this device remembers besides records: who you are, who you trust, what you joined,
//! settings. Persisted as JSON blobs ("state", "keys"); never sent anywhere.

use std::collections::{BTreeMap, BTreeSet};

use hoi_types::Id;
use serde::{Deserialize, Serialize};

use crate::dto::Settings;

/// Local persistence compatibility is separate from wire protocol and public index versions.
/// Bump this before changing stored state/key shapes incompatibly; older binaries fail closed.
pub(crate) const STORAGE_FORMAT_VERSION: u32 = 3;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StorageFormat {
    pub version: u32,
}

impl StorageFormat {
    pub fn encoded() -> Vec<u8> {
        serde_json::to_vec(&Self { version: STORAGE_FORMAT_VERSION }).expect("storage format is serializable")
    }
}

/// PDQ hashes a device refuses: saved as hex (as hash lists write them), held parsed so a check
/// is a scan over bytes, not over strings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdqList {
    hashes: BTreeSet<[u8; 32]>,
}

impl PdqList {
    /// Adds a hash; whether it was new.
    pub fn insert(&mut self, h: [u8; 32]) -> bool {
        self.hashes.insert(h)
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    /// Whether any way of turning or mirroring a picture is close to a listed hash.
    pub fn blocks(&self, v: &hoi_validate::similar::Variants) -> bool {
        self.hashes.iter().any(|h| v.matches(h))
    }
}

impl Serialize for PdqList {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.hashes.iter().map(hoi_validate::similar::to_hex))
    }
}

impl<'de> Deserialize<'de> for PdqList {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let hex = Vec::<String>::deserialize(d)?;
        let hashes =
            hex.iter().map(|h| hoi_validate::similar::from_hex(h).ok_or_else(|| serde::de::Error::custom("not a PDQ hash"))).collect::<Result<_, _>>()?;
        Ok(PdqList { hashes })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub root: Id,
    pub device: Id,
    pub name: String,
    /// Local selection. None is unset; empty explicitly restores the identicon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Vec<u8>>,
    /// Revision of this device's last explicit picture edit; remote updates may be newer.
    #[serde(default)]
    pub avatar_revision: u64,
    /// None is unset; empty explicitly restores the key-derived identicon color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_color: Option<String>,
    #[serde(default)]
    pub avatar_color_revision: u64,
    pub created_at: u64,
    pub has_root: bool,
    /// Nonce of this root's identity work (public; kept so the identity record can be
    /// published again without redoing the work).
    #[serde(default)]
    pub work: Option<u64>,
    /// The owner burned this identity. Nothing it signs is accepted anywhere.
    #[serde(default)]
    pub burned: bool,
    /// The generation of the root this device signs with (`hoi_crypto::recovery::root_at`):
    /// 0 for the person's first root, one more after each rotation.
    #[serde(default)]
    pub generation: u32,
    /// Brought back from a recovery line and not yet signing with the person's current root.
    #[serde(default)]
    pub restoring: bool,
    /// What this root's identity record commits to and whom it continues, so it can be
    /// published again without the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rc: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<Id>,
    /// This device signs nothing itself: the person's computer keeps its device key and signs
    /// for it over the home link.
    #[serde(default)]
    pub remote: bool,
}

/// Records by a newcomer this device's person has read and not hidden (auto-vouch).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingRead {
    pub community: Id,
    pub author: Id,
    pub ids: BTreeSet<Id>,
    pub first: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Joined {
    pub at: u64,
    pub pinning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bounce {
    pub community: Id,
    pub at: u64,
    pub reason: String,
    /// When it lifts. Bounces saved before they lifted on their own have none: they last
    /// `hoi_spam::BOUNCE_FOR` from `at`.
    #[serde(default)]
    pub until: u64,
    /// The how-manyth bounce in a row of this key; each lasts twice as long as the one before.
    #[serde(default)]
    pub times: u32,
}

impl Bounce {
    pub fn lifts_at(&self) -> u64 {
        if self.until == 0 { self.at.saturating_add(hoi_spam::BOUNCE_FOR) } else { self.until }
    }

    pub fn active(&self, now: u64) -> bool {
        now < self.lifts_at()
    }
}

/// What stays of a removed or deleted record: no body, just where it was and why it went.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skeleton {
    pub community: Id,
    pub parent: Option<Id>,
    pub author: Id,
    pub kind: String,
    pub created_at: u64,
    pub expires_at: u64,
    /// Root that removed it; `None` when the author deleted it.
    pub by: Option<Id>,
    pub reason: String,
    pub at: u64,
    pub ledger: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairInfo {
    /// WebSocket address; empty when the home node is reachable only over WebRTC.
    pub url: String,
    /// What this device shows its home node: the pairing link's token until the home node
    /// answers with a credential of this device's own, then that credential.
    pub token: String,
    pub name: Option<String>,
    /// WebRTC routes (`rtc:<fingerprint>@<addr>`), refreshed from the home node's hello.
    #[serde(default)]
    pub rtc: Vec<String>,
    /// The link this device paired with, hashed: opening that spent link again keeps the pairing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spent: Option<Id>,
    /// The home node's current pairing link token, for showing the next device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// The key `link` messages between the home node and its devices are sealed with (base64url).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_key: Option<String>,
    /// The home node delivered valid records: from then on its routes are taken.
    #[serde(default)]
    pub proven: bool,
}

/// A device this node serves, known by a hash of its own credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub credential: Id,
    pub seen: u64,
}

/// A route of a community member this device met on an authenticated connection, so its
/// invites can point newcomers at members who are around, not only at whoever invited them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedRoute {
    pub hint: String,
    pub seen: u64,
}

/// A public node this device reached itself, or one whose link its person opened (crate::nodes).
/// Kept so the next start has somewhere to look for communities without any configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownNode {
    /// The routes this device dialed it by, or that it gave for its own endpoint or certificate.
    pub routes: Vec<String>,
    /// When it last answered as a public node; 0 for a link opened here that hasn't yet.
    pub seen: u64,
}

/// An operator's notice this device met (crate::notice): the words, so they can be shown
/// again without a connection, and what the person decided about them. Kept by the node's
/// endpoint key; the certificate fingerprints it was met by say which `rtc:` routes are its.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeRecord {
    pub hash: Id,
    /// The notice's canonical bytes (`NodeNotice::canonical`), base64url in the saved state.
    #[serde(with = "crate::notice::b64bytes")]
    pub notice: Vec<u8>,
    #[serde(default)]
    pub certs: Vec<String>,
    /// Where it was met (an address), for the person's orientation; never sent anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub seen: u64,
    /// The node itself asked, in a hello on a connection: the app shows it unasked. A notice
    /// only read from a link waits on that link's page instead.
    #[serde(default)]
    pub asked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declined: Option<u64>,
}

/// Public connection hints from an explicitly accepted invite, a paired device or a home node
/// that delivered records. Routes from a registry or an announcement are never kept here until
/// they delivered records (`Node::trial_routes`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BootstrapHints {
    pub peers: Vec<String>,
    pub relays: Vec<String>,
    /// Trackers an accepted invite named, before the community's own limits arrive.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trackers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPublicJoin {
    pub name: Option<String>,
    pub error: Option<String>,
}

/// Why this node carries a community nobody paired or joined it into, and when
/// another holder was last seen, so carrying on request stays bounded. The origin is a word
/// (`index` or `request`), never who asked.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Carried {
    pub since: u64,
    pub origin: String,
    pub last_other_holder: Option<u64>,
}

/// One item this node's operator suppressed: when, and the operator's own reference
/// (a case number, say), never anything about who reported it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Suppression {
    pub at: u64,
    pub note: Option<String>,
}

/// What this node's operator suppressed with `hoi-node admin` (crate::suppress): never fetched,
/// stored or served here again. Communities by key, records by id, media by BLAKE3 hash.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Suppressed {
    pub communities: BTreeMap<Id, Suppression>,
    pub records: BTreeMap<Id, Suppression>,
    pub media: BTreeMap<Id, Suppression>,
}

impl Suppressed {
    pub fn is_empty(&self) -> bool {
        self.communities.is_empty() && self.records.is_empty() && self.media.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalState {
    /// What the operator suppressed. Left out of the saved state while empty, so a store that
    /// never used it still opens in older software, and one that did refuses to open there
    /// rather than fetch and serve it all again.
    #[serde(skip_serializing_if = "Suppressed::is_empty")]
    pub suppressed: Suppressed,
    /// Privacy survives unavailable MLS state: never downgrade a private group to raw sync.
    pub private_communities: BTreeSet<Id>,
    /// Global records first learned inside an encrypted group stay off raw gossip and Want.
    /// Identity/delegation carriers of an intentionally public signed post may be disclosed
    /// with that post, but private-only profiles are not made public by knowing their IDs.
    pub private_carriers: BTreeSet<Id>,
    /// Blob hashes learned privately cannot become public through an untrusted public
    /// reference. Only an explicit local publication or receiving the full public bytes
    /// can disclose them on plaintext media requests.
    pub private_blobs: BTreeSet<Id>,
    /// Invite routes, scoped to communities this device joined.
    pub bootstrap: BTreeMap<Id, BootstrapHints>,
    /// Routes of members met on authenticated connections, per community, newest first.
    pub learned: BTreeMap<Id, Vec<LearnedRoute>>,
    /// Public nodes reached before, newest first (crate::nodes). Left out of the saved state
    /// while empty, so a store that never met one still opens in older software.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<KnownNode>,
    /// Operators' notices met and what was decided about them (crate::notice). Left out of the
    /// saved state while empty, so a store that never met one still opens in older software.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub notices: BTreeMap<Id, NoticeRecord>,
    /// Bytes exchanged with other people's devices today: (UTC day, bytes), for the relay cap.
    pub relay_day: (u64, u64),
    /// Native nodes start with relaying on once (opt-out, TECHNICAL.md §4); this records that.
    pub relay_defaulted: bool,
    /// The desktop starts as the companion once; this records that.
    pub companion_defaulted: bool,
    pub me: Option<Identity>,
    pub joined: BTreeMap<Id, Joined>,
    /// Explicit public join requests, awaiting signed open-community details from peers.
    pub public_joins: BTreeMap<Id, PendingPublicJoin>,
    /// Communities carried for an index or on a peer's request (native nodes).
    pub carried: BTreeMap<Id, Carried>,
    /// Public signatures authorizing a name claim for communities created here. The genesis
    /// key is still destroyed after creation; this proof cannot sign records or rename a claim.
    pub registration_proofs: BTreeMap<Id, String>,
    /// Version-bound public index proofs, separate from legacy name reservations.
    pub publication_proofs: BTreeMap<Id, crate::dto::CommunityPublication>,
    pub friends: BTreeMap<Id, u64>,
    pub muted: BTreeSet<Id>,
    pub petnames: BTreeMap<Id, String>,
    /// Self-chosen names seen on records: root → (name, when).
    pub names: BTreeMap<Id, (String, u64)>,
    pub saved: BTreeSet<Id>,
    pub first_seen: BTreeMap<Id, u64>,
    pub bounced: BTreeMap<Id, Bounce>,
    pub bounced_records: u64,
    pub settings: Settings,
    pub home: Option<PairInfo>,
    pub node_id: Option<Id>,
    /// The token of the pairing link this node shows: good for one device, then replaced.
    pub serve_token: Option<String>,
    /// Devices paired with this node, each by its own credential; least recently seen go first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paired: Vec<PairedDevice>,
    /// Seals `link` messages between this node and its devices (base64url, 32 bytes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_key: Option<String>,
    pub passed_along: u64,
    pub skeletons: BTreeMap<Id, Skeleton>,
    pub device_labels: BTreeMap<Id, String>,
    pub labelers: BTreeSet<Id>,
    /// Communities this person shares whom they trust in.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub trust_shared: BTreeSet<Id>,
    /// List keepers whose blocklists this person follows.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub blocklists: BTreeSet<Id>,
    /// Aggregators whose counts this person follows, and communities this device counts for.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub aggregators: BTreeSet<Id>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub aggregating: BTreeSet<Id>,
    /// Domains checked as handles, and whom they named.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub handles: BTreeMap<String, crate::handle::Checked>,
    pub vote_seq: u64,
    pub reads: Vec<PendingRead>,
    /// Media this device refuses (exact BLAKE3), e.g. from an illegal-content hide.
    pub media_blocklist: BTreeSet<Id>,
    /// Media whose bytes failed the checks: their posts stay held.
    pub bad_media: BTreeSet<Id>,
    /// Perceptual fingerprints (PDQ) of pictures this device refuses, near copies included:
    /// from illegal-content hides and removals, and from hash lists the person loaded.
    pub similar_blocklist: PdqList,
    /// PDQ hashes of frames of videos this device refuses (crate::look).
    #[serde(default, skip_serializing_if = "PdqList::is_empty")]
    pub video_blocklist: PdqList,
    /// Identity work already verified: root → (work nonce, bits). Saves an Argon2id run per
    /// identity at every start.
    pub identity_work: BTreeMap<Id, (u64, u32)>,
    /// Per community, when this device last finished catching up with a peer: anything that
    /// shows up afterwards claiming to be older was not out there then (PROTOCOL.md §15.5).
    pub synced: BTreeMap<Id, u64>,
    /// Completion times for connection diagnostics, separate from the arrival-time security watermark.
    pub sync_completed: BTreeMap<Id, u64>,
    /// Vouches, reviews and ledger entries that arrived late: they count from when this device
    /// first saw them, not from the time they claim.
    pub late: BTreeMap<Id, u64>,
}

/// Secret keys, hex. Kept in their own blob so hosts can move it to an OS keychain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Secrets {
    pub transport: Option<String>,
    pub root: Option<String>,
    pub device: Option<String>,
    pub pending_device: Option<String>,
    pub communities: BTreeMap<Id, String>,
    /// The WebRTC DTLS certificate and its key (DER, base64url): its fingerprint is in pairing
    /// links and invites, so it must outlive restarts.
    pub rtc_cert: Option<(String, String)>,
    /// The recovery line's secret, until the person says it is saved.
    #[serde(default)]
    pub master: Option<String>,
    /// Device keys this computer keeps and signs with for browsers: device → (secret,
    /// label).
    #[serde(default)]
    pub remote: BTreeMap<Id, (String, String)>,
}

impl Secrets {
    pub fn wipe(&mut self) {
        use zeroize::Zeroize;
        for s in [&mut self.root, &mut self.device, &mut self.pending_device, &mut self.transport, &mut self.master].into_iter().flatten() {
            s.zeroize();
        }
        if let Some((_, key)) = &mut self.rtc_cert {
            key.zeroize();
        }
        for (secret, _) in self.remote.values_mut() {
            secret.zeroize();
        }
        for v in self.communities.values_mut() {
            v.zeroize();
        }
    }
}
