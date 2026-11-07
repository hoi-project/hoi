//! The registry's public log of names: every owner-signed claim it retained, in the
//! order it did, hashed into an RFC 6962 Merkle tree whose head it signs with its own Ed25519 log
//! key. The same entries, tree and answers as `registry/log.mjs`; `core/vectors/name-log.json`
//! holds both to it.

use std::collections::HashMap;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Entries an `entries` request returns at most.
pub const MAX_LOG_ENTRIES: u64 = 256;
/// The log's four read paths.
pub const LOG_PATHS: [&str; 4] = ["/v2/log/head", "/v2/log/entries", "/v2/log/proof", "/v2/log/consistency"];

pub type Hash = [u8; 32];

/// The text a log entry hashes: one retained claim. `index` is (protocol, index version) for a
/// version 2 claim.
pub fn entry_text(version: u8, name: &str, community: &str, index: Option<(&str, i64)>, signature: &str) -> String {
    let (protocol, index) = match index {
        Some((p, i)) if version == 2 => (p.to_string(), i.to_string()),
        _ => (String::new(), String::new()),
    };
    format!("hoi name log v1\n{version}\n{name}\n{community}\n{protocol}\n{index}\n{signature}")
}

/// The exact text a log key signs for a tree head.
pub fn head_statement(log_key: &str, size: u64, root: &str, timestamp: u64) -> String {
    format!("hoi name log head v1\n{log_key}\n{size}\n{root}\n{timestamp}")
}

pub fn leaf_hash(entry: &str) -> Hash {
    let mut h = Sha256::new();
    h.update([0u8]);
    h.update(entry.as_bytes());
    h.finalize().into()
}

fn node_hash(left: &Hash, right: &Hash) -> Hash {
    let mut h = Sha256::new();
    h.update([1u8]);
    h.update(left);
    h.update(right);
    h.finalize().into()
}

/// The largest power of two below `n` (n ≥ 2).
fn split(n: usize) -> usize {
    let mut k = 1;
    while k * 2 < n {
        k *= 2;
    }
    k
}

/// MTH(D): the root of the tree over `leaves`.
pub fn tree_root(leaves: &[Hash]) -> Hash {
    match leaves.len() {
        0 => Sha256::digest([]).into(),
        1 => leaves[0],
        n => {
            let k = split(n);
            node_hash(&tree_root(&leaves[..k]), &tree_root(&leaves[k..]))
        }
    }
}

/// PATH(index, D): one entry's audit path.
pub fn inclusion_path(index: usize, leaves: &[Hash]) -> Vec<Hash> {
    let n = leaves.len();
    if n <= 1 {
        return Vec::new();
    }
    let k = split(n);
    let (mut path, sibling) = if index < k {
        (inclusion_path(index, &leaves[..k]), tree_root(&leaves[k..]))
    } else {
        (inclusion_path(index - k, &leaves[k..]), tree_root(&leaves[..k]))
    };
    path.push(sibling);
    path
}

/// PROOF(m, D): that the tree of the first `m` entries is a prefix of this one (0 < m ≤ n).
pub fn consistency_path(m: usize, leaves: &[Hash]) -> Vec<Hash> {
    fn sub(m: usize, leaves: &[Hash], whole: bool) -> Vec<Hash> {
        let n = leaves.len();
        if m == n {
            return if whole { Vec::new() } else { vec![tree_root(leaves)] };
        }
        let k = split(n);
        let (mut path, sibling) =
            if m <= k { (sub(m, &leaves[..k], whole), tree_root(&leaves[k..])) } else { (sub(m - k, &leaves[k..], false), tree_root(&leaves[..k])) };
        path.push(sibling);
        path
    }
    if m == 0 { Vec::new() } else { sub(m, leaves, true) }
}

fn hex(h: &Hash) -> String {
    hoi_types::hex_encode(h)
}

/// A whole number as a log query takes it.
fn number(value: Option<&str>) -> Option<u64> {
    let v = value?;
    if v.is_empty() || v.len() > 10 || !v.bytes().all(|b| b.is_ascii_digit()) || (v.len() > 1 && v.starts_with('0')) {
        return None;
    }
    v.parse().ok()
}

/// What the log answers on one of `LOG_PATHS`, given the entries it holds (text, and whether the
/// operator withholds that listing) and its key.
pub struct Log {
    keypair: hoi_crypto::Keypair,
    leaves: Vec<Hash>,
    index: HashMap<Hash, usize>,
}

impl Log {
    pub fn new(secret: [u8; 32]) -> Log {
        Log { keypair: hoi_crypto::Keypair::from_secret(&secret), leaves: Vec::new(), index: HashMap::new() }
    }

    pub fn log_key(&self) -> String {
        self.keypair.public().hex()
    }

    /// Takes entries the store appended since the last call (they never change).
    pub fn extend(&mut self, entries: impl IntoIterator<Item = String>) {
        for entry in entries {
            let leaf = leaf_hash(&entry);
            self.index.insert(leaf, self.leaves.len());
            self.leaves.push(leaf);
        }
    }

    pub fn size(&self) -> u64 {
        self.leaves.len() as u64
    }

    /// A newly signed head of the whole log.
    pub fn head(&self, timestamp: u64) -> Value {
        let root = hex(&tree_root(&self.leaves));
        let key = self.log_key();
        let signature = hoi_types::hex_encode(&self.keypair.sign(head_statement(&key, self.size(), &root, timestamp).as_bytes()).0);
        json!({ "logKey": key, "size": self.size(), "root": root, "timestamp": timestamp, "signature": signature })
    }

