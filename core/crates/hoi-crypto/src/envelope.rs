//! Sealed envelopes for scattered delivery (SCATTER.md §5.8, PROTOCOL.md §21).
//!
//! A sender seals each request to the holder's X25519 key (`xk`) with HPKE (RFC 9180, base
//! mode, DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305); the forwarder between them
//! sees only the envelope. The holder answers sealed to a one-time key the request carries, so
//! the forwarder cannot read the answer either, and an old answer opens under no new request.
//!
//! `info` names the kind of message, so an envelope made for one kind opens as no other; the AAD
//! binds requests to the holder's endpoint id and key id, and answers to the locator asked
//! about. Requests are one fixed size ([`REQUEST_LEN`]), short answers another
//! ([`SHORT_REPLY_LEN`]), pieces travel at their class's piece length: sizes say which kind of
//! message passed, never which request.
//!
//! The HPKE implementation is `hpke-rs` with the RustCrypto provider (already vetted through
//! OpenMLS). Nothing cryptographic is implemented here; [`Suite`] only narrows the provider to
//! this one suite and lets the ephemeral key come from the system or, for conformance vectors,
//! from a given seed.

use hpke_rs::hpke_types::{AeadAlgorithm, KdfAlgorithm, KemAlgorithm, SingleStageKdfAlgorithm, TwoStageKdfAlgorithm};
use hpke_rs::{Hpke, HpkePrivateKey, HpkePublicKey, Mode};
use hpke_rs_crypto::error::Error as HpkeCryptoError;
use hpke_rs_crypto::{HpkeCrypto, TryCryptoRng, TryRng};
use hpke_rs_rust_crypto::HpkeRustCrypto;
use zeroize::{Zeroize, Zeroizing};

use crate::{CryptoError, random_bytes};

/// Every request envelope (put excepted, which carries a piece) is this long.
pub const REQUEST_LEN: usize = 256;
/// Every short answer (stored, has, dropped) is this long.
pub const SHORT_REPLY_LEN: usize = 64;
/// What HPKE adds: the encapsulated key and the AEAD tag.
pub const OVERHEAD: usize = 32 + 16;
/// Bytes a put adds to its piece, envelope included.
pub const PUT_OVERHEAD: usize = OVERHEAD + PUT_HEADER;
/// Bytes a got answer adds to its piece, envelope included.
pub const GOT_OVERHEAD: usize = OVERHEAD + GOT_HEADER;

const PUT_HEADER: usize = 1 + 32 + 32 + 4 + 32 + 16 + 8 + 4;
const GOT_HEADER: usize = 1 + 1 + 4;

/// What an envelope is: bound into HPKE's `info`, so one kind never opens as another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Put,
    Get,
    Has,
    Drop,
    Stored,
    Got,
    Had,
    Dropped,
}

impl Kind {
    fn info(self) -> &'static [u8] {
        match self {
            Kind::Put => b"hoi scatter v1 put",
            Kind::Get => b"hoi scatter v1 get",
            Kind::Has => b"hoi scatter v1 has",
            Kind::Drop => b"hoi scatter v1 drop",
            Kind::Stored => b"hoi scatter v1 stored",
            Kind::Got => b"hoi scatter v1 got",
            Kind::Had => b"hoi scatter v1 had",
            Kind::Dropped => b"hoi scatter v1 dropped",
        }
    }
    fn tag(self) -> u8 {
        match self {
            Kind::Put => 1,
            Kind::Get => 2,
            Kind::Has => 3,
            Kind::Drop => 4,
            Kind::Stored => 5,
            Kind::Got => 6,
            Kind::Had => 7,
            Kind::Dropped => 8,
        }
    }
    /// The kind of answer a request gets.
    pub fn answer(self) -> Option<Kind> {
        match self {
            Kind::Put => Some(Kind::Stored),
            Kind::Get => Some(Kind::Got),
            Kind::Has => Some(Kind::Had),
            Kind::Drop => Some(Kind::Dropped),
            _ => None,
        }
    }
}

/// An X25519 key pair: a holder's `xk`, or a one-time answer key. The secret stays inside
/// hpke-rs's key type, which zeroizes it on drop.
pub struct XKey {
    secret: HpkePrivateKey,
    public: [u8; 32],
}

impl std::fmt::Debug for XKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "XKey({})", hoi_types::hex_encode(&self.public[..4]))
    }
}

impl XKey {
    /// A fresh key from the system's randomness.
    pub fn generate() -> Result<XKey, CryptoError> {
        let mut seed = random_bytes::<32>()?;
        let k = XKey::from_seed(&seed);
        seed.zeroize();
        k
    }

    /// HPKE's DeriveKeyPair over a 32-byte seed (RFC 9180 §7.1.3).
    pub fn from_seed(seed: &[u8; 32]) -> Result<XKey, CryptoError> {
        let (sk, pk) = hpke(Rng::system()).derive_key_pair(seed).map_err(|_| CryptoError::Key)?.into_keys();
        let public: [u8; 32] = pk.as_slice().try_into().map_err(|_| CryptoError::Key)?;
        Ok(XKey { secret: sk, public })
    }

    pub fn public(&self) -> [u8; 32] {
        self.public
    }
}

/// A holder key's id, as requests bind it: 8 bytes of BLAKE3 over the public key.
pub fn key_id(xk: &[u8; 32]) -> [u8; 8] {
    let mut h = blake3::Hasher::new_derive_key("hoi scatter xk id v1");
    h.update(xk);
    let mut out = [0u8; 8];
    out.copy_from_slice(&h.finalize().as_bytes()[..8]);
    out
}

/// What a node's endpoint key signs to publish its `xk` until `expires_at`.
pub fn xk_statement(endpoint: &[u8; 32], xk: &[u8; 32], expires_at: u64) -> Vec<u8> {
    let mut s = b"hoi/scatter-xk/1\n".to_vec();
    s.extend_from_slice(endpoint);
    s.extend_from_slice(xk);
    s.extend_from_slice(&expires_at.to_be_bytes());
    s
}

