// The registry's public log of names, as a client checks it. Every name a registry
// reserves or publishes is appended to an RFC 6962 Merkle tree; the registry signs the tree's head
// with its own log key. A client keeps the last head it saw for each registry, asks for a proof
// that the new head extends it (consistency), and a proof that the name it was given is in the
// tree (inclusion). A registry that gives two people two different keys for one name must then
// show both entries to anyone who reads its log. WebCrypto only, so browsers, Node and the
// conformance suite share it.

const KEY = /^[0-9a-f]{64}$/;
const SIGNATURE = /^[0-9a-f]{128}$/;
const encoder = new TextEncoder();

/** The text a log entry hashes: one retained, owner-signed claim. */
export function logEntryText({ name, community, proof }) {
  const v2 = proof.version === 2;
  return `hoi name log v1\n${proof.version}\n${name}\n${community}\n${v2 ? proof.protocolVersion : ''}\n${v2 ? proof.indexVersion : ''}\n${proof.signature}`;
}

/** The exact bytes a registry's log key signs for a tree head. */
export function logHeadStatement(logKey, size, root, timestamp) {
  return `hoi name log head v1\n${logKey}\n${size}\n${root}\n${timestamp}`;
}

export const toHex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
export const fromHex = (hex) => Uint8Array.from(hex.match(/../g) ?? [], (pair) => Number.parseInt(pair, 16));

async function sha256(...parts) {
  const total = parts.reduce((n, p) => n + p.length, 0);
  const joined = new Uint8Array(total);
  let at = 0;
  for (const part of parts) { joined.set(part, at); at += part.length; }
  return new Uint8Array(await crypto.subtle.digest('SHA-256', joined));
}

const LEAF = Uint8Array.of(0);
const NODE = Uint8Array.of(1);
const equal = (a, b) => a.length === b.length && a.every((v, i) => v === b[i]);

/** SHA-256(0x00 ‖ entry), the entry's leaf in the tree. */
export const logLeaf = (entryText) => sha256(LEAF, encoder.encode(entryText));

const validPath = (path) => Array.isArray(path) && path.length <= 64 && path.every((p) => typeof p === 'string' && KEY.test(p));

/** RFC 9162 §2.1.3.2: whether `path` proves `leaf` is entry `index` of the tree of `size` with `root`. */
export async function verifyInclusion(leaf, index, size, path, root) {
  if (!Number.isSafeInteger(index) || !Number.isSafeInteger(size) || index < 0 || index >= size || !validPath(path)) return false;
  let fn = index;
  let sn = size - 1;
  let r = leaf;
  for (const hex of path) {
    const p = fromHex(hex);
    if (sn === 0) return false;
    if (fn % 2 === 1 || fn === sn) {
      r = await sha256(NODE, p, r);
      if (fn % 2 === 0) while (fn % 2 === 0 && fn !== 0) { fn = Math.floor(fn / 2); sn = Math.floor(sn / 2); }
    } else {
      r = await sha256(NODE, r, p);
    }
    fn = Math.floor(fn / 2);
    sn = Math.floor(sn / 2);
  }
  return sn === 0 && equal(r, root);
}

/** RFC 9162 §2.1.4.2: whether `path` proves the tree of `second` entries extends the one of `first`. */
export async function verifyConsistency(first, second, firstRoot, secondRoot, path) {
  if (!Number.isSafeInteger(first) || !Number.isSafeInteger(second) || first < 0 || first > second || second > 2 ** 30 || !validPath(path)) return false;
  if (first === 0) return path.length === 0;
  if (first === second) return path.length === 0 && equal(firstRoot, secondRoot);
  const nodes = path.map(fromHex);
  if (first <= 2 ** 30 && (first & (first - 1)) === 0) nodes.unshift(firstRoot);
  if (nodes.length === 0) return false;
  let fn = first - 1;
  let sn = second - 1;
  while (fn % 2 === 1) { fn = Math.floor(fn / 2); sn = Math.floor(sn / 2); }
  let fr = nodes[0];
  let sr = nodes[0];
  for (const c of nodes.slice(1)) {
    if (sn === 0) return false;
    if (fn % 2 === 1 || fn === sn) {
      fr = await sha256(NODE, c, fr);
      sr = await sha256(NODE, c, sr);
      if (fn % 2 === 0) while (fn % 2 === 0 && fn !== 0) { fn = Math.floor(fn / 2); sn = Math.floor(sn / 2); }
    } else {
      sr = await sha256(NODE, sr, c);
    }
    fn = Math.floor(fn / 2);
    sn = Math.floor(sn / 2);
  }
  return sn === 0 && equal(fr, firstRoot) && equal(sr, secondRoot);
}

/** A signed tree head as a registry serves it, checked against its own log key, or null. */
export async function verifiedLogHead(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || Object.keys(value).sort().join(',') !== 'logKey,root,signature,size,timestamp') return null;
  const { logKey, size, root, timestamp, signature } = value;
  if (typeof logKey !== 'string' || !KEY.test(logKey) || typeof root !== 'string' || !KEY.test(root)
    || typeof signature !== 'string' || !SIGNATURE.test(signature)
    || !Number.isSafeInteger(size) || size < 0 || !Number.isSafeInteger(timestamp) || timestamp < 0) return null;
  try {
    const key = await crypto.subtle.importKey('raw', fromHex(logKey), { name: 'Ed25519' }, false, ['verify']);
    if (!await crypto.subtle.verify('Ed25519', key, fromHex(signature), encoder.encode(logHeadStatement(logKey, size, root, timestamp)))) return null;
  } catch { return null; }
  return { logKey, size, root, timestamp, signature };
}
