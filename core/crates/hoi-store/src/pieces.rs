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

/// Reserve one bounded metadata slot per admitted piece, even after its payload is dropped.
/// One slot per 128 bytes of payload budget bounds metadata independently of piece size
/// (RFC-0001); this is a count limit, not a measurement of SQLite's physical disk usage.
const MARKER_SLOT_BYTES: u64 = 128;

fn marker_slots(budget: u64) -> u64 {
    (budget / MARKER_SLOT_BYTES).max(1)
}

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
    /// An authorized drop still protects this locator from replay.
    Dropped,
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
    /// Authentication metadata of an unexpired piece or authorized-drop marker.
    fn drop_meta(&self, loc: &[u8; 32], today: u32) -> Result<Option<PieceMeta>, String>;
    /// After authenticating against `drop_meta`, replace the payload with its marker.
    /// A repeat returns true without extending the original inclusive expiry day.
    fn mark_dropped(&mut self, loc: &[u8; 32], today: u32) -> Result<bool, String>;
    /// Whether a successful marker write is durable across restarts.
    fn durable_drops(&self) -> bool {
        false
    }
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
    dropped: BTreeMap<[u8; 32], PieceMeta>,
}

impl MemPieces {
    pub fn new(budget: u64) -> MemPieces {
        MemPieces { budget, used: 0, pieces: BTreeMap::new(), dropped: BTreeMap::new() }
    }

    fn fits(&self, need: u64) -> bool {
        need <= self.budget.saturating_sub(self.used) && ((self.pieces.len() + self.dropped.len()) as u64) < marker_slots(self.budget)
    }

