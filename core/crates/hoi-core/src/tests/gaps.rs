//! M0–M2 gaps: search from an index, autobans written to the ledger with their
//! evidence, members found on the same network.
use super::*;
use hoi_types::Kind;
use std::collections::BTreeSet;

#[test]
fn search_finds_words_and_their_beginnings_in_posts_edits_and_comments() {
    let (_t, mut a, _cid, c) = owner();
    let p = post(&mut a, &c, "Which messengers survived the audit");
    a.reply(&p.id, "Signal and the relay one did").unwrap();
    post(&mut a, &c, "Gardening in November");
    let titles = |a: &mut Node, q: &str| a.search(q, None).unwrap().into_iter().map(|p| p.title).collect::<Vec<_>>();
    assert_eq!(titles(&mut a, "messengers"), vec!["Which messengers survived the audit"]);
    assert_eq!(titles(&mut a, "mess"), vec!["Which messengers survived the audit"], "the start of a word");
    assert_eq!(titles(&mut a, "RELAY"), vec!["Which messengers survived the audit"], "a comment counts for its post, any case");
    assert_eq!(titles(&mut a, "audit relay"), vec!["Which messengers survived the audit"], "every word, wherever it is");
    assert!(titles(&mut a, "audit gardening").is_empty());
    assert!(titles(&mut a, "x").is_empty(), "one letter is no word");
    assert_eq!(titles(&mut a, "november").len(), 1);
    // gone from the index when the post goes
    a.delete_own(&p.id).unwrap();
    assert!(titles(&mut a, "messengers").is_empty());
}

#[test]
fn an_autoban_on_a_moderators_device_goes_to_the_ledger_with_its_reports() {
    let (t, mut a, cid, c) = owner();
    let mut l = a.community(&c).unwrap().limits;
    l.autoban_threshold = 1;
    a.set_limits(&c, &l).unwrap();
    // two established members, and a spammer
    let members = members(&mut a, &c, 2);
    t.store(T0 + 2 * DAY, Ordering::SeqCst);
    let spammer = Actor::new();
    spammer.intro(&mut a, T0 + 2 * DAY);
    let now = a.now();
    for m in &members {
        let r = rec(cid, Some(Id::from_hex(&spammer.key()).unwrap()), Body::Report { reason: "spam".into(), evidence: Vec::new() }, now);
        m.send(&mut a, r).unwrap();
    }
    let st = &a.communities[&cid];
    assert!(st.is_banned(&Id::from_hex(&spammer.key()).unwrap()), "the ban is in the ledger, not only bounced here");
    let ban = a
        .store
        .of_kind(&cid, Kind::Ledger)
        .find(|s| matches!(&s.record.body, Body::Ledger(l) if l.action == hoi_types::LedgerAction::Ban))
        .expect("a ban entry");
    let Body::Ledger(l) = &ban.record.body else { unreachable!() };
    assert!(l.evidence.as_ref().is_some_and(|e| !e.is_empty()), "with the reports behind it");
    // once for a person
    let bans = |a: &Node| {
        a.store.of_kind(&cid, Kind::Ledger).filter(|s| matches!(&s.record.body, Body::Ledger(l) if l.action == hoi_types::LedgerAction::Ban)).count()
    };
    let r = rec(cid, Some(Id::from_hex(&spammer.key()).unwrap()), Body::Report { reason: "spam".into(), evidence: Vec::new() }, now + 1);
    let _ = members[0].send(&mut a, r);
    assert_eq!(bans(&a), 1);
}

#[test]
fn members_on_the_same_network_find_each_other_by_beacon() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    a.create_identity("mira").unwrap();
    let c = a.create_community(draft("garden")).unwrap();
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lena").unwrap();
    b.join(&a.invite(&c.id).unwrap().uri).unwrap();
    let mut stranger = node("desktop", &t, &Mem::default());
    stranger.create_identity("kim").unwrap();
    stranger.create_community(draft("other")).unwrap();

    a.transport_secret().unwrap();
    let beacon = a.lan_beacon(7843).expect("a node with a community sends one");
    assert!(crate::lan::parse(&beacon).is_some());
    let endpoint = hoi_crypto::Keypair::from_secret(&a.transport_secret_now().unwrap()).public().hex();
    assert!(!beacon.windows(32).any(|w| hoi_types::hex_encode(w) == endpoint), "no endpoint in the clear");
    let from: std::net::IpAddr = "192.168.1.20".parse().unwrap();
    b.lan_heard(&beacon, from);
    let route = format!("{endpoint}@192.168.1.20:7843");
    assert!(b.bootstrap_hints().iter().any(|(r, _)| *r == route), "a member puts the route on trial");
    stranger.lan_heard(&beacon, from);
    assert!(!stranger.bootstrap_hints().iter().any(|(r, _)| *r == route), "a stranger learns nothing");
    // a beacon from twenty minutes ago is stale, its own beacon is ignored, and off means off
    t.store(T0 + 1300, Ordering::SeqCst);
    let mut c2 = node("desktop", &t, &Mem::default());
    c2.create_identity("jo").unwrap();
    c2.join(&a.invite(&c.id).unwrap().uri).unwrap();
    c2.lan_heard(&beacon, from);
    assert!(!c2.bootstrap_hints().iter().any(|(r, _)| *r == route));
    a.lan_heard(&a.lan_beacon(7843).unwrap(), from);
    assert!(!a.bootstrap_hints().iter().any(|(r, _)| r.starts_with(&endpoint)));
    a.set_lan(false);
    assert!(a.lan_beacon(7843).is_none());
}

