import { createHash, createPublicKey, verify } from 'node:crypto';
import { keyIsStrong } from '../registry/protocol.mjs';

export const API_VERSION = 2;
export const MAX_OBJECT_BYTES = 1024 * 1024;
export const MIN_OBJECT_BYTES = 4096;
export const MAX_TTL_SECONDS = 31 * 24 * 60 * 60;
export const READ_WINDOW_SECONDS = 300;
export const PUBLIC_KEY = /^[0-9a-f]{64}$/;
const SIGNATURE = /^[0-9a-f]{128}$/;
const NONCE = /^[0-9a-f]{32}$/;
const ED25519_SPKI = Buffer.from('302a300506032b6570032100', 'hex');

export function objectStatement(room, id, expires, size) {
  return `hoi relay object v2\n${room}\n${id}\n${expires}\n${size}`;
}

export function readStatement(room, target, time, nonce) {
  return `hoi relay read v2\n${room}\n${target}\n${time}\n${nonce}`;
}

export function objectId(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

export function integer(value) {
  if (typeof value !== 'string' || !/^(?:0|[1-9][0-9]*)$/.test(value)) return null;
  const number = Number(value);
  return Number.isSafeInteger(number) ? number : null;
}

/** Stored sizes keep at most four significant bits: 4 KiB, 4.5 KiB … 7.5 KiB, 8 KiB, 9 KiB … 1 MiB. */
export function isPaddedSize(size) {
  return Number.isSafeInteger(size) && size >= MIN_OBJECT_BYTES && size <= MAX_OBJECT_BYTES && size % (1 << (28 - Math.clz32(size))) === 0;
}

/** The smallest padded size that holds `size` bytes; clients pad to it, at most 1/8 above 4 KiB. */
export function paddedSize(size) {
  const at = Math.max(MIN_OBJECT_BYTES, size);
  const step = 1 << (28 - Math.clz32(at));
  return Math.ceil(at / step) * step;
}

function signed(key, statement, signature) {
  // Small-order and non-canonical points verify forged signatures; no room may use one.
  if (!PUBLIC_KEY.test(key || '') || !SIGNATURE.test(signature || '') || !keyIsStrong(key)) return false;
  try {
    const publicKey = createPublicKey({ key: Buffer.concat([ED25519_SPKI, Buffer.from(key, 'hex')]), format: 'der', type: 'spki' });
    return verify(null, Buffer.from(statement, 'utf8'), publicKey, Buffer.from(signature, 'hex'));
  } catch { return false; }
}

/**
 * Verify an upload. The room is the epoch's public room key, so any current member may sign and
 * none is named. Version 1 writer and grant headers are refused rather than silently stored.
 */
export function verifyEnvelope(room, id, headers, bytes, now) {
  const expires = integer(headers.get('x-hoi-expires'));
  if (!PUBLIC_KEY.test(room) || !PUBLIC_KEY.test(id) || expires === null || expires <= now || expires > now + MAX_TTL_SECONDS
    || !isPaddedSize(bytes.byteLength) || objectId(bytes) !== id
    || ['x-hoi-author', 'x-hoi-grant-expires', 'x-hoi-grant-signature'].some(name => headers.has(name))) return null;
  if (!signed(room, objectStatement(room, id, expires, bytes.byteLength), headers.get('x-hoi-signature'))) return null;
  return { id, expires, size: bytes.byteLength };
}

/**
 * Verify a read: the room key signs this exact target, a time near the relay's clock and a fresh
 * nonce. The returned digest names the read for replay refusal without naming the room.
 */
export function verifyRead(room, target, headers, now) {
  const time = integer(headers.get('x-hoi-read-time'));
  const nonce = headers.get('x-hoi-read-nonce');
  if (time === null || Math.abs(time - now) > READ_WINDOW_SECONDS || !NONCE.test(nonce || '')) return null;
  if (!signed(room, readStatement(room, target, time, nonce), headers.get('x-hoi-read-signature'))) return null;
  return { digest: createHash('sha256').update(`hoi relay read v2\n${room}\n${nonce}`).digest('hex'), expires: time + READ_WINDOW_SECONDS };
}

/** Operator removals: `<room>`, `<room>/<id>` or `v1` (every legacy object), separated by commas or whitespace. */
export function removals(value) {
  const entries = [...new Set(String(value ?? '').split(/[\s,]+/).filter(Boolean))];
  for (const entry of entries) if (entry !== 'v1' && !/^[0-9a-f]{64}(?:\/[0-9a-f]{64})?$/.test(entry)) throw new Error('Invalid HOI_RELAY_PURGE.');
  return entries;
}