    fn evict_for(&mut self, need: u64, today: u32) {
        let _ = self.sweep(today);
        if self.fits(need) {
            return;
        }
        let mut idle: Vec<([u8; 32], u32)> =
            self.pieces.iter().filter(|(_, e)| today.saturating_sub(e.touched) >= IDLE_DAYS).map(|(k, e)| (*k, e.touched)).collect();
        idle.sort_by_key(|(k, t)| (*t, *k));
        for (k, _) in idle {
            if self.fits(need) {
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
        if self.dropped.get(loc).is_some_and(|m| m.expiry_day >= today) {
            return Ok(PutOutcome::Dropped);
        }
        self.dropped.remove(loc);
        if self.pieces.contains_key(loc) {
            return Ok(PutOutcome::Exists);
        }
        let need = bytes.len() as u64;
        if !self.fits(need) {
            self.evict_for(need, today);
            if !self.fits(need) {
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

    fn drop_meta(&self, loc: &[u8; 32], today: u32) -> Result<Option<PieceMeta>, String> {
        Ok(self.dropped.get(loc).copied().or_else(|| self.pieces.get(loc).map(|e| e.meta)).filter(|m| m.expiry_day >= today))
    }

    fn mark_dropped(&mut self, loc: &[u8; 32], today: u32) -> Result<bool, String> {
        let Some(meta) = self.drop_meta(loc, today)? else { return Ok(false) };
        self.remove(loc)?;
        self.dropped.insert(*loc, meta);
        Ok(true)
    }

    fn sweep(&mut self, today: u32) -> Result<u64, String> {
        self.dropped.retain(|_, m| m.expiry_day >= today);
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
    durable: bool,
    budget: u64,
    used: u64,
    count: u64,
    marked: u64,
}

#[cfg(feature = "sqlite")]
impl SqlitePieces {
    pub fn open(path: &std::path::Path, budget: u64) -> Result<SqlitePieces, String> {
        Self::init(rusqlite::Connection::open(path).map_err(|e| e.to_string())?, budget)
    }

    pub fn memory(budget: u64) -> Result<SqlitePieces, String> {
        Self::init(rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?, budget)
    }

    /// Open a supported snapshot restore. Payloads may predate unknown drops, so discard them
    /// before returning a store that can serve requests; available markers survive (RFC-0001).
    pub fn restore(path: &std::path::Path, budget: u64) -> Result<SqlitePieces, String> {
        let mut store = Self::open(path, budget)?;
        store.discard_payloads()?;
        Ok(store)
    }

    /// Stop holding payloads without forgetting acknowledged drops, including across re-enable.
    pub fn discard_payloads(&mut self) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM held_pieces", []).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.count = 0;
        self.used = 0;
        Ok(())
    }

    fn init(mut conn: rusqlite::Connection, budget: u64) -> Result<SqlitePieces, String> {
        let durable = conn.path().is_some_and(|path| !path.is_empty());
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if !(0..=1).contains(&version) {
            return Err("This piece database requires a newer storage implementation.".into());
        }
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL; PRAGMA secure_delete = ON;").map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        if version == 0 {
            let legacy: bool = tx
                .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'pieces')", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if legacy {
                tx.execute_batch("ALTER TABLE pieces RENAME TO held_pieces; DROP INDEX IF EXISTS pieces_expiry; DROP INDEX IF EXISTS pieces_touched;")
                    .map_err(|e| e.to_string())?;
            }
            // Older releases ignore user_version. This incompatible sentinel makes their
            // initialization fail before they can accept replay while ignoring live markers.
            tx.execute_batch("CREATE TABLE pieces(requires_drop_journal INTEGER NOT NULL);").map_err(|e| e.to_string())?;
        }
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS held_pieces(
               loc BLOB PRIMARY KEY, expiry_day INTEGER NOT NULL, touched_day INTEGER NOT NULL,
               drop_hash BLOB NOT NULL, drop_key_id BLOB NOT NULL, bytes BLOB NOT NULL);
             CREATE INDEX IF NOT EXISTS held_pieces_expiry ON held_pieces(expiry_day);
             CREATE INDEX IF NOT EXISTS held_pieces_touched ON held_pieces(touched_day);
             CREATE TABLE IF NOT EXISTS dropped_pieces(
               loc BLOB PRIMARY KEY, expiry_day INTEGER NOT NULL, drop_hash BLOB NOT NULL, drop_key_id BLOB NOT NULL);
             CREATE INDEX IF NOT EXISTS dropped_pieces_expiry ON dropped_pieces(expiry_day);
             PRAGMA user_version = 1;",
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        let (count, used): (i64, i64) = conn
            .query_row("SELECT COUNT(*), COALESCE(SUM(LENGTH(bytes)), 0) FROM held_pieces", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        let marked: i64 = conn.query_row("SELECT COUNT(*) FROM dropped_pieces", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        Ok(SqlitePieces { conn, durable, budget, used: used as u64, count: count as u64, marked: marked as u64 })
    }

    fn fits(&self, need: u64) -> bool {
        need <= self.budget.saturating_sub(self.used) && self.count + self.marked < marker_slots(self.budget)
    }

    fn evict_for(&mut self, need: u64, today: u32) -> Result<(), String> {
        self.sweep(today)?;
        while !self.fits(need) {
            let idle: Option<(Vec<u8>, i64)> = self
                .conn
                .query_row(
                    "SELECT loc, LENGTH(bytes) FROM held_pieces WHERE touched_day <= ?1 ORDER BY touched_day, loc LIMIT 1",
                    [today.saturating_sub(IDLE_DAYS) as i64],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map(Some)
                .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e.to_string()) })?;
            let Some((loc, len)) = idle else { break };
            self.conn.execute("DELETE FROM held_pieces WHERE loc = ?1", [&loc]).map_err(|e| e.to_string())?;
            self.used -= len as u64;
            self.count -= 1;
        }
        Ok(())
    }
}

#[cfg(feature = "sqlite")]
impl PieceStore for SqlitePieces {
    fn put(&mut self, loc: &[u8; 32], meta: PieceMeta, bytes: &[u8], today: u32) -> Result<PutOutcome, String> {
        let marked: bool = self
            .conn
            .query_row("SELECT EXISTS(SELECT 1 FROM dropped_pieces WHERE loc = ?1 AND expiry_day >= ?2)", rusqlite::params![&loc[..], today as i64], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if marked {
            return Ok(PutOutcome::Dropped);
        }
        if self.meta(loc)?.is_some() {
            return Ok(PutOutcome::Exists);
        }
        let need = bytes.len() as u64;
        if !self.fits(need) {
            self.evict_for(need, today)?;
            if !self.fits(need) {
                return Ok(PutOutcome::Full);
            }
        }
        self.conn
            .execute(
                "INSERT INTO held_pieces(loc, expiry_day, touched_day, drop_hash, drop_key_id, bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
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
            .query_row("SELECT bytes FROM held_pieces WHERE loc = ?1 AND expiry_day >= ?2", rusqlite::params![&loc[..], today as i64], |r| r.get(0))
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e.to_string()) })?;
        if found.is_some() {
            self.conn
                .execute("UPDATE held_pieces SET touched_day = ?2 WHERE loc = ?1", rusqlite::params![&loc[..], today as i64])
                .map_err(|e| e.to_string())?;
        }
        Ok(found)
    }

    fn has(&mut self, loc: &[u8; 32], today: u32) -> Result<Option<u32>, String> {
        let n = self
            .conn
            .execute("UPDATE held_pieces SET touched_day = ?2 WHERE loc = ?1 AND expiry_day >= ?2", rusqlite::params![&loc[..], today as i64])
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        self.conn
            .query_row("SELECT LENGTH(bytes) FROM held_pieces WHERE loc = ?1", [&loc[..]], |r| r.get::<_, i64>(0))
            .map(|l| Some(l as u32))
            .map_err(|e| e.to_string())
    }

    fn meta(&self, loc: &[u8; 32]) -> Result<Option<PieceMeta>, String> {
        self.conn
            .query_row("SELECT expiry_day, drop_hash, drop_key_id FROM held_pieces WHERE loc = ?1", [&loc[..]], |r| {
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
            .query_row("SELECT LENGTH(bytes) FROM held_pieces WHERE loc = ?1", [&loc[..]], |r| r.get(0))
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e.to_string()) })?;
        let Some(len) = len else { return Ok(false) };
        self.conn.execute("DELETE FROM held_pieces WHERE loc = ?1", [&loc[..]]).map_err(|e| e.to_string())?;
        self.used -= len as u64;
        self.count -= 1;
        Ok(true)
    }

