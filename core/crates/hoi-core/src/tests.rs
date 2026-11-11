//! Node-level tests: a community's whole life on one device, two devices syncing through a home
//! node, threshold cosigning, expiry, and spam bouncing off.

#[path = "tests/discovery.rs"]
mod discovery;

#[path = "tests/directory.rs"]
mod directory;

#[path = "tests/carrier.rs"]
mod carrier;

#[path = "tests/carry.rs"]
mod carry;
#[path = "tests/trackers.rs"]
mod trackers;
// M0–M2 gaps
#[path = "tests/age.rs"]
mod age;
#[path = "tests/blocklists.rs"]
mod blocklists;
// Media over iroh-blobs (M5).
mod blobs;
// Direct links between browsers (M5).
#[path = "tests/gaps.rs"]
mod gaps;
#[path = "tests/handle.rs"]
mod handle;
#[path = "tests/preview.rs"]
mod preview;
mod signal;
#[path = "tests/tally.rs"]
mod tally;
#[path = "tests/trust.rs"]
mod trust;
#[path = "tests/wiki.rs"]
mod wiki;
// recovery without losing standing
mod recover;
// remote signing for browsers
mod remote;

#[path = "tests/nodes.rs"]
mod nodes;
#[path = "tests/notice.rs"]
mod notice;

#[path = "tests/public_nodes.rs"]
mod public_nodes;

#[path = "tests/suppress.rs"]
mod suppress;

#[path = "tests/warmth.rs"]
mod warmth;

#[path = "tests/strangers.rs"]
mod strangers;

#[path = "tests/public_join.rs"]
mod public_join;

#[path = "tests/storage_compatibility.rs"]
mod storage_compatibility;

#[path = "tests/bulk.rs"]
mod bulk;
#[path = "tests/cleanup.rs"]
mod cleanup;
#[path = "tests/deletion.rs"]
mod deletion;
#[path = "tests/drop_share.rs"]
mod drop_share;

#[path = "tests/private_isolation.rs"]
mod private_isolation;

#[path = "tests/private_mls.rs"]
mod private_mls;

#[path = "tests/profile.rs"]
mod profile;
mod profile_color;

#[path = "tests/hardening.rs"]
mod hardening;

#[path = "tests/scatter.rs"]
mod scatter;

#[path = "tests/forward.rs"]
mod forward;

#[path = "tests/members_only.rs"]
mod members_only;

#[path = "tests/open.rs"]
mod open;

#[path = "tests/address.rs"]
mod address;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use hoi_crypto::Keypair;
use hoi_store::{Change, Loaded, Persist};
use hoi_types::{Body, Id, Record};

use crate::dto::*;
use crate::{Node, PeerMeta, Role, Source};

type Records = BTreeMap<Id, Vec<u8>>;
type Blobs = BTreeMap<String, Vec<u8>>;

#[derive(Clone, Default)]
struct Mem(Arc<Mutex<(Records, Blobs)>>);

impl Persist for Mem {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        let mut m = self.0.lock().unwrap();
        for c in changes {
            match c {
                Change::Put { id, wire, .. } => {
                    m.0.insert(*id, wire.clone());
                }
                Change::Delete { id } => {
                    m.0.remove(id);
                }
                Change::Blob { key, value: Some(v) } => {
                    m.1.insert(key.clone(), v.clone());
                }
                Change::Blob { key, value: None } => {
                    m.1.remove(key);
                }
                Change::Wipe => {
                    m.0.clear();
                    m.1.clear();
                }
            }
        }
        Ok(())
    }
    fn load(&mut self) -> Result<Loaded, String> {
        let m = self.0.lock().unwrap();
        Ok(Loaded { records: m.0.values().cloned().collect(), blobs: m.1.iter().map(|(k, v)| (k.clone(), v.clone())).collect() })
    }
}

const T0: u64 = 1_790_000_000;

/// Someone else on the network, made by hand: identity, a blessed device, records signed
/// without any client in between (so tests can do what an attacker's tool would).
struct Actor {
    root: Keypair,
    device: Keypair,
    work: u64,
}

impl Actor {
    fn new() -> Actor {
        let root = Keypair::generate().unwrap();
        let work = hoi_crypto::identity::mine(&root.public(), 4);
        Actor { root, device: Keypair::generate().unwrap(), work }
    }
    fn key(&self) -> String {
        self.root.public().hex()
    }
    fn identity(&self, now: u64) -> Vec<u8> {
        let mut r = Record {
            v: 1,
            community: None,
            author: self.root.public(),
            parent: None,
            created_at: now,
            expires_at: now + hoi_validate::KEEP,
            body: Body::Identity { work: self.work, rc: None, of: None },
            nonce: 0,
        };
        r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
        self.root.sign_record(r).0.encode()
    }
    fn delegation(&self, now: u64) -> Vec<u8> {
        let mut r = Node::delegation_record(&self.root, self.device.public(), now);
        r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
        self.device.sign_record(r).0.encode()
    }
    /// Signs `r` with the device key at `bits` of stamp.
    fn sign(&self, mut r: Record, bits: u32) -> (Vec<u8>, Id) {
        r.author = self.device.public();
        r.nonce = hoi_crypto::pow::mine(&r, bits);
        let (w, id) = self.device.sign_record(r);
        (w.encode(), id)
    }
    /// Hands `n` this actor's identity and device blessing out of band (as an invite or a link
    /// code would). On the wire they travel alongside the actor's first community record; a
    /// stranger's lone identity or device is refused (`strangers_cannot_fill_a_node_with_keys`).
    fn intro(&self, n: &mut Node, now: u64) {
        for w in [self.identity(now), self.delegation(now)] {
            n.ingest(&w, &Source::Import).unwrap();
        }
    }
    fn send(&self, n: &mut Node, r: Record) -> Result<Id, hoi_validate::Reject> {
        let live = r.created_at + 600 >= n.now();
        let (w, _) = self.sign(r, 8);
        let res = n.ingest(&w, &Source::Peer { peer: "p".into(), live }).map(|a| a.id);
        n.commit().unwrap();
        res
    }
}

fn rec(c: Id, parent: Option<Id>, body: Body, now: u64) -> Record {
    Record { v: 1, community: Some(c), author: Id::ZERO, parent, created_at: now, expires_at: now + 86_400, body, nonce: 0 }
}

fn text_post(title: &str, text: &str) -> Body {
    Body::Post { title: title.into(), text: text.into(), link: None, name: None, media: Vec::new() }
}

fn node(platform: &str, t: &Arc<AtomicU64>, mem: &Mem) -> Node {
    let t = t.clone();
    let mut n = Node::open(Box::new(mem.clone()), platform, "https://hoi.sh/app/", Box::new(move || t.load(Ordering::SeqCst))).unwrap();
    n.set_identity_bits(4);
    n
}

fn cheap_limits() -> LimitsPatch {
    LimitsPatch {
        pow_bits: Some(8),
        max_retention: Some(7 * 86_400),
        newcomer_posts_per_day: Some(5),
        ladder: Some(LadderPatch { identity_bits: Some(4), established_age: Some(86_400), contest_window: Some(3600), ..Default::default() }),
        ..Default::default()
    }
}

fn draft(name: &str) -> CommunityDraft {
    CommunityDraft {
        name: name.into(),
        description: "Practical privacy for normal people.".into(),
        color: "mint".into(),
        limits: Some(cheap_limits()),
        rules: Some("1. Be kind.".into()),
        co_owners: None,
        threshold: None,
    }
}

fn post(n: &mut Node, c: &str, title: &str) -> PostSummary {
    n.create_post(&PostDraft { community: c.into(), title: title.into(), text: "Short version: it's fine.".into(), link: None, media: None }).unwrap()
}

/// Like [`pump`], with the names each side uses for the other.
fn pump_named(a: &mut Node, a_calls_b: &str, b: &mut Node, b_calls_a: &str) {
    for _ in 0..50 {
        let out_a: Vec<Vec<u8>> = a.take_outbox().into_iter().filter(|(p, _)| p == a_calls_b).map(|(_, m)| m).collect();
        let out_b: Vec<Vec<u8>> = b.take_outbox().into_iter().filter(|(p, _)| p == b_calls_a).map(|(_, m)| m).collect();
        if out_a.is_empty() && out_b.is_empty() {
            break;
        }
        for m in out_a {
            assert!(b.net_receive(b_calls_a, &m), "{b_calls_a} dropped by b");
        }
        for m in out_b {
            assert!(a.net_receive(a_calls_b, &m), "{a_calls_b} dropped by a");
        }
    }
    a.commit().unwrap();
    b.commit().unwrap();
}

/// Pairs `device` with `home` through the home node's current pairing link (they know each other
/// as `home_name` and `device_name`). The device's hello spends the link at once, so the next
/// device gets a link of its own, and the home node's answer arrives in order; what either side
/// says next waits for the next pump.
fn pair_now(home: &mut Node, device: &mut Node, home_name: &str, device_name: &str) {
    device.pair(&home.net_status().serving.unwrap().pairing).unwrap();
    device.net_connect(home_name, Role::Home);
    home.net_connect(device_name, Role::Client);
    for (_, m) in device.take_outbox() {
        assert!(home.net_receive(device_name, &m), "the home node took the pairing link");
    }
    for (to, m) in home.take_outbox() {
        if to == device_name {
            assert!(device.net_receive(home_name, &m));
        } else {
            home.outbox.push((to, m));
        }
    }
}

/// Delivers queued messages both ways until nothing is left. `a` knows `b` as "b" and vice versa.
fn pump(a: &mut Node, b: &mut Node) {
    for _ in 0..50 {
        let out_a = a.take_outbox();
        let out_b = b.take_outbox();
        if out_a.is_empty() && out_b.is_empty() {
            break;
        }
        for (_, m) in out_a {
            assert!(b.net_receive("a", &m), "b dropped a");
        }
        for (_, m) in out_b {
            assert!(a.net_receive("b", &m), "a dropped b: {:?}", hoi_sync::Msg::decode(&m).map(|x| format!("{x:?}").chars().take(300).collect::<String>()));
        }
    }
    a.commit().unwrap();
    b.commit().unwrap();
}

#[test]
fn a_community_on_one_device() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut n = node("web", &t, &mem);
    assert!(!n.boot().has_identity);
    let me = n.create_identity("lena").unwrap();
    assert!(me.has_root && me.root.starts_with("did:key:z6Mk"));
    assert_eq!(me.devices.len(), 1);

    let c = n.create_community(draft("privacy")).unwrap();
    assert_eq!(c.role, "owner");
    assert_eq!(c.rules.as_ref().unwrap().version, 1);
    let p = post(&mut n, &c.id, "Which messengers survived the audit?");
    assert!(p.mine && p.author.name == "lena");
    let cm = n.reply(&p.id, "Defaults count, not capability.").unwrap();
    assert_eq!(n.vote(&p.id, 1).unwrap(), 1);
    assert_eq!(n.vote(&p.id, 0).unwrap(), 0, "retracting a vote");
    n.vote(&p.id, 1).unwrap();
    n.edit(&p.id, "Edited: three apps close the gap.", None).unwrap();

    let view = n.post(&p.id).unwrap();
    assert_eq!(view.comments.len(), 1);
    assert!(view.summary.edited && view.text.starts_with("Edited"));
    assert_eq!(view.summary.score, 1);
    assert_eq!(view.checks.signed_by, "device");
    assert!(view.checks.stamp_bits >= 8);
    assert_eq!(n.search("three apps", None).unwrap().len(), 1);

    match n.set_limits(&c.id, &LimitsDto { newcomer_posts_per_day: 3, ..n.community(&c.id).unwrap().limits }).unwrap() {
        LedgerResult::Published { entry } => assert_eq!(entry.action, "limits"),
        _ => panic!("threshold 1 publishes at once"),
    }
    n.delete_own(&cm.id).unwrap();
    let view = n.post(&p.id).unwrap();
    assert!(view.comments[0].removed.is_some(), "deleted comment keeps its place as a Poof box");
    n.commit().unwrap();

    // everything survives a restart
    drop(n);
    let mut n = node("web", &t, &mem);
    assert_eq!(n.me().unwrap().name, "lena");
    assert_eq!(n.communities().len(), 1);
    let feed = n.feed(None, "hot", None).unwrap();
    assert_eq!(feed.len(), 1);
    assert!(feed[0].edited);
    assert_eq!(n.ledger(&c.id).unwrap().len(), 2);
    assert!(n.post(&p.id).unwrap().comments[0].removed.is_some());
    assert_eq!(n.community(&c.id).unwrap().limits.newcomer_posts_per_day, 3);
}

#[test]
fn two_devices_through_a_home_node() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    let mut web = node("web", &t, &Mem::default());
    home.create_identity("mira").unwrap();
    web.create_identity("lena").unwrap();
    let c = home.create_community(draft("privacy")).unwrap();
    let p = post(&mut home, &c.id, "Start here");
    let status = home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let pairing = status.serving.unwrap().pairing;

    let inv = home.invite(&c.id).unwrap();
    let preview = web.preview_invite(&inv.web).unwrap();
    assert!(preview.verified && preview.community.name == "privacy");
    web.join(&inv.uri).unwrap();
    web.pair(&pairing).unwrap();
    web.net_connect("a", Role::Home);
    home.net_connect("b", Role::Client);
    pump(&mut home, &mut web);

    assert_eq!(web.net_status().mode, "home-node");
    assert!(web.net_status().label.starts_with("via mira"), "{}", web.net_status().label);
    let feed = web.feed(Some(&c.id), "fresh", None).unwrap();
    assert_eq!(feed.len(), 1, "the browser caught up through its home node");
    assert_eq!(feed[0].author.name, "mira");

    // a reply travels back live
    let r = web.reply(&p.id, "Can we pin this?").unwrap();
    pump(&mut home, &mut web);
    assert_eq!(home.post(&p.id).unwrap().comments.len(), 1);
    assert_eq!(home.post(&p.id).unwrap().checks.received_from, 0);

    // the owner removes it: it naps while the removal can be contested…
    home.remove(&r.id, "rule 1").unwrap();
    pump(&mut home, &mut web);
    let v = web.post(&p.id).unwrap();
    assert!(v.comments[0].removed.is_none());
    assert!(v.comments[0].napping.as_ref().expect("napping on the browser").label.starts_with("removal pending"));
    // …then the window passes and the browser shows Poof
    t.store(T0 + 3601, Ordering::SeqCst);
    home.sweep();
    web.sweep();
    let v = web.post(&p.id).unwrap();
    let rm = v.comments[0].removed.as_ref().expect("Poof on the browser too");
    assert_eq!(rm.reason, "rule 1");

    // and bans lena: her next post is refused on her own device
    home.ban(&c.id, &web.me().unwrap().root_key, "rule 1 again").unwrap();
    pump(&mut home, &mut web);
    let err = web.create_post(&PostDraft { community: c.id.clone(), title: "hi".into(), text: "x".into(), link: None, media: None }).unwrap_err();
    assert_eq!(err.code, "banned");

    // a wrong pairing token is turned away
    let mut stranger = node("web", &t, &Mem::default());
    stranger.create_identity("x").unwrap();
    stranger.pair("hoi://pair?u=ws%3A%2F%2F127.0.0.1%3A7842&t=nope").unwrap();
    stranger.net_connect("a", Role::Home);
    home.net_connect("s", Role::Client);
    let hello = stranger.take_outbox().pop().unwrap().1;
    assert!(!home.net_receive("s", &hello));

    // time removes everything
    t.store(T0 + 8 * 86_400, Ordering::SeqCst);
    home.sweep();
    web.sweep();
    assert!(home.feed(None, "hot", None).unwrap().is_empty());
    assert!(web.feed(None, "hot", None).unwrap().is_empty());
    assert_eq!(home.communities().len(), 1, "the community itself never expires");
}

