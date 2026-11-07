//! Scattered delivery, the pieces library (SCATTER.md §5, PROTOCOL.md §21). No network here:
//! sealing an object, cutting a generation of it into pieces any k of which rebuild it,
//! locators, piece hashes, drop secrets, and the sizing that keeps a party with less than the
//! community's share of places from rebuilding or withholding ([`size`]).
//!
//! ```text
//! content ──pad to its class──► plain ──XChaCha20-Poly1305(K_obj)──► sealed
//! sealed ──AONT(K_a,g)──► package ──split into k──► data pieces ──Reed–Solomon──► piece i, any i < 256
//! ```
//!
//! Every byte layout here is fixed by `core/vectors/scatter.json`.

pub mod aont;
pub mod rs;
pub mod size;

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

use crate::{CryptoError, random_bytes};

pub use hoi_types::scatter::{CLASSES, Class, LEVELS, MAX_M, MAX_N, MAX_PIECE, MIN_K, budget};

const OBJECT_AAD: &[u8] = b"hoi scatter object v1";
const NONCE: usize = 24;
const LEN_PREFIX: usize = 4;

/// A fresh object id: 32 random bytes. Not secret.
pub fn new_object_id() -> Result<[u8; 32], CryptoError> {
    random_bytes::<32>()
}

/// A fresh secret (an object key, a locator secret).
pub fn new_secret() -> Result<[u8; 32], CryptoError> {
    random_bytes::<32>()
}

fn object_aad(object: &[u8; 32], class: Class) -> Vec<u8> {
    let mut aad = Vec::with_capacity(OBJECT_AAD.len() + 33);
    aad.extend_from_slice(OBJECT_AAD);
    aad.extend_from_slice(object);
    aad.push(class.index());
    aad
}

/// The nonce an object is sealed with: synthetic, from the object key, the object id, its class
/// and the padded plaintext (`BLAKE3_keyed(derive_key("hoi scatter nonce v1", K_obj), object ‖
/// class ‖ plain)`, the first 24 bytes). The same content under the same key and id always seals
/// to the same bytes, so a member who kept the content can make the sealed object again
/// (SCATTER.md §5.10); different content never shares a nonce, even under a key someone
/// reused.
fn object_nonce(key: &[u8; 32], object: &[u8; 32], class: Class, plain: &[u8]) -> [u8; NONCE] {
    let mut nkey = blake3::derive_key("hoi scatter nonce v1", key);
    let mut h = blake3::Hasher::new_keyed(&nkey);
    nkey.zeroize();
    h.update(object);
    h.update(&[class.index()]);
    h.update(plain);
    let mut out = [0u8; NONCE];
    out.copy_from_slice(&h.finalize().as_bytes()[..NONCE]);
    out
}

