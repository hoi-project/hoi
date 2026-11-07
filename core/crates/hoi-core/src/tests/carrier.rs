//! A public carrier: a native node that mirrors a public index, takes communities from their
//! founders' browsers without pairing, serves newcomers while founders are away, and signs an
//! announcement of its routes for the registry.
use super::*;

fn routes() -> Vec<String> {
    vec![format!("rtc:{}@203.0.113.10:7844", "q".repeat(43))]
}

/// The registry the carrier announces itself to, by its origin.
const REGISTRY: &str = "https://registry.example";

fn carrier(t: &Arc<AtomicU64>, memory: &Mem) -> Node {
    let mut n = node("node", t, memory);
    if n.me().is_none() {
        n.create_identity("public carrier").unwrap();
    }
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    n
}

/// The browser dials the carrier's WebRTC route, as the web app does with the routes it tried.
fn connect_browser(carrier: &mut Node, browser: &mut Node) {
    carrier.net_connect("b", Role::Peer);
    carrier.set_peer_meta("b", PeerMeta { transport: "webrtc".into(), incoming: true, ..Default::default() });
    browser.net_connect("a", Role::Peer);
    browser.set_peer_meta("a", PeerMeta { transport: "webrtc".into(), dialed: routes(), ..Default::default() });
}

#[test]
fn a_carrier_takes_an_indexed_community_from_its_founders_browser_and_serves_a_newcomer() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut carrier = carrier(&t, &memory);
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let community = founder.create_community(draft("Garden")).unwrap();
    let id = Id::from_hex(&community.id).unwrap();
    let message = post(&mut founder, &community.id, "Carried without pairing");
    // The founder's browser learns the carrier's route from the registry and offers its community.
    founder.public_join(&community.id, &routes(), None).unwrap();
    connect_browser(&mut carrier, &mut founder);
    pump(&mut carrier, &mut founder);
    assert!(carrier.communities().is_empty(), "being offered a community joins nothing by itself");
    assert_eq!(carrier.offered_unknown(), vec![community.id.clone()]);
    assert_eq!(founder.offered_unknown(), Vec::<String>::new(), "a browser never carries");
    assert!(founder.carry(&community.id, None).is_err());

    // The index confirms the key under this name; the carrier asks the browser for it.
    assert_eq!(carrier.carry(&community.id, Some("garden")).unwrap().status, "pending");
    assert!(carrier.offered_unknown().is_empty());
    pump(&mut carrier, &mut founder);
    assert_eq!(carrier.public_join_status(&community.id).unwrap().status, "joined");
    assert_eq!(carrier.feed(Some(&community.id), "fresh", None).unwrap()[0].id, message.id);
    assert!(carrier.state.joined[&id].pinning, "a carrier pins what it carries");
    assert_eq!(carrier.carry(&community.id, Some("garden")).unwrap().status, "joined", "carrying again is idempotent");

    // Its announcement names its own routes and the community, signed by the transport key.
    let endpoint = Id::from_hex(
        &carrier
            .net_status()
            .transport
            .map(|t| t.endpoint_id)
            .unwrap_or_else(|| hoi_crypto::Keypair::from_secret(&carrier.transport_secret().unwrap()).public().hex()),
    )
    .unwrap();
    let native = format!("{}@203.0.113.10:7843", endpoint.hex());
    let foreign = format!("{}@203.0.113.10:7843", Id([9; 32]).hex());
    let announcement = carrier.node_announcement(REGISTRY, &[routes()[0].clone(), native.clone(), foreign, routes()[0].clone()]).unwrap();
    assert_eq!(announcement.registry, REGISTRY);
    assert_eq!(announcement.endpoint, endpoint.hex());
    assert_eq!(announcement.routes, vec![routes()[0].clone(), native], "another node's route and duplicates are dropped");
    assert_eq!(announcement.communities, vec![community.id.clone()]);
    assert_eq!(announcement.issued_at, carrier.now());
    let statement = crate::node_announcement_statement(REGISTRY, &endpoint, announcement.issued_at, &announcement.routes, &[id]);
    assert!(statement.starts_with("hoi public node v2\nhttps://registry.example\n"));
    let signature = hoi_types::Sig::from_slice(&hoi_types::hex_decode(&announcement.signature).unwrap()).unwrap();
    assert!(hoi_crypto::verify(&endpoint, statement.as_bytes(), &signature));
    assert!(!hoi_crypto::verify(&endpoint, statement.replace("7843", "7842").as_bytes(), &signature));
    // Signed for one registry, it is no good at another: the origin is part of the statement.
    let elsewhere = crate::node_announcement_statement("https://other.example", &endpoint, announcement.issued_at, &announcement.routes, &[id]);
    assert!(!hoi_crypto::verify(&endpoint, elsewhere.as_bytes(), &signature));
    assert!(carrier.node_announcement(REGISTRY, &[]).is_err(), "nothing to announce without a public route");
    for bad in ["https://registry.example/registry", "http://registry.example", "https://registry.example\n"] {
        assert!(carrier.node_announcement(bad, &routes()).is_err(), "{bad}");
    }

    // The founder goes away; someone with only the key and the carrier's route catches up.
    carrier.net_disconnect("b");
    drop(founder);
    let mut reader = node("web", &t, &Mem::default());
    reader.create_identity("Lena").unwrap();
    assert_eq!(reader.public_join(&community.id, &routes(), Some("garden")).unwrap().status, "pending");
    connect_browser(&mut carrier, &mut reader);
    pump(&mut carrier, &mut reader);
    assert_eq!(reader.public_join_status(&community.id).unwrap().status, "joined");
    assert_eq!(reader.feed(Some(&community.id), "fresh", None).unwrap()[0].id, message.id);

    // The carrier keeps everything across a restart and still lists the community.
    drop(carrier);
    let mut carrier = carrier_again(&t, &memory);
    assert_eq!(carrier.communities().len(), 1);
    assert_eq!(carrier.node_announcement(REGISTRY, &routes()).unwrap().communities, vec![community.id]);
}

