//! Encrypted SQLite payloads and an authenticated inventory. The inventory binds every
//! ciphertext address and hash: deletion, injection and replay of individual old cells fail.
//! Rolling back an entire valid database remains outside local-only integrity protection.

use crate::{Change, Loaded, Persist, sqlite::Sqlite};
use hoi_crypto::vault::LocalVault;
use hoi_types::cbor::{self, Value};
use std::collections::BTreeMap;

pub const HEADER_KEY: &str = "vault";
pub const MANIFEST_KEY: &str = "vault-manifest";
pub const GUARD: &[u8] = b"{\"version\":4294967295}";
const PREFIX: &str = "v:";
const DAMAGED: &str = "Saved data is damaged or cannot be unlocked. No data was replaced.";
type Cells = BTreeMap<String, String>;

pub struct EncryptedSqlite {
    db: Sqlite,
    vault: LocalVault,
    cells: Cells,
}

fn encrypted_marker(c: &Change) -> bool {
    matches!(c, Change::Blob { key, value: Some(v) } if key.starts_with(PREFIX) || key == MANIFEST_KEY || (key == "format" && v == GUARD))
}

impl EncryptedSqlite {
    pub fn header(db: &Sqlite) -> Result<Option<Vec<u8>>, String> {
        let snapshot = db.snapshot()?;
        let header = snapshot.iter().find_map(|c| match c {
            Change::Blob { key, value } if key == HEADER_KEY => value.clone(),
            _ => None,
        });
        if header.is_none() && snapshot.iter().any(encrypted_marker) {
            return Err(DAMAGED.into());
        }
        if header.is_some() && !snapshot.iter().any(|c| matches!(c, Change::Blob { key, value: Some(_) } if key == MANIFEST_KEY)) {
            return Err(DAMAGED.into());
        }
        Ok(header)
    }

    /// `extra` imports old platform-keystore contents atomically with the migration.
    pub fn open(mut db: Sqlite, vault: LocalVault, extra: &[Change]) -> Result<Self, String> {
        let snapshot = db.snapshot()?;
        let header = snapshot.iter().find_map(|c| match c {
            Change::Blob { key, value: Some(v) } if key == HEADER_KEY => Some(v),
            _ => None,
        });
        if let Some(header) = header {
            if header != vault.header() || !extra.is_empty() {
                return Err(DAMAGED.into());
            }
            let mut store = Self { db, vault, cells: Cells::new() };
            let (_, cells) = store.decode(snapshot)?;
            store.cells = cells;
            // A previous migration may have committed just before a crash or busy reader.
            // Retry cleanup on every successful open before any plaintext is exposed.
            store.db.checkpoint()?;
            return Ok(store);
        }
        if snapshot.iter().any(encrypted_marker) {
            return Err(DAMAGED.into());
        }
        let mut changes = vec![Change::Wipe];
        let mut cells = Cells::new();
        for c in snapshot.iter().chain(extra) {
            let encrypted = Self::encrypt(&vault, c)?;
            Self::update_cells(&mut cells, &encrypted)?;
            changes.push(encrypted);
        }
        changes.extend(Self::metadata(&vault, &cells)?);
        db.apply(&changes)?;
        db.checkpoint()?;
        Ok(Self { db, vault, cells })
    }

    // Native manifests use the project's existing canonical CBOR implementation. Browser
    // manifests use JSON with the same schema; database files are platform-specific.
    fn metadata(vault: &LocalVault, cells: &Cells) -> Result<[Change; 3], String> {
        let manifest =
            Value::map([("version", Value::U(1)), ("cells", Value::map(cells.iter().map(|(address, hash)| (address.clone(), Value::T(hash.clone())))))])
                .encode();
        let sealed = vault.seal("manifest", &manifest).map_err(|_| DAMAGED.to_string())?;
        Ok([
            Change::Blob { key: HEADER_KEY.into(), value: Some(vault.header().to_vec()) },
            Change::Blob { key: "format".into(), value: Some(GUARD.to_vec()) },
            Change::Blob { key: MANIFEST_KEY.into(), value: Some(sealed) },
        ])
    }

