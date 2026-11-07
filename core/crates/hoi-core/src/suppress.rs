//! Operators can stop serving; nobody can forge. The operator of a native node can
//! suppress an identified community, record or media blob: it is deleted here, never fetched,
//! accepted or served again, and the refusal survives restarts. Nothing is signed and nothing is
//! sent: a suppressed item is simply absent from this node, and every other node decides for
//! itself. Hosts reach this through `hoi-node admin` while the node is stopped; it is not part of
//! the JSON dispatch the apps use, so no client and no peer can ask for it.
//!
//! An operator who must keep evidence first takes a copy with [`Node::suppression_material`]:
//! the signed records exactly as they arrived, which anyone can verify later, and the media.

use std::collections::BTreeSet;

use hoi_store::Change;
use hoi_types::{Body, Id, Kind};

use crate::api::{R, parse_id};
use crate::state::Suppression;
use crate::{ApiError, Node};

/// What an operator names: a community by its key, a record by its id, media by its hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Community,
    Record,
    Media,
}

impl Target {
    pub fn as_str(self) -> &'static str {
        match self {
            Target::Community => "community",
            Target::Record => "record",
            Target::Media => "media",
        }
    }

    pub fn parse(s: &str) -> Option<Target> {
        match s {
            "community" => Some(Target::Community),
            "record" => Some(Target::Record),
            "media" => Some(Target::Media),
            _ => None,
        }
    }
}

/// One suppression, as `hoi-node admin suppressions` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppressionEntry {
    pub target: Target,
    pub id: String,
    pub at: u64,
    pub note: Option<String>,
}

/// One signed record as it arrived here, for an operator's evidence copy.
#[derive(Debug, Clone)]
pub struct MaterialRecord {
    pub id: Id,
    pub kind: &'static str,
    pub community: Option<Id>,
    pub author: Id,
    pub created_at: u64,
    pub expires_at: u64,
    pub received_at: u64,
    /// The canonical signed bytes: they verify on their own, wherever they are kept.
    pub wire: Vec<u8>,
}

/// What a suppression would delete here.
#[derive(Debug, Clone, Default)]
pub struct Material {
    pub records: Vec<MaterialRecord>,
    /// Media bytes by BLAKE3 hash.
    pub media: Vec<(Id, Vec<u8>)>,
}

/// What a suppression deleted here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Purged {
    pub records: usize,
    pub media: usize,
}

/// An operator's note is their own short reference, a case number say.
const MAX_NOTE: usize = 200;

/// Records an operator may suppress one by one: what people wrote or showed. A record whose
/// absence would change who may speak or what is hidden (a genesis, an identity, a device or its
/// revocation, the ledger, vouches, reviews, deletions, labels, votes) is not: suppressing it
/// could bring back what a community removed. Suppress its community, or the post it is about.
fn suppressible(kind: Kind) -> bool {
    matches!(kind, Kind::Post | Kind::Comment | Kind::Edit | Kind::Report | Kind::Profile | Kind::ProfileColor)
}

fn note(note: Option<&str>) -> R<Option<String>> {
    match note.map(str::trim).filter(|n| !n.is_empty()) {
        None => Ok(None),
        Some(n) if n.chars().count() <= MAX_NOTE && !n.chars().any(char::is_control) => Ok(Some(n.to_string())),
        Some(_) => Err(ApiError::invalid("A note is one line of at most 200 characters: your own reference, not the report itself.")),
    }
}

impl Node {
    /// Whether the operator suppressed this record or its community.
    pub(crate) fn suppressed(&self, id: &Id, community: Option<Id>) -> bool {
        self.state.suppressed.records.contains_key(id) || community.is_some_and(|c| self.state.suppressed.communities.contains_key(&c))
    }

    pub(crate) fn suppressed_community(&self, c: &Id) -> bool {
        self.state.suppressed.communities.contains_key(c)
    }

    /// At open: suppressed records count as deleted for good, so sync never asks for them.
    pub(crate) fn seed_suppressions(&mut self) {
        let ids: Vec<Id> = self.state.suppressed.records.keys().copied().collect();
        for id in ids {
            self.store.mark_deleted(id, u64::MAX);
        }
    }

    /// A record and the later versions of it: an edit says the same thing again.
    fn record_family(&self, id: &Id) -> Vec<Id> {
        let mut ids = vec![*id];
        ids.extend(self.store.children(id).filter(|c| c.record.kind() == Kind::Edit).map(|c| c.id));
        ids
    }

    /// Deletes `hashes` that no record here refers to any more. Returns how many went.
    fn drop_unused_media(&mut self, hashes: BTreeSet<Id>) -> usize {
        let mut gone = 0;
        for h in hashes {
            if self.store.media_users(&h).next().is_none() && self.store.remove_blob(&h) {
                self.changes.push(Change::Blob { key: crate::node::media_key(&h), value: None });
                gone += 1;
            }
        }
        gone
    }

    fn media_of(records: &[&hoi_store::Stored]) -> BTreeSet<Id> {
        let mut hashes = BTreeSet::new();
        for s in records {
            if let Body::Post { media, .. } = &s.record.body {
                hashes.extend(media.iter().map(|m| m.hash));
            }
        }
        hashes
    }

