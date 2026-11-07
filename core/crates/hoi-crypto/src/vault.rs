//! Local storage envelope: a random data key, wrapped once per way in (a slot), and
//! independently authenticated cells that bind their storage address. Encryption protects a
//! locked database, not an unlocked process or rollback of a complete older database snapshot.
//!
//! A passphrase vault (`HOIV\x03`) has one passphrase slot (Argon2id, `keystore`) and up to four
//! passkey slots: a WebAuthn credential's PRF output for the slot's salt, through
//! BLAKE3's key derivation, wraps the same data key. The header lists each passkey's credential id
//! and salt in the clear, so the page can ask the authenticator before anything is unlocked.
//! `HOIV\x02` is a vault whose key an operating system's keystore holds (desktop and phones).

use zeroize::Zeroize;

use crate::{CryptoError, keystore, random_bytes, sealed};

const PASSWORD_MAGIC: &[u8] = b"HOIV\x03";
const PLATFORM_MAGIC: &[u8] = b"HOIV\x02";
const SLOT_PASSPHRASE: u8 = 1;
const SLOT_PASSKEY: u8 = 2;
const PASSKEY_CONTEXT: &str = "hoi local vault passkey slot v1";
const PASSKEY_WRAP: &str = "hoi local vault passkey wrap v1";
/// Passkeys one vault opens with.
pub const MAX_PASSKEYS: usize = 4;
/// What WebAuthn allows a credential id to be.
const MAX_CREDENTIAL_ID: usize = 1023;
/// The passphrase slot: Argon2id parameters, salt, nonce, wrapped key and tag (`keystore`).
const PASSPHRASE_SLOT_LEN: usize = 54 + 32 + 16;

/// A passkey that opens a vault: its WebAuthn credential id and the PRF salt to ask it for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passkey {
    pub credential: Vec<u8>,
    pub salt: [u8; 32],
}

/// The slots of a passphrase vault's header, in order.
fn slots(header: &[u8]) -> Option<Vec<(u8, &[u8])>> {
    let mut rest = header.strip_prefix(PASSWORD_MAGIC)?;
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.len() < 3 {
            return None;
        }
        let kind = rest[0];
        let len = u16::from_le_bytes([rest[1], rest[2]]) as usize;
        let body = rest.get(3..3 + len)?;
        out.push((kind, body));
        rest = &rest[3 + len..];
    }
    let passphrases = out.iter().filter(|(k, _)| *k == SLOT_PASSPHRASE).count();
    let passkeys = out.iter().filter(|(k, _)| *k == SLOT_PASSKEY).count();
    (passphrases == 1 && passkeys <= MAX_PASSKEYS && out.len() == passphrases + passkeys).then_some(out)
}

fn push_slot(header: &mut Vec<u8>, kind: u8, body: &[u8]) {
    header.push(kind);
    header.extend((body.len() as u16).to_le_bytes());
    header.extend(body);
}

/// A passkey slot's parts: credential id, salt, wrapped key.
fn passkey_slot(body: &[u8]) -> Option<(&[u8], [u8; 32], &[u8])> {
    let n = u16::from_le_bytes([*body.first()?, *body.get(1)?]) as usize;
    let credential = body.get(2..2 + n)?;
    let salt: [u8; 32] = body.get(2 + n..2 + n + 32)?.try_into().ok()?;
    Some((credential, salt, body.get(2 + n + 32..)?))
}

fn passkey_wrap(prf: &[u8; 32], salt: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key(PASSKEY_WRAP);
    h.update(prf);
    h.update(salt);
    *h.finalize().as_bytes()
}

/// The passkeys a vault header lists: what the page asks an authenticator for before unlocking.
pub fn passkeys(header: &[u8]) -> Vec<Passkey> {
    slots(header)
        .into_iter()
        .flatten()
        .filter(|(k, _)| *k == SLOT_PASSKEY)
        .filter_map(|(_, body)| passkey_slot(body).map(|(credential, salt, _)| Passkey { credential: credential.to_vec(), salt }))
        .collect()
}
const CHECK_CONTEXT: &str = "hoi local vault platform key check v1";
const CHECK: &[u8] = b"hoi local vault key verified";

