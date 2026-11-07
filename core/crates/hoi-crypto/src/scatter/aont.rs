//! The all-or-nothing transform of AONT-RS (Resch and Plank, FAST 2011; SCATTER.md §5.3).
//!
//! `package = C ‖ (K ⊕ H(C))` where `C = ChaCha20(K, nonce 0)(sealed ‖ canary)`, `H` is BLAKE3
//! in derive mode under `"hoi scatter aont v1"`, and the canary is 16 zero bytes. Without every
//! byte of `C` nobody learns `K`, and without `K` no byte of `sealed` follows: fewer than k pieces
//! of a generation reveal nothing, whatever key the inner seal used. Every generation draws a
//! fresh `K`, so packages of two generations share no keystream and their pieces never combine.

use chacha20::ChaCha20;
use chacha20::cipher::{KeyIvInit, StreamCipher};
use zeroize::Zeroize;

use crate::CryptoError;

const CONTEXT: &str = "hoi scatter aont v1";
/// Bytes the transform adds: the canary and the masked key.
pub const OVERHEAD: usize = CANARY + 32;
const CANARY: usize = 16;
const _: () = assert!(OVERHEAD == hoi_types::scatter::AONT_OVERHEAD);

fn keystream(key: &[u8; 32], buf: &mut [u8]) {
    // A key is used for exactly one package, so the zero nonce never repeats under it.
    let mut c = ChaCha20::new(key.into(), &[0u8; 12].into());
    c.apply_keystream(buf);
}

/// Wraps `sealed` under the one-time key `key`. The caller draws `key` from the CSPRNG for each
/// generation ([`package_random`]); a key must never be used twice.
pub fn package(sealed: &[u8], key: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(sealed.len() + OVERHEAD);
    out.extend_from_slice(sealed);
    out.extend_from_slice(&[0u8; CANARY]);
    keystream(key, &mut out);
    let mut mask = blake3::derive_key(CONTEXT, &out);
    for (m, k) in mask.iter_mut().zip(key) {
        *m ^= k;
    }
    out.extend_from_slice(&mask);
    mask.zeroize();
    out
}

/// [`package`] under a fresh random key, which is dropped (and zeroized) at once.
pub fn package_random(sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let mut key = crate::random_bytes::<32>()?;
    let out = package(sealed, &key);
    key.zeroize();
    Ok(out)
}

/// Recovers `sealed` from a whole package; fails if the canary is not zero (a wrong or damaged
/// package).
pub fn unpackage(package: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if package.len() < OVERHEAD {
        return Err(CryptoError::Format);
    }
    let (body, masked) = package.split_at(package.len() - 32);
    let mut key = blake3::derive_key(CONTEXT, body);
    for (k, m) in key.iter_mut().zip(masked) {
        *k ^= m;
    }
    let mut out = body.to_vec();
    keystream(&key, &mut out);
    key.zeroize();
    let (sealed, canary) = out.split_at(out.len() - CANARY);
    if canary.iter().any(|&b| b != 0) {
        out.zeroize();
        return Err(CryptoError::Open);
    }
    let sealed = sealed.to_vec();
    out.zeroize();
    Ok(sealed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_damage() {
        let sealed = b"an object sealed under its own key".to_vec();
        let p = package(&sealed, &[7u8; 32]);
        assert_eq!(p.len(), sealed.len() + OVERHEAD);
        assert_ne!(&p[..sealed.len()], &sealed[..], "the package hides the sealed bytes");
        assert_eq!(unpackage(&p).unwrap(), sealed);
        // any changed byte, in the body or the masked key, fails the canary
        for i in [0, sealed.len() / 2, sealed.len(), p.len() - 1] {
            let mut bad = p.clone();
            bad[i] ^= 1;
            assert_eq!(unpackage(&bad), Err(CryptoError::Open), "byte {i}");
        }
        assert_eq!(unpackage(&p[..10]), Err(CryptoError::Format));
    }

    #[test]
    fn fresh_keys_share_nothing() {
        let sealed = vec![0u8; 256];
        let a = package_random(&sealed).unwrap();
        let b = package_random(&sealed).unwrap();
        assert_ne!(a, b);
        assert_eq!(unpackage(&a).unwrap(), sealed);
        assert_eq!(unpackage(&b).unwrap(), sealed);
    }
}
