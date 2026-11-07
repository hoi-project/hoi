// The registry's public log of names, as a registry keeps it: an append-only list
// of the owner-signed claims it retained, hashed into an RFC 6962 Merkle tree whose head it signs
// with its own Ed25519 log key. Shared by the Node service and the Worker; `hoi-node --registry`
// answers the same (`core/crates/hoi-node/src/registry/log.rs`, `core/vectors/name-log.json`).
import { createHash, createPrivateKey, createPublicKey, sign } from 'node:crypto';
import { logEntryText, logHeadStatement } from './public/log.js';

export { logEntryText, logHeadStatement };

/** Entries an `entries` request returns at most. */
export const MAX_LOG_ENTRIES = 256;

const sha = (...parts) => {
  const hash = createHash('sha256');
  for (const part of parts) hash.update(part);
  return hash.digest();
};
const LEAF = Buffer.from([0]);
const NODE = Buffer.from([1]);

export const leafHash = (entry) => sha(LEAF, Buffer.from(entry, 'utf8'));
const nodeHash = (left, right) => sha(NODE, left, right);
/** The largest power of two below `n` (n ≥ 2). */
const split = (n) => { let k = 1; while (k * 2 < n) k *= 2; return k; };

/** MTH(D[lo:hi]). */
export function treeRoot(leaves, lo = 0, hi = leaves.length) {
  const n = hi - lo;
  if (n === 0) return sha();
  if (n === 1) return leaves[lo];
  const k = split(n);
  return nodeHash(treeRoot(leaves, lo, lo + k), treeRoot(leaves, lo + k, hi));
}

/** PATH(index, D[lo:hi]): the audit path of one entry. */
export function inclusionPath(index, leaves, lo = 0, hi = leaves.length) {
  const n = hi - lo;
  if (n <= 1) return [];
  const k = split(n);
  return index < k
    ? [...inclusionPath(index, leaves, lo, lo + k), treeRoot(leaves, lo + k, hi)]
    : [...inclusionPath(index - k, leaves, lo + k, hi), treeRoot(leaves, lo, lo + k)];
}

/** PROOF(m, D[lo:hi]): that the first `m` entries' tree is a prefix of this one (0 < m ≤ n). */
export function consistencyPath(m, leaves, lo = 0, hi = leaves.length, whole = true) {
  const n = hi - lo;
  if (m === n) return whole ? [] : [treeRoot(leaves, lo, hi)];
  const k = split(n);
  return m <= k
    ? [...consistencyPath(m, leaves, lo, lo + k, whole), treeRoot(leaves, lo + k, hi)]
    : [...consistencyPath(m - k, leaves, lo + k, hi, false), treeRoot(leaves, lo, lo + k)];
}

const PKCS8_ED25519 = Buffer.from('302e020100300506032b657004220420', 'hex');

/** The registry's log key from its 32-byte secret (hex): its public key and a signer. */
export function logSigner(secretHex) {
  const privateKey = createPrivateKey({ key: Buffer.concat([PKCS8_ED25519, Buffer.from(secretHex, 'hex')]), format: 'der', type: 'pkcs8' });
  const logKey = createPublicKey(privateKey).export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
  return { logKey, sign: (statement) => sign(null, Buffer.from(statement, 'utf8'), privateKey).toString('hex') };
}

/** The log as the API serves it, over a store with `logSize()`, `logEntries(start, end)` (rows of
 * `{ entry, withheld }`) and `logSecret()`. Leaf hashes are kept in memory as entries arrive:
 * entries never change, so the cache is never wrong, only behind. */
