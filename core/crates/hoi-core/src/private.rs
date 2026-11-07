//! Client-held MLS state and an opaque, immutable relay outbox. Relay operators never receive
//! this storage blob, signed inner records, media, Welcome secrets, or anything exported from an
//! MLS epoch. Membership is coordinated by the owner's device; in a group that follows its
//! records, by its owner's and moderators' devices in turn. Existing public communities
//! cannot be converted.
//!
//! Relay protocol 2: each epoch's exporter secret derives the room key, which is the room's
//! address, signs every upload and read for every member alike, and seals each padded object.
//! The relay sees no community key, MLS header, member key or exact size.

use std::collections::{BTreeMap, BTreeSet};

use hoi_crypto::Keypair;
use hoi_types::{
    Body, Id, Kind, hex_encode,
    uri::{b64, unb64},
};
use openmls::prelude::{
    tls_codec::{Deserialize as TlsDeserialize, Serialize as TlsSerialize},
    *,
};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::storage::StorageProvider as _;
use openmls_traits::{OpenMlsProvider, types::SignatureScheme};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::{
    Node, Source,
    api::R,
    dto::{CommunityDraft, CommunityView},
    error::ApiError,
    state::Joined,
};

const SUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;
const MAX_OBJECT: usize = 1024 * 1024;
const MAX_PENDING: usize = 64;
const MAX_RECEIVED: usize = 4096;
const CHUNK: usize = 256 * 1024;
pub(crate) const RETENTION: u64 = 30 * 86400;
const RELAY_LABEL: &str = "hoi relay v2";
const SEAL_CONTEXT: &str = "hoi relay v2 object";
// Sealed objects are 4 KiB or more with four significant bits: at most 1/8 overhead past 4 KiB.
const PAD_MIN: usize = 4096;
const SEAL_OVERHEAD: usize = 24 + 16;
const MAX_READS: usize = 65;
// What every private group requires of every leaf, from its creation: a key package
// that lacks one of these is refused, and MLS refuses to add it.
const PROFILE_CAPABILITY: ExtensionType = ExtensionType::Unknown(0xff42);
const PROFILE_COLOR_CAPABILITY: ExtensionType = ExtensionType::Unknown(0xff43);
/// Mailboxes (crate::mailbox): the group context carries the mailbox list under this type.
pub(crate) const MAILBOX_EXTENSION: u16 = 0xff44;
const MAILBOX_CAPABILITY: ExtensionType = ExtensionType::Unknown(MAILBOX_EXTENSION);
/// Scattered delivery (crate::keep, SCATTER.md S3): `pointer` payloads, posts kept in pieces.
const SCATTER_CAPABILITY: ExtensionType = ExtensionType::Unknown(0xff45);
/// Several committers (SCATTER.md S4): following commits from the moderators' devices
/// of a group whose membership follows its records, and going back to an earlier committer's
/// commit of the same epoch.
const COMMITTERS_CAPABILITY: ExtensionType = ExtensionType::Unknown(0xff46);
/// Every capability a leaf of a private group must list.
const REQUIRED: [ExtensionType; 5] = [PROFILE_CAPABILITY, PROFILE_COLOR_CAPABILITY, MAILBOX_CAPABILITY, SCATTER_CAPABILITY, COMMITTERS_CAPABILITY];
/// How long a change the records call for waits for each committer ahead of a device before
/// that device makes it: the first in line makes it at once, the second after an hour of
/// silence, the third after two.
pub(crate) const TURN: u64 = 3600;
/// How long a member keeps the epoch it followed a later committer's commit out of: an earlier
/// committer's commit of that epoch that turns up meanwhile counts instead.
pub(crate) const CONTEST: u64 = 86_400;
/// A committer reads an epoch at least this long before it changes membership by itself, so it
/// does not commit over a change it has not read yet.
pub(crate) const CATCH_UP: u64 = 60;
/// What the drop key's work is due under, beside the devices to add or remove and
/// [`Id::ZERO`] for the mailbox list.
pub(crate) const DROP_KEY_DUE: Id = Id([0xff; 32]);
/// Media chunks one scattered object carries at most: under 4 MiB once encoded.
const SCATTER_MEDIA_CHUNKS: usize = 11;

fn profile_capabilities() -> Capabilities {
    Capabilities::new(None, None, Some(&REQUIRED), None, None)
}

/// Whether a leaf lists every capability a private group requires.
fn capable(capabilities: &Capabilities) -> bool {
    REQUIRED.iter().all(|c| capabilities.extensions().contains(c))
}

/// One epoch of a private group as its mailboxes see it (crate::mailbox).
#[derive(Debug, Clone)]
pub(crate) struct MailEpoch {
    pub epoch: u64,
    /// The relay room of the epoch: which queued messages belong to it.
    pub room: String,
    pub own: u32,
    pub leaves: Vec<u32>,
    pub slot_key: [u8; 32],
    pub mailboxes: Vec<crate::mailbox::Mailbox>,
    /// The group's drop key, as a committer published it with the mailboxes, and the
    /// committers' devices its secret was given to.
    pub drop_key: Option<[u8; 32]>,
    pub holders: Vec<Id>,
    pub owner: bool,
    /// This device's place among the devices that may commit in this epoch (0: first in line),
    /// if it is one of them.
    pub rank: Option<usize>,
    pub members: usize,
    pub active: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct PrivateState {
    storage: Vec<(Vec<u8>, Vec<u8>)>,
    groups: BTreeMap<Id, SavedGroup>,
    pending_request: Option<PendingRequest>,
    /// In a group whose membership follows its records, on each committing device (the
    /// owner's and moderators'): join requests members passed on, by group, with when they came.
    #[serde(default)]
    requests: BTreeMap<Id, Vec<(String, u64)>>,
    /// On the device that passed them on: join requests, by group, with the requesting device,
    /// when they were passed and the epoch they last went to. A commit that ends that epoch
    /// leaves them unread by the committers who moved on, so they go again in each new epoch
    /// until their device is in the group or they expire.
    #[serde(default)]
    passed: BTreeMap<Id, Vec<(String, Id, u64, u64)>>,
    /// The seal keys committers' devices announced (base64), by group and device: what this
    /// device seals the group's drop key to when it gives it.
    #[serde(default)]
    pub(crate) seal_keys: BTreeMap<Id, BTreeMap<Id, String>>,
    /// The epoch this device last announced its own seal key in, by group.
    #[serde(default)]
    announced: BTreeMap<Id, u64>,
    /// On any member's device: invitations a committing device made for requests, by group and
    /// requesting device, to hand on.
    #[serde(default)]
    invitations: BTreeMap<Id, BTreeMap<String, String>>,
    /// The epoch each of those invitations came in: one that came in a branch a group went back
    /// from is let go with it.
    #[serde(default)]
    invited_in: BTreeMap<Id, BTreeMap<String, u64>>,
    /// On each committing device: since when each change the records call for has been due
    /// here, by group, keyed by the device to add or remove ([`Id::ZERO`]: the mailbox list). A
    /// device later in line makes one only after waiting [`TURN`] for each committer ahead.
    #[serde(default)]
    due: BTreeMap<Id, BTreeMap<Id, u64>>,
    /// Epochs this device followed a later committer's commit out of, by group, oldest first:
    /// kept for [`CONTEST`] in case an earlier committer's commit of the same epoch turns up.
    #[serde(default)]
    contests: BTreeMap<Id, Vec<Contest>>,
    #[serde(skip)]
    pub(crate) dirty: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingRequest {
    code: String,
    created: u64,
    key_ref: KeyPackageRef,
    /// The seed of the one-time key the invitation is sealed to, and the holders this device
    /// looks for it on (crate::pickup).
    pickup: String,
    holders: Vec<crate::mailbox::Mailbox>,
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        self.pickup.zeroize();
    }
}

impl PrivateState {
    pub(crate) fn contains_group(&self, c: &Id) -> bool {
        self.groups.contains_key(c)
    }
    pub(crate) fn wipe(&mut self) {
        for (k, v) in self.storage.iter_mut().chain(self.contests.values_mut().flatten().flat_map(|ct| ct.storage.iter_mut())) {
            k.zeroize();
            v.zeroize();
        }
        *self = Self::default();
    }
    fn provider(&self) -> OpenMlsRustCrypto {
        let p = OpenMlsRustCrypto::default();
        p.storage().values.write().expect("MLS storage lock").extend(self.storage.iter().cloned());
        p
    }
    fn save(&mut self, p: &OpenMlsRustCrypto) {
        for (_, v) in &mut self.storage {
            v.zeroize();
        }
        self.storage = p.storage().values.read().expect("MLS storage lock").iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        self.dirty = true;
    }
}

/// An epoch a member followed a later committer's commit out of: its MLS state, kept
/// so that an earlier committer's commit of the same epoch, should one turn up in that epoch's
/// slots, can still be followed instead.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Contest {
    pub epoch: u64,
    /// The place in line of the commit followed out of it: only an earlier one replaces it.
    pub rank: usize,
    pub until: u64,
    /// The group's entries of the MLS storage in that epoch (and its own leaf's key), nothing else.
    storage: Vec<(Vec<u8>, Vec<u8>)>,
    /// The epoch's slot key and mailboxes, and the leaves of the committers ahead, each with the
    /// next of its slots to read.
    pub slot_key: [u8; 32],
    pub mailboxes: Vec<crate::mailbox::Mailbox>,
    pub ahead: BTreeMap<u32, u32>,
}

/// A device that may commit in a group, as the group's records and leaves say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Committer {
    pub leaf: u32,
    pub device: Id,
    pub root: Id,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedGroup {
    /// The device that created the group coordinates membership and must not leave it.
    owner: bool,
    sent_records: BTreeSet<Id>,
    sent_media: BTreeMap<Id, usize>,
    received: Vec<String>,
    objects: Vec<CipherObject>,
    partial: BTreeMap<Id, PartialMedia>,
    /// The epoch each record (or media item, by hash) was first sent in: after a commit only what
    /// first went in the epoch it ended goes again, not everything (S6). A resend keeps the
    /// first epoch, so a record goes again once, not after every commit that follows.
    sent_in: BTreeMap<Id, u64>,
}