/// Clear a host-owned passphrase after the KDF finishes, including unsuccessful unlocks.
pub fn clear_passphrase(passphrase: &mut String) {
    passphrase.zeroize();
}

pub struct LocalVault {
    key: [u8; 32],
    header: Vec<u8>,
}

impl Drop for LocalVault {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl LocalVault {
    pub fn create(passphrase: &str) -> Result<Self, CryptoError> {
        Self::create_with_cost(passphrase, keystore::Cost::default())
    }

    fn create_with_cost(passphrase: &str, cost: keystore::Cost) -> Result<Self, CryptoError> {
        if passphrase.chars().count() < 15 || passphrase.len() > 1024 {
            return Err(CryptoError::Format);
        }
        let mut key = random_bytes::<32>()?;
        let result = keystore::seal(&key, passphrase, cost);
        let wrapped = match result {
            Ok(wrapped) => wrapped,
            Err(error) => {
                key.zeroize();
                return Err(error);
            }
        };
        let mut header = PASSWORD_MAGIC.to_vec();
        push_slot(&mut header, SLOT_PASSPHRASE, &wrapped);
        Ok(Self { key, header })
    }

    pub fn unlock(header: &[u8], passphrase: &str) -> Result<Self, CryptoError> {
        // Headers generated here are fixed-size. Reject attacker-selected expensive KDF
        // parameters before deriving anything; local vaults have exactly one supported cost.
        if passphrase.len() > 1024 {
            return Err(CryptoError::Open);
        }
        let slots = slots(header).ok_or(CryptoError::Open)?;
        let wrapped = slots.iter().find(|(k, _)| *k == SLOT_PASSPHRASE).map(|(_, b)| *b).ok_or(CryptoError::Open)?;
        if wrapped.len() != PASSPHRASE_SLOT_LEN {
            return Err(CryptoError::Open);
        }
        let cost = keystore::Cost::default();
        let expected_cost = wrapped[5..9] == cost.m_kib.to_le_bytes() && wrapped[9..13] == cost.t.to_le_bytes() && wrapped[13] == cost.p;
        #[cfg(test)]
        let expected_cost = expected_cost || (wrapped[5..9] == 8192u32.to_le_bytes() && wrapped[9..13] == 1u32.to_le_bytes() && wrapped[13] == 1);
        if !expected_cost {
            return Err(CryptoError::Open);
        }
        let mut plain = keystore::open(wrapped, passphrase).map_err(|_| CryptoError::Open)?;
        let key = <[u8; 32]>::try_from(plain.as_slice()).map_err(|_| CryptoError::Open);
        plain.zeroize();
        Ok(Self { key: key?, header: header.to_vec() })
    }

    /// Opens a passphrase vault with a passkey: `prf` is what its authenticator answered for
    /// the slot's salt (WebAuthn's `prf` extension).
    pub fn unlock_passkey(header: &[u8], credential: &[u8], prf: &[u8; 32]) -> Result<Self, CryptoError> {
        let slots = slots(header).ok_or(CryptoError::Open)?;
        for (kind, body) in slots {
            if kind != SLOT_PASSKEY {
                continue;
            }
            let Some((id, salt, wrapped)) = passkey_slot(body) else { continue };
            if id != credential {
                continue;
            }
            let mut wrap = passkey_wrap(prf, &salt);
            let opened = sealed::open(PASSKEY_CONTEXT, &wrap, wrapped);
            wrap.zeroize();
            let mut plain = opened.map_err(|_| CryptoError::Open)?;
            let key = <[u8; 32]>::try_from(plain.as_slice()).map_err(|_| CryptoError::Open);
            plain.zeroize();
            return Ok(Self { key: key?, header: header.to_vec() });
        }
        Err(CryptoError::Open)
    }

