//! Turning records and community state into what UIs draw.

use std::collections::{BTreeMap, BTreeSet};

use hoi_moderation::ladder::ProposalStatus;
use hoi_moderation::{Community, Level, label_hides};
use hoi_store::Stored;
use hoi_types::{Body, Id, Kind, Ladder, LedgerAction, Limits, Verdict};

use crate::dto::*;
use crate::node::Node;

pub(crate) fn limits_dto(l: &Limits) -> LimitsDto {
    LimitsDto {
        pow_bits: l.pow_bits,
        max_retention: l.max_retention,
        newcomer_posts_per_day: l.newcomer_posts_per_day,
        newcomer_age: l.newcomer_age,
        admission: l.admission.as_str().into(),
        autoban_threshold: l.autoban_threshold,
        labelers: l.labelers.iter().map(|k| k.hex()).collect(),
        ladder: ladder_dto(&l.ladder),
        scatter_resist: l.scatter.resist,
        scatter_open: l.scatter.open,
        trackers: l.trackers.clone(),
        age_issuers: l.age_issuers.iter().map(|k| hoi_types::hex_encode(k)).collect(),
    }
}

pub(crate) fn ladder_dto(l: &Ladder) -> LadderDto {
    LadderDto {
        identity_bits: l.identity_bits,
        admit_vouches: l.admit_vouches,
        established_age: l.established_age,
        trusted_posts: l.trusted_posts,
        media_score: l.media_score,
        hide_quorum: l.hide_quorum,
        block_quorum: l.block_quorum,
        veto_quorum: l.veto_quorum,
        contest_window: l.contest_window,
    }
}

/// Votes on a target: latest `seq` per voter root wins (PROTOCOL.md §3).
pub(crate) struct Tally {
    pub score: i64,
    pub mine: i64,
    pub up_w: f64,
    pub down_w: f64,
    /// Up-votes from friends and from friends of friends, which count fully and by half.
    pub friends_up: u32,
    pub fof_up: u32,
}

fn excerpt(text: &str, n: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= n {
        flat
    } else {
        let cut: String = flat.chars().take(n).collect();
        format!("{}…", cut.trim_end())
    }
}

impl Node {
    pub(crate) fn person(&self, root: &Id, community: Option<&Community>) -> Person {
        let now = self.now();
        let level = self.trust_level(root);
        let age = self.state.first_seen.get(root).map_or(0, |t| now.saturating_sub(*t) / 86_400);
        Person {
            avatar: self.avatar_for(root, community),
            avatar_color: self.avatar_color_for(root, community),
            key: root.hex(),
            short: root.short_key(),
            name: self.display_name(root),
            name_hint: self.state.names.get(root).map(|(n, _)| n.clone()),
            petname: self.state.petnames.get(root).cloned(),
            trust: level.as_str().into(),
            muted: self.state.muted.contains(root),
            role: community.and_then(|c| {
                if c.is_owner(root) {
                    Some("owner".to_string())
                } else if c.is_mod(root) {
                    Some("mod".to_string())
                } else {
                    None
                }
            }),
            banned: community.map(|c| c.is_banned(root) || c.standing.blocked(root) || self.bounced(root).is_some()),
            key_age_days: age,
            standing: community.map(|c| c.level(root).as_str().to_string()),
            trusted_by: self.fof.get(root).map(|by| by.iter().take(3).map(|f| self.display_name(f)).collect()).unwrap_or_default(),
            handle: self.handle_of(root),
        }
    }

    pub(crate) fn tally(&self, target: &Id) -> Tally {
        let mut latest: BTreeMap<Id, (u64, i64)> = BTreeMap::new();
        for s in self.store.children(target) {
            if let Body::Vote { value, seq } = s.record.body {
                let e = latest.entry(s.author_root).or_insert((0, 0));
                if seq >= e.0 {
                    *e = (seq, value);
                }
            }
        }
        let me = self.me_root();
        let mut t = Tally { score: 0, mine: 0, up_w: 0.0, down_w: 0.0, friends_up: 0, fof_up: 0 };
        for (voter, (_, v)) in latest {
            if self.bounced(&voter).is_some() {
                continue;
            }
            t.score += v;
            if Some(voter) == me {
                t.mine = v;
            }
            let level = self.trust_level(&voter);
            let w = level.weight();
            if v > 0 {
                t.up_w += w;
                match level {
                    hoi_trust::Level::Friend => t.friends_up += 1,
                    hoi_trust::Level::Fof => t.fof_up += 1,
                    _ => {}
                }
            } else if v < 0 {
                t.down_w += w;
            }
        }
        t
    }

