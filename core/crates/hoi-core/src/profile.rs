//! Small signed profile sidecars. Existing post/comment encodings stay unchanged; older
//! peers can continue handling contributions while declining the new profile kind.
use hoi_moderation::{Community, Level};
use hoi_store::Stored;
use hoi_types::{Body, Id, Kind, Record, uri::b64};
use hoi_validate::media;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{ApiError, Node, dto::Me};

/// Retain only ordering metadata until all superseded pictures could have expired. This
/// prevents a short-lived removal from revealing a still-live older picture after restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileHead {
    pub(crate) revision: (u64, u64, Id),
    until: u64,
}
pub(crate) type ProfileHeads = BTreeMap<Id, BTreeMap<Id, ProfileHead>>;

impl Node {
    pub fn set_avatar(&mut self, image: &[u8]) -> Result<Me, ApiError> {
        self.need_me()?;
        if image.len() > hoi_types::MAX_AVATAR_BYTES {
            return Err(ApiError::invalid("Profile pictures must fit in 4 KB."));
        }
        let clean =
            if image.is_empty() { Vec::new() } else { media::sanitize(image).map_err(|e| ApiError::invalid(format!("Invalid profile picture: {e}")))?.0 };
        hoi_validate::avatar(&clean)?;
        if self.avatar_blocked(&clean) {
            return Err(ApiError::not_allowed("That picture matches one blocked on this device."));
        }
        let root = self.me_root().ok_or_else(ApiError::no_identity)?;
        let seen = self.profile_heads.values().filter_map(|heads| heads.get(&root).map(|head| head.revision.0)).max().unwrap_or(0);
        let clock_revision = self.now().saturating_mul(1000);
        let me = self.state.me.as_mut().ok_or_else(ApiError::no_identity)?;
        me.avatar_revision = clock_revision.max(seen.saturating_add(1)).max(me.avatar_revision.saturating_add(1));
        me.avatar = Some(clean);
        self.mark_state();
        self.changed_everywhere();
        self.me().ok_or_else(ApiError::no_identity)
    }

    pub(crate) fn avatar_blocked(&self, image: &[u8]) -> bool {
        if image.is_empty() {
            return false;
        }
        self.media_refused(&Id::hash(image))
            || ((!self.state.similar_blocklist.is_empty() || !self.lists_pdq.is_empty())
                && match hoi_validate::similar::examine_variants(media::JPEG, image) {
                    Ok(Some(fp)) => self.similar_blocked(&fp),
                    Ok(None) => false,
                    Err(_) => true,
                })
    }

    pub(crate) fn purge_blocked_profiles(&mut self) {
        let clear_revision = self.state.me.as_ref().map(|me| {
            self.profile_heads
                .values()
                .filter_map(|heads| heads.get(&me.root).map(|head| head.revision.0))
                .max()
                .unwrap_or(0)
                .max(me.avatar_revision)
                .saturating_add(1)
                .max(self.now().saturating_mul(1000))
        });
        let blocked: Vec<Id> = self
            .store
            .all()
            .filter_map(|s| match &s.record.body {
                Body::Profile { image, .. } if self.avatar_blocked(image) => Some(s.id),
                _ => None,
            })
            .collect();
        for id in blocked {
            self.drop_record(&id);
        }
        let mine_blocked = self.state.me.as_ref().and_then(|me| me.avatar.as_deref()).is_some_and(|image| self.avatar_blocked(image));
        if mine_blocked {
            if let Some(me) = self.state.me.as_mut() {
                me.avatar = Some(Vec::new());
                me.avatar_revision = clear_revision.unwrap_or(me.avatar_revision);
            }
            self.mark_state();
        }
    }

    pub(crate) fn latest_profile_record(&self, root: &Id, community: Option<&Community>, kind: Kind) -> Option<&Stored> {
        let heads = if kind == Kind::ProfileColor { &self.profile_color_heads } else { &self.profile_heads };
        self.store
            .all()
            .filter(|s| {
                s.author_root == *root
                    && s.record.kind() == kind
                    && s.record.expires_at > self.now()
                    && s.record
                        .community
                        .and_then(|c| heads.get(&c))
                        .and_then(|heads| heads.get(root))
                        .is_none_or(|head| matches!(s.record.body, Body::Profile { seq, .. } | Body::ProfileColor { seq, .. } if (seq, s.record.created_at, s.id) == head.revision))
                    && s.record.community.is_some_and(|c| match community {
                        Some(scope) => c == scope.key,
                        None => !self.is_private_community(&c),
                    })
            })
            .max_by_key(|s| match &s.record.body {
                Body::Profile { seq, .. } | Body::ProfileColor { seq, .. } => (*seq, s.record.created_at, s.id),
                _ => unreachable!(),
            })
    }

    fn latest_profile(&self, root: &Id, community: Option<&Community>) -> Option<&Stored> {
        self.latest_profile_record(root, community, Kind::Profile)
    }

    pub(crate) fn remember_profile_head(&mut self, root: Id, record: &Record) {
        let Some(community) = record.community else { return };
        let (seq, heads) = match &record.body {
            Body::Profile { seq, .. } => (seq, &mut self.profile_heads),
            Body::ProfileColor { seq, .. } => (seq, &mut self.profile_color_heads),
            _ => return,
        };
        let revision = (*seq, record.created_at, record.id());
        let head = heads.entry(community).or_default().entry(root).or_insert(ProfileHead { revision, until: record.expires_at });
        head.revision = head.revision.max(revision);
        head.until = head.until.max(record.expires_at);
        self.profile_heads_dirty = true;
    }