#[test]
fn your_linked_browser_shows_what_your_computer_posts() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut desk = node("desktop", &t, &Mem::default());
    let mut web = node("web", &t, &Mem::default());
    desk.create_identity("mira").unwrap();
    let c = desk.create_community(draft("privacy")).unwrap();
    let p = post(&mut desk, &c.id, "Posted on the computer");

    // the browser becomes another device of the same person and learns the community
    let req = web.link_request("mira's browser").unwrap();
    let ok = desk.link_approve(&req).unwrap();
    web.link_complete(&ok).unwrap();
    assert_eq!(web.me().unwrap().root_key, desk.me().unwrap().root_key);
    assert_eq!(web.communities().len(), 1, "the approval carried the community");
    assert!(web.feed(Some(&c.id), "fresh", None).unwrap().is_empty(), "posts come from a peer, and there is none yet");

    // pairing with the computer's home node brings them over
    let pairing = desk.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    web.pair(&pairing).unwrap();
    web.net_connect("a", Role::Home);
    desk.net_connect("b", Role::Client);
    pump(&mut desk, &mut web);
    assert_eq!(web.feed(Some(&c.id), "fresh", None).unwrap()[0].id, p.id);

    // and everything after travels live, both ways
    let p2 = post(&mut desk, &c.id, "And one more from the computer");
    pump(&mut desk, &mut web);
    assert!(web.feed(Some(&c.id), "fresh", None).unwrap().iter().any(|x| x.id == p2.id));
    let r = web.reply(&p.id, "Seen from the browser").unwrap();
    pump(&mut desk, &mut web);
    let v = desk.post(&p.id).unwrap();
    assert_eq!(v.comments[0].id, r.id);
    assert_eq!(v.comments[0].author.name, "mira", "the browser posts as the same person");
}

/// The `net` events of `n` since the last call.
fn net_events(n: &mut Node) -> Vec<NetStatus> {
    n.take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Net { status } => Some(*status),
            _ => None,
        })
        .collect()
}

#[test]
fn the_app_hears_when_devices_come_and_go() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut desk = node("desktop", &t, &Mem::default());
    let mut web = node("web", &t, &Mem::default());
    desk.create_identity("mira").unwrap();
    web.create_identity("lena").unwrap();
    let c = desk.create_community(draft("privacy")).unwrap();
    pair(&mut desk, &mut web, &c.id);

    let st = net_events(&mut desk).pop().expect("the computer heard its browser arrive");
    assert_eq!((st.peers, st.serving.unwrap().clients), (1, 1));
    let st = net_events(&mut web).pop().expect("the browser heard its home node answer");
    assert_eq!(st.mode, "home-node");
    assert!(st.home_node.unwrap().connected);

    desk.commit().unwrap();
    assert!(net_events(&mut desk).is_empty(), "nothing changed, nothing said");
    desk.net_disconnect("b");
    desk.commit().unwrap();
    assert_eq!(net_events(&mut desk).pop().expect("and leave").serving.unwrap().clients, 0);
}

#[test]
fn threshold_needs_two_owners() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    let mut b = node("desktop", &t, &Mem::default());
    a.create_identity("mira").unwrap();
    let bk = b.create_identity("jonas").unwrap().root_key;
    let mut d = draft("privacy");
    d.co_owners = Some(vec![bk]);
    d.threshold = Some(2);
    let c = a.create_community(d).unwrap();
    assert_eq!(c.owners.len(), 2);
    assert!(c.rules.is_none(), "first rules wait for both owners");
    b.join(&a.invite(&c.id).unwrap().uri).unwrap();

    let code = match a.set_rules(&c.id, "1. Be kind. 2. Disclose vendors.").unwrap() {
        LedgerResult::NeedsCosign { proposal, have, need } => {
            assert_eq!((have, need), (1, 2));
            proposal
        }
        _ => panic!("needs the second owner"),
    };
    match b.cosign(&code).unwrap() {
        LedgerResult::Published { entry } => assert_eq!(entry.by.len(), 2),
        _ => panic!("two of two signed"),
    }
    let st = a.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    b.pair(&st.serving.unwrap().pairing).unwrap();
    b.net_connect("a", Role::Home);
    a.net_connect("b", Role::Client);
    pump(&mut a, &mut b);
    assert_eq!(a.community(&c.id).unwrap().rules.unwrap().signed_by.len(), 2);
}

#[test]
fn spam_bounces_off() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    a.create_identity("mira").unwrap();
    let mut d = draft("privacy");
    d.limits = Some(LimitsPatch { newcomer_posts_per_day: Some(2), ..cheap_limits() });
    let c = a.create_community(d).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();

    // a fresh key floods records straight at a's node, skipping its own client's limits
    let spammer = Actor::new();
    spammer.intro(&mut a, T0);
    let mut accepted = 0;
    for i in 0..40 {
        let now = t.load(Ordering::SeqCst) + i;
        let body = Body::Post { title: format!("FREE CRYPTO {i}"), text: "click".into(), link: None, name: Some("totally mira".into()), media: Vec::new() };
        let (w, _) = spammer.sign(rec(cid, None, body, now), 8);
        if a.ingest(&w, &Source::Peer { peer: "evil".into(), live: true }).is_ok() {
            accepted += 1;
        }
    }
    assert_eq!(accepted, 2, "newcomer allowance");
    assert_eq!(a.stats().bounced_keys, 1, "the key got bounced after enough strikes");
    assert!(a.feed(None, "hot", None).unwrap().iter().all(|p| !p.title.contains("FREE")), "bounced keys vanish from the feed");
    let cv = a.community(&c.id).unwrap();
    assert!(cv.banned.iter().any(|b| b.auto));
    assert_eq!(cv.flags.len(), 1, "a signed report with evidence went out");
}

#[test]
fn a_fresh_home_node_learns_communities_from_its_devices() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("node", &t, &Mem::default());
    let mut mira = node("web", &t, &Mem::default());
    let mut lena = node("web", &t, &Mem::default());
    home.create_identity("the pi").unwrap();
    mira.create_identity("mira").unwrap();
    lena.create_identity("lena").unwrap();
    let pairing = home.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;

    // mira's community exists only on her browser when she pairs
    let c = mira.create_community(draft("privacy")).unwrap();
    mira.pair(&pairing).unwrap();
    mira.net_connect("a", Role::Home);
    home.net_connect("b", Role::Client);
    pump(&mut home, &mut mira);
    assert_eq!(home.communities().len(), 1, "the home node picked up mira's community");
    let p = post(&mut mira, &c.id, "Live after pairing");
    pump(&mut home, &mut mira);
    assert_eq!(home.feed(None, "fresh", None).unwrap().len(), 1, "live post reached the home node");

    // lena joins by invite and pairs with the same home node, with its next link: mira's is spent
    lena.join(&mira.invite(&c.id).unwrap().web).unwrap();
    let next = home.net_status().serving.unwrap().pairing;
    assert_ne!(next, pairing);
    lena.pair(&next).unwrap();
    lena.net_connect("a", Role::Home);
    home.net_connect("c", Role::Client);
    let (mut h2, mut l2) = (home, lena);
    // the home node knows lena as "c"; route by hand
    for _ in 0..20 {
        let out_h: Vec<_> = h2.take_outbox();
        let out_l = l2.take_outbox();
        if out_h.is_empty() && out_l.is_empty() {
            break;
        }
        for (to, m) in out_h {
            if to == "c" {
                l2.net_receive("a", &m);
            }
        }
        for (_, m) in out_l {
            h2.net_receive("c", &m);
        }
    }
    assert_eq!(l2.feed(None, "fresh", None).unwrap().len(), 1, "lena caught up through the home node");
    let r = l2.reply(&p.id, "hoi").unwrap();
    // lena → home → mira, live
    for (to, m) in l2.take_outbox() {
        assert_eq!(to, "a");
        h2.net_receive("c", &m);
    }
    for (to, m) in h2.take_outbox() {
        if to == "b" {
            mira.net_receive("a", &m);
        }
    }
    assert_eq!(mira.post(&p.id).unwrap().comments[0].id, r.id);
}

// ---------------------------------------------------------------- the ladder, juries, vetoes

const DAY: u64 = 86_400;

/// An owner's node with a community, and the clock.
fn owner() -> (Arc<AtomicU64>, Node, Id, String) {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    a.create_identity("mira").unwrap();
    let c = a.create_community(draft("privacy")).unwrap();
    (t, a, Id::from_hex(&c.id).unwrap(), c.id)
}

/// `n` people the owner vouched in at T0, established one day later.
fn members(a: &mut Node, c: &str, n: usize) -> Vec<Actor> {
    let v: Vec<Actor> = (0..n).map(|_| Actor::new()).collect();
    for m in &v {
        m.intro(a, T0);
        a.vouch(c, &m.key(), 1, None).unwrap();
    }
    v
}

fn pair(home: &mut Node, web: &mut Node, c: &str) {
    let pairing = home.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    web.join(&home.invite(c).unwrap().uri).unwrap();
    web.pair(&pairing).unwrap();
    web.net_connect("a", Role::Home);
    home.net_connect("b", Role::Client);
    pump(home, web);
}

#[test]
fn newcomers_earn_links_by_being_read() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    let mut web = node("web", &t, &Mem::default());
    home.create_identity("mira").unwrap();
    web.create_identity("lena").unwrap();
    let c = home.create_community(draft("privacy")).unwrap();
    pair(&mut home, &mut web, &c.id);

    // a newcomer shares text only: links are refused before they leave the device
    let st = web.standing(&c.id, None).unwrap();
    assert_eq!((st.level.as_str(), st.capabilities.clone()), ("newcomer", vec!["text".to_string()]));
    assert!(st.next.unwrap().missing[0].starts_with("vouches"));
    let linky = PostDraft { community: c.id.clone(), title: "my blog".into(), text: "read https://lena.example/hoi".into(), link: None, media: None };
    assert_eq!(web.create_post(&linky).unwrap_err().code, "not-allowed");

    let p = post(&mut web, &c.id, "Hello from lena");
    web.reply(&p.id, "and a first comment with a few more words in it").unwrap();
    pump(&mut home, &mut web);
    let feed = home.feed(Some(&c.id), "fresh", None).unwrap();
    assert!(feed[0].unvalidated, "content from someone the community hasn't validated is marked");
    assert_eq!(feed[0].author.standing.as_deref(), Some("newcomer"));
    assert_eq!(home.review_queue(&c.id).unwrap().len(), 1);
    assert_eq!(home.community(&c.id).unwrap().review_queue, 1);

    // mira reads it and doesn't hide it; ten minutes later her device vouches
    home.post(&p.id).unwrap();
    home.sweep();
    assert_eq!(web.standing(&c.id, None).unwrap().level, "newcomer", "not straight away");
    t.store(T0 + 601, Ordering::SeqCst);
    home.sweep();
    pump(&mut home, &mut web);
    let st = web.standing(&c.id, None).unwrap();
    assert_eq!(st.level, "member");
    assert_eq!(st.vouched_by[0].key, home.me().unwrap().root_key);
    assert!(home.review_queue(&c.id).unwrap().is_empty());
    assert!(!home.feed(Some(&c.id), "fresh", None).unwrap()[0].unvalidated);

    // links unlock; media doesn't yet
    web.create_post(&linky).unwrap();
    let st = web.standing(&c.id, None).unwrap();
    assert!(st.capabilities.contains(&"links".to_string()) && !st.capabilities.contains(&"media".to_string()));
    // a day later lena is established: her reviews and vouches count
    t.store(T0 + DAY + 700, Ordering::SeqCst);
    web.sweep();
    assert_eq!(web.standing(&c.id, None).unwrap().level, "established");
}

#[test]
fn receivers_hold_back_what_a_newcomer_shouldnt_share() {
    let (_t, mut a, cid, c) = owner();
    let x = Actor::new();
    x.intro(&mut a, T0);
    // a hand-made post with a link, straight from x's own tool past any client check
    let body = Body::Post {
        title: "deal".into(),
        text: "buy at https://shop.example now".into(),
        link: Some("https://shop.example".into()),
        name: None,
        media: Vec::new(),
    };
    let id = x.send(&mut a, rec(cid, None, body, T0)).expect("stored, so it isn't fetched again and again");
    assert!(a.feed(Some(&c), "fresh", None).unwrap().is_empty(), "held: shown to nobody");
    assert_eq!(a.post(&id.hex()).unwrap_err().code, "not-found");
    let bundle = a.export_bundle(&c).unwrap();
    assert!(!bundle.windows(12).any(|w| w == b"shop.example"), "and never passed on");
    // plain text from the same newcomer is fine, marked as unvalidated
    x.send(&mut a, rec(cid, None, text_post("hi", "just words"), T0 + 1)).unwrap();
    assert!(a.feed(Some(&c), "fresh", None).unwrap()[0].unvalidated);
}

#[test]
fn the_community_hides_and_blocks_without_a_moderator() {
    let (t, mut a, cid, c) = owner();
    let ms = members(&mut a, &c, 3);
    t.store(T0 + DAY + 10, Ordering::SeqCst);
    a.sweep();
    for m in &ms {
        assert_eq!(a.standing(&c, Some(&m.key())).unwrap().level, "established");
    }
    let spammer = Actor::new();
    let now = T0 + DAY + 20;
    spammer.intro(&mut a, now);
    let p = spammer.send(&mut a, rec(cid, None, text_post("cheap pills", "best prices, message me"), now)).unwrap();
    // hiding needs a reason from the list: disagreeing isn't one
    assert_eq!(a.review(&p.hex(), "hide", Some("i disagree")).unwrap_err().code, "invalid");
    for m in &ms {
        m.send(&mut a, rec(cid, Some(p), Body::Review { verdict: hoi_types::Verdict::Hide, seq: 1, reason: Some("spam".into()) }, now + 5)).unwrap();
    }
    let f = a.feed(Some(&c), "fresh", None).unwrap();
    let nap = f[0].napping.as_ref().expect("napping for everyone, still peekable");
    assert!(nap.label.starts_with("hidden by 3 members: spam"), "{}", nap.label);
    assert_eq!(nap.by, "the community");
    let st = a.standing(&c, Some(&spammer.key())).unwrap();
    assert!(st.blocked && st.capabilities.is_empty(), "blocked from the community");
    // what a blocked author sends next is stored but not passed on
    let q = spammer.send(&mut a, rec(cid, None, text_post("again", "still selling, still here"), now + 60)).unwrap();
    assert!(!a.forwardable(a.store.get(&q).unwrap()));
    assert!(a.community(&c).unwrap().banned.iter().any(|b| b.person.key == spammer.key() && b.auto));
}

#[test]
fn vetoes_undo_a_moderators_removal_and_strikes_suspend_them() {
    let (t, mut a, cid, c) = owner();
    let ms = members(&mut a, &c, 3);
    t.store(T0 + 60, Ordering::SeqCst);
    let m = Actor::new();
    m.intro(&mut a, T0 + 60);
    match a.add_mod(&c, &m.key()).unwrap() {
        LedgerResult::Published { .. } => {}
        _ => panic!("threshold 1"),
    }
    t.store(T0 + DAY + 10, Ordering::SeqCst);
    a.sweep();
    let now = T0 + DAY + 10;
    let mut removed = Vec::new();
    for (seq, i) in (11..).zip(0..3u64) {
        let p = ms[0].send(&mut a, rec(cid, None, text_post(&format!("an opinion {i}"), "the mods won't like this one bit"), now + i)).unwrap();
        let entry = hoi_types::LedgerBody {
            seq,
            action: hoi_types::LedgerAction::Remove,
            target: Some(p),
            reason: Some("rule 1".into()),
            evidence: None,
            cosign: None,
            text: None,
            limits: None,
            drop_key: None,
            patch: None,
        };
        let e = m.send(&mut a, rec(cid, None, Body::Ledger(entry), now + 10 + i)).unwrap();
        removed.push((p, e));
    }
    // a removal naps first: the body stays while the community can contest it
    let (p0, e0) = removed[0];
    let v = a.post(&p0.hex()).unwrap();
    assert!(v.summary.napping.unwrap().label.starts_with("removal pending: rule 1"));
    let entry = a.ledger(&c).unwrap().into_iter().find(|x| x.id == e0.hex()).unwrap();
    assert!(entry.vetoable && entry.pending_until.is_some() && !entry.overturned);
    // the three members (a third of a voice each, vouched in together) veto every one; the
    // author's own veto counts too, since the entry targets the post, not her
    for (_, e) in &removed {
        for v in &ms {
            v.send(&mut a, rec(cid, Some(*e), Body::Review { verdict: hoi_types::Verdict::Veto, seq: 1, reason: Some("no rule says that".into()) }, now + 100))
                .unwrap();
        }
    }
    let entry = a.ledger(&c).unwrap().into_iter().find(|x| x.id == e0.hex()).expect("still in the ledger, for everyone to see");
    assert!(entry.overturned && entry.vetoes == 3, "overturned by the community");
    let v = a.post(&p0.hex()).unwrap();
    assert!(v.summary.napping.is_none(), "the post is back");
    // three overturned actions suspend the moderator
    let cv = a.community(&c).unwrap();
    assert!(cv.mods.is_empty(), "suspended");
    // and the window passing changes nothing
    t.store(now + 2 * 3600, Ordering::SeqCst);
    a.sweep();
    assert!(a.post(&p0.hex()).unwrap().summary.napping.is_none());
}

