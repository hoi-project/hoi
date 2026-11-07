//! The trust ladder and consent with hundreds of members and adversarial vouching.
//! A model, like [`crate::life`]: no nodes and no
//! network, only the core's own arithmetic. Every week the community's records so far go through
//! `hoi_moderation::ladder::settle`, the function every device runs, and the outcome is scored.
//!
//! Honest people arrive, are vouched in by members who know them, post, go away for a while or
//! for good; established members hide spam they come across and defend members they vouched for.
//! The owner proposes a change every month, which honest electors agree to or leave unanswered.
//!
//! A party plays one strategy against them:
//!
//! - **ring**: Sybils vouch only for each other. Nobody established ever vouched for them.
//! - **turncoat**: two established members turn together (one alone can't admit anyone: the
//!   ladder wants two established vouchers). From then on each vouches a batch of Sybils in every
//!   quarter, the other countersigning, and every established Sybil does the same; the ring hides
//!   honest posts, tries to block an honest member, and objects to every proposal. Its Sybils also
//!   post spam.
//! - **patient**: Sybils that behave. Each talks one honest member into a vouch now and then and
//!   they vouch for each other; they post well and wait half a year, then all strike at once.
//!
//! Scored: the party's share of established voice over time, against the bound the ladder is
//! built on (a voice given away at most once a quarter, nobody above their strongest voucher: a
//! party's voice can at most double a quarter, plus what honest vouches hand it); honest members
//! blocked and honest posts hidden; the owner's proposals passed, stalled and lapsed.

use std::collections::{BTreeMap, BTreeSet};

use hoi_moderation::ladder::{self as lad, Contribution, Inputs, LedgerView, ProposalIn, ProposalStatus, ReviewIn, VouchIn};
use hoi_types::{Id, Ladder, Verdict};

const DAY: u64 = 86_400;
/// The start of a vouch epoch, so quarters line up with the ladder's.
const T0: u64 = 231 * lad::VOUCH_EPOCH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    None,
    Ring,
    Turncoat,
    Patient,
}

impl Strategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Strategy::None => "none",
            Strategy::Ring => "ring",
            Strategy::Turncoat => "turncoat",
            Strategy::Patient => "patient",
        }
    }
    pub fn all() -> [Strategy; 4] {
        [Strategy::None, Strategy::Ring, Strategy::Turncoat, Strategy::Patient]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Honest people the community grows to.
    pub honest: usize,
    pub days: u32,
    pub strategy: Strategy,
    /// Sybils each attacker vouches in per quarter (turncoat), or tries to bring in (patient).
    pub batch: usize,
    /// Chance per day that an established honest member hides a spam post they come across.
    pub detect: f64,
    /// Chance per quarter that a patient Sybil talks an honest member into a vouch.
    pub social: f64,
    pub seed: u64,
}

