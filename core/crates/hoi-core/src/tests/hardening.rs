//! Hardening: bounces that lift and spare records older
//! than the rules, one link ask per device, name proofs in the key file, and
//! identity work checked before the node lock.
use super::*;

use hoi_validate::Reject;

use crate::IdentityWork;

const PASS: &str = "a long enough passphrase";

/// A post whose title hides a direction override: the text rules refuse it (PROTOCOL.md §18.1).
fn smuggled(i: u64) -> Body {
    text_post(&format!("totally normal \u{202e}{i}"), "words")
}

fn evil(live: bool) -> Source {
    Source::Peer { peer: "evil".into(), live }
}

// ------------------------------------------------------------------ bounces

#[test]
fn a_bounce_lifts_on_its_own_and_a_repeat_lasts_longer() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut a = node("desktop", &t, &mem);
    a.create_identity("mira").unwrap();
    let c = a.create_community(draft("privacy")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    let x = Actor::new();
    x.intro(&mut a, T0);
    let root = x.root.public();
    let strike_out = |a: &mut Node, now: u64| {
        for i in 0..4 {
            let (w, _) = x.sign(rec(cid, None, smuggled(i), now), 8);
            assert!(matches!(a.ingest(&w, &evil(true)), Err(Reject::Content(_))));
        }
    };
    strike_out(&mut a, T0);
    assert_eq!(a.stats().bounced_keys, 1);
    let (fine, _) = x.sign(rec(cid, None, text_post("hello", "a fine post"), T0 + 1), 8);
    assert_eq!(a.ingest(&fine, &evil(true)).unwrap_err(), Reject::Banned, "nothing it signs is taken while bounced");
    a.commit().unwrap();

    drop(a);
    let mut a = node("desktop", &t, &mem);
    assert_eq!(a.stats().bounced_keys, 1, "still bounced after a restart");
    // A day later it lifts by itself.
    t.store(T0 + hoi_spam::BOUNCE_FOR, Ordering::SeqCst);
    assert_eq!(a.stats().bounced_keys, 0);
    assert!(a.community(&c.id).unwrap().banned.is_empty());
    let (fine, _) = x.sign(rec(cid, None, text_post("hello", "a fine post"), T0 + hoi_spam::BOUNCE_FOR), 8);
    a.ingest(&fine, &evil(true)).unwrap();

    // Bounced again while the last one is remembered: twice as long.
    let now = T0 + hoi_spam::BOUNCE_FOR + 10;
    t.store(now, Ordering::SeqCst);
    strike_out(&mut a, now);
    let b = a.state.bounced.get(&root).unwrap().clone();
    assert_eq!((b.times, b.lifts_at()), (2, now + 2 * hoi_spam::BOUNCE_FOR));
    // A month after it lifted, it's forgotten.
    t.store(b.lifts_at() + hoi_spam::BOUNCE_MAX, Ordering::SeqCst);
    a.sweep();
    assert!(a.state.bounced.is_empty());
}

#[test]
fn bounces_saved_before_they_lifted_on_their_own_load_and_lift() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut a = node("desktop", &t, &mem);
    a.create_identity("mira").unwrap();
    let c = a.create_community(draft("privacy")).unwrap();
    let x = Id([7; 32]);
    a.bounce(x, Id::from_hex(&c.id).unwrap(), "too many bad records", T0);
    a.commit().unwrap();
    drop(a);
    // As an older build saved it: no `until`, no `times`.
    {
        let mut m = mem.0.lock().unwrap();
        let mut state: serde_json::Value = serde_json::from_slice(&m.1["state"]).unwrap();
        let b = state["bounced"][x.hex().as_str()].as_object_mut().unwrap();
        assert!(b.remove("until").is_some() && b.remove("times").is_some());
        m.1.insert("state".into(), serde_json::to_vec(&state).unwrap());
    }
    let a = node("desktop", &t, &mem);
    assert!(a.bounced(&x).is_some());
    t.store(T0 + hoi_spam::BOUNCE_FOR, Ordering::SeqCst);
    assert!(a.bounced(&x).is_none(), "a bounce that used to be forever lasts a day from when it was made");
}