#[test]
fn nobody_can_claim_someone_elses_device() {
    let (_t, mut a, _cid, _c) = owner();
    let victim = Actor::new();
    victim.intro(&mut a, T0);
    let thief = Actor::new();
    thief.intro(&mut a, T0);
    let peer = Source::Peer { peer: "p".into(), live: true };
    // the thief blesses the victim's device under the thief's root: only the device can consent
    let mut r = Node::delegation_record(&thief.root, victim.device.public(), T0 + 1);
    r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
    let (w, _) = thief.device.sign_record(r.clone());
    assert_eq!(a.ingest(&w.encode(), &peer).unwrap_err(), hoi_validate::Reject::BadSignature);
    // even with the device's consent, a device belongs to the first root that blessed it
    let (w, _) = victim.device.sign_record(r);
    assert!(matches!(a.ingest(&w.encode(), &peer), Err(hoi_validate::Reject::NotAllowed(_))));
    // a blessing the root never gave doesn't verify
    let mut forged = Node::delegation_record(&thief.root, thief.device.public(), T0 + 2);
    if let Body::Delegation { root, .. } = &mut forged.body {
        *root = victim.root.public();
    }
    forged.nonce = hoi_crypto::pow::mine(&forged, hoi_types::GLOBAL_POW_BITS);
    let (w, _) = thief.device.sign_record(forged);
    assert_eq!(a.ingest(&w.encode(), &peer).unwrap_err(), hoi_validate::Reject::BadSignature);
}

#[test]
fn every_author_needs_identity_work() {
    let (_t, mut a, cid, c) = owner();
    let x = Actor::new();
    // a device blessing without an identity behind it
    a.ingest(&x.delegation(T0), &Source::Import).unwrap();
    assert_eq!(x.send(&mut a, rec(cid, None, text_post("hi", "words"), T0)).unwrap_err(), hoi_validate::Reject::NoIdentity);
    a.ingest(&x.identity(T0), &Source::Import).unwrap();
    let have = hoi_crypto::identity::work_bits(&x.root.public(), x.work);
    // a community can ask for more work than this key carries (owners first strengthen their own)
    let need = have.max(a.me().unwrap().identity_bits) + 1;
    let mut l = a.community(&c).unwrap().limits;
    l.ladder.identity_bits = need;
    assert_eq!(a.set_limits(&c, &l).unwrap_err().code, "not-allowed", "an owner can't lock themselves out");
    assert!(a.strengthen_identity(need).unwrap().identity_bits >= need);
    a.set_limits(&c, &l).unwrap();
    assert!(matches!(x.send(&mut a, rec(cid, None, text_post("hi", "words"), T0 + 1)), Err(hoi_validate::Reject::WeakIdentity { .. })));
    l.ladder.identity_bits = have;
    a.set_limits(&c, &l).unwrap();
    x.send(&mut a, rec(cid, None, text_post("hi", "words"), T0 + 2)).unwrap();
}

#[test]
fn a_new_key_can_be_mined_elsewhere_one_try_at_a_time() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut n = node("web", &t, &mem);
    let first = n.draft_identity("lena", &[7; 32]).unwrap();
    // drafting again replaces the key in the making
    let d = n.draft_identity("lena", &[7; 32]).unwrap();
    assert_ne!(first.root_key, d.root_key, "the same stir never makes the same key");
    assert!(d.root.starts_with("did:key:z6Mk"));
    assert_eq!(d.bits, 4);
    assert!(n.me().is_none(), "nothing is kept until the work arrives");

    // what the page's miners do: one try at a time, every output visible
    let root = Id::from_hex(&d.root_key).unwrap();
    let mut m = hoi_crypto::identity::Miner::new(root);
    let bits = |m: &mut hoi_crypto::identity::Miner, w| hoi_crypto::pow::leading_zero_bits(&m.hash(w));
    let work = (0u64..).find(|&w| bits(&mut m, w) >= d.bits).unwrap();
    let weak = (0u64..).find(|&w| bits(&mut m, w) < d.bits).unwrap();
    assert_eq!(n.publish_identity(&d.root_key, weak).unwrap_err().code, "invalid");
    assert_eq!(n.publish_identity(&first.root_key, work).unwrap_err().code, "not-found");
    let me = n.publish_identity(&d.root_key, work).unwrap();
    assert_eq!(me.root_key, d.root_key);
    assert_eq!(me.fingerprint, d.fingerprint);
    assert!(me.identity_bits >= d.bits);
    assert_eq!(me.devices.len(), 1);

    // once per device, by either road
    assert_eq!(n.draft_identity("again", &[]).unwrap_err().code, "not-allowed");
    assert_eq!(n.publish_identity(&d.root_key, work).unwrap_err().code, "not-allowed");
    n.commit().unwrap();
    assert_eq!(node("web", &t, &mem).me().unwrap().root_key, d.root_key, "it survives a restart like any key");
}

#[test]
fn more_key_work_can_be_mined_elsewhere() {
    use serde_json::json;
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    let me = n.create_identity_stirred("lena", &[]).unwrap();
    let want = me.identity_bits + 1;
    let root = Id::from_hex(&me.root_key).unwrap();
    let mut m = hoi_crypto::identity::Miner::new(root);
    let bits = |m: &mut hoi_crypto::identity::Miner, w| hoi_crypto::pow::leading_zero_bits(&m.hash(w));
    let weak = (0u64..).find(|&w| bits(&mut m, w) < want).unwrap();
    assert_eq!(n.strengthen_identity_with(want, weak).unwrap_err().code, "invalid", "work is checked, not taken on trust");
    let work = (0u64..).find(|&w| bits(&mut m, w) >= want).unwrap();
    let e = n.call("strengthenIdentity", json!([want, work])).unwrap();
    assert!(serde_json::from_value::<Me>(e).unwrap().identity_bits >= want);
    // without work, the core mines it itself, as before
    assert!(n.call("strengthenIdentity", json!([want])).is_ok());
}

#[test]
fn devices_can_be_renamed_here() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    let me = n.create_identity_stirred("lena", &[]).unwrap();
    assert_eq!(me.devices[0].label, "This computer");
    let me = n.set_device_label(&me.device, "  Studio   Mac ").unwrap();
    assert_eq!(me.devices[0].label, "Studio Mac");
    assert_eq!(n.set_device_label(&me.device, "").unwrap_err().code, "invalid");
    assert_eq!(n.set_device_label(&"ab".repeat(32), "Phone").unwrap_err().code, "not-found", "only your own devices");
}

#[test]
fn favorites_and_order_stay_a_local_view() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("web", &t, &Mem::default());
    let (a, b) = ("ab".repeat(32), "CD".repeat(32));
    let s = n.set_settings(&SettingsPatch { favorites: Some(vec![b.clone(), a.clone(), b.clone()]), ..Default::default() }).unwrap();
    assert_eq!(s.favorites, vec!["cd".repeat(32), a.clone()], "each once, as hex");
    let s = n.set_settings(&SettingsPatch { community_order: Some(vec![a.clone()]), ..Default::default() }).unwrap();
    assert_eq!(s.community_order, vec![a]);
    assert_eq!(s.favorites.len(), 2, "untouched");
    assert_eq!(n.set_settings(&SettingsPatch { favorites: Some(vec!["not an id".into()]), ..Default::default() }).unwrap_err().code, "invalid");
}

#[test]
fn hosts_stir_new_keys_through_the_dispatcher() {
    use hoi_types::uri::b64;
    use serde_json::json;
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("web", &t, &Mem::default());
    let e = n.call("draftIdentity", json!(["lena", b64(&[1; 65])])).unwrap_err();
    assert_eq!(e.code, "invalid", "a stir is a digest, not a recording");
    let e = n.call("draftIdentity", json!(["lena", "not base64!"])).unwrap_err();
    assert_eq!(e.code, "invalid");
    let d: IdentityDraft = serde_json::from_value(n.call("draftIdentity", json!(["lena", b64(&[1; 32])])).unwrap()).unwrap();
    let work = n.call("mineIdentity", json!([d.root_key, d.bits])).unwrap();
    assert_eq!(n.call("mineIdentity", json!([d.root_key, 40])).unwrap_err().code, "invalid", "nobody asks for forever");
    let me: Me = serde_json::from_value(n.call("publishIdentity", json!([d.root_key, work])).unwrap()).unwrap();
    assert_eq!(me.name, "lena");

    // one call, stir or none
    let mut a = node("desktop", &t, &Mem::default());
    let me: Me = serde_json::from_value(a.call("createIdentity", json!(["mira", b64(&[2; 32])])).unwrap()).unwrap();
    assert!(me.identity_bits >= 4);
    let mut b = node("desktop", &t, &Mem::default());
    assert!(b.call("createIdentity", json!(["tobi"])).is_ok());
}

#[test]
fn backdating_doesnt_dodge_the_rate_limit() {
    let (t, mut a, cid, _c) = owner();
    let now = T0 + 3 * DAY;
    t.store(now, Ordering::SeqCst);
    let x = Actor::new();
    x.intro(&mut a, now);
    // ten posts backdated an hour apart arrive as "catch-up", which used to skip the limit
    let ok = (0..10u64)
        .filter(|i| x.send(&mut a, rec(cid, None, text_post(&format!("old news {i}"), "from long ago, honestly"), now - 3600 * (i + 1))).is_ok())
        .count();
    assert_eq!(ok, 5, "a newcomer posts 5 a day here, whenever they claim to have written them");
}

#[test]
fn copy_paste_floods_bounce() {
    let (_t, mut a, cid, _c) = owner();
    let words = "Get rich quick with this one weird trick, today only";
    let results: Vec<bool> = (0..3)
        .map(|i| {
            let x = Actor::new();
            x.intro(&mut a, T0);
            x.send(&mut a, rec(cid, None, text_post("opportunity", words), T0 + i)).is_ok()
        })
        .collect();
    assert_eq!(results, vec![true, true, false], "the third newcomer with the same words is a flood");
}

#[test]
fn a_burned_identity_takes_everything_with_it() {
    let (_t, mut a, cid, c) = owner();
    let x = Actor::new();
    x.intro(&mut a, T0);
    let p = x.send(&mut a, rec(cid, None, text_post("hi", "my words"), T0)).unwrap();
    let mut burn = Record {
        v: 1,
        community: None,
        author: x.root.public(),
        parent: None,
        created_at: T0 + 5,
        expires_at: T0 + hoi_validate::KEEP,
        body: Body::Revoke { device: x.root.public() },
        nonce: 0,
    };
    burn.nonce = hoi_crypto::pow::mine(&burn, hoi_types::GLOBAL_POW_BITS);
    a.ingest(&x.root.sign_record(burn).0.encode(), &Source::Peer { peer: "p".into(), live: true }).unwrap();
    assert!(a.post(&p.hex()).is_err());
    assert_eq!(x.send(&mut a, rec(cid, None, text_post("back", "again"), T0 + 6)).unwrap_err(), hoi_validate::Reject::Burned);
    // and on your own device: burning needs the key's short form, and can't be undone
    let me = a.me().unwrap();
    assert_eq!(a.burn_identity("yes").unwrap_err().code, "invalid");
    a.burn_identity(&me.short).unwrap();
    assert!(a.me().unwrap().burned);
    assert!(a.create_post(&PostDraft { community: c, title: "x".into(), text: "y".into(), link: None, media: None }).is_err());
}

#[test]
fn a_genesis_never_changes() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    let me = a.create_identity("mira").unwrap();
    let ck = Keypair::generate().unwrap();
    let genesis = |owner: Id, at: u64| {
        let g = hoi_types::Genesis {
            name: "privacy".into(),
            description: "".into(),
            color: "mint".into(),
            owners: vec![owner],
            threshold: 1,
            rules_text_cid: None,
            limits: hoi_types::Limits::default(),
        };
        let mut r = Record {
            v: 1,
            community: Some(ck.public()),
            author: ck.public(),
            parent: None,
            created_at: at,
            expires_at: at + hoi_validate::KEEP,
            body: Body::Community(g),
            nonce: 0,
        };
        r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
        ck.sign_record(r).0.encode()
    };
    let first = genesis(Id::from_hex(&me.root_key).unwrap(), T0);
    let inv = hoi_types::Invite { community: ck.public(), genesis: Some(first), ..Default::default() };
    a.join(&inv.to_uri()).unwrap();
    // whoever holds the community key later can't rewrite who owns it
    let thief = Actor::new();
    let err = a.ingest(&genesis(thief.root.public(), T0 + 1), &Source::Import).unwrap_err();
    assert!(matches!(err, hoi_validate::Reject::NotAllowed(_)));
    assert_eq!(a.community(&ck.public().hex()).unwrap().owners[0].key, me.root_key);
}

#[test]
fn sending_refuses_smuggled_text() {
    let (_t, mut a, _cid, c) = owner();
    for (title, text) in [
        ("invoice\u{202E}fdp.exe", "x"),
        ("hi", "[my bank](javascript:alert(1))"),
        ("hi", "log in at https://bank.example@evil.example"),
        ("hi", "x\u{E0041}\u{E0042}"),
    ] {
        let err = a.create_post(&PostDraft { community: c.clone(), title: title.into(), text: text.into(), link: None, media: None }).unwrap_err();
        assert_eq!(err.code, "invalid", "{title:?} {text:?}");
    }
}

// ---------------------------------------------------------------- media

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &x in data {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// A 1×1 PNG carrying a location in a text chunk.
fn png_with_location() -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |ty: &[u8], data: &[u8]| {
        b.extend_from_slice(&(data.len() as u32).to_be_bytes());
        b.extend_from_slice(ty);
        b.extend_from_slice(data);
        b.extend_from_slice(&crc32(&[ty, data].concat()).to_be_bytes());
    };
    chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    chunk(b"tEXt", b"Location\x0052.37N 4.89E");
    chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0, 0, 3, 1, 1, 0]);
    chunk(b"IEND", &[]);
    b
}

