//! The battle: real `hoi-core` nodes in one process, around a hub, with honest members and
//! adversaries, run over accelerated time. Everything honest goes through the ordinary node API
//! (so the core computes standing, not us); everything hostile is injected as raw wire over the
//! sync protocol (so the core's pipeline judges it, not us). At the end we read the honest nodes
//! and score the protocol's invariants (report.rs).
//!
//! Adversarial vouching, floods and governance capture, shrunk to run in a second so it can
//! live in CI and on a laptop.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hoi_core::dto::{CommunityDraft, LadderPatch, LimitsPatch, PostDraft};
use hoi_core::{Node, Role};
use hoi_sync::Msg;
use hoi_types::{Body, Id, Verdict};

use crate::forge::{self, Expect, Forger};
use crate::report::Scorecard;

const T0: u64 = 1_790_000_000;
const DAY: u64 = 86_400;
/// Cheap but real: ~instant stamps, short "established" wait, so a full siege runs in a second.
const POW_BITS: u32 = 8;

/// Who a participant is, for the scoreboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Founder,
    Honest,
    Turncoat,
    Sybil,
    Outsider,
}

pub struct Battle {
    clock: Arc<AtomicU64>,
    /// The hub relays between everyone, like a session's `hoi-node` (TECHNICAL.md §4).
    hub: Node,
    /// Honest nodes by peer name ("founder", "h0", …). Full nodes driven through the API.
    honest: BTreeMap<String, Node>,
    /// Adversaries are raw signers (no node); they speak the wire protocol straight at the hub.
    attackers: Vec<Forger>,
    community: Id,
    community_hex: String,
    pow_bits: u32,
    /// Churn: the share of honest members (founder aside) offline at each settling, in percent,
    /// a different share each time; and who is offline now.
    churn: u64,
    offline: std::collections::BTreeSet<String>,
    round: u64,
}

impl Battle {
    fn now(&self) -> u64 {
        self.clock.load(Ordering::SeqCst)
    }

    /// The hub's current pairing link. Each one pairs a single device, so everyone who joins the
    /// session, attackers included, is handed a fresh one: this hub pairs anyone.
    fn pairing(&self) -> String {
        self.hub.net_status().serving.expect("the hub serves").pairing
    }

    fn link_token(&self) -> String {
        hoi_types::uri::Pair::parse(&self.pairing()).expect("a pairing link").token
    }

    fn advance(&mut self, secs: u64) {
        self.clock.fetch_add(secs, Ordering::SeqCst);
        let now = self.now();
        self.hub.sweep();
        for n in self.honest.values_mut() {
            let _ = now;
            n.sweep();
        }
        self.settle();
    }

    /// Churn: who is offline for this settling. Those leaving drop their link to the hub; those
    /// coming back dial it again, and catch up as any device does.
    fn churn(&mut self) {
        self.round += 1;
        let now: std::collections::BTreeSet<String> = if self.churn == 0 {
            Default::default()
        } else {
            self.honest
                .keys()
                .filter(|n| n.as_str() != "founder")
                .filter(|n| {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    (n.as_str(), self.round).hash(&mut h);
                    h.finish() % 100 < self.churn
                })
                .cloned()
                .collect()
        };
        for gone in now.difference(&self.offline) {
            self.hub.net_disconnect(gone);
            if let Some(n) = self.honest.get_mut(gone) {
                n.net_disconnect("hub");
            }
        }
        // A home node keeps credentials for its 32 most recent devices; one forgotten while
        // away pairs again, as its person would scan the code again. One at a time: each link
        // pairs a single device.
        let back: Vec<String> = self.offline.difference(&now).cloned().collect();
        for name in back {
            let link = self.pairing();
            if let Some(n) = self.honest.get_mut(&name) {
                let _ = n.pair(&link);
                n.net_connect("hub", Role::Home);
                self.hub.net_connect(&name, Role::Client);
                for (_, m) in n.take_outbox() {
                    self.hub.net_receive(&name, &m);
                }
            }
        }
        self.offline = now;
    }

