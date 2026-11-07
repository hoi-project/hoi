//! hoi-store: the records this device holds.
//!
//! The working set lives in memory with the indexes the app needs (per community by time,
//! per parent, delegations). Durability goes through [`Persist`]: SQLite on desktop and mobile
//! (feature `sqlite`), and a [`Journal`] the browser host drains into IndexedDB. Expiry is the
//! real deletion mechanism (TECHNICAL.md §8): [`Store::expired`] lists what the sweeper deletes.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use hoi_types::{Body, Id, Kind, Record, Sig};
use hoi_validate::DelegationInfo;

pub mod pieces;
#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "sqlite")]
pub mod vault;

#[derive(Debug, Clone)]
pub struct Stored {
    pub id: Id,
    pub record: Record,
    pub sig: Sig,
    pub wire: Vec<u8>,
    /// Root key behind `record.author`.
    pub author_root: Id,
    pub received_at: u64,
    /// Peers that handed it over; 0 when made on this device.
    pub from_peers: u32,
    pub stamp_bits: u32,
    /// Identity work for `identity` records (PROTOCOL.md §4.1), 0 otherwise.
    pub work_bits: u32,
}

#[derive(Debug, Clone)]
pub struct Delegation {
    pub record: Id,
    pub info: DelegationInfo,
}

/// The best identity record a root has published.
#[derive(Debug, Clone, Copy)]
pub struct IdentityInfo {
    pub record: Id,
    pub bits: u32,
    pub created_at: u64,
    /// The commitment to the recovery key that may replace this root, and the person a later
    /// root continues, as the first identity record this store took for the root said.
    pub rc: Option<Id>,
    pub of: Option<Id>,
}

/// A root replaced by another under a recovery key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rotation {
    pub record: Id,
    pub person: Id,
    pub from: Id,
    pub to: Id,
    pub at: u64,
}

pub type Fingerprint = [u8; 16];

/// Words of a post, comment or edit for search: lowercase, letters and digits only,
/// two characters at least, at most 64 characters each, at most 2,000 per record.
pub fn words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .map(|w| w.to_lowercase().chars().take(64).collect::<String>())
        .take(2000)
        .collect()
}

fn record_words(r: &Record) -> BTreeSet<String> {
    match &r.body {
        Body::Post { title, text, link, .. } => {
            let mut w = words(title);
            w.extend(words(text));
            if let Some(l) = link {
                w.extend(words(l));
            }
            w
        }
        Body::Comment { text, .. } => words(text),
        Body::Edit { text, title } => {
            let mut w = words(text);
            if let Some(t) = title {
                w.extend(words(t));
            }
            w
        }
        _ => BTreeSet::new(),
    }
}

/// Who wrote the same words, when, in which record.
pub type TextUse = (Id, u64, Id);

#[derive(Default)]
pub struct Store {
    records: HashMap<Id, Stored>,
    by_community: HashMap<Id, BTreeSet<(u64, Id)>>,
    children: HashMap<Id, BTreeSet<(u64, Id)>>,
    /// (community, author root) → (created_at, id): rate windows count by records' own time.
    by_author: HashMap<(Id, Id), BTreeSet<(u64, Id)>>,
    /// (community, text fingerprint) → (author root, created_at, id): copy-paste floods.
    texts: HashMap<(Id, Fingerprint), Vec<TextUse>>,
    /// Media hash → records referencing it.
    media_refs: HashMap<Id, BTreeSet<Id>>,
    /// Word → the posts, comments and edits it occurs in, for search. Built from what
    /// this device holds and reads, in memory only: nothing about it is written to disk.
    words: BTreeMap<String, BTreeSet<Id>>,
    delegations: HashMap<Id, Delegation>,
    /// Revocations that arrived before (or without) the delegation: device → (root, when).
    /// Revocations that arrived before their delegation: device → root that revoked → when.
    /// Keyed by root too, so a stranger's revoke for a device it never blessed cannot hide the
    /// real root's revoke behind it.
    revocations: HashMap<Id, BTreeMap<Id, u64>>,
    identities: HashMap<Id, IdentityInfo>,
    /// Roots their owners burned, and when; also every person a burned current root
    /// spoke for, and roots that signed two different recovery commitments.
    burned: HashMap<Id, u64>,
    /// The rotation away from each root (the earliest valid one): root → rotation.
    rotations: HashMap<Id, Rotation>,
    /// Every root after a person's first, to that person.
    persons: HashMap<Id, Id>,
    /// Community key → genesis record id.
    genesis: HashMap<Id, Id>,
    /// Ids whose bodies were deleted (removed or deleted by the author): never re-ingest before `until`.
    deleted: HashMap<Id, u64>,
    /// Media bytes by BLAKE3 hash.
    blobs: HashMap<Id, Vec<u8>>,
    blob_bytes: u64,
    bytes: u64,
}

