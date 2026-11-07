//! Public nodes carry communities on request: anyone with the signed genesis can ask a
//! node that takes requests; the node stores and serves without a key of its own; junk stays
//! bounded by policy, refusals, rates, budgets, open admission and idleness, and what is let go
//! is deleted.
use super::*;
use crate::CarryPolicy;

fn public_node(t: &Arc<AtomicU64>, memory: &Mem, limit: usize) -> Node {
    let mut n = node("node", t, memory);
    n.set_node_name(Some("public node".into()));
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    n.set_carry_policy(CarryPolicy::Any, limit);
    n
}

fn dial(native: &mut Node, native_peer: &str, browser: &mut Node, browser_peer: &str) {
    native.net_connect(native_peer, Role::Peer);
    native.set_peer_meta(native_peer, PeerMeta { transport: "webrtc".into(), incoming: true, ..Default::default() });
    browser.net_connect(browser_peer, Role::Peer);
    browser.set_peer_meta(browser_peer, PeerMeta { transport: "webrtc".into(), ..Default::default() });
}

fn notices(n: &mut Node) -> Vec<String> {
    n.commit().unwrap();
    n.take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Notice { text } => Some(text),
            _ => None,
        })
        .collect()
}

/// The `carried` answers a node queued, decoded.
fn answers(n: &mut Node) -> Vec<(Id, bool, Option<String>)> {
    n.take_outbox()
        .into_iter()
        .filter_map(|(_, m)| match hoi_sync::Msg::decode(&m) {
            Ok(hoi_sync::Msg::Carried { community, ok, reason }) => Some((community, ok, reason)),
            _ => None,
        })
        .collect()
}

fn carry(genesis: &[u8]) -> Vec<u8> {
    hoi_sync::Msg::Carry { genesis: genesis.to_vec() }.encode()
}

fn genesis_of(n: &Node, id: &str) -> Vec<u8> {
    n.store.genesis_of(&Id::from_hex(id).unwrap()).unwrap().wire.clone()
}

