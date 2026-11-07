//! hoi-sync: what two nodes say to each other, over any byte pipe: the home-node WebSocket,
//! `hoi/sync/1` on iroh between native nodes, and WebRTC data channels from browsers. Live
//! records ride the same link, so there is no separate gossip protocol.
//!
//! ```text
//! A → B  hello { communities }            B → A  hello { communities }
//!        (to strangers: only those its routes led here for, the rest as salted hashes)
//! B → A  rng   { c, s, r }  (each shared community: B starts a session to learn what A holds,
//! A → B  rng   { c, r }      fingerprints of ranges, split where they differ)
//! B → A  want  { ids }      (what B lacks)          …and A starts its own session
//!        (to a peer that doesn't say `range`: have { c, ids }, every live id, instead)
//! A → B  recs  { [wire] }   (genesis, identities and delegations first)
//! later: recs  { [wire] }   (live: every new record, gossiped to peers in its community)
//! A → B  bwant { ids }      (media bytes a visible post references, by BLAKE3 hash)
//! B → A  blob  { id, data } (one item per message; A checks the hash and the bytes)
//! A → B  carry { g }        (to a public node: keep this community available; B answers carried)
//! A → B  find  { c }        (to a node that said `find`: who carries c? B answers found { c, r })
//! A → B  dht   { a }        (between public nodes that said `dht`: where their DHT nodes listen)
//! N → B  bind  { s }        (a native node to a browser on WebRTC, right after its hello)
//! B → A  nodes { routes, h } (from a public node: other public nodes it reached itself, and the
//!                            holders it saw online for days itself, SCATTER.md §5.7)
//! A → B  piece { op, rid, env } (scattered delivery: an envelope sealed to the holder B,
//!                               answered by B with one sealed to A's one-time key; SCATTER.md)
//! A → F  piece { op, rid, to, env } (the same through a forwarder F, which passes the envelope
//!                               to the holder `to` and its answer back, or says `gone`)
//! ```
//!
//! What a node sees on these links (who listed and offered what) is what its [`warmth`]
//! reading of a community is made of.
//!
//! Every list has a hard cap and decoding refuses anything longer, so a peer can't make a node
//! allocate or loop on its say-so.
//!
//! `have` lists full id sets. That is fine for communities the size of a dinner table; the
//! range-based reconciliation of PROTOCOL.md §7 (Negentropy) replaces it when they grow,
//! without changing the other messages.

use std::collections::BTreeSet;

use hoi_types::cbor::{self, Value};
use hoi_types::{Id, TypesError};

/// Version 2: hellos to anyone but a node's own devices carry salted community hashes, and a
/// home node gives each device it pairs a credential of its own (PROTOCOL.md §14).
pub const SYNC_VERSION: u64 = 2;
/// Records per `recs` message we send; we accept up to [`MAX_RECS`].
pub const BATCH: usize = 200;
pub const MAX_RECS: usize = 500;
/// Ids per `want`.
pub const MAX_WANT: usize = 5000;
/// Ids per `have`.
pub const MAX_HAVE: usize = 100_000;
/// Communities per `hello`, in each of its two lists.
pub const MAX_HELLO: usize = 1024;
/// Separates the salted community hashes in a hello from every other use of BLAKE3.
pub const COMMUNITY_HASH_CONTEXT: &str = "hoi 2026 hello community v1";
/// Hashes per `bwant`.
pub const MAX_BWANT: usize = 32;
/// Largest media item (video, PROTOCOL.md §17).
pub const MAX_BLOB: usize = 16 * 1024 * 1024;
/// Largest message a node reads: a full `blob` plus framing.
pub const MAX_MESSAGE: usize = MAX_BLOB + 64 * 1024;

/// Bounded route hints exchanged only on authenticated home-node links.
pub const MAX_HINT_COMMUNITIES: usize = 64;
pub const MAX_HINT_PEERS: usize = 16;
pub const MAX_HINT_RELAYS: usize = 8;
/// The sender's own routes in `hello.routes` (iroh `endpoint@addr` and `rtc:` hints).
pub const MAX_ROUTES: usize = 8;
/// Feature names in `hello.fx`, so newer messages only go to peers that understand them. Up to
/// 16 are read and sent (releases before scattered delivery refused more than 8; nothing older
/// is in use).
pub const MAX_FEATURES: usize = 16;
pub const SEND_FEATURES: usize = 16;
/// The sender answers `find`: which nodes carry a community, from what it knows and, for a node
/// that runs one, from the DHT among public nodes.
pub const FEATURE_FIND: &str = "find";
/// The sender runs a DHT node among public nodes and takes `dht` contacts.
pub const FEATURE_DHT: &str = "dht";
/// The sender reconciles with `rng` messages: a peer that says it gets them in
/// place of a `have` list.
pub const FEATURE_RANGE: &str = "range";
/// The sender fetches media over iroh-blobs on the same endpoint and serves the media of the
/// communities it pins that way, to peers it has a sync link with. Only native nodes
/// say it; a peer without it gets `bwant`.
pub const FEATURE_BLOBS: &str = "blobs";
/// From a native node: it passes `sig` between browsers linked to it that share a community and
/// said `direct`, and its WebRTC port answers STUN.
pub const FEATURE_SIGNAL: &str = "signal";
/// From a browser: it takes direct links with other members' browsers, so a node that
/// said `signal` lists it to them in `here`.
pub const FEATURE_DIRECT: &str = "direct";
/// Handles in a `here` message at most.
pub const MAX_HERE: usize = 16;
/// A `sig` payload (an offer or an answer, opaque to the node passing it) at most.
pub const MAX_SIG: usize = 8 * 1024;
/// Routes in a `found` answer at most.
pub const MAX_FOUND_ROUTES: usize = 16;
/// DHT contacts (`ip:port`) in a `dht` message at most.
pub const MAX_DHT_CONTACTS: usize = 4;
/// The one feature this version adds: `link` messages between a home node and its devices.
pub const FEATURE_LINK: &str = "link";
/// Placement and delivery receipts on authenticated home links only.
pub const FEATURE_HOME_SCATTER: &str = "home-scatter";
/// Public communities held by the connected node, offered without subscribing to their posts.
pub const FEATURE_CATALOG: &str = "catalog";
/// Optional per-community profile sidecars; old peers still exchange ordinary posts.
pub const FEATURE_PROFILE: &str = "profile";
/// Palette selections are separate from JPEG profiles so older clients keep photo support.
pub const FEATURE_PROFILE_COLOR: &str = "profile-color";
/// The sender is a public node that takes `carry` requests: anyone holding a community's signed
/// genesis may ask it to keep that community available (PROTOCOL.md §10).
pub const FEATURE_CARRY: &str = "carry";
/// The sender keeps records across restarts and can be reached by others (a desktop or a
/// headless node, not a browser or phone). Self-asserted; used only as a Warmth input, where it
/// decides whether the sender's copy counts as durable in the receiver's reading (§20).
pub const FEATURE_KEEPS: &str = "keeps";
/// The sender understands `nodes`: lists of public nodes, passed from node to node so a device
/// that reached one can find the others without any configured directory (PROTOCOL.md §14.2).
pub const FEATURE_NODES: &str = "nodes";
/// The sender publishes no address of its own (a browser, a phone, a desktop that isn't a
/// public node): don't remember the address its connection came from, don't pass it on in
/// invites, catalogs or node lists, and don't dial it back (PROTOCOL.md §14.3).
pub const FEATURE_UNLISTED: &str = "unlisted";
/// The sender takes `catalog` entries marked relayed (`x`): open communities it heard of from
/// other nodes, passed on node to node so every node comes to know every listing, and kept by
/// the receiver as a directory of its own rather than as what the sender holds (PROTOCOL.md
/// §14.4). Peers without it never get a relayed entry.
pub const FEATURE_DIRECTORY: &str = "directory";
/// Routes per `nodes` message: a few for each of a handful of nodes.
pub const MAX_NODE_ROUTES: usize = 32;
/// Holders one `nodes` message vouches for, and the most days known it reports.
pub const MAX_VOUCHES: usize = 32;
pub const MAX_VOUCH_DAYS: u16 = 30;
pub const MAX_CATALOG_ENTRIES: usize = 64;
pub const MAX_CATALOG_BYTES: usize = 256 * 1024;
/// A signed genesis in a `catalog` entry or a `carry` request (the community record's cap).
pub const MAX_GENESIS: usize = 64 * 1024;
/// The largest warmth a node reports for a community it holds (`catalog` entries' `w`): the
/// durable copies it saw itself, quantised so catalogs change rarely (PROTOCOL.md §20).
pub const MAX_WARMTH_HINT: u8 = 3;
/// A `carried` answer's reason.
pub const MAX_CARRIED_REASON: usize = 256;
/// A sealed `link` payload: one approval carries an identity record and community geneses.
pub const MAX_LINK: usize = 512 * 1024;
/// A sealed envelope in a `piece` message: a 1 MiB piece and its put header, with room to spare;
/// the envelope's own parser checks the exact size (`hoi_crypto::envelope`).
pub const MAX_ENVELOPE: usize = (1 << 20) + 1024;

/// A holder's published X25519 key for scattered delivery, signed by its endpoint key over
/// `hoi_crypto::envelope::xk_statement` (PROTOCOL.md §21). Its presence in a hello says the
/// sender holds pieces and answers `piece` messages; older peers ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HolderKey {
    pub xk: [u8; 32],
    pub expires_at: u64,
    pub sig: [u8; 64],
    /// `r`, optional: binds the certificate the holder serves WebRTC with to its endpoint, so a
    /// browser, which sees no endpoint on a WebRTC link, can check the key (older peers ignore it).
    pub rtc: Option<RtcBinding>,
}

/// A holder's endpoint and the SHA-256 of its WebRTC certificate, signed by the endpoint key
/// over `hoi_crypto::envelope::rtc_statement`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtcBinding {
    pub endpoint: Id,
    pub cert: [u8; 32],
    pub sig: [u8; 64],
}

