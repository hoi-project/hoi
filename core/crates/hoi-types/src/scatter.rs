//! Scattered delivery's shapes (SCATTER.md §5, PROTOCOL.md §21): size classes, the limits every
//! generation keeps, and the manifest a pointer record carries. Pure: the cryptography is in
//! `hoi_crypto::scatter`.

use std::fmt;

use crate::cbor::{self, Value};
use crate::record::Fields;
use crate::{Id, TypesError};

/// What an object may hold, by class: 4, 16, 64 and 256 KiB, 1, 2 and 4 MiB. Longer media are cut
/// into objects of 4 MiB first.
pub const CLASSES: [usize; 7] = [4 << 10, 16 << 10, 64 << 10, 256 << 10, 1 << 20, 2 << 20, 4 << 20];
/// No piece is longer than this.
pub const MAX_PIECE: usize = 1 << 20;
/// The fewest pieces that rebuild a generation.
pub const MIN_K: usize = 3;
/// The most pieces a generation places at first.
pub const MAX_N: usize = 160;
/// The most indexes a generation may ever use: `MAX_N` and its budget.
pub const MAX_M: usize = MAX_N + MAX_N / 4;
/// The levels a community may choose, in percent of places; the first is the default and floor.
pub const LEVELS: [u8; 3] = [10, 15, 20];

/// A sealed object: the XChaCha20-Poly1305 nonce, a 4-byte length, the padded content, the tag.
const SEALED_OVERHEAD: usize = 24 + 4 + 16;
/// What the all-or-nothing transform adds: a 16-byte canary and the 32-byte masked key.
pub const AONT_OVERHEAD: usize = 16 + 32;

/// The index budget of a generation that places `n` at first: a quarter more.
pub fn budget(n: usize) -> usize {
    n + n.div_ceil(4)
}

/// An object's size class, by index into [`CLASSES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Class(u8);

impl Class {
    /// The smallest class that holds `len` bytes of content.
    pub fn for_len(len: usize) -> Option<Class> {
        CLASSES.iter().position(|&c| len <= c).map(|i| Class(i as u8))
    }
    pub fn from_index(i: u8) -> Option<Class> {
        ((i as usize) < CLASSES.len()).then_some(Class(i))
    }
    pub fn index(self) -> u8 {
        self.0
    }
    /// Content bytes an object of this class holds.
    pub fn capacity(self) -> usize {
        CLASSES[self.0 as usize]
    }
    /// Length of the sealed object.
    pub fn sealed_len(self) -> usize {
        self.capacity() + SEALED_OVERHEAD
    }
    /// Length of the package the transform makes of it.
    pub fn package_len(self) -> usize {
        self.sealed_len() + AONT_OVERHEAD
    }
    /// Length of each piece when the package is cut into `k` (the last data piece padded with
    /// zeros).
    pub fn piece_len(self, k: usize) -> usize {
        self.package_len().div_ceil(k.max(1))
    }
}

/// The classes of the objects `len` bytes of a media item go in, each holding `header` bytes
/// besides its share (S6): one object of the smallest class that holds the rest whenever that
/// class is at most twice the rest, else an object of the largest class it fills and the rest
/// again. A 300 KB picture goes in a 256 KiB and a 64 KiB object (1.1× its size), not one of
/// 1 MiB.
pub fn fit(len: usize, header: usize) -> Vec<Class> {
    let mut out = Vec::new();
    let mut left = len;
    loop {
        match Class::for_len(left + header) {
            Some(c) if c.capacity() <= 2 * (left + header) || c.index() == 0 => {
                out.push(c);
                return out;
            }
            _ => {
                let full = (0..CLASSES.len() as u8)
                    .rev()
                    .filter_map(Class::from_index)
                    .find(|c| c.capacity() > header && c.capacity() - header <= left)
                    .unwrap_or(Class(0));
                out.push(full);
                left = left.saturating_sub(full.capacity().saturating_sub(header));
                if left == 0 {
                    return out;
                }
            }
        }
    }
}

