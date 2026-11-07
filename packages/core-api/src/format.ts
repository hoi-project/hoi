// Small, shared display helpers.
import type { Unix } from './types';

export const now = (): Unix => Math.floor(Date.now() / 1000);

/** 'a7f2·91c0' for keys */
export const shortKey = (hex: string): string => `${hex.slice(0, 4)}·${hex.slice(4, 8)}`;

/** 'b3:9e1c…4f0a' for record ids */
export const shortId = (hex: string): string => `b3:${hex.slice(0, 4)}…${hex.slice(-4)}`;

/** Remaining life of a record as 0..1, for life bars. */
export function lifeLeft(createdAt: Unix, expiresAt: Unix, at: Unix = now()): number {
  const total = expiresAt - createdAt;
  if (total <= 0) return 0;
  return Math.max(0, Math.min(1, (expiresAt - at) / total));
}

/** 'fades in 88 d', 'fades in 5 h', 'fades in a moment' */
export function fadesIn(expiresAt: Unix, at: Unix = now()): string {
  const s = expiresAt - at;
  if (s <= 60) return 'fades in a moment';
  if (s < 3600) return `fades in ${Math.round(s / 60)} min`;
  if (s < 86400 * 2) return `fades in ${Math.round(s / 3600)} h`;
  return `fades in ${Math.round(s / 86400)} d`;
}

/** '2h', '5 d', 'just now' */
export function ago(t: Unix, at: Unix = now()): string {
  const s = Math.max(0, at - t);
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 86400 * 60) return `${Math.floor(s / 86400)} d`;
  return `${Math.floor(s / (86400 * 30))} mo`;
}

/** '180 days', '2 weeks', '36 hours' for retention windows */
export function duration(seconds: number): string {
  const d = seconds / 86400;
  if (d >= 1 && Number.isInteger(d)) return `${d} ${d === 1 ? 'day' : 'days'}`;
  const h = Math.round(seconds / 3600);
  return `${h} ${h === 1 ? 'hour' : 'hours'}`;
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(n < 10240 ? 1 : 0)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(n < 10 * 1024 ** 2 ? 1 : 0)} MB`;
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

/** Rough wall-clock for a stamp: expected tries = 2^bits. */
export function stampEstimateMs(bits: number, hashesPerSecond: number): number {
  return Math.round((2 ** bits / Math.max(1, hashesPerSecond)) * 1000);
}

/** Leading zero bits of a hash: the work a stamp or an identity nonce carries (PROTOCOL.md §4). */
export function leadingZeroBits(hash: Uint8Array): number {
  let n = 0;
  for (const b of hash) {
    if (b === 0) {
      n += 8;
      continue;
    }
    return n + Math.clz32(b) - 24;
  }
  return n;
}

// ---------------------------------------------------------------- recovery keys

/** A recovery key starts with this (`hoi_crypto::recovery`): the root key as one line of z-base-32. */
export const RECOVERY_KEY_PREFIX = 'hoi-key-1-';
/** The body after the prefix: `secret[32] ‖ check[3]` is 35 bytes, 56 characters. */
const RECOVERY_KEY_CHARS = 56;

const ZB32 = 'ybndrfg8ejkmcpqxot1uwisza345h769';

/** z-base-32, as the core writes keys and recovery lines. */
export function zbase32Encode(data: Uint8Array): string {
  let out = '';
  let buf = 0;
  let bits = 0;
  for (const b of data) {
    buf = ((buf << 8) | b) >>> 0;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out += ZB32[(buf >>> bits) & 31];
    }
  }
  if (bits > 0) out += ZB32[(buf << (5 - bits)) & 31];
  return out;
}

export function zbase32Decode(s: string): Uint8Array | null {
  const out: number[] = [];
  let buf = 0;
  let bits = 0;
  for (const c of s.toLowerCase()) {
    const v = ZB32.indexOf(c);
    if (v < 0) return null;
    buf = ((buf << 5) | v) >>> 0;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      out.push((buf >>> bits) & 255);
    }
  }
  return Uint8Array.from(out);
}

/** The body of a pasted recovery key, forgiving case, hyphens, spaces and `0`/`l` for `o`/`1`;
 * null when the text isn't shaped like one. The check behind it is the core's to verify. */
export function recoveryKeyBody(text: string): string | null {
  const body = text.replace(/[^A-Za-z0-9]/g, '').toLowerCase().replace(/0/g, 'o').replace(/l/g, '1');
  const prefix = RECOVERY_KEY_PREFIX.replace(/[^a-z0-9]/g, '');
  if (!body.startsWith(prefix)) return null;
  const rest = body.slice(prefix.length);
  return rest.length === RECOVERY_KEY_CHARS ? rest : null;
}

/** Whether `text` looks like a recovery key rather than a passphrase or a link. */
export const looksLikeRecoveryKey = (text: string): boolean => recoveryKeyBody(text) !== null;

/** The line as the core prints it: the prefix, then seven groups of eight. */
export function formatRecoveryKey(body: string): string {
  return RECOVERY_KEY_PREFIX + (body.match(/.{1,8}/g) ?? []).join('-');
}
