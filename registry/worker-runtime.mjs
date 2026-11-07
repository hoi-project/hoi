import { normalizeName, normalizeCommunityKey, verifiedClaim, admitAnnouncement, validatePublicNodes, validateAppUrl, selectPublicNodes, registryOrigin,
  validateAnnouncers, ANNOUNCER_REFUSED, WITHHELD } from './protocol.mjs';
import { createRegistryStore, DATABASE_VERSION, SCHEMA, upgradeSchema, withheldKey } from './store.mjs';
import { validateRelease } from '../scripts/release-schema.mjs';
import { LOG_PATHS, answerLog, nameLog } from './log.mjs';

const MAX_BODY = 2048;
/** A node announcement lists up to 64 community keys and 8 routes. */
const MAX_ANNOUNCEMENT_BODY = 8192;
const API_PATHS = new Set(['/registry/v1/meta', '/registry/release.json', '/registry/v1/communities', '/registry/v2/communities', '/registry/v2/nodes',
  ...LOG_PATHS.map((path) => `/registry${path}`)]);
const HEADERS = { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff', 'Referrer-Policy': 'no-referrer', 'Strict-Transport-Security': 'max-age=31536000; includeSubDomains', Vary: 'Origin' };
const unavailable = () => new Response(JSON.stringify({ error: 'unavailable', message: 'Public names are temporarily unavailable. Please try again later.' }), { status: 503, headers: HEADERS });

function rateLimit(capacity, now) {
  let started = now();
  let used = 0;
  return () => {
    const time = now();
    if (time - started >= 60_000) { started = time; used = 0; }
    return ++used <= capacity ? 0 : Math.max(1, Math.ceil((60_000 - (time - started)) / 1000));
  };
}

function allowedOrigins(configured) {
  return new Set((configured || '').split(',').map(value => value.trim()).filter(Boolean).map(origin => {
    if (origin === 'tauri://localhost') return origin;
    const url = new URL(origin);
    if (!['https:', 'http:'].includes(url.protocol) || url.origin !== origin) throw new Error('Invalid allowed registry origin.');
    return origin;
  }));
}

async function readClaim(request, max = MAX_BODY) {
  const tooLarge = () => Object.assign(new Error(`Request body exceeds ${max} bytes.`), { status: 413 });
  if (Number(request.headers.get('content-length')) > max) throw tooLarge();
  if (!request.body) throw Object.assign(new Error('Expected a JSON registration proof.'), { status: 400 });
  const reader = request.body.getReader();
  const chunks = [];
  let length = 0;
  // A body that trickles in holds a request open; ten seconds is plenty for a few kilobytes.
  let timer;
  const deadline = new Promise((_, reject) => { timer = setTimeout(() => reject(Object.assign(new Error('The request took too long.'), { status: 408 })), 10_000); });
  try {
    while (true) {
      const { done, value } = await Promise.race([reader.read(), deadline]);
      if (done) break;
      length += value.byteLength;
      if (length > max) {
        await reader.cancel();
        throw tooLarge();
      }
      chunks.push(value);
    }
  } catch (error) {
    void reader.cancel().catch(() => {});
    throw error;
  } finally { clearTimeout(timer); reader.releaseLock(); }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  try { return JSON.parse(new TextDecoder().decode(bytes)); }
  catch { throw Object.assign(new Error('Expected a JSON registration proof.'), { status: 400 }); }
}

/** One durable object owns the entire public namespace, including both uniqueness constraints. */
export class RegistryObject {
  constructor(ctx, env, manifest, now = Date.now) {
    this.release = validateRelease(manifest);
    this.nodes = validatePublicNodes(JSON.parse(env.HOI_PUBLIC_NODES || '[]'));
    this.appUrl = env.HOI_APP_URL ? validateAppUrl(env.HOI_APP_URL) : null;
    this.origins = allowedOrigins(env.HOI_REGISTRY_ORIGINS);
    const announcements = env.HOI_REGISTRY_NODE_ANNOUNCEMENTS || 'on';
    if (!['on', 'off'].includes(announcements)) throw new Error('HOI_REGISTRY_NODE_ANNOUNCEMENTS must be on or off.');
    this.acceptAnnouncements = announcements === 'on';
    // Without HOI_REGISTRY_URL, announcements must name the origin requests arrive at, which
    // Cloudflare routes by host name: another registry's origin can never reach this Worker.
    this.registryOrigin = env.HOI_REGISTRY_URL ? registryOrigin(env.HOI_REGISTRY_URL) : null;
    this.announcers = validateAnnouncers(env.HOI_REGISTRY_ANNOUNCERS || null);
    // Accepting loopback and private routes is a test option of the Node service only: a
    // Worker is reachable from everywhere, and its announced routes are dialed by every visitor.
    if (env.HOI_REGISTRY_ALLOW_PRIVATE_ROUTES !== undefined) throw new Error('The Worker never accepts private or loopback announced routes; HOI_REGISTRY_ALLOW_PRIVATE_ROUTES is a mistake here.');
    // The operator's lists come from configuration: no shell reaches a Durable Object's database.
    const listed = (value, valid) => (value || '').split(',').map(item => item.trim()).filter(Boolean).map(item => {
      const key = valid(item);
      if (!key) throw new Error('HOI_REGISTRY_WITHHELD lists public names or community keys; HOI_REGISTRY_REFUSED_NODES lists endpoint keys.');
      return key;
    });
    const withheld = listed(env.HOI_REGISTRY_WITHHELD, withheldKey);
    const refusedNodes = listed(env.HOI_REGISTRY_REFUSED_NODES, normalizeCommunityKey);
    const requestLimit = Number(env.HOI_REGISTRY_REQUESTS_PER_MINUTE || 600);
    const writeLimit = Number(env.HOI_REGISTRY_WRITES_PER_MINUTE || 30);
    const announcementLimit = Number(env.HOI_REGISTRY_ANNOUNCEMENTS_PER_MINUTE || 120);
    if (![requestLimit, writeLimit, announcementLimit].every(value => Number.isInteger(value) && value > 0)) throw new Error('Invalid registry rate limits.');
    this.now = now;
    this.requests = rateLimit(requestLimit, now);
    this.writes = rateLimit(writeLimit, now);
    this.announcements = rateLimit(announcementLimit, now);
    const sql = ctx.storage.sql;
    // Durable Object SQLite does not support PRAGMA user_version or explicit BEGIN statements.
    ctx.storage.transactionSync(() => {
      sql.exec('CREATE TABLE IF NOT EXISTS registry_schema (version INTEGER NOT NULL) STRICT');
      const version = [...sql.exec('SELECT version FROM registry_schema')][0]?.version;
      if (version !== undefined && ![2, 3, 4, DATABASE_VERSION].includes(version)) throw new Error('Unsupported registry database schema.');
      for (const statement of SCHEMA) sql.exec(statement);
      if (version === undefined) sql.exec(`INSERT INTO registry_schema(version) VALUES (${DATABASE_VERSION})`);
      else if (version !== DATABASE_VERSION) sql.exec(`UPDATE registry_schema SET version = ${DATABASE_VERSION}`);
    });
    const db = { prepare(query) {
      return {
        get: (...args) => [...sql.exec(query, ...args)][0],
        all: (...args) => [...sql.exec(query, ...args)],
        run: (...args) => { sql.exec(query, ...args); },
      };
    } };
    ctx.storage.transactionSync(() => upgradeSchema(db));
    this.registry = createRegistryStore(db, callback => ctx.storage.transactionSync(callback));
    this.registry.syncOperatorLists(withheld, refusedNodes, Math.floor(now() / 1000));
    this.log = nameLog(this.registry);
  }

  /** Operator-configured routes first, then nodes that announced themselves recently; nobody is
   * named as a carrier of a withheld community. */
  publicNodes(community = null) {
    const seconds = Math.floor(this.now() / 1000);
    const live = this.acceptAnnouncements ? this.registry.liveNodes(seconds).filter(node => !this.announcers || this.announcers.has(node.endpoint)) : [];
    return selectPublicNodes(this.nodes, live, community && !this.registry.isWithheld(community) ? community : null);
  }

  async fetch(request) {
    const headers = new Headers(HEADERS);
    const send = (status, body) => new Response(body === undefined ? null : JSON.stringify(body), { status, headers });
    try {
      const url = new URL(request.url);
      const origin = request.headers.get('origin');
      // As a tracker the public nodes carrying a community are public to any app, and
      // so is the log of names.
      const logPath = LOG_PATHS.find((path) => url.pathname === `/registry${path}`);
      const trackerRead = request.method === 'GET' && (url.pathname === '/registry/v2/nodes' || !!logPath);
      const known = !origin || origin === url.origin || this.origins.has(origin);
      if (!known && !trackerRead) return send(403, { error: 'origin-denied', message: 'Public names are unavailable from this app.' });
      if (origin) headers.set('Access-Control-Allow-Origin', known ? origin : '*');
      if (!API_PATHS.has(url.pathname)) return send(404, { error: 'not-found' });
      const metadata = ['/registry/v1/meta', '/registry/release.json'].includes(url.pathname);
      const nodesApi = url.pathname === '/registry/v2/nodes';
      const methods = metadata || logPath ? ['GET'] : ['GET', 'PUT'];
      if (request.method === 'OPTIONS') {
        const method = request.headers.get('access-control-request-method');
        if (method && !methods.includes(method)) return send(405, { error: 'method-not-allowed' });
        headers.set('Access-Control-Allow-Methods', methods.join(', '));
        headers.set('Access-Control-Allow-Headers', 'Content-Type');
        return send(204);
      }
      if (!methods.includes(request.method)) {
        headers.set('Allow', [...methods, 'OPTIONS'].join(', '));
        return send(405, { error: 'method-not-allowed' });
      }
      // Reads and the two kinds of writes each have their own budget, so a flood of reads
      // cannot stop carriers from announcing themselves or names from being claimed.
      const retry = request.method === 'PUT' ? (nodesApi ? this.announcements() : this.writes()) : this.requests();
      if (retry) {
        headers.set('Retry-After', String(retry));
        return send(429, { error: 'rate-limited', message: 'The directory is busy. Please try again in a moment.' });
      }
      if (logPath) {
        const answer = answerLog(this.log, logPath, url.searchParams, Math.floor(this.now() / 1000));
        return send(answer.status, answer.body);
      }
      const release = this.release;
      if (metadata) {
        if (url.search) return send(400, { error: 'invalid', message: 'Metadata requests do not use query parameters.' });
        return send(200, url.pathname === '/registry/release.json' ? release : {
          release, registry: { apiVersion: 2, schemaVersion: 3, indexVersion: release.indexVersion },
          publicNodes: this.publicNodes(), appUrl: this.appUrl || new URL('/app/', url).href,
        });
      }
      if (nodesApi) {
        if (request.method === 'GET') {
          const keys = [...url.searchParams.keys()];
          const community = keys.length === 1 && keys[0] === 'community' ? normalizeCommunityKey(url.searchParams.get('community')) : null;
          if (!community) return send(400, { error: 'invalid', message: 'Ask for the public nodes carrying one community key.' });
          return send(200, { publicNodes: this.publicNodes(community) });
        }
        if (!this.acceptAnnouncements) return send(403, { error: 'announcements-disabled', message: 'This registry lists only operator-configured public nodes.' });
        if (url.search) return send(400, { error: 'invalid', message: 'Announcements do not use query parameters.' });
        if (!/^application\/json(?:\s*;|$)/i.test(request.headers.get('content-type') || '')) return send(415, { error: 'content-type', message: 'Send application/json.' });
        const seconds = Math.floor(this.now() / 1000);
        const registries = [this.registryOrigin ?? url.origin];
        const verdict = admitAnnouncement(await readClaim(request, MAX_ANNOUNCEMENT_BODY), seconds, { registries, announcers: this.announcers });
        if (!verdict.announcement) return send(verdict.status, { error: verdict.error, message: verdict.message });
        const kept = this.registry.announce(verdict.announcement, seconds);
        return kept ? send(200, kept) : send(403, ANNOUNCER_REFUSED);
      }
      const version = url.pathname === '/registry/v2/communities' ? 2 : 1;
      if (request.method === 'GET') {
        const keys = [...url.searchParams.keys()];
        const paginated = version === 2 && keys.length === 2 && keys.includes('prefix') && keys.includes('after');
        const byKey = version === 2 && keys.length === 1 && keys[0] === 'community';
        if (!paginated && !byKey && (keys.length !== 1 || !['name', 'prefix'].includes(keys[0]))) return send(400, { error: 'invalid', message: 'Use one name, community or prefix query; v2 prefix may also include after.' });
        if (byKey) {
          const community = normalizeCommunityKey(url.searchParams.get('community'));
          if (!community) return send(400, { error: 'invalid', message: 'Invalid community key.' });
          const found = this.registry.findPublicationByKey(community, release);
          if (found?.withheld) return send(404, WITHHELD);
          return found ? send(200, found) : send(404, { error: 'not-found', message: 'This community key has no public name.' });
        }
        if (url.searchParams.has('name')) {
          const name = normalizeName(url.searchParams.get('name'));
          if (!name) return send(400, { error: 'invalid', message: 'Invalid public name.' });
          const found = version === 2 ? this.registry.findPublication(name, release) : this.registry.find(name);
          if (found?.withheld) return send(404, WITHHELD);
          return found ? send(200, found) : send(404, { error: 'not-found', message: 'This public name has not been reserved.' });
        }
        const rawPrefix = url.searchParams.get('prefix').trim();
        const prefix = rawPrefix === '' ? '' : normalizeName(rawPrefix);
        if (prefix === null) return send(400, { error: 'invalid', message: 'Invalid public name prefix.' });
        if (version === 1) return send(200, this.registry.list(prefix));
        const after = url.searchParams.has('after') ? normalizeName(url.searchParams.get('after')) : '';
        if (after === null) return send(400, { error: 'invalid', message: 'Invalid pagination cursor.' });
        return send(200, this.registry.listPublications(prefix, after, release));
      }
      if (url.search) return send(400, { error: 'invalid', message: 'Reservation requests do not use query parameters.' });
      if (!/^application\/json(?:\s*;|$)/i.test(request.headers.get('content-type') || '')) return send(415, { error: 'content-type', message: 'Send application/json.' });
      const claim = verifiedClaim(await readClaim(request), version);
      if (!claim) return send(400, { error: 'invalid-proof', message: 'The community publication proof could not be verified.' });
      const result = version === 2 ? this.registry.publish(claim, release) : this.registry.reserve(claim);
      return send(result.status, result.body);
    } catch (error) {
      if ([400, 408, 413].includes(error.status)) return send(error.status, { error: error.status === 408 ? 'request-timeout' : 'invalid-request', message: error.message });
      return send(503, { error: 'unavailable', message: 'Public names are temporarily unavailable. Please try again later.' });
    }
  }
}

export async function fetchRegistry(request, env) {
  const url = new URL(request.url);
  if (API_PATHS.has(url.pathname)) {
    try {
      const id = env.REGISTRY.idFromName('public-community-names');
      return await env.REGISTRY.get(id).fetch(request);
    } catch { return unavailable(); }
  }
  return new Response(JSON.stringify({ error: 'not-found' }), { status: 404, headers: HEADERS });
}