#[test]
fn a_public_node_without_a_key_carries_on_request_and_serves_a_newcomer_by_key() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut carrier = public_node(&t, &memory, 8);
    assert!(carrier.me().is_none(), "a public node has no person behind it");
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    // A member (not the founder) holds the invite and asks: the founder's automatic request
    // must not be what makes this work.
    let community = founder.create_community(draft("Garden")).unwrap();
    let message = post(&mut founder, &community.id, "Carried on request");
    let mut member = node("web", &t, &Mem::default());
    member.create_identity("Lena").unwrap();
    member.join(&founder.invite(&community.id).unwrap().uri).unwrap();
    // The member catches up with the founder first, so it has records to hand over.
    founder.net_connect("m", Role::Peer);
    member.net_connect("f", Role::Peer);
    pump_named(&mut founder, "m", &mut member, "f");
    assert_eq!(member.feed(Some(&community.id), "fresh", None).unwrap()[0].id, message.id);
    founder.net_disconnect("m");
    member.net_disconnect("f");
    dial(&mut carrier, "b", &mut member, "a");
    pump(&mut carrier, &mut member);
    assert!(carrier.communities().is_empty(), "nothing is carried by merely connecting, and a member is not the founder");
    assert_eq!(carrier.warmth().directory.carry_nodes, 0);
    assert_eq!(member.warmth().directory.carry_nodes, 1, "the node advertised that it takes requests");

    let asked = member.request_carry(&community.id).unwrap();
    assert_eq!((asked.asked, asked.carrying), (1, 0));
    // The request is the genesis and nothing else: no routes for the node to dial.
    let sent: Vec<hoi_sync::Msg> = member.take_outbox().into_iter().map(|(_, m)| hoi_sync::Msg::decode(&m).unwrap()).collect();
    let genesis = genesis_of(&member, &community.id);
    assert_eq!(sent, vec![hoi_sync::Msg::Carry { genesis: genesis.clone() }]);
    assert!(carrier.net_receive("b", &carry(&genesis)));
    pump(&mut carrier, &mut member);
    assert_eq!(carrier.communities().len(), 1);
    assert_eq!(carrier.carried_on_request(), vec![community.id.clone()]);
    assert!(carrier.state.joined[&Id::from_hex(&community.id).unwrap()].pinning, "a carrier pins what it carries");
    assert!(carrier.me().is_none(), "carrying made no key");
    assert!(carrier.state.learned.is_empty(), "a request taught the node no route");
    assert_eq!(carrier.community_warmth(&community.id).unwrap().carried.as_deref(), Some("request"));
    assert!(notices(&mut member).iter().any(|n| n.contains("now keeps c/Garden available")), "the member hears the answer");
    assert_eq!(member.request_carry(&community.id).unwrap().carrying, 1, "asking again finds it carried");
    assert_eq!(member.request_carry(&community.id).unwrap().asked, 0);

    // The member brought the records; the founder goes away; a stranger joins by key.
    pump(&mut carrier, &mut member);
    assert_eq!(carrier.feed(Some(&community.id), "fresh", None).unwrap()[0].id, message.id);
    carrier.net_disconnect("b");
    drop(member);
    drop(founder);
    let mut reader = node("web", &t, &Mem::default());
    reader.create_identity("Mira").unwrap();
    reader.public_join(&community.id, &[format!("rtc:{}@203.0.113.10:7844", "q".repeat(43))], Some("garden")).unwrap();
    dial(&mut carrier, "b", &mut reader, "a");
    pump(&mut carrier, &mut reader);
    assert_eq!(reader.public_join_status(&community.id).unwrap().status, "joined");
    assert_eq!(reader.feed(Some(&community.id), "fresh", None).unwrap()[0].id, message.id);
    let w = reader.community_warmth(&community.id).unwrap();
    assert_eq!((w.level.as_str(), w.holders_durable), ("cool", 1), "{w:?}");

    // Across a restart the node still knows why it carries the community.
    drop(carrier);
    let carrier = public_node(&t, &memory, 8);
    assert_eq!(carrier.carried_on_request(), vec![community.id.clone()]);
    assert_eq!(carrier.communities().len(), 1);
}

