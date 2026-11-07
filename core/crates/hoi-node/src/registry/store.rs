//! The registry's database: permanent `name → community` reservations with two uniqueness
//! constraints, separate version-bound index metadata, the owner-signed proofs, the one table
//! with a time in it (public node announcements, a node's own routes for half an hour) and the
//! operator's two lists: listings withheld by name or key, nodes refused; and the public
//! log of names with its key. The same eight tables, STRICT and at `user_version` 5, as
//! `registry/store.mjs`, so a namespace moves between `hoi-node --registry` and the Node service
//! by copying the file. A mirror's audit of the registries it mirrors (`witnessed`,
//! `witnessed_leaves`, `witnessed_names`) is hoi-node's own, made only with `--registry-mirror`.
//! No post, profile, owner or client-address column, and no access log.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use super::protocol::{Announcement, Claim, LiveNode, MAX_ANNOUNCED_NODES, NODE_TTL_SECONDS, WITHHELD, normalize_community_key, normalize_name};
use super::release::Release;

type R<T> = Result<T, rusqlite::Error>;

/// Every table the registry keeps.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS communities (name TEXT PRIMARY KEY, community TEXT NOT NULL UNIQUE) STRICT",
    "CREATE TABLE IF NOT EXISTS community_index (community TEXT PRIMARY KEY REFERENCES communities(community), protocol_version TEXT NOT NULL, index_version INTEGER NOT NULL CHECK(index_version > 0)) STRICT",
    "CREATE TABLE IF NOT EXISTS community_proofs (community TEXT NOT NULL REFERENCES communities(community), claim_version INTEGER NOT NULL CHECK(claim_version IN (1, 2)), signature TEXT NOT NULL, PRIMARY KEY(community, claim_version)) STRICT",
    "CREATE TABLE IF NOT EXISTS public_nodes (endpoint TEXT PRIMARY KEY, routes TEXT NOT NULL, communities TEXT NOT NULL, issued_at INTEGER NOT NULL CHECK(issued_at >= 0), first_seen INTEGER NOT NULL DEFAULT 0) STRICT",
    "CREATE TABLE IF NOT EXISTS withheld (key TEXT PRIMARY KEY, withheld_at INTEGER NOT NULL) STRICT",
    "CREATE TABLE IF NOT EXISTS refused_nodes (endpoint TEXT PRIMARY KEY, refused_at INTEGER NOT NULL) STRICT",
    "CREATE TABLE IF NOT EXISTS name_log (idx INTEGER PRIMARY KEY CHECK(idx >= 0), name TEXT NOT NULL, community TEXT NOT NULL, entry TEXT NOT NULL UNIQUE) STRICT",
    "CREATE TABLE IF NOT EXISTS log_secret (id INTEGER PRIMARY KEY CHECK(id = 1), secret TEXT NOT NULL) STRICT",
];
/// A mirror's audit of the registries it mirrors, made only when this registry mirrors
/// one: their log key and last head, every leaf of their log, and the names it held.
const WITNESS_SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS witnessed (source TEXT PRIMARY KEY, log_key TEXT NOT NULL, size INTEGER NOT NULL, root TEXT NOT NULL, fault TEXT) STRICT",
    "CREATE TABLE IF NOT EXISTS witnessed_leaves (source TEXT NOT NULL, idx INTEGER NOT NULL, leaf TEXT NOT NULL, PRIMARY KEY(source, idx)) STRICT",
    "CREATE TABLE IF NOT EXISTS witnessed_names (source TEXT NOT NULL, name TEXT NOT NULL, community TEXT NOT NULL, PRIMARY KEY(source, name)) STRICT",
];
/// The file's version: 4 added the operator's lists, 5 the public log of names; a registry older
/// than that refuses the file. The API's `schemaVersion` stays 3.
const SCHEMA_VERSION: i64 = 5;

/// A name or community key as the operator's withheld list stores it.
pub fn withheld_key(value: &str) -> Option<String> {
    normalize_community_key(value).or_else(|| normalize_name(value))
}

/// One entry of the operator's withheld list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithheldEntry {
    /// The name or community key as stored.
    pub key: String,
    pub since: u64,
    /// The reservation it covers, name and community, if one exists.
    pub covers: Option<(String, String)>,
}

/// What the operator sees with `hoi-node admin registry list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorView {
    pub withheld: Vec<WithheldEntry>,
    /// (endpoint, since).
    pub refused_nodes: Vec<(String, u64)>,
    /// Nodes announced now: (endpoint, routes, communities, issued at), to build an allowlist from.
    pub announced: Vec<(String, usize, usize, u64)>,
}

/// What a mirror last checked of another registry's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Witnessed {
    pub log_key: String,
    pub size: u64,
    pub root: String,
    pub fault: Option<String>,
}

