//! Looking at pictures and videos: their perceptual fingerprints (hoi_validate::similar), checked
//! against what this device refuses.
//!
//! Native hosts decode here, behind `catch_unwind`. A browser's core runs as wasm with
//! `panic = "abort"`, where a decoder bug would take the whole node down, and it can't decode
//! video at all; so a host may **look elsewhere** ([`Node::set_look_elsewhere`]): media that
//! needs a look waits in memory, the host takes it with [`Node::looks_wanted`], decodes it in a
//! worker of its own (pictures with hoi-wasm, video frames with the platform's player), and hands
//! back what it saw with [`Node::looked`]. A worker that dies is a picture nobody can read
//! (`Look::Unreadable`), never a dead node. Pictures someone is about to attach are looked at the
//! same way before the draft ([`Node::looked`] with a hash nobody asked for keeps the look for
//! the composer, a few at a time).
//!
//! Video gets a look only where a host can play it: a handful of frames spread over its length,
//! each fingerprinted like a picture. A video matches when at least half its frames (and at least
//! two) match frames of a video this device refuses. Hosts that can't play a video (a headless
//! node, a browser without the codec) say `Nothing`: the exact hash list still applies.

use std::collections::BTreeMap;

use hoi_types::{Body, Id};
use hoi_validate::similar::{self, Variants};
use serde::{Deserialize, Serialize};

use crate::node::Node;

/// Media waiting for a look at most, and bytes held for it.
const MAX_PENDING: usize = 64;
const MAX_PENDING_BYTES: usize = 64 << 20;
/// Looks kept for the composer at most.
const MAX_KNOWN: usize = 16;
/// Frames of a video looked at at most.
pub const MAX_FRAMES: usize = 16;

/// What a look found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Look {
    Picture(Box<Variants>),
    /// The sampled frames of a video, in order.
    Video(Vec<Variants>),
    /// Nothing to fingerprint: not a picture, too flat, or a video this host can't play.
    Nothing,
    /// A picture that passed the checks and still can't be decoded: refused like bad media.
    Unreadable,
}

/// A look as hosts pass it: `{"picture": {hashes, quality}}`, `{"video": [{hashes, quality}]}`,
/// `"nothing"`, `"unreadable"`. Hashes are 64 hex digits, eight per picture or frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LookDto {
    Picture(VariantsDto),
    Video(Vec<VariantsDto>),
    Nothing,
    Unreadable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantsDto {
    pub hashes: Vec<String>,
    pub quality: u8,
}

impl From<&Variants> for VariantsDto {
    fn from(v: &Variants) -> Self {
        VariantsDto { hashes: v.hashes.iter().map(similar::to_hex).collect(), quality: v.quality }
    }
}

impl VariantsDto {
    fn parse(&self) -> Option<Variants> {
        let mut hashes = [[0u8; 32]; 8];
        if self.hashes.len() != 8 {
            return None;
        }
        for (h, s) in hashes.iter_mut().zip(&self.hashes) {
            *h = similar::from_hex(s)?;
        }
        Some(Variants { hashes, quality: self.quality })
    }
}

impl From<&Look> for LookDto {
    fn from(l: &Look) -> Self {
        match l {
            Look::Picture(v) => LookDto::Picture(v.as_ref().into()),
            Look::Video(f) => LookDto::Video(f.iter().map(Into::into).collect()),
            Look::Nothing => LookDto::Nothing,
            Look::Unreadable => LookDto::Unreadable,
        }
    }
}

impl LookDto {
    /// A look a host sent; anything malformed reads as unreadable.
    pub fn parse(&self) -> Look {
        match self {
            LookDto::Picture(v) => v.parse().map_or(Look::Unreadable, |v| Look::Picture(Box::new(v))),
            LookDto::Video(f) => {
                let frames: Option<Vec<Variants>> = f.iter().take(MAX_FRAMES).map(VariantsDto::parse).collect();
                frames.map_or(Look::Unreadable, |f| if f.is_empty() { Look::Nothing } else { Look::Video(f) })
            }
            LookDto::Nothing => Look::Nothing,
            LookDto::Unreadable => Look::Unreadable,
        }
    }
}

/// Looks a picture over here: what native hosts do. Video gets nothing (no decoder here).
pub fn look_here(mime: &str, bytes: &[u8]) -> Look {
    if !hoi_validate::media::is_image(mime) {
        return Look::Nothing;
    }
    match similar::examine_variants(mime, bytes) {
        Ok(Some(v)) => Look::Picture(Box::new(v)),
        Ok(None) => Look::Nothing,
        Err(_) => Look::Unreadable,
    }
}

