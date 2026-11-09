//! `hoi-node admin`: the operator's tools (launch review gap 4). Operators can stop
//! serving; nobody can forge. Every command works on the data directory from a shell, never over
//! the network: there is no admin endpoint to find or guess. Suppressing changes the node's own
//! store, so it needs the node stopped (a lock file says whether it runs); the registry's lists
//! are a separate SQLite file the registry reads on every request, so they work while it runs.
//!
//! An operator who must keep evidence passes `--evidence DIR`: before anything is deleted, the
//! signed records exactly as they arrived (they verify on their own, wherever they are kept) and
//! the media go into a new directory under DIR, readable by this user only. Off by default.
//! Nothing here prints a record's contents.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use hoi_core::suppress::{Material, Target};
use serde_json::json;

use crate::registry::store::Store as RegistryStore;

/// Held while a node runs on a data directory; `admin` takes it too, so it never edits a store a
/// running node would overwrite.
const LOCK_FILE: &str = "hoi-node.lock";

fn usage() -> i32 {
    eprintln!(
        "Usage: hoi-node admin [--data DIR] COMMAND

On this node's store (stop the node first):
  suppress community KEY [--note TEXT] [--evidence DIR]
                           delete a community's records and media here and refuse it for good,
                           whoever asks or lists it
  suppress record ID [--note TEXT] [--evidence DIR]
                           delete one post, comment, edit, report or profile (with its edits)
                           and never take it again
  suppress media HASH [--note TEXT] [--evidence DIR]
                           delete media bytes (BLAKE3 hash) and never take them again; posts
                           showing them are served to nobody
  unsuppress ID            lift a suppression (nothing comes back by itself)
  suppressions             list what is suppressed here

  --note TEXT              your own short reference, a case number say (no reporter details)
  --evidence DIR           first copy the signed records and media into a new directory in DIR

On this node's registry (DIR/registry.db; works while it runs):
  registry list            withheld listings, refused nodes, nodes announced now
  registry hide NAME|KEY   withhold a listing: still reserved, no longer served
  registry unhide NAME|KEY
  registry drop-node KEY   forget a node's announcement and refuse its next ones
  registry allow-node KEY
  registry unwitness URL   start the audit of a mirrored registry's log over (after it stopped
                           the mirror: a rewritten log, a moved name, a new log key)

  relay list               endpoints this node's packet relay refuses (DIR/packet-relay-blocked.txt;
                           a running relay reads it again within half a minute)
  relay block KEY          refuse this endpoint key on the packet relay
  relay unblock KEY

  endpoint                 print this node's endpoint key, for a registry's allowlist
  pieces                   how many encrypted pieces this node holds for scattered delivery
                           (DIR/pieces.db; works while it runs): totals only, nothing in them
                           is readable here

On an operator notice, anywhere, the node running or not:
  notice check FILE        read a notice file (JSON, as the installer and the app write it):
                           say what keeps it from being used, or print its hash and the notice
                           as people read it"
    );
    2
}

/// Takes the data directory's lock, or says why not. Keep the file open while it must hold.
pub fn lock(data: &Path) -> Result<File, String> {
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(data.join(LOCK_FILE)).map_err(|e| format!("can't open the lock file: {e}"))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(format!("a hoi-node is running on {}; stop it first", data.display())),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("can't lock {}: {e}", data.display())),
    }
}