    /// Latest edit by the author, if any: (text, title). Held edits (say, a newcomer adding
    /// links) don't count until they may go out.
    pub(crate) fn latest_edit(&self, s: &Stored) -> Option<(String, Option<String>)> {
        self.store
            .children(&s.id)
            .filter(|e| e.author_root == s.author_root && (self.held(e).is_none() || Some(e.author_root) == self.me_root()))
            .filter_map(|e| match &e.record.body {
                Body::Edit { text, title } => Some((e.record.created_at, text.clone(), title.clone())),
                _ => None,
            })
            .max_by_key(|(at, _, _)| *at)
            .map(|(_, t, ti)| (t, ti))
    }

    /// My own latest review of a record.
    pub(crate) fn my_review(&self, id: &Id) -> Option<Verdict> {
        let me = self.me_root()?;
        self.store
            .children(id)
            .filter(|s| s.author_root == me)
            .filter_map(|s| match &s.record.body {
                Body::Review { verdict, seq, .. } => Some((*seq, s.record.created_at, *verdict)),
                _ => None,
            })
            .max_by_key(|(seq, at, _)| (*seq, *at))
            .map(|(_, _, v)| v)
    }

    /// Why a record naps (hidden, peekable), and notes. In order: a removal waiting out its
    /// contest window, the community's hides, an author the community blocked, your own hide,
    /// moderator labels, labels from labelers you follow.
    pub(crate) fn labels_for(&self, id: &Id, c: &Community) -> (Option<Napping>, Vec<String>) {
        let mut nap = None;
        let mut notes = Vec::new();
        if let Some(rm) = c.tombstones.get(id) {
            nap = Some(Napping { label: format!("removal pending: {}", rm.reason), by: self.display_name(&rm.by) });
        }
        if let Some(h) = c.standing.hidden.get(id) {
            nap.get_or_insert(Napping { label: format!("hidden by {} members: {}", h.hiders, h.reason), by: "the community".into() });
        }
        if let Some(s) = self.store.get(id)
            && c.standing.blocked(&s.author_root)
        {
            nap.get_or_insert(Napping { label: "the community blocked its author".into(), by: "the community".into() });
        }
        if self.my_review(id) == Some(Verdict::Hide) {
            nap.get_or_insert(Napping { label: "you hid this".into(), by: "you".into() });
        }
        if let Some(ls) = c.labels.get(id) {
            for l in ls {
                if label_hides(&l.label) {
                    nap.get_or_insert(Napping { label: l.label.clone(), by: "the mods".into() });
                } else if !notes.contains(&l.label) {
                    notes.push(l.label.clone());
                }
            }
        }
        for s in self.store.children(id) {
            if let Body::Label { label, .. } = &s.record.body {
                let followed =
                    self.state.labelers.contains(&s.author_root) || c.limits.labelers.contains(&s.author_root) || Some(s.author_root) == self.me_root();
                if !followed {
                    continue;
                }
                if label_hides(label) {
                    nap.get_or_insert(Napping { label: label.clone(), by: self.display_name(&s.author_root) });
                } else if !notes.contains(label) {
                    notes.push(label.clone());
                }
            }
        }
        (nap, notes)
    }

    pub(crate) fn community_ref(&self, c: &Community) -> CommunityRef {
        CommunityRef { id: c.key.hex(), name: c.genesis.name.clone(), color: c.genesis.color.clone() }
    }

    /// Comments under `id`, counted all the way down.
    pub(crate) fn reply_count(&self, id: &Id) -> u64 {
        let mut n = 0;
        let mut stack = vec![*id];
        while let Some(x) = stack.pop() {
            for s in self.store.children(&x) {
                if s.record.kind() == Kind::Comment {
                    n += 1;
                    stack.push(s.id);
                }
            }
        }
        n
    }

