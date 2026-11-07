//! Recovery without losing standing: the recovery line replaces a person's root
//! and they keep their place; the old root and its devices sign nothing that counts from then
//! on; friends bring the line back, three of five.
use super::*;

fn hub(t: &Arc<AtomicU64>) -> Node {
    let mut h = node("desktop", t, &Mem::default());
    h.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    h
}

fn person_of(me: &Me) -> Id {
    Id::from_hex(&me.root_key).unwrap()
}

fn feed_titles(n: &mut Node, c: &str) -> Vec<String> {
    n.feed(Some(c), "new", Some(50)).unwrap().into_iter().map(|p| p.title).collect()
}

#[test]
fn a_new_root_keeps_the_persons_standing_and_the_old_one_is_shut_out() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = hub(&t);
    let mut robin = node("desktop", &t, &Mem::default());
    robin.create_identity("robin").unwrap();
    let c = robin.create_community(draft("garden")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    let mut mira = node("web", &t, &Mem::default());
    let me = mira.create_identity("mira").unwrap();
    let person = person_of(&me);
    mira.join(&robin.invite(&c.id).unwrap().web).unwrap();
    pair_now(&mut home, &mut robin, "home", "robin");
    pair_now(&mut home, &mut mira, "home", "mira");
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira)]);
    post(&mut mira, &c.id, "Mira, first");
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira)]);
    robin.vouch(&c.id, &me.root_key, 1, None).unwrap();
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira)]);
    assert!(robin.communities[&cid].is_admitted(&person), "robin vouched mira in");
    let line = mira.recovery_key().unwrap();

    // mira's laptop is gone; she brings her line back on a new one
    t.store(T0 + 3600, Ordering::SeqCst);
    let mut back = node("desktop", &t, &Mem::default());
    let restored = back.restore_identity(&line, "mira").unwrap();
    assert_eq!(restored.root_key, me.root_key);
    back.join(&robin.invite(&c.id).unwrap().web).unwrap();
    pair_now(&mut home, &mut back, "home", "back");
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira), ("back", &mut back)]);
    t.store(T0 + 3700, Ordering::SeqCst);
    let new = post(&mut back, &c.id, "Mira, from the new laptop");
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira), ("back", &mut back)]);

    // the new laptop heard from the network, so it is done restoring; the old one knows it
    // was replaced
    assert!(!back.me().unwrap().restoring);
    assert!(back.me().unwrap().holds_line, "it keeps the line until told it was saved");
    assert!(mira.me().unwrap().replaced, "the old laptop's root was replaced");
    assert!(!back.me().unwrap().replaced);

    // robin sees the same person, with the standing she had
    let new_root = back.root_kp.as_ref().unwrap().public();
    assert_eq!(robin.store.current_root(&person), new_root, "the rotation reached robin with the post");
    assert_eq!(robin.store.get(&Id::from_hex(&new.id).unwrap()).unwrap().author_root, person);
    assert!(robin.communities[&cid].is_admitted(&person), "the standing stayed with the person");
    assert!(feed_titles(&mut robin, &c.id).contains(&"Mira, from the new laptop".to_string()));

    // whoever has the old laptop: what it signs from now on counts nowhere
    t.store(T0 + 3800, Ordering::SeqCst);
    let old = mira.create_post(&PostDraft { community: c.id.clone(), title: "Not mira".into(), text: "x".into(), link: None, media: None });
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira), ("back", &mut back)]);
    if let Ok(old) = old {
        assert!(robin.store.get(&Id::from_hex(&old.id).unwrap()).is_none(), "the replaced root's device is refused");
    }
    assert!(!feed_titles(&mut robin, &c.id).contains(&"Not mira".to_string()));
    // and what mira posted before stays
    assert!(feed_titles(&mut robin, &c.id).contains(&"Mira, first".to_string()));

    // the key file of the new root carries the chain to another device
    let file = back.export_identity("a long enough passphrase").unwrap();
    let mut third = node("desktop", &t, &Mem::default());
    let moved = third.import_identity(&file, "a long enough passphrase").unwrap();
    assert_eq!(moved.root_key, me.root_key);
    assert_eq!(third.store.current_root(&person), new_root);
    assert!(!third.me().unwrap().replaced);
}