/// Writes `bytes` to a new file only this user can read (0600 on Unix), replacing `path`
/// atomically, so a reader never sees half of it.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    let builder = &mut std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Runs `hoi-node admin ARGS`; returns the exit code.
pub fn run(args: Vec<String>) -> i32 {
    let mut data = PathBuf::from("hoi-node-data");
    let mut rest = Vec::new();
    let mut note: Option<String> = None;
    let mut evidence: Option<PathBuf> = None;
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--data" => match it.next() {
                Some(d) => data = PathBuf::from(d),
                None => return usage(),
            },
            "--note" => match it.next() {
                Some(n) => note = Some(n),
                None => return usage(),
            },
            "--evidence" => match it.next() {
                Some(d) => evidence = Some(PathBuf::from(d)),
                None => return usage(),
            },
            "-h" | "--help" => return usage(),
            _ => rest.push(arg),
        }
    }
    let words: Vec<&str> = rest.iter().map(String::as_str).collect();
    let result = match words.as_slice() {
        ["suppress", target, id] => match Target::parse(target) {
            Some(target) => suppress(&data, target, id, note.as_deref(), evidence.as_deref()),
            None => return usage(),
        },
        ["unsuppress", id] if note.is_none() && evidence.is_none() => unsuppress(&data, id),
        ["suppressions"] if note.is_none() && evidence.is_none() => suppressions(&data),
        ["endpoint"] => endpoint(&data),
        ["relay", command @ ..] if note.is_none() && evidence.is_none() => relay(&data, command),
        ["pieces"] if note.is_none() && evidence.is_none() => pieces(&data),
        ["notice", "check", file] if note.is_none() && evidence.is_none() => notice_check(Path::new(file)),
        ["registry", command @ ..] if note.is_none() && evidence.is_none() => registry(&data, command),
        _ => return usage(),
    };
    match result {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            0
        }
        Err(e) => {
            eprintln!("hoi-node admin: {e}");
            1
        }
    }
}