/// What a look-elsewhere host's worker does with a file someone attaches: sanitizes it as the
/// core will, and looks at the clean bytes. `(hash of the clean bytes, type, look)`; the host
/// hands the look to [`Node::looked`] under that hash before the draft.
pub fn sanitize_and_look(raw: &[u8]) -> Result<(Id, String, Look), String> {
    let (clean, info) = hoi_validate::media::sanitize(raw).map_err(|e| e.to_string())?;
    Ok((Id::hash(&clean), info.mime.to_string(), look_here(info.mime, &clean)))
}

/// Whether a video matches frames on a list: at least half its frames, and at least two.
pub(crate) fn video_blocked(list: &crate::state::PdqList, frames: &[Variants]) -> bool {
    let hits = frames.iter().filter(|f| list.blocks(f)).count();
    hits >= 2 && hits * 2 >= frames.len()
}

/// Whether media of this type needs a look at all.
pub fn wants_look(mime: &str) -> bool {
    hoi_validate::media::is_image(mime) || mime.starts_with("video/")
}

/// Why a look was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Why {
    /// Bytes that arrived for a post: kept, refused or struck once looked at.
    Arrived,
    /// Media removed as illegal: remember what it looks like.
    Remember,
    /// Stored media to check against a list just loaded.
    Recheck,
}

#[derive(Debug, Clone)]
pub(crate) struct Pending {
    pub mime: String,
    /// The bytes, unless they are in the store ([`Why::Recheck`]).
    pub data: Option<Vec<u8>>,
    pub why: Why,
}

/// Media waiting for a host to look at it, and looks kept for the composer.
#[derive(Debug, Default)]
pub(crate) struct Looks {
    /// Not yet handed out.
    pub waiting: BTreeMap<Id, Pending>,
    /// Handed out, waiting for the answer.
    pub out: BTreeMap<Id, Pending>,
    /// Looks the composer supplied, by hash.
    pub known: BTreeMap<Id, Look>,
}

impl Looks {
    fn bytes(&self) -> usize {
        self.waiting.values().chain(self.out.values()).filter_map(|p| p.data.as_ref()).map(Vec::len).sum()
    }
}

impl Node {
    /// Whether this host decodes media in a worker of its own and hands the looks back (see the
    /// module docs). Off by default: native hosts look here.
    pub fn set_look_elsewhere(&mut self, on: bool) {
        self.look_elsewhere = on;
    }

    /// Asks for a look at media: here at once on native hosts, else queued for the host.
    /// Returns the look when it was taken here.
    pub(crate) fn look_at(&mut self, id: Id, mime: &str, data: Option<Vec<u8>>, why: Why) -> Option<Look> {
        if !self.look_elsewhere {
            let bytes = match &data {
                Some(d) => d.as_slice(),
                None => self.store.blob(&id)?,
            };
            return Some(look_here(mime, bytes));
        }
        if self.looks.waiting.contains_key(&id) || self.looks.out.contains_key(&id) {
            return None;
        }
        let size = data.as_ref().map_or(0, Vec::len);
        if self.looks.waiting.len() + self.looks.out.len() >= MAX_PENDING || self.looks.bytes() + size > MAX_PENDING_BYTES {
            // too much in flight: arrivals are fetched again later, rechecks wait for the next load
            return None;
        }
        self.looks.waiting.insert(id, Pending { mime: mime.to_string(), data, why });
        None
    }

    /// Media the host should look at now: (hash, type, bytes). Each is handed out once; answer
    /// with [`Node::looked`].
    pub fn looks_wanted(&mut self) -> Vec<(Id, String, Vec<u8>)> {
        let mut out = Vec::new();
        for (id, p) in std::mem::take(&mut self.looks.waiting) {
            let bytes = match &p.data {
                Some(d) => d.clone(),
                None => match self.store.blob(&id) {
                    Some(b) => b.to_vec(),
                    None => continue,
                },
            };
            out.push((id, p.mime.clone(), bytes));
            self.looks.out.insert(id, p);
        }
        out
    }