#[test]
fn requests_are_refused_when_not_accepting_not_open_full_or_invalid_and_floods_are_ignored() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let open = founder.create_community(draft("Garden")).unwrap();
    let open_id = Id::from_hex(&open.id).unwrap();
    let genesis = genesis_of(&founder, &open.id);
    let mut closed = draft("Members only");
    closed.limits.as_mut().unwrap().admission = Some("invite".into());
    let closed = founder.create_community(closed).unwrap();
    let closed_genesis = genesis_of(&founder, &closed.id);
    // Someone who is not the founder connects, so no community is offered unasked.
    let mut member = node("web", &t, &Mem::default());
    member.create_identity("Lena").unwrap();
    member.join(&founder.invite(&open.id).unwrap().uri).unwrap();

    // A node that does not take requests says so and counts the request against the peer.
    let mut home = node("node", &t, &Mem::default());
    home.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    home.create_identity("home").unwrap();
    assert_eq!(home.carry_policy(), CarryPolicy::Off);
    dial(&mut home, "b", &mut member, "a");
    pump(&mut home, &mut member);
    assert_eq!(member.request_carry(&open.id).unwrap().asked, 0, "nobody advertised it");
    assert!(home.net_receive("b", &carry(&genesis)));
    assert_eq!(answers(&mut home), vec![(open_id, false, Some("not accepting".into()))]);
    assert!(home.communities().is_empty());
    home.net_disconnect("b");
    member.net_disconnect("a");

    // Invite-only communities are kept by their members' own nodes, never by a public node.
    let mut carrier = public_node(&t, &Mem::default(), 3);
    assert!(founder.request_carry(&closed.id).is_err());
    dial(&mut carrier, "b", &mut member, "a");
    pump(&mut carrier, &mut member);
    assert!(carrier.net_receive("b", &carry(&closed_genesis)));
    assert_eq!(answers(&mut carrier), vec![(Id::from_hex(&closed.id).unwrap(), false, Some("not open".into()))]);
    assert!(carrier.communities().is_empty());

    // Requests may fill half the budget; the rest is kept for an index.
    let mut other = node("web", &t, &Mem::default());
    other.create_identity("Lena").unwrap();
    let second = other.create_community(draft("Second")).unwrap();
    let third = other.create_community(draft("Third")).unwrap();
    assert!(carrier.net_receive("b", &carry(&genesis)));
    assert!(carrier.net_receive("b", &carry(&genesis_of(&other, &second.id))));
    assert!(carrier.net_receive("b", &carry(&genesis_of(&other, &third.id))));
    let got = answers(&mut carrier);
    assert_eq!(got.iter().filter(|a| a.1).count(), 2, "{got:?}");
    assert!(got.iter().any(|a| a.2.as_deref() == Some("full")), "{got:?}");
    assert_eq!(carrier.carried_count(), 2);
    // The same community again (a minute later, under the rate) is a yes without another slot.
    t.fetch_add(60, Ordering::SeqCst);
    assert!(carrier.net_receive("b", &carry(&genesis)));
    assert_eq!(answers(&mut carrier), vec![(open_id, true, None)]);
    assert_eq!(carrier.carried_count(), 2);

    // A tampered genesis is the peer's fault.
    let mut forged = genesis.clone();
    let last = forged.len() - 1;
    forged[last] ^= 1;
    let mut fresh = public_node(&t, &Mem::default(), 8);
    dial(&mut fresh, "b", &mut member, "x");
    pump_named(&mut fresh, "b", &mut member, "x");
    assert!(fresh.net_receive("b", &carry(&forged)));
    assert_eq!(answers(&mut fresh), vec![(open_id, false, Some("invalid".into()))]);
    assert!(fresh.communities().is_empty());

    // More than four requests in a minute are ignored, not answered, and count against the peer.
    let mut flooded = public_node(&t, &Mem::default(), 8);
    dial(&mut flooded, "b", &mut member, "y");
    pump_named(&mut flooded, "b", &mut member, "y");
    for _ in 0..4 {
        assert!(flooded.net_receive("b", &carry(&genesis)));
    }
    assert_eq!(answers(&mut flooded).len(), 4);
    assert!(flooded.net_receive("b", &carry(&genesis)), "one request over the pace does not end the link");
    assert!(answers(&mut flooded).is_empty(), "the fifth request in a minute gets no answer");
    let mut lost = 0;
    for _ in 0..40 {
        if !flooded.net_receive("b", &carry(&genesis)) {
            lost += 1;
            break;
        }
    }
    assert_eq!(lost, 1, "a peer that keeps flooding is dropped like any other misbehaving peer");
    // The requester paces itself the same way: more than four to one node wait for the next minute.
    let mut eager = node("web", &t, &Mem::default());
    eager.create_identity("Eager").unwrap();
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(eager.create_community(draft(&format!("Plenty {i}"))).unwrap().id);
    }
    let mut patient = public_node(&t, &Mem::default(), 64);
    dial(&mut patient, "b", &mut eager, "p");
    pump_named(&mut patient, "b", &mut eager, "p");
    // The founder's browser asks by itself for what it founded, and paces itself all the same.
    assert_eq!(patient.communities().len(), 4, "the fifth request waits for the next minute");
    let waiting = ids.iter().find(|id| !patient.communities().iter().any(|c| &c.id == *id)).unwrap().clone();
    assert_eq!(eager.request_carry(&waiting).unwrap().asked, 0, "asking by hand doesn't jump the queue");
    t.fetch_add(60, Ordering::SeqCst);
    assert_eq!(eager.request_carry(&waiting).unwrap().asked, 1);
}

