// BLAKE3 for inputs up to one chunk (1024 bytes), which is all the wiki's demos need: a record
// of a few hundred bytes, or a stamp's id ‖ nonce. Checked against the reference vectors in
// the tests below the function (see blake3.test in scripts/wiki-demos.test.mjs).
const IV = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);
const PERM = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];
const CHUNK_START = 1, CHUNK_END = 2, ROOT = 8;

const s = new Uint32Array(16);
const m = new Uint32Array(16);
const t = new Uint32Array(16);

function g(a: number, b: number, c: number, d: number, x: number, y: number): void {
  s[a] = (s[a] + s[b] + x) | 0; s[d] ^= s[a]; s[d] = (s[d] >>> 16) | (s[d] << 16);
  s[c] = (s[c] + s[d]) | 0; s[b] ^= s[c]; s[b] = (s[b] >>> 12) | (s[b] << 20);
  s[a] = (s[a] + s[b] + y) | 0; s[d] ^= s[a]; s[d] = (s[d] >>> 8) | (s[d] << 24);
  s[c] = (s[c] + s[d]) | 0; s[b] ^= s[c]; s[b] = (s[b] >>> 7) | (s[b] << 25);
}

function compress(cv: Uint32Array, len: number, flags: number): void {
  s.set(cv, 0);
  s[8] = IV[0]; s[9] = IV[1]; s[10] = IV[2]; s[11] = IV[3];
  s[12] = 0; s[13] = 0; s[14] = len; s[15] = flags;
  for (let r = 0; r < 7; r++) {
    g(0, 4, 8, 12, m[0], m[1]); g(1, 5, 9, 13, m[2], m[3]); g(2, 6, 10, 14, m[4], m[5]); g(3, 7, 11, 15, m[6], m[7]);
    g(0, 5, 10, 15, m[8], m[9]); g(1, 6, 11, 12, m[10], m[11]); g(2, 7, 8, 13, m[12], m[13]); g(3, 4, 9, 14, m[14], m[15]);
    if (r < 6) { for (let i = 0; i < 16; i++) t[i] = m[PERM[i]]; m.set(t); }
  }
  for (let i = 0; i < 8; i++) cv[i] = s[i] ^ s[i + 8];
}

/** BLAKE3-256 of `data` (at most 1024 bytes). */
export function blake3(data: Uint8Array): Uint8Array {
  if (data.length > 1024) throw new Error('blake3: one chunk only');
  const cv = new Uint32Array(IV);
  const blocks = Math.max(1, Math.ceil(data.length / 64));
  for (let b = 0; b < blocks; b++) {
    const off = b * 64;
    const len = Math.min(64, data.length - off);
    m.fill(0);
    for (let i = 0; i < len; i++) m[i >> 2] |= data[off + i] << ((i & 3) * 8);
    let flags = 0;
    if (b === 0) flags |= CHUNK_START;
    if (b === blocks - 1) flags |= CHUNK_END | ROOT;
    compress(cv, len, flags);
  }
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) out[i] = (cv[i >> 2] >>> ((i & 3) * 8)) & 0xff;
  return out;
}

export const hex = (b: Uint8Array): string => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');

/** Leading zero bits of a hash, as hoi-crypto's pow.rs counts them. */
export function leadingZeroBits(h: Uint8Array): number {
  let n = 0;
  for (const byte of h) {
    if (byte === 0) { n += 8; continue; }
    return n + Math.clz32(byte) - 24;
  }
  return n;
}