impl Store {
    pub fn new() -> Store {
        Store::default()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn contains(&self, id: &Id) -> bool {
        self.records.contains_key(id)
    }

    pub fn get(&self, id: &Id) -> Option<&Stored> {
        self.records.get(id)
    }

    pub fn get_mut(&mut self, id: &Id) -> Option<&mut Stored> {
        self.records.get_mut(id)
    }

    pub fn is_deleted(&self, id: &Id) -> bool {
        self.deleted.contains_key(id)
    }

    pub fn mark_deleted(&mut self, id: Id, until: u64) {
        let e = self.deleted.entry(id).or_insert(until);
        *e = (*e).max(until);
    }

    pub fn insert(&mut self, s: Stored) -> bool {
        if self.records.contains_key(&s.id) {
            return false;
        }
        let r = &s.record;
        if let Some(c) = r.community {
            self.by_community.entry(c).or_default().insert((r.created_at, s.id));
            self.by_author.entry((c, s.author_root)).or_default().insert((r.created_at, s.id));
            if let Some(f) = hoi_validate::text_fingerprint(r) {
                self.texts.entry((c, f)).or_default().push((s.author_root, r.created_at, s.id));
            }
        }
        if let Some(p) = r.parent {
            self.children.entry(p).or_default().insert((r.created_at, s.id));
        }
        for w in record_words(r) {
            self.words.entry(w).or_default().insert(s.id);
        }
        match &r.body {
            Body::Community(_) => {
                // the first genesis for a key stays (the node refuses others)
                self.genesis.entry(r.author).or_insert(s.id);
            }
            Body::Delegation { root, scope, expires_at, .. } => {
                let device = r.author;
                let pending = self.revocations.get(&device).and_then(|by_root| by_root.get(root)).copied();
                let keep_old = self.delegations.get(&device).is_some_and(|d| d.info.root != *root || d.info.expires_at >= *expires_at);
                if !keep_old {
                    let revoked_at = self.delegations.get(&device).and_then(|d| d.info.revoked_at).or(pending);
                    self.delegations.insert(
                        device,
                        Delegation { record: s.id, info: DelegationInfo { root: *root, scope: scope.clone(), expires_at: *expires_at, revoked_at } },
                    );
                }
            }
            Body::Revoke { device } if *device == r.author => {
                self.burned.entry(r.author).or_insert(r.created_at);
                let person = self.person_of(&r.author);
                self.burned.entry(person).or_insert(r.created_at);
            }
            Body::Revoke { device } => match self.delegations.get_mut(device) {
                Some(d) if d.info.root == r.author => {
                    d.info.revoked_at = Some(d.info.revoked_at.map_or(r.created_at, |x| x.min(r.created_at)));
                }
                Some(_) => {}
                None => {
                    let e = self.revocations.entry(*device).or_default().entry(r.author).or_insert(r.created_at);
                    *e = (*e).min(r.created_at);
                }
            },
            Body::Identity { rc, of, .. } => match self.identities.get(&r.author).copied() {
                // a root that commits to two recovery keys, or to two persons, was taken: nobody
                // may replace it, and it signs nothing more
                Some(i) if (i.rc, i.of) != (*rc, *of) => {
                    self.burned.entry(r.author).or_insert(r.created_at);
                    let person = self.person_of(&r.author);
                    self.burned.entry(person).or_insert(r.created_at);
                }
                Some(i) if s.work_bits <= i.bits => {}
                _ => {
                    self.identities.insert(r.author, IdentityInfo { record: s.id, bits: s.work_bits, created_at: r.created_at, rc: *rc, of: *of });
                }
            },
            Body::Rotate { person, to } => {
                // the node checked it: the recovery key is the one a root of `person` committed to
                if let Some(from) = self
                    .chain(person)
                    .into_iter()
                    .find(|root| self.identities.get(root).and_then(|i| i.rc) == Some(hoi_crypto::recovery::commitment(&r.author)))
                {
                    let earlier = self.rotations.get(&from).is_none_or(|x| (r.created_at, s.id) < (x.at, x.record));
                    if earlier {
                        self.rotations.insert(from, Rotation { record: s.id, person: *person, from, to: *to, at: r.created_at });
                        self.relink(person);
                    }
                }
            }
            Body::Post { media, .. } => {
                for m in media {
                    self.media_refs.entry(m.hash).or_default().insert(s.id);
                }
            }
            _ => {}
        }
        self.bytes += s.wire.len() as u64;
        self.records.insert(s.id, s);
        true
    }

    pub fn remove(&mut self, id: &Id) -> Option<Stored> {
        let s = self.records.remove(id)?;
        let r = &s.record;
        if let Some(c) = r.community {
            if let Some(set) = self.by_community.get_mut(&c) {
                set.remove(&(r.created_at, *id));
            }
            if let Some(set) = self.by_author.get_mut(&(c, s.author_root)) {
                set.remove(&(r.created_at, *id));
            }
            if let Some(f) = hoi_validate::text_fingerprint(r)
                && let Some(v) = self.texts.get_mut(&(c, f))
            {
                v.retain(|x| x.2 != *id);
                if v.is_empty() {
                    self.texts.remove(&(c, f));
                }
            }
        }
        for w in record_words(r) {
            if let Some(set) = self.words.get_mut(&w) {
                set.remove(id);
                if set.is_empty() {
                    self.words.remove(&w);
                }
            }
        }
        if let Body::Post { media, .. } = &r.body {
            for m in media {
                if let Some(set) = self.media_refs.get_mut(&m.hash) {
                    set.remove(id);
                    if set.is_empty() {
                        self.media_refs.remove(&m.hash);
                    }
                }
            }
        }
        if let Body::Identity { .. } = r.body
            && self.identities.get(&r.author).is_some_and(|i| i.record == *id)
        {
            self.identities.remove(&r.author);
        }
        if let Body::Rotate { person, .. } = r.body
            && let Some(from) = self.rotations.iter().find(|(_, x)| x.record == *id).map(|(from, _)| *from)
        {
            self.rotations.remove(&from);
            self.relink(&person);
        }
        if let Some(p) = r.parent
            && let Some(set) = self.children.get_mut(&p)
        {
            set.remove(&(r.created_at, *id));
        }
        if let Body::Community(_) = r.body
            && self.genesis.get(&r.author) == Some(id)
        {
            self.genesis.remove(&r.author);
        }
        self.bytes = self.bytes.saturating_sub(s.wire.len() as u64);
        Some(s)
    }

    /// Records holding `word` (one of [`words`]), as a word or the start of one ("rel" finds
    /// "relay").
    pub fn word_hits(&self, word: &str) -> BTreeSet<Id> {
        let mut hits = BTreeSet::new();
        for (_, ids) in self.words.range(word.to_string()..).take_while(|(w, _)| w.starts_with(word)) {
            hits.extend(ids.iter().copied());
        }
        hits
    }

    pub fn delegation(&self, device: &Id) -> Option<&Delegation> {
        self.delegations.get(device)
    }

    pub fn delegations_of(&self, root: &Id) -> Vec<(Id, &Delegation)> {
        self.delegations.iter().filter(|(_, d)| d.info.root == *root).map(|(k, d)| (*k, d)).collect()
    }

    /// The person behind a signing key: the key itself unless it is a known device or a root
    /// that took over from an earlier one.
    pub fn root_of(&self, key: &Id) -> Id {
        self.person_of(&self.delegations.get(key).map_or(*key, |d| d.info.root))
    }

    /// The person behind a signing key at time `at`, or `None` for a device whose blessing had
    /// expired or been revoked by then, or a root replaced by then: what a cosignature made at
    /// `at` is worth.
    pub fn root_of_at(&self, key: &Id, at: u64) -> Option<Id> {
        let root = match self.delegations.get(key) {
            None => *key,
            Some(d) if at <= d.info.expires_at && d.info.revoked_at.is_none_or(|rv| at < rv) => d.info.root,
            Some(_) => return None,
        };
        self.replaced_at(&root).is_none_or(|t| at < t).then(|| self.person_of(&root))
    }

    // ------------------------------------------------------------------ rotations

    /// The person a root speaks for: the root itself, or the first root of the identity it
    /// took over.
    pub fn person_of(&self, root: &Id) -> Id {
        self.persons.get(root).copied().unwrap_or(*root)
    }

    /// When `root` was replaced, if it was.
    pub fn replaced_at(&self, root: &Id) -> Option<u64> {
        self.rotations.get(root).map(|r| r.at)
    }

    /// `person`'s roots in order, the first being the person's id and the last the current one.
    pub fn chain(&self, person: &Id) -> Vec<Id> {
        let mut out = vec![*person];
        while let Some(next) = self.rotations.get(out.last().expect("never empty")).map(|r| r.to) {
            if out.contains(&next) || out.len() > 64 {
                break;
            }
            out.push(next);
        }
        out
    }

    /// The identity record with the most work among `person`'s roots: what a community's
    /// `identity_bits` is checked against.
    pub fn best_identity(&self, person: &Id) -> Option<IdentityInfo> {
        self.chain(person).iter().filter_map(|r| self.identities.get(r).copied()).max_by_key(|i| (i.bits, std::cmp::Reverse(i.created_at)))
    }

    /// The root `person` signs with now.
    pub fn current_root(&self, person: &Id) -> Id {
        *self.chain(person).last().expect("never empty")
    }

    /// The rotations along `person`'s chain, in order.
    pub fn rotations_of(&self, person: &Id) -> Vec<Rotation> {
        self.chain(person).iter().filter_map(|root| self.rotations.get(root).copied()).collect()
    }

    /// The records someone who checks a record of `person` needs to see who they are: each later
    /// root's identity and each rotation, in order (the first identity travels on its own).
    pub fn chain_records(&self, person: &Id) -> Vec<Id> {
        let mut out = Vec::new();
        for r in self.rotations_of(person) {
            if let Some(i) = self.identities.get(&r.to) {
                out.push(i.record);
            }
            out.push(r.record);
        }
        out
    }

    fn relink(&mut self, person: &Id) {
        self.persons.retain(|_, p| p != person);
        for root in self.chain(person).into_iter().skip(1) {
            self.persons.insert(root, *person);
        }
    }

    /// Live (unrevoked, unexpired) devices blessed by `root` at `now`.
    pub fn live_devices(&self, root: &Id, now: u64) -> usize {
        self.delegations.values().filter(|d| d.info.root == *root && d.info.expires_at >= now && d.info.revoked_at.is_none_or(|rv| rv > now)).count()
    }

    pub fn identity(&self, root: &Id) -> Option<&IdentityInfo> {
        self.identities.get(root)
    }

    pub fn burned_at(&self, root: &Id) -> Option<u64> {
        self.burned.get(root).copied()
    }

    pub fn burned(&self) -> impl Iterator<Item = &Id> {
        self.burned.keys()
    }

    /// Times of an author's records in a community whose kind counts in `group`.
    pub fn author_times(&self, community: &Id, root: &Id, group: u8) -> Vec<u64> {
        self.by_author
            .get(&(*community, *root))
            .into_iter()
            .flatten()
            .filter(|(_, id)| self.records.get(id).is_some_and(|s| hoi_validate::rate_group(s.record.kind()) == group))
            .map(|(t, _)| *t)
            .collect()
    }

    /// Every record an author signed in a community (any device), oldest first.
    pub fn by_author<'a>(&'a self, community: &Id, root: &Id) -> impl Iterator<Item = &'a Stored> + 'a {
        self.by_author.get(&(*community, *root)).into_iter().flatten().filter_map(|(_, id)| self.records.get(id))
    }