#[test]
fn media_is_earned_cleaned_and_checked() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    let mut web = node("web", &t, &Mem::default());
    home.create_identity("mira").unwrap();
    web.create_identity("lena").unwrap();
    let c = home.create_community(draft("privacy")).unwrap();
    pair(&mut home, &mut web, &c.id);
    let png = png_with_location();
    let with_media = |c: &str| PostDraft {
        community: c.into(),
        title: "a dot".into(),
        text: "".into(),
        link: None,
        media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&png), alt: Some("one red pixel".into()) }]),
    };
    // a newcomer can't share media
    assert_eq!(web.create_post(&with_media(&c.id)).unwrap_err().code, "not-allowed");
    // the owner can; the location is gone before anything is hashed
    let p = home.create_post(&with_media(&c.id)).unwrap();
    let m = &p.media[0];
    assert!(m.available && m.mime == "image/png" && (m.w, m.h) == (Some(1), Some(1)));
    let blob = hoi_types::uri::unb64(&home.media(&m.hash).unwrap().data).unwrap();
    assert!(!blob.windows(8).any(|w| w == b"Location"));
    assert!(blob.len() < png.len());
    // the browser gets the post, asks for the bytes, checks them and keeps them
    pump(&mut home, &mut web);
    let wp = &web.feed(Some(&c.id), "fresh", None).unwrap()[0];
    assert!(wp.media[0].available, "fetched over the home-node link");
    assert_eq!(hoi_types::uri::unb64(&web.media(&m.hash).unwrap().data).unwrap(), blob);

    // a trusted member's hand-made post whose bytes aren't what it says: held, and a strike
    let (_t2, mut a, cid, c2) = owner();
    let md = Actor::new();
    md.intro(&mut a, T0);
    a.add_mod(&c2, &md.key()).unwrap();
    let gif = b"GIF89a\x02\x00\x02\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff\x2c\x00\x00\x00\x00\x02\x00\x02\x00\x00\x02\x02\x44\x01\x00\x3b".to_vec();
    let hash = Id::hash(&gif);
    let mref = hoi_types::MediaRef { hash, mime: "image/png".into(), size: gif.len() as u64, w: Some(2), h: Some(2), alt: None };
    let body = Body::Post { title: "pic".into(), text: "".into(), link: None, name: None, media: vec![mref] };
    md.send(&mut a, rec(cid, None, body, T0 + 5)).unwrap();
    assert_eq!(a.feed(Some(&c2), "fresh", None).unwrap().len(), 1);
    assert!(a.on_blob(hash, gif), "the hash matched, so the peer isn't at fault");
    assert!(a.feed(Some(&c2), "fresh", None).unwrap().is_empty(), "held once its media failed");
    assert!(a.media(&hash.hex()).is_err());
    // bytes that don't match their hash are the peer's fault
    assert!(!a.on_blob(Id([7; 32]), b"anything".to_vec()));
}

#[test]
fn lowering_a_requirement_doesnt_lose_posts_on_restart() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut a = node("desktop", &t, &mem);
    a.create_identity("mira").unwrap();
    let x = Actor::new();
    let weak = hoi_crypto::identity::work_bits(&x.root.public(), x.work);
    // the genesis asks for more work than x's key carries…
    let strong = a.strengthen_identity(weak + 1).unwrap().identity_bits;
    let mut d = draft("privacy");
    d.limits = Some(LimitsPatch { ladder: Some(LadderPatch { identity_bits: Some(strong), ..Default::default() }), ..cheap_limits() });
    let c = a.create_community(d).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    x.intro(&mut a, T0);
    assert!(matches!(x.send(&mut a, rec(cid, None, text_post("hi", "too weak for now"), T0)), Err(hoi_validate::Reject::WeakIdentity { .. })));
    // …then the owners lower it, and x posts
    let mut l = a.community(&c.id).unwrap().limits;
    l.ladder.identity_bits = weak;
    t.store(T0 + 10, Ordering::SeqCst);
    a.set_limits(&c.id, &l).unwrap();
    let p = x.send(&mut a, rec(cid, None, text_post("hi", "a weaker key, allowed now"), T0 + 20)).unwrap();
    a.commit().unwrap();
    drop(a);
    let mut a = node("desktop", &t, &mem);
    assert!(a.post(&p.hex()).is_ok(), "still there after a restart");
}

// ---------------------------------------------------------------- changes by consent, late records

fn answer(m: &Actor, a: &mut Node, t: &AtomicU64, cid: Id, p: Id, v: hoi_types::Verdict, now: u64) {
    t.store(now, Ordering::SeqCst);
    let reason = (v == hoi_types::Verdict::Veto).then(|| "too strict for us".to_string());
    let mut r = rec(cid, Some(p), Body::Review { verdict: v, seq: now, reason }, now);
    // answers to a proposal last as long as the proposal: they decide it
    r.expires_at = now + hoi_validate::KEEP;
    m.send(a, r).unwrap();
}

fn propose_hide_quorum(a: &mut Node, c: &str, q: u32) -> String {
    let mut l = a.community(c).unwrap().limits;
    l.ladder.hide_quorum = q;
    let LedgerResult::Published { entry } = a.set_limits(c, &l).unwrap() else { panic!("threshold 1") };
    entry.id
}

#[test]
fn limits_change_only_when_the_community_agrees() {
    use hoi_types::Verdict;
    let (t, mut a, cid, c) = owner();
    let ms = members(&mut a, &c, 2);
    let now = T0 + DAY + 10;
    t.store(now, Ordering::SeqCst);
    a.sweep();
    for (i, m) in ms.iter().enumerate() {
        m.send(&mut a, rec(cid, None, text_post(&format!("hello {i}"), "here and reading along"), now + i as u64)).unwrap();
    }
    let before = a.community(&c).unwrap().limits.ladder.hide_quorum;
    let id = propose_hide_quorum(&mut a, &c, before + 1);
    let cv = a.community(&c).unwrap();
    assert_eq!(cv.limits.ladder.hide_quorum, before, "proposed, not in effect");
    let p = &cv.proposals[0];
    assert_eq!((p.status.as_str(), p.electors, p.waiting.len(), p.my_answer.as_deref()), ("open", 3, 2, Some("agree")));
    assert_eq!((p.changes[0].field.as_str(), p.changes[0].to.as_str()), ("hide_quorum", &*(before + 1).to_string()));
    assert_eq!(a.ledger(&c).unwrap().iter().find(|e| e.id == id).unwrap().proposal.as_deref(), Some("open"));
    assert_eq!(a.review(&id, "ok", None).unwrap_err().code, "not-allowed", "the proposer's signature already says yes");
    let pid = Id::from_hex(&id).unwrap();
    answer(&ms[0], &mut a, &t, cid, pid, Verdict::Ok, now + 60);
    assert_eq!(a.community(&c).unwrap().limits.ladder.hide_quorum, before, "one member is around and hasn't answered");
    answer(&ms[1], &mut a, &t, cid, pid, Verdict::Ok, now + 61);
    let cv = a.community(&c).unwrap();
    assert_eq!(cv.limits.ladder.hide_quorum, before + 1, "everyone agreed");
    assert!(cv.proposals[0].unanimous && cv.proposals[0].status == "passed");
    // a member taking it back afterwards changes nothing
    answer(&ms[1], &mut a, &t, cid, pid, Verdict::Veto, now + 62);
    assert_eq!(a.community(&c).unwrap().limits.ladder.hide_quorum, before + 1);
}

#[test]
fn a_device_that_joins_later_takes_what_was_posted_under_older_limits() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    home.create_identity("robin").unwrap();
    let c = home.create_community(draft("garden")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let mut lena = node("web", &t, &Mem::default());
    let mut kim = node("mobile", &t, &Mem::default());
    for (d, name) in [(&mut lena, "lena"), (&mut kim, "kim")] {
        d.create_identity(name).unwrap();
        pair_now(&mut home, d, "home", name);
    }
    fn star(home: &mut Node, lena: &mut Node, kim: &mut Node) {
        pump_star(home, &mut [("lena", lena), ("kim", kim)]);
    }
    star(&mut home, &mut lena, &mut kim);
    lena.join(&lena.discover_communities()[0].invite).unwrap();
    star(&mut home, &mut lena, &mut kim);
    let before = [post(&mut lena, &c.id, "Lena, before the change"), post(&mut home, &c.id, "Robin, before the change")];
    star(&mut home, &mut lena, &mut kim);

    // the owner tightens the stamp and shortens the window; with nobody else established it passes at once
    t.store(T0 + 60, Ordering::SeqCst);
    let mut limits = home.community(&c.id).unwrap().limits;
    limits.pow_bits += 4;
    limits.max_retention = DAY;
    home.set_limits(&c.id, &limits).unwrap();
    star(&mut home, &mut lena, &mut kim);
    assert_eq!(lena.community(&c.id).unwrap().limits.max_retention, DAY);
    t.store(T0 + 120, Ordering::SeqCst);
    let after = post(&mut lena, &c.id, "Lena, after the change");
    star(&mut home, &mut lena, &mut kim);

    // the phone joins later and still gets everything members took under the limits of its day
    t.store(T0 + 700, Ordering::SeqCst);
    kim.join(&kim.discover_communities()[0].invite).unwrap();
    star(&mut home, &mut lena, &mut kim);
    let seen: Vec<String> = kim.feed(Some(&c.id), "fresh", None).unwrap().into_iter().map(|p| p.id).collect();
    for p in before.iter().chain([&after]) {
        assert!(seen.contains(&p.id), "{} reached the phone", p.title);
    }

    // records made since answer to today's limits
    let m = Actor::new();
    m.intro(&mut kim, T0 + 700);
    let now = T0 + 700;
    let mut long = rec(cid, None, text_post("Kept too long", "a window from before the change"), now);
    long.expires_at = now + 2 * DAY;
    let (w, _) = m.sign(long, limits.pow_bits);
    assert_eq!(kim.ingest(&w, &Source::Peer { peer: "p".into(), live: true }).unwrap_err(), hoi_validate::Reject::ExpiryTooFar);
    let cheap = (0..).find_map(|i| {
        let (w, id) = m.sign(rec(cid, None, text_post("Stamped cheaply", "the old difficulty"), now + i), 8);
        let nonce = hoi_types::Wire::decode(&w).unwrap().0.record.nonce;
        (hoi_crypto::pow::stamp_bits(&id, nonce) < limits.pow_bits).then_some(w)
    });
    assert!(kim.ingest(&cheap.unwrap(), &Source::Peer { peer: "p".into(), live: true }).is_err(), "a new record needs today's stamp");
}

#[test]
fn members_who_are_away_dont_hold_a_change_up() {
    use hoi_types::Verdict;
    let (t, mut a, cid, c) = owner();
    let ms = members(&mut a, &c, 2);
    let now = T0 + DAY + 10;
    t.store(now, Ordering::SeqCst);
    a.sweep();
    // only the first member has anything here; the second is away
    ms[0].send(&mut a, rec(cid, None, text_post("hello", "here and reading along"), now)).unwrap();
    let before = a.community(&c).unwrap().limits.ladder.hide_quorum;
    let id = propose_hide_quorum(&mut a, &c, before + 1);
    assert_eq!(a.community(&c).unwrap().proposals[0].status, "open");
    answer(&ms[0], &mut a, &t, cid, Id::from_hex(&id).unwrap(), Verdict::Ok, now + 60);
    let cv = a.community(&c).unwrap();
    assert_eq!((cv.proposals[0].status.as_str(), cv.proposals[0].away), ("in-effect", 1));
    assert_eq!(cv.limits.ladder.hide_quorum, before + 1);
    // the second member comes back and objects: the change stops until the deadline
    ms[1].send(&mut a, rec(cid, None, text_post("back", "and I don't like this"), now + 100)).unwrap();
    answer(&ms[1], &mut a, &t, cid, Id::from_hex(&id).unwrap(), Verdict::Veto, now + 101);
    let cv = a.community(&c).unwrap();
    assert_eq!(cv.proposals[0].status, "open");
    assert_eq!(cv.proposals[0].objecting[0].reason, "too strict for us");
    assert_eq!(cv.limits.ladder.hide_quorum, before);
}

#[test]
fn owners_retire_an_elector_who_left_for_good() {
    use hoi_types::Verdict;
    // answers stay open half a year here, long enough for someone to be gone for good
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    a.create_identity("mira").unwrap();
    let mut d = draft("privacy");
    d.limits.as_mut().unwrap().max_retention = Some(180 * DAY);
    let cv = a.create_community(d).unwrap();
    let (cid, c) = (Id::from_hex(&cv.id).unwrap(), cv.id);
    let ms = members(&mut a, &c, 2);
    let now = T0 + DAY + 10;
    t.store(now, Ordering::SeqCst);
    a.sweep();
    ms[0].send(&mut a, rec(cid, None, text_post("hello", "here and reading along"), now)).unwrap();
    let before = a.community(&c).unwrap().limits.ladder.hide_quorum;
    let id = propose_hide_quorum(&mut a, &c, before + 1);
    answer(&ms[0], &mut a, &t, cid, Id::from_hex(&id).unwrap(), Verdict::Ok, now + 60);
    let cv = a.community(&c).unwrap();
    assert_eq!((cv.proposals[0].status.as_str(), cv.proposals[0].away), ("in-effect", 1), "in effect, but not for good");
    assert!(cv.proposals[0].retirable.is_empty(), "nobody has been silent for a month yet");
    let gone = ms[1].key();
    assert_eq!(a.retire(&c, &gone).unwrap_err().code, "not-allowed");
    assert_eq!(a.retire(&c, &a.me().unwrap().root_key).unwrap_err().code, "not-allowed", "owners can't be retired");

    // a month on, the owner says the second member left for good
    t.store(now + 31 * DAY, Ordering::SeqCst);
    a.sweep();
    let cv = a.community(&c).unwrap();
    assert_eq!(cv.proposals[0].retirable.iter().map(|p| p.key.clone()).collect::<Vec<_>>(), vec![gone.clone()]);
    let LedgerResult::Published { entry } = a.retire(&c, &gone).unwrap() else { panic!("threshold 1") };
    assert_eq!(entry.action, "retire");
    assert_eq!(a.community(&c).unwrap().proposals[0].status, "in-effect", "a week's notice first");

    // after the notice their silence holds nothing up: everyone else agreed, for good
    t.store(now + 39 * DAY, Ordering::SeqCst);
    a.sweep();
    let p = &a.community(&c).unwrap().proposals[0];
    assert_eq!((p.status.as_str(), p.unanimous, p.retired, p.away), ("passed", true, 1, 0));
    assert_eq!(a.community(&c).unwrap().limits.ladder.hide_quorum, before + 1);
}

#[test]
fn backdated_vouches_count_from_when_they_turn_up() {
    let (t, mut a, cid, c) = owner();
    let ms = members(&mut a, &c, 2);
    let x = Actor::new();
    x.intro(&mut a, T0);
    // this device caught up with its peers two days in
    let now = T0 + 2 * DAY;
    t.store(now, Ordering::SeqCst);
    a.state.synced.insert(cid, now);
    // then two members' vouches for x show up, dated the day before: x would be established now
    let claimed = T0 + DAY + 100;
    t.store(now + 3600 * 2, Ordering::SeqCst);
    for m in &ms {
        let r = Record { expires_at: claimed + hoi_validate::KEEP, ..rec(cid, Some(x.root.public()), Body::Vouch { value: 1, seq: 1, reason: None }, claimed) };
        m.send(&mut a, r).unwrap();
    }
    let st = a.standing(&c, Some(&x.key())).unwrap();
    assert_eq!(st.level, "member", "admitted, but counting from when the vouches arrived");
    assert_eq!(st.admitted_at, Some(now + 3600 * 2));
}

#[test]
fn invite_only_communities_hold_newcomers_until_someone_vouches() {
    let (_t, mut a, cid, c) = owner();
    let mut l = a.community(&c).unwrap().limits;
    l.admission = "invite".into();
    a.set_limits(&c, &l).unwrap();
    assert_eq!(a.community(&c).unwrap().limits.admission, "invite", "the owner alone decides while nobody else is established");
    let x = Actor::new();
    x.intro(&mut a, T0);
    x.send(&mut a, rec(cid, None, text_post("hi", "may I come in please"), T0)).unwrap();
    assert!(a.feed(Some(&c), "fresh", None).unwrap().is_empty(), "held until someone vouches");
    a.vouch(&c, &x.key(), 1, None).unwrap();
    assert_eq!(a.feed(Some(&c), "fresh", None).unwrap().len(), 1, "the vouch is the invite");
}

#[test]
fn sharing_holders_is_for_desktops_and_nodes() {
    let t = Arc::new(AtomicU64::new(T0));
    for (platform, allowed) in [("web", false), ("mobile", false), ("desktop", true), ("node", true)] {
        let mut n = node(platform, &t, &Mem::default());
        assert!(!n.settings().share_holders, "off at first on {platform}");
        let s = n.set_settings(&SettingsPatch { share_holders: Some(true), ..Default::default() }).unwrap();
        assert_eq!(s.share_holders, allowed, "{platform}");
    }
}

#[test]
fn less_data_is_for_browsers_and_phones() {
    let t = Arc::new(AtomicU64::new(T0));
    for (platform, allowed) in [("web", true), ("mobile", true), ("desktop", false), ("node", false)] {
        let mut n = node(platform, &t, &Mem::default());
        assert!(!n.settings().less_data, "off at first on {platform}");
        let s = n.set_settings(&SettingsPatch { less_data: Some(true), ..Default::default() }).unwrap();
        assert_eq!(s.less_data, allowed, "{platform}");
    }
}