#[test]
fn a_node_takes_eight_communities_a_minute_whoever_asks_and_answers_busy_beyond() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let ids: Vec<String> = (0..10).map(|i| founder.create_community(draft(&format!("Many {i}"))).unwrap().id).collect();
    let mut carrier = public_node(&t, &Mem::default(), 64);
    // Three connections, each under its own pace, together over the node's.
    let mut browsers: Vec<Node> = (0..3)
        .map(|i| {
            let mut b = node("web", &t, &Mem::default());
            b.create_identity(&format!("b{i}")).unwrap();
            b
        })
        .collect();
    for (i, b) in browsers.iter_mut().enumerate() {
        dial(&mut carrier, &format!("in{i}"), b, "out");
        pump_named(&mut carrier, &format!("in{i}"), b, "out");
    }
    let mut outcomes = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let from = format!("in{}", i % 3);
        assert!(carrier.net_receive(&from, &carry(&genesis_of(&founder, id))));
        outcomes.extend(answers(&mut carrier).into_iter().map(|a| a.2));
    }
    assert_eq!(outcomes.iter().filter(|r| r.is_none()).count(), 8, "{outcomes:?}");
    assert_eq!(outcomes.iter().filter(|r| r.as_deref() == Some("busy")).count(), 2, "{outcomes:?}");
    assert_eq!(carrier.carried_count(), 8);
    // The next minute has room again.
    t.fetch_add(60, Ordering::SeqCst);
    assert!(carrier.net_receive("in0", &carry(&genesis_of(&founder, &ids[8]))));
    assert_eq!(answers(&mut carrier), vec![(Id::from_hex(&ids[8]).unwrap(), true, None)]);
}

#[test]
fn an_operator_s_refusal_is_final_and_deletes_what_was_carried() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let c = founder.create_community(draft("Unwanted")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    post(&mut founder, &c.id, "Something");
    let mut carrier = public_node(&t, &Mem::default(), 8);
    dial(&mut carrier, "b", &mut founder, "a");
    pump(&mut carrier, &mut founder);
    assert_eq!(carrier.carried_on_request(), vec![c.id.clone()], "the founder's browser asked, unasked");
    assert!(carrier.store.in_community(&cid).count() >= 2);
    carrier.set_refused(std::slice::from_ref(&c.id)).unwrap();
    assert!(carrier.communities().is_empty());
    assert_eq!(carrier.store.in_community(&cid).count(), 0, "a refused community leaves no records behind");
    assert!(notices(&mut carrier).iter().any(|n| n.contains("Stopped carrying c/Unwanted: refused")));
    pump(&mut carrier, &mut founder);
    assert!(carrier.communities().is_empty(), "the browser does not ask twice on one connection");
    // Asked again, by anyone: refused. Listed by an index: refused. Offered by a member: unseen.
    let genesis = genesis_of(&founder, &c.id);
    assert!(carrier.net_receive("b", &carry(&genesis)));
    assert_eq!(answers(&mut carrier), vec![(cid, false, Some("refused".into()))]);
    assert!(carrier.carry(&c.id, Some("unwanted")).is_err());
    assert!(carrier.offered_unknown().is_empty());
    assert_eq!(carrier.refused(), vec![c.id.clone()]);
    assert!(notices(&mut founder).iter().any(|n| n.contains("operator does not carry c/Unwanted")), "the founder hears why");
}