    /// Whether this device shows records by `root` in `c` to me.
    pub(crate) fn visible_author(&self, root: &Id, c: &Community) -> bool {
        if self.bounced(root).is_some() && Some(*root) != self.me_root() {
            return false;
        }
        if self.state.muted.contains(root) {
            return false;
        }
        if c.limits.admission == hoi_types::Admission::Approve && !c.is_admitted(root) {
            // newcomers wait at the door: shown to themselves and to the people who review them
            let me = self.me_root();
            return Some(*root) == me || me.is_some_and(|m| c.level(&m) >= Level::Established);
        }
        true
    }

    /// Whether a record is shown here: visible author, and not held unless it's mine.
    pub(crate) fn shown(&self, s: &Stored, c: &Community) -> bool {
        self.visible_author(&s.author_root, c) && (self.held(s).is_none() || Some(s.author_root) == self.me_root())
    }

    pub(crate) fn media_views(&self, s: &Stored) -> Vec<MediaView> {
        match &s.record.body {
            Body::Post { media, .. } => media
                .iter()
                .map(|m| MediaView {
                    hash: m.hash.hex(),
                    mime: m.mime.clone(),
                    size: m.size,
                    w: m.w,
                    h: m.h,
                    alt: m.alt.clone(),
                    available: self.store.has_blob(&m.hash),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn post_summary(&self, s: &Stored) -> Option<PostSummary> {
        let Body::Post { title, text, link, .. } = &s.record.body else { return None };
        let c = self.communities.get(&s.record.community?)?;
        let edit = self.latest_edit(s);
        let (title, text, edited) = match &edit {
            Some((t, ti)) => (ti.clone().unwrap_or_else(|| title.clone()), t.clone(), true),
            None => (title.clone(), text.clone(), false),
        };
        let tally = self.tally(&s.id);
        let (napping, labels) = self.labels_for(&s.id, c);
        Some(PostSummary {
            id: s.id.hex(),
            short: s.id.short_id(),
            community: self.community_ref(c),
            author: self.person(&s.author_root, Some(c)),
            title,
            excerpt: excerpt(&text, 220),
            link: link.clone(),
            created_at: s.record.created_at,
            expires_at: s.record.expires_at,
            score: tally.score,
            my_vote: tally.mine,
            friends_up: tally.friends_up,
            fof_up: tally.fof_up,
            tallies: self.tallies_for(&c.key, &s.id),
            replies: self.reply_count(&s.id),
            pinned: c.pins.contains(&s.id),
            napping,
            labels,
            edited,
            mine: Some(s.author_root) == self.me_root(),
            saved: self.state.saved.contains(&s.id),
            waiting: c.limits.admission == hoi_types::Admission::Approve && !c.is_admitted(&s.author_root),
            unvalidated: !c.is_admitted(&s.author_root),
            media: self.media_views(s),
            held: if Some(s.author_root) == self.me_root() { self.held(s).map(String::from) } else { None },
        })
    }

    pub(crate) fn removal_of(&self, id: &Id) -> Option<Removal> {
        let sk = self.state.skeletons.get(id)?;
        let c = self.communities.get(&sk.community);
        Some(Removal { by: sk.by.map(|b| self.person(&b, c)), reason: sk.reason.clone(), at: sk.at, ledger_id: sk.ledger.map(|l| l.hex()) })
    }

    pub(crate) fn comment_tree(&self, parent: &Id, c: &Community, depth: usize) -> Vec<CommentView> {
        if depth > 32 {
            return Vec::new();
        }
        let me = self.me_root();
        let mut out = Vec::new();
        for s in self.store.children(parent) {
            let Body::Comment { text, .. } = &s.record.body else { continue };
            if !self.shown(s, c) {
                continue;
            }
            let edit = self.latest_edit(s);
            let tally = self.tally(&s.id);
            let (napping, _) = self.labels_for(&s.id, c);
            out.push(CommentView {
                id: s.id.hex(),
                short: s.id.short_id(),
                parent: parent.hex(),
                author: self.person(&s.author_root, Some(c)),
                text: edit.as_ref().map_or_else(|| text.clone(), |(t, _)| t.clone()),
                created_at: s.record.created_at,
                expires_at: s.record.expires_at,
                score: tally.score,
                my_vote: tally.mine,
                mine: Some(s.author_root) == me,
                edited: edit.is_some(),
                napping,
                removed: None,
                waiting: c.limits.admission == hoi_types::Admission::Approve && !c.is_admitted(&s.author_root),
                unvalidated: !c.is_admitted(&s.author_root),
                held: if Some(s.author_root) == me { self.held(s).map(String::from) } else { None },
                children: self.comment_tree(&s.id, c, depth + 1),
            });
        }
        // removed or deleted comments keep their place as a Poof box
        for (id, sk) in self.state.skeletons.iter().filter(|(_, sk)| sk.parent == Some(*parent) && sk.kind == "comment") {
            out.push(CommentView {
                id: id.hex(),
                short: id.short_id(),
                parent: parent.hex(),
                author: self.person(&sk.author, Some(c)),
                text: String::new(),
                created_at: sk.created_at,
                expires_at: sk.expires_at,
                score: 0,
                my_vote: 0,
                mine: Some(sk.author) == me,
                edited: false,
                napping: None,
                removed: self.removal_of(id),
                waiting: false,
                unvalidated: false,
                held: None,
                children: self.comment_tree(id, c, depth + 1),
            });
        }
        // friends first, then louder, then older
        let rank = |p: &Person| match p.trust.as_str() {
            "me" | "friend" => 0,
            "fof" => 1,
            _ => 2,
        };
        out.sort_by(|a, b| rank(&a.author).cmp(&rank(&b.author)).then(b.score.cmp(&a.score)).then(a.created_at.cmp(&b.created_at)));
        out
    }

    pub(crate) fn checks(&self, s: &Stored, c: &Community) -> Checks {
        let need = hoi_crypto::pow::required_bits(crate::node::limits_for(c, s.record.created_at).pow_bits, s.record.kind().pow_discount(), 0);
        Checks {
            signature: "valid".into(),
            signed_by: if s.record.author == s.author_root { "root".into() } else { "device".into() },
            stamp_bits: s.stamp_bits.max(hoi_crypto::pow::stamp_bits(&s.id, s.record.nonce)),
            required_bits: need,
            received_from: s.from_peers,
            created_at: s.record.created_at,
            fades_at: s.record.expires_at,
            identity_bits: self.store.best_identity(&s.author_root).map_or(0, |i| i.bits),
            standing: c.level(&s.author_root).as_str().into(),
            vouched_by: c.standing.counted_vouches(&s.author_root) as u32,
            address_hidden: self.address_hidden(s),
            home_delivery: self.home_delivery_status(s.id),
        }
    }

    pub(crate) fn my_role(&self, c: &Community) -> &'static str {
        match self.me_root() {
            Some(m) if c.is_owner(&m) => "owner",
            Some(m) if c.is_mod(&m) => "mod",
            _ => "member",
        }
    }

    pub(crate) fn community_summary(&self, c: &Community) -> CommunitySummary {
        let posts = self.store.of_kind(&c.key, Kind::Post).count() as u64;
        let last = self.store.in_community(&c.key).next_back().map_or(c.created_at, |s| s.record.created_at);
        let joined = self.state.joined.get(&c.key);
        CommunitySummary {
            id: c.key.hex(),
            name: c.genesis.name.clone(),
            color: c.genesis.color.clone(),
            description: c.genesis.description.clone(),
            created_at: c.created_at,
            joined: joined.is_some(),
            pinning: joined.is_some_and(|j| j.pinning),
            role: self.my_role(c).into(),
            posts,
            last_activity: last,
        }
    }

    pub(crate) fn ledger_target(&self, action: LedgerAction, target: &Id, c: &Community) -> LedgerTarget {
        match action {
            LedgerAction::Ban | LedgerAction::Unban | LedgerAction::Admit | LedgerAction::ModAdd | LedgerAction::ModRemove | LedgerAction::Retire => {
                LedgerTarget::Key { person: self.person(target, Some(c)) }
            }
            _ => self.record_target(target, c),
        }
    }

    /// The post whose thread holds `id`, following parents through removed records too.
    pub(crate) fn thread_of(&self, id: &Id) -> Option<Id> {
        let mut cur = *id;
        for _ in 0..64 {
            let (kind, parent) = match (self.store.get(&cur), self.state.skeletons.get(&cur)) {
                (Some(s), _) => (s.record.kind().as_str().to_string(), s.record.parent),
                (None, Some(sk)) => (sk.kind.clone(), sk.parent),
                _ => return None,
            };
            match kind.as_str() {
                "post" => return Some(cur),
                "comment" => cur = parent?,
                _ => return None,
            }
        }
        None
    }

    pub(crate) fn record_target(&self, target: &Id, c: &Community) -> LedgerTarget {
        let post = self.thread_of(target).map(|p| p.hex());
        if let Some(s) = self.store.get(target) {
            let title = match &s.record.body {
                Body::Post { title, .. } => Some(title.clone()),
                Body::Comment { text, .. } => Some(excerpt(text, 60)),
                _ => None,
            };
            LedgerTarget::Record {
                id: target.hex(),
                short: target.short_id(),
                title,
                author: Some(self.person(&s.author_root, Some(c))),
                record_kind: Some(s.record.kind().as_str().into()),
                post,
            }
        } else if let Some(sk) = self.state.skeletons.get(target) {
            LedgerTarget::Record {
                id: target.hex(),
                short: target.short_id(),
                title: None,
                author: Some(self.person(&sk.author, Some(c))),
                record_kind: Some(sk.kind.clone()),
                post,
            }
        } else {
            LedgerTarget::Record { id: target.hex(), short: target.short_id(), title: None, author: None, record_kind: None, post: None }
        }
    }

    pub(crate) fn ledger_view(&self, id: &Id, c: &Community) -> Option<LedgerEntryView> {
        let s = self.store.get(id)?;
        let Body::Ledger(l) = &s.record.body else { return None };
        let signers = c.applied.iter().find(|a| a.entry == *id).map(|a| a.signers.clone()).unwrap_or_else(|| vec![s.author_root]);
        let tally = c.standing.vetoes.get(id);
        let pending_until = match l.action {
            LedgerAction::Remove => {
                l.target.and_then(|t| c.tombstones.get(&t)).filter(|rm| rm.entry == *id && rm.effective_at > self.now()).map(|rm| rm.effective_at)
            }
            _ => None,
        };
        Some(LedgerEntryView {
            id: id.hex(),
            seq: l.seq,
            at: s.record.created_at,
            action: l.action.as_str().into(),
            by: signers.iter().map(|k| self.person(k, Some(c))).collect(),
            auto: false,
            target: l.target.map(|t| self.ledger_target(l.action, &t, c)),
            reason: l.reason.clone(),
            text: l.text.clone(),
            limits: l.limits.as_ref().map(limits_dto),
            evidence: l.evidence.as_ref().map_or(0, |e| e.len() as u64),
            expires_at: if l.action.kept_until_superseded() { None } else { Some(s.record.expires_at) },
            vetoes: tally.map_or(0, |t| t.vetoes as u32),
            vetoes_voice: tally.map_or(0.0, |t| voices(t.vetoes_voice)),
            vetoes_needed: voices(tally.map_or(c.standing.quorums.veto, |t| t.need)),
            support: tally.map_or(0, |t| t.support as u32),
            overturned: c.standing.overturned.contains(id),
            vetoable: l.action.vetoable(),
            my_review: self.my_review(id).and_then(|v| match v {
                Verdict::Veto => Some("veto".to_string()),
                Verdict::Ok => Some("ok".to_string()),
                _ => None,
            }),
            pending_until,
            proposal: c.standing.proposals.get(id).map(|t| status_str(&t.status).into()),
        })
    }

    /// A proposed change to the rules or limits, with where the community stands on it.
    pub(crate) fn proposal_view(&self, c: &Community, p: &hoi_moderation::Proposal) -> Option<ProposalView> {
        let t = c.standing.proposals.get(&p.id)?;
        let me = self.me_root();
        let now = self.now();
        let mut answer_of: BTreeMap<Id, (u64, u64, Verdict, Option<String>)> = BTreeMap::new();
        for s in self.store.children(&p.id) {
            if let Body::Review { verdict, seq, reason } = &s.record.body
                && answer_of.get(&s.author_root).is_none_or(|a| (a.0, a.1) < (*seq, s.record.created_at))
            {
                answer_of.insert(s.author_root, (*seq, s.record.created_at, *verdict, reason.clone()));
            }
        }
        let (status, decided_at, unanimous) = match t.status {
            ProposalStatus::Open => ("open", None, false),
            ProposalStatus::InEffect => ("in-effect", None, false),
            ProposalStatus::Passed { at, unanimous } => ("passed", Some(at), unanimous),
            ProposalStatus::Lapsed => ("lapsed", Some(t.deadline), false),
        };
        let open = matches!(t.status, ProposalStatus::Open | ProposalStatus::InEffect);
        let people = |v: &[Id]| v.iter().map(|k| self.person(k, Some(c))).collect::<Vec<_>>();
        Some(ProposalView {
            id: p.id.hex(),
            kind: p.action.as_str().into(),
            at: p.at,
            deadline: t.deadline,
            status: status.into(),
            decided_at,
            unanimous,
            by: people(&p.signers),
            changes: p.changes().into_iter().map(|(f, a, b)| ChangeView { field: f.into(), from: a, to: b }).collect(),
            text: p.text.clone(),
            electors: t.electors.len() as u32,
            agreed: people(&t.agreed),
            objecting: t
                .objecting
                .iter()
                .map(|k| Objection { person: self.person(k, Some(c)), reason: answer_of.get(k).and_then(|a| a.3.clone()).unwrap_or_default() })
                .collect(),
            waiting: people(&t.waiting),
            away: t.away.len() as u32,
            retired: t.retired.len() as u32,
            retirable: if open && me.is_some_and(|m| c.is_owner(&m)) {
                t.away.iter().filter(|k| !c.retired.contains_key(*k) && self.retirable(c, k)).map(|k| self.person(k, Some(c))).collect()
            } else {
                Vec::new()
            },
            agreed_voice: voices(t.agreed_voice),
            objecting_voice: voices(t.objecting_voice),
            waiting_voice: voices(t.waiting_voice),
            my_answer: me.and_then(|m| {
                if p.signers.contains(&m) {
                    return Some("agree".to_string());
                }
                match answer_of.get(&m).map(|a| a.2) {
                    Some(Verdict::Ok) => Some("agree".to_string()),
                    Some(Verdict::Veto) => Some("object".to_string()),
                    _ => None,
                }
            }),
            can_answer: open && now < t.deadline && me.is_some_and(|m| t.electors.contains(&m) && !p.signers.contains(&m)),
        })
    }

    pub(crate) fn community_view(&self, c: &Community) -> CommunityView {
        let sum = self.community_summary(c);
        let me = self.me_root();
        let privileged = me.is_some_and(|m| c.is_owner(&m) || c.is_mod(&m));
        let reviewer = me.is_some_and(|m| c.level(&m) >= Level::Established);
        let mut people = BTreeSet::new();
        let (mut posts, mut comments, mut bytes) = (0, 0, 0);
        let mut pending = BTreeSet::new();
        for s in self.store.in_community(&c.key) {
            bytes += s.wire.len() as u64;
            match s.record.kind() {
                Kind::Post => posts += 1,
                Kind::Comment => comments += 1,
                _ => {}
            }
            if matches!(s.record.kind(), Kind::Post | Kind::Comment) {
                people.insert(s.author_root);
                if !c.is_admitted(&s.author_root) && !c.standing.blocked(&s.author_root) {
                    pending.insert(s.author_root);
                }
            }
        }
        let mut banned: Vec<BanView> =
            c.bans.iter().map(|(k, b)| BanView { person: self.person(k, Some(c)), reason: b.reason.clone(), at: b.at, auto: b.auto }).collect();
        for (k, p) in c.standing.people.iter().filter(|(k, p)| p.blocked && !c.bans.contains_key(*k)) {
            banned.push(BanView {
                person: self.person(k, Some(c)),
                reason: format!("blocked by the community: {} established members hid their posts or objected", p.hiders),
                at: c.created_at.max(self.now()),
                auto: true,
            });
        }
        let now = self.now();
        for (k, b) in self.state.bounced.iter().filter(|(k, b)| b.community == c.key && b.active(now) && !c.bans.contains_key(*k)) {
            banned.push(BanView { person: self.person(k, Some(c)), reason: format!("bounced on this device: {}", b.reason), at: b.at, auto: true });
        }
        banned.sort_by_key(|b| std::cmp::Reverse(b.at));
        CommunityView {
            trackers: self.trackers_of(&c.key),
            private_copy: self.can_copy_private(&c.key),
            pieces: if self.open_scattered_at(c.key, self.now()) { self.pieces_level(c.key) } else { None },
            id: sum.id,
            name: sum.name,
            color: sum.color,
            description: sum.description,
            created_at: sum.created_at,
            joined: sum.joined,
            pinning: sum.pinning,
            role: sum.role,
            posts: sum.posts,
            last_activity: sum.last_activity,
            owners: c.genesis.owners.iter().map(|k| self.person(k, Some(c))).collect(),
            threshold: c.genesis.threshold,
            mods: c.mods.keys().map(|k| self.person(k, Some(c))).collect(),
            limits: limits_dto(&c.limits),
            rules: c.rules.as_ref().map(|r| RulesView {
                // the passed changes merged (crate::wiki)
                text: self.rules_text(&c.key).unwrap_or_else(|| r.text.clone()),
                version: r.version,
                at: r.at,
                signed_by: r.signers.iter().map(|k| self.person(k, Some(c))).collect(),
            }),
            stats: CommunityStats { posts, comments, people: people.len() as u64, stored_bytes: bytes, ledger_entries: c.applied.len() as u64 },
            pending: if privileged || reviewer { pending.iter().map(|k| self.person(k, Some(c))).collect() } else { Vec::new() },
            banned,
            flags: if privileged { self.flags(c) } else { Vec::new() },
            established: c.standing.established as u64,
            established_voice: voices(c.standing.established_voice),
            me: me.map(|m| self.standing_view(c, &m)),
            review_queue: if reviewer { self.review_queue_of(c).len() as u64 } else { 0 },
            proposals: {
                let now = self.now();
                let mut v: Vec<ProposalView> = c
                    .proposals
                    .values()
                    .filter(|p| {
                        c.standing.proposals.get(&p.id).is_some_and(|t| match t.status {
                            ProposalStatus::Passed { at, .. } => at + 30 * 86_400 > now,
                            ProposalStatus::Lapsed => t.deadline + 30 * 86_400 > now,
                            _ => true,
                        })
                    })
                    .filter_map(|p| self.proposal_view(c, p))
                    .collect();
                v.sort_by_key(|p| std::cmp::Reverse(p.at));
                v
            },
        }
    }

    /// Newcomers' posts and comments I can validate: not mine, shown, not judged by me yet.
    /// Returns the posts whose threads hold them, newest first.
    pub(crate) fn review_queue_of(&self, c: &Community) -> Vec<Id> {
        let Some(me) = self.me_root() else { return Vec::new() };
        if c.level(&me) < Level::Established {
            return Vec::new();
        }
        let mut posts: BTreeSet<(u64, Id)> = BTreeSet::new();
        for s in self.store.in_community(&c.key) {
            if !matches!(s.record.kind(), Kind::Post | Kind::Comment)
                || s.author_root == me
                || c.is_admitted(&s.author_root)
                || c.standing.blocked(&s.author_root)
            {
                continue;
            }
            if !self.shown(s, c) || self.my_review(&s.id).is_some() || self.state.muted.contains(&s.author_root) {
                continue;
            }
            if let Some(p) = self.thread_of(&s.id) {
                let at = self.store.get(&p).map_or(s.record.created_at, |x| x.record.created_at);
                posts.insert((at, p));
            }
        }
        posts.into_iter().rev().map(|(_, p)| p).collect()
    }

    pub(crate) fn standing_view(&self, c: &Community, root: &Id) -> StandingView {
        let p = c.standing.person(root).cloned().unwrap_or_default();
        let level = c.level(root);
        let lad = &c.limits.ladder;
        // what admission counts (Node::accept): the best identity along the person's roots
        let identity_bits = self.store.best_identity(root).map_or(0, |i| i.bits);
        let my_vouch = self.me_root().and_then(|me| self.my_vouch(c, &me, root)).unwrap_or(0);
        let need_vouches = c.standing.quorums.admit as u32;
        let mut missing = Vec::new();
        let next = match level {
            _ if c.standing.blocked(root) => None,
            Level::Newcomer => {
                if identity_bits < lad.identity_bits {
                    missing.push(format!("identity work: {identity_bits} of {} bits", lad.identity_bits));
                }
                missing.push(format!("vouches from established members: {} of {need_vouches}", p.vouchers.len()));
                Some(NextStep { level: "member".into(), missing })
            }
            Level::Member => {
                let at = p.admitted_at.unwrap_or(0).saturating_add(lad.established_age);
                let days = at.saturating_sub(self.now()).div_ceil(86_400);
                missing.push(format!("time as a member: {days} more day{}", if days == 1 { "" } else { "s" }));
                Some(NextStep { level: "established".into(), missing })
            }
            Level::Established => {
                if (p.contributions as u32) < lad.trusted_posts {
                    missing.push(format!("posts or comments in the last 90 days: {} of {}", p.contributions, lad.trusted_posts));
                }
                if p.score < lad.media_score as i64 {
                    missing.push(format!("response from the community: {} of {}", p.score, lad.media_score));
                }
                Some(NextStep { level: "trusted".into(), missing })
            }
            Level::Trusted => None,
        };
        StandingView {
            community: c.key.hex(),
            person: self.person(root, Some(c)),
            level: level.as_str().into(),
            capabilities: if c.standing.blocked(root) { Vec::new() } else { level.capabilities().iter().map(|s| s.to_string()).collect() },
            admitted_at: p.admitted_at,
            established_at: p.established_at,
            vouched_by: p.vouchers.iter().map(|k| self.person(k, Some(c))).collect(),
            vouches_needed: need_vouches,
            objections: p.objectors.len() as u32,
            contributions: p.contributions as u32,
            contributions_needed: lad.trusted_posts,
            score: p.score,
            score_needed: lad.media_score as i64,
            blocked: c.standing.blocked(root),
            struck: p.struck,
            my_vouch,
            voice: voices(if c.is_owner(root) || c.is_mod(root) { hoi_moderation::ladder::VOICE } else { p.voice }),
            vouched_this_quarter: p.vouched_this_epoch as u32,
            identity_bits,
            identity_needed: lad.identity_bits,
            next,
        }
    }

    /// `voucher`'s latest vouch for `subject` in `c`.
    pub(crate) fn my_vouch(&self, c: &Community, voucher: &Id, subject: &Id) -> Option<i64> {
        self.store
            .by_author(&c.key, voucher)
            .filter(|s| s.record.parent == Some(*subject))
            .filter_map(|s| match &s.record.body {
                Body::Vouch { value, seq, .. } => Some((*seq, s.record.created_at, *value)),
                _ => None,
            })
            .max_by_key(|(seq, at, _)| (*seq, *at))
            .map(|(_, _, v)| v)
    }

    fn flags(&self, c: &Community) -> Vec<FlagView> {
        struct Acc {
            reasons: BTreeSet<String>,
            by: BTreeMap<Id, f64>,
            last: u64,
        }
        let mut by_target: BTreeMap<Id, Acc> = BTreeMap::new();
        for s in self.store.of_kind(&c.key, Kind::Report) {
            let Body::Report { reason, evidence } = &s.record.body else { continue };
            // Group by the flagged record when we hold it, else by the flagged key.
            let mut targets: Vec<Id> = evidence.iter().filter(|e| self.store.contains(e)).copied().collect();
            if targets.is_empty() {
                targets.extend(s.record.parent);
            }
            for t in targets {
                if c.tombstones.contains_key(&t) || self.state.skeletons.contains_key(&t) {
                    continue;
                }
                let a = by_target.entry(t).or_insert(Acc { reasons: BTreeSet::new(), by: BTreeMap::new(), last: 0 });
                a.reasons.insert(reason.clone());
                a.by.insert(s.author_root, self.trust_level(&s.author_root).weight());
                a.last = a.last.max(s.record.created_at);
            }
        }
        let mut out: Vec<FlagView> = by_target
            .into_iter()
            .map(|(t, a)| {
                let target = if self.store.contains(&t) { self.record_target(&t, c) } else { LedgerTarget::Key { person: self.person(&t, Some(c)) } };
                FlagView {
                    target,
                    reasons: a.reasons.into_iter().collect(),
                    count: a.by.len() as u64,
                    weight: (a.by.values().sum::<f64>() * 100.0).round() / 100.0,
                    last_at: a.last,
                }
            })
            .collect();
        out.sort_by_key(|f| std::cmp::Reverse(f.last_at));
        out
    }
}

/// Thousandths of a voice as voices.
pub(crate) fn voices(milli: u64) -> f64 {
    milli as f64 / hoi_moderation::ladder::VOICE as f64
}

pub(crate) fn status_str(s: &ProposalStatus) -> &'static str {
    match s {
        ProposalStatus::Open => "open",
        ProposalStatus::InEffect => "in-effect",
        ProposalStatus::Passed { .. } => "passed",
        ProposalStatus::Lapsed => "lapsed",
    }
}
