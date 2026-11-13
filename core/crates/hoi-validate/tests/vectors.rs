//! Conformance: every record in core/vectors/ decodes, hashes to its id, verifies and carries
//! its stamp; and the generator still produces exactly the committed files.

use std::path::PathBuf;

fn vectors() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vectors")
}

fn unhex(s: &str) -> Vec<u8> {
    hoi_types::hex_decode(s).expect("hex")
}

#[test]
fn durable_drop_capability_is_bound_to_the_holder_key() {
    use hoi_crypto::envelope;
    use hoi_types::{Id, Sig};
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("scatter.json")).unwrap()).unwrap();
    let holder = &v["holder"];
    let bytes32 = |v: &serde_json::Value| <[u8; 32]>::try_from(unhex(v.as_str().unwrap())).unwrap();
    let endpoint = bytes32(&holder["endpoint"]);
    let xk = bytes32(&holder["xk"]);
    let expires = holder["expires_at"].as_u64().unwrap();
    let capability = &holder["drop_journal"];
    let statement = unhex(capability["statement"].as_str().expect("durable drop capability vector"));
    assert_eq!(envelope::drop_journal_statement(&endpoint, &xk, expires), statement);
    let proof = Sig(unhex(capability["sig"].as_str().unwrap()).try_into().unwrap());
    assert!(hoi_crypto::verify(&Id(endpoint), &statement, &proof));
    let ordinary = Sig(unhex(holder["sig"].as_str().unwrap()).try_into().unwrap());
    assert!(!hoi_crypto::verify(&Id(endpoint), &statement, &ordinary), "ordinary holder keys never imply durable drops");
    for altered in [
        envelope::drop_journal_statement(&[0; 32], &xk, expires),
        envelope::drop_journal_statement(&endpoint, &[0; 32], expires),
        envelope::drop_journal_statement(&endpoint, &xk, expires + 1),
    ] {
        assert!(!hoi_crypto::verify(&Id(endpoint), &altered, &proof), "proof is bound to endpoint, key and expiry");
    }
    for field in ["hello", "nodes"] {
        let bytes = unhex(capability[field].as_str().unwrap());
        let msg = hoi_sync::Msg::decode(&bytes).unwrap();
        assert_eq!(msg.encode(), bytes);
        let key = match msg {
            hoi_sync::Msg::Hello { holder, features, .. } => {
                assert!(features.iter().any(|f| f == hoi_sync::FEATURE_DROP_JOURNAL));
                holder.unwrap()
            }
            hoi_sync::Msg::Nodes { holders, .. } => {
                assert_eq!(holders[0].endpoint, Id(endpoint));
                holders[0].key.unwrap()
            }
            _ => panic!("unexpected capability message"),
        };
        assert_eq!((key.xk, key.expires_at, key.drop_journal), (xk, expires, Some(proof.0)));
    }
}

#[test]
fn records_pass_stages_one_to_three() {
    let text = std::fs::read_to_string(vectors().join("records.json")).expect("run the vectors example");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let bits = v["pow_bits"].as_u64().unwrap() as u32;
    let records = v["records"].as_array().unwrap();
    assert_eq!(records.len(), 20);
    for r in records {
        let wire = unhex(r["wire_cbor"].as_str().unwrap());
        let d = hoi_validate::decode(&wire).expect("decodes");
        assert_eq!(d.id.hex(), r["id"].as_str().unwrap(), "{}", r["name"]);
        assert_eq!(hoi_types::hex_encode(&d.wire.record.encode()), r["record_cbor"].as_str().unwrap());
        hoi_validate::signature(&d).expect("signature");
        hoi_validate::stamp(&d.id, d.wire.record.nonce, bits).expect("stamp");
        hoi_validate::content(&d.wire.record).expect("content rules");
    }
    // the rotation: the first root committed to the recovery key that signs it, and
    // the new root names the person it continues
    let get = |name: &str| {
        let r = records.iter().find(|r| r["name"] == name).unwrap();
        hoi_validate::decode(&unhex(r["wire_cbor"].as_str().unwrap())).unwrap().wire.record
    };
    let (first, next, rotate) = (get("identity-committed"), get("identity-continues"), get("rotate"));
    let hoi_types::Body::Identity { rc: Some(rc), of: None, .. } = first.body else { panic!("a committed identity") };
    assert_eq!(rc, hoi_crypto::recovery::commitment(&rotate.author));
    let hoi_types::Body::Rotate { person, to } = rotate.body else { panic!("a rotation") };
    assert_eq!((person, to), (first.author, next.author));
    assert!(matches!(next.body, hoi_types::Body::Identity { of: Some(p), .. } if p == first.author));
}