/// Seals `content` under the object key, padded to its class, with its synthetic nonce: the
/// same content, id and key give the same sealed bytes.
pub fn seal_object(content: &[u8], object: &[u8; 32], key: &[u8; 32]) -> Result<(Class, Vec<u8>), CryptoError> {
    let class = Class::for_len(content.len()).ok_or(CryptoError::Format)?;
    let mut plain = vec![0u8; LEN_PREFIX + class.capacity()];
    plain[..LEN_PREFIX].copy_from_slice(&(content.len() as u32).to_be_bytes());
    plain[LEN_PREFIX..LEN_PREFIX + content.len()].copy_from_slice(content);
    let nonce = object_nonce(key, object, class, &plain);
    let cipher = XChaCha20Poly1305::new(key.into());
    let aad = object_aad(object, class);
    let ct = cipher.encrypt(&XNonce::from(nonce), Payload { msg: &plain, aad: &aad });
    plain.zeroize();
    let ct = ct.map_err(|_| CryptoError::Format)?;
    let mut out = Vec::with_capacity(class.sealed_len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    debug_assert_eq!(out.len(), class.sealed_len());
    Ok((class, out))
}

/// Opens a sealed object: the length must be its class's, the tag must verify, the padding
/// after the content must be zero, and the nonce must be the one the content seals with.
pub fn open_object(sealed: &[u8], object: &[u8; 32], class: Class, key: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() != class.sealed_len() {
        return Err(CryptoError::Format);
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XNonce::try_from(&sealed[..NONCE]).map_err(|_| CryptoError::Format)?;
    let aad = object_aad(object, class);
    let mut plain = cipher.decrypt(&nonce, Payload { msg: &sealed[NONCE..], aad: &aad }).map_err(|_| CryptoError::Open)?;
    let len = u32::from_be_bytes(plain[..LEN_PREFIX].try_into().expect("4 bytes")) as usize;
    let canonical: bool = object_nonce(key, object, class, &plain).ct_eq(&sealed[..NONCE]).into();
    let ok = canonical && len <= class.capacity() && plain[LEN_PREFIX + len..].iter().all(|&b| b == 0);
    let content = ok.then(|| plain[LEN_PREFIX..LEN_PREFIX + len].to_vec());
    plain.zeroize();
    content.ok_or(CryptoError::Format)
}

/// The sealed object's hash, fixed in generation 0's manifest: every later generation must
/// rebuild to these bytes.
pub fn sealed_hash(sealed: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("hoi scatter sealed v1");
    h.update(sealed);
    *h.finalize().as_bytes()
}

/// A piece's hash as the manifest lists it: 16 bytes of BLAKE3 over the object, generation,
/// index and bytes, so a piece can stand for no other.
pub fn piece_hash(object: &[u8; 32], generation: u8, index: u8, piece: &[u8]) -> [u8; 16] {
    let mut h = blake3::Hasher::new_derive_key("hoi scatter piece v1");
    h.update(object);
    h.update(&[generation, index]);
    h.update(&(piece.len() as u32).to_be_bytes());
    h.update(piece);
    let mut out = [0u8; 16];
    out.copy_from_slice(&h.finalize().as_bytes()[..16]);
    out
}

/// `loc_g,i = BLAKE3_keyed(derive_key("hoi scatter loc v1", S_g), object ‖ g ‖ i)`: where a
/// holder keeps piece i. Without the generation's secret `S_g`, which only the sealed manifest
/// carries, nobody can compute a sibling's locator from one they hold.
pub fn locator(loc_secret: &[u8; 32], object: &[u8; 32], generation: u8, index: u8) -> [u8; 32] {
    let mut key = blake3::derive_key("hoi scatter loc v1", loc_secret);
    let mut msg = [0u8; 34];
    msg[..32].copy_from_slice(object);
    msg[32] = generation;
    msg[33] = index;
    let out = *blake3::keyed_hash(&key, &msg).as_bytes();
    key.zeroize();
    out
}

/// The drop token for a locator, from the creator's own drop secret (the author's, kept in their
/// vault, or a repairer's): `D = BLAKE3_keyed(derive_key("hoi scatter drop v1", A), loc)`. Only
/// [`drop_hash`] of it travels with a piece; `D` itself only in a `drop`.
pub fn drop_token(drop_secret: &[u8; 32], loc: &[u8; 32]) -> [u8; 32] {
    let mut key = blake3::derive_key("hoi scatter drop v1", drop_secret);
    let out = *blake3::keyed_hash(&key, loc).as_bytes();
    key.zeroize();
    out
}

/// What a holder keeps to check a drop token later.
pub fn drop_hash(token: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("hoi scatter drop hash v1");
    h.update(token);
    *h.finalize().as_bytes()
}

/// Whether `token` opens `hash`, in constant time.
pub fn drop_token_matches(token: &[u8; 32], hash: &[u8; 32]) -> bool {
    use subtle::ConstantTimeEq;
    drop_hash(token).ct_eq(hash).into()
}

/// The id of a community drop key, as a piece is stored with it.
pub fn drop_key_id(public: &[u8; 32]) -> [u8; 16] {
    let mut h = blake3::Hasher::new_derive_key("hoi scatter drop key v1");
    h.update(public);
    let mut out = [0u8; 16];
    out.copy_from_slice(&h.finalize().as_bytes()[..16]);
    out
}

/// What a moderator's device signs with the community drop key to drop the piece at `loc`.
pub fn drop_statement(loc: &[u8; 32]) -> Vec<u8> {
    let mut s = b"hoi/scatter-drop/1\n".to_vec();
    s.extend_from_slice(loc);
    s
}

/// Work every piece below 1 KiB carries; each doubling of its length past that adds one bit.
pub const STAMP_BASE_BITS: u32 = 8;

/// The work a put of a piece of `len` bytes must carry: storage costs its sender in proportion
/// to what it asks a holder to keep (SCATTER.md §5.11).
pub fn stamp_bits(len: usize) -> u32 {
    let kib = (len as u64).div_ceil(1024);
    STAMP_BASE_BITS + (u64::BITS - kib.saturating_sub(1).leading_zeros())
}

/// The hash a piece stamp is judged by: bound to the holder, the locator, the length and the
/// expiry day, so a stamp made for one holder or piece is worth nothing for another.
pub fn stamp_hash(holder: &[u8; 32], loc: &[u8; 32], len: usize, expiry_day: u32, nonce: u64) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("hoi scatter stamp v1");
    h.update(holder);
    h.update(loc);
    h.update(&(len as u32).to_be_bytes());
    h.update(&expiry_day.to_be_bytes());
    h.update(&nonce.to_be_bytes());
    *h.finalize().as_bytes()
}

/// Whether `nonce` stamps a put of `len` bytes under `loc` at `holder` until `expiry_day`.
pub fn stamp_ok(holder: &[u8; 32], loc: &[u8; 32], len: usize, expiry_day: u32, nonce: u64) -> bool {
    crate::pow::leading_zero_bits(&stamp_hash(holder, loc, len, expiry_day, nonce)) >= stamp_bits(len)
}

/// Finds a stamp, from nonce 0. A 1 MiB piece needs 18 bits: about a quarter million hashes.
pub fn mine_stamp(holder: &[u8; 32], loc: &[u8; 32], len: usize, expiry_day: u32) -> u64 {
    (0..).find(|&n| stamp_ok(holder, loc, len, expiry_day, n)).expect("nonce space is large enough")
}

/// One generation's data pieces, from which any piece follows: made from a sealed object
/// (by the author or a repairer making a generation) or rebuilt from any k pieces (by a reader,
/// or a repairer making fresh indexes of the same generation).
pub struct Shards {
    class: Class,
    code: rs::Code,
    data: Vec<Vec<u8>>,
}

impl Drop for Shards {
    fn drop(&mut self) {
        for d in &mut self.data {
            d.zeroize();
        }
    }
}

fn check_k(class: Class, k: usize) -> Result<(), CryptoError> {
    if !(MIN_K..=MAX_N).contains(&k) || class.piece_len(k) > MAX_PIECE {
        return Err(CryptoError::Format);
    }
    Ok(())
}

impl Shards {
    /// A new generation of `sealed`, under a fresh transform key.
    pub fn new(sealed: &[u8], class: Class, k: usize) -> Result<Shards, CryptoError> {
        let mut key = random_bytes::<32>()?;
        let s = Shards::with_key(sealed, class, k, &key);
        key.zeroize();
        s
    }

    /// [`Shards::new`] with the transform key given: for conformance vectors only. A key must
    /// never wrap two generations.
    #[cfg(feature = "deterministic")]
    pub fn new_with_key(sealed: &[u8], class: Class, k: usize, aont_key: &[u8; 32]) -> Result<Shards, CryptoError> {
        Shards::with_key(sealed, class, k, aont_key)
    }

    fn with_key(sealed: &[u8], class: Class, k: usize, aont_key: &[u8; 32]) -> Result<Shards, CryptoError> {
        check_k(class, k)?;
        if sealed.len() != class.sealed_len() {
            return Err(CryptoError::Format);
        }
        let mut package = aont::package(sealed, aont_key);
        let len = class.piece_len(k);
        package.resize(len * k, 0);
        let data = package.chunks(len).map(|c| c.to_vec()).collect();
        package.zeroize();
        Ok(Shards { class, code: rs::Code::new(k), data })
    }

    /// Rebuilds a generation from exactly k pieces with distinct indexes, each of the class's
    /// piece length. Check each piece against the manifest's hash before calling this.
    pub fn rebuild(class: Class, k: usize, pieces: &[(u8, &[u8])]) -> Result<Shards, CryptoError> {
        check_k(class, k)?;
        let len = class.piece_len(k);
        if pieces.len() != k || pieces.iter().any(|(_, p)| p.len() != len) {
            return Err(CryptoError::Format);
        }
        let code = rs::Code::new(k);
        let indexed: Vec<(usize, &[u8])> = pieces.iter().map(|&(i, p)| (i as usize, p)).collect();
        let data = code.decode(&indexed).ok_or(CryptoError::Format)?;
        let s = Shards { class, code, data };
        // the zero padding after the package is part of the code word: refuse anything else
        let pad = len * k - class.package_len();
        if s.data[k - 1][len - pad..].iter().any(|&b| b != 0) {
            return Err(CryptoError::Open);
        }
        Ok(s)
    }

    pub fn k(&self) -> usize {
        self.code.k()
    }

    pub fn class(&self) -> Class {
        self.class
    }

    /// Piece `index` of this generation.
    pub fn piece(&self, index: u8) -> Vec<u8> {
        let refs: Vec<&[u8]> = self.data.iter().map(|d| d.as_slice()).collect();
        self.code.encode(&refs, index as usize)
    }

    /// The sealed object: undoes the transform (its canary must be zero).
    pub fn sealed(&self) -> Result<Vec<u8>, CryptoError> {
        let mut package: Vec<u8> = self.data.concat();
        package.truncate(self.class.package_len());
        let sealed = aont::unpackage(&package);
        package.zeroize();
        let sealed = sealed?;
        if sealed.len() != self.class.sealed_len() {
            return Err(CryptoError::Format);
        }
        Ok(sealed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const OBJ: [u8; 32] = [3; 32];
    const KEY: [u8; 32] = [4; 32];

    #[test]
    fn piece_lengths() {
        // the largest object's pieces stay under the limit from k = 5 on
        let big = Class::from_index(6).unwrap();
        assert!(big.piece_len(4) > MAX_PIECE);
        assert!(big.piece_len(5) <= MAX_PIECE);
        let small = Class::from_index(0).unwrap();
        assert_eq!(small.piece_len(15), (4096 + 4 + 24 + 16 + aont::OVERHEAD).div_ceil(15));
    }

    #[test]
    fn seal_and_open() {
        let (class, sealed) = seal_object(b"hoi, in pieces", &OBJ, &KEY).unwrap();
        assert_eq!(class, Class::from_index(0).unwrap());
        assert_eq!(sealed.len(), class.sealed_len());
        assert_eq!(open_object(&sealed, &OBJ, class, &KEY).unwrap(), b"hoi, in pieces");
        assert_eq!(open_object(&sealed, &[9; 32], class, &KEY), Err(CryptoError::Open), "bound to the object id");
        assert_eq!(open_object(&sealed, &OBJ, class, &[9; 32]), Err(CryptoError::Open));
        assert_eq!(open_object(&sealed, &OBJ, Class::from_index(1).unwrap(), &KEY), Err(CryptoError::Format), "and to its class");
        let mut bad = sealed.clone();
        bad[40] ^= 1;
        assert_eq!(open_object(&bad, &OBJ, class, &KEY), Err(CryptoError::Open));
        // the same content, id and key seal to the same bytes (a synthetic nonce);
        // other content, another id or another key to others, with another nonce
        assert_eq!(seal_object(b"x", &OBJ, &KEY).unwrap(), seal_object(b"x", &OBJ, &KEY).unwrap());
        let x = seal_object(b"x", &OBJ, &KEY).unwrap().1;
        for other in [seal_object(b"y", &OBJ, &KEY).unwrap().1, seal_object(b"x", &[9; 32], &KEY).unwrap().1, seal_object(b"x", &OBJ, &[9; 32]).unwrap().1] {
            assert_ne!(other[..NONCE], x[..NONCE]);
        }
    }

    #[test]
    fn a_sealed_object_whose_nonce_is_not_its_own_is_refused() {
        // sealed by hand with another nonce: the tag verifies, but the object is not canonical
        let class = Class::for_len(5).unwrap();
        let mut plain = vec![0u8; LEN_PREFIX + class.capacity()];
        plain[..LEN_PREFIX].copy_from_slice(&5u32.to_be_bytes());
        plain[LEN_PREFIX..LEN_PREFIX + 5].copy_from_slice(b"hello");
        let nonce = [3u8; NONCE];
        let ct = XChaCha20Poly1305::new((&KEY).into()).encrypt(&XNonce::from(nonce), Payload { msg: &plain, aad: &object_aad(&OBJ, class) }).unwrap();
        let mut sealed = nonce.to_vec();
        sealed.extend_from_slice(&ct);
        assert_eq!(open_object(&sealed, &OBJ, class, &KEY), Err(CryptoError::Format));
        let (class2, good) = seal_object(b"hello", &OBJ, &KEY).unwrap();
        assert_eq!(class2, class);
        assert_eq!(open_object(&good, &OBJ, class, &KEY).unwrap(), b"hello");
    }

    #[test]
    fn a_generation_rebuilds_from_any_k_and_makes_fresh_indexes() {
        let (class, sealed) = seal_object(&[0x5a; 3000], &OBJ, &KEY).unwrap();
        let (k, m) = (15usize, 35u8);
        let gen0 = Shards::new(&sealed, class, k).unwrap();
        let pieces: Vec<Vec<u8>> = (0..m).map(|i| gen0.piece(i)).collect();
        assert!(pieces.iter().all(|p| p.len() == class.piece_len(k)));
        // the last k pieces rebuild it, including indexes made "later"
        let tail: Vec<(u8, &[u8])> = (m - k as u8..m).map(|i| (i, pieces[i as usize].as_slice())).collect();
        let back = Shards::rebuild(class, k, &tail).unwrap();
        assert_eq!(back.sealed().unwrap(), sealed);
        // a repairer's fresh index from the rebuilt shards equals the author's
        assert_eq!(back.piece(200), gen0.piece(200));
        // k − 1 pieces are refused
        assert_eq!(Shards::rebuild(class, k, &tail[1..]).err(), Some(CryptoError::Format));
        // a swapped piece decodes to garbage the transform refuses
        let mut swapped: Vec<(u8, Vec<u8>)> = tail.iter().map(|&(i, p)| (i, p.to_vec())).collect();
        swapped[3].1[0] ^= 0x80;
        let refs: Vec<(u8, &[u8])> = swapped.iter().map(|(i, p)| (*i, p.as_slice())).collect();
        assert!(Shards::rebuild(class, k, &refs).and_then(|s| s.sealed()).is_err());
    }

    #[test]
    fn generations_never_combine() {
        let (class, sealed) = seal_object(&[1; 1000], &OBJ, &KEY).unwrap();
        let k = 5;
        let g0 = Shards::new(&sealed, class, k).unwrap();
        let g1 = Shards::new(&sealed, class, k).unwrap();
        // three pieces of one generation and two of another rebuild nothing
        let mixed: Vec<(u8, Vec<u8>)> = vec![(0, g0.piece(0)), (1, g0.piece(1)), (2, g0.piece(2)), (7, g1.piece(7)), (8, g1.piece(8))];
        let refs: Vec<(u8, &[u8])> = mixed.iter().map(|(i, p)| (*i, p.as_slice())).collect();
        assert!(Shards::rebuild(class, k, &refs).and_then(|s| s.sealed()).is_err());
    }

    #[test]
    fn locators_and_drops() {
        let s = [8u8; 32];
        let a = locator(&s, &OBJ, 0, 0);
        assert_ne!(a, locator(&s, &OBJ, 0, 1));
        assert_ne!(a, locator(&s, &OBJ, 1, 0));
        assert_ne!(a, locator(&[9; 32], &OBJ, 0, 0));
        let d = drop_token(&[5; 32], &a);
        assert!(drop_token_matches(&d, &drop_hash(&d)));
        assert!(!drop_token_matches(&drop_token(&[6; 32], &a), &drop_hash(&d)));
        assert_ne!(piece_hash(&OBJ, 0, 1, b"p"), piece_hash(&OBJ, 0, 2, b"p"));
    }

    #[test]
    fn stamps_cost_by_size_and_bind_everything() {
        assert_eq!(stamp_bits(1), 8);
        assert_eq!(stamp_bits(1024), 8);
        assert_eq!(stamp_bits(1025), 9);
        assert_eq!(stamp_bits(280 * 1024), 17);
        assert_eq!(stamp_bits(MAX_PIECE), 18);
        let (h, l) = ([1u8; 32], [2u8; 32]);
        let n = mine_stamp(&h, &l, 300, 20_000);
        assert!(stamp_ok(&h, &l, 300, 20_000, n));
        // the same nonce means nothing for another holder, piece, length or day (with high odds)
        let elsewhere = [
            stamp_ok(&[9; 32], &l, 300, 20_000, n),
            stamp_ok(&h, &[9; 32], 300, 20_000, n),
            stamp_ok(&h, &l, 301, 20_000, n),
            stamp_ok(&h, &l, 300, 20_001, n),
        ];
        assert!(elsewhere.iter().filter(|&&x| x).count() <= 1);
    }

    #[test]
    fn k_limits() {
        let (class, sealed) = seal_object(&[1; 10], &OBJ, &KEY).unwrap();
        assert!(Shards::new(&sealed, class, 2).is_err());
        assert!(Shards::new(&sealed, class, MAX_N + 1).is_err());
        assert!(Shards::new(&sealed[1..], class, 3).is_err());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        /// Whatever pieces turn up, rebuilding never panics and never yields a sealed object of
        /// the wrong length.
        #[test]
        fn rebuilding_any_bytes_is_safe(class in 0u8..2, k in 0usize..12, idx in proptest::collection::vec(any::<u8>(), 0..12), fill in any::<u8>(), cut in 0usize..3) {
            let class = Class::from_index(class).unwrap();
            let len = class.piece_len(k.max(1)).saturating_sub(cut);
            let bytes: Vec<Vec<u8>> = idx.iter().map(|&i| vec![fill ^ i; len]).collect();
            let refs: Vec<(u8, &[u8])> = idx.iter().zip(&bytes).map(|(&i, b)| (i, b.as_slice())).collect();
            if let Ok(sealed) = Shards::rebuild(class, k, &refs).and_then(|s| s.sealed()) {
                prop_assert_eq!(sealed.len(), class.sealed_len());
            }
        }

        #[test]
        fn unpackaging_and_opening_any_bytes_is_safe(bytes in proptest::collection::vec(any::<u8>(), 0..5000)) {
            let _ = aont::unpackage(&bytes);
            let class = Class::from_index(0).unwrap();
            let _ = open_object(&bytes, &OBJ, class, &KEY);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]
        #[test]
        fn round_trips(len in 0usize..20_000, k in 3usize..30, skip in 0usize..30) {
            let content: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let (class, sealed) = seal_object(&content, &OBJ, &KEY).unwrap();
            let s = Shards::new(&sealed, class, k).unwrap();
            let chosen: Vec<(u8, Vec<u8>)> = (skip..skip + k).map(|i| (i as u8, s.piece(i as u8))).collect();
            let refs: Vec<(u8, &[u8])> = chosen.iter().map(|(i, p)| (*i, p.as_slice())).collect();
            let back = Shards::rebuild(class, k, &refs).unwrap().sealed().unwrap();
            prop_assert_eq!(open_object(&back, &OBJ, class, &KEY).unwrap(), content);
        }
    }
}