#[test]
fn communities_nobody_else_touches_are_let_go_and_deleted_unless_an_index_lists_them() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut carrier = public_node(&t, &Mem::default(), 8);
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let asked = founder.create_community(draft("Asked")).unwrap();
    let asked_id = Id::from_hex(&asked.id).unwrap();
    post(&mut founder, &asked.id, "Day one");
    dial(&mut carrier, "b", &mut founder, "a");
    pump(&mut carrier, &mut founder);
    // The founder's browser asks by itself for the open community it made with a public name.
    assert_eq!(carrier.carried_on_request(), vec![asked.id.clone()], "a founder's browser asks unasked");
    assert!(carrier.store.in_community(&asked_id).count() >= 2);
    carrier.net_disconnect("b");
    founder.net_disconnect("a");
    // An index-listed community waits for its founder as long as it takes.
    let listed = Id([5; 32]);
    assert_eq!(carrier.carry(&listed.hex(), Some("listed")).unwrap().status, "pending");
    assert_eq!(carrier.carried_count(), 2);

    t.fetch_add(10 * 86_400, Ordering::SeqCst);
    carrier.sweep();
    assert_eq!(carrier.communities().len(), 1, "ten days idle is fine");
    // Someone delivers a record on day ten: the clock starts over.
    post(&mut founder, &asked.id, "Day ten");
    dial(&mut carrier, "b", &mut founder, "a");
    pump(&mut carrier, &mut founder);
    carrier.net_disconnect("b");
    founder.net_disconnect("a");
    t.fetch_add(10 * 86_400, Ordering::SeqCst);
    carrier.sweep();
    assert!(carrier.state.joined.contains_key(&asked_id), "touched again within two weeks");
    t.fetch_add(5 * 86_400, Ordering::SeqCst);
    carrier.sweep();
    assert!(!carrier.state.joined.contains_key(&asked_id), "fifteen days without another holder: let go");
    assert_eq!(carrier.store.in_community(&asked_id).count(), 0, "and deleted: the records were never the operator's choice");
    assert!(carrier.carried_on_request().is_empty());
    assert!(carrier.state.carried.contains_key(&listed), "the index-listed one stays");
    assert!(notices(&mut carrier).iter().any(|n| n.contains("Stopped carrying c/Asked")));

    // Leaving a community on purpose forgets the carry note too.
    carrier.leave(&listed.hex()).unwrap();
    assert_eq!(carrier.carried_count(), 0);
}

#[test]
fn a_request_whose_records_never_come_is_let_go_after_an_hour() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let c = founder.create_community(draft("Empty")).unwrap();
    let cid = Id::from_hex(&c.id).unwrap();
    let mut carrier = public_node(&t, &Mem::default(), 8);
    // Someone who holds the invite but is no member asks: the genesis arrives, nothing else.
    let mut passerby = node("web", &t, &Mem::default());
    passerby.create_identity("Passer-by").unwrap();
    dial(&mut carrier, "b", &mut passerby, "a");
    pump(&mut carrier, &mut passerby);
    assert!(carrier.net_receive("b", &carry(&genesis_of(&founder, &c.id))));
    assert_eq!(answers(&mut carrier), vec![(cid, true, None)]);
    assert_eq!(carrier.carried_count(), 1);
    t.fetch_add(1800, Ordering::SeqCst);
    carrier.sweep();
    assert_eq!(carrier.carried_count(), 1, "half an hour is not an hour");
    t.fetch_add(1800, Ordering::SeqCst);
    carrier.sweep();
    assert_eq!(carrier.carried_count(), 0);
    assert_eq!(carrier.store.in_community(&cid).count(), 0);
    assert!(notices(&mut carrier).iter().any(|n| n.contains("nobody brought its records within an hour")));
}