#[test]
fn identity_and_delegation_verify() {
    let text = std::fs::read_to_string(vectors().join("delegation-chain.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let wires = v["wires"].as_array().unwrap();
    let ident = hoi_validate::decode(&unhex(wires[0].as_str().unwrap())).unwrap();
    assert!(hoi_validate::identity(&ident.wire.record).unwrap() >= hoi_crypto::identity::MIN_BITS);
    let deleg = hoi_validate::decode(&unhex(wires[1].as_str().unwrap())).unwrap();
    hoi_validate::signature(&deleg).expect("the device consents");
    assert_eq!(hoi_validate::delegation_consent(&deleg.wire.record).unwrap().hex(), v["root"].as_str().unwrap(), "the root blesses");
}

#[test]
fn media_sanitizes_as_committed() {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("media.json")).unwrap()).unwrap();
    let dirty = unhex(v["input"].as_str().unwrap());
    let clean = unhex(v["sanitized"].as_str().unwrap());
    assert_eq!(hoi_validate::media::sanitize(&dirty).unwrap().0, clean);
    assert!(hoi_validate::media::check("image/png", &dirty).is_err());
    hoi_validate::media::check("image/png", &clean).unwrap();
}

#[test]
fn pdq_reads_the_same_everywhere_within_its_tolerance() {
    use hoi_validate::similar;
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("pdq.json")).unwrap()).unwrap();
    let picture = unhex(v["picture"].as_str().unwrap());
    let tolerance = v["tolerance"].as_u64().unwrap() as u32;
    let got = similar::examine_variants(v["mime"].as_str().unwrap(), &picture).unwrap().expect("a fingerprint");
    let want: Vec<[u8; 32]> = v["hashes"].as_array().unwrap().iter().map(|h| similar::from_hex(h.as_str().unwrap()).unwrap()).collect();
    assert_eq!(want.len(), 8);
    for (n, (g, w)) in got.hashes.iter().zip(&want).enumerate() {
        let d = similar::distance(g, w);
        assert!(d <= tolerance, "variant {n}: {d} bits from the committed hash");
    }
    assert!((got.quality as i64 - v["quality"].as_i64().unwrap()).abs() <= 2);
    assert_eq!(v["match_distance"].as_u64().unwrap() as u32, similar::MATCH_DISTANCE);
}

#[test]
fn threshold_entry_cosigns_verify() {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("threshold.json")).unwrap()).unwrap();
    let d = hoi_validate::decode(&unhex(v["entry"]["wire_cbor"].as_str().unwrap())).unwrap();
    let pid = d.wire.record.proposal_id();
    assert_eq!(pid.hex(), v["proposal_id"].as_str().unwrap());
    let hoi_types::Body::Ledger(l) = &d.wire.record.body else { panic!("ledger") };
    let cos = l.cosign.as_ref().unwrap();
    assert_eq!(cos.len(), 2);
    assert!(cos.iter().all(|c| hoi_crypto::verify(&c.key, pid.as_bytes(), &c.sig)));
}

