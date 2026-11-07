//! `hoi://` links (PROTOCOL.md §10): invites and home-node pairing.
//!
//! `hoi://join?code=<base64url obfuscated envelope>` packs the community, routes and signed
//! genesis into one code. This is public, reversible obfuscation, not encryption. Plain CBOR
//! codes and legacy `c/r/p/i/g` queries still open. The genesis lets a newcomer check the
//! community before any peer answers.
//! The same query can follow `#/join?` in an https link to the web app.
//!
//! `hoi://node?code=<base64url obfuscated envelope>` is a public node's address, packed and
//! masked the same way, so its IP addresses are not readable from the link either. Plain
//! `w/p/nt/ns` queries still open.
//!
//! `hoi://pair?code=<base64url obfuscated envelope>` pairs a browser or phone with a home node: its
//! address, token, name and WebRTC routes (base64url DTLS fingerprint `@` UDP socket address, which
//! a phone on the same network can use without a certificate) packed and masked like a node link,
//! so no address shows in the link. The same query can follow `#/pair?` in an https link to the
//! web app, so a phone's camera opens it directly. Plain `u/t/n/w` queries still open.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::cbor::{self, Value};
use crate::record::Fields;
use crate::{Id, Sig, TypesError};

const ZB32: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";
/// Magic, then a 16-byte checksum, then the masked payload.
const CODE_HEADER: usize = 4 + 16;
const INVITE_MAGIC: &[u8; 4] = b"HOI\x02";
const INVITE_MASK_CONTEXT: &str = "hoi invite obfuscation v2";
const NODE_MAGIC: &[u8; 4] = b"HON\x01";
const PAIR_MAGIC: &[u8; 4] = b"HOP\x01";
const PAIR_MASK_CONTEXT: &str = "hoi pairing link obfuscation v1";
/// The longest pairing link code: an address, a token, a name and four routes.
const MAX_PAIR_CODE: usize = 4096;
const NODE_MASK_CONTEXT: &str = "hoi node link obfuscation v1";
/// The longest node link code: routes and a notice at most fit well inside it.
const MAX_NODE_CODE: usize = 16 * 1024;

/// A public mask, not a secret key: a generic Base64 decoder no longer exposes addresses,
/// but anyone with the link and this algorithm can undo it. The checksum detects damage;
/// it does not authenticate the link. Signed genesis and notice checks happen separately.
fn mask(context: &str, payload: &mut [u8], checksum: &[u8]) {
    let mut hasher = blake3::Hasher::new_derive_key(context);
    hasher.update(checksum);
    let mut stream = hasher.finalize_xof();
    let mut mask = [0u8; 64];
    for chunk in payload.chunks_mut(mask.len()) {
        stream.fill(&mut mask[..chunk.len()]);
        for (byte, mask_byte) in chunk.iter_mut().zip(mask.iter()) {
            *byte ^= mask_byte;
        }
    }
}

fn encode_code(magic: &[u8; 4], context: &str, mut payload: Vec<u8>) -> String {
    let digest = blake3::hash(&payload);
    let checksum = &digest.as_bytes()[..16];
    mask(context, &mut payload, checksum);
    let mut envelope = Vec::with_capacity(CODE_HEADER + payload.len());
    envelope.extend_from_slice(magic);
    envelope.extend_from_slice(checksum);
    envelope.extend_from_slice(&payload);
    b64(&envelope)
}

/// Undoes [`encode_code`] for an envelope that starts with `magic`.
fn open_code(mut envelope: Vec<u8>, context: &str, what: &str) -> Result<Vec<u8>, TypesError> {
    if envelope.len() <= CODE_HEADER {
        return Err(TypesError::Uri(format!("{what} code is truncated")));
    }
    let (header, payload) = envelope.split_at_mut(CODE_HEADER);
    mask(context, payload, &header[4..]);
    if blake3::hash(payload).as_bytes()[..16] != header[4..] {
        return Err(TypesError::Uri(format!("{what} code is damaged")));
    }
    Ok(payload.to_vec())
}

fn encode_invite_code(payload: Vec<u8>) -> String {
    encode_code(INVITE_MAGIC, INVITE_MASK_CONTEXT, payload)
}

fn decode_invite_code(code: &str) -> Result<Vec<u8>, TypesError> {
    let envelope = unb64(code)?;
    if !envelope.starts_with(b"HOI") {
        return Ok(envelope); // Original, unobfuscated CBOR code.
    }
    if !envelope.starts_with(INVITE_MAGIC) {
        return Err(TypesError::Uri("unsupported invite code version".into()));
    }
    open_code(envelope, INVITE_MASK_CONTEXT, "invite")
}