    pub(crate) fn profile_is_current(&self, stored: &Stored) -> bool {
        let (seq, heads) = match stored.record.body {
            Body::Profile { seq, .. } => (seq, &self.profile_heads),
            Body::ProfileColor { seq, .. } => (seq, &self.profile_color_heads),
            _ => return true,
        };
        stored
            .record
            .community
            .and_then(|community| heads.get(&community))
            .and_then(|heads| heads.get(&stored.author_root))
            .is_none_or(|head| (seq, stored.record.created_at, stored.id) == head.revision)
    }

    pub(crate) fn sweep_profile_heads(&mut self) {
        let now = self.now();
        for map in [&mut self.profile_heads, &mut self.profile_color_heads] {
            let before: usize = map.values().map(BTreeMap::len).sum();
            map.retain(|_, heads| {
                heads.retain(|_, head| head.until > now);
                !heads.is_empty()
            });
            self.profile_heads_dirty |= before != map.values().map(BTreeMap::len).sum::<usize>();
        }
    }

    /// A newer public edit from another of your devices supersedes a stale local
    /// selection. Private-only profiles never become a default for another community.
    fn own_avatar_choice(&self, root: &Id) -> Option<(&[u8], u64)> {
        let me = self.state.me.as_ref().filter(|me| me.root == *root)?;
        let local = me.avatar.as_deref().map(|image| (image, me.avatar_revision));
        let public = self.latest_profile(root, None).and_then(|s| match &s.record.body {
            Body::Profile { image, seq } => Some((image.as_slice(), *seq)),
            _ => None,
        });
        match (local, public) {
            (Some(a), Some(b)) => Some(if b.1 >= a.1 { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    pub(crate) fn remember_public_avatar(&mut self, root: Id) {
        if Some(root) != self.me_root() {
            return;
        }
        let choice = self.latest_profile(&root, None).and_then(|s| match &s.record.body {
            Body::Profile { image, seq } => Some((image.clone(), *seq)),
            _ => None,
        });
        if let Some((image, revision)) = choice
            && let Some(me) = self.state.me.as_mut()
            && revision >= me.avatar_revision
        {
            me.avatar = Some(image);
            me.avatar_revision = revision;
        }
    }

    pub(crate) fn avatar_for(&self, root: &Id, community: Option<&Community>) -> Option<String> {
        if !self.state.settings.auto_media && Some(*root) != self.me_root() {
            return None;
        }
        if community.is_some_and(|c| {
            !self.visible_author(root, c)
                || c.standing.blocked(root)
                || c.is_banned(root)
                || (!c.is_owner(root) && !c.is_mod(root) && c.level(root) < Level::Trusted)
        }) {
            return None;
        }
        let latest = self.latest_profile(root, community);
        let remote_revision =
            community.and_then(|c| self.profile_heads.get(&c.key)).and_then(|heads| heads.get(root)).map(|head| head.revision.0).or_else(|| {
                latest.map(|s| match s.record.body {
                    Body::Profile { seq, .. } => seq,
                    _ => 0,
                })
            });
        if let Some((image, revision)) = self.own_avatar_choice(root)
            && remote_revision.is_none_or(|remote| revision > remote)
        {
            return (!image.is_empty() && !self.avatar_blocked(image)).then(|| b64(image));
        }
        let s = latest?;
        let c = self.communities.get(&s.record.community?)?;
        if !self.shown(s, c) || self.held(s).is_some() || c.standing.blocked(root) || c.is_banned(root) {
            return None;
        }
        match &s.record.body {
            Body::Profile { image, .. } if !image.is_empty() && !self.avatar_blocked(image) => Some(b64(image)),
            _ => None,
        }
    }

    /// Called after a successful local contribution. Profiles use the community's ordinary
    /// retention, encryption, identity, media standing, stamp and rate checks.
    pub(crate) fn publish_profile(&mut self, community: Id) -> Result<(), ApiError> {
        let me = self.need_me()?;
        let Some((image, revision)) = self.own_avatar_choice(&me.root).map(|(image, revision)| (image.to_vec(), revision)) else { return Ok(()) };
        let st = self.communities.get(&community).ok_or_else(|| ApiError::not_found("community"))?;
        if !image.is_empty() && !st.is_owner(&me.root) && !st.is_mod(&me.root) && st.level(&me.root) < Level::Trusted {
            return Ok(());
        }
        let previous = self.latest_profile(&me.root, Some(st));
        if self.profile_heads.get(&community).and_then(|heads| heads.get(&me.root)).is_some_and(|head| head.revision.0 > revision) {
            return Ok(());
        }
        if previous.is_some_and(|s| matches!(s.record.body, Body::Profile { seq, .. } if seq >= revision)) {
            return Ok(());
        }
        if previous.is_some_and(|s| matches!(&s.record.body, Body::Profile { image: old, .. } if *old == image)) {
            return Ok(());
        }
        let seq = previous.map_or(0, |s| match s.record.body {
            Body::Profile { seq, .. } => seq,
            _ => 0,
        });
        let now = self.now();
        let record = Record {
            v: 1,
            community: Some(community),
            author: me.device,
            parent: None,
            created_at: now,
            expires_at: now + st.limits.max_retention,
            body: Body::Profile { image, seq: revision.max(seq.saturating_add(1)) },
            nonce: 0,
        };
        self.publish(record)?;
        Ok(())
    }
}