    fn decode_manifest(&self, cipher: &[u8]) -> Result<Cells, String> {
        let plain = self.vault.unseal("manifest", cipher).map_err(|_| DAMAGED.to_string())?;
        let Value::M(fields) = cbor::decode(&plain).map_err(|_| DAMAGED.to_string())? else {
            return Err(DAMAGED.into());
        };
        if fields.len() != 2 || !fields.iter().any(|(key, value)| key == "version" && *value == Value::U(1)) {
            return Err(DAMAGED.into());
        }
        let Some(Value::M(entries)) = fields.iter().find(|(key, _)| key == "cells").map(|(_, value)| value) else {
            return Err(DAMAGED.into());
        };
        entries
            .iter()
            .map(|(address, value)| {
                let Value::T(hash) = value else {
                    return Err(DAMAGED.to_string());
                };
                if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
                    return Err(DAMAGED.into());
                }
                Ok((address.clone(), hash.clone()))
            })
            .collect()
    }

    fn encrypt(vault: &LocalVault, c: &Change) -> Result<Change, String> {
        match c {
            Change::Put { id, community, expires_at, wire } => Ok(Change::Put {
                id: *id,
                community: *community,
                expires_at: *expires_at,
                wire: vault.seal(&format!("record:{}", id.hex()), wire).map_err(|_| DAMAGED.to_string())?,
            }),
            Change::Blob { key, value } => Ok(Change::Blob {
                key: format!("{PREFIX}{key}"),
                value: value.as_ref().map(|v| vault.seal(&format!("blob:{key}"), v)).transpose().map_err(|_| DAMAGED.to_string())?,
            }),
            c => Ok(c.clone()),
        }
    }

    fn update_cells(cells: &mut Cells, change: &Change) -> Result<(), String> {
        match change {
            Change::Put { id, wire, .. } => {
                cells.insert(format!("record:{}", id.hex()), LocalVault::fingerprint(wire));
            }
            Change::Delete { id } => {
                cells.remove(&format!("record:{}", id.hex()));
            }
            Change::Blob { key, value } => {
                let key = key.strip_prefix(PREFIX).ok_or(DAMAGED)?;
                let address = format!("blob:{key}");
                if let Some(cipher) = value {
                    cells.insert(address, LocalVault::fingerprint(cipher));
                } else {
                    cells.remove(&address);
                }
            }
            Change::Wipe => cells.clear(),
        }
        Ok(())
    }

    fn decode(&self, snapshot: Vec<Change>) -> Result<(Loaded, Cells), String> {
        let mut loaded = Loaded::default();
        let mut guard = false;
        let mut header = false;
        let mut manifest = None;
        let mut actual = Cells::new();
        for c in snapshot {
            match c {
                Change::Put { id, wire, .. } => {
                    let address = format!("record:{}", id.hex());
                    actual.insert(address.clone(), LocalVault::fingerprint(&wire));
                    loaded.records.push(self.vault.unseal(&address, &wire).map_err(|_| DAMAGED.to_string())?);
                }
                Change::Blob { key, value: Some(v) } if key == HEADER_KEY && v == self.vault.header() => header = true,
                Change::Blob { key, value: Some(v) } if key == "format" && v == GUARD => guard = true,
                Change::Blob { key, value: Some(v) } if key == MANIFEST_KEY => manifest = Some(self.decode_manifest(&v)?),
                Change::Blob { key, value: Some(v) } if key.starts_with(PREFIX) => {
                    let key = key[PREFIX.len()..].to_string();
                    let address = format!("blob:{key}");
                    actual.insert(address.clone(), LocalVault::fingerprint(&v));
                    loaded.blobs.push((key, self.vault.unseal(&address, &v).map_err(|_| DAMAGED.to_string())?));
                }
                _ => return Err(DAMAGED.into()),
            }
        }
        if !guard || !header || manifest.as_ref() != Some(&actual) {
            return Err(DAMAGED.into());
        }
        Ok((loaded, actual))
    }
}