    /// Records in `community` with the same words: (author root, created_at, id).
    pub fn same_text(&self, community: &Id, f: &Fingerprint) -> &[TextUse] {
        self.texts.get(&(*community, *f)).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Records referencing a media hash.
    pub fn media_users(&self, hash: &Id) -> impl Iterator<Item = &Stored> {
        self.media_refs.get(hash).into_iter().flatten().filter_map(|id| self.records.get(id))
    }

    pub fn blobs(&self) -> impl Iterator<Item = (&Id, &Vec<u8>)> {
        self.blobs.iter()
    }

    pub fn blob(&self, hash: &Id) -> Option<&Vec<u8>> {
        self.blobs.get(hash)
    }

    pub fn has_blob(&self, hash: &Id) -> bool {
        self.blobs.contains_key(hash)
    }

    pub fn put_blob(&mut self, hash: Id, bytes: Vec<u8>) {
        if !self.blobs.contains_key(&hash) {
            self.blob_bytes += bytes.len() as u64;
            self.blobs.insert(hash, bytes);
        }
    }

    pub fn remove_blob(&mut self, hash: &Id) -> bool {
        match self.blobs.remove(hash) {
            Some(b) => {
                self.blob_bytes = self.blob_bytes.saturating_sub(b.len() as u64);
                true
            }
            None => false,
        }
    }

    pub fn blob_bytes(&self) -> u64 {
        self.blob_bytes
    }

    /// Blobs no stored record references any more.
    pub fn orphan_blobs(&self) -> Vec<Id> {
        self.blobs.keys().filter(|h| !self.media_refs.contains_key(*h)).copied().collect()
    }

    pub fn genesis_of(&self, community: &Id) -> Option<&Stored> {
        self.genesis.get(community).and_then(|id| self.records.get(id))
    }

    pub fn communities(&self) -> impl Iterator<Item = &Id> {
        self.genesis.keys()
    }

    /// Records of a community, oldest first.
    pub fn in_community<'a>(&'a self, community: &Id) -> impl DoubleEndedIterator<Item = &'a Stored> + 'a {
        self.by_community.get(community).into_iter().flat_map(|set| set.iter()).filter_map(|(_, id)| self.records.get(id))
    }