#[test]
fn a_root_that_commits_twice_is_burned_never_taken_over() {
    // A thief with mira's root signs a second identity record committing to their own recovery
    // key: the root can then be replaced by nobody and signs nothing more.
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    let me = a.create_identity("mira").unwrap();
    let person = person_of(&me);
    let thief = hoi_crypto::Keypair::generate().unwrap();
    let root = a.root_kp.clone().unwrap();
    let work = hoi_crypto::identity::mine(&person, hoi_crypto::identity::MIN_BITS);
    let forged = Record {
        v: 1,
        community: None,
        author: person,
        parent: None,
        created_at: T0 + 10,
        expires_at: T0 + 10 + hoi_validate::KEEP,
        body: Body::Identity { work, rc: Some(hoi_crypto::recovery::commitment(&thief.public())), of: None },
        nonce: 0,
    };
    a.publish_as(forged, &root).unwrap();
    assert!(a.store.burned_at(&person).is_some(), "two commitments burn the root");
    // the thief's rotation is refused
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("thief").unwrap();
    let rotate = Record {
        v: 1,
        community: None,
        author: thief.public(),
        parent: None,
        created_at: T0 + 20,
        expires_at: T0 + 20 + hoi_validate::KEEP,
        body: Body::Rotate { person, to: b.root_kp.as_ref().unwrap().public() },
        nonce: 0,
    };
    assert!(a.publish_as(rotate, &thief).is_err());
}

#[test]
fn three_of_five_friends_bring_the_line_back() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = hub(&t);
    let mut mira = node("web", &t, &Mem::default());
    let me = mira.create_identity("mira").unwrap();
    let c = mira.create_community(draft("garden")).unwrap();
    pair_now(&mut home, &mut mira, "home", "mira");
    let names = ["ana", "ben", "cai", "dev", "eli"];
    let mut friends: Vec<Node> = names
        .iter()
        .map(|n| {
            let mut f = node("web", &t, &Mem::default());
            f.create_identity(n).unwrap();
            f.join(&mira.invite(&c.id).unwrap().web).unwrap();
            pair_now(&mut home, &mut f, "home", n);
            f
        })
        .collect();
    let pump_all = |home: &mut Node, mira: &mut Node, friends: &mut Vec<Node>| {
        let mut devices: Vec<(&str, &mut Node)> = vec![("mira", mira)];
        for (name, f) in names.iter().zip(friends.iter_mut()) {
            devices.push((name, f));
        }
        pump_star(home, &mut devices);
    };
    pump_all(&mut home, &mut mira, &mut friends);
    for (f, n) in friends.iter_mut().zip(names) {
        post(f, &c.id, &format!("{n} says hi"));
    }
    pump_all(&mut home, &mut mira, &mut friends);
    let keys: Vec<String> = friends.iter().map(|f| f.me().unwrap().root_key).collect();

    // shares: three to seven different people, not mira herself
    assert_eq!(mira.give_shares(&c.id, &keys[..2]).unwrap_err().code, "invalid");
    assert_eq!(mira.give_shares(&c.id, &[keys[0].clone(), keys[0].clone(), keys[1].clone()]).unwrap_err().code, "invalid");
    assert_eq!(mira.give_shares(&c.id, &[me.root_key.clone(), keys[0].clone(), keys[1].clone()]).unwrap_err().code, "invalid");
    let status = mira.give_shares(&c.id, &keys).unwrap();
    assert_eq!(status.friends.len(), 5);
    pump_all(&mut home, &mut mira, &mut friends);
    for f in &friends {
        let held = f.held_shares().unwrap();
        assert_eq!(held.len(), 1, "each friend keeps one share");
        assert_eq!(held[0].person, me.root_key);
    }
    // the line is not in the records: no single friend, nor the community, can read it
    let line = mira.recovery_key().unwrap();
    mira.forget_recovery_key().unwrap();

    // everything is gone; a new phone asks
    let mut phone = node("mobile", &t, &Mem::default());
    let ask = phone.ask_friends().unwrap();
    let other = node("mobile", &t, &Mem::default()).ask_friends().unwrap();
    assert_ne!(ask.digits, other.digits);
    let mut answers = Vec::new();
    for i in [0, 2, 4] {
        assert_eq!(friends[i].request_digits(&ask.code).unwrap(), ask.digits, "the friend reads the same digits");
        answers.push(friends[i].answer_request(&me.root_key, &ask.code).unwrap().code);
    }
    // an answer to another request is refused, and one friend twice counts once
    let stray = friends[1].answer_request(&me.root_key, &other.code).unwrap().code;
    assert_eq!(phone.take_answer(&stray, "mira").unwrap_err().code, "invalid");
    assert_eq!(phone.take_answer(&answers[0], "mira").unwrap().0, 1);
    assert_eq!(phone.take_answer(&answers[0], "mira").unwrap().0, 1);
    assert_eq!(phone.take_answer(&answers[1], "mira").unwrap().0, 2);
    let (n, restored) = phone.take_answer(&answers[2], "mira").unwrap();
    assert_eq!(n, 3);
    let restored = restored.expect("three answers bring mira back");
    assert_eq!(restored.root_key, me.root_key);
    assert_eq!(phone.recovery_key().unwrap(), line, "the line itself is back");
    assert!(phone.root_kp.is_some());
    assert_eq!(phone.store.rotations_of(&person_of(&me)).len(), 1, "and it replaced the root at once");
}

