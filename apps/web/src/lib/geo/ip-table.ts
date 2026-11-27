// Reads the range tables written by scripts/geo-assets.mjs. Layout after inflating:
//   "HOIC" u8 version=1  u8 family(4|6)  u16le C  u32le N
//   C × 2 ASCII country codes (index 1..C; 0 means not listed)
//   N LEB128 start deltas from 0 (IPv4 address, or IPv6 /64 prefix)
//   N u8 code indices
// Segment i covers [start i, start i+1). Starts are kept as two 32-bit halves so phones need no BigInt.
import { publicAddress } from '../core/browser-egress';

export interface CountryTable {
  family: 4 | 6;
  codes: string[];
  hi: Uint32Array;
  lo: Uint32Array;
  index: Uint8Array;
}

export interface AddressKey { family: 4 | 6; hi: number; lo: number }

const TWO32 = 2 ** 32;
const invalid = (why: string) => new Error(`The country table is damaged (${why}).`);

export function decodeTable(raw: Uint8Array): CountryTable {
  if (raw.length < 12) throw invalid('too short');
  const view = new DataView(raw.buffer, raw.byteOffset, raw.byteLength);
  if (String.fromCharCode(raw[0], raw[1], raw[2], raw[3]) !== 'HOIC') throw invalid('unknown format');
  if (raw[4] !== 1) throw invalid('unsupported version');
  const family = raw[5];
  if (family !== 4 && family !== 6) throw invalid('unknown address family');
  const count = view.getUint16(6, true);
  const n = view.getUint32(8, true);
  if (count > 255 || n === 0 || 12 + count * 2 + n * 2 > raw.length) throw invalid('bad counts');
  const codes: string[] = [];
  for (let i = 0; i < count; i++) {
    const code = String.fromCharCode(raw[12 + 2 * i], raw[13 + 2 * i]);
    if (!/^[A-Z]{2}$/.test(code)) throw invalid('bad code');
    codes.push(code);
  }
  let at = 12 + count * 2;
  const hi = new Uint32Array(n), lo = new Uint32Array(n);
  let prevHi = 0, prevLo = 0;
  for (let i = 0; i < n; i++) {
    let dHi = 0, dLo = 0, shift = 0, byte: number;
    do {
      if (at >= raw.length || shift >= 64) throw invalid('bad start');
      byte = raw[at++];
      const bits = byte & 0x7f;
      if (shift + 7 <= 32) dLo += bits * 2 ** shift;
      else if (shift >= 32) dHi += bits * 2 ** (shift - 32);
      else {
        const low = 32 - shift;
        dLo += (bits & ((1 << low) - 1)) * 2 ** shift;
        dHi += bits >>> low;
      }
      shift += 7;
    } while (byte & 0x80);
    if (dHi >= TWO32) throw invalid('bad start');
    if (i === 0 ? dHi || dLo : !dHi && !dLo) throw invalid('starts out of order');
    let nextLo = prevLo + dLo;
    let nextHi = prevHi + dHi;
    if (nextLo >= TWO32) { nextLo -= TWO32; nextHi += 1; }
    if (nextHi >= TWO32 || (family === 4 && nextHi !== 0)) throw invalid('start out of range');
    hi[i] = prevHi = nextHi;
    lo[i] = prevLo = nextLo;
  }
  if (at + n !== raw.length) throw invalid('bad length');
  const index = raw.slice(at, at + n);
  for (const value of index) if (value > count) throw invalid('bad index');
  return { family, codes, hi, lo, index };
}

/** A lookup key for a public address, or null for anything private, special-use or malformed. */
export function addressKey(address: string): AddressKey | null {
  let ip = address;
  if (ip.includes(':')) {
    try { ip = new URL(`http://[${address}]/`).hostname.slice(1, -1); }
    catch { return null; }
    const mapped = /^::ffff:([a-f0-9]{1,4}):([a-f0-9]{1,4})$/.exec(ip);
    if (mapped) {
      const high = Number.parseInt(mapped[1], 16), low = Number.parseInt(mapped[2], 16);
      ip = [high >> 8, high & 255, low >> 8, low & 255].join('.');
    }
  }
  const normal = publicAddress(ip);
  if (!normal) return null;
  if (!normal.includes(':')) {
    return { family: 4, hi: 0, lo: normal.split('.').reduce((n, part) => n * 256 + Number(part), 0) };
  }
  const [head, tail] = normal.includes('::') ? normal.split('::') : [normal, ''];
  const left = head ? head.split(':') : [];
  const right = tail ? tail.split(':') : [];
  const words = [...left, ...Array(8 - left.length - right.length).fill('0'), ...right].map(w => Number.parseInt(w, 16));
  return { family: 6, hi: words[0] * 65536 + words[1], lo: words[2] * 65536 + words[3] };
}

/** The country listed for a key, or null when the table lists none. */
export function lookup(table: CountryTable, key: AddressKey): string | null {
  if (table.family !== key.family) return null;
  let low = 0, high = table.hi.length - 1;
  while (low < high) {
    const mid = (low + high + 1) >>> 1;
    const before = table.hi[mid] < key.hi || (table.hi[mid] === key.hi && table.lo[mid] <= key.lo);
    if (before) low = mid; else high = mid - 1;
  }
  const value = table.index[low];
  return value ? table.codes[value - 1] : null;
}