#[test]
fn records_in_an_older_format_are_preserved_without_being_served() {
    use hoi_types::cbor::Value;
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut first = node("web", &t, &mem);
    first.create_identity("mira").unwrap();
    first.commit().unwrap();
    let old = Value::map([("r", Value::map([("v", Value::U(1)), ("kind", Value::T("post".into()))])), ("sig", Value::B(vec![0; 64]))]).encode();
    let oid = hoi_types::Wire::loose_id(&old).unwrap();
    mem.0.lock().unwrap().0.insert(oid, old);
    let n = node("web", &t, &mem);
    assert_eq!(n.boot().cleaned, 0);
    assert_eq!(n.boot().preserved_records, 1);
    assert!(mem.0.lock().unwrap().0.contains_key(&oid), "retained for compatible software");
    assert!(!n.store.contains(&oid), "unsupported records are not indexed or served");
    assert!(n.me().is_some(), "your identity stays");
    drop(n);
    let reopened = node("web", &t, &mem);
    assert_eq!(reopened.boot().preserved_records, 1);
    assert!(mem.0.lock().unwrap().0.contains_key(&oid), "preservation survives another startup");
}

/// A picture with enough structure to fingerprint, as `format` at `side` pixels.
fn picture(side: u32, format: image::ImageFormat) -> Vec<u8> {
    let img = image::RgbImage::from_fn(side, side, |x, y| {
        let (u, v) = (x as f32 / side as f32, y as f32 / side as f32);
        let ring = (((u - 0.4).powi(2) + (v - 0.6).powi(2)).sqrt() * 18.0).sin();
        image::Rgb([(u * 255.0) as u8, ((ring + 1.0) * 120.0) as u8, (v * 200.0) as u8])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img).write_to(&mut out, format).unwrap();
    out.into_inner()
}

#[test]
fn altered_copies_of_a_blocked_picture_are_refused() {
    let (_t, mut a, _cid, c) = owner();
    let post = |a: &mut Node, data: Vec<u8>| {
        a.create_post(&PostDraft {
            community: c.clone(),
            title: "a picture".into(),
            text: "".into(),
            link: None,
            media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&data), alt: None }]),
        })
    };
    let p = post(&mut a, picture(300, image::ImageFormat::Png)).unwrap();
    // removed as illegal: the bytes go, and the picture's fingerprint is remembered
    a.remove(&p.id, "illegal: a picture").unwrap();
    assert_eq!(a.stats().blocked_pictures, 1);
    // a resized JPEG of the same picture has other bytes, and is refused all the same
    assert_eq!(post(&mut a, picture(240, image::ImageFormat::Jpeg)).unwrap_err().code, "not-allowed");
    // so is the picture turned a quarter or mirrored
    let turn = |f: &dyn Fn(&image::DynamicImage) -> image::DynamicImage| {
        let img = image::load_from_memory(&picture(300, image::ImageFormat::Png)).unwrap();
        let mut out = std::io::Cursor::new(Vec::new());
        f(&img).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    assert_eq!(post(&mut a, turn(&|i| i.rotate90())).unwrap_err().code, "not-allowed");
    assert_eq!(post(&mut a, turn(&|i| i.fliph())).unwrap_err().code, "not-allowed");
    // a different picture is fine
    let other = image::RgbImage::from_fn(256, 256, |x, y| image::Rgb([((x * y) % 256) as u8, (x % 7 * 30) as u8, (255 - y) as u8]));
    let mut b = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(other).write_to(&mut b, image::ImageFormat::Png).unwrap();
    post(&mut a, b.into_inner()).unwrap();
    // hash lists load from text, one PDQ hash per line
    assert_eq!(a.import_blocklist(&format!("# a list\n{},100\n", "ab".repeat(32))).unwrap(), 1);
    assert_eq!(a.import_blocklist("nothing here").unwrap_err().code, "invalid");
}

#[test]
fn nobody_floods_a_proposal_with_answers() {
    use hoi_types::Verdict;
    let (t, mut a, cid, c) = owner();
    let ms = members(&mut a, &c, 1);
    let now = T0 + DAY + 10;
    t.store(now, Ordering::SeqCst);
    a.sweep();
    let pid = Id::from_hex(&propose_hide_quorum(&mut a, &c, 4)).unwrap();
    for i in 0..hoi_moderation::ladder::MAX_ANSWERS as u64 {
        answer(&ms[0], &mut a, &t, cid, pid, if i % 2 == 0 { Verdict::Veto } else { Verdict::None }, now + 60 + i);
    }
    let mut r = rec(cid, Some(pid), Body::Review { verdict: Verdict::Ok, seq: now + 999, reason: None }, now + 999);
    r.expires_at = now + 999 + hoi_validate::KEEP;
    t.store(now + 999, Ordering::SeqCst);
    assert!(matches!(ms[0].send(&mut a, r), Err(hoi_validate::Reject::NotAllowed(_))), "the seventeenth answer is refused");
}

/// Delivers queued messages over `links` (node, its name for the peer, peer node, the peer's
/// name for it) until nothing moves. Within a round small messages overtake big ones, as a
/// one-line request overtakes a picture on its way up a slow link.
fn mesh(nodes: &mut [&mut Node], links: &[(usize, &str, usize, &str)]) {
    for _ in 0..100 {
        let mut q: Vec<(usize, String, Vec<u8>)> = Vec::new();
        for (i, n) in nodes.iter_mut().enumerate() {
            for (peer, m) in n.take_outbox() {
                let &(_, _, to, back) = links.iter().find(|l| l.0 == i && l.1 == peer).expect("a link");
                q.push((to, back.to_string(), m));
            }
        }
        if q.is_empty() {
            break;
        }
        q.sort_by_key(|x| x.2.len());
        for (to, from, m) in q {
            if let Ok(hoi_sync::Msg::BWant { ids }) = hoi_sync::Msg::decode(&m) {
                assert_eq!(ids.len(), 1, "media catch-up requests one blob at a time");
            }
            nodes[to].net_receive(&from, &m);
        }
    }
    for n in nodes.iter_mut() {
        n.commit().unwrap();
    }
}

#[test]
fn pictures_reach_every_device_behind_a_home_node() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut hub = node("node", &t, &Mem::default());
    let mut mira = node("web", &t, &Mem::default());
    let mut lena = node("web", &t, &Mem::default());
    let mut jo = node("web", &t, &Mem::default());
    hub.create_identity("the pi").unwrap();
    mira.create_identity("mira").unwrap();
    lena.create_identity("lena").unwrap();
    jo.create_identity("jo").unwrap();
    hub.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let c = mira.create_community(draft("privacy")).unwrap();
    let pic_post = |n: &mut Node, side: u32| {
        let data = hoi_types::uri::b64(&picture(side, image::ImageFormat::Png));
        n.create_post(&PostDraft {
            community: c.id.clone(),
            title: "a picture".into(),
            text: "".into(),
            link: None,
            media: Some(vec![MediaDraft { data, alt: None }]),
        })
        .unwrap()
    };
    let available = |n: &mut Node, p: &PostSummary| n.post(&p.id).unwrap().summary.media[0].available;
    let links = [(0, "h", 1, "m"), (1, "m", 0, "h"), (2, "h", 1, "l"), (1, "l", 2, "h"), (3, "h", 1, "j"), (1, "j", 3, "h")];

    // mira and lena pair with a fresh home node at the same time: it learns the community from
    // mira's browser, and tells lena's it carries it too
    lena.join(&mira.invite(&c.id).unwrap().web).unwrap();
    let first = pic_post(&mut mira, 300);
    for (n, me) in [(&mut mira, "m"), (&mut lena, "l")] {
        pair_now(&mut hub, n, "h", me);
    }
    mesh(&mut [&mut mira, &mut hub, &mut lena, &mut jo], &links);
    assert!(available(&mut hub, &first) && available(&mut lena, &first), "mira's picture reached lena through the home node");

    // live: lena asks the home node for the picture before it has finished coming up from
    // mira's browser; the home node sends it on once it's there
    let second = pic_post(&mut mira, 280);
    mesh(&mut [&mut mira, &mut hub, &mut lena, &mut jo], &links);
    assert!(available(&mut lena, &second), "the picture followed its post");
    assert_eq!(lena.media(&second.media[0].hash).unwrap().data, mira.media(&second.media[0].hash).unwrap().data);

    // jo turns up later and catches up, pictures included
    jo.join(&mira.invite(&c.id).unwrap().web).unwrap();
    pair_now(&mut hub, &mut jo, "h", "j");
    mesh(&mut [&mut mira, &mut hub, &mut lena, &mut jo], &links);
    assert!(available(&mut jo, &first) && available(&mut jo, &second));
}

#[test]
fn a_browser_looks_at_pictures_outside_its_core() {
    use crate::look::Look;
    let t = Arc::new(AtomicU64::new(T0));
    let mut hub = node("node", &t, &Mem::default());
    let mut mira = node("web", &t, &Mem::default());
    let mut lena = node("web", &t, &Mem::default());
    lena.set_look_elsewhere(true);
    hub.create_identity("the pi").unwrap();
    mira.create_identity("mira").unwrap();
    lena.create_identity("lena").unwrap();
    hub.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let c = mira.create_community(draft("privacy")).unwrap();
    lena.join(&mira.invite(&c.id).unwrap().web).unwrap();
    let draft_with = |data: &[u8]| PostDraft {
        community: c.id.clone(),
        title: "a picture".into(),
        text: "".into(),
        link: None,
        media: Some(vec![MediaDraft { data: hoi_types::uri::b64(data), alt: None }]),
    };
    let links = [(0, "h", 1, "m"), (1, "m", 0, "h"), (2, "h", 1, "l"), (1, "l", 2, "h")];
    let available = |n: &mut Node, p: &PostSummary| n.store.has_blob(&Id::from_hex(&p.media[0].hash).unwrap());

    // the picture arrives and waits for a look; the host's worker looks and hands it back
    let first = mira.create_post(&draft_with(&picture(300, image::ImageFormat::Png))).unwrap();
    for (n, me) in [(&mut mira, "m"), (&mut lena, "l")] {
        pair_now(&mut hub, n, "h", me);
    }
    mesh(&mut [&mut mira, &mut hub, &mut lena], &links);
    assert!(!available(&mut lena, &first), "not kept before it was looked at");
    let wanted = lena.looks_wanted();
    assert_eq!(wanted.len(), 1);
    assert!(lena.looks_wanted().is_empty(), "handed out once");
    let (id, mime, data) = &wanted[0];
    lena.looked(*id, crate::look::look_here(mime, data));
    assert!(available(&mut lena, &first));

    // a picture the worker died on counts as unreadable: refused, and its author struck
    let second = mira.create_post(&draft_with(&picture(250, image::ImageFormat::Jpeg))).unwrap();
    mesh(&mut [&mut mira, &mut hub, &mut lena], &links);
    let (id, _, _) = lena.looks_wanted().remove(0);
    lena.looked(id, Look::Unreadable);
    assert!(!available(&mut lena, &second));
    assert!(lena.state.bad_media.contains(&id));

    // attaching: the composer's look comes first, else the core refuses to guess
    mira.set_look_elsewhere(true);
    let pic = picture(260, image::ImageFormat::Png);
    assert_eq!(mira.create_post(&draft_with(&pic)).unwrap_err().code, "invalid");
    let (hash, _, look) = crate::look::sanitize_and_look(&pic).unwrap();
    mira.looked(hash, look);
    mira.create_post(&draft_with(&pic)).unwrap();
}

#[test]
fn transport_identity_is_secret_persistent_and_reset_on_wipe() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut n = node("desktop", &t, &mem);
    let secret = n.transport_secret().unwrap();
    assert_ne!(secret, n.state.node_id.unwrap().0);
    n.commit().unwrap();
    let state = mem.0.lock().unwrap().1.get("state").unwrap().clone();
    assert!(!String::from_utf8(state).unwrap().contains(&hoi_types::hex_encode(&secret)));
    let mut reopened = node("desktop", &t, &mem);
    assert_eq!(reopened.transport_secret().unwrap(), secret);
    reopened.wipe_all().unwrap();
    assert_ne!(reopened.transport_secret().unwrap(), secret);
}

#[test]
fn invite_routes_survive_restart_and_leave_removes_them() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut owner = node("desktop", &t, &Mem::default());
    owner.create_identity("mira").unwrap();
    let c = owner.create_community(draft("routes")).unwrap();
    let endpoint = Id([9; 32]).hex();
    let route = format!("{endpoint}@127.0.0.1:1234");
    owner.set_transport_address(Some(endpoint.clone()), vec![route.clone()], vec!["https://relay.example".into()]);
    let invite = owner.invite(&c.id).unwrap().uri;
    let parsed = hoi_types::Invite::parse(&invite).unwrap();
    assert!(parsed.peers.contains(&route));
    assert!(parsed.peers.contains(&endpoint));
    let mem = Mem::default();
    let mut guest = node("web", &t, &mem);
    guest.create_identity("lena").unwrap();
    guest.join(&invite).unwrap();
    guest.commit().unwrap();
    let mut reopened = node("web", &t, &mem);
    assert!(reopened.bootstrap_hints().contains(&(route, vec!["https://relay.example".into()])));
    reopened.leave(&c.id).unwrap();
    assert!(reopened.bootstrap_hints().is_empty());
}

#[test]
fn browser_passes_invite_routes_to_paired_home_and_gets_live_routes_back() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut owner = node("desktop", &t, &Mem::default());
    owner.create_identity("mira").unwrap();
    let c = owner.create_community(draft("routes")).unwrap();
    let endpoint = Id([9; 32]).hex();
    owner.set_transport_address(Some(endpoint.clone()), vec![], vec!["https://relay.example".into()]);
    let mut browser = node("web", &t, &Mem::default());
    browser.create_identity("lena").unwrap();
    browser.join(&owner.invite(&c.id).unwrap().uri).unwrap();
    let mut home = node("node", &t, &Mem::default());
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let home_id = Id([10; 32]).hex();
    home.set_transport_address(Some(home_id.clone()), vec![], vec!["https://home-relay.example".into()]);
    browser.pair(&home.net_status().serving.unwrap().pairing).unwrap();
    browser.net_connect("b", Role::Home);
    home.net_connect("a", Role::Client);
    pump(&mut browser, &mut home);
    assert!(home.bootstrap_hints().iter().any(|(p, _)| p == &endpoint));
    // A home node paired a moment ago could be anyone's: its routes count only once it delivered
    // records the browser didn't have.
    let invite_peers = |b: &Node| hoi_types::Invite::parse(&b.invite(&c.id).unwrap().uri).unwrap().peers;
    assert!(!invite_peers(&browser).contains(&home_id));
    assert!(!browser.state.home.as_ref().unwrap().proven);
    let news = post(&mut owner, &c.id, "News the browser hasn't seen");
    home.import_bundle(&owner.export_bundle(&c.id).unwrap()).unwrap();
    pump(&mut browser, &mut home);
    assert!(browser.store.contains(&Id::from_hex(&news.id).unwrap()));
    assert!(browser.state.home.as_ref().unwrap().proven);
    assert!(invite_peers(&browser).contains(&home_id));
}