fn rtc_binding_from(value: Option<Value>) -> Option<RtcBinding> {
    let Some(Value::M(entries)) = value else { return None };
    let mut m: std::collections::BTreeMap<String, Value> = entries.into_iter().collect();
    let endpoint = match m.remove("e") {
        Some(Value::B(b)) => Id::from_slice(&b).ok()?,
        _ => return None,
    };
    let cert = match m.remove("c") {
        Some(Value::B(b)) => b.try_into().ok()?,
        _ => return None,
    };
    let sig = match m.remove("s") {
        Some(Value::B(b)) => b.try_into().ok()?,
        _ => return None,
    };
    Some(RtcBinding { endpoint, cert, sig })
}

/// A holder the sender of a `nodes` list vouches for (SCATTER.md §5.7): one it saw itself, at
/// its own hourly samples, long and steadily enough to take pieces. Only first-hand sightings
/// are vouched for, never what the sender was told. A receiver counts a holder its own devices
/// vouch for, or two introducers in different places do, as if it had seen it itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vouch {
    /// The holder's endpoint, as transports authenticate it.
    pub endpoint: Id,
    /// The share of the sender's samples the holder was linked in, in percent.
    pub up: u8,
    /// Days since the sender first saw it, at most [`MAX_VOUCH_DAYS`].
    pub days: u16,
    /// `k`, optional: the holder's signed key as the holder's own hello gave it (never an `r`
    /// binding), so a device not linked to the holder can seal envelopes to it and send them
    /// through the sender (SCATTER.md §5.8, S3). Receivers check the signature against
    /// `endpoint`; a sender cannot make one up.
    pub key: Option<HolderKey>,
    /// `r`, optional: the holder's routes as its hello gave them, at most [`MAX_VOUCH_ROUTES`].
    /// A claim: receivers only ever use it to merge places, never to add one.
    pub routes: Vec<String>,
}

impl Vouch {
    /// A vouch with nothing but the figures (what releases before forwarding sent).
    pub fn bare(endpoint: Id, up: u8, days: u16) -> Vouch {
        Vouch { endpoint, up, days, key: None, routes: Vec::new() }
    }
}

/// Routes one vouch carries for its holder.
pub const MAX_VOUCH_ROUTES: usize = 2;

/// A `nodes` list's vouches. Lenient: an entry that isn't one, and a list that is too long, read
/// as no vouch, never as a reason to lose the node list beside them.
fn vouches_from(value: Option<Value>) -> Vec<Vouch> {
    let Some(Value::A(items)) = value else { return Vec::new() };
    if items.len() > MAX_VOUCHES {
        return Vec::new();
    }
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter_map(|v| {
            let Value::M(fields) = v else { return None };
            let mut f: std::collections::BTreeMap<String, Value> = fields.into_iter().collect();
            let endpoint = match f.remove("e") {
                Some(Value::B(b)) => Id::from_slice(&b).ok()?,
                _ => return None,
            };
            let up = match f.remove("u") {
                Some(Value::U(u)) if u <= 100 => u as u8,
                _ => return None,
            };
            let days = match f.remove("d") {
                Some(Value::U(d)) => d.min(MAX_VOUCH_DAYS as u64) as u16,
                _ => return None,
            };
            // Lenient: a key or routes this release can't read are left out, the vouch kept.
            let key = holder_from(f.remove("k")).ok().flatten().map(|k| HolderKey { rtc: None, ..k });
            let routes = texts_from(f.remove("r"), MAX_VOUCH_ROUTES, 128).unwrap_or_default();
            seen.insert(endpoint).then_some(Vouch { endpoint, up, days, key, routes })
        })
        .collect()
}

/// What a `piece` message carries: the envelope kinds of `hoi_crypto::envelope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceOp {
    Put,
    Get,
    Has,
    Drop,
    Stored,
    Got,
    Had,
    Dropped,
    /// From a forwarder: the holder a request named could not be reached, so the request went
    /// nowhere. Unsealed, one byte of envelope; only ever sent to a peer that asked through the
    /// forwarder (`to`), which understands it.
    Gone,
    /// A three-hop request's outer layer, for the exit forwarder (S7): it names the holder and
    /// carries the request's own envelope.
    Wrap,
    /// A device's own new records, for the node that injects them into the network (S7). No
    /// answer.
    Inject,
}

impl PieceOp {
    pub fn as_str(self) -> &'static str {
        match self {
            PieceOp::Put => "put",
            PieceOp::Get => "get",
            PieceOp::Has => "has",
            PieceOp::Drop => "drop",
            PieceOp::Stored => "stored",
            PieceOp::Got => "got",
            PieceOp::Had => "had",
            PieceOp::Dropped => "dropped",
            PieceOp::Gone => "gone",
            PieceOp::Wrap => "wrap",
            PieceOp::Inject => "inject",
        }
    }
    pub fn parse(s: &str) -> Option<PieceOp> {
        Some(match s {
            "put" => PieceOp::Put,
            "get" => PieceOp::Get,
            "has" => PieceOp::Has,
            "drop" => PieceOp::Drop,
            "stored" => PieceOp::Stored,
            "got" => PieceOp::Got,
            "had" => PieceOp::Had,
            "dropped" => PieceOp::Dropped,
            "gone" => PieceOp::Gone,
            "wrap" => PieceOp::Wrap,
            "inject" => PieceOp::Inject,
            _ => return None,
        })
    }
    /// A request a holder answers, rather than an answer.
    pub fn is_request(self) -> bool {
        matches!(self, PieceOp::Put | PieceOp::Get | PieceOp::Has | PieceOp::Drop)
    }
}

pub mod place;
pub mod range;
pub mod warmth;

