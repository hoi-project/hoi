//! The recovery key: one line of text, for a password manager or a piece of paper. It
//! holds a person's master recovery secret, from which every root key they will
//! ever sign with and every recovery key that can replace one are derived ([`root_at`],
//! [`recovery_at`]). The first root is the person's id for good; each identity record commits to
//! the recovery key of the next generation ([`commitment`]), so whoever holds the line can always
//! replace a root that leaked, and the standing stays.
//!
//! ```text
//! hoi-key-1-xxxxxxxx-xxxxxxxx-xxxxxxxx-xxxxxxxx-xxxxxxxx-xxxxxxxx-xxxxxxxx
//! ```
//!
//! The groups are z-base-32 (no `0`, `l`, `v` or `2`, so it survives being read aloud or
//! written down) of `secret[32] ‖ check[3]`, where `check` is the first three bytes of
//! `BLAKE3-derive-key("hoi 2026 recovery key v1", secret)`: a typo is caught, a stranger's
//! string is refused, and nothing about the key leaks from the check (it is a keyed hash of
//! the secret itself). Whoever holds this line holds the key: it is the key, not an encrypted
//! copy of it, so it belongs in a password manager, which is encrypted, or on paper in a drawer.
//! The `.hoikey` file (`keystore`) is the passphrase-protected alternative for a disk or a cloud.

use hoi_types::uri::{zbase32_decode, zbase32_encode};
use zeroize::Zeroize;

use crate::CryptoError;

/// The root key of `generation` for the master secret behind a recovery line: generation 0 is
/// the person's first root, whose public key is their id.
pub fn root_at(master: &[u8; 32], generation: u32) -> crate::Keypair {
    derived("hoi 2026 root key v1", master, generation)
}

/// The recovery key that may replace the root of `generation - 1` with the root of
/// `generation`; the identity record of that earlier root commits to it.
pub fn recovery_at(master: &[u8; 32], generation: u32) -> crate::Keypair {
    derived("hoi 2026 recovery key v1 by generation", master, generation)
}

fn derived(context: &str, master: &[u8; 32], generation: u32) -> crate::Keypair {
    let mut h = blake3::Hasher::new_derive_key(context);
    h.update(master);
    h.update(&generation.to_le_bytes());
    let mut seed = *h.finalize().as_bytes();
    let kp = crate::Keypair::from_secret(&seed);
    seed.zeroize();
    kp
}

/// What an identity record carries for the recovery key that may replace its root (`rc`): a
/// hash, so the key itself is shown only when it is used.
pub fn commitment(recovery: &hoi_types::Id) -> hoi_types::Id {
    hoi_types::Id(blake3::derive_key("hoi 2026 recovery commitment v1", &recovery.0))
}

/// Says what the line is and which format it uses; a later format bumps the number.
pub const PREFIX: &str = "hoi-key-1-";
const CHECK_CONTEXT: &str = "hoi 2026 recovery key v1";
const CHECK_LEN: usize = 3;
const GROUP: usize = 8;
/// `secret[32] ‖ check[3]` is 35 bytes: exactly 56 z-base-32 characters, seven groups of eight.
const CHARS: usize = 56;

fn check(secret: &[u8; 32]) -> [u8; CHECK_LEN] {
    let h = blake3::derive_key(CHECK_CONTEXT, secret);
    h[..CHECK_LEN].try_into().expect("3 of 32 bytes")
}

/// The one line for `secret`.
pub fn encode(secret: &[u8; 32]) -> String {
    let mut payload = [0u8; 32 + CHECK_LEN];
    payload[..32].copy_from_slice(secret);
    payload[32..].copy_from_slice(&check(secret));
    let mut chars = zbase32_encode(&payload);
    payload.zeroize();
    debug_assert_eq!(chars.len(), CHARS);
    let mut out = String::with_capacity(PREFIX.len() + CHARS + CHARS / GROUP);
    out.push_str(PREFIX);
    for (i, c) in chars.chars().enumerate() {
        if i > 0 && i % GROUP == 0 {
            out.push('-');
        }
        out.push(c);
    }
    chars.zeroize();
    out
}

/// What people type: any case, with or without the hyphens, spaces or line breaks a password
/// manager, a printout or a phone keyboard may add, `0` for `o` and `l` for `1` forgiven (z-base-32
/// has neither). Returns the body after the prefix, or `None` when it isn't a recovery key at all.
fn normalize(text: &str) -> Option<String> {
    let mut body: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| match c.to_ascii_lowercase() {
            '0' => 'o',
            'l' => '1',
            c => c,
        })
        .collect();
    let prefix: String = PREFIX.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if !body.starts_with(&prefix) {
        body.zeroize();
        return None;
    }
    let out = body[prefix.len()..].to_string();
    body.zeroize();
    Some(out)
}