#[test]
fn more_key_work_after_a_restore_is_mined_for_the_root_that_signs() {
    // A restored key signs with a later root than the person's first. The host mines
    // more work outside the core (You → Key work); it must mine it for the root the core signs
    // the stronger identity with, or the core refuses it as too weak.
    let t = Arc::new(AtomicU64::new(T0));
    let mut mira = node("web", &t, &Mem::default());
    mira.create_identity("mira").unwrap();
    let line = mira.recovery_key().unwrap();
    let mut back = node("web", &t, &Mem::default());
    let me = back.restore_identity(&line, "mira").unwrap();
    let signing = back.root_kp.as_ref().unwrap().public();
    assert_ne!(signing, person_of(&me), "a restore signs with the next root");
    assert_eq!(me.work_key, signing.hex(), "the host is told which root to mine for");

    let want = me.identity_bits + 2;
    let mut m = hoi_crypto::identity::Miner::new(Id::from_hex(&me.work_key).unwrap());
    let work = (0u64..).find(|&w| hoi_crypto::pow::leading_zero_bits(&m.hash(w)) >= want).unwrap();
    let stronger = back.strengthen_identity_with(want, work).unwrap();
    assert!(stronger.identity_bits >= want, "{} bits", stronger.identity_bits);
    assert_eq!(back.store.best_identity(&person_of(&me)).unwrap().bits, stronger.identity_bits, "what communities count");
    // the stronger record commits to what the root's first one did: nothing is burned
    assert!(!stronger.burned && back.store.burned_at(&signing).is_none() && back.store.burned_at(&person_of(&me)).is_none());
    let c = back.create_community(draft("after more work")).unwrap();
    post(&mut back, &c.id, "still me");
    assert!(feed_titles(&mut back, &c.id).contains(&"still me".to_string()), "the key still signs");

    // work for the first root is not work for the one that signs
    let more = stronger.identity_bits + 1;
    let mut first = hoi_crypto::identity::Miner::new(person_of(&me));
    let wrong = (0u64..).find(|&w| hoi_crypto::pow::leading_zero_bits(&first.hash(w)) >= more && hoi_crypto::identity::work_bits(&signing, w) < more);
    assert_eq!(back.strengthen_identity_with(more, wrong.unwrap()).unwrap_err().code, "invalid");

    // asking for what the person already carries mines nothing
    let again = back.call("strengthenIdentity", serde_json::json!([stronger.identity_bits])).unwrap();
    assert_eq!(serde_json::from_value::<Me>(again).unwrap().identity_bits, stronger.identity_bits);
}