/// One generation of an object, as its pointer record carries it, sealed (SCATTER.md §7). It
/// holds the object key and the generation's locator secret: whoever reads a manifest can fetch
/// and open the object, so it only ever travels sealed to members (or under the community's read
/// key, for open communities).
#[derive(Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The object's id (random, not secret).
    pub object: [u8; 32],
    /// BLAKE3 of the sealed object, the same in every generation.
    pub sealed: [u8; 32],
    pub class: Class,
    pub generation: u8,
    /// The level it was sized for, in percent.
    pub level: u8,
    /// The places the device that made it could choose from.
    pub places: u32,
    pub k: u16,
    pub n: u16,
    pub m: u16,
    /// The hash of each index placed so far, index 0 first; at least `n`, at most `m`.
    pub hashes: Vec<[u8; 16]>,
    /// The holder of each of those indexes: endpoint ids, each at most once.
    pub holders: Vec<Id>,
    /// The generation's locator secret `S_g`.
    pub loc_secret: [u8; 32],
    /// The object key `K_obj`.
    pub key: [u8; 32],
    /// When its pieces expire (Unix seconds).
    pub expires_at: u64,
    /// The X25519 key each holder had when it took its piece, in index order (S6): a reader whose
    /// forwarders do not vouch for a holder seals to it, and a forwarder passes the request on
    /// to one of its links that does. Empty, or one per holder.
    pub keys: Vec<[u8; 32]>,
}

impl fmt::Debug for Manifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Manifest")
            .field("object", &crate::hex_encode(&self.object[..4]))
            .field("generation", &self.generation)
            .field("k", &self.k)
            .field("n", &self.n)
            .field("m", &self.m)
            .field("placed", &self.hashes.len())
            .finish_non_exhaustive()
    }
}

impl Drop for Manifest {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.key.zeroize();
        self.loc_secret.zeroize();
    }
}

impl Manifest {
    /// The floors every manifest keeps, whatever the level (PROTOCOL.md §21). Whether a
    /// generation meets its level is `hoi_crypto::scatter::size::reading`.
    pub fn check(&self) -> Result<(), TypesError> {
        let (k, n, m) = (self.k as usize, self.n as usize, self.m as usize);
        let bad = |s: &str| Err(TypesError::Field(format!("manifest: {s}")));
        if !LEVELS.contains(&self.level) {
            return bad("unknown level");
        }
        if k < MIN_K || k + 2 > n || n > MAX_N || m < n || m > budget(n) {
            return bad("k, n and m out of range");
        }
        if self.places < m as u32 {
            return bad("more indexes than places");
        }
        if self.class.piece_len(k) > MAX_PIECE {
            return bad("pieces too long for their class");
        }
        let placed = self.hashes.len();
        if placed != self.holders.len() || placed < n || placed > m {
            return bad("hashes and holders must list every index placed");
        }
        if !self.keys.is_empty() && self.keys.len() != placed {
            return bad("a key for every holder, or none");
        }
        let mut holders: Vec<&Id> = self.holders.iter().collect();
        holders.sort();
        if holders.windows(2).any(|w| w[0] == w[1]) {
            return bad("a holder holds at most one index of a generation");
        }
        Ok(())
    }

    pub fn to_value(&self) -> Value {
        let mut e = vec![
            ("v", Value::U(1)),
            ("o", Value::B(self.object.to_vec())),
            ("s", Value::B(self.sealed.to_vec())),
            ("c", Value::U(self.class.0 as u64)),
            ("g", Value::U(self.generation as u64)),
            ("l", Value::U(self.level as u64)),
            ("p", Value::U(self.places as u64)),
            ("k", Value::U(self.k as u64)),
            ("n", Value::U(self.n as u64)),
            ("m", Value::U(self.m as u64)),
            ("h", Value::A(self.hashes.iter().map(|h| Value::B(h.to_vec())).collect())),
            ("d", Value::A(self.holders.iter().map(|h| Value::B(h.0.to_vec())).collect())),
            ("x", Value::B(self.loc_secret.to_vec())),
            ("y", Value::B(self.key.to_vec())),
            ("e", Value::U(self.expires_at)),
        ];
        if !self.keys.is_empty() {
            e.push(("z", Value::A(self.keys.iter().map(|k| Value::B(k.to_vec())).collect())));
        }
        Value::map(e)
    }

    /// Canonical DAG-CBOR with one-letter keys.
    pub fn encode(&self) -> Vec<u8> {
        self.to_value().encode()
    }

