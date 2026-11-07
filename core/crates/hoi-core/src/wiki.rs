//! Rules and wiki text as Loro documents (M3).
//!
//! **Rules** still change only by consent (PROTOCOL §6.1). A rules entry carries, besides the
//! text its proposers saw, its change as a Loro update to the rules document as it stood when
//! they wrote it. The rules shown are the passed entries' changes merged, in the order they
//! passed: two proposals written against the same version both count once both pass, instead of
//! the later one overwriting the earlier.
//!
//! **The wiki**: pages of a community, each a Loro document built from `wiki` records, each an
//! update to it. Members edit (admitted, not banned or blocked; owners and moderators always).
//! Edits merge whatever order they arrive in; an edit whose predecessors haven't arrived waits
//! inside the document. An edit can't be taken out without breaking the ones after it, so a bad
//! edit is undone by editing; a banned person's later edits are refused. A page holds at most
//! [`PAGE_BYTES`] of edits, a community at most [`PAGES`] pages.

use std::collections::{BTreeMap, BTreeSet};

use hoi_store::Stored;
use hoi_types::{Body, Id, Kind, LedgerAction, Record};
use hoi_validate::Reject;
use loro::{ExportMode, LoroDoc, UpdateOptions};

use crate::api::{R, parse_id};
use crate::dto::{WikiPageSummary, WikiPageView};
use crate::error::ApiError;
use crate::node::{Node, Source};

const RULES: &str = "rules";
const BODY: &str = "body";
/// A page's edits together at most.
pub const PAGE_BYTES: usize = 1024 * 1024;
/// Pages one community's wiki holds at most.
pub const PAGES: usize = 256;
/// A page's text at most, as an editor saves it.
pub const PAGE_TEXT: usize = 64 * 1024;

/// A fresh Loro peer for this edit: random, never 0 or 1.
fn fresh_peer() -> R<u64> {
    let b = hoi_crypto::random_bytes::<8>().map_err(|_| ApiError::new("internal", "no randomness"))?;
    Ok(u64::from_le_bytes(b).max(2))
}

fn loro_error(e: impl std::fmt::Display) -> ApiError {
    ApiError::new("internal", format!("The text could not be merged: {e}"))
}

impl Node {
    /// The community's rules document: the passed rules entries' changes, in the order they passed.
    pub(crate) fn rules_doc(&self, c: &Id) -> Option<LoroDoc> {
        let st = self.communities.get(c)?;
        let mut passed: Vec<_> = st.proposals.values().filter(|p| p.action == LedgerAction::Rules && p.applied_at.is_some() && p.patch.is_some()).collect();
        passed.sort_by_key(|p| (p.applied_at, p.id));
        let doc = LoroDoc::new();
        for p in passed {
            let _ = doc.import(p.patch.as_deref().unwrap_or_default());
        }
        Some(doc)
    }

    /// The rules as the passed changes made them, when any passed.
    pub(crate) fn rules_text(&self, c: &Id) -> Option<String> {
        let st = self.communities.get(c)?;
        st.rules.as_ref()?;
        Some(self.rules_doc(c)?.get_text(RULES).to_string())
    }

    /// The change from the rules as they are here to `text`, as a Loro update.
    pub(crate) fn rules_patch(&self, c: &Id, text: &str) -> R<Vec<u8>> {
        let doc = self.rules_doc(c).unwrap_or_default();
        let before = doc.oplog_vv();
        doc.set_peer_id(fresh_peer()?).map_err(loro_error)?;
        doc.get_text(RULES).update(text, UpdateOptions::default()).map_err(loro_error)?;
        doc.commit();
        let patch = doc.export(ExportMode::updates(&before)).map_err(loro_error)?;
        if patch.len() > hoi_types::MAX_RULES_PATCH {
            return Err(ApiError::invalid("That change to the rules is too large for one entry. Change less at once."));
        }
        Ok(patch)
    }