impl SavedGroup {
    fn new(owner: bool) -> SavedGroup {
        SavedGroup {
            owner,
            sent_records: BTreeSet::new(),
            sent_media: BTreeMap::new(),
            received: vec![],
            objects: vec![],
            partial: BTreeMap::new(),
            sent_in: BTreeMap::new(),
        }
    }
    fn sent(&mut self, ids: impl IntoIterator<Item = Id>, epoch: u64) {
        for id in ids {
            self.sent_records.insert(id);
            self.sent_in.entry(id).or_insert(epoch);
        }
    }
    fn sent_media_to(&mut self, hash: Id, chunks: usize, epoch: u64) {
        self.sent_media.insert(hash, chunks);
        self.sent_in.entry(hash).or_insert(epoch);
    }
    /// Everything goes again: the device made a commit that adds someone, or took up another
    /// branch, or what it queued expired.
    fn send_all_again(&mut self) {
        self.sent_records.clear();
        self.sent_media.clear();
        self.sent_in.clear();
    }
    /// After a commit ended `epoch`: what went in it may have landed after the commit, where
    /// members who moved on never read it; it goes again. `own`: on a commit that added someone,
    /// this device's own records and media go again too, so the newcomer has every author's.
    fn send_again_after(&mut self, epoch: u64, own: &BTreeSet<Id>) {
        let again: Vec<Id> = self.sent_in.iter().filter(|(id, e)| **e >= epoch || own.contains(id)).map(|(id, _)| *id).collect();
        for id in again {
            self.sent_records.remove(&id);
            self.sent_media.remove(&id);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartialMedia {
    total: usize,
    /// The parts in, by their offset in the item: 256 KiB chunks sent whole, or the slices of
    /// objects kept in pieces.
    parts: BTreeMap<usize, Vec<u8>>,
}

impl PartialMedia {
    fn held(&self) -> usize {
        self.parts.values().map(Vec::len).sum()
    }
    /// Adds a part; the whole item once the parts cover it.
    fn add(&mut self, offset: usize, data: Vec<u8>) -> Option<Vec<u8>> {
        self.parts.insert(offset, data);
        let mut pos = 0;
        for (off, d) in &self.parts {
            if *off > pos {
                return None;
            }
            pos = pos.max(off + d.len());
        }
        if pos < self.total {
            return None;
        }
        let mut all = vec![0u8; self.total];
        for (off, d) in &self.parts {
            let end = (off + d.len()).min(self.total);
            all[*off..end].copy_from_slice(&d[..end - off]);
        }
        Some(all)
    }
}

/// What starts a media object's content (S6): raw bytes, not base64 inside JSON.
const SLICE_MAGIC: &[u8] = b"hoi slice v1\n";

/// The header of a media object: which item, how long it is, where this slice of it starts,
/// and the posts it is kept for.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Slice {
    v: u8,
    community: Id,
    hash: Id,
    total: usize,
    offset: usize,
    posts: Vec<Id>,
}

fn slice_content(header: &Slice, bytes: &[u8]) -> Vec<u8> {
    let h = serde_json::to_vec(header).unwrap_or_default();
    let mut out = Vec::with_capacity(SLICE_MAGIC.len() + 4 + h.len() + bytes.len());
    out.extend_from_slice(SLICE_MAGIC);
    out.extend_from_slice(&(h.len() as u32).to_be_bytes());
    out.extend_from_slice(&h);
    out.extend_from_slice(bytes);
    out
}

/// A media object's header and bytes, checked: none for any other content.
fn read_slice(c: Id, content: &[u8]) -> Option<R<(Slice, Vec<u8>)>> {
    let rest = content.strip_prefix(SLICE_MAGIC)?;
    let read = || -> R<(Slice, Vec<u8>)> {
        let len = u32::from_be_bytes(rest.get(..4).ok_or_else(fail)?.try_into().map_err(mls)?) as usize;
        let header: Slice = decoded(rest.get(4..4 + len).ok_or_else(fail)?)?;
        let bytes = rest[4 + len..].to_vec();
        if header.v != 1
            || header.community != c
            || header.total == 0
            || header.total > 16 * 1024 * 1024
            || bytes.is_empty()
            || header.offset + bytes.len() > header.total
            || header.posts.len() > MAX_MEDIA_POSTS
        {
            return Err(fail());
        }
        Ok((header, bytes))
    };
    Some(read())
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivateMember {
    pub device: String,
    pub root: String,
}

/// A BasicCredential's identity is cryptographically bound to the hoi root's signed device
/// blessing. A leaf cannot acquire owner powers by merely claiming the owner's root in JSON.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundCredential {
    device: String,
    root: String,
    delegation: String,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PrivateGroup {
    pub community: String,
    pub room: String,
    pub epoch: u64,
    pub active: bool,
    pub is_owner: bool,
    pub members: Vec<PrivateMember>,
    pub pending: usize,
    pub received: Vec<String>,
    /// How the group syncs: `mailboxes` (on holders the group agreed on), or `waiting` (no
    /// mailboxes yet: a relay carries it where the host configured one).
    pub transport: String,
    /// The group's mailboxes and whether this device is linked to each now.
    pub mailboxes: Vec<MailboxView>,
    /// How posts go out now: `pieces` (in pieces through forwarders, crate::keep), or `whole`
    /// (too few places or forwarders are around).
    pub pieces: String,
    /// The level the group's posts kept in pieces here meet, the lowest among them (percent of
    /// places a party would need): what the app's words come from, never the setting alone.
    pub pieces_level: Option<u8>,
    /// Membership follows the community's own records (its admission is by invitation or
    /// approval): the owner's device adds whom they admit and removes whom they ban, and every
    /// member refuses anything else.
    pub follows_records: bool,
    /// Invitations a committing device made for requests, ready to hand to whoever asked.
    pub invitations: Vec<ReadyInvitation>,
    /// The devices that change membership in this epoch, in their order: the owner's,
    /// then moderators' by how long they have been moderators. The first makes each change at
    /// once; when it stays silent, the next takes over after an hour, and so on.
    pub committers: usize,
    /// This device's place among them (0: first in line), if it is one.
    pub rank: Option<usize>,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReadyInvitation {
    pub device: String,
    pub invitation: String,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MailboxView {
    pub endpoint: String,
    pub linked: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CipherObject {
    pub id: String,
    /// The room of the epoch the message was made in: a commit goes to the room it leaves.
    pub room: String,
    pub body: String,
    pub expires: u64,
    pub size: usize,
    pub signature: String,
}

/// Signed relay reads for the current epoch's room, one per requested target, each usable once.
#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PrivateRead {
    pub room: String,
    pub reads: Vec<ReadProof>,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReadProof {
    pub time: u64,
    pub nonce: String,
    pub signature: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JoinRequest {
    v: u8,
    package: String,
    records: Vec<String>,
    /// Where the newcomer's app looks for its invitation, and the key it is sealed to
    /// (crate::pickup).
    pickup: crate::pickup::Pickup,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invitation {
    v: u8,
    welcome: String,
    bootstrap: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum Payload {
    Bootstrap {
        v: u8,
        community: Id,
        records: Vec<String>,
    },
    Records {
        v: u8,
        community: Id,
        records: Vec<String>,
    },
    Media {
        v: u8,
        community: Id,
        hash: Id,
        total: usize,
        index: usize,
        bytes: String,
        /// The posts it is sent for: once they are gone, its object is let go (and a removed
        /// post's dropped) even if other posts use the same item, which then goes again.
        posts: Vec<Id>,
    },
    /// Objects kept in pieces (`hoi_types::scatter::Pointer`, base64): each holds a JSON array
    /// of `records` and `media` payloads. Also a repair's update of one sent before.
    Pointer {
        v: u8,
        community: Id,
        pointers: Vec<String>,
    },
    /// A join request a member passes on to the owner's device, in a group whose membership
    /// follows its records (`hoi:private-request:…`).
    Request {
        v: u8,
        community: Id,
        request: String,
    },
    /// The owner's device's invitation for a request it added: for whoever passes it on.
    Welcome {
        v: u8,
        community: Id,
        device: Id,
        invitation: String,
    },
    /// A committer's device's seal key (X25519, base64), in each epoch it is in line: what the
    /// group's drop key is sealed to for it.
    SealKey {
        v: u8,
        community: Id,
        key: String,
    },
    /// The group's drop key, for the committer's device `to` alone: its public part and its
    /// secret sealed to that device's seal key (base64), from a committer's device that holds
    /// it. Every member receives it; only `to` can open it.
    DropKey {
        v: u8,
        community: Id,
        to: Id,
        key: String,
        sealed: String,
    },
}

/// What a committer does with the group's drop key when it next writes the mailbox list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyWork {
    /// The list keeps its key and the key's holders.
    Keep,
    /// The key's secret goes to these committers' devices, which join its holders.
    Give(Vec<Id>),
    /// A new key, its secret to every committer's device in line.
    Make,
}

/// Requests an owner's device keeps per group, and for how long.
const MAX_REQUESTS: usize = 32;
const REQUEST_TTL: u64 = 7 * 86_400;

/// Posts one media payload names at most.
const MAX_MEDIA_POSTS: usize = 64;

/// At most this many pointers in one payload.
const MAX_POINTERS: usize = 64;

/// One object of outgoing payloads on its way into pieces: its content (a JSON array of
/// payloads), the records it carries and the media items it finishes (hash, chunks).
/// The fourth: the posts the object is kept for (a media object's, the posts using the item).
type Scattered = (Vec<u8>, Vec<Id>, Vec<(Id, usize)>, Vec<Id>);

fn fail() -> ApiError {
    ApiError::new("private-invalid", "This encrypted message could not be authenticated.")
}
fn mls<E: std::fmt::Debug>(_e: E) -> ApiError {
    fail()
}
fn parse(s: &str) -> R<Id> {
    Id::from_hex(s).map_err(Into::into)
}
fn encoded<T: Serialize>(v: &T) -> R<Vec<u8>> {
    serde_json::to_vec(v).map_err(mls)
}
fn decoded<T: serde::de::DeserializeOwned>(v: &[u8]) -> R<T> {
    serde_json::from_slice(v).map_err(mls)
}
fn code<T: serde::de::DeserializeOwned>(v: &str, prefix: &str) -> R<T> {
    if v.len() > 2 * MAX_OBJECT {
        return Err(fail());
    }
    decoded(&unb64(v.strip_prefix(prefix).ok_or_else(fail)?).map_err(mls)?)
}
fn config() -> MlsGroupCreateConfig {
    // Every capability is required of every leaf from the first epoch on.
    let required = Extension::RequiredCapabilities(RequiredCapabilitiesExtension::new(&REQUIRED, &[], &[]));
    MlsGroupCreateConfig::builder()
        .ciphersuite(SUITE)
        .capabilities(profile_capabilities())
        .with_group_context_extensions(Extensions::single(required).expect("a group context extension"))
        .use_ratchet_tree_extension(true)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .max_past_epochs(0)
        .sender_ratchet_configuration(SenderRatchetConfiguration::new(64, 4096))
        .build()
}
fn load(p: &OpenMlsRustCrypto, c: Id) -> R<MlsGroup> {
    MlsGroup::load(p.storage(), &GroupId::from_slice(c.as_bytes())).map_err(mls)?.ok_or_else(|| ApiError::not_found("private keys"))
}
fn member(credential: &Credential, signature_key: Option<&[u8]>) -> R<PrivateMember> {
    if credential.credential_type() != CredentialType::Basic {
        return Err(fail());
    }
    let m: BoundCredential = decoded(credential.serialized_content())?;
    let device = parse(&m.device)?;
    let root = parse(&m.root)?;
    if signature_key.is_some_and(|key| key != device.as_bytes()) {
        return Err(fail());
    }
    let d = hoi_validate::decode(&unb64(&m.delegation).map_err(mls)?)?;
    hoi_validate::signature(&d)?;
    hoi_validate::content(&d.wire.record)?;
    if d.wire.record.author != device || hoi_validate::delegation_consent(&d.wire.record)? != root {
        return Err(fail());
    }
    Ok(PrivateMember { device: m.device, root: m.root })
}

/// What every member of one MLS epoch derives for the relay, and nobody else can: the room key
/// (its public half is the room's address) and the secret that seals each object. It changes
/// with every commit, so a removed device cannot find, read or write the next room.
struct Relay {
    key: Keypair,
    secret: [u8; 32],
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}
fn relay(p: &OpenMlsRustCrypto, g: &MlsGroup) -> R<Relay> {
    let mut exported = g.export_secret(p.crypto(), RELAY_LABEL, &[], 32).map_err(mls)?;
    let secret: [u8; 32] = exported.as_slice().try_into().map_err(mls)?;
    exported.zeroize();
    Ok(Relay::from_secret(secret))
}
impl Relay {
    fn from_secret(secret: [u8; 32]) -> Relay {
        let mut seed = blake3::derive_key("hoi relay v2 room key", &secret);
        let key = Keypair::from_secret(&seed);
        seed.zeroize();
        Relay { key, secret }
    }
    fn room(&self) -> Id {
        self.key.public()
    }
    /// The MLS header (group id = community key, epoch, content type) and the exact length stay
    /// inside: `len (u32 BE) ‖ message ‖ zeros`, sealed to a padded size.
    fn seal(&self, message: &[u8]) -> R<Vec<u8>> {
        let size = padded(message.len() + 4 + SEAL_OVERHEAD);
        if size > MAX_OBJECT {
            return Err(ApiError::new("private-pending", "This encrypted message is too large to send."));
        }
        let mut plain = Vec::with_capacity(size - SEAL_OVERHEAD);
        plain.extend_from_slice(&(message.len() as u32).to_be_bytes());
        plain.extend_from_slice(message);
        plain.resize(size - SEAL_OVERHEAD, 0);
        hoi_crypto::sealed::seal(SEAL_CONTEXT, &self.secret, &plain).map_err(mls)
    }
    fn open(&self, object: &[u8]) -> R<Vec<u8>> {
        if object.len() > MAX_OBJECT || padded(object.len()) != object.len() {
            return Err(fail());
        }
        let plain = hoi_crypto::sealed::open(SEAL_CONTEXT, &self.secret, object).map_err(mls)?;
        let length = u32::from_be_bytes(plain.get(..4).ok_or_else(fail)?.try_into().map_err(mls)?) as usize;
        if length > plain.len() - 4 || plain[4 + length..].iter().any(|b| *b != 0) {
            return Err(fail());
        }
        Ok(plain[4..4 + length].to_vec())
    }
    fn signed(&self, statement: &str) -> String {
        hex_encode(&self.key.sign(statement.as_bytes()).0)
    }
}
/// The smallest size of at least 4 KiB with at most four significant bits that holds `size`.
fn padded(size: usize) -> usize {
    let size = size.max(PAD_MIN);
    let step = 1usize << (usize::BITS - 4 - size.leading_zeros());
    size.div_ceil(step) * step
}
fn object_text(room: Id, id: &str, expires: u64, size: usize) -> String {
    format!("hoi relay object v2\n{}\n{id}\n{expires}\n{size}", room.hex())
}
fn read_text(room: Id, target: &str, time: u64, nonce: &str) -> String {
    format!("hoi relay read v2\n{}\n{target}\n{time}\n{nonce}", room.hex())
}
/// The only requests a read may be signed for: a listing page or one object of the room.
fn read_target(target: &str) -> bool {
    if let Some(after) = target.strip_prefix("objects?after=") {
        return (1..=15).contains(&after.len()) && after.bytes().all(|b| b.is_ascii_digit()) && (after == "0" || !after.starts_with('0'));
    }
    target.strip_prefix("objects/").is_some_and(|id| id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}
fn remember(saved: &mut SavedGroup, id: String) {
    if !saved.received.contains(&id) {
        saved.received.push(id);
    }
    if saved.received.len() > MAX_RECEIVED {
        saved.received.remove(0);
    }
}

impl Node {
    pub(crate) fn private_forget(&mut self, c: Id) -> R<()> {
        if self.private.groups.get(&c).is_some_and(|g| g.owner) {
            return Err(ApiError::not_allowed("Keep the coordinating owner device in this community. Ownership transfer is not available yet."));
        }
        let p = self.private.provider();
        if let Ok(mut group) = load(&p, c) {
            group.delete(p.storage()).map_err(mls)?;
        }
        self.private.groups.remove(&c);
        self.private.passed.remove(&c);
        self.private.seal_keys.remove(&c);
        self.private.announced.remove(&c);
        self.private.save(&p);
        Ok(())
    }
    pub(crate) fn private_writable(&self, c: Id) -> bool {
        self.private.groups.contains_key(&c) && load(&self.private.provider(), c).is_ok_and(|g| g.is_active())
    }
    fn private_signer(&self) -> R<(SignatureKeyPair, CredentialWithKey)> {
        let me = self.state.me.as_ref().ok_or_else(ApiError::no_identity)?;
        let key = self.device_kp.as_ref().ok_or_else(ApiError::no_identity)?;
        let signer = SignatureKeyPair::from_raw(SignatureScheme::ED25519, key.secret().to_vec(), key.public().0.to_vec());
        let delegation = self.store.delegation(&me.device).and_then(|d| self.store.get(&d.record)).ok_or_else(fail)?;
        let credential =
            BasicCredential::new(encoded(&BoundCredential { device: me.device.hex(), root: me.root.hex(), delegation: b64(&delegation.wire) })?).into();
        self.private_sender(&credential)?;
        let with_key = CredentialWithKey { credential, signature_key: me.device.0.to_vec().into() };
        Ok((signer, with_key))
    }
    fn private_sender(&self, credential: &Credential) -> R<PrivateMember> {
        let who = member(credential, None)?;
        let root = parse(&who.root)?;
        let device = parse(&who.device)?;
        let bound: BoundCredential = decoded(credential.serialized_content())?;
        let d = hoi_validate::decode(&unb64(&bound.delegation).map_err(mls)?)?;
        if self.store.burned_at(&root).is_some() || self.store.delegation(&device).is_some_and(|d| d.info.root != root || d.info.revoked_at.is_some()) {
            return Err(ApiError::not_allowed("This device's identity authorization has expired or been revoked."));
        }
        if d.wire.record.expires_at <= self.now() {
            // MLS credentials bind a key, not a permanently frozen authorization lifetime.
            // Only a later root-signed renewal for the same device/root can extend it.
            let renewed = self.store.delegation(&device).and_then(|d| self.store.get(&d.record)).ok_or_else(fail)?;
            self.check_private_renewal(&d.wire.record, &renewed.wire, root, device)?;
        } else {
            hoi_validate::time(&d.wire.record, self.now(), false, None)?;
        }
        Ok(who)
    }
    fn check_private_renewal(&self, original: &hoi_types::Record, bytes: &[u8], root: Id, device: Id) -> R<()> {
        let next = hoi_validate::decode(bytes)?;
        hoi_validate::signature(&next)?;
        hoi_validate::content(&next.wire.record)?;
        hoi_validate::time(&next.wire.record, self.now(), false, None)?;
        hoi_validate::stamp(&next.id, next.wire.record.nonce, hoi_types::GLOBAL_POW_BITS)?;
        if next.wire.record.author != device
            || hoi_validate::delegation_consent(&next.wire.record)? != root
            || next.wire.record.created_at <= original.created_at
            || next.wire.record.expires_at <= original.expires_at
        {
            return Err(fail());
        }
        Ok(())
    }
    fn private_application_sender(&mut self, credential: &Credential, payload: &Payload) -> R<PrivateMember> {
        let who = member(credential, None)?;
        let root = parse(&who.root)?;
        let device = parse(&who.device)?;
        let bound: BoundCredential = decoded(credential.serialized_content())?;
        let original = hoi_validate::decode(&unb64(&bound.delegation).map_err(mls)?)?;
        if original.wire.record.expires_at <= self.now() && self.private_sender(credential).is_err() {
            if self.store.burned_at(&root).is_some() || self.store.delegation(&device).is_some_and(|d| d.info.root != root || d.info.revoked_at.is_some()) {
                return Err(fail());
            }
            if let Payload::Records { records, .. } | Payload::Bootstrap { records, .. } = payload {
                for encoded in records {
                    let bytes = unb64(encoded).map_err(mls)?;
                    if self.check_private_renewal(&original.wire.record, &bytes, root, device).is_ok() {
                        match self.ingest(&bytes, &Source::PrivateRelay) {
                            Ok(_) | Err(hoi_validate::Reject::Duplicate) => {}
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
            }
        }
        self.private_sender(credential)
    }
    fn private_membership_author(&self, c: Id, credential: &Credential) -> R<()> {
        let who = self.private_sender(credential)?;
        if !self.private_committer_root(c, parse(&who.root)?) {
            return Err(ApiError::not_allowed("Only the owner can authorize encrypted membership updates."));
        }
        let bound: BoundCredential = decoded(credential.serialized_content())?;
        let d = hoi_validate::decode(&unb64(&bound.delegation).map_err(mls)?)?;
        let Body::Delegation { scope, .. } = d.wire.record.body else {
            return Err(fail());
        };
        let required = [format!("moderate:{}", c.hex())];
        if !hoi_validate::scope_covers(&scope, &required)
            || self.store.delegation(&parse(&who.device)?).is_some_and(|d| !hoi_validate::scope_covers(&d.info.scope, &required))
        {
            return Err(ApiError::not_allowed("This owner device has no membership management permission."));
        }
        Ok(())
    }
    fn private_owner(&self, c: Id, root: Id) -> bool {
        self.communities.get(&c).is_some_and(|g| g.genesis.threshold == 1 && g.genesis.owners.len() == 1 && g.is_owner(&root))
    }
    /// This device may change the group's membership now: its owner's, or in a group that
    /// follows its records a moderator's whose every member follows moderators' commits, there
    /// through mailboxes (their slots are where an earlier committer's commit would be found).
    fn require_private_committer(&mut self, c: Id) -> R<()> {
        self.renew_delegation();
        let root = self.me_root().ok_or_else(ApiError::no_identity)?;
        if !self.private_committer_root(c, root) {
            return Err(ApiError::not_allowed("Only the community owner can change encrypted membership."));
        }
        let provider = self.private.provider();
        let group = load(&provider, c)?;
        if !group.is_active() {
            return Err(ApiError::not_allowed("This device no longer has active encrypted membership."));
        }
        self.private_membership_author(c, group.credential().map_err(mls)?)?;
        if !self.private_owner(c, root) {
            if self.own_rank(c, &group).is_none() {
                return Err(ApiError::not_allowed("Only the devices of this community's owner and moderators change its membership."));
            }
            if self.private_epoch(c).is_none_or(|e| e.mailboxes.is_empty()) {
                return Err(ApiError::not_allowed("Moderators' devices change membership once the group uses mailboxes."));
            }
        }
        Ok(())
    }
    fn identity_wires(&self, root: Id, device: Id) -> R<Vec<String>> {
        let i = self.store.identity(&root).and_then(|i| self.store.get(&i.record)).ok_or_else(fail)?;
        let d = self.store.delegation(&device).and_then(|d| self.store.get(&d.record)).ok_or_else(fail)?;
        Ok(vec![b64(&i.wire), b64(&d.wire)])
    }
    fn checked_request(&self, request: &str, p: &OpenMlsRustCrypto) -> R<(KeyPackage, PrivateMember, Vec<Vec<u8>>)> {
        let req: JoinRequest = code(request.trim(), "hoi:private-request:")?;
        if req.v != 1 || req.records.len() != 2 || req.pickup.read().is_none() {
            return Err(fail());
        }
        let kp =
            KeyPackageIn::tls_deserialize_exact(unb64(&req.package).map_err(mls)?).map_err(mls)?.validate(p.crypto(), ProtocolVersion::Mls10).map_err(mls)?;
        if kp.ciphersuite() != SUITE {
            return Err(fail());
        }
        // every capability a private group requires, from its first epoch
        if !capable(kp.leaf_node().capabilities()) {
            return Err(ApiError::not_allowed("This request comes from an app too old for private communities: they need to update it and ask again."));
        }
        let who = member(kp.leaf_node().credential(), Some(kp.leaf_node().signature_key().as_slice()))?;
        self.private_sender(kp.leaf_node().credential())?;
        let root = parse(&who.root)?;
        let device = parse(&who.device)?;
        let wires: Vec<Vec<u8>> = req.records.iter().map(|r| unb64(r).map_err(mls)).collect::<R<_>>()?;
        let mut identity = false;
        let mut delegation = false;
        for raw in &wires {
            let d = hoi_validate::decode(raw)?;
            hoi_validate::signature(&d)?;
            hoi_validate::content(&d.wire.record)?;
            hoi_validate::time(&d.wire.record, self.now(), false, None)?;
            match &d.wire.record.body {
                Body::Identity { .. } if d.wire.record.author == root => {
                    hoi_validate::identity(&d.wire.record)?;
                    identity = true;
                }
                Body::Delegation { root: r, expires_at, .. } if *r == root && d.wire.record.author == device && *expires_at > self.now() => {
                    hoi_validate::delegation_consent(&d.wire.record)?;
                    delegation = true;
                }
                _ => return Err(fail()),
            }
        }
        if !identity || !delegation || self.store.delegation(&device).is_some_and(|d| d.info.revoked_at.is_some()) {
            return Err(fail());
        }
        Ok((kp, who, wires))
    }
    /// The pickup key and holders a join request names.
    fn request_pickup(request: &str) -> Option<([u8; 32], Vec<crate::mailbox::Mailbox>)> {
        code::<JoinRequest>(request.trim(), "hoi:private-request:").ok()?.pickup.read()
    }
    /// This device's join request still out: its pickup key and the holders it looks at.
    pub(crate) fn private_pickup(&self) -> Option<(hoi_crypto::envelope::XKey, Vec<crate::mailbox::Mailbox>)> {
        let pending = self.private.pending_request.as_ref().filter(|p| self.now() < p.created.saturating_add(crate::pickup::PICKUP_TTL))?;
        if pending.holders.is_empty() {
            return None;
        }
        let mut seed: [u8; 32] = unb64(&pending.pickup).ok()?.try_into().ok()?;
        let key = hoi_crypto::envelope::XKey::from_seed(&seed).ok();
        seed.zeroize();
        Some((key?, pending.holders.clone()))
    }
    pub fn private_preview_request(&self, request: &str) -> R<PrivateMember> {
        Ok(self.checked_request(request, &self.private.provider())?.1)
    }
    pub fn create_private_community(&mut self, mut draft: CommunityDraft) -> R<CommunityView> {
        if draft.threshold.unwrap_or(1) != 1 || draft.co_owners.as_ref().is_some_and(|v| !v.is_empty()) {
            return Err(ApiError::not_allowed("Encrypted membership currently requires one owner."));
        }
        self.renew_delegation();
        let (signer, credential) = self.private_signer()?;
        let rules = draft.rules.take();
        let view = self.create_community_with_privacy(draft, true)?;
        let c = parse(&view.id)?;
        let p = self.private.provider();
        MlsGroup::new_with_group_id(&p, &signer, &config(), GroupId::from_slice(c.as_bytes()), credential).map_err(mls)?;
        self.private.groups.insert(c, SavedGroup::new(true));
        self.private.save(&p);
        self.state.registration_proofs.remove(&c);
        self.state.publication_proofs.remove(&c);
        self.mark_state();
        if let Some(rules) = rules.filter(|r| !r.trim().is_empty()) {
            self.set_rules(&view.id, &rules)?;
        }
        self.community(&view.id)
    }
    pub fn private_groups(&self) -> R<Vec<PrivateGroup>> {
        self.state
            .private_communities
            .iter()
            .chain(self.private.groups.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|c| self.private.groups.contains_key(c) || self.store.genesis_of(c).is_some())
            .map(|c| self.private_group(&c.hex()))
            .collect()
    }
    pub fn private_group(&self, community: &str) -> R<PrivateGroup> {
        let c = parse(community)?;
        if !self.is_private_community(&c) {
            return Err(ApiError::not_found("private community"));
        }
        let locked = |room: String, pending: usize| PrivateGroup {
            community: c.hex(),
            room,
            epoch: 0,
            active: false,
            is_owner: false,
            members: vec![],
            pending,
            received: vec![],
            transport: "waiting".into(),
            mailboxes: vec![],
            pieces: "whole".into(),
            pieces_level: None,
            follows_records: false,
            invitations: vec![],
            committers: 0,
            rank: None,
        };
        let Some(s) = self.private.groups.get(&c) else {
            return Ok(locked(String::new(), 0));
        };
        let p = self.private.provider();
        let Ok(g) = load(&p, c) else {
            return Ok(locked(String::new(), s.objects.len()));
        };
        let members = g.members().map(|m| member(&m.credential, Some(&m.signature_key))).collect::<R<Vec<_>>>()?;
        let ep = self.epoch_of(&p, &g, s).ok();
        let linked: BTreeSet<Id> = self.mail_links().into_keys().collect();
        let transport = if ep.as_ref().is_some_and(|e| !e.mailboxes.is_empty()) { "mailboxes" } else { "waiting" };
        let mailboxes = ep
            .map(|e| e.mailboxes.iter().map(|m| MailboxView { endpoint: m.endpoint.hex(), linked: linked.contains(&m.endpoint) }).collect())
            .unwrap_or_default();
        let pieces = if self.can_scatter(c) { "pieces" } else { "whole" };
        let invitations = self
            .private
            .invitations
            .get(&c)
            .map(|m| m.iter().map(|(device, invitation)| ReadyInvitation { device: device.clone(), invitation: invitation.clone() }).collect())
            .unwrap_or_default();
        let line = if g.is_active() && self.follows_records(c) { self.committers(c, &g) } else { vec![] };
        let me = self.state.me.as_ref().map(|m| m.device);
        Ok(PrivateGroup {
            committers: line.len(),
            rank: line.iter().position(|x| Some(x.device) == me),
            follows_records: self.follows_records(c),
            invitations,
            pieces: pieces.into(),
            pieces_level: self.pieces_level(c),
            community: c.hex(),
            room: if g.is_active() { relay(&p, &g)?.room().hex() } else { String::new() },
            epoch: g.epoch().as_u64(),
            active: g.is_active(),
            is_owner: s.owner && self.me_root().is_some_and(|r| self.private_owner(c, r)),
            members,
            pending: s.objects.len(),
            received: s.received.clone(),
            transport: transport.into(),
            mailboxes,
        })
    }
    /// The private groups this device holds MLS state for.
    pub(crate) fn private_group_ids(&self) -> Vec<Id> {
        self.private.groups.keys().copied().collect()
    }
    /// The current epoch of group `c` as its mailboxes see it.
    pub(crate) fn private_epoch(&self, c: Id) -> Option<MailEpoch> {
        let s = self.private.groups.get(&c)?;
        let p = self.private.provider();
        let g = load(&p, c).ok()?;
        self.epoch_of(&p, &g, s).ok()
    }
    fn epoch_of(&self, p: &OpenMlsRustCrypto, g: &MlsGroup, s: &SavedGroup) -> R<MailEpoch> {
        let relay = relay(p, g)?;
        let c = Id::from_slice(g.group_id().as_slice()).unwrap_or(Id::ZERO);
        Ok(MailEpoch {
            rank: if g.is_active() && self.follows_records(c) { self.own_rank(c, g) } else { None },
            epoch: g.epoch().as_u64(),
            room: relay.room().hex(),
            own: g.own_leaf_index().u32(),
            leaves: g.members().map(|m| m.index.u32()).collect(),
            slot_key: crate::mailbox::slot_key(&relay.secret),
            mailboxes: g.extensions().unknown(MAILBOX_EXTENSION).map(|u| crate::mailbox::decode_list(&u.0)).unwrap_or_default(),
            drop_key: g.extensions().unknown(MAILBOX_EXTENSION).and_then(|u| crate::mailbox::decode_drop_key(&u.0)),
            holders: g.extensions().unknown(MAILBOX_EXTENSION).map(|u| crate::mailbox::decode_holders(&u.0)).unwrap_or_default(),
            owner: s.owner && self.me_root().is_some_and(|r| self.private_owner(c, r)),
            members: g.members().count(),
            active: g.is_active(),
        })
    }
    /// Messages of group `c` waiting to go out.
    pub(crate) fn private_pending(&self, c: Id) -> Vec<CipherObject> {
        self.private.groups.get(&c).map(|s| s.objects.clone()).unwrap_or_default()
    }
    /// Whether a membership change may be made now: nothing is waiting to go out, or everything
    /// waiting already has its place in this device's mailbox slots, ahead of the change.
    fn ready_for_commit(&self, c: Id, saved: &SavedGroup) -> bool {
        saved.objects.is_empty() || self.mailbox_ordered(c, &saved.objects)
    }
    /// The owner puts the group's mailboxes into the group context, by a commit that goes to
    /// the epoch it ends (its mailboxes, or a relay; a group of one has nobody to tell).
    pub(crate) fn private_set_mailboxes(&mut self, c: Id, list: Vec<crate::mailbox::Mailbox>, work: KeyWork) -> R<()> {
        self.require_private_committer(c)?;
        let p = self.private.provider();
        let mut g = load(&p, c)?;
        let mut saved = self.private.groups.get(&c).cloned().ok_or_else(fail)?;
        if !self.ready_for_commit(c, &saved) && g.members().count() > 1 {
            return Err(ApiError::new("private-pending", "Synchronize pending encrypted data first."));
        }
        let (signer, _) = self.private_signer()?;
        // the group requires the mailbox capability of every leaf since its creation
        let mut extensions = g.extensions().clone();
        // The drop key: kept, given to committers' devices that lack it, or a new one
        // for every committer's device whose seal key this device heard; sealed to each, it goes
        // ahead of the commit, in the epoch the commit ends.
        let listed = g.extensions().unknown(MAILBOX_EXTENSION).and_then(|u| crate::mailbox::decode_drop_key(&u.0));
        let holders = g.extensions().unknown(MAILBOX_EXTENSION).map(|u| crate::mailbox::decode_holders(&u.0)).unwrap_or_default();
        let me = self.state.me.as_ref().ok_or_else(ApiError::no_identity)?.device;
        let line: Vec<Committer> = if self.follows_records(c) { self.committers(c, &g) } else { vec![] };
        let sealable: Vec<(Committer, [u8; 32])> =
            line.iter().filter(|x| x.device != me).filter_map(|x| Some((x.clone(), self.seal_key_of(c, x.device)?))).collect();
        let work = match (listed, work) {
            (None, _) => KeyWork::Make,
            (Some(d), KeyWork::Give(_)) if self.held_drop_key(c, &d).is_none() => KeyWork::Keep,
            (_, w) => w,
        };
        let (drop_key, holders, give, mut secret) = match (work, listed) {
            (KeyWork::Give(to), Some(d)) => {
                let give: Vec<(Committer, [u8; 32])> = sealable.into_iter().filter(|(x, _)| to.contains(&x.device)).collect();
                let mut h: Vec<Id> = holders.into_iter().filter(|x| line.iter().any(|l| l.device == *x)).collect();
                for x in [me].into_iter().chain(give.iter().map(|(x, _)| x.device)) {
                    if !h.contains(&x) {
                        h.push(x);
                    }
                }
                (d, h, give, self.held_drop_key(c, &d))
            }
            (KeyWork::Make, _) => {
                let kp = hoi_crypto::Keypair::generate()?;
                let h: Vec<Id> = [me].into_iter().chain(sealable.iter().map(|(x, _)| x.device)).collect();
                (kp.public().0, h, sealable, Some(kp.secret()))
            }
            (_, d) => (d.ok_or_else(fail)?, holders, vec![], None),
        };
        let made = listed != Some(drop_key);
        extensions
            .add_or_replace(Extension::Unknown(MAILBOX_EXTENSION, UnknownExtension(crate::mailbox::encode_list(&list, Some(drop_key), &holders))))
            .map_err(mls)?;
        let leaving = relay(&p, &g)?;
        if let Some(key) = secret.filter(|_| !give.is_empty()) {
            // what makes the giver and each device given the key committers, ahead of it: a
            // moderator appointed a moment ago may not hold its own `mod-add` yet
            let roots: BTreeSet<Id> = give.iter().map(|(x, _)| x.root).chain(self.me_root()).filter(|r| !self.private_owner(c, *r)).collect();
            let mut proof: BTreeMap<Id, Vec<u8>> = BTreeMap::new();
            for root in roots {
                for wire in self.role_proof(c, root) {
                    if let Ok(d) = hoi_validate::decode(&wire) {
                        proof.insert(d.id, wire);
                    }
                }
            }
            self.queue_backing(&p, &mut g, &signer, &mut saved, &leaving, c, proof.into_iter().collect())?;
            for (x, xk) in &give {
                let sealed = hoi_crypto::envelope::seal_drop_key(xk, &drop_key_aad(c, x.device, &drop_key), &key).map_err(|_| fail())?;
                let payload = Payload::DropKey { v: 1, community: c, to: x.device, key: b64(&drop_key), sealed: b64(&sealed) };
                self.queue_payload(&p, &mut g, &signer, &mut saved, &leaving, &payload)?;
            }
        }
        let before = self.epoch_of(&p, &g, &saved)?;
        let contest = self.contest_point(&p, &g, c, before.rank);
        let (commit, _, _) = g.update_group_context_extensions(&p, extensions, &signer).map_err(mls)?;
        self.queue_private(&mut saved, &leaving, &commit.tls_serialize_detached().map_err(mls)?)?;
        self.assign_slots(c, &before, &saved.objects);
        g.merge_pending_commit(&p).map_err(mls)?;
        // nobody new: everything this device queued went ahead of the commit, members read it
        self.private.groups.insert(c, saved);
        self.private.save(&p);
        self.keep_contest(c, contest);
        if made && let Some(key) = secret {
            self.hold_drop_key(c, key);
        }
        secret.zeroize();
        Ok(())
    }
    pub fn private_join_request(&mut self) -> R<String> {
        self.renew_delegation();
        let (signer, credential) = self.private_signer()?;
        let p = self.private.provider();
        if let Some(pending) = &self.private.pending_request {
            if pending.created.saturating_add(7 * 86400) > self.now() {
                return Ok(pending.code.clone());
            }
            p.storage().delete_key_package(&pending.key_ref).map_err(mls)?;
        }
        let package = KeyPackage::builder().leaf_node_capabilities(profile_capabilities()).build(SUITE, &p, &signer, credential).map_err(mls)?;
        let me = self.state.me.as_ref().ok_or_else(ApiError::no_identity)?;
        // a one-time key, and the holders this device will look for the invitation on
        let mut seed = hoi_crypto::random_bytes::<32>().map_err(mls)?;
        let key = hoi_crypto::envelope::XKey::from_seed(&seed).map_err(mls)?;
        let holders = self.pickup_holders();
        let request = JoinRequest {
            v: 1,
            package: b64(&package.key_package().tls_serialize_detached().map_err(mls)?),
            records: self.identity_wires(me.root, me.device)?,
            pickup: crate::pickup::Pickup { key: b64(&key.public()), holders: holders.clone() },
        };
        let code = format!("hoi:private-request:{}", b64(&encoded(&request)?));
        self.private.pending_request = Some(PendingRequest {
            code: code.clone(),
            created: self.now(),
            key_ref: package.key_package().hash_ref(p.crypto()).map_err(mls)?,
            pickup: b64(&seed),
            holders,
        });
        seed.zeroize();
        self.private.save(&p);
        Ok(code)
    }
    /// Seals an MLS message for the room of the epoch `relay` came from and signs the upload with
    /// that room's key, so the relay learns which room but not which member. Returns the object.
    fn queue_private(&self, saved: &mut SavedGroup, relay: &Relay, message: &[u8]) -> R<Vec<u8>> {
        if saved.objects.len() >= MAX_PENDING {
            return Err(ApiError::new("private-pending", "Synchronize pending encrypted data first."));
        }
        let sealed = relay.seal(message)?;
        let id = hex_encode(&Sha256::digest(&sealed));
        let expires = self.now() + RETENTION;
        let room = relay.room();
        let object = CipherObject {
            id: id.clone(),
            room: room.hex(),
            size: sealed.len(),
            body: b64(&sealed),
            expires,
            signature: relay.signed(&object_text(room, &id, expires, sealed.len())),
        };
        remember(saved, id);
        saved.objects.push(object);
        Ok(sealed)
    }
    fn queue_payload(
        &self,
        p: &OpenMlsRustCrypto,
        g: &mut MlsGroup,
        signer: &SignatureKeyPair,
        s: &mut SavedGroup,
        relay: &Relay,
        payload: &Payload,
    ) -> R<Vec<u8>> {
        let raw = encoded(payload)?;
        let message = g.create_message(p, signer, &raw).map_err(mls)?.tls_serialize_detached().map_err(mls)?;
        self.queue_private(s, relay, &message)
    }
    fn queue_membership_renewal(&self, p: &OpenMlsRustCrypto, g: &mut MlsGroup, signer: &SignatureKeyPair, saved: &mut SavedGroup, c: Id) -> R<()> {
        let credential = g.credential().map_err(mls)?;
        let bound: BoundCredential = decoded(credential.serialized_content())?;
        let original = hoi_validate::decode(&unb64(&bound.delegation).map_err(mls)?)?;
        if original.wire.record.expires_at <= self.now() {
            self.private_membership_author(c, credential)?;
            let who = member(credential, None)?;
            let records = self.identity_wires(parse(&who.root)?, parse(&who.device)?)?;
            let relay = relay(p, g)?;
            self.queue_payload(p, g, signer, saved, &relay, &Payload::Records { v: 1, community: c, records })?;
        }
        Ok(())
    }
    pub fn private_invite(&mut self, community: &str, request: &str) -> R<String> {
        let c = parse(community)?;
        self.require_private_committer(c)?;
        // Where membership follows the records, an owner's or moderator's choice goes in the ledger first,
        // as an `admit` every member sees before the commit that follows it.
        let mut admit = None;
        if self.follows_records(c) {
            let p = self.private.provider();
            let (_, who, _) = self.checked_request(request, &p)?;
            drop(p);
            let root = parse(&who.root)?;
            let st = self.communities.get(&c).ok_or_else(fail)?;
            if st.is_banned(&root) {
                return Err(ApiError::not_allowed("This community banned this person."));
            }
            if !st.is_admitted(&root) {
                let entry = self.admit(community, &who.root)?;
                admit = Some(parse(&entry.id)?);
            }
        }
        let p = self.private.provider();
        let (kp, who, carriers) = self.checked_request(request, &p)?;
        let mut g = load(&p, c)?;
        if g.members().count() >= 128 || g.members().any(|m| m.signature_key == parse(&who.device).unwrap().0) {
            return Err(ApiError::not_allowed("This device is already a member, or the community is full."));
        }
        let me_owner = self.private_owner(c, self.me_root().ok_or_else(ApiError::no_identity)?);
        let mut saved = self.private.groups.get(&c).cloned().ok_or_else(fail)?;
        if !self.ready_for_commit(c, &saved) {
            return Err(ApiError::new("private-pending", "Synchronize pending encrypted data before inviting someone."));
        }
        let (signer, _) = self.private_signer()?;
        self.queue_membership_renewal(&p, &mut g, &signer, &mut saved, c)?;
        // The commit is read by members still in this epoch, so it goes to the room it leaves.
        let leaving = relay(&p, &g)?;
        if self.follows_records(c) {
            // who it is and what admits them, ahead of the commit (an `admit` just made included)
            let _ = admit;
            let mut backing = self.backing_records(c, parse(&who.root)?, parse(&who.device)?, true);
            if backing.len() < 2 {
                backing.extend(carriers.iter().filter_map(|w| hoi_validate::decode(w).ok().map(|d| (d.id, w.clone()))));
            }
            self.queue_backing(&p, &mut g, &signer, &mut saved, &leaving, c, backing)?;
        }
        let before = self.epoch_of(&p, &g, &saved)?;
        let contest = self.contest_point(&p, &g, c, before.rank);
        let (commit, welcome, _) = g.add_members(&p, &signer, &[kp]).map_err(mls)?;
        self.queue_private(&mut saved, &leaving, &commit.tls_serialize_detached().map_err(mls)?)?;
        self.assign_slots(c, &before, &saved.objects);
        g.merge_pending_commit(&p).map_err(mls)?;
        self.forget_passed(c, &g);
        let me = self.state.me.as_ref().ok_or_else(ApiError::no_identity)?;
        let mut records = self.identity_wires(me.root, me.device)?;
        records.push(b64(&self.store.genesis_of(&c).ok_or_else(fail)?.wire));
        if !me_owner {
            // what makes this device's person a moderator, for a newcomer who holds nothing yet
            records.extend(self.role_proof(c, me.root).iter().map(|w| b64(w)));
        }
        let entering = relay(&p, &g)?;
        let bootstrap = self.queue_payload(&p, &mut g, &signer, &mut saved, &entering, &Payload::Bootstrap { v: 2, community: c, records })?;
        self.private_ingest(c, carriers)?;
        // the newcomer reads only from this epoch on: the history goes again, from this device
        saved.send_all_again();
        let after = self.epoch_of(&p, &g, &saved)?;
        self.assign_slots(c, &after, &saved.objects);
        self.private.groups.insert(c, saved);
        self.private.save(&p);
        self.keep_contest(c, contest);
        let invitation = Invitation { v: 2, welcome: b64(&welcome.tls_serialize_detached().map_err(mls)?), bootstrap: b64(&bootstrap) };
        let invitation = format!("hoi:private:{}", b64(&encoded(&invitation)?));
        // stored for the newcomer's app to pick up by itself
        if let Some((key, holders)) = Self::request_pickup(request) {
            self.queue_pickup(key, holders, &invitation);
        }
        Ok(invitation)
    }
    pub fn private_join(&mut self, invitation: &str) -> R<CommunityView> {
        let inv: Invitation = code(invitation.trim(), "hoi:private:")?;
        if inv.v != 2 {
            return Err(fail());
        }
        let p = self.private.provider();
        let incoming = MlsMessageIn::tls_deserialize_exact(unb64(&inv.welcome).map_err(mls)?).map_err(mls)?;
        let MlsMessageBodyIn::Welcome(welcome) = incoming.extract() else {
            return Err(fail());
        };
        let processed = ProcessedWelcome::new_from_welcome(&p, config().join_config(), welcome).map_err(mls)?;
        let c = Id::from_slice(processed.unverified_group_info().group_id().as_slice()).map_err(mls)?;
        let known = self.state.joined.contains_key(&c) || self.store.genesis_of(&c).is_some();
        let already_active = self.private.groups.contains_key(&c) && load(&self.private.provider(), c).is_ok_and(|g| g.is_active());
        // A device that joined lately takes a newer invitation: the commit that added it may have
        // lost to an earlier committer's, and it asked again.
        let rejoin = already_active
            && self.is_private_community(&c)
            && self.follows_records(c)
            && !self.private.groups.get(&c).is_some_and(|s| s.owner)
            && self.state.joined.get(&c).is_some_and(|j| self.now() < j.at.saturating_add(CONTEST));
        if (already_active && !rejoin) || (known && !self.is_private_community(&c)) {
            return Err(ApiError::not_allowed("This community already exists on this device. Public communities cannot become private."));
        }
        // Work only on the temporary provider. Rejoining must remove the old inactive MLS
        // group before staging a fresh Welcome; no durable state changes until all checks pass.
        if let Ok(mut old) = load(&p, c) {
            old.delete(p.storage()).map_err(mls)?;
        }
        let mut g = processed.into_staged_welcome(&p, None).map_err(mls)?.into_group(&p).map_err(mls)?;
        for m in g.members() {
            member(&m.credential, Some(&m.signature_key))?;
        }
        let sealed = unb64(&inv.bootstrap).map_err(mls)?;
        let raw = relay(&p, &g)?.open(&sealed)?;
        let msg = MlsMessageIn::tls_deserialize_exact(&raw).map_err(mls)?.try_into_protocol_message().map_err(mls)?;
        if msg.wire_format() != WireFormat::PrivateMessage || msg.group_id().as_slice() != c.as_bytes() {
            return Err(fail());
        }
        let processed = g.process_message(&p, msg).map_err(mls)?;
        let credential = processed.credential().clone();
        let ProcessedMessageContent::ApplicationMessage(app) = processed.into_content() else {
            return Err(fail());
        };
        let payload: Payload = decoded(&app.into_bytes())?;
        let sender = self.private_application_sender(&credential, &payload)?;
        let Payload::Bootstrap { v: 2, community, records } = payload else {
            return Err(fail());
        };
        if community != c {
            return Err(fail());
        }
        let wires = decode_records(c, &records)?;
        let genesis = wires.iter().filter_map(|r| hoi_validate::decode(r).ok()).find(|d| d.wire.record.kind() == Kind::Community).ok_or_else(fail)?;
        let Body::Community(genesis_body) = &genesis.wire.record.body else {
            return Err(fail());
        };
        let sender_root = parse(&sender.root)?;
        if genesis_body.threshold != 1 || genesis_body.owners.len() != 1 {
            return Err(fail());
        }
        // From the owner's device, or a moderator's that shows what made them one.
        let by_owner = genesis_body.owners[0] == sender_root;
        if by_owner {
            self.validate_private_bootstrap(c, &sender, &wires)?;
        } else {
            self.validate_moderator_bootstrap(c, &sender, genesis_body.owners[0], &wires)?;
        }
        let existing_records: BTreeSet<Id> = self.store.in_community(&c).map(|s| s.id).collect();
        self.state.private_communities.insert(c);
        let now = self.now();
        if rejoin {
            // the day to take a newer invitation runs from the first join
            self.state.joined.entry(c).or_insert(Joined { at: now, pinning: false });
        } else {
            self.state.joined.insert(c, Joined { at: now, pinning: false });
        }
        self.mark_state();
        let ingested = self.private_ingest(c, wires).and_then(|_| if by_owner || self.private_moderator(c, sender_root) { Ok(()) } else { Err(fail()) });
        if let Err(error) = ingested {
            let ids: Vec<Id> = self.store.in_community(&c).map(|s| s.id).filter(|id| !existing_records.contains(id)).collect();
            for id in ids {
                self.store.remove(&id);
                self.push_change(hoi_store::Change::Delete { id });
            }
            if !rejoin {
                self.state.joined.remove(&c);
                self.communities.remove(&c);
            }
            self.mark_state();
            return Err(error);
        }
        if rejoin {
            self.mail_restart(c);
            self.private.contests.remove(&c);
        }
        let saved = SavedGroup { received: vec![hex_encode(&Sha256::digest(&sealed))], ..SavedGroup::new(false) };
        self.private.groups.insert(c, saved);
        self.private.pending_request = None;
        self.private.save(&p);
        self.community(&c.hex())
    }
    pub fn private_remove(&mut self, community: &str, device: &str) -> R<PrivateGroup> {
        let c = parse(community)?;
        let device = parse(device)?;
        self.require_private_committer(c)?;
        let p = self.private.provider();
        let mut g = load(&p, c)?;
        let target = g.members().find(|m| m.signature_key == device.0).ok_or_else(|| ApiError::not_found("member"))?;
        if device == self.state.me.as_ref().ok_or_else(ApiError::no_identity)?.device {
            return Err(ApiError::not_allowed("Keep this device in the community while managing other devices."));
        }
        if self.follows_records(c) {
            let who = member(&target.credential, None)?;
            if !self.excluded(c, parse(&who.root)?, device) {
                return Err(ApiError::not_allowed("Membership here follows the community's records: ban this person first, and their devices leave."));
            }
        }
        let (signer, _) = self.private_signer()?;
        let mut saved = self.private.groups.get(&c).cloned().ok_or_else(fail)?;
        if !self.ready_for_commit(c, &saved) {
            return Err(ApiError::new("private-pending", "Synchronize pending encrypted data before removing a member."));
        }
        self.queue_membership_renewal(&p, &mut g, &signer, &mut saved, c)?;
        // The removed device reads this commit in the room it leaves and cannot derive the next.
        let leaving = relay(&p, &g)?;
        if self.follows_records(c) {
            let who = member(&target.credential, None)?;
            let backing = self.backing_records(c, parse(&who.root)?, device, false);
            self.queue_backing(&p, &mut g, &signer, &mut saved, &leaving, c, backing)?;
        }
        let before = self.epoch_of(&p, &g, &saved)?;
        let contest = self.contest_point(&p, &g, c, before.rank);
        let (commit, _, _) = g.remove_members(&p, &signer, &[target.index]).map_err(mls)?;
        self.queue_private(&mut saved, &leaving, &commit.tls_serialize_detached().map_err(mls)?)?;
        self.assign_slots(c, &before, &saved.objects);
        g.merge_pending_commit(&p).map_err(mls)?;
        // nobody new, and everything queued went ahead of the commit: nothing goes again
        self.private.groups.insert(c, saved);
        self.private.save(&p);
        self.keep_contest(c, contest);
        self.private_group(community)
    }
    pub fn private_outbox(&mut self, community: &str) -> R<Vec<CipherObject>> {
        self.renew_delegation();
        let c = parse(community)?;
        let p = self.private.provider();
        let mut g = load(&p, c)?;
        let mut saved = self.private.groups.get(&c).cloned().ok_or_else(fail)?;
        if !g.is_active() {
            return Ok(saved.objects);
        }
        let (signer, _) = self.private_signer()?;
        let relay = relay(&p, &g)?;
        if saved.objects.iter().any(|o| o.expires <= self.now()) {
            saved.objects.retain(|o| o.expires > self.now());
            saved.send_all_again();
        }
        let mut candidates = BTreeMap::new();
        // each media item, with the posts that use it
        let mut media: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
        // What may go in pieces: posts, comments and their edits.
        let mut content = BTreeSet::new();
        // A device's authorization renewal must travel even in an otherwise empty group.
        let me = self.state.me.as_ref().ok_or_else(ApiError::no_identity)?;
        for wire in self.identity_wires(me.root, me.device)? {
            let bytes = unb64(&wire).map_err(mls)?;
            let d = hoi_validate::decode(&bytes)?;
            candidates.insert((0, d.wire.record.created_at, d.id), bytes);
        }
        for s in self.store.in_community(&c).filter(|s| self.forwardable(s)) {
            for r in self.carriers(s) {
                candidates.insert((0, r.record.created_at, r.id), r.wire.clone());
            }
            candidates.insert((if s.record.kind() == Kind::Community { 1 } else { 2 }, s.record.created_at, s.id), s.wire.clone());
            if matches!(s.record.kind(), Kind::Post | Kind::Comment | Kind::Edit) {
                content.insert(s.id);
            }
            if let Body::Post { media: refs, .. } = &s.record.body {
                for r in refs {
                    media.entry(r.hash).or_default().push(s.id);
                }
            }
        }
        let mut batch = vec![];
        let live: BTreeSet<Id> = candidates.keys().map(|(_, _, id)| *id).collect();
        saved.sent_records.retain(|id| live.contains(id));
        saved.sent_media.retain(|id, _| media.contains_key(id));
        saved.sent_in.retain(|id, _| live.contains(id) || media.contains_key(id));
        saved.partial.retain(|id, _| media.contains_key(id));
        // Posts, comments and edits go in pieces, one object each, when enough places and
        // forwarders are around (crate::keep); everything else
        // (votes, deletions, profiles, a device's own authorization records) goes whole, so a
        // renewed member is recognised before any pointer from it.
        let scatter = self.can_scatter(c);
        let busy_records = self.scatter.repair.inflight_records.get(&c).cloned().unwrap_or_default();
        let busy_media = self.scatter.repair.inflight_media.get(&c).cloned().unwrap_or_default();
        let mut scattered: Vec<Scattered> = Vec::new();
        let mut ids = vec![];
        let mut size = 0;
        for ((_, _, id), wire) in candidates {
            if saved.sent_records.contains(&id) || busy_records.contains(&id) {
                continue;
            }
            if scatter && content.contains(&id) {
                let payload = Payload::Records { v: 1, community: c, records: vec![b64(&wire)] };
                scattered.push((encoded(&vec![payload])?, vec![id], vec![], vec![id]));
                continue;
            }
            if size + wire.len() > CHUNK && !batch.is_empty() {
                if saved.objects.len() >= MAX_PENDING {
                    break;
                }
                self.queue_payload(&p, &mut g, &signer, &mut saved, &relay, &Payload::Records { v: 1, community: c, records: std::mem::take(&mut batch) })?;
                saved.sent(std::mem::take(&mut ids), g.epoch().as_u64());
                size = 0;
            }
            if saved.objects.len() >= MAX_PENDING {
                break;
            }
            size += wire.len();
            batch.push(b64(&wire));
            ids.push(id);
        }
        if !batch.is_empty() && saved.objects.len() < MAX_PENDING {
            self.queue_payload(&p, &mut g, &signer, &mut saved, &relay, &Payload::Records { v: 1, community: c, records: batch })?;
            saved.sent(ids, g.epoch().as_u64());
        }
        // Join requests this device passed on in an epoch since ended go again, until their
        // device is in the group: committers that moved on with a commit never read that epoch.
        let (now, epoch) = (self.now(), g.epoch().as_u64());
        let in_group: BTreeSet<Vec<u8>> = g.members().map(|m| m.signature_key.clone()).collect();
        let mut again = Vec::new();
        if let Some(list) = self.private.passed.get_mut(&c) {
            list.retain(|(_, device, at, _)| now < at + REQUEST_TTL && !in_group.contains(device.0.as_slice()));
            for (request, _, _, sent_in) in list.iter_mut().filter(|x| x.3 != epoch) {
                again.push(request.clone());
                *sent_in = epoch;
                self.private.dirty = true;
            }
        }
        for request in again {
            if saved.objects.len() >= MAX_PENDING {
                break;
            }
            self.queue_payload(&p, &mut g, &signer, &mut saved, &relay, &Payload::Request { v: 1, community: c, request })?;
        }
        // A committer's device announces its seal key in each epoch it is in line, where
        // moderators' devices commit too: what the group's drop key is sealed to for it.
        let mailboxes = g.extensions().unknown(MAILBOX_EXTENSION).is_some_and(|u| !crate::mailbox::decode_list(&u.0).is_empty());
        if mailboxes
            && self.follows_records(c)
            && self.private.announced.get(&c) != Some(&epoch)
            && saved.objects.len() < MAX_PENDING
            && self.own_rank(c, &g).is_some()
            && let Some(key) = self.seal_key()
        {
            self.queue_payload(&p, &mut g, &signer, &mut saved, &relay, &Payload::SealKey { v: 1, community: c, key: b64(&key.public()) })?;
            self.private.announced.insert(c, epoch);
            self.private.dirty = true;
        }
        for (hash, mut posts) in media {
            posts.truncate(MAX_MEDIA_POSTS);
            if busy_media.contains(&hash) {
                continue;
            }
            let Some(bytes) = self.store.blob(&hash) else {
                continue;
            };
            let start = *saved.sent_media.get(&hash).unwrap_or(&0);
            if scatter {
                // the item in raw slices, each object as full as a class it fits
                let total = bytes.len().div_ceil(CHUNK);
                for content in media_contents(c, hash, bytes, &posts) {
                    scattered.push((content, vec![], vec![(hash, total)], posts.clone()));
                }
                continue;
            }
            for (index, part) in bytes.chunks(CHUNK).enumerate().skip(start) {
                if saved.objects.len() >= MAX_PENDING {
                    break;
                }
                let payload = Payload::Media { v: 1, community: c, hash, total: bytes.len(), index, bytes: b64(part), posts: posts.clone() };
                self.queue_payload(&p, &mut g, &signer, &mut saved, &relay, &payload)?;
                saved.sent_media_to(hash, index + 1, g.epoch().as_u64());
            }
        }
        if let Ok(ep) = self.epoch_of(&p, &g, &saved) {
            self.assign_slots(c, &ep, &saved.objects);
        }
        let out = saved.objects.clone();
        self.private.groups.insert(c, saved);
        self.private.save(&p);
        if !scattered.is_empty() {
            // Started only now that the group's state is saved: what finishes at once sends its
            // pointers (or the payloads whole) from the saved state.
            let records: Vec<Id> = scattered.iter().flat_map(|s| s.1.iter().copied()).collect();
            let mut items: Vec<(Id, usize)> = scattered.iter().flat_map(|s| s.2.iter().copied()).collect();
            items.dedup();
            let contents: Vec<Vec<u8>> = scattered.iter().map(|s| s.0.clone()).collect();
            let parts = scattered
                .into_iter()
                .map(|(content, _, media, carries)| crate::keep::OutPart { content, records: carries, media: media.first().map(|(h, _)| *h) })
                .collect();
            let expires_at = self.now() + RETENTION;
            if !self.scatter_outgoing(c, parts, records.clone(), items.clone(), expires_at) {
                self.send_whole(c, contents, records, items);
            }
            return Ok(self.private.groups.get(&c).map(|s| s.objects.clone()).unwrap_or(out));
        }
        Ok(out)
    }

    /// Queues payloads made outside the outbox into the group's current epoch: pointers to
    /// objects kept in pieces, or what could not be kept so. `records` and `media` count as sent.
    fn queue_now(&mut self, c: Id, payloads: &[Payload], records: &[Id], media: &[(Id, usize)]) -> R<()> {
        let p = self.private.provider();
        let mut g = load(&p, c)?;
        let mut saved = self.private.groups.get(&c).cloned().ok_or_else(fail)?;
        if !g.is_active() {
            return Ok(());
        }
        if saved.objects.len() + payloads.len() > MAX_PENDING {
            return Err(ApiError::new("private-pending", "Synchronize pending encrypted data first."));
        }
        let (signer, _) = self.private_signer()?;
        let relay = relay(&p, &g)?;
        for payload in payloads {
            self.queue_payload(&p, &mut g, &signer, &mut saved, &relay, payload)?;
        }
        let epoch = g.epoch().as_u64();
        saved.sent(records.iter().copied(), epoch);
        for (hash, total) in media {
            saved.sent_media_to(*hash, *total, epoch);
        }
        if let Ok(ep) = self.epoch_of(&p, &g, &saved) {
            self.assign_slots(c, &ep, &saved.objects);
        }
        self.private.groups.insert(c, saved);
        self.private.save(&p);
        Ok(())
    }

    /// Sends pointers to the group (crate::keep): new objects, or a repair's update.
    pub(crate) fn send_pointers(&mut self, c: Id, pointers: Vec<hoi_types::scatter::Pointer>, records: Vec<Id>, media: Vec<(Id, usize)>) {
        let encoded: Vec<String> = pointers.iter().map(|p| b64(&p.encode())).collect();
        let payloads: Vec<Payload> = encoded.chunks(MAX_POINTERS).map(|chunk| Payload::Pointer { v: 1, community: c, pointers: chunk.to_vec() }).collect();
        if self.queue_now(c, &payloads, &records, &media).is_err() {
            // the outbox is full: the payloads go again, whole, on a later round
            self.forget_inflight(c, &records, &media);
        }
    }

    /// Payloads that could not be kept in pieces go whole, as they always did.
    pub(crate) fn send_whole(&mut self, c: Id, contents: Vec<Vec<u8>>, records: Vec<Id>, media: Vec<(Id, usize)>) {
        let mut payloads = Vec::new();
        for content in &contents {
            if let Ok(list) = decoded::<Vec<Payload>>(content) {
                payloads.extend(list);
            }
        }
        let fits = payloads.len() <= MAX_PENDING;
        if !fits || self.queue_now(c, &payloads, &records, &media).is_err() {
            self.forget_inflight(c, &records, &media);
        }
    }

    /// A media item whose object was let go while other posts still use it goes again, for them.
    pub(crate) fn private_resend_media(&mut self, c: Id, hash: Id) {
        if let Some(saved) = self.private.groups.get_mut(&c)
            && saved.sent_media.remove(&hash).is_some()
        {
            self.private.dirty = true;
        }
    }

    /// Records and media that never went out are sent again by a later outbox: nothing marks
    /// them sent.
    fn forget_inflight(&mut self, c: Id, records: &[Id], media: &[(Id, usize)]) {
        if let Some(set) = self.scatter.repair.inflight_records.get_mut(&c) {
            for r in records {
                set.remove(r);
            }
        }
        if let Some(set) = self.scatter.repair.inflight_media.get_mut(&c) {
            for (h, _) in media {
                set.remove(h);
            }
        }
    }

    /// The content of an object rebuilt for a group: its payloads, applied as if they had come
    /// whole. `private-pending` when something they belong to has not arrived yet.
    pub(crate) fn private_apply_content(&mut self, c: Id, content: &[u8]) -> R<()> {
        if let Some(slice) = read_slice(c, content) {
            let (h, bytes) = slice?;
            if !self.store.media_users(&h.hash).any(|s| s.record.community == Some(c) && self.forwardable(s)) {
                return Err(ApiError::new("private-pending", "The encrypted post for this attachment is still missing."));
            }
            if self.store.has_blob(&h.hash) {
                return Ok(());
            }
            let mut saved = self.private.groups.get(&c).cloned().ok_or_else(fail)?;
            let done = take_chunk(&mut saved.partial, h.hash, h.total, h.offset, bytes)?;
            self.private.groups.insert(c, saved);
            self.private.dirty = true;
            if let Some(all) = done
                && !self.on_blob(h.hash, all)
            {
                return Err(fail());
            }
            return Ok(());
        }
        let payloads: Vec<Payload> = decoded(content)?;
        if payloads.len() > SCATTER_MEDIA_CHUNKS.max(1) || payloads.iter().any(|p| !matches!(p, Payload::Records { .. } | Payload::Media { .. })) {
            return Err(fail());
        }
        let mut saved = self.private.groups.get(&c).cloned().ok_or_else(fail)?;
        let mut result = Ok(());
        for payload in payloads {
            if let Err(e) = self.apply_private_payload(c, &mut saved, payload) {
                result = Err(e);
            }
        }
        self.private.groups.insert(c, saved);
        self.private.dirty = true;
        result
    }
    pub fn private_ack(&mut self, community: &str, ids: &[String]) -> R<PrivateGroup> {
        if ids.len() > MAX_PENDING {
            return Err(fail());
        }
        let c = parse(community)?;
        let s = self.private.groups.get_mut(&c).ok_or_else(fail)?;
        s.objects.retain(|o| !ids.contains(&o.id));
        self.private.dirty = true;
        self.private_group(community)
    }
    /// Signs relay reads for the current epoch's room. Each target is one request below the room
    /// (`objects?after=<n>` or `objects/<id>`); the relay accepts each signature once, near now.
    pub fn private_read(&self, community: &str, targets: &[String]) -> R<PrivateRead> {
        if targets.is_empty() || targets.len() > MAX_READS || !targets.iter().all(|t| read_target(t)) {
            return Err(ApiError::new("invalid", "This encrypted relay request is not allowed."));
        }
        let c = parse(community)?;
        if !self.private.groups.contains_key(&c) {
            return Err(ApiError::not_found("private community"));
        }
        let p = self.private.provider();
        let relay = relay(&p, &load(&p, c)?)?;
        let (room, time) = (relay.room(), self.now());
        let reads = targets
            .iter()
            .map(|target| {
                let nonce = hex_encode(&hoi_crypto::random_bytes::<16>().map_err(mls)?);
                Ok(ReadProof { time, signature: relay.signed(&read_text(room, target, time, &nonce)), nonce })
            })
            .collect::<R<_>>()?;
        Ok(PrivateRead { room: room.hex(), reads })
    }
    pub fn private_receive(&mut self, community: &str, body: &str) -> R<PrivateGroup> {
        if body.len() > MAX_OBJECT * 4 / 3 + 8 {
            return Err(fail());
        }
        let c = parse(community)?;
        let sealed = unb64(body).map_err(mls)?;
        let id = hex_encode(&Sha256::digest(&sealed));
        let mut saved = self.private.groups.get(&c).cloned().ok_or_else(fail)?;
        if saved.received.contains(&id) {
            return self.private_group(community);
        }
        let p = self.private.provider();
        let mut g = load(&p, c)?;
        // Only this epoch's members can open an object of its room; MLS never sees anything else.
        let raw = relay(&p, &g)?.open(&sealed)?;
        let msg = MlsMessageIn::tls_deserialize_exact(&raw).map_err(mls)?.try_into_protocol_message().map_err(mls)?;
        if msg.wire_format() != WireFormat::PrivateMessage || msg.group_id().as_slice() != c.as_bytes() {
            return Err(fail());
        }
        if msg.epoch() > g.epoch() {
            return Err(ApiError::new("private-pending", "An earlier encrypted membership update is still missing."));
        }
        // Joining at a later epoch deliberately gives no access to previous epochs. Live retained
        // history is resent in the new epoch by members, without retaining obsolete epoch keys.
        if msg.epoch() < g.epoch() {
            remember(&mut saved, id);
            self.private.groups.insert(c, saved);
            self.private.dirty = true;
            return self.private_group(community);
        }
        let processed = g.process_message(&p, msg).map_err(mls)?;
        let sender_credential = processed.credential().clone();
        let mut contest = None;
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(app) => {
                let payload: Payload = decoded(&app.into_bytes())?;
                let sender = self.private_application_sender(&sender_credential, &payload)?;
                if let Payload::Request { v: 1, community, request } = &payload {
                    if *community != c || request.len() > 64 * 1024 {
                        return Err(fail());
                    }
                    self.note_request(c, request.clone());
                } else if let Payload::Welcome { v: 1, community, device, invitation } = &payload {
                    // Only committing devices (the owner's and moderators') make invitations.
                    if *community != c || invitation.len() > 256 * 1024 || !self.private_committer_root(c, parse(&sender.root)?) {
                        return Err(fail());
                    }
                    let list = self.private.invitations.entry(c).or_default();
                    if list.len() < MAX_REQUESTS || list.contains_key(&device.hex()) {
                        list.insert(device.hex(), invitation.clone());
                        self.private.invited_in.entry(c).or_default().insert(device.hex(), g.epoch().as_u64());
                        self.private.dirty = true;
                    }
                } else if matches!(payload, Payload::SealKey { .. } | Payload::DropKey { .. }) {
                    self.take_key_payload(c, &g, &sender, payload)?;
                } else if let Payload::Pointer { v: 1, community, pointers } = &payload {
                    if *community != c || pointers.len() > MAX_POINTERS {
                        return Err(fail());
                    }
                    let from = parse(&sender.device)?;
                    for encoded in pointers {
                        // An update that breaks the rules is ignored, not the message.
                        if let Ok(pointer) = hoi_types::scatter::Pointer::decode(&unb64(encoded).map_err(mls)?) {
                            let _ = self.take_pointer(Some(c), pointer, Some(from), true);
                        }
                    }
                } else {
                    self.apply_private_payload(c, &mut saved, payload)?;
                }
            }
            ProcessedMessageContent::StagedCommitMessage(commit) => {
                self.private_membership_author(c, &sender_credential)?;
                // Only a device in this epoch's line commits (a moderator's only where every leaf
                // follows moderators' commits).
                let sender = parse(&member(&sender_credential, None)?.device)?;
                let rank = self.committers(c, &g).iter().position(|x| x.device == sender);
                if rank.is_none() {
                    return Err(ApiError::not_allowed("Only the owner can authorize encrypted membership updates."));
                }
                if self.follows_records(c) {
                    // The records decide: nothing they do not back is followed.
                    self.check_record_commit(c, &g, &commit)?;
                }
                // A commit of someone later in line: the epoch is kept a while, in case an
                // earlier committer's commit of it turns up.
                contest = self.contest_point(&p, &g, c, rank);
                let ended = g.epoch().as_u64();
                let adds = commit.add_proposals().next().is_some();
                g.merge_staged_commit(&p, *commit).map_err(mls)?;
                for m in g.members() {
                    member(&m.credential, Some(&m.signature_key))?;
                }
                self.forget_passed(c, &g);
                // what went in the epoch it ended goes again; on a commit that adds someone this
                // device's own records too (the committer sends the rest of the history)
                let own: BTreeSet<Id> = if adds { self.own_in_group(c) } else { BTreeSet::new() };
                saved.send_again_after(ended, &own);
                // Data made before revocation must never be published after removal.
                if !g.is_active() {
                    saved.objects.clear();
                    saved.partial.clear();
                }
            }
            _ => return Err(fail()),
        }
        remember(&mut saved, id);
        self.private.groups.insert(c, saved);
        self.private.save(&p);
        self.keep_contest(c, contest);
        self.private_group(community)
    }
    /// This device's own records in group `c`, its own posts' media items, and its person's
    /// identity and its own blessing, which no community record of its may have carried yet.
    fn own_in_group(&self, c: Id) -> BTreeSet<Id> {
        let Some(me) = self.device_kp.as_ref().map(|k| k.public()) else { return BTreeSet::new() };
        let mut out = BTreeSet::new();
        if let Some(root) = self.me_root() {
            out.extend(self.store.identity(&root).map(|i| i.record));
        }
        out.extend(self.store.delegation(&me).map(|d| d.record));
        for s in self.store.in_community(&c).filter(|s| s.record.author == me) {
            out.insert(s.id);
            if let Body::Post { media, .. } = &s.record.body {
                out.extend(media.iter().map(|m| m.hash));
            }
        }
        out
    }

    /// Whether a sealed object of group `c` is a commit or proposal of its current epoch: what
    /// would end the epoch if followed (crate::mailbox holds one found ahead of its slot).
    pub(crate) fn private_is_commit(&self, c: Id, body: &[u8]) -> bool {
        let p = self.private.provider();
        let Ok(g) = load(&p, c) else { return false };
        let Ok(raw) = relay(&p, &g).and_then(|r| r.open(body)) else { return false };
        MlsMessageIn::tls_deserialize_exact(&raw)
            .ok()
            .and_then(|m| m.try_into_protocol_message().ok())
            .is_some_and(|m| m.epoch() == g.epoch() && m.is_handshake_message())
    }
    fn private_ingest(&mut self, c: Id, wires: Vec<Vec<u8>>) -> R<()> {
        for w in &wires {
            validate_wire(c, w)?;
        }
        let mut rejected = None;
        self.ingest_batch(wires, &|_| Source::PrivateRelay, &mut |_, _, result| {
            if let Err(e) = result
                && !matches!(e, hoi_validate::Reject::Duplicate | hoi_validate::Reject::Expired)
            {
                rejected = Some(e);
            }
        });
        if let Some(e) = rejected {
            // A vote, edit, deletion or reply that came whole before the post it is about, which
            // is still being fetched from its pieces: it gets another try (crate::keep).
            let orphan = matches!(&e, hoi_validate::Reject::NotAllowed(why) if why.contains("we don't have"));
            if orphan && self.scatter.saved.kept.values().any(|k| k.community == Some(c) && k.apply) {
                return Err(ApiError::new("private-pending", "The post this is about is still being fetched."));
            }
            return Err(e.into());
        }
        Ok(())
    }
    fn validate_private_bootstrap(&self, c: Id, sender: &PrivateMember, wires: &[Vec<u8>]) -> R<()> {
        if wires.len() != 3 {
            return Err(fail());
        }
        let root = parse(&sender.root)?;
        let device = parse(&sender.device)?;
        if self.store.delegation(&root).is_some()
            || self.store.identity(&device).is_some()
            || self.store.burned_at(&root).is_some()
            || self.store.delegation(&device).is_some_and(|d| d.info.root != root || d.info.revoked_at.is_some())
        {
            return Err(fail());
        }
        let mut kinds = BTreeSet::new();
        for bytes in wires {
            let d = hoi_validate::decode(bytes)?;
            let r = &d.wire.record;
            hoi_validate::signature(&d)?;
            hoi_validate::content(r)?;
            hoi_validate::time(r, self.now(), false, None)?;
            hoi_validate::stamp(&d.id, r.nonce, hoi_types::GLOBAL_POW_BITS)?;
            match &r.body {
                Body::Community(g) if r.author == c && g.owners == vec![root] && g.threshold == 1 => {
                    kinds.insert(0);
                }
                Body::Identity { .. } if r.author == root => {
                    hoi_validate::identity(r)?;
                    kinds.insert(1);
                }
                Body::Delegation { root: rroot, .. } if r.author == device && *rroot == root => {
                    hoi_validate::delegation_consent(r)?;
                    kinds.insert(2);
                }
                _ => return Err(fail()),
            }
        }
        if kinds.len() != 3 {
            return Err(fail());
        }
        Ok(())
    }
    /// A bootstrap from a moderator's device: the genesis, the moderator's identity
    /// and blessing, and what made them one, the `mod-add` and its signer's identity and
    /// blessing. Whether that makes them one is the community's own replay, checked once in.
    fn validate_moderator_bootstrap(&self, c: Id, sender: &PrivateMember, owner: Id, wires: &[Vec<u8>]) -> R<()> {
        if wires.len() > 16 {
            return Err(fail());
        }
        let root = parse(&sender.root)?;
        let device = parse(&sender.device)?;
        if root == owner
            || self.store.delegation(&root).is_some()
            || self.store.identity(&device).is_some()
            || self.store.burned_at(&root).is_some()
            || self.store.delegation(&device).is_some_and(|d| d.info.root != root || d.info.revoked_at.is_some())
        {
            return Err(fail());
        }
        let mut kinds = BTreeSet::new();
        for bytes in wires {
            let d = hoi_validate::decode(bytes)?;
            let r = &d.wire.record;
            hoi_validate::signature(&d)?;
            hoi_validate::content(r)?;
            hoi_validate::time(r, self.now(), false, None)?;
            match &r.body {
                Body::Community(g) if r.author == c && g.owners == vec![owner] && g.threshold == 1 => {
                    hoi_validate::stamp(&d.id, r.nonce, hoi_types::GLOBAL_POW_BITS)?;
                    kinds.insert(0);
                }
                Body::Identity { .. } if r.author == root || r.author == owner => {
                    hoi_validate::identity(r)?;
                    if r.author == root {
                        kinds.insert(1);
                    }
                }
                Body::Delegation { root: of, .. } if (r.author == device && *of == root) || *of == owner => {
                    hoi_validate::delegation_consent(r)?;
                    if r.author == device {
                        kinds.insert(2);
                    }
                }
                Body::Ledger(l) if r.community == Some(c) && l.action == hoi_types::LedgerAction::ModAdd && l.target == Some(root) => {
                    kinds.insert(3);
                }
                _ => return Err(fail()),
            }
        }
        if kinds.len() != 4 {
            return Err(fail());
        }
        Ok(())
    }
    /// What makes `root` a moderator of `c`, for a newcomer who holds none of the community's
    /// records yet: the `mod-add` that made them one, and its signer's identity and blessing.
    fn role_proof(&self, c: Id, root: Id) -> Vec<Vec<u8>> {
        let Some(st) = self.communities.get(&c) else { return vec![] };
        let made = st
            .applied
            .iter()
            .rev()
            .filter_map(|a| self.store.get(&a.entry))
            .find(|s| matches!(&s.record.body, Body::Ledger(l) if l.action == hoi_types::LedgerAction::ModAdd && l.target == Some(root)));
        let Some(s) = made else { return vec![] };
        let mut out: Vec<Vec<u8>> = self.carriers(s).into_iter().map(|r| r.wire.clone()).collect();
        out.push(s.wire.clone());
        out
    }
    fn apply_private_payload(&mut self, c: Id, saved: &mut SavedGroup, payload: Payload) -> R<()> {
        match payload {
            Payload::Bootstrap { v: 2, community, records } | Payload::Records { v: 1, community, records } if community == c => {
                self.private_ingest(c, decode_records(c, &records)?)?
            }
            Payload::Media { v: 1, community, hash, total, index, bytes, posts } if community == c && posts.len() <= MAX_MEDIA_POSTS => {
                let data = unb64(&bytes).map_err(mls)?;
                if total == 0 || total > 16 * 1024 * 1024 || index >= total.div_ceil(CHUNK) || data.len() != CHUNK.min(total - index * CHUNK) {
                    return Err(fail());
                }
                if !self.store.media_users(&hash).any(|s| s.record.community == Some(c) && self.forwardable(s)) {
                    return Err(ApiError::new("private-pending", "The encrypted post for this attachment is still missing."));
                }
                if self.store.has_blob(&hash) {
                    return Ok(());
                }
                if let Some(bytes) = take_chunk(&mut saved.partial, hash, total, index * CHUNK, data)?
                    && !self.on_blob(hash, bytes)
                {
                    return Err(fail());
                }
            }
            _ => return Err(fail()),
        }
        Ok(())
    }
}

/// What an object of scattered content holds, as read back (crate::open).
pub(crate) enum Item {
    Records(Vec<Vec<u8>>),
    Media { hash: Id, total: usize, offset: usize, bytes: Vec<u8> },
}

/// The content of an object that holds one record: a `records` payload, as private groups'.
pub(crate) fn records_content(c: Id, wire: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&vec![Payload::Records { v: 1, community: c, records: vec![b64(wire)] }]).unwrap_or_default()
}

/// The contents of the objects that hold a media item, sent for `posts`: raw slices, each
/// object as full as a class it fits (`scatter::fit`), so a 300 KB picture costs 320 KiB of
/// objects, not the 1 MiB that base64 inside JSON in one object did.
pub(crate) fn media_contents(c: Id, hash: Id, bytes: &[u8], posts: &[Id]) -> Vec<Vec<u8>> {
    let posts: Vec<Id> = posts.iter().take(MAX_MEDIA_POSTS).copied().collect();
    let mut header = Slice { v: 1, community: c, hash, total: bytes.len(), offset: bytes.len(), posts };
    // the most a header takes: the offset written with as many digits as the length
    let overhead = SLICE_MAGIC.len() + 4 + serde_json::to_vec(&header).map(|h| h.len()).unwrap_or(0);
    let mut out = Vec::new();
    let mut at = 0;
    for class in hoi_types::scatter::fit(bytes.len(), overhead) {
        let share = class.capacity().saturating_sub(overhead).min(bytes.len() - at);
        header.offset = at;
        out.push(slice_content(&header, &bytes[at..at + share]));
        at += share;
        if at >= bytes.len() {
            break;
        }
    }
    out
}

/// What an object's content holds: only records and media of community `c`.
pub(crate) fn content_items(c: Id, content: &[u8]) -> R<Vec<Item>> {
    if let Some(slice) = read_slice(c, content) {
        let (h, bytes) = slice?;
        return Ok(vec![Item::Media { hash: h.hash, total: h.total, offset: h.offset, bytes }]);
    }
    let payloads: Vec<Payload> = decoded(content)?;
    if payloads.len() > SCATTER_MEDIA_CHUNKS.max(1) {
        return Err(fail());
    }
    payloads
        .into_iter()
        .map(|p| match p {
            Payload::Records { v: 1, community, records } if community == c => Ok(Item::Records(decode_records(c, &records)?)),
            Payload::Media { v: 1, community, hash, total, index, bytes, posts } if community == c && posts.len() <= MAX_MEDIA_POSTS => {
                let data = unb64(&bytes).map_err(mls)?;
                if total == 0 || total > 16 * 1024 * 1024 || index >= total.div_ceil(CHUNK) || data.len() != CHUNK.min(total - index * CHUNK) {
                    return Err(fail());
                }
                Ok(Item::Media { hash, total, offset: index * CHUNK, bytes: data })
            }
            _ => Err(fail()),
        })
        .collect()
}

/// Keeps one part of a media item being rebuilt, at `offset`; the whole item once the parts
/// cover it.
pub(crate) fn take_chunk(partial: &mut BTreeMap<Id, PartialMedia>, hash: Id, total: usize, offset: usize, data: Vec<u8>) -> R<Option<Vec<u8>>> {
    let held: usize = partial.values().map(PartialMedia::held).sum();
    if held + data.len() > 32 * 1024 * 1024 {
        return Err(ApiError::new("too-big", "Too many incomplete attachments."));
    }
    let p = partial.entry(hash).or_insert(PartialMedia { total, parts: BTreeMap::new() });
    if p.total != total || offset + data.len() > total {
        return Err(fail());
    }
    let all = p.add(offset, data);
    if all.is_some() {
        partial.remove(&hash);
    }
    Ok(all)
}

/// What an object rebuilt for a group carries: the records of its `records` payloads (by id)
/// and the media item of its `media` payloads.
pub(crate) fn carried(content: &[u8]) -> (Vec<Id>, Option<Id>) {
    if let Some(rest) = content.strip_prefix(SLICE_MAGIC)
        && let Some(len) = rest.get(..4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(|b| u32::from_be_bytes(b) as usize)
        && let Some(header) = rest.get(4..4 + len).and_then(|h| serde_json::from_slice::<Slice>(h).ok())
    {
        return (header.posts, Some(header.hash));
    }
    let Ok(payloads) = decoded::<Vec<Payload>>(content) else { return (Vec::new(), None) };
    let mut records = Vec::new();
    let mut media = None;
    for p in payloads {
        match p {
            Payload::Records { records: rs, .. } => {
                for r in rs {
                    if let Some(id) = unb64(&r).ok().and_then(|b| hoi_validate::decode(&b).ok()).map(|d| d.id) {
                        records.push(id);
                    }
                }
            }
            Payload::Media { hash, posts, .. } => {
                media = Some(hash);
                for p in posts {
                    if !records.contains(&p) {
                        records.push(p);
                    }
                }
            }
            _ => {}
        }
    }
    (records, media)
}

impl Node {
    /// Whether a private group's membership follows its records: its admission is by
    /// invitation or approval, as members-only communities' is (SCATTER.md S4). The owner's
    /// device then adds the people the community admits and removes those it bans or whose
    /// identity or device is gone, and every member checks each commit against the records.
    pub(crate) fn follows_records(&self, c: Id) -> bool {
        self.is_private_community(&c)
            && self.communities.get(&c).is_some_and(|st| matches!(st.limits.admission, hoi_types::Admission::Invite | hoi_types::Admission::Approve))
    }

    /// Whether the records exclude a member: banned, its identity burned, or its device
    /// revoked or bound to another person.
    fn excluded(&self, c: Id, root: Id, device: Id) -> bool {
        self.communities.get(&c).is_some_and(|st| st.is_banned(&root))
            || self.store.burned_at(&root).is_some()
            || self.store.delegation(&device).is_some_and(|d| d.info.revoked_at.is_some() || d.info.root != root)
    }

    /// A commit of a group whose membership follows its records: every person added must be
    /// admitted by the records this device holds, and not banned; every member removed must be
    /// excluded by them. Otherwise the commit waits (its records may still be on their way) and
    /// is never followed if they do not come.
    fn check_record_commit(&self, c: Id, g: &MlsGroup, commit: &StagedCommit) -> R<()> {
        let st = self.communities.get(&c).ok_or_else(fail)?;
        for add in commit.add_proposals() {
            let who = member(add.add_proposal().key_package().leaf_node().credential(), None)?;
            let root = parse(&who.root)?;
            if !st.is_admitted(&root) || st.is_banned(&root) {
                return Err(ApiError::new("private-pending", "Someone was added whom this community has not admitted (yet)."));
            }
        }
        for rm in commit.remove_proposals() {
            let Some(m) = g.member_at(rm.remove_proposal().removed()) else { continue };
            let who = member(&m.credential, None)?;
            if !self.excluded(c, parse(&who.root)?, parse(&who.device)?) {
                return Err(ApiError::new("private-pending", "Someone was removed whom this community has not banned."));
            }
        }
        Ok(())
    }

    /// The records that back a membership change for `root`'s `device`, for a group whose
    /// membership follows its records: its identity and delegation, and the vouches and `admit`
    /// entries for it (adding), or the bans of it and revocations by it (removing). They go to
    /// the members in the epoch the commit ends, ahead of it, so every member can check it.
    fn backing_records(&self, c: Id, root: Id, device: Id, adding: bool) -> Vec<(Id, Vec<u8>)> {
        let mut out: Vec<(Id, Vec<u8>)> = Vec::new();
        if let Some(i) = self.store.identity(&root).and_then(|i| self.store.get(&i.record)) {
            out.push((i.id, i.wire.clone()));
        }
        if let Some(d) = self.store.delegation(&device).and_then(|d| self.store.get(&d.record)) {
            out.push((d.id, d.wire.clone()));
        }
        for s in self.store.in_community(&c) {
            let backs = match &s.record.body {
                Body::Vouch { value, .. } if adding => *value == 1 && s.record.parent == Some(root),
                Body::Ledger(l) if adding => l.action == hoi_types::LedgerAction::Admit && l.target == Some(root),
                Body::Ledger(l) => l.action == hoi_types::LedgerAction::Ban && l.target == Some(root),
                _ => false,
            };
            if backs {
                out.push((s.id, s.wire.clone()));
            }
        }
        if !adding {
            for s in self.store.all().filter(|s| s.record.kind() == Kind::Revoke && s.author_root == root) {
                out.push((s.id, s.wire.clone()));
            }
        }
        out.truncate(64);
        out
    }

    /// Queues `records` in the room `relay` belongs to, before a commit.
    #[allow(clippy::too_many_arguments)]
    fn queue_backing(
        &self,
        p: &OpenMlsRustCrypto,
        g: &mut MlsGroup,
        signer: &SignatureKeyPair,
        saved: &mut SavedGroup,
        relay: &Relay,
        c: Id,
        records: Vec<(Id, Vec<u8>)>,
    ) -> R<()> {
        if records.is_empty() {
            return Ok(());
        }
        let ids: Vec<Id> = records.iter().map(|(id, _)| *id).collect();
        let wires: Vec<String> = records.iter().map(|(_, w)| b64(w)).collect();
        self.queue_payload(p, g, signer, saved, relay, &Payload::Records { v: 1, community: c, records: wires })?;
        saved.sent(ids, g.epoch().as_u64());
        Ok(())
    }

    /// A join request a member passed on: each committing device keeps it until the records
    /// admit its person and their device is in the group (or a week passes).
    fn note_request(&mut self, c: Id, request: String) {
        if !self.private_epoch(c).is_some_and(|e| e.rank.is_some()) {
            return;
        }
        let now = self.now();
        let list = self.private.requests.entry(c).or_default();
        list.retain(|(_, at)| now < at + REQUEST_TTL);
        if list.len() < MAX_REQUESTS && !list.iter().any(|(r, _)| *r == request) {
            list.push((request, now));
            self.private.dirty = true;
        }
    }

    /// Passes someone's join request on to the group, for a group whose membership follows its
    /// records: in a community admitted by invitation this device vouches for the person (the
    /// vouch is the invitation), and a committing device adds them once the records admit them.
    /// Its invitation comes back to the group (`PrivateGroup::invitations`), to hand on.
    pub fn private_pass_request(&mut self, community: &str, request: &str) -> R<PrivateMember> {
        let c = parse(community)?;
        if !self.follows_records(c) {
            return Err(ApiError::not_allowed("Only the owner admits people to this private community."));
        }
        let p = self.private.provider();
        let (_, who, _) = self.checked_request(request, &p)?;
        drop(p);
        let invite = self.communities.get(&c).is_some_and(|st| st.limits.admission == hoi_types::Admission::Invite);
        if invite {
            match self.vouch(community, &who.root, 1, None) {
                Ok(_) => {}
                Err(e) if e.code == "invalid" => {}
                Err(e) => return Err(e),
            }
        }
        // a committing device keeps it, and passes it to the others too: any of them may add them
        if self.private_epoch(c).is_some_and(|e| e.rank.is_some()) {
            self.note_request(c, request.trim().to_string());
        }
        self.queue_now(c, &[Payload::Request { v: 1, community: c, request: request.trim().to_string() }], &[], &[])?;
        if let (Ok(device), Some(e)) = (parse(&who.device), self.private_epoch(c)) {
            let now = self.now();
            let list = self.private.passed.entry(c).or_default();
            list.retain(|(r, ..)| r != request.trim());
            if list.len() < MAX_REQUESTS {
                list.push((request.trim().to_string(), device, now, e.epoch));
            }
            self.private.dirty = true;
        }
        Ok(who)
    }

    /// Join requests passed on whose device a commit followed here added: done, even should that
    /// commit's branch lose, since the invitation the device opened used up its request (it asks
    /// again).
    fn forget_passed(&mut self, c: Id, g: &MlsGroup) {
        if let Some(list) = self.private.passed.get_mut(&c) {
            let before = list.len();
            list.retain(|(_, device, ..)| !g.members().any(|m| m.signature_key.as_slice() == device.0.as_slice()));
            if list.len() != before {
                self.private.dirty = true;
            }
        }
    }

    /// Whether this device, `rank`th in line, has waited its turn for change `key` of group `c`
    /// (a device to add or remove, or [`Id::ZERO`] for the mailbox list): the first in line
    /// makes a change at once, each later one [`TURN`] after the one before it, so a change
    /// waits on a silent committer only that long. Notes since when the change is due.
    pub(crate) fn waited_turn(&mut self, c: Id, key: Id, rank: usize) -> bool {
        let now = self.now();
        let since = *self.private.due.entry(c).or_default().entry(key).or_insert_with(|| {
            self.private.dirty = true;
            now
        });
        now >= since.saturating_add(rank as u64 * TURN)
    }

    /// Change `key` of group `c` is no longer due here (made, or no longer called for).
    pub(crate) fn not_due(&mut self, c: Id, key: Id) {
        if let Some(due) = self.private.due.get_mut(&c)
            && due.remove(&key).is_some()
        {
            self.private.dirty = true;
        }
    }

    /// On a committing device of a group whose membership follows its records: adds one person
    /// the records admit whose request is waiting, sending their invitation back into the
    /// group; or removes one member the records exclude. One change a round, each once this
    /// device has waited its turn for it.
    pub(crate) fn follow_records(&mut self, c: Id) {
        if !self.follows_records(c) {
            return;
        }
        let now = self.now();
        let p = self.private.provider();
        let Ok(g) = load(&p, c) else { return };
        let Some(rank) = self.own_rank(c, &g) else { return };
        let members: Vec<(Id, Id)> = g
            .members()
            .filter_map(|m| member(&m.credential, Some(&m.signature_key)).ok())
            .filter_map(|w| Some((parse(&w.root).ok()?, parse(&w.device).ok()?)))
            .collect();
        let requests = self.private.requests.get(&c).cloned().unwrap_or_default();
        let mut keep = Vec::new();
        let mut admitted = Vec::new();
        for (request, at) in requests {
            if now >= at + REQUEST_TTL {
                continue;
            }
            let Ok((_, who, _)) = self.checked_request(&request, &p) else { continue };
            let (Ok(root), Ok(device)) = (parse(&who.root), parse(&who.device)) else { continue };
            // in the group: done (a device added by a commit that lost asks again: the
            // invitation it opened used up its request)
            if members.iter().any(|(_, d)| *d == device) {
                continue;
            }
            if self.communities.get(&c).is_some_and(|st| st.is_admitted(&root) && !st.is_banned(&root)) {
                admitted.push((request.clone(), device, at));
            }
            keep.push((request, at));
        }
        drop(p);
        let me = self.state.me.as_ref().map(|m| m.device);
        let excluded: Vec<Id> = members.iter().filter(|(root, device)| Some(*device) != me && self.excluded(c, *root, *device)).map(|(_, d)| *d).collect();
        // what is due now, since when; anything no longer due is forgotten
        let due: BTreeSet<Id> = admitted.iter().map(|(_, d, _)| *d).chain(excluded.iter().copied()).chain([Id::ZERO, DROP_KEY_DUE]).collect();
        if let Some(list) = self.private.due.get_mut(&c) {
            list.retain(|k, _| due.contains(k));
        }
        let ready_add =
            admitted.into_iter().map(|a| (self.waited_turn(c, a.1, rank), a)).collect::<Vec<_>>().into_iter().find(|(ready, _)| *ready).map(|(_, a)| a);
        let ready_remove =
            excluded.into_iter().map(|d| (self.waited_turn(c, d, rank), d)).collect::<Vec<_>>().into_iter().find(|(ready, _)| *ready).map(|(_, d)| d);
        self.private.requests.insert(c, keep);
        self.private.dirty = true;
        if let Some((request, device, _)) = ready_add {
            if let Ok(invitation) = self.private_invite(&c.hex(), &request) {
                let _ = self.queue_now(c, &[Payload::Welcome { v: 1, community: c, device, invitation: invitation.clone() }], &[], &[]);
                self.private.invitations.entry(c).or_default().insert(device.hex(), invitation);
                if let Some(e) = self.private_epoch(c) {
                    self.private.invited_in.entry(c).or_default().insert(device.hex(), e.epoch);
                }
                self.not_due(c, device);
            }
            return;
        }
        if let Some(device) = ready_remove
            && self.private_remove(&c.hex(), &device.hex()).is_ok()
        {
            self.not_due(c, device);
        }
        // invitations of devices that joined since are done
        if let Some(list) = self.private.invitations.get_mut(&c) {
            list.retain(|d, _| !members.iter().any(|(_, m)| m.hex() == *d));
        }
        if let Some(list) = self.private.invited_in.get_mut(&c) {
            let ready = self.private.invitations.get(&c);
            list.retain(|d, _| ready.is_some_and(|r| r.contains_key(d)));
        }
    }

    /// A moderator of a group whose membership follows its records: its devices commit too, in
    /// their turn.
    fn private_moderator(&self, c: Id, root: Id) -> bool {
        self.follows_records(c)
            && self.communities.get(&c).is_some_and(|st| st.genesis.threshold == 1 && st.genesis.owners.len() == 1 && st.is_mod(&root) && !st.is_banned(&root))
    }

    /// A device of the owner or, in a group whose membership follows its records, of a moderator.
    fn private_committer_root(&self, c: Id, root: Id) -> bool {
        self.private_owner(c, root) || self.private_moderator(c, root)
    }

    /// The devices that may commit in group `c` in the epoch `g` is in, first in line first:
    /// by how long their person has held the role, by signed time (the owner since the
    /// community began, each moderator since the `mod-add` that made them one), then by the age
    /// of the blessing in the device's leaf. Only leaves whose blessing covers membership count,
    /// and moderators' only in a group that follows its records and whose every leaf follows
    /// their commits. Every member holding the same records computes the same line.
    pub(crate) fn committers(&self, c: Id, g: &MlsGroup) -> Vec<Committer> {
        let Some(st) = self.communities.get(&c) else { return vec![] };
        let several = self.follows_records(c);
        // when each moderator was made one: the latest `mod-add` that took effect
        let mut appointed: BTreeMap<Id, u64> = BTreeMap::new();
        if several && !st.mods.is_empty() {
            for a in &st.applied {
                let Some(s) = self.store.get(&a.entry) else { continue };
                if let Body::Ledger(l) = &s.record.body
                    && l.action == hoi_types::LedgerAction::ModAdd
                    && let Some(t) = l.target
                {
                    appointed.insert(t, s.record.created_at);
                }
            }
        }
        let required = [format!("moderate:{}", c.hex())];
        let mut line: Vec<((u64, Id, u64, Id), Committer)> = Vec::new();
        for m in g.members() {
            let Ok(bound) = decoded::<BoundCredential>(m.credential.serialized_content()) else { continue };
            let Ok(root) = parse(&bound.root) else { continue };
            let since = if self.private_owner(c, root) {
                st.created_at
            } else if several && self.private_moderator(c, root) {
                match appointed.get(&root) {
                    Some(t) => *t,
                    None => continue,
                }
            } else {
                continue;
            };
            let Ok(who) = member(&m.credential, Some(&m.signature_key)) else { continue };
            let Ok(device) = parse(&who.device) else { continue };
            if self.excluded(c, root, device) {
                continue;
            }
            let Some(d) = unb64(&bound.delegation).ok().and_then(|b| hoi_validate::decode(&b).ok()) else { continue };
            let Body::Delegation { scope, .. } = &d.wire.record.body else { continue };
            if !hoi_validate::scope_covers(scope, &required)
                || self.store.delegation(&device).is_some_and(|x| !hoi_validate::scope_covers(&x.info.scope, &required))
            {
                continue;
            }
            line.push(((since, root, d.wire.record.created_at, device), Committer { leaf: m.index.u32(), device, root }));
        }
        line.sort_by_key(|a| a.0);
        line.into_iter().map(|(_, x)| x).collect()
    }

    /// This device's place in line in the epoch of `g`, if it may commit there.
    fn own_rank(&self, c: Id, g: &MlsGroup) -> Option<usize> {
        let me = self.state.me.as_ref()?.device;
        self.committers(c, g).iter().position(|x| x.device == me)
    }

    /// The drop key work due on this device in group `c`, and how many turns it waits
    /// for it: its own place in line, or, when it only lacks the listed key's secret, one past
    /// the last device in line that holds it, so each of those had its turn to give it first.
    ///
    /// A list without a key, or whose key a device that has since left the line was given,
    /// needs a new one; committers' devices the key was not given to, once they announced their
    /// seal key, get it from one that holds it. Where the owner's device alone commits, it keeps
    /// its own key.
    pub(crate) fn drop_key_work(&self, c: Id) -> (KeyWork, usize) {
        let p = self.private.provider();
        let (Some(s), Ok(g)) = (self.private.groups.get(&c), load(&p, c)) else { return (KeyWork::Keep, 0) };
        let Ok(ep) = self.epoch_of(&p, &g, s) else { return (KeyWork::Keep, 0) };
        if !self.follows_records(c) {
            return match ep.drop_key {
                _ if !ep.owner => (KeyWork::Keep, 0),
                None => (KeyWork::Make, 0),
                Some(d) if self.held_drop_key(c, &d).is_some() => (KeyWork::Keep, 0),
                Some(_) => (KeyWork::Make, 1),
            };
        }
        let line: Vec<Id> = self.committers(c, &g).iter().map(|x| x.device).collect();
        let Some(rank) = self.state.me.as_ref().and_then(|m| line.iter().position(|x| *x == m.device)) else { return (KeyWork::Keep, 0) };
        let Some(d) = ep.drop_key else { return (KeyWork::Make, rank) };
        if ep.holders.iter().any(|h| !line.contains(h)) {
            return (KeyWork::Make, rank);
        }
        if self.held_drop_key(c, &d).is_none() {
            let last = line.iter().enumerate().filter(|(_, x)| ep.holders.contains(x)).map(|(i, _)| i).max().unwrap_or(0);
            return (KeyWork::Make, rank.max(last) + 1);
        }
        // those whose seal key this device heard: one that never announced one cannot take it
        let missing: Vec<Id> =
            line.iter().enumerate().filter(|(i, x)| *i != rank && !ep.holders.contains(x) && self.seal_key_of(c, **x).is_some()).map(|(_, x)| *x).collect();
        if missing.is_empty() { (KeyWork::Keep, 0) } else { (KeyWork::Give(missing), rank) }
    }

    /// This device's seal key: an X25519 key derived from its device key, used for nothing
    /// but taking the drop keys of groups it commits in.
    pub(crate) fn seal_key(&self) -> Option<hoi_crypto::envelope::XKey> {
        seal_xkey(self.device_kp.as_ref()?)
    }

    /// The seal key a committer's device announced in group `c`, if this device heard it.
    fn seal_key_of(&self, c: Id, device: Id) -> Option<[u8; 32]> {
        self.private.seal_keys.get(&c)?.get(&device).and_then(|k| unb64(k).ok()).and_then(|b| <[u8; 32]>::try_from(b).ok())
    }

    /// A seal key or drop key from `sender`, in `g`'s epoch: taken only from a committer's
    /// device, and a drop key only by the committer's device it is for. Nothing for
    /// any other member.
    fn take_key_payload(&mut self, c: Id, g: &MlsGroup, sender: &PrivateMember, payload: Payload) -> R<()> {
        let from = parse(&sender.device)?;
        let line = self.committers(c, g);
        let in_line = |d: Id| line.iter().any(|x| x.device == d);
        let me = self.state.me.as_ref().ok_or_else(ApiError::no_identity)?.device;
        match payload {
            Payload::SealKey { v: 1, community, key } if community == c => {
                let Some(key) = unb64(&key).ok().filter(|k| k.len() == 32) else { return Err(fail()) };
                if !self.follows_records(c) || !in_line(from) {
                    return Err(ApiError::not_allowed("Only committers' devices hold the group's drop key."));
                }
                let keys = self.private.seal_keys.entry(c).or_default();
                keys.retain(|d, _| in_line(*d));
                keys.insert(from, b64(&key));
                self.private.dirty = true;
                Ok(())
            }
            Payload::DropKey { v: 1, community, to, key, sealed } if community == c => {
                if to != me {
                    return Ok(());
                }
                if !self.follows_records(c) || !in_line(from) || !in_line(me) {
                    return Err(ApiError::not_allowed("Only committers' devices hold the group's drop key."));
                }
                let public: [u8; 32] = unb64(&key).ok().and_then(|b| b.try_into().ok()).ok_or_else(fail)?;
                let sealed = unb64(&sealed).map_err(mls)?;
                let mine = self.seal_key().ok_or_else(ApiError::no_identity)?;
                let mut secret = hoi_crypto::envelope::open_drop_key(&mine, &drop_key_aad(c, me, &public), &sealed).map_err(|_| fail())?;
                if hoi_crypto::Keypair::from_secret(&secret).public().0 != public {
                    secret.zeroize();
                    return Err(fail());
                }
                self.hold_drop_key(c, secret);
                secret.zeroize();
                Ok(())
            }
            _ => Err(fail()),
        }
    }

    /// Before a commit of the `rank`th committer in `g`'s epoch is followed (this device's own
    /// included): while committers ahead of it could commit in the same epoch, the epoch is
    /// kept, so that their commit, should one turn up in their slots, counts instead.
    /// Only epochs with mailboxes are kept: their slots are where such a commit would be.
    fn contest_point(&self, p: &OpenMlsRustCrypto, g: &MlsGroup, c: Id, rank: Option<usize>) -> Option<Contest> {
        let rank = rank.filter(|r| *r > 0 && self.follows_records(c))?;
        let ep = self.epoch_of(p, g, self.private.groups.get(&c)?).ok()?;
        if ep.mailboxes.is_empty() {
            return None;
        }
        let read = self.scatter.saved.mailboxes.get(&c).filter(|m| m.read_epoch == ep.epoch).map(|m| m.read.clone()).unwrap_or_default();
        let ahead: BTreeMap<u32, u32> =
            self.committers(c, g).iter().take(rank).filter(|x| x.leaf != ep.own).map(|x| (x.leaf, read.get(&x.leaf).copied().unwrap_or(0))).collect();
        if ahead.is_empty() {
            return None;
        }
        Some(Contest { epoch: ep.epoch, rank, until: self.now() + CONTEST, storage: snapshot(p, g, c), slot_key: ep.slot_key, mailboxes: ep.mailboxes, ahead })
    }

    /// Keeps a contest point, once the commit it was taken before has been followed.
    fn keep_contest(&mut self, c: Id, contest: Option<Contest>) {
        if let Some(ct) = contest {
            let list = self.private.contests.entry(c).or_default();
            list.retain(|x| x.epoch < ct.epoch);
            list.push(ct);
            self.private.dirty = true;
        }
    }

    /// The epochs of group `c` still contested, expired ones let go: what their slots are read for.
    pub(crate) fn private_contests(&mut self, c: Id) -> Vec<Contest> {
        let now = self.now();
        let current = self.private_epoch(c).map(|e| e.epoch);
        let Some(list) = self.private.contests.get_mut(&c) else { return vec![] };
        let before = list.len();
        for ct in list.iter_mut().filter(|ct| now >= ct.until || current.is_none_or(|e| ct.epoch >= e)) {
            for (k, v) in &mut ct.storage {
                k.zeroize();
                v.zeroize();
            }
        }
        list.retain(|ct| now < ct.until && current.is_some_and(|e| ct.epoch < e));
        if list.len() != before {
            self.private.dirty = true;
        }
        let out = list.clone();
        if out.is_empty() {
            self.private.contests.remove(&c);
        }
        out
    }

    /// Slot `seq` of the committer at `leaf` in contested epoch `epoch` is read: the next is due.
    pub(crate) fn private_contest_read(&mut self, c: Id, epoch: u64, leaf: u32, seq: u32) {
        if let Some(next) = self.private.contests.get_mut(&c).and_then(|l| l.iter_mut().find(|ct| ct.epoch == epoch)).and_then(|ct| ct.ahead.get_mut(&leaf)) {
            *next = (*next).max(seq + 1);
            self.private.dirty = true;
        }
    }

    /// A slot of a committer ahead, in an epoch this device followed a later committer's commit
    /// out of. Records it carries are taken; a commit of that committer that the records back
    /// counts instead of the one followed: the group goes back to that epoch, follows it, and
    /// sends everything again in the epoch it leads to. Whatever this device and others did in
    /// the epochs it leaves is let go. Returns whether the group went back.
    pub(crate) fn private_contest_receive(&mut self, c: Id, epoch: u64, body: &[u8]) -> R<bool> {
        let Some(at) = self.private.contests.get(&c).and_then(|l| l.iter().position(|ct| ct.epoch == epoch)) else { return Ok(false) };
        let ct = self.private.contests[&c][at].clone();
        let p = provider_of(&ct.storage);
        let mut g = load(&p, c)?;
        if g.epoch().as_u64() != epoch || !g.is_active() {
            return Err(fail());
        }
        let raw = relay(&p, &g)?.open(body)?;
        let msg = MlsMessageIn::tls_deserialize_exact(&raw).map_err(mls)?.try_into_protocol_message().map_err(mls)?;
        if msg.wire_format() != WireFormat::PrivateMessage || msg.group_id().as_slice() != c.as_bytes() || msg.epoch() != g.epoch() {
            return Err(fail());
        }
        let processed = g.process_message(&p, msg).map_err(mls)?;
        let credential = processed.credential().clone();
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(app) => {
                // what backs a commit comes ahead of it: records are taken, anything else was sent
                // again in the epoch everyone moved to
                let payload: Payload = decoded(&app.into_bytes())?;
                let sender = self.private_application_sender(&credential, &payload)?;
                if let Payload::Records { v: 1, community, records } = &payload
                    && *community == c
                {
                    self.private_ingest(c, decode_records(c, records)?)?;
                }
                // a seal key or drop key given in that branch is kept, whichever branch counts
                if matches!(payload, Payload::SealKey { .. } | Payload::DropKey { .. }) {
                    self.take_key_payload(c, &g, &sender, payload)?;
                }
                if let Some(x) = self.private.contests.get_mut(&c).and_then(|l| l.get_mut(at)) {
                    x.storage = snapshot(&p, &g, c);
                    self.private.dirty = true;
                }
                Ok(false)
            }
            ProcessedMessageContent::StagedCommitMessage(commit) => {
                self.private_membership_author(c, &credential)?;
                let sender = parse(&member(&credential, None)?.device)?;
                let line = self.committers(c, &g);
                let Some(rank) = line.iter().position(|x| x.device == sender) else { return Err(fail()) };
                if rank >= ct.rank {
                    return Ok(false);
                }
                self.check_record_commit(c, &g, &commit)?;
                // the epoch as it was, for a contest that goes on while someone earlier still could
                let mut next = None;
                if rank > 0 {
                    let ahead: BTreeMap<u32, u32> = line.iter().take(rank).filter_map(|x| ct.ahead.get(&x.leaf).map(|s| (x.leaf, *s))).collect();
                    if !ahead.is_empty() {
                        next = Some(Contest { rank, ahead, storage: snapshot(&p, &g, c), ..ct.clone() });
                    }
                }
                g.merge_staged_commit(&p, *commit).map_err(mls)?;
                for m in g.members() {
                    member(&m.credential, Some(&m.signature_key))?;
                }
                self.forget_passed(c, &g);
                let active = g.is_active();
                drop(g);
                self.adopt(c, epoch, &p)?;
                if let Some(list) = self.private.contests.get_mut(&c) {
                    for x in list.iter_mut().skip(at) {
                        for (k, v) in &mut x.storage {
                            k.zeroize();
                            v.zeroize();
                        }
                    }
                    list.truncate(at);
                }
                self.keep_contest(c, next);
                if !active && let Some(saved) = self.private.groups.get_mut(&c) {
                    saved.partial.clear();
                }
                Ok(true)
            }
            _ => Err(fail()),
        }
    }

    /// Takes up the MLS state `from` holds for group `c` in place of this device's: the branch
    /// a later committer's commit led to is let go, and the group's sending starts afresh in
    /// the epoch taken up (what was queued for the branch let go is dropped, everything is sent
    /// again).
    fn adopt(&mut self, c: Id, epoch: u64, from: &OpenMlsRustCrypto) -> R<()> {
        let needle = group_needle(c);
        let current = self.private.provider();
        let leaf_key = |g: &MlsGroup| g.own_leaf_node().and_then(|l| serde_json::to_vec(l.encryption_key()).ok());
        let abandoned = load(&current, c).ok().and_then(|g| leaf_key(&g));
        let kept = leaf_key(&load(from, c)?);
        {
            let mut values = current.storage().values.write().expect("MLS storage lock");
            // the branch's own leaf key goes with it (when this device's commit in it made a new one)
            let gone = abandoned.filter(|a| kept.as_ref() != Some(a));
            values.retain(|k, _| !has(k, &needle) && !gone.as_ref().is_some_and(|l| k.starts_with(LEAF_KEY_LABEL) && has(k, l)));
            for (k, v) in from.storage().values.read().expect("MLS storage lock").iter() {
                values.insert(k.clone(), v.clone());
            }
        }
        if let Some(saved) = self.private.groups.get_mut(&c) {
            saved.objects.clear();
            saved.send_all_again();
        }
        // invitations that came after `epoch` were made in the branch let go
        let late: Vec<String> =
            self.private.invited_in.get(&c).map(|m| m.iter().filter(|(_, e)| **e > epoch).map(|(d, _)| d.clone()).collect()).unwrap_or_default();
        for d in late {
            self.private.invitations.get_mut(&c).map(|m| m.remove(&d));
            self.private.invited_in.get_mut(&c).map(|m| m.remove(&d));
        }
        // the seal key goes again in the epoch taken up
        self.private.announced.remove(&c);
        self.mail_restart(c);
        self.private.save(&current);
        Ok(())
    }

    /// A `dropKey` message carrying `secret` as group `c`'s drop key, from this device to
    /// `to`, sealed for the current epoch's room: what a committer's device gives, made here by
    /// any member (tests check who may give one).
    #[cfg(test)]
    pub(crate) fn sealed_drop_key(&mut self, c: Id, to: Id, secret: [u8; 32]) -> String {
        let p = self.private.provider();
        let mut g = load(&p, c).unwrap();
        let (signer, _) = self.private_signer().unwrap();
        let public = hoi_crypto::Keypair::from_secret(&secret).public().0;
        let xk = self.seal_key_of(c, to).unwrap();
        let sealed = hoi_crypto::envelope::seal_drop_key(&xk, &drop_key_aad(c, to, &public), &secret).unwrap();
        let raw = encoded(&Payload::DropKey { v: 1, community: c, to, key: b64(&public), sealed: b64(&sealed) }).unwrap();
        let m = g.create_message(&p, &signer, &raw).unwrap();
        let body = b64(&relay(&p, &g).unwrap().seal(&m.tls_serialize_detached().unwrap()).unwrap());
        self.private.save(&p);
        body
    }

    /// For an owner of a members-only community that is not private (its records travel in
    /// the clear): a private copy, an MLS group with the same name, rules and limits, whose
    /// membership follows its own records. Posts stay where they were; people join the copy by
    /// request, as its admission says (SCATTER.md S4).
    pub fn private_copy(&mut self, community: &str) -> R<CommunityView> {
        let c = parse(community)?;
        if self.is_private_community(&c) {
            return Err(ApiError::invalid("This community is private already."));
        }
        let me = self.state.me.as_ref().ok_or_else(ApiError::no_identity)?.root;
        let st = self.communities.get(&c).ok_or_else(|| ApiError::not_found("community"))?;
        if !st.is_owner(&me) {
            return Err(ApiError::not_allowed("Only an owner can make a private copy."));
        }
        if !matches!(st.limits.admission, hoi_types::Admission::Invite | hoi_types::Admission::Approve) {
            return Err(ApiError::not_allowed("An open community is public: make a new private community instead."));
        }
        let draft = CommunityDraft {
            name: st.genesis.name.clone(),
            description: st.genesis.description.clone(),
            color: st.genesis.color.clone(),
            limits: Some(crate::api::patch_of(&crate::views::limits_dto(&st.limits))),
            rules: st.rules.as_ref().map(|r| r.text.clone()),
            co_owners: None,
            threshold: None,
        };
        self.create_private_community(draft)
    }

    /// Whether a public community could be given a private copy by this device (its owner).
    pub(crate) fn can_copy_private(&self, c: &Id) -> bool {
        let Some(me) = self.state.me.as_ref() else { return false };
        !self.is_private_community(c)
            && self
                .communities
                .get(c)
                .is_some_and(|st| st.is_owner(&me.root) && matches!(st.limits.admission, hoi_types::Admission::Invite | hoi_types::Admission::Approve))
    }
}

/// A device's seal key: X25519, derived from its device key, used for nothing but taking drop
/// keys, a private group's and an open community's (S5).
pub(crate) fn seal_xkey(device: &Keypair) -> Option<hoi_crypto::envelope::XKey> {
    let mut secret = device.secret();
    let mut seed = blake3::derive_key("hoi drop key seal v1", &secret);
    secret.zeroize();
    let key = hoi_crypto::envelope::XKey::from_seed(&seed).ok();
    seed.zeroize();
    key
}

/// The public part of a device's seal key, as its delegation carries it.
pub(crate) fn seal_public(device: &Keypair) -> Option<Id> {
    seal_xkey(device).map(|k| Id(k.public()))
}

/// What a drop key sealed to a committer's device is bound to: the group, the device, and the
/// key's public part.
pub(crate) fn drop_key_aad(c: Id, to: Id, public: &[u8; 32]) -> Vec<u8> {
    let mut aad = b"hoi drop key v1".to_vec();
    aad.extend_from_slice(c.as_bytes());
    aad.extend_from_slice(to.as_bytes());
    aad.extend_from_slice(public);
    aad
}

/// The label OpenMLS's memory storage files a leaf's encryption key pair under.
const LEAF_KEY_LABEL: &[u8] = b"EncryptionKeyPair";
/// What every storage key of group `c` contains: its group id as the storage writes it.
fn group_needle(c: Id) -> Vec<u8> {
    serde_json::to_vec(&GroupId::from_slice(c.as_bytes())).unwrap_or_default()
}
fn has(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}
/// Group `c`'s entries of the MLS storage, and its own leaf's key pair: everything needed to
/// take up its epoch again, nothing of other groups.
fn snapshot(p: &OpenMlsRustCrypto, g: &MlsGroup, c: Id) -> Vec<(Vec<u8>, Vec<u8>)> {
    let needle = group_needle(c);
    let leaf = g.own_leaf_node().and_then(|l| serde_json::to_vec(l.encryption_key()).ok());
    p.storage()
        .values
        .read()
        .expect("MLS storage lock")
        .iter()
        .filter(|(k, _)| has(k, &needle) || leaf.as_ref().is_some_and(|l| k.starts_with(LEAF_KEY_LABEL) && has(k, l)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
fn provider_of(storage: &[(Vec<u8>, Vec<u8>)]) -> OpenMlsRustCrypto {
    let p = OpenMlsRustCrypto::default();
    p.storage().values.write().expect("MLS storage lock").extend(storage.iter().cloned());
    p
}

fn validate_wire(c: Id, bytes: &[u8]) -> R<()> {
    let d = hoi_validate::decode(bytes)?;
    hoi_validate::signature(&d)?;
    if d.wire.record.community.is_some_and(|other| other != c)
        || (d.wire.record.community.is_none() && !matches!(d.wire.record.kind(), Kind::Identity | Kind::Delegation | Kind::Revoke))
    {
        return Err(fail());
    }
    Ok(())
}
fn decode_records(c: Id, records: &[String]) -> R<Vec<Vec<u8>>> {
    if records.len() > 2048 {
        return Err(fail());
    }
    records
        .iter()
        .map(|r| {
            let bytes = unb64(r).map_err(mls)?;
            validate_wire(c, &bytes)?;
            Ok(bytes)
        })
        .collect()
}

#[cfg(test)]
mod adversarial {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    fn client(name: &str, now: &Arc<AtomicU64>) -> Node {
        let time = now.clone();
        let mut n = Node::open(Box::new(hoi_store::Ephemeral), "web", "https://hoi.invalid/", Box::new(move || time.load(Ordering::SeqCst))).unwrap();
        n.set_identity_bits(4);
        n.create_identity(name).unwrap();
        n
    }
    fn community(a: &mut Node) -> String {
        a.create_private_community(CommunityDraft {
            name: "private test".into(),
            description: "".into(),
            color: "mint".into(),
            limits: Some(crate::dto::LimitsPatch {
                pow_bits: Some(8),
                ladder: Some(crate::dto::LadderPatch { identity_bits: Some(4), ..Default::default() }),
                ..Default::default()
            }),
            rules: None,
            co_owners: None,
            threshold: None,
        })
        .unwrap()
        .id
    }
    fn sync(a: &mut Node, b: &mut Node, c: &str) {
        let out = a.private_outbox(c).unwrap();
        deliver(b, c, &out);
        a.private_ack(c, &out.iter().map(|o| o.id.clone()).collect::<Vec<_>>()).unwrap();
    }
    fn pair(a: &mut Node, b: &mut Node, c: &str) {
        let request = b.private_join_request().unwrap();
        let inv = a.private_invite(c, &request).unwrap();
        b.private_join(&inv).unwrap();
        sync(a, b, c);
    }
    #[test]
    fn a_commit_sends_again_only_what_went_in_the_epoch_it_ended() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let mut b = client("member", &now);
        let mut d = client("leaving", &now);
        let mut e = client("leaving too", &now);
        let c = community(&mut a);
        let cid = parse(&c).unwrap();
        pair(&mut a, &mut d, &c);
        pair(&mut a, &mut e, &c);
        pair(&mut a, &mut b, &c);
        let post = |n: &mut Node, title: &str| {
            parse(
                &n.create_post(&crate::dto::PostDraft { community: c.clone(), title: title.into(), text: String::new(), link: None, media: None }).unwrap().id,
            )
            .unwrap()
        };
        let early = post(&mut b, "sent two epochs ago");
        let out = b.private_outbox(&c).unwrap();
        b.private_ack(&c, &out.iter().map(|o| o.id.clone()).collect::<Vec<_>>()).unwrap();
        // a removes d: b follows, and what b sent in the epoch that ended goes again
        a.private_remove(&c, &d.me().unwrap().device).unwrap();
        sync(&mut a, &mut b, &c);
        assert!(!b.private.groups[&cid].sent_records.contains(&early), "sent in the epoch the commit ended: again");
        b.private_outbox(&c).unwrap();
        let late = post(&mut b, "sent one epoch ago");
        let out = b.private_outbox(&c).unwrap();
        b.private_ack(&c, &out.iter().map(|o| o.id.clone()).collect::<Vec<_>>()).unwrap();
        assert!(b.private.groups[&cid].sent_records.contains(&early) && b.private.groups[&cid].sent_records.contains(&late));
        // a removes e: only what went in the epoch since goes again, not the whole history
        a.private_remove(&c, &e.me().unwrap().device).unwrap();
        sync(&mut a, &mut b, &c);
        let sent = &b.private.groups[&cid].sent_records;
        assert!(!sent.contains(&late), "the last epoch's records go again");
        assert!(sent.contains(&early), "older ones do not: every member had them");
        assert!(!a.private.groups[&cid].sent_records.is_empty() || a.private_pending(cid).len() <= 2, "the committer sends nothing again for a removal");
    }
    #[test]
    fn requests_without_every_capability_are_refused() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut owner = client("owner", &now);
        let mut old = client("old app", &now);
        let community = community(&mut owner);
        let cid = parse(&community).unwrap();
        let required = load(&owner.private.provider(), cid).unwrap().extensions().required_capabilities().map(|r| r.extension_types().to_vec());
        assert_eq!(required, Some(REQUIRED.to_vec()), "every capability is required from the group's first epoch");
        for listed in [&[][..], &REQUIRED[..4], &REQUIRED[1..]] {
            let request = request_with_capabilities(&mut old, Capabilities::new(None, None, Some(listed), None, None));
            assert_eq!(owner.private_preview_request(&request).unwrap_err().code, "not-allowed");
            assert!(owner.private_invite(&community, &request).is_err());
        }
        let epoch = owner.private_group(&community).unwrap().epoch;
        let request = old.private_join_request().unwrap();
        old.private_join(&owner.private_invite(&community, &request).unwrap()).unwrap();
        assert_eq!(owner.private_group(&community).unwrap().epoch, epoch + 1, "a request listing them all is added");
    }
    fn request_with_capabilities(node: &mut Node, capabilities: Capabilities) -> String {
        let (signer, credential) = node.private_signer().unwrap();
        let provider = node.private.provider();
        let package = KeyPackage::builder().leaf_node_capabilities(capabilities).build(SUITE, &provider, &signer, credential).unwrap();
        let me = node.state.me.as_ref().unwrap();
        let request = JoinRequest {
            v: 1,
            package: b64(&package.key_package().tls_serialize_detached().unwrap()),
            records: node.identity_wires(me.root, me.device).unwrap(),
            pickup: crate::pickup::Pickup { key: b64(&[7; 32]), holders: vec![] },
        };
        let code = format!("hoi:private-request:{}", b64(&encoded(&request).unwrap()));
        node.private.save(&provider);
        code
    }
    #[test]
    fn private_mls_nonowner_cannot_commit_and_failed_commit_does_not_advance_epoch() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let mut b = client("member", &now);
        let mut outsider = client("outsider", &now);
        let c = community(&mut a);
        pair(&mut a, &mut b, &c);
        let request = outsider.private_join_request().unwrap();
        let p = b.private.provider();
        let mut g = load(&p, parse(&c).unwrap()).unwrap();
        let (signer, _) = b.private_signer().unwrap();
        let (kp, _, _) = b.checked_request(&request, &p).unwrap();
        let sealer = relay(&p, &g).unwrap();
        let (commit, _, _) = g.add_members(&p, &signer, &[kp]).unwrap();
        let epoch = a.private_group(&c).unwrap().epoch;
        let error = a.private_receive(&c, &b64(&sealer.seal(&commit.tls_serialize_detached().unwrap()).unwrap())).unwrap_err();
        assert_eq!(error.code, "not-allowed");
        assert_eq!(a.private_group(&c).unwrap().epoch, epoch);
        let invite = a.private_invite(&c, &request).unwrap();
        outsider.private_join(&invite).unwrap();
        assert_eq!(a.private_group(&c).unwrap().epoch, epoch + 1);
    }
    #[test]
    fn private_mls_claimed_owner_root_requires_its_real_device_blessing() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let a = client("owner", &now);
        let b = client("attacker", &now);
        let p = b.private.provider();
        let (signer, _) = b.private_signer().unwrap();
        let me = b.state.me.as_ref().unwrap();
        let records = b.identity_wires(me.root, me.device).unwrap();
        let credential = BasicCredential::new(
            encoded(&BoundCredential { root: a.me_root().unwrap().hex(), device: me.device.hex(), delegation: records[1].clone() }).unwrap(),
        )
        .into();
        let kp = KeyPackage::builder().build(SUITE, &p, &signer, CredentialWithKey { credential, signature_key: me.device.0.to_vec().into() }).unwrap();
        let request = JoinRequest {
            v: 1,
            package: b64(&kp.key_package().tls_serialize_detached().unwrap()),
            records,
            pickup: crate::pickup::Pickup { key: b64(&[7; 32]), holders: vec![] },
        };
        let code = format!("hoi:private-request:{}", b64(&encoded(&request).unwrap()));
        assert!(a.private_preview_request(&code).is_err());
    }
    fn verifies(room: &str, statement: &str, signature: &str) -> bool {
        let room = parse(room).unwrap();
        let sig = hoi_types::Sig::from_slice(&hoi_types::hex_decode(signature).unwrap()).unwrap();
        hoi_crypto::verify(&room, statement.as_bytes(), &sig)
    }
    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }
    /// Delivers objects the way the browser transport does: only those of the room this device is
    /// reading now, so a commit moves it on and later objects of the old room are left behind.
    fn deliver(node: &mut Node, c: &str, objects: &[CipherObject]) {
        for o in objects {
            if node.private_group(c).unwrap().room == o.room {
                node.private_receive(c, &o.body).unwrap();
            }
        }
    }
    #[test]
    fn relay_v2_members_upload_after_a_month_without_owner_renewals() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let mut b = client("member", &now);
        let c = community(&mut a);
        pair(&mut a, &mut b, &c);
        now.fetch_add(RETENTION + 1, Ordering::SeqCst);
        let out = b.private_outbox(&c).unwrap();
        assert!(!out.is_empty(), "membership alone lets a device upload; no owner-signed grant expires");
        assert!(out.iter().all(|o| o.expires == b.now() + RETENTION));
        assert!(!b.private.groups[&parse(&c).unwrap()].owner);
        assert!(b.leave(&c).is_ok() && a.leave(&c).is_err(), "only the coordinating owner device must stay");
    }
    #[test]
    fn relay_v2_members_share_the_room_and_outsiders_with_the_community_key_find_nothing() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let mut b = client("member", &now);
        let outsider = client("outsider", &now);
        let c = community(&mut a);
        let cid = parse(&c).unwrap();
        pair(&mut a, &mut b, &c);
        let group = a.private_group(&c).unwrap();
        assert_eq!(group.room, b.private_group(&c).unwrap().room);
        assert_ne!(group.room, c, "the room is not the community key or MLS group id");
        // Public material derives nothing: not the room, and no key that opens an object.
        let guessed = Relay::from_secret(blake3::derive_key(RELAY_LABEL, cid.as_bytes()));
        assert_ne!(guessed.room().hex(), group.room);
        assert!(outsider.private_read(&c, &["objects?after=0".into()]).is_err());
        assert!(outsider.private_group(&c).is_err());
        for node in [&a, &b] {
            let read = node.private_read(&c, &["objects?after=0".into()]).unwrap();
            assert_eq!(read.room, group.room);
            let proof = &read.reads[0];
            assert!(verifies(&read.room, &read_text(parse(&read.room).unwrap(), "objects?after=0", proof.time, &proof.nonce), &proof.signature));
        }
        let post =
            a.create_post(&crate::dto::PostDraft { community: c.clone(), title: "members only".into(), text: String::new(), link: None, media: None }).unwrap();
        let out = a.private_outbox(&c).unwrap();
        let devices = [a.me().unwrap().device, b.me().unwrap().device].map(|d| parse(&d).unwrap());
        let p = a.private.provider();
        let inner = relay(&p, &load(&p, cid).unwrap()).unwrap();
        for o in &out {
            let body = unb64(&o.body).unwrap();
            assert_eq!((o.room.as_str(), body.len(), padded(o.size)), (group.room.as_str(), o.size, o.size));
            assert!(verifies(&o.room, &object_text(parse(&o.room).unwrap(), &o.id, o.expires, o.size), &o.signature));
            assert!(contains(&inner.open(&body).unwrap(), cid.as_bytes()), "MLS puts the group id in its header");
            assert!(!contains(&body, cid.as_bytes()), "the seal hides it from the relay");
            assert!(devices.iter().all(|d| !contains(&body, d.as_bytes())));
            assert!(guessed.open(&body).is_err());
        }
        let json = serde_json::to_string(&out).unwrap();
        assert!(!json.contains(&c) && devices.iter().all(|d| !json.contains(&d.hex())), "uploads name no community or member");
        deliver(&mut b, &c, &out);
        assert!(b.post(&post.id).is_ok());
    }
    #[test]
    fn relay_v2_rooms_follow_epochs_for_new_members_and_not_for_removed_ones() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let mut b = client("member", &now);
        let mut d = client("newcomer", &now);
        let c = community(&mut a);
        pair(&mut a, &mut b, &c);
        let early = a
            .create_post(&crate::dto::PostDraft { community: c.clone(), title: "before the newcomer".into(), text: String::new(), link: None, media: None })
            .unwrap();
        sync(&mut a, &mut b, &c);
        let first = a.private_group(&c).unwrap().room;
        let invitation = a.private_invite(&c, &d.private_join_request().unwrap()).unwrap();
        d.private_join(&invitation).unwrap();
        let second = a.private_group(&c).unwrap().room;
        assert_ne!(first, second, "a commit moves the group to a new room");
        assert_eq!(d.private_group(&c).unwrap().room, second, "the Welcome gives a newcomer the current room");
        let out = a.private_outbox(&c).unwrap();
        assert_eq!(out[0].room, first, "the commit is left where current members read");
        assert!(out[1..].iter().all(|o| o.room == second));
        assert!(d.private_group(&c).unwrap().received.contains(&out[1].id), "the invitation's bootstrap is the uploaded object");
        deliver(&mut b, &c, &out);
        deliver(&mut d, &c, &out);
        a.private_ack(&c, &out.iter().map(|o| o.id.clone()).collect::<Vec<_>>()).unwrap();
        assert_eq!(b.private_group(&c).unwrap().room, second);
        assert!(d.post(&early.id).is_ok(), "history the newcomer may read is resent in its room");

        a.private_remove(&c, &b.me().unwrap().device).unwrap();
        let third = a.private_group(&c).unwrap().room;
        // a removal sends nothing again (S6); what is new goes in the next room
        a.create_post(&crate::dto::PostDraft { community: c.clone(), title: "after the removal".into(), text: String::new(), link: None, media: None })
            .unwrap();
        let removal = a.private_outbox(&c).unwrap();
        assert_eq!(removal[0].room, second);
        assert!(removal[1..].iter().all(|o| o.room == third) && !removal[1..].is_empty());
        deliver(&mut b, &c, &removal[..1]);
        deliver(&mut d, &c, &removal);
        let removed = b.private_group(&c).unwrap();
        assert!(!removed.active && removed.room.is_empty(), "a removed device cannot name the next room");
        assert!(b.private_read(&c, &["objects?after=0".into()]).is_err());
        assert!(removal[1..].iter().all(|o| b.private_receive(&c, &o.body).is_err()));
        assert_eq!(d.private_group(&c).unwrap().room, third);
    }
    #[test]
    fn relay_v2_padding_is_bounded_and_seals_round_trip() {
        let mut previous = 0;
        for size in (1..=MAX_OBJECT).step_by(997) {
            let bucket = padded(size);
            assert!(bucket >= size.max(PAD_MIN) && bucket >= previous && bucket <= MAX_OBJECT.max(size));
            assert!(size <= PAD_MIN || bucket * 8 <= size * 9, "at most 1/8 overhead past 4 KiB: {size} → {bucket}");
            assert_eq!(bucket % (1 << (usize::BITS - 4 - bucket.leading_zeros())), 0, "four significant bits");
            previous = bucket;
        }
        assert_eq!([1, 4096, 4097, 8191, 8193, 15361, MAX_OBJECT].map(padded), [4096, 4096, 4608, 8192, 9216, 16384, MAX_OBJECT]);
        let relay = Relay::from_secret([7; 32]);
        for len in [0, 1, PAD_MIN - 44, PAD_MIN - 43, 100_000, MAX_OBJECT - 44] {
            let message: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let sealed = relay.seal(&message).unwrap();
            assert_eq!(sealed.len(), padded(len + 44));
            assert_eq!(relay.open(&sealed).unwrap(), message);
            assert!(Relay::from_secret([8; 32]).open(&sealed).is_err(), "another epoch cannot open it");
            let mut tampered = sealed.clone();
            tampered[30] ^= 1;
            assert!(relay.open(&tampered).is_err());
            assert!(relay.open(&sealed[..sealed.len() - 512]).is_err());
        }
        assert!(relay.seal(&vec![0; MAX_OBJECT - 43]).is_err());
        let mut plain = vec![0u8; PAD_MIN - SEAL_OVERHEAD];
        plain[..4].copy_from_slice(&1u32.to_be_bytes());
        plain[100] = 1;
        assert!(relay.open(&hoi_crypto::sealed::seal(SEAL_CONTEXT, &[7; 32], &plain).unwrap()).is_err(), "padding must be zeros");
        plain[100] = 0;
        plain[..4].copy_from_slice(&(PAD_MIN as u32).to_be_bytes());
        assert!(relay.open(&hoi_crypto::sealed::seal(SEAL_CONTEXT, &[7; 32], &plain).unwrap()).is_err(), "length within the object");
    }
    #[test]
    fn relay_v2_reads_are_signed_once_per_allowed_target() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let c = community(&mut a);
        let targets: Vec<String> = vec!["objects?after=0".into(), "objects?after=17".into(), format!("objects/{}", "a1".repeat(32))];
        let read = a.private_read(&c, &targets).unwrap();
        assert_eq!(read.room, a.private_group(&c).unwrap().room);
        let room = parse(&read.room).unwrap();
        let mut nonces = BTreeSet::new();
        for (target, proof) in targets.iter().zip(&read.reads) {
            assert_eq!(proof.time, a.now());
            assert!(proof.nonce.len() == 32 && nonces.insert(proof.nonce.clone()));
            assert!(verifies(&read.room, &read_text(room, target, proof.time, &proof.nonce), &proof.signature));
            assert!(!verifies(&read.room, &read_text(room, "objects?after=1", proof.time, &proof.nonce), &proof.signature));
        }
        let upper = format!("objects/{}", "A1".repeat(32));
        for bad in ["", "objects", "objects?after=", "objects?after=01", "objects?after=-1", "objects?after=1&x=1", "objects/../meta", "meta", &upper] {
            assert_eq!(a.private_read(&c, &[bad.to_string()]).unwrap_err().code, "invalid", "{bad}");
        }
        assert!(a.private_read(&c, &[]).is_err());
        assert!(a.private_read(&c, &vec!["objects?after=0".into(); MAX_READS + 1]).is_err());
        let many = a.private_read(&c, &vec!["objects?after=0".into(); MAX_READS]).unwrap();
        assert_eq!(many.reads.iter().map(|r| &r.nonce).collect::<BTreeSet<_>>().len(), MAX_READS);
    }
    /// The same bytes are checked by relay/worker.test.mjs, so both sides agree on every statement.
    #[test]
    fn relay_v2_shared_vector() {
        let relay = Relay::from_secret([7; 32]);
        let room = relay.room();
        let id = "5".repeat(64);
        let object = relay.signed(&object_text(room, &id, 1_800_003_600, 8192));
        let read = relay.signed(&read_text(room, "objects?after=0", 1_800_000_000, &"0f".repeat(16)));
        assert_eq!(room.hex(), VECTOR_ROOM);
        assert_eq!(object, VECTOR_OBJECT);
        assert_eq!(read, VECTOR_READ);
    }
    const VECTOR_ROOM: &str = "c7c825f514ccb7c593e05a62df9f4d376bc1f1015580caf4c5cca3f36c486d36";
    const VECTOR_OBJECT: &str =
        "7b6eca5889483df1a0386d9b3027db90c87af5ceba4d94c4041ecf162dcbb25cadb42b8f80a9b1ce6329c5a59ecf60044fc79ee3aee673debb968b75a6d4b50b";
    const VECTOR_READ: &str =
        "525ce23556d6718103e9b49970dd0976c5569bf5951dc8136ada36794c7246a8fa0ce7e70f9c5c152537485744095e0fea4d8f032dda4d843989e756becf0304";
    #[test]
    fn private_mls_revoked_owner_device_cannot_sign_membership() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let mut b = client("member", &now);
        let c = community(&mut a);
        pair(&mut a, &mut b, &c);
        let device = a.state.me.as_ref().unwrap().device;
        let delegation = a.store.delegation(&device).unwrap().record;
        let original = a.store.get(&delegation).unwrap().wire.clone();
        let root = a.root_kp.as_ref().unwrap();
        let mut revoke = hoi_types::Record {
            v: 1,
            community: None,
            author: root.public(),
            parent: None,
            created_at: a.now(),
            expires_at: a.now() + hoi_validate::KEEP,
            body: Body::Revoke { device },
            nonce: 0,
        };
        revoke.nonce = hoi_crypto::pow::mine(&revoke, hoi_types::GLOBAL_POW_BITS);
        let wire = root.sign_record(revoke).0.encode();
        a.ingest(&wire, &Source::PrivateRelay).unwrap();
        assert!(a.private_outbox(&c).is_err());
        let d = BoundCredential { device: device.hex(), root: a.me_root().unwrap().hex(), delegation: b64(&original) };
        let cred = BasicCredential::new(encoded(&d).unwrap()).into();
        assert!(a.private_sender(&cred).is_err());
    }
    #[test]
    fn private_mls_join_request_reuses_one_pending_key_package() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("member", &now);
        let request = a.private_join_request().unwrap();
        let stored = a.private.storage.len();
        for _ in 0..32 {
            assert_eq!(request, a.private_join_request().unwrap());
        }
        assert_eq!(stored, a.private.storage.len());
        now.fetch_add(8 * 86400, Ordering::SeqCst);
        assert_ne!(request, a.private_join_request().unwrap());
        assert_eq!(stored, a.private.storage.len());
    }
    #[test]
    fn private_mls_restricted_owner_device_cannot_manage_membership() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let c = community(&mut a);
        let me = a.state.me.as_ref().unwrap();
        let root = a.root_kp.as_ref().unwrap();
        let scope = vec!["post".to_string()];
        let expires = a.now() + 86400;
        let blessing = hoi_crypto::bless(root, &me.device, &scope, expires);
        let record = hoi_types::Record {
            v: 1,
            community: None,
            author: me.device,
            parent: None,
            created_at: a.now(),
            expires_at: expires,
            body: Body::Delegation { root: me.root, scope, expires_at: expires, root_sig: blessing, seal: None },
            nonce: 0,
        };
        let wire = a.device_kp.as_ref().unwrap().sign_record(record).0.encode();
        let bound = BoundCredential { root: me.root.hex(), device: me.device.hex(), delegation: b64(&wire) };
        let cred = BasicCredential::new(encoded(&bound).unwrap()).into();
        assert!(a.private_sender(&cred).is_ok());
        assert!(a.private_membership_author(parse(&c).unwrap(), &cred).is_err());
        now.fetch_add(86401, Ordering::SeqCst);
        assert!(a.private_sender(&cred).is_err());
    }
    #[test]
    fn private_mls_signed_renewal_recovers_offline_members_after_original_blessing_expires() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let mut b = client("member", &now);
        let c = community(&mut a);
        pair(&mut a, &mut b, &c);
        let p = a.private.provider();
        let g = load(&p, parse(&c).unwrap()).unwrap();
        let old = g.credential().unwrap().clone();
        now.fetch_add(crate::api::DELEGATION_LIFETIME + 1, Ordering::SeqCst);
        assert!(a.private_sender(&old).is_err());
        assert!(b.private_sender(&old).is_err());
        a.renew_delegation();
        assert!(a.private_sender(&old).is_ok(), "newer root-signed authorization extends the same MLS device");
        assert!(a.private_membership_author(parse(&c).unwrap(), &old).is_ok());
        let out = a.private_outbox(&c).unwrap();
        // Grant renewal may precede the signed identity renewal on a relay. The receiver can
        // safely retry it after the records message supplies the root's fresh authorization.
        let mut pending = out.clone();
        for _ in 0..3 {
            pending.retain(|o| b.private_receive(&c, &o.body).is_err());
        }
        assert!(pending.is_empty());
        assert!(b.private_sender(&old).is_ok());
        a.private_ack(&c, &out.iter().map(|o| o.id.clone()).collect::<Vec<_>>()).unwrap();
        b.renew_delegation();
        assert!(b.private_outbox(&c).is_ok());
        // Membership control also remains operational with the old immutable credential.
        a.private_remove(&c, &b.me().unwrap().device).unwrap();
        let removal = a.private_outbox(&c).unwrap();
        for o in removal {
            let _ = b.private_receive(&c, &o.body);
        }
        assert!(!b.private_group(&c).unwrap().active);
    }
    #[test]
    fn private_mls_renewal_cannot_expand_original_membership_scope_or_revive_revoked_device() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let c = community(&mut a);
        let me = a.state.me.as_ref().unwrap().clone();
        let root = a.root_kp.as_ref().unwrap();
        let scope = vec!["post".to_string()];
        let expires = a.now() + 86400;
        let blessing = hoi_crypto::bless(root, &me.device, &scope, expires);
        let record = hoi_types::Record {
            v: 1,
            community: None,
            author: me.device,
            parent: None,
            created_at: a.now(),
            expires_at: expires,
            body: Body::Delegation { root: me.root, scope, expires_at: expires, root_sig: blessing, seal: None },
            nonce: 0,
        };
        let wire = a.device_kp.as_ref().unwrap().sign_record(record).0.encode();
        let cred = BasicCredential::new(encoded(&BoundCredential { root: me.root.hex(), device: me.device.hex(), delegation: b64(&wire) }).unwrap()).into();
        now.fetch_add(crate::api::DELEGATION_LIFETIME + 1, Ordering::SeqCst);
        a.renew_delegation();
        assert!(a.private_sender(&cred).is_ok());
        assert!(a.private_membership_author(parse(&c).unwrap(), &cred).is_err(), "a broader renewal does not expand the original MLS leaf's scope");
        let root = a.root_kp.as_ref().unwrap();
        let mut revoke = hoi_types::Record {
            v: 1,
            community: None,
            author: me.root,
            parent: None,
            created_at: a.now(),
            expires_at: a.now() + hoi_validate::KEEP,
            body: Body::Revoke { device: me.device },
            nonce: 0,
        };
        revoke.nonce = hoi_crypto::pow::mine(&revoke, hoi_types::GLOBAL_POW_BITS);
        a.ingest(&root.sign_record(revoke).0.encode(), &Source::PrivateRelay).unwrap();
        assert!(a.private_sender(&cred).is_err(), "a genuine renewal never overrides revocation");
    }
    #[test]
    fn private_mls_local_membership_uses_original_leaf_scope_before_advancing_epoch() {
        let now = Arc::new(AtomicU64::new(1_790_000_000));
        let mut a = client("owner", &now);
        let me = a.state.me.as_ref().unwrap().clone();
        let scope = vec!["post".to_string()];
        let expires = a.now() + crate::api::DELEGATION_LIFETIME + 86400;
        let blessing = hoi_crypto::bless(a.root_kp.as_ref().unwrap(), &me.device, &scope, expires);
        let mut record = hoi_types::Record {
            v: 1,
            community: None,
            author: me.device,
            parent: None,
            created_at: a.now(),
            expires_at: expires,
            body: Body::Delegation { root: me.root, scope, expires_at: expires, root_sig: blessing, seal: None },
            nonce: 0,
        };
        record.nonce = hoi_crypto::pow::mine(&record, hoi_types::GLOBAL_POW_BITS);
        a.ingest(&a.device_kp.as_ref().unwrap().sign_record(record).0.encode(), &Source::PrivateRelay).unwrap();
        let c = community(&mut a);
        now.fetch_add(crate::api::DELEGATION_LIFETIME - 20 * 86400, Ordering::SeqCst);
        a.renew_delegation();
        assert!(a.store.delegation(&me.device).unwrap().info.scope.contains(&"*".to_string()));
        let mut b = client("member", &now);
        let request = b.private_join_request().unwrap();
        let epoch = a.private_group(&c).unwrap().epoch;
        assert!(a.private_invite(&c, &request).is_err());
        assert_eq!(a.private_group(&c).unwrap().epoch, epoch);
        assert_eq!(a.private_group(&c).unwrap().pending, 0);
    }
}