    fn drop_meta(&self, loc: &[u8; 32], today: u32) -> Result<Option<PieceMeta>, String> {
        self.conn
            .query_row(
                "SELECT expiry_day, drop_hash, drop_key_id FROM dropped_pieces WHERE loc = ?1 AND expiry_day >= ?2
                 UNION ALL SELECT expiry_day, drop_hash, drop_key_id FROM held_pieces WHERE loc = ?1 AND expiry_day >= ?2 LIMIT 1",
                rusqlite::params![&loc[..], today as i64],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?)),
            )
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e.to_string()) })?
            .map(|(day, hash, id)| {
                Ok(PieceMeta {
                    expiry_day: u32::try_from(day).map_err(|_| "damaged piece expiry".to_string())?,
                    drop_hash: hash.try_into().map_err(|_| "damaged drop hash".to_string())?,
                    drop_key_id: id.try_into().map_err(|_| "damaged drop key id".to_string())?,
                })
            })
            .transpose()
    }

    fn mark_dropped(&mut self, loc: &[u8; 32], today: u32) -> Result<bool, String> {
        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        let exists: bool = tx
            .query_row("SELECT EXISTS(SELECT 1 FROM dropped_pieces WHERE loc = ?1 AND expiry_day >= ?2)", rusqlite::params![&loc[..], today as i64], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if exists {
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(true);
        }
        let len: Option<i64> = tx
            .query_row("SELECT LENGTH(bytes) FROM held_pieces WHERE loc = ?1 AND expiry_day >= ?2", rusqlite::params![&loc[..], today as i64], |r| r.get(0))
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e.to_string()) })?;
        let Some(len) = len else { return Ok(false) };
        // A collected locator may be reused after expiry. Never extend an unexpired marker.
        let old = tx.execute("DELETE FROM dropped_pieces WHERE loc = ?1", [&loc[..]]).map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO dropped_pieces(loc, expiry_day, drop_hash, drop_key_id)
             SELECT loc, expiry_day, drop_hash, drop_key_id FROM held_pieces WHERE loc = ?1",
            [&loc[..]],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM held_pieces WHERE loc = ?1", [&loc[..]]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.used -= len as u64;
        self.count -= 1;
        self.marked += 1 - old as u64;
        Ok(true)
    }

    fn durable_drops(&self) -> bool {
        self.durable
    }

    fn sweep(&mut self, today: u32) -> Result<u64, String> {
        let (n, bytes): (i64, i64) = self
            .conn
            .query_row("SELECT COUNT(*), COALESCE(SUM(LENGTH(bytes)), 0) FROM held_pieces WHERE expiry_day < ?1", [today as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(|e| e.to_string())?;
        if n > 0 {
            self.conn.execute("DELETE FROM held_pieces WHERE expiry_day < ?1", [today as i64]).map_err(|e| e.to_string())?;
            self.used -= bytes as u64;
            self.count -= n as u64;
        }
        let expired = self.conn.execute("DELETE FROM dropped_pieces WHERE expiry_day < ?1", [today as i64]).map_err(|e| e.to_string())?;
        self.marked -= expired as u64;
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

    fn exercise_drop_markers(s: &mut dyn PieceStore) {
        let day = 20_000;
        let loc = [1; 32];
        assert!(!s.mark_dropped(&loc, day).unwrap(), "unknown locators cannot gain a marker");
        assert_eq!(s.put(&loc, meta(day + 10), &[7; 100], day).unwrap(), PutOutcome::Stored);
        assert_eq!(s.put(&[2; 32], meta(day + 100), &[8; 100], day).unwrap(), PutOutcome::Stored);
        assert!(s.mark_dropped(&loc, day).unwrap());
        assert!(s.mark_dropped(&loc, day + 5).unwrap(), "authenticated repeats have the same outcome");
        assert_eq!(s.drop_meta(&loc, day + 5).unwrap(), Some(meta(day + 10)), "retries never extend retention");
        assert_eq!(s.get(&loc, day).unwrap(), None);
        assert_eq!(s.has(&loc, day).unwrap(), None);
        assert_eq!(s.meta(&loc).unwrap(), None, "a marker is not a stored payload");
        assert!(!s.remove(&loc).unwrap(), "ordinary cache removal must not erase a marker");
        assert_eq!(s.get(&[2; 32], day).unwrap(), Some(vec![8; 100]), "unrelated content survives");
        assert_eq!(s.put(&loc, meta(day + 200), &[9; 100], day + 10).unwrap(), PutOutcome::Dropped);
        s.sweep(day + 10).unwrap();
        assert_eq!(s.put(&loc, meta(day + 200), &[7; 100], day + 10).unwrap(), PutOutcome::Dropped, "expiry day is inclusive");
        assert!(!s.mark_dropped(&loc, day + 11).unwrap(), "an expired marker cannot acknowledge a new request");
        s.sweep(day + 11).unwrap();
        assert_eq!(s.drop_meta(&loc, day + 11).unwrap(), None);
        assert_eq!(s.put(&loc, meta(day + 200), &[9; 100], day + 11).unwrap(), PutOutcome::Stored);
        assert!(s.remove(&loc).unwrap());
        assert_eq!(s.drop_meta(&loc, day + 11).unwrap(), None, "eviction is not authorized deletion");
    }

    fn exercise_marker_capacity(s: &mut dyn PieceStore) {
        let day = 20_000;
        // A 256-byte budget reserves two metadata slots, including for empty payloads.
        for id in [1, 2] {
            assert_eq!(s.put(&[id; 32], meta(day + 100), &[], day).unwrap(), PutOutcome::Stored);
            assert!(s.mark_dropped(&[id; 32], day).unwrap());
        }
        assert_eq!(s.stats().bytes, 0);
        assert_eq!(s.put(&[3; 32], meta(day + 200), &[], day + 99).unwrap(), PutOutcome::Full);
        assert_eq!(s.put(&[1; 32], meta(day + 200), &[], day + 99).unwrap(), PutOutcome::Dropped);
        assert_eq!(s.put(&[3; 32], meta(day + 200), &[], day + 101).unwrap(), PutOutcome::Stored, "expired markers release capacity");
    }

    #[test]
    fn memory_markers_block_replay_through_expiry_without_claiming_durability() {
        let mut s = MemPieces::new(1000);
        assert!(!s.durable_drops());
        exercise_drop_markers(&mut s);
        exercise_marker_capacity(&mut MemPieces::new(256));
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_markers_block_replay_and_reserve_capacity() {
        let mut s = SqlitePieces::memory(1000).unwrap();
        assert!(!s.durable_drops(), "in-memory SQLite cannot advertise restart durability");
        exercise_drop_markers(&mut s);
        exercise_marker_capacity(&mut SqlitePieces::memory(256).unwrap());
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn failed_marker_transaction_leaves_payload_and_accounting_retryable() {
        let mut s = SqlitePieces::memory(1000).unwrap();
        s.put(&[1; 32], meta(20_100), &[7; 100], 20_000).unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_drop BEFORE DELETE ON held_pieces BEGIN SELECT RAISE(ABORT, 'disk failure'); END;").unwrap();
        assert!(s.mark_dropped(&[1; 32], 20_000).is_err());
        assert_eq!(s.stats().count, 1);
        assert_eq!(s.stats().bytes, 100);
        assert_eq!(s.marked, 0);
        assert_eq!(s.get(&[1; 32], 20_000).unwrap(), Some(vec![7; 100]));
        let markers: i64 = s.conn.query_row("SELECT COUNT(*) FROM dropped_pieces", [], |r| r.get(0)).unwrap();
        assert_eq!(markers, 0, "marker insertion rolls back with the failed removal");
        s.conn.execute_batch("DROP TRIGGER fail_drop;").unwrap();
        assert!(s.mark_dropped(&[1; 32], 20_000).unwrap());
        assert_eq!(s.stats().count, 0);
        assert_eq!(s.marked, 1);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn legacy_payloads_migrate_and_old_or_future_formats_fail_closed() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE pieces(loc BLOB PRIMARY KEY, expiry_day INTEGER NOT NULL, touched_day INTEGER NOT NULL,
              drop_hash BLOB NOT NULL, drop_key_id BLOB NOT NULL, bytes BLOB NOT NULL);
             CREATE INDEX pieces_expiry ON pieces(expiry_day); CREATE INDEX pieces_touched ON pieces(touched_day);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pieces VALUES (?1, 20100, 20000, ?2, ?3, ?4)",
            rusqlite::params![&[1u8; 32][..], &[1u8; 32][..], &[2u8; 16][..], &[7u8; 100][..]],
        )
        .unwrap();
        let mut s = SqlitePieces::init(conn, 1000).unwrap();
        assert_eq!(s.get(&[1; 32], 20_000).unwrap(), Some(vec![7; 100]));
        assert_eq!(s.marked, 0, "migration never invents historical acknowledgements");
        // These are the older implementation's initialization statements, not a version check
        // that old binaries never made. They must fail before any requests can be served.
        assert!(
            s.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS pieces(loc BLOB PRIMARY KEY, expiry_day INTEGER NOT NULL, touched_day INTEGER NOT NULL,
             drop_hash BLOB NOT NULL, drop_key_id BLOB NOT NULL, bytes BLOB NOT NULL);
             CREATE INDEX IF NOT EXISTS pieces_expiry ON pieces(expiry_day);",
                )
                .is_err()
        );
        assert!(s.mark_dropped(&[1; 32], 20_000).unwrap());
        s.conn.execute_batch("PRAGMA user_version = 2;").unwrap();
        assert!(SqlitePieces::init(s.conn, 1000).is_err(), "newer formats cannot be downgraded");
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn restart_preserves_markers_and_supported_restore_discards_stale_payloads() {
        let path = std::env::temp_dir().join(format!("hoi-piece-restore-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let mut s = SqlitePieces::open(&path, 1000).unwrap();
            assert!(s.durable_drops());
            s.put(&[1; 32], meta(20_100), &[7; 100], 20_000).unwrap();
            s.put(&[2; 32], meta(20_100), &[8; 100], 20_000).unwrap();
            assert!(s.mark_dropped(&[1; 32], 20_000).unwrap());
        }
        {
            let mut s = SqlitePieces::open(&path, 1000).unwrap();
            assert_eq!(s.stats().count, 1);
            assert_eq!(s.get(&[2; 32], 20_000).unwrap(), Some(vec![8; 100]), "ordinary restart retains live payloads");
            assert!(s.mark_dropped(&[1; 32], 20_000).unwrap());
            assert_eq!(s.put(&[1; 32], meta(20_100), &[7; 100], 20_000).unwrap(), PutOutcome::Dropped);
        }
        {
            let mut s = SqlitePieces::restore(&path, 1000).unwrap();
            assert_eq!(s.stats().count, 0);
            assert_eq!(s.get(&[2; 32], 20_000).unwrap(), None, "restored payloads cannot serve before reconciliation");
            assert_eq!(s.put(&[1; 32], meta(20_100), &[7; 100], 20_000).unwrap(), PutOutcome::Dropped);
            assert_eq!(s.put(&[2; 32], meta(20_100), &[8; 100], 20_000).unwrap(), PutOutcome::Stored, "live content can be repaired");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn a_reopened_piece_file_keeps_only_locators_days_drop_metadata_and_opaque_bytes() {
        // RFC-0004: what a holder retains, column by column. A sender, community, author or key
        // stored here would move the holder's privacy boundary.
        let path = std::env::temp_dir().join(format!("hoi-piece-columns-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let mut s = SqlitePieces::open(&path, 10_000).unwrap();
            s.put(&[1; 32], meta(20_100), &[7; 100], 20_000).unwrap();
            s.put(&[2; 32], meta(20_100), &[8; 100], 20_000).unwrap();
            assert!(s.mark_dropped(&[2; 32], 20_000).unwrap());
        }
        let s = SqlitePieces::open(&path, 10_000).unwrap();
        let names = |sql: &str| -> Vec<String> { s.conn.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect() };
        assert_eq!(names("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name"), ["dropped_pieces", "held_pieces", "pieces"]);
        assert_eq!(names("SELECT name FROM pragma_table_info('held_pieces')"), ["loc", "expiry_day", "touched_day", "drop_hash", "drop_key_id", "bytes"]);
        assert_eq!(names("SELECT name FROM pragma_table_info('dropped_pieces')"), ["loc", "expiry_day", "drop_hash", "drop_key_id"]);
        assert_eq!(names("SELECT name FROM pragma_table_info('pieces')"), ["requires_drop_journal"]);
        let sentinel_rows: i64 = s.conn.query_row("SELECT COUNT(*) FROM pieces", [], |r| r.get(0)).unwrap();
        assert_eq!(sentinel_rows, 0, "the table that stops older releases holds nothing");
        type Held = (Vec<u8>, i64, i64, Vec<u8>, Vec<u8>, Vec<u8>);
        type Marker = (Vec<u8>, i64, Vec<u8>, Vec<u8>);
        let held: Vec<Held> = s
            .conn
            .prepare("SELECT * FROM held_pieces")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(held, [(vec![1; 32], 20_100, 20_000, vec![1; 32], vec![2; 16], vec![7; 100])]);
        let dropped: Vec<Marker> = s
            .conn
            .prepare("SELECT * FROM dropped_pieces")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(dropped, [(vec![2; 32], 20_100, vec![1; 32], vec![2; 16])], "a drop marker keeps no payload");
        drop(s);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_drop_acknowledgements_require_full_durability() {
        let s = SqlitePieces::memory(1000).unwrap();
        let synchronous: i64 = s.conn.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap();
        assert_eq!(synchronous, 2, "a successful drop transaction must survive a power loss");
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
