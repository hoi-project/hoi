// Local observation adapters. Requests still pass through the production services;
// only the sandbox process can inspect these bounded, explicitly selected fields.
import { createServer } from 'node:http';
import { Readable } from 'node:stream';
import { DatabaseSync } from 'node:sqlite';
import { existsSync } from 'node:fs';
import { RelayObject } from '../relay/worker-runtime.mjs';
import { MAX_OBJECT_BYTES, objectId } from '../relay/protocol.mjs';
import { createRegistryServer } from '../registry/server.mjs';

const LIMIT = 32;
const CIPHER_PREVIEW_BYTES = 64;
const PUBLIC_PREVIEW_BYTES = 512;
const HEX_ID = /^[a-f0-9]{64}$/;

function localOrigins(origins) {
  if (!Array.isArray(origins)) throw new Error('Sandbox origins must be a list.');
  return origins.map(value => {
    const url = new URL(value);
    if (url.protocol !== 'http:' || !['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
      || url.origin !== value || url.username || url.password) throw new Error('Sandbox services accept only exact loopback origins.');
    return value;
  });
}

function canaryBytes(value) {
  if (value === undefined) return null;
  // The composer limits characters; UTF-8 messages may need several bytes each.
  // Match the controller's bounded request size so an accepted post stays inspectable.
  if (typeof value !== 'string' || !value || Buffer.byteLength(value) > 16384) throw new Error('The plaintext check needs 1–16384 bytes.');
  return Buffer.from(value);
}

function closeServer(server) {
  return new Promise((resolve, reject) => {
    server.close(error => error && error.code !== 'ERR_SERVER_NOT_RUNNING' ? reject(error) : resolve());
    server.closeAllConnections();
  });
}

/** The production relay's Durable Object storage contract, backed by local SQLite. */
export function createLocalRelay({ filename, origins = [], now = Date.now } = {}) {
  const allowed = localOrigins(origins);
  const db = new DatabaseSync(filename);
  let timer, relay, closed = false, closing;
  const active = new Set();
  const storage = {
    sql: { exec(query, ...args) { return db.prepare(query).all(...args.map(value => value instanceof ArrayBuffer ? new Uint8Array(value) : value)); } },
    transactionSync(callback) {
      db.exec('BEGIN IMMEDIATE');
      try { const result = callback(); db.exec('COMMIT'); return result; }
      catch (error) { db.exec('ROLLBACK'); throw error; }
    },
    async setAlarm(at) {
      clearTimeout(timer);
      if (closed) return;
      // Node timers cap at 2^31−1 ms; long-lived objects are checked again later.
      timer = setTimeout(() => {
        if (!closed) void relay.alarm().catch(() => {});
      }, Math.min(2 ** 31 - 1, Math.max(1, at - now())));
      timer.unref();
    },
    async deleteAlarm() { clearTimeout(timer); },
  };
  try { relay = new RelayObject({ storage }, { HOI_RELAY_ORIGINS: allowed.join(',') }, now); }
  catch (error) { db.close(); throw error; }
  // Restored objects also expire without waiting for a new client request.
  const ready = relay.alarm();
  const server = createServer({ maxHeaderSize: 8192, requestTimeout: 10_000, headersTimeout: 10_000 }, (request, response) => {
    const operation = (async () => {
      try {
        const port = request.socket.localPort;
        const hosts = [`127.0.0.1:${port}`, `localhost:${port}`, `[::1]:${port}`];
        if (!hosts.includes(request.headers.host) || !request.url.startsWith('/') || request.url.startsWith('//')) {
          response.writeHead(403, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' });
          response.end('{"error":"local-only"}');
          return;
        }
        await ready;
        const url = new URL(request.url, `http://${request.headers.host}`);
        const init = { method: request.method, headers: request.headers };
        if (!['GET', 'HEAD'].includes(request.method)) { init.body = Readable.toWeb(request); init.duplex = 'half'; }
        const result = await relay.fetch(new Request(url, init));
        response.writeHead(result.status, Object.fromEntries(result.headers));
        response.end(Buffer.from(await result.arrayBuffer()));
      } catch {
        if (!response.headersSent) response.writeHead(503, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' });
        response.end('{"error":"unavailable"}');
      }
    })();
    active.add(operation);
    void operation.finally(() => active.delete(operation));
  });
  server.setTimeout(15_000, socket => socket.destroy());
  return {
    server,
    inspectObject({ id, room, expectedBody } = {}) {
      if (closed) throw new Error('The local relay is closed.');
      if (typeof id !== 'string' || !HEX_ID.test(id) || typeof room !== 'string' || !HEX_ID.test(room)) {
        throw new Error('Select an encrypted object by its 64-character room and object IDs.');
      }
      if (expectedBody !== undefined && !(expectedBody instanceof Uint8Array)) throw new Error('Expected ciphertext must be a byte array.');
      if (expectedBody !== undefined && (expectedBody.byteLength < 1 || expectedBody.byteLength > MAX_OBJECT_BYTES)) {
        throw new Error('Expected ciphertext must be bounded by the relay object limit.');
      }
      relay.cleanup();
      const row = db.prepare('SELECT id, room, sequence, expires, size, length(body) AS bytes FROM objects_v2 WHERE room = ? AND id = ?').get(room, id);
      if (!row) return { present: false };
      // Check length before loading the BLOB: even a damaged local database cannot make
      // this observation allocate an unbounded body or claim inconsistent size metadata.
      if (row.bytes < 1 || row.bytes > MAX_OBJECT_BYTES || row.bytes !== row.size) throw new Error('Stored ciphertext exceeds its bounded object size.');
      const bytes = Buffer.from(db.prepare('SELECT body FROM objects_v2 WHERE room = ? AND id = ?').get(room, id).body);
      const sha256 = objectId(bytes);
      // This observes byte identity only. A relay cannot establish whether its opaque
      // bytes are encrypted or decrypt them, and no signing/read credentials are returned.
      return { present: true, record: {
        id: row.id, room: row.room, sequence: row.sequence, expires: row.expires, size: row.size,
        sha256, hashMatchesId: sha256 === row.id, ciphertextPrefix: bytes.subarray(0, CIPHER_PREVIEW_BYTES).toString('hex'),
      }, ...(expectedBody === undefined ? {} : { matchesExpected: bytes.equals(expectedBody) }) };
    },
    snapshot({ plaintextCanary } = {}) {
      if (closed) throw new Error('The local relay is closed.');
      const canary = canaryBytes(plaintextCanary);
      relay.cleanup();
      const total = db.prepare('SELECT COUNT(*) AS count FROM objects_v2').get().count;
      const rows = db.prepare('SELECT id, room, sequence, expires, size, substr(body, 1, ?) AS preview FROM objects_v2 ORDER BY sequence DESC LIMIT ?').all(CIPHER_PREVIEW_BYTES, LIMIT);
      const records = rows.map(row => ({ id: row.id, room: row.room, sequence: row.sequence, expires: row.expires, size: row.size,
        ciphertextPrefix: Buffer.from(row.preview).toString('hex') }));
      const result = {
        kind: 'relay', status: 'ready', records, total, truncated: total > records.length,
        visibleFields: ['Room public key', 'Object hash', 'Padded size', 'Expiry', 'Sequence', 'Ciphertext'],
        hiddenFields: ['Post title and text', 'Community name', 'Member identity', 'Decryption keys'],
      };
      if (canary) result.containsCanary = Boolean(db.prepare('SELECT 1 AS found FROM objects_v2 WHERE instr(body, ?) > 0 LIMIT 1').get(canary));
      return result;
    },
    close() {
      return closing ??= (async () => {
        closed = true;
        clearTimeout(timer);
        await closeServer(server);
        await Promise.allSettled([...active, ready]);
        db.close();
      })();
    },
  };
}

export function createLocalRegistry({ filename, origins = [], now = Date.now } = {}) {
  const allowed = localOrigins(origins);
  const server = createRegistryServer({ filename, origins: allowed, now, publicNodes: [],
    appUrl: `${allowed[0] || 'http://127.0.0.1'}/`, allowPrivateRoutesForTests: true });
  let closed = false, closing;
  return {
    server,
    snapshot() {
      if (closed) throw new Error('The local registry is closed.');
      // No log_secret, signing material, or generic table browser crosses this boundary.
      const db = new DatabaseSync(filename, { readOnly: true });
      try {
        const names = db.prepare(`SELECT c.name, c.community, i.protocol_version, i.index_version
          FROM communities c LEFT JOIN community_index i ON i.community = c.community ORDER BY c.name LIMIT ?`).all(LIMIT);
        const nodes = db.prepare('SELECT endpoint, routes, communities, issued_at FROM public_nodes ORDER BY endpoint LIMIT ?').all(LIMIT);
        const total = db.prepare('SELECT (SELECT COUNT(*) FROM communities) + (SELECT COUNT(*) FROM public_nodes) AS count').get().count;
        const records = [
          ...names.map(row => ({ kind: 'community', name: row.name, community: row.community, protocolVersion: row.protocol_version, indexVersion: row.index_version })),
          ...nodes.map(row => ({ kind: 'node', endpoint: row.endpoint, routes: row.routes.split(',').slice(0, 8), communities: row.communities ? row.communities.split(',').slice(0, 64) : [], issuedAt: row.issued_at })),
        ];
        return { kind: 'registry', status: 'ready', records, total, truncated: total > records.length,
          visibleFields: ['Public community name and key', 'Public node key and routes', 'Announced community keys', 'Announcement time'],
          hiddenFields: ['Post text', 'Private community contents', 'Member decryption keys'] };
      } finally { db.close(); }
    },
    close() { return closing ??= (async () => { closed = true; await closeServer(server); })(); },
  };
}

/** Inspect only the selected public record, never the node's identity/configuration blobs. */
export function inspectPublicRecord({ filename, postId, plaintextCanary } = {}) {
  if (typeof postId !== 'string' || !HEX_ID.test(postId)) throw new Error('Select one public record by its 64-character ID.');
  const canary = canaryBytes(plaintextCanary);
  if (!existsSync(filename)) return { present: false };
  const db = new DatabaseSync(filename, { readOnly: true });
  try {
    const row = db.prepare(`SELECT id, community, expires_at, length(wire) AS bytes, substr(wire, 1, ?) AS preview
      FROM records WHERE id = ?`).get(PUBLIC_PREVIEW_BYTES, Buffer.from(postId, 'hex'));
    if (!row) return { present: false };
    const preview = Buffer.from(row.preview);
    const result = { present: true, record: {
      id: Buffer.from(row.id).toString('hex'), community: row.community ? Buffer.from(row.community).toString('hex') : null,
      expires: row.expires_at, bytes: row.bytes, wireHex: preview.toString('hex'),
      wireText: preview.toString('utf8').replace(/[\u0000-\u001f\u007f-\u009f]/g, '·'),
    } };
    if (canary) result.containsCanary = Boolean(db.prepare('SELECT 1 AS found FROM records WHERE id = ? AND instr(wire, ?) > 0').get(Buffer.from(postId, 'hex'), canary));
    return result;
  } finally { db.close(); }
}
