//! hoi-moderation: a community's state is a pure function of its genesis, the ledger entries
//! and the community's own judgements (vouches and reviews) a node holds.
//!
//! Entries are replayed in `(at, seq, id)` order, `at` being each entry's effective time, and
//! each one is checked against the state before it: owners (with cosignatures reaching the
//! threshold) propose rules and limits and change moderators; owners and moderators remove,
//! ban, label, pin and admit. The trust ladder ([`ladder`]) then decides who stands where, what
//! the community hid, which moderator or owner actions established members overturned, and
//! which proposals the community accepted; overturned entries, suspended moderators and
//! accepted proposals feed back into the replay until nothing changes. Replaying from scratch
//! makes the result independent of arrival order.

pub mod ladder;

use std::collections::{BTreeMap, BTreeSet};

use hoi_types::{Genesis, Id, LedgerAction, LedgerBody, Limits};

pub use ladder::{Level, Standing};

/// Labels that make a record "napping" (hidden, peekable). Anything else is shown as a note.
pub const HIDING_LABELS: [&str; 6] = ["spam", "likely spam", "hide", "nsfw", "illegal", "off-topic"];

pub fn label_hides(label: &str) -> bool {
    HIDING_LABELS.contains(&label.to_ascii_lowercase().as_str())
}

/// A ledger record as the node hands it over: signature, stamp and delegation already checked.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: Id,
    pub created_at: u64,
    /// Effective time: `created_at`, or when this device first saw the entry if it arrived late
    /// (PROTOCOL.md §15.5).
    pub at: u64,
    pub expires_at: u64,
    /// The key that signed the record (root or device).
    pub signer: Id,
    /// The root behind `signer`.
    pub signer_root: Id,
    pub body: LedgerBody,
    /// What cosigners sign: `Record::proposal_id()`.
    pub proposal: Id,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Mod,
    Member,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    pub text: String,
    pub version: u32,
    pub at: u64,
    pub signers: Vec<Id>,
    pub entry: Id,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ban {
    pub reason: String,
    pub at: u64,
    pub by: Id,
    pub entry: Id,
    pub auto: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub by: Id,
    pub reason: String,
    pub at: u64,
    pub entry: Id,
    /// When the body is deleted. Until then the record naps and the removal can be vetoed
    /// (PROTOCOL.md §16.4). Removals for illegal content or private data take effect at once.
    pub effective_at: u64,
}

/// Removals that can't wait for the contest window.
pub fn removal_is_urgent(reason: &str) -> bool {
    let r = reason.trim().to_ascii_lowercase();
    r.starts_with("illegal") || r.starts_with("privacy")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModLabel {
    pub label: String,
    pub by: Id,
    pub at: u64,
    pub entry: Id,
}

/// An entry that took effect, with every root that signed it (signer first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub entry: Id,
    pub signers: Vec<Id>,
}

/// A proposed change to the rules or the limits. It takes effect only when the community
/// agrees (PROTOCOL.md §6.1); a limits proposal changes only the fields it differs in from the
/// limits in effect when it was made, so proposals about different things don't undo each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub id: Id,
    pub action: LedgerAction,
    pub at: u64,
    pub signers: Vec<Id>,
    pub base: Limits,
    pub limits: Option<Limits>,
    pub text: Option<String>,
    /// For rules: the change as a Loro update; nodes merge the passed ones.
    pub patch: Option<Vec<u8>>,
    pub deadline: u64,
    /// When it took effect, if it did.
    pub applied_at: Option<u64>,
}

impl Proposal {
    /// The fields a limits proposal changes: (field, from, to).
    pub fn changes(&self) -> Vec<(&'static str, String, String)> {
        let Some(l) = &self.limits else { return Vec::new() };
        let mut out = Vec::new();
        diff_limits(&self.base, l, &mut |f, a, b| out.push((f, a, b)));
        out
    }
}