/// The holder's durable-marker capability, bound to the same key and expiry as its hello.
/// Separate from the ordinary key signature so a forwarder cannot upgrade a legacy holder.
pub fn drop_journal_statement(endpoint: &[u8; 32], xk: &[u8; 32], expires_at: u64) -> Vec<u8> {
    let mut statement = xk_statement(endpoint, xk, expires_at);
    statement.extend_from_slice(b"\0drop-journal");
    statement
}

/// What a holder signs with its endpoint key to bind the certificate it serves WebRTC with to
/// that endpoint: a browser pins the certificate when it dials but sees no endpoint, and needs
/// one to know whose holder key a hello carries (PROTOCOL.md §21).
pub fn rtc_statement(endpoint: &[u8; 32], cert: &[u8; 32]) -> Vec<u8> {
    let mut s = b"hoi/rtc-endpoint/1\n".to_vec();
    s.extend_from_slice(endpoint);
    s.extend_from_slice(cert);
    s
}

/// Whether `sig` binds the WebRTC certificate `cert` (its SHA-256) to `endpoint`.
pub fn rtc_valid(endpoint: &[u8; 32], cert: &[u8; 32], sig: &hoi_types::Sig) -> bool {
    crate::verify(&hoi_types::Id(*endpoint), &rtc_statement(endpoint, cert), sig)
}

/// Whether `sig` publishes `xk` for `endpoint` and it has not expired at `now`. Senders seal only
/// to keys this accepts.
pub fn xk_valid(endpoint: &[u8; 32], xk: &[u8; 32], expires_at: u64, sig: &hoi_types::Sig, now: u64) -> bool {
    now < expires_at && crate::verify(&hoi_types::Id(*endpoint), &xk_statement(endpoint, xk, expires_at), sig)
}

/// How a drop is allowed: by the creator's token, or by the community drop key's signature over
/// [`crate::scatter::drop_statement`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropAuth {
    Token([u8; 32]),
    Signed { key: [u8; 32], sig: [u8; 64] },
}

/// A request a holder opens. `reply` is the one-time public key its answer is sealed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Keep `piece` under `loc` until the end of UTC day `expiry_day` (days since 1970).
    Put {
        loc: [u8; 32],
        reply: [u8; 32],
        expiry_day: u32,
        drop_hash: [u8; 32],
        drop_key_id: [u8; 16],
        stamp: u64,
        piece: Vec<u8>,
    },
    /// The piece under `loc`; the answer is padded to `want_len` whether or not it is there.
    Get {
        loc: [u8; 32],
        reply: [u8; 32],
        want_len: u32,
    },
    /// Whether a piece is kept under `loc` (repair's probe).
    Has {
        loc: [u8; 32],
        reply: [u8; 32],
    },
    Drop {
        loc: [u8; 32],
        reply: [u8; 32],
        auth: DropAuth,
    },
}

/// How a holder answered a put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    Ok,
    Full,
    BadStamp,
    Refused,
    /// Something else is already kept under this locator; it is never overwritten (a mailbox
    /// writer takes the next slot).
    Taken,
}

/// How a holder answered a drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    Dropped,
    Unknown,
    Refused,
}

/// An answer, sealed to the request's one-time key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Stored(Stored),
    /// The piece, or `None`; padded to the request's `want_len` either way.
    Got(Option<Vec<u8>>),
    /// Whether a piece is kept, and its length when it is (so a reader can ask for exactly that).
    Had(Option<u32>),
    Dropped(Dropped),
}

impl Request {
    pub fn kind(&self) -> Kind {
        match self {
            Request::Put { .. } => Kind::Put,
            Request::Get { .. } => Kind::Get,
            Request::Has { .. } => Kind::Has,
            Request::Drop { .. } => Kind::Drop,
        }
    }

    pub fn loc(&self) -> &[u8; 32] {
        match self {
            Request::Put { loc, .. } | Request::Get { loc, .. } | Request::Has { loc, .. } | Request::Drop { loc, .. } => loc,
        }
    }

    pub fn reply_key(&self) -> &[u8; 32] {
        match self {
            Request::Put { reply, .. } | Request::Get { reply, .. } | Request::Has { reply, .. } | Request::Drop { reply, .. } => reply,
        }
    }

    /// The plaintext inside the envelope: fixed layouts, requests other than put padded with
    /// zeros to one size.
    pub fn to_plaintext(&self) -> Result<Vec<u8>, CryptoError> {
        let mut out = vec![self.kind().tag()];
        out.extend_from_slice(self.loc());
        out.extend_from_slice(self.reply_key());
        match self {
            Request::Put { expiry_day, drop_hash, drop_key_id, stamp, piece, .. } => {
                if piece.is_empty() || piece.len() > crate::scatter::MAX_PIECE {
                    return Err(CryptoError::Format);
                }
                out.extend_from_slice(&expiry_day.to_be_bytes());
                out.extend_from_slice(drop_hash);
                out.extend_from_slice(drop_key_id);
                out.extend_from_slice(&stamp.to_be_bytes());
                out.extend_from_slice(&(piece.len() as u32).to_be_bytes());
                out.extend_from_slice(piece);
                return Ok(out);
            }
            Request::Get { want_len, .. } => {
                if *want_len == 0 || *want_len as usize > crate::scatter::MAX_PIECE {
                    return Err(CryptoError::Format);
                }
                out.extend_from_slice(&want_len.to_be_bytes());
            }
            Request::Has { .. } => {}
            Request::Drop { auth: DropAuth::Token(d), .. } => {
                out.push(0);
                out.extend_from_slice(d);
            }
            Request::Drop { auth: DropAuth::Signed { key, sig }, .. } => {
                out.push(1);
                out.extend_from_slice(key);
                out.extend_from_slice(sig);
            }
        }
        out.resize(REQUEST_LEN - OVERHEAD, 0);
        Ok(out)
    }