/// A community as a hello to a stranger names it: `BLAKE3-derive-key(context, salt ‖ key)`. Only a
/// peer that already knows the key can match it, and a fresh salt per connection keeps the same
/// list from being recognised on the next one.
pub fn community_hash(salt: &[u8; 16], community: &Id) -> Id {
    let mut h = blake3::Hasher::new_derive_key(COMMUNITY_HASH_CONTEXT);
    h.update(salt);
    h.update(community.as_bytes());
    Id(*h.finalize().as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hints {
    pub community: Id,
    pub peers: Vec<String>,
    pub relays: Vec<String>,
}

/// A signed community genesis and public join routes. Never includes a pairing credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub genesis: Vec<u8>,
    pub peers: Vec<String>,
    pub relays: Vec<String>,
    /// How many durable copies the sender saw of this community itself, 0 to
    /// [`MAX_WARMTH_HINT`]; absent from older peers. A hint for the receiver's own Warmth,
    /// never added up (PROTOCOL.md §20).
    pub warmth: Option<u8>,
    /// Heard of from another node, not held by the sender: the routes are where that node
    /// said it is carried. Only sent to peers that advertised [`FEATURE_DIRECTORY`].
    pub relayed: bool,
    /// For a relayed entry: seconds since a node last listed the community as its own, as far
    /// as the sender knows. Passing a listing on never makes it younger, so a community nobody
    /// holds any more fades from every directory instead of circling between them.
    pub age: Option<u64>,
}

/// The oldest `age` a relayed entry may carry: past it, a listing is forgotten anyway.
pub const MAX_LISTING_AGE: u64 = 60 * 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // messages are handled one at a time; boxing the hello would only complicate every match
pub enum Msg {
    Hello {
        v: u64,
        /// Transport identity, separate from any posting key (TECHNICAL.md §9).
        node: Id,
        name: Option<String>,
        /// From a device to its home node: the pairing link's token, or the device's own
        /// credential. From a home node to a device that just showed the link's token: that
        /// device's own credential, to use from now on (the link is spent).
        token: Option<String>,
        /// Communities named in the clear: every public one between a home node and its devices;
        /// to anyone else, only those whose routes led to this connection.
        communities: Vec<Id>,
        /// To anyone else, the sender's other public communities as [`community_hash`]es under
        /// this connection's `salt`.
        salt: Option<[u8; 16]>,
        hashed: Vec<Id>,
        /// This node serves paired browsers and phones (a home node).
        serves: bool,
        hints: Vec<Hints>,
        /// The sender's own routes, so the other side can tell members how to reach it.
        routes: Vec<String>,
        /// Optional messages the sender understands ([`FEATURE_LINK`]).
        features: Vec<String>,
        /// From a home node to its devices: the token of its current pairing link, so a device can
        /// show the next one, and the key `link` messages are sealed with.
        pairing: Option<String>,
        link_key: Option<[u8; 32]>,
        /// From a public node to a stranger: its operator's notice, as the canonical
        /// bytes of a `hoi_types::NodeNotice`, and their BLAKE3 hash. A client shows the notice
        /// before it says anything of its own.
        notice_hash: Option<Id>,
        notice: Option<Vec<u8>>,
        /// From a node that holds pieces for scattered delivery: its signed holder key.
        holder: Option<HolderKey>,
        /// `fw`: the sender passes `piece` requests on to the holders it can reach (SCATTER.md
        /// §5.8, S3). Older peers ignore it; a peer without it is never asked to forward.
        forwards: bool,
        /// `rb`, from a home node to its own devices only: it fetches and rebuilds objects kept
        /// in pieces for them (`rebuild`, SCATTER.md §5.12). Older peers ignore it.
        rebuilds: bool,
    },
    Have {
        community: Id,
        ids: Vec<Id>,
    },
    Want {
        ids: Vec<Id>,
    },
    Recs {
        recs: Vec<Vec<u8>>,
    },
    BWant {
        ids: Vec<Id>,
    },
    Blob {
        id: Id,
        data: Vec<u8>,
    },
    Bye {
        reason: String,
    },
    /// Linking a new device through a home node: `mailbox` says whose devices it is for, `sealed`
    /// is encrypted with a key only those devices hold (PROTOCOL.md §14).
    Link {
        mailbox: [u8; 16],
        sealed: Vec<u8>,
    },
    /// A replacement snapshot, sent only when both peers advertised `catalog`.
    Catalog {
        entries: Vec<CatalogEntry>,
    },
    /// Asks a public node that advertised [`FEATURE_CARRY`] to keep a community available:
    /// its signed genesis, as an invite carries it, and nothing else. No routes travel with
    /// it: a stranger's request must never steer what a node dials (PROTOCOL.md §10), and a
    /// carrier learns members' routes only from their own authenticated connections. The node
    /// answers with [`Msg::Carried`] and, when it takes the community on, lists it in a fresh
    /// `hello`.
    Carry {
        genesis: Vec<u8>,
    },
    /// The answer to a `carry`: taken on (or already carried), or refused with a short reason
    /// ("full", "not open", "not accepting", "invalid"). Informational: Warmth measures what
    /// actually arrived.
    /// From a node that serves strangers: routes of other public nodes it reached itself, at
    /// most [`MAX_NODE_ROUTES`]. Only tried by the receiver, never trusted (PROTOCOL.md §14.2).
    /// `holders` (`h`, optional): the holders it vouches for ([`Vouch`]); older peers ignore it.
    Nodes {
        routes: Vec<String>,
        holders: Vec<Vouch>,
    },
    Carried {
        community: Id,
        ok: bool,
        reason: Option<String>,
    },
    /// Scattered delivery (SCATTER.md §5.8): an envelope sealed to a holder, or a holder's
    /// answer sealed to the asker's one-time key. `rid` pairs an answer with its request on this
    /// link; `to` names the holder when the peer only forwards (phase S3; absent: the peer is the
    /// holder).
    Piece {
        op: PieceOp,
        rid: u32,
        to: Option<Id>,
        env: Vec<u8>,
    },
    /// From a device to its home node only (one that said `rb`): fetch and rebuild the object a
    /// manifest names (`hoi_types::scatter::Manifest`, canonical CBOR), as the device's own
    /// device would (SCATTER.md §5.12).
    Rebuild {
        rid: u32,
        manifest: Vec<u8>,
    },
    /// The home node's answer: the object's content, or none when it could not be rebuilt.
    HomeScatter {
        rid: u32,
        reply: bool,
        payload: Vec<u8>,
    },
    Rebuilt {
        rid: u32,
        content: Option<Vec<u8>>,
    },
    /// To a node that said `find`: which nodes carry this community? Sent only when
    /// no route the asker holds answers; the node learns that the asker looks for it.
    Find {
        community: Id,
    },
    /// The answer: routes of nodes that carry it, as far as the node knows (its own directory,
    /// the public nodes it is connected to, the DHT). Only tried, never trusted, like any routes
    /// a stranger suggests (PROTOCOL.md §14.2).
    Found {
        community: Id,
        routes: Vec<String>,
    },
    /// Between public nodes that said `dht`: where their DHT nodes listen (`ip:port`), so the
    /// DHT among public nodes bootstraps from hoi's own links, never from outside routers.
    Dht {
        contacts: Vec<String>,
    },
    /// From a node that said `signal` to a browser that said `direct`: the browser's
    /// own handle on this link, and those of the other browsers linked here that said `direct`
    /// and share `community`. Handles are random per link.
    Here {
        community: Id,
        you: [u8; 16],
        handles: Vec<[u8; 16]>,
    },
    /// An offer or an answer between two browsers, through a node that said `signal`. To the
    /// node, `peer` is the handle to pass it to; from it, the sender's.
    Sig {
        peer: [u8; 16],
        data: Vec<u8>,
    },
    /// Right after a native node's hello on a WebRTC link: its endpoint key's
    /// signature over both DTLS certificate fingerprints, the ICE username fragment and that key,
    /// so the browser knows the node that said hello is the one at the other end of this channel.
    Bind {
        sig: [u8; 64],
    },
    /// Range-based reconciliation of one community, to a peer that said `range`, in
    /// place of `have`: `starter` says the sender began this session (it learns what the
    /// receiver holds), `count` is how many records the sender holds there, and `entries`
    /// partition the order up to [`range::Bound::END`].
    Range {
        community: Id,
        count: u64,
        starter: bool,
        entries: Vec<range::Entry>,
    },
}

/// A rebuilt object in a `rebuilt` answer: the largest class's content.
pub const MAX_REBUILT: usize = 4 << 20;
/// A manifest in a `rebuild` request.
pub const MAX_REBUILD_MANIFEST: usize = 512 + hoi_types::scatter::MAX_M * 64;

fn ids(v: &[Id]) -> Value {
    Value::A(v.iter().map(|i| Value::B(i.0.to_vec())).collect())
}

fn ids_from(v: Option<Value>, max: usize) -> Result<Vec<Id>, TypesError> {
    match v {
        Some(Value::A(items)) if items.len() > max => Err(TypesError::Field("too many ids".into())),
        Some(Value::A(items)) => items
            .into_iter()
            .map(|i| match i {
                Value::B(b) => Id::from_slice(&b),
                _ => Err(TypesError::Field("expected ids".into())),
            })
            .collect(),
        _ => Err(TypesError::Field("expected ids".into())),
    }
}

/// An optional list of short strings; absent means empty (older peers).
fn range_entries(entries: &[range::Entry]) -> Value {
    Value::A(
        entries
            .iter()
            .map(|e| {
                let mut v = vec![Value::U(e.upper.at), Value::B(e.upper.id.0[..e.upper.id_len()].to_vec())];
                match &e.range {
                    range::Range::Skip => v.push(Value::U(0)),
                    range::Range::Fingerprint(fp) => v.extend([Value::U(1), Value::B(fp.to_vec())]),
                    range::Range::Ids(list) => v.extend([Value::U(2), ids(list)]),
                }
                Value::A(v)
            })
            .collect(),
    )
}

/// Ranges as they came: at most [`range::MAX_RANGES`] of them and [`MAX_HAVE`] ids in all, each
/// bound above the one before, the last at the end of the order.
fn range_entries_from(v: Option<Value>) -> Result<Vec<range::Entry>, TypesError> {
    let bad = |m: &str| TypesError::Field(format!("rng: {m}"));
    let Some(Value::A(list)) = v else { return Err(bad("without ranges")) };
    if list.is_empty() || list.len() > range::MAX_RANGES {
        return Err(bad("too many ranges"));
    }
    let mut out: Vec<range::Entry> = Vec::with_capacity(list.len());
    let mut listed = 0usize;
    for item in list {
        let Value::A(parts) = item else { return Err(bad("range is not a list")) };
        let mut parts = parts.into_iter();
        let (Some(Value::U(at)), Some(Value::B(prefix)), Some(Value::U(mode))) = (parts.next(), parts.next(), parts.next()) else {
            return Err(bad("invalid range"));
        };
        if prefix.len() > 32 {
            return Err(bad("bound id longer than 32 bytes"));
        }
        let mut id = [0u8; 32];
        id[..prefix.len()].copy_from_slice(&prefix);
        let upper = range::Bound { at, id: Id(id) };
        if out.last().is_some_and(|l| l.upper >= upper) {
            return Err(bad("bounds out of order"));
        }
        let range = match (mode, parts.next()) {
            (0, None) => range::Range::Skip,
            (1, Some(Value::B(fp))) => range::Range::Fingerprint(fp.try_into().map_err(|_| bad("fingerprint is 16 bytes"))?),
            (2, list @ Some(Value::A(_))) => {
                let got = ids_from(list, MAX_HAVE - listed)?;
                listed += got.len();
                range::Range::Ids(got)
            }
            _ => return Err(bad("invalid range")),
        };
        if parts.next().is_some() {
            return Err(bad("invalid range"));
        }
        out.push(range::Entry { upper, range });
    }
    if out.last().is_some_and(|l| l.upper != range::Bound::END) {
        return Err(bad("ranges end before the end"));
    }
    Ok(out)
}

fn texts_from(value: Option<Value>, max: usize, length: usize) -> Result<Vec<String>, TypesError> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::A(items)) if items.len() <= max => items
            .into_iter()
            .map(|v| match v {
                Value::T(s) if s.len() <= length => Ok(s),
                _ => Err(TypesError::Field("invalid text in list".into())),
            })
            .collect(),
        _ => Err(TypesError::Field("list too long".into())),
    }
}

/// A hello's holder key; a malformed one refuses the hello rather than being half read.
fn holder_from(value: Option<Value>) -> Result<Option<HolderKey>, TypesError> {
    let Some(value) = value else { return Ok(None) };
    let Value::M(entries) = value else { return Err(TypesError::Field("invalid holder key".into())) };
    let mut m: std::collections::BTreeMap<String, Value> = entries.into_iter().collect();
    let bad = || TypesError::Field("invalid holder key".into());
    let xk = match m.remove("k") {
        Some(Value::B(b)) => b.try_into().map_err(|_| bad())?,
        _ => return Err(bad()),
    };
    let expires_at = match m.remove("e") {
        Some(Value::U(e)) => e,
        _ => return Err(bad()),
    };
    let sig = match m.remove("s") {
        Some(Value::B(b)) => b.try_into().map_err(|_| bad())?,
        _ => return Err(bad()),
    };
    // Lenient: a binding this release can't read is no binding, not a refused hello.
    let rtc = rtc_binding_from(m.remove("r"));
    Ok(Some(HolderKey { xk, expires_at, sig, rtc }))
}