/// Calls `f(field, from, to)` for every field `want` changes relative to `base`.
fn diff_limits(base: &Limits, want: &Limits, f: &mut dyn FnMut(&'static str, String, String)) {
    macro_rules! cmp {
        ($($name:literal => $a:expr, $b:expr;)*) => {$(
            if $a != $b {
                f($name, $a.to_string(), $b.to_string());
            }
        )*};
    }
    let (a, b) = (&base.ladder, &want.ladder);
    cmp! {
        "pow_bits" => base.pow_bits, want.pow_bits;
        "max_retention" => base.max_retention, want.max_retention;
        "newcomer_posts_per_day" => base.newcomer_posts_per_day, want.newcomer_posts_per_day;
        "newcomer_age" => base.newcomer_age, want.newcomer_age;
        "admission" => base.admission.as_str(), want.admission.as_str();
        "autoban_threshold" => base.autoban_threshold, want.autoban_threshold;
        "labelers" => base.labelers.len(), want.labelers.len();
        "identity_bits" => a.identity_bits, b.identity_bits;
        "admit_vouches" => a.admit_vouches, b.admit_vouches;
        "established_age" => a.established_age, b.established_age;
        "trusted_posts" => a.trusted_posts, b.trusted_posts;
        "media_score" => a.media_score, b.media_score;
        "hide_quorum" => a.hide_quorum, b.hide_quorum;
        "block_quorum" => a.block_quorum, b.block_quorum;
        "veto_quorum" => a.veto_quorum, b.veto_quorum;
        "contest_window" => a.contest_window, b.contest_window;
        "scatter_resist" => base.scatter.resist, want.scatter.resist;
        "scatter_open" => base.scatter.open, want.scatter.open;
    }
    if base.labelers != want.labelers && base.labelers.len() == want.labelers.len() {
        f("labelers", base.labelers.len().to_string(), want.labelers.len().to_string());
    }
    if base.trackers != want.trackers {
        let list = |t: &[String]| if t.is_empty() { "none".to_string() } else { t.join(", ") };
        f("trackers", list(&base.trackers), list(&want.trackers));
    }
    if base.age_issuers != want.age_issuers {
        f("age_issuers", base.age_issuers.len().to_string(), want.age_issuers.len().to_string());
    }
}

/// `cur` with the fields `want` changes relative to `base` set to `want`'s values.
pub fn patch_limits(base: &Limits, want: &Limits, cur: &Limits) -> Limits {
    let mut l = cur.clone();
    macro_rules! take {
        ($($($f:ident).+;)*) => {$(
            if base.$($f).+ != want.$($f).+ {
                l.$($f).+ = want.$($f).+.clone();
            }
        )*};
    }
    take! {
        pow_bits; max_retention; newcomer_posts_per_day; newcomer_age; admission; autoban_threshold; labelers;
        ladder.identity_bits; ladder.admit_vouches; ladder.established_age; ladder.trusted_posts; ladder.media_score;
        ladder.hide_quorum; ladder.block_quorum; ladder.veto_quorum; ladder.contest_window;
        scatter.resist; scatter.open; trackers; age_issuers;
    }
    l
}

#[derive(Debug, Clone)]
pub struct Community {
    pub key: Id,
    pub genesis_id: Id,
    pub genesis: Genesis,
    pub created_at: u64,
    pub limits: Limits,
    /// Every set of limits a change put in effect after the genesis, with when, oldest first.
    pub limits_since: Vec<(u64, Limits)>,
    pub rules: Option<Rules>,
    pub mods: BTreeMap<Id, u64>,
    pub bans: BTreeMap<Id, Ban>,
    pub admitted: BTreeSet<Id>,
    pub tombstones: BTreeMap<Id, Removal>,
    pub pins: BTreeSet<Id>,
    pub labels: BTreeMap<Id, Vec<ModLabel>>,
    pub applied: Vec<Applied>,
    /// Entries that didn't take effect, with why. They may later if an earlier entry shows up.
    pub refused: Vec<(Id, &'static str)>,
    pub max_seq: u64,
    /// `admit` entries that took effect.
    pub admits: Vec<ladder::AdmitIn>,
    /// Proposed rules and limits, whatever became of them.
    pub proposals: BTreeMap<Id, Proposal>,
    /// The trust ladder, settled together with the ledger.
    pub standing: Standing,
    /// Drop keys owners set (`drop-key`, SCATTER.md S5), oldest first: the last is listed.
    pub drop_keys: Vec<DropKeyIn>,
    /// Electors the owners said left for good (`retire`): person → (entry time, entry).
    /// Whether it counts is the ladder's call (it lapses when they sign anything again).
    pub retired: BTreeMap<Id, (u64, Id)>,
}

/// A drop key a `drop-key` entry set: when, by which entry and owner, and the key with its
/// secret sealed to each device it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropKeyIn {
    pub at: u64,
    pub entry: Id,
    pub by: Id,
    pub grant: hoi_types::DropKeyGrant,
}

impl Community {
    pub fn new(key: Id, genesis_id: Id, genesis: Genesis, created_at: u64) -> Community {
        Community {
            key,
            genesis_id,
            limits: genesis.limits.clone(),
            limits_since: Vec::new(),
            genesis,
            created_at,
            rules: None,
            mods: BTreeMap::new(),
            bans: BTreeMap::new(),
            admitted: BTreeSet::new(),
            tombstones: BTreeMap::new(),
            pins: BTreeSet::new(),
            labels: BTreeMap::new(),
            applied: Vec::new(),
            refused: Vec::new(),
            max_seq: 0,
            admits: Vec::new(),
            proposals: BTreeMap::new(),
            standing: Standing::default(),
            drop_keys: Vec::new(),
            retired: BTreeMap::new(),
        }
    }

    /// The drop key the ledger lists now, if any.
    pub fn drop_key(&self) -> Option<&DropKeyIn> {
        self.drop_keys.last()
    }

    pub fn is_owner(&self, root: &Id) -> bool {
        self.genesis.owners.contains(root)
    }

    /// The limits in effect at `t`: the genesis limits, as changed by every proposal that had
    /// taken effect by then (PROTOCOL.md §6.1).
    pub fn limits_at(&self, t: u64) -> &Limits {
        self.limits_since.iter().rev().find(|(at, _)| *at <= t).map_or(&self.genesis.limits, |(_, l)| l)
    }

    pub fn is_mod(&self, root: &Id) -> bool {
        self.mods.contains_key(root)
    }

    pub fn role(&self, root: &Id) -> Role {
        if self.is_owner(root) {
            Role::Owner
        } else if self.is_mod(root) {
            Role::Mod
        } else {
            Role::Member
        }
    }

    pub fn is_banned(&self, root: &Id) -> bool {
        self.bans.contains_key(root)
    }

    /// Whether the community validated this person: admitted by the ladder, or an owner or
    /// moderator (PROTOCOL.md §15).
    pub fn is_admitted(&self, root: &Id) -> bool {
        self.is_owner(root) || self.is_mod(root) || self.standing.admitted(root)
    }

    /// Where this person stands on the ladder.
    pub fn level(&self, root: &Id) -> Level {
        if self.is_owner(root) || self.is_mod(root) { Level::Trusted } else { self.standing.level(root) }
    }

    /// Replays `entries` (any order) from the genesis state, without the ladder. `root_of` maps
    /// a cosigner key to its root as of the entry's creation time (identity for root keys, a
    /// delegation that was unexpired and unrevoked then for device keys): a stolen device whose
    /// blessing was revoked cannot cosign owner decisions afterwards (PROTOCOL.md §2).
    pub fn rebuild(&mut self, entries: &[Entry], root_of: &dyn Fn(&Id, u64) -> Option<Id>) {
        let standing = std::mem::take(&mut self.standing);
        self.replay(entries, root_of, &ladder::Penalties::default());
        self.standing = standing;
    }

    fn replay(&mut self, entries: &[Entry], root_of: &dyn Fn(&Id, u64) -> Option<Id>, pen: &ladder::Penalties) {
        let mut fresh = Community::new(self.key, self.genesis_id, self.genesis.clone(), self.created_at);
        // entries at their effective time; accepted proposals take effect when they were decided
        enum Step<'a> {
            Entry(&'a Entry),
            Decided(Id),
        }
        // (a decision sorts right after its proposal's entry when both fall on the same moment)
        let seq_of: BTreeMap<Id, u64> = entries.iter().map(|e| (e.id, e.body.seq)).collect();
        let mut steps: Vec<(u64, u64, Id, u8, Step)> = entries.iter().map(|e| (e.at, e.body.seq, e.id, 0, Step::Entry(e))).collect();
        steps.extend(pen.passed.iter().map(|(id, t)| (*t, seq_of.get(id).copied().unwrap_or(0), *id, 1, Step::Decided(*id))));
        steps.sort_by_key(|a| (a.0, a.1, a.2, a.3));
        for (t, _, _, _, step) in steps {
            let e = match step {
                Step::Entry(e) => e,
                Step::Decided(id) => {
                    fresh.enact(&id, t);
                    continue;
                }
            };
            fresh.max_seq = fresh.max_seq.max(e.body.seq);
            if pen.overturned.contains(&e.id) {
                fresh.refused.push((e.id, "overturned by the community"));
                continue;
            }
            if pen.suspended.get(&e.signer_root).is_some_and(|since| e.at >= *since) && !fresh.is_owner(&e.signer_root) {
                fresh.refused.push((e.id, "the moderator was suspended by the community"));
                continue;
            }
            match fresh.apply(e, root_of) {
                Ok(signers) => fresh.applied.push(Applied { entry: e.id, signers }),
                Err(why) => fresh.refused.push((e.id, why)),
            }
        }
        for (m, since) in &pen.suspended {
            if !fresh.is_owner(m) && fresh.mods.get(m).is_some_and(|added| added <= since) {
                fresh.mods.remove(m);
            }
        }
        *self = fresh;
    }

    /// An accepted proposal takes effect at `t`.
    fn enact(&mut self, id: &Id, t: u64) {
        let Some(p) = self.proposals.get_mut(id) else { return };
        if p.applied_at.is_some() || t < p.at {
            return;
        }
        p.applied_at = Some(t);
        match p.action {
            LedgerAction::Limits => {
                if let Some(want) = &p.limits {
                    self.limits = patch_limits(&p.base, want, &self.limits);
                    self.limits_since.push((t, self.limits.clone()));
                }
            }
            LedgerAction::Rules => {
                let version = self.rules.as_ref().map_or(1, |r| r.version + 1);
                self.rules = Some(Rules { text: p.text.clone().unwrap_or_default(), version, at: t, signers: p.signers.clone(), entry: p.id });
            }
            _ => {}
        }
    }

    /// What the ladder needs from the ledger state.
    fn ledger_view(&self, entries: &[Entry], root_of: &dyn Fn(&Id, u64) -> Option<Id>) -> ladder::LedgerView {
        ladder::LedgerView {
            owners: self.genesis.owners.iter().copied().collect(),
            mods: self.mods.clone(),
            genesis_at: self.created_at,
            banned: self.bans.keys().copied().collect(),
            admits: self.admits.clone(),
            removed: self.tombstones.iter().map(|(t, r)| (*t, r.at)).collect(),
            proposals: self
                .proposals
                .values()
                .map(|p| ladder::ProposalIn { id: p.id, at: p.at, signers: p.signers.clone(), ladder: p.base.ladder.clone(), deadline: p.deadline })
                .collect(),
            entries: entries
                .iter()
                .map(|e| {
                    let mut signers = vec![e.signer_root];
                    for c in e.body.cosign.iter().flatten() {
                        if let Some(r) = root_of(&c.key, e.created_at)
                            && !signers.contains(&r)
                        {
                            signers.push(r);
                        }
                    }
                    ladder::EntryIn { id: e.id, created_at: e.at, signers, target: e.body.target, vetoable: e.body.action.vetoable() }
                })
                .collect(),
            ladder: self.limits.ladder.clone(),
            retention: self.limits.max_retention,
            retired: self.retired.iter().map(|(p, (at, _))| (*p, *at)).collect(),
            admits_only: self.limits.admission == hoi_types::Admission::AgeProof,
        }
    }

    /// Replays the ledger and settles the trust ladder with it (PROTOCOL.md §15, §16).
    pub fn settle(&mut self, entries: &[Entry], root_of: &dyn Fn(&Id, u64) -> Option<Id>, inputs: &ladder::Inputs) {
        let (standing, _, pen) = ladder::settle(
            inputs,
            &mut |pen| {
                self.replay(entries, root_of, pen);
                self.ledger_view(entries, root_of)
            },
            6,
        );
        // the last replay may have run with the previous round's penalties: replay once more
        // with the final ones so the ledger state matches the standing kept
        self.replay(entries, root_of, &pen);
        self.standing = standing;
    }

    /// Distinct owner roots that signed `e`: the signer plus valid cosignatures.
    pub fn owner_signers(&self, e: &Entry, root_of: &dyn Fn(&Id, u64) -> Option<Id>) -> Vec<Id> {
        let mut out = Vec::new();
        if self.is_owner(&e.signer_root) {
            out.push(e.signer_root);
        }
        for c in e.body.cosign.iter().flatten() {
            if !hoi_crypto::verify(&c.key, e.proposal.as_bytes(), &c.sig) {
                continue;
            }
            if let Some(root) = root_of(&c.key, e.created_at)
                && self.is_owner(&root)
                && !out.contains(&root)
            {
                out.push(root);
            }
        }
        out
    }

    fn apply(&mut self, e: &Entry, root_of: &dyn Fn(&Id, u64) -> Option<Id>) -> Result<Vec<Id>, &'static str> {
        let b = &e.body;
        let by = e.signer_root;
        if b.action.needs_threshold() {
            let signers = self.owner_signers(e, root_of);
            if !self.is_owner(&by) {
                return Err("only owners sign this");
            }
            if (signers.len() as u32) < self.genesis.threshold {
                return Err("not enough owner signatures");
            }
            match b.action {
                // rules and limits are proposals: the community decides (PROTOCOL.md §6.1)
                LedgerAction::Rules | LedgerAction::Limits => {
                    if b.action == LedgerAction::Limits && b.limits.is_none() {
                        return Err("no limits");
                    }
                    self.proposals.insert(
                        e.id,
                        Proposal {
                            id: e.id,
                            action: b.action,
                            at: e.at,
                            signers: signers.clone(),
                            base: self.limits.clone(),
                            limits: b.limits.clone(),
                            text: b.text.clone(),
                            patch: b.patch.clone(),
                            deadline: e.at.saturating_add(self.limits.max_retention),
                            applied_at: None,
                        },
                    );
                }
                LedgerAction::ModAdd => {
                    self.mods.insert(b.target.ok_or("no target")?, e.at);
                }
                LedgerAction::ModRemove => {
                    self.mods.remove(&b.target.ok_or("no target")?);
                }
                LedgerAction::Retire => {
                    let target = b.target.ok_or("no target")?;
                    if self.is_owner(&target) {
                        return Err("owners can't be retired");
                    }
                    self.retired.insert(target, (e.at, e.id));
                }
                _ => unreachable!("threshold actions"),
            }
            return Ok(signers);
        }

        if b.action == LedgerAction::DropKey {
            // any one owner: the key only drops what a removal explains (SCATTER.md §5.11)
            if !self.is_owner(&by) {
                return Err("only owners set the drop key");
            }
            let grant = b.drop_key.clone().ok_or("no drop key")?;
            self.drop_keys.push(DropKeyIn { at: e.at, entry: e.id, by, grant });
            return Ok(vec![by]);
        }
        if !(self.is_owner(&by) || self.is_mod(&by)) {
            return Err("only owners and moderators sign this");
        }
        let target = b.target.ok_or("no target")?;
        let reason = b.reason.clone().unwrap_or_default();
        match b.action {
            LedgerAction::Remove => {
                let effective_at = if removal_is_urgent(&reason) { e.at } else { e.at.saturating_add(self.limits.ladder.contest_window) };
                self.tombstones.insert(target, Removal { by, reason, at: e.at, entry: e.id, effective_at });
                self.pins.remove(&target);
            }
            LedgerAction::Ban => {
                if self.is_owner(&target) {
                    return Err("owners can't be banned");
                }
                self.bans.insert(target, Ban { reason, at: e.at, by, entry: e.id, auto: false });
                self.mods.remove(&target);
            }
            LedgerAction::Unban => {
                self.bans.remove(&target);
            }
            LedgerAction::Label => {
                let l = self.labels.entry(target).or_default();
                l.retain(|x| x.label != reason);
                l.push(ModLabel { label: reason, by, at: e.at, entry: e.id });
            }
            LedgerAction::Pin => {
                self.pins.insert(target);
            }
            LedgerAction::Unpin => {
                self.pins.remove(&target);
            }
            LedgerAction::Admit => {
                self.admitted.insert(target);
                self.admits.push(ladder::AdmitIn { person: target, at: e.at, by, entry: e.id });
            }
            _ => unreachable!("handled above"),
        }
        Ok(vec![by])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_crypto::Keypair;
    use hoi_types::{Body, Cosig, Record};

    fn genesis(owners: Vec<Id>, threshold: u32) -> Genesis {
        Genesis { name: "privacy".into(), description: "".into(), color: "mint".into(), owners, threshold, rules_text_cid: None, limits: Limits::default() }
    }

    fn entry(signer: &Keypair, at: u64, seq: u64, action: LedgerAction, target: Option<Id>, cosigners: &[&Keypair]) -> Entry {
        let mut body = LedgerBody {
            seq,
            action,
            target,
            reason: Some("r".into()),
            evidence: None,
            cosign: None,
            text: Some("rules".into()),
            limits: None,
            drop_key: None,
            patch: None,
        };
        let rec = Record {
            v: 1,
            community: Some(Id([1; 32])),
            author: signer.public(),
            parent: None,
            created_at: at,
            expires_at: at + 100,
            body: Body::Ledger(body.clone()),
            nonce: 0,
        };
        let proposal = rec.proposal_id();
        if !cosigners.is_empty() {
            body.cosign = Some(cosigners.iter().map(|k| Cosig { key: k.public(), sig: k.sign(proposal.as_bytes()) }).collect());
        }
        Entry {
            id: Id::hash(&[seq as u8, at as u8]),
            created_at: at,
            at,
            expires_at: at + 100,
            signer: signer.public(),
            signer_root: signer.public(),
            body,
            proposal,
        }
    }

    #[test]
    fn threshold_and_mods() {
        let a = Keypair::generate().unwrap();
        let b = Keypair::generate().unwrap();
        let m = Keypair::generate().unwrap();
        let mut c = Community::new(Id([1; 32]), Id([2; 32]), genesis(vec![a.public(), b.public()], 2), 0);
        let ident = |k: &Id, _: u64| Some(*k);

        // one owner alone can't add a mod with threshold 2
        c.rebuild(&[entry(&a, 10, 1, LedgerAction::ModAdd, Some(m.public()), &[])], &ident);
        assert!(!c.is_mod(&m.public()));
        // with b's cosignature it works, and the mod can then remove a post
        let post = Id([9; 32]);
        let es = vec![entry(&m, 20, 3, LedgerAction::Remove, Some(post), &[]), entry(&a, 10, 2, LedgerAction::ModAdd, Some(m.public()), &[&b])];
        c.rebuild(&es, &ident);
        assert!(c.is_mod(&m.public()));
        assert!(c.tombstones.contains_key(&post), "order independent: replayed by time");
        // a stranger can't ban
        let s = Keypair::generate().unwrap();
        c.rebuild(&[es[0].clone(), es[1].clone(), entry(&s, 30, 4, LedgerAction::Ban, Some(m.public()), &[])], &ident);
        assert!(!c.is_banned(&m.public()));
        assert_eq!(c.refused.len(), 1);
    }

    fn limits_entry(signer: &Keypair, at: u64, seq: u64, f: impl Fn(&mut Limits)) -> Entry {
        let mut e = entry(signer, at, seq, LedgerAction::Limits, None, &[]);
        let mut l = Limits::default();
        f(&mut l);
        e.body.limits = Some(l);
        e
    }

    #[test]
    fn limits_are_proposals_and_apply_as_patches() {
        let a = Keypair::generate().unwrap();
        let mut c = Community::new(Id([1; 32]), Id([2; 32]), genesis(vec![a.public()], 1), 0);
        let ident = |k: &Id, _: u64| Some(*k);
        let p1 = limits_entry(&a, 10, 1, |l| l.ladder.hide_quorum = 4);
        let p2 = limits_entry(&a, 20, 2, |l| l.pow_bits = 20);
        let es = vec![p1.clone(), p2.clone()];
        c.rebuild(&es, &ident);
        assert_eq!(c.limits, Limits::default(), "proposed, not applied");
        assert_eq!(c.proposals.len(), 2);
        assert_eq!(c.proposals[&p1.id].changes(), vec![("hide_quorum", "3".into(), "4".into())]);
        // p2 is accepted first, p1 later: both changes stick, neither undoes the other
        let pen = ladder::Penalties { passed: [(p2.id, 30), (p1.id, 40)].into(), ..Default::default() };
        c.replay(&es, &ident, &pen);
        assert_eq!(c.limits.ladder.hide_quorum, 4);
        assert_eq!(c.limits.pow_bits, 20);
        assert_eq!(c.proposals[&p1.id].applied_at, Some(40));
        // and what was in effect when stays known
        assert_eq!(c.limits_at(29), &Limits::default());
        assert_eq!((c.limits_at(30).pow_bits, c.limits_at(30).ladder.hide_quorum), (20, 3));
        assert_eq!(c.limits_at(40), &c.limits);
    }

    #[test]
    fn scatter_fields_patch_like_the_others() {
        let base = Limits::default();
        let want = Limits { scatter: hoi_types::Scatter { resist: 15, open: false }, ..Limits::default() };
        // a concurrent proposal changed the stamp meanwhile: both changes compose
        let cur = Limits { pow_bits: 20, ..Limits::default() };
        let out = patch_limits(&base, &want, &cur);
        assert_eq!((out.pow_bits, out.scatter.resist), (20, 15));
        let mut changes = Vec::new();
        diff_limits(&base, &want, &mut |f, a, b| changes.push((f, a, b)));
        assert_eq!(changes, vec![("scatter_resist", "10".to_string(), "15".to_string())]);
    }
}
