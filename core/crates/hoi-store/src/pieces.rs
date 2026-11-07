//! Pieces a holder keeps for scattered delivery (SCATTER.md §5.6, §5.11): opaque bytes under a
//! locator, with the day they expire, the creator's drop hash and the community drop key's id,
//! and nothing about who sent them. Read on demand: a holder's budget runs to gigabytes, so
//! pieces are never loaded whole at startup the way records and blobs are.
//!
//! A full store sheds, in order: expired pieces, then pieces nobody asked for or probed in 30
//! days (oldest first), and only then answers `Full`.

use std::collections::BTreeMap;

/// Days without a get or a probe after which a piece may be evicted to make room.
pub const IDLE_DAYS: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PieceMeta {
    /// The last UTC day (days since 1970) the piece is kept.
    pub expiry_day: u32,
    pub drop_hash: [u8; 32],
    pub drop_key_id: [u8; 16],
}

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    Stored,
    /// A piece is already kept under this locator; it is left as it was.
    Exists,
    Full,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PieceStats {
    pub count: u64,
    pub bytes: u64,
    pub budget: u64,
}

pub trait PieceStore: Send {
    fn put(&mut self, loc: &[u8; 32], meta: PieceMeta, bytes: &[u8], today: u32) -> Result<PutOutcome, String>;
    /// The piece, if kept and not expired; counts as asked for today.
    fn get(&mut self, loc: &[u8; 32], today: u32) -> Result<Option<Vec<u8>>, String>;
    /// Whether a piece is kept, and its length (a probe); counts as asked for today.
    fn has(&mut self, loc: &[u8; 32], today: u32) -> Result<Option<u32>, String>;
    fn meta(&self, loc: &[u8; 32]) -> Result<Option<PieceMeta>, String>;
    fn remove(&mut self, loc: &[u8; 32]) -> Result<bool, String>;
    /// Removes every piece whose expiry day has passed; returns how many.
    fn sweep(&mut self, today: u32) -> Result<u64, String>;
    fn stats(&self) -> PieceStats;
}

struct Entry {
    meta: PieceMeta,
    touched: u32,
    bytes: Vec<u8>,
}

/// Pieces in memory: tests, and nodes that hold only a little.
pub struct MemPieces {
    budget: u64,
    used: u64,
    pieces: BTreeMap<[u8; 32], Entry>,
}

impl MemPieces {
    pub fn new(budget: u64) -> MemPieces {
        MemPieces { budget, used: 0, pieces: BTreeMap::new() }
    }

    fn evict_for(&mut self, need: u64, today: u32) {
        let _ = self.sweep(today);
        if self.used + need <= self.budget {
            return;
        }
        let mut idle: Vec<([u8; 32], u32)> =
            self.pieces.iter().filter(|(_, e)| today.saturating_sub(e.touched) >= IDLE_DAYS).map(|(k, e)| (*k, e.touched)).collect();
        idle.sort_by_key(|(k, t)| (*t, *k));
        for (k, _) in idle {
            if self.used + need <= self.budget {
                break;
            }
            if let Some(e) = self.pieces.remove(&k) {
                self.used -= e.bytes.len() as u64;
            }
        }
    }
}

impl PieceStore for MemPieces {
    fn put(&mut self, loc: &[u8; 32], meta: PieceMeta, bytes: &[u8], today: u32) -> Result<PutOutcome, String> {
        if self.pieces.contains_key(loc) {
            return Ok(PutOutcome::Exists);
        }
        let need = bytes.len() as u64;
        if self.used + need > self.budget {
            self.evict_for(need, today);
            if self.used + need > self.budget {
                return Ok(PutOutcome::Full);
            }
        }
        self.used += need;
        self.pieces.insert(*loc, Entry { meta, touched: today, bytes: bytes.to_vec() });
        Ok(PutOutcome::Stored)
    }

    fn get(&mut self, loc: &[u8; 32], today: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(self.pieces.get_mut(loc).filter(|e| e.meta.expiry_day >= today).map(|e| {
            e.touched = today;
            e.bytes.clone()
        }))
    }

    fn has(&mut self, loc: &[u8; 32], today: u32) -> Result<Option<u32>, String> {
        Ok(self.pieces.get_mut(loc).filter(|e| e.meta.expiry_day >= today).map(|e| {
            e.touched = today;
            e.bytes.len() as u32
        }))
    }

    fn meta(&self, loc: &[u8; 32]) -> Result<Option<PieceMeta>, String> {
        Ok(self.pieces.get(loc).map(|e| e.meta))
    }

    fn remove(&mut self, loc: &[u8; 32]) -> Result<bool, String> {
        Ok(match self.pieces.remove(loc) {
            Some(e) => {
                self.used -= e.bytes.len() as u64;
                true
            }
            None => false,
        })
    }

    fn sweep(&mut self, today: u32) -> Result<u64, String> {
        let expired: Vec<[u8; 32]> = self.pieces.iter().filter(|(_, e)| e.meta.expiry_day < today).map(|(k, _)| *k).collect();
        for k in &expired {
            self.remove(k)?;
        }
        Ok(expired.len() as u64)
    }

