//! Phones keep a random vault key in the platform keystore. Every SQLite payload,
//! including MLS state, is encrypted. Missing or locked keys fail closed.

use crate::KeyStore;
use hoi_crypto::vault::LocalVault;
use hoi_store::{Change, Loaded, Persist, sqlite::Sqlite, vault::EncryptedSqlite};
use std::sync::Arc;

pub(crate) const KEYS: &str = "keys";
pub(crate) const VAULT_KEY: &str = "vault-key-v1";

pub(crate) struct Split {
    raw: Option<Sqlite>,
    encrypted: Option<EncryptedSqlite>,
    keys: Arc<dyn KeyStore>,
}

impl Split {
    pub(crate) fn new(db: Sqlite, keys: Arc<dyn KeyStore>) -> Split {
        Split { raw: Some(db), encrypted: None, keys }
    }

    fn open(&mut self) -> Result<(), String> {
        if self.encrypted.is_some() {
            return Ok(());
        }
        let mut db = self.raw.take().ok_or("Storage could not be unlocked. Reopen the app to retry.")?;
        if let Some(header) = EncryptedSqlite::header(&db)? {
            let mut stored = self.keys.get(VAULT_KEY.into()).ok_or("Unlock this device to access saved data. Its protected key is unavailable.")?;
            let key = <[u8; 32]>::try_from(stored.as_slice()).map_err(|_| "The protected storage key is damaged.");
            stored.fill(0);
            let vault = LocalVault::unlock_platform(&header, key?).map_err(|_| "The protected storage key does not match this database.")?;
            let encrypted = EncryptedSqlite::open(db, vault, &[])?;
            self.remove_legacy_keys()?;
            self.encrypted = Some(encrypted);
            return Ok(());
        }
        let mut blobs = Vec::new();
        for c in db.snapshot()? {
            if let Change::Blob { key, value: Some(value) } = c {
                blobs.push((key, value));
            }
        }
        let state = blobs.iter().find(|(key, _)| key == "state");
        let had_identity =
            state.is_some_and(|(_, bytes)| serde_json::from_slice::<serde_json::Value>(bytes).is_ok_and(|v| v.get("me").is_some_and(|m| !m.is_null())));
        let disk_keys = blobs.iter().find(|(key, _)| key == KEYS).map(|(_, value)| value);
        let legacy = if state.is_some() { self.keys.get(KEYS.into()) } else { None };
        if had_identity && legacy.is_none() && disk_keys.is_none() {
            return Err("Unlock this device to migrate saved data. Its identity keys are unavailable.".into());
        }
        if legacy.as_ref().zip(disk_keys).is_some_and(|(a, b)| a != b) {
            return Err("Saved identity keys disagree. No data was replaced.".into());
        }
        let extra = match (disk_keys, legacy) {
            (None, Some(value)) => vec![Change::Blob { key: KEYS.into(), value: Some(value) }],
            _ => vec![],
        };
        let mut candidate = db.load()?;
        for change in &extra {
            if let Change::Blob { key, value: Some(value) } = change {
                candidate.blobs.push((key.clone(), value.clone()));
            }
        }
        hoi_core::Node::open(Box::new(hoi_store::Journal::with_initial(candidate)), "mobile", "https://hoi.sh/app/", hoi_core::system_clock())
            .map_err(|e| e.to_string())?;
        let key = hoi_crypto::random_bytes::<32>().map_err(|e| e.to_string())?;
        self.keys.set(VAULT_KEY.into(), Some(key.to_vec()));
        // The foreign keystore has no Result. Verify persistence before deleting plaintext.
        let mut readback = self.keys.get(VAULT_KEY.into()).ok_or("The device could not save its protected storage key.")?;
        let matches = readback == key;
        readback.fill(0);
        if !matches {
            return Err("The device could not verify its protected storage key.".into());
        }
        let vault = LocalVault::create_platform(key).map_err(|e| e.to_string())?;
        let encrypted = EncryptedSqlite::open(db, vault, &extra)?;
        self.remove_legacy_keys()?;
        self.encrypted = Some(encrypted);
        Ok(())
    }

    fn remove_legacy_keys(&self) -> Result<(), String> {
        self.keys.set(KEYS.into(), None);
        if self.keys.get(KEYS.into()).is_some() {
            return Err("The device could not finish securing its old keys. Unlock the device and retry.".into());
        }
        Ok(())
    }
}