    /// Lets a passkey open this vault too (a passphrase vault, unlocked): its credential id,
    /// the salt the page asked it for, and its PRF answer. The same credential replaces its slot.
    pub fn add_passkey(&mut self, credential: &[u8], salt: &[u8; 32], prf: &[u8; 32]) -> Result<(), CryptoError> {
        if credential.is_empty() || credential.len() > MAX_CREDENTIAL_ID {
            return Err(CryptoError::Format);
        }
        let slots = slots(&self.header).ok_or(CryptoError::Format)?;
        let kept: Vec<(u8, Vec<u8>)> = slots
            .into_iter()
            .filter(|(k, b)| *k == SLOT_PASSPHRASE || passkey_slot(b).is_some_and(|(id, ..)| id != credential))
            .map(|(k, b)| (k, b.to_vec()))
            .collect();
        if kept.iter().filter(|(k, _)| *k == SLOT_PASSKEY).count() >= MAX_PASSKEYS {
            return Err(CryptoError::Format);
        }
        let mut wrap = passkey_wrap(prf, salt);
        let wrapped = sealed::seal(PASSKEY_CONTEXT, &wrap, &self.key);
        wrap.zeroize();
        let mut body = (credential.len() as u16).to_le_bytes().to_vec();
        body.extend(credential);
        body.extend(salt);
        body.extend(wrapped?);
        let mut header = PASSWORD_MAGIC.to_vec();
        for (k, b) in &kept {
            push_slot(&mut header, *k, b);
        }
        push_slot(&mut header, SLOT_PASSKEY, &body);
        self.header = header;
        Ok(())
    }

    /// Forgets every passkey: only the passphrase opens this vault again.
    pub fn remove_passkeys(&mut self) -> Result<(), CryptoError> {
        let slots = slots(&self.header).ok_or(CryptoError::Format)?;
        let mut header = PASSWORD_MAGIC.to_vec();
        for (k, b) in slots.into_iter().filter(|(k, _)| *k == SLOT_PASSPHRASE) {
            push_slot(&mut header, k, b);
        }
        self.header = header;
        Ok(())
    }

    /// A random data key protected by an OS keystore needs no password derivation.
    pub fn create_platform(mut key: [u8; 32]) -> Result<Self, CryptoError> {
        let check = sealed::seal(CHECK_CONTEXT, &key, CHECK);
        let check = match check {
            Ok(check) => check,
            Err(error) => {
                key.zeroize();
                return Err(error);
            }
        };
        let mut header = PLATFORM_MAGIC.to_vec();
        header.extend(check);
        Ok(Self { key, header })
    }

    pub fn unlock_platform(header: &[u8], mut key: [u8; 32]) -> Result<Self, CryptoError> {
        let valid = header.strip_prefix(PLATFORM_MAGIC).and_then(|body| sealed::open(CHECK_CONTEXT, &key, body).ok()).is_some_and(|plain| plain == CHECK);
        if !valid {
            key.zeroize();
            return Err(CryptoError::Open);
        }
        Ok(Self { key, header: header.to_vec() })
    }

    /// Optional native OS unlock for a password vault. Only store this secret in the OS
    /// credential store, never in the database or web UI. Binding it to the exact header
    /// prevents an old credential from opening a replaced vault. The password slot stays.
    pub fn os_unlock_secret(&self) -> Result<zeroize::Zeroizing<Vec<u8>>, CryptoError> {
        slots(&self.header).ok_or(CryptoError::Format)?;
        // Reserve before copying the key: extending must not leave a prior secret allocation
        // behind when Vec reallocates. Zeroizing clears the single allocation on drop.
        let mut secret = zeroize::Zeroizing::new(Vec::with_capacity(64));
        secret.extend_from_slice(&self.key);
        secret.extend_from_slice(blake3::hash(&self.header).as_bytes());
        Ok(secret)
    }