#[test]
fn records_older_than_the_rules_they_break_earn_no_strike() {
    let (t, mut a, cid, c) = owner();
    let x = Actor::new();
    x.intro(&mut a, T0);
    let root = x.root.public();
    // This device's text rules tightened at T0 + 100, in an update say: records made before
    // that are refused, and nothing more.
    a.rules_since = T0 + 100;
    t.store(T0 + 200, Ordering::SeqCst);
    for i in 0..8 {
        let (w, _) = x.sign(rec(cid, None, smuggled(i), T0 + 10 + i), 8);
        assert!(matches!(a.ingest(&w, &evil(false)), Err(Reject::Content(_))));
    }
    assert_eq!(a.strikes.score(&root, &cid, T0 + 200), 0.0);
    assert_eq!(a.stats().bounced_keys, 0, "refused, never bounced");
    let (w, _) = x.sign(rec(cid, None, smuggled(99), T0 + 150), 8);
    assert!(matches!(a.ingest(&w, &evil(false)), Err(Reject::Content(_))));
    assert!(a.strikes.score(&root, &cid, T0 + 200) > 0.0, "made under them: a strike");

    // The community's limits changed at T0 + 300: a record made before that earns no strike
    // for its rate either.
    t.store(T0 + 300, Ordering::SeqCst);
    a.set_limits(&c, &LimitsDto { newcomer_posts_per_day: 6, ..a.community(&c).unwrap().limits }).unwrap();
    let before = a.strikes.score(&root, &cid, T0 + 300);
    let posts = |a: &mut Node, from: u64, n: u64| -> Vec<Result<Id, Reject>> {
        (0..n)
            .map(|i| {
                let (w, _) = x.sign(rec(cid, None, text_post(&format!("post {from} {i}"), "words"), from + i), 8);
                a.ingest(&w, &evil(true)).map(|a| a.id)
            })
            .collect()
    };
    let early = posts(&mut a, T0 + 250, 8);
    assert_eq!(early.iter().filter(|r| r.is_ok()).count(), 6, "six a day for newcomers");
    assert!(early.iter().filter(|r| r.is_err()).all(|r| r == &Err(Reject::RateLimited)));
    assert_eq!(a.strikes.score(&root, &cid, T0 + 300), before, "limited, not struck");
    let late = posts(&mut a, T0 + 310, 2);
    assert!(late.iter().all(|r| r == &Err(Reject::RateLimited)));
    assert!(a.strikes.score(&root, &cid, T0 + 300) > before, "made under the new limits: struck");
}

// ------------------------------------------------------------------ link asks

fn star(home: &mut Node, b: &mut Node, p: &mut Node, e: &mut Node) {
    pump_star(home, &mut [("b", b), ("p", p), ("e", e)]);
}

#[test]
fn a_token_holder_cannot_push_out_an_ask_on_screen() {
    // The person's browser holds the key and reaches its home node like every other device;
    // someone else holding the pairing token asks again and again, completing each ask.
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("node", &t, &Mem::default());
    home.create_identity("the pi").unwrap();
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let (mut browser, mut phone, mut intruder) = (node("web", &t, &Mem::default()), node("web", &t, &Mem::default()), node("web", &t, &Mem::default()));
    browser.create_identity("mira").unwrap();
    // Each pairing link pairs one device: each pairs with the link the one before left.
    for (n, d) in [("b", &mut browser), ("p", &mut phone), ("e", &mut intruder)] {
        pair_now(&mut home, d, "home", n);
    }
    star(&mut home, &mut browser, &mut phone, &mut intruder);
    phone.link_home("Phone").unwrap();
    star(&mut home, &mut browser, &mut phone, &mut intruder);
    let asked = link_asks(&mut browser);
    assert_eq!(asked.len(), 1);
    assert_eq!(link_checks(&mut phone), vec![asked[0].check.clone()]);
    for i in 0..6 {
        intruder.link_home(&format!("Phone {i}")).unwrap();
        star(&mut home, &mut browser, &mut phone, &mut intruder);
    }
    let shown = browser.link_asks();
    assert_eq!(shown.len(), 4, "the slots fill up");
    assert!(shown.iter().any(|a| a.id == asked[0].id && a.check == asked[0].check), "but the digits on screen stay");
    browser.link_answer(asked[0].id, true).unwrap();
    star(&mut home, &mut browser, &mut phone, &mut intruder);
    assert_eq!(phone.me().unwrap().root_key, browser.me().unwrap().root_key, "and the person's yes still links the phone");
    assert!(intruder.me().is_none());
}

