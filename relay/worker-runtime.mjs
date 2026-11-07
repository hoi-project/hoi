import { API_VERSION, MAX_OBJECT_BYTES, MAX_TTL_SECONDS, MIN_OBJECT_BYTES, READ_WINDOW_SECONDS, integer, isPaddedSize, removals, verifyEnvelope, verifyRead } from './protocol.mjs';

const MiB = 1024 * 1024;
const PAGE_SIZE = 64;
const HEADERS = { 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff', 'Referrer-Policy': 'no-referrer', 'Strict-Transport-Security': 'max-age=31536000; includeSubDomains', Vary: 'Origin' };
const REQUEST_HEADERS = 'Content-Type, X-Hoi-Expires, X-Hoi-Signature, X-Hoi-Read-Time, X-Hoi-Read-Nonce, X-Hoi-Read-Signature';
const COLUMNS = 'id, sequence, expires, size';
// Readers get what they need to fetch and check an object; how long it stays is the uploader's business.
const listed = (row) => ({ id: row.id, sequence: row.sequence, size: row.size });
const receipt = (row) => ({ id: row.id, sequence: row.sequence, size: row.size, expires: row.expires });
const json = (status, body, headers = HEADERS) => new Response(JSON.stringify(body), { status, headers: { ...Object.fromEntries(new Headers(headers)), 'Content-Type': 'application/json; charset=utf-8' } });
const unavailable = () => json(503, { error: 'unavailable', message: 'Encrypted storage is temporarily unavailable. Please try again later.' });

function setting(env, name, fallback, maximum) {
  const value = env[name] === undefined ? fallback : integer(String(env[name]));
  if (value === null || value < 1 || value > maximum) throw new Error(`Invalid ${name}.`);
  return value;
}

function allowedOrigins(configured) {
  return new Set((configured || '').split(',').map(value => value.trim()).filter(Boolean).map(origin => {
    if (origin === 'tauri://localhost') return origin;
    const url = new URL(origin);
    if (!['https:', 'http:'].includes(url.protocol) || url.origin !== origin) throw new Error('Invalid relay origin.');
    return origin;
  }));
}

function rateLimit(capacity, now) {
  let start = now();
  let used = 0;
  return () => {
    const time = now();
    if (time - start >= 60_000) { start = time; used = 0; }
    return ++used <= capacity ? 0 : Math.max(1, Math.ceil((60_000 - (time - start)) / 1000));
  };
}

async function readBody(request) {
  const length = request.headers.get('content-length');
  if (length !== null && (integer(length) === null || Number(length) > MAX_OBJECT_BYTES)) throw Object.assign(new Error(), { status: 413 });
  if (!request.body) return new Uint8Array();
  const reader = request.body.getReader();
  // Bound allocation count as well as payload bytes when a sender uses tiny chunks.
  const bytes = new Uint8Array(MAX_OBJECT_BYTES);
  let size = 0;
  let timer;
  const deadline = new Promise((_, reject) => { timer = setTimeout(() => reject(Object.assign(new Error(), { status: 408 })), 10_000); });
  try {
    while (true) {
      const part = await Promise.race([reader.read(), deadline]);
      if (part.done) break;
      if (size + part.value.byteLength > MAX_OBJECT_BYTES) {
        await reader.cancel();
        throw Object.assign(new Error(), { status: 413 });
      }
      bytes.set(part.value, size);
      size += part.value.byteLength;
    }
  } catch (error) {
    void reader.cancel().catch(() => {});
    throw error;
  } finally { clearTimeout(timer); reader.releaseLock(); }
  return bytes.slice(0, size);
}

/** Bounded pilot storage; one object enforces total quotas across every opaque room. */
export class RelayObject {
  constructor(ctx, env, now = Date.now) {
    this.ctx = ctx;
    this.sql = ctx.storage.sql;
    this.now = now;
    this.origins = allowedOrigins(env.HOI_RELAY_ORIGINS);
    this.maxBytes = setting(env, 'HOI_RELAY_MAX_BYTES', 512 * MiB, 512 * MiB);
    this.roomBytes = setting(env, 'HOI_RELAY_ROOM_BYTES', 64 * MiB, 64 * MiB);
    this.roomObjects = setting(env, 'HOI_RELAY_ROOM_OBJECTS', 4096, 4096);
    // The byte limit alone would permit an excessive number of tiny objects and signatures.
    this.maxObjects = setting(env, 'HOI_RELAY_MAX_OBJECTS', 32768, 32768);
    this.requests = rateLimit(setting(env, 'HOI_RELAY_REQUESTS_PER_MINUTE', 600, 100000), now);
    this.writes = rateLimit(setting(env, 'HOI_RELAY_WRITES_PER_MINUTE', 60, 10000), now);
    const purge = removals(env.HOI_RELAY_PURGE);
    ctx.storage.transactionSync(() => {
      this.sql.exec('CREATE TABLE IF NOT EXISTS relay_schema (version INTEGER NOT NULL) STRICT');
      const version = this.one('SELECT version FROM relay_schema')?.version;
      if (version !== undefined && version !== 1 && version !== 2) throw new Error('Unsupported encrypted storage schema.');
      // Version 1 objects stay in `objects`, never served again, until they expire (31 days at most).
      this.sql.exec(`CREATE TABLE IF NOT EXISTS objects (
        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
        room TEXT NOT NULL, id TEXT NOT NULL, author TEXT NOT NULL,
        expires INTEGER NOT NULL, size INTEGER NOT NULL CHECK(size > 0), signature TEXT NOT NULL,
        grant_expires INTEGER, grant_signature TEXT, body BLOB NOT NULL,
        UNIQUE(room, id)
      ) STRICT`);
      this.sql.exec('CREATE INDEX IF NOT EXISTS objects_expiry ON objects(expires)');
      this.sql.exec(`CREATE TABLE IF NOT EXISTS objects_v2 (
        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
        room TEXT NOT NULL, id TEXT NOT NULL, expires INTEGER NOT NULL, size INTEGER NOT NULL CHECK(size > 0), body BLOB NOT NULL,
        UNIQUE(room, id)
      ) STRICT`);
      this.sql.exec('CREATE INDEX IF NOT EXISTS objects_v2_room_sequence ON objects_v2(room, sequence)');
      this.sql.exec('CREATE INDEX IF NOT EXISTS objects_v2_expiry ON objects_v2(expires)');
      // One digest per accepted read, kept only while its signature could still be replayed.
      this.sql.exec('CREATE TABLE IF NOT EXISTS reads (digest TEXT PRIMARY KEY, expires INTEGER NOT NULL) STRICT');
      this.sql.exec('CREATE INDEX IF NOT EXISTS reads_expiry ON reads(expires)');
      // Identifiers an operator removed (id '' is a whole room), so the same bytes cannot return.
      this.sql.exec('CREATE TABLE IF NOT EXISTS removed (room TEXT NOT NULL, id TEXT NOT NULL, PRIMARY KEY(room, id)) STRICT');
      if (version === undefined) this.sql.exec('INSERT INTO relay_schema(version) VALUES (2)');
      else if (version === 1) this.sql.exec('UPDATE relay_schema SET version = 2');
      for (const entry of purge) {
        if (entry === 'v1') this.sql.exec('DELETE FROM objects');
        else this.sql.exec('INSERT OR IGNORE INTO removed(room, id) VALUES (?, ?)', ...(entry + '/').split('/').slice(0, 2));
      }
      // Reapplied at every start, so storage restored from an earlier point is purged again.
      for (const table of ['objects', 'objects_v2']) {
        this.sql.exec(`DELETE FROM ${table} WHERE EXISTS (SELECT 1 FROM removed WHERE removed.room = ${table}.room AND removed.id IN ('', ${table}.id))`);
      }
    });
  }

  one(query, ...args) { return [...this.sql.exec(query, ...args)][0]; }

  removed(room, id = '') {
    return !!this.one("SELECT 1 AS hit FROM removed WHERE room = ? AND id IN ('', ?)", room, id);
  }

  cleanup() {
    const now = Math.floor(this.now() / 1000);
    for (const table of ['objects', 'objects_v2']) this.sql.exec(`DELETE FROM ${table} WHERE expires <= ?`, now);
    this.sql.exec('DELETE FROM reads WHERE expires < ?', now);
  }

  async scheduleExpiry() {
    const next = this.one(`SELECT MIN(expires) AS expires FROM (SELECT MIN(expires) AS expires FROM objects
      UNION ALL SELECT MIN(expires) FROM objects_v2 UNION ALL SELECT MIN(expires) + 1 FROM reads)`)?.expires;
    this.alarmAt = next ?? null;
    if (next !== null && next !== undefined) await this.ctx.storage.setAlarm(next * 1000);
    else await this.ctx.storage.deleteAlarm();
  }

  async alarm() {
    this.cleanup();
    await this.scheduleExpiry();
  }

  /** Accept a read once: a signature seen before is a replay, whoever sends it. */
  async authorized(room, target, headers) {
    const read = verifyRead(room, target, headers, Math.floor(this.now() / 1000));
    if (!read) return false;
    const fresh = this.ctx.storage.transactionSync(() => {
      this.sql.exec('DELETE FROM reads WHERE expires < ?', Math.floor(this.now() / 1000));
      return !!this.one('INSERT INTO reads(digest, expires) VALUES (?, ?) ON CONFLICT DO NOTHING RETURNING digest', read.digest, read.expires);
    });
    // An alarm removes the last digests even when no later read arrives to clean them up.
    if (fresh && (this.alarmAt == null || read.expires + 1 < this.alarmAt)) await this.scheduleExpiry();
    return fresh;
  }

  async fetch(request) {
    const headers = new Headers(HEADERS);
    const send = (status, body) => json(status, body, headers);
    try {
      const url = new URL(request.url);
      const origin = request.headers.get('origin');
      if (origin && origin !== url.origin && !this.origins.has(origin)) return send(403, { error: 'origin-denied', message: 'Encrypted storage is unavailable from this app.' });
      if (origin) headers.set('Access-Control-Allow-Origin', origin);
      if (url.pathname.startsWith('/relay/v1/')) return send(410, { error: 'protocol-gone', message: 'This relay speaks protocol version 2. Update the app.' });
      const meta = url.pathname === '/relay/v2/meta';
      const match = /^\/relay\/v2\/rooms\/([0-9a-f]{64})\/objects(?:\/([0-9a-f]{64}))?$/.exec(url.pathname);
      if (!meta && !match) return send(404, { error: 'not-found' });
      const methods = match?.[2] ? ['GET', 'PUT'] : ['GET'];
      if (request.method === 'OPTIONS') {
        const method = request.headers.get('access-control-request-method');
        if (method && !methods.includes(method)) return send(405, { error: 'method-not-allowed' });
        headers.set('Access-Control-Allow-Methods', [...methods, 'OPTIONS'].join(', '));
        headers.set('Access-Control-Allow-Headers', REQUEST_HEADERS);
        headers.set('Access-Control-Max-Age', '600');
        return new Response(null, { status: 204, headers });
      }
      if (!methods.includes(request.method)) {
        headers.set('Allow', [...methods, 'OPTIONS'].join(', '));
        return send(405, { error: 'method-not-allowed' });
      }
      const retry = this.requests() || (request.method === 'PUT' ? this.writes() : 0);
      if (retry) {
        headers.set('Retry-After', String(retry));
        return send(429, { error: 'rate-limited', message: 'Encrypted storage is busy. Please try again shortly.' });
      }
      if (meta) {
        if (url.search) return send(400, { error: 'invalid-request' });
        return send(200, { apiVersion: API_VERSION, maxObjectBytes: MAX_OBJECT_BYTES, minObjectBytes: MIN_OBJECT_BYTES, maxTtlSeconds: MAX_TTL_SECONDS,
          readWindowSeconds: READ_WINDOW_SECONDS, pageSize: PAGE_SIZE, maxBytes: this.maxBytes, roomBytes: this.roomBytes, maxObjects: this.maxObjects,
          roomObjects: this.roomObjects, removed: this.one('SELECT COUNT(*) AS count FROM removed').count });
      }
      const [, room, id] = match;
      const denied = () => send(401, { error: 'read-denied', message: 'This device could not prove access to these encrypted messages.' });
      const gone = () => send(410, { error: 'removed', message: 'The relay operator removed these encrypted messages.' });
      if (!id) {
        const keys = [...url.searchParams.keys()];
        if (keys.length > 1 || keys.length === 1 && keys[0] !== 'after') return send(400, { error: 'invalid-request' });
        const after = integer(url.searchParams.get('after') ?? '0');
        if (after === null) return send(400, { error: 'invalid-cursor' });
        if (!await this.authorized(room, `objects${url.search}`, request.headers)) return denied();
        if (this.removed(room)) return gone();
        this.cleanup();
        const rows = [...this.sql.exec(`SELECT ${COLUMNS} FROM objects_v2 WHERE room = ? AND sequence > ? ORDER BY sequence LIMIT ?`, room, after, PAGE_SIZE + 1)];
        const objects = rows.slice(0, PAGE_SIZE).map(listed);
        return send(200, { objects, ...(rows.length > PAGE_SIZE ? { nextCursor: objects.at(-1).sequence } : {}) });
      }
      if (url.search) return send(400, { error: 'invalid-request' });
      if (request.method === 'GET') {
        if (!await this.authorized(room, `objects/${id}`, request.headers)) return denied();
        if (this.removed(room, id)) return gone();
        this.cleanup();
        const row = this.one('SELECT size, body FROM objects_v2 WHERE room = ? AND id = ?', room, id);
        if (!row) return send(404, { error: 'not-found' });
        const bytes = row.body instanceof ArrayBuffer ? new Uint8Array(row.body) : row.body;
        headers.set('Content-Type', 'application/octet-stream');
        headers.set('Content-Length', String(row.size));
        return new Response(bytes, { status: 200, headers });
      }
      if (!/^application\/octet-stream(?:\s*;|$)/i.test(request.headers.get('content-type') || '')) return send(415, { error: 'content-type', message: 'An encrypted binary object is required.' });
      const bytes = await readBody(request);
      if (!isPaddedSize(bytes.byteLength)) return send(400, { error: 'unpadded', message: 'Encrypted objects must be padded to a storage size.' });
      const entry = verifyEnvelope(room, id, request.headers, bytes, Math.floor(this.now() / 1000));
      if (!entry) return send(400, { error: 'invalid-proof', message: 'The encrypted object could not be verified.' });
      if (this.removed(room, id)) return gone();
      const result = this.ctx.storage.transactionSync(() => {
        this.cleanup();
        const previous = this.one(`SELECT ${COLUMNS} FROM objects_v2 WHERE room = ? AND id = ?`, room, id);
        if (previous) {
          if (previous.expires !== entry.expires) return { status: 409, body: { error: 'object-conflict', message: 'This encrypted object already has a different signed envelope.' } };
          return { status: 200, body: receipt(previous) };
        }
        const total = this.one('SELECT COUNT(*) AS count, COALESCE(SUM(size), 0) AS size FROM (SELECT size FROM objects UNION ALL SELECT size FROM objects_v2)');
        const used = this.one('SELECT COUNT(*) AS count, COALESCE(SUM(size), 0) AS size FROM objects_v2 WHERE room = ?', room);
        if (total.size + bytes.byteLength > this.maxBytes || used.size + bytes.byteLength > this.roomBytes
          || total.count >= this.maxObjects || used.count >= this.roomObjects) return { status: 507, body: { error: 'storage-full', message: 'Encrypted storage has reached its limit. Try another relay.' } };
        const row = this.one(`INSERT INTO objects_v2(room, id, expires, size, body) VALUES (?, ?, ?, ?, ?) RETURNING ${COLUMNS}`,
          room, id, entry.expires, entry.size, bytes.buffer);
        return { status: 201, body: receipt(row) };
      });
      if (result.status === 200 || result.status === 201) await this.scheduleExpiry();
      return send(result.status, result.body);
    } catch (error) {
      if (error.status === 413) return send(413, { error: 'too-large', message: 'The encrypted object exceeds the storage size limit.' });
      if (error.status === 408) return send(408, { error: 'request-timeout', message: 'The encrypted upload took too long. Please try again.' });
      return send(503, { error: 'unavailable', message: 'Encrypted storage is temporarily unavailable. Please try again later.' });
    }
  }
}

/** A separate binding keeps opaque content out of the public-name registry. */
export async function fetchRelay(request, env) {
  try {
    if (!new URL(request.url).pathname.startsWith('/relay/')) return json(404, { error: 'not-found' });
    const id = env.OPAQUE_RELAY.idFromName('opaque-objects');
    return await env.OPAQUE_RELAY.get(id).fetch(request);
  } catch { return unavailable(); }
}