    /// Delivers every queued message until the mesh goes quiet (star topology: honest ⇄ hub).
    fn settle(&mut self) {
        self.churn();
        for _ in 0..200 {
            let mut moved = false;
            // honest → hub
            for (name, n) in self.honest.iter_mut() {
                if self.offline.contains(name) {
                    continue;
                }
                for (_, m) in n.take_outbox() {
                    self.hub.net_receive(name, &m);
                    moved = true;
                }
            }
            // hub → honest
            let out = self.hub.take_outbox();
            for (peer, m) in out {
                if self.offline.contains(&peer) {
                    continue;
                }
                if let Some(n) = self.honest.get_mut(&peer) {
                    n.net_receive("hub", &m);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        self.hub.commit().ok();
        for n in self.honest.values_mut() {
            n.commit().ok();
        }
    }

    /// Opens a fresh honest node paired to the hub and joined to the community.
    fn add_honest(&mut self, name: &str, display: &str, invite: &str) {
        let clock = self.clock.clone();
        let mut n = Node::open(Box::new(hoi_store::Ephemeral), "desktop", crate::WEB_BASE, Box::new(move || clock.load(Ordering::SeqCst))).unwrap();
        n.set_identity_bits(4);
        n.create_identity(display).unwrap();
        n.join(invite).unwrap();
        n.pair(&self.pairing()).unwrap();
        n.net_connect("hub", Role::Home);
        self.hub.net_connect(name, Role::Client);
        self.honest.insert(name.to_string(), n);
        self.settle();
    }

    /// Feeds raw wire from an attacker to the hub, as if their tool dialed in and spoke sync.
    fn inject(&mut self, atk: usize, wires: Vec<Vec<u8>>) {
        let peer = format!("atk{atk}");
        let token = self.link_token();
        let f = &self.attackers[atk];
        self.hub.net_connect(&peer, Role::Client);
        let hello = Msg::Hello {
            v: hoi_sync::SYNC_VERSION,
            node: f.root_key(),
            name: Some("mallory".into()),
            token: Some(token),
            communities: vec![self.community],
            salt: None,
            hashed: Vec::new(),
            serves: false,
            hints: Vec::new(),
            routes: Vec::new(),
            features: Vec::new(),
            pairing: None,
            link_key: None,
            notice_hash: None,
            notice: None,
            holder: None,
            forwards: false,
            rebuilds: false,
        };
        self.hub.net_receive(&peer, &hello.encode());
        for chunk in wires.chunks(hoi_sync::MAX_RECS) {
            let keep = self.hub.net_receive(&peer, &Msg::Recs { recs: chunk.to_vec() }.encode());
            if !keep {
                break; // the hub cut the attacker off for too many invalid records
            }
        }
        self.hub.net_disconnect(&peer);
        self.settle();
    }

    fn feed_len(&mut self, who: &str) -> usize {
        self.honest.get_mut(who).map(|n| n.feed(Some(&self.community_hex), "fresh", None).map(|f| f.len()).unwrap_or(0)).unwrap_or(0)
    }

    /// A person's level and voice from the founder's node. `key` is hex.
    fn standing(&self, key: &str) -> (String, f64, bool) {
        let f = self.honest.get("founder").unwrap();
        f.standing(&self.community_hex, Some(key)).map(|s| (s.level, s.voice, s.blocked)).unwrap_or_else(|_| ("newcomer".into(), 0.0, false))
    }
}

/// Runs the full siege and returns a filled scorecard.
pub fn siege(honest: usize, attackers: usize, days: u64, pow_bits: Option<u32>) -> Scorecard {
    siege_with_churn(honest, attackers, days, pow_bits, 0)
}

/// [`siege`] with `churn` percent of honest members offline at each settling, a different share
/// each time; at the end everyone comes back and must hold what the founder holds.
pub fn siege_with_churn(honest: usize, attackers: usize, days: u64, pow_bits: Option<u32>, churn: u64) -> Scorecard {
    let honest = honest.max(2);
    let attackers = attackers.max(1);
    let bits = pow_bits.unwrap_or(POW_BITS);
    let clock = Arc::new(AtomicU64::new(T0));

    // The hub: a headless node, like a session's hoi-node.
    let c2 = clock.clone();
    let mut hub = Node::open(Box::new(hoi_store::Ephemeral), "node", crate::WEB_BASE, Box::new(move || c2.load(Ordering::SeqCst))).unwrap();
    hub.set_identity_bits(4);
    hub.create_identity("hub").unwrap();
    let pairing = hub.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;

    // The founder creates the community with cheap, fast limits (so the sim runs quickly).
    let c2 = clock.clone();
    let mut founder = Node::open(Box::new(hoi_store::Ephemeral), "desktop", crate::WEB_BASE, Box::new(move || c2.load(Ordering::SeqCst))).unwrap();
    founder.set_identity_bits(4);
    founder.create_identity("mira").unwrap();
    let cv = founder
        .create_community(CommunityDraft {
            name: "privacy".into(),
            description: "Practical privacy for normal people.".into(),
            color: "mint".into(),
            limits: Some(LimitsPatch {
                pow_bits: Some(bits),
                max_retention: Some(30 * DAY),
                newcomer_posts_per_day: Some(4),
                // admit on one established voucher, so the Sybil demonstration is vivid: the ring
                // gets in on the turncoat's single vouch, yet shares only his one voice.
                ladder: Some(LadderPatch {
                    identity_bits: Some(4),
                    admit_vouches: Some(1),
                    established_age: Some(DAY),
                    contest_window: Some(3600),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            rules: Some("1. Be kind. 2. No spam. 3. Stay on topic.".into()),
            co_owners: None,
            threshold: None,
        })
        .unwrap();
    let community = Id::from_hex(&cv.id).unwrap();
    let community_hex = cv.id.clone();
    let invite = founder.invite(&community_hex).unwrap().uri;
    founder.pair(&pairing).unwrap();
    founder.net_connect("hub", Role::Home);
    hub.net_connect("founder", Role::Client);

    let mut b = Battle {
        clock: clock.clone(),
        hub,
        honest: BTreeMap::from([("founder".to_string(), founder)]),
        attackers: (0..attackers).map(|_| Forger::new()).collect(),
        community,
        community_hex: community_hex.clone(),
        pow_bits: bits,
        churn: 0,
        offline: Default::default(),
        round: 0,
    };
    b.settle();

    let mut card = Scorecard::new("siege", honest, attackers, days);

    // ── Honest members join and the founder vouches them in. ──────────────────────────────
    let mut honest_keys: Vec<String> = Vec::new();
    for i in 0..honest {
        let name = format!("h{i}");
        b.add_honest(&name, &format!("honest-{i}"), &invite);
        let key = b.honest.get(&name).unwrap().me().unwrap().root_key; // hex
        honest_keys.push(key);
    }
    // The founder vouches every honest member in (the invite in an open community is a vouch).
    {
        let f = b.honest.get_mut("founder").unwrap();
        for k in &honest_keys {
            f.vouch(&community_hex, k, 1, None).unwrap();
        }
    }
    b.settle();
    // Time passes: the honest members cross `established_age`.
    b.advance(DAY + 60);

    let established_now = honest_keys.iter().filter(|k| b.standing(k).0 == "established" || b.standing(k).0 == "trusted").count();
    card.check(
        "honest members become established by being vouched in and waiting",
        established_now == honest,
        format!("{established_now}/{honest} honest members are established"),
    );

    // From here on, members come and go.
    b.churn = churn.min(90);

    // ── Honest members post legitimately and read each other (building voice). ────────────
    for i in 0..honest {
        let name = format!("h{i}");
        let n = b.honest.get_mut(&name).unwrap();
        let _ = n.create_post(&PostDraft {
            community: community_hex.clone(),
            title: format!("hello from {i}"),
            text: "A normal, friendly, on-topic first post.".into(),
            link: None,
            media: None,
        });
    }
    b.settle();
    let legit_posts = b.feed_len("founder");
    card.tally("honest_posts", legit_posts as i64);

    // ── Attack 1: a forgery barrage straight at the hub. ──────────────────────────────────
    // The malformed records (bad signature, future/expired time, no stamp, oversize, non-canonical
    // bytes, a delegation without consent, a key with no identity work) are injected raw over the
    // sync protocol. Every one must be refused; none may reach the feed. The attacker's identity is
    // *not* introduced first — a key with no identity on the hub is exactly one of the things that
    // must bounce, and it keeps the genuinely-invalid records from being mistaken for a newcomer's
    // ordinary (and legitimately shown) text post, which the spam phase below covers instead.
    let mut forged_total = 0i64;
    {
        let now = b.now();
        for i in 0..attackers {
            for attack in forge::catalogue() {
                let items = forge::forge(attack, &b.attackers[i], community, now, b.pow_bits);
                let wires: Vec<Vec<u8>> = items.iter().filter(|f| f.expect == Expect::Refused && !f.wire.is_empty()).map(|f| f.wire.clone()).collect();
                forged_total += wires.len() as i64;
                b.inject(i, wires);
            }
        }
    }
    b.settle();
    // what the forgers signed that the founder shows (counted by author, so a member's post that
    // arrives late under churn is not mistaken for one)
    let forgers: std::collections::BTreeSet<String> = b.attackers.iter().map(|f| f.root_key().hex()).collect();
    let forged_shown = {
        let f = b.honest.get_mut("founder").unwrap();
        f.feed(Some(&community_hex), "fresh", None).unwrap().into_iter().filter(|p| forgers.contains(&p.author.key)).count()
    };
    card.tally("forged_injected", forged_total);
    card.check(
        "forged records (bad signatures, backdating, oversize, unstamped, no identity, …) never reach the feed",
        forged_shown == 0,
        format!("feed held at {legit_posts} legit posts; {forged_total} malformed records injected, {forged_shown} displayed"),
    );

    // ── Attack 2: a spam flood from fresh outsider keys. ──────────────────────────────────
    let spam_each = 5;
    {
        let now = b.now();
        for i in 0..attackers {
            let f = Forger::new();
            let creds = f.credentials(now);
            // reuse the attacker slot's connection by swapping in a throwaway forger's wires
            let mut wires = creds;
            for j in 0..spam_each {
                let r = f.record(
                    community,
                    None,
                    Body::Post {
                        title: format!("CHEAP PILLS {j}"),
                        text: "best prices, message me now, limited offer".into(),
                        link: None,
                        name: None,
                        media: Vec::new(),
                    },
                    now,
                    DAY,
                );
                wires.push(f.sign(r, b.pow_bits).0);
            }
            // Inject under a fresh peer so creds+posts arrive together.
            let peer = format!("spam{i}");
            b.hub.net_connect(&peer, Role::Client);
            let hello = Msg::Hello {
                v: hoi_sync::SYNC_VERSION,
                node: f.root_key(),
                name: None,
                token: Some(b.link_token()),
                communities: vec![community],
                serves: false,
                hints: Vec::new(),
                routes: Vec::new(),
                features: Vec::new(),
                salt: None,
                hashed: Vec::new(),
                pairing: None,
                link_key: None,
                notice_hash: None,
                notice: None,
                holder: None,
                forwards: false,
                rebuilds: false,
            };
            b.hub.net_receive(&peer, &hello.encode());
            for chunk in wires.chunks(hoi_sync::MAX_RECS) {
                if !b.hub.net_receive(&peer, &Msg::Recs { recs: chunk.to_vec() }.encode()) {
                    break;
                }
            }
            b.hub.net_disconnect(&peer);
        }
    }
    b.settle();
    let after_spam = b.feed_len("founder");
    let spam_displayed = after_spam.saturating_sub(legit_posts);
    card.tally("spam_posted", (attackers * spam_each) as i64);

    // Honest members (and the founder) hide what spam did get shown, reaching the quorum.
    {
        let fkey_posts: Vec<String> = {
            let f = b.honest.get_mut("founder").unwrap();
            f.feed(Some(&community_hex), "fresh", None).unwrap().into_iter().filter(|p| p.title.contains("CHEAP PILLS")).map(|p| p.id).collect()
        };
        for pid in &fkey_posts {
            for name in b.honest.keys().cloned().collect::<Vec<_>>() {
                let n = b.honest.get_mut(&name).unwrap();
                let _ = n.review(pid, "hide", Some("spam"));
            }
        }
    }
    b.settle();
    b.advance(60);
    let visible_spam = {
        let f = b.honest.get_mut("founder").unwrap();
        f.feed(Some(&community_hex), "fresh", None).unwrap().into_iter().filter(|p| p.title.contains("CHEAP PILLS") && p.napping.is_none()).count()
    };
    card.tally("spam_reached_hub", spam_displayed as i64);
    card.check(
        "spam is held for newcomers and the community naps the rest without a moderator",
        visible_spam == 0,
        format!("{} spam posts, 0 left showing after community hides", attackers * spam_each),
    );

    // ── Attack 3: a Sybil ring vouched in together by one defector. ───────────────────────
    // The defector is an established honest member who turns; they vouch in a batch of fresh keys.
    let ring = 6usize;
    let sybils: Vec<Forger> = (0..ring).map(|_| Forger::new()).collect();
    let turncoat_name = "h0".to_string();
    let turncoat_key = honest_keys[0].clone();
    {
        let now = b.now();
        // Introduce the sybils' identities/devices to the hub.
        for s in &sybils {
            let peer = format!("syb-{}", s.root_key().short_key());
            b.hub.net_connect(&peer, Role::Client);
            let hello = Msg::Hello {
                v: hoi_sync::SYNC_VERSION,
                node: s.root_key(),
                name: None,
                token: Some(b.link_token()),
                communities: vec![community],
                serves: false,
                hints: Vec::new(),
                routes: Vec::new(),
                features: Vec::new(),
                salt: None,
                hashed: Vec::new(),
                pairing: None,
                link_key: None,
                notice_hash: None,
                notice: None,
                holder: None,
                forwards: false,
                rebuilds: false,
            };
            b.hub.net_receive(&peer, &hello.encode());
            b.hub.net_receive(&peer, &Msg::Recs { recs: s.credentials(now) }.encode());
            b.hub.net_disconnect(&peer);
        }
        b.settle();
        // The turncoat vouches every sybil in, all in the same quarter (one batch).
        let n = b.honest.get_mut(&turncoat_name).unwrap();
        for (i, s) in sybils.iter().enumerate() {
            let _ = n.vouch(&community_hex, &s.root_key().hex(), 1, Some(&format!("friend {i}")));
        }
    }
    b.settle();
    b.advance(DAY + 60); // let the sybils cross established_age too

    let turncoat_voice = b.standing(&turncoat_key).1;
    let ring_voice: f64 = sybils.iter().map(|s| b.standing(&s.root_key().hex()).1).sum();
    card.tally("sybil_ring_size", ring as i64);
    card.check(
        "a Sybil ring vouched in together shares one voucher's voice, never multiplies it",
        ring_voice <= turncoat_voice + 1e-6,
        format!("{ring} sybils hold {ring_voice:.3} voice between them; their voucher holds {turncoat_voice:.3} — the ring did not multiply it"),
    );

    // ── Attack 4: the ring tries to block an honest member and capture governance. ────────
    let victim = Id::from_hex(&honest_keys[honest - 1]).unwrap(); // a loyal honest member
    {
        let now = b.now();
        // Every sybil objects to the victim (vouch -1) and hides a legit post of theirs.
        let victim_posts: Vec<Id> = {
            let f = b.honest.get("founder").unwrap();
            f.feed(Some(&community_hex), "fresh", None)
                .unwrap()
                .into_iter()
                .filter(|p| p.author.key == victim.hex())
                .filter_map(|p| Id::from_hex(&p.id).ok())
                .collect()
        };
        for (i, s) in sybils.iter().enumerate() {
            let mut wires = vec![s.review(community, victim, Verdict::Hide, 1, Some("spam"), now, b.pow_bits)];
            // object to the victim as a person
            let v = s.record(community, Some(victim), Body::Vouch { value: -1, seq: 1, reason: Some("spam".into()) }, now, 365 * DAY);
            wires.push(s.sign(v, b.pow_bits.saturating_sub(8)).0);
            for p in &victim_posts {
                wires.push(s.review(community, *p, Verdict::Hide, 2 + i as u64, Some("off-topic"), now, b.pow_bits));
            }
            let peer = format!("ring-{}", s.root_key().short_key());
            b.hub.net_connect(&peer, Role::Client);
            let hello = Msg::Hello {
                v: hoi_sync::SYNC_VERSION,
                node: s.root_key(),
                name: None,
                token: Some(b.link_token()),
                communities: vec![community],
                serves: false,
                hints: Vec::new(),
                routes: Vec::new(),
                features: Vec::new(),
                salt: None,
                hashed: Vec::new(),
                pairing: None,
                link_key: None,
                notice_hash: None,
                notice: None,
                holder: None,
                forwards: false,
                rebuilds: false,
            };
            b.hub.net_receive(&peer, &hello.encode());
            for chunk in wires.chunks(hoi_sync::MAX_RECS) {
                b.hub.net_receive(&peer, &Msg::Recs { recs: chunk.to_vec() }.encode());
            }
            b.hub.net_disconnect(&peer);
        }
    }
    b.settle();
    b.advance(60);
    let victim_blocked = b.standing(&victim.hex()).2;
    card.check(
        "a Sybil mob cannot block a real member (voice, not heads, decides §16)",
        !victim_blocked,
        if victim_blocked {
            "a loyal member was blocked by the ring — BROKE"
        } else {
            "the ring's hides and objections did not reach quorum; the member stands"
        },
    );

    // ── Governance: the founder's limits can't be rewritten by the ring; and an owner can't
    //    change them alone once others are established. We check the ring can't force it. ──
    let bar_before = {
        let f = b.honest.get("founder").unwrap();
        f.community(&community_hex).unwrap().limits.ladder.identity_bits
    };
    // The founder proposes raising the identity bar; it needs the community's consent, which the
    // (now-established) honest members never give. It must not be in effect yet.
    {
        let f = b.honest.get_mut("founder").unwrap();
        let mut limits = f.community(&community_hex).unwrap().limits.clone();
        limits.ladder.identity_bits = 16; // a steep hike
        let _ = f.set_limits(&community_hex, &limits);
    }
    b.settle();
    b.advance(60);
    let bar_after = {
        let f = b.honest.get("founder").unwrap();
        f.community(&community_hex).unwrap().limits.ladder.identity_bits
    };
    card.check(
        "rules and limits change only by the community's consent, not an owner alone",
        bar_after == bar_before,
        format!("a proposal to raise the identity bar from {bar_before} to 16 bits is pending consent, not in effect (still {bar_after})"),
    );

    // ── Everyone comes back: whoever was offline catches up and holds what the founder holds.
    if churn > 0 {
        b.churn = 0;
        b.settle();
        b.advance(60);
        let founder_feed = b.feed_len("founder");
        let names: Vec<String> = b.honest.keys().cloned().collect();
        let behind = names.iter().filter(|n| b.feed_len(n) != founder_feed).count();
        card.tally("churn_percent", churn as i64);
        card.check(
            "members who came and went catch up with everything once back",
            behind == 0,
            format!("{} of {} honest nodes hold the founder's {founder_feed} posts after {churn}% churn", names.len() - behind, names.len()),
        );
    }

    // ── Convergence: every honest node agrees on who is established. ───────────────────────
    let mut agree = true;
    let founder_view: BTreeMap<String, String> = honest_keys.iter().map(|k| (k.clone(), b.standing(k).0)).collect();
    for name in b.honest.keys().cloned().collect::<Vec<_>>() {
        if name == "founder" {
            continue;
        }
        for (k, lvl) in &founder_view {
            let n = b.honest.get(&name).unwrap();
            let here = n.standing(&community_hex, Some(k)).map(|s| s.level).unwrap_or_default();
            if &here != lvl {
                agree = false;
            }
        }
    }
    card.check(
        "honest nodes holding the same records compute the same standing",
        agree,
        if agree { "all honest nodes agree on every member's level" } else { "honest nodes disagreed on a level — BROKE" },
    );

    card
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_protocol_holds_against_a_small_siege() {
        // A quick siege in CI: every invariant must stand.
        let card = siege(3, 2, 30, Some(8));
        assert!(card.held(), "the protocol broke:\n{}", card.render());
        // Sanity: we actually exercised the attacks.
        let tally = |k: &str| card.tallies.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or(0);
        assert!(tally("forged_injected") > 0, "no forgeries were injected");
        assert!(tally("spam_posted") > 0, "no spam was posted");
    }

    #[test]
    fn a_siege_with_churn_holds() {
        // a third of the members offline at any time, a different third each time
        let card = siege_with_churn(6, 3, 30, Some(8), 33);
        assert!(
            card.held(),
            "the protocol broke under churn:
{}",
            card.render()
        );
    }

    /// M4's harness: two hundred real nodes, a fifth offline at any time (`release-major.yml`).
    #[test]
    #[ignore = "minutes: run with --ignored, as the major-release suites do"]
    fn two_hundred_nodes_with_churn_hold() {
        let card = siege_with_churn(200, 20, 30, Some(8), 20);
        assert!(
            card.held(),
            "the protocol broke at scale:
{}",
            card.render()
        );
    }

    #[test]
    fn a_bigger_siege_also_holds() {
        let card = siege(5, 4, 30, Some(8));
        assert!(card.held(), "the protocol broke:\n{}", card.render());
    }
}