export function nameLog(store) {
  const signer = logSigner(store.logSecret());
  const leaves = [];
  const index = new Map();
  const sync = () => {
    const size = store.logSize();
    if (leaves.length < size) {
      for (const { entry } of store.logEntries(leaves.length, size)) {
        index.set(leafHash(entry).toString('hex'), leaves.length);
        leaves.push(leafHash(entry));
      }
    }
    return size;
  };
  return {
    logKey: signer.logKey,
    size: sync,
    /** A newly signed head of the whole log. */
    head(timestamp) {
      const size = sync();
      const root = treeRoot(leaves, 0, size).toString('hex');
      return { logKey: signer.logKey, size, root, timestamp, signature: signer.sign(logHeadStatement(signer.logKey, size, root, timestamp)) };
    },
    /** Entries `start` to `end` (exclusive), at most 256: each one's leaf hash, and its text unless
     * the operator withholds that listing (the leaf stays, so the tree still checks). */
    entries(start, end) {
      const size = sync();
      if (!(start >= 0 && start < end && end <= size && end - start <= MAX_LOG_ENTRIES)) return null;
      const rows = store.logEntries(start, end);
      return { entries: rows.map((row) => row.withheld ? null : row.entry), leaves: rows.map((row) => leafHash(row.entry).toString('hex')) };
    },
    /** The audit path of the entry with leaf hash `leaf` in the tree of `size`, or null. */
    proof(leaf, size) {
      const current = sync();
      if (!(size >= 1 && size <= current)) return null;
      const at = index.get(leaf);
      if (at === undefined || at >= size) return null;
      return { index: at, size, path: inclusionPath(at, leaves, 0, size).map((h) => h.toString('hex')) };
    },
    /** That the tree of `second` entries extends the one of `first`, or null. */
    consistency(first, second) {
      const current = sync();
      if (!(first >= 0 && first <= second && second <= current)) return null;
      return { first, second, path: first === 0 ? [] : consistencyPath(first, leaves, 0, second).map((h) => h.toString('hex')) };
    },
  };
}

/** Parses a log query: a whole number in range, or null. */
export const logNumber = (value) => /^(0|[1-9][0-9]{0,9})$/.test(value ?? '') ? Number(value) : null;

/** The log's four read paths, under the API root. */
export const LOG_PATHS = ['/v2/log/head', '/v2/log/entries', '/v2/log/proof', '/v2/log/consistency'];

/** What a registry answers on one of `LOG_PATHS`: `{ status, body }`. Any app may read the log. */
export function answerLog(log, path, query, now) {
  const keys = [...query.keys()];
  const only = (...names) => keys.length === names.length && new Set(keys).size === keys.length && names.every((n) => keys.includes(n));
  const invalid = (message) => ({ status: 400, body: { error: 'invalid', message } });
  if (path === '/v2/log/head') return keys.length ? invalid('A log head takes no query.') : { status: 200, body: log.head(now) };
  if (path === '/v2/log/entries') {
    const start = only('start', 'end') ? logNumber(query.get('start')) : null;
    const end = only('start', 'end') ? logNumber(query.get('end')) : null;
    const page = start === null || end === null ? null : log.entries(start, end);
    return page ? { status: 200, body: page } : invalid(`Ask for at most ${MAX_LOG_ENTRIES} entries the log holds, from start to before end.`);
  }
  if (path === '/v2/log/proof') {
    const leaf = only('leaf', 'size') ? query.get('leaf') : null;
    const size = only('leaf', 'size') ? logNumber(query.get('size')) : null;
    if (!/^[0-9a-f]{64}$/.test(leaf ?? '') || size === null || size < 1 || size > log.size()) return invalid('Ask with one leaf hash and a size the log has reached.');
    const proof = log.proof(leaf, size);
    return proof ? { status: 200, body: proof } : { status: 404, body: { error: 'not-logged', message: 'This entry is not among the first entries of this log.' } };
  }
  const first = only('first', 'second') ? logNumber(query.get('first')) : null;
  const second = only('first', 'second') ? logNumber(query.get('second')) : null;
  const proof = first === null || second === null ? null : log.consistency(first, second);
  return proof ? { status: 200, body: proof } : invalid('Ask with two sizes the log has reached, the smaller first.');
}