#[test]
fn warmth_reads_the_same_everywhere() {
    use hoi_sync::warmth::{Holder, Link, Observation, assess, assess_links, hint};
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("warmth.json")).unwrap()).unwrap();
    let cases = v["cases"].as_array().unwrap();
    assert!(cases.len() >= 10);
    for case in cases {
        let o = &case["observation"];
        let observation = Observation {
            own_durable: o["own_durable"].as_bool().unwrap(),
            records: o["records"].as_u64().unwrap(),
            missing: o["missing"].as_u64().unwrap(),
            media_wanted: o["media_wanted"].as_u64().unwrap(),
            media_present: o["media_present"].as_u64().unwrap(),
            holders: o["holders"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| Holder {
                    coverage: h["coverage"].as_f64().unwrap(),
                    durable: h["durable"].as_bool().unwrap(),
                    operator: h["operator"].as_u64(),
                    network: h["network"].as_u64(),
                    guarded: h["guarded"].as_bool().unwrap(),
                })
                .collect(),
            last_sync: o["last_sync"].as_u64(),
            now: o["now"].as_u64().unwrap(),
            live: o["live"].as_bool().unwrap(),
            hinted: o["hinted"].as_u64().unwrap() as u8,
        };
        let w = assess(&observation);
        let want = &case["warmth"];
        let name = case["name"].as_str().unwrap();
        assert_eq!(w.level.as_str(), want["level"].as_str().unwrap(), "{name}");
        assert_eq!(w.score as u64, want["score"].as_u64().unwrap(), "{name}");
        assert_eq!(w.copies, want["copies"].as_f64().unwrap(), "{name}");
        assert_eq!(w.durable_copies, want["durable_copies"].as_f64().unwrap(), "{name}");
        assert_eq!(w.network_copies, want["network_copies"].as_f64().unwrap(), "{name}");
        assert_eq!(w.complete, want["complete"].as_f64().unwrap(), "{name}");
        assert_eq!(w.media, want["media"].as_f64().unwrap(), "{name}");
        assert_eq!(w.fresh, want["fresh"].as_bool().unwrap(), "{name}");
        assert_eq!(hint(&w) as u64, want["hint"].as_u64().unwrap(), "{name}");
        assert_eq!(w.kept, want["kept"].as_f64().unwrap(), "{name}");
        assert_eq!(w.places as u64, want["places"].as_u64().unwrap(), "{name}");
        assert_eq!(w.spread, want["spread"].as_f64().unwrap(), "{name}");
        assert_eq!(w.guarded, want["guarded"].as_f64().unwrap(), "{name}");
    }
    let links = v["links"].as_array().unwrap();
    assert!(links.len() >= 7);
    for case in links {
        let name = case["name"].as_str().unwrap();
        let input: Vec<Link> = case["links"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| Link { operator: l["operator"].as_u64(), network: l["network"].as_u64(), guarded: l["guarded"].as_bool().unwrap() })
            .collect();
        let w = assess_links(&input);
        let want = &case["warmth"];
        assert_eq!(w.level.as_str(), want["level"].as_str().unwrap(), "{name}");
        assert_eq!(w.score as u64, want["score"].as_u64().unwrap(), "{name}");
        assert_eq!((w.links as u64, w.places as u64), (want["links"].as_u64().unwrap(), want["places"].as_u64().unwrap()), "{name}");
        assert_eq!(
            (w.effective, w.largest, w.guarded),
            (want["effective"].as_f64().unwrap(), want["largest"].as_f64().unwrap(), want["guarded"].as_f64().unwrap()),
            "{name}"
        );
    }
}