fn hints_from(value: Option<Value>) -> Result<Vec<Hints>, TypesError> {
    fn texts(value: Option<Value>, max: usize, length: usize) -> Result<Vec<String>, TypesError> {
        match value {
            Some(Value::A(items)) if items.len() <= max => items
                .into_iter()
                .map(|v| match v {
                    Value::T(s) if s.len() <= length => Ok(s),
                    _ => Err(TypesError::Field("invalid route hint".into())),
                })
                .collect(),
            _ => Err(TypesError::Field("too many route hints".into())),
        }
    }
    match value {
        None => Ok(Vec::new()), // Older peers omit this extension.
        Some(Value::A(items)) if items.len() <= MAX_HINT_COMMUNITIES => items
            .into_iter()
            .map(|v| {
                let Value::M(entries) = v else {
                    return Err(TypesError::Field("invalid hints".into()));
                };
                let mut m: std::collections::BTreeMap<String, Value> = entries.into_iter().collect();
                let community = match m.remove("c") {
                    Some(Value::B(b)) => Id::from_slice(&b)?,
                    _ => return Err(TypesError::Field("hints without community".into())),
                };
                Ok(Hints { community, peers: texts(m.remove("p"), MAX_HINT_PEERS, 128)?, relays: texts(m.remove("r"), MAX_HINT_RELAYS, 512)? })
            })
            .collect(),
        _ => Err(TypesError::Field("too many hinted communities".into())),
    }
}

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        let t = |s: &str| Value::T(s.to_string());
        let v = match self {
            Msg::Hello {
                v,
                node,
                name,
                token,
                communities,
                salt,
                hashed,
                serves,
                hints,
                routes,
                features,
                pairing,
                link_key,
                notice_hash,
                notice,
                holder,
                forwards,
                rebuilds,
            } => {
                let mut e = vec![
                    ("t", t("hello")),
                    ("v", Value::U(*v)),
                    ("node", Value::B(node.0.to_vec())),
                    ("c", ids(communities)),
                    ("serves", Value::Bool(*serves)),
                ];
                if let Some(n) = name {
                    e.push(("name", t(n)));
                }
                if let Some(k) = token {
                    e.push(("token", t(k)));
                }
                if let Some(s) = salt {
                    e.push(("n", Value::B(s.to_vec())));
                    e.push(("ch", ids(hashed)));
                }
                if let Some(p) = pairing {
                    e.push(("pt", t(p)));
                }
                if let Some(k) = link_key {
                    e.push(("lk", Value::B(k.to_vec())));
                }
                if let Some(h) = notice_hash {
                    e.push(("nh", Value::B(h.0.to_vec())));
                }
                if let Some(n) = notice {
                    e.push(("nt", Value::B(n.clone())));
                }
                if let Some(h) = holder {
                    let mut k = vec![("k", Value::B(h.xk.to_vec())), ("e", Value::U(h.expires_at)), ("s", Value::B(h.sig.to_vec()))];
                    if let Some(r) = &h.rtc {
                        k.push(("r", Value::map([("e", Value::B(r.endpoint.0.to_vec())), ("c", Value::B(r.cert.to_vec())), ("s", Value::B(r.sig.to_vec()))])));
                    }
                    e.push(("sk", Value::map(k)));
                }
                if *forwards {
                    e.push(("fw", Value::Bool(true)));
                }
                if *rebuilds {
                    e.push(("rb", Value::Bool(true)));
                }
                if !hints.is_empty() {
                    e.push((
                        "hints",
                        Value::A(
                            hints
                                .iter()
                                .map(|h| {
                                    Value::map([
                                        ("c", Value::B(h.community.0.to_vec())),
                                        ("p", Value::A(h.peers.iter().map(|p| t(p)).collect())),
                                        ("r", Value::A(h.relays.iter().map(|r| t(r)).collect())),
                                    ])
                                })
                                .collect(),
                        ),
                    ));
                }
                if !routes.is_empty() {
                    e.push(("routes", Value::A(routes.iter().map(|r| t(r)).collect())));
                }
                if !features.is_empty() {
                    e.push(("fx", Value::A(features.iter().map(|f| t(f)).collect())));
                }
                Value::map(e)
            }
            Msg::Have { community, ids: list } => Value::map([("t", t("have")), ("c", Value::B(community.0.to_vec())), ("ids", ids(list))]),
            Msg::Want { ids: list } => Value::map([("t", t("want")), ("ids", ids(list))]),
            Msg::Recs { recs } => Value::map([("t", t("recs")), ("recs", Value::A(recs.iter().map(|r| Value::B(r.clone())).collect()))]),
            Msg::BWant { ids: list } => Value::map([("t", t("bwant")), ("ids", ids(list))]),
            Msg::Blob { id, data } => Value::map([("t", t("blob")), ("id", Value::B(id.0.to_vec())), ("data", Value::B(data.clone()))]),
            Msg::Bye { reason } => Value::map([("t", t("bye")), ("reason", t(reason))]),
            Msg::Link { mailbox, sealed } => Value::map([("t", t("link")), ("m", Value::B(mailbox.to_vec())), ("b", Value::B(sealed.clone()))]),
            Msg::Catalog { entries } => Value::map([
                ("t", t("catalog")),
                (
                    "entries",
                    Value::A(
                        entries
                            .iter()
                            .map(|e| {
                                let mut fields = vec![
                                    ("g", Value::B(e.genesis.clone())),
                                    ("p", Value::A(e.peers.iter().map(|p| t(p)).collect())),
                                    ("r", Value::A(e.relays.iter().map(|r| t(r)).collect())),
                                ];
                                if let Some(w) = e.warmth {
                                    fields.push(("w", Value::U(w.min(MAX_WARMTH_HINT) as u64)));
                                }
                                if e.relayed {
                                    fields.push(("x", Value::U(1)));
                                    if let Some(a) = e.age {
                                        fields.push(("a", Value::U(a.min(MAX_LISTING_AGE))));
                                    }
                                }
                                Value::map(fields)
                            })
                            .collect(),
                    ),
                ),
            ]),
            Msg::Carry { genesis } => Value::map([("t", t("carry")), ("g", Value::B(genesis.clone()))]),
            Msg::Nodes { routes, holders } => {
                let mut e = vec![("t", t("nodes")), ("routes", Value::A(routes.iter().map(|r| t(r)).collect()))];
                if !holders.is_empty() {
                    e.push((
                        "h",
                        Value::A(
                            holders
                                .iter()
                                .map(|v| {
                                    let mut f =
                                        vec![("e", Value::B(v.endpoint.0.to_vec())), ("u", Value::U(v.up.min(100) as u64)), ("d", Value::U(v.days as u64))];
                                    if let Some(k) = &v.key {
                                        f.push((
                                            "k",
                                            Value::map([("k", Value::B(k.xk.to_vec())), ("e", Value::U(k.expires_at)), ("s", Value::B(k.sig.to_vec()))]),
                                        ));
                                    }
                                    if !v.routes.is_empty() {
                                        f.push(("r", Value::A(v.routes.iter().take(MAX_VOUCH_ROUTES).map(|r| t(r)).collect())));
                                    }
                                    Value::map(f)
                                })
                                .collect(),
                        ),
                    ));
                }
                Value::map(e)
            }
            Msg::Carried { community, ok, reason } => {
                let mut e = vec![("t", t("carried")), ("c", Value::B(community.0.to_vec())), ("ok", Value::Bool(*ok))];
                if let Some(r) = reason {
                    e.push(("reason", t(r)));
                }
                Value::map(e)
            }
            Msg::HomeScatter { rid, reply, payload } => {
                Value::map([("t", t("home-scatter")), ("r", Value::U(*rid as u64)), ("reply", Value::Bool(*reply)), ("p", Value::B(payload.clone()))])
            }
            Msg::Rebuild { rid, manifest } => Value::map([("t", t("rebuild")), ("r", Value::U(*rid as u64)), ("m", Value::B(manifest.clone()))]),
            Msg::Rebuilt { rid, content } => {
                let mut e = vec![("t", t("rebuilt")), ("r", Value::U(*rid as u64))];
                if let Some(c) = content {
                    e.push(("c", Value::B(c.clone())));
                }
                Value::map(e)
            }
            Msg::Find { community } => Value::map([("t", t("find")), ("c", Value::B(community.0.to_vec()))]),
            Msg::Found { community, routes } => {
                Value::map([("t", t("found")), ("c", Value::B(community.0.to_vec())), ("r", Value::A(routes.iter().map(|r| t(r)).collect()))])
            }
            Msg::Dht { contacts } => Value::map([("t", t("dht")), ("a", Value::A(contacts.iter().map(|r| t(r)).collect()))]),
            Msg::Here { community, you, handles } => Value::map([
                ("t", t("here")),
                ("c", Value::B(community.0.to_vec())),
                ("y", Value::B(you.to_vec())),
                ("h", Value::A(handles.iter().map(|h| Value::B(h.to_vec())).collect())),
            ]),
            Msg::Sig { peer, data } => Value::map([("t", t("sig")), ("p", Value::B(peer.to_vec())), ("d", Value::B(data.clone()))]),
            Msg::Bind { sig } => Value::map([("t", t("bind")), ("s", Value::B(sig.to_vec()))]),
            Msg::Range { community, count, starter, entries } => Value::map([
                ("t", t("rng")),
                ("c", Value::B(community.0.to_vec())),
                ("n", Value::U(*count)),
                ("s", Value::Bool(*starter)),
                ("r", range_entries(entries)),
            ]),
            Msg::Piece { op, rid, to, env } => {
                let mut e = vec![("t", t("piece")), ("o", t(op.as_str())), ("r", Value::U(*rid as u64)), ("e", Value::B(env.clone()))];
                if let Some(h) = to {
                    e.push(("to", Value::B(h.0.to_vec())));
                }
                Value::map(e)
            }
        };
        v.encode()
    }

    pub fn decode(bytes: &[u8]) -> Result<Msg, TypesError> {
        if bytes.len() > MAX_MESSAGE {
            return Err(TypesError::Field("message too large".into()));
        }
        let Value::M(entries) = cbor::decode(bytes)? else {
            return Err(TypesError::Field("message must be a map".into()));
        };
        let mut m: std::collections::BTreeMap<String, Value> = entries.into_iter().collect();
        let bounded_text = |m: &mut std::collections::BTreeMap<String, Value>, k: &str, max: usize| -> Result<Option<String>, TypesError> {
            match m.remove(k) {
                None => Ok(None),
                Some(Value::T(s)) if s.len() <= max => Ok(Some(s)),
                _ => Err(TypesError::Field(format!("invalid or oversized {k}"))),
            }
        };
        let kind = bounded_text(&mut m, "t", 16)?.ok_or_else(|| TypesError::Field("message without type".into()))?;
        Ok(match kind.as_str() {
            "hello" => {
                let salt: Option<[u8; 16]> = match m.remove("n") {
                    None => None,
                    Some(Value::B(b)) => Some(b.try_into().map_err(|_| TypesError::Field("hello salt must be 16 bytes".into()))?),
                    _ => return Err(TypesError::Field("invalid hello salt".into())),
                };
                // Hashes mean nothing without the salt they were made with.
                let hashed = match (salt, m.remove("ch")) {
                    (_, None) => Vec::new(),
                    (Some(_), list) => ids_from(list, MAX_HELLO)?,
                    (None, Some(_)) => return Err(TypesError::Field("hashed communities without a salt".into())),
                };
                Msg::Hello {
                    v: match m.remove("v") {
                        Some(Value::U(n)) => n,
                        _ => 0,
                    },
                    node: match m.remove("node") {
                        Some(Value::B(b)) => Id::from_slice(&b)?,
                        _ => return Err(TypesError::Field("hello without node".into())),
                    },
                    name: bounded_text(&mut m, "name", 256)?,
                    token: bounded_text(&mut m, "token", 256)?,
                    communities: ids_from(m.remove("c"), MAX_HELLO)?,
                    salt,
                    hashed,
                    serves: matches!(m.remove("serves"), Some(Value::Bool(true))),
                    hints: hints_from(m.remove("hints"))?,
                    routes: texts_from(m.remove("routes"), MAX_ROUTES, 128)?,
                    features: texts_from(m.remove("fx"), MAX_FEATURES, 16)?,
                    pairing: bounded_text(&mut m, "pt", 256)?,
                    link_key: match m.remove("lk") {
                        None => None,
                        Some(Value::B(b)) => Some(b.try_into().map_err(|_| TypesError::Field("link key must be 32 bytes".into()))?),
                        _ => return Err(TypesError::Field("invalid link key".into())),
                    },
                    notice_hash: match m.remove("nh") {
                        None => None,
                        Some(Value::B(b)) => Some(Id::from_slice(&b)?),
                        _ => return Err(TypesError::Field("invalid notice hash".into())),
                    },
                    notice: match m.remove("nt") {
                        None => None,
                        Some(Value::B(b)) if b.len() <= hoi_types::notice::MAX_NOTICE_WIRE => Some(b),
                        _ => return Err(TypesError::Field("invalid or oversized notice".into())),
                    },
                    holder: holder_from(m.remove("sk"))?,
                    forwards: matches!(m.remove("fw"), Some(Value::Bool(true))),
                    rebuilds: matches!(m.remove("rb"), Some(Value::Bool(true))),
                }
            }
            "have" => Msg::Have {
                community: match m.remove("c") {
                    Some(Value::B(b)) => Id::from_slice(&b)?,
                    _ => return Err(TypesError::Field("have without community".into())),
                },
                ids: ids_from(m.remove("ids"), MAX_HAVE)?,
            },
            "want" => Msg::Want { ids: ids_from(m.remove("ids"), MAX_WANT)? },
            "bwant" => Msg::BWant { ids: ids_from(m.remove("ids"), MAX_BWANT)? },
            "blob" => Msg::Blob {
                id: match m.remove("id") {
                    Some(Value::B(b)) => Id::from_slice(&b)?,
                    _ => return Err(TypesError::Field("blob without id".into())),
                },
                data: match m.remove("data") {
                    Some(Value::B(b)) if b.len() <= MAX_BLOB => b,
                    _ => return Err(TypesError::Field("blob without data, or too big".into())),
                },
            },
            "recs" => Msg::Recs {
                recs: match m.remove("recs") {
                    Some(Value::A(items)) if items.len() > MAX_RECS => return Err(TypesError::Field("too many records".into())),
                    Some(Value::A(items)) => items
                        .into_iter()
                        .map(|i| match i {
                            Value::B(b) => Ok(b),
                            _ => Err(TypesError::Field("recs must be bytes".into())),
                        })
                        .collect::<Result<_, _>>()?,
                    _ => return Err(TypesError::Field("recs without records".into())),
                },
            },
            "bye" => Msg::Bye { reason: bounded_text(&mut m, "reason", 1024)?.unwrap_or_default() },
            "link" => Msg::Link {
                mailbox: match m.remove("m") {
                    Some(Value::B(b)) => b.try_into().map_err(|_| TypesError::Field("link mailbox must be 16 bytes".into()))?,
                    _ => return Err(TypesError::Field("link without mailbox".into())),
                },
                sealed: match m.remove("b") {
                    Some(Value::B(b)) if !b.is_empty() && b.len() <= MAX_LINK => b,
                    _ => return Err(TypesError::Field("link without payload, or too big".into())),
                },
            },
            "catalog" => {
                if bytes.len() > MAX_CATALOG_BYTES {
                    return Err(TypesError::Field("catalog too large".into()));
                }
                let Some(Value::A(items)) = m.remove("entries") else {
                    return Err(TypesError::Field("catalog without entries".into()));
                };
                if items.len() > MAX_CATALOG_ENTRIES {
                    return Err(TypesError::Field("too many catalog entries".into()));
                }
                let entries = items
                    .into_iter()
                    .map(|v| {
                        let Value::M(fields) = v else { return Err(TypesError::Field("invalid catalog entry".into())) };
                        let mut fields: std::collections::BTreeMap<String, Value> = fields.into_iter().collect();
                        let genesis = match fields.remove("g") {
                            Some(Value::B(b)) if !b.is_empty() && b.len() <= MAX_GENESIS => b,
                            _ => return Err(TypesError::Field("invalid catalog genesis".into())),
                        };
                        // A hint over the cap is clamped, never a reason to lose the whole
                        // listing: entries ignore what they don't know, so a newer node's
                        // higher cap doesn't cost older nodes their directory.
                        let warmth = match fields.remove("w") {
                            Some(Value::U(w)) => Some(w.min(MAX_WARMTH_HINT as u64) as u8),
                            _ => None,
                        };
                        let relayed = matches!(fields.remove("x"), Some(Value::U(1)));
                        let age = match fields.remove("a") {
                            Some(Value::U(a)) if relayed => Some(a.min(MAX_LISTING_AGE)),
                            _ => None,
                        };
                        Ok(CatalogEntry {
                            genesis,
                            peers: texts_from(fields.remove("p"), MAX_HINT_PEERS, 128)?,
                            relays: texts_from(fields.remove("r"), MAX_HINT_RELAYS, 512)?,
                            warmth,
                            relayed,
                            age,
                        })
                    })
                    .collect::<Result<_, _>>()?;
                Msg::Catalog { entries }
            }
            "carry" => Msg::Carry {
                genesis: match m.remove("g") {
                    Some(Value::B(b)) if !b.is_empty() && b.len() <= MAX_GENESIS => b,
                    _ => return Err(TypesError::Field("carry without a genesis, or too big".into())),
                },
            },
            "nodes" => Msg::Nodes { routes: texts_from(m.remove("routes"), MAX_NODE_ROUTES, 128)?, holders: vouches_from(m.remove("h")) },
            "carried" => Msg::Carried {
                community: match m.remove("c") {
                    Some(Value::B(b)) => Id::from_slice(&b)?,
                    _ => return Err(TypesError::Field("carried without community".into())),
                },
                ok: match m.remove("ok") {
                    Some(Value::Bool(ok)) => ok,
                    _ => return Err(TypesError::Field("carried without ok".into())),
                },
                reason: bounded_text(&mut m, "reason", MAX_CARRIED_REASON)?,
            },
            "piece" => Msg::Piece {
                op: bounded_text(&mut m, "o", 8)?.as_deref().and_then(PieceOp::parse).ok_or_else(|| TypesError::Field("piece without a known op".into()))?,
                rid: match m.remove("r") {
                    Some(Value::U(r)) if r <= u32::MAX as u64 => r as u32,
                    _ => return Err(TypesError::Field("piece without a request id".into())),
                },
                to: match m.remove("to") {
                    None => None,
                    Some(Value::B(b)) => Some(Id::from_slice(&b)?),
                    _ => return Err(TypesError::Field("invalid piece holder".into())),
                },
                env: match m.remove("e") {
                    Some(Value::B(b)) if !b.is_empty() && b.len() <= MAX_ENVELOPE => b,
                    _ => return Err(TypesError::Field("piece without an envelope, or too big".into())),
                },
            },
            "home-scatter" => Msg::HomeScatter {
                rid: match m.remove("r") {
                    Some(Value::U(r)) if r <= u32::MAX as u64 => r as u32,
                    _ => return Err(TypesError::Field("home scatter request id".into())),
                },
                reply: matches!(m.remove("reply"), Some(Value::Bool(true))),
                payload: match m.remove("p") {
                    Some(Value::B(p)) if p.len() <= 6 * 1024 * 1024 => p,
                    _ => return Err(TypesError::Field("home scatter payload too big".into())),
                },
            },
            "rebuild" => Msg::Rebuild {
                rid: match m.remove("r") {
                    Some(Value::U(r)) if r <= u32::MAX as u64 => r as u32,
                    _ => return Err(TypesError::Field("rebuild without a request id".into())),
                },
                manifest: match m.remove("m") {
                    Some(Value::B(b)) if !b.is_empty() && b.len() <= MAX_REBUILD_MANIFEST => b,
                    _ => return Err(TypesError::Field("rebuild without a manifest, or too big".into())),
                },
            },
            "rebuilt" => Msg::Rebuilt {
                rid: match m.remove("r") {
                    Some(Value::U(r)) if r <= u32::MAX as u64 => r as u32,
                    _ => return Err(TypesError::Field("rebuilt without a request id".into())),
                },
                content: match m.remove("c") {
                    None => None,
                    Some(Value::B(b)) if b.len() <= MAX_REBUILT => Some(b),
                    _ => return Err(TypesError::Field("rebuilt content too big".into())),
                },
            },
            "find" => Msg::Find {
                community: match m.remove("c") {
                    Some(Value::B(b)) => Id::from_slice(&b)?,
                    _ => return Err(TypesError::Field("find without community".into())),
                },
            },
            "found" => Msg::Found {
                community: match m.remove("c") {
                    Some(Value::B(b)) => Id::from_slice(&b)?,
                    _ => return Err(TypesError::Field("found without community".into())),
                },
                routes: texts_from(m.remove("r"), MAX_FOUND_ROUTES, 128)?,
            },
            "dht" => Msg::Dht { contacts: texts_from(m.remove("a"), MAX_DHT_CONTACTS, 64)? },
            "here" => Msg::Here {
                community: match m.remove("c") {
                    Some(Value::B(b)) => Id::from_slice(&b)?,
                    _ => return Err(TypesError::Field("here without community".into())),
                },
                you: match m.remove("y") {
                    Some(Value::B(b)) => b.try_into().map_err(|_| TypesError::Field("a handle is 16 bytes".into()))?,
                    _ => return Err(TypesError::Field("here without your handle".into())),
                },
                handles: match m.remove("h") {
                    Some(Value::A(a)) if a.len() <= MAX_HERE => a
                        .into_iter()
                        .map(|v| match v {
                            Value::B(b) => b.try_into().map_err(|_| TypesError::Field("a handle is 16 bytes".into())),
                            _ => Err(TypesError::Field("a handle is 16 bytes".into())),
                        })
                        .collect::<Result<Vec<[u8; 16]>, _>>()?,
                    _ => return Err(TypesError::Field("here without handles, or too many".into())),
                },
            },
            "sig" => Msg::Sig {
                peer: match m.remove("p") {
                    Some(Value::B(b)) => b.try_into().map_err(|_| TypesError::Field("a handle is 16 bytes".into()))?,
                    _ => return Err(TypesError::Field("sig without a handle".into())),
                },
                data: match m.remove("d") {
                    Some(Value::B(b)) if !b.is_empty() && b.len() <= MAX_SIG => b,
                    _ => return Err(TypesError::Field("sig without data, or too big".into())),
                },
            },
            "bind" => Msg::Bind {
                sig: match m.remove("s") {
                    Some(Value::B(b)) => b.try_into().map_err(|_| TypesError::Field("bind signature is 64 bytes".into()))?,
                    _ => return Err(TypesError::Field("bind without a signature".into())),
                },
            },
            "rng" => Msg::Range {
                community: match m.remove("c") {
                    Some(Value::B(b)) => Id::from_slice(&b)?,
                    _ => return Err(TypesError::Field("rng without community".into())),
                },
                count: match m.remove("n") {
                    Some(Value::U(n)) => n,
                    _ => return Err(TypesError::Field("rng without count".into())),
                },
                starter: matches!(m.remove("s"), Some(Value::Bool(true))),
                entries: range_entries_from(m.remove("r"))?,
            },
            other => return Err(TypesError::Field(format!("unknown message {other:?}"))),
        })
    }
}