impl Persist for EncryptedSqlite {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        if changes.is_empty() {
            return Ok(());
        }
        let mut next = self.cells.clone();
        let mut encrypted = Vec::with_capacity(changes.len() + 3);
        for c in changes {
            let c = Self::encrypt(&self.vault, c)?;
            Self::update_cells(&mut next, &c)?;
            encrypted.push(c);
        }
        encrypted.extend(Self::metadata(&self.vault, &next)?);
        self.db.apply(&encrypted)?;
        self.cells = next; // only the committed inventory can authorize the next transaction
        Ok(())
    }

    fn load(&mut self) -> Result<Loaded, String> {
        let (loaded, cells) = self.decode(self.db.snapshot()?)?;
        self.cells = cells;
        Ok(loaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_types::Id;

    fn protected() -> EncryptedSqlite {
        let mut store = EncryptedSqlite::open(Sqlite::memory().unwrap(), LocalVault::create_platform([3; 32]).unwrap(), &[]).unwrap();
        store
            .apply(&[
                Change::Put { id: Id([1; 32]), community: None, expires_at: 1, wire: b"signed record".to_vec() },
                Change::Blob { key: "keys".into(), value: Some(b"identity keys".to_vec()) },
                Change::Blob { key: "mls".into(), value: Some(b"private group".to_vec()) },
                Change::Blob { key: "state".into(), value: Some(b"private boundaries".to_vec()) },
            ])
            .unwrap();
        store
    }

    #[test]
    fn inventory_rejects_deleted_critical_blobs_records_and_manifest() {
        for key in ["v:keys", "v:mls", "v:state", MANIFEST_KEY] {
            let mut store = protected();
            store.db.apply(&[Change::Blob { key: key.into(), value: None }]).unwrap();
            assert!(store.load().is_err(), "deletion of {key} must fail closed");
        }
        let mut store = protected();
        store.db.apply(&[Change::Delete { id: Id([1; 32]) }]).unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn inventory_rejects_old_cells_old_manifest_and_injected_valid_cells() {
        let mut store = protected();
        let old = store.db.snapshot().unwrap();
        let old_cell = old.iter().find(|c| matches!(c, Change::Blob { key, .. } if key == "v:mls")).unwrap().clone();
        let old_manifest = old.iter().find(|c| matches!(c, Change::Blob { key, .. } if key == MANIFEST_KEY)).unwrap().clone();
        store.apply(&[Change::Blob { key: "mls".into(), value: Some(b"rotated group secret".to_vec()) }]).unwrap();
        let current = store.db.snapshot().unwrap();
        store.db.apply(&[old_cell]).unwrap();
        assert!(store.load().is_err(), "an old valid ciphertext must not replace the latest cell");
        store.db.apply(&current).unwrap();
        store.db.apply(&[old_manifest]).unwrap();
        assert!(store.load().is_err(), "an old valid inventory must not bless newer cells");
        store.db.apply(&current).unwrap();
        let extra = store.vault.seal("blob:injected", b"valid but unauthorized").unwrap();
        store.db.apply(&[Change::Blob { key: "v:injected".into(), value: Some(extra) }]).unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn failed_transaction_keeps_the_last_committed_inventory_and_can_retry() {
        let mut store = protected();
        let before = store.db.snapshot().unwrap();
        let cells = store.cells.clone();
        store.db.test_execute("CREATE TABLE fail_control(enabled INTEGER); INSERT INTO fail_control VALUES (1); CREATE TRIGGER fail_manifest BEFORE INSERT ON blobs WHEN NEW.key = 'vault-manifest' AND (SELECT enabled FROM fail_control) = 1 BEGIN SELECT RAISE(FAIL, 'test storage failure'); END;");
        let changes = [Change::Blob { key: "mls".into(), value: Some(b"new secret".to_vec()) }];
        assert!(store.apply(&changes).is_err());
        assert_eq!(store.cells, cells);
        assert_eq!(store.db.snapshot().unwrap(), before);
        store.db.test_execute("UPDATE fail_control SET enabled = 0;");
        store.apply(&changes).unwrap();
        assert!(store.load().unwrap().blobs.iter().any(|(key, value)| key == "mls" && value == b"new secret"));
    }

    #[test]
    fn reopening_retries_a_checkpoint_interrupted_after_migration_commit() {
        let path = std::env::temp_dir().join(format!("hoi-vault-retry-{}.db", hoi_types::hex_encode(&hoi_crypto::random_bytes::<16>().unwrap())));
        let secret = b"plaintext key before interrupted cleanup bdc0196649f7";
        let mut raw = Sqlite::open(&path).unwrap();
        raw.test_execute("PRAGMA busy_timeout=0;");
        raw.apply(&[Change::Blob { key: "keys".into(), value: Some(secret.to_vec()) }]).unwrap();
        let reader = rusqlite::Connection::open(&path).unwrap();
        reader.execute_batch("BEGIN;").unwrap();
        let _: Vec<u8> = reader.query_row("SELECT value FROM blobs WHERE key = 'keys'", [], |row| row.get(0)).unwrap();
        let vault = LocalVault::create_platform([5; 32]).unwrap();
        let header = vault.header().to_vec();
        assert!(EncryptedSqlite::open(raw, vault, &[]).is_err(), "the reader prevents WAL cleanup");
        let raw = Sqlite::open(&path).unwrap();
        raw.test_execute("PRAGMA busy_timeout=0;");
        assert_eq!(EncryptedSqlite::header(&raw).unwrap(), Some(header.clone()), "migration itself already committed");
        assert!(EncryptedSqlite::open(raw, LocalVault::unlock_platform(&header, [5; 32]).unwrap(), &[]).is_err());
        reader.execute_batch("COMMIT;").unwrap();
        drop(reader);
        let store = EncryptedSqlite::open(Sqlite::open(&path).unwrap(), LocalVault::unlock_platform(&header, [5; 32]).unwrap(), &[]).unwrap();
        for suffix in ["", "-wal"] {
            if let Ok(bytes) = std::fs::read(format!("{}{suffix}", path.display())) {
                assert!(!bytes.windows(secret.len()).any(|window| window == secret));
            }
        }
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn migration_removes_plaintext_from_current_database_and_wal() {
        let name = format!("hoi-vault-{}.db", hoi_types::hex_encode(&hoi_crypto::random_bytes::<16>().unwrap()));
        let path = std::env::temp_dir().join(name);
        let secret = b"unique previously plaintext private key and MLS data 639a8c42f65d";
        let mut raw = Sqlite::open(&path).unwrap();
        raw.apply(&[Change::Blob { key: "keys".into(), value: Some(secret.to_vec()) }]).unwrap();
        let vault = LocalVault::create_platform([9; 32]).unwrap();
        let header = vault.header().to_vec();
        let store = EncryptedSqlite::open(raw, vault, &[]).unwrap();
        for file in [path.clone(), std::path::PathBuf::from(format!("{}-wal", path.display()))] {
            if let Ok(bytes) = std::fs::read(file) {
                assert!(!bytes.windows(secret.len()).any(|window| window == secret));
            }
        }
        drop(store);
        let mut reopened = EncryptedSqlite::open(Sqlite::open(&path).unwrap(), LocalVault::unlock_platform(&header, [9; 32]).unwrap(), &[]).unwrap();
        assert!(reopened.load().unwrap().blobs.iter().any(|(key, value)| key == "keys" && value == secret));
        drop(reopened);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn migrate_encrypt_reopen_tamper_and_wipe() {
        let mut raw = Sqlite::memory().unwrap();
        raw.apply(&[
            Change::Put { id: Id([1; 32]), community: None, expires_at: 12, wire: b"a private record".to_vec() },
            Change::Blob { key: "keys".into(), value: Some(b"identity secret".to_vec()) },
            Change::Blob { key: "mls".into(), value: Some(b"group secret".to_vec()) },
        ])
        .unwrap();
        let key = [42; 32];
        let vault = LocalVault::create_platform(key).unwrap();
        let mut store = EncryptedSqlite::open(raw, vault, &[]).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.records[0], b"a private record");
        assert!(loaded.blobs.iter().any(|(k, v)| k == "mls" && v == b"group secret"));
        let disk = store.db.load().unwrap();
        assert!(disk.blobs.iter().all(|(k, v)| k != "keys" && v != b"identity secret" && v != b"group secret"));
        assert_ne!(disk.records[0], b"a private record");
        let mut bad = disk.blobs.iter().find(|(k, _)| k == "v:mls").unwrap().1.clone();
        *bad.last_mut().unwrap() ^= 1;
        store.db.apply(&[Change::Blob { key: "v:mls".into(), value: Some(bad) }]).unwrap();
        assert!(store.load().is_err());
        store.apply(&[Change::Wipe]).unwrap();
        assert!(store.load().unwrap().blobs.is_empty());
        assert!(EncryptedSqlite::header(&store.db).unwrap().is_some());
    }

    #[test]
    fn removed_header_and_swapped_cells_fail_closed() {
        let vault = LocalVault::create_platform([4; 32]).unwrap();
        let mut store = EncryptedSqlite::open(Sqlite::memory().unwrap(), vault, &[]).unwrap();
        store.apply(&[Change::Blob { key: "keys".into(), value: Some(b"identity".to_vec()) }]).unwrap();
        let cipher = store.db.load().unwrap().blobs.into_iter().find(|(k, _)| k == "v:keys").unwrap().1;
        store.db.apply(&[Change::Blob { key: "v:mls".into(), value: Some(cipher) }]).unwrap();
        assert!(store.load().is_err());
        store.db.apply(&[Change::Blob { key: HEADER_KEY.into(), value: None }]).unwrap();
        assert!(EncryptedSqlite::open(store.db, LocalVault::create_platform([8; 32]).unwrap(), &[]).is_err());
    }
}
