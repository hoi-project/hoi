//! Generates the protocol conformance vectors in core/vectors/ (PROTOCOL.md §13).
//! Everything is deterministic: fixed seeds, fixed times, nonces searched from 0.
//!
//!     cargo run -p hoi-validate --example vectors            # rewrite core/vectors/*.json
//!     cargo run -p hoi-validate --example vectors -- --check # fail if they would change

use std::collections::BTreeSet;
use std::path::PathBuf;

use hoi_crypto::Keypair;
use hoi_moderation::ladder::{self, Contribution, Inputs, LedgerView, ProposalIn, ProposalStatus, ReviewIn, VouchIn};
use hoi_types::cbor::Value as Cbor;
use hoi_types::{Body, Cosig, Genesis, Id, Ladder, LedgerAction, LedgerBody, Limits, MediaRef, Record, Verdict, Wire, hex_encode};
use serde_json::{Value, json};

const T: u64 = 1_790_000_000;
const DAY: u64 = 86_400;

fn key(n: u8) -> Keypair {
    Keypair::from_secret(&[n; 32])
}

/// Stamps at `bits`, signs, and describes the result.
fn make(name: &str, signer: &Keypair, mut r: Record, bits: u32) -> (Value, Wire, Id) {
    r.nonce = hoi_crypto::pow::Puzzle::new(&r, bits).search(0, 1, u64::MAX).expect("nonce");
    let canonical = r.encode();
    let (wire, id) = signer.sign_record(r);
    let v = json!({
        "name": name,
        "kind": wire.record.kind().as_str(),
        "signer_seed": hex_encode(&signer.secret()),
        "signer": signer.public().hex(),
        "record_cbor": hex_encode(&canonical),
        "id": id.hex(),
        "nonce": wire.record.nonce,
        "stamp_bits": hoi_crypto::pow::stamp_bits(&id, wire.record.nonce),
        "sig": hex_encode(&wire.sig.0),
        "wire_cbor": hex_encode(&wire.encode()),
    });
    (v, wire, id)
}

fn crate_share() -> usize {
    hoi_validate::SEALED_SHARE
}

fn rec(community: Option<Id>, author: Id, parent: Option<Id>, at: u64, ttl: u64, body: Body) -> Record {
    Record { v: 1, community, author, parent, created_at: at, expires_at: at + ttl, body, nonce: 0 }
}