#[test]
fn operator_notices_hash_sign_and_travel_as_committed() {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("node-notice.json")).unwrap()).unwrap();
    let notice: hoi_types::NodeNotice = serde_json::from_value(v["json"].clone()).unwrap();
    assert!(notice.validate().is_empty());
    let canonical = unhex(v["canonical"].as_str().unwrap());
    assert_eq!(notice.canonical(), canonical, "one notice, one canonical form");
    assert_eq!(hoi_types::NodeNotice::from_canonical(&canonical).unwrap(), notice);
    let hash = hoi_types::Id::from_hex(v["hash"].as_str().unwrap()).unwrap();
    assert_eq!(notice.hash(), hash);
    assert_eq!(hoi_types::notice::hash_of(&canonical), hash);
    let seed: [u8; 32] = unhex(v["seed"].as_str().unwrap()).try_into().unwrap();
    let node = hoi_crypto::Keypair::from_secret(&seed);
    assert_eq!(node.public().hex(), v["endpoint"].as_str().unwrap());
    let statement = v["statement"].as_str().unwrap();
    assert_eq!(statement, hoi_types::notice::statement(&node.public(), &hash));
    let sig = hoi_types::Sig(unhex(v["signature"].as_str().unwrap()).try_into().unwrap());
    assert!(hoi_crypto::verify(&node.public(), statement.as_bytes(), &sig));
    let link = hoi_types::NodeLink::parse(v["link"].as_str().unwrap()).unwrap();
    assert_eq!((link.notice.as_deref(), link.signature, link.endpoint()), (Some(canonical.as_slice()), Some(sig), Some(node.public())));
    assert_eq!(hoi_types::NodeLink::parse(v["web_link"].as_str().unwrap()).unwrap(), link);
    // the link is a masked code: its addresses can't be read from it; the clear-text form still opens
    assert!(v["link"].as_str().unwrap().starts_with("hoi://node?code=") && !v["link"].as_str().unwrap().contains("203.0.113.9"));
    assert_eq!(hoi_types::NodeLink::parse(v["legacy_link"].as_str().unwrap()).unwrap(), link);
    let hoi_sync::Msg::Hello { notice_hash, notice: sent, .. } = hoi_sync::Msg::decode(&unhex(v["hello"].as_str().unwrap())).unwrap() else {
        panic!("a hello")
    };
    assert_eq!((notice_hash, sent.as_deref()), (Some(hash), Some(canonical.as_slice())));
    assert_eq!(notice.text(), v["text"].as_str().unwrap());
}

#[test]
fn node_declarations_hash_sign_and_travel_as_committed() {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("node-declaration.json")).unwrap()).unwrap();
    let notice: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("node-notice.json")).unwrap()).unwrap();
    let canonical = unhex(v["canonical"].as_str().unwrap());
    let d = hoi_types::NodeDeclaration::from_canonical(&canonical).unwrap();
    assert_eq!(d.canonical(), canonical, "one configuration, one canonical form");
    assert!(d.validate().is_empty() && !d.unknown);
    let hash = hoi_types::Id::from_hex(v["hash"].as_str().unwrap()).unwrap();
    assert_eq!((d.hash(), hoi_types::notice::hash_of(&canonical)), (hash, hash));
    // it names the node and the notice it goes with
    assert_eq!(d.notice.hex(), notice["hash"].as_str().unwrap());
    assert_eq!(d.notice.hex(), v["notice_hash"].as_str().unwrap());
    let seed: [u8; 32] = unhex(v["seed"].as_str().unwrap()).try_into().unwrap();
    let node = hoi_crypto::Keypair::from_secret(&seed);
    assert_eq!((d.endpoint, node.public().hex()), (node.public(), v["endpoint"].as_str().unwrap().to_string()));
    let statement = v["statement"].as_str().unwrap();
    assert_eq!(statement, hoi_types::declaration::statement(&node.public(), &hash));
    assert!(statement.starts_with("hoi node declaration v1\n"));
    let sig = hoi_types::Sig(unhex(v["signature"].as_str().unwrap()).try_into().unwrap());
    assert!(hoi_crypto::verify(&node.public(), statement.as_bytes(), &sig));
    let hoi_sync::Msg::Hello { notice_hash, declaration: Some(sent), .. } = hoi_sync::Msg::decode(&unhex(v["hello"].as_str().unwrap())).unwrap() else {
        panic!("a hello with a declaration")
    };
    assert_eq!((notice_hash, sent.bytes.as_slice(), sent.sig), (Some(d.notice), canonical.as_slice(), sig.0));
    let sections: Vec<serde_json::Value> = d.sections().into_iter().map(|s| serde_json::json!({ "title": s.title, "text": s.text })).collect();
    assert_eq!(serde_json::Value::Array(sections), v["sections"]);
}