#[test]
fn one_ask_per_device() {
    // The desktop holds the key and is the home node: asks come straight from its clients.
    let t = Arc::new(AtomicU64::new(T0));
    let mut desk = node("desktop", &t, &Mem::default());
    desk.create_identity("mira").unwrap();
    desk.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let (mut phone, mut intruder) = (node("web", &t, &Mem::default()), node("web", &t, &Mem::default()));
    for (n, d) in [("p", &mut phone), ("e", &mut intruder)] {
        pair_now(&mut desk, d, "home", n);
    }
    pump_star(&mut desk, &mut [("p", &mut phone), ("e", &mut intruder)]);
    phone.link_home("Phone").unwrap();
    pump_star(&mut desk, &mut [("p", &mut phone), ("e", &mut intruder)]);
    let phone_ask = link_asks(&mut desk).pop().unwrap();
    for i in 0..5 {
        intruder.link_home(&format!("Phone {i}")).unwrap();
        pump_star(&mut desk, &mut [("p", &mut phone), ("e", &mut intruder)]);
    }
    let asks = desk.link_asks();
    assert_eq!(asks.len(), 2, "one each: the phone's, and the other device's latest");
    assert!(asks.iter().any(|a| a.id == phone_ask.id && a.check == phone_ask.check));
    assert_eq!(vec![asks.iter().find(|a| a.id != phone_ask.id).unwrap().check.clone()], link_checks(&mut intruder));

    // The phone's device key asked again from another connection, with a new commitment:
    // someone replaying it. The digits on screen stay as they are.
    let token = desk.state.serve_token.clone().unwrap();
    let request = hoi_types::cbor::Value::map([
        ("t", hoi_types::cbor::Value::T("link-req".into())),
        ("device", hoi_types::cbor::Value::B(Id::from_hex(&phone_ask.device).unwrap().0.to_vec())),
        ("label", hoi_types::cbor::Value::T("Phone".into())),
    ]);
    let ask = hoi_types::cbor::Value::map([
        ("t", hoi_types::cbor::Value::T("req".into())),
        ("code", hoi_types::cbor::Value::T(format!("hoi:link:{}", hoi_types::uri::b64(&request.encode())))),
        ("c", hoi_types::cbor::Value::B(vec![5; 32])),
    ]);
    let sealed = hoi_crypto::sealed::seal("hoi 2026 link box v1", token.as_bytes(), &ask.encode()).unwrap();
    let before = desk.link_asks.len();
    assert!(desk.net_receive("e", &hoi_sync::Msg::Link { mailbox: crate::link::mailbox(token.as_bytes()), sealed }.encode()));
    assert_eq!(desk.link_asks.len(), before, "no new ask");
    assert!(desk.link_asks().iter().any(|a| a.id == phone_ask.id && a.check == phone_ask.check));
    desk.link_answer(phone_ask.id, true).unwrap();
    pump_star(&mut desk, &mut [("p", &mut phone), ("e", &mut intruder)]);
    assert_eq!(phone.me().unwrap().root_key, desk.me().unwrap().root_key);
}

// ------------------------------------------------------------------ name proofs in the key file

#[test]
fn the_key_file_carries_name_proofs_so_a_restored_device_can_claim_again() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("Robin").unwrap();
    let garden = n.create_community(draft("Garden")).unwrap();
    let unnamed = n.create_community(draft("jardin partagé")).unwrap();
    let registration = n.community_registration(&garden.id).unwrap();
    let publication = n.community_publication(&garden.id).unwrap();
    let file = n.export_identity(PASS).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&hoi_crypto::keystore::open(&file, PASS).unwrap()).unwrap();
    assert_eq!(doc["v"], 2);
    assert_eq!(doc["registrations"].as_array().unwrap().len(), 1, "only names a registry takes have proofs");

    let mut restored = node("web", &t, &Mem::default());
    restored.import_identity(&file, PASS).unwrap();
    let r = restored.community_registration(&garden.id).unwrap();
    assert_eq!((r.name, r.signature), (registration.name, registration.signature));
    let p = restored.community_publication(&garden.id).unwrap();
    assert_eq!(
        (p.name, p.signature, p.protocol_version, p.index_version),
        (publication.name, publication.signature, publication.protocol_version, publication.index_version)
    );
    assert_eq!(restored.community_registration(&unnamed.id).unwrap_err().code, "invalid");

    // The recovery key is the key and nothing else.
    let mut bare = node("web", &t, &Mem::default());
    bare.restore_identity(&n.recovery_key().unwrap(), "Robin").unwrap();
    assert!(bare.community_registration(&garden.id).is_err());
}