    /// The wiki edits of a page, oldest first.
    fn wiki_edits<'a>(&'a self, c: &Id, page: &str) -> Vec<&'a Stored> {
        let mut v: Vec<&Stored> = self
            .store
            .of_kind(c, Kind::Wiki)
            .filter(|s| matches!(&s.record.body, Body::Wiki { page: p, .. } if p == page))
            .filter(|s| self.usable(s))
            .collect();
        v.sort_by_key(|s| (s.record.created_at, s.id));
        v
    }

    fn wiki_doc(&self, c: &Id, page: &str) -> (LoroDoc, Vec<&Stored>) {
        let edits = self.wiki_edits(c, page);
        let doc = LoroDoc::new();
        for s in &edits {
            if let Body::Wiki { update, .. } = &s.record.body {
                let _ = doc.import(update);
            }
        }
        (doc, edits)
    }

    fn may_edit_wiki(&self, c: &Id, root: &Id) -> bool {
        self.communities
            .get(c)
            .is_some_and(|st| !st.is_banned(root) && !st.standing.blocked(root) && (st.is_owner(root) || st.is_mod(root) || st.is_admitted(root)))
    }

    /// The pages of a community's wiki, most recently edited first.
    pub fn wiki_pages(&self, community: &str) -> R<Vec<WikiPageSummary>> {
        let c = parse_id(community)?;
        self.community_of(&c)?;
        let mut pages: BTreeMap<String, (u64, BTreeSet<Id>)> = BTreeMap::new();
        for s in self.store.of_kind(&c, Kind::Wiki).filter(|s| self.usable(s)) {
            if let Body::Wiki { page, .. } = &s.record.body {
                let e = pages.entry(page.clone()).or_default();
                e.0 = e.0.max(s.record.created_at);
                e.1.insert(s.author_root);
            }
        }
        let mut out: Vec<WikiPageSummary> = pages
            .into_iter()
            .map(|(page, (at, editors))| {
                let chars = self.wiki_doc(&c, &page).0.get_text(BODY).len_unicode() as u64;
                WikiPageSummary { page, chars, updated_at: at, editors: editors.len() as u32 }
            })
            .collect();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.page.cmp(&b.page)));
        Ok(out)
    }

    /// One page of the wiki, as its edits made it.
    pub fn wiki_page(&self, community: &str, page: &str) -> R<WikiPageView> {
        let c = parse_id(community)?;
        let st = self.community_of(&c)?;
        if !hoi_types::is_wiki_page(page) {
            return Err(ApiError::invalid("A page's name is 1 to 64 of a-z, 0-9 and -."));
        }
        let (doc, edits) = self.wiki_doc(&c, page);
        let mut editors: Vec<Id> = Vec::new();
        for s in edits.iter().rev() {
            if !editors.contains(&s.author_root) {
                editors.push(s.author_root);
            }
        }
        Ok(WikiPageView {
            page: page.to_string(),
            text: doc.get_text(BODY).to_string(),
            updated_at: edits.last().map_or(0, |s| s.record.created_at),
            edits: edits.len() as u32,
            editors: editors.iter().take(8).map(|k| self.person(k, Some(st))).collect(),
            can_edit: self.me_root().is_some_and(|me| self.may_edit_wiki(&c, &me)),
        })
    }

    /// Saves a page as `text`: the change from the page as it is here, as a signed edit.
    pub fn edit_wiki(&mut self, community: &str, page: &str, text: &str) -> R<WikiPageView> {
        let me = self.need_me()?;
        let c = parse_id(community)?;
        if !hoi_types::is_wiki_page(page) {
            return Err(ApiError::invalid("A page's name is 1 to 64 of a-z, 0-9 and -."));
        }
        if !self.may_edit_wiki(&c, &me.root) {
            return Err(ApiError::not_allowed("Members edit the wiki: once you're admitted here, you can too."));
        }
        if text.len() > PAGE_TEXT {
            return Err(ApiError::invalid("A page holds 64 KB of text at most. Split it into two pages."));
        }
        let (doc, edits) = self.wiki_doc(&c, page);
        if edits.is_empty() && self.wiki_pages(community)?.len() >= PAGES {
            return Err(ApiError::not_allowed("This wiki has as many pages as it can hold."));
        }
        if doc.get_text(BODY).to_string() == text {
            return self.wiki_page(community, page);
        }
        let before = doc.oplog_vv();
        doc.set_peer_id(fresh_peer()?).map_err(loro_error)?;
        doc.get_text(BODY).update(text, UpdateOptions::default()).map_err(loro_error)?;
        doc.commit();
        let update = doc.export(ExportMode::updates(&before)).map_err(loro_error)?;
        if update.len() > hoi_types::MAX_WIKI_UPDATE {
            return Err(ApiError::invalid("That edit is too large to send at once. Save in smaller steps."));
        }
        let now = self.now();
        let r = Record {
            v: 1,
            community: Some(c),
            author: self.device_key()?,
            parent: None,
            created_at: now,
            expires_at: now + hoi_validate::KEEP,
            body: Body::Wiki { page: page.to_string(), update },
            nonce: 0,
        };
        self.publish(r)?;
        self.wiki_page(community, page)
    }

    /// At ingest: who may edit, whether the update reads, and whether the page has room.
    pub(crate) fn check_wiki(&self, r: &Record, root: Id, src: &Source) -> Result<(), Reject> {
        let (Some(c), Body::Wiki { page, update }) = (r.community, &r.body) else { return Ok(()) };
        if !self.communities.contains_key(&c) {
            return Err(Reject::UnknownCommunity);
        }
        if !matches!(src, Source::Load) && !self.may_edit_wiki(&c, &root) {
            return Err(Reject::NotAllowed("members edit the wiki".into()));
        }
        if LoroDoc::decode_import_blob_meta(update, true).is_err() {
            return Err(Reject::Content("a wiki edit that isn't one".into()));
        }
        if matches!(src, Source::Load) {
            return Ok(());
        }
        let held: usize = self.wiki_edits(&c, page).iter().map(|s| s.wire.len()).sum();
        if held + update.len() > PAGE_BYTES {
            return Err(Reject::NotAllowed("this wiki page is full".into()));
        }
        Ok(())
    }
}