pub fn zbase32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 8 / 5 + 1);
    let mut buf: u32 = 0;
    let mut bits = 0;
    for &b in data {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ZB32[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ZB32[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn zbase32_decode(s: &str) -> Result<Vec<u8>, TypesError> {
    let mut out = Vec::with_capacity(s.len() * 5 / 8);
    let mut buf: u32 = 0;
    let mut bits = 0;
    for c in s.bytes() {
        let v = ZB32.iter().position(|&x| x == c.to_ascii_lowercase()).ok_or(TypesError::Uri("bad z-base32".into()))? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

pub fn b64(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

pub fn unb64(s: &str) -> Result<Vec<u8>, TypesError> {
    URL_SAFE_NO_PAD.decode(s.trim().trim_end_matches('=')).map_err(|_| TypesError::Uri("bad base64".into()))
}

/// How links were written in the clear; they still open, and tests write them.
#[cfg(test)]
fn pct_encode(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

/// Like [`pct_encode`], but keeps `:` and `@` (allowed in a query).
#[cfg(test)]
fn pct_encode_route(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'@' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

fn pct_decode(s: &str) -> Result<String, TypesError> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).map_err(|_| TypesError::Uri("bad escape".into()))?;
            out.push(u8::from_str_radix(h, 16).map_err(|_| TypesError::Uri("bad escape".into()))?);
            i += 3;
        } else if b[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| TypesError::Uri("bad utf-8".into()))
}

/// Finds `<verb>?` in a hoi:// URI or an https link with `#/<verb>?` and returns its query pairs.
fn query_after(link: &str, verb: &str) -> Result<Vec<(String, String)>, TypesError> {
    let link = link.trim();
    let needle = format!("{verb}?");
    let start = link.find(&needle).ok_or_else(|| TypesError::Uri(format!("not a hoi {verb} link")))?;
    let q = &link[start + needle.len()..];
    let q = q.split('#').next().unwrap_or("");
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            Ok((k.to_string(), pct_decode(v)?))
        })
        .collect()
}

/// Trackers a community names at most.
pub const MAX_TRACKERS: usize = 2;
/// The longest tracker address.
pub const MAX_TRACKER_LEN: usize = 200;

/// Whether `s` can be a tracker: an https address of a registry (plain http only on this
/// machine, for tests), without credentials, query or fragment, at most [`MAX_TRACKER_LEN`] bytes.
pub fn tracker_ok(s: &str) -> bool {
    let rest = match s.strip_prefix("https://") {
        Some(rest) => rest,
        None => match s.strip_prefix("http://") {
            Some(rest) if ["127.0.0.1:", "localhost:", "[::1]:"].iter().any(|h| rest.starts_with(h)) => rest,
            _ => return false,
        },
    };
    let host = rest.split('/').next().unwrap_or("");
    s.len() <= MAX_TRACKER_LEN
        && !host.is_empty()
        && !host.starts_with(':')
        && !s.contains(['?', '#', '@', ' ', '\\', '\n', '\r', '\t'])
        && s.chars().all(|c| c.is_ascii_graphic())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Invite {
    pub community: Id,
    pub relays: Vec<String>,
    pub peers: Vec<String>,
    pub invite: Option<Id>,
    /// Wire bytes of the genesis record.
    pub genesis: Option<Vec<u8>>,
    /// Trackers the community names (`t`): asked when no route here answers.
    pub trackers: Vec<String>,
}

impl Invite {
    fn query(&self) -> String {
        // Keep keys and genesis as bytes: unlike encoding the old query as a whole, this
        // avoids double-base64 overhead and percent escapes in QR codes.
        let mut fields = vec![
            ("v", Value::U(1)),
            ("c", Value::B(self.community.as_bytes().to_vec())),
            ("r", Value::A(self.relays.iter().cloned().map(Value::T).collect())),
            ("p", Value::A(self.peers.iter().cloned().map(Value::T).collect())),
        ];
        if let Some(i) = &self.invite {
            fields.push(("i", Value::B(i.as_bytes().to_vec())));
        }
        if let Some(g) = &self.genesis {
            fields.push(("g", Value::B(g.clone())));
        }
        if !self.trackers.is_empty() {
            fields.push(("t", Value::A(self.trackers.iter().cloned().map(Value::T).collect())));
        }
        format!("code={}", encode_invite_code(Value::map(fields).encode()))
    }

    pub fn to_uri(&self) -> String {
        format!("hoi://join?{}", self.query())
    }

    /// `base` is the web app's address, e.g. `https://hoi.sh/app/`.
    pub fn to_web(&self, base: &str) -> String {
        format!("{}#/join?{}", base.split('#').next().unwrap_or(base), self.query())
    }

    pub fn parse(link: &str) -> Result<Invite, TypesError> {
        let query = query_after(link, "join")?;
        if let Some((_, code)) = query.iter().find(|(k, _)| k == "code") {
            // Do not let extra clear-text fields override what someone previewed in a code.
            if query.iter().filter(|(k, _)| k == "code").count() != 1 || query.iter().any(|(k, _)| matches!(k.as_str(), "c" | "r" | "p" | "i" | "g")) {
                return Err(TypesError::Uri("invite mixes encoded and legacy fields".into()));
            }
            if code.len() > 128 * 1024 {
                return Err(TypesError::Uri("invite code is too long".into()));
            }
            let mut f = Fields::new(cbor::decode(&decode_invite_code(code)?)?, "invite")?;
            if f.u64("v")? != 1 {
                return Err(TypesError::Uri("unsupported invite code version".into()));
            }
            let inv = Invite {
                community: f.id("c")?,
                relays: f.texts("r")?,
                peers: f.texts("p")?,
                invite: f.opt_id("i")?,
                genesis: match f.opt("g") {
                    None => None,
                    Some(Value::B(g)) => Some(g),
                    Some(_) => return Err(TypesError::Uri("invite genesis must be bytes".into())),
                },
                trackers: match f.opt("t") {
                    None => Vec::new(),
                    Some(Value::A(v)) if !v.is_empty() && v.len() <= MAX_TRACKERS => v
                        .into_iter()
                        .map(|t| match t {
                            Value::T(s) if tracker_ok(&s) => Ok(s),
                            _ => Err(TypesError::Uri("an invite's tracker is an https address".into())),
                        })
                        .collect::<Result<_, _>>()?,
                    Some(_) => return Err(TypesError::Uri("an invite names one or two trackers, or none".into())),
                },
            };
            f.done()?;
            return Ok(inv);
        }
        let mut inv = Invite::default();
        let mut have_c = false;
        for (k, v) in query {
            match k.as_str() {
                "c" => {
                    inv.community = Id::from_slice(&zbase32_decode(&v)?)?;
                    have_c = true;
                }
                "r" => inv.relays = v.split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
                "p" => inv.peers = v.split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
                "i" => inv.invite = Some(Id::from_slice(&zbase32_decode(&v)?)?),
                "g" => inv.genesis = Some(unb64(&v)?),
                _ => {} // forward compatible: ignore unknown parameters
            }
        }
        if !have_c {
            return Err(TypesError::Uri("invite has no community key".into()));
        }
        Ok(inv)
    }
}

/// A pairing link's address, token, name and routes, before they are checked.
type PairFields = (Option<String>, Option<String>, Option<String>, Vec<String>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// WebSocket address; empty when the node is reachable only over WebRTC.
    pub url: String,
    pub token: String,
    pub name: Option<String>,
    /// WebRTC routes as `rtc:<fingerprint>@<socket address>` hints.
    pub rtc: Vec<String>,
}

/// Routes per pairing link: a QR code has to stay scannable.
pub const MAX_PAIR_ROUTES: usize = 4;

impl Pair {
    /// What the code packs, before it is deflated and masked: `{v: 1, u, t, w: [<fp>@<addr>], n?}`.
    fn payload(&self) -> Vec<u8> {
        let routes = self.rtc.iter().filter_map(|r| r.strip_prefix("rtc:")).take(MAX_PAIR_ROUTES).map(|r| Value::T(r.into()));
        let mut fields = vec![("v", Value::U(1)), ("u", Value::T(self.url.clone())), ("t", Value::T(self.token.clone())), ("w", Value::A(routes.collect()))];
        if let Some(n) = &self.name {
            fields.push(("n", Value::T(n.clone())));
        }
        Value::map(fields).encode()
    }

    fn query(&self) -> String {
        format!("code={}", encode_code(PAIR_MAGIC, PAIR_MASK_CONTEXT, deflate(&self.payload())))
    }

    /// (address, token, name, routes) from a code.
    fn from_code(code: &str) -> Result<PairFields, TypesError> {
        if code.len() > MAX_PAIR_CODE {
            return Err(TypesError::Uri("pairing link code is too long".into()));
        }
        let envelope = unb64(code)?;
        if !envelope.starts_with(PAIR_MAGIC) {
            return Err(TypesError::Uri("unsupported pairing link code".into()));
        }
        let packed = open_code(envelope, PAIR_MASK_CONTEXT, "pairing link")?;
        let payload = inflate(&packed, MAX_PAIR_CODE).map_err(|_| TypesError::Uri("pairing link code is damaged".into()))?;
        let mut f = Fields::new(cbor::decode(&payload)?, "pairing link")?;
        if f.u64("v")? != 1 {
            return Err(TypesError::Uri("unsupported pairing link code version".into()));
        }
        let url = Some(f.text("u")?).filter(|u| !u.is_empty());
        let token = Some(f.text("t")?).filter(|t| !t.is_empty());
        let rtc = f.texts("w")?.into_iter().filter(|s| !s.is_empty()).take(MAX_PAIR_ROUTES).map(|s| format!("rtc:{s}")).collect();
        let name = f.opt_text("n")?;
        f.done()?;
        Ok((url, token, name, rtc))
    }

    pub fn to_uri(&self) -> String {
        format!("hoi://pair?{}", self.query())
    }

    /// `base` is the web app's address, e.g. `https://hoi.sh/app/`: what a phone's camera opens.
    pub fn to_web(&self, base: &str) -> String {
        format!("{}#/pair?{}", base.split('#').next().unwrap_or(base), self.query())
    }

    pub fn parse(link: &str) -> Result<Pair, TypesError> {
        let query = query_after(link, "pair")?;
        let (url, token, name, rtc) = match query.iter().find(|(k, _)| k == "code") {
            Some((_, code)) => {
                // Clear-text fields beside a code could change where it pairs: refused.
                if query.iter().filter(|(k, _)| k == "code").count() != 1 || query.iter().any(|(k, _)| matches!(k.as_str(), "u" | "t" | "n" | "w")) {
                    return Err(TypesError::Uri("pairing link mixes a code and clear-text fields".into()));
                }
                Pair::from_code(code)?
            }
            None => {
                let (mut url, mut token, mut name, mut rtc) = (None, None, None, Vec::new());
                for (k, v) in query {
                    match k.as_str() {
                        "u" => url = Some(v),
                        "t" => token = Some(v),
                        "n" => name = Some(v),
                        "w" => rtc = v.split(',').filter(|s| !s.is_empty()).take(MAX_PAIR_ROUTES).map(|s| format!("rtc:{s}")).collect(),
                        _ => {}
                    }
                }
                (url, token, name, rtc)
            }
        };
        let url = match url {
            Some(u) if u.starts_with("wss://") => u,
            // Plain ws:// carries the pairing token unencrypted: only to this very machine.
            Some(u) if u.starts_with("ws://") && loopback_ws(&u) => u,
            Some(u) if u.starts_with("ws://") => return Err(TypesError::Uri("a home node elsewhere needs wss:// (or its WebRTC routes)".into())),
            Some(_) => return Err(TypesError::Uri("home node address must be ws:// or wss://".into())),
            None if !rtc.is_empty() => String::new(),
            None => return Err(TypesError::Uri("pair link has no address".into())),
        };
        Ok(Pair { url, token: token.ok_or_else(|| TypesError::Uri("pair link has no token".into()))?, name, rtc })
    }
}

/// A public node's address, for anyone to open: no credential, no community. Whoever opens it
/// connects to that node and sees the open communities it carries and the other public nodes it
/// knows (PROTOCOL.md §14.2). `hoi://node?code=<code>`, or the same query after `#/node?` in the
/// web app's address.
///
/// The code is a DAG-CBOR map `{v: 1, w: [<fp>@<addr>, …], p: [<endpoint>@<addr>, …], n: <notice>,
/// s: <signature>}`, deflated and masked like an invite's code, so the node's IP addresses are
/// not readable from the link, not even after a Base64 decoder. A node with an operator notice
/// (crate::notice) adds `n` (its canonical bytes) and `s` (the endpoint key's signature
/// over [`crate::notice::statement`]), so whoever opens the link reads the notice, and checks
/// that the node named in `p` stands behind it, before connecting.
///
/// Earlier links spelled the same in the clear, `w=…&p=…&nt=<deflated notice>&ns=<signature>`,
/// and still open.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NodeLink {
    /// WebRTC routes (`rtc:<fingerprint>@<socket address>`), for browsers and phones.
    pub rtc: Vec<String>,
    /// Native routes (`<endpoint>@<socket address>`), for desktop apps and headless nodes.
    pub peers: Vec<String>,
    /// The operator's notice, as its canonical bytes (`NodeNotice::canonical`).
    pub notice: Option<Vec<u8>>,
    /// The node's endpoint key's signature binding the notice to this node.
    pub signature: Option<Sig>,
}

/// Deflate for a notice in a link: raw DEFLATE, best compression; English prose halves.
pub fn deflate(bytes: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(bytes, 10)
}

/// Inflates what [`deflate`] wrote, refusing anything that would grow past `limit` bytes.
pub fn inflate(bytes: &[u8], limit: usize) -> Result<Vec<u8>, TypesError> {
    miniz_oxide::inflate::decompress_to_vec_with_limit(bytes, limit).map_err(|_| TypesError::Uri("notice in the link is damaged or too long".into()))
}

impl NodeLink {
    /// The endpoint key the link names, from its first native route: what the notice's signature
    /// is checked against.
    pub fn endpoint(&self) -> Option<Id> {
        self.peers.iter().find_map(|p| Id::from_hex(p.split_once('@').map_or(p.as_str(), |(id, _)| id)).ok())
    }

    /// What the code packs, before it is deflated and masked.
    fn payload(&self) -> Vec<u8> {
        let rtc = self.rtc.iter().filter_map(|r| r.strip_prefix("rtc:")).take(MAX_PAIR_ROUTES).map(|r| Value::T(r.into()));
        let peers = self.peers.iter().filter(|p| !p.starts_with("rtc:")).take(MAX_PAIR_ROUTES).map(|p| Value::T(p.clone()));
        let mut fields = vec![("v", Value::U(1)), ("w", Value::A(rtc.collect())), ("p", Value::A(peers.collect()))];
        if let Some(n) = &self.notice {
            fields.push(("n", Value::B(n.clone())));
            if let Some(s) = &self.signature {
                fields.push(("s", Value::B(s.0.to_vec())));
            }
        }
        Value::map(fields).encode()
    }

    fn query(&self) -> String {
        format!("code={}", encode_code(NODE_MAGIC, NODE_MASK_CONTEXT, deflate(&self.payload())))
    }

    pub fn to_uri(&self) -> String {
        format!("hoi://node?{}", self.query())
    }

    /// `base` is the web app's address, e.g. `https://hoi.sh/app/`.
    pub fn to_web(&self, base: &str) -> String {
        format!("{}#/node?{}", base.split('#').next().unwrap_or(base), self.query())
    }

    pub fn parse(link: &str) -> Result<NodeLink, TypesError> {
        let query = query_after(link, "node")?;
        let out = match query.iter().find(|(k, _)| k == "code") {
            Some((_, code)) => {
                // Clear-text fields beside a code could change what someone previewed: refused.
                if query.iter().filter(|(k, _)| k == "code").count() != 1 || query.iter().any(|(k, _)| matches!(k.as_str(), "w" | "p" | "nt" | "ns")) {
                    return Err(TypesError::Uri("node link mixes a code and clear-text fields".into()));
                }
                NodeLink::from_code(code)?
            }
            None => NodeLink::from_query(query)?,
        };
        if out.rtc.is_empty() && out.peers.is_empty() {
            return Err(TypesError::Uri("node link has no address".into()));
        }
        if out.notice.is_some() && out.signature.is_none() {
            return Err(TypesError::Uri("node link carries a notice without its signature".into()));
        }
        Ok(out)
    }

    fn from_code(code: &str) -> Result<NodeLink, TypesError> {
        if code.len() > MAX_NODE_CODE {
            return Err(TypesError::Uri("node link code is too long".into()));
        }
        let envelope = unb64(code)?;
        if !envelope.starts_with(NODE_MAGIC) {
            return Err(TypesError::Uri("unsupported node link code".into()));
        }
        let packed = open_code(envelope, NODE_MASK_CONTEXT, "node link")?;
        // routes, a notice and its signature: well under this once inflated
        let payload = inflate(&packed, crate::notice::MAX_NOTICE_WIRE + 4096).map_err(|_| TypesError::Uri("node link code is damaged".into()))?;
        let mut f = Fields::new(cbor::decode(&payload)?, "node link")?;
        if f.u64("v")? != 1 {
            return Err(TypesError::Uri("unsupported node link code version".into()));
        }
        let rtc = f.texts("w")?.into_iter().filter(|s| !s.is_empty()).take(MAX_PAIR_ROUTES).map(|s| format!("rtc:{s}")).collect();
        let peers = f.texts("p")?.into_iter().filter(|s| !s.is_empty() && !s.starts_with("rtc:")).take(MAX_PAIR_ROUTES).collect();
        let notice = match f.opt("n") {
            None => None,
            Some(Value::B(n)) if n.len() <= crate::notice::MAX_NOTICE_WIRE => Some(n),
            Some(Value::B(_)) => return Err(TypesError::Uri("notice in the link is too long".into())),
            Some(_) => return Err(TypesError::Uri("notice in the link must be bytes".into())),
        };
        let signature = match f.opt("s") {
            None => None,
            Some(Value::B(s)) => Some(Sig(s.try_into().map_err(|_| TypesError::Uri("notice signature must be 64 bytes".into()))?)),
            Some(_) => return Err(TypesError::Uri("notice signature must be bytes".into())),
        };
        if signature.is_some() && notice.is_none() {
            return Err(TypesError::Uri("node link carries a signature without a notice".into()));
        }
        f.done()?;
        Ok(NodeLink { rtc, peers, notice, signature })
    }

    /// An earlier link, in the clear: `w=…&p=…&nt=…&ns=…`.
    fn from_query(query: Vec<(String, String)>) -> Result<NodeLink, TypesError> {
        let mut out = NodeLink::default();
        for (k, v) in query {
            match k.as_str() {
                "w" => out.rtc = v.split(',').filter(|s| !s.is_empty()).take(MAX_PAIR_ROUTES).map(|s| format!("rtc:{s}")).collect(),
                "p" => out.peers = v.split(',').filter(|s| !s.is_empty() && !s.starts_with("rtc:")).take(MAX_PAIR_ROUTES).map(String::from).collect(),
                "nt" => {
                    let packed = unb64(&v)?;
                    if packed.len() > crate::notice::MAX_NOTICE_WIRE {
                        return Err(TypesError::Uri("notice in the link is too long".into()));
                    }
                    out.notice = Some(inflate(&packed, crate::notice::MAX_NOTICE_WIRE)?);
                }
                "ns" => {
                    let bytes = unb64(&v)?;
                    let sig: [u8; 64] = bytes.try_into().map_err(|_| TypesError::Uri("notice signature must be 64 bytes".into()))?;
                    out.signature = Some(Sig(sig));
                }
                _ => {}
            }
        }
        Ok(out)
    }
}

/// Whether a `ws://` address names this machine: `localhost`, `127.0.0.0/8` or `[::1]`, with
/// nothing before the host that could smuggle another one in (`ws://localhost@evil/`).
fn loopback_ws(url: &str) -> bool {
    let rest = &url["ws://".len()..];
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(v6) = authority.strip_prefix('[') { v6.split(']').next().unwrap_or("") } else { authority.split(':').next().unwrap_or("") };
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
        || host.parse::<std::net::Ipv6Addr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_websocket_pairing_stays_on_this_machine() {
        for ok in ["ws://127.0.0.1:7842", "ws://localhost:7842/x", "ws://[::1]:7842", "ws://127.5.6.7"] {
            assert!(Pair::parse(&format!("hoi://pair?u={}&t=tok", pct_encode(ok))).is_ok(), "{ok}");
        }
        for bad in ["ws://192.168.1.5:7842", "ws://home.example:7842", "ws://localhost@evil.example/", "ws://[fe80::1]:7842", "ws://127.0.0.1.evil.example/"] {
            assert!(Pair::parse(&format!("hoi://pair?u={}&t=tok", pct_encode(bad))).is_err(), "{bad}");
        }
        assert!(Pair::parse(&format!("hoi://pair?u={}&t=tok", pct_encode("wss://home.example:7842"))).is_ok());
    }

    #[test]
    fn zbase32_roundtrip() {
        for n in 0..40 {
            let d: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(zbase32_decode(&zbase32_encode(&d)).unwrap()[..n], d[..]);
        }
        assert_eq!(zbase32_encode(&[0xf0, 0xbf, 0xc7]), "6n9hq");
    }

    #[test]
    fn trackers_are_https_addresses_and_two_at_most() {
        for good in ["https://tracker.example", "https://registry.example:8443/registry", "https://[2001:db8::1]/v"] {
            assert!(tracker_ok(good), "{good}");
        }
        assert!(tracker_ok("http://127.0.0.1:4100"), "plain http on this machine, for tests");
        for bad in [
            "http://tracker.example",
            "http://127.0.0.2:80",
            "https://",
            "https://user@host",
            "https://host/?a=1",
            "https://host/#x",
            "https://ho st",
            "ftp://x",
            "https://:443",
        ] {
            assert!(!tracker_ok(bad), "{bad}");
        }
        assert!(!tracker_ok(&format!("https://{}", "a".repeat(MAX_TRACKER_LEN))));
        let three = Invite {
            community: Id([5; 32]),
            trackers: vec!["https://a.example".into(), "https://b.example".into(), "https://c.example".into()],
            ..Default::default()
        };
        assert!(Invite::parse(&three.to_uri()).is_err(), "three trackers");
        let bad = Invite { community: Id([5; 32]), trackers: vec!["http://a.example".into()], ..Default::default() };
        assert!(Invite::parse(&bad.to_uri()).is_err());
    }

    #[test]
    fn invite_roundtrip() {
        let inv = Invite {
            community: Id([5; 32]),
            relays: vec!["https://relay.mira.example:443".into()],
            peers: vec!["abc".into()],
            invite: None,
            genesis: Some(vec![1, 2, 3, 250]),
            trackers: vec!["https://tracker.example".into()],
        };
        assert_eq!(Invite::parse(&inv.to_uri()).unwrap(), inv);
        assert_eq!(Invite::parse(&inv.to_web("https://hoi.sh/app/")).unwrap(), inv);
        assert!(inv.to_web("https://hoi.sh/app/#/x").starts_with("https://hoi.sh/app/#/join?code="));
    }

    #[test]
    fn invite_codes_hide_addresses_without_discarding_them() {
        let inv = Invite {
            community: Id([5; 32]),
            relays: vec!["https://relay.example:443/path?a=1&b=2".into()],
            peers: vec!["rtc:fingerprint@203.0.113.7:7844".into(), "endpoint@[2001:db8::7]:7843".into()],
            invite: Some(Id([9; 32])),
            genesis: Some(vec![0, 1, 2, 255]),
            trackers: Vec::new(),
        };
        for link in [inv.to_uri(), inv.to_web("https://hoi.sh/app/#/old")] {
            assert!(!link.contains("203.0.113.7") && !link.contains("2001:db8") && !link.contains("relay.example"));
            assert_eq!(Invite::parse(&link).unwrap(), inv);
            let code = link.split_once("code=").unwrap().1;
            assert!(code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
            let decoded = unb64(code).unwrap();
            assert!(decoded.starts_with(INVITE_MAGIC));
            for address in inv.relays.iter().chain(&inv.peers) {
                assert!(!decoded.windows(address.len()).any(|w| w == address.as_bytes()), "Base64 alone must not expose addresses");
            }
        }
        let minimal = Invite { community: inv.community, ..Default::default() };
        assert_eq!(Invite::parse(&minimal.to_uri()).unwrap(), minimal);
    }

    #[test]
    fn legacy_invites_still_open() {
        let inv = Invite {
            community: Id([5; 32]),
            relays: vec!["https://relay.example".into()],
            peers: vec!["rtc:fp@203.0.113.7:7844".into(), "endpoint@[2001:db8::7]:7843".into()],
            invite: Some(Id([9; 32])),
            genesis: Some(vec![1, 2, 255]),
            trackers: Vec::new(),
        };
        let q = format!(
            "c={}&r={}&p={}&i={}&g={}",
            zbase32_encode(inv.community.as_bytes()),
            pct_encode(&inv.relays[0]),
            inv.peers.iter().map(|p| pct_encode(p)).collect::<Vec<_>>().join(","),
            zbase32_encode(inv.invite.unwrap().as_bytes()),
            b64(inv.genesis.as_ref().unwrap()),
        );
        for prefix in ["hoi://join?", "https://hoi.sh/app/#/join?"] {
            assert_eq!(Invite::parse(&format!("{prefix}{q}")).unwrap(), inv);
        }
    }

    #[test]
    fn original_cbor_invite_codes_still_open() {
        // Frozen v1 code, independent of the current encoder.
        let code = "pGFjWCAFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBWFwgGFygGF2AQ";
        let expected = Invite { community: Id([5; 32]), ..Default::default() };
        for prefix in ["hoi://join?code=", "https://hoi.sh/app/#/join?code="] {
            assert_eq!(Invite::parse(&format!("{prefix}{code}")).unwrap(), expected);
        }
    }

    #[test]
    fn obfuscated_invites_reject_damage_and_unknown_versions() {
        let inv = Invite { community: Id([5; 32]), genesis: Some(vec![7; 200]), ..Default::default() };
        let link = inv.to_uri();
        let envelope = unb64(link.split_once("code=").unwrap().1).unwrap();
        // Truncation and changes to the version, checksum or any masked payload byte fail.
        for end in 0..envelope.len() {
            assert!(Invite::parse(&format!("hoi://join?code={}", b64(&envelope[..end]))).is_err(), "length {end}");
        }
        for at in 0..envelope.len() {
            let mut damaged = envelope.clone();
            damaged[at] ^= 1;
            assert!(Invite::parse(&format!("hoi://join?code={}", b64(&damaged))).is_err(), "byte {at}");
        }
        let mut extended = envelope;
        extended.push(0);
        assert!(Invite::parse(&format!("hoi://join?code={}", b64(&extended))).is_err());
    }

    #[test]
    fn obfuscated_invites_still_validate_the_cbor_schema() {
        let fields = vec![("v", Value::U(1)), ("c", Value::B(vec![5; 32])), ("r", Value::A(vec![])), ("p", Value::A(vec![]))];
        for (key, bad) in [("v", Value::U(2)), ("c", Value::B(vec![5; 31])), ("r", Value::A(vec![Value::U(1)])), ("p", Value::T("route".into()))] {
            let mut invalid = fields.clone();
            invalid.iter_mut().find(|(k, _)| *k == key).unwrap().1 = bad;
            let code = encode_invite_code(Value::map(invalid).encode());
            assert!(Invite::parse(&format!("hoi://join?code={code}")).is_err());
        }
    }

    #[test]
    fn malformed_or_ambiguous_invite_codes_are_rejected() {
        let good = Invite { community: Id([5; 32]), ..Default::default() }.to_uri();
        for suffix in ["&code=abc", "&c=abc", "&p=rtc:other", "&r=https://other", "&g=AA", "&i=abc"] {
            assert!(Invite::parse(&format!("{good}{suffix}")).is_err());
        }
        for code in ["".into(), "%%%".into(), b64(&[0]), b64(&[0xa1, 0x61, b'v', 2]), "A".repeat(128 * 1024 + 1)] {
            assert!(Invite::parse(&format!("hoi://join?code={code}")).is_err());
        }
        let mut fields = vec![("v", Value::U(1)), ("c", Value::B(vec![5; 32])), ("r", Value::A(vec![])), ("p", Value::A(vec![]))];
        for (key, bad) in [("v", Value::U(2)), ("c", Value::B(vec![5; 31])), ("r", Value::A(vec![Value::U(1)])), ("p", Value::T("route".into()))] {
            let mut invalid = fields.clone();
            invalid.iter_mut().find(|(k, _)| *k == key).unwrap().1 = bad;
            assert!(Invite::parse(&format!("hoi://join?code={}", b64(&Value::map(invalid).encode()))).is_err());
        }
        fields.push(("g", Value::T("not bytes".into())));
        assert!(Invite::parse(&format!("hoi://join?code={}", b64(&Value::map(fields).encode()))).is_err());
    }

    /// A node link as nodes wrote it before codes: the same fields in the clear.
    fn legacy_query(link: &NodeLink) -> String {
        let mut q = Vec::new();
        let rtc: Vec<String> = link.rtc.iter().filter_map(|r| r.strip_prefix("rtc:")).map(pct_encode_route).collect();
        if !rtc.is_empty() {
            q.push(format!("w={}", rtc.join(",")));
        }
        if !link.peers.is_empty() {
            q.push(format!("p={}", link.peers.iter().map(|p| pct_encode_route(p)).collect::<Vec<_>>().join(",")));
        }
        if let Some(n) = &link.notice {
            q.push(format!("nt={}", b64(&deflate(n))));
        }
        if let Some(sig) = &link.signature {
            q.push(format!("ns={}", b64(&sig.0)));
        }
        q.join("&")
    }

    fn code_of(link: &str) -> &str {
        link.split_once("code=").unwrap().1
    }

    #[test]
    fn node_link_roundtrip() {
        let link = NodeLink {
            rtc: vec![format!("rtc:{}@203.0.113.7:7844", "q".repeat(43)), format!("rtc:{}@[2001:db8::7]:7844", "q".repeat(43))],
            peers: vec![format!("{}@203.0.113.7:7843", Id([3; 32]).hex())],
            ..NodeLink::default()
        };
        assert_eq!(NodeLink::parse(&link.to_uri()).unwrap(), link);
        assert_eq!(NodeLink::parse(&link.to_web("https://hoi.sh/app/")).unwrap(), link);
        assert!(link.to_web("https://hoi.sh/app/#/x").starts_with("https://hoi.sh/app/#/node?code="));
        assert!(link.to_uri().starts_with("hoi://node?code="));
        assert!(NodeLink::parse("hoi://node?").is_err(), "a link without an address");
        let many = NodeLink { rtc: (0..9).map(|i| format!("rtc:{}@203.0.113.{i}:7844", "q".repeat(43))).collect(), ..NodeLink::default() };
        assert_eq!(NodeLink::parse(&many.to_uri()).unwrap().rtc.len(), MAX_PAIR_ROUTES, "a link stays scannable");
        let none = NodeLink::default();
        assert!(NodeLink::parse(&none.to_uri()).unwrap_err().to_string().contains("no address"));
    }

    #[test]
    fn node_links_hide_addresses_without_discarding_them() {
        let link = NodeLink {
            rtc: vec![format!("rtc:{}@203.0.113.7:7844", "q".repeat(43)), format!("rtc:{}@[2001:db8::7]:7844", "q".repeat(43))],
            peers: vec![format!("{}@203.0.113.7:7843", Id([3; 32]).hex())],
            ..NodeLink::default()
        };
        for text in [link.to_uri(), link.to_web("https://hoi.sh/app/")] {
            // No address, no key and no credential to read: neither in the link nor after Base64.
            assert!(!text.contains("203.0.113.7") && !text.contains("2001:db8") && !text.contains(&Id([3; 32]).hex()), "{text}");
            assert!(!text.contains("&w=") && !text.contains("&p=") && !text.contains("t=") && !text.contains("c="), "{text}");
            let code = code_of(&text);
            assert!(code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "{code}");
            let decoded = unb64(code).unwrap();
            assert!(decoded.starts_with(NODE_MAGIC));
            for needle in ["203.0.113.7", "2001:db8", "7844", "7843", &"q".repeat(8), &Id([3; 32]).hex()[..16]] {
                assert!(!decoded.windows(needle.len()).any(|w| w == needle.as_bytes()), "Base64 alone must not expose {needle}");
            }
            // and the deflated payload alone, without the mask, gives nothing away either
            let masked = &decoded[CODE_HEADER..];
            if let Ok(inflated) = inflate(masked, 1 << 16) {
                assert!(!inflated.windows(11).any(|w| w == b"203.0.113.7"));
            }
            assert_eq!(NodeLink::parse(&text).unwrap(), link);
        }
        // An invite's code is no node link, nor the other way round.
        let invite = Invite { community: Id([5; 32]), peers: link.peers.clone(), ..Default::default() };
        assert!(NodeLink::parse(&format!("hoi://node?code={}", code_of(&invite.to_uri()))).is_err());
        assert!(Invite::parse(&format!("hoi://join?code={}", code_of(&link.to_uri()))).is_err());
    }

    #[test]
    fn node_link_codes_reject_damage_and_mixed_fields() {
        let link = NodeLink { rtc: vec![format!("rtc:{}@203.0.113.7:7844", "q".repeat(43))], ..NodeLink::default() };
        let uri = link.to_uri();
        let envelope = unb64(code_of(&uri)).unwrap();
        for end in 0..envelope.len() {
            assert!(NodeLink::parse(&format!("hoi://node?code={}", b64(&envelope[..end]))).is_err(), "length {end}");
        }
        for at in 0..envelope.len() {
            let mut damaged = envelope.clone();
            damaged[at] ^= 1;
            assert!(NodeLink::parse(&format!("hoi://node?code={}", b64(&damaged))).is_err(), "byte {at}");
        }
        for extra in ["&code=abc", "&w=qq@198.51.100.1:7844", "&p=x@198.51.100.1:7843", "&nt=AA", "&ns=AA"] {
            assert!(NodeLink::parse(&format!("{uri}{extra}")).is_err(), "{extra}");
        }
        for code in ["", "%%%", "AAAA", &"A".repeat(MAX_NODE_CODE + 1)] {
            assert!(NodeLink::parse(&format!("hoi://node?code={code}")).is_err(), "{code:.20}");
        }
        // what the code holds is checked like any record's fields
        let fields = || vec![("v", Value::U(1)), ("w", Value::A(vec![Value::T(format!("{}@203.0.113.7:7844", "q".repeat(43)))])), ("p", Value::A(vec![]))];
        let packed =
            |f: Vec<(&'static str, Value)>| format!("hoi://node?code={}", encode_code(NODE_MAGIC, NODE_MASK_CONTEXT, deflate(&Value::map(f).encode())));
        assert!(NodeLink::parse(&packed(fields())).is_ok());
        for (key, bad) in [("v", Value::U(2)), ("w", Value::T("route".into())), ("p", Value::A(vec![Value::U(1)]))] {
            let mut f = fields();
            f.iter_mut().find(|(k, _)| *k == key).unwrap().1 = bad;
            assert!(NodeLink::parse(&packed(f)).is_err(), "{key}");
        }
        let mut unknown = fields();
        unknown.push(("x", Value::U(1)));
        assert!(NodeLink::parse(&packed(unknown)).is_err(), "unknown fields");
        let mut long = fields();
        long.push(("n", Value::B(vec![b'a'; crate::notice::MAX_NOTICE_WIRE + 1])));
        long.push(("s", Value::B(vec![0; 64])));
        assert!(NodeLink::parse(&packed(long)).is_err(), "a notice too long");
        let mut short_sig = fields();
        short_sig.push(("n", Value::B(vec![1])));
        short_sig.push(("s", Value::B(vec![0; 63])));
        assert!(NodeLink::parse(&packed(short_sig)).is_err(), "a signature of 63 bytes");
        let mut lone_sig = fields();
        lone_sig.push(("s", Value::B(vec![0; 64])));
        assert!(NodeLink::parse(&packed(lone_sig)).is_err(), "a signature without a notice");
        // a code in the clear, without the mask, is refused rather than read
        assert!(NodeLink::parse(&format!("hoi://node?code={}", b64(&Value::map(fields()).encode()))).is_err());
    }

    #[test]
    fn node_link_carries_a_signed_notice_and_stays_short() {
        let notice = crate::notice::NodeNotice {
            operator: "Lena".into(),
            contact: "lena@example.org".into(),
            country: "XX".into(),
            updated: "2026-11-03".into(),
            ..crate::notice::NodeNotice::template("sleepy-walrus")
        };
        let endpoint = Id([7; 32]).hex();
        let link = NodeLink {
            rtc: vec![format!("rtc:{}@203.0.113.9:7844", "a".repeat(43))],
            peers: vec![format!("{endpoint}@203.0.113.9:7843")],
            notice: Some(notice.canonical()),
            signature: Some(Sig([9; 64])),
        };
        let uri = link.to_uri();
        assert!(uri.len() < 900, "deflated, a plain notice keeps the link scannable: {} bytes", uri.len());
        assert!(uri.len() <= format!("hoi://node?{}", legacy_query(&link)).len() + 40, "about as short as the clear-text link");
        assert!(!uri.contains("Lena") && !uri.contains("203.0.113.9"));
        let back = NodeLink::parse(&uri).unwrap();
        assert_eq!(back, link);
        assert_eq!(back.endpoint(), Some(Id([7; 32])));
        assert_eq!(crate::notice::NodeNotice::from_canonical(back.notice.as_deref().unwrap()).unwrap(), notice);
        // The web form keeps it too, and a link without the signature is refused.
        assert_eq!(NodeLink::parse(&link.to_web("https://hoi.sh/app/")).unwrap(), link);
        let unsigned = NodeLink { signature: None, ..link.clone() };
        assert!(NodeLink::parse(&unsigned.to_uri()).is_err());
        // A link without a notice opens.
        let plain = NodeLink { notice: None, signature: None, ..link.clone() };
        assert_eq!(NodeLink::parse(&plain.to_uri()).unwrap(), plain);
    }

    #[test]
    fn clear_text_node_links_still_open() {
        let notice =
            crate::notice::NodeNotice { operator: "Lena".into(), updated: "2026-11-03".into(), ..crate::notice::NodeNotice::template("sleepy-walrus") };
        let link = NodeLink {
            rtc: vec![format!("rtc:{}@203.0.113.7:7844", "q".repeat(43)), format!("rtc:{}@[2001:db8::7]:7844", "q".repeat(43))],
            peers: vec![format!("{}@203.0.113.7:7843", Id([3; 32]).hex())],
            notice: Some(notice.canonical()),
            signature: Some(Sig([9; 64])),
        };
        let q = legacy_query(&link);
        for prefix in ["hoi://node?", "https://hoi.sh/app/#/node?"] {
            assert_eq!(NodeLink::parse(&format!("{prefix}{q}")).unwrap(), link);
        }
        // Frozen, from before codes: independent of the encoder above.
        let frozen = format!("https://hoi.sh/app/#/node?w={}@203.0.113.7:7844&p={}@203.0.113.7:7843", "q".repeat(43), Id([3; 32]).hex());
        let old = NodeLink::parse(&frozen).unwrap();
        assert_eq!((old.rtc.len(), old.peers.len(), old.endpoint()), (1, 1, Some(Id([3; 32]))));
        // the same checks as before
        let unsigned = q.split("&ns=").next().unwrap();
        assert!(NodeLink::parse(&format!("hoi://node?{unsigned}")).is_err());
        assert!(NodeLink::parse(&format!("hoi://node?p=rtc:{}@203.0.113.7:7844", "q".repeat(43))).unwrap_err().to_string().contains("no address"));
        let plain = NodeLink { notice: None, signature: None, ..link };
        assert!(NodeLink::parse(&format!("hoi://node?{}&nt=!!!", legacy_query(&plain))).is_err());
    }

    #[test]
    fn pair_roundtrip() {
        let p = Pair { url: "ws://127.0.0.1:7842".into(), token: "t0k3n".into(), name: Some("lena's laptop".into()), rtc: vec![] };
        assert_eq!(Pair::parse(&p.to_uri()).unwrap(), p);
        assert!(Pair::parse("hoi://pair?u=http://x&t=1").is_err());
        assert!(Pair::parse("hoi://pair?t=1").is_err());
        let fp = "q".repeat(43);
        let p = Pair {
            url: "ws://127.0.0.1:7842".into(),
            token: "t0k3n".into(),
            name: None,
            rtc: vec![format!("rtc:{fp}@192.168.1.20:7844"), format!("rtc:{fp}@[fd00::1]:7844")],
        };
        let web = p.to_web("https://hoi.sh/app/");
        assert!(web.starts_with("https://hoi.sh/app/#/pair?code="), "{web}");
        // no address, token or route readable, not even after Base64
        let decoded = unb64(web.split_once("code=").unwrap().1).unwrap();
        for needle in ["192.168.1.20", "fd00", "127.0.0.1", "7842", "t0k3n", &fp[..8]] {
            assert!(!web.contains(needle) && !decoded.windows(needle.len()).any(|w| w == needle.as_bytes()), "{needle} in {web}");
        }
        assert_eq!(Pair::parse(&web).unwrap(), p);
        // clear-text links still open; a code with clear-text fields beside it does not
        let legacy = format!("https://hoi.sh/app/#/pair?u={}&t=t0k3n&w={fp}@192.168.1.20:7844,{fp}@%5Bfd00::1%5D:7844", pct_encode(&p.url));
        assert_eq!(Pair::parse(&legacy).unwrap(), p);
        assert!(Pair::parse(&format!("{web}&t=other")).is_err());
        assert!(Pair::parse(&format!("hoi://pair?code={}", web.split_once("code=").unwrap().1.replace('A', "B"))).is_err());
        assert_eq!(Pair::parse(&p.to_uri()).unwrap(), p);
        // WebRTC alone is enough to pair.
        let only = Pair { url: String::new(), rtc: vec![format!("rtc:{fp}@10.0.0.2:7844")], ..p };
        assert_eq!(Pair::parse(&only.to_uri()).unwrap(), only);
    }
}