    /// Parses a request's plaintext strictly: its tag, exact length and zero padding.
    pub fn from_plaintext(kind: Kind, b: &[u8]) -> Result<Request, CryptoError> {
        let mut r = Reader::new(b);
        if r.u8()? != kind.tag() {
            return Err(CryptoError::Format);
        }
        let loc = r.arr::<32>()?;
        let reply = r.arr::<32>()?;
        let req = match kind {
            Kind::Put => {
                let expiry_day = r.u32()?;
                let drop_hash = r.arr::<32>()?;
                let drop_key_id = r.arr::<16>()?;
                let stamp = r.u64()?;
                let len = r.u32()? as usize;
                if len == 0 || len > crate::scatter::MAX_PIECE {
                    return Err(CryptoError::Format);
                }
                let piece = r.take(len)?.to_vec();
                r.end()?;
                return Ok(Request::Put { loc, reply, expiry_day, drop_hash, drop_key_id, stamp, piece });
            }
            Kind::Get => {
                let want_len = r.u32()?;
                if want_len == 0 || want_len as usize > crate::scatter::MAX_PIECE {
                    return Err(CryptoError::Format);
                }
                Request::Get { loc, reply, want_len }
            }
            Kind::Has => Request::Has { loc, reply },
            Kind::Drop => {
                let auth = match r.u8()? {
                    0 => DropAuth::Token(r.arr::<32>()?),
                    1 => DropAuth::Signed { key: r.arr::<32>()?, sig: r.arr::<64>()? },
                    _ => return Err(CryptoError::Format),
                };
                Request::Drop { loc, reply, auth }
            }
            _ => return Err(CryptoError::Format),
        };
        if b.len() != REQUEST_LEN - OVERHEAD {
            return Err(CryptoError::Format);
        }
        r.zeros()?;
        Ok(req)
    }
}

impl Reply {
    pub fn kind(&self) -> Kind {
        match self {
            Reply::Stored(_) => Kind::Stored,
            Reply::Got(_) => Kind::Got,
            Reply::Had(_) => Kind::Had,
            Reply::Dropped(_) => Kind::Dropped,
        }
    }

    /// The plaintext of an answer; a got is padded to the get's `want_len`.
    pub fn to_plaintext(&self, want_len: usize) -> Result<Vec<u8>, CryptoError> {
        let mut out = vec![self.kind().tag()];
        match self {
            Reply::Stored(s) => out.push(*s as u8),
            Reply::Had(h) => {
                if h.is_some_and(|len| len == 0 || len as usize > crate::scatter::MAX_PIECE) {
                    return Err(CryptoError::Format);
                }
                out.push(h.is_some() as u8);
                out.extend_from_slice(&h.unwrap_or(0).to_be_bytes());
            }
            Reply::Dropped(d) => out.push(*d as u8),
            Reply::Got(p) => {
                if want_len == 0 || want_len > crate::scatter::MAX_PIECE {
                    return Err(CryptoError::Format);
                }
                out.push(p.is_some() as u8);
                let piece = p.as_deref().unwrap_or(&[]);
                if piece.len() > want_len {
                    return Err(CryptoError::Format);
                }
                out.extend_from_slice(&(piece.len() as u32).to_be_bytes());
                out.extend_from_slice(piece);
                out.resize(GOT_HEADER + want_len, 0);
                return Ok(out);
            }
        }
        out.resize(SHORT_REPLY_LEN - OVERHEAD, 0);
        Ok(out)
    }

    /// Parses an answer's plaintext strictly.
    pub fn from_plaintext(kind: Kind, want_len: usize, b: &[u8]) -> Result<Reply, CryptoError> {
        let mut r = Reader::new(b);
        if r.u8()? != kind.tag() {
            return Err(CryptoError::Format);
        }
        let status = r.u8()?;
        let reply = match (kind, status) {
            (Kind::Stored, 0) => Reply::Stored(Stored::Ok),
            (Kind::Stored, 1) => Reply::Stored(Stored::Full),
            (Kind::Stored, 2) => Reply::Stored(Stored::BadStamp),
            (Kind::Stored, 3) => Reply::Stored(Stored::Refused),
            (Kind::Stored, 4) => Reply::Stored(Stored::Taken),
            (Kind::Had, 0 | 1) => {
                let len = r.u32()?;
                match (status, len) {
                    (0, 0) => Reply::Had(None),
                    (1, l) if l > 0 && l as usize <= crate::scatter::MAX_PIECE => Reply::Had(Some(l)),
                    _ => return Err(CryptoError::Format),
                }
            }
            (Kind::Dropped, 0) => Reply::Dropped(Dropped::Dropped),
            (Kind::Dropped, 1) => Reply::Dropped(Dropped::Unknown),
            (Kind::Dropped, 2) => Reply::Dropped(Dropped::Refused),
            (Kind::Got, 0 | 1) => {
                // the same bounds the encoder keeps, so whatever parses writes back
                if want_len == 0 || want_len > crate::scatter::MAX_PIECE || b.len() != GOT_HEADER + want_len {
                    return Err(CryptoError::Format);
                }
                let len = r.u32()? as usize;
                if len > want_len || (status == 0 && len != 0) || (status == 1 && len == 0) {
                    return Err(CryptoError::Format);
                }
                let piece = r.take(len)?.to_vec();
                r.zeros()?;
                return Ok(Reply::Got((status == 1).then_some(piece)));
            }
            _ => return Err(CryptoError::Format),
        };
        if b.len() != SHORT_REPLY_LEN - OVERHEAD {
            return Err(CryptoError::Format);
        }
        r.zeros()?;
        Ok(reply)
    }
}

fn request_aad(endpoint: &[u8; 32], xk: &[u8; 32]) -> Vec<u8> {
    let mut aad = b"hoi scatter to v1".to_vec();
    aad.extend_from_slice(endpoint);
    aad.extend_from_slice(&key_id(xk));
    aad
}

fn reply_aad(loc: &[u8; 32]) -> Vec<u8> {
    let mut aad = b"hoi scatter reply v1".to_vec();
    aad.extend_from_slice(loc);
    aad
}

/// Seals `request` to the holder at `endpoint` whose valid key is `xk`.
pub fn seal_request(endpoint: &[u8; 32], xk: &[u8; 32], request: &Request) -> Result<Vec<u8>, CryptoError> {
    seal(Rng::system(), request.kind(), xk, &request_aad(endpoint, xk), &mut request.to_plaintext()?)
}