#[test]
fn older_key_files_still_import_and_name_proofs_must_hold() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("Robin").unwrap();
    let garden = n.create_community(draft("Garden")).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&hoi_crypto::keystore::open(&n.export_identity(PASS).unwrap(), PASS).unwrap()).unwrap();
    let import = |doc: &serde_json::Value| {
        let cost = hoi_crypto::keystore::Cost { m_kib: 8 * 1024, t: 1, p: 1 };
        let file = hoi_crypto::keystore::seal(&serde_json::to_vec(doc).unwrap(), PASS, cost).unwrap();
        let mut m = node("web", &t, &Mem::default());
        m.import_identity(&file, PASS).unwrap();
        m
    };
    let claims = |m: &Node| (m.community_registration(&garden.id).is_ok(), m.community_publication(&garden.id).is_ok());

    // A file from before the proofs travelled.
    let mut v1 = doc.clone();
    v1["v"] = 1.into();
    v1.as_object_mut().unwrap().retain(|k, _| k != "registrations" && k != "publications");
    let m = import(&v1);
    assert_eq!(m.communities().len(), 1, "imports as it always did");
    assert_eq!(claims(&m), (false, false));

    // Proofs that don't hold are skipped: a damaged signature, another edition, a community
    // the file doesn't hold.
    let mut bad = doc.clone();
    let sig = bad["registrations"][0]["signature"].as_str().unwrap().to_string();
    let flipped = format!("{}{}", if sig.starts_with('0') { '1' } else { '0' }, &sig[1..]);
    bad["registrations"][0]["signature"] = flipped.into();
    bad["publications"][0]["indexVersion"] = (doc["publications"][0]["indexVersion"].as_u64().unwrap() + 1).into();
    let mut stranger = doc["registrations"][0].clone();
    stranger["community"] = Id([3; 32]).hex().into();
    bad["registrations"].as_array_mut().unwrap().push(stranger);
    let m = import(&bad);
    assert_eq!(claims(&m), (false, false));
    assert!(m.state.registration_proofs.is_empty() && m.state.publication_proofs.is_empty());

    // Only the first 64 of each are read.
    let mut many = doc.clone();
    let mut junk = vec![serde_json::json!({ "name": "x", "community": Id([3; 32]).hex(), "signature": "00" }); 64];
    junk.push(doc["registrations"][0].clone());
    many["registrations"] = junk.into();
    assert_eq!(claims(&import(&many)), (false, true));
    assert_eq!(claims(&import(&doc)), (true, true));
}

#[test]
fn a_founder_restored_from_the_key_file_still_asks_public_nodes_to_carry() {
    // A founder's device asks public nodes, unasked, to carry the open communities it published;
    // it knows them by their publication proofs, so a restore must bring those back.
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("web", &t, &Mem::default());
    n.create_identity("Robin").unwrap();
    let garden = n.create_community(draft("Garden")).unwrap();
    let mut restored = node("web", &t, &Mem::default());
    restored.import_identity(&n.export_identity(PASS).unwrap(), PASS).unwrap();
    let mut carrier = node("node", &t, &Mem::default());
    carrier.set_node_name(Some("public node".into()));
    carrier.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    carrier.set_carry_policy(crate::CarryPolicy::Any, 8);
    carrier.net_connect("b", Role::Peer);
    carrier.set_peer_meta("b", PeerMeta { transport: "webrtc".into(), incoming: true, ..Default::default() });
    restored.net_connect("a", Role::Peer);
    restored.set_peer_meta("a", PeerMeta { transport: "webrtc".into(), ..Default::default() });
    pump(&mut carrier, &mut restored);
    assert_eq!(carrier.carried_on_request(), vec![garden.id.clone()]);
}

// ------------------------------------------------------------------ identity work outside the lock

fn recs(wires: Vec<Vec<u8>>) -> Vec<u8> {
    hoi_sync::Msg::Recs { recs: wires }.encode()
}

/// A stranger's connection that said hello.
fn stranger(a: &mut Node, peer: &str) {
    a.net_connect(peer, Role::Peer);
    a.peers.get_mut(peer).unwrap().said_hello = true;
}

/// An identity record with any `work`, stamped and signed as it should be.
fn identity_record(root: &Keypair, work: u64, now: u64) -> Vec<u8> {
    let mut r = Record {
        v: 1,
        community: None,
        author: root.public(),
        parent: None,
        created_at: now,
        expires_at: now + hoi_validate::KEEP,
        body: Body::Identity { work, rc: None, of: None },
        nonce: 0,
    };
    r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
    root.sign_record(r).0.encode()
}

/// Work values that fall short of the protocol floor for `root`: made up.
fn forged_work(root: &Keypair, n: usize) -> Vec<u64> {
    (0..).filter(|w| hoi_crypto::identity::work_bits(&root.public(), *w) < hoi_crypto::identity::MIN_BITS).take(n).collect()
}