/// M1's acceptance, scripted: a flood of 1,000 records from 50 fresh keys,
/// each record sent to all five nodes of a community. None of it shows; every node bounces the
/// keys and reports them; the owner's device bans every key in the ledger with the reports as
/// evidence, one entry a key, on every node within 30 seconds of processing; a moderator's removal
/// is on every node within 5.
#[test]
#[ignore = "a minute in a debug build: the major-release suites run it"]
fn m1_acceptance_a_flood_from_fifty_fresh_keys_at_five_nodes() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut hub = node("desktop", &t, &Mem::default());
    hub.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let mut owner = node("desktop", &t, &Mem::default());
    owner.create_identity("mod").unwrap();
    let c = owner.create_community(draft("m1")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    let mut l = owner.community(&c.id).unwrap().limits;
    l.autoban_threshold = 2;
    owner.set_limits(&c.id, &l).unwrap();
    let names = ["owner", "n1", "n2", "n3", "n4"];
    let mut nodes: Vec<Node> = vec![owner];
    for name in &names[1..] {
        let mut n = node("desktop", &t, &Mem::default());
        n.create_identity(name).unwrap();
        n.join(&nodes[0].invite(&c.id).unwrap().web).unwrap();
        nodes.push(n);
    }
    for (n, name) in nodes.iter_mut().zip(names) {
        pair_now(&mut hub, n, "home", name);
    }
    let pump = |hub: &mut Node, nodes: &mut Vec<Node>| {
        let mut devices: Vec<(&str, &mut Node)> = names.iter().copied().zip(nodes.iter_mut()).collect();
        pump_star(hub, &mut devices);
    };
    pump(&mut hub, &mut nodes);
    // the four members are vouched in and established a day later
    for i in 1..5 {
        let key = nodes[i].me().unwrap().root_key;
        nodes[0].vouch(&c.id, &key, 1, None).unwrap();
    }
    pump(&mut hub, &mut nodes);
    t.store(T0 + 2 * DAY, Ordering::SeqCst);
    for n in nodes.iter_mut() {
        n.sweep();
    }
    pump(&mut hub, &mut nodes);

    // the flood: fifty fresh keys, twenty posts each, every one sent to every node live
    let spammers: Vec<Actor> = (0..50).map(|_| Actor::new()).collect();
    let now = t.load(Ordering::SeqCst);
    let t0 = std::time::Instant::now();
    for (k, a) in spammers.iter().enumerate() {
        for n in nodes.iter_mut() {
            for w in [a.identity(now), a.delegation(now)] {
                let _ = n.ingest(&w, &Source::Import);
            }
        }
        for j in 0..20u64 {
            let r = rec(cid, None, text_post(&format!("CHEAP PILLS {k}-{j}"), "best prices, message me now"), now + j);
            for n in nodes.iter_mut() {
                let _ = a.send(n, r.clone());
            }
        }
    }
    pump(&mut hub, &mut nodes);
    pump(&mut hub, &mut nodes);
    let took = t0.elapsed();
    for (n, name) in nodes.iter_mut().zip(names) {
        let shown = n.feed(Some(&c.id), "new", Some(500)).unwrap().into_iter().filter(|p| p.title.starts_with("CHEAP PILLS")).count();
        assert_eq!(shown, 0, "{name}: no spam shows");
        let st = &n.communities[&cid];
        let banned = spammers.iter().filter(|a| st.is_banned(&Id::from_hex(&a.key()).unwrap())).count();
        assert_eq!(banned, 50, "{name}: every key banned in the ledger");
        let bans: Vec<&hoi_store::Stored> =
            n.store.of_kind(&cid, Kind::Ledger).filter(|s| matches!(&s.record.body, Body::Ledger(l) if l.action == hoi_types::LedgerAction::Ban)).collect();
        assert_eq!(bans.len(), 50, "{name}: one ban a key");
        assert!(bans.iter().all(|s| matches!(&s.record.body, Body::Ledger(l) if l.evidence.as_ref().is_some_and(|e| !e.is_empty()))), "{name}: with evidence");
    }
    assert!(took < std::time::Duration::from_secs(30), "the flood was banned everywhere in {took:?}");
    println!("m1: 50 keys banned on five nodes in {} ms", took.as_millis());

    // a moderator's removal, everywhere within five seconds
    let p = post(&mut nodes[1], &c.id, "A phone number by mistake");
    pump(&mut hub, &mut nodes);
    let t0 = std::time::Instant::now();
    nodes[0].remove(&p.id, "privacy: a phone number").unwrap();
    pump(&mut hub, &mut nodes);
    let took = t0.elapsed();
    let pid = Id::from_hex(&p.id).unwrap();
    for (n, name) in nodes.iter().zip(names) {
        assert!(n.communities[&cid].tombstones.get(&pid).is_some_and(|r| r.effective_at <= n.now()), "{name}: removed");
    }
    assert!(took < std::time::Duration::from_secs(5), "the removal took {took:?}");
    println!("m1: a removal on five nodes in {} ms", took.as_millis());
}