#[test]
fn over_the_byte_budget_the_longest_untouched_community_goes_first() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let first = founder.create_community(draft("First")).unwrap();
    let first_id = Id::from_hex(&first.id).unwrap();
    for i in 0..3 {
        post(&mut founder, &first.id, &format!("First {i}"));
    }
    let mut carrier = public_node(&t, &Mem::default(), 8);
    dial(&mut carrier, "b", &mut founder, "a");
    pump(&mut carrier, &mut founder);
    assert_eq!(carrier.carried_on_request(), vec![first.id.clone()]);
    let first_bytes = carrier.community_bytes(&first_id);
    assert!(first_bytes > 0);
    founder.net_disconnect("a");
    carrier.net_disconnect("b");
    // An hour later a second community arrives, touched more recently.
    t.fetch_add(3600, Ordering::SeqCst);
    let second = founder.create_community(draft("Second")).unwrap();
    let second_id = Id::from_hex(&second.id).unwrap();
    post(&mut founder, &second.id, "Second 0");
    dial(&mut carrier, "b", &mut founder, "a");
    pump(&mut carrier, &mut founder);
    assert_eq!(carrier.carried_on_request().len(), 2);
    let second_bytes = carrier.community_bytes(&second_id);
    assert!(first_bytes + second_bytes > second_bytes);
    carrier.set_carry_bytes(second_bytes);
    carrier.sweep();
    assert_eq!(carrier.carried_on_request(), vec![second.id.clone()], "the first, untouched longer, made room");
    assert_eq!(carrier.store.in_community(&first_id).count(), 0);
    assert!(notices(&mut carrier).iter().any(|n| n.contains("space for carried communities is used up")));
}

#[test]
fn only_a_founder_s_own_open_communities_are_offered_unasked() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let theirs = founder.create_community(draft("Theirs")).unwrap();
    let mut closed = draft("Closed");
    closed.limits.as_mut().unwrap().admission = Some("invite".into());
    founder.create_community(closed).unwrap();
    let mut member = node("web", &t, &Mem::default());
    member.create_identity("Lena").unwrap();
    member.join(&founder.invite(&theirs.id).unwrap().uri).unwrap();
    member.create_community(draft("Mine")).unwrap();
    member.create_community(draft("Nur für uns")).unwrap();
    let mut carrier = public_node(&t, &Mem::default(), 8);
    dial(&mut carrier, "b", &mut member, "a");
    pump(&mut carrier, &mut member);
    let mut carried: Vec<String> = carrier.communities().iter().map(|c| c.name.clone()).collect();
    carried.sort();
    assert_eq!(
        carried,
        vec!["Mine", "Nur für uns"],
        "the member asks for the open communities they founded, public name or not, never for someone else's or a closed one"
    );
    // A browser does not take requests and cannot be made to carry.
    assert!(!member.accepts_carry());
    member.set_carry_policy(CarryPolicy::Any, 8);
    assert_eq!(member.carry_policy(), CarryPolicy::Off);
    // A desktop with the policy off does not advertise it either, and with the host's policy
    // on it still waits for the person to make it a public node.
    let mut desktop = node("desktop", &t, &Mem::default());
    desktop.create_identity("desk").unwrap();
    assert!(!desktop.accepts_carry());
    desktop.set_carry_policy(CarryPolicy::Any, 8);
    assert!(!desktop.accepts_carry(), "relaying alone doesn't make a desktop public");
    let s = desktop.set_settings(&SettingsPatch { public_node: Some(true), ..Default::default() }).unwrap();
    assert!(s.relay, "a public node relays");
    assert!(desktop.accepts_carry() && desktop.is_public_node());
    desktop.set_settings(&SettingsPatch { relay: Some(false), ..Default::default() }).unwrap();
    assert!(!desktop.settings().public_node && !desktop.accepts_carry(), "no relaying, no public node");
}

#[test]
fn a_native_node_joins_by_invite_without_a_key_and_a_browser_still_needs_one() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let c = founder.create_community(draft("Garden")).unwrap();
    let invite = founder.invite(&c.id).unwrap().uri;
    let mut keyless = node("node", &t, &Mem::default());
    keyless.join(&invite).unwrap();
    assert!(keyless.me().is_none());
    assert_eq!(keyless.communities().len(), 1);
    let mut web = node("web", &t, &Mem::default());
    assert_eq!(web.join(&invite).unwrap_err().code, "no-identity");
}
