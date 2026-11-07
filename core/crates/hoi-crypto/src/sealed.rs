//! Short messages sealed with a shared secret: BLAKE3-derived key, XChaCha20-Poly1305.
//!
//! Layout: `nonce[24] ‖ ciphertext`. `context` separates uses (BLAKE3 `derive_key`), so one
//! secret never encrypts two kinds of message with the same key.

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use zeroize::Zeroize;

use crate::{CryptoError, random_bytes};

fn key(context: &str, secret: &[u8]) -> [u8; 32] {
    blake3::derive_key(context, secret)
}

pub fn seal(context: &str, secret: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let nonce = random_bytes::<24>()?;
    let mut k = key(context, secret);
    let cipher = XChaCha20Poly1305::new((&k).into());
    k.zeroize();
    let ct = cipher.encrypt(&XNonce::from(nonce), Payload { msg: plaintext, aad: context.as_bytes() }).map_err(|_| CryptoError::Format)?;
    let mut out = Vec::with_capacity(24 + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn open(context: &str, secret: &[u8], sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < 24 + 16 {
        return Err(CryptoError::Format);
    }
    let mut k = key(context, secret);
    let cipher = XChaCha20Poly1305::new((&k).into());
    k.zeroize();
    let nonce = XNonce::try_from(&sealed[..24]).map_err(|_| CryptoError::Format)?;
    cipher.decrypt(&nonce, Payload { msg: &sealed[24..], aad: context.as_bytes() }).map_err(|_| CryptoError::Open)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_and_tamper() {
        let s = seal("hoi test v1", b"secret", b"hello").unwrap();
        assert_eq!(open("hoi test v1", b"secret", &s).unwrap(), b"hello");
        assert!(open("hoi test v1", b"other", &s).is_err());
        assert!(open("hoi other v1", b"secret", &s).is_err());
        let mut bad = s.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(open("hoi test v1", b"secret", &bad).is_err());
        assert!(open("hoi test v1", b"secret", &s[..30]).is_err());
    }
}