/// Delivers queued messages both ways like [`pump`], counting what went: (`have` lists, `rng`
/// messages, bytes of both).
fn pump_counting(a: &mut Node, b: &mut Node) -> (usize, usize, usize) {
    let (mut haves, mut ranges, mut bytes) = (0, 0, 0);
    let mut count = |m: &[u8]| match hoi_sync::Msg::decode(m) {
        Ok(hoi_sync::Msg::Have { .. }) => {
            haves += 1;
            bytes += m.len();
        }
        Ok(hoi_sync::Msg::Range { .. }) => {
            ranges += 1;
            bytes += m.len();
        }
        _ => {}
    };
    for _ in 0..200 {
        let out_a = a.take_outbox();
        let out_b = b.take_outbox();
        if out_a.is_empty() && out_b.is_empty() {
            break;
        }
        for (_, m) in out_a {
            count(&m);
            assert!(b.net_receive("a", &m), "b dropped a");
        }
        for (_, m) in out_b {
            count(&m);
            assert!(a.net_receive("b", &m), "a dropped b");
        }
    }
    a.commit().unwrap();
    b.commit().unwrap();
    (haves, ranges, bytes)
}

#[test]
fn two_nodes_of_a_thousand_records_reconcile_by_ranges() {
    let (t, mut a, cid, c) = owner();
    for i in 0..1000 {
        post(&mut a, &c, &format!("Post number {i}"));
    }
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lu").unwrap();
    b.join(&a.invite(&c).unwrap().web).unwrap();
    let held = |n: &Node| n.store.in_community(&cid).map(|s| s.id).collect::<BTreeSet<Id>>();
    // first contact: the newcomer holds the genesis, and learns of the other 1,000
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    let (haves, ranges, _) = pump_counting(&mut a, &mut b);
    assert_eq!(haves, 0, "no full lists between nodes that reconcile");
    assert!(ranges > 0);
    assert_eq!(held(&a), held(&b));
    assert!(held(&b).len() > 1000);

    // apart for a while: each side gets records the other lacks
    a.net_disconnect("b");
    b.net_disconnect("a");
    t.store(T0 + 3600, Ordering::SeqCst);
    for i in 0..7 {
        post(&mut a, &c, &format!("Later on a {i}"));
    }
    for i in 0..4 {
        post(&mut b, &c, &format!("Later on b {i}"));
    }
    assert_ne!(held(&a), held(&b));
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    let (haves, ranges, bytes) = pump_counting(&mut a, &mut b);
    assert_eq!(haves, 0);
    assert!(ranges >= 2, "each side learns what it lacks");
    assert_eq!(held(&a), held(&b), "both hold everything");
    // a sixth of the two full lists a `have` each way would be (2 × 1,012 × 33 bytes); about
    // 5 KB when this was written
    assert!(bytes < 2 * 1012 * 33 / 6, "{bytes} bytes of reconciliation");
}

#[test]
fn a_node_that_leaves_and_comes_back_reconciles_afresh_on_the_same_link() {
    let (_t, mut a, cid, c) = owner();
    for i in 0..30 {
        post(&mut a, &c, &format!("Before {i}"));
    }
    let mut b = node("desktop", &_t, &Mem::default());
    b.create_identity("lu").unwrap();
    let invite = a.invite(&c).unwrap().web;
    b.join(&invite).unwrap();
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    pump_counting(&mut a, &mut b);
    let held = |n: &Node| n.store.in_community(&cid).count();
    assert_eq!(held(&b), held(&a));
    // away for a while: the other side knows, and doesn't push what is new
    b.leave(&c).unwrap();
    pump_counting(&mut a, &mut b);
    for i in 0..5 {
        post(&mut a, &c, &format!("While away {i}"));
    }
    b.join(&invite).unwrap();
    let (_, ranges, _) = pump_counting(&mut a, &mut b);
    assert!(ranges > 0, "a new session, not the one from before");
    assert_eq!(held(&b), held(&a), "everything, the posts made while away too");
    // and gone and back at once, before the other side hears of it: leaving dropped the records
    b.leave(&c).unwrap();
    b.join(&invite).unwrap();
    pump_counting(&mut a, &mut b);
    assert_eq!(held(&b), held(&a), "everything again");
}
