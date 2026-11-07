import type { HoiCore, PrivateGroup } from '@hoi/core-api';
import { fromBase64Url, toBase64Url } from './core/rpc-core';

export const RELAY_URLS: string[] = typeof __HOI_RELAY_URLS__ === 'undefined' ? [] : __HOI_RELAY_URLS__;
const HASH = /^[0-9a-f]{64}$/;
const MAX_OBJECT = 1_048_576;
const MIN_OBJECT = 4096;
const MAX_OBJECTS = 4096;
// Each commit moves a group to a new room; a device that was away catches up this many per pass.
const MAX_EPOCHS = 8;
type Entry = { id: string; sequence: number; size: number };
export interface PrivateSyncStatus { busy: boolean; replicas: number; total: number; lastSync?: number; error?: string }

async function bounded(response: Response, limit: number, signal?: AbortSignal): Promise<Uint8Array> {
  signal?.throwIfAborted();
  if (Number(response.headers.get('content-length')) > limit) throw new Error('The relay returned too much data.');
  if (!response.body) return new Uint8Array();
  const reader = response.body.getReader();
  const buffer = new Uint8Array(limit);
  let size = 0;
  const cancel = () => { void reader.cancel().catch(() => undefined); };
  signal?.addEventListener('abort', cancel, { once: true });
  try {
    for (;;) {
      signal?.throwIfAborted();
      const part = await reader.read();
      if (part.done) break;
      if (size + part.value.length > limit) { await reader.cancel(); throw new Error('The relay returned too much data.'); }
      buffer.set(part.value, size);
      size += part.value.length;
    }
  } finally { signal?.removeEventListener('abort', cancel); reader.releaseLock(); }
  signal?.throwIfAborted();
  return buffer.slice(0, size);
}

async function digest(bytes: Uint8Array): Promise<string> {
  return [...new Uint8Array(await crypto.subtle.digest('SHA-256', new Uint8Array(bytes)))].map(b => b.toString(16).padStart(2, '0')).join('');
}

function entry(value: unknown): Entry {
  const v = value as Entry;
  if (!v || !HASH.test(v.id) || !Number.isSafeInteger(v.sequence) || v.sequence < 1
    || !Number.isSafeInteger(v.size) || v.size < MIN_OBJECT || v.size > MAX_OBJECT) throw new Error('The relay returned an invalid object list.');
  return v;
}

/**
 * Hosts receive sealed ciphertext and room signatures only. Every listing and download is
 * signed by the current epoch's room key inside the core; invites never choose a destination.
 */