#[test]
fn identity_work_checked_before_the_lock_is_only_looked_up_under_it() {
    let (_t, mut a, cid, _c) = owner();
    stranger(&mut a, "s");
    let x = Actor::new();
    let (post, pid) = x.sign(rec(cid, None, text_post("hi", "with my papers"), T0), 8);
    let msg = recs(vec![post, x.identity(T0), x.delegation(T0)]);
    let work = IdentityWork::scan(&msg);
    assert_eq!(work.len(), 1, "the identity record, its stamp and signature checked");
    assert!(a.identity_work_wanted("nobody", IdentityWork::scan(&msg)).is_empty(), "nothing for a connection that isn't there");
    let work = a.identity_work_wanted("s", work);
    assert_eq!(work.len(), 1, "a stranger may bring an identity with a post of it");
    // No node is needed here: hosts run this with the lock released.
    let work = work.verify();
    assert!(a.net_receive_checked("s", &msg, work));
    let root = x.root.public();
    assert!(a.store.get(&pid).is_some());
    assert_eq!(a.store.identity(&root).map(|i| i.bits), Some(hoi_crypto::identity::work_bits(&root, x.work)));
    assert!(a.checked_work.is_empty(), "answers live only while their message is taken in");
    // Verified once is verified: another copy of the same work needs no run.
    assert!(a.identity_work_wanted("s", IdentityWork::scan(&recs(vec![x.identity(T0 + 1)]))).is_empty());
    // A stranger's identity on its own isn't taken (PROTOCOL.md §7), so it isn't worth a run either.
    assert!(a.identity_work_wanted("s", IdentityWork::scan(&recs(vec![Actor::new().identity(T0)]))).is_empty());

    // Under the lock the pipeline looks the answer up and goes by it.
    let z = Actor::new();
    a.checked_work.insert((z.root.public(), z.work), Err(Reject::BadIdentity("checked ahead".into())));
    assert_eq!(a.ingest(&z.identity(T0), &Source::Import).unwrap_err(), Reject::BadIdentity("checked ahead".into()));
    a.ingest(&z.identity(T0), &Source::Import).unwrap();
}

#[test]
fn forged_identity_work_checked_ahead_is_refused_and_drops_the_peer() {
    let (_t, mut a, _cid, _c) = owner();
    stranger(&mut a, "s");
    // However many good records the peer brought before.
    for _ in 0..1000 {
        a.peer_scores.reward("s");
    }
    let x = Actor::new();
    x.intro(&mut a, T0);
    let root = x.root.public();
    // Ten identity records with made-up work for one key this node knows, in one message.
    let msg = recs(forged_work(&x.root, 10).into_iter().enumerate().map(|(i, w)| identity_record(&x.root, w, T0 + i as u64)).collect());
    let work = a.identity_work_wanted("s", IdentityWork::scan(&msg));
    assert_eq!(work.len(), 10);
    let work = work.verify();
    assert_eq!(work.verified(), 1, "one Argon2id run: the first forgery ends it");
    assert!(!a.net_receive_checked("s", &msg, work), "and drops the peer, as before");
    assert_eq!(a.store.identity(&root).map(|i| i.bits), Some(hoi_crypto::identity::work_bits(&root, x.work)), "the real work still counts");
    assert!(a.checked_work.is_empty());
}

#[test]
fn a_peers_batch_ends_at_forged_identity_work() {
    let (_t, mut a, cid, _c) = owner();
    stranger(&mut a, "s");
    let x = Actor::new();
    x.intro(&mut a, T0);
    let mut wires: Vec<Vec<u8>> = forged_work(&x.root, 3).into_iter().map(|w| identity_record(&x.root, w, T0)).collect();
    wires.push(x.sign(rec(cid, None, text_post("hi", "still here"), T0), 8).0);
    let src = Source::Peer { peer: "s".into(), live: false };
    let mut results = Vec::new();
    a.ingest_batch(wires.clone(), &|_| src.clone(), &mut |_, _, r| results.push(r.map(|a| a.id)));
    assert_eq!(results.len(), 1, "nothing after the first forgery is worth another run");
    assert!(matches!(results[0], Err(Reject::BadIdentity(_))));
    // From an import there is no peer to drop: everything is looked at.
    results.clear();
    a.ingest_batch(wires, &|_| Source::Import, &mut |_, _, r| results.push(r.map(|a| a.id)));
    assert_eq!(results.len(), 4);
    assert!(results[3].is_ok());
}