/// What an API call answers: a status code and a JSON body.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub status: u16,
    pub body: Value,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens or creates the database. A file written by a newer registry is refused rather
    /// than downgraded; schema 0–2 files (older Node services) are upgraded in place without
    /// inventing proofs or versions for their rows.
    pub fn open(path: &Path) -> Result<Store, String> {
        Self::init(Connection::open(path).map_err(|e| e.to_string())?)
    }

    #[cfg(test)]
    pub fn memory() -> Result<Store, String> {
        Self::init(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }

    fn init(conn: Connection) -> Result<Store, String> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if version > SCHEMA_VERSION {
            return Err("This registry database requires a newer registry server.".into());
        }
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;").map_err(|e| e.to_string())?;
        let mut sql = String::from("BEGIN IMMEDIATE;\n");
        for statement in SCHEMA {
            sql.push_str(statement);
            sql.push_str(";\n");
        }
        sql.push_str(&format!("PRAGMA user_version={SCHEMA_VERSION};\nCOMMIT;"));
        conn.execute_batch(&sql).map_err(|e| e.to_string())?;
        // A schema-3 database made before `first_seen` existed gains the column. Idempotent.
        let has_first_seen = conn
            .prepare("PRAGMA table_info(public_nodes)")
            .and_then(|mut st| st.query_map([], |r| r.get::<_, String>(1)).map(|rows| rows.flatten().any(|c| c == "first_seen")))
            .map_err(|e| e.to_string())?;
        if !has_first_seen {
            conn.execute("ALTER TABLE public_nodes ADD COLUMN first_seen INTEGER NOT NULL DEFAULT 0", []).map_err(|e| e.to_string())?;
        }
        Ok(Store { conn })
    }

    fn transaction<T>(&mut self, f: impl FnOnce(&Connection) -> R<T>) -> R<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match f(&self.conn) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // ------------------------------------------------------------------ rows and shapes

    fn by_name(conn: &Connection, name: &str) -> R<Option<(String, String)>> {
        conn.query_row("SELECT name, community FROM communities WHERE name = ?1", [name], |r| Ok((r.get(0)?, r.get(1)?))).optional()
    }

    fn by_community(conn: &Connection, community: &str) -> R<Option<(String, String)>> {
        conn.query_row("SELECT name, community FROM communities WHERE community = ?1", [community], |r| Ok((r.get(0)?, r.get(1)?))).optional()
    }

    fn withheld_row(conn: &Connection, row: &(String, String)) -> R<bool> {
        Ok(conn.query_row("SELECT 1 FROM withheld WHERE key = ?1 OR key = ?2", [&row.0, &row.1], |_| Ok(())).optional()?.is_some())
    }

    /// A claim touching a withheld name or key changes nothing and learns nothing about the row.
    fn blocked(conn: &Connection, name: &str, community: &str) -> R<bool> {
        if conn.query_row("SELECT 1 FROM withheld WHERE key = ?1 OR key = ?2", [name, community], |_| Ok(())).optional()?.is_some() {
            return Ok(true);
        }
        for row in [Self::by_name(conn, name)?, Self::by_community(conn, community)?].into_iter().flatten() {
            if Self::withheld_row(conn, &row)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn withheld_conflict() -> Outcome {
        Outcome { status: 409, body: json!({ "error": WITHHELD.0, "message": WITHHELD.1 }) }
    }

    /// An exact lookup: the row in `shape`, `{withheld: true}` for a withheld one, or `None`.
    fn lookup(conn: &Connection, row: Option<(String, String)>, shape: impl FnOnce(&(String, String)) -> R<Value>) -> R<Option<Value>> {
        match row {
            Some(row) if Self::withheld_row(conn, &row)? => Ok(Some(json!({ "withheld": true }))),
            Some(row) => shape(&row).map(Some),
            None => Ok(None),
        }
    }

    fn index_of(conn: &Connection, community: &str) -> R<Option<(String, i64)>> {
        conn.query_row("SELECT protocol_version, index_version FROM community_index WHERE community = ?1", [community], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
    }

    /// The newest retained proof: `{version, signature}` plus the index fields for a v2 proof.
    fn proof_of(conn: &Connection, community: &str) -> R<Value> {
        let saved: Option<(i64, String)> = conn
            .query_row("SELECT claim_version, signature FROM community_proofs WHERE community = ?1 ORDER BY claim_version DESC LIMIT 1", [community], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        Ok(match saved {
            None => Value::Null,
            Some((version, signature)) => {
                let mut proof = Map::new();
                proof.insert("version".into(), json!(version));
                proof.insert("signature".into(), json!(signature));
                if version == 2
                    && let Some((protocol, index)) = Self::index_of(conn, community)?
                {
                    proof.insert("protocolVersion".into(), json!(protocol));
                    proof.insert("indexVersion".into(), json!(index));
                }
                Value::Object(proof)
            }
        })
    }

    /// `{name, community, proof}`: the v1 shape.
    fn with_proof(conn: &Connection, row: &(String, String)) -> R<Value> {
        Ok(json!({ "name": row.0, "community": row.1, "proof": Self::proof_of(conn, &row.1)? }))
    }

    /// The v2 shape: the v1 shape plus versions and compatibility against `release`.
    fn describe(conn: &Connection, row: &(String, String), release: &Release) -> R<Value> {
        let index = Self::index_of(conn, &row.1)?;
        let signed = Self::with_proof(conn, row)?;
        let proof_version = signed["proof"]["version"].as_i64();
        let compatible = proof_version == Some(2)
            && index
                .as_ref()
                .is_some_and(|(protocol, version)| *version == release.index_version as i64 && release.supported_protocols.iter().any(|p| p == protocol));
        let mut out = signed.as_object().cloned().unwrap_or_default();
        out.insert("protocolVersion".into(), index.as_ref().map_or(Value::Null, |(p, _)| json!(p)));
        out.insert("indexVersion".into(), index.as_ref().map_or(Value::Null, |(_, v)| json!(v)));
        out.insert("compatible".into(), json!(compatible));
        out.insert("indexed".into(), json!(compatible));
        Ok(Value::Object(out))
    }

    fn merged(base: &Value, extra: Value) -> Value {
        let mut out = base.as_object().cloned().unwrap_or_default();
        if let Value::Object(extra) = extra {
            out.extend(extra);
        }
        Value::Object(out)
    }

    /// Both uniqueness constraints are arbitrated by SQLite, including across processes.
    fn reserve_row(conn: &Connection, name: &str, community: &str) -> R<(u16, Value, (String, String))> {
        let created: Option<(String, String)> = conn
            .query_row("INSERT INTO communities(name, community) VALUES (?1, ?2) ON CONFLICT DO NOTHING RETURNING name, community", [name, community], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if let Some(row) = created {
            return Ok((200, json!({ "name": row.0, "community": row.1 }), row));
        }
        if let Some(named) = Self::by_name(conn, name)? {
            if named.1 == community {
                return Ok((200, json!({ "name": named.0, "community": named.1 }), named));
            }
            let body = json!({ "error": "name-taken", "message": "This public name belongs to another community.", "name": named.0, "community": named.1 });
            return Ok((409, body, named));
        }
        let registered = Self::by_community(conn, community)?.unwrap_or_else(|| (String::new(), community.to_string()));
        let body =
            json!({ "error": "community-registered", "message": "This community already has a public name.", "name": registered.0, "community": registered.1 });
        Ok((409, body, registered))
    }

    /// The public log takes each claim the moment it is retained, in the same transaction, once.
    fn log_proof(conn: &Connection, name: &str, community: &str, version: u8) -> R<()> {
        let Some(signature): Option<String> = conn
            .query_row("SELECT signature FROM community_proofs WHERE community = ?1 AND claim_version = ?2", params![community, version], |r| r.get(0))
            .optional()?
        else {
            return Ok(());
        };
        let index = if version == 2 { Self::index_of(conn, community)? } else { None };
        let entry = super::log::entry_text(version, name, community, index.as_ref().map(|(p, i)| (p.as_str(), *i)), &signature);
        conn.execute(
            "INSERT INTO name_log(idx, name, community, entry) SELECT (SELECT COUNT(*) FROM name_log), ?1, ?2, ?3 WHERE NOT EXISTS (SELECT 1 FROM name_log WHERE entry = ?3)",
            params![name, community, entry],
        )?;
        Ok(())
    }

    /// How many entries the public log holds.
    pub fn log_size(&self) -> R<u64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM name_log", [], |r| r.get::<_, i64>(0))?.max(0) as u64)
    }

    /// Log entries `start` to `end` (exclusive), each with whether the operator withholds it.
    pub fn log_entries(&self, start: u64, end: u64) -> R<Vec<(String, bool)>> {
        let mut st = self.conn.prepare(
            "SELECT entry, (name IN (SELECT key FROM withheld) OR community IN (SELECT key FROM withheld)) FROM name_log WHERE idx >= ?1 AND idx < ?2 ORDER BY idx",
        )?;
        st.query_map(params![start as i64, end as i64], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)))?.collect()
    }

    /// The log key's secret, made on first use and kept with the names it signs for.
    pub fn log_secret(&mut self) -> Result<[u8; 32], String> {
        let fresh = hoi_types::hex_encode(&hoi_crypto::random_bytes::<32>().map_err(|e| e.to_string())?);
        self.conn.execute("INSERT INTO log_secret(id, secret) VALUES (1, ?1) ON CONFLICT DO NOTHING", [fresh]).map_err(|e| e.to_string())?;
        let secret: String = self.conn.query_row("SELECT secret FROM log_secret WHERE id = 1", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        hoi_types::hex_decode(&secret).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| "The registry's log key is damaged.".to_string())
    }

    // ------------------------------------------------------------------ a mirror's audit

    /// Makes the mirror's audit tables (only a registry that mirrors another has them).
    pub fn witness_tables(&self) -> R<()> {
        for statement in WITNESS_SCHEMA {
            self.conn.execute(statement, [])?;
        }
        Ok(())
    }

    /// What this registry last checked of `source`'s log: its key, size, root, and the fault
    /// that stopped the mirror, if any.
    pub fn witnessed(&self, source: &str) -> R<Option<Witnessed>> {
        self.conn
            .query_row("SELECT log_key, size, root, fault FROM witnessed WHERE source = ?1", [source], |r| {
                Ok(Witnessed { log_key: r.get(0)?, size: r.get::<_, i64>(1)?.max(0) as u64, root: r.get(2)?, fault: r.get(3)? })
            })
            .optional()
    }

    /// Every leaf of `source`'s log this registry has read, in order.
    pub fn witnessed_leaves(&self, source: &str) -> R<Vec<String>> {
        let mut st = self.conn.prepare("SELECT leaf FROM witnessed_leaves WHERE source = ?1 ORDER BY idx")?;
        st.query_map([source], |r| r.get(0))?.collect()
    }

    /// The community `source`'s log first gave `name`, and the name it first gave `community`.
    pub fn witnessed_name(&self, source: &str, name: &str, community: &str) -> R<(Option<String>, Option<String>)> {
        let by_name = self.conn.query_row("SELECT community FROM witnessed_names WHERE source = ?1 AND name = ?2", [source, name], |r| r.get(0)).optional()?;
        let by_key = self
            .conn
            .query_row("SELECT name FROM witnessed_names WHERE source = ?1 AND community = ?2 LIMIT 1", [source, community], |r| r.get(0))
            .optional()?;
        Ok((by_name, by_key))
    }

    /// One more leaf of `source`'s log at `idx`, and the name it holds when its text was shown.
    pub fn witness_leaf(&mut self, source: &str, idx: u64, leaf: &str, named: Option<(&str, &str)>) -> R<()> {
        self.conn.execute("INSERT OR REPLACE INTO witnessed_leaves(source, idx, leaf) VALUES (?1, ?2, ?3)", params![source, idx as i64, leaf])?;
        if let Some((name, community)) = named {
            self.conn.execute("INSERT OR IGNORE INTO witnessed_names(source, name, community) VALUES (?1, ?2, ?3)", params![source, name, community])?;
        }
        Ok(())
    }

    /// Records the head this registry checked `source`'s log against, or the fault it found.
    pub fn witness_head(&mut self, source: &str, log_key: &str, size: u64, root: &str, fault: Option<&str>) -> R<()> {
        self.conn.execute(
            "INSERT INTO witnessed(source, log_key, size, root, fault) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(source) DO UPDATE SET log_key = excluded.log_key, size = excluded.size, root = excluded.root, fault = excluded.fault",
            params![source, log_key, size as i64, root, fault],
        )?;
        Ok(())
    }

    /// Forgets everything this registry checked of `source`'s log (`hoi-node admin registry
    /// unwitness`), so the next pass starts over from its current head.
    pub fn unwitness(&mut self, source: &str) -> R<bool> {
        self.witness_tables()?;
        let n = self.conn.execute("DELETE FROM witnessed WHERE source = ?1", [source])?;
        self.conn.execute("DELETE FROM witnessed_leaves WHERE source = ?1", [source])?;
        self.conn.execute("DELETE FROM witnessed_names WHERE source = ?1", [source])?;
        Ok(n > 0)
    }

    // ------------------------------------------------------------------ the API's questions

    pub fn find(&self, name: &str) -> R<Option<Value>> {
        Self::lookup(&self.conn, Self::by_name(&self.conn, name)?, |row| Self::with_proof(&self.conn, row))
    }

    pub fn list(&self, prefix: &str) -> R<Vec<Value>> {
        let mut st = self.conn.prepare(
            "SELECT name, community FROM communities WHERE name >= ?1 AND name < ?2
             AND name NOT IN (SELECT key FROM withheld) AND community NOT IN (SELECT key FROM withheld) ORDER BY name LIMIT 50",
        )?;
        let rows: Vec<(String, String)> = st.query_map([prefix, &format!("{prefix}\x7f")], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<R<_>>()?;
        rows.iter().map(|row| Self::with_proof(&self.conn, row)).collect()
    }

    pub fn reserve(&mut self, claim: &Claim) -> R<Outcome> {
        let claim = claim.clone();
        self.transaction(move |conn| {
            if Self::blocked(conn, &claim.name, &claim.community)? {
                return Ok(Self::withheld_conflict());
            }
            let (status, body, row) = Self::reserve_row(conn, &claim.name, &claim.community)?;
            if status == 200 {
                conn.execute(
                    "INSERT INTO community_proofs(community, claim_version, signature) VALUES (?1, 1, ?2) ON CONFLICT DO NOTHING",
                    params![claim.community, claim.signature],
                )?;
                Self::log_proof(conn, &row.0, &claim.community, 1)?;
            }
            Ok(Outcome { status, body: Self::merged(&body, Self::with_proof(conn, &row)?) })
        })
    }

    pub fn find_publication(&self, name: &str, release: &Release) -> R<Option<Value>> {
        Self::lookup(&self.conn, Self::by_name(&self.conn, name)?, |row| Self::describe(&self.conn, row, release))
    }

    pub fn find_publication_by_key(&self, community: &str, release: &Release) -> R<Option<Value>> {
        Self::lookup(&self.conn, Self::by_community(&self.conn, community)?, |row| Self::describe(&self.conn, row, release))
    }

    /// Currently compatible entries from `prefix`, after the cursor `after`, at most 50.
    pub fn list_publications(&self, prefix: &str, after: &str, release: &Release) -> R<Vec<Value>> {
        let placeholders: Vec<String> = (0..release.supported_protocols.len()).map(|i| format!("?{}", i + 5)).collect();
        let sql = format!(
            "SELECT c.name, c.community FROM communities c JOIN community_index i USING(community)
             JOIN community_proofs p ON p.community = c.community AND p.claim_version = 2
             WHERE c.name >= ?1 AND c.name < ?2 AND c.name > ?3 AND i.index_version = ?4 AND i.protocol_version IN ({})
             AND c.name NOT IN (SELECT key FROM withheld) AND c.community NOT IN (SELECT key FROM withheld)
             ORDER BY c.name LIMIT 50",
            placeholders.join(",")
        );
        let mut st = self.conn.prepare(&sql)?;
        let mut values: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(prefix.to_string()), Box::new(format!("{prefix}\x7f")), Box::new(after.to_string()), Box::new(release.index_version as i64)];
        for protocol in &release.supported_protocols {
            values.push(Box::new(protocol.clone()));
        }
        let refs: Vec<&dyn rusqlite::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let rows: Vec<(String, String)> = st.query_map(refs.as_slice(), |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<R<_>>()?;
        rows.iter().map(|row| Self::describe(&self.conn, row, release)).collect()
    }

    /// Reservation and index metadata are inserted atomically; a conflicting version for a
    /// reserved key is refused and nothing changes.
    pub fn publish(&mut self, claim: &Claim, release: &Release) -> R<Outcome> {
        let claim = claim.clone();
        let release = release.clone();
        let result = self.transaction(move |conn| {
            if Self::blocked(conn, &claim.name, &claim.community)? {
                return Ok(Err(Self::withheld_conflict()));
            }
            let (status, body, row) = Self::reserve_row(conn, &claim.name, &claim.community)?;
            if status != 200 {
                return Ok(Err(Outcome { status, body: Self::merged(&body, Self::describe(conn, &row, &release)?) }));
            }
            let previous = Self::index_of(conn, &claim.community)?;
            let (protocol, index) = (claim.protocol_version.clone().unwrap_or_default(), claim.index_version.unwrap_or_default() as i64);
            if let Some((p, v)) = &previous
                && (*p != protocol || *v != index)
            {
                let body = json!({ "error": "index-version-conflict", "message": "This community already has immutable index metadata." });
                return Ok(Err(Outcome { status: 409, body: Self::merged(&body, Self::describe(conn, &row, &release)?) }));
            }
            if previous.is_none() {
                conn.execute(
                    "INSERT INTO community_index(community, protocol_version, index_version) VALUES (?1, ?2, ?3)",
                    params![claim.community, protocol, index],
                )?;
            }
            conn.execute(
                "INSERT INTO community_proofs(community, claim_version, signature) VALUES (?1, 2, ?2) ON CONFLICT DO NOTHING",
                params![claim.community, claim.signature],
            )?;
            Self::log_proof(conn, &row.0, &claim.community, 2)?;
            Ok(Ok(Outcome { status: 200, body: Self::describe(conn, &row, &release)? }))
        });
        // A conflict is an answer, not a storage failure: the transaction above committed
        // nothing it should not have (the competing claim inserted nothing), and the caller
        // gets the 409 body.
        match result {
            Ok(Ok(outcome)) | Ok(Err(outcome)) => Ok(outcome),
            Err(e) => Err(e),
        }
    }

    /// Records a verified announcement and returns what discovery now holds for that node: the
    /// node itself, or the announcement with `retained: false` when the list is full. `None`
    /// when the operator refused the node: nothing is stored.
    pub fn announce(&mut self, announcement: &Announcement, now: u64) -> R<Option<Value>> {
        let a = announcement.clone();
        self.transaction(move |conn| {
            if conn.query_row("SELECT 1 FROM refused_nodes WHERE endpoint = ?1", [&a.endpoint], |_| Ok(())).optional()?.is_some() {
                return Ok(None);
            }
            conn.execute("DELETE FROM public_nodes WHERE issued_at < ?1", [now.saturating_sub(NODE_TTL_SECONDS) as i64])?;
            // A newer announcement replaces the node's earlier one; a replayed older one changes
            // nothing. `first_seen` stays with the node's first live announcement: when the list
            // is full, the nodes that were there first stay and newcomers wait.
            conn.execute(
                "INSERT INTO public_nodes(endpoint, routes, communities, issued_at, first_seen) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(endpoint) DO UPDATE SET routes = excluded.routes, communities = excluded.communities, issued_at = excluded.issued_at
                 WHERE excluded.issued_at >= public_nodes.issued_at",
                params![a.endpoint, a.routes.join(","), a.communities.join(","), a.issued_at as i64, now as i64],
            )?;
            conn.execute(
                &format!(
                    "DELETE FROM public_nodes WHERE endpoint IN (SELECT endpoint FROM public_nodes ORDER BY first_seen ASC, issued_at ASC, endpoint LIMIT -1 OFFSET {MAX_ANNOUNCED_NODES})"
                ),
                [],
            )?;
            let kept: Option<LiveNode> = conn
                .query_row("SELECT endpoint, routes, communities, issued_at FROM public_nodes WHERE endpoint = ?1", [&a.endpoint], |r| {
                    Ok(Self::live_node((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                })
                .optional()?;
            Ok(Some(match kept {
                Some(node) => json!({ "endpoint": node.endpoint, "routes": node.routes, "communities": node.communities, "issuedAt": node.issued_at }),
                None => Self::merged(&a.to_json(), json!({ "retained": false })),
            }))
        })
    }

    fn live_node((endpoint, routes, communities, issued_at): (String, String, String, i64)) -> LiveNode {
        LiveNode {
            endpoint,
            routes: routes.split(',').map(String::from).collect(),
            communities: if communities.is_empty() { Vec::new() } else { communities.split(',').map(String::from).collect() },
            issued_at: issued_at.max(0) as u64,
        }
    }

    /// Nodes announced within the last half hour, newest first, refused ones left out.
    pub fn live_nodes(&self, now: u64) -> R<Vec<LiveNode>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT endpoint, routes, communities, issued_at FROM public_nodes WHERE issued_at >= ?1
             AND endpoint NOT IN (SELECT endpoint FROM refused_nodes) ORDER BY issued_at DESC, endpoint LIMIT {MAX_ANNOUNCED_NODES}"
        ))?;
        let rows = st.query_map([now.saturating_sub(NODE_TTL_SECONDS) as i64], |r| Ok(Self::live_node((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))))?;
        rows.collect()
    }

    /// Whether discovery should keep quiet about who carries `community`.
    pub fn is_withheld(&self, community: &str) -> R<bool> {
        let hit = self
            .conn
            .query_row("SELECT 1 FROM withheld WHERE key = ?1 OR key IN (SELECT name FROM communities WHERE community = ?1)", [community], |_| Ok(()))
            .optional()?;
        Ok(hit.is_some())
    }

    // ------------------------------------------------------------------ the operator's tools

    /// Withholds a listing by canonical name or key; returns the stored key, `None` if invalid.
    /// Nothing here is reachable over HTTP (`hoi-node admin registry`).
    pub fn hide(&mut self, value: &str, now: u64) -> R<Option<String>> {
        let Some(key) = withheld_key(value) else { return Ok(None) };
        self.conn.execute("INSERT INTO withheld(key, withheld_at) VALUES (?1, ?2) ON CONFLICT DO NOTHING", params![key, now as i64])?;
        Ok(Some(key))
    }

    pub fn unhide(&mut self, value: &str) -> R<bool> {
        let Some(key) = withheld_key(value) else { return Ok(false) };
        Ok(self.conn.execute("DELETE FROM withheld WHERE key = ?1", [key])? > 0)
    }

    /// Drops a node's announcement and refuses its next ones.
    pub fn drop_node(&mut self, endpoint: &str, now: u64) -> R<bool> {
        let Some(key) = normalize_community_key(endpoint) else { return Ok(false) };
        self.transaction(move |conn| {
            conn.execute("DELETE FROM public_nodes WHERE endpoint = ?1", [&key])?;
            conn.execute("INSERT INTO refused_nodes(endpoint, refused_at) VALUES (?1, ?2) ON CONFLICT DO NOTHING", params![key, now as i64])?;
            Ok(true)
        })
    }

    pub fn allow_node(&mut self, endpoint: &str) -> R<bool> {
        let Some(key) = normalize_community_key(endpoint) else { return Ok(false) };
        Ok(self.conn.execute("DELETE FROM refused_nodes WHERE endpoint = ?1", [key])? > 0)
    }

    pub fn operator_view(&self, now: u64) -> R<OperatorView> {
        let mut st = self.conn.prepare(
            "SELECT w.key, w.withheld_at, c.name, c.community FROM withheld w LEFT JOIN communities c ON c.name = w.key OR c.community = w.key ORDER BY w.key",
        )?;
        let withheld = st
            .query_map([], |r| {
                let covers = match (r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?) {
                    (Some(name), Some(community)) => Some((name, community)),
                    _ => None,
                };
                Ok(WithheldEntry { key: r.get(0)?, since: r.get::<_, i64>(1)?.max(0) as u64, covers })
            })?
            .collect::<R<_>>()?;
        let mut st = self.conn.prepare("SELECT endpoint, refused_at FROM refused_nodes ORDER BY endpoint")?;
        let refused_nodes = st.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)?.max(0) as u64)))?.collect::<R<_>>()?;
        let announced = self.live_nodes(now)?.into_iter().map(|n| (n.endpoint, n.routes.len(), n.communities.len(), n.issued_at)).collect();
        Ok(OperatorView { withheld, refused_nodes, announced })
    }

    /// How many names and live announced nodes this registry holds, for its status line.
    pub fn counts(&self, now: u64) -> R<(u64, u64)> {
        let names: i64 = self.conn.query_row("SELECT COUNT(*) FROM communities", [], |r| r.get(0))?;
        let nodes: i64 =
            self.conn.query_row("SELECT COUNT(*) FROM public_nodes WHERE issued_at >= ?1", [now.saturating_sub(NODE_TTL_SECONDS) as i64], |r| r.get(0))?;
        Ok((names.max(0) as u64, nodes.max(0) as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::protocol::{verified_announcement, verified_claim};
    use hoi_crypto::Keypair;

    fn release() -> Release {
        Release::compiled().unwrap()
    }

    fn pair(seed: u8) -> (Keypair, String) {
        let kp = Keypair::from_secret(&[seed; 32]);
        let hex = kp.public().hex();
        (kp, hex)
    }

    fn claim(name: &str, seed: u8) -> Claim {
        let (kp, community) = pair(seed);
        let statement = hoi_core::community_registration_statement(name, &kp.public());
        let signature = hoi_types::hex_encode(&kp.sign(statement.as_bytes()).0);
        verified_claim(&json!({ "name": name, "community": community, "signature": signature }), 1).unwrap()
    }

    fn publication(name: &str, seed: u8, protocol: &str, index: u32) -> Claim {
        let (kp, community) = pair(seed);
        let statement = hoi_core::community_publication_statement(name, &kp.public(), protocol, index);
        let signature = hoi_types::hex_encode(&kp.sign(statement.as_bytes()).0);
        verified_claim(&json!({ "name": name, "community": community, "protocolVersion": protocol, "indexVersion": index, "signature": signature }), 2).unwrap()
    }

    fn announcement(seed: u8, communities: &[&str], issued_at: u64) -> Announcement {
        let (kp, endpoint) = pair(seed);
        let routes = vec![format!("{endpoint}@203.0.113.{seed}:7843")];
        let ids: Vec<hoi_types::Id> = communities.iter().map(|c| hoi_types::Id::from_hex(c).unwrap()).collect();
        let registry = "https://registry.example";
        let statement = hoi_core::node_announcement_statement(registry, &kp.public(), issued_at, &routes, &ids);
        let signature = hoi_types::hex_encode(&kp.sign(statement.as_bytes()).0);
        verified_announcement(
            &json!({ "registry": registry, "endpoint": endpoint, "routes": routes, "communities": communities, "issuedAt": issued_at, "signature": signature }),
            issued_at,
            &[registry.to_string()],
        )
        .unwrap()
    }

    #[test]
    fn first_claim_wins_retries_are_idempotent_and_proofs_are_kept() {
        let mut store = Store::memory().unwrap();
        let hiking = claim("hiking", 1);
        let first = store.reserve(&hiking).unwrap();
        assert_eq!(first.status, 200);
        assert_eq!(first.body, json!({ "name": "hiking", "community": hiking.community, "proof": { "version": 1, "signature": hiking.signature } }));
        assert_eq!(store.reserve(&hiking).unwrap(), first, "retry after an uncertain response is idempotent");
        let loser = store.reserve(&claim("hiking", 2)).unwrap();
        assert_eq!(loser.status, 409);
        assert_eq!(loser.body["error"], "name-taken");
        assert_eq!(loser.body["community"], json!(hiking.community));
        assert_eq!(loser.body["proof"]["version"], 1);
        let second_name = store.reserve(&claim("another-name", 1)).unwrap();
        assert_eq!(second_name.status, 409);
        assert_eq!(second_name.body["error"], "community-registered");
        assert_eq!(second_name.body["name"], "hiking");
        assert!(store.find("another-name").unwrap().is_none());
        assert_eq!(store.find("hiking").unwrap().unwrap(), first.body);
        assert_eq!(store.list("hi").unwrap(), vec![first.body.clone()]);
        assert!(store.list("x").unwrap().is_empty());
        assert_eq!(store.counts(0).unwrap(), (1, 0));
    }

    #[test]
    fn underscore_is_literal_in_prefixes_and_pages_are_bounded() {
        let mut store = Store::memory().unwrap();
        store.reserve(&claim("art_x", 1)).unwrap();
        store.reserve(&claim("artax", 2)).unwrap();
        for i in 0..53u8 {
            assert_eq!(store.reserve(&claim(&format!("community-{i:02}"), 10 + i)).unwrap().status, 200);
        }
        let page = store.list("community-").unwrap();
        assert_eq!(page.len(), 50);
        assert_eq!(page[0]["name"], "community-00");
        assert_eq!(page[49]["name"], "community-49");
        assert_eq!(store.list("art_").unwrap().iter().map(|v| v["name"].as_str().unwrap().to_string()).collect::<Vec<_>>(), vec!["art_x"]);
    }

    #[test]
    fn publications_bind_versions_keep_names_across_index_changes_and_never_mutate_metadata() {
        let mut store = Store::memory().unwrap();
        let release = release();
        let garden = publication("garden", 3, "0.3", 1);
        let reserved = json!({
            "name": "garden", "community": garden.community, "proof": { "version": 2, "signature": garden.signature, "protocolVersion": "0.3", "indexVersion": 1 },
            "protocolVersion": "0.3", "indexVersion": 1, "compatible": true, "indexed": true,
        });
        assert_eq!(store.publish(&garden, &release).unwrap(), Outcome { status: 200, body: reserved.clone() });
        assert_eq!(store.publish(&garden, &release).unwrap().body, reserved);
        assert_eq!(store.list_publications("", "", &release).unwrap(), vec![reserved.clone()]);
        let conflict = store.publish(&publication("garden", 4, "0.3", 1), &release).unwrap();
        assert_eq!(conflict.status, 409);
        assert_eq!(
            conflict.body,
            Store::merged(&json!({ "error": "name-taken", "message": "This public name belongs to another community." }), reserved.clone())
        );
        assert_eq!(store.publish(&publication("other-name", 3, "0.3", 1), &release).unwrap().status, 409);
        let mutated = store.publish(&publication("garden", 3, "0.4", 2), &release).unwrap();
        assert_eq!(mutated.status, 409, "a proof cannot mutate a previous index binding");
        assert_eq!(mutated.body["error"], "index-version-conflict");
        let next = Release { index_version: 2, protocol_version: "0.4".into(), supported_protocols: vec!["0.4".into()], ..release.clone() };
        assert!(store.list_publications("", "", &next).unwrap().is_empty());
        let hidden = store.find_publication("garden", &next).unwrap().unwrap();
        assert_eq!((hidden["compatible"].as_bool(), hidden["indexed"].as_bool()), (Some(false), Some(false)));
        assert_eq!(store.publish(&publication("garden", 5, "0.4", 2), &next).unwrap().status, 409, "hiding old discovery never frees its name");
        assert_eq!(store.find("garden").unwrap().unwrap()["proof"]["version"], 2, "v1 includes the retained proof");
        assert_eq!(store.find_publication_by_key(&garden.community, &release).unwrap().unwrap(), reserved);
        assert!(store.find_publication_by_key(&"0".repeat(64), &release).unwrap().is_none());
    }

    #[test]
    fn a_legacy_reservation_gains_metadata_only_from_its_own_signed_proof() {
        let mut store = Store::memory().unwrap();
        let release = release();
        let legacy = claim("legacy", 6);
        store.conn.execute("INSERT INTO communities(name, community) VALUES (?1, ?2)", params![legacy.name, legacy.community]).unwrap();
        let described = store.find_publication("legacy", &release).unwrap().unwrap();
        assert_eq!(
            described,
            json!({ "name": "legacy", "community": legacy.community, "protocolVersion": null, "indexVersion": null, "compatible": false, "indexed": false, "proof": null })
        );
        assert!(store.list_publications("legacy", "", &release).unwrap().is_empty());
        assert_eq!(store.publish(&publication("legacy", 7, "0.3", 1), &release).unwrap().status, 409);
        assert_eq!(store.publish(&publication("legacy", 6, "0.3", 1), &release).unwrap().status, 200);
        assert_eq!(store.list_publications("legacy", "", &release).unwrap().len(), 1);
    }

    #[test]
    fn directory_filtering_happens_before_the_page_limit_with_a_cursor() {
        let mut store = Store::memory().unwrap();
        let release = release();
        for i in 0..52u8 {
            assert_eq!(store.publish(&publication(&format!("a-old-{i}"), 100 + i, "0.2", 1), &release).unwrap().status, 200);
            assert_eq!(store.publish(&publication(&format!("garden-{i:02}"), 20 + i, "0.3", 1), &release).unwrap().status, 200);
        }
        let page = store.list_publications("", "", &release).unwrap();
        assert_eq!(page.len(), 50);
        assert_eq!(page[0]["name"], "garden-00");
        let last = store.list_publications("", page[49]["name"].as_str().unwrap(), &release).unwrap();
        assert_eq!(last.iter().map(|v| v["name"].as_str().unwrap().to_string()).collect::<Vec<_>>(), vec!["garden-50", "garden-51"]);
    }

    #[test]
    fn announcements_replace_expire_and_keep_the_first_arrivals_when_full() {
        let mut store = Store::memory().unwrap();
        let now = 1_000_000u64;
        let garden = "a".repeat(64);
        let first = announcement(1, &[&garden], now);
        let kept = store.announce(&first, now).unwrap().unwrap();
        assert_eq!(kept, json!({ "endpoint": first.endpoint, "routes": first.routes, "communities": [garden], "issuedAt": now }));
        let replayed = announcement(1, &[], now - 1);
        store.announce(&replayed, now).unwrap();
        assert_eq!(store.live_nodes(now).unwrap()[0].communities, vec![garden.clone()], "a replayed older announcement changes nothing");
        let moved = announcement(1, &[], now + 30);
        store.announce(&moved, now + 30).unwrap();
        assert!(store.live_nodes(now + 30).unwrap()[0].communities.is_empty(), "a newer one replaces the routes and keys");
        assert_eq!(store.counts(now + 30).unwrap(), (0, 1));
        assert!(store.live_nodes(now + 31 * 60).unwrap().is_empty(), "half an hour of silence drops a node from discovery");
        // Sixty-four nodes fill the list; the sixty-fifth is answered with retained: false and
        // takes nobody's place, however fresh its announcement.
        let mut store = Store::memory().unwrap();
        for i in 0..64u8 {
            let a = announcement(10 + i, &[], now + i as u64);
            assert!(store.announce(&a, now + i as u64).unwrap().unwrap().get("retained").is_none());
        }
        let late = announcement(99, &[], now + 100);
        let answer = store.announce(&late, now + 100).unwrap().unwrap();
        assert_eq!(answer["retained"], json!(false));
        assert_eq!(store.live_nodes(now + 100).unwrap().len(), 64);
        assert!(!store.live_nodes(now + 100).unwrap().iter().any(|n| n.endpoint == late.endpoint));
        // Once the earliest node falls silent, the newcomer is kept on its next announcement.
        let later = now + 31 * 60;
        let again = announcement(99, &[], later);
        assert!(store.announce(&again, later).unwrap().unwrap().get("retained").is_none());
    }

    #[test]
    fn the_operator_withholds_listings_and_drops_nodes_and_reservations_stay() {
        let mut store = Store::memory().unwrap();
        let release = release();
        let garden = publication("garden", 3, "0.3", 1);
        let kept = publication("meadow", 4, "0.3", 1);
        store.publish(&garden, &release).unwrap();
        store.publish(&kept, &release).unwrap();
        let carrier = announcement(5, &[&garden.community], 1_000_000);
        store.announce(&carrier, 1_000_000).unwrap();
        for by in ["Garden", garden.community.as_str()] {
            let key = store.hide(by, 7).unwrap().unwrap();
            assert_eq!(store.find("garden").unwrap().unwrap(), json!({ "withheld": true }));
            assert_eq!(store.find_publication("garden", &release).unwrap().unwrap(), json!({ "withheld": true }));
            assert_eq!(store.find_publication_by_key(&garden.community, &release).unwrap().unwrap(), json!({ "withheld": true }));
            assert!(store.is_withheld(&garden.community).unwrap());
            assert_eq!(store.list("").unwrap().len(), 1);
            assert_eq!(store.list_publications("", "", &release).unwrap().len(), 1);
            // Still reserved: nobody else gets the name, the owner cannot list it again, and the
            // answer names no key.
            for claim in [garden.clone(), publication("garden", 6, "0.3", 1), publication("renamed", 3, "0.3", 1)] {
                let refused = store.publish(&claim, &release).unwrap();
                assert_eq!((refused.status, refused.body["error"].as_str()), (409, Some("withheld")));
                assert!(!refused.body.to_string().contains(&garden.community));
            }
            assert_eq!(store.reserve(&claim("garden", 6)).unwrap().status, 409);
            let view = store.operator_view(1_000_000).unwrap();
            assert_eq!(view.withheld, vec![WithheldEntry { key, since: 7, covers: Some(("garden".to_string(), garden.community.clone())) }]);
            assert!(store.unhide(by).unwrap());
            assert!(!store.unhide(by).unwrap(), "nothing left to lift");
            assert_eq!(store.find_publication("garden", &release).unwrap().unwrap()["community"], json!(garden.community));
        }
        assert_eq!(store.hide("Not A Name!", 1).unwrap(), None);
        // A name withheld before anyone claimed it cannot be claimed.
        store.hide("later", 1).unwrap();
        assert_eq!(store.publish(&publication("later", 8, "0.3", 1), &release).unwrap().status, 409);
        assert!(store.find("later").unwrap().is_none());

        assert_eq!(store.operator_view(1_000_000).unwrap().announced.len(), 1);
        assert!(store.drop_node(&carrier.endpoint, 9).unwrap());
        assert!(store.live_nodes(1_000_000).unwrap().is_empty());
        assert_eq!(store.announce(&announcement(5, &[], 1_000_010), 1_000_010).unwrap(), None, "a dropped node's next announcement is refused");
        assert_eq!(store.operator_view(1_000_010).unwrap().refused_nodes, vec![(carrier.endpoint.clone(), 9)]);
        assert!(store.allow_node(&carrier.endpoint).unwrap());
        assert!(store.announce(&announcement(5, &[], 1_000_020), 1_000_020).unwrap().is_some());
        assert!(!store.drop_node("abc", 1).unwrap());
    }

    #[test]
    fn a_newer_schema_is_refused_and_an_older_one_is_upgraded() {
        let dir = std::env::temp_dir().join(format!("hoi-registry-store-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<8>().unwrap())));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("registry.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE communities (name TEXT PRIMARY KEY, community TEXT NOT NULL UNIQUE) STRICT;").unwrap();
            conn.execute("INSERT INTO communities VALUES (?1, ?2)", params!["legacy", "a".repeat(64)]).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.find("legacy").unwrap().unwrap()["proof"], Value::Null);
        let version: i64 = store.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 5);
        drop(store);
        Connection::open(&path).unwrap().execute_batch("PRAGMA user_version=6").unwrap();
        assert!(Store::open(&path).err().is_some_and(|e| e.contains("newer registry server")));
        for name in ["registry.db", "registry.db-wal", "registry.db-shm"] {
            let _ = std::fs::remove_file(dir.join(name));
        }
        std::fs::remove_dir(dir).unwrap();
    }
}