    /// Stops carrying `id` for good: its records and the media only they used are deleted, it is
    /// refused whoever asks or lists it, and it is no longer announced. Unlike `--refuse`, this
    /// also covers a community the operator joined by invite.
    pub fn suppress_community(&mut self, id: &str, why: Option<&str>) -> R<Purged> {
        let c = parse_id(id)?;
        let note = note(why)?;
        if self.is_private_community(&c) {
            return Err(ApiError::not_allowed("A private group is not held here in the open; leave it instead."));
        }
        let records: Vec<&hoi_store::Stored> = self.store.in_community(&c).collect();
        let count = records.len();
        let media = Self::media_of(&records);
        self.state.suppressed.communities.insert(c, Suppression { at: self.now(), note });
        self.evict_carried(&c);
        self.state.learned.remove(&c);
        self.state.synced.remove(&c);
        self.state.skeletons.retain(|_, s| s.community != c);
        let media = self.drop_unused_media(media);
        self.mark_state();
        self.announce();
        Ok(Purged { records: count, media })
    }

    /// Deletes one record (with its edits) and refuses it from now on. A record not held here
    /// is suppressed all the same, so it is never fetched.
    pub fn suppress_record(&mut self, id: &str, why: Option<&str>) -> R<Purged> {
        let id = parse_id(id)?;
        let note = note(why)?;
        if let Some(s) = self.store.get(&id)
            && !suppressible(s.record.kind())
        {
            return Err(ApiError::not_allowed(
                "That record decides who may speak or what is hidden; suppressing it alone could bring back what a community removed. Suppress its community, or the post it is about.",
            ));
        }
        let family = self.record_family(&id);
        let now = self.now();
        let mut media = BTreeSet::new();
        let mut records = 0;
        for x in family {
            if let Some(s) = self.store.remove(&x) {
                media.extend(Self::media_of(&[&s]));
                self.changes.push(Change::Delete { id: x });
                self.state.saved.remove(&x);
                if let Some(c) = s.record.community {
                    self.dirty.insert(c);
                    self.mark_changed(c);
                }
                records += 1;
            }
            self.store.mark_deleted(x, u64::MAX);
            self.state.suppressed.records.entry(x).or_insert(Suppression { at: now, note: note.clone() });
        }
        let media = self.drop_unused_media(media);
        self.mark_state();
        Ok(Purged { records, media })
    }

    /// Deletes media bytes and refuses them from now on; posts that show them are held, served
    /// to nobody. The hash covers the exact bytes: a profile picture is suppressed by the hash
    /// of its image.
    pub fn suppress_media(&mut self, hash: &str, why: Option<&str>) -> R<Purged> {
        let h = parse_id(hash)?;
        let note = note(why)?;
        let removed = self.store.remove_blob(&h);
        if removed {
            self.changes.push(Change::Blob { key: crate::node::media_key(&h), value: None });
        }
        self.state.media_blocklist.insert(h);
        self.state.suppressed.media.insert(h, Suppression { at: self.now(), note });
        self.media_wanted.remove(&h);
        let touched: BTreeSet<Id> = self.store.media_users(&h).filter_map(|s| s.record.community).collect();
        for c in touched {
            self.mark_changed(c);
        }
        self.mark_state();
        Ok(Purged { records: 0, media: usize::from(removed) })
    }

    /// Lifts a suppression. Nothing comes back by itself: what was deleted stays deleted until
    /// sync, an index or a request brings it again.
    pub fn unsuppress(&mut self, id: &str) -> R<Target> {
        let x = parse_id(id)?;
        let target = if self.state.suppressed.communities.remove(&x).is_some() {
            Target::Community
        } else if self.state.suppressed.records.remove(&x).is_some() {
            self.store.forget_deleted(&x);
            Target::Record
        } else if self.state.suppressed.media.remove(&x).is_some() {
            self.state.media_blocklist.remove(&x);
            Target::Media
        } else {
            return Err(ApiError::not_found("suppression"));
        };
        self.mark_state();
        Ok(target)
    }

    /// Everything suppressed here, communities first.
    pub fn suppressions(&self) -> Vec<SuppressionEntry> {
        let s = &self.state.suppressed;
        let entry = |target, (id, x): (&Id, &Suppression)| SuppressionEntry { target, id: id.hex(), at: x.at, note: x.note.clone() };
        s.communities
            .iter()
            .map(|e| entry(Target::Community, e))
            .chain(s.records.iter().map(|e| entry(Target::Record, e)))
            .chain(s.media.iter().map(|e| entry(Target::Media, e)))
            .collect()
    }

    /// A copy of what suppressing `id` would delete here, for an operator who must hold
    /// evidence: the signed records as they arrived and the media they show. For media, the
    /// records that show it come along as context.
    pub fn suppression_material(&self, target: Target, id: &str) -> R<Material> {
        let x = parse_id(id)?;
        let records: Vec<&hoi_store::Stored> = match target {
            Target::Community => self.store.in_community(&x).collect(),
            Target::Record => self.record_family(&x).iter().filter_map(|i| self.store.get(i)).collect(),
            Target::Media => self.store.media_users(&x).collect(),
        };
        let mut hashes = Self::media_of(&records);
        if target == Target::Media {
            hashes.insert(x);
        }
        Ok(Material {
            records: records
                .iter()
                .map(|s| MaterialRecord {
                    id: s.id,
                    kind: s.record.kind().as_str(),
                    community: s.record.community,
                    author: s.author_root,
                    created_at: s.record.created_at,
                    expires_at: s.record.expires_at,
                    received_at: s.received_at,
                    wire: s.wire.clone(),
                })
                .collect(),
            media: hashes.into_iter().filter_map(|h| self.store.blob(&h).map(|b| (h, b.clone()))).collect(),
        })
    }
}