/// A 1×1 PNG with a location in a tEXt chunk, and what sanitizing leaves of it.
fn media_sample() -> (Vec<u8>, Vec<u8>) {
    fn crc(d: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &x in d {
            c ^= x as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    for (ty, data) in [
        (&b"IHDR"[..], &[0u8, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0][..]),
        (b"tEXt", b"Location\x0052.37N 4.89E"),
        (b"IDAT", &[0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0, 0, 3, 1, 1, 0]),
        (b"IEND", &[]),
    ] {
        b.extend_from_slice(&(data.len() as u32).to_be_bytes());
        b.extend_from_slice(ty);
        b.extend_from_slice(data);
        b.extend_from_slice(&crc(&[ty, data].concat()).to_be_bytes());
    }
    let (clean, _) = hoi_validate::media::sanitize(&b).expect("sanitizes");
    (b, clean)
}

/// A small community run through the ladder (PROTOCOL.md §15, §16): inputs and the outcome
/// every implementation must reach.
/// PDQ: one sanitized picture and its eight hashes, as shown and turned or
/// mirrored every way. Floats round differently across hosts and implementations, so readers
/// compare within `tolerance` bits, never exactly.
fn pdq_vectors() -> Value {
    use hoi_validate::{media, similar};
    let side = 96u32;
    let img = image::GrayImage::from_fn(side, side, |x, y| {
        let (u, v) = (x as f32 / side as f32, y as f32 / side as f32);
        let ring = (((u - 0.35).powi(2) + (v - 0.6).powi(2)).sqrt() * 17.0).sin();
        image::Luma([((u * 140.0) + (ring + 1.0) * 55.0) as u8])
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(img).write_to(&mut png, image::ImageFormat::Png).expect("png");
    let (clean, info) = media::sanitize(&png.into_inner()).expect("sanitizes");
    let v = similar::examine_variants(info.mime, &clean).expect("decodes").expect("has a fingerprint");
    json!({
        "note": "PDQ hashes of `picture` (sanitized) as shown, then mirrored left to right, upside down, turned half way, and those four transposed (hoi_validate::similar::Variants). Compare within `tolerance` bits: floats differ across hosts. A picture matches a listed hash when any variant is within `match_distance`.",
        "mime": info.mime,
        "picture": hex_encode(&clean),
        "quality": v.quality,
        "hashes": v.hashes.iter().map(similar::to_hex).collect::<Vec<_>>(),
        "tolerance": 4,
        "match_distance": similar::MATCH_DISTANCE,
    })
}

fn ladder_scenario() -> Value {
    let k = |n: u8| Id([n; 32]);
    let mut n = 0u8;
    let mut next = || {
        n += 1;
        Id::hash(&[n])
    };
    let lad = Ladder { admit_vouches: 2, established_age: 10 * DAY, hide_quorum: 3, block_quorum: 5, veto_quorum: 5, ..Ladder::default() };
    let mut lv = LedgerView { owners: [k(1)].into(), genesis_at: T, retention: 180 * DAY, ladder: lad.clone(), ..Default::default() };
    let mut inp = Inputs { now: T + 30 * DAY, ..Default::default() };
    let mut vouch = |from: u8, to: u8, value: i64, at: u64, inp: &mut Inputs| {
        let id = next();
        inp.vouches.push(VouchIn { id, voucher: k(from), subject: k(to), value, seq: 1, created_at: at });
    };
    // the owner (1) vouches 2, 3, 5, 6 in on day 0 (a quarter voice each); 2 and 3 vouch 4 in on
    // day 11 (a quarter too: nobody rises above their strongest voucher)
    for p in [2, 3, 5, 6] {
        vouch(1, p, 1, T + p as u64, &mut inp);
    }
    vouch(2, 4, 1, T + 11 * DAY, &mut inp);
    vouch(3, 4, 1, T + 11 * DAY + 1, &mut inp);
    // 7 is vouched by 4 on day 12, before 4 is established: it doesn't count
    vouch(4, 7, 1, T + 12 * DAY, &mut inp);
    // 9 posts; 2, 3, 5, 6 hide it as spam (one voice together)
    let spam = Id::hash(b"spam post");
    inp.contributions.push(Contribution { id: spam, author: k(9), created_at: T + 29 * DAY });
    inp.authors.insert(spam, k(9));
    // 2 and 5 are around (a live post each); 3, 4 and 6 are away
    for p in [2u8, 5] {
        let id = Id::hash(&[b'p', p]);
        inp.contributions.push(Contribution { id, author: k(p), created_at: T + 28 * DAY });
        inp.authors.insert(id, k(p));
    }
    let mut m = 100u8;
    let mut review = |r: u8, target: Id, verdict: Verdict, reason: Option<&str>, at: u64, inp: &mut Inputs| {
        m += 1;
        inp.reviews.push(ReviewIn { id: Id::hash(&[m]), reviewer: k(r), target, verdict, reason: reason.map(Into::into), seq: 1, created_at: at });
    };
    for r in [2, 3, 5, 6] {
        review(r, spam, Verdict::Hide, Some("spam"), T + 29 * DAY + 60, &mut inp);
    }
    // the owner proposes new limits on day 29: 2 agrees, 5 objects (a quarter voice, under half
    // a voice); 3, 4 and 6 are away and silent, so it is in effect until the deadline
    let proposal = Id::hash(b"proposal");
    lv.proposals.push(ProposalIn { id: proposal, at: T + 29 * DAY, signers: vec![k(1)], ladder: lad.clone(), deadline: T + 209 * DAY });
    review(2, proposal, Verdict::Ok, None, T + 29 * DAY + 10, &mut inp);
    review(5, proposal, Verdict::Veto, Some("too strict"), T + 29 * DAY + 20, &mut inp);
    let (st, _, _) = ladder::settle(&inp, &mut |_| lv.clone(), 6);
    let people: serde_json::Map<String, Value> = st
        .people
        .iter()
        .map(|(p, x)| {
            (
                p.hex(),
                json!({ "level": x.level().as_str(), "admitted_at": x.admitted_at, "vouchers": x.vouchers.iter().map(|v| v.hex()).collect::<Vec<_>>(), "blocked": x.blocked, "voice": x.voice }),
            )
        })
        .collect();
    let tally = &st.proposals[&proposal];
    let status_of = |s: ProposalStatus| match s {
        ProposalStatus::Open => "open",
        ProposalStatus::InEffect => "in-effect",
        ProposalStatus::Passed { .. } => "passed",
        ProposalStatus::Lapsed => "lapsed",
    };
    let status = status_of(tally.status);
    // a retirement: 04 last signed something on day 12; the owner retires 04 on day
    // 45, and from day 52 its silence counts for nothing
    let mut lv_r = lv.clone();
    lv_r.retired.push((k(4), T + 45 * DAY));
    let mut inp_r = inp.clone();
    inp_r.now = T + 60 * DAY;
    for (who, at) in inp
        .vouches
        .iter()
        .map(|v| (v.voucher, v.created_at))
        .chain(inp.reviews.iter().map(|r| (r.reviewer, r.created_at)))
        .chain(inp.contributions.iter().map(|c| (c.author, c.created_at)))
    {
        let e = inp_r.last_seen.entry(who).or_insert(at);
        *e = (*e).max(at);
    }
    let (st_r, _, _) = ladder::settle(&inp_r, &mut |_| lv_r.clone(), 6);
    let t_r = &st_r.proposals[&proposal];
    json!({
        "note": "Keys are 32 repeated bytes (01…, 02…). Owner 01. Times are seconds. Voices are in units of voice_unit. Hides from established members weighing one voice put the post to sleep and block its newcomer author; 07's only vouch came from 04 before 04 was established. The proposal is in effect while everyone who hasn't agreed is away or objects with less than half a voice.",
        "voice_unit": ladder::VOICE,
        "vouch_epoch": ladder::VOUCH_EPOCH,
        "ladder": { "admit_vouches": lad.admit_vouches, "established_age": lad.established_age, "hide_quorum": lad.hide_quorum, "block_quorum": lad.block_quorum, "veto_quorum": lad.veto_quorum, "trusted_posts": lad.trusted_posts, "media_score": lad.media_score },
        "owners": [k(1).hex()],
        "genesis_at": T,
        "now": inp.now,
        "vouches": inp.vouches.iter().map(|v| json!({ "voucher": v.voucher.hex(), "subject": v.subject.hex(), "value": v.value, "created_at": v.created_at })).collect::<Vec<_>>(),
        "posts": inp.contributions.iter().map(|c| json!({ "id": c.id.hex(), "author": c.author.hex(), "created_at": c.created_at })).collect::<Vec<_>>(),
        "reviews": inp.reviews.iter().map(|r| json!({ "reviewer": r.reviewer.hex(), "target": r.target.hex(), "verdict": r.verdict.as_str(), "reason": r.reason, "created_at": r.created_at })).collect::<Vec<_>>(),
        "proposals": [{ "id": proposal.hex(), "at": T + 29 * DAY, "signers": [k(1).hex()], "deadline": T + 209 * DAY }],
        "expect": {
            "established": st.established,
            "established_voice": st.established_voice,
            "quorums": { "admit": st.quorums.admit, "hide": st.quorums.hide, "block": st.quorums.block, "veto": st.quorums.veto },
            "hidden": st.hidden.keys().map(|h| h.hex()).collect::<Vec<_>>(),
            "people": people,
            "proposals": { proposal.hex(): { "status": status, "electors": tally.electors.len(), "agreed_voice": tally.agreed_voice, "objecting_voice": tally.objecting_voice, "away": tally.away.len() } },
        },
        "retirement": {
            "note": "The same community on day 60, after the owner retired 04 on day 45: a retirement counts for someone silent, and a member, for 30 days before it, takes effect 7 days on, lapses if they sign anything again, and excuses only silence. last_seen is the latest time each person signed anything.",
            "retired": [{ "person": k(4).hex(), "at": T + 45 * DAY }],
            "now": inp_r.now,
            "last_seen": inp_r.last_seen.iter().map(|(p, t)| (p.hex(), json!(t))).collect::<serde_json::Map<_, _>>(),
            "expect": { "status": status_of(t_r.status), "retired": t_r.retired.iter().map(|p| p.hex()).collect::<Vec<_>>(), "away": t_r.away.iter().map(|p| p.hex()).collect::<Vec<_>>(), "retired_voice": t_r.retired_voice },
        },
    })
}

/// Scattered delivery (PROTOCOL.md §21, SCATTER.md): sizing for every level, a sealed object, one
/// generation of it cut 15 of 28 with its whole budget of 35 indexes, locators, piece hashes,
/// drops, a published holder key, every envelope, and the manifest. Secrets, nonces and
/// ephemeral seeds are fixed here (hoi-crypto's `deterministic` feature) so the file reproduces;
/// real devices draw them all from the system.
fn scatter_vectors() -> Value {
    use hoi_crypto::envelope::{self, DropAuth, Reply, Request, Stored, XKey};
    use hoi_crypto::scatter::{self, Shards, size};
    use hoi_types::scatter::Manifest;

    let h = |b: &[u8]| hex_encode(b);
    const MORE: u32 = u32::MAX;

    // ---- sizing: the answer for each level, generation and number of places seen
    let mut cases = Vec::new();
    for &level in &size::LEVELS {
        for g in [0u8, 1, 2, 4, 8, 16] {
            for places in [9u32, 11, 12, 15, 20, 30, 40, 50, 60, 80, 90, 100, 150, 200, 300, 500, 1000, 1001, 5000, MORE] {
                let s = size::size(level, g, places);
                cases.push(json!({
                    "level": level, "generation": g, "places": places,
                    "size": s.map(|s| json!({ "k": s.k, "n": s.n, "m": s.m, "hard": size::hard(level, places, s.k) })),
                }));
            }
        }
    }
    let readings: Vec<Value> = [
        (0u8, MORE, 15usize, 28usize, 35usize),
        (0, MORE, 39, 71, 89),
        (8, MORE, 15, 28, 35),
        (0, MORE, 3, 6, 6),
        (0, 30, 5, 12, 15),
        (0, 80, 9, 18, 23),
    ]
    .iter()
    .map(
        |&(g, places, k, n, m)| json!({ "generation": g, "places": places, "k": k, "n": n, "m": m, "level": size::reading(g, places, size::Size { k, n, m }) }),
    )
    .collect();

    // ---- Reed–Solomon alone: three data pieces, any index
    let rs_data: Vec<Vec<u8>> = vec![b"scatter".to_vec(), b"gathers".to_vec(), b"quietly".to_vec()];
    let code = scatter::rs::Code::new(3);
    let refs: Vec<&[u8]> = rs_data.iter().map(|d| d.as_slice()).collect();
    let rs_pieces: Vec<Value> = [0usize, 1, 2, 3, 4, 5, 100, 255].iter().map(|&i| json!({ "index": i, "piece": h(&code.encode(&refs, i)) })).collect();

    // ---- an object, sealed
    let content = b"a post, padded to its class, sealed under its own key, then cut into pieces".to_vec();
    let object = [0x0b; 32];
    let obj_key = [0x0c; 32];
    // the nonce is synthetic: the same content, id and key always seal alike
    let (class, sealed) = scatter::seal_object(&content, &object, &obj_key).expect("seal");
    let nonce = sealed[..24].to_vec();
    assert_eq!(scatter::open_object(&sealed, &object, class, &obj_key).unwrap(), content);

    // ---- generation 0 at the default level in a large network
    let g0 = size::size(size::DEFAULT_LEVEL, 0, MORE).expect("size");
    let aont_key = [0x0e; 32];
    let loc_secret = [0x0f; 32];
    let shards = Shards::new_with_key(&sealed, class, g0.k, &aont_key).expect("shards");
    let mut pieces = Vec::new();
    let mut hashes = Vec::new();
    for i in 0..g0.m as u8 {
        let p = shards.piece(i);
        let hash = scatter::piece_hash(&object, 0, i, &p);
        hashes.push(hash);
        pieces.push(json!({ "index": i, "piece": h(&p), "hash": h(&hash), "locator": h(&scatter::locator(&loc_secret, &object, 0, i)) }));
    }
    // subsets a reader might hold: the data pieces, the last k indexes, every other one
    let subsets: Vec<Vec<u8>> =
        vec![(0..g0.k as u8).collect(), ((g0.m - g0.k) as u8..g0.m as u8).collect(), (0..g0.m as u8).filter(|i| i % 2 == 1).take(g0.k).collect()];
    for sub in &subsets {
        let ps: Vec<(u8, Vec<u8>)> = sub.iter().map(|&i| (i, shards.piece(i))).collect();
        let refs: Vec<(u8, &[u8])> = ps.iter().map(|(i, p)| (*i, p.as_slice())).collect();
        assert_eq!(Shards::rebuild(class, g0.k, &refs).unwrap().sealed().unwrap(), sealed);
    }

    // ---- drops: the creator's token, and the community drop key
    let loc0 = scatter::locator(&loc_secret, &object, 0, 0);
    let drop_secret = [0x10; 32];
    let token = scatter::drop_token(&drop_secret, &loc0);
    let drop_key = key(0x11);
    let statement = scatter::drop_statement(&loc0);
    let drop_sig = drop_key.sign(&statement);

    // ---- a holder publishes its key; a reader and a sender use one-time answer keys
    let node = key(0x12);
    let endpoint = node.public().0;
    let xk = XKey::from_seed(&[0x13; 32]).expect("xk");
    let xk_expires = T + 30 * DAY;
    let xk_statement = envelope::xk_statement(&endpoint, &xk.public(), xk_expires);
    let xk_sig = node.sign(&xk_statement);
    let answer = XKey::from_seed(&[0x14; 32]).expect("answer key");
    let piece_len = class.piece_len(g0.k);
    let first = shards.piece(0);
    let requests = [
        (
            "put",
            Request::Put {
                loc: loc0,
                reply: answer.public(),
                expiry_day: ((T + 90 * DAY) / DAY) as u32,
                drop_hash: scatter::drop_hash(&token),
                drop_key_id: scatter::drop_key_id(&drop_key.public().0),
                stamp: 7,
                piece: first.clone(),
            },
        ),
        ("get", Request::Get { loc: loc0, reply: answer.public(), want_len: piece_len as u32 }),
        ("has", Request::Has { loc: loc0, reply: answer.public() }),
        ("drop by token", Request::Drop { loc: loc0, reply: answer.public(), auth: DropAuth::Token(token) }),
        ("drop by drop key", Request::Drop { loc: loc0, reply: answer.public(), auth: DropAuth::Signed { key: drop_key.public().0, sig: drop_sig.0 } }),
    ];
    let mut envelopes = Vec::new();
    for (i, (name, req)) in requests.iter().enumerate() {
        let seed = [0x20 + i as u8; 32];
        let env = envelope::seal_request_with_seed(&endpoint, &xk.public(), req, &seed).expect("seal request");
        assert_eq!(&envelope::open_request(req.kind(), &endpoint, &xk, &env).unwrap(), req);
        envelopes.push(json!({ "name": name, "to": "holder", "ephemeral_seed": h(&seed), "envelope": h(&env) }));
    }
    let replies = [
        ("stored", Reply::Stored(Stored::Ok), 0),
        ("got", Reply::Got(Some(first.clone())), piece_len),
        ("got nothing", Reply::Got(None), piece_len),
        ("had", Reply::Had(Some(piece_len as u32)), 0),
        ("dropped", Reply::Dropped(envelope::Dropped::Dropped), 0),
    ];
    for (i, (name, rep, want)) in replies.iter().enumerate() {
        let seed = [0x30 + i as u8; 32];
        let env = envelope::seal_reply_with_seed(&answer.public(), &loc0, rep, *want, &seed).expect("seal reply");
        assert_eq!(&envelope::open_reply(rep.kind(), &answer, &loc0, *want, &env).unwrap(), rep);
        envelopes.push(json!({ "name": name, "to": "answer key", "want_len": want, "ephemeral_seed": h(&seed), "envelope": h(&env) }));
    }

    // ---- the manifest of generation 0 as its pointer record carries it, sealed
    let holders: Vec<Id> = (0..g0.n as u8).map(|i| key(0x40 + i).public()).collect();
    let manifest = Manifest {
        object,
        sealed: scatter::sealed_hash(&sealed),
        class,
        generation: 0,
        level: size::DEFAULT_LEVEL,
        places: MORE,
        k: g0.k as u16,
        n: g0.n as u16,
        m: g0.m as u16,
        hashes: hashes[..g0.n].to_vec(),
        holders,
        loc_secret,
        key: obj_key,
        expires_at: T + 90 * DAY,
        keys: Vec::new(),
    };
    let manifest_cbor = manifest.encode();
    assert_eq!(Manifest::decode(&manifest_cbor).unwrap(), manifest);

    // ---- pointers (S3): the manifest as members pass it, and a repair's update of it
    let pointer = hoi_types::scatter::Pointer { made_at: T + 60, manifest: manifest.clone() };
    let pointer_cbor = pointer.encode();
    let mut repaired = manifest.clone();
    let fresh = g0.n as u8;
    repaired.hashes.push(hashes[fresh as usize]);
    repaired.holders.push(key(0x40 + fresh).public());
    assert_eq!(manifest.update_to(&repaired), Ok(hoi_types::scatter::Update::Extend));
    let update_cbor = hoi_types::scatter::Pointer { made_at: T + 2 * DAY, manifest: repaired }.encode();

    json!({
        "note": "Scattered delivery (PROTOCOL.md §21). `sizing` answers size(level, generation, places): the smallest k of n (and index budget m) a party at the level's share of the places seen cannot rebuild or withhold, null when too few places; `places` above 1000 is binomial, 4294967295 stands for any large network. `object` is padded to its class and sealed with XChaCha20-Poly1305 (AAD \"hoi scatter object v1\" ‖ object ‖ class). `generation` wraps it in the all-or-nothing transform under `aont_key` and cuts it with systematic Reed–Solomon over GF(2^8) (0x11D, Vandermonde rows r^c times the inverse of the top k×k block); every listed subset of k indexes rebuilds `sealed`. Envelopes are HPKE base mode, DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305, ephemeral key derived from `ephemeral_seed`. Fixed secrets here are for reproduction only.",
        "sizing": { "life": size::LIFE, "exact_up_to": size::EXACT_UP_TO, "links": size::LINKS, "cases": cases, "readings": readings },
        "reed_solomon": { "k": 3, "data": rs_data.iter().map(|d| h(d)).collect::<Vec<_>>(), "pieces": rs_pieces },
        "object": { "content": h(&content), "object": h(&object), "key": h(&obj_key), "nonce": h(&nonce), "class": class.index(), "sealed": h(&sealed), "sealed_hash": h(&scatter::sealed_hash(&sealed)) },
        "generation": {
            "generation": 0, "level": size::DEFAULT_LEVEL, "k": g0.k, "n": g0.n, "m": g0.m,
            "aont_key": h(&aont_key), "loc_secret": h(&loc_secret), "piece_len": piece_len,
            "pieces": pieces, "rebuild_from": subsets,
        },
        "drops": {
            "drop_secret": h(&drop_secret), "loc": h(&loc0), "token": h(&token), "hash": h(&scatter::drop_hash(&token)),
            "drop_key": h(drop_key.public().as_bytes()), "drop_key_id": h(&scatter::drop_key_id(&drop_key.public().0)),
            "statement": h(&statement), "sig": h(&drop_sig.0),
        },
        "holder": {
            "endpoint": h(&endpoint), "xk_seed": h(&[0x13; 32]), "xk": h(&xk.public()), "key_id": h(&envelope::key_id(&xk.public())),
            "expires_at": xk_expires, "statement": h(&xk_statement), "sig": h(&xk_sig.0),
            "answer_key_seed": h(&[0x14; 32]), "answer_key": h(&answer.public()),
        },
        "envelopes": envelopes,
        "manifest": { "cbor": h(&manifest_cbor), "holder_seeds_from": 0x40, "expires_at": T + 90 * DAY },
        "pointer": { "cbor": h(&pointer_cbor), "made_at": T + 60, "update": h(&update_cbor), "update_made_at": T + 2 * DAY, "update_kind": "extend" },
    })
}

fn main() {
    let check = std::env::args().any(|a| a == "--check");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vectors");
    let (ckey, root, device, mira, jonas) = (key(1), key(2), key(3), key(4), key(5));
    let c = ckey.public();
    let limits = Limits { pow_bits: 8, ..Limits::default() };
    let bits = 8;

    // ---------------------------------------------------------------- records of every kind
    let mut records = Vec::new();
    let genesis = Genesis {
        name: "privacy".into(),
        description: "Practical privacy for normal people.".into(),
        color: "mint".into(),
        owners: vec![root.public(), mira.public(), jonas.public()],
        threshold: 2,
        rules_text_cid: None,
        limits: limits.clone(),
    };
    let (v, gw, gid) = make("genesis", &ckey, rec(Some(c), c, None, T, 100 * 365 * DAY, Body::Community(genesis)), bits);
    records.push(v);
    // identity work is searched from nonce 0 at the protocol floor, so it is deterministic
    let work = hoi_crypto::identity::search(&root.public(), hoi_crypto::identity::MIN_BITS, 0, 1, u64::MAX).expect("work");
    let (mut v, iw, _) = make("identity", &root, rec(None, root.public(), None, T, 100 * 365 * DAY, Body::Identity { work, rc: None, of: None }), bits);
    v["identity_bits"] = json!(hoi_crypto::identity::work_bits(&root.public(), work));
    records.push(v);
    let scope = vec!["*".to_string()];
    let root_sig = hoi_crypto::bless(&root, &device.public(), &scope, T + 180 * DAY);
    let deleg = Body::Delegation { root: root.public(), scope, expires_at: T + 180 * DAY, root_sig, seal: None };
    let (v, dw, did) = make("delegation", &device, rec(None, device.public(), None, T, 180 * DAY, deleg), bits);
    records.push(v);
    let post = Body::Post {
        title: "Which messengers survived the audit?".into(),
        text: "Short version: three apps close the gap.".into(),
        link: None,
        name: Some("lena".into()),
        media: Vec::new(),
    };
    let (v, pw, pid) = make("post", &device, rec(Some(c), device.public(), None, T + 60, 180 * DAY, post), bits);
    records.push(v);
    let (png_dirty, png_clean) = media_sample();
    let photo = Body::Post {
        title: "The meetup, from the back".into(),
        text: "".into(),
        link: None,
        name: Some("mira".into()),
        media: vec![MediaRef {
            hash: Id::hash(&png_clean),
            mime: "image/png".into(),
            size: png_clean.len() as u64,
            w: Some(1),
            h: Some(1),
            alt: Some("one pixel".into()),
        }],
    };
    let (v, ..) = make("post-media", &mira, rec(Some(c), mira.public(), None, T + 70, 180 * DAY, photo), bits);
    records.push(v);
    let comment = Body::Comment { text: "Defaults count, not capability.".into(), name: Some("mira".into()) };
    let (v, cw, cid) = make("comment", &mira, rec(Some(c), mira.public(), Some(pid), T + 120, 180 * DAY, comment), bits);
    records.push(v);
    let (v, ..) = make("vote", &mira, rec(Some(c), mira.public(), Some(pid), T + 130, 180 * DAY, Body::Vote { value: 1, seq: 1 }), bits);
    records.push(v);
    let (v, ..) =
        make("vouch", &mira, rec(Some(c), mira.public(), Some(root.public()), T + 135, 100 * 365 * DAY, Body::Vouch { value: 1, seq: 1, reason: None }), bits);
    records.push(v);
    let review = Body::Review { verdict: Verdict::Hide, seq: 1, reason: Some("off-topic".into()) };
    let (v, ..) = make("review", &jonas, rec(Some(c), jonas.public(), Some(pid), T + 138, 180 * DAY, review), bits);
    records.push(v);
    let (v, ..) = make(
        "edit",
        &device,
        rec(Some(c), device.public(), Some(pid), T + 140, 180 * DAY, Body::Edit { text: "Edited: three apps.".into(), title: None }),
        bits,
    );
    records.push(v);
    let (v, ..) = make(
        "label",
        &jonas,
        rec(Some(c), jonas.public(), Some(pid), T + 150, 180 * DAY, Body::Label { label: "needs a threat model".into(), note: None }),
        bits,
    );
    records.push(v);
    let report = Body::Report { reason: "rule:4".into(), evidence: vec![cid] };
    let (v, ..) = make("report", &jonas, rec(Some(c), jonas.public(), Some(mira.public()), T + 160, 30 * DAY, report), bits);
    records.push(v);
    let (v, ..) = make("delete", &mira, rec(Some(c), mira.public(), Some(cid), T + 170, 180 * DAY, Body::Delete), bits);
    records.push(v);
    let (v, ..) = make("revoke", &root, rec(None, root.public(), None, T + 180, 100 * 365 * DAY, Body::Revoke { device: device.public() }), bits);
    records.push(v);
    // a post kept in pieces: its pointer, the manifest sealed under the community's read key
    // (the sealed bytes stand in here; PROTOCOL.md §21 and scatter.json give their format)
    let pointer = Body::Pointer { record: pid, of: hoi_types::Kind::Post, media: None, sealed: vec![0x5a; 96] };
    let (v, ..) = make("pointer", &device, rec(Some(c), device.public(), None, T + 190, 30 * DAY, pointer), bits);
    records.push(v);
    // the holders a member's durable device saw up for days, sorted
    let mut holders = vec![key(40).public(), key(41).public(), key(42).public()];
    holders.sort();
    let (v, ..) = make("roster", &device, rec(Some(c), device.public(), None, T + 200, 3 * DAY, Body::Roster { holders }), bits);
    records.push(v);
    // a root replaced under a recovery key: the line's master secret derives every
    // root and recovery key; the first root commits to the next recovery key, the new root names
    // the person it continues, and the recovery key signs the rotation
    let master = [0x5e; 32];
    let first = hoi_crypto::recovery::root_at(&master, 0);
    let next = hoi_crypto::recovery::root_at(&master, 1);
    let recovery = hoi_crypto::recovery::recovery_at(&master, 1);
    let rc = |g: u32| Some(hoi_crypto::recovery::commitment(&hoi_crypto::recovery::recovery_at(&master, g).public()));
    let work = hoi_crypto::identity::search(&first.public(), hoi_crypto::identity::MIN_BITS, 0, 1, u64::MAX).expect("work");
    let (v, ..) = make("identity-committed", &first, rec(None, first.public(), None, T, 100 * 365 * DAY, Body::Identity { work, rc: rc(1), of: None }), bits);
    records.push(v);
    let work = hoi_crypto::identity::search(&next.public(), hoi_crypto::identity::MIN_BITS, 0, 1, u64::MAX).expect("work");
    let continues = Body::Identity { work, rc: rc(2), of: Some(first.public()) };
    let (v, ..) = make("identity-continues", &next, rec(None, next.public(), None, T + 300, 100 * 365 * DAY, continues), bits);
    records.push(v);
    let rotate = Body::Rotate { person: first.public(), to: next.public() };
    let (v, ..) = make("rotate", &recovery, rec(None, recovery.public(), None, T + 300, 100 * 365 * DAY, rotate), bits);
    records.push(v);
    // a share of a recovery line for one friend, sealed to each of their devices (81 bytes each)
    let share = Body::Share { sealed: vec![(key(43).public(), vec![0x5b; crate_share()])], seq: 1 };
    let (v, ..) = make("share", &device, rec(Some(c), device.public(), Some(mira.public()), T + 310, 100 * 365 * DAY, share), bits);
    records.push(v);

    // ---------------------------------------------------------------- delegation chain
    let chain = json!({
        "root": root.public().hex(),
        "device": device.public().hex(),
        "delegation_id": did.hex(),
        "post_id": pid.hex(),
        "note": "The root's identity record (Argon2id work, PROTOCOL.md §4.1) comes first. The delegation is signed by the device (its consent) and carries root_sig, the root's signature over delegation_statement(root, device, scope, expires_at). The post is signed by the device key: receivers MUST hold both to accept it. The revoke at T+180 stops records from the device created at or after that time.",
        "statement": hoi_types::delegation_statement(&root.public(), &device.public(), &["*".to_string()], T + 180 * DAY).hex(),
        "wires": [hex_encode(&iw.encode()), hex_encode(&dw.encode()), hex_encode(&pw.encode())],
    });

    // ---------------------------------------------------------------- tombstoned post (remove)
    let remove = LedgerBody {
        seq: 1,
        action: LedgerAction::Remove,
        target: Some(cid),
        reason: Some("rule 4, undisclosed vendor".into()),
        evidence: None,
        cosign: None,
        text: None,
        limits: None,
        drop_key: None,
        patch: None,
    };
    let (rv, rw, _) = make("ledger-remove", &mira, rec(Some(c), mira.public(), None, T + 200, 180 * DAY, Body::Ledger(remove)), bits);
    let tomb = json!({
        "comment_id": cid.hex(),
        "comment_wire": hex_encode(&cw.encode()),
        "remove": rv,
        "note": "mira is an owner. After the remove, compliant nodes delete the comment's body, keep its id and the entry, and refuse to ingest it again.",
        "remove_wire": hex_encode(&rw.encode()),
    });

    // ---------------------------------------------------------------- threshold ledger entry
    let mut rules = Record {
        v: 1,
        community: Some(c),
        author: root.public(),
        parent: None,
        created_at: T + 300,
        expires_at: T + 300 + 100 * 365 * DAY,
        body: Body::Ledger(LedgerBody {
            seq: 2,
            action: LedgerAction::Rules,
            target: None,
            reason: None,
            evidence: None,
            cosign: None,
            text: Some("1. Be kind.\n2. Disclose vendors.".into()),
            limits: None,
            drop_key: None,
            patch: None,
        }),
        nonce: 0,
    };
    let proposal = rules.proposal_id();
    if let Body::Ledger(l) = &mut rules.body {
        l.cosign =
            Some(vec![Cosig { key: root.public(), sig: root.sign(proposal.as_bytes()) }, Cosig { key: jonas.public(), sig: jonas.sign(proposal.as_bytes()) }]);
    }
    let (tv, ..) = make("ledger-rules-2-of-3", &root, rules, bits);
    let threshold = json!({
        "proposal_id": proposal.hex(),
        "note": "Cosigners sign proposal_id = BLAKE3(canonical record with body.cosign removed, nonce 0, author zeroed). Threshold 2 of owners [root, mira, jonas]; signed by root (publisher) and jonas.",
        "entry": tv,
    });

    // ---------------------------------------------------------------- sync transcript
    // Two nodes share 10 records and each holds 10 the other lacks (ids only; the protocol
    // under test is the message flow, PROTOCOL.md §7).
    let ids: Vec<Id> = (0u8..30).map(|i| Id::hash(&[i])).collect();
    let a: Vec<Id> = ids[0..20].to_vec();
    let b: Vec<Id> = ids[10..30].to_vec();
    let have_a = hoi_sync::Msg::Have { community: c, ids: a.clone() };
    let set_b: BTreeSet<Id> = b.iter().copied().collect();
    let want_b = hoi_sync::Msg::Want { ids: hoi_sync::missing(&a, |i| set_b.contains(i)) };
    let have_b = hoi_sync::Msg::Have { community: c, ids: b.clone() };
    let set_a: BTreeSet<Id> = a.iter().copied().collect();
    let want_a = hoi_sync::Msg::Want { ids: hoi_sync::missing(&b, |i| set_a.contains(i)) };
    let hello = hoi_sync::Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([7; 32]),
        name: Some("lena's browser".into()),
        token: Some("t0k3n".into()),
        communities: vec![c],
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
    let sync = json!({
        "a_ids": a.iter().map(|i| i.hex()).collect::<Vec<_>>(),
        "b_ids": b.iter().map(|i| i.hex()).collect::<Vec<_>>(),
        "messages": [
            { "from": "a", "msg": hex_encode(&hello.encode()) },
            { "from": "a", "msg": hex_encode(&have_a.encode()) },
            { "from": "b", "msg": hex_encode(&want_b.encode()), "wants": 10 },
            { "from": "b", "msg": hex_encode(&have_b.encode()) },
            { "from": "a", "msg": hex_encode(&want_a.encode()), "wants": 10 },
        ],
        "note": "After both `want`s are answered with `recs`, both sides hold all 30 ids. Range-based reconciliation (Negentropy) will replace full `have` lists without changing the other messages.",
    });

    let invite = hoi_types::Invite { community: c, genesis: Some(gw.encode()), ..Default::default() };
    // Preserve the original unmasked code as a backward-compatibility vector.
    let v1_code = hoi_types::uri::b64(
        &Cbor::map([("v", Cbor::U(1)), ("c", Cbor::B(c.as_bytes().to_vec())), ("r", Cbor::A(vec![])), ("p", Cbor::A(vec![])), ("g", Cbor::B(gw.encode()))])
            .encode(),
    );
    let legacy_query = format!("c={}&g={}", hoi_types::uri::zbase32_encode(c.as_bytes()), hoi_types::uri::b64(&gw.encode()));
    let routed = hoi_types::Invite {
        peers: vec![format!("rtc:{}@203.0.113.7:7844", "q".repeat(43)), format!("{}@[2001:db8::7]:7843", c.hex())],
        relays: vec!["https://relay.example:443".into()],
        invite: Some(gid),
        trackers: vec!["https://tracker.example".into(), "https://registry.example/registry".into()],
        ..invite.clone()
    };
    let uris = json!({
        "community": c.hex(),
        "community_zbase32": hoi_types::uri::zbase32_encode(c.as_bytes()),
        "invite": invite.to_uri(),
        "invite_web": invite.to_web("https://hoi.sh/app/"),
        "v1_invite": format!("hoi://join?code={v1_code}"),
        "v1_invite_web": format!("https://hoi.sh/app/#/join?code={v1_code}"),
        "legacy_invite": format!("hoi://join?{legacy_query}"),
        "legacy_invite_web": format!("https://hoi.sh/app/#/join?{legacy_query}"),
        "routed_invite": routed.to_uri(),
        "routed_peers": routed.peers,
        "routed_relays": routed.relays,
        "routed_trackers": routed.trackers,
        "did_key_root": hoi_types::did_key(&root.public()),
        "genesis_id": gid.hex(),
    });

    // ---------------------------------------------------------------- media sanitizing
    let media = json!({
        "note": "sanitize() strips everything but the picture: the tEXt chunk (a location) goes, the rest is copied as is. check() accepts the clean bytes and refuses the dirty ones. MediaRef.hash is BLAKE3 of the clean bytes.",
        "mime": "image/png",
        "input": hex_encode(&png_dirty),
        "sanitized": hex_encode(&png_clean),
        "hash": Id::hash(&png_clean).hex(),
        "w": 1,
        "h": 1,
    });

    let hello_hints = hoi_sync::Msg::Hello {
        v: hoi_sync::SYNC_VERSION,
        node: Id([1; 32]),
        name: None,
        token: None,
        communities: vec![Id([2; 32])],
        salt: None,
        hashed: Vec::new(),
        serves: false,
        hints: vec![hoi_sync::Hints { community: Id([2; 32]), peers: vec![Id([3; 32]).hex()], relays: vec!["https://relay.example".into()] }],
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

    // ---------------------------------------------------------------- hellos to strangers and devices (PROTOCOL.md §14)
    // A stranger hears in the clear only the community whose routes led to it; the rest are
    // salted hashes it can match only for keys it already holds. A home node answers a device
    // that spent its pairing link with that device's own credential, the next link and the
    // link key.
    let salted_hello = {
        let salt = [0x5a; 16];
        let other = key(6).public();
        let mut hashed = vec![hoi_sync::community_hash(&salt, &other)];
        hashed.sort();
        let stranger = hoi_sync::Msg::Hello {
            v: hoi_sync::SYNC_VERSION,
            node: Id([8; 32]),
            name: None,
            token: None,
            communities: vec![c],
            salt: Some(salt),
            hashed,
            serves: false,
            hints: Vec::new(),
            routes: Vec::new(),
            features: vec![hoi_sync::FEATURE_CATALOG.into()],
            pairing: None,
            link_key: None,
            notice_hash: None,
            notice: None,
            holder: None,
            forwards: false,
            rebuilds: false,
        };
        let mut both = vec![c, other];
        both.sort();
        let to_device = hoi_sync::Msg::Hello {
            v: hoi_sync::SYNC_VERSION,
            node: Id([9; 32]),
            name: Some("lena's laptop".into()),
            token: Some("ZGV2aWNlIGNyZWRlbnRpYWwgZm9yIGEgdGVzdCB2ZWN0b3I".into()),
            communities: both,
            salt: None,
            hashed: Vec::new(),
            serves: true,
            hints: Vec::new(),
            routes: Vec::new(),
            features: vec![hoi_sync::FEATURE_LINK.into()],
            pairing: Some("bmV4dCBwYWlyaW5nIGxpbmsh".into()),
            link_key: Some([0x4c; 32]),
            notice_hash: None,
            notice: None,
            holder: None,
            forwards: false,
            rebuilds: false,
        };
        json!({
            "note": "Sync version 2. To a stranger, `c` names only the communities whose routes led to this connection; `ch` lists the sender's other public communities as BLAKE3-derive-key(context, n ‖ key), sorted, under the per-connection salt `n` (16 bytes). A receiver matches it against the keys it holds; no other key can be read from it. Between a home node and its devices `c` lists every public community and there is no `n`/`ch`. A home node's hello to a device that showed the pairing link's token carries `token` (the device's own credential from now on), `pt` (the token of the next pairing link) and `lk` (the 32-byte key `link` messages are sealed with).",
            "context": hoi_sync::COMMUNITY_HASH_CONTEXT,
            "salt": hex_encode(&salt),
            "communities": [
                { "key": c.hex(), "hash": hoi_sync::community_hash(&salt, &c).hex(), "named": true },
                { "key": other.hex(), "hash": hoi_sync::community_hash(&salt, &other).hex(), "named": false },
            ],
            "stranger_hello": hex_encode(&stranger.encode()),
            "home_to_device_hello": hex_encode(&to_device.encode()),
        })
    };
    // ---------------------------------------------------------------- warmth (PROTOCOL.md §20)
    // Observations every implementation must read the same way: the level, the score and the
    // copies follow from first-hand observations alone; a connected node's hint only raises
    // `network_copies`.
    let warmth = {
        use hoi_sync::warmth::{Holder, Link, Observation, assess, assess_links, hint};
        let durable = Holder::new(1.0, true);
        let browser = Holder::new(1.0, false);
        let at = |network: u64| Holder { network: Some(network), ..Holder::new(1.0, true) };
        let cases: Vec<(&str, Observation)> = vec![
            ("a browser alone, nothing missing, never synced", Observation { records: 40, now: T, ..Default::default() }),
            (
                "a browser with its home node",
                Observation { records: 40, holders: vec![durable], last_sync: Some(T - 60), now: T, live: true, ..Default::default() },
            ),
            (
                "a desktop with one member's browser connected",
                Observation { own_durable: true, records: 40, holders: vec![browser], now: T, live: true, ..Default::default() },
            ),
            (
                "two members' nodes, complete",
                Observation { records: 40, holders: vec![durable, durable], last_sync: Some(T - 60), now: T, live: true, ..Default::default() },
            ),
            (
                "three members' nodes, complete and fresh",
                Observation { records: 40, holders: vec![durable, durable, durable], last_sync: Some(T - 60), now: T, live: true, ..Default::default() },
            ),
            (
                "three members' nodes, 7% still missing",
                Observation {
                    records: 40,
                    missing: 3,
                    holders: vec![durable, durable, durable],
                    last_sync: Some(T - 60),
                    now: T,
                    live: true,
                    ..Default::default()
                },
            ),
            (
                "two nodes, less than half here",
                Observation { records: 40, missing: 60, holders: vec![durable, durable], now: T, live: true, ..Default::default() },
            ),
            (
                "two nodes, last catch-up eight days ago, nothing connected",
                Observation { records: 40, holders: vec![durable, durable], last_sync: Some(T - 8 * DAY), now: T, ..Default::default() },
            ),
            (
                "partial copies on two nodes",
                Observation {
                    records: 40,
                    holders: vec![Holder::new(0.5, true), Holder::new(0.75, true)],
                    last_sync: Some(T - 60),
                    now: T,
                    live: true,
                    ..Default::default()
                },
            ),
            (
                "a public join still waiting, the carrier listed 12 records",
                Observation { records: 0, missing: 12, holders: vec![durable], now: T, live: true, ..Default::default() },
            ),
            (
                "one node seen first hand, a connected node reports three",
                Observation { records: 40, holders: vec![durable], last_sync: Some(T - 60), now: T, live: true, hinted: 3, ..Default::default() },
            ),
            (
                "media: one of four pictures here",
                Observation {
                    records: 40,
                    media_wanted: 4,
                    media_present: 1,
                    holders: vec![durable],
                    last_sync: Some(T - 60),
                    now: T,
                    live: true,
                    ..Default::default()
                },
            ),
            (
                "three nodes in one network: one place",
                Observation { records: 40, holders: vec![at(7), at(7), at(7)], last_sync: Some(T - 60), now: T, live: true, ..Default::default() },
            ),
            (
                "three nodes in three networks: three places",
                Observation { records: 40, holders: vec![at(1), at(2), at(3)], last_sync: Some(T - 60), now: T, live: true, ..Default::default() },
            ),
            (
                "one operator in two networks, and a node of its own",
                Observation {
                    records: 40,
                    holders: vec![Holder { operator: Some(42), ..at(1) }, Holder { operator: Some(42), ..at(2) }, at(3)],
                    last_sync: Some(T - 60),
                    now: T,
                    live: true,
                    ..Default::default()
                },
            ),
            (
                "two of three copies only over unchecked links",
                Observation {
                    records: 40,
                    holders: vec![Holder { guarded: false, ..at(1) }, Holder { guarded: false, ..at(2) }, at(3)],
                    last_sync: Some(T - 60),
                    now: T,
                    live: true,
                    ..Default::default()
                },
            ),
        ];
        let cases: Vec<Value> = cases
            .into_iter()
            .map(|(name, o)| {
                let w = assess(&o);
                json!({
                    "name": name,
                    "observation": {
                        "own_durable": o.own_durable, "records": o.records, "missing": o.missing,
                        "media_wanted": o.media_wanted, "media_present": o.media_present,
                        "holders": o.holders.iter().map(|h| json!({ "coverage": h.coverage, "durable": h.durable, "operator": h.operator, "network": h.network, "guarded": h.guarded })).collect::<Vec<_>>(),
                        "last_sync": o.last_sync, "now": o.now, "live": o.live, "hinted": o.hinted,
                    },
                    "warmth": {
                        "level": w.level.as_str(), "score": w.score, "copies": w.copies, "durable_copies": w.durable_copies,
                        "network_copies": w.network_copies, "complete": w.complete, "media": w.media, "fresh": w.fresh,
                        "holders_durable": w.holders_durable, "holders_transient": w.holders_transient, "hint": hint(&w),
                        "kept": w.kept, "places": w.places, "spread": w.spread, "guarded": w.guarded,
                    },
                })
            })
            .collect();
        let link = |network: u64| Link { operator: None, network: Some(network), guarded: true };
        let link_cases: Vec<(&str, Vec<Link>)> = vec![
            ("no connection", vec![]),
            ("one link: everything through one place", vec![link(1)]),
            ("three links into one network", vec![link(1), link(1), link(1)]),
            ("three links, three places, all guarded", vec![link(1), link(2), link(3)]),
            ("four links, three into one place", vec![link(1), link(1), link(1), link(2)]),
            ("three places, two links unguarded", vec![Link { guarded: false, ..link(1) }, Link { guarded: false, ..link(2) }, link(3)]),
            ("one operator across two networks, and a third", vec![Link { operator: Some(42), ..link(1) }, Link { operator: Some(42), ..link(2) }, link(3)]),
        ];
        let link_cases: Vec<Value> = link_cases
            .into_iter()
            .map(|(name, links)| {
                let w = assess_links(&links);
                json!({
                    "name": name,
                    "links": links.iter().map(|l| json!({ "operator": l.operator, "network": l.network, "guarded": l.guarded })).collect::<Vec<_>>(),
                    "warmth": { "level": w.level.as_str(), "score": w.score, "links": w.links, "places": w.places, "effective": w.effective, "largest": w.largest, "guarded": w.guarded },
                })
            })
            .collect();
        json!({
            "note": "hoi_sync::warmth::assess over first-hand observations (PROTOCOL.md §20). Copies are counted once per independent place: holders sharing an operator key or a network key (IPv4 /24, IPv6 /48, this machine), directly or through another, are one place, and `kept` is the sum over places of the most any holder there showed. Levels read `kept` (the observer's own copy never warms its own reading): cold below one, cool below two, warm below three or while incomplete, hot from three; one level colder when less than half is here, nothing caught up for a week, or under half of the other durable copies are reached over a guarded link. Score: 0.6 × min(1, kept / 4) + 0.1 × spread + 0.1 × guarded + 0.12 × complete + 0.08 × fresh, spread = kept / other durable copies. `hint` is ⌊kept⌋ capped at 3, what a node attaches to its catalog entry (`w`). `links` are hoi_sync::warmth::assess_links over a device's own connections: places as above, effective = 1 / Σ share², cold with none, cool through one place or under half guarded, hot from 3 effective places with none over half the links and nine in ten guarded; score 0.6 × min(1, effective / 4) + 0.25 × guarded + 0.15 × (1 − largest), kept inside its level's band (cool ≤ 49, warm 50–79, hot ≥ 80).",
            "cases": cases,
            "links": link_cases,
        })
    };

    // ---------------------------------------------------------------- carry requests (PROTOCOL.md §14)
    let carry_msgs = {
        let carry = hoi_sync::Msg::Carry { genesis: gw.encode() };
        let yes = hoi_sync::Msg::Carried { community: c, ok: true, reason: None };
        let no = hoi_sync::Msg::Carried { community: c, ok: false, reason: Some("full".into()) };
        let catalog = hoi_sync::Msg::Catalog {
            entries: vec![hoi_sync::CatalogEntry { genesis: gw.encode(), peers: vec![], relays: vec![], warmth: Some(2), relayed: false, age: None }],
        };
        json!({
            "note": "A `carry` request carries the signed genesis as an invite does, and nothing else: no routes, so a stranger never makes a public node dial an address. `carried` answers it (reasons: invalid, not open, not accepting, full, busy, refused). Only nodes listing `carry` in `hello.fx` receive requests. `catalog.w` is the durable copies the sender saw besides its own, quantised: a hint taken as a maximum.",
            "community": c.hex(),
            "carry": hex_encode(&carry.encode()),
            "carried_ok": hex_encode(&yes.encode()),
            "carried_full": hex_encode(&no.encode()),
            "catalog_with_warmth": hex_encode(&catalog.encode()),
        })
    };

    // ---------------------------------------------------------------- node announcements (PROTOCOL.md §10)
    // The body a registry receives, plus the seed and the statement for checking: the statement
    // format is hoi_core::node_announcement_statement's and registry/protocol.mjs's, and both
    // implementations' tests compare their own against this file.
    let announcement = {
        let node = Keypair::from_secret(&[0x4b; 32]);
        let registry = "https://registry.example";
        let issued_at = 1_800_000_000u64;
        let routes = [format!("rtc:{}@203.0.113.10:7844", hoi_types::uri::b64(&[0x11; 32])), format!("{}@203.0.113.10:7843", node.public().hex())];
        let communities = [Id([0x11; 32]).hex(), Id([0x22; 32]).hex()];
        let statement = format!("hoi public node v2\n{registry}\n{}\n{issued_at}\n{}\n{}", node.public().hex(), routes.join(","), communities.join(","));
        json!({
            "seed": hex_encode(&node.secret()),
            "registry": registry,
            "endpoint": node.public().hex(),
            "issuedAt": issued_at,
            "routes": routes,
            "communities": communities,
            "statement": statement,
            "signature": hex_encode(&node.sign(statement.as_bytes()).0),
        })
    };

    // ---------------------------------------------------------------- operator notices (PROTOCOL.md §14.5)
    // A notice as an operator writes it (JSON), the canonical bytes every implementation hashes,
    // the statement its node's endpoint key signs for the node link, and that link.
    let node_notice = {
        let node = Keypair::from_secret(&[0x4e; 32]);
        let notice = hoi_types::NodeNotice {
            node: "sleepy-walrus-48213".into(),
            operator: "Lena".into(),
            contact: "lena@example.org".into(),
            country: "XX".into(),
            provider: Some("Example Hosting".into()),
            ip_logging: true,
            retention_days: 7,
            updated: "2026-11-03".into(),
            ..hoi_types::NodeNotice::template("sleepy-walrus-48213")
        };
        assert!(notice.validate().is_empty());
        let canonical = notice.canonical();
        let hash = notice.hash();
        let statement = hoi_types::notice::statement(&node.public(), &hash);
        let signature = node.sign(statement.as_bytes());
        let link = hoi_types::NodeLink {
            rtc: vec![format!("rtc:{}@203.0.113.9:7844", "a".repeat(43))],
            peers: vec![format!("{}@203.0.113.9:7843", node.public().hex())],
            notice: Some(canonical.clone()),
            signature: Some(signature),
        };
        let hello = hoi_sync::Msg::Hello {
            v: hoi_sync::SYNC_VERSION,
            node: node.public(),
            name: None,
            token: None,
            communities: vec![],
            salt: Some([0x5a; 16]),
            hashed: vec![],
            serves: false,
            hints: vec![],
            routes: link.peers.clone(),
            features: vec![hoi_sync::FEATURE_CARRY.into(), hoi_sync::FEATURE_NODES.into()],
            pairing: None,
            link_key: None,
            notice_hash: Some(hash),
            notice: Some(canonical.clone()),
            holder: None,
            forwards: false,
            rebuilds: false,
        };
        json!({
            "note": "An operator's notice: `json` is the file an operator writes; `canonical` is hoi_types::NodeNotice::canonical (DAG-CBOR, one-letter keys, optional fields left out when empty), hashed with BLAKE3 into `hash`; operator `o`, contact `c` and country `k` are always written and may be empty, since an operator may leave who and where out. A public node sends both in its hello to strangers (`nh`, `nt`) and puts the canonical bytes in its node link (`n` in the masked code), with the endpoint key's Ed25519 signature over `statement` (`s`); the earlier clear-text link spelled them `nt=` (deflated) and `ns=`, and `legacy_link` still opens. Clients show the notice before they say anything of their own.",
            "seed": hex_encode(&node.secret()),
            "endpoint": node.public().hex(),
            "json": notice,
            "canonical": hex_encode(&canonical),
            "hash": hash.hex(),
            "statement": statement,
            "signature": hex_encode(&node.sign(statement.as_bytes()).0),
            "link": link.to_uri(),
            "web_link": link.to_web("https://hoi.sh/app/"),
            "legacy_link": format!(
                "hoi://node?w={}@203.0.113.9:7844&p={}@203.0.113.9:7843&nt={}&ns={}",
                "a".repeat(43),
                node.public().hex(),
                hoi_types::uri::b64(&hoi_types::uri::deflate(&canonical)),
                hoi_types::uri::b64(&signature.0)
            ),
            "hello": hex_encode(&hello.encode()),
            "text": notice.text(),
            "pieces": {
                "note": "The same notice from a node that keeps encrypted pieces for others (`hoi-node --hold-pieces`): `s: true` is added and the fixed sentence hoi_types::notice::PIECES shows after what the node processes. Older readers ignore the key; the hash covers it.",
                "canonical": hex_encode(&hoi_types::NodeNotice { pieces: true, ..notice.clone() }.canonical()),
                "text": hoi_types::NodeNotice { pieces: true, ..notice.clone() }.text(),
            },
        })
    };

    let files = [
        ("node-announcement.json", announcement),
        ("node-notice.json", node_notice),
        (
            "hello-hints.json",
            json!({ "msg": hex_encode(&hello_hints.encode()), "note": "Optional hello routing extension, accepted only on paired home-node links (a home node's own once it delivered records)." }),
        ),
        ("hello-salted.json", salted_hello),
        ("warmth.json", warmth),
        ("scatter.json", scatter_vectors()),
        ("carry.json", carry_msgs),
        ("records.json", json!({ "protocol": 1, "pow_bits": bits, "created_at_base": T, "records": records })),
        ("ladder.json", ladder_scenario()),
        ("media.json", media),
        ("pdq.json", pdq_vectors()),
        ("delegation-chain.json", chain),
        ("tombstone.json", tomb),
        ("threshold.json", threshold),
        ("sync-transcript.json", sync),
        ("uris.json", uris),
    ];
    let mut stale = 0;
    for (name, v) in files {
        let path = dir.join(name);
        let text = serde_json::to_string_pretty(&v).expect("json") + "\n";
        if check {
            // Git may check JSON fixtures out with CRLF on Windows. Ignore that checkout
            // representation only; generated values and encoded protocol bytes must match.
            if std::fs::read_to_string(&path).ok().map(|s| s.replace("\r\n", "\n")).as_deref() != Some(text.as_str()) {
                eprintln!("stale: core/vectors/{name}");
                stale += 1;
            }
        } else {
            std::fs::write(&path, text).expect("write vector");
            println!("wrote core/vectors/{name}");
        }
    }
    if stale > 0 {
        std::process::exit(1);
    }
}