impl Persist for Split {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        self.open()?;
        self.encrypted.as_mut().ok_or("Storage is locked.")?.apply(changes)
    }
    fn load(&mut self) -> Result<Loaded, String> {
        self.open()?;
        self.encrypted.as_mut().ok_or("Storage is locked.")?.load()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::{collections::HashMap, sync::Mutex};
    #[derive(Default)]
    struct Keys(Mutex<HashMap<String, Vec<u8>>>);
    impl KeyStore for Keys {
        fn get(&self, key: String) -> Option<Vec<u8>> {
            self.0.lock().unwrap().get(&key).cloned()
        }
        fn set(&self, key: String, value: Option<Vec<u8>>) {
            let mut map = self.0.lock().unwrap();
            if let Some(v) = value {
                map.insert(key, v);
            } else {
                map.remove(&key);
            }
        }
    }
    #[test]
    fn migrates_legacy_identity_and_mls_together() {
        let keys = Arc::new(Keys::default());
        let identity = format!("{{\"root\":\"{}\"}}", "01".repeat(32)).into_bytes();
        let mls = br#"{"storage":[[[1],[2,3,4]]]}"#.to_vec();
        keys.set(KEYS.into(), Some(identity.clone()));
        let mut db = Sqlite::memory().unwrap();
        db.apply(&[Change::Blob { key: "state".into(), value: Some(b"{}".to_vec()) }, Change::Blob { key: "mls".into(), value: Some(mls.clone()) }]).unwrap();
        let mut split = Split::new(db, keys.clone());
        let loaded = split.load().unwrap();
        assert!(loaded.blobs.iter().any(|(key, v)| key == "keys" && v == &identity));
        assert!(loaded.blobs.iter().any(|(key, v)| key == "mls" && v == &mls));
        assert!(keys.get(KEYS.into()).is_none());
        assert_eq!(keys.get(VAULT_KEY.into()).unwrap().len(), 32);
    }
    #[test]
    fn missing_legacy_key_does_not_create_replacement() {
        let keys = Arc::new(Keys::default());
        let mut db = Sqlite::memory().unwrap();
        db.apply(&[Change::Blob { key: "state".into(), value: Some(br#"{"me":{}}"#.to_vec()) }]).unwrap();
        let mut split = Split::new(db, keys.clone());
        assert!(split.load().is_err());
        assert!(keys.get(VAULT_KEY.into()).is_none());
    }
    struct Broken;
    impl KeyStore for Broken {
        fn get(&self, _: String) -> Option<Vec<u8>> {
            None
        }
        fn set(&self, _: String, _: Option<Vec<u8>>) {}
    }
    #[test]
    fn failed_keystore_write_does_not_open_a_node() {
        assert!(Split::new(Sqlite::memory().unwrap(), Arc::new(Broken)).load().is_err());
    }

    struct PendingCleanup {
        keys: Keys,
        block: AtomicBool,
    }
    impl KeyStore for PendingCleanup {
        fn get(&self, key: String) -> Option<Vec<u8>> {
            self.keys.get(key)
        }
        fn set(&self, key: String, value: Option<Vec<u8>>) {
            if key == KEYS && value.is_none() && self.block.load(Ordering::Relaxed) {
                return;
            }
            self.keys.set(key, value);
        }
    }

    #[test]
    fn reopened_vault_retries_legacy_key_cleanup_and_refuses_failed_deletion() {
        let path = std::env::temp_dir().join(format!("hoi-mobile-cleanup-{}.db", hoi_types::hex_encode(&hoi_crypto::random_bytes::<16>().unwrap())));
        let keys = Arc::new(PendingCleanup { keys: Keys::default(), block: AtomicBool::new(true) });
        keys.set(VAULT_KEY.into(), Some(vec![7; 32]));
        keys.set(KEYS.into(), Some(b"old direct identity key".to_vec()));
        drop(EncryptedSqlite::open(Sqlite::open(&path).unwrap(), LocalVault::create_platform([7; 32]).unwrap(), &[]).unwrap());
        // Simulate crash after ciphertext commit but before the old platform entry was erased.
        let mut split = Split::new(Sqlite::open(&path).unwrap(), keys.clone());
        assert!(split.load().is_err());
        assert!(keys.get(KEYS.into()).is_some());
        drop(split);
        keys.block.store(false, Ordering::Relaxed);
        let mut split = Split::new(Sqlite::open(&path).unwrap(), keys.clone());
        assert!(split.load().is_ok());
        assert!(keys.get(KEYS.into()).is_none());
        assert_eq!(keys.get(VAULT_KEY.into()).unwrap(), vec![7; 32]);
        drop(split);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}