#[test]
fn a_fresh_home_node_steers_no_routes_until_it_delivers_records() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut browser = node("web", &t, &Mem::default());
    browser.create_identity("lena").unwrap();
    let c = browser.create_community(draft("routes")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    let fp = |ch: &str| ch.repeat(43);
    let pairing = hoi_types::uri::Pair {
        url: String::new(),
        token: "link".into(),
        name: Some("lena's laptop".into()),
        rtc: vec![format!("rtc:{}@192.168.1.20:7844", fp("q"))],
    };
    browser.pair(&pairing.to_uri()).unwrap();
    browser.net_connect("home", Role::Home);
    browser.take_outbox();
    // Whoever answers on the link's address sends routes for our community and a new address.
    let elsewhere = format!("rtc:{}@198.51.100.7:7844", fp("x"));
    let moved = format!("rtc:{}@192.168.1.21:7844", fp("q"));
    let hello = |hints: Vec<hoi_sync::Hints>, routes: Vec<String>| hoi_sync::Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([5; 32]),
        name: None,
        token: Some("own credential".into()),
        communities: vec![cid],
        salt: None,
        hashed: vec![],
        serves: true,
        hints,
        routes,
        features: vec![],
        pairing: Some("next".into()),
        link_key: Some([3; 32]),
        notice_hash: None,
        notice: None,
        holder: None,
        forwards: false,
        rebuilds: false,
    };
    let steer = hoi_sync::Hints { community: cid, peers: vec![Id([66; 32]).hex(), elsewhere.clone()], relays: vec!["https://relay.example".into()] };
    assert!(browser.net_receive("home", &hello(vec![steer.clone()], vec![elsewhere.clone(), moved.clone()]).encode()));
    assert!(browser.bootstrap_hints().is_empty() && browser.rtc_candidates().is_empty(), "nothing to dial on its word yet");
    assert!(!hoi_types::Invite::parse(&browser.invite(&c.id).unwrap().uri).unwrap().peers.contains(&elsewhere));
    let home = browser.state.home.clone().unwrap();
    assert_eq!(home.rtc, vec![moved.clone()], "the paired certificate's new address is taken, another node's isn't");
    assert_eq!((home.token.as_str(), home.next.as_deref()), ("own credential", Some("next")), "the device switched to its own credential");
    assert!(browser.pair(&pairing.to_uri()).is_ok() && browser.state.home.as_ref().unwrap().token == "own credential", "the spent link keeps the pairing");
    // Once it delivered records we lacked, it is our home node: its routes count.
    let x = Actor::new();
    let (w, _) = x.sign(rec(cid, None, text_post("from home", "a post we didn't have"), T0), 8);
    assert!(browser.net_receive("home", &hoi_sync::Msg::Recs { recs: vec![x.identity(T0), x.delegation(T0), w] }.encode()));
    assert!(browser.state.home.as_ref().unwrap().proven);
    assert!(browser.bootstrap_hints().iter().any(|(p, _)| p == &Id([66; 32]).hex()));
    assert_eq!(browser.state.home.as_ref().unwrap().rtc, vec![elsewhere, moved]);
}

#[test]
fn internet_peer_cannot_claim_to_be_home_or_inject_routes() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("mira").unwrap();
    let c = n.create_community(draft("routes")).unwrap();
    n.net_connect("peer-test", Role::Peer);
    let hello = hoi_sync::Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([9; 32]),
        name: None,
        token: None,
        communities: vec![Id::from_hex(&c.id).unwrap()],
        serves: true,
        hints: vec![hoi_sync::Hints { community: Id::from_hex(&c.id).unwrap(), peers: vec![Id([8; 32]).hex()], relays: vec!["https://relay.example".into()] }],
        routes: Vec::new(),
        features: Vec::new(),
        salt: None,
        hashed: vec![],
        pairing: None,
        link_key: None,
        notice_hash: None,
        notice: None,
        holder: None,
        forwards: false,
        rebuilds: false,
    };
    assert!(n.net_receive("peer-test", &hello.encode()));
    assert!(n.bootstrap_hints().is_empty());
    assert_eq!(n.net_status().mode, "iroh");
    n.set_peer_route("peer-test", "direct");
    assert_eq!(n.net_status().connections[0].route, "direct");
    assert_eq!(n.net_status().connections[0].role, "peer");
}

#[test]
fn hello_hint_conformance_vector() {
    let vector: serde_json::Value = serde_json::from_str(include_str!("../../../vectors/hello-hints.json")).unwrap();
    let bytes = hoi_types::hex_decode(vector["msg"].as_str().unwrap()).unwrap();
    let msg = hoi_sync::Msg::decode(&bytes).unwrap();
    assert_eq!(msg.encode(), bytes);
    let hoi_sync::Msg::Hello { hints, .. } = msg else {
        panic!("expected hello");
    };
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].peers, vec![Id([3; 32]).hex()]);
}

#[test]
fn invite_hints_are_bounded_and_reject_unusable_addresses() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("web", &t, &Mem::default());
    n.create_identity("mira").unwrap();
    let c = n.create_community(draft("routes")).unwrap();
    let id = Id::from_hex(&c.id).unwrap();
    let mut peers = vec!["not-an-endpoint".into(), format!("{}@not-a-socket", Id([8; 32]).hex())];
    peers.extend((1u8..40).map(|i| Id([i; 32]).hex()));
    let relays =
        vec!["http://relay.example".into(), "https://user:secret@relay.example".into(), "https://relay.example\n".into(), "https://relay.example".into()];
    n.remember_hints(id, &peers, &relays);
    let hints = n.bootstrap_hints();
    assert_eq!(hints.len(), hoi_sync::MAX_HINT_PEERS);
    assert!(hints.iter().all(|(_, relays)| relays == &vec!["https://relay.example".to_string()]));
    n.remember_hints(Id([99; 32]), &[Id([98; 32]).hex()], &[]);
    assert!(!n.state.bootstrap.contains_key(&Id([99; 32])));
}

#[test]
fn fresh_routes_replace_old_hints_and_departed_communities_hide_last_sync() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("mira").unwrap();
    let c = n.create_community(draft("routes")).unwrap();
    let id = Id::from_hex(&c.id).unwrap();
    let old: Vec<String> = (1u8..=16).map(|i| Id([i; 32]).hex()).collect();
    n.remember_hints(id, &old, &[]);
    let fresh = Id([42; 32]).hex();
    n.remember_hints(id, std::slice::from_ref(&fresh), &[]);
    assert_eq!(n.state.bootstrap[&id].peers[0], fresh);
    assert_eq!(n.state.bootstrap[&id].peers.len(), hoi_sync::MAX_HINT_PEERS);
    n.state.synced.insert(id, T0 - 100);
    n.state.sync_completed.insert(id, T0);
    assert_eq!(n.net_status().last_sync, Some(T0));
    n.leave(&c.id).unwrap();
    assert_eq!(n.net_status().last_sync, None);
}

#[test]
fn transport_does_not_start_with_an_unpersisted_identity() {
    // Storage can become unavailable when the transport identity is first requested.
    #[derive(Clone)]
    struct ToggleStore {
        fail: Arc<AtomicU64>,
        mem: Mem,
    }
    impl Persist for ToggleStore {
        fn load(&mut self) -> Result<Loaded, String> {
            self.mem.load()
        }
        fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
            if self.fail.load(Ordering::SeqCst) != 0 { Err("disk unavailable".into()) } else { self.mem.apply(changes) }
        }
    }
    let fail = Arc::new(AtomicU64::new(0));
    let mut n = Node::open(Box::new(ToggleStore { fail: fail.clone(), mem: Mem::default() }), "desktop", "https://hoi.sh/app/", Box::new(|| T0)).unwrap();
    fail.store(1, Ordering::SeqCst);
    assert!(n.transport_secret().is_err());
    assert!(n.transport_kp.is_none());
    assert!(n.transport_secret().is_err());
    fail.store(0, Ordering::SeqCst);
    assert!(n.transport_secret().is_ok());
}

#[test]
fn catch_up_requests_every_id_beyond_old_five_thousand_limit() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("mira").unwrap();
    let c = n.create_community(draft("pages")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    n.net_connect("peer", Role::Peer);
    let hello = hoi_sync::Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([7; 32]),
        name: None,
        token: None,
        communities: vec![cid],
        serves: false,
        hints: vec![],
        routes: vec![],
        features: vec![],
        salt: None,
        hashed: vec![],
        pairing: None,
        link_key: None,
        notice_hash: None,
        notice: None,
        holder: None,
        forwards: false,
        rebuilds: false,
    };
    assert!(n.net_receive("peer", &hello.encode()));
    n.take_outbox();
    let all: std::collections::BTreeSet<Id> = (0u32..5001).map(|i| Id::hash(&i.to_be_bytes())).collect();
    assert!(n.net_receive("peer", &hoi_sync::Msg::Have { community: cid, ids: all.iter().copied().collect() }.encode()));
    let mut asked = std::collections::BTreeSet::new();
    loop {
        let requests: Vec<Vec<Id>> = n
            .take_outbox()
            .into_iter()
            .filter_map(|(_, bytes)| match hoi_sync::Msg::decode(&bytes).unwrap() {
                hoi_sync::Msg::Want { ids } => Some(ids),
                _ => None,
            })
            .collect();
        if requests.is_empty() {
            break;
        }
        assert_eq!(requests.len(), 1);
        assert!(requests[0].len() <= hoi_sync::BATCH);
        for id in &requests[0] {
            assert!(asked.insert(*id), "duplicate request");
        }
        assert_eq!(n.net_status().last_sync, None, "not caught up before the final page arrives");
        // Isolate request scheduling from signature mining: represent successful arrival of
        // this batch, then drive the same completion path used by Recs.
        n.peers.get_mut("peer").unwrap().wants.get_mut(&cid).unwrap().1.clear();
        t.fetch_add(1, Ordering::SeqCst);
        assert!(n.net_receive("peer", &hoi_sync::Msg::Recs { recs: vec![] }.encode()));
    }
    assert_eq!(asked, all);
    assert_eq!(n.net_status().last_sync, Some(t.load(Ordering::SeqCst)));
    assert_eq!(n.state.synced[&cid], T0, "arrival-time security watermark stays at the initial snapshot");
}

#[test]
fn communities_share_one_catch_up_request_budget_per_peer() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("mira").unwrap();
    let first = Id::from_hex(&n.create_community(draft("first")).unwrap().id).unwrap();
    let second = Id::from_hex(&n.create_community(draft("second")).unwrap().id).unwrap();
    n.net_connect("peer", Role::Peer);
    let hello = hoi_sync::Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([7; 32]),
        name: None,
        token: None,
        communities: vec![first, second],
        serves: false,
        hints: vec![],
        routes: vec![],
        features: vec![],
        salt: None,
        hashed: vec![],
        pairing: None,
        link_key: None,
        notice_hash: None,
        notice: None,
        holder: None,
        forwards: false,
        rebuilds: false,
    };
    assert!(n.net_receive("peer", &hello.encode()));
    n.take_outbox();
    assert!(n.net_receive("peer", &hoi_sync::Msg::Have { community: first, ids: vec![Id([8; 32])] }.encode()));
    assert!(n.net_receive("peer", &hoi_sync::Msg::Have { community: second, ids: vec![Id([9; 32])] }.encode()));
    let wants = |n: &mut Node| {
        n.take_outbox()
            .into_iter()
            .filter_map(|(_, b)| match hoi_sync::Msg::decode(&b).unwrap() {
                hoi_sync::Msg::Want { ids } => Some(ids),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(wants(&mut n), vec![vec![Id([8; 32])]]);
    n.peers.get_mut("peer").unwrap().wants.get_mut(&first).unwrap().1.clear();
    assert!(n.net_receive("peer", &hoi_sync::Msg::Recs { recs: vec![] }.encode()));
    assert_eq!(wants(&mut n), vec![vec![Id([9; 32])]]);
}

#[test]
fn identity_exports_never_transfer_transport_identity() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut source = node("desktop", &t, &Mem::default());
    source.create_identity("mira").unwrap();
    let source_transport = source.transport_secret().unwrap();
    let sealed = source.export_identity("test passphrase").unwrap();
    let plain = hoi_crypto::keystore::open(&sealed, "test passphrase").unwrap();
    let json: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    assert!(json.get("transport").is_none());
    assert!(!String::from_utf8(plain).unwrap().contains(&hoi_types::hex_encode(&source_transport)));
    let mut destination = node("desktop", &t, &Mem::default());
    let destination_transport = destination.transport_secret().unwrap();
    destination.import_identity(&sealed, "test passphrase").unwrap();
    assert_eq!(destination.transport_secret().unwrap(), destination_transport);
    assert_ne!(source_transport, destination_transport);
}

#[test]
fn recovery_key_brings_the_person_back_under_a_new_root() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("web", &t, &Mem::default());
    let me = a.create_identity("mira").unwrap();
    let line = a.recovery_key().unwrap();
    assert!(line.starts_with(hoi_crypto::recovery::PREFIX));
    assert!(hoi_crypto::recovery::looks_like(&line));
    let master = hoi_crypto::recovery::decode(&line).unwrap();
    // the line is the master secret: the first root is derived from it, and its identity
    // commits to the next generation's recovery key
    assert_eq!(hoi_crypto::recovery::root_at(&master, 0).public().hex(), me.root_key);
    assert_ne!(a.root_kp.as_ref().unwrap().secret(), master, "the root is not the line");
    let person = Id::from_hex(&me.root_key).unwrap();
    assert_eq!(a.store.identity(&person).unwrap().rc, Some(hoi_crypto::recovery::commitment(&hoi_crypto::recovery::recovery_at(&master, 1).public())));
    assert_eq!(a.recovery_key().unwrap(), line, "the same line every time, while the device keeps it");
    // a device without the root key (linked) has no line
    let mut phone = node("mobile", &t, &Mem::default());
    let request = phone.link_request("phone").unwrap();
    let approval = a.link_approve(&request).unwrap();
    phone.link_complete(&approval).unwrap();
    assert!(!phone.me().unwrap().has_root);
    assert_eq!(phone.recovery_key().unwrap_err().code, "not-allowed");

    // restoring on a fresh device: the same person, a new root that took over, a new device
    let mut b = node("desktop", &t, &Mem::default());
    assert_eq!(b.restore_identity(&line, "").unwrap_err().code, "invalid");
    assert_eq!(b.restore_identity("hoi-key-1-yyyyyyyy", "mira").unwrap_err().code, "bad-passphrase");
    assert_eq!(b.restore_identity("correct horse battery staple", "mira").unwrap_err().code, "invalid");
    assert!(b.me().is_none(), "a failed restore leaves the device without a key");
    let restored = b.restore_identity(&line, "mira again").unwrap();
    assert_eq!(restored.root_key, me.root_key, "the person is who they were");
    assert!(restored.has_root);
    assert_eq!(restored.name, "mira again");
    assert_ne!(restored.device, me.device);
    let root = b.root_kp.as_ref().unwrap().public();
    assert_eq!(root, hoi_crypto::recovery::root_at(&master, 1).public());
    assert_eq!(b.store.current_root(&person), root);
    assert_eq!(b.store.person_of(&root), person);
    assert_eq!(b.store.rotations_of(&person).len(), 1);
    assert_eq!(b.store.delegation(&Id::from_hex(&restored.device).unwrap()).unwrap().info.root, root, "the new device is blessed by the new root");
    assert_eq!(b.recovery_status().unwrap().rotations, 1);
    assert_eq!(b.restore_identity(&line, "mira").unwrap_err().code, "not-allowed", "one key per device");
    // until it has heard from the network it is still restoring, and keeps the line
    assert!(b.me().unwrap().restoring);
    assert_eq!(b.recovery_key().unwrap(), line);
    assert_eq!(b.forget_recovery_key().unwrap_err().code, "not-allowed");
    // told it was saved, a device forgets the line
    assert!(!a.forget_recovery_key().unwrap().holds_line);
    assert_eq!(a.recovery_key().unwrap_err().code, "not-allowed");

    // the encrypted file is held to the vault's passphrase standard
    assert_eq!(a.export_identity("too short").unwrap_err().code, "invalid");
    assert!(a.export_identity("fifteen chars ok").is_ok());
}