#[test]
fn carry_messages_decode_as_committed() {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("carry.json")).unwrap()).unwrap();
    let community = hoi_types::Id::from_hex(v["community"].as_str().unwrap()).unwrap();
    let decode = |key: &str| hoi_sync::Msg::decode(&unhex(v[key].as_str().unwrap())).unwrap();
    let hoi_sync::Msg::Carry { genesis } = decode("carry") else { panic!("carry") };
    let d = hoi_validate::decode(&genesis).unwrap();
    assert_eq!(d.wire.record.author, community);
    hoi_validate::signature(&d).unwrap();
    assert_eq!(decode("carried_ok"), hoi_sync::Msg::Carried { community, ok: true, reason: None });
    assert_eq!(decode("carried_full"), hoi_sync::Msg::Carried { community, ok: false, reason: Some("full".into()) });
    let hoi_sync::Msg::Catalog { entries } = decode("catalog_with_warmth") else { panic!("catalog") };
    assert_eq!(entries[0].warmth, Some(2));
}

#[test]
fn salted_hellos_match_only_known_keys() {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("hello-salted.json")).unwrap()).unwrap();
    assert_eq!(v["context"].as_str().unwrap(), hoi_sync::COMMUNITY_HASH_CONTEXT);
    let salt: [u8; 16] = unhex(v["salt"].as_str().unwrap()).try_into().unwrap();
    let hoi_sync::Msg::Hello { v: version, communities, salt: sent, hashed, name, token, .. } =
        hoi_sync::Msg::decode(&unhex(v["stranger_hello"].as_str().unwrap())).unwrap()
    else {
        panic!("a hello")
    };
    assert_eq!((version, sent, name, token), (hoi_sync::SYNC_VERSION, Some(salt), None, None), "a stranger hears no name and no credential");
    for c in v["communities"].as_array().unwrap() {
        let key = hoi_types::Id::from_hex(c["key"].as_str().unwrap()).unwrap();
        let hash = hoi_sync::community_hash(&salt, &key);
        assert_eq!(hash.hex(), c["hash"].as_str().unwrap());
        if c["named"].as_bool().unwrap() {
            assert!(communities.contains(&key) && !hashed.contains(&hash), "named in the clear, not hashed again");
        } else {
            assert!(!communities.contains(&key) && hashed.contains(&hash), "a key the receiver holds is found by its hash");
        }
    }
    assert!(!hashed.iter().any(|h| communities.contains(h)), "no key travels in the hashed list");
    let hoi_sync::Msg::Hello { salt, hashed, token, pairing, link_key, communities, .. } =
        hoi_sync::Msg::decode(&unhex(v["home_to_device_hello"].as_str().unwrap())).unwrap()
    else {
        panic!("a hello")
    };
    assert!(salt.is_none() && hashed.is_empty() && communities.len() == 2, "a device hears every community in the clear");
    assert!(token.is_some() && pairing.is_some() && link_key == Some([0x4c; 32]));
}