    /// The answer on `path`: a status and body. `rows(start, end)` reads entries from the store.
    pub fn answer(&self, path: &str, pairs: &[(String, String)], now: u64, rows: impl FnOnce(u64, u64) -> Vec<(String, bool)>) -> (u16, Value) {
        let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        let only = |names: &[&str]| keys.len() == names.len() && names.iter().all(|n| keys.iter().filter(|k| *k == n).count() == 1);
        let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        let invalid = |message: &str| (400, json!({ "error": "invalid", "message": message }));
        match path {
            "/v2/log/head" => {
                if keys.is_empty() {
                    (200, self.head(now))
                } else {
                    invalid("A log head takes no query.")
                }
            }
            "/v2/log/entries" => {
                let (start, end) = if only(&["start", "end"]) { (number(get("start")), number(get("end"))) } else { (None, None) };
                match (start, end) {
                    (Some(start), Some(end)) if start < end && end <= self.size() && end - start <= MAX_LOG_ENTRIES => {
                        let rows = rows(start, end);
                        let entries: Vec<Value> = rows.iter().map(|(entry, withheld)| if *withheld { Value::Null } else { json!(entry) }).collect();
                        let leaves: Vec<String> = rows.iter().map(|(entry, _)| hex(&leaf_hash(entry))).collect();
                        (200, json!({ "entries": entries, "leaves": leaves }))
                    }
                    _ => invalid(&format!("Ask for at most {MAX_LOG_ENTRIES} entries the log holds, from start to before end.")),
                }
            }
            "/v2/log/proof" => {
                let leaf = if only(&["leaf", "size"]) { get("leaf") } else { None };
                let size = if only(&["leaf", "size"]) { number(get("size")) } else { None };
                let leaf =
                    leaf.filter(|l| l.len() == 64 && l.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).and_then(|l| hoi_types::hex_decode(l).ok());
                let (Some(leaf), Some(size)) = (leaf, size) else { return invalid("Ask with one leaf hash and a size the log has reached.") };
                if size < 1 || size > self.size() {
                    return invalid("Ask with one leaf hash and a size the log has reached.");
                }
                let Ok(leaf) = <Hash>::try_from(leaf.as_slice()) else { return invalid("Ask with one leaf hash and a size the log has reached.") };
                match self.index.get(&leaf) {
                    Some(&at) if (at as u64) < size => {
                        let path: Vec<String> = inclusion_path(at, &self.leaves[..size as usize]).iter().map(hex).collect();
                        (200, json!({ "index": at, "size": size, "path": path }))
                    }
                    _ => (404, json!({ "error": "not-logged", "message": "This entry is not among the first entries of this log." })),
                }
            }
            _ => {
                let (first, second) = if only(&["first", "second"]) { (number(get("first")), number(get("second"))) } else { (None, None) };
                match (first, second) {
                    (Some(first), Some(second)) if first <= second && second <= self.size() => {
                        let path: Vec<String> = consistency_path(first as usize, &self.leaves[..second as usize]).iter().map(hex).collect();
                        (200, json!({ "first": first, "second": second, "path": path }))
                    }
                    _ => invalid("Ask with two sizes the log has reached, the smaller first."),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `core/vectors/name-log.json`: the entries, their leaves, every root, and some proofs, as the
    /// JS registry computes them.
    #[test]
    fn the_tree_matches_the_shared_vectors() {
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../vectors/name-log.json")).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        let entries: Vec<String> = v["entries"].as_array().unwrap().iter().map(|e| e.as_str().unwrap().to_string()).collect();
        let leaves: Vec<Hash> = entries.iter().map(|e| leaf_hash(e)).collect();
        for (leaf, want) in leaves.iter().zip(v["leaves"].as_array().unwrap()) {
            assert_eq!(hex(leaf), want.as_str().unwrap());
        }
        for (n, want) in v["roots"].as_array().unwrap().iter().enumerate() {
            assert_eq!(hex(&tree_root(&leaves[..n])), want.as_str().unwrap(), "root of {n}");
        }
        for case in v["inclusion"].as_array().unwrap() {
            let (index, size) = (case["index"].as_u64().unwrap() as usize, case["size"].as_u64().unwrap() as usize);
            let path: Vec<String> = inclusion_path(index, &leaves[..size]).iter().map(hex).collect();
            assert_eq!(json!(path), case["path"], "inclusion {index} of {size}");
        }
        for case in v["consistency"].as_array().unwrap() {
            let (first, second) = (case["first"].as_u64().unwrap() as usize, case["second"].as_u64().unwrap() as usize);
            let path: Vec<String> = consistency_path(first, &leaves[..second]).iter().map(hex).collect();
            assert_eq!(json!(path), case["path"], "consistency {first} to {second}");
        }
        let head = &v["head"];
        let secret: [u8; 32] = hoi_types::hex_decode(head["secret"].as_str().unwrap()).unwrap().try_into().unwrap();
        let mut log = Log::new(secret);
        log.extend(entries.clone());
        assert_eq!(log.head(head["timestamp"].as_u64().unwrap()), head["signed"]);
        let entry = &v["entry"];
        let index = entry["protocolVersion"].as_str().map(|p| (p, entry["indexVersion"].as_i64().unwrap()));
        let text = entry_text(
            entry["version"].as_u64().unwrap() as u8,
            entry["name"].as_str().unwrap(),
            entry["community"].as_str().unwrap(),
            index,
            entry["signature"].as_str().unwrap(),
        );
        assert_eq!(text, entry["text"].as_str().unwrap());
    }
}