#[test]
fn strangers_cannot_fill_a_node_with_keys() {
    // Identity, device and revocation records have no community and are kept for years. A
    // stranger may not hand them over on their own: only with a community record of the
    // same root in the batch, or for a root this node has heard from. Paired devices and the
    // home node are not strangers.
    let (_t, mut a, cid, _c) = owner();
    a.net_connect("stranger", Role::Peer);
    a.peers.get_mut("stranger").unwrap().said_hello = true;
    let x = Actor::new();
    let src = Source::Peer { peer: "stranger".into(), live: true };
    for w in [x.identity(T0), x.delegation(T0)] {
        let e = a.ingest(&w, &src).unwrap_err();
        assert!(matches!(e, hoi_validate::Reject::NotAllowed(_)), "{e:?}");
        assert_eq!(e.strike(), hoi_validate::Strike::None, "dropped without penalty: an older peer may still send them");
    }
    assert!(a.store.identity(&x.root.public()).is_none());
    // the same two, with the post they vouch for, in one batch: taken, post included
    let (post, pid) = x.sign(rec(cid, None, text_post("hi", "with my papers"), T0), 8);
    let mut results = Vec::new();
    a.ingest_batch(vec![post, x.identity(T0), x.delegation(T0)], &|_| src.clone(), &mut |_, _, r| results.push(r.map(|a| a.id)));
    assert!(results.iter().all(|r| r.is_ok()), "{results:?}");
    assert!(results.iter().any(|r| r.as_ref().ok() == Some(&pid)));
    assert!(a.store.identity(&x.root.public()).is_some());
    // now the root is known: a revocation of one of its devices on its own is fine
    let mut rv = Record {
        v: 1,
        community: None,
        author: x.root.public(),
        parent: None,
        created_at: T0 + 1,
        expires_at: T0 + hoi_validate::KEEP,
        body: Body::Revoke { device: x.device.public() },
        nonce: 0,
    };
    rv.nonce = hoi_crypto::pow::mine(&rv, hoi_types::GLOBAL_POW_BITS);
    a.ingest(&x.root.sign_record(rv).0.encode(), &src).unwrap();
    // a paired device of ours hands over whatever it likes
    let y = Actor::new();
    a.net_connect("mine", Role::Client);
    a.peers.get_mut("mine").unwrap().said_hello = true;
    a.ingest(&y.identity(T0), &Source::Peer { peer: "mine".into(), live: true }).unwrap();
    // and a root blesses at most sixteen live devices
    let z = Actor::new();
    a.ingest(&z.identity(T0), &Source::Import).unwrap();
    for i in 0..crate::node::MAX_LIVE_DEVICES {
        let dev = Keypair::generate().unwrap();
        let mut r = Node::delegation_record(&z.root, dev.public(), T0 + i as u64);
        r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
        a.ingest(&dev.sign_record(r).0.encode(), &Source::Import).unwrap();
    }
    let one_more = Keypair::generate().unwrap();
    let mut r = Node::delegation_record(&z.root, one_more.public(), T0 + 99);
    r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
    assert!(matches!(a.ingest(&one_more.sign_record(r).0.encode(), &Source::Import).unwrap_err(), hoi_validate::Reject::NotAllowed(_)));
}

#[test]
fn a_home_node_says_nothing_before_the_credential_and_strangers_get_no_name() {
    let (t, mut a, cid, _c) = owner();
    let hello_of = |n: &mut Node| {
        n.take_outbox().into_iter().find_map(|(_, bytes)| match hoi_sync::Msg::decode(&bytes).unwrap() {
            hoi_sync::Msg::Hello { name, communities, hints, routes, node, .. } => Some((name, communities, hints, routes, node)),
            _ => None,
        })
    };
    let device_hello = |token: &str| {
        hoi_sync::Msg::Hello {
            v: hoi_sync::SYNC_VERSION,
            node: Id::ZERO,
            name: Some("mira's phone".into()),
            token: Some(token.into()),
            communities: vec![cid],
            salt: None,
            hashed: vec![],
            serves: false,
            hints: vec![],
            routes: vec![],
            features: vec![],
            pairing: None,
            link_key: None,
            notice_hash: None,
            notice: None,
            holder: None,
            forwards: false,
            rebuilds: false,
        }
        .encode()
    };
    // to a device that has not shown a credential: nothing at all, not even the version
    a.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    a.take_outbox();
    a.net_connect("device", Role::Client);
    assert!(a.take_outbox().is_empty());
    // a wrong token: the link ends without a word
    assert!(!a.net_receive("device", &device_hello("nope")));
    assert!(a.take_outbox().is_empty(), "no bye with a reason either");
    // the pairing link's token: the full hello follows, with a credential of the device's own
    let link = a.state.serve_token.clone().unwrap();
    a.net_connect("device2", Role::Client);
    assert!(a.net_receive("device2", &device_hello(&link)));
    let full = a.take_outbox().into_iter().find_map(|(_, bytes)| hoi_sync::Msg::decode(&bytes).ok().filter(|m| matches!(m, hoi_sync::Msg::Hello { .. })));
    let Some(hoi_sync::Msg::Hello { name, communities, token, pairing, link_key, salt, .. }) = full else { panic!("the real hello after the token") };
    assert_eq!(name.as_deref(), Some("mira's computer"));
    assert_eq!(communities, vec![cid]);
    assert!(salt.is_none(), "our own devices hear every community in the clear");
    let credential = token.expect("the device's own credential");
    let next = pairing.expect("the next pairing link to show");
    assert!(credential != link && next != link, "the link is spent");
    assert_eq!(a.state.serve_token.as_deref(), Some(next.as_str()));
    assert_eq!(link_key, a.link_key());
    assert!(!serde_json::to_string(&a.state).unwrap().contains(&credential), "only a hash of the credential is kept");
    // the spent link pairs nobody else; the credential keeps working on a new connection
    a.net_connect("thief", Role::Client);
    assert!(!a.net_receive("thief", &device_hello(&link)));
    a.net_connect("device2-again", Role::Client);
    assert!(a.net_receive("device2-again", &device_hello(&credential)));
    let again = a.take_outbox().into_iter().find_map(|(_, bytes)| match hoi_sync::Msg::decode(&bytes) {
        Ok(hoi_sync::Msg::Hello { token, pairing, .. }) => Some((token, pairing)),
        _ => None,
    });
    assert_eq!(again, Some((None, Some(next))), "no second credential, the same next link");
    // a new pairing link revokes every device, including the links open now
    a.rotate_pairing().unwrap();
    assert!(!a.net_receive("device2-again", &hoi_sync::Msg::Want { ids: vec![] }.encode()));
    a.net_connect("device2-later", Role::Client);
    assert!(!a.net_receive("device2-later", &device_hello(&credential)));
    // a stranger (a member's node, a carrier) gets no name; a browser gives it a fresh id every
    // time, and its communities only as hashes salted for that connection
    let mut b = node("web", &t, &Mem::default());
    b.create_identity("lena").unwrap();
    let mine = Id::from_hex(&b.create_community(draft("garden")).unwrap().id).unwrap();
    b.take_outbox();
    let stranger_hello = |n: &mut Node| {
        n.take_outbox().into_iter().find_map(|(_, bytes)| match hoi_sync::Msg::decode(&bytes).unwrap() {
            hoi_sync::Msg::Hello { name, node, communities, salt, hashed, .. } => Some((name, node, communities, salt, hashed)),
            _ => None,
        })
    };
    b.net_connect("carrier", Role::Peer);
    let (name, first, communities, salt, first_hashes) = stranger_hello(&mut b).unwrap();
    assert!(name.is_none());
    assert!(communities.is_empty(), "no route led here: nothing in the clear");
    assert_eq!(first_hashes, vec![hoi_sync::community_hash(&salt.unwrap(), &mine)]);
    b.net_connect("carrier2", Role::Peer);
    let (_, second, _, _, second_hashes) = stranger_hello(&mut b).unwrap();
    assert_ne!(first, second);
    assert_ne!(Some(first), b.state.node_id);
    assert_ne!(second_hashes, first_hashes, "another connection, other hashes");
    // its own home node still learns its name and stable id
    b.net_connect("home", Role::Home);
    let (name, _, _, _, id) = hello_of(&mut b).unwrap();
    assert_eq!(name.as_deref(), Some("lena's browser"));
    assert_eq!(Some(id), b.state.node_id);
}

#[test]
fn unavailable_media_does_not_starve_later_available_media() {
    let (t, mut n, cid, c) = owner();
    let mut bytes = BTreeMap::new();
    for side in [200, 220] {
        let p = n
            .create_post(&PostDraft {
                community: c.clone(),
                title: format!("picture {side}"),
                text: "".into(),
                link: None,
                media: Some(vec![MediaDraft { data: hoi_types::uri::b64(&picture(side, image::ImageFormat::Png)), alt: None }]),
            })
            .unwrap();
        let hash = Id::from_hex(&p.media[0].hash).unwrap();
        bytes.insert(hash, n.store.blob(&hash).unwrap().clone());
        n.store.remove_blob(&hash);
    }
    n.net_connect("peer", Role::Peer);
    let peer = n.peers.get_mut("peer").unwrap();
    peer.said_hello = true;
    peer.communities.insert(cid);
    n.take_outbox();
    let requests = |n: &mut Node| {
        n.take_outbox()
            .into_iter()
            .filter_map(|(_, b)| match hoi_sync::Msg::decode(&b).unwrap() {
                hoi_sync::Msg::BWant { ids } => Some(ids[0]),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    n.request_missing_media();
    let first = requests(&mut n)[0];
    // Nobody has the first blob. Repeated foreground calls must not flood retries.
    n.request_missing_media();
    assert!(requests(&mut n).is_empty());
    t.fetch_add(121, Ordering::SeqCst);
    n.request_missing_media();
    let second = requests(&mut n)[0];
    assert_ne!(first, second, "a timed-out image yields to an unattempted one");
    assert!(n.on_blob(second, bytes.remove(&second).unwrap()));
    assert!(n.store.has_blob(&second));
}

#[test]
fn stalled_sync_watchdog_tracks_requested_records_not_unrelated_traffic() {
    let (t, mut source, cid, c) = owner();
    let first = post(&mut source, &c, "first record");
    let second = post(&mut source, &c, "second record");
    let first_id = Id::from_hex(&first.id).unwrap();
    let second_id = Id::from_hex(&second.id).unwrap();
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("lena").unwrap();
    n.join(&source.invite(&c).unwrap().uri).unwrap();
    n.net_connect("peer", Role::Peer);
    let hello = hoi_sync::Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([7; 32]),
        name: None,
        token: None,
        communities: vec![cid],
        serves: false,
        hints: vec![],
        routes: vec![],
        features: vec![],
        salt: None,
        hashed: vec![],
        pairing: None,
        link_key: None,
        notice_hash: None,
        notice: None,
        holder: None,
        forwards: false,
        rebuilds: false,
    };
    assert!(n.net_receive("peer", &hello.encode()));
    assert!(!n.sync_stalled("peer"), "an idle peer is healthy");
    assert!(n.net_receive("peer", &hoi_sync::Msg::Have { community: cid, ids: vec![first_id, second_id] }.encode()));
    t.fetch_add(100, Ordering::SeqCst);
    let record = source.store.get(&first_id).unwrap();
    let mut recs: Vec<Vec<u8>> = source.carriers(record).iter().map(|s| s.wire.clone()).collect();
    recs.push(record.wire.clone());
    assert!(n.net_receive("peer", &hoi_sync::Msg::Recs { recs: recs.clone() }.encode()));
    assert!(n.post(&first.id).is_ok());
    t.fetch_add(100, Ordering::SeqCst);
    assert!(!n.sync_stalled("peer"), "actual progress reset the deadline");
    // Replaying the already received record does not advance the missing second record.
    assert!(n.net_receive("peer", &hoi_sync::Msg::Recs { recs }.encode()));
    t.fetch_add(20, Ordering::SeqCst);
    assert!(n.sync_stalled("peer"), "missing record triggers reconnect despite other traffic");
    assert_eq!(n.net_status().last_sync, None, "incomplete catch-up never claims success");
    n.net_disconnect("peer");
    assert!(!n.sync_stalled("peer"));
}

/// Delivers queued messages around a home node and its devices until nothing is left. The home
/// knows each device by its name in `devices`; devices know the home as "home".
fn pump_star(home: &mut Node, devices: &mut [(&str, &mut Node)]) {
    for _ in 0..80 {
        let mut quiet = true;
        for (to, m) in home.take_outbox() {
            quiet = false;
            if let Some((_, d)) = devices.iter_mut().find(|(n, _)| *n == to) {
                d.net_receive("home", &m);
            }
        }
        for (name, d) in devices.iter_mut() {
            for (_, m) in d.take_outbox() {
                quiet = false;
                home.net_receive(name, &m);
            }
        }
        if quiet {
            break;
        }
    }
    home.commit().unwrap();
    for (_, d) in devices.iter_mut() {
        d.commit().unwrap();
    }
}

fn link_asks(n: &mut Node) -> Vec<LinkAsk> {
    n.take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::LinkAsk { ask } => Some(ask),
            _ => None,
        })
        .collect()
}

/// The digits the new device shows, from its latest `linkCheck` event.
fn link_checks(n: &mut Node) -> Vec<String> {
    n.take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::LinkCheck { checks } => Some(checks),
            _ => None,
        })
        .next_back()
        .unwrap_or_default()
}

#[test]
fn a_phone_links_through_the_home_node_with_one_scan() {
    let t = Arc::new(AtomicU64::new(T0));
    // A companion without a profile, the browser that holds the key, a fresh phone.
    let mut home = node("node", &t, &Mem::default());
    let mut browser = node("web", &t, &Mem::default());
    let mut phone = node("web", &t, &Mem::default());
    assert!(home.me().is_none());
    let st = home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    home.set_rtc_routes(vec![format!("rtc:{}@192.168.1.20:7844", "q".repeat(43))]);
    let serving = home.net_status().serving.unwrap();
    assert!(serving.pairing_web.starts_with("https://hoi.sh/app/#/pair?code=") && !serving.pairing_web.contains("192.168"), "{}", serving.pairing_web);
    let scanned = hoi_types::uri::Pair::parse(&serving.pairing_web).unwrap();
    assert!(scanned.rtc.iter().any(|r| r.ends_with("@192.168.1.20:7844")), "the QR carries the LAN route, packed: {}", serving.pairing_web);
    assert!(serving.phone_can_reach);
    assert_eq!(st.serving.unwrap().url, "ws://127.0.0.1:7842");

    browser.create_identity("lena").unwrap();
    let c = browser.create_community(draft("privacy")).unwrap();
    let p = post(&mut browser, &c.id, "Start here");
    browser.pair(&serving.pairing).unwrap();
    browser.net_connect("home", Role::Home);
    home.net_connect("b", Role::Client);
    pump_star(&mut home, &mut [("b", &mut browser)]);
    assert_eq!(browser.net_status().home_node.unwrap().via.as_deref(), Some("websocket"));
    // The browser can show the home node's next pairing link to a phone, routes included; never
    // the credential it got for itself.
    let shown = browser.net_status().home_node.unwrap().pairing_web;
    let read = hoi_types::uri::Pair::parse(&shown).unwrap();
    assert!(read.rtc.iter().any(|r| r.ends_with("@192.168.1.20:7844")), "{shown}");
    assert!(&read.token == home.state.serve_token.as_ref().unwrap() && read.token != browser.state.home.as_ref().unwrap().token);
    let preview = phone.preview_pair(&shown).unwrap();
    assert!(!preview.paired_here && preview.routes.iter().any(|r| r.ends_with("@192.168.1.20:7844")));

    // The phone scans it: pairs, then asks to be linked.
    phone.pair(&shown).unwrap();
    assert!(phone.link_home("Pixel").is_ok(), "asking before the link is up waits for it");
    phone.link_cancel();
    phone.net_connect("home", Role::Home);
    home.net_connect("p", Role::Client);
    pump_star(&mut home, &mut [("b", &mut browser), ("p", &mut phone)]);
    let started = phone.link_home("Pixel").unwrap();
    assert_eq!(started.sent, 1);
    pump_star(&mut home, &mut [("b", &mut browser), ("p", &mut phone)]);
    assert!(link_asks(&mut home).is_empty(), "the home node holds no key and reads nothing");
    let asks = link_asks(&mut browser);
    assert_eq!(asks.len(), 1);
    assert_eq!(link_checks(&mut phone), vec![asks[0].check.clone()], "both devices show the same digits");
    assert_eq!(asks[0].label, "Pixel");
    assert_eq!(browser.link_asks().len(), 1);

    browser.link_answer(asks[0].id, true).unwrap();
    pump_star(&mut home, &mut [("b", &mut browser), ("p", &mut phone)]);
    let me = phone.me().expect("the phone is linked");
    assert_eq!(me.root_key, browser.me().unwrap().root_key);
    assert!(!me.has_root);
    assert!(phone.take_events().iter().any(|e| matches!(e, Event::Linked { .. })));
    pump_star(&mut home, &mut [("b", &mut browser), ("p", &mut phone)]);
    assert!(phone.post(&p.id).is_ok(), "the phone opened the community and caught up");
    assert!(browser.link_asks().is_empty(), "the ask is gone once the device is blessed");
    assert!(browser.me().unwrap().devices.iter().any(|d| d.label == "Pixel"));

    // Another device asks; the person says no. The phone spent the link it scanned: every device
    // of ours shows the next one now.
    let next = browser.net_status().home_node.unwrap().pairing_web;
    assert_ne!(next, shown);
    assert_eq!(phone.net_status().home_node.unwrap().pairing_web, next);
    let mut tablet = node("web", &t, &Mem::default());
    tablet.pair(&next).unwrap();
    tablet.net_connect("home", Role::Home);
    home.net_connect("tab", Role::Client);
    tablet.link_home("Tablet").unwrap();
    pump_star(&mut home, &mut [("b", &mut browser), ("p", &mut phone), ("tab", &mut tablet)]);
    let asks = link_asks(&mut browser);
    assert_eq!(asks.len(), 1);
    assert!(link_asks(&mut phone).is_empty(), "only a device holding the root key is asked");
    browser.link_answer(asks[0].id, false).unwrap();
    pump_star(&mut home, &mut [("b", &mut browser), ("p", &mut phone), ("tab", &mut tablet)]);
    assert!(tablet.me().is_none());
    assert!(tablet.take_events().iter().any(|e| matches!(e, Event::Notice { text } if text.contains("said no"))));
    assert!(browser.link_answer(asks[0].id, true).is_err(), "an answer counts once");
}