    /// What the host saw. For media nobody asked about, the look is kept for the composer.
    pub fn looked(&mut self, id: Id, look: Look) {
        let Some(p) = self.looks.out.remove(&id) else {
            if self.looks.known.len() >= MAX_KNOWN
                && let Some(first) = self.looks.known.keys().next().copied()
            {
                self.looks.known.remove(&first);
            }
            self.looks.known.insert(id, look);
            return;
        };
        match p.why {
            Why::Arrived => {
                if let Some(data) = p.data {
                    self.take_blob(id, data, look);
                }
            }
            Why::Remember => self.remember_look(&look),
            Why::Recheck => {
                if self.blocked_look(&look) {
                    self.refuse_stored_media(id);
                }
            }
        }
    }

    /// Whether this device refuses what the look shows.
    pub(crate) fn blocked_look(&self, look: &Look) -> bool {
        match look {
            Look::Picture(v) => self.similar_blocked(v),
            Look::Video(frames) => video_blocked(&self.state.video_blocklist, frames),
            _ => false,
        }
    }

    /// Remembers what refused media looks like, so altered copies are refused too.
    pub(crate) fn remember_look(&mut self, look: &Look) {
        let changed = match look {
            Look::Picture(v) => self.state.similar_blocklist.insert(v.hashes[0]),
            Look::Video(frames) => frames.iter().fold(false, |c, f| self.state.video_blocklist.insert(f.hashes[0]) | c),
            _ => false,
        };
        if changed {
            self.mark_state();
        }
    }

    /// Stored media that turned out to match a list: deleted and refused from now on.
    pub(crate) fn refuse_stored_media(&mut self, h: Id) {
        self.state.media_blocklist.insert(h);
        self.drop_stored_media(h);
    }

    /// Stored media a followed keeper's list refuses: deleted, and refused while the
    /// list says so; not added to this person's own list.
    pub(crate) fn drop_stored_media(&mut self, h: Id) {
        if self.store.remove_blob(&h) {
            self.push_change(hoi_store::Change::Blob { key: format!("m:{}", h.hex()), value: None });
        }
        self.mark_state();
        self.changed_everywhere();
    }

    /// The type a stored blob was posted as.
    pub(crate) fn mime_of_blob(&self, h: &Id) -> Option<String> {
        self.store.media_users(h).find_map(|s| match &s.record.body {
            Body::Post { media, .. } => media.iter().find(|m| m.hash == *h).map(|m| m.mime.clone()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(n: u8) -> Variants {
        // eight hashes far apart from each other's frames: frame n is n repeated
        let mut hashes = [[0u8; 32]; 8];
        for (k, h) in hashes.iter_mut().enumerate() {
            *h = [n.wrapping_mul(37).wrapping_add((k as u8).wrapping_mul(101)); 32];
        }
        Variants { hashes, quality: 90 }
    }

    #[test]
    fn a_video_matches_when_half_its_frames_do() {
        let mut list = crate::state::PdqList::default();
        for n in [1, 2, 3, 4] {
            list.insert(frame(n).hashes[0]);
        }
        let video = |ns: &[u8]| ns.iter().map(|n| frame(*n)).collect::<Vec<_>>();
        assert!(video_blocked(&list, &video(&[1, 2, 3, 4])));
        assert!(video_blocked(&list, &video(&[1, 2, 50, 51])), "half the frames");
        assert!(!video_blocked(&list, &video(&[1, 50, 51, 52])), "a quarter");
        assert!(!video_blocked(&list, &video(&[1])), "one frame is not enough");
        // a mirrored copy: its frames' variants match the listed frames as shown
        let mirrored: Vec<Variants> = video(&[1, 2, 3, 4])
            .into_iter()
            .map(|mut v| {
                v.hashes.swap(0, 1);
                v
            })
            .collect();
        assert!(video_blocked(&list, &mirrored));
    }

    #[test]
    fn looks_travel_as_hosts_send_them() {
        let l = Look::Video(vec![frame(1), frame(2)]);
        let dto = LookDto::from(&l);
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.starts_with("{\"video\":["));
        assert_eq!(serde_json::from_str::<LookDto>(&json).unwrap().parse(), l);
        assert_eq!(serde_json::from_str::<LookDto>("\"nothing\"").unwrap().parse(), Look::Nothing);
        // malformed hashes read as unreadable, never as nothing to see
        let bad = r#"{"picture":{"hashes":["zz"],"quality":1}}"#;
        assert_eq!(serde_json::from_str::<LookDto>(bad).unwrap().parse(), Look::Unreadable);
    }
}