#[test]
fn encoded_and_legacy_invites_keep_the_same_verified_genesis() {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("uris.json")).unwrap()).unwrap();
    let parse = |key: &str| hoi_types::Invite::parse(v[key].as_str().unwrap()).unwrap();
    let inv = parse("invite");
    for key in ["invite_web", "v1_invite", "v1_invite_web", "legacy_invite", "legacy_invite_web"] {
        assert_eq!(parse(key), inv);
    }
    assert_eq!(inv.to_uri(), v["invite"].as_str().unwrap());
    assert_eq!(inv.community.hex(), v["community"].as_str().unwrap());
    let d = hoi_validate::decode(inv.genesis.as_ref().unwrap()).unwrap();
    hoi_validate::signature(&d).unwrap();
    assert_eq!(d.id.hex(), v["genesis_id"].as_str().unwrap());
    let routed = parse("routed_invite");
    assert_eq!(routed.genesis, inv.genesis);
    assert_eq!(routed.invite, Some(d.id));
    assert_eq!(serde_json::json!(routed.peers), v["routed_peers"]);
    assert_eq!(serde_json::json!(routed.relays), v["routed_relays"]);
    assert!(!v["routed_invite"].as_str().unwrap().contains("203.0.113.7"));
}

#[test]
fn committed_vectors_are_current() {
    let status = std::process::Command::new(env!("CARGO"))
        .args(["run", "-q", "-p", "hoi-validate", "--example", "vectors", "--", "--check"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("cargo");
    assert!(status.success(), "core/vectors is stale: cargo run -p hoi-validate --example vectors");
}

#[test]
fn scatter_vectors_hold() {
    use hoi_crypto::envelope::{self, Kind, Reply, Request, XKey};
    use hoi_crypto::scatter::{self, Class, Shards, size};
    use hoi_types::scatter::Manifest;

    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors().join("scatter.json")).unwrap()).unwrap();
    let hex = |x: &serde_json::Value| unhex(x.as_str().unwrap());
    let a32 = |x: &serde_json::Value| -> [u8; 32] { hex(x).try_into().unwrap() };
    let num = |x: &serde_json::Value| x.as_u64().unwrap() as usize;

    // sizing: every level, generation and number of places
    let sizing = &v["sizing"];
    assert_eq!(sizing["life"].as_u64().unwrap(), size::LIFE);
    let cases = sizing["cases"].as_array().unwrap();
    assert!(cases.len() >= 300);
    for c in cases {
        let (level, g, places) = (num(&c["level"]) as u8, num(&c["generation"]) as u8, c["places"].as_u64().unwrap() as u32);
        let got = size::size(level, g, places);
        match &c["size"] {
            serde_json::Value::Null => assert_eq!(got, None, "{c}"),
            s => {
                let want = size::Size { k: num(&s["k"]), n: num(&s["n"]), m: num(&s["m"]) };
                assert_eq!(got, Some(want), "{c}");
                assert_eq!(size::hard(level, places, want.k), s["hard"].as_bool().unwrap(), "{c}");
                assert!(size::meets(level, g, places, want), "{c}");
            }
        }
    }
    for r in sizing["readings"].as_array().unwrap() {
        let s = size::Size { k: num(&r["k"]), n: num(&r["n"]), m: num(&r["m"]) };
        let want = r["level"].as_u64().map(|l| l as u8);
        assert_eq!(size::reading(num(&r["generation"]) as u8, r["places"].as_u64().unwrap() as u32, s), want, "{r}");
    }

    // Reed–Solomon alone
    let rs = &v["reed_solomon"];
    let data: Vec<Vec<u8>> = rs["data"].as_array().unwrap().iter().map(hex).collect();
    let refs: Vec<&[u8]> = data.iter().map(|d| d.as_slice()).collect();
    let code = scatter::rs::Code::new(num(&rs["k"]));
    for p in rs["pieces"].as_array().unwrap() {
        assert_eq!(code.encode(&refs, num(&p["index"])), hex(&p["piece"]));
    }

    // the object
    let o = &v["object"];
    let (object, key, class) = (a32(&o["object"]), a32(&o["key"]), Class::from_index(num(&o["class"]) as u8).unwrap());
    let sealed = hex(&o["sealed"]);
    assert_eq!(scatter::open_object(&sealed, &object, class, &key).unwrap(), hex(&o["content"]));
    // the nonce is synthetic: sealing the content again gives the same bytes
    assert_eq!(hex(&o["nonce"]), sealed[..24].to_vec());
    assert_eq!(scatter::seal_object(&hex(&o["content"]), &object, &key).unwrap(), (class, sealed.clone()));
    assert_eq!(scatter::sealed_hash(&sealed).to_vec(), hex(&o["sealed_hash"]));

    // the generation: every piece, hash and locator; every listed subset rebuilds
    let g = &v["generation"];
    let k = num(&g["k"]);
    let loc_secret = a32(&g["loc_secret"]);
    let shards = Shards::new_with_key(&sealed, class, k, &a32(&g["aont_key"])).unwrap();
    let pieces = g["pieces"].as_array().unwrap();
    assert_eq!(pieces.len(), num(&g["m"]));
    let mut by_index = Vec::new();
    for p in pieces {
        let i = num(&p["index"]) as u8;
        let bytes = hex(&p["piece"]);
        assert_eq!(bytes.len(), num(&g["piece_len"]));
        assert_eq!(shards.piece(i), bytes, "piece {i}");
        assert_eq!(scatter::piece_hash(&object, 0, i, &bytes).to_vec(), hex(&p["hash"]));
        assert_eq!(scatter::locator(&loc_secret, &object, 0, i).to_vec(), hex(&p["locator"]));
        by_index.push(bytes);
    }
    for sub in g["rebuild_from"].as_array().unwrap() {
        let chosen: Vec<(u8, &[u8])> = sub.as_array().unwrap().iter().map(|i| (num(i) as u8, by_index[num(i)].as_slice())).collect();
        assert_eq!(chosen.len(), k);
        assert_eq!(Shards::rebuild(class, k, &chosen).unwrap().sealed().unwrap(), sealed);
        assert!(Shards::rebuild(class, k, &chosen[1..]).is_err(), "k − 1 pieces rebuild nothing");
    }

    // drops
    let d = &v["drops"];
    let loc = a32(&d["loc"]);
    let token = scatter::drop_token(&a32(&d["drop_secret"]), &loc);
    assert_eq!(token.to_vec(), hex(&d["token"]));
    assert!(scatter::drop_token_matches(&token, &a32(&d["hash"])));
    assert_eq!(scatter::drop_key_id(&a32(&d["drop_key"])).to_vec(), hex(&d["drop_key_id"]));
    assert_eq!(scatter::drop_statement(&loc), hex(&d["statement"]));
    let sig = hoi_types::Sig(hex(&d["sig"]).try_into().unwrap());
    assert!(hoi_crypto::verify(&hoi_types::Id(a32(&d["drop_key"])), &hex(&d["statement"]), &sig));

    // the holder's published key, and every envelope
    let hd = &v["holder"];
    let endpoint = a32(&hd["endpoint"]);
    let xk = XKey::from_seed(&a32(&hd["xk_seed"])).unwrap();
    assert_eq!(xk.public(), a32(&hd["xk"]));
    assert_eq!(envelope::key_id(&xk.public()).to_vec(), hex(&hd["key_id"]));
    let expires = hd["expires_at"].as_u64().unwrap();
    assert_eq!(envelope::xk_statement(&endpoint, &xk.public(), expires), hex(&hd["statement"]));
    assert!(envelope::xk_valid(&endpoint, &xk.public(), expires, &hoi_types::Sig(hex(&hd["sig"]).try_into().unwrap()), expires - 1));
    let answer = XKey::from_seed(&a32(&hd["answer_key_seed"])).unwrap();
    assert_eq!(answer.public(), a32(&hd["answer_key"]));
    let kinds = [("put", Kind::Put), ("get", Kind::Get), ("has", Kind::Has), ("drop by token", Kind::Drop), ("drop by drop key", Kind::Drop)];
    let answers = [
        ("stored", Kind::Stored),
        ("got", Kind::Got),
        ("got nothing", Kind::Got),
        ("had", Kind::Had),
        ("dropped", Kind::Dropped),
        ("drop unknown", Kind::Dropped),
        ("drop refused", Kind::Dropped),
    ];
    let envs = v["envelopes"].as_array().unwrap();
    assert_eq!(envs.len(), kinds.len() + answers.len());
    for e in envs {
        let name = e["name"].as_str().unwrap();
        let bytes = hex(&e["envelope"]);
        let seed = a32(&e["ephemeral_seed"]);
        if let Some((_, kind)) = kinds.iter().find(|(n, _)| *n == name) {
            let req = envelope::open_request(*kind, &endpoint, &xk, &bytes).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert_eq!(req.loc(), &loc);
            assert_eq!(req.reply_key(), &answer.public());
            if let Request::Put { piece, .. } = &req {
                assert_eq!(piece, &by_index[0]);
            }
            assert_eq!(envelope::seal_request_with_seed(&endpoint, &xk.public(), &req, &seed).unwrap(), bytes, "{name} reproduces");
        } else {
            let (_, kind) = answers.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("unknown envelope {name}"));
            let want = num(&e["want_len"]);
            let rep = envelope::open_reply(*kind, &answer, &loc, want, &bytes).unwrap_or_else(|err| panic!("{name}: {err}"));
            if name == "got" {
                assert_eq!(rep, Reply::Got(Some(by_index[0].clone())));
            }
            match name {
                "dropped" => assert_eq!(rep, Reply::Dropped(envelope::Dropped::Dropped)),
                "drop unknown" => assert_eq!(rep, Reply::Dropped(envelope::Dropped::Unknown)),
                "drop refused" => assert_eq!(rep, Reply::Dropped(envelope::Dropped::Refused)),
                _ => {}
            }
            assert_eq!(envelope::seal_reply_with_seed(&answer.public(), &loc, &rep, want, &seed).unwrap(), bytes, "{name} reproduces");
        }
    }

    // the manifest: decodes strictly, re-encodes to the same bytes, names this generation, meets its level
    let cbor = hex(&v["manifest"]["cbor"]);
    let m = Manifest::decode(&cbor).unwrap();
    assert_eq!(m.encode(), cbor);
    assert_eq!((m.object, m.class, m.key, m.loc_secret), (object, class, key, loc_secret));
    assert_eq!(m.sealed, scatter::sealed_hash(&sealed));
    assert_eq!(m.hashes.len(), m.n as usize);
    for (i, h) in m.hashes.iter().enumerate() {
        assert_eq!(h.to_vec(), hex(&pieces[i]["hash"]));
    }
    let s = size::Size { k: m.k as usize, n: m.n as usize, m: m.m as usize };
    assert_eq!(size::reading(m.generation, m.places, s), Some(m.level));

    // pointers (S3): strict, canonical, and a repair's update follows by the update rules
    use hoi_types::scatter::{Pointer, Update};
    let pv = &v["pointer"];
    let p = Pointer::decode(&hex(&pv["cbor"])).unwrap();
    assert_eq!(p.encode(), hex(&pv["cbor"]));
    assert_eq!((p.made_at, &p.manifest), (pv["made_at"].as_u64().unwrap(), &m));
    let u = Pointer::decode(&hex(&pv["update"])).unwrap();
    assert_eq!(u.made_at, pv["update_made_at"].as_u64().unwrap());
    assert_eq!(m.update_to(&u.manifest), Ok(Update::Extend));
    assert_eq!(u.manifest.hashes.last().unwrap().to_vec(), hex(&pieces[m.n as usize]["hash"]), "the fresh index is the next one");
    assert!(u.manifest.update_to(&m).is_err(), "never back");
}