/// Ids in `theirs` that `has` doesn't know, capped at [`MAX_WANT`].
pub fn missing(theirs: &[Id], has: impl Fn(&Id) -> bool) -> Vec<Id> {
    let mut seen = BTreeSet::new();
    theirs.iter().filter(|i| seen.insert(**i) && !has(i)).take(MAX_WANT).copied().collect()
}

/// Splits wires into `recs` messages of at most [`BATCH`] records or ~1 MiB.
pub fn batches(wires: Vec<Vec<u8>>) -> Vec<Msg> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut size = 0;
    for w in wires {
        if !cur.is_empty() && (cur.len() >= BATCH || size + w.len() > 1 << 20) {
            out.push(Msg::Recs { recs: std::mem::take(&mut cur) });
            size = 0;
        }
        size += w.len();
        cur.push(w);
    }
    if !cur.is_empty() {
        out.push(Msg::Recs { recs: cur });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hello with nothing in it, for tests to fill in.
    fn bare() -> Msg {
        Msg::Hello {
            v: SYNC_VERSION,
            node: Id::ZERO,
            name: None,
            token: None,
            communities: vec![],
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
    }

    #[test]
    fn roundtrip() {
        let msgs = vec![
            Msg::Hello {
                v: 1,
                node: Id([1; 32]),
                name: Some("lena's laptop".into()),
                token: Some("t".into()),
                communities: vec![Id([2; 32])],
                salt: None,
                hashed: Vec::new(),
                serves: true,
                hints: Vec::new(),
                routes: vec!["rtc:abc@192.168.1.2:7844".into()],
                features: vec![FEATURE_LINK.into()],
                pairing: Some("next".into()),
                link_key: Some([9; 32]),
                notice_hash: None,
                notice: None,
                holder: None,
                forwards: false,
                rebuilds: false,
            },
            Msg::Hello {
                v: SYNC_VERSION,
                node: Id([1; 32]),
                name: None,
                token: None,
                communities: vec![Id([2; 32])],
                salt: Some([4; 16]),
                hashed: vec![community_hash(&[4; 16], &Id([3; 32]))],
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
            },
            Msg::Have { community: Id([2; 32]), ids: vec![Id([3; 32]), Id([4; 32])] },
            Msg::Want { ids: vec![Id([3; 32])] },
            Msg::Recs { recs: vec![vec![1, 2, 3]] },
            Msg::BWant { ids: vec![Id([5; 32])] },
            Msg::Blob { id: Id([5; 32]), data: vec![1, 2] },
            Msg::Bye { reason: "bad token".into() },
            Msg::Link { mailbox: [7; 16], sealed: vec![1, 2, 3] },
            Msg::Catalog {
                entries: vec![CatalogEntry { genesis: vec![1, 2, 3], peers: vec![Id([3; 32]).hex()], relays: vec![], warmth: None, relayed: false, age: None }],
            },
            Msg::Catalog { entries: vec![CatalogEntry { genesis: vec![1, 2, 3], peers: vec![], relays: vec![], warmth: Some(2), relayed: false, age: None }] },
            Msg::Catalog {
                entries: vec![CatalogEntry {
                    genesis: vec![1, 2, 3],
                    peers: vec![Id([3; 32]).hex()],
                    relays: vec![],
                    warmth: None,
                    relayed: true,
                    age: Some(3600),
                }],
            },
            Msg::Carry { genesis: vec![1, 2, 3] },
            Msg::Find { community: Id([3; 32]) },
            Msg::Found { community: Id([3; 32]), routes: vec!["rtc:x@203.0.113.1:7844".into()] },
            Msg::Found { community: Id([3; 32]), routes: vec![] },
            Msg::Dht { contacts: vec!["203.0.113.1:7845".into()] },
            Msg::Here { community: Id([3; 32]), you: [1; 16], handles: vec![[2; 16], [4; 16]] },
            Msg::Here { community: Id([3; 32]), you: [1; 16], handles: vec![] },
            Msg::Sig { peer: [2; 16], data: b"{\"type\":\"offer\"}".to_vec() },
            Msg::Bind { sig: [7; 64] },
            Msg::Range {
                community: Id([3; 32]),
                count: 3,
                starter: true,
                entries: vec![
                    range::Entry { upper: range::Bound { at: 5, id: Id([0; 32]) }, range: range::Range::Skip },
                    range::Entry {
                        upper: range::Bound { at: 9, id: Id([[4; 3].as_slice(), &[0; 29]].concat().try_into().unwrap()) },
                        range: range::Range::Fingerprint([2; 16]),
                    },
                    range::Entry { upper: range::Bound::END, range: range::Range::Ids(vec![Id([5; 32]), Id([6; 32])]) },
                ],
            },
            Msg::Carried { community: Id([2; 32]), ok: true, reason: None },
            Msg::Carried { community: Id([2; 32]), ok: false, reason: Some("full".into()) },
            Msg::Nodes { routes: vec![format!("rtc:{}@203.0.113.1:7844", "q".repeat(43)), format!("{}@203.0.113.1:7843", Id([3; 32]).hex())], holders: vec![] },
            Msg::Nodes { routes: vec![], holders: vec![] },
            Msg::Nodes { routes: vec![], holders: vec![Vouch::bare(Id([6; 32]), 97, 12), Vouch::bare(Id([7; 32]), 80, 30)] },
            Msg::Nodes {
                routes: vec![],
                holders: vec![Vouch {
                    endpoint: Id([6; 32]),
                    up: 97,
                    days: 12,
                    key: Some(HolderKey { xk: [7; 32], expires_at: 1_800_000_000, sig: [9; 64], rtc: None }),
                    routes: vec![format!("{}@203.0.113.9:7843", Id([6; 32]).hex())],
                }],
            },
        ];
        for m in msgs {
            assert_eq!(Msg::decode(&m.encode()).unwrap(), m);
        }
    }

    #[test]
    fn node_lists_are_bounded() {
        let route = |i: usize| format!("rtc:{}@203.0.113.1:{}", "q".repeat(43), 1000 + i);
        assert!(Msg::decode(&Msg::Nodes { routes: (0..MAX_NODE_ROUTES).map(route).collect(), holders: vec![] }.encode()).is_ok());
        assert!(Msg::decode(&Msg::Nodes { routes: (0..MAX_NODE_ROUTES + 1).map(route).collect(), holders: vec![] }.encode()).is_err());
        assert!(Msg::decode(&Msg::Nodes { routes: vec!["x".repeat(129)], holders: vec![] }.encode()).is_err());
        // A list without its field is an empty list: the message itself is what says "public".
        assert_eq!(Msg::decode(&Value::map([("t", Value::T("nodes".into()))]).encode()).unwrap(), Msg::Nodes { routes: vec![], holders: vec![] });
    }

    #[test]
    fn vouches_are_bounded_and_never_cost_the_node_list() {
        let v = |i: u8| Vouch::bare(Id([i; 32]), 90, 5);
        let full = Msg::Nodes { routes: vec![], holders: (0..MAX_VOUCHES as u8).map(v).collect() };
        assert_eq!(Msg::decode(&full.encode()).unwrap(), full);
        // one too many reads as none; the routes beside them still count
        let route = format!("{}@203.0.113.1:7843", Id([3; 32]).hex());
        let over = Msg::Nodes { routes: vec![route.clone()], holders: (0..MAX_VOUCHES as u8 + 1).map(v).collect() };
        assert_eq!(Msg::decode(&over.encode()).unwrap(), Msg::Nodes { routes: vec![route.clone()], holders: vec![] });
        // a malformed entry is skipped, a repeated holder counts once, days are capped
        let entry = |e: Value, u: u64, d: u64| Value::map([("e", e), ("u", Value::U(u)), ("d", Value::U(d))]);
        let raw = Value::map([
            ("t", Value::T("nodes".into())),
            ("routes", Value::A(vec![Value::T(route.clone())])),
            (
                "h",
                Value::A(vec![
                    entry(Value::B(vec![1; 32]), 99, 400),
                    entry(Value::B(vec![1; 32]), 95, 3),
                    entry(Value::B(vec![2; 31]), 99, 3),
                    entry(Value::B(vec![3; 32]), 101, 3),
                    Value::T("not a vouch".into()),
                ]),
            ),
        ])
        .encode();
        assert_eq!(Msg::decode(&raw).unwrap(), Msg::Nodes { routes: vec![route.clone()], holders: vec![Vouch::bare(Id([1; 32]), 99, MAX_VOUCH_DAYS)] });
        // a malformed key or routes are left out; the vouch stays
        let raw = Value::map([
            ("t", Value::T("nodes".into())),
            (
                "h",
                Value::A(vec![Value::map([
                    ("e", Value::B(vec![1; 32])),
                    ("u", Value::U(90)),
                    ("d", Value::U(3)),
                    ("k", Value::map([("k", Value::B(vec![7; 31]))])),
                    ("r", Value::A(vec![Value::T("a".into()), Value::T("b".into()), Value::T("c".into())])),
                ])]),
            ),
        ])
        .encode();
        assert_eq!(Msg::decode(&raw).unwrap(), Msg::Nodes { routes: vec![], holders: vec![Vouch::bare(Id([1; 32]), 90, 3)] });
        // a field of another shape is no vouch at all
        let odd = Value::map([("t", Value::T("nodes".into())), ("h", Value::T("x".into()))]).encode();
        assert_eq!(Msg::decode(&odd).unwrap(), Msg::Nodes { routes: vec![], holders: vec![] });
    }

    #[test]
    fn carry_requests_and_answers_are_bounded() {
        assert!(Msg::decode(&Msg::Carry { genesis: vec![1; MAX_GENESIS] }.encode()).is_ok());
        assert!(Msg::decode(&Msg::Carry { genesis: vec![1; MAX_GENESIS + 1] }.encode()).is_err());
        assert!(Msg::decode(&Msg::Carry { genesis: vec![] }.encode()).is_err());
        // Routes in a request are not a field: a sender that adds them is ignored, not obeyed.
        let with_routes =
            Value::map([("t", Value::T("carry".into())), ("g", Value::B(vec![1])), ("p", Value::A(vec![Value::T("rtc:x@203.0.113.1:7844".into())]))]).encode();
        assert_eq!(Msg::decode(&with_routes).unwrap(), Msg::Carry { genesis: vec![1] });
        assert!(Msg::decode(&Msg::Carried { community: Id::ZERO, ok: false, reason: Some("r".repeat(MAX_CARRIED_REASON)) }.encode()).is_ok());
        assert!(Msg::decode(&Msg::Carried { community: Id::ZERO, ok: false, reason: Some("r".repeat(MAX_CARRIED_REASON + 1)) }.encode()).is_err());
        let no_ok = Value::map([("t", Value::T("carried".into())), ("c", Value::B(vec![0; 32]))]).encode();
        assert!(Msg::decode(&no_ok).is_err());
    }

    #[test]
    fn catalog_warmth_is_optional_and_bounded() {
        // An older peer's entry has no `w`; a newer one's is at most the quantised maximum.
        let plain = Value::map([
            ("t", Value::T("catalog".into())),
            ("entries", Value::A(vec![Value::map([("g", Value::B(vec![1])), ("p", Value::A(vec![])), ("r", Value::A(vec![]))])])),
        ])
        .encode();
        assert_eq!(
            Msg::decode(&plain).unwrap(),
            Msg::Catalog { entries: vec![CatalogEntry { genesis: vec![1], peers: vec![], relays: vec![], warmth: None, relayed: false, age: None }] }
        );
        let hinted = |w: u64| {
            Value::map([
                ("t", Value::T("catalog".into())),
                ("entries", Value::A(vec![Value::map([("g", Value::B(vec![1])), ("p", Value::A(vec![])), ("r", Value::A(vec![])), ("w", Value::U(w))])])),
            ])
            .encode()
        };
        assert!(Msg::decode(&hinted(MAX_WARMTH_HINT as u64)).is_ok());
        // Over the cap is clamped on reading too; a hint of the wrong shape is simply absent.
        assert_eq!(
            Msg::decode(&hinted(MAX_WARMTH_HINT as u64 + 1)).unwrap(),
            Msg::Catalog {
                entries: vec![CatalogEntry { genesis: vec![1], peers: vec![], relays: vec![], warmth: Some(MAX_WARMTH_HINT), relayed: false, age: None }]
            }
        );
        let odd = Value::map([
            ("t", Value::T("catalog".into())),
            (
                "entries",
                Value::A(vec![Value::map([("g", Value::B(vec![1])), ("p", Value::A(vec![])), ("r", Value::A(vec![])), ("w", Value::T("hot".into()))])]),
            ),
        ])
        .encode();
        assert_eq!(
            Msg::decode(&odd).unwrap(),
            Msg::Catalog { entries: vec![CatalogEntry { genesis: vec![1], peers: vec![], relays: vec![], warmth: None, relayed: false, age: None }] }
        );
        // Encoding clamps what a host hands over, so a node never sends more than the cap.
        let clamped =
            Msg::Catalog { entries: vec![CatalogEntry { genesis: vec![1], peers: vec![], relays: vec![], warmth: Some(200), relayed: false, age: None }] };
        assert_eq!(
            Msg::decode(&clamped.encode()).unwrap(),
            Msg::Catalog {
                entries: vec![CatalogEntry { genesis: vec![1], peers: vec![], relays: vec![], warmth: Some(MAX_WARMTH_HINT), relayed: false, age: None }]
            }
        );
    }

    #[test]
    fn relayed_entries_say_so_and_older_entries_never_do() {
        // An entry without `x` (every older node's) is the sender's own; `x` of another shape
        // is ignored, never a reason to lose the listing.
        let entry = |x: Option<Value>| {
            let mut fields = vec![("g", Value::B(vec![1])), ("p", Value::A(vec![])), ("r", Value::A(vec![]))];
            if let Some(x) = x {
                fields.push(("x", x));
            }
            Value::map([("t", Value::T("catalog".into())), ("entries", Value::A(vec![Value::map(fields)]))]).encode()
        };
        let relayed = |bytes: Vec<u8>| match Msg::decode(&bytes).unwrap() {
            Msg::Catalog { entries } => entries[0].relayed,
            other => panic!("not a catalog: {other:?}"),
        };
        assert!(!relayed(entry(None)));
        assert!(relayed(entry(Some(Value::U(1)))));
        assert!(!relayed(entry(Some(Value::T("yes".into())))));
        assert!(!relayed(entry(Some(Value::U(7)))));
        // An age is read only on a relayed entry, and never older than the cap.
        let aged = |x: Option<Value>, a: u64| {
            let mut fields = vec![("g", Value::B(vec![1])), ("p", Value::A(vec![])), ("r", Value::A(vec![])), ("a", Value::U(a))];
            if let Some(x) = x {
                fields.push(("x", x));
            }
            match Msg::decode(&Value::map([("t", Value::T("catalog".into())), ("entries", Value::A(vec![Value::map(fields)]))]).encode()).unwrap() {
                Msg::Catalog { entries } => entries[0].age,
                other => panic!("not a catalog: {other:?}"),
            }
        };
        assert_eq!(aged(None, 60), None);
        assert_eq!(aged(Some(Value::U(1)), 60), Some(60));
        assert_eq!(aged(Some(Value::U(1)), u64::MAX), Some(MAX_LISTING_AGE));
    }

    #[test]
    fn oversized_lists_are_refused() {
        assert!(Msg::decode(&Msg::Want { ids: vec![Id([1; 32]); MAX_WANT + 1] }.encode()).is_err());
        assert!(Msg::decode(&Msg::BWant { ids: vec![Id([1; 32]); MAX_BWANT + 1] }.encode()).is_err());
        assert!(Msg::decode(&Msg::Recs { recs: vec![vec![0]; MAX_RECS + 1] }.encode()).is_err());
        let mut hello = bare();
        if let Msg::Hello { communities, .. } = &mut hello {
            *communities = vec![Id::ZERO; MAX_HELLO + 1];
        }
        assert!(Msg::decode(&hello.encode()).is_err());
        let mut hello = bare();
        if let Msg::Hello { salt, hashed, .. } = &mut hello {
            *salt = Some([1; 16]);
            *hashed = vec![Id::ZERO; MAX_HELLO];
        }
        assert!(Msg::decode(&hello.encode()).is_ok());
        if let Msg::Hello { hashed, .. } = &mut hello {
            hashed.push(Id::ZERO);
        }
        assert!(Msg::decode(&hello.encode()).is_err());
    }

    #[test]
    fn salted_hashes_need_their_salt_and_match_only_the_key_they_were_made_from() {
        let key = Id([7; 32]);
        let a = community_hash(&[1; 16], &key);
        assert_eq!(a, community_hash(&[1; 16], &key));
        assert_ne!(a, community_hash(&[2; 16], &key), "a fresh salt per connection: no list is recognisable twice");
        assert_ne!(a, community_hash(&[1; 16], &Id([8; 32])));
        assert_ne!(a, Id::hash(&[[1u8; 16].as_slice(), key.as_bytes()].concat()), "domain separated from plain BLAKE3");
        let fields = |extra: Vec<(&'static str, Value)>| {
            let mut e = vec![("t", Value::T("hello".into())), ("v", Value::U(SYNC_VERSION)), ("node", Value::B(vec![0; 32])), ("c", Value::A(vec![]))];
            e.extend(extra);
            Value::map(e).encode()
        };
        assert!(Msg::decode(&fields(vec![("ch", Value::A(vec![Value::B(a.0.to_vec())]))])).is_err(), "hashes without a salt");
        assert!(Msg::decode(&fields(vec![("n", Value::B(vec![1; 15])), ("ch", Value::A(vec![]))])).is_err());
        assert!(Msg::decode(&fields(vec![("n", Value::T("salt".into()))])).is_err());
        assert!(Msg::decode(&fields(vec![("lk", Value::B(vec![1; 31]))])).is_err());
        assert!(Msg::decode(&fields(vec![("pt", Value::T("p".repeat(257)))])).is_err());
        let salted = Msg::decode(&fields(vec![("n", Value::B(vec![1; 16])), ("ch", Value::A(vec![Value::B(a.0.to_vec())]))])).unwrap();
        let Msg::Hello { salt, hashed, .. } = salted else { panic!("a hello") };
        assert_eq!((salt, hashed), (Some([1; 16]), vec![a]));
    }

    #[test]
    fn catalogs_are_bounded() {
        let entry = CatalogEntry { genesis: vec![1], peers: vec![], relays: vec![], warmth: None, relayed: false, age: None };
        assert!(Msg::decode(&Msg::Catalog { entries: vec![entry.clone(); MAX_CATALOG_ENTRIES + 1] }.encode()).is_err());
        assert!(Msg::decode(&Msg::Catalog { entries: vec![CatalogEntry { genesis: vec![1; 64 * 1024 + 1], ..entry.clone() }] }.encode()).is_err());
        assert!(Msg::decode(&Msg::Catalog { entries: vec![CatalogEntry { genesis: vec![1; 64 * 1024], ..entry.clone() }; 4] }.encode()).is_err());
        assert!(Msg::decode(&Msg::Catalog { entries: vec![CatalogEntry { peers: vec!["x".repeat(129)], ..entry }] }.encode()).is_err());
    }

    #[test]
    fn route_hint_extension_is_optional_and_bounded() {
        let mut hello = bare();
        if let Msg::Hello { node, communities, hints, .. } = &mut hello {
            *node = Id([1; 32]);
            *communities = vec![Id([2; 32])];
            *hints = vec![Hints { community: Id([2; 32]), peers: vec![Id([3; 32]).hex()], relays: vec!["https://relay.example".into()] }];
        }
        assert_eq!(Msg::decode(&hello.encode()).unwrap(), hello);
        if let Msg::Hello { hints, .. } = &mut hello {
            hints[0].peers = vec!["p".into(); MAX_HINT_PEERS + 1];
        }
        assert!(Msg::decode(&hello.encode()).is_err());
        if let Msg::Hello { hints, .. } = &mut hello {
            hints[0].peers = vec!["p".repeat(129)];
        }
        assert!(Msg::decode(&hello.encode()).is_err());
        if let Msg::Hello { hints, .. } = &mut hello {
            hints.clear();
        }
        assert_eq!(Msg::decode(&hello.encode()).unwrap(), hello);
    }

    #[test]
    fn peer_control_text_is_bounded() {
        let mut hello = bare();
        if let Msg::Hello { name, .. } = &mut hello {
            *name = Some("n".repeat(256));
        }
        assert!(Msg::decode(&hello.encode()).is_ok());
        if let Msg::Hello { name, .. } = &mut hello {
            *name = Some("n".repeat(257));
        }
        assert!(Msg::decode(&hello.encode()).is_err());
        if let Msg::Hello { name, token, .. } = &mut hello {
            *name = None;
            *token = Some("t".repeat(257));
        }
        assert!(Msg::decode(&hello.encode()).is_err());
        assert!(Msg::decode(&Msg::Bye { reason: "x".repeat(1025) }.encode()).is_err());
    }

    #[test]
    fn routes_features_and_link_are_bounded() {
        let hello = |r: Vec<String>, f: Vec<String>| {
            let mut h = bare();
            if let Msg::Hello { routes, features, .. } = &mut h {
                *routes = r;
                *features = f;
            }
            h.encode()
        };
        assert!(Msg::decode(&hello(vec!["r".into(); MAX_ROUTES], vec![])).is_ok());
        assert!(Msg::decode(&hello(vec!["r".into(); MAX_ROUTES + 1], vec![])).is_err());
        assert!(Msg::decode(&hello(vec!["r".repeat(129)], vec![])).is_err());
        assert!(Msg::decode(&hello(vec![], vec!["f".repeat(17)])).is_err());
        assert!(Msg::decode(&Msg::Link { mailbox: [0; 16], sealed: vec![0; MAX_LINK + 1] }.encode()).is_err());
        assert!(Msg::decode(&Msg::Link { mailbox: [0; 16], sealed: vec![] }.encode()).is_err());
        let short = Value::map([("t", Value::T("link".into())), ("m", Value::B(vec![1; 15])), ("b", Value::B(vec![1]))]).encode();
        assert!(Msg::decode(&short).is_err());
    }

    #[test]
    fn missing_and_batches() {
        let have = [Id([1; 32])];
        assert_eq!(missing(&[Id([1; 32]), Id([2; 32]), Id([2; 32])], |i| have.contains(i)), vec![Id([2; 32])]);
        assert_eq!(batches(vec![vec![0; 10]; 450]).len(), 3);
    }

    #[test]
    fn holder_keys_travel_in_hellos_and_older_peers_ignore_them() {
        let mut h = bare();
        let key = HolderKey { xk: [7; 32], expires_at: 1_800_000_000, sig: [9; 64], rtc: None };
        if let Msg::Hello { holder, .. } = &mut h {
            *holder = Some(key);
        }
        let bytes = h.encode();
        assert_eq!(Msg::decode(&bytes).unwrap(), h);
        // with the WebRTC binding a browser checks it by
        let bound = HolderKey { rtc: Some(RtcBinding { endpoint: Id([3; 32]), cert: [4; 32], sig: [5; 64] }), ..key };
        if let Msg::Hello { holder, .. } = &mut h {
            *holder = Some(bound);
        }
        assert_eq!(Msg::decode(&h.encode()).unwrap(), h);
        // a binding this release can't read is no binding; the key still counts
        let Value::M(mut e) = cbor::decode(&bare().encode()).unwrap() else { unreachable!() };
        e.push((
            "sk".into(),
            Value::map([("k", Value::B(vec![7; 32])), ("e", Value::U(1_800_000_000)), ("s", Value::B(vec![9; 64])), ("r", Value::B(vec![1]))]),
        ));
        match Msg::decode(&Value::map(e).encode()).unwrap() {
            Msg::Hello { holder, .. } => assert_eq!(holder, Some(key)),
            other => panic!("{other:?}"),
        }
        // the key rides in its own field, so a hello without it decodes as before
        assert_eq!(Msg::decode(&bare().encode()).unwrap(), bare());
        // a malformed key refuses the hello instead of being half read
        let Value::M(mut e) = cbor::decode(&bare().encode()).unwrap() else { unreachable!() };
        e.push(("sk".into(), Value::map([("k", Value::B(vec![7; 31])), ("e", Value::U(1)), ("s", Value::B(vec![9; 64]))])));
        assert!(Msg::decode(&Value::map(e).encode()).is_err());
    }

    #[test]
    fn piece_messages() {
        for op in [PieceOp::Put, PieceOp::Get, PieceOp::Has, PieceOp::Drop, PieceOp::Stored, PieceOp::Got, PieceOp::Had, PieceOp::Dropped, PieceOp::Gone] {
            assert_eq!(PieceOp::parse(op.as_str()), Some(op));
            let m = Msg::Piece { op, rid: 42, to: None, env: vec![1; 256] };
            assert_eq!(Msg::decode(&m.encode()).unwrap(), m);
        }
        assert!(PieceOp::Put.is_request() && !PieceOp::Got.is_request() && !PieceOp::Gone.is_request());
        let forwarded = Msg::Piece { op: PieceOp::Get, rid: u32::MAX, to: Some(Id([3; 32])), env: vec![2; 256] };
        assert_eq!(Msg::decode(&forwarded.encode()).unwrap(), forwarded);
        assert!(Msg::decode(&Msg::Piece { op: PieceOp::Put, rid: 1, to: None, env: vec![] }.encode()).is_err());
        assert!(Msg::decode(&Msg::Piece { op: PieceOp::Put, rid: 1, to: None, env: vec![0; MAX_ENVELOPE + 1] }.encode()).is_err());
        let unknown = Value::map([("t", Value::T("piece".into())), ("o", Value::T("list".into())), ("r", Value::U(1)), ("e", Value::B(vec![1]))]).encode();
        assert!(Msg::decode(&unknown).is_err(), "holders answer only exact requests, never a listing");
    }

    #[test]
    fn rebuild_messages_are_bounded() {
        for m in
            [Msg::Rebuild { rid: 7, manifest: vec![1; 300] }, Msg::Rebuilt { rid: 7, content: Some(vec![2; 5000]) }, Msg::Rebuilt { rid: 8, content: None }]
        {
            assert_eq!(Msg::decode(&m.encode()).unwrap(), m);
        }
        assert!(Msg::decode(&Msg::Rebuild { rid: 1, manifest: vec![] }.encode()).is_err());
        assert!(Msg::decode(&Msg::Rebuild { rid: 1, manifest: vec![0; MAX_REBUILD_MANIFEST + 1] }.encode()).is_err());
        assert!(Msg::decode(&Msg::Rebuilt { rid: 1, content: Some(vec![0; MAX_REBUILT + 1]) }.encode()).is_err());
    }

    #[test]
    fn forwarding_is_said_in_the_hello_and_older_hellos_mean_no() {
        let mut h = bare();
        assert!(!bare().encode().windows(2).any(|w| w == b"fw"), "written only when true");
        if let Msg::Hello { forwards, .. } = &mut h {
            *forwards = true;
        }
        assert_eq!(Msg::decode(&h.encode()).unwrap(), h);
        match Msg::decode(&bare().encode()).unwrap() {
            Msg::Hello { forwards, .. } => assert!(!forwards),
            _ => unreachable!(),
        }
    }

    #[test]
    fn up_to_sixteen_features_are_read() {
        let mut h = bare();
        if let Msg::Hello { features, .. } = &mut h {
            *features = (0..MAX_FEATURES).map(|i| format!("f{i}")).collect();
        }
        assert!(Msg::decode(&h.encode()).is_ok());
        if let Msg::Hello { features, .. } = &mut h {
            features.push("one-more".into());
        }
        assert!(Msg::decode(&h.encode()).is_err());
    }

    #[test]
    fn range_messages_must_partition_the_whole_order() {
        let msg = |entries: Vec<range::Entry>| Msg::Range { community: Id([3; 32]), count: 1, starter: true, entries }.encode();
        let at = |n: u64, range: range::Range| range::Entry { upper: range::Bound { at: n, id: Id([0; 32]) }, range };
        assert!(Msg::decode(&msg(vec![at(5, range::Range::Skip), range::Entry { upper: range::Bound::END, range: range::Range::Skip }])).is_ok());
        // ends short of the end
        assert!(Msg::decode(&msg(vec![at(5, range::Range::Skip)])).is_err());
        // out of order, or no ranges at all
        assert!(
            Msg::decode(&msg(vec![at(5, range::Range::Skip), at(5, range::Range::Skip), range::Entry { upper: range::Bound::END, range: range::Range::Skip }]))
                .is_err()
        );
        assert!(Msg::decode(&msg(vec![])).is_err());
        // more ids in all than a have list may carry
        let half = range::Range::Ids(vec![Id([1; 32]); MAX_HAVE / 2 + 1]);
        assert!(Msg::decode(&msg(vec![at(5, half.clone()), range::Entry { upper: range::Bound::END, range: half }])).is_err());
    }
}