impl Config {
    pub fn new(honest: usize, strategy: Strategy, seed: u64) -> Config {
        Config { honest, days: 360, strategy, batch: 4, detect: 0.2, social: 0.15, seed }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub honest_admitted: usize,
    pub party_admitted: usize,
    /// The party's share of established voice: the largest seen, and at the end.
    pub max_share: f64,
    pub end_share: f64,
    /// The party's voice at each quarter's end, in voices.
    pub party_voice: Vec<f64>,
    /// Quarters in which the party's voice more than doubled beyond what honest vouches gave it.
    pub over_doubling: usize,
    pub honest_blocked: usize,
    pub party_blocked: usize,
    pub honest_hidden: usize,
    pub spam_hidden: usize,
    pub spam_posted: usize,
    pub proposals: usize,
    pub passed: usize,
    pub lapsed: usize,
    pub open: usize,
    /// Electors of a proposal who were vouched in after it was made (must stay zero).
    pub late_electors: usize,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

#[derive(Debug, Clone)]
struct Person {
    id: Id,
    honest: bool,
    joined: u64,
    /// Gone for a while (comes back) or for good.
    away_until: u64,
    gone: bool,
    /// When this person last vouched in a batch (turncoat and its Sybils).
    last_batch: Option<u64>,
}

struct World {
    cfg: Config,
    rng: Rng,
    people: Vec<Person>,
    inp: Inputs,
    lv: LedgerView,
    ladder: Ladder,
    next: u64,
    /// Posts: id → (author, spam).
    posts: BTreeMap<Id, (Id, bool)>,
    /// Who vouched for whom (honest defenders).
    vouched: BTreeMap<Id, Vec<Id>>,
    turned: Vec<Id>,
    party_from_honest: u64,
    proposals: Vec<(Id, u64)>,
    standing: lad::Standing,
    /// (reviewer, target) pairs answered already.
    reviewed: BTreeSet<(Id, Id)>,
}

/// The party grows to at most this many Sybils.
const MAX_SYBILS: usize = 400;

impl World {
    fn id(&mut self) -> Id {
        self.next += 1;
        Id::hash(&self.next.to_le_bytes())
    }

    fn now(&self) -> u64 {
        self.inp.now
    }

    fn person(&mut self, honest: bool) -> Id {
        let id = self.id();
        let now = self.now();
        self.people.push(Person { id, honest, joined: now, away_until: 0, gone: false, last_batch: None });
        id
    }

    fn vouch(&mut self, from: Id, to: Id) {
        let id = self.id();
        let now = self.now();
        self.inp.vouches.push(VouchIn { id, voucher: from, subject: to, value: 1, seq: now, created_at: now });
        self.vouched.entry(to).or_default().push(from);
    }

    fn post(&mut self, by: Id, spam: bool) -> Id {
        let id = self.id();
        let now = self.now();
        self.inp.contributions.push(Contribution { id, author: by, created_at: now });
        self.inp.authors.insert(id, by);
        self.posts.insert(id, (by, spam));
        id
    }

    fn review(&mut self, by: Id, target: Id, verdict: Verdict) {
        if !self.reviewed.insert((by, target)) {
            return;
        }
        let id = self.id();
        let now = self.now();
        let reason = matches!(verdict, Verdict::Hide | Verdict::Veto).then(|| "spam".to_string());
        self.inp.reviews.push(ReviewIn { id, reviewer: by, target, verdict, reason, seq: now, created_at: now });
    }

    fn established(&self, p: &Id) -> bool {
        self.standing.level(p) >= lad::Level::Established || self.lv.owners.contains(p)
    }

    fn here(&self, p: &Person) -> bool {
        !p.gone && p.away_until <= self.now()
    }

    fn party(&self, p: &Id) -> bool {
        self.turned.contains(p) || self.people.iter().any(|x| x.id == *p && !x.honest)
    }

    fn settle(&mut self) {
        // what has faded is gone from every device
        let window = self.lv.retention;
        let now = self.now();
        self.inp.contributions.retain(|c| c.created_at + window > now);
        let lv = {
            let mut lv = self.lv.clone();
            lv.ladder = self.ladder.clone();
            lv
        };
        let (st, _, _) = lad::settle(&self.inp, &mut |_| lv.clone(), 6);
        self.standing = st;
    }

    fn party_voice(&self) -> u64 {
        self.standing.people.iter().filter(|(p, x)| self.party(p) && x.level() >= lad::Level::Established && !x.blocked).map(|(_, x)| x.voice).sum()
    }
}

/// One community over `cfg.days`.
pub fn run(cfg: Config) -> Outcome {
    let mut w = World {
        cfg,
        rng: Rng(cfg.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1),
        people: Vec::new(),
        inp: Inputs { now: T0, ..Default::default() },
        lv: LedgerView { genesis_at: T0, retention: 90 * DAY, ..Default::default() },
        ladder: Ladder { admit_vouches: 2, established_age: 14 * DAY, ..Ladder::default() },
        next: 0,
        posts: BTreeMap::new(),
        vouched: BTreeMap::new(),
        turned: Vec::new(),
        party_from_honest: 0,
        proposals: Vec::new(),
        standing: lad::Standing::default(),
        reviewed: BTreeSet::new(),
    };
    let owner = w.person(true);
    w.lv.owners.insert(owner);
    let mut out = Outcome::default();
    let mut quarter_voice = vec![0u64];
    let mut honest_gift: u64 = 0;

    for day in 1..=cfg.days as u64 {
        w.inp.now = T0 + day * DAY;
        let now = w.now();
        honest_day(&mut w, &mut out);
        party_day(&mut w, day, &mut out, &mut honest_gift);
        // the owner proposes a change every month; honest electors answer within the week, or not
        if day % 30 == 15 {
            let id = w.id();
            w.lv.proposals.push(ProposalIn { id, at: now, signers: vec![owner], ladder: w.ladder.clone(), deadline: now + w.lv.retention });
            w.proposals.push((id, now));
        }
        if day % 7 == 0 {
            w.settle();
            answer_proposals(&mut w);
            let est = w.standing.established_voice.max(1);
            let share = w.party_voice() as f64 / est as f64;
            out.max_share = out.max_share.max(share);
            out.end_share = share;
        }
        if (now - T0).is_multiple_of(lad::VOUCH_EPOCH) {
            w.settle();
            quarter_voice.push(w.party_voice());
            let (before, after) = (quarter_voice[quarter_voice.len() - 2], *quarter_voice.last().unwrap());
            // what the party's own vouches can add: its voice again, plus what honest people gave
            if after > (2 * before).max(lad::VOICE) + std::mem::take(&mut honest_gift) {
                out.over_doubling += 1;
            }
        }
    }
    w.settle();
    let st = &w.standing;
    out.party_voice = quarter_voice.iter().map(|v| *v as f64 / lad::VOICE as f64).collect();
    for p in &w.people {
        let level = st.level(&p.id);
        let party = w.party(&p.id);
        if level >= lad::Level::Member {
            if party { out.party_admitted += 1 } else { out.honest_admitted += 1 }
        }
        if st.blocked(&p.id) {
            if party { out.party_blocked += 1 } else { out.honest_blocked += 1 }
        }
    }
    for (post, (author, spam)) in &w.posts {
        if st.hidden.contains_key(post) {
            if *spam {
                out.spam_hidden += 1
            } else if !w.party(author) {
                out.honest_hidden += 1
            }
        }
    }
    out.spam_posted = w.posts.values().filter(|(_, s)| *s).count();
    for (id, at) in &w.proposals {
        let Some(t) = st.proposals.get(id) else { continue };
        out.proposals += 1;
        match t.status {
            ProposalStatus::Passed { .. } => out.passed += 1,
            ProposalStatus::Lapsed => out.lapsed += 1,
            _ => out.open += 1,
        }
        // electors are fixed when it was proposed: nobody vouched in later decides it
        out.late_electors += t.electors.iter().filter(|e| w.people.iter().any(|p| p.id == **e && p.joined > *at)).count();
    }
    out
}

/// Honest life: arrivals vouched in by people who know them, posts, time away, leaving, hiding
/// spam and defending members.
fn honest_day(w: &mut World, out: &mut Outcome) {
    let now = w.now();
    let honest_count = w.people.iter().filter(|p| p.honest).count();
    // growth: about a newcomer a day until the community is full
    if honest_count < w.cfg.honest && w.rng.chance(0.9) {
        let newbie = w.person(true);
        let vouchers: Vec<Id> = w.people.iter().filter(|p| p.honest && w.here(p) && w.established(&p.id)).map(|p| p.id).collect();
        let mut picked = BTreeSet::new();
        for _ in 0..4 {
            if picked.len() >= 2 || vouchers.is_empty() {
                break;
            }
            picked.insert(vouchers[w.rng.below(vouchers.len())]);
        }
        for v in picked {
            w.vouch(v, newbie);
        }
    }
    let honest: Vec<usize> = (0..w.people.len()).filter(|&i| w.people[i].honest).collect();
    for i in honest {
        if !w.here(&w.people[i]) {
            continue;
        }
        let id = w.people[i].id;
        // a member goes away for a while now and then, a few for good
        if w.rng.chance(0.004) {
            w.people[i].away_until = now + (20 + w.rng.below(60) as u64) * DAY;
            continue;
        }
        if w.people[i].id != w.lv.owners.iter().next().copied().unwrap_or(id) && w.rng.chance(0.0008) {
            w.people[i].gone = true;
            continue;
        }
        if w.rng.chance(0.3) {
            w.post(id, false);
        }
        if !w.established(&id) {
            continue;
        }
        // established members come across a recent post: hide spam, defend what is good
        if w.rng.chance(w.cfg.detect) && !w.posts.is_empty() {
            let recent: Vec<(Id, Id, bool)> =
                w.inp.contributions.iter().rev().take(60).filter_map(|c| w.posts.get(&c.id).map(|(a, s)| (c.id, *a, *s))).collect();
            if !recent.is_empty() {
                let (post, author, spam) = recent[w.rng.below(recent.len())];
                if author != id {
                    w.review(id, post, if spam { Verdict::Hide } else { Verdict::Ok });
                }
            }
        }
    }
    let _ = out;
}

/// The party's day, by strategy.
fn party_day(w: &mut World, day: u64, out: &mut Outcome, honest_gift: &mut u64) {
    let now = w.now();
    let quarter_start = (now - T0).is_multiple_of(lad::VOUCH_EPOCH) || day == 1;
    match w.cfg.strategy {
        Strategy::None => {}
        Strategy::Ring => {
            // a ring that vouches only for itself, posting spam
            if day == 1 {
                let ring: Vec<Id> = (0..w.cfg.batch * 4).map(|_| w.person(false)).collect();
                for a in &ring {
                    for b in &ring {
                        if a != b {
                            w.vouch(*a, *b);
                        }
                    }
                }
            }
            spam_and_strike(w, out, true);
        }
        Strategy::Turncoat => {
            if w.turned.is_empty() && day >= 60 {
                // the two longest-standing established honest members turn together
                let t: Vec<Id> = w.people.iter().skip(1).filter(|p| p.honest && w.established(&p.id)).take(2).map(|p| p.id).collect();
                if t.len() == 2 {
                    for p in w.people.iter_mut().filter(|p| t.contains(&p.id)) {
                        p.honest = false;
                    }
                    w.turned = t;
                }
            }
            if !w.turned.is_empty() {
                // every established member of the party vouches a batch in, once a quarter
                let recruiters: Vec<usize> = (0..w.people.len())
                    .filter(|&i| !w.people[i].honest && w.established(&w.people[i].id) && !w.standing.blocked(&w.people[i].id))
                    .filter(|&i| w.people[i].last_batch.is_none_or(|t| (t - T0) / lad::VOUCH_EPOCH < (now - T0) / lad::VOUCH_EPOCH))
                    .collect();
                for i in recruiters {
                    if w.people.iter().filter(|p| !p.honest).count() >= MAX_SYBILS {
                        break;
                    }
                    w.people[i].last_batch = Some(now);
                    let from = w.people[i].id;
                    for _ in 0..w.cfg.batch {
                        let s = w.person(false);
                        w.vouch(from, s);
                        // the second vouch admission needs: another member of the party
                        let other: Vec<Id> = w.people.iter().filter(|p| !p.honest && p.id != from && w.established(&p.id)).map(|p| p.id).collect();
                        if !other.is_empty() {
                            let o = other[w.rng.below(other.len())];
                            w.vouch(o, s);
                        }
                    }
                }
                spam_and_strike(w, out, true);
            }
        }
        Strategy::Patient => {
            if day == 1 {
                for _ in 0..w.cfg.batch * 3 {
                    w.person(false);
                }
            }
            let sybils: Vec<Id> = w.people.iter().filter(|p| !p.honest).map(|p| p.id).collect();
            if quarter_start {
                // each Sybil may talk one honest established member into a vouch; they vouch for each other
                let honest: Vec<Id> = w.people.iter().filter(|p| p.honest && w.established(&p.id)).map(|p| p.id).collect();
                for s in &sybils {
                    if !honest.is_empty() && w.rng.chance(w.cfg.social) {
                        let h = honest[w.rng.below(honest.len())];
                        let before = w.standing.voice(&h);
                        w.vouch(h, *s);
                        *honest_gift += before;
                        w.party_from_honest += 1;
                    }
                    if w.established(s) {
                        for t in &sybils {
                            if t != s && !w.established(t) {
                                w.vouch(*s, *t);
                            }
                        }
                    }
                }
            }
            // behave: post well
            for s in &sybils {
                if w.rng.chance(0.3) {
                    w.post(*s, false);
                }
            }
            if day >= 180 {
                spam_and_strike(w, out, false);
            }
        }
    }
}

/// What an established party does: post spam (if `spam`), hide honest posts, try to block one
/// honest member, object to every proposal.
fn spam_and_strike(w: &mut World, out: &mut Outcome, spam: bool) {
    let party: Vec<Id> = w.people.iter().filter(|p| !p.honest).map(|p| p.id).collect();
    if spam {
        for p in &party {
            if w.rng.chance(0.2) {
                w.post(*p, true);
            }
        }
    }
    let established: Vec<Id> = party.iter().filter(|p| w.established(p) && !w.standing.blocked(p)).copied().collect();
    if established.is_empty() {
        return;
    }
    // the target: the most recent honest poster who isn't the owner
    let target =
        w.inp.contributions.iter().rev().filter_map(|c| w.posts.get(&c.id).map(|(a, _)| (c.id, *a))).find(|(_, a)| !w.party(a) && !w.lv.owners.contains(a));
    if let Some((post, _)) = target {
        let hiders = established.clone();
        for h in hiders {
            w.review(h, post, Verdict::Hide);
        }
    }
    let now = w.now();
    let open: Vec<Id> = w.proposals.iter().filter(|(_, at)| *at + w.lv.retention > now).map(|(id, _)| *id).collect();
    for p in open {
        for h in &established {
            w.review(*h, p, Verdict::Veto);
        }
    }
    let _ = out;
}

/// Honest electors agree with most proposals within a week, and leave some unanswered.
fn answer_proposals(w: &mut World) {
    let now = w.now();
    let props: Vec<(Id, u64)> = w.proposals.iter().filter(|(_, at)| *at + 7 * DAY >= now && *at <= now).copied().collect();
    for (p, _) in props {
        let Some(t) = w.standing.proposals.get(&p).cloned() else { continue };
        for e in t.electors {
            let honest = w.people.iter().any(|x| x.id == e && x.honest && w.here(x));
            if honest && !w.reviewed.contains(&(e, p)) && w.rng.chance(0.85) {
                w.review(e, p, Verdict::Ok);
            }
        }
    }
}

/// Every strategy at two community sizes.
pub fn report(seed: u64, days: u32) -> Vec<String> {
    let mut lines = vec![
        format!("hoi-sim ladder · {days} days · settled weekly with hoi_moderation::ladder::settle · seed {seed}"),
        "strategy   honest  admitted(h/p)  share max/end  quarters over 2×  blocked(h/p)  hidden honest  spam hidden  proposals passed/lapsed/open  late electors".to_string(),
    ];
    for honest in [100, 300] {
        for s in Strategy::all() {
            let o = run(Config { days, ..Config::new(honest, s, seed) });
            let share = format!("{:.1}%/{:.1}%", o.max_share * 100.0, o.end_share * 100.0);
            lines.push(format!(
                "{:<10} {:>6}  {:>6}/{:<6}  {:>13}  {:>16}  {:>5}/{:<6}  {:>13}  {:>5}/{:<5}  {:>9}/{}/{}  {:>13}",
                s.as_str(),
                honest,
                o.honest_admitted,
                o.party_admitted,
                share,
                o.over_doubling,
                o.honest_blocked,
                o.party_blocked,
                o.honest_hidden,
                o.spam_hidden,
                o.spam_posted,
                o.passed,
                o.lapsed,
                o.open,
                o.late_electors
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quick(s: Strategy) -> Outcome {
        run(Config { days: 200, ..Config::new(80, s, 7) })
    }

    #[test]
    fn a_ring_nobody_vouched_for_gets_nothing() {
        let o = quick(Strategy::Ring);
        assert_eq!(o.party_admitted, 0);
        assert_eq!(o.max_share, 0.0);
        assert_eq!(o.honest_hidden, 0);
        assert!(o.honest_admitted > 50);
    }

    #[test]
    fn a_turncoat_ring_at_most_doubles_a_quarter_and_electors_stay_fixed() {
        let o = quick(Strategy::Turncoat);
        assert!(o.party_admitted > 0, "the turncoat's batch got in");
        assert_eq!(o.over_doubling, 0, "voice per quarter: {:?}", o.party_voice);
        assert_eq!(o.late_electors, 0);
        assert_eq!(o.honest_blocked, 0, "{o:?}");
    }

    #[test]
    fn patient_sybils_stay_within_what_they_were_given() {
        let o = quick(Strategy::Patient);
        assert_eq!(o.over_doubling, 0, "voice per quarter: {:?}", o.party_voice);
        assert_eq!(o.late_electors, 0);
    }

    #[test]
    fn without_a_party_changes_pass() {
        let o = quick(Strategy::None);
        assert!(o.proposals >= 5);
        assert_eq!(o.lapsed, 0, "{o:?}");
        assert_eq!(o.honest_blocked + o.honest_hidden, 0);
    }
}
