import { createServer } from 'node:http';
import { DatabaseSync } from 'node:sqlite';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { loadRelease, validateRelease } from '../scripts/release-config.mjs';
import { normalizeName, normalizeCommunityKey, verifiedClaim, admitAnnouncement, validatePublicNodes, validateAppUrl, selectPublicNodes, registryOrigin,
  loopbackOrigins, validateAnnouncers, ANNOUNCER_REFUSED, WITHHELD } from './protocol.mjs';
import { createRegistryStore, DATABASE_VERSION, SCHEMA, upgradeSchema } from './store.mjs';
import { LOG_PATHS, answerLog, nameLog } from './log.mjs';
export { normalizeName, registrationStatement, publicationStatement, announcementStatement, validatePublicNodes, validateRoute, selectPublicNodes } from './protocol.mjs';

const MAX_BODY = 2048;
/** A node announcement lists up to 64 community keys and 8 routes. */
const MAX_ANNOUNCEMENT_BODY = 8192;
/** Permanent public reservations are independent of version-filtered discovery metadata. */
export function openRegistry(filename) {
  const db = new DatabaseSync(filename);
  try {
    const schemaVersion = db.prepare('PRAGMA user_version').get().user_version;
    if (schemaVersion > DATABASE_VERSION) throw new Error('This registry database requires a newer registry server.');
    db.exec('PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;');
    // A legacy database starts at user_version 0. Never invent a protocol for its claims.
    db.exec(`BEGIN IMMEDIATE;\n${SCHEMA.join(';\n')};\nPRAGMA user_version=${DATABASE_VERSION};\nCOMMIT;`);
    upgradeSchema(db);
  } catch (error) { db.close(); throw error; }
  const transaction = (callback) => {
    db.exec('BEGIN IMMEDIATE');
    try {
      const result = callback();
      db.exec('COMMIT');
      return result;
    } catch (error) { db.exec('ROLLBACK'); throw error; }
  };
  return { ...createRegistryStore(db, transaction), close() { db.close(); } };
}

function rateLimit(capacity, now) {
  let started = now();
  let used = 0;
  return () => {
    const time = now();
    if (time - started >= 60_000) { started = time; used = 0; }
    if (++used <= capacity) return 0;
    return Math.max(1, Math.ceil((60_000 - (time - started)) / 1000));
  };
}

function readBody(req, max = MAX_BODY) {
  return new Promise((resolveBody, reject) => {
    const parts = [];
    let size = 0;
    req.on('data', (part) => {
      size += part.length;
      if (size <= max) parts.push(part);
      else { parts.length = 0; reject(Object.assign(new Error(`Request body exceeds ${max} bytes.`), { status: 413 })); }
    });
    req.on('end', () => {
      if (size > max) return;
      try { resolveBody(JSON.parse(Buffer.concat(parts).toString('utf8'))); }
      catch { reject(Object.assign(new Error('Expected a JSON registration proof.'), { status: 400 })); }
    });
    req.on('error', reject);
    req.on('aborted', () => reject(new Error('Request aborted.')));
  });
}

/** Only public signed claims are retained; no access log or client-address table.
 * - `registryUrl`: this registry's public URL, whose origin announcements must name; without it
 *   only announcements signed for the loopback address a connection arrived at are taken.
 * - `announcers`: allowlist mode, the endpoint keys whose announcements are accepted (null: any).
 * - `allowPrivateRoutesForTests`: accept loopback and private announced routes, for an end-to-end
 *   test on one machine. Never set from the environment; the Worker has no such option. */
