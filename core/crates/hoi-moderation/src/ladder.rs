//! The trust ladder (PROTOCOL.md §15, §16): who a community trusts,
//! decided by the community's own signed records and nothing else, the same way on every device.
//!
//! ```text
//! newcomer ──(vouches from established members)──▶ member ──(time)──▶ established ──(keeps
//!  text only      links, votes count                      reviews, vouches   contributing,
//!                                                         and vetoes count   good response)──▶ trusted
//!                                                                                              media
//! ```
//!
//! - **Admission.** A newcomer becomes a member once enough members who were *established at
//!   the time they vouched* vouch for them (owners and moderators are established from the
//!   start). Clients vouch automatically when an established member reads a newcomer's posts
//!   and doesn't hide them, so validation is a side effect of reading.
//! - **Voice.** Admission counts people; decisions weigh them. Every established member gives
//!   away at most one voice per [`VOUCH_EPOCH`], split evenly among everyone they vouched for in
//!   it and scaled by their own voice, and nobody holds more voice than their strongest
//!   voucher. Someone who vouches in all their friends at once gives each a sliver: the friends
//!   are welcome, but they don't arrive as a voting bloc, and together they can't raise anyone
//!   above what they hold. Founders hold a full voice; nobody holds more.
//! - **Hiding.** Established members hide records with a reason from a closed list (spam,
//!   abuse, illegal, off-topic, a rule). Enough voice hiding, outweighing defenders two to one,
//!   puts a record to sleep for everyone (peekable, never deleted). Enough voice across a
//!   person's records, or standing objections, blocks the person from the community.
//! - **Checks on power.** Established members veto moderators' and owners' actions; a vetoed
//!   action is undone, and a moderator overturned three times is suspended. Hides that the
//!   community defends count against the hider. Vouching for people who end up blocked costs
//!   the voucher their voice in reviews and vetoes, and stops their later vouches from
//!   counting (earlier admissions stand). Quorums never exceed a third of the established
//!   voice, so owners can't set them out of reach; every decision needs two people at least,
//!   so nobody decides alone.
//! - **Changes by consent.** Owners propose new rules or limits; the people established when
//!   it was proposed decide. A change applies once everyone agreed, or, while only people who
//!   are away (nothing of theirs left in the community) haven't answered, for as long as that
//!   holds; one retention window later it applies for good unless objections weigh half a
//!   voice. See [`decide`].
//!
//! Everything here is a pure function of the records a device holds and the clock. Two devices
//! holding the same records compute the same ladder.

use std::collections::{BTreeMap, BTreeSet};

use hoi_types::{Id, Ladder, Verdict};

/// Contributions count toward "trusted" for this long (capped by the community's retention).
pub const STANDING_WINDOW: u64 = 90 * 86_400;
/// Overturned actions that suspend a moderator.
pub const MOD_STRIKES: usize = 3;
/// Defended hides that cost a reviewer their hiding power.
pub const REVIEW_STRIKES: usize = 3;
/// Vouchees who end up blocked or banned that cost a voucher their vouching power.
pub const VOUCH_STRIKES: usize = 2;
/// One voice. Voices are whole numbers so every device gets the same sums; this one divides
/// evenly by every count from 1 to 16, so a voice shared among a few people adds back up.
pub const VOICE: u64 = 720_720;
/// Each established member gives away at most one voice per epoch (`created_at / VOUCH_EPOCH`),
/// split evenly among everyone they vouched for in it.
pub const VOUCH_EPOCH: u64 = 90 * 86_400;
/// Objections (and, while it counts, silence) weighing this much stop a change to rules or limits.
pub const DISSENT: u64 = VOICE / 2;
/// Hides, blocks and vetoes need at least this many distinct people, whatever their voice.
pub const MIN_HEADS: usize = 2;
/// Answers per person per proposal that count (the earliest ones); nodes refuse the rest, so
/// nobody can fill every device with answers that are kept as long as the proposal.
pub const MAX_ANSWERS: usize = 16;
/// A `retire` entry counts only for someone who has signed nothing in the community for this
/// long before it.
pub const RETIRE_AWAY: u64 = 30 * 86_400;
/// A retirement takes effect this long after its entry, so the person can come back (anything
/// they sign voids it) and established members can veto it.
pub const RETIRE_NOTICE: u64 = 7 * 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Newcomer,
    Member,
    Established,
    Trusted,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Newcomer => "newcomer",
            Level::Member => "member",
            Level::Established => "established",
            Level::Trusted => "trusted",
        }
    }

    /// What a person at this level may do (PROTOCOL.md §15.3).
    pub fn capabilities(&self) -> &'static [&'static str] {
        match self {
            Level::Newcomer => &["text"],
            Level::Member => &["text", "links", "vote"],
            Level::Established => &["text", "links", "vote", "review", "vouch", "veto"],
            Level::Trusted => &["text", "links", "vote", "review", "vouch", "veto", "media"],
        }
    }
}

/// A post or comment that is still alive.
#[derive(Debug, Clone)]
pub struct Contribution {
    pub id: Id,
    pub author: Id,
    pub created_at: u64,
}

/// `created_at` is the record's effective time: when it says it was made, or when this device
/// first saw it if it arrived late (PROTOCOL.md §15.5).
#[derive(Debug, Clone)]
pub struct VouchIn {
    pub id: Id,
    pub voucher: Id,
    pub subject: Id,
    pub value: i64,
    pub seq: u64,
    pub created_at: u64,
}

/// `created_at` is the effective time, as for [`VouchIn`].
#[derive(Debug, Clone)]
pub struct ReviewIn {
    pub id: Id,
    pub reviewer: Id,
    pub target: Id,
    pub verdict: Verdict,
    pub reason: Option<String>,
    pub seq: u64,
    pub created_at: u64,
}

/// A vote's latest value per (voter, target).
#[derive(Debug, Clone)]
pub struct VoteIn {
    pub voter: Id,
    pub target: Id,
    pub value: i64,
}

/// A ledger entry as the ladder sees it, for vetoes.
#[derive(Debug, Clone)]
pub struct EntryIn {
    pub id: Id,
    pub created_at: u64,
    pub signers: Vec<Id>,
    pub target: Option<Id>,
    pub vetoable: bool,
}

/// An `admit` entry that took effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmitIn {
    pub person: Id,
    pub at: u64,
    /// The owner or moderator who admitted them: an admission is their vouch.
    pub by: Id,
    pub entry: Id,
}

/// A proposed change to the rules or limits (PROTOCOL.md §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalIn {
    pub id: Id,
    /// Effective time of the proposal: the electorate is who was established then.
    pub at: u64,
    /// Owners who signed it: they agree.
    pub signers: Vec<Id>,
    /// The ladder in effect when it was proposed (who counts as established for it).
    pub ladder: Ladder,
    /// One retention window after `at`: answers close, silence counts as consent.
    pub deadline: u64,
}

/// Everything the ladder reads, gathered by the node from the records it holds.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub now: u64,
    pub contributions: Vec<Contribution>,
    /// Author of every post or comment we know of, live or removed.
    pub authors: BTreeMap<Id, Id>,
    pub vouches: Vec<VouchIn>,
    /// Every review the node holds. Answers to proposals are all kept (their history decides);
    /// for anything else only the latest per (reviewer, target) counts.
    pub reviews: Vec<ReviewIn>,
    pub votes: Vec<VoteIn>,
    pub burned: BTreeSet<Id>,
    /// The latest effective time of anything each person signed in the community, ledger
    /// entries included (a `retire` lapses when its person signs again).
    pub last_seen: BTreeMap<Id, u64>,
}