    fn stats(&self) -> PieceStats {
        PieceStats { count: self.pieces.len() as u64, bytes: self.used, budget: self.budget }
    }
}

/// Pieces in a SQLite file of their own, next to the node's store and never in its vault: they
/// are ciphertext the node cannot read, and the metadata beside them names nobody.
#[cfg(feature = "sqlite")]
pub struct SqlitePieces {
    conn: rusqlite::Connection,
    budget: u64,
    used: u64,
    count: u64,
}

#[cfg(feature = "sqlite")]
impl SqlitePieces {
    pub fn open(path: &std::path::Path, budget: u64) -> Result<SqlitePieces, String> {
        Self::init(rusqlite::Connection::open(path).map_err(|e| e.to_string())?, budget)
    }

    pub fn memory(budget: u64) -> Result<SqlitePieces, String> {
        Self::init(rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?, budget)
    }

    fn init(conn: rusqlite::Connection, budget: u64) -> Result<SqlitePieces, String> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS pieces(
               loc BLOB PRIMARY KEY, expiry_day INTEGER NOT NULL, touched_day INTEGER NOT NULL,
               drop_hash BLOB NOT NULL, drop_key_id BLOB NOT NULL, bytes BLOB NOT NULL);
             CREATE INDEX IF NOT EXISTS pieces_expiry ON pieces(expiry_day);
             CREATE INDEX IF NOT EXISTS pieces_touched ON pieces(touched_day);",
        )
        .map_err(|e| e.to_string())?;
        let (count, used): (i64, i64) =
            conn.query_row("SELECT COUNT(*), COALESCE(SUM(LENGTH(bytes)), 0) FROM pieces", [], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
        Ok(SqlitePieces { conn, budget, used: used as u64, count: count as u64 })
    }

    fn evict_for(&mut self, need: u64, today: u32) -> Result<(), String> {
        self.sweep(today)?;
        while self.used + need > self.budget {
            let idle: Option<(Vec<u8>, i64)> = self
                .conn
                .query_row(
                    "SELECT loc, LENGTH(bytes) FROM pieces WHERE touched_day <= ?1 ORDER BY touched_day, loc LIMIT 1",
                    [today.saturating_sub(IDLE_DAYS) as i64],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map(Some)
                .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e.to_string()) })?;
            let Some((loc, len)) = idle else { break };
            self.conn.execute("DELETE FROM pieces WHERE loc = ?1", [&loc]).map_err(|e| e.to_string())?;
            self.used -= len as u64;
            self.count -= 1;
        }
        Ok(())
    }
}

#[cfg(feature = "sqlite")]
impl PieceStore for SqlitePieces {
    fn put(&mut self, loc: &[u8; 32], meta: PieceMeta, bytes: &[u8], today: u32) -> Result<PutOutcome, String> {
        if self.meta(loc)?.is_some() {
            return Ok(PutOutcome::Exists);
        }
        let need = bytes.len() as u64;
        if self.used + need > self.budget {
            self.evict_for(need, today)?;
            if self.used + need > self.budget {
                return Ok(PutOutcome::Full);
            }
        }
        self.conn
            .execute(
                "INSERT INTO pieces(loc, expiry_day, touched_day, drop_hash, drop_key_id, bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![&loc[..], meta.expiry_day as i64, today as i64, &meta.drop_hash[..], &meta.drop_key_id[..], bytes],
            )
            .map_err(|e| e.to_string())?;
        self.used += need;
        self.count += 1;
        Ok(PutOutcome::Stored)
    }