export function createRegistryServer({ filename = 'hoi-registry.db', origins = [], requestsPerMinute = 600, writesPerMinute = 30, announcementsPerMinute = 120, now = Date.now,
  publicNodes = [], nodeAnnouncements = true, appUrl = 'https://hoi.sh/app/', releaseProvider = loadRelease, registryUrl = null, announcers = null,
  allowPrivateRoutesForTests = false } = {}) {
  if (![requestsPerMinute, writesPerMinute, announcementsPerMinute].every((n) => Number.isInteger(n) && n > 0)) throw new Error('Rate limits must be positive integers.');
  if (typeof nodeAnnouncements !== 'boolean') throw new Error('nodeAnnouncements must be true or false.');
  if (typeof allowPrivateRoutesForTests !== 'boolean') throw new Error('allowPrivateRoutesForTests must be true or false.');
  const nodes = validatePublicNodes(publicNodes);
  const applicationUrl = validateAppUrl(appUrl);
  const ownOrigin = registryUrl === null ? null : registryOrigin(registryUrl);
  const allowedAnnouncers = validateAnnouncers(announcers);
  const registry = openRegistry(filename);
  const log = nameLog(registry);
  const seconds = () => Math.floor(now() / 1000);
  const live = () => nodeAnnouncements ? registry.liveNodes(seconds()).filter((node) => !allowedAnnouncers || allowedAnnouncers.has(node.endpoint)) : [];
  // Discovery keeps quiet about who carries a withheld community: the general order answers.
  const discovered = (community = null) => selectPublicNodes(nodes, live(), community && !registry.isWithheld(community) ? community : null);
  const metadata = () => {
    const release = validateRelease(releaseProvider());
    return { release, registry: { apiVersion: 2, schemaVersion: 3, indexVersion: release.indexVersion }, publicNodes: discovered(), appUrl: applicationUrl };
  };
  metadata(); // Invalid deployment configuration fails at startup, before binding a port.
  const allowed = new Set(origins.map((origin) => {
    // Tauri on macOS uses this explicit custom origin. Never allow the unrelated opaque
    // browser origin "null", including when a desktop origin was explicitly configured.
    if (origin === 'tauri://localhost') return origin;
    const url = new URL(origin);
    if (!['https:', 'http:'].includes(url.protocol) || url.origin !== origin) throw new Error('Each allowed origin must be an exact http(s) origin or tauri://localhost, without a path or trailing slash.');
    return origin;
  }));
  // Global fixed-size counters avoid retaining IP addresses or other client identifiers.
  const requests = rateLimit(requestsPerMinute, now);
  const writes = rateLimit(writesPerMinute, now);
  const announcements = rateLimit(announcementsPerMinute, now);
  const server = createServer({ maxHeaderSize: 8192, requestTimeout: 10_000, headersTimeout: 10_000 }, async (req, res) => {
    res.setHeader('Cache-Control', 'no-store');
    res.setHeader('X-Content-Type-Options', 'nosniff');
    res.setHeader('Referrer-Policy', 'no-referrer');
    res.setHeader('Strict-Transport-Security', 'max-age=31536000; includeSubDomains');
    res.setHeader('Vary', 'Origin');
    const send = (status, body) => {
      res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' });
      res.end(body === undefined ? undefined : JSON.stringify(body));
    };
    try {
      const origin = req.headers.origin;
      const url = new URL(req.url, 'http://registry.local');
      // As a tracker the list of public nodes carrying a community is public: any
      // app, wherever it is served, may read it, and so is the log of names.
      // Everything else keeps the operator's origins.
      const isLog = LOG_PATHS.includes(url.pathname);
      const trackerRead = req.method === 'GET' && (url.pathname === '/v2/nodes' || isLog);
      if (origin && !allowed.has(origin) && !trackerRead) return send(403, { error: 'origin-denied', message: 'This app origin is not allowed by the registry operator.' });
      if (origin) res.setHeader('Access-Control-Allow-Origin', allowed.has(origin) ? origin : '*');
      const version = url.pathname === '/v2/communities' ? 2 : 1;
      const isCommunities = ['/v1/communities', '/v2/communities'].includes(url.pathname);
      const isNodes = url.pathname === '/v2/nodes';
      const isMetadata = ['/v1/meta', '/release.json'].includes(url.pathname);
      if (!isCommunities && !isMetadata && !isNodes && !isLog) {
        return send(404, { error: 'not-found' });
      }
      const methods = isMetadata || isLog ? ['GET'] : ['GET', 'PUT'];
      if (req.method === 'OPTIONS') {
        if (req.headers['access-control-request-method'] && !methods.includes(req.headers['access-control-request-method'])) return send(405, { error: 'method-not-allowed' });
        res.setHeader('Access-Control-Allow-Methods', methods.join(', '));
        res.setHeader('Access-Control-Allow-Headers', 'Content-Type');
        return send(204);
      }
      if (!methods.includes(req.method)) {
        res.setHeader('Allow', [...methods, 'OPTIONS'].join(', '));
        return send(405, { error: 'method-not-allowed' });
      }
      // Reads and the two kinds of writes each have their own budget (see worker-runtime.mjs).
      const retry = req.method === 'PUT' ? (isNodes ? announcements() : writes()) : requests();
      if (retry) {
        res.setHeader('Retry-After', retry);
        return send(429, { error: 'rate-limited', message: 'The registry is busy. Retry after the indicated delay.' });
      }
      if (isLog) {
        const answer = answerLog(log, url.pathname, url.searchParams, seconds());
        return send(answer.status, answer.body);
      }
      const meta = metadata();
      if (isMetadata) return url.search ? send(400, { error: 'invalid', message: 'Metadata requests do not use query parameters.' }) : send(200, url.pathname === '/release.json' ? meta.release : meta);
      if (isNodes) {
        if (req.method === 'GET') {
          const query = [...url.searchParams.keys()];
          const community = query.length === 1 && query[0] === 'community' ? normalizeCommunityKey(url.searchParams.get('community')) : null;
          if (!community) return send(400, { error: 'invalid', message: 'Ask for the public nodes carrying one community key.' });
          return send(200, { publicNodes: discovered(community) });
        }
        if (!nodeAnnouncements) { req.resume(); return send(403, { error: 'announcements-disabled', message: 'This registry lists only operator-configured public nodes.' }); }
        if (url.search) { req.resume(); return send(400, { error: 'invalid', message: 'Announcements do not use query parameters.' }); }
        if (!/^application\/json(?:\s*;|$)/i.test(req.headers['content-type'] ?? '')) { req.resume(); return send(415, { error: 'content-type', message: 'Send application/json.' }); }
        if (Number(req.headers['content-length']) > MAX_ANNOUNCEMENT_BODY) { req.resume(); return send(413, { error: 'too-large', message: `Request body exceeds ${MAX_ANNOUNCEMENT_BODY} bytes.` }); }
        // The origin an announcement must name: the configured one, or (unconfigured) the
        // loopback address this connection reached, read from the socket, never from a header.
        const registries = ownOrigin ? [ownOrigin] : loopbackOrigins(req.socket.localAddress, req.socket.localPort);
        if (!registries.length) req.resume();
        const at = seconds();
        const verdict = admitAnnouncement(registries.length ? await readBody(req, MAX_ANNOUNCEMENT_BODY) : null, at,
          { registries, announcers: allowedAnnouncers, publicOnly: !allowPrivateRoutesForTests });
        if (!verdict.announcement) return send(verdict.status, { error: verdict.error, message: verdict.message });
        const kept = registry.announce(verdict.announcement, at);
        return kept ? send(200, kept) : send(403, ANNOUNCER_REFUSED);
      }
      if (req.method === 'GET') {
        const query = [...url.searchParams.keys()];
        const paginated = version === 2 && query.length === 2 && query.includes('prefix') && query.includes('after');
        const byKey = version === 2 && query.length === 1 && query[0] === 'community';
        if (!paginated && !byKey && (query.length !== 1 || !['name', 'prefix'].includes(query[0]))) return send(400, { error: 'invalid', message: 'Use one name, community or prefix query; v2 prefix may also include after.' });
        if (byKey) {
          const community = normalizeCommunityKey(url.searchParams.get('community'));
          if (!community) return send(400, { error: 'invalid', message: 'Invalid community key.' });
          const found = registry.findPublicationByKey(community, meta.release);
          if (found?.withheld) return send(404, WITHHELD);
          return found ? send(200, found) : send(404, { error: 'not-found', message: 'This community key has no public name.' });
        }
        if (url.searchParams.has('name')) {
          const name = normalizeName(url.searchParams.get('name'));
          if (!name) return send(400, { error: 'invalid', message: 'Use a public name of 1–32 ASCII letters, digits, dots, underscores or hyphens, starting with a letter or digit.' });
          const found = version === 2 ? registry.findPublication(name, meta.release) : registry.find(name);
          if (found?.withheld) return send(404, WITHHELD);
          return found ? send(200, found) : send(404, { error: 'not-found', message: 'This public name has not been reserved.' });
        }
        const rawPrefix = url.searchParams.get('prefix').trim();
        const prefix = rawPrefix === '' ? '' : normalizeName(rawPrefix);
        if (prefix === null) return send(400, { error: 'invalid', message: 'Invalid public name prefix.' });
        if (version === 2) {
          const after = url.searchParams.has('after') ? normalizeName(url.searchParams.get('after')) : '';
          if (after === null) return send(400, { error: 'invalid', message: 'Invalid pagination cursor.' });
          return send(200, registry.listPublications(prefix, after, meta.release));
        }
        return send(200, registry.list(prefix));
      }
      if (url.search) return send(400, { error: 'invalid', message: 'Reservation requests do not use query parameters.' });
      if (!/^application\/json(?:\s*;|$)/i.test(req.headers['content-type'] ?? '')) return send(415, { error: 'content-type', message: 'Send application/json.' });
      if (Number(req.headers['content-length']) > MAX_BODY) { req.resume(); return send(413, { error: 'too-large', message: 'Request body exceeds 2048 bytes.' }); }
      const claim = verifiedClaim(await readBody(req), version);
      if (!claim) return send(400, { error: 'invalid-proof', message: 'The community key must sign the exact public name, community ID and, for v2, protocol and index versions.' });
      const result = version === 2 ? registry.publish(claim, meta.release) : registry.reserve(claim);
      return send(result.status, result.body);
    } catch (e) {
      // Operational details and database paths stay out of public responses and access logs.
      if (!res.headersSent) send(e.status === 413 ? 413 : e.status === 400 ? 400 : 503, {
        error: e.status ? 'invalid-request' : 'unavailable',
        message: e.status ? e.message : 'The registry could not complete this request. Retry later.',
      });
    }
  });
  server.maxRequestsPerSocket = 100;
  server.setTimeout(15_000, (socket) => socket.destroy());
  server.once('close', () => registry.close());
  return server;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const host = process.env.HOI_REGISTRY_HOST || '127.0.0.1';
  const port = Number(process.env.HOI_REGISTRY_PORT || 7845);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('HOI_REGISTRY_PORT must be between 1 and 65535.');
  const announcements = process.env.HOI_REGISTRY_NODE_ANNOUNCEMENTS || 'on';
  if (!['on', 'off'].includes(announcements)) throw new Error('HOI_REGISTRY_NODE_ANNOUNCEMENTS must be on or off.');
  const announcers = validateAnnouncers(process.env.HOI_REGISTRY_ANNOUNCERS || null);
  const server = createRegistryServer({
    filename: process.env.HOI_REGISTRY_DB || 'hoi-registry.db',
    origins: (process.env.HOI_REGISTRY_ORIGINS || '').split(',').map((s) => s.trim()).filter(Boolean),
    requestsPerMinute: Number(process.env.HOI_REGISTRY_REQUESTS_PER_MINUTE || 600),
    writesPerMinute: Number(process.env.HOI_REGISTRY_WRITES_PER_MINUTE || 30),
    announcementsPerMinute: Number(process.env.HOI_REGISTRY_ANNOUNCEMENTS_PER_MINUTE || 120),
    publicNodes: JSON.parse(process.env.HOI_PUBLIC_NODES || '[]'),
    nodeAnnouncements: announcements === 'on',
    appUrl: process.env.HOI_APP_URL || 'https://hoi.sh/app/',
    registryUrl: process.env.HOI_REGISTRY_URL || null,
    announcers: announcers ? [...announcers] : null,
  });
  server.listen(port, host, () => {
    console.log(`hoi public name registry listening on ${host}:${port}`);
    if (announcements === 'on' && !process.env.HOI_REGISTRY_URL) {
      console.log('note: HOI_REGISTRY_URL is not set, so node announcements are taken only when signed for this machine\'s loopback address; set it to the URL carriers use');
    }
    if (announcers) console.log(`node announcements: allowlist mode, ${announcers.size} endpoint key${announcers.size === 1 ? '' : 's'} (HOI_REGISTRY_ANNOUNCERS)`);
  });
  const stop = () => server.close();
  process.once('SIGINT', stop);
  process.once('SIGTERM', stop);
}