/// Whether `text` is shaped like a recovery key (the prefix and the right length), so a form can
/// tell a pasted key from a passphrase before trying it. Says nothing about the check.
pub fn looks_like(text: &str) -> bool {
    normalize(text).is_some_and(|body| body.len() == CHARS)
}

/// The secret behind the line, or [`CryptoError::Open`] when the line is damaged, mistyped or
/// not a hoi recovery key.
pub fn decode(text: &str) -> Result<[u8; 32], CryptoError> {
    let mut body = normalize(text).ok_or(CryptoError::Format)?;
    if body.len() != CHARS {
        body.zeroize();
        return Err(CryptoError::Open);
    }
    let decoded = zbase32_decode(&body);
    body.zeroize();
    let mut payload = decoded.map_err(|_| CryptoError::Open)?;
    if payload.len() != 32 + CHECK_LEN {
        payload.zeroize();
        return Err(CryptoError::Open);
    }
    let mut secret = [0u8; 32];
    secret.copy_from_slice(&payload[..32]);
    let ok = check(&secret) == payload[32..];
    payload.zeroize();
    if !ok {
        secret.zeroize();
        return Err(CryptoError::Open);
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generations_are_distinct_and_fixed() {
        let master = [9u8; 32];
        let (a, b) = (root_at(&master, 0).public(), root_at(&master, 1).public());
        assert_ne!(a, b);
        assert_eq!(root_at(&master, 0).public(), a, "the same line gives the same person");
        assert_ne!(recovery_at(&master, 1).public(), b);
        assert_ne!(commitment(&recovery_at(&master, 1).public()), commitment(&recovery_at(&master, 2).public()));
        assert_ne!(root_at(&[8u8; 32], 0).public(), a);
    }

    #[test]
    fn round_trip_and_shape() {
        let secret = [7u8; 32];
        let line = encode(&secret);
        assert!(line.starts_with(PREFIX));
        assert_eq!(line.len(), PREFIX.len() + CHARS + 6);
        assert_eq!(line.split('-').count(), 10);
        assert!(line.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
        assert!(looks_like(&line));
        assert_eq!(decode(&line).unwrap(), secret);
        // a different key gives a different line, start to end
        let other = encode(&[8u8; 32]);
        assert_ne!(other, line);
        assert_ne!(&other[PREFIX.len()..PREFIX.len() + GROUP], &line[PREFIX.len()..PREFIX.len() + GROUP]);
    }

    #[test]
    fn forgives_what_people_and_password_managers_do_to_it() {
        let secret = crate::random_bytes::<32>().unwrap();
        let line = encode(&secret);
        let body = &line[PREFIX.len()..];
        assert_eq!(decode(&line.to_uppercase()).unwrap(), secret);
        assert_eq!(decode(&format!("  {}\n", line.replace('-', " "))).unwrap(), secret);
        assert_eq!(decode(&format!("HOI KEY 1 {}", body.replace('-', ""))).unwrap(), secret);
        assert_eq!(decode(&line.replace('o', "0").replace('1', "l")).unwrap(), secret);
        assert!(looks_like(&format!("hoikey1{}", body.replace('-', ""))));
    }

    #[test]
    fn refuses_typos_strangers_and_other_things() {
        let secret = [9u8; 32];
        let line = encode(&secret);
        // one character off anywhere in the body
        for i in PREFIX.len()..line.len() {
            let mut chars: Vec<char> = line.chars().collect();
            if chars[i] == '-' {
                continue;
            }
            chars[i] = if chars[i] == 'y' { 'b' } else { 'y' };
            let wrong: String = chars.into_iter().collect();
            assert_eq!(decode(&wrong), Err(CryptoError::Open), "{wrong}");
        }
        // the right alphabet, the wrong check
        let mut payload = [1u8; 35];
        payload[..32].copy_from_slice(&secret);
        let forged = format!("{PREFIX}{}", zbase32_encode(&payload));
        assert!(looks_like(&forged));
        assert_eq!(decode(&forged), Err(CryptoError::Open));
        // too short, too long, not a key
        assert_eq!(decode(&line[..line.len() - 1]), Err(CryptoError::Open));
        assert_eq!(decode(&format!("{line}y")), Err(CryptoError::Open));
        assert!(!looks_like(&line[..line.len() - 1]));
        assert_eq!(decode("correct horse battery staple"), Err(CryptoError::Format));
        assert_eq!(decode("hoi-key-2-yyyyyyyy"), Err(CryptoError::Format));
        assert!(!looks_like("hoi://join?c=abc"));
        assert!(!looks_like(""));
    }
}