    /// The caller must authenticate the encrypted store's inventory before exposing any
    /// contents. A matching header hash is not itself authentication of the data key.
    pub fn unlock_os_secret(header: &[u8], secret: &[u8]) -> Result<Self, CryptoError> {
        slots(header).ok_or(CryptoError::Open)?;
        if secret.len() != 64 || secret[32..] != *blake3::hash(header).as_bytes() {
            return Err(CryptoError::Open);
        }
        let key = secret[..32].try_into().map_err(|_| CryptoError::Open)?;
        Ok(Self { key, header: header.to_vec() })
    }

    pub fn header(&self) -> &[u8] {
        &self.header
    }

    /// Hash ciphertext for the separately authenticated inventory. This is not a MAC.
    pub fn fingerprint(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    pub fn seal(&self, address: &str, plain: &[u8]) -> Result<Vec<u8>, CryptoError> {
        sealed::seal(&format!("hoi local vault cell v1:{address}"), &self.key, plain)
    }

    pub fn unseal(&self, address: &str, cipher: &[u8]) -> Result<Vec<u8>, CryptoError> {
        sealed::open(&format!("hoi local vault cell v1:{address}"), &self.key, cipher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_os_unlock_keeps_password_and_binds_the_original_vault() {
        let pass = "a sufficiently long passphrase";
        let vault = LocalVault::create_with_cost(pass, keystore::Cost { m_kib: 8192, t: 1, p: 1 }).unwrap();
        let original_header = vault.header().to_vec();
        let cipher = vault.seal("blob:keys", b"private identity").unwrap();
        let secret = vault.os_unlock_secret().unwrap();
        let unlocked = LocalVault::unlock_os_secret(&original_header, &secret).unwrap();
        assert_eq!(unlocked.unseal("blob:keys", &cipher).unwrap(), b"private identity");
        assert_eq!(vault.header(), original_header);
        assert_eq!(LocalVault::unlock(&original_header, pass).unwrap().unseal("blob:keys", &cipher).unwrap(), b"private identity");
        let other = LocalVault::create_with_cost(pass, keystore::Cost { m_kib: 8192, t: 1, p: 1 }).unwrap();
        assert!(LocalVault::unlock_os_secret(other.header(), &secret).is_err());
        assert!(LocalVault::unlock_os_secret(&original_header, &secret[..32]).is_err());
        let mut wrong = secret.to_vec();
        wrong[0] ^= 1;
        let wrong = LocalVault::unlock_os_secret(&original_header, &wrong).unwrap();
        assert!(wrong.unseal("blob:keys", &cipher).is_err());
    }

    fn cheap(passphrase: &str) -> LocalVault {
        LocalVault::create_with_cost(passphrase, keystore::Cost { m_kib: 8192, t: 1, p: 1 }).unwrap()
    }

    #[test]
    fn a_passkey_opens_the_same_vault_and_only_its_own_answer_does() {
        let mut vault = cheap("a sufficiently long passphrase");
        let cipher = vault.seal("blob:keys", b"the keys").unwrap();
        assert!(passkeys(vault.header()).is_empty());
        let (credential, salt, prf) = (b"credential-one".to_vec(), [4u8; 32], [9u8; 32]);
        vault.add_passkey(&credential, &salt, &prf).unwrap();
        assert_eq!(passkeys(vault.header()), vec![Passkey { credential: credential.clone(), salt }]);
        let header = vault.header().to_vec();
        let opened = LocalVault::unlock_passkey(&header, &credential, &prf).unwrap();
        assert_eq!(opened.unseal("blob:keys", &cipher).unwrap(), b"the keys");
        assert!(LocalVault::unlock_passkey(&header, &credential, &[8u8; 32]).is_err(), "another answer");
        assert!(LocalVault::unlock_passkey(&header, b"credential-two", &prf).is_err(), "another passkey");
        // the passphrase still opens it
        assert_eq!(LocalVault::unlock(&header, "a sufficiently long passphrase").unwrap().unseal("blob:keys", &cipher).unwrap(), b"the keys");
        // a second passkey, the first again (replaced), then gone
        vault.add_passkey(b"credential-two", &[5u8; 32], &[6u8; 32]).unwrap();
        vault.add_passkey(&credential, &[7u8; 32], &prf).unwrap();
        assert_eq!(passkeys(vault.header()).len(), 2);
        assert!(LocalVault::unlock_passkey(vault.header(), &credential, &prf).is_ok());
        for i in 0..2 {
            vault.add_passkey(&[i], &[i; 32], &[i; 32]).unwrap();
        }
        assert!(vault.add_passkey(b"a fifth", &[1; 32], &[1; 32]).is_err(), "four at most");
        vault.remove_passkeys().unwrap();
        assert!(passkeys(vault.header()).is_empty());
        assert!(LocalVault::unlock_passkey(vault.header(), &credential, &prf).is_err());
        assert!(LocalVault::unlock(vault.header(), "a sufficiently long passphrase").is_ok());
    }

    #[test]
    fn damaged_slot_lists_are_refused() {
        let vault = cheap("a sufficiently long passphrase");
        let mut two = vault.header().to_vec();
        let slot = two[PASSWORD_MAGIC.len()..].to_vec();
        two.extend(&slot);
        assert!(LocalVault::unlock(&two, "a sufficiently long passphrase").is_err(), "two passphrase slots");
        let mut cut = vault.header().to_vec();
        cut.pop();
        assert!(LocalVault::unlock(&cut, "a sufficiently long passphrase").is_err());
        let mut unknown = vault.header().to_vec();
        unknown.extend([9, 1, 0, 0]);
        assert!(LocalVault::unlock(&unknown, "a sufficiently long passphrase").is_err());
    }

    #[test]
    fn password_and_cells_are_authenticated_and_randomized() {
        let vault = LocalVault::create_with_cost("a sufficiently long passphrase", keystore::Cost { m_kib: 8192, t: 1, p: 1 }).unwrap();
        let restored = LocalVault::unlock(vault.header(), "a sufficiently long passphrase").unwrap();
        assert!(LocalVault::unlock(vault.header(), "a wrong passphrase").is_err());
        let cipher = vault.seal("blob:mls", b"private group secret").unwrap();
        assert_ne!(cipher, vault.seal("blob:mls", b"private group secret").unwrap());
        assert_eq!(restored.unseal("blob:mls", &cipher).unwrap(), b"private group secret");
        assert!(restored.unseal("blob:keys", &cipher).is_err());
        let mut bad = cipher.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(restored.unseal("blob:mls", &bad).is_err());
        assert!(LocalVault::create("short").is_err());
        assert!(LocalVault::unlock(&vault.header()[..20], "a sufficiently long passphrase").is_err());
        let mut expensive = vault.header().to_vec();
        expensive[PASSWORD_MAGIC.len() + 3 + 5..PASSWORD_MAGIC.len() + 3 + 9].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(LocalVault::unlock(&expensive, "a sufficiently long passphrase").is_err());
        let mut altered_salt = vault.header().to_vec();
        altered_salt[PASSWORD_MAGIC.len() + 3 + 14] ^= 1;
        assert!(LocalVault::unlock(&altered_salt, "a sufficiently long passphrase").is_err());
    }

    #[test]
    fn platform_key_must_match_existing_header() {
        let vault = LocalVault::create_platform([1; 32]).unwrap();
        assert!(LocalVault::unlock_platform(vault.header(), [2; 32]).is_err());
        let same = LocalVault::unlock_platform(vault.header(), [1; 32]).unwrap();
        assert_eq!(same.unseal("record:abc", &vault.seal("record:abc", b"private").unwrap()).unwrap(), b"private");
    }
}
