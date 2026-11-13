//! RFC-0004: the privacy boundary of a storage-only holder, checked against what holders are
//! actually sent and what they keep (SCATTER.md §5.6, §5.8, §5.13).
//!
//! Members of a private group post and read a post with an attachment through public nodes that
//! keep pieces: the group's MLS messages in mailbox slots, the post and its attachment as
//! scattered pieces. Those nodes have their own transport and envelope keys and nothing else: no
//! identity, MLS state, community secret or object key. Every request they are sent is opened
//! with their own keys and checked field by field, every answer's size is checked, and every row
//! they keep is read back after their SQLite files are reopened.
//!
//! Distinctive synthetic content, identifiers and keys serve as leakage sentinels. Not finding
//! them supplements the field checks; it is not a cryptographic proof.
use super::forward::{Mesh, group, mesh};
use super::*;
use crate::scatter::ScatterOutcome;
use hoi_crypto::envelope::{self, GOT_OVERHEAD, Kind, REQUEST_LEN, Reply, Request, SHORT_REPLY_LEN, XKey};
use hoi_crypto::scatter as sc;
use hoi_store::pieces::{PieceMeta, PieceStore, SqlitePieces};
use hoi_sync::{Msg, PieceOp};
use hoi_types::scatter::Manifest;
use hoi_types::uri::{b64, unb64};
use std::collections::BTreeSet;
use std::path::PathBuf;

const DAY: u64 = 86_400;
const BUDGET: u64 = 1 << 30;
const GROUP: &str = "qx7 sentinel garden";
const AUTHOR: &str = "qx7 sentinel author";
const READER: &str = "qx7 sentinel reader";
const TITLE: &str = "qx7 sentinel title, for members only";
const TEXT: &str = "qx7 sentinel text that no holder may read";
const ALT: &str = "qx7 sentinel description of the photo";
/// The bell's leaf in mailbox slots (`mailbox::BELL`): no member has this index.
const BELL: u32 = u32::MAX;

/// A folder for the holders' SQLite files, removed when the test ends, however it ends.
struct Folder(PathBuf);

