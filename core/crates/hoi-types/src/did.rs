//! `did:key` for Ed25519 public keys (multicodec 0xed01, multibase base58btc), and parsing of
//! keys pasted by people: hex or did:key.

use crate::{Id, TypesError};

const ED25519_PUB: [u8; 2] = [0xed, 0x01];

pub fn did_key(pk: &Id) -> String {
    let mut b = Vec::with_capacity(34);
    b.extend_from_slice(&ED25519_PUB);
    b.extend_from_slice(pk.as_bytes());
    format!("did:key:z{}", bs58::encode(b).into_string())
}

pub fn parse_did_key(s: &str) -> Result<Id, TypesError> {
    let rest = s.trim().strip_prefix("did:key:z").ok_or(TypesError::Did)?;
    let b = bs58::decode(rest).into_vec().map_err(|_| TypesError::Did)?;
    if b.len() != 34 || b[..2] != ED25519_PUB {
        return Err(TypesError::Did);
    }
    Id::from_slice(&b[2..])
}

/// Accepts a 64-char hex key or a did:key.
pub fn parse_key(s: &str) -> Result<Id, TypesError> {
    let s = s.trim();
    if s.starts_with("did:key:") { parse_did_key(s) } else { Id::from_hex(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_roundtrip() {
        let pk = Id([7; 32]);
        let d = did_key(&pk);
        assert!(d.starts_with("did:key:z6Mk"), "{d}");
        assert_eq!(parse_did_key(&d).unwrap(), pk);
        assert_eq!(parse_key(&pk.hex()).unwrap(), pk);
    }
}