    fn get(&mut self, loc: &[u8; 32], today: u32) -> Result<Option<Vec<u8>>, String> {
        let found: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT bytes FROM pieces WHERE loc = ?1 AND expiry_day >= ?2", rusqlite::params![&loc[..], today as i64], |r| r.get(0))
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e.to_string()) })?;
        if found.is_some() {
            self.conn.execute("UPDATE pieces SET touched_day = ?2 WHERE loc = ?1", rusqlite::params![&loc[..], today as i64]).map_err(|e| e.to_string())?;
        }
        Ok(found)
    }

    fn has(&mut self, loc: &[u8; 32], today: u32) -> Result<Option<u32>, String> {
        let n = self
            .conn
            .execute("UPDATE pieces SET touched_day = ?2 WHERE loc = ?1 AND expiry_day >= ?2", rusqlite::params![&loc[..], today as i64])
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        self.conn
            .query_row("SELECT LENGTH(bytes) FROM pieces WHERE loc = ?1", [&loc[..]], |r| r.get::<_, i64>(0))
            .map(|l| Some(l as u32))
            .map_err(|e| e.to_string())
    }

    fn meta(&self, loc: &[u8; 32]) -> Result<Option<PieceMeta>, String> {
        self.conn
            .query_row("SELECT expiry_day, drop_hash, drop_key_id FROM pieces WHERE loc = ?1", [&loc[..]], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?))
            })
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e.to_string()) })?
            .map(|(day, hash, id)| {
                Ok(PieceMeta {
                    expiry_day: day as u32,
                    drop_hash: hash.try_into().map_err(|_| "damaged drop hash".to_string())?,
                    drop_key_id: id.try_into().map_err(|_| "damaged drop key id".to_string())?,
                })
            })
            .transpose()
    }

    fn remove(&mut self, loc: &[u8; 32]) -> Result<bool, String> {
        let len: Option<i64> = self
            .conn
            .query_row("SELECT LENGTH(bytes) FROM pieces WHERE loc = ?1", [&loc[..]], |r| r.get(0))
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e.to_string()) })?;
        let Some(len) = len else { return Ok(false) };
        self.conn.execute("DELETE FROM pieces WHERE loc = ?1", [&loc[..]]).map_err(|e| e.to_string())?;
        self.used -= len as u64;
        self.count -= 1;
        Ok(true)
    }

    fn sweep(&mut self, today: u32) -> Result<u64, String> {
        let (n, bytes): (i64, i64) = self
            .conn
            .query_row("SELECT COUNT(*), COALESCE(SUM(LENGTH(bytes)), 0) FROM pieces WHERE expiry_day < ?1", [today as i64], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        if n > 0 {
            self.conn.execute("DELETE FROM pieces WHERE expiry_day < ?1", [today as i64]).map_err(|e| e.to_string())?;
            self.used -= bytes as u64;
            self.count -= n as u64;
        }
        Ok(n as u64)
    }

    fn stats(&self) -> PieceStats {
        PieceStats { count: self.count, bytes: self.used, budget: self.budget }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(expiry_day: u32) -> PieceMeta {
        PieceMeta { expiry_day, drop_hash: [1; 32], drop_key_id: [2; 16] }
    }

    fn exercise(s: &mut dyn PieceStore) {
        let today = 20_000;
        assert_eq!(s.put(&[1; 32], meta(today + 100), &[7; 100], today).unwrap(), PutOutcome::Stored);
        assert_eq!(s.put(&[1; 32], meta(today + 100), &[8; 100], today).unwrap(), PutOutcome::Exists, "never overwritten");
        assert_eq!(s.get(&[1; 32], today).unwrap(), Some(vec![7; 100]));
        assert_eq!(s.has(&[1; 32], today).unwrap(), Some(100));
        assert_eq!(s.has(&[9; 32], today).unwrap(), None);
        assert_eq!(s.meta(&[1; 32]).unwrap(), Some(meta(today + 100)));
        assert_eq!(s.get(&[9; 32], today).unwrap(), None);
        // fills to the budget (1,000 bytes), then sheds expired pieces before refusing
        assert_eq!(s.put(&[2; 32], meta(today), &[0; 400], today).unwrap(), PutOutcome::Stored);
        assert_eq!(s.put(&[3; 32], meta(today + 99), &[0; 400], today).unwrap(), PutOutcome::Stored);
        assert_eq!(s.put(&[4; 32], meta(today + 99), &[0; 400], today).unwrap(), PutOutcome::Full);
        assert_eq!(s.stats(), PieceStats { count: 3, bytes: 900, budget: 1000 });
        // a day later piece 2 has expired: it is no longer served, and gives way
        assert_eq!(s.get(&[2; 32], today + 1).unwrap(), None);
        assert_eq!(s.put(&[4; 32], meta(today + 99), &[0; 400], today + 1).unwrap(), PutOutcome::Stored);
        assert_eq!(s.stats().count, 3);
        // pieces nobody asked for in 30 days give way next; touched ones stay
        let later = today + 1 + IDLE_DAYS;
        assert!(s.has(&[1; 32], later).unwrap().is_some());
        assert_eq!(s.put(&[5; 32], meta(later + 5), &[0; 400], later).unwrap(), PutOutcome::Stored);
        assert!(s.meta(&[1; 32]).unwrap().is_some(), "probed today");
        assert!(s.meta(&[3; 32]).unwrap().is_none(), "idle, oldest first");
        assert!(s.remove(&[5; 32]).unwrap());
        assert!(!s.remove(&[5; 32]).unwrap());
        assert_eq!(s.sweep(later + 1000).unwrap(), 2);
        assert_eq!(s.stats(), PieceStats { count: 0, bytes: 0, budget: 1000 });
    }

    #[test]
    fn in_memory() {
        exercise(&mut MemPieces::new(1000));
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn in_sqlite_and_across_restarts() {
        exercise(&mut SqlitePieces::memory(1000).unwrap());
        let dir = std::env::temp_dir().join(format!("hoi-pieces-{}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        {
            let mut s = SqlitePieces::open(&dir, 10_000).unwrap();
            s.put(&[1; 32], meta(30_000), &[5; 123], 20_000).unwrap();
        }
        let mut s = SqlitePieces::open(&dir, 10_000).unwrap();
        assert_eq!(s.stats(), PieceStats { count: 1, bytes: 123, budget: 10_000 });
        assert_eq!(s.get(&[1; 32], 20_000).unwrap(), Some(vec![5; 123]));
        drop(s);
        let _ = std::fs::remove_file(&dir);
    }
}