fn carrier_again(t: &Arc<AtomicU64>, memory: &Mem) -> Node {
    carrier(t, memory)
}

#[test]
fn a_carrier_refuses_what_the_index_did_not_promise_and_never_announces_private_or_closed_groups() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut carrier = carrier(&t, &Mem::default());
    let mut founder = node("web", &t, &Mem::default());
    founder.create_identity("Robin").unwrap();
    let open = founder.create_community(draft("Garden")).unwrap();
    let mut closed_draft = draft("Members only");
    closed_draft.limits.as_mut().unwrap().admission = Some("invite".into());
    let closed = founder.create_community(closed_draft).unwrap();
    founder.public_join(&open.id, &routes(), None).unwrap();
    connect_browser(&mut carrier, &mut founder);
    pump(&mut carrier, &mut founder);
    // Only the community whose routes led the browser here is named; the invite-only one stays
    // a salted hash the carrier can't read.
    assert_eq!(carrier.offered_unknown(), vec![open.id.clone()]);

    // A name the signed genesis does not carry is rejected, and not offered again.
    assert_eq!(carrier.carry(&open.id, Some("orchard")).unwrap().status, "pending");
    pump(&mut carrier, &mut founder);
    assert_eq!(carrier.public_join_status(&open.id).unwrap().status, "rejected");
    assert!(carrier.communities().is_empty());
    assert!(carrier.offered_unknown().is_empty());

    // An invite-only community is never carried, whatever the index says; asking for its key
    // finds the browser's copy through the salted hashes all the same.
    assert_eq!(carrier.carry(&closed.id, None).unwrap().status, "pending");
    pump(&mut carrier, &mut founder);
    assert_eq!(carrier.public_join_status(&closed.id).unwrap().status, "rejected");
    assert!(carrier.offered_unknown().is_empty());
    assert!(carrier.node_announcement(REGISTRY, &routes()).unwrap().communities.is_empty());

    // The right name works on retry, and only that community is announced afterwards.
    assert_eq!(carrier.carry(&open.id, Some("garden")).unwrap().status, "pending");
    pump(&mut carrier, &mut founder);
    assert_eq!(carrier.public_join_status(&open.id).unwrap().status, "joined");
    assert_eq!(carrier.node_announcement(REGISTRY, &routes()).unwrap().communities, vec![open.id]);
}

#[test]
fn a_carrier_can_wait_for_a_whole_index_while_a_browser_is_limited() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut carrier = carrier(&t, &Mem::default());
    for i in 0..40u8 {
        let mut key = [0u8; 32];
        key[0] = 1;
        key[31] = i;
        assert_eq!(carrier.carry(&Id(key).hex(), None).unwrap().status, "pending");
    }
    assert_eq!(carrier.state.public_joins.len(), 40);
    let mut browser = node("web", &t, &Mem::default());
    browser.create_identity("Lena").unwrap();
    for i in 0..16u8 {
        let mut key = [0u8; 32];
        key[0] = 2;
        key[31] = i;
        browser.public_join(&Id(key).hex(), &routes(), None).unwrap();
    }
    let mut key = [0u8; 32];
    key[0] = 3;
    assert!(browser.public_join(&Id(key).hex(), &routes(), None).is_err(), "a person finishes or cancels earlier joins first");
}

#[test]
fn rust_and_the_registry_share_the_exact_announcement_statement() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../vectors/node-announcement.json")).unwrap();
    let seed: [u8; 32] = hoi_types::hex_decode(fixture["seed"].as_str().unwrap()).unwrap().try_into().unwrap();
    let key = Keypair::from_secret(&seed);
    let endpoint = key.public();
    assert_eq!(endpoint.hex(), fixture["endpoint"].as_str().unwrap());
    let routes: Vec<String> = fixture["routes"].as_array().unwrap().iter().map(|r| r.as_str().unwrap().to_string()).collect();
    let communities: Vec<Id> = fixture["communities"].as_array().unwrap().iter().map(|c| Id::from_hex(c.as_str().unwrap()).unwrap()).collect();
    let issued_at = fixture["issuedAt"].as_u64().unwrap();
    let registry = fixture["registry"].as_str().unwrap();
    assert!(crate::registry_origin_ok(registry));
    let statement = crate::node_announcement_statement(registry, &endpoint, issued_at, &routes, &communities);
    assert_eq!(statement, fixture["statement"].as_str().unwrap());
    assert_eq!(hoi_types::hex_encode(&key.sign(statement.as_bytes()).0), fixture["signature"].as_str().unwrap());
    assert_eq!(crate::release_versions(), crate::registration::publication_version());
}
