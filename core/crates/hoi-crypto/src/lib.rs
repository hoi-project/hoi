//! hoi-crypto: Ed25519 keys and signatures, BLAKE3 proof-of-work stamps, the keystore, the
//! recovery key, and scattered delivery's pieces and envelopes.

pub mod envelope;
pub mod identity;
pub mod keystore;
pub mod pow;
pub mod recovery;
pub mod scatter;
pub mod sealed;
pub mod shamir;
pub mod vault;

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use hoi_types::{Id, Record, Sig, Wire};
use thiserror::Error;
use zeroize::Zeroize;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("no randomness available")]
    Rng,
    #[error("bad key")]
    Key,
    #[error("wrong passphrase or damaged file")]
    Open,
    #[error("unsupported keystore format")]
    Format,
}

pub fn random_bytes<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|_| CryptoError::Rng)?;
    Ok(b)
}

/// An Ed25519 signing key. The secret never leaves this type except through [`Keypair::secret`]
/// for the keystore.
pub struct Keypair {
    sk: SigningKey,
}

impl Clone for Keypair {
    fn clone(&self) -> Self {
        Keypair { sk: SigningKey::from_bytes(&self.sk.to_bytes()) }
    }
}

impl std::fmt::Debug for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Keypair({})", self.public().short_key())
    }
}

impl Keypair {
    pub fn generate() -> Result<Keypair, CryptoError> {
        let mut seed = random_bytes::<32>()?;
        let kp = Keypair::from_secret(&seed);
        seed.zeroize();
        Ok(kp)
    }

    /// A key from the system's randomness with `stir` mixed in: randomness the person made
    /// themselves (the web app's first run hashes their pointer moves and key timings into it).
    /// `seed = BLAKE3-derive-key("hoi 2026 key seed v1", csprng32 ‖ stir)`. The system's 32
    /// bytes alone make the key safe; the stir can only add to them, never stand in for them, and
    /// the same gestures never make the same key.
    pub fn generate_stirred(stir: &[u8]) -> Result<Keypair, CryptoError> {
        let mut rnd = random_bytes::<32>()?;
        let mut h = blake3::Hasher::new_derive_key("hoi 2026 key seed v1");
        h.update(&rnd);
        h.update(stir);
        let mut seed = *h.finalize().as_bytes();
        rnd.zeroize();
        let kp = Keypair::from_secret(&seed);
        seed.zeroize();
        Ok(kp)
    }

    pub fn from_secret(seed: &[u8; 32]) -> Keypair {
        Keypair { sk: SigningKey::from_bytes(seed) }
    }

    pub fn secret(&self) -> [u8; 32] {
        self.sk.to_bytes()
    }

    pub fn public(&self) -> Id {
        Id(self.sk.verifying_key().to_bytes())
    }

    pub fn sign(&self, msg: &[u8]) -> Sig {
        Sig(self.sk.sign(msg).to_bytes())
    }

    /// `sig = Ed25519.sign(author_sk, id)` (PROTOCOL.md §2).
    pub fn sign_record(&self, record: Record) -> (Wire, Id) {
        let id = record.id();
        let sig = self.sign(id.as_bytes());
        (Wire { record, sig }, id)
    }
}

pub fn verify(pk: &Id, msg: &[u8], sig: &Sig) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(pk.as_bytes()) else { return false };
    let s = ed25519_dalek::Signature::from_bytes(&sig.0);
    vk.verify_strict(msg, &s).is_ok()
}

/// A public key that is a valid curve point outside the small-order subgroup. Weak keys can
/// produce signatures that verify for many messages; no identity may use one.
pub fn key_is_strong(pk: &Id) -> bool {
    VerifyingKey::from_bytes(pk.as_bytes()).is_ok_and(|vk| !vk.is_weak())
}

/// The root's blessing for a device, over [`hoi_types::delegation_statement`].
pub fn bless(root: &Keypair, device: &Id, scope: &[String], expires_at: u64) -> Sig {
    root.sign(hoi_types::delegation_statement(&root.public(), device, scope, expires_at).as_bytes())
}

/// `e04c · 2a9f · 7b31 · 0cd8`: what two people compare in person.
pub fn fingerprint(pk: &Id) -> String {
    let mut h = blake3::Hasher::new_derive_key("hoi 2026 key fingerprint v1");
    h.update(pk.as_bytes());
    let d = h.finalize();
    let hex = hoi_types::hex_encode(&d.as_bytes()[..8]);
    format!("{} · {} · {} · {}", &hex[0..4], &hex[4..8], &hex[8..12], &hex[12..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify() {
        let k = Keypair::generate().unwrap();
        let s = k.sign(b"hoi");
        assert!(verify(&k.public(), b"hoi", &s));
        assert!(!verify(&k.public(), b"hei", &s));
        let k2 = Keypair::from_secret(&k.secret());
        assert_eq!(k2.public(), k.public());
        assert_eq!(fingerprint(&k.public()).chars().count(), 25);
        assert!(key_is_strong(&k.public()));
        // the identity point is small-order
        let mut ident = [0u8; 32];
        ident[0] = 1;
        assert!(!key_is_strong(&Id(ident)));
    }

    #[test]
    fn stirred_keys_never_repeat() {
        // the same gestures (or none) still give different, strong keys: the system's
        // randomness is always in the seed
        let a = Keypair::generate_stirred(b"the very same wiggle").unwrap();
        let b = Keypair::generate_stirred(b"the very same wiggle").unwrap();
        let c = Keypair::generate_stirred(&[]).unwrap();
        assert_ne!(a.public(), b.public());
        assert_ne!(a.public(), c.public());
        for k in [&a, &b, &c] {
            assert!(key_is_strong(&k.public()));
            let s = k.sign(b"hoi");
            assert!(verify(&k.public(), b"hoi", &s));
        }
    }
}
