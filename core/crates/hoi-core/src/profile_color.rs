//! Signed palette selections, independent from photo permissions and photo revisions.
use hoi_moderation::Community;
use hoi_types::{Body, Id, Kind, Record};

use crate::{ApiError, Node, dto::Me};

impl Node {
    pub fn set_avatar_color(&mut self, color: Option<&str>) -> Result<Me, ApiError> {
        let color = color.unwrap_or("");
        if !hoi_types::is_avatar_color(color) {
            return Err(ApiError::invalid("Choose a supported identicon color."));
        }
        let root = self.need_me()?.root;
        let seen = self.profile_color_heads.values().filter_map(|heads| heads.get(&root).map(|head| head.revision.0)).max().unwrap_or(0);
        let clock_revision = self.now().saturating_mul(1000);
        let me = self.state.me.as_mut().ok_or_else(ApiError::no_identity)?;
        me.avatar_color_revision = clock_revision.max(seen.saturating_add(1)).max(me.avatar_color_revision.saturating_add(1));
        me.avatar_color = Some(color.to_string());
        self.mark_state();
        self.changed_everywhere();
        self.me().ok_or_else(ApiError::no_identity)
    }

    fn own_avatar_color_choice(&self, root: &Id) -> Option<(&str, u64)> {
        let me = self.state.me.as_ref().filter(|me| me.root == *root)?;
        let local = me.avatar_color.as_deref().map(|color| (color, me.avatar_color_revision));
        let public = self.latest_profile_record(root, None, Kind::ProfileColor).and_then(|s| match &s.record.body {
            Body::ProfileColor { color, seq } => Some((color.as_str(), *seq)),
            _ => None,
        });
        match (local, public) {
            (Some(a), Some(b)) => Some(if b.1 >= a.1 { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    pub(crate) fn remember_public_avatar_color(&mut self, root: Id) {
        if Some(root) != self.me_root() {
            return;
        }
        let choice = self.latest_profile_record(&root, None, Kind::ProfileColor).and_then(|s| match &s.record.body {
            Body::ProfileColor { color, seq } => Some((color.clone(), *seq)),
            _ => None,
        });
        if let Some((color, revision)) = choice
            && let Some(me) = self.state.me.as_mut()
            && revision >= me.avatar_color_revision
        {
            me.avatar_color = Some(color);
            me.avatar_color_revision = revision;
        }
    }

    pub(crate) fn avatar_color_for(&self, root: &Id, community: Option<&Community>) -> Option<String> {
        if community.is_some_and(|c| !self.visible_author(root, c) || c.standing.blocked(root) || c.is_banned(root)) {
            return None;
        }
        let latest = self.latest_profile_record(root, community, Kind::ProfileColor);
        let remote_revision =
            community.and_then(|c| self.profile_color_heads.get(&c.key)).and_then(|heads| heads.get(root)).map(|head| head.revision.0).or_else(|| {
                latest.and_then(|s| match s.record.body {
                    Body::ProfileColor { seq, .. } => Some(seq),
                    _ => None,
                })
            });
        if let Some((color, revision)) = self.own_avatar_color_choice(root)
            && remote_revision.is_none_or(|remote| revision > remote)
        {
            return (!color.is_empty()).then(|| color.to_string());
        }
        let s = latest?;
        let c = self.communities.get(&s.record.community?)?;
        if !self.shown(s, c) || self.held(s).is_some() || c.standing.blocked(root) || c.is_banned(root) {
            return None;
        }
        match &s.record.body {
            Body::ProfileColor { color, .. } if !color.is_empty() => Some(color.clone()),
            _ => None,
        }
    }

    /// Small signed text metadata uses ordinary identity, moderation, stamp and rate checks.
    pub(crate) fn publish_profile_color(&mut self, community: Id) -> Result<(), ApiError> {
        let me = self.need_me()?;
        let Some((color, revision)) = self.own_avatar_color_choice(&me.root).map(|(color, revision)| (color.to_string(), revision)) else { return Ok(()) };
        let st = self.communities.get(&community).ok_or_else(|| ApiError::not_found("community"))?;
        let previous = self.latest_profile_record(&me.root, Some(st), Kind::ProfileColor);
        if self.profile_color_heads.get(&community).and_then(|heads| heads.get(&me.root)).is_some_and(|head| head.revision.0 > revision) {
            return Ok(());
        }
        if previous.is_some_and(|s| matches!(&s.record.body, Body::ProfileColor { color: old, seq } if *seq >= revision || *old == color)) {
            return Ok(());
        }
        let seq = previous.map_or(0, |s| match s.record.body {
            Body::ProfileColor { seq, .. } => seq,
            _ => 0,
        });
        let now = self.now();
        self.publish(Record {
            v: 1,
            community: Some(community),
            author: me.device,
            parent: None,
            created_at: now,
            expires_at: now + st.limits.max_retention,
            body: Body::ProfileColor { color, seq: revision.max(seq.saturating_add(1)) },
            nonce: 0,
        })?;
        Ok(())
    }
}