/// Opens a request at the holder `endpoint` with its key; `kind` comes from the message that
/// carried it.
pub fn open_request(kind: Kind, endpoint: &[u8; 32], key: &XKey, envelope: &[u8]) -> Result<Request, CryptoError> {
    let fits = match kind {
        Kind::Put => envelope.len() > PUT_OVERHEAD && envelope.len() <= PUT_OVERHEAD + crate::scatter::MAX_PIECE,
        _ => envelope.len() == REQUEST_LEN,
    };
    if kind.answer().is_none() || !fits {
        return Err(CryptoError::Format);
    }
    let mut plain = open(kind, key, &request_aad(endpoint, &key.public), envelope)?;
    let r = Request::from_plaintext(kind, &plain);
    plain.zeroize();
    r
}

/// Seals the answer to a request about `loc` to its one-time key `reply`. `want_len` is the get's
/// (ignored for other answers).
pub fn seal_reply(reply: &[u8; 32], loc: &[u8; 32], answer: &Reply, want_len: usize) -> Result<Vec<u8>, CryptoError> {
    seal(Rng::system(), answer.kind(), reply, &reply_aad(loc), &mut answer.to_plaintext(want_len)?)
}

/// Opens an answer with the one-time key the request carried.
pub fn open_reply(kind: Kind, key: &XKey, loc: &[u8; 32], want_len: usize, envelope: &[u8]) -> Result<Reply, CryptoError> {
    let expected = match kind {
        Kind::Got => OVERHEAD + GOT_HEADER + want_len,
        Kind::Stored | Kind::Had | Kind::Dropped => SHORT_REPLY_LEN,
        _ => return Err(CryptoError::Format),
    };
    if envelope.len() != expected {
        return Err(CryptoError::Format);
    }
    let mut plain = open(kind, key, &reply_aad(loc), envelope)?;
    let r = Reply::from_plaintext(kind, want_len, &plain);
    plain.zeroize();
    r
}

/// [`seal_request`] with the ephemeral key derived from `seed`: for conformance vectors only.
/// A seed must never be used twice.
#[cfg(feature = "deterministic")]
pub fn seal_request_with_seed(endpoint: &[u8; 32], xk: &[u8; 32], request: &Request, seed: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    seal(Rng::fixed(seed), request.kind(), xk, &request_aad(endpoint, xk), &mut request.to_plaintext()?)
}

/// [`seal_reply`] with the ephemeral key derived from `seed`: for conformance vectors only.
#[cfg(feature = "deterministic")]
pub fn seal_reply_with_seed(reply: &[u8; 32], loc: &[u8; 32], answer: &Reply, want_len: usize, seed: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    seal(Rng::fixed(seed), answer.kind(), reply, &reply_aad(loc), &mut answer.to_plaintext(want_len)?)
}

/// What a sealed drop key is: bound into HPKE's `info`, apart from every envelope kind.
const DROP_KEY_INFO: &[u8] = b"hoi drop key v1";
/// A sealed drop key's length: the encapsulated key, the 32-byte secret and the tag.
pub const DROP_KEY_LEN: usize = OVERHEAD + 32;