/// What the community's ledger says, as the ladder needs it.
#[derive(Debug, Clone, Default)]
pub struct LedgerView {
    pub owners: BTreeSet<Id>,
    /// Active moderators and since when.
    pub mods: BTreeMap<Id, u64>,
    pub genesis_at: u64,
    pub banned: BTreeSet<Id>,
    pub admits: Vec<AdmitIn>,
    /// Removed records (pending or done): (record, when removed).
    pub removed: Vec<(Id, u64)>,
    pub entries: Vec<EntryIn>,
    pub proposals: Vec<ProposalIn>,
    pub ladder: Ladder,
    /// The community's retention window, seconds.
    pub retention: u64,
    /// `retire` entries in effect on the ledger: (person, entry time).
    pub retired: Vec<(Id, u64)>,
    /// Only `admit` entries let people in (age-proof communities): a vouch still
    /// gives voice to someone admitted, but admits nobody.
    pub admits_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Person {
    pub level: Option<Level>,
    pub founder: bool,
    pub admitted_at: Option<u64>,
    pub established_at: Option<u64>,
    /// Established members whose vouches counted.
    pub vouchers: Vec<Id>,
    /// Established members objecting right now.
    pub objectors: Vec<Id>,
    pub hiders: usize,
    pub defenders: usize,
    pub blocked: bool,
    /// Live contributions in the standing window.
    pub contributions: usize,
    pub score: i64,
    /// Vouched for too many people who ended up blocked: no voice in reviews and vetoes, and
    /// later vouches don't count.
    pub struck: bool,
    /// Hides of theirs the community defended.
    pub overruled: usize,
    /// Their weight in hides, blocks and vetoes, in units of [`VOICE`].
    pub voice: u64,
    /// People they vouched for in the current epoch: each of those vouches carries
    /// `voice / vouched_this_epoch`.
    pub vouched_this_epoch: usize,
}

impl Person {
    pub fn level(&self) -> Level {
        self.level.unwrap_or(Level::Newcomer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hidden {
    pub hiders: usize,
    pub defenders: usize,
    /// The reason most hiders gave.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VetoTally {
    pub vetoes: usize,
    pub support: usize,
    pub vetoes_voice: u64,
    pub support_voice: u64,
    /// Voice the vetoes need.
    pub need: u64,
    pub overturned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalStatus {
    /// Waiting for answers.
    Open,
    /// Everyone still around agreed; the people who haven't answered are away. In effect for as
    /// long as that holds, final at the deadline.
    InEffect,
    /// In effect for good: everyone agreed (`unanimous`), or the deadline passed without
    /// objections weighing half a voice.
    Passed { at: u64, unanimous: bool },
    /// The deadline passed with objections: never takes effect.
    Lapsed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalTally {
    pub status: ProposalStatus,
    pub deadline: u64,
    /// Who decides: the people established when it was proposed.
    pub electors: BTreeSet<Id>,
    pub agreed: Vec<Id>,
    pub objecting: Vec<Id>,
    /// Around (something of theirs is live here) and haven't answered.
    pub waiting: Vec<Id>,
    /// Away (nothing of theirs is left here) and haven't answered.
    pub away: Vec<Id>,
    /// Retired by the owners after leaving for good, without having answered: their
    /// silence holds nothing up.
    pub retired: Vec<Id>,
    pub agreed_voice: u64,
    pub objecting_voice: u64,
    pub waiting_voice: u64,
    pub away_voice: u64,
    pub retired_voice: u64,
}

/// Effective quorums right now. `admit` counts people; the others are voice (see [`VOICE`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quorums {
    pub admit: usize,
    pub hide: u64,
    pub block: u64,
    pub veto: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Standing {
    pub people: BTreeMap<Id, Person>,
    pub hidden: BTreeMap<Id, Hidden>,
    pub vetoes: BTreeMap<Id, VetoTally>,
    pub overturned: BTreeSet<Id>,
    /// Moderators suspended by the community, from when.
    pub suspended: BTreeMap<Id, u64>,
    pub proposals: BTreeMap<Id, ProposalTally>,
    pub established: usize,
    /// The established members' voice.
    pub established_voice: u64,
    pub quorums: Quorums,
}

impl Standing {
    pub fn person(&self, root: &Id) -> Option<&Person> {
        self.people.get(root)
    }

    pub fn level(&self, root: &Id) -> Level {
        self.people.get(root).map_or(Level::Newcomer, Person::level)
    }

    pub fn blocked(&self, root: &Id) -> bool {
        self.people.get(root).is_some_and(|p| p.blocked)
    }

    pub fn admitted(&self, root: &Id) -> bool {
        self.level(root) >= Level::Member
    }

    pub fn counted_vouches(&self, root: &Id) -> usize {
        self.people.get(root).map_or(0, |p| p.vouchers.len())
    }

    pub fn voice(&self, root: &Id) -> u64 {
        self.people.get(root).map_or(0, |p| p.voice)
    }
}

/// A quorum in voice: `q` voices, capped at a third of the established
/// voice (rounded up), never below one voice. Decisions also need [`MIN_HEADS`] people.
pub fn quorum(q: u32, established_voice: u64) -> u64 {
    let cap = VOICE.max(established_voice.div_ceil(3));
    (q as u64 * VOICE).clamp(VOICE, cap)
}

/// The penalty sets one round of the fixpoint produces and the next one reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Penalties {
    pub blocked: BTreeSet<Id>,
    /// Struck vouchers and the time of the last vouch that struck them.
    pub struck: BTreeMap<Id, u64>,
    pub overruled: BTreeSet<Id>,
    pub overturned: BTreeSet<Id>,
    pub suspended: BTreeMap<Id, u64>,
    /// Proposals in effect and from when (`now` for those in effect while people are away).
    pub passed: BTreeMap<Id, u64>,
}

fn latest_vouches(v: &[VouchIn]) -> BTreeMap<(Id, Id), &VouchIn> {
    let mut m: BTreeMap<(Id, Id), &VouchIn> = BTreeMap::new();
    for x in v {
        let e = m.entry((x.voucher, x.subject)).or_insert(x);
        if (x.seq, x.created_at, x.id) > (e.seq, e.created_at, e.id) {
            *e = x;
        }
    }
    m
}

fn latest_reviews<'a>(v: impl Iterator<Item = &'a ReviewIn>) -> BTreeMap<(Id, Id), &'a ReviewIn> {
    let mut m: BTreeMap<(Id, Id), &ReviewIn> = BTreeMap::new();
    for x in v {
        let e = m.entry((x.reviewer, x.target)).or_insert(x);
        if (x.seq, x.created_at, x.id) > (e.seq, e.created_at, e.id) {
            *e = x;
        }
    }
    m
}

/// A counted vouch (or an admission): `from` passes part of their voice to `to`.
#[derive(Debug, Clone)]
struct Gift {
    from: Id,
    to: Id,
    at: u64,
    id: Id,
}

struct Admission {
    admitted_at: BTreeMap<Id, u64>,
    counted: BTreeMap<Id, BTreeSet<Id>>,
    gifts: Vec<Gift>,
}

/// Replays vouches and `admit` entries in time order. A vouch counts when its author was
/// established at the vouch's time, is not excluded now, and wasn't struck before it.
fn admission(
    lv: &LedgerView,
    founders: &BTreeMap<Id, u64>,
    vouches: &BTreeMap<(Id, Id), &VouchIn>,
    excluded: &BTreeSet<Id>,
    struck: &BTreeMap<Id, u64>,
    ladder: &Ladder,
) -> Admission {
    enum Ev<'a> {
        Admit(&'a AdmitIn),
        Vouch(&'a VouchIn),
    }
    let mut evs: Vec<(u64, Id, Ev)> = Vec::new();
    for a in &lv.admits {
        evs.push((a.at, a.entry, Ev::Admit(a)));
    }
    for v in vouches.values().filter(|v| v.value > 0) {
        evs.push((v.created_at, v.id, Ev::Vouch(v)));
    }
    evs.sort_by_key(|a| (a.0, a.1));
    let mut admitted_at: BTreeMap<Id, u64> = founders.iter().filter(|(k, _)| !excluded.contains(*k)).map(|(k, v)| (*k, *v)).collect();
    let mut counted: BTreeMap<Id, BTreeSet<Id>> = BTreeMap::new();
    let mut gifts = Vec::new();
    let est_since = |admitted_at: &BTreeMap<Id, u64>, p: &Id| -> Option<u64> {
        if let Some(t) = founders.get(p) {
            return Some(*t);
        }
        admitted_at.get(p).map(|t| t.saturating_add(ladder.established_age))
    };
    for (t, _, ev) in evs {
        match ev {
            Ev::Admit(a) => {
                if !excluded.contains(&a.person) {
                    admitted_at.entry(a.person).or_insert(t);
                    if founders.get(&a.by).is_some_and(|s| *s <= t) && a.by != a.person {
                        gifts.push(Gift { from: a.by, to: a.person, at: t, id: a.entry });
                    }
                }
            }
            Ev::Vouch(v) => {
                if excluded.contains(&v.subject) || excluded.contains(&v.voucher) || struck.get(&v.voucher).is_some_and(|s| v.created_at > *s) {
                    continue;
                }
                if est_since(&admitted_at, &v.voucher).is_none_or(|s| s > t) {
                    continue;
                }
                let set = counted.entry(v.subject).or_default();
                set.insert(v.voucher);
                gifts.push(Gift { from: v.voucher, to: v.subject, at: t, id: v.id });
                if !admitted_at.contains_key(&v.subject) && !lv.admits_only {
                    let established_now = admitted_at.keys().filter(|p| est_since(&admitted_at, p).is_some_and(|s| s <= t)).count();
                    let need = (ladder.admit_vouches as usize).clamp(1, established_now.max(1));
                    if set.len() >= need {
                        admitted_at.insert(v.subject, t);
                    }
                }
            }
        }
    }
    Admission { admitted_at, counted, gifts }
}

/// Everyone's voice over time, from the counted vouches of an [`Admission`]: a gift passes the
/// giver's voice at that moment divided by how many people they vouched for in that epoch, and
/// nobody rises above the strongest voice that vouched for them. Gifts are replayed in time
/// order and read only earlier ones, so voice can't loop back on itself, and a group can't lift
/// anyone above what its members hold.
struct Voices<'a> {
    founders: &'a BTreeMap<Id, u64>,
    points: BTreeMap<Id, Vec<(u64, u64)>>,
    load: BTreeMap<(Id, u64), u64>,
}

impl<'a> Voices<'a> {
    fn new(adm: &Admission, founders: &'a BTreeMap<Id, u64>) -> Voices<'a> {
        let mut load: BTreeMap<(Id, u64), u64> = BTreeMap::new();
        for g in &adm.gifts {
            *load.entry((g.from, g.at / VOUCH_EPOCH)).or_default() += 1;
        }
        let mut gifts: Vec<&Gift> = adm.gifts.iter().collect();
        gifts.sort_by_key(|g| (g.at, g.id));
        let mut v = Voices { founders, points: BTreeMap::new(), load };
        let mut ceiling: BTreeMap<Id, u64> = BTreeMap::new();
        for g in gifts {
            let n = v.load.get(&(g.from, g.at / VOUCH_EPOCH)).copied().unwrap_or(1).max(1);
            let giver = v.at(&g.from, g.at);
            let top = ceiling.entry(g.to).or_default();
            *top = (*top).max(giver);
            let cur = v.at(&g.to, g.at);
            let next = (cur + giver / n).min(*top).min(VOICE);
            if next > cur {
                v.points.entry(g.to).or_default().push((g.at, next));
            }
        }
        v
    }

    fn at(&self, p: &Id, t: u64) -> u64 {
        if self.founders.get(p).is_some_and(|s| *s <= t) {
            return VOICE;
        }
        self.points.get(p).and_then(|v| v.iter().rev().find(|(at, _)| *at <= t)).map_or(0, |x| x.1)
    }

    fn sum<'b>(&self, people: impl IntoIterator<Item = &'b Id>, t: u64) -> u64 {
        people.into_iter().map(|p| self.at(p, t)).sum()
    }

    fn vouched_in(&self, p: &Id, t: u64) -> usize {
        self.load.get(&(*p, t / VOUCH_EPOCH)).copied().unwrap_or(0) as usize
    }
}

/// Decides a proposed change to the rules or limits (PROTOCOL.md §6.1). `electors` are the
/// people established when it was proposed; `answers` are all of their answers (reviews of the
/// proposal) with the times that count; `voice` is each elector's voice when it was proposed;
/// `around` says whether something of theirs is still live and visible in the community.
///
/// - **Passed, unanimous**: at the first moment the electors who hadn't agreed weighed less
///   than [`DISSENT`]. Final: later answers don't undo it.
/// - **In effect** (before the deadline): the electors who object, or who are around and
///   haven't agreed, weigh less than [`DISSENT`]. People who are away don't hold it up; if they
///   come back and object before the deadline, it stops again.
/// - **Passed** at the deadline (one retention window after it was proposed, when everything
///   anyone had posted before has faded) unless objections weigh [`DISSENT`]; **lapsed**
///   otherwise. Answers made at or after the deadline don't count.
///
/// `retired` maps electors the owners retired to when that took effect: from then on
/// their silence counts for nothing, neither as dissent nor as waiting. A retirement never
/// takes away an answer someone gave.
pub fn decide(
    p: &ProposalIn,
    electors: &BTreeSet<Id>,
    answers: &[&ReviewIn],
    voice: &dyn Fn(&Id) -> u64,
    around: &dyn Fn(&Id) -> bool,
    retired: &BTreeMap<Id, u64>,
    now: u64,
) -> ProposalTally {
    let mut by: BTreeMap<Id, Vec<&ReviewIn>> = BTreeMap::new();
    for a in answers.iter().filter(|a| electors.contains(&a.reviewer) && a.created_at < p.deadline && a.created_at <= now) {
        by.entry(a.reviewer).or_default().push(a);
    }
    for v in by.values_mut() {
        v.sort_by_key(|a| (a.created_at, a.id));
        v.truncate(MAX_ANSWERS);
    }
    let answer = |x: &Id, t: u64| -> Option<Verdict> {
        if p.signers.contains(x) {
            return Some(Verdict::Ok);
        }
        by.get(x)?.iter().filter(|a| a.created_at.max(p.at) <= t).max_by_key(|a| (a.seq, a.created_at, a.id)).map(|a| a.verdict)
    };
    let gone = |x: &Id, t: u64| retired.get(x).is_some_and(|r| *r <= t);
    let mut times: Vec<u64> = by
        .values()
        .flatten()
        .map(|a| a.created_at.max(p.at))
        .chain([p.at])
        .chain(retired.iter().filter(|(x, _)| electors.contains(*x)).map(|(_, r)| (*r).max(p.at)).filter(|r| *r < p.deadline && *r <= now))
        .collect();
    times.sort_unstable();
    times.dedup();
    let unanimous = times.into_iter().find(|t| {
        electors
            .iter()
            .filter(|x| match answer(x, *t) {
                Some(Verdict::Ok) => false,
                None => !gone(x, *t),
                _ => true,
            })
            .map(voice)
            .sum::<u64>()
            < DISSENT
    });

    let at = if now < p.deadline { now } else { p.deadline - 1 };
    let mut t = ProposalTally {
        status: ProposalStatus::Open,
        deadline: p.deadline,
        electors: electors.clone(),
        agreed: Vec::new(),
        objecting: Vec::new(),
        waiting: Vec::new(),
        away: Vec::new(),
        retired: Vec::new(),
        agreed_voice: 0,
        objecting_voice: 0,
        waiting_voice: 0,
        away_voice: 0,
        retired_voice: 0,
    };
    for x in electors {
        let (list, sum) = match answer(x, at) {
            Some(Verdict::Ok) => (&mut t.agreed, &mut t.agreed_voice),
            Some(Verdict::Veto) => (&mut t.objecting, &mut t.objecting_voice),
            None if gone(x, at) => (&mut t.retired, &mut t.retired_voice),
            _ if around(x) => (&mut t.waiting, &mut t.waiting_voice),
            _ => (&mut t.away, &mut t.away_voice),
        };
        list.push(*x);
        *sum += voice(x);
    }
    t.status = match unanimous {
        Some(at) => ProposalStatus::Passed { at, unanimous: true },
        None if now < p.deadline => {
            if t.objecting_voice + t.waiting_voice < DISSENT {
                ProposalStatus::InEffect
            } else {
                ProposalStatus::Open
            }
        }
        None if t.objecting_voice < DISSENT => ProposalStatus::Passed { at: p.deadline, unanimous: false },
        None => ProposalStatus::Lapsed,
    };
    t
}

/// One round: the ladder given the penalties of the previous round.
fn round(lv: &LedgerView, inp: &Inputs, pen: &Penalties) -> (Standing, Penalties) {
    let ladder = &lv.ladder;
    let now = inp.now;
    let window = STANDING_WINDOW.min(lv.retention.max(1));
    let mut founders: BTreeMap<Id, u64> = lv.owners.iter().map(|o| (*o, lv.genesis_at)).collect();
    for (m, since) in &lv.mods {
        if !pen.suspended.contains_key(m) {
            founders.entry(*m).or_insert(*since);
        }
    }
    let vouches = latest_vouches(&inp.vouches);
    let proposal_ids: BTreeSet<Id> = lv.proposals.iter().map(|p| p.id).collect();
    let reviews = latest_reviews(inp.reviews.iter().filter(|r| !proposal_ids.contains(&r.target)));

    let mut excluded: BTreeSet<Id> = lv.banned.union(&inp.burned).copied().collect();
    excluded.extend(pen.blocked.iter().filter(|p| !lv.owners.contains(*p)));
    excluded.retain(|p| !lv.owners.contains(p));
    let adm = admission(lv, &founders, &vouches, &excluded, &pen.struck, ladder);

    // Who could veto: established ignoring ledger bans and blocks, so a moderator can't ban the
    // people who would overturn their bans. (Blocked people are filtered out where it matters;
    // this pass also shows whose vouches brought blocked people in.)
    let adm_v = admission(lv, &founders, &vouches, &inp.burned, &pen.struck, ladder);
    let voice = Voices::new(&adm, &founders);
    let voice_v = Voices::new(&adm_v, &founders);

    let level_of = |a: &Admission, p: &Id| -> Option<Level> {
        if lv.owners.contains(p) {
            return Some(Level::Trusted);
        }
        if founders.contains_key(p) {
            return Some(Level::Trusted);
        }
        let t = a.admitted_at.get(p)?;
        Some(if t.saturating_add(ladder.established_age) <= now { Level::Established } else { Level::Member })
    };
    let established = |p: &Id| level_of(&adm, p).is_some_and(|l| l >= Level::Established);
    // whose judgement counts in hides, defences and objections
    let juror = |p: &Id| established(p) && !pen.struck.contains_key(p);
    let est: Vec<Id> = adm.admitted_at.keys().filter(|p| established(p)).copied().collect();
    let est_v: Vec<Id> =
        adm_v.admitted_at.keys().filter(|p| !pen.blocked.contains(*p) && level_of(&adm_v, p).is_some_and(|l| l >= Level::Established)).copied().collect();
    let est_voice = voice.sum(&est, now);
    let est_voice_v = voice_v.sum(&est_v, now);
    let q = Quorums {
        admit: (ladder.admit_vouches as usize).clamp(1, est.len().max(1)),
        hide: quorum(ladder.hide_quorum, est_voice),
        block: quorum(ladder.block_quorum, est_voice),
        veto: quorum(ladder.veto_quorum, est_voice_v),
    };
    let weigh = |set: &BTreeSet<Id>| voice.sum(set, now);

    // ---- record outcomes
    let mut by_target: BTreeMap<Id, Vec<&ReviewIn>> = BTreeMap::new();
    for r in reviews.values() {
        by_target.entry(r.target).or_default().push(r);
    }
    let contrib_ids: BTreeSet<Id> = inp.contributions.iter().map(|c| c.id).collect();
    let mut hiders_of: BTreeMap<Id, BTreeSet<Id>> = BTreeMap::new();
    let mut defenders_of: BTreeMap<Id, BTreeSet<Id>> = BTreeMap::new();
    let mut hidden = BTreeMap::new();
    let mut overruled_count: BTreeMap<Id, usize> = BTreeMap::new();
    for c in &inp.contributions {
        let Some(rs) = by_target.get(&c.id) else { continue };
        let hiders: BTreeSet<Id> = rs
            .iter()
            .filter(|r| r.verdict == Verdict::Hide && r.reviewer != c.author && juror(&r.reviewer) && !pen.overruled.contains(&r.reviewer))
            .map(|r| r.reviewer)
            .collect();
        let defenders: BTreeSet<Id> =
            rs.iter().filter(|r| r.verdict == Verdict::Ok && r.reviewer != c.author && juror(&r.reviewer)).map(|r| r.reviewer).collect();
        let (hv, dv) = (weigh(&hiders), weigh(&defenders));
        if hiders.len() >= MIN_HEADS && hv >= q.hide && hv > 2 * dv {
            let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
            for r in rs.iter().filter(|r| hiders.contains(&r.reviewer)) {
                *reasons.entry(r.reason.as_deref().unwrap_or("spam")).or_default() += 1;
            }
            let reason = reasons.iter().max_by_key(|(k, n)| (**n, std::cmp::Reverse(**k))).map_or("spam", |(k, _)| *k).to_string();
            hidden.insert(c.id, Hidden { hiders: hiders.len(), defenders: defenders.len(), reason });
        }
        if !hiders.is_empty() && defenders.len() >= MIN_HEADS && dv >= q.hide && dv >= 2 * hv {
            // the community defended it: every hide on it was overruled, counted or not
            for r in rs.iter().filter(|r| r.verdict == Verdict::Hide) {
                *overruled_count.entry(r.reviewer).or_default() += 1;
            }
        }
        hiders_of.insert(c.id, hiders);
        defenders_of.insert(c.id, defenders);
    }

    // ---- people
    let mut universe: BTreeSet<Id> = founders.keys().copied().collect();
    universe.extend(inp.contributions.iter().map(|c| c.author));
    universe.extend(adm.admitted_at.keys().copied());
    universe.extend(vouches.keys().flat_map(|(a, b)| [*a, *b]));
    universe.extend(reviews.keys().map(|(a, _)| *a));

    let mut by_author: BTreeMap<Id, Vec<&Contribution>> = BTreeMap::new();
    for c in &inp.contributions {
        by_author.entry(c.author).or_default().push(c);
    }
    let mut votes_on: BTreeMap<Id, Vec<&VoteIn>> = BTreeMap::new();
    for v in &inp.votes {
        if contrib_ids.contains(&v.target) {
            votes_on.entry(v.target).or_default().push(v);
        }
    }
    let mut removed_of: BTreeMap<Id, usize> = BTreeMap::new();
    for (rec, at) in &lv.removed {
        if at.saturating_add(window) >= now
            && let Some(a) = inp.authors.get(rec)
        {
            *removed_of.entry(*a).or_default() += 1;
        }
    }

    let mut people = BTreeMap::new();
    let mut blocked = BTreeSet::new();
    for p in &universe {
        let mine = by_author.get(p).map(Vec::as_slice).unwrap_or(&[]);
        let mut hiders: BTreeSet<Id> = mine.iter().flat_map(|c| hiders_of.get(&c.id).into_iter().flatten().copied()).collect();
        let mut defenders: BTreeSet<Id> = mine.iter().flat_map(|c| defenders_of.get(&c.id).into_iter().flatten().copied()).collect();
        let mut objectors = Vec::new();
        for ((voucher, subject), v) in &vouches {
            if subject != p || voucher == p || !juror(voucher) {
                continue;
            }
            match v.value {
                1 => {
                    defenders.insert(*voucher);
                }
                -1 if !pen.overruled.contains(voucher) => {
                    hiders.insert(*voucher);
                    objectors.push(*voucher);
                }
                _ => {}
            }
        }
        let admitted = adm.admitted_at.contains_key(p);
        let need = if admitted { q.block } else { q.hide };
        let (hv, dv) = (weigh(&hiders), weigh(&defenders));
        let is_blocked = !lv.owners.contains(p) && hiders.len() >= MIN_HEADS && hv >= need && hv > 2 * dv;
        if is_blocked {
            blocked.insert(*p);
        }
        let recent: Vec<&&Contribution> = mine.iter().filter(|c| c.created_at.saturating_add(window) >= now).collect();
        let mut score: i64 = 0;
        for c in &recent {
            let (mut ups, mut downs) = (0i64, 0i64);
            for v in votes_on.get(&c.id).into_iter().flatten() {
                if v.voter == *p || pen.blocked.contains(&v.voter) || level_of(&adm, &v.voter).is_none() {
                    continue;
                }
                match v.value {
                    1 => ups += 1,
                    -1 => downs += 1,
                    _ => {}
                }
            }
            let oks = defenders_of.get(&c.id).map_or(0, |s| s.len()) as i64;
            let hides = hiders_of.get(&c.id).map_or(0, |s| s.len()) as i64;
            score += ups + oks - downs.min(2) - 3 * hides;
        }
        score -= 5 * removed_of.get(p).copied().unwrap_or(0) as i64;
        let mut level = if pen.blocked.contains(p) && !lv.owners.contains(p) { None } else { level_of(&adm, p) };
        if level == Some(Level::Established) && recent.len() >= ladder.trusted_posts as usize && score >= ladder.media_score as i64 {
            level = Some(Level::Trusted);
        }
        let admitted_at = adm.admitted_at.get(p).copied();
        people.insert(
            *p,
            Person {
                level,
                founder: founders.contains_key(p),
                admitted_at,
                established_at: if founders.contains_key(p) {
                    admitted_at
                } else {
                    admitted_at.map(|t| t.saturating_add(ladder.established_age)).filter(|t| *t <= now)
                },
                vouchers: adm.counted.get(p).map(|s| s.iter().copied().collect()).unwrap_or_default(),
                objectors,
                hiders: hiders.len(),
                defenders: defenders.len(),
                blocked: is_blocked,
                contributions: recent.len(),
                score,
                struck: pen.struck.contains_key(p),
                overruled: overruled_count.get(p).copied().unwrap_or(0),
                voice: if level.is_some() { voice.at(p, now) } else { 0 },
                vouched_this_epoch: voice.vouched_in(p, now),
            },
        );
    }

    // ---- struck vouchers: counted vouches for people now blocked or banned
    let mut vouched_bad: BTreeMap<Id, Vec<u64>> = BTreeMap::new();
    for (subject, vs) in &adm_v.counted {
        if blocked.contains(subject) || lv.banned.contains(subject) {
            for v in vs {
                if let Some(x) = vouches.get(&(*v, *subject)) {
                    vouched_bad.entry(*v).or_default().push(x.created_at);
                }
            }
        }
    }
    let struck: BTreeMap<Id, u64> = vouched_bad
        .into_iter()
        .filter(|(v, ts)| ts.len() >= VOUCH_STRIKES && !lv.owners.contains(v))
        .map(|(v, ts)| (v, ts.into_iter().max().unwrap_or(0)))
        .collect();
    let overruled: BTreeSet<Id> = overruled_count.iter().filter(|(_, n)| **n >= REVIEW_STRIKES).map(|(k, _)| *k).collect();

    // ---- vetoes
    let can_veto = |p: &Id| level_of(&adm_v, p).is_some_and(|l| l >= Level::Established) && !pen.struck.contains_key(p) && !pen.blocked.contains(p);
    let mut vetoes = BTreeMap::new();
    let mut overturned = BTreeSet::new();
    for e in lv.entries.iter().filter(|e| e.vetoable) {
        let Some(rs) = by_target.get(&e.id) else { continue };
        let eligible = |r: &&&ReviewIn| !e.signers.contains(&r.reviewer) && Some(r.reviewer) != e.target && can_veto(&r.reviewer);
        let v: BTreeSet<Id> = rs.iter().filter(|r| r.verdict == Verdict::Veto).filter(eligible).map(|r| r.reviewer).collect();
        let s: BTreeSet<Id> = rs.iter().filter(|r| r.verdict == Verdict::Ok).filter(eligible).map(|r| r.reviewer).collect();
        let (vv, sv) = (voice_v.sum(&v, now), voice_v.sum(&s, now));
        let over = v.len() >= MIN_HEADS && vv >= q.veto && vv > sv;
        if over {
            overturned.insert(e.id);
        }
        vetoes.insert(e.id, VetoTally { vetoes: v.len(), support: s.len(), vetoes_voice: vv, support_voice: sv, need: q.veto, overturned: over });
    }

    // ---- suspended moderators: three overturned actions, or blocked by the community
    let mut suspended = BTreeMap::new();
    let mut strikes: BTreeMap<Id, Vec<u64>> = BTreeMap::new();
    for e in lv.entries.iter().filter(|e| overturned.contains(&e.id)) {
        for s in e.signers.iter().take(1) {
            strikes.entry(*s).or_default().push(e.created_at);
        }
    }
    for (m, mut ts) in strikes {
        if lv.owners.contains(&m) {
            continue;
        }
        ts.sort_unstable();
        if ts.len() >= MOD_STRIKES {
            suspended.insert(m, ts[MOD_STRIKES - 1]);
        }
    }
    for m in lv.mods.keys().chain(pen.suspended.keys()) {
        if blocked.contains(m) {
            suspended.entry(*m).or_insert(now);
        }
    }

    // ---- proposed changes to the rules and limits: decided by the people established when
    // they were proposed, counted as if ledger bans didn't exist (a moderator can't ban the
    // people who would object), without the blocked and the struck
    let removed: BTreeSet<Id> = lv.removed.iter().map(|(r, _)| *r).collect();
    let around = |p: &Id| by_author.get(p).is_some_and(|cs| cs.iter().any(|c| !hidden.contains_key(&c.id) && !removed.contains(&c.id)));
    let mut excluded_p: BTreeSet<Id> = inp.burned.clone();
    excluded_p.extend(pen.blocked.iter().filter(|p| !lv.owners.contains(*p)));
    // retirements that count: the person had been silent (and a member) a month before the
    // entry and has signed nothing since; in effect after the notice
    let retired: BTreeMap<Id, u64> = lv
        .retired
        .iter()
        .filter(|(x, at)| {
            let since = inp.last_seen.get(x).copied().into_iter().chain(adm_v.admitted_at.get(x).copied()).max().unwrap_or(0);
            !lv.owners.contains(x) && since.saturating_add(RETIRE_AWAY) <= *at
        })
        .map(|(x, at)| (*x, at.saturating_add(RETIRE_NOTICE)))
        .collect();
    let mut by_ladder: BTreeMap<(u32, u64), Admission> = BTreeMap::new();
    let mut proposals = BTreeMap::new();
    let mut passed = BTreeMap::new();
    for p in &lv.proposals {
        let a = by_ladder
            .entry((p.ladder.admit_vouches, p.ladder.established_age))
            .or_insert_with(|| admission(lv, &founders, &vouches, &excluded_p, &pen.struck, &p.ladder));
        let electors: BTreeSet<Id> = a
            .admitted_at
            .iter()
            .filter(|(x, t)| founders.get(*x).map_or(t.saturating_add(p.ladder.established_age), |s| *s) <= p.at)
            .map(|(x, _)| *x)
            .filter(|x| lv.owners.contains(x) || (!pen.struck.contains_key(x) && !pen.blocked.contains(x)))
            .collect();
        let answers: Vec<&ReviewIn> = inp.reviews.iter().filter(|r| r.target == p.id).collect();
        let tally = decide(p, &electors, &answers, &|x| voice_v.at(x, p.at), &around, &retired, now);
        match tally.status {
            ProposalStatus::Passed { at, .. } => {
                passed.insert(p.id, at);
            }
            ProposalStatus::InEffect => {
                passed.insert(p.id, now);
            }
            _ => {}
        }
        proposals.insert(p.id, tally);
    }

    let standing = Standing {
        people,
        hidden,
        vetoes,
        overturned: overturned.clone(),
        suspended: suspended.clone(),
        proposals,
        established: est.len(),
        established_voice: est_voice,
        quorums: q,
    };
    (standing, Penalties { blocked, struck, overruled, overturned, suspended, passed })
}

/// Runs rounds until the penalties stop changing (at most `max_rounds`). `replay` turns the
/// overturned entries, suspended moderators and passed proposals of a round into the ledger
/// view of the next.
pub fn settle(inp: &Inputs, replay: &mut dyn FnMut(&Penalties) -> LedgerView, max_rounds: usize) -> (Standing, LedgerView, Penalties) {
    let mut pen = Penalties::default();
    let mut lv = replay(&pen);
    let mut last = None;
    for _ in 0..max_rounds.max(1) {
        let (st, next) = round(&lv, inp, &pen);
        let ledger_changed = next.overturned != pen.overturned || next.suspended != pen.suspended || next.passed != pen.passed;
        let done = next == pen;
        pen = next;
        last = Some(st);
        if done {
            break;
        }
        if ledger_changed {
            lv = replay(&pen);
        }
    }
    (last.unwrap_or_default(), lv, pen)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;
    // the start of a vouch epoch, so tests know which vouches share one
    const T0: u64 = 231 * VOUCH_EPOCH;

    fn k(n: u8) -> Id {
        Id([n; 32])
    }

    struct World {
        lv: LedgerView,
        inp: Inputs,
        ladder: Ladder,
        n: u8,
    }

    impl World {
        fn new(owner: u8) -> World {
            let lv = LedgerView { owners: [k(owner)].into(), genesis_at: T0, retention: 180 * DAY, ..Default::default() };
            let inp = Inputs { now: T0, ..Default::default() };
            World { lv, inp, ladder: Ladder { admit_vouches: 2, established_age: 10 * DAY, ..Ladder::default() }, n: 100 }
        }
        fn id(&mut self) -> Id {
            self.n += 1;
            Id([self.n; 32])
        }
        fn vouch(&mut self, from: u8, to: u8, value: i64, at: u64) {
            let id = self.id();
            self.inp.vouches.push(VouchIn { id, voucher: k(from), subject: k(to), value, seq: at, created_at: at });
        }
        fn post(&mut self, by: u8, at: u64) -> Id {
            let id = self.id();
            self.inp.contributions.push(Contribution { id, author: k(by), created_at: at });
            self.inp.authors.insert(id, k(by));
            id
        }
        fn review(&mut self, by: u8, target: Id, v: Verdict, at: u64) {
            let id = self.id();
            let reason = matches!(v, Verdict::Hide | Verdict::Veto).then(|| "spam".to_string());
            self.inp.reviews.push(ReviewIn { id, reviewer: k(by), target, verdict: v, reason, seq: at, created_at: at });
        }
        fn propose(&mut self, by: u8, at: u64) -> Id {
            let id = self.id();
            let ladder = self.ladder.clone();
            self.lv.proposals.push(ProposalIn { id, at, signers: vec![k(by)], ladder, deadline: at + self.lv.retention });
            id
        }
        fn run(&self) -> Standing {
            settle_n(self, 6).0
        }
    }

    fn settle_n(w: &World, rounds: usize) -> (Standing, Penalties) {
        let mut lv = w.lv.clone();
        lv.ladder = w.ladder.clone();
        let (st, _, pen) = settle(
            &w.inp,
            &mut |pen| {
                let mut v = lv.clone();
                v.mods.retain(|m, _| !pen.suspended.contains_key(m));
                v
            },
            rounds,
        );
        (st, pen)
    }

    /// Owner 1 brings in 2 and 3 (half a voice each); after aging, 2 and 3 bring in 4 (all of
    /// theirs, but nobody rises above their strongest voucher: half a voice).
    fn grown() -> World {
        let mut w = World::new(1);
        w.vouch(1, 2, 1, T0 + 10);
        w.vouch(1, 3, 1, T0 + 20);
        w.vouch(2, 4, 1, T0 + 11 * DAY);
        w.vouch(3, 4, 1, T0 + 11 * DAY + 5);
        w.inp.now = T0 + 30 * DAY;
        w
    }

    /// `grown`, plus 5, 6, 7 admitted by the owner one epoch later (a third of a voice each).
    fn bigger() -> World {
        let mut w = grown();
        for p in [5, 6, 7] {
            let entry = w.id();
            w.lv.admits.push(AdmitIn { person: k(p), at: T0 + VOUCH_EPOCH + p as u64, by: k(1), entry });
        }
        w.inp.now = T0 + VOUCH_EPOCH + 30 * DAY;
        w
    }

    #[test]
    fn newcomers_climb_the_ladder() {
        let w = grown();
        let st = w.run();
        assert_eq!(st.level(&k(1)), Level::Trusted, "owners are trusted");
        assert_eq!(st.level(&k(2)), Level::Established);
        assert_eq!(st.level(&k(4)), Level::Established, "admitted at day 11, established at day 21");
        assert_eq!(st.person(&k(4)).unwrap().vouchers.len(), 2);
        assert_eq!(st.level(&k(9)), Level::Newcomer);
    }

    #[test]
    fn vouches_from_the_not_yet_established_dont_count() {
        let mut w = World::new(1);
        w.vouch(1, 2, 1, T0 + 10);
        // 2 is a member, not established yet, when vouching for 5
        w.vouch(2, 5, 1, T0 + DAY);
        w.vouch(1, 5, 1, T0 + DAY + 1);
        w.inp.now = T0 + 2 * DAY;
        let st = w.run();
        assert_eq!(st.level(&k(2)), Level::Member);
        // with only the owner established, one vouch is enough (quorum clamps to who exists)
        assert_eq!(st.level(&k(5)), Level::Member);
        assert_eq!(st.person(&k(5)).unwrap().vouchers, vec![k(1)]);
    }

    #[test]
    fn voice_is_split_among_everyone_vouched_in_together() {
        let w = bigger();
        let st = w.run();
        assert_eq!(st.voice(&k(1)), VOICE, "founders speak with a full voice");
        assert_eq!(st.voice(&k(2)), VOICE / 2, "the owner vouched for two people that epoch");
        assert_eq!(st.voice(&k(4)), VOICE / 2, "two half voices don't make a whole one");
        assert_eq!(st.voice(&k(5)), VOICE / 3, "three in the next epoch: a third each");
        assert_eq!(st.person(&k(1)).unwrap().vouched_this_epoch, 3);
        // a spree: ten friends at once get a tenth each, and pass on no more than they hold
        let mut w = World::new(1);
        for p in 20..30 {
            w.vouch(1, p, 1, T0 + p as u64);
        }
        w.vouch(20, 40, 1, T0 + 20 * DAY);
        w.vouch(21, 40, 1, T0 + 20 * DAY + 1);
        w.inp.now = T0 + 40 * DAY;
        let st = w.run();
        assert_eq!(st.voice(&k(25)), VOICE / 10);
        assert_eq!(st.level(&k(40)), Level::Established, "friends still get in");
        assert_eq!(st.voice(&k(40)), VOICE / 10, "but a bloc of them can't lift anyone above what they hold");
    }

    #[test]
    fn hides_nap_records_and_block_people() {
        let mut w = bigger();
        let now = w.inp.now;
        let spam = w.post(9, now - DAY);
        // the quorum is capped at a third of the established voice (1 + 0.5·3 + ⅓·3 = 3.5)
        assert_eq!(w.run().quorums.hide, (VOICE * 7 / 2).div_ceil(3));
        // 5, 6 and 7 hold a third of a voice each: together one voice, not enough
        for r in [5, 6, 7] {
            w.review(r, spam, Verdict::Hide, now - DAY + 60);
        }
        let st = w.run();
        assert!(!st.hidden.contains_key(&spam), "{:?}", st.quorums);
        w.review(2, spam, Verdict::Hide, now - DAY + 61);
        let st = w.run();
        assert!(st.hidden.contains_key(&spam), "half a voice more tips it");
        assert!(st.blocked(&k(9)), "a newcomer with that much hiding is blocked");
        // a defended post is not hidden, and its hiders are noted
        let fine = w.post(4, now - DAY);
        w.review(2, fine, Verdict::Hide, now - DAY + 1);
        for r in [1, 3, 5, 6] {
            w.review(r, fine, Verdict::Ok, now - DAY + 2);
        }
        let st = w.run();
        assert!(!st.hidden.contains_key(&fine));
        assert_eq!(st.person(&k(2)).unwrap().overruled, 1);
        assert!(!st.blocked(&k(4)));
    }

    #[test]
    fn newcomers_cant_gang_up() {
        let mut w = grown();
        let p = w.post(4, T0 + 29 * DAY);
        for r in 50..60u8 {
            w.review(r, p, Verdict::Hide, T0 + 29 * DAY + r as u64);
        }
        let st = w.run();
        assert!(!st.hidden.contains_key(&p), "hides from newcomers count for nothing");
    }

    #[test]
    fn nobody_decides_alone() {
        let mut w = grown();
        let p = w.post(4, T0 + 29 * DAY);
        w.review(1, p, Verdict::Hide, T0 + 29 * DAY + 1);
        assert!(!w.run().hidden.contains_key(&p), "a full voice, but one person");
        w.review(2, p, Verdict::Hide, T0 + 29 * DAY + 2);
        assert!(w.run().hidden.contains_key(&p));
    }

    #[test]
    fn owners_cant_be_blocked_and_quorums_stay_reachable() {
        let mut w = grown();
        let p = w.post(1, T0 + 29 * DAY);
        for r in [2, 3, 4] {
            w.review(r, p, Verdict::Hide, T0 + 29 * DAY + 1);
        }
        w.ladder.hide_quorum = 100;
        w.ladder.veto_quorum = 100;
        let st = w.run();
        // established voice 1 + 0.5 + 0.5 + 0.5 = 2.5 → a third is under a voice → one voice
        assert_eq!(st.quorums.hide, VOICE);
        assert_eq!(st.quorums.veto, VOICE);
        assert!(st.hidden.contains_key(&p), "an owner's post can be hidden");
        assert!(!st.blocked(&k(1)), "but the owner isn't blocked");
    }

    #[test]
    fn vetoes_overturn_and_suspend() {
        let mut w = bigger();
        let m = k(8);
        w.lv.mods.insert(m, T0 + 100);
        let now = w.inp.now;
        let mut ids = Vec::new();
        for i in 0..3u64 {
            let id = w.id();
            w.lv.entries.push(EntryIn { id, created_at: now - 10 * DAY + i, signers: vec![m], target: Some(k(4)), vetoable: true });
            ids.push(id);
        }
        // the target (4) can't veto its own case; 2, 3, 5, 6, 7 can: 0.5 + 0.5 + ⅓·3 = 2 reaches
        // the cap of a third of the established voice (3.5 + the moderator's 1)
        for id in &ids {
            for r in [2, 3, 4, 5, 6, 7] {
                w.review(r, *id, Verdict::Veto, now - 9 * DAY);
            }
        }
        let st = w.run();
        for id in &ids {
            let t = &st.vetoes[id];
            assert!(t.overturned, "{t:?}");
            assert_eq!(t.vetoes, 5);
        }
        assert_eq!(st.suspended.get(&m), Some(&(now - 10 * DAY + 2)), "three strikes");
        assert_eq!(st.level(&m), Level::Newcomer, "a suspended moderator is no founder");
    }

    #[test]
    fn careless_vouching_costs_vouching_power() {
        let mut w = bigger();
        let t = w.inp.now;
        // 2 vouches two spammers in alone (quorum would need 2, so they stay newcomers),
        // 3 joins in for both: they get admitted
        for s in [40, 41] {
            w.vouch(2, s, 1, t);
            w.vouch(3, s, 1, t + 1);
        }
        w.inp.now = t + DAY;
        assert_eq!(w.run().level(&k(40)), Level::Member);
        // the owner and 4, 5, 6, 7 object: 1 + 0.5 + 1 = 2.5 voices against 2 and 3's 1
        for s in [40, 41] {
            for o in [1, 4, 5, 6, 7] {
                w.vouch(o, s, -1, t + 100);
            }
        }
        w.inp.now = t + 20 * DAY;
        let st = w.run();
        assert!(st.blocked(&k(40)) && st.blocked(&k(41)));
        assert!(st.person(&k(2)).unwrap().struck && st.person(&k(3)).unwrap().struck);
        assert_eq!(st.level(&k(4)), Level::Established, "people they vouched for earlier keep their place");
        // a later vouch from a struck voucher doesn't count
        w.vouch(2, 42, 1, t + 10 * DAY);
        w.vouch(5, 42, 1, t + 10 * DAY + 1);
        let st = w.run();
        assert_eq!(st.person(&k(42)).unwrap().vouchers, vec![k(5)]);
        assert_eq!(st.level(&k(42)), Level::Newcomer);
    }

    #[test]
    fn trust_needs_recent_contributions_and_good_response() {
        let mut w = grown();
        w.ladder.trusted_posts = 2;
        w.ladder.media_score = 3;
        let a = w.post(2, T0 + 25 * DAY);
        let b = w.post(2, T0 + 26 * DAY);
        assert_eq!(w.run().level(&k(2)), Level::Established, "no response yet");
        for (voter, target) in [(3, a), (4, a), (1, b)] {
            w.inp.votes.push(VoteIn { voter: k(voter), target, value: 1 });
        }
        assert_eq!(w.run().level(&k(2)), Level::Trusted);
        // and it fades when they stop contributing
        w.inp.now = T0 + 200 * DAY;
        assert_eq!(w.run().level(&k(2)), Level::Established);
    }

    #[test]
    fn changes_need_everyone() {
        let mut w = grown();
        let now = w.inp.now;
        // 2, 3, 4 are established and around
        for p in [2, 3, 4] {
            w.post(p, now - DAY);
        }
        let p = w.propose(1, now);
        w.inp.now = now + DAY;
        let st = w.run();
        let t = &st.proposals[&p];
        assert_eq!(t.status, ProposalStatus::Open);
        assert_eq!(t.electors.len(), 4);
        assert_eq!(t.agreed, vec![k(1)], "the proposer agrees");
        assert_eq!(t.waiting.len(), 3);
        for (i, x) in [2u8, 3].iter().enumerate() {
            w.review(*x, p, Verdict::Ok, now + 2 * DAY + i as u64);
        }
        w.inp.now = now + 3 * DAY;
        assert_eq!(w.run().proposals[&p].status, ProposalStatus::Open, "4 is around and hasn't answered");
        w.review(4, p, Verdict::Ok, now + 4 * DAY);
        w.inp.now = now + 5 * DAY;
        assert_eq!(w.run().proposals[&p].status, ProposalStatus::Passed { at: now + 4 * DAY, unanimous: true });
        // taking it back later changes nothing: it already passed
        w.review(4, p, Verdict::Veto, now + 6 * DAY);
        w.inp.now = now + 7 * DAY;
        assert_eq!(w.run().proposals[&p].status, ProposalStatus::Passed { at: now + 4 * DAY, unanimous: true });
    }

    #[test]
    fn people_who_are_away_dont_hold_it_up_for_good() {
        let mut w = grown();
        let now = w.inp.now;
        let mine = w.post(4, now - DAY);
        let p = w.propose(1, now);
        w.review(2, p, Verdict::Ok, now + 10);
        w.review(3, p, Verdict::Ok, now + 11);
        w.inp.now = now + DAY;
        assert_eq!(w.run().proposals[&p].status, ProposalStatus::Open, "4 is around");
        // 4's last post fades: 4 is away, the change is in effect
        w.inp.contributions.retain(|c| c.id != mine);
        let st = w.run();
        assert_eq!(st.proposals[&p].status, ProposalStatus::InEffect);
        assert_eq!(st.proposals[&p].away, vec![k(4)]);
        // 4 comes back and objects before the deadline: it stops
        w.post(4, now + 2 * DAY);
        w.review(4, p, Verdict::Veto, now + 2 * DAY + 1);
        w.inp.now = now + 3 * DAY;
        assert_eq!(w.run().proposals[&p].status, ProposalStatus::Open);
        // and at the deadline the objection (half a voice) makes it lapse
        w.inp.now = now + 181 * DAY;
        assert_eq!(w.run().proposals[&p].status, ProposalStatus::Lapsed);
    }

    #[test]
    fn a_retired_elector_stops_holding_a_change_up_after_the_notice() {
        let mut w = grown();
        let now = w.inp.now;
        // 4 last signed something 40 days before the retirement
        w.inp.last_seen.insert(k(4), now - 10 * DAY);
        let p = w.propose(1, now);
        w.review(2, p, Verdict::Ok, now + 10);
        w.review(3, p, Verdict::Ok, now + 11);
        w.inp.now = now + 30 * DAY;
        let st = w.run();
        assert_eq!(st.proposals[&p].status, ProposalStatus::InEffect, "4 is away: in effect, not final");
        w.lv.retired.push((k(4), now + 30 * DAY));
        w.inp.now = now + 30 * DAY + RETIRE_NOTICE - 1;
        let st = w.run();
        assert_eq!(st.proposals[&p].status, ProposalStatus::InEffect, "the notice hasn't passed");
        assert!(st.proposals[&p].retired.is_empty());
        w.inp.now = now + 40 * DAY;
        let st = w.run();
        let t = &st.proposals[&p];
        assert_eq!(t.status, ProposalStatus::Passed { at: now + 30 * DAY + RETIRE_NOTICE, unanimous: true });
        assert_eq!(t.retired, vec![k(4)]);
        assert!(t.away.is_empty());
    }

    #[test]
    fn a_retirement_lapses_when_the_person_comes_back_and_never_takes_an_answer() {
        let mut w = grown();
        let now = w.inp.now;
        w.inp.last_seen.insert(k(4), now - 10 * DAY);
        let p = w.propose(1, now);
        w.review(2, p, Verdict::Ok, now + 10);
        w.review(3, p, Verdict::Ok, now + 11);
        w.lv.retired.push((k(4), now + 30 * DAY));
        // 4 signs something during the notice: back, the retirement is void
        w.inp.last_seen.insert(k(4), now + 31 * DAY);
        w.inp.now = now + 40 * DAY;
        assert_eq!(w.run().proposals[&p].status, ProposalStatus::InEffect);
        assert!(w.run().proposals[&p].retired.is_empty());

        // someone who objected and then left keeps their objection
        let mut w = grown();
        let now = w.inp.now;
        let p = w.propose(1, now);
        w.review(2, p, Verdict::Ok, now + 10);
        w.review(4, p, Verdict::Veto, now + 11);
        w.inp.last_seen.insert(k(4), now + 11);
        w.lv.retired.push((k(4), now + 50 * DAY));
        w.inp.now = now + 181 * DAY;
        assert_eq!(w.run().proposals[&p].status, ProposalStatus::Lapsed);
    }

    #[test]
    fn nobody_is_retired_who_was_here_last_month() {
        let mut w = grown();
        let now = w.inp.now;
        w.inp.last_seen.insert(k(4), now + 10 * DAY);
        let p = w.propose(1, now);
        w.review(2, p, Verdict::Ok, now + 10);
        w.review(3, p, Verdict::Ok, now + 11);
        // only 20 days of silence before the entry
        w.lv.retired.push((k(4), now + 30 * DAY));
        w.inp.now = now + 60 * DAY;
        let st = w.run();
        assert_eq!(st.proposals[&p].status, ProposalStatus::InEffect);
        assert!(st.proposals[&p].retired.is_empty());
        // owners are never retired
        let mut w = grown();
        let now = w.inp.now;
        w.lv.retired.push((k(1), now));
        let p = w.propose(1, now);
        w.inp.now = now + 60 * DAY;
        assert!(w.run().proposals[&p].retired.is_empty());
    }

    #[test]
    fn silence_until_the_deadline_is_consent_and_answers_after_it_dont_count() {
        let mut w = grown();
        let now = w.inp.now;
        w.post(4, now + 170 * DAY);
        let p = w.propose(1, now);
        w.inp.now = now + 179 * DAY;
        assert_eq!(w.run().proposals[&p].status, ProposalStatus::Open);
        w.inp.now = now + 180 * DAY;
        w.review(4, p, Verdict::Veto, now + 180 * DAY);
        assert_eq!(w.run().proposals[&p].status, ProposalStatus::Passed { at: now + 180 * DAY, unanimous: false });
    }

    #[test]
    fn friends_vouched_in_after_a_proposal_dont_decide_it() {
        let mut w = grown();
        let now = w.inp.now;
        let p = w.propose(1, now);
        for x in [2, 3] {
            w.review(x, p, Verdict::Ok, now + 1);
        }
        // 4 brings in ten friends right after: they become established later, and object
        for f in 60..70u8 {
            w.vouch(4, f, 1, now + 2);
            w.vouch(2, f, 1, now + 3);
            w.post(f, now + 20 * DAY);
            w.review(f, p, Verdict::Veto, now + 21 * DAY);
        }
        w.review(4, p, Verdict::Ok, now + 22 * DAY);
        w.inp.now = now + 23 * DAY;
        let st = w.run();
        assert_eq!(st.level(&k(60)), Level::Established);
        assert!(!st.proposals[&p].electors.contains(&k(60)));
        assert_eq!(st.proposals[&p].status, ProposalStatus::Passed { at: now + 22 * DAY, unanimous: true });
    }

    #[test]
    fn a_sliver_of_voice_cant_block_alone() {
        // six friends vouched in together hold a sixth of a voice each
        let run = |objectors: &[u8]| {
            let mut w = World::new(1);
            for p in 20..26 {
                w.vouch(1, p, 1, T0 + p as u64);
            }
            let now = T0 + 20 * DAY;
            for f in 20..26 {
                w.post(f, now - DAY);
            }
            let p = w.propose(1, now);
            for f in 20..26u8 {
                w.review(f, p, if objectors.contains(&f) { Verdict::Veto } else { Verdict::Ok }, now + 1);
            }
            w.inp.now = now + DAY;
            w.run().proposals[&p].status
        };
        assert!(matches!(run(&[20]), ProposalStatus::Passed { unanimous: true, .. }));
        assert!(matches!(run(&[20, 21]), ProposalStatus::Passed { unanimous: true, .. }), "a third of a voice");
        assert_eq!(run(&[20, 21, 22]), ProposalStatus::Open, "three of them do weigh half a voice");
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
        #[test]
        fn random_communities_settle(
            vouches in proptest::collection::vec((1u8..13, 1u8..13, -1i64..2, 0u64..40), 0..60),
            reviews in proptest::collection::vec((1u8..13, 0usize..20, 0u8..3), 0..80),
        ) {
            let mut w = World::new(1);
            w.inp.now = T0 + 40 * DAY;
            for (a, b, v, d) in vouches {
                if a != b {
                    w.vouch(a, b, v, T0 + d * DAY);
                }
            }
            let posts: Vec<Id> = (0..20u8).map(|i| w.post(1 + i % 12, T0 + 35 * DAY)).collect();
            for (r, p, v) in reviews {
                let verdict = [Verdict::Ok, Verdict::Hide, Verdict::None][v as usize];
                w.review(r, posts[p], verdict, T0 + 36 * DAY);
            }
            let (a, pa) = settle_n(&w, 12);
            let (b, pb) = settle_n(&w, 13);
            proptest::prop_assert_eq!(&pa, &pb, "settles instead of oscillating");
            proptest::prop_assert_eq!(&a.people, &b.people);
            w.inp.vouches.reverse();
            w.inp.reviews.reverse();
            let (c, _) = settle_n(&w, 12);
            proptest::prop_assert_eq!(&a.people, &c.people, "order of arrival doesn't matter");
            proptest::prop_assert_eq!(&a.hidden, &c.hidden);
            for p in a.people.values() {
                proptest::prop_assert!(p.voice <= VOICE);
            }
        }
    }

    #[test]
    fn same_records_same_ladder() {
        let mut w = grown();
        let p = w.post(4, T0 + 29 * DAY);
        w.review(2, p, Verdict::Hide, T0 + 29 * DAY);
        let a = w.run();
        w.inp.vouches.reverse();
        w.inp.reviews.reverse();
        w.inp.contributions.reverse();
        let b = w.run();
        assert_eq!(a.people, b.people);
        assert_eq!(a.hidden, b.hidden);
    }
}