    /// Decodes strictly (unknown fields refused) and checks the floors.
    pub fn decode(bytes: &[u8]) -> Result<Manifest, TypesError> {
        // A manifest of the largest generation: about 85 bytes per index, keys included.
        if bytes.len() > 512 + MAX_M * 100 {
            return Err(TypesError::Field("manifest: too long".into()));
        }
        let mut f = Fields::new(cbor::decode(bytes)?, "manifest")?;
        let v = f.u64("v")?;
        if v != 1 {
            return Err(TypesError::Version(v));
        }
        let small = |x: u64, max: u64, what: &str| -> Result<u64, TypesError> {
            if x <= max { Ok(x) } else { Err(TypesError::Field(format!("manifest: {what} out of range"))) }
        };
        let arr32 = |b: Vec<u8>| -> Result<[u8; 32], TypesError> { b.as_slice().try_into().map_err(|_| TypesError::Length { want: 32, got: b.len() }) };
        let object = arr32(f.bytes("o")?)?;
        let sealed = arr32(f.bytes("s")?)?;
        let class = Class::from_index(small(f.u64("c")?, 255, "class")? as u8).ok_or_else(|| TypesError::Field("manifest: unknown class".into()))?;
        let generation = small(f.u64("g")?, 255, "generation")? as u8;
        let level = small(f.u64("l")?, 255, "level")? as u8;
        let places = f.u32("p")?;
        let k = small(f.u64("k")?, MAX_N as u64, "k")? as u16;
        let n = small(f.u64("n")?, MAX_N as u64, "n")? as u16;
        let m = small(f.u64("m")?, MAX_M as u64, "m")? as u16;
        let list = |v: Value, what: &str| -> Result<Vec<Vec<u8>>, TypesError> {
            match v {
                Value::A(items) if items.len() <= MAX_M => items
                    .into_iter()
                    .map(|i| match i {
                        Value::B(b) => Ok(b),
                        _ => Err(TypesError::Field(format!("manifest: {what} must be bytes"))),
                    })
                    .collect(),
                _ => Err(TypesError::Field(format!("manifest: {what} must be a short array"))),
            }
        };
        let hashes = list(f.take("h")?, "hashes")?
            .into_iter()
            .map(|b| <[u8; 16]>::try_from(b.as_slice()).map_err(|_| TypesError::Length { want: 16, got: b.len() }))
            .collect::<Result<Vec<_>, _>>()?;
        let holders = list(f.take("d")?, "holders")?.into_iter().map(|b| Id::from_slice(&b)).collect::<Result<Vec<_>, _>>()?;
        let loc_secret = arr32(f.bytes("x")?)?;
        let key = arr32(f.bytes("y")?)?;
        let expires_at = f.u64("e")?;
        let keys = match f.opt("z") {
            Some(v) => list(v, "keys")?.into_iter().map(arr32).collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        };
        f.done()?;
        let m = Manifest { object, sealed, class, generation, level, places, k, n, m, hashes, holders, loc_secret, key, expires_at, keys };
        m.check()?;
        Ok(m)
    }
}

/// What a later manifest of the same object is to an earlier one (SCATTER.md §5.10, rule 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Update {
    /// The same generation with fresh indexes placed by a repair.
    Extend,
    /// The next generation: fresh transform key, locator secret, indexes and places.
    Regenerate,
}

impl Manifest {
    /// Whether `next` may follow `self` for the same object, and how: never another object or
    /// another sealed object, key, or class; within a generation, nothing but fresh indexes
    /// appended within the budget, every earlier index and holder left as it was; a new
    /// generation only as the next one, with a locator secret of its own, kept no shorter. A
    /// reader also checks the new generation's size meets a level (`hoi_crypto`) and that at
    /// most one regeneration a day is taken (crate::keep in the core).
    pub fn update_to(&self, next: &Manifest) -> Result<Update, TypesError> {
        let bad = |s: &str| Err(TypesError::Field(format!("update: {s}")));
        next.check()?;
        if next.object != self.object || next.sealed != self.sealed || next.key != self.key || next.class != self.class {
            return bad("another object");
        }
        if next.generation == self.generation {
            let same = next.level == self.level
                && next.places == self.places
                && (next.k, next.n, next.m) == (self.k, self.n, self.m)
                && next.loc_secret == self.loc_secret
                && next.expires_at == self.expires_at;
            if !same {
                return bad("a repair changes nothing but adds indexes");
            }
            let placed = self.hashes.len();
            if next.hashes.len() <= placed || next.hashes[..placed] != self.hashes[..] || next.holders[..placed] != self.holders[..] {
                return bad("a repair keeps every index placed and adds fresh ones");
            }
            if !self.keys.is_empty() && (next.keys.len() < placed || next.keys[..placed] != self.keys[..]) {
                return bad("a repair keeps every holder's key");
            }
            return Ok(Update::Extend);
        }
        if self.generation.checked_add(1) != Some(next.generation) {
            return bad("generations follow one another");
        }
        if next.loc_secret == self.loc_secret || next.expires_at < self.expires_at {
            return bad("a new generation has secrets of its own and lives no shorter");
        }
        Ok(Update::Regenerate)
    }
}

