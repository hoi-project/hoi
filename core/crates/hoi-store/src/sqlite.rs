//! SQLite persistence for desktop, mobile and hoi-node. One file, records and small blobs.
//! The full indexed schema of TECHNICAL.md §12 (with FTS5) replaces the in-memory index once
//! communities outgrow it; the file format here is forward compatible with that (records keep
//! their canonical wire bytes).

use rusqlite::{Connection, params};

use crate::{Change, Loaded, Persist};

pub struct Sqlite {
    conn: Connection,
}

impl Sqlite {
    pub fn open(path: &std::path::Path) -> Result<Sqlite, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        Self::init(conn)
    }

    pub fn memory() -> Result<Sqlite, String> {
        Self::init(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }

    fn init(conn: Connection) -> Result<Sqlite, String> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA secure_delete = ON;
             CREATE TABLE IF NOT EXISTS records(
               id BLOB PRIMARY KEY, community BLOB, expires_at INTEGER NOT NULL, wire BLOB NOT NULL);
             CREATE INDEX IF NOT EXISTS records_expiry ON records(expires_at);
             CREATE TABLE IF NOT EXISTS blobs(key TEXT PRIMARY KEY, value BLOB NOT NULL);",
        )
        .map_err(|e| e.to_string())?;
        Ok(Sqlite { conn })
    }

    /// Includes row identities so an encrypted adapter can authenticate storage addresses.
    pub fn snapshot(&self) -> Result<Vec<Change>, String> {
        let mut out = Vec::new();
        let mut st = self.conn.prepare("SELECT id, community, expires_at, wire FROM records").map_err(|e| e.to_string())?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<Vec<u8>>>(1)?, r.get::<_, i64>(2)?, r.get::<_, Vec<u8>>(3)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, community, expires, wire) = row.map_err(|e| e.to_string())?;
            let id = hoi_types::Id::from_slice(&id).map_err(|e| e.to_string())?;
            let community = community.as_deref().map(hoi_types::Id::from_slice).transpose().map_err(|e| e.to_string())?;
            out.push(Change::Put { id, community, expires_at: expires as u64, wire });
        }
        let mut st = self.conn.prepare("SELECT key, value FROM blobs").map_err(|e| e.to_string())?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))).map_err(|e| e.to_string())?;
        for row in rows {
            let (key, value) = row.map_err(|e| e.to_string())?;
            out.push(Change::Blob { key, value: Some(value) });
        }
        Ok(out)
    }

    /// Flush old WAL frames after a plaintext-to-encrypted migration. secure_delete zeroes
    /// deleted live pages; this cannot erase filesystem snapshots or earlier backups.
    pub fn checkpoint(&self) -> Result<(), String> {
        let busy = self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())?;
        if busy != 0 {
            return Err("Close other processes using this storage and retry.".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn test_execute(&self, sql: &str) {
        self.conn.execute_batch(sql).unwrap();
    }
}

impl Persist for Sqlite {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        if changes.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        for c in changes {
            let r = match c {
                Change::Put { id, community, expires_at, wire } => tx.execute(
                    "INSERT OR REPLACE INTO records(id, community, expires_at, wire) VALUES (?1, ?2, ?3, ?4)",
                    params![&id.0[..], community.as_ref().map(|c| c.0.to_vec()), *expires_at as i64, wire],
                ),
                Change::Delete { id } => tx.execute("DELETE FROM records WHERE id = ?1", params![&id.0[..]]),
                Change::Blob { key, value: Some(v) } => tx.execute("INSERT OR REPLACE INTO blobs(key, value) VALUES (?1, ?2)", params![key, v]),
                Change::Blob { key, value: None } => tx.execute("DELETE FROM blobs WHERE key = ?1", params![key]),
                Change::Wipe => tx.execute_batch("DELETE FROM records; DELETE FROM blobs;").map(|_| 0),
            };
            r.map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    fn load(&mut self) -> Result<Loaded, String> {
        let mut out = Loaded::default();
        let mut st = self.conn.prepare("SELECT wire FROM records").map_err(|e| e.to_string())?;
        let rows = st.query_map([], |r| r.get::<_, Vec<u8>>(0)).map_err(|e| e.to_string())?;
        for r in rows {
            out.records.push(r.map_err(|e| e.to_string())?);
        }
        let mut st = self.conn.prepare("SELECT key, value FROM blobs").map_err(|e| e.to_string())?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))).map_err(|e| e.to_string())?;
        for r in rows {
            out.blobs.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_types::Id;

    #[test]
    fn put_load_delete() {
        let mut s = Sqlite::memory().unwrap();
        s.apply(&[
            Change::Put { id: Id([1; 32]), community: None, expires_at: 9, wire: vec![1, 2, 3] },
            Change::Blob { key: "state".into(), value: Some(b"{}".to_vec()) },
        ])
        .unwrap();
        let l = s.load().unwrap();
        assert_eq!(l.records, vec![vec![1, 2, 3]]);
        assert_eq!(l.blobs.len(), 1);
        s.apply(&[Change::Delete { id: Id([1; 32]) }, Change::Wipe]).unwrap();
        assert!(s.load().unwrap().records.is_empty());
    }
}
