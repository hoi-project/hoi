//! Remote signing: a browser that keeps no signing key; the computer with the
//! person's root keeps it and signs over the home link, posts at once and moderation on a yes.
use super::*;
use crate::Published;

fn signed(n: &mut Node) -> Vec<(String, bool, Option<String>)> {
    n.take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Signed { id, ok, message } => Some((id, ok, message)),
            _ => None,
        })
        .collect()
}

fn mine_draft(n: &mut Node, d: &DraftDto) -> u64 {
    n.call("mine", serde_json::json!([d.prefix, d.suffix, d.bits])).unwrap().as_u64().unwrap()
}

fn sign_asks(n: &mut Node) -> Vec<crate::remote::SignAsk> {
    n.take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::SignAsk { ask } => Some(ask),
            _ => None,
        })
        .collect()
}

#[test]
fn a_browser_with_no_key_posts_through_the_computer_that_keeps_it() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("node", &t, &Mem::default());
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let mut desk = node("desktop", &t, &Mem::default());
    let mut browser = node("web", &t, &Mem::default());
    desk.create_identity("lena").unwrap();
    let c = desk.create_community(draft("privacy")).unwrap();
    post(&mut desk, &c.id, "Start here");
    desk.pair(&home.net_status().serving.unwrap().pairing).unwrap();
    desk.net_connect("home", Role::Home);
    home.net_connect("d", Role::Client);
    pump_star(&mut home, &mut [("d", &mut desk)]);

    // the browser pairs and asks, keeping no key
    browser.pair(&desk.net_status().home_node.unwrap().pairing_web).unwrap();
    browser.net_connect("home", Role::Home);
    home.net_connect("b", Role::Client);
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    browser.link_home_remote("Work browser").unwrap();
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    let asks = link_asks(&mut desk);
    assert_eq!(asks.len(), 1);
    desk.link_answer(asks[0].id, true).unwrap();
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    let me = browser.me().expect("linked");
    assert!(me.remote, "the browser signs through the computer");
    assert_eq!(me.root_key, desk.me().unwrap().root_key);
    assert!(browser.device_kp.is_none() && browser.pending_kp.is_none() && browser.root_kp.is_none(), "no signing key here at all");
    let device = Id::from_hex(&me.device).unwrap();
    assert!(desk.me().unwrap().devices.iter().any(|d| d.key == me.device && d.signed_here));

    // a post: drafted and stamped here, signed there, then here like any other
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    let d = browser.draft_post(&PostDraft { community: c.id.clone(), title: "From the browser".into(), text: "hi".into(), link: None, media: None }).unwrap();
    let nonce = mine_draft(&mut browser, &d);
    let Published::Signing(id) = browser.publish_draft(d.token, nonce).unwrap() else { panic!("sent to be signed") };
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    assert_eq!(signed(&mut browser), vec![(id.clone(), true, None)]);
    let s = browser.store.get(&Id::from_hex(&id).unwrap()).expect("stored once signed");
    assert_eq!(s.record.author, device);
    assert_eq!(s.author_root, Id::from_hex(&me.root_key).unwrap());
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    assert!(desk.store.get(&Id::from_hex(&id).unwrap()).is_some(), "and it reached the computer");

    // a ledger entry waits for a yes on the computer
    let cid = Id::from_hex(&c.id).unwrap();
    let mut limits = browser.community(&c.id).unwrap().limits;
    limits.pow_bits += 1;
    let _ = browser.set_limits(&c.id, &limits);
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    let waiting = sign_asks(&mut desk);
    assert_eq!(waiting.len(), 1, "the computer asks first");
    assert_eq!(waiting[0].kind, "ledger");
    assert!(signed(&mut browser).is_empty(), "nothing signed yet");
    desk.sign_answer(&waiting[0].id, false).unwrap();
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    let refused = signed(&mut browser);
    assert_eq!(refused.len(), 1);
    assert!(!refused[0].1 && refused[0].2.as_deref().is_some_and(|m| m.contains("said no")));
    assert_eq!(browser.communities[&cid].limits.pow_bits, limits.pow_bits - 1);

    // revoked on the computer, its key goes and nothing more is signed
    desk.revoke_device(&me.device).unwrap();
    assert!(!desk.me().unwrap().devices.iter().any(|d| d.key == me.device && d.signed_here));
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    let d = browser.draft_post(&PostDraft { community: c.id.clone(), title: "After".into(), text: "hi".into(), link: None, media: None });
    if let Ok(d) = d {
        let nonce = mine_draft(&mut browser, &d);
        if let Ok(Published::Signing(_)) = browser.publish_draft(d.token, nonce) {
            pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
            assert!(signed(&mut browser).iter().all(|(_, ok, _)| !ok), "nobody signs for it anymore");
        }
    }
}