impl Folder {
    fn new(what: &str) -> Folder {
        let path = std::env::temp_dir().join(format!("hoi-{what}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Folder(path)
    }

    fn file(&self, node: &str) -> PathBuf {
        self.0.join(format!("{node}.db"))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The mesh's public nodes: forwarders and holders, every one keeping pieces.
fn storage(name: &str) -> bool {
    name.starts_with('f') || name.starts_with('h')
}

fn storage_nodes(m: &Mesh) -> Vec<String> {
    m.nodes.keys().filter(|n| storage(n)).cloned().collect()
}

/// Three forwarders and fourteen holders, every public node keeping its pieces in a SQLite file
/// of its own.
fn sqlite_mesh(dir: &Folder) -> Mesh {
    let mut m = mesh(3, 14);
    for name in storage_nodes(&m) {
        let store = SqlitePieces::open(&dir.file(&name), BUDGET).unwrap();
        m.n(&name).set_piece_store(Some(Box::new(store)));
    }
    m.pump();
    // the forwarders' next hourly sample vouches for the holders' new, durable keys
    m.t.fetch_add(3600, Ordering::SeqCst);
    m.run(5);
    m
}

/// A person's desktop with a distinctive name, linked to the three forwarders.
fn member(m: &mut Mesh, name: &str, identity: &str, addr: &str) {
    let mut n = node("desktop", &m.t, &Mem::default());
    n.create_identity(identity).unwrap();
    m.add(name, n, addr);
    for f in 0..3 {
        m.dial(name, &format!("f{f}"));
    }
    m.pump();
}

/// `a` makes a private group; `b` joins it with an invitation bound to its own request.
fn private_group(m: &mut Mesh) -> (String, Id) {
    member(m, "a", AUTHOR, "198.51.100.10:7843");
    member(m, "b", READER, "192.0.2.20:7843");
    let c = m.n("a").create_private_community(draft(GROUP)).unwrap().id;
    m.run(10);
    let request = m.n("b").private_join_request().unwrap();
    let invitation = m.n("a").private_invite(&c, &request).unwrap();
    m.n("b").private_join(&invitation).unwrap();
    m.run(60);
    assert_eq!(m.n("b").private_group(&c).unwrap().transport, "mailboxes");
    let ca = Id::from_hex(&c).unwrap();
    (c, ca)
}

/// `who` posts a photo with a sentinel title, text and description; returns the post and the
/// photo's bytes as they are kept.
fn photo_post(m: &mut Mesh, who: &str, c: &str, title: &str) -> (PostSummary, Vec<u8>) {
    let draft = PostDraft {
        community: c.into(),
        title: title.into(),
        text: TEXT.into(),
        link: None,
        media: Some(vec![MediaDraft { data: b64(&picture(48, image::ImageFormat::Png)), alt: Some(ALT.into()) }]),
    };
    let p = m.n(who).create_post(&draft).unwrap();
    let photo = unb64(&m.n(who).media(&p.media[0].hash).unwrap().data).unwrap();
    (p, photo)
}

/// Runs the mesh until `who` shows `p` with its photo, at most `secs`.
fn read_photo_post(m: &mut Mesh, who: &str, p: &PostSummary, photo: &[u8], secs: u64) {
    for _ in 0..secs.div_ceil(5) {
        let n = m.n(who);
        if n.post(&p.id).is_ok() && n.media(&p.media[0].hash).is_ok() {
            break;
        }
        m.run(5);
    }
    assert_eq!(m.n(who).post(&p.id).unwrap().summary.title, p.title, "{who} reads the post");
    assert_eq!(unb64(&m.n(who).media(&p.media[0].hash).unwrap().data).unwrap(), photo, "{who} has the photo");
}

/// Every generation `who` keeps for community `c`: the manifests hold the objects' keys.
fn manifests(m: &Mesh, who: &str, c: Id) -> Vec<Manifest> {
    let kept = m.nodes[who].scatter.saved.kept.values().filter(|k| k.community == Some(c));
    kept.flat_map(|k| k.gens.iter().chain(&k.cleanup_gens).filter_map(|g| g.read())).collect()
}

/// What no storage-only node may be sent or keep, each looked for raw and, for binary values, as
/// hex too (JSON and links carry keys as hex).
#[derive(Default)]
struct Sentinels(Vec<(String, Vec<u8>)>);

impl Sentinels {
    fn text(&mut self, what: &str, s: &str) {
        self.0.push((what.into(), s.as_bytes().to_vec()));
    }

    fn bytes(&mut self, what: &str, b: &[u8]) {
        self.0.push((what.into(), b.to_vec()));
        self.0.push((format!("{what} as hex"), hoi_types::hex_encode(b).into_bytes()));
    }

    /// Windows of a long plaintext, raw and in base64 (as a whole media payload carries it).
    fn windows(&mut self, what: &str, b: &[u8]) {
        let text = b64(b).into_bytes();
        for (form, all) in [("", b), (" in base64", text.as_slice())] {
            for (i, w) in all.chunks(512).filter(|w| w.len() >= 24).enumerate() {
                self.0.push((format!("{what}{form}, part {i}"), w[..24].to_vec()));
            }
        }
    }

    /// The first sentinel `bytes` contain.
    fn found(&self, bytes: &[u8]) -> Option<&str> {
        self.0.iter().find(|(_, s)| bytes.windows(s.len()).any(|w| w == s.as_slice())).map(|(what, _)| what.as_str())
    }
}

/// Content, its community and author, and every key that opens it or finds it.
fn sentinels(m: &Mesh, c: Id, posts: &[(&PostSummary, &[u8])], slot_keys: &BTreeSet<[u8; 32]>) -> Sentinels {
    let mut s = Sentinels::default();
    for t in [GROUP, AUTHOR, READER, TEXT, ALT] {
        s.text(t, t);
    }
    s.bytes("community key", &c.0);
    for (p, photo) in posts {
        s.text("title", &p.title);
        s.bytes("post id", &Id::from_hex(&p.id).unwrap().0);
        s.bytes("photo hash", &Id::from_hex(&p.media[0].hash).unwrap().0);
        s.windows("photo", photo);
    }
    for who in ["a", "b"] {
        let n = &m.nodes[who];
        s.bytes(&format!("{who}'s person key"), &n.me_root().unwrap().0);
        let device = n.device_kp.as_ref().unwrap();
        s.bytes(&format!("{who}'s device key"), &device.public().0);
        s.bytes(&format!("{who}'s device secret"), &device.secret());
        for man in manifests(m, who, c) {
            s.bytes("an object key", &man.key);
            s.bytes("a locator secret", &man.loc_secret);
        }
        for secret in n.held_drop_keys(c) {
            s.bytes("the group's drop key secret", &secret);
        }
    }
    for key in slot_keys {
        s.bytes("a mailbox slot key", key);
    }
    s
}

/// The slot keys of the epochs `a` and `b` are in now.
fn note_slot_keys(m: &Mesh, c: Id, keys: &mut BTreeSet<[u8; 32]>, leaves: &mut BTreeSet<u32>) {
    for who in ["a", "b"] {
        if let Some(ep) = m.nodes[who].private_epoch(c).filter(|e| e.active) {
            keys.insert(ep.slot_key);
            leaves.extend(ep.leaves);
        }
    }
}

/// A message's kind, for failure messages: never its content.
fn label(msg: &Msg) -> String {
    format!("{msg:?}").split([' ', '{', '(']).next().unwrap_or_default().to_string()
}

fn kind_of(op: PieceOp) -> Option<Kind> {
    match op {
        PieceOp::Put => Some(Kind::Put),
        PieceOp::Get => Some(Kind::Get),
        PieceOp::Has => Some(Kind::Has),
        PieceOp::Drop => Some(Kind::Drop),
        _ => None,
    }
}

/// A request a storage node was sent, as that node opens it.
struct Seen {
    /// Where in the mesh's log it was delivered.
    at: usize,
    from: String,
    to: String,
    rid: u32,
    request: Request,
}

/// Every request delivered to a storage node, opened with that node's own keys. Each must open:
/// a node is only ever sent requests sealed to it.
fn opened(m: &Mesh) -> Vec<Seen> {
    let keys: BTreeMap<String, (Id, Vec<XKey>)> =
        storage_nodes(m).into_iter().map(|n| (n.clone(), (Id::from_hex(&m.eps[&n]).unwrap(), m.nodes[&n].fresh_keys()))).collect();
    let mut out = Vec::new();
    for (at, (from, to, msg)) in m.log.iter().enumerate() {
        let Msg::Piece { op, rid, to: None, env } = msg else { continue };
        let (Some(kind), Some((endpoint, keys))) = (kind_of(*op), keys.get(to)) else { continue };
        let request = keys.iter().find_map(|k| envelope::open_request(kind, &endpoint.0, k, env).ok());
        let request = request.unwrap_or_else(|| panic!("a {op:?} from {from} to {to} does not open with {to}'s own keys"));
        if kind != Kind::Put {
            assert_eq!(env.len(), REQUEST_LEN, "requests other than puts have one size");
        }
        out.push(Seen { at, from: from.clone(), to: to.clone(), rid: *rid, request });
    }
    out
}

#[test]
fn a_storage_only_holder_relays_a_private_post_and_photo_it_cannot_read() {
    let dir = Folder::new("holder-privacy");
    let mut m = sqlite_mesh(&dir);
    let (c, ca) = private_group(&mut m);
    let mut slot_keys = BTreeSet::new();
    let mut leaves = BTreeSet::from([BELL]);
    note_slot_keys(&m, ca, &mut slot_keys, &mut leaves);

    // the author posts a photo; the reader gets both through holders that are not members
    let start = m.log.len();
    let (p, photo) = photo_post(&mut m, "a", &c, TITLE);
    m.run(120);
    read_photo_post(&mut m, "b", &p, &photo, 240);
    note_slot_keys(&m, ca, &mut slot_keys, &mut leaves);
    let pid = Id::from_hex(&p.id).unwrap();
    let hash = Id::from_hex(&p.media[0].hash).unwrap();
    let manifests: Vec<Manifest> = manifests(&m, "a", ca).into_iter().chain(manifests(&m, "b", ca)).collect();
    let post_object = m.nodes["b"].scatter.saved.kept.values().find(|k| k.records == vec![pid]).and_then(|k| k.newest()).expect("b kept the post's object");
    assert!(m.nodes["b"].scatter.saved.kept.values().any(|k| k.media == Some(hash)), "the photo came in pieces too");
    let sentinels = sentinels(&m, ca, &[(&p, &photo)], &slot_keys);
    // the search finds them where they belong: in the reader's own copy of the post
    assert!(sentinels.found(&m.nodes["b"].store.get(&pid).unwrap().wire).is_some());

    // no storage node is a member or holds anything that opens the content
    let drop_key = m.nodes["a"].private_epoch(ca).and_then(|e| e.drop_key).expect("the group has a drop key");
    for name in storage_nodes(&m) {
        let n = &m.nodes[&name];
        assert!(n.me().is_none() && n.device_kp.is_none(), "{name} has no identity");
        assert!(n.private_group_ids().is_empty() && !n.state.private_communities.contains(&ca), "{name} has no MLS state");
        assert!(n.community_drop_key(ca).is_none() && n.held_drop_keys(ca).is_empty(), "{name} has no community secret");
        assert!(n.scatter.saved.kept.is_empty(), "{name} has no manifest, so no object key");
        assert!(n.store.genesis_of(&ca).is_none() && n.store.in_community(&ca).next().is_none() && !n.store.has_blob(&hash), "{name} has no record");
        assert!(!n.discover_communities().iter().any(|d| d.id == c), "{name} lists no private community");
        for s in n.store.all() {
            assert_eq!(sentinels.found(&s.wire), None, "{name} keeps a record with it");
        }
    }

    // What storage nodes were sent: nothing carries content, its community, its author or a key
    // that opens it. Holders that are not linked to a member never hear a member's address either.
    let mut hidden = Sentinels::default();
    for who in ["a", "b"] {
        hidden.bytes(&format!("{who}'s endpoint"), &Id::from_hex(&m.eps[who]).unwrap().0);
        hidden.text(&format!("{who}'s address"), &m.addrs[who]);
    }
    for (from, to, msg) in m.log.iter().filter(|(_, to, _)| storage(to)) {
        let bytes = msg.encode();
        assert_eq!(sentinels.found(&bytes), None, "{from} → {to}: {}", label(msg));
        if to.starts_with('h') {
            assert!(storage(from), "{to} hears only from public nodes");
            assert_eq!(hidden.found(&bytes), None, "{from} → {to}");
        }
    }

    // every request, opened by its holder: only the fields its operation defines, none of them a
    // known identifier, one-time answer keys, and drop metadata that is opaque per locator
    let today = (m.t.load(Ordering::SeqCst) / DAY) as u32;
    let ids: BTreeSet<[u8; 32]> = [ca.0, pid.0, hash.0, m.nodes["a"].me_root().unwrap().0, m.nodes["b"].me_root().unwrap().0]
        .into_iter()
        .chain(manifests.iter().map(|man| man.object))
        .collect();
    let seen = opened(&m);
    let mut replies: BTreeMap<[u8; 32], (String, [u8; 32])> = BTreeMap::new();
    let mut drop_hashes: BTreeMap<[u8; 32], [u8; 32]> = BTreeMap::new();
    let mut wants: BTreeMap<(String, String, u32), usize> = BTreeMap::new();
    for s in &seen {
        let (loc, reply) = (*s.request.loc(), *s.request.reply_key());
        assert!(!ids.contains(&loc), "{} was asked about a known identifier", s.to);
        let same = replies.entry(reply).or_insert((s.to.clone(), loc));
        assert_eq!(*same, (s.to.clone(), loc), "an answer key is used once");
        match &s.request {
            Request::Put { expiry_day, drop_hash, drop_key_id, stamp, piece, .. } => {
                assert_eq!(sentinels.found(piece), None, "a piece {} keeps", s.to);
                assert!(*expiry_day <= today + crate::scatter::MAX_TTL_DAYS);
                assert!(sc::stamp_ok(&Id::from_hex(&m.eps[&s.to]).unwrap().0, &loc, piece.len(), *expiry_day, *stamp));
                assert!(*drop_key_id == [0; 16] || *drop_key_id == sc::drop_key_id(&drop_key), "the group's drop key id, or none");
                if *drop_hash != [0; 32] {
                    assert_eq!(*drop_hashes.entry(*drop_hash).or_insert(loc), loc, "a drop hash names one locator");
                }
            }
            Request::Get { want_len, .. } => {
                wants.insert((s.from.clone(), s.to.clone(), s.rid), *want_len as usize);
            }
            Request::Has { .. } => {}
            Request::Drop { .. } => panic!("nothing was deleted"),
        }
    }
    assert!(seen.iter().any(|s| s.to.starts_with('h') && matches!(s.request, Request::Put { .. })), "holders were sent pieces");

    // Since the post, every locator is one only members derive: a slot of the epoch's mailboxes
    // (or its bell), or a piece of an object a member kept. Pieces carry the group's drop key id
    // (a correlation handle every piece of the group shares) and exactly the bytes the manifest
    // names; slots carry none.
    let pieces: BTreeMap<[u8; 32], (&Manifest, u8)> = manifests
        .iter()
        .flat_map(|man| (0..man.hashes.len() as u8).map(move |i| (sc::locator(&man.loc_secret, &man.object, man.generation, i), (man, i))))
        .collect();
    let slots: BTreeSet<[u8; 32]> =
        slot_keys.iter().flat_map(|key| leaves.iter().flat_map(move |&leaf| (0..512).map(move |seq| crate::mailbox::slot(key, leaf, seq)))).collect();
    let (mut piece_puts, mut slot_puts) = (0, 0);
    for s in seen.iter().filter(|s| s.at >= start) {
        let loc = s.request.loc();
        match (&s.request, pieces.get(loc)) {
            (Request::Put { drop_key_id, piece, .. }, Some((man, i))) => {
                assert_eq!(*drop_key_id, sc::drop_key_id(&drop_key));
                assert_eq!(sc::piece_hash(&man.object, man.generation, *i, piece), man.hashes[*i as usize], "the manifest's own piece");
                piece_puts += 1;
            }
            (Request::Put { drop_key_id, .. }, None) => {
                assert!(slots.contains(loc), "{} was sent a locator no member derives", s.to);
                assert_eq!(*drop_key_id, [0; 16], "a slot names no drop key");
                slot_puts += 1;
            }
            (_, found) => assert!(found.is_some() || slots.contains(loc), "{} was asked about a locator no member derives", s.to),
        }
    }
    assert!(piece_puts >= 24 && slot_puts > 0, "the post and its photo went in pieces, their pointers in slots: {piece_puts}, {slot_puts}");

    // Answers say only what their kind defines, at sizes that do not depend on what is kept:
    // short answers one size, a got as long as the asker wanted.
    let mut gots = 0;
    for (from, to, msg) in m.log.iter().filter(|(from, _, _)| storage(from)) {
        let Msg::Piece { op, rid, to: None, env } = msg else { continue };
        match op {
            PieceOp::Stored | PieceOp::Had | PieceOp::Dropped => assert_eq!(env.len(), SHORT_REPLY_LEN),
            // a holder's own answer; a forwarder passing one back sends the same envelope on
            PieceOp::Got => {
                if let Some(want) = wants.get(&(to.clone(), from.clone(), *rid)) {
                    assert_eq!(env.len(), GOT_OVERHEAD + want, "a got is padded to what was asked");
                    gots += 1;
                }
            }
            _ => {}
        }
    }
    assert!(gots > 0);

    // What each holder keeps on disk, after closing and reopening its file: exactly the
    // locators it was sent, each with the expiry day, drop hash and drop key id of its put and
    // the bytes it was given, and nothing else.
    for name in storage_nodes(&m) {
        let before = m.n(&name).piece_stats().unwrap();
        m.n(&name).set_piece_store(None);
        let path = dir.file(&name);
        for file in [path.clone(), path.with_extension("db-wal")] {
            if let Ok(bytes) = std::fs::read(&file) {
                assert_eq!(sentinels.found(&bytes), None, "{} on disk", file.display());
            }
        }
        let mut store = SqlitePieces::open(&path, BUDGET).unwrap();
        assert_eq!(store.stats(), before, "{name} keeps the same after reopening");
        // what each locator was sent with (a slot two writers raced for was sent twice)
        let mut sent: BTreeMap<[u8; 32], Vec<(PieceMeta, &[u8])>> = BTreeMap::new();
        for s in seen.iter().filter(|s| s.to == name) {
            if let Request::Put { loc, expiry_day, drop_hash, drop_key_id, piece, .. } = &s.request {
                let meta = PieceMeta { expiry_day: *expiry_day, drop_hash: *drop_hash, drop_key_id: *drop_key_id };
                sent.entry(*loc).or_default().push((meta, piece.as_slice()));
            }
        }
        let mut rows = 0;
        for (loc, puts) in &sent {
            let Some(meta) = store.meta(loc).unwrap() else { continue };
            let bytes = store.get(loc, today).unwrap().unwrap();
            assert!(puts.iter().any(|(m, b)| *m == meta && *b == bytes.as_slice()), "{name} keeps a row as one put gave it");
            rows += 1;
        }
        assert_eq!(rows, before.count, "{name} keeps no row it was not sent");
        m.n(&name).set_piece_store(Some(Box::new(store)));
    }
    m.pump();

    // the reopened holders still serve the post to the reader
    let op = m.n("b").scatter_load(&post_object).unwrap();
    assert!(matches!(m.wait("b", op, 120), ScatterOutcome::Loaded(_)), "rebuilt from reopened holders");

    // Known and unknown locators, asked as a reader would through a forwarder: presence and
    // length, or the piece; nothing at all for a locator that is not kept, however it was
    // chosen. There is no request that lists what a holder keeps.
    let i = post_object.holders.iter().position(|e| m.eps.iter().any(|(n, x)| n.starts_with('h') && *x == e.hex())).unwrap();
    let holder = post_object.holders[i];
    let name = m.eps.iter().find(|(_, x)| **x == holder.hex()).map(|(n, _)| n.clone()).unwrap();
    let xk = m.nodes[&name].holder_hello().unwrap().xk;
    let known = sc::locator(&post_object.loc_secret, &post_object.object, post_object.generation, i as u8);
    let len = post_object.class.piece_len(post_object.k as usize);
    let mut rid = 9000;
    // `want`: a get for that many bytes, or none for a has
    let mut ask = |m: &mut Mesh, loc: [u8; 32], want: Option<u32>| {
        rid += 1;
        let reply = XKey::generate().unwrap();
        let (op, request) = match want {
            None => (PieceOp::Has, Request::Has { loc, reply: reply.public() }),
            Some(want_len) => (PieceOp::Get, Request::Get { loc, reply: reply.public(), want_len }),
        };
        let env = envelope::seal_request(&holder.0, &xk, &request).unwrap();
        let start = m.log.len();
        m.n("f0").net_receive("b", &Msg::Piece { op, rid, to: Some(holder), env }.encode());
        m.pump();
        let back = m.log[start..].iter().find_map(|(f, to, msg)| match msg {
            Msg::Piece { rid: r, env, .. } if f == "f0" && to == "b" && *r == rid => Some(env.clone()),
            _ => None,
        });
        let back = back.expect("an answer");
        let answer = envelope::open_reply(request.kind().answer().unwrap(), &reply, &loc, want.unwrap_or(0) as usize, &back).unwrap();
        (answer, back.len())
    };
    assert_eq!(ask(&mut m, known, None), (Reply::Had(Some(len as u32)), SHORT_REPLY_LEN));
    let (got, size) = ask(&mut m, known, Some(len as u32));
    let Reply::Got(Some(piece)) = got else { panic!("the piece: {got:?}") };
    assert_eq!(sc::piece_hash(&post_object.object, post_object.generation, i as u8, &piece), post_object.hashes[i]);
    assert_eq!(size, GOT_OVERHEAD + len);
    let unknown = [hoi_crypto::random_bytes::<32>().unwrap(), ca.0, pid.0, hash.0, post_object.object, [0; 32], [0xff; 32]];
    for loc in unknown {
        assert_eq!(ask(&mut m, loc, None), (Reply::Had(None), SHORT_REPLY_LEN));
        assert_eq!(ask(&mut m, loc, Some(len as u32)), (Reply::Got(None), GOT_OVERHEAD + len), "nothing, at the size of a piece");
    }
}

/// Every slot locator of an epoch, its bell's included, up to sequence 511.
fn epoch_slots(ep: &crate::private::MailEpoch) -> BTreeSet<[u8; 32]> {
    let leaves: Vec<u32> = ep.leaves.iter().copied().chain([BELL]).collect();
    leaves.iter().flat_map(|&leaf| (0..512).map(move |seq| crate::mailbox::slot(&ep.slot_key, leaf, seq))).collect()
}

#[test]
fn removal_rotates_what_holders_are_sent_and_cannot_take_back_what_was_received() {
    let mut m = mesh(3, 14);
    let c = group(&mut m);
    let ca = Id::from_hex(&c).unwrap();
    let (before, photo) = photo_post(&mut m, "a", &c, TITLE);
    m.run(120);
    read_photo_post(&mut m, "b", &before, &photo, 240);
    let pid = Id::from_hex(&before.id).unwrap();
    let given = m.nodes["b"].scatter.saved.kept.values().find(|k| k.records == vec![pid]).and_then(|k| k.newest()).expect("b was given the manifest");
    let old = m.nodes["b"].private_epoch(ca).unwrap();

    // The owner removes b's device. The commit starts a new epoch: new slot locators, which
    // b cannot derive.
    let device = m.n("b").me().unwrap().device;
    m.n("a").private_remove(&c, &device).unwrap();
    m.run(60);
    assert!(!m.n("b").private_group(&c).unwrap().active, "b read its removal");
    let new = m.nodes["a"].private_epoch(ca).unwrap();
    assert!(new.epoch > old.epoch);
    assert_ne!(new.slot_key, old.slot_key, "slot locators change with every epoch");
    assert!(epoch_slots(&new).is_disjoint(&epoch_slots(&old)));

    // Removal cannot take back what b received: the post and its photo stay on b's device, and
    // holders, which check no membership, still serve the pieces b holds a manifest for, until
    // they expire or are dropped.
    read_photo_post(&mut m, "b", &before, &photo, 0);
    let op = m.n("b").scatter_load(&given).unwrap();
    assert!(matches!(m.wait("b", op, 120), ScatterOutcome::Loaded(_)), "the pieces b was told about");

    // A newcomer is added in a later epoch. The owner shares the live history into it, so the
    // newcomer reads the post made before it joined; what is posted now reaches no slot of the
    // epoch b was in and no manifest b holds.
    super::forward::join(&mut m, "c", "203.0.113.30:7843", &[0, 1, 2]);
    let request = m.n("c").private_join_request().unwrap();
    let invitation = m.n("a").private_invite(&c, &request).unwrap();
    m.n("c").private_join(&invitation).unwrap();
    m.run(60);
    read_photo_post(&mut m, "c", &before, &photo, 240);
    let start = m.log.len();
    let (after, later) = photo_post(&mut m, "a", &c, "qx7 sentinel title after the removal");
    m.run(120);
    read_photo_post(&mut m, "c", &after, &later, 240);
    assert!(m.n("b").post(&after.id).is_err(), "b cannot read what came after its removal");
    let aid = Id::from_hex(&after.id).unwrap();
    assert!(!m.nodes["b"].scatter.saved.kept.values().any(|k| k.records.contains(&aid)), "b was given no manifest for it");
    let old_slots = epoch_slots(&old);
    let sent = opened(&m);
    assert!(sent.iter().any(|s| s.at >= start && matches!(s.request, Request::Put { .. })));
    assert!(!sent.iter().any(|s| s.at >= start && old_slots.contains(s.request.loc())), "nothing new is put in or asked of b's old slots");
}