/// Seals a community drop key's secret to a committer's device seal key `to`. `aad`
/// binds where it belongs (the community, the device, the key's public part), so it opens for
/// nothing else.
pub fn seal_drop_key(to: &[u8; 32], aad: &[u8], secret: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    let mut h = hpke(Rng::system());
    let mut plain = Zeroizing::new(secret.to_vec());
    let (enc, ct) = h.seal(&HpkePublicKey::new(to.to_vec()), DROP_KEY_INFO, aad, &plain, None, None, None).map_err(|_| CryptoError::Key)?;
    plain.zeroize();
    let mut out = Vec::with_capacity(DROP_KEY_LEN);
    out.extend_from_slice(&enc);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Opens a drop key sealed to `key` with [`seal_drop_key`] for the same `aad`.
pub fn open_drop_key(key: &XKey, aad: &[u8], sealed: &[u8]) -> Result<[u8; 32], CryptoError> {
    if sealed.len() != DROP_KEY_LEN {
        return Err(CryptoError::Format);
    }
    let (enc, ct) = sealed.split_at(32);
    let plain = Zeroizing::new(hpke(Rng::system()).open(enc, &key.secret, DROP_KEY_INFO, aad, ct, None, None, None).map_err(|_| CryptoError::Open)?);
    <[u8; 32]>::try_from(plain.as_slice()).map_err(|_| CryptoError::Format)
}

/// Seals `plain` to the X25519 key `to`, the envelopes' suite in base mode, for one purpose
/// (`info`) and bound to `aad`: `enc[32] ‖ ciphertext`. An invitation sealed to a newcomer's
/// one-time pickup key (S4), a drop key sealed to a moderator's delegation key (S5).
pub fn seal_to(to: &[u8; 32], info: &[u8], aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let (enc, ct) = hpke(Rng::system()).seal(&HpkePublicKey::new(to.to_vec()), info, aad, plain, None, None, None).map_err(|_| CryptoError::Key)?;
    let mut out = Vec::with_capacity(enc.len() + ct.len());
    out.extend_from_slice(&enc);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Opens what [`seal_to`] sealed to `key` for the same `info` and `aad`.
pub fn open_sealed(key: &XKey, info: &[u8], aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if sealed.len() < OVERHEAD {
        return Err(CryptoError::Format);
    }
    let (enc, ct) = sealed.split_at(32);
    Ok(Zeroizing::new(hpke(Rng::system()).open(enc, &key.secret, info, aad, ct, None, None, None).map_err(|_| CryptoError::Open)?))
}

const WRAP_INFO: &[u8] = b"hoi scatter v1 wrap";
const INJECT_INFO: &[u8] = b"hoi scatter v1 inject";
/// An injected batch is padded to a multiple of this, so its size says little.
const INJECT_PAD: usize = 1024;
/// An injected batch holds at most this many records, of at most this many bytes in all.
pub const MAX_INJECT: usize = 16;
pub const MAX_INJECT_BYTES: usize = 192 * 1024;

impl Kind {
    fn of_request_tag(tag: u8) -> Option<Kind> {
        [Kind::Put, Kind::Get, Kind::Has, Kind::Drop].into_iter().find(|k| k.tag() == tag)
    }
}

/// The outer layer of a three-hop request (S7): sealed to the exit forwarder at `endpoint` (its
/// key `xk`), it names the holder `to` and carries the request's own envelope, of kind `inner`,
/// sealed to that holder. The forwarder a device sends it through sees the exit, never the
/// holder; the exit sees the holder, never the device.
pub fn seal_wrap(endpoint: &[u8; 32], xk: &[u8; 32], to: &[u8; 32], inner: Kind, env: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if inner.answer().is_none() || env.is_empty() || env.len() > PUT_OVERHEAD + crate::scatter::MAX_PIECE {
        return Err(CryptoError::Format);
    }
    let mut plain = Zeroizing::new(Vec::with_capacity(33 + env.len()));
    plain.extend_from_slice(to);
    plain.push(inner.tag());
    plain.extend_from_slice(env);
    seal_to(xk, WRAP_INFO, &request_aad(endpoint, xk), &plain)
}

/// Opens a wrap at the exit `endpoint` with its key: the holder, the inner kind and envelope.
pub fn open_wrap(endpoint: &[u8; 32], key: &XKey, envelope: &[u8]) -> Result<([u8; 32], Kind, Vec<u8>), CryptoError> {
    if envelope.len() > OVERHEAD + 33 + PUT_OVERHEAD + crate::scatter::MAX_PIECE {
        return Err(CryptoError::Format);
    }
    let plain = open_sealed(key, WRAP_INFO, &request_aad(endpoint, &key.public), envelope)?;
    if plain.len() < 34 {
        return Err(CryptoError::Format);
    }
    let to: [u8; 32] = plain[..32].try_into().map_err(|_| CryptoError::Format)?;
    let inner = Kind::of_request_tag(plain[32]).ok_or(CryptoError::Format)?;
    Ok((to, inner, plain[33..].to_vec()))
}

/// A device's own new records, sealed to the node at `endpoint` that injects them into the
/// network for it (S7): `u16(count) ‖ (u32(len) ‖ record)* ‖ zeros` to a multiple of 1 KiB.
pub fn seal_inject(endpoint: &[u8; 32], xk: &[u8; 32], wires: &[Vec<u8>]) -> Result<Vec<u8>, CryptoError> {
    let total: usize = wires.iter().map(Vec::len).sum();
    if wires.is_empty() || wires.len() > MAX_INJECT || total > MAX_INJECT_BYTES || wires.iter().any(Vec::is_empty) {
        return Err(CryptoError::Format);
    }
    let mut plain = Zeroizing::new(Vec::with_capacity(total + 4 * wires.len() + INJECT_PAD));
    plain.extend_from_slice(&(wires.len() as u16).to_be_bytes());
    for w in wires {
        plain.extend_from_slice(&(w.len() as u32).to_be_bytes());
        plain.extend_from_slice(w);
    }
    let padded = plain.len().div_ceil(INJECT_PAD) * INJECT_PAD;
    plain.resize(padded, 0);
    seal_to(xk, INJECT_INFO, &request_aad(endpoint, xk), &plain)
}

/// Opens an injected batch at `endpoint`: the records, strictly (count, lengths, zero padding).
pub fn open_inject(endpoint: &[u8; 32], key: &XKey, envelope: &[u8]) -> Result<Vec<Vec<u8>>, CryptoError> {
    if envelope.len() > OVERHEAD + MAX_INJECT_BYTES + 4 * MAX_INJECT + 2 + INJECT_PAD {
        return Err(CryptoError::Format);
    }
    let plain = open_sealed(key, INJECT_INFO, &request_aad(endpoint, &key.public), envelope)?;
    if plain.len() % INJECT_PAD != 0 {
        return Err(CryptoError::Format);
    }
    let mut r = Reader::new(&plain);
    let count = u16::from_be_bytes([r.u8()?, r.u8()?]) as usize;
    if count == 0 || count > MAX_INJECT {
        return Err(CryptoError::Format);
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let len = r.u32()? as usize;
        if len == 0 || len > MAX_INJECT_BYTES {
            return Err(CryptoError::Format);
        }
        out.push(r.take(len)?.to_vec());
    }
    r.zeros()?;
    Ok(out)
}

fn seal(rng: Rng, kind: Kind, to: &[u8; 32], aad: &[u8], plain: &mut Vec<u8>) -> Result<Vec<u8>, CryptoError> {
    let mut h = hpke(rng);
    let sealed = h.seal(&HpkePublicKey::new(to.to_vec()), kind.info(), aad, plain, None, None, None);
    plain.zeroize();
    let (enc, ct) = sealed.map_err(|_| CryptoError::Key)?;
    let mut out = Vec::with_capacity(enc.len() + ct.len());
    out.extend_from_slice(&enc);
    out.extend_from_slice(&ct);
    Ok(out)
}

fn open(kind: Kind, key: &XKey, aad: &[u8], envelope: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if envelope.len() < OVERHEAD {
        return Err(CryptoError::Format);
    }
    let (enc, ct) = envelope.split_at(32);
    hpke(Rng::system()).open(enc, &key.secret, kind.info(), aad, ct, None, None, None).map_err(|_| CryptoError::Open)
}

fn hpke(rng: Rng) -> Hpke<Suite> {
    Hpke::new_with_rng(Mode::Base, KemAlgorithm::DhKem25519, KdfAlgorithm::HkdfSha256, AeadAlgorithm::ChaCha20Poly1305, rng)
}

/// Where HPKE's randomness comes from: the system, or (vectors only) a seed given once.
pub(crate) struct Rng(Source);

enum Source {
    System,
    #[cfg_attr(not(feature = "deterministic"), allow(dead_code))]
    Fixed(Zeroizing<Vec<u8>>),
}

impl Rng {
    fn system() -> Rng {
        Rng(Source::System)
    }
    #[cfg(feature = "deterministic")]
    fn fixed(seed: &[u8; 32]) -> Rng {
        Rng(Source::Fixed(Zeroizing::new(seed.to_vec())))
    }
}

impl TryRng for Rng {
    type Error = CryptoError;

    fn try_next_u32(&mut self) -> Result<u32, CryptoError> {
        let mut b = [0u8; 4];
        self.try_fill_bytes(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn try_next_u64(&mut self) -> Result<u64, CryptoError> {
        let mut b = [0u8; 8];
        self.try_fill_bytes(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), CryptoError> {
        match &mut self.0 {
            Source::System => getrandom::fill(dst).map_err(|_| CryptoError::Rng),
            // A fixed seed is handed out once, exactly as long as asked for (the ephemeral
            // key's 32 bytes); anything else is refused rather than repeated.
            Source::Fixed(seed) => {
                if seed.len() != dst.len() {
                    return Err(CryptoError::Rng);
                }
                dst.copy_from_slice(seed);
                seed.zeroize();
                seed.clear();
                Ok(())
            }
        }
    }
}

impl TryCryptoRng for Rng {}

/// The RustCrypto HPKE provider narrowed to this one suite, with [`Rng`] as its randomness.
#[derive(Debug)]
pub(crate) struct Suite;

impl HpkeCrypto for Suite {
    type HpkePrng = Rng;

    fn name() -> String {
        "hoi scatter (RustCrypto)".into()
    }
    fn supports_kdf(alg: KdfAlgorithm) -> Result<(), HpkeCryptoError> {
        if alg == KdfAlgorithm::HkdfSha256 { Ok(()) } else { Err(HpkeCryptoError::UnknownKdfAlgorithm) }
    }
    fn supports_kem(alg: KemAlgorithm) -> Result<(), HpkeCryptoError> {
        if alg == KemAlgorithm::DhKem25519 { Ok(()) } else { Err(HpkeCryptoError::UnknownKemAlgorithm) }
    }
    fn supports_aead(alg: AeadAlgorithm) -> Result<(), HpkeCryptoError> {
        if alg == AeadAlgorithm::ChaCha20Poly1305 { Ok(()) } else { Err(HpkeCryptoError::UnknownAeadAlgorithm) }
    }
    fn kdf_extract(alg: TwoStageKdfAlgorithm, salt: &[u8], ikm: &[u8]) -> Result<Vec<u8>, HpkeCryptoError> {
        HpkeRustCrypto::kdf_extract(alg, salt, ikm)
    }
    fn kdf_expand(alg: TwoStageKdfAlgorithm, prk: &[u8], info: &[u8], output_size: usize) -> Result<Vec<u8>, HpkeCryptoError> {
        HpkeRustCrypto::kdf_expand(alg, prk, info, output_size)
    }
    fn kdf_derive(alg: SingleStageKdfAlgorithm, ikm: &[u8], l: usize) -> Result<Vec<u8>, HpkeCryptoError> {
        HpkeRustCrypto::kdf_derive(alg, ikm, l)
    }
    fn dh(alg: KemAlgorithm, pk: &[u8], sk: &[u8]) -> Result<Vec<u8>, HpkeCryptoError> {
        HpkeRustCrypto::dh(alg, pk, sk)
    }
    fn secret_to_public(alg: KemAlgorithm, sk: &[u8]) -> Result<Vec<u8>, HpkeCryptoError> {
        HpkeRustCrypto::secret_to_public(alg, sk)
    }
    // DH-based KEMs derive their keys from randomness generically (RFC 9180 §7.1.3); hpke-rs
    // never calls these two for X25519, and nothing here should.
    fn kem_key_gen(_alg: KemAlgorithm, _prng: &mut Rng) -> Result<(Vec<u8>, Vec<u8>), HpkeCryptoError> {
        Err(HpkeCryptoError::UnsupportedKemOperation)
    }
    fn kem_encaps(_alg: KemAlgorithm, _pk_r: &[u8], _prng: &mut Rng) -> Result<(Vec<u8>, Vec<u8>), HpkeCryptoError> {
        Err(HpkeCryptoError::UnsupportedKemOperation)
    }
    fn kem_key_gen_derand(alg: KemAlgorithm, seed: &[u8]) -> Result<(Vec<u8>, Vec<u8>), HpkeCryptoError> {
        HpkeRustCrypto::kem_key_gen_derand(alg, seed)
    }
    fn kem_encaps_derand(alg: KemAlgorithm, pk_r: &[u8], randomness: &[u8]) -> Result<(Vec<u8>, Vec<u8>), HpkeCryptoError> {
        HpkeRustCrypto::kem_encaps_derand(alg, pk_r, randomness)
    }
    fn kem_decaps(alg: KemAlgorithm, ct: &[u8], sk_r: &[u8]) -> Result<Vec<u8>, HpkeCryptoError> {
        HpkeRustCrypto::kem_decaps(alg, ct, sk_r)
    }
    fn dh_validate_sk(alg: KemAlgorithm, sk: &[u8]) -> Result<Vec<u8>, HpkeCryptoError> {
        HpkeRustCrypto::dh_validate_sk(alg, sk)
    }
    fn aead_seal(alg: AeadAlgorithm, key: &[u8], nonce: &[u8], aad: &[u8], msg: &[u8]) -> Result<Vec<u8>, HpkeCryptoError> {
        HpkeRustCrypto::aead_seal(alg, key, nonce, aad, msg)
    }
    fn aead_open(alg: AeadAlgorithm, key: &[u8], nonce: &[u8], aad: &[u8], msg: &[u8]) -> Result<Vec<u8>, HpkeCryptoError> {
        HpkeRustCrypto::aead_open(alg, key, nonce, aad, msg)
    }
}

/// A cursor over a fixed layout that refuses short input.
struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Reader<'a> {
        Reader { b, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], CryptoError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(CryptoError::Format)?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N], CryptoError> {
        Ok(self.take(N)?.try_into().expect("length checked"))
    }
    fn u8(&mut self) -> Result<u8, CryptoError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, CryptoError> {
        Ok(u32::from_be_bytes(self.arr()?))
    }
    fn u64(&mut self) -> Result<u64, CryptoError> {
        Ok(u64::from_be_bytes(self.arr()?))
    }
    fn end(&self) -> Result<(), CryptoError> {
        if self.pos == self.b.len() { Ok(()) } else { Err(CryptoError::Format) }
    }
    /// The rest must be zero padding.
    fn zeros(&mut self) -> Result<(), CryptoError> {
        let rest = &self.b[self.pos..];
        self.pos = self.b.len();
        if rest.iter().all(|&x| x == 0) { Ok(()) } else { Err(CryptoError::Format) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_key_opens_only_with_its_key_and_for_its_aad() {
        let to = XKey::generate().unwrap();
        let other = XKey::generate().unwrap();
        let secret = [7u8; 32];
        let sealed = seal_drop_key(&to.public(), b"community-device-key", &secret).unwrap();
        assert_eq!(sealed.len(), DROP_KEY_LEN);
        assert_eq!(open_drop_key(&to, b"community-device-key", &sealed).unwrap(), secret);
        assert!(open_drop_key(&other, b"community-device-key", &sealed).is_err(), "another key");
        assert!(open_drop_key(&to, b"community-other-key", &sealed).is_err(), "another binding");
        assert!(open_drop_key(&to, b"community-device-key", &sealed[1..]).is_err());
        // an envelope sealed to the same key opens as no drop key, and the other way round
        let reply = XKey::generate().unwrap();
        let req = Request::Has { loc: [1; 32], reply: reply.public() };
        let env = seal_request(&[2; 32], &to.public(), &req).unwrap();
        assert!(open_drop_key(&to, b"community-device-key", &env[..DROP_KEY_LEN]).is_err());
    }

    const ENDPOINT: [u8; 32] = [7; 32];

    fn put(piece: Vec<u8>, reply: [u8; 32]) -> Request {
        Request::Put { loc: [1; 32], reply, expiry_day: 20_800, drop_hash: [2; 32], drop_key_id: [3; 16], stamp: 99, piece }
    }

    #[test]
    fn requests_open_only_at_their_holder_as_their_kind() {
        let holder = XKey::generate().unwrap();
        let other = XKey::generate().unwrap();
        let answer = XKey::generate().unwrap();
        for req in [
            put(vec![9; 280], answer.public()),
            Request::Get { loc: [1; 32], reply: answer.public(), want_len: 280 },
            Request::Has { loc: [1; 32], reply: answer.public() },
            Request::Drop { loc: [1; 32], reply: answer.public(), auth: DropAuth::Token([4; 32]) },
            Request::Drop { loc: [1; 32], reply: answer.public(), auth: DropAuth::Signed { key: [5; 32], sig: [6; 64] } },
        ] {
            let env = seal_request(&ENDPOINT, &holder.public(), &req).unwrap();
            if req.kind() == Kind::Put {
                assert_eq!(env.len(), PUT_OVERHEAD + 280);
            } else {
                assert_eq!(env.len(), REQUEST_LEN, "{:?}", req.kind());
            }
            assert_eq!(open_request(req.kind(), &ENDPOINT, &holder, &env).unwrap(), req);
            assert_eq!(open_request(req.kind(), &ENDPOINT, &other, &env), Err(CryptoError::Open), "another holder");
            assert_eq!(open_request(req.kind(), &[8; 32], &holder, &env), Err(CryptoError::Open), "another endpoint");
            let wrong = if req.kind() == Kind::Has { Kind::Drop } else { Kind::Has };
            assert!(open_request(wrong, &ENDPOINT, &holder, &env).is_err(), "another kind");
            let mut bad = env.clone();
            *bad.last_mut().unwrap() ^= 1;
            assert_eq!(open_request(req.kind(), &ENDPOINT, &holder, &bad), Err(CryptoError::Open));
        }
        // two seals of one request differ: a fresh ephemeral key each time
        let r = Request::Has { loc: [1; 32], reply: answer.public() };
        assert_ne!(seal_request(&ENDPOINT, &holder.public(), &r).unwrap(), seal_request(&ENDPOINT, &holder.public(), &r).unwrap());
        // answers are not requests
        assert!(open_request(Kind::Got, &ENDPOINT, &holder, &[0; REQUEST_LEN]).is_err());
    }

    #[test]
    fn answers_open_only_with_their_one_time_key() {
        let k = XKey::generate().unwrap();
        let loc = [1u8; 32];
        for (answer, want) in [
            (Reply::Stored(Stored::Full), 0),
            (Reply::Had(Some(280)), 0),
            (Reply::Had(None), 0),
            (Reply::Dropped(Dropped::Unknown), 0),
            (Reply::Got(Some(vec![3; 280])), 280),
            (Reply::Got(None), 280),
        ] {
            let env = seal_reply(&k.public(), &loc, &answer, want).unwrap();
            let len = if answer.kind() == Kind::Got { GOT_OVERHEAD + want } else { SHORT_REPLY_LEN };
            assert_eq!(env.len(), len);
            assert_eq!(open_reply(answer.kind(), &k, &loc, want, &env).unwrap(), answer);
            assert_eq!(open_reply(answer.kind(), &k, &[2; 32], want, &env), Err(CryptoError::Open), "about another locator");
            assert_eq!(open_reply(answer.kind(), &XKey::generate().unwrap(), &loc, want, &env), Err(CryptoError::Open));
        }
        // found and missing pieces look the same from outside
        let a = seal_reply(&k.public(), &loc, &Reply::Got(Some(vec![1; 100])), 100).unwrap();
        let b = seal_reply(&k.public(), &loc, &Reply::Got(None), 100).unwrap();
        assert_eq!(a.len(), b.len());
        assert!(seal_reply(&k.public(), &loc, &Reply::Got(Some(vec![1; 101])), 100).is_err());
    }

    #[test]
    fn layouts_are_strict() {
        let mut plain = put(vec![1; 10], [0; 32]).to_plaintext().unwrap();
        plain.push(0);
        assert!(Request::from_plaintext(Kind::Put, &plain).is_err(), "trailing bytes");
        let mut has = Request::Has { loc: [1; 32], reply: [2; 32] }.to_plaintext().unwrap();
        assert!(Request::from_plaintext(Kind::Has, &has).is_ok());
        *has.last_mut().unwrap() = 1;
        assert!(Request::from_plaintext(Kind::Has, &has).is_err(), "padding must be zero");
        assert!(Request::Get { loc: [0; 32], reply: [0; 32], want_len: 0 }.to_plaintext().is_err());
        assert!(put(Vec::new(), [0; 32]).to_plaintext().is_err());
        assert!(put(vec![0; crate::scatter::MAX_PIECE + 1], [0; 32]).to_plaintext().is_err());
        let mut short = Reply::Had(Some(5)).to_plaintext(0).unwrap();
        short[1] = 2;
        assert!(Reply::from_plaintext(Kind::Had, 0, &short).is_err(), "unknown status");
        assert!(Reply::Had(Some(0)).to_plaintext(0).is_err(), "a kept piece has a length");
        let mut lying = Reply::Had(None).to_plaintext(0).unwrap();
        lying[5] = 1;
        assert!(Reply::from_plaintext(Kind::Had, 0, &lying).is_err(), "nothing kept, yet a length");
        // found by fuzzing: a got for a wanted length of zero parsed, though none can be sealed
        assert!(Reply::from_plaintext(Kind::Got, 0, &[6, 0, 0, 0, 0, 0]).is_err());
        assert!(Reply::from_plaintext(Kind::Got, crate::scatter::MAX_PIECE + 1, &[6, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn keys_from_seeds_and_published_keys() {
        let a = XKey::from_seed(&[1; 32]).unwrap();
        assert_eq!(a.public(), XKey::from_seed(&[1; 32]).unwrap().public());
        assert_ne!(a.public(), XKey::from_seed(&[2; 32]).unwrap().public());
        let node = crate::Keypair::from_secret(&[9; 32]);
        let endpoint = node.public().0;
        let sig = node.sign(&xk_statement(&endpoint, &a.public(), 2_000));
        assert!(xk_valid(&endpoint, &a.public(), 2_000, &sig, 1_999));
        assert!(!xk_valid(&endpoint, &a.public(), 2_000, &sig, 2_000), "expired");
        assert!(!xk_valid(&endpoint, &a.public(), 2_001, &sig, 1_000), "another date");
        assert!(!xk_valid(&[1; 32], &a.public(), 2_000, &sig, 1_000), "another node");
        let capability = drop_journal_statement(&endpoint, &a.public(), 2_000);
        assert!(!crate::verify(&node.public(), &capability, &sig), "a key signature is not a durable capability");
        let proof = node.sign(&capability);
        assert!(crate::verify(&node.public(), &capability, &proof));
        assert!(!crate::verify(&node.public(), &drop_journal_statement(&endpoint, &a.public(), 2_001), &proof));
        assert!(!crate::verify(&node.public(), &drop_journal_statement(&endpoint, &[0; 32], 2_000), &proof));
    }

    #[test]
    fn three_hop_wraps_and_injections_open_only_where_they_are_sealed() {
        let exit = XKey::generate().unwrap();
        let holder = XKey::generate().unwrap();
        let (e, h) = ([1u8; 32], [2u8; 32]);
        let inner = seal_request(&h, &holder.public(), &Request::Has { loc: [3; 32], reply: [4; 32] }).unwrap();
        let wrap = seal_wrap(&e, &exit.public(), &h, Kind::Has, &inner).unwrap();
        let (to, kind, env) = open_wrap(&e, &exit, &wrap).unwrap();
        assert_eq!((to, kind), (h, Kind::Has));
        assert_eq!(open_request(Kind::Has, &h, &holder, &env).unwrap(), Request::Has { loc: [3; 32], reply: [4; 32] });
        assert!(open_wrap(&[9; 32], &exit, &wrap).is_err(), "bound to the exit's endpoint");
        assert!(open_wrap(&e, &holder, &wrap).is_err(), "only the exit opens it");
        assert!(open_request(Kind::Has, &e, &exit, &wrap).is_err(), "a wrap is no request");
        assert!(seal_wrap(&e, &exit.public(), &h, Kind::Had, &inner).is_err(), "only requests are wrapped");
        let wires = vec![vec![7u8; 300], vec![8u8; 2000]];
        let sealed = seal_inject(&e, &exit.public(), &wires).unwrap();
        assert_eq!(sealed.len(), 3 * 1024 + OVERHEAD, "padded to whole kibibytes");
        assert_eq!(open_inject(&e, &exit, &sealed).unwrap(), wires);
        assert!(open_inject(&e, &holder, &sealed).is_err());
        assert!(open_wrap(&e, &exit, &sealed).is_err(), "an injection is no wrap");
        assert!(seal_inject(&e, &exit.public(), &[]).is_err());
        assert!(seal_inject(&e, &exit.public(), &vec![vec![1u8]; MAX_INJECT + 1]).is_err());
    }

    #[test]
    fn the_suite_is_only_this_one() {
        assert!(Suite::supports_kem(KemAlgorithm::DhKemP256).is_err());
        assert!(Suite::supports_aead(AeadAlgorithm::Aes128Gcm).is_err());
        assert!(Suite::supports_kdf(KdfAlgorithm::HkdfSha512).is_err());
    }

    mod robust {
        use super::*;
        use proptest::prelude::*;

        const KINDS: [Kind; 8] = [Kind::Put, Kind::Get, Kind::Has, Kind::Drop, Kind::Stored, Kind::Got, Kind::Had, Kind::Dropped];

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(512))]
            /// Any bytes: no panic, and whatever parses writes back to the same bytes.
            #[test]
            fn plaintexts_parse_canonically(kind in 0usize..8, want in 1usize..400, bytes in proptest::collection::vec(any::<u8>(), 0..600)) {
                let kind = KINDS[kind];
                if let Ok(r) = Request::from_plaintext(kind, &bytes) {
                    prop_assert_eq!(r.to_plaintext().unwrap(), bytes.clone());
                }
                if let Ok(r) = Reply::from_plaintext(kind, want, &bytes) {
                    prop_assert_eq!(r.to_plaintext(want).unwrap(), bytes);
                }
            }

            #[test]
            fn envelopes_of_any_bytes_never_open(kind in 0usize..8, bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
                let key = XKey::from_seed(&[1; 32]).unwrap();
                prop_assert!(open_request(KINDS[kind], &ENDPOINT, &key, &bytes).is_err());
                prop_assert!(open_reply(KINDS[kind], &key, &[0; 32], 100, &bytes).is_err());
            }
        }

        #[test]
        fn tagged_plaintexts_parse_canonically() {
            // a valid header with random tails still never panics
            for kind in KINDS {
                let mut b = vec![kind.tag()];
                b.extend_from_slice(&[0; 200]);
                for len in [0usize, 1, 65, 69, 70, 129, 130, 160, 161, 201] {
                    let _ = Request::from_plaintext(kind, &b[..len.min(b.len())]);
                    let _ = Reply::from_plaintext(kind, 64, &b[..len.min(b.len())]);
                }
            }
        }
    }
}