    pub fn of_kind<'a>(&'a self, community: &Id, kind: Kind) -> impl DoubleEndedIterator<Item = &'a Stored> + 'a {
        self.in_community(community).filter(move |s| s.record.kind() == kind)
    }

    /// Records whose parent is `id`, oldest first.
    pub fn children<'a>(&'a self, id: &Id) -> impl DoubleEndedIterator<Item = &'a Stored> + 'a {
        self.children.get(id).into_iter().flat_map(|set| set.iter()).filter_map(|(_, id)| self.records.get(id))
    }

    pub fn all(&self) -> impl Iterator<Item = &Stored> {
        self.records.values()
    }

    /// Ids past their expiry.
    pub fn expired(&self, now: u64) -> Vec<Id> {
        self.records.values().filter(|s| s.record.expires_at <= now).map(|s| s.id).collect()
    }

    /// Lets a record in again (its removal was overturned).
    pub fn forget_deleted(&mut self, id: &Id) {
        self.deleted.remove(id);
    }

    pub fn forget_deleted_before(&mut self, now: u64) {
        self.deleted.retain(|_, until| *until > now);
    }

    pub fn clear(&mut self) {
        *self = Store::default();
    }
}

// ------------------------------------------------------------------ persistence

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Put {
        id: Id,
        community: Option<Id>,
        expires_at: u64,
        wire: Vec<u8>,
    },
    Delete {
        id: Id,
    },
    /// `None` deletes the blob.
    Blob {
        key: String,
        value: Option<Vec<u8>>,
    },
    Wipe,
}

#[derive(Debug, Default, Clone)]
pub struct Loaded {
    pub records: Vec<Vec<u8>>,
    pub blobs: Vec<(String, Vec<u8>)>,
}

pub trait Persist: Send {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String>;
    fn load(&mut self) -> Result<Loaded, String>;
}

/// Keeps changes for the host to drain (the browser writes them to IndexedDB).
#[derive(Default)]
pub struct Journal {
    pending: Vec<Change>,
    initial: Option<Loaded>,
}

impl Journal {
    pub fn with_initial(l: Loaded) -> Journal {
        Journal { pending: Vec::new(), initial: Some(l) }
    }

    pub fn drain(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.pending)
    }
}

impl Persist for Journal {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        self.pending.extend_from_slice(changes);
        Ok(())
    }

    fn load(&mut self) -> Result<Loaded, String> {
        Ok(self.initial.take().unwrap_or_default())
    }
}

/// Nothing survives a restart. Tests and the simulator.
#[derive(Default)]
pub struct Ephemeral;

impl Persist for Ephemeral {
    fn apply(&mut self, _: &[Change]) -> Result<(), String> {
        Ok(())
    }
    fn load(&mut self) -> Result<Loaded, String> {
        Ok(Loaded::default())
    }
}