#[test]
fn the_desktop_that_holds_the_key_answers_its_own_phone() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut desk = node("desktop", &t, &Mem::default());
    let mut phone = node("web", &t, &Mem::default());
    desk.create_identity("mira").unwrap();
    desk.create_community(draft("privacy")).unwrap();
    let pairing = desk.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing_web;
    phone.pair(&pairing).unwrap();
    phone.net_connect("home", Role::Home);
    desk.net_connect("p", Role::Client);
    pump_star(&mut desk, &mut [("p", &mut phone)]);
    phone.link_home("Phone").unwrap();
    pump_star(&mut desk, &mut [("p", &mut phone)]);
    let asks = link_asks(&mut desk);
    assert_eq!(asks.len(), 1);
    assert_eq!(link_checks(&mut phone), vec![asks[0].check.clone()]);
    // The phone's link drops and comes back: it asks again, and both keep the same digits.
    phone.net_disconnect("home");
    desk.net_disconnect("p");
    phone.net_connect("home", Role::Home);
    desk.net_connect("p", Role::Client);
    pump_star(&mut desk, &mut [("p", &mut phone)]);
    assert!(link_checks(&mut phone).is_empty(), "no new digits on the phone");
    assert!(link_asks(&mut desk).is_empty(), "no second question on the desktop");
    assert_eq!(desk.link_asks().len(), 1);
    assert_eq!(desk.link_asks()[0].check, asks[0].check);
    desk.link_answer(asks[0].id, true).unwrap();
    pump_star(&mut desk, &mut [("p", &mut phone)]);
    assert_eq!(phone.me().unwrap().root_key, desk.me().unwrap().root_key);
    assert_eq!(phone.communities().len(), 1);
}

#[test]
fn the_digits_need_the_nonce_the_request_committed_to() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut desk = node("desktop", &t, &Mem::default());
    let mut phone = node("web", &t, &Mem::default());
    desk.create_identity("mira").unwrap();
    let pairing = desk.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing_web;
    let token = desk.link_key().unwrap();
    phone.pair(&pairing).unwrap();
    phone.net_connect("home", Role::Home);
    desk.net_connect("p", Role::Client);
    pump_star(&mut desk, &mut [("p", &mut phone)]);
    assert_eq!(phone.state.home.as_ref().unwrap().link_key, Some(hoi_types::uri::b64(&token)), "the home node's link key, not a pairing token");
    phone.link_home("Phone").unwrap();
    // The request goes out with a commitment; the desktop answers with its own nonce.
    for (_, m) in phone.take_outbox() {
        desk.net_receive("p", &m);
    }
    for (_, m) in desk.take_outbox() {
        phone.net_receive("home", &m);
    }
    assert!(link_asks(&mut desk).is_empty(), "nothing shows before the reveal");
    let reveal = phone.take_outbox().pop().expect("the phone reveals").1;
    // Another device holding the link key tries another nonce, to steer the digits.
    let hoi_sync::Msg::Link { mailbox, sealed } = hoi_sync::Msg::decode(&reveal).unwrap() else { panic!("a link message") };
    let plain = hoi_crypto::sealed::open("hoi 2026 link box v1", &token, &sealed).unwrap();
    let hoi_types::cbor::Value::M(fields) = hoi_types::cbor::decode(&plain).unwrap() else { panic!("a map") };
    let forged: Vec<_> = fields.into_iter().map(|(k, v)| if k == "n" { (k, hoi_types::cbor::Value::B(vec![9; 32])) } else { (k, v) }).collect();
    let forged = hoi_crypto::sealed::seal("hoi 2026 link box v1", &token, &hoi_types::cbor::Value::M(forged).encode()).unwrap();
    assert!(desk.net_receive("p", &hoi_sync::Msg::Link { mailbox, sealed: forged }.encode()));
    assert!(link_asks(&mut desk).is_empty(), "a nonce the request didn't commit to shows nothing");
    assert!(desk.link_asks().is_empty());
    // The real reveal does.
    assert!(desk.net_receive("p", &reveal));
    let asks = link_asks(&mut desk);
    assert_eq!(asks.len(), 1);
    assert_eq!(link_checks(&mut phone), vec![asks[0].check.clone()]);
}

#[test]
fn a_headless_node_with_its_own_key_stays_out_of_linking() {
    let t = Arc::new(AtomicU64::new(T0));
    // hoi-node makes a key of its own to join communities with, and has nobody to ask.
    let mut home = node("node", &t, &Mem::default());
    home.create_identity("the pi").unwrap();
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let mut browser = node("web", &t, &Mem::default());
    let mut phone = node("web", &t, &Mem::default());
    browser.create_identity("mira").unwrap();
    for (n, d) in [("b", &mut browser), ("p", &mut phone)] {
        pair_now(&mut home, d, "home", n);
    }
    pump_star(&mut home, &mut [("b", &mut browser), ("p", &mut phone)]);
    phone.link_home("Phone").unwrap();
    pump_star(&mut home, &mut [("b", &mut browser), ("p", &mut phone)]);
    assert!(link_asks(&mut home).is_empty() && home.link_asks().is_empty());
    let asks = link_asks(&mut browser);
    assert_eq!(asks.len(), 1);
    assert_eq!(link_checks(&mut phone), vec![asks[0].check.clone()], "one set of digits, from the browser that can ask");
}

#[test]
fn a_stranger_cannot_ask_or_answer_for_your_devices() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut desk = node("desktop", &t, &Mem::default());
    desk.create_identity("mira").unwrap();
    desk.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    // An internet peer sends a link message: not for this node's devices, nothing happens.
    desk.net_connect("x", Role::Peer);
    let hello = hoi_sync::Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([7; 32]),
        name: None,
        token: None,
        communities: vec![],
        serves: false,
        hints: vec![],
        routes: vec![],
        features: vec![hoi_sync::FEATURE_LINK.into()],
        salt: None,
        hashed: vec![],
        pairing: None,
        link_key: None,
        notice_hash: None,
        notice: None,
        holder: None,
        forwards: false,
        rebuilds: false,
    };
    assert!(desk.net_receive("x", &hello.encode()));
    let sealed = hoi_crypto::sealed::seal("hoi 2026 link box v1", b"guess", b"junk").unwrap();
    let m = crate::link::mailbox(b"guess");
    assert!(desk.net_receive("x", &hoi_sync::Msg::Link { mailbox: m, sealed }.encode()));
    assert!(link_asks(&mut desk).is_empty());
    let mut fresh = node("web", &t, &Mem::default());
    assert!(fresh.link_home("x").is_err(), "linking needs a home node first");
}

#[test]
fn invites_point_at_members_met_since() {
    let (t, mut a, cid, c) = owner();
    // A member's desktop that is a public node: only those publish an address.
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lena").unwrap();
    b.set_carry_policy(crate::CarryPolicy::Any, 8);
    b.set_settings(&SettingsPatch { public_node: Some(true), ..Default::default() }).unwrap();
    b.join(&a.invite(&c).unwrap().uri).unwrap();
    let b_ep = Id([0xb0; 32]).hex();
    let other = Id([0xcc; 32]).hex();
    let rtc = format!("rtc:{}@203.0.113.5:7844", "w".repeat(43));
    let elsewhere = format!("rtc:{}@192.0.2.1:7844", "z".repeat(43));
    b.set_transport_address(Some(b_ep.clone()), vec![format!("{b_ep}@10.0.0.9:4433")], vec![]);
    b.set_rtc_routes(vec![rtc.clone()]);
    // B dials A over QUIC; A knows who it is and where it came from.
    a.net_connect("b", Role::Peer);
    a.set_peer_meta(
        "b",
        PeerMeta { transport: "quic".into(), incoming: true, endpoint: Some(b_ep.clone()), addr: Some("203.0.113.5:4433".into()), ..Default::default() },
    );
    b.net_connect("a", Role::Peer);
    // B also claims a route for an endpoint that isn't B, and a WebRTC route at an address its
    // connection didn't come from: neither may stick.
    let mut b_hello = hoi_sync::Msg::decode(&b.take_outbox().pop().unwrap().1).unwrap();
    if let hoi_sync::Msg::Hello { routes, .. } = &mut b_hello {
        routes.insert(0, elsewhere.clone());
        routes.push(format!("{other}@192.0.2.1:9"));
    }
    assert!(a.net_receive("b", &b_hello.encode()));
    pump(&mut a, &mut b);
    let p = post(&mut a, &c, "Hi all");
    pump(&mut a, &mut b);
    b.reply(&p.id, "hello back").unwrap();
    pump(&mut a, &mut b);

    let routes = hoi_types::Invite::parse(&a.invite(&c).unwrap().uri).unwrap().peers;
    assert!(routes.contains(&format!("{b_ep}@203.0.113.5:4433")), "the address B came from: {routes:?}");
    assert!(routes.contains(&format!("{b_ep}@10.0.0.9:4433")), "B's own route for itself: {routes:?}");
    assert!(routes.contains(&rtc), "B's WebRTC route, for browsers: {routes:?}");
    assert!(!routes.iter().any(|r| r.starts_with(&other)), "routes B claims for others are ignored");
    assert!(!routes.contains(&elsewhere), "a WebRTC route away from B's address is ignored: {routes:?}");
    assert!(a.bootstrap_hints().iter().any(|(h, _)| h == &format!("{b_ep}@203.0.113.5:4433")), "A can dial B later");
    assert!(!a.bootstrap_hints().iter().any(|(h, _)| h.starts_with("rtc:")), "WebRTC routes are for browsers");

    // A browser that joins with A's invite can reach B's node even while A is away.
    let mut web = node("web", &t, &Mem::default());
    web.create_identity("kim").unwrap();
    web.join(&a.invite(&c).unwrap().uri).unwrap();
    assert_eq!(web.rtc_candidates(), vec![rtc.clone()]);
    // Routes are remembered with the community and leave with it.
    assert!(a.state.learned.contains_key(&cid));
    t.store(T0 + 31 * 86_400, Ordering::SeqCst);
    assert!(!hoi_types::Invite::parse(&a.invite(&c).unwrap().uri).unwrap().peers.contains(&rtc), "old routes fade");
}

#[test]
fn traffic_shows_amounts_never_content() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    let mut web = node("web", &t, &Mem::default());
    home.create_identity("mira").unwrap();
    web.create_identity("lena").unwrap();
    let c = home.create_community(draft("privacy")).unwrap();
    post(&mut home, &c.id, "Start here");
    pair(&mut home, &mut web, &c.id);
    home.set_peer_meta("b", PeerMeta { transport: "websocket".into(), local: true, incoming: true, ..Default::default() });
    post(&mut home, &c.id, "Second");
    pump(&mut home, &mut web);
    let tr = home.traffic();
    assert!(tr.bytes_in > 0 && tr.bytes_out > 0);
    assert!(tr.records_out >= 2, "{tr:?}");
    assert_eq!(tr.minutes.len(), 60);
    assert_eq!(tr.minutes.iter().map(|m| m.bytes_out).sum::<u64>(), tr.bytes_out);
    assert_eq!(tr.connections.len(), 1);
    assert_eq!(tr.connections[0].role, "client");
    assert_eq!(tr.connections[0].encryption, "stays on this device");
    assert!(web.traffic().records_in >= 2);
    let json = serde_json::to_string(&tr).unwrap();
    assert!(!json.contains("Start here") && !json.contains("Second"), "no content in the traffic view");
    assert!(!tr.relay.napping && tr.relay.on && tr.relay.possible, "native nodes relay by default");
    assert!(!web.traffic().relay.possible);
    web.set_settings(&SettingsPatch { relay: Some(true), ..Default::default() }).unwrap();
    assert!(!web.settings().relay, "a browser can't take connections");
}

#[test]
fn the_desktop_opens_as_the_companion_until_someone_picks_the_full_app() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut desk = node("desktop", &t, &mem);
    assert!(desk.settings().companion, "light by default");
    assert!(!node("web", &t, &Mem::default()).settings().companion && !node("node", &t, &Mem::default()).settings().companion);
    desk.set_settings(&SettingsPatch { companion: Some(false), ..Default::default() }).unwrap();
    desk.commit().unwrap();
    drop(desk);
    let mut desk = node("desktop", &t, &mem);
    assert!(!desk.settings().companion, "the choice of the full app sticks");
    desk.wipe().unwrap();
    assert!(desk.settings().companion, "a wiped computer starts light again");
}

#[test]
fn the_relay_switch_and_daily_cap_turn_members_away() {
    let (t, mut a, cid, c) = owner();
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lena").unwrap();
    b.join(&a.invite(&c).unwrap().uri).unwrap();
    let hello_from_b = |b: &mut Node| {
        b.net_connect("a", Role::Peer);
        b.take_outbox().pop().unwrap().1
    };
    // Relaying off: someone else's node that dials in is turned away.
    a.set_settings(&SettingsPatch { relay: Some(false), ..Default::default() }).unwrap();
    assert!(!a.accepts_members());
    a.net_connect("b", Role::Peer);
    a.set_peer_meta("b", PeerMeta { transport: "quic".into(), incoming: true, ..Default::default() });
    let h = hello_from_b(&mut b);
    assert!(!a.net_receive("b", &h));
    a.net_disconnect("b");
    b.net_disconnect("a");
    // Our own dials are our own sync, never gated.
    a.net_connect("b", Role::Peer);
    let h = hello_from_b(&mut b);
    assert!(a.net_receive("b", &h));
    a.net_disconnect("b");
    b.net_disconnect("a");
    // On, with a tiny cap: the first exchange uses it up, later messages are refused.
    a.set_settings(&SettingsPatch { relay: Some(true), daily_cap_gb: Some(0.1), ..Default::default() }).unwrap();
    assert!(a.accepts_members());
    a.state.relay_day = (T0 / 86_400, 100_000_000);
    assert!(!a.accepts_members() && a.traffic().relay.napping);
    a.net_connect("b", Role::Peer);
    a.set_peer_meta("b", PeerMeta { transport: "quic".into(), incoming: true, ..Default::default() });
    let h = hello_from_b(&mut b);
    assert!(!a.net_receive("b", &h));
    // A new day, a new cap.
    t.store(T0 + 86_400, Ordering::SeqCst);
    assert!(a.accepts_members());
    let _ = cid;
}