export async function syncPrivateGroup(core: HoiCore, community: string, endpoints = RELAY_URLS, fetcher: typeof fetch = fetch, signal?: AbortSignal): Promise<PrivateSyncStatus> {
  signal?.throwIfAborted();
  if (!endpoints.length) return { busy: false, replicas: 0, total: 0, error: 'Private syncing is unavailable. Your data stays on this device.' };
  let group: PrivateGroup = await core.privateGroup(community);
  if (!group.active) return { busy: false, replicas: 0, total: endpoints.length, error: 'This device no longer belongs to the private group.' };
  const failed = new Set<string>();
  let invalid = false;
  let downloaded = 0;
  const call = async (url: string, init?: RequestInit) => {
    signal?.throwIfAborted();
    const timeout = AbortSignal.timeout(15_000);
    const response = await fetcher(url, { ...init, credentials: 'omit', referrerPolicy: 'no-referrer', cache: 'no-store', redirect: 'error', signal: signal ? AbortSignal.any([signal, timeout]) : timeout });
    if (!response.ok) { await response.body?.cancel(); throw new Error('The encrypted relay is unavailable. Your changes remain saved on this device.'); }
    return response;
  };
  // The relay accepts each signature once, so ask for exactly the requests about to be sent.
  const authorize = async (room: string, targets: string[]) => {
    const signed = await core.privateRead(community, targets);
    if (signed.room !== room || signed.reads.length !== targets.length) throw new Error('The private group moved to a new epoch.');
    return signed.reads.map(read => ({ 'x-hoi-read-time': String(read.time), 'x-hoi-read-nonce': read.nonce, 'x-hoi-read-signature': read.signature }));
  };

  for (let epoch = 0; epoch < MAX_EPOCHS && group.active; epoch++) {
    const room = group.room;
    if (!HASH.test(room)) throw new Error('This private group has an invalid relay address.');
    const seen = new Set(group.received);
    const received = new Map<string, string>();
    for (const endpoint of endpoints) {
      if (failed.has(endpoint)) continue;
      try {
        let cursor = 0;
        let count = 0;
        const base = `${endpoint}/v2/rooms/${room}/`;
        for (let page = 0; page < 64; page++) {
          const target = `objects?after=${cursor}`;
          const [listing] = await authorize(room, [target]);
          const list = JSON.parse(new TextDecoder().decode(await bounded(await call(base + target, { headers: listing }), 65_536, signal)));
          if (!Array.isArray(list.objects) || list.objects.length > 64) throw new Error('Invalid encrypted relay listing.');
          const items = list.objects.map(entry) as Entry[];
          const wanted: Entry[] = [];
          for (const item of items) {
            if (item.sequence <= cursor || ++count > MAX_OBJECTS) throw new Error('Invalid encrypted relay cursor.');
            cursor = item.sequence;
            if (!seen.has(item.id) && !received.has(item.id) && !wanted.some(other => other.id === item.id)) wanted.push(item);
          }
          const reads = wanted.length ? await authorize(room, wanted.map(item => `objects/${item.id}`)) : [];
          for (const [index, item] of wanted.entries()) {
            signal?.throwIfAborted();
            if (downloaded + item.size > 64 * MAX_OBJECT) throw new Error('The encrypted relay exceeded the download limit.');
            const bytes = await bounded(await call(`${base}objects/${item.id}`, { headers: reads[index] }), MAX_OBJECT, signal);
            downloaded += bytes.length;
            if (bytes.length !== item.size || await digest(bytes) !== item.id) { invalid = true; continue; }
            received.set(item.id, toBase64Url(bytes));
          }
          if (list.nextCursor === undefined) break;
          if (!items.length || list.nextCursor !== cursor) throw new Error('Invalid encrypted relay cursor.');
          if (page === 63) throw new Error('The encrypted relay listing exceeds its limit.');
        }
      } catch { signal?.throwIfAborted(); failed.add(endpoint); /* Independent replicas continue when one is unavailable. */ }
    }

    // Replicas can list one epoch's messages in different orders. Revisit deferred messages
    // after each pass that made progress; the core alone decides validity.
    for (let pass = 0; received.size && pass < 8 && group.room === room; pass++) {
      let progress = false;
      for (const [id, body] of received) {
        signal?.throwIfAborted();
        if (group.room !== room) break;
        try {
          group = await core.privateReceive(community, body);
          received.delete(id);
          progress = true;
        } catch { /* Retry after a missing message, never fall back to plaintext. */ }
      }
      if (!progress) break;
    }
    // A commit moved this device to the next epoch's room: what is left here belongs to an epoch
    // it no longer reads, and members resend live data in the new one.
    if (group.room === room || !group.active) { invalid ||= group.active && received.size > 0; break; }
  }
  if (!group.active) return { busy: false, replicas: endpoints.length - failed.size, total: endpoints.length, error: 'This device was removed. New messages are no longer available here.' };

  signal?.throwIfAborted();
  const outbox = await core.privateOutbox(community);
  const acknowledged: string[] = [];
  for (const item of outbox) {
    signal?.throwIfAborted();
    const body = fromBase64Url(item.body);
    if (!HASH.test(item.room) || body.length !== item.size || body.length > MAX_OBJECT || await digest(body) !== item.id) throw new Error('A saved encrypted message could not be verified.');
    let copies = 0;
    for (const endpoint of endpoints) {
      try {
        const headers = { 'Content-Type': 'application/octet-stream', 'x-hoi-expires': String(item.expires), 'x-hoi-signature': item.signature };
        const response = await call(`${endpoint}/v2/rooms/${item.room}/objects/${item.id}`, { method: 'PUT', headers, body: new Uint8Array(body) });
        const receipt = JSON.parse(new TextDecoder().decode(await bounded(response, 4096, signal)));
        if (receipt.id !== item.id || receipt.size !== item.size || receipt.expires !== item.expires) throw new Error('Invalid encrypted relay receipt.');
        copies++;
      } catch { signal?.throwIfAborted(); failed.add(endpoint); }
    }
    if (copies === endpoints.length) acknowledged.push(item.id);
  }
  signal?.throwIfAborted();
  if (acknowledged.length) await core.privateAck(community, acknowledged);
  const complete = !failed.size && !invalid;
  return {
    busy: false, replicas: endpoints.length - failed.size, total: endpoints.length,
    ...(complete ? { lastSync: Date.now() } : {}),
    ...(!complete ? { error: invalid ? 'Some encrypted updates could not be verified yet. Existing messages are unchanged.' : 'Some relay copies are unavailable. Your changes remain saved for retry.' } : {}),
  };
}