/// A pointer to an object kept in pieces, as members pass it to each other, always sealed
/// (inside MLS for private groups): a generation's manifest and when it was made. The first for
/// an object makes it known; later ones are updates (`Manifest::update_to`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pointer {
    pub made_at: u64,
    pub manifest: Manifest,
}

impl Pointer {
    /// Canonical DAG-CBOR: `{ v: 1, t: made_at, m: manifest }`.
    pub fn encode(&self) -> Vec<u8> {
        Value::map([("v", Value::U(1)), ("t", Value::U(self.made_at)), ("m", self.manifest.to_value())]).encode()
    }

    /// Decodes strictly and checks the manifest's floors.
    pub fn decode(bytes: &[u8]) -> Result<Pointer, TypesError> {
        if bytes.len() > 1024 + MAX_M * 64 {
            return Err(TypesError::Field("pointer: too long".into()));
        }
        let mut f = Fields::new(cbor::decode(bytes)?, "pointer")?;
        let v = f.u64("v")?;
        if v != 1 {
            return Err(TypesError::Version(v));
        }
        let made_at = f.u64("t")?;
        let manifest = Manifest::decode(&f.take("m")?.encode())?;
        f.done()?;
        Ok(Pointer { made_at, manifest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            object: [1; 32],
            sealed: [2; 32],
            class: Class(0),
            generation: 0,
            level: 10,
            places: 5000,
            k: 15,
            n: 28,
            m: 35,
            hashes: (0..28u8).map(|i| [i; 16]).collect(),
            holders: (0..28u8).map(|i| Id([i; 32])).collect(),
            loc_secret: [3; 32],
            key: [4; 32],
            expires_at: 1_790_000_000,
            keys: Vec::new(),
        }
    }

    #[test]
    fn media_fits_its_classes_closely() {
        let objects = |len| fit(len, 300);
        assert_eq!(objects(300_000).iter().map(|c| c.capacity()).collect::<Vec<_>>(), vec![256 * 1024, 64 * 1024]);
        assert_eq!(objects(260_000).len(), 1);
        for len in [1, 4000, 70_000, 300_000, 1 << 20, (1 << 20) + 100_000, 3 << 20, 16 << 20] {
            let v = objects(len);
            let held: usize = v.iter().map(|c| c.capacity() - 300).sum();
            assert!(held >= len, "{len}");
            assert!(held <= 2 * len + 4096, "{len}: {held}");
            assert!(v.len() <= 2 + len / (4 << 20) + 2, "{len}: {}", v.len());
        }
    }

    #[test]
    fn classes() {
        assert_eq!(Class::for_len(4096), Some(Class(0)));
        assert_eq!(Class::for_len(4097), Some(Class(1)));
        assert_eq!(Class::for_len((4 << 20) + 1), None);
        assert_eq!(Class(0).package_len(), 4096 + 44 + 48);
        assert!(Class(6).piece_len(4) > MAX_PIECE && Class(6).piece_len(5) <= MAX_PIECE);
        assert_eq!(budget(28), 35);
    }

    #[test]
    fn round_trip_and_size() {
        let m = sample();
        let b = m.encode();
        assert!(b.len() < 1900, "{} bytes at 15 of 28", b.len());
        assert_eq!(Manifest::decode(&b).unwrap(), m);
        assert!(!format!("{m:?}").contains("0404"), "secrets stay out of Debug");
    }

    #[test]
    fn floors() {
        let ok = |f: &dyn Fn(&mut Manifest)| {
            let mut m = sample();
            f(&mut m);
            Manifest::decode(&m.encode()).is_ok()
        };
        assert!(!ok(&|m| m.level = 12));
        assert!(!ok(&|m| m.k = 2));
        assert!(!ok(&|m| m.k = 27), "n ≥ k + 2");
        assert!(!ok(&|m| m.m = 36), "budget");
        assert!(!ok(&|m| m.places = 34));
        assert!(!ok(&|m| m.holders[3] = m.holders[4]), "distinct holders");
        assert!(!ok(&|m| m.hashes.truncate(27)), "every index listed");
        assert!(
            !ok(&|m| {
                m.class = Class(6);
                m.k = 4;
                m.n = 6;
                m.m = 7;
                m.hashes.truncate(6);
                m.holders.truncate(6);
            }),
            "piece too long"
        );
        assert!(
            ok(&|m| {
                m.hashes.push([9; 16]);
                m.holders.push(Id([99; 32]));
            }),
            "a repaired index"
        );
    }

    #[test]
    fn strict_decoding() {
        let entries = |v: Value| match v {
            Value::M(e) => e,
            _ => unreachable!(),
        };
        let mut e = entries(sample().to_value());
        e.push(("q".into(), Value::U(0)));
        assert!(Manifest::decode(&Value::map(e).encode()).unwrap_err().to_string().contains("unknown field"));
        // holders' keys: one per holder, or none
        let mut with = sample();
        with.keys = vec![[5; 32]; with.holders.len()];
        assert_eq!(Manifest::decode(&with.encode()).unwrap().keys, with.keys);
        with.keys.pop();
        assert!(Manifest::decode(&with.encode()).is_err(), "a key for every holder");
        let mut e = entries(sample().to_value());
        e.push(("z".into(), Value::U(0)));
        assert!(Manifest::decode(&Value::map(e).encode()).is_err(), "keys are an array of keys");
        let mut e = entries(sample().to_value());
        e.retain(|(k, _)| k != "v");
        e.push(("v".into(), Value::U(2)));
        assert_eq!(Manifest::decode(&Value::map(e).encode()), Err(TypesError::Version(2)));
        let mut e = entries(sample().to_value());
        e.retain(|(k, _)| k != "y");
        assert!(Manifest::decode(&Value::map(e).encode()).is_err(), "missing key");
        assert!(Manifest::decode(&[0xa0]).is_err());
        assert!(Manifest::decode(&vec![0u8; 20_000]).is_err());
    }

    #[test]
    fn updates_extend_or_regenerate_and_nothing_else() {
        let a = sample();
        let mut ext = sample();
        ext.hashes.push([9; 16]);
        ext.holders.push(Id([99; 32]));
        assert_eq!(a.update_to(&ext), Ok(Update::Extend));
        assert!(ext.update_to(&a).is_err(), "never takes an index back");
        assert!(a.update_to(&a).is_err(), "nothing new");
        let mut moved = ext.clone();
        moved.holders[3] = Id([100; 32]);
        assert!(a.update_to(&moved).is_err(), "an earlier index stays where it was");
        let mut regen = sample();
        regen.generation = 1;
        regen.loc_secret = [5; 32];
        regen.holders = (0..28u8).map(|i| Id([i + 100; 32])).collect();
        assert_eq!(a.update_to(&regen), Ok(Update::Regenerate));
        let mut same_secret = regen.clone();
        same_secret.loc_secret = a.loc_secret;
        assert!(a.update_to(&same_secret).is_err());
        let mut skip = regen.clone();
        skip.generation = 2;
        assert!(a.update_to(&skip).is_err(), "generations follow one another");
        for f in [|m: &mut Manifest| m.object = [7; 32], |m: &mut Manifest| m.sealed = [7; 32], |m: &mut Manifest| m.key = [7; 32]] {
            let mut other = regen.clone();
            f(&mut other);
            assert!(a.update_to(&other).is_err(), "never another object");
        }
    }

    #[test]
    fn pointers_round_trip_strictly() {
        let p = Pointer { made_at: 1_790_000_000, manifest: sample() };
        let b = p.encode();
        assert!(b.len() < 2000);
        assert_eq!(Pointer::decode(&b).unwrap(), p);
        let Value::M(mut e) = cbor::decode(&b).unwrap() else { unreachable!() };
        e.push(("z".into(), Value::U(1)));
        assert!(Pointer::decode(&Value::map(e).encode()).is_err());
        assert!(Pointer::decode(&[0xa0]).is_err());
    }

    mod robust {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(512))]
            /// Any bytes: no panic, and whatever decodes re-encodes to the same bytes.
            #[test]
            fn decoding_is_total_and_canonical(bytes in proptest::collection::vec(any::<u8>(), 0..3000)) {
                if let Ok(m) = Manifest::decode(&bytes) {
                    prop_assert_eq!(m.encode(), bytes);
                }
            }

            /// Mutations of a valid manifest: no panic, and anything still accepted keeps the floors.
            #[test]
            fn mutated_manifests_keep_the_floors(pos in any::<usize>(), byte in any::<u8>()) {
                let mut b = sample().encode();
                let i = pos % b.len();
                b[i] = byte;
                if let Ok(m) = Manifest::decode(&b) {
                    prop_assert!(m.check().is_ok());
                    prop_assert_eq!(m.encode(), b);
                }
            }
        }
    }
}
