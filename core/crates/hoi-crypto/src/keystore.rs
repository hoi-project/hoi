//! Sealed key files: Argon2id + XChaCha20-Poly1305 (TECHNICAL.md §11).
//!
//! Layout: `"HOIK" ‖ 1 ‖ m_cost u32le ‖ t_cost u32le ‖ p_cost u8 ‖ salt[16] ‖ nonce[24] ‖ ciphertext`.
//! The header is authenticated as associated data, so parameters can't be swapped.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use zeroize::Zeroize;

use crate::{CryptoError, random_bytes};

const MAGIC: &[u8; 4] = b"HOIK";
const HEADER: usize = 4 + 1 + 4 + 4 + 1 + 16 + 24;

/// Argon2id cost. 64 MiB, 3 passes: about a second on a laptop, a few in a browser.
#[derive(Debug, Clone, Copy)]
pub struct Cost {
    pub m_kib: u32,
    pub t: u32,
    pub p: u8,
}

impl Default for Cost {
    fn default() -> Self {
        Cost { m_kib: 64 * 1024, t: 3, p: 1 }
    }
}

fn derive(pass: &str, salt: &[u8], c: Cost) -> Result<[u8; 32], CryptoError> {
    let params = Params::new(c.m_kib, c.t, c.p as u32, Some(32)).map_err(|_| CryptoError::Format)?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params).hash_password_into(pass.as_bytes(), salt, &mut key).map_err(|_| CryptoError::Format)?;
    Ok(key)
}

pub fn seal(plaintext: &[u8], passphrase: &str, cost: Cost) -> Result<Vec<u8>, CryptoError> {
    let salt = random_bytes::<16>()?;
    let nonce = random_bytes::<24>()?;
    let mut out = Vec::with_capacity(HEADER + plaintext.len() + 16);
    out.extend_from_slice(MAGIC);
    out.push(1);
    out.extend_from_slice(&cost.m_kib.to_le_bytes());
    out.extend_from_slice(&cost.t.to_le_bytes());
    out.push(cost.p);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    let mut key = derive(passphrase, &salt, cost)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    key.zeroize();
    let ct = cipher.encrypt(&XNonce::from(nonce), Payload { msg: plaintext, aad: &out[..HEADER] }).map_err(|_| CryptoError::Format)?;
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn open(sealed: &[u8], passphrase: &str) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < HEADER + 16 || &sealed[..4] != MAGIC || sealed[4] != 1 {
        return Err(CryptoError::Format);
    }
    let m_kib = u32::from_le_bytes(sealed[5..9].try_into().expect("4"));
    let t = u32::from_le_bytes(sealed[9..13].try_into().expect("4"));
    let p = sealed[13];
    // A key file sets its own cost; cap it so a crafted file can't pin a device for minutes or
    // ask for gigabytes, and refuse costs too low to protect anything.
    if !(8 * 1024..=256 * 1024).contains(&m_kib) || !(1..=16).contains(&t) || !(1..=8).contains(&p) {
        return Err(CryptoError::Format);
    }
    let salt = &sealed[14..30];
    let nonce = &sealed[30..54];
    let mut key = derive(passphrase, salt, Cost { m_kib, t, p })?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    key.zeroize();
    cipher
        .decrypt(&XNonce::try_from(nonce).map_err(|_| CryptoError::Format)?, Payload { msg: &sealed[HEADER..], aad: &sealed[..HEADER] })
        .map_err(|_| CryptoError::Open)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open() {
        let cheap = Cost { m_kib: 8 * 1024, t: 1, p: 1 };
        let s = seal(b"root key", "correct horse", cheap).unwrap();
        assert_eq!(open(&s, "correct horse").unwrap(), b"root key");
        assert_eq!(open(&s, "wrong"), Err(CryptoError::Open));
        let mut tampered = s.clone();
        tampered[10] ^= 1;
        assert!(open(&tampered, "correct horse").is_err());
        let greedy = seal(b"x", "pw", Cost { m_kib: 8 * 1024, t: 1, p: 1 }).map(|mut v| {
            v[5..9].copy_from_slice(&(4u32 * 1024 * 1024).to_le_bytes());
            v
        });
        assert_eq!(open(&greedy.unwrap(), "pw"), Err(CryptoError::Format), "absurd costs are refused before any work");
    }
}