/// The packet relay's block list: a text file the running relay reads again when it
/// changes, so this works while the node runs.
fn relay(data: &Path, command: &[&str]) -> Result<Vec<String>, String> {
    let path = data.join(crate::packet_relay::BLOCKED_FILE);
    let key = |k: &str| hoi_types::Id::from_hex(&k.to_ascii_lowercase()).map_err(|_| format!("{k} isn't an endpoint key (64 hex characters)"));
    let write = |set: &std::collections::BTreeSet<[u8; 32]>| -> Result<(), String> {
        let mut text = String::from("# Endpoints this node's packet relay refuses (hoi-node admin relay block|unblock).\n");
        for k in set {
            text.push_str(&hoi_types::hex_encode(k));
            text.push('\n');
        }
        std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    let mut set = crate::packet_relay::read_blocked(&path);
    match command {
        ["list"] => Ok(if set.is_empty() { vec!["the packet relay refuses nobody".into()] } else { set.iter().map(|k| hoi_types::hex_encode(k)).collect() }),
        ["block", k] => {
            let id = key(k)?;
            let new = set.insert(id.0);
            write(&set)?;
            Ok(vec![if new { format!("the packet relay refuses {} from now on", id.hex()) } else { format!("{} was refused already", id.hex()) }])
        }
        ["unblock", k] => {
            let id = key(k)?;
            let had = set.remove(&id.0);
            write(&set)?;
            Ok(vec![if had { format!("the packet relay takes {} again", id.hex()) } else { format!("{} wasn't refused", id.hex()) }])
        }
        _ => Err("relay list | relay block KEY | relay unblock KEY".into()),
    }
}

/// Reads a notice file the way `hoi-node --notice` does, and shows it the way a client does.
fn notice_check(file: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("can't read {}: {e}", file.display()))?;
    let notice: hoi_types::NodeNotice = serde_json::from_str(&text).map_err(|e| format!("{}: not a notice file (JSON): {e}", file.display()))?;
    let problems = notice.validate();
    if !problems.is_empty() {
        return Err(format!("{}: {}", file.display(), problems.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")));
    }
    let bytes = notice.canonical();
    let mut lines = vec![
        format!("ok: a notice {}", notice.byline()),
        format!("hash: {}", notice.hash().hex()),
        format!("size: {} of {} bytes", bytes.len(), hoi_types::notice::MAX_NOTICE_BYTES),
    ];
    // Who and where are the operator's to leave out; they should know what people read then.
    lines.extend(notice.omissions().into_iter().map(|o| format!("note: {o} (optional, but you should give it)")));
    lines.push(String::new());
    lines.extend(notice.text().lines().map(String::from));
    Ok(lines)
}

/// Opens the node's store with the node stopped.
fn open(data: &Path) -> Result<(File, hoi_core::Node), String> {
    let path = data.join("hoi.db");
    if !path.exists() {
        return Err(format!("no hoi-node store at {}", path.display()));
    }
    let lock = lock(data)?;
    let db = hoi_store::sqlite::Sqlite::open(&path).map_err(|e| format!("can't open storage: {e}"))?;
    let node = hoi_core::Node::open(Box::new(db), "node", "https://hoi.sh/app/", hoi_core::system_clock()).map_err(|e| e.to_string())?;
    Ok((lock, node))
}

/// Closes the node and flushes old pages out of SQLite's write-ahead log, so deleted rows do not
/// linger in it. Backups and filesystem snapshots taken earlier still hold them.
fn close(data: &Path, mut node: hoi_core::Node) -> Result<(), String> {
    node.commit().map_err(|e| e.to_string())?;
    drop(node);
    hoi_store::sqlite::Sqlite::open(&data.join("hoi.db")).and_then(|db| db.checkpoint()).map_err(|e| format!("deleted, but the log could not be flushed: {e}"))
}

fn suppress(data: &Path, target: Target, id: &str, note: Option<&str>, evidence: Option<&Path>) -> Result<Vec<String>, String> {
    let (_lock, mut node) = open(data)?;
    let mut lines = Vec::new();
    if let Some(dir) = evidence {
        let material = node.suppression_material(target, id).map_err(|e| e.to_string())?;
        let kept = export(dir, target, id, note, &material).map_err(|e| format!("evidence could not be written, nothing was deleted: {e}"))?;
        lines.push(format!("evidence: {} records and {} media files in {}", material.records.len(), material.media.len(), kept.display()));
    }
    let purged = match target {
        Target::Community => node.suppress_community(id, note),
        Target::Record => node.suppress_record(id, note),
        Target::Media => node.suppress_media(id, note),
    }
    .map_err(|e| e.to_string())?;
    close(data, node)?;
    lines.insert(
        0,
        format!(
            "suppressed {} {id}: deleted {} record{} and {} media file{} here; never fetched, stored or served again",
            target.as_str(),
            purged.records,
            if purged.records == 1 { "" } else { "s" },
            purged.media,
            if purged.media == 1 { "" } else { "s" }
        ),
    );
    lines.push("backups and snapshots taken before now still hold it: delete or expire them as your retention rules say".into());
    Ok(lines)
}

/// Writes the evidence copy into a new directory under `dir`; returns that directory.
fn export(dir: &Path, target: Target, id: &str, note: Option<&str>, material: &Material) -> std::io::Result<PathBuf> {
    let at = now();
    let short: String = id.chars().filter(|c| c.is_ascii_hexdigit()).take(16).collect();
    let root = dir.join(format!("hoi-evidence-{at}-{}-{short}", target.as_str()));
    private_dir(&root)?;
    private_dir(&root.join("records"))?;
    private_dir(&root.join("media"))?;
    let mut records = Vec::new();
    for r in &material.records {
        let file = format!("records/{}.hoi", r.id.hex());
        write_private(&root.join(&file), &r.wire)?;
        records.push(json!({
            "id": r.id.hex(), "kind": r.kind, "community": r.community.map(|c| c.hex()), "author": r.author.hex(),
            "created_at": r.created_at, "expires_at": r.expires_at, "received_at": r.received_at, "file": file,
        }));
    }
    let mut media = Vec::new();
    for (hash, bytes) in &material.media {
        let file = format!("media/{}", hash.hex());
        write_private(&root.join(&file), bytes)?;
        media.push(json!({ "hash": hash.hex(), "bytes": bytes.len(), "file": file }));
    }
    let manifest = json!({
        "exported_at": at,
        "software": format!("hoi-node {}", hoi_core::VERSION),
        "target": target.as_str(),
        "id": id,
        "note": note,
        "records": records,
        "media": media,
        "verify": "Each records/*.hoi file is one signed hoi record as this node received it (PROTOCOL.md §2): it decodes to the id in its name and its signature verifies against its author's key. Each media file's BLAKE3 hash is its name.",
    });
    write_private(&root.join("manifest.json"), serde_json::to_string_pretty(&manifest).unwrap_or_default().as_bytes())?;
    Ok(root)
}

fn unsuppress(data: &Path, id: &str) -> Result<Vec<String>, String> {
    let (_lock, mut node) = open(data)?;
    let target = node.unsuppress(id).map_err(|e| e.to_string())?;
    close(data, node)?;
    Ok(vec![format!("lifted the suppression of {} {id}; nothing deleted comes back by itself", target.as_str())])
}

fn suppressions(data: &Path) -> Result<Vec<String>, String> {
    let (_lock, node) = open(data)?;
    let list = node.suppressions();
    let mut lines = vec![format!("{} suppression{}", list.len(), if list.len() == 1 { "" } else { "s" })];
    for s in list {
        lines.push(format!("  {} {} since {}{}", s.target.as_str(), s.id, s.at, s.note.map(|n| format!(" · {n}")).unwrap_or_default()));
    }
    Ok(lines)
}

/// This node's endpoint key, read without touching a running node: the transport key's public
/// half, which every announcement carries anyway.
fn endpoint(data: &Path) -> Result<Vec<String>, String> {
    let path = data.join("hoi.db");
    if !path.exists() {
        return Err(format!("no hoi-node store at {}", path.display()));
    }
    let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    let keys: Option<Vec<u8>> = conn.query_row("SELECT value FROM blobs WHERE key = 'keys'", [], |r| r.get(0)).ok();
    let secret = keys
        .and_then(|k| serde_json::from_slice::<serde_json::Value>(&k).ok())
        .and_then(|v| v["transport"].as_str().and_then(|h| hoi_types::hex_decode(h).ok()))
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .ok_or("this node has no endpoint key yet; start it once")?;
    Ok(vec![hoi_crypto::Keypair::from_secret(&secret).public().hex()])
}

/// What `--hold-pieces` keeps, in totals: how many pieces, how many bytes, how many expire
/// within a week. Read only, so it works beside a running node.
fn pieces(data: &Path) -> Result<Vec<String>, String> {
    let path = data.join("pieces.db");
    if !path.exists() {
        return Err(format!("this node holds no pieces ({} does not exist; see --hold-pieces)", path.display()));
    }
    let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    // Read either payload schema without migrating a database a running node may own.
    // Schema 1 keeps only a compatibility sentinel at `pieces`; drop markers are not payloads.
    let table = match version {
        0 => "pieces",
        1 => "held_pieces",
        _ => return Err("This piece database requires a newer storage implementation.".into()),
    };
    let today = (now() / 86_400) as i64;
    let query = format!("SELECT COUNT(*), COALESCE(SUM(LENGTH(bytes)), 0), COALESCE(SUM(expiry_day < ?1 + 7), 0) FROM {table} WHERE expiry_day >= ?1");
    let (count, bytes, soon): (i64, i64, i64) = conn.query_row(&query, [today], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(|e| e.to_string())?;
    Ok(vec![format!("pieces held: {count} · {:.1} MB · none readable here", bytes as f64 / 1e6), format!("expiring within a week: {soon}")])
}

fn registry(data: &Path, command: &[&str]) -> Result<Vec<String>, String> {
    let path = data.join("registry.db");
    if !path.exists() {
        return Err(format!("no registry database at {}", path.display()));
    }
    let mut store = RegistryStore::open(&path)?;
    let db = |e: rusqlite::Error| format!("registry database: {e}");
    match command {
        ["list"] => {
            let view = store.operator_view(now()).map_err(db)?;
            let mut lines = vec![format!("withheld ({}):", view.withheld.len())];
            for w in view.withheld {
                let covers = w.covers.map(|(name, c)| format!("c/{name} {c}")).unwrap_or_else(|| "not reserved".into());
                lines.push(format!("  {} · {covers} · since {}", w.key, w.since));
            }
            lines.push(format!("refused nodes ({}):", view.refused_nodes.len()));
            for (endpoint, at) in view.refused_nodes {
                lines.push(format!("  {endpoint} since {at}"));
            }
            lines.push(format!("announced now ({}):", view.announced.len()));
            for (endpoint, routes, communities, at) in view.announced {
                lines.push(format!("  {endpoint} · {routes} route{} · {communities} communities · issued {at}", if routes == 1 { "" } else { "s" }));
            }
            Ok(lines)
        }
        ["hide", value] => match store.hide(value, now()).map_err(db)? {
            Some(key) => Ok(vec![format!("withheld {key}: still reserved, no longer listed, looked up or announced as carried")]),
            None => Err("give a public name or a 64-character community key".into()),
        },
        ["unhide", value] => {
            if store.unhide(value).map_err(db)? {
                Ok(vec![format!("listed again: {value}")])
            } else {
                Err("that name or key is not withheld".into())
            }
        }
        ["drop-node", endpoint] => {
            if store.drop_node(endpoint, now()).map_err(db)? {
                Ok(vec![format!("dropped node {endpoint}: its announcement is gone and its next ones are refused")])
            } else {
                Err("give a 64-character endpoint key".into())
            }
        }
        ["allow-node", endpoint] => {
            if store.allow_node(endpoint).map_err(db)? {
                Ok(vec![format!("node {endpoint} may announce itself again")])
            } else {
                Err("that endpoint key is not refused".into())
            }
        }
        ["unwitness", url] => {
            if store.unwitness(url).map_err(db)? {
                Ok(vec![format!("forgot what this registry checked of {url}'s log: the next mirror pass starts over from its current head")])
            } else {
                Err("this registry has not audited that URL (give it exactly as --registry-mirror names it)".into())
            }
        }
        _ => {
            usage();
            Err("unknown registry command".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoi-admin-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<8>().unwrap())));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn notice_check_reads_a_notice_file_the_way_the_node_does() {
        let dir = temp_dir();
        let good = dir.join("notice.json");
        std::fs::write(
            &good,
            r#"{"v":1,"node":"sleepy-walrus","operator":"Lena","contact":"lena@example.org","country":"XX","provider":"Example Hosting","processes":"Nothing beyond that.","ipLogging":true,"retentionDays":7,"updated":"2026-11-03"}"#,
        )
        .unwrap();
        let lines = notice_check(&good).unwrap();
        assert_eq!(lines[0], "ok: a notice by Lena · contact lena@example.org · server in XX");
        assert!(!lines.iter().any(|l| l.starts_with("note: ")));
        assert!(lines[1].starts_with("hash: ") && lines[1].len() == 70);
        assert!(lines.iter().any(|l| l == "Join notice – sleepy-walrus"));
        assert!(lines.iter().any(|l| l.contains("logged for 7 days")));
        let bad = dir.join("bad.json");
        let anonymous = dir.join("anonymous.json");
        std::fs::write(&anonymous, r#"{"v":1,"processes":"Nothing beyond that.","updated":"2026-11-03"}"#).unwrap();
        let lines = notice_check(&anonymous).unwrap();
        assert_eq!(lines[0], "ok: a notice by an operator who gave no name · no contact given");
        assert_eq!(lines.iter().filter(|l| l.starts_with("note: ")).count(), 3, "{lines:?}");
        assert!(lines.iter().any(|l| l == "Operator. The operator of this node chose not to say who they are or how to reach them."));
        std::fs::write(&bad, r#"{"v":1,"operator":"","contact":"x","country":"Somewhere","processes":"p","updated":"yesterday"}"#).unwrap();
        let e = notice_check(&bad).unwrap_err();
        assert!(!e.contains("operator:") && e.contains("country:") && e.contains("updated:"), "{e}");
        assert!(notice_check(&dir.join("missing.json")).unwrap_err().contains("can't read"));
        assert_eq!(run(vec!["notice".into(), "check".into(), good.to_string_lossy().into()]), 0);
        assert_eq!(run(vec!["notice".into(), "check".into(), bad.to_string_lossy().into()]), 1);
        assert_eq!(run(vec!["notice".into(), "check".into()]), 2, "usage");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_lock_keeps_admin_off_a_running_node_and_files_are_private() {
        let dir = temp_dir();
        let held = lock(&dir).unwrap();
        assert!(lock(&dir).unwrap_err().contains("running"), "a second holder is refused");
        drop(held);
        let again = lock(&dir).unwrap();
        drop(again);
        let file = dir.join("secret");
        write_private(&file, b"one").unwrap();
        write_private(&file, b"two").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn suppressing_on_a_stopped_node_deletes_exports_evidence_and_survives_a_restart() {
        let dir = temp_dir();
        // A node with a community and a post, as `--join` would leave it.
        let t = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1_790_000_000));
        let clock = |t: &std::sync::Arc<std::sync::atomic::AtomicU64>| -> hoi_core::Clock {
            let t = t.clone();
            Box::new(move || t.load(std::sync::atomic::Ordering::SeqCst))
        };
        let mut founder = hoi_core::Node::open(Box::new(hoi_store::Ephemeral), "desktop", "https://hoi.sh/app/", clock(&t)).unwrap();
        founder.set_identity_bits(4);
        founder.create_identity("Robin").unwrap();
        let draft = hoi_core::dto::CommunityDraft {
            name: "Unwanted".into(),
            description: "A test.".into(),
            color: "mint".into(),
            limits: Some(hoi_core::dto::LimitsPatch { pow_bits: Some(8), ..Default::default() }),
            rules: None,
            co_owners: None,
            threshold: None,
        };
        let community = founder.create_community(draft).unwrap();
        let invite = founder.invite(&community.id).unwrap().uri;
        {
            let db = hoi_store::sqlite::Sqlite::open(&dir.join("hoi.db")).unwrap();
            let mut node = hoi_core::Node::open(Box::new(db), "node", "https://hoi.sh/app/", clock(&t)).unwrap();
            node.join(&invite).unwrap();
            node.commit().unwrap();
        }
        let running = lock(&dir).unwrap();
        let refused = run(vec!["--data".into(), dir.display().to_string(), "suppress".into(), "community".into(), community.id.clone()]);
        assert_eq!(refused, 1, "a running node's store is left alone");
        drop(running);

        let evidence = dir.join("evidence");
        std::fs::create_dir(&evidence).unwrap();
        let args = |more: &[&str]| -> Vec<String> { ["--data", dir.to_str().unwrap()].iter().chain(more).map(|s| s.to_string()).collect() };
        assert_eq!(run(args(&["suppress", "community", &community.id, "--note", "case 14", "--evidence", evidence.to_str().unwrap()])), 0);
        let exported = std::fs::read_dir(&evidence).unwrap().next().unwrap().unwrap().path();
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(exported.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["target"], "community");
        assert_eq!(manifest["note"], "case 14");
        let records = manifest["records"].as_array().unwrap();
        assert!(!records.is_empty());
        for r in records {
            let wire = std::fs::read(exported.join(r["file"].as_str().unwrap())).unwrap();
            let (_, id) = hoi_types::Wire::decode(&wire).unwrap();
            assert_eq!(id.hex(), r["id"].as_str().unwrap(), "the evidence is the signed record itself");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&exported).unwrap().permissions().mode() & 0o777, 0o700);
        }

        // Stopped and started again, the node holds none of it and refuses it.
        let db = hoi_store::sqlite::Sqlite::open(&dir.join("hoi.db")).unwrap();
        let mut node = hoi_core::Node::open(Box::new(db), "node", "https://hoi.sh/app/", clock(&t)).unwrap();
        assert!(node.communities().is_empty());
        assert!(node.join(&invite).is_err());
        assert_eq!(node.suppressions().len(), 1);
        drop(node);
        assert_eq!(run(args(&["suppressions"])), 0);
        assert_eq!(run(args(&["unsuppress", &community.id])), 0);
        assert_eq!(run(args(&["unsuppress", &community.id])), 1, "nothing left to lift");
        assert_eq!(run(args(&["suppress", "nonsense", &community.id])), 2);
        assert_eq!(run(args(&["suppress", "record", "not-an-id"])), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_registry_lists_are_edited_from_a_shell() {
        let dir = temp_dir();
        let args = |more: &[&str]| -> Vec<String> { ["--data", dir.to_str().unwrap()].iter().chain(more).map(|s| s.to_string()).collect() };
        assert_eq!(run(args(&["registry", "list"])), 1, "no registry here");
        drop(RegistryStore::open(&dir.join("registry.db")).unwrap());
        assert_eq!(run(args(&["registry", "hide", "garden"])), 0);
        assert_eq!(run(args(&["registry", "hide", "Not A Name!"])), 1);
        assert_eq!(run(args(&["registry", "unhide", "garden"])), 0);
        assert_eq!(run(args(&["registry", "unhide", "garden"])), 1);
        let key = "ab".repeat(32);
        assert_eq!(run(args(&["registry", "drop-node", &key])), 0);
        assert_eq!(run(args(&["registry", "allow-node", &key])), 0);
        assert_eq!(run(args(&["registry", "list"])), 0);
        assert_eq!(run(args(&["registry", "frobnicate"])), 1);
        assert_eq!(run(args(&["endpoint"])), 1, "no node store yet");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pieces_are_counted_without_opening_the_node() {
        let dir = temp_dir();
        assert!(pieces(&dir).unwrap_err().contains("holds no pieces"));
        {
            use hoi_store::pieces::{PieceMeta, PieceStore, SqlitePieces};
            let mut s = SqlitePieces::open(&dir.join("pieces.db"), 1 << 20).unwrap();
            let today = (now() / 86_400) as u32;
            let meta = |d| PieceMeta { expiry_day: d, drop_hash: [0; 32], drop_key_id: [0; 16] };
            s.put(&[1; 32], meta(today + 3), &[0; 2000], today).unwrap();
            s.put(&[2; 32], meta(today + 90), &[0; 3000], today).unwrap();
            s.put(&[3; 32], meta(today + 3), &[0; 4000], today).unwrap();
            assert!(s.mark_dropped(&[3; 32], today).unwrap());
            assert_eq!(pieces(&dir).unwrap()[0], "pieces held: 2 · 0.0 MB · none readable here", "markers are not held pieces, even while the store is open");
        }
        assert_eq!(pieces(&dir).unwrap(), vec!["pieces held: 2 · 0.0 MB · none readable here".to_string(), "expiring within a week: 1".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_piece_totals_are_read_without_migrating_the_database() {
        let dir = temp_dir();
        let path = dir.join("pieces.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE pieces(loc BLOB PRIMARY KEY, expiry_day INTEGER NOT NULL, touched_day INTEGER NOT NULL,
             drop_hash BLOB NOT NULL, drop_key_id BLOB NOT NULL, bytes BLOB NOT NULL);",
        )
        .unwrap();
        let today = (now() / 86_400) as i64;
        for (id, expiry) in [(1u8, today), (2, today + 7), (3, today - 1)] {
            conn.execute(
                "INSERT INTO pieces VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![&[id; 32][..], expiry, today, &[0u8; 32][..], &[0u8; 16][..], &[0u8; 100_000][..]],
            )
            .unwrap();
        }
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        assert_eq!(pieces(&dir).unwrap(), vec!["pieces held: 2 · 0.2 MB · none readable here", "expiring within a week: 1"]);
        assert_eq!(std::fs::read(&path).unwrap(), before, "inspection must not migrate or modify a legacy store");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_piece_schema_is_refused_without_modifying_the_database() {
        let dir = temp_dir();
        let path = dir.join("pieces.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version = 2;").unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        assert!(pieces(&dir).unwrap_err().contains("newer storage implementation"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
