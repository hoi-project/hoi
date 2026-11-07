import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { generateKeyPairSync, sign } from 'node:crypto';
import { mkdtempSync, readdirSync, unlinkSync, rmdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { RegistryObject, fetchRegistry } from './worker-runtime.mjs';
import { publicationStatement, registrationStatement, announcementStatement } from './protocol.mjs';
import { ALLOWED_ANNOUNCER, runAllowlistConformance, runLogConformance, runOperatorConformance } from './conformance.mjs';
import { readFileSync } from 'node:fs';
import { loadRelease } from '../scripts/release-config.mjs';

const release = loadRelease();
const base = 'https://hoi.example/registry';
function proof(name, key = generateKeyPairSync('ed25519'), protocolVersion = release.protocolVersion, indexVersion = release.indexVersion) {
  const community = key.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
  const statement = protocolVersion === null ? registrationStatement(name, community) : publicationStatement(name, community, protocolVersion, indexVersion);
  return { name, community, ...(protocolVersion === null ? {} : { protocolVersion, indexVersion }), signature: sign(null, Buffer.from(statement), key.privateKey).toString('hex') };
}

// Exercise the same SQL and transaction boundaries with SQLite, without emulating SQL queries.
function storage(db) {
  db.exec('PRAGMA foreign_keys=ON');
  return {
    sql: { exec(query, ...args) { return db.prepare(query).all(...args); } },
    transactionSync(callback) {
      db.exec('BEGIN IMMEDIATE');
      try { const result = callback(); db.exec('COMMIT'); return result; }
      catch (error) { db.exec('ROLLBACK'); throw error; }
    },
  };
}
function start(t, env = {}, manifest = release, now = Date.now, filename = ':memory:') {
  const db = new DatabaseSync(filename);
  t.after(() => db.close());
  const instance = new RegistryObject({ storage: storage(db) }, { HOI_REGISTRY_REQUESTS_PER_MINUTE: '10000', HOI_REGISTRY_WRITES_PER_MINUTE: '10000', ...env }, manifest, now);
  return { db, instance, get: path => instance.fetch(new Request(base + path)), put: (claim, headers = {}, version = 2) => instance.fetch(new Request(`${base}/v${version}/communities`, { method: 'PUT', headers: { 'Content-Type': 'application/json', ...headers }, body: JSON.stringify(claim) })) };
}

test('standalone Worker serves registry APIs only, without website assets or relay routes', async () => {
  const requests = [];
  const names = [];
  const env = {
    REGISTRY: { idFromName(name) { names.push(name); return 'one-id'; }, get(id) { assert.equal(id, 'one-id'); return { fetch(request) { requests.push(request.url); return new Response('api'); } }; } },
    ASSETS: { fetch(request) { return new Response(new URL(request.url).pathname); } },
  };
  assert.equal((await fetchRegistry(new Request('https://hoi.example/registry'), env)).status, 404);
  for (const path of ['/v1/meta', '/release.json', '/v1/communities?name=hello', '/v2/communities?prefix=']) assert.equal(await (await fetchRegistry(new Request(base + path), env)).text(), 'api');
  assert.deepEqual(names, Array(4).fill('public-community-names'));
  assert.equal(requests.length, 4);
  for (const path of ['/', '/app/', '/registry/', '/registry/assets/tokens.css', '/relay/v1/rooms']) {
    const response = await fetchRegistry(new Request('https://hoi.example' + path), env);
    assert.equal(response.status, 404);
    assert.match(response.headers.get('content-type'), /application\/json/);
  }
  const missing = await fetchRegistry(new Request(base + '/v1/meta'), { ASSETS: env.ASSETS });
  assert.equal(missing.status, 503);
  assert.equal(missing.headers.get('Cache-Control'), 'no-store');
  assert.doesNotMatch(await missing.text(), /REGISTRY|binding|TypeError/);
});

test('signed competing names have one winner, idempotent retries and persistent public metadata only', async (t) => {
  const folder = mkdtempSync(join(tmpdir(), 'hoi-worker-registry-'));
  const filename = join(folder, 'names.db');
  const db = new DatabaseSync(filename);
  const env = { HOI_REGISTRY_WRITES_PER_MINUTE: '10000' };
  const instance = new RegistryObject({ storage: storage(db) }, env, release);
  const put = claim => instance.fetch(new Request(base + '/v2/communities', { method: 'PUT', headers: { 'Content-Type': 'application/json', Origin: 'https://hoi.example' }, body: JSON.stringify(claim) }));
  const claims = Array.from({ length: 12 }, () => proof('hiking'));
  const results = await Promise.all(claims.map(put));
  assert.equal(results.filter(result => result.status === 200).length, 1);
  assert.equal(results.filter(result => result.status === 409).length, 11);
  const winner = claims[results.findIndex(result => result.status === 200)];
  const entry = await (await put(winner)).json();
  assert.equal(entry.community, winner.community);
  assert.equal(entry.indexed, true);
  assert.deepEqual(db.prepare('PRAGMA table_info(communities)').all().map(row => row.name), ['name', 'community']);
  assert.deepEqual(db.prepare('PRAGMA table_info(community_index)').all().map(row => row.name), ['community', 'protocol_version', 'index_version']);
  db.close();
  const reopened = new DatabaseSync(filename);
  t.after(() => { reopened.close(); for (const name of readdirSync(folder)) unlinkSync(join(folder, name)); rmdirSync(folder); });
  const restarted = new RegistryObject({ storage: storage(reopened) }, env, release);
  assert.deepEqual(await (await restarted.fetch(new Request(base + '/v2/communities?name=hiking'))).json(), entry);
});

test('versioned publication preserves old reservations and filters before pagination', async (t) => {
  const server = start(t);
  const key = generateKeyPairSync('ed25519');
  assert.equal((await server.put(proof('legacy', key, null), {}, 1)).status, 200);
  assert.equal((await (await server.get('/v2/communities?name=legacy')).json()).indexed, false);
  assert.equal((await server.put(proof('legacy', key))).status, 200);
  assert.equal((await server.put(proof('renamed', key))).status, 409);
  assert.equal((await server.put(proof('legacy', key, 'future', release.indexVersion + 1))).status, 409);
  for (let i = 0; i < 52; i++) {
    assert.equal((await server.put(proof(`a-old-${i}`, undefined, 'old'))).status, 200);
    assert.equal((await server.put(proof(`b-${String(i).padStart(2, '0')}`))).status, 200);
  }
  const first = await (await server.get('/v2/communities?prefix=')).json();
  assert.equal(first.length, 50);
  assert.equal(first[0].name, 'b-00');
  assert.deepEqual((await (await server.get('/v2/communities?prefix=&after=b-49')).json()).map(row => row.name), ['b-50', 'b-51', 'legacy']);
  const changed = new RegistryObject({ storage: storage(server.db) }, {}, { ...release, protocolVersion: 'future', supportedProtocols: ['future'], indexVersion: release.indexVersion + 1 });
  assert.deepEqual(await (await changed.fetch(new Request(base + '/v2/communities?prefix='))).json(), []);
  assert.equal((await changed.fetch(new Request(base + '/v2/communities?name=legacy'))).status, 200);
});

test('CORS, proof verification, request bounds and unsupported methods cannot reserve names', async (t) => {
  const server = start(t, { HOI_REGISTRY_ORIGINS: 'tauri://localhost' });
  const valid = proof('private');
  for (const origin of ['https://other.example', 'null', 'tauri://other']) assert.equal((await server.put(valid, { Origin: origin })).status, 403);
  for (const bad of [{ ...valid, name: 'changed' }, { ...valid, indexVersion: 99 }, { ...valid, profile: 'private' }, { ...valid, signature: '0'.repeat(128) }]) assert.equal((await server.put(bad)).status, 400);
  assert.equal((await server.put({ ...valid, padding: 'x'.repeat(3000) })).status, 413);
  assert.equal((await server.get('/v2/communities?name=private')).status, 404);
  for (const query of ['', '?prefix=&after=x&after=y', '?name=private&name=other', '?prefix=%25']) assert.equal((await server.get('/v2/communities' + query)).status, 400);
  assert.equal((await server.instance.fetch(new Request(base + '/v2/communities', { method: 'DELETE' }))).status, 405);
  const preflight = await server.instance.fetch(new Request(base + '/v2/communities', { method: 'OPTIONS', headers: { Origin: 'tauri://localhost', 'Access-Control-Request-Method': 'PUT' } }));
  assert.equal(preflight.status, 204);
  assert.equal(preflight.headers.get('Access-Control-Allow-Origin'), 'tauri://localhost');
  const result = await server.put(valid, { Origin: 'https://hoi.example' });
  assert.equal(result.status, 200);
  assert.equal(result.headers.get('Access-Control-Allow-Origin'), 'https://hoi.example');
  assert.equal(result.headers.get('Cache-Control'), 'no-store');
  assert.equal(result.headers.get('Access-Control-Allow-Credentials'), null);
});

test('metadata follows the app origin and global rate limits retain no requester identifiers', async (t) => {
  let now = 0;
  const server = start(t, { HOI_REGISTRY_WRITES_PER_MINUTE: '1' }, release, () => now);
  const meta = await (await server.get('/v1/meta')).json();
  assert.equal(meta.appUrl, 'https://hoi.example/app/');
  assert.deepEqual(meta.publicNodes, []);
  assert.deepEqual(meta.release, release);
  assert.deepEqual(meta.registry, { apiVersion: 2, schemaVersion: 3, indexVersion: release.indexVersion });
  assert.equal((await server.put(proof('first'))).status, 200);
  const busy = await server.put(proof('second'));
  assert.equal(busy.status, 429);
  assert.equal(busy.headers.get('Retry-After'), '60');
  now = 60_001;
  assert.equal((await server.put(proof('second'))).status, 200);
  assert.deepEqual(server.db.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").all().map(row => row.name),
    ['communities', 'community_index', 'community_proofs', 'log_secret', 'name_log', 'public_nodes', 'refused_nodes', 'registry_schema', 'withheld']);
  assert.equal(server.db.prepare('SELECT version FROM registry_schema').get().version, 5);
});

test('storage failures roll back both tables and do not disclose operational details', async (t) => {
  const server = start(t);
  server.db.exec("CREATE TRIGGER fail_index BEFORE INSERT ON community_index BEGIN SELECT RAISE(ABORT, 'private database detail'); END");
  const result = await server.put(proof('atomic'));
  assert.equal(result.status, 503);
  assert.doesNotMatch(await result.text(), /private database detail|trigger|SQLite/);
  assert.equal((await server.get('/v1/communities?name=atomic')).status, 404);
  server.db.exec('UPDATE registry_schema SET version = 6');
  assert.throws(() => new RegistryObject({ storage: storage(server.db) }, {}, release), /Unsupported registry database schema/);
  assert.equal(server.db.prepare('SELECT version FROM registry_schema').get().version, 6);
});

test('schema-2 publication migration preserves reservations but requires the original owner proof', async (t) => {
  const db = new DatabaseSync(':memory:');
  t.after(() => db.close());
  const owner = generateKeyPairSync('ed25519');
  const original = proof('legacy-proof', owner);
  db.exec(`CREATE TABLE registry_schema (version INTEGER NOT NULL) STRICT;
    INSERT INTO registry_schema VALUES (2);
    CREATE TABLE communities (name TEXT PRIMARY KEY, community TEXT NOT NULL UNIQUE) STRICT;
    CREATE TABLE community_index (community TEXT PRIMARY KEY REFERENCES communities(community), protocol_version TEXT NOT NULL, index_version INTEGER NOT NULL) STRICT;`);
  db.prepare('INSERT INTO communities VALUES (?, ?)').run(original.name, original.community);
  db.prepare('INSERT INTO community_index VALUES (?, ?, ?)').run(original.community, original.protocolVersion, original.indexVersion);
  const instance = new RegistryObject({ storage: storage(db) }, {}, release);
  const get = path => instance.fetch(new Request(base + path));
  const put = claim => instance.fetch(new Request(base + '/v2/communities', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(claim) }));
  assert.equal(db.prepare('SELECT version FROM registry_schema').get().version, 5);
  const legacy = await (await get('/v2/communities?name=legacy-proof')).json();
  assert.equal(legacy.community, original.community);
  assert.equal(legacy.proof, null);
  assert.equal(legacy.indexed, false);
  assert.deepEqual(await (await get('/v2/communities?prefix=')).json(), []);
  assert.equal((await put(proof('legacy-proof'))).status, 409);
  assert.equal((await put(proof('legacy-proof', owner, 'different'))).status, 409);
  assert.equal((await put(original)).status, 200);
  const restored = await (await get('/v2/communities?name=legacy-proof')).json();
  assert.deepEqual(restored.proof, { version: 2, signature: original.signature, protocolVersion: original.protocolVersion, indexVersion: original.indexVersion });
  assert.equal(restored.indexed, true);
});

test('proof persistence failure rolls back the reservation and index as one transaction', async (t) => {
  const server = start(t);
  server.db.exec("CREATE TRIGGER fail_proof BEFORE INSERT ON community_proofs BEGIN SELECT RAISE(ABORT, 'private proof detail'); END");
  const result = await server.put(proof('no-partial-name'));
  assert.equal(result.status, 503);
  assert.doesNotMatch(await result.text(), /private proof detail|trigger|SQLite/);
  for (const table of ['communities', 'community_index', 'community_proofs']) {
    assert.equal(server.db.prepare(`SELECT COUNT(*) AS n FROM ${table}`).get().n, 0);
  }
});

const SECOND = 1_000_000;
const rtcRoute = (byte) => `rtc:${Buffer.alloc(32, byte).toString('base64url')}@203.0.113.${byte}:7844`;
function announcement(overrides = {}, key = generateKeyPairSync('ed25519'), now = SECOND) {
  const endpoint = key.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
  const at = overrides.rtc ?? 7;
  const routes = overrides.routes ?? [rtcRoute(at), `${endpoint}@203.0.113.${at}:7843`];
  const communities = overrides.communities ?? [];
  const issuedAt = overrides.issuedAt ?? now;
  // Without HOI_REGISTRY_URL the Worker answers to the origin requests reach it at.
  const registry = overrides.registry ?? new URL(base).origin;
  const statement = announcementStatement(registry, endpoint, issuedAt, routes, communities);
  const signature = sign(null, Buffer.from(statement), key.privateKey).toString('hex');
  return { body: { registry, endpoint, routes, communities, issuedAt, signature, ...(overrides.extra ?? {}) }, key, endpoint };
}
const announce = (server, body, headers = {}) => server.instance.fetch(new Request(`${base}/v2/nodes`, { method: 'PUT', headers: { 'Content-Type': 'application/json', ...headers }, body: JSON.stringify(body) }));

test('nodes announce their own routes; discovery lists configured routes, carriers of a community, then the rest; announcements expire', async (t) => {
  let now = SECOND * 1000;
  const configured = rtcRoute(1);
  const server = start(t, { HOI_PUBLIC_NODES: JSON.stringify([configured]) }, release, () => now);
  const garden = 'a'.repeat(64);
  const bikes = 'b'.repeat(64);
  const first = announcement({ communities: [garden] });
  const second = announcement({ rtc: 8, communities: [bikes, garden].sort(), issuedAt: SECOND + 1 });
  const saved = await announce(server, first.body);
  assert.equal(saved.status, 200);
  assert.deepEqual(await saved.json(), { endpoint: first.endpoint, routes: first.body.routes, communities: [garden], issuedAt: SECOND });
  assert.equal((await announce(server, second.body)).status, 200);
  const meta = await (await server.get('/v1/meta')).json();
  // Routes are taken one per node per round, newest announcement first, after the operator's.
  const interleaved = (a, b) => [a.routes[0], b.routes[0], a.routes[1], b.routes[1]];
  assert.deepEqual(meta.publicNodes, [configured, ...interleaved(second.body, first.body)], 'operator routes first, then one route per node per round');
  assert.deepEqual((await (await server.get(`/v2/nodes?community=${bikes}`)).json()).publicNodes, [configured, ...second.body.routes, ...first.body.routes]);
  assert.deepEqual((await (await server.get(`/v2/nodes?community=${garden}`)).json()).publicNodes, [configured, ...interleaved(second.body, first.body)]);
  assert.deepEqual((await (await server.get(`/v2/nodes?community=${'c'.repeat(64)}`)).json()).publicNodes, [configured, ...interleaved(second.body, first.body)]);
  for (const query of ['', '?community=abc', `?community=${garden}&community=${bikes}`, '?prefix=']) assert.equal((await server.get('/v2/nodes' + query)).status, 400);
  assert.deepEqual(server.db.prepare('PRAGMA table_info(public_nodes)').all().map(row => row.name), ['endpoint', 'routes', 'communities', 'issued_at', 'first_seen']);

  // A newer announcement replaces the node's routes; a replayed older one changes nothing.
  const moved = announcement({ routes: [rtcRoute(9)], communities: [garden], issuedAt: SECOND + 30 }, first.key);
  assert.deepEqual((await (await announce(server, moved.body)).json()).routes, [rtcRoute(9)]);
  const replayed = await (await announce(server, first.body)).json();
  assert.deepEqual(replayed, { endpoint: first.endpoint, routes: [rtcRoute(9)], communities: [garden], issuedAt: SECOND + 30 });
  assert.equal(server.db.prepare('SELECT COUNT(*) AS n FROM public_nodes').get().n, 2);

  // Half an hour of silence drops a node from discovery; a stale announcement is refused.
  now += 31 * 60 * 1000;
  assert.deepEqual((await (await server.get('/v1/meta')).json()).publicNodes, [configured]);
  assert.equal((await announce(server, first.body)).status, 400);
  const fresh = announcement({ communities: [garden] }, first.key, Math.floor(now / 1000));
  assert.equal((await announce(server, fresh.body)).status, 200);
  assert.deepEqual((await (await server.get('/v1/meta')).json()).publicNodes, [configured, ...fresh.body.routes]);
  assert.equal(server.db.prepare('SELECT COUNT(*) AS n FROM public_nodes').get().n, 1, 'expired rows are pruned on write');
});

test('an announcement is bound to the endpoint key that signs it, dated within ten minutes, bounded and optional', async (t) => {
  const now = SECOND * 1000;
  const server = start(t, {}, release, () => now);
  const valid = announcement();
  const other = announcement();
  const unsorted = ['b'.repeat(64), 'a'.repeat(64)];
  for (const bad of [
    { ...valid.body, signature: other.body.signature },
    { ...valid.body, routes: [...valid.body.routes].reverse() },
    { ...valid.body, routes: [`${other.endpoint}@203.0.113.7:7843`] },
    { ...valid.body, routes: [] },
    { ...valid.body, routes: Array.from({ length: 9 }, (_, i) => rtcRoute(10 + i)) },
    { ...valid.body, routes: [rtcRoute(7), rtcRoute(7)] },
    { ...valid.body, communities: unsorted },
    { ...valid.body, communities: ['a'.repeat(64), 'a'.repeat(64)] },
    { ...valid.body, communities: ['not a key'] },
    { ...valid.body, issuedAt: SECOND + 601 },
    { ...valid.body, issuedAt: SECOND - 601 },
    { ...valid.body, issuedAt: '1000000' },
    { ...valid.body, name: 'unexpected' },
    { ...valid.body, endpoint: '01' + '00'.repeat(31), signature: '01' + '00'.repeat(63) },
    announcement({ issuedAt: SECOND + 601 }).body,
    announcement({ routes: [`${other.endpoint}@203.0.113.7:7843`] }).body,
    announcement({ communities: unsorted }).body,
  ]) assert.equal((await announce(server, bad)).status, 400, JSON.stringify(bad).slice(0, 120));
  assert.deepEqual((await (await server.get('/v1/meta')).json()).publicNodes, []);
  const full = announcement({ communities: Array.from({ length: 64 }, (_, i) => i.toString(16).padStart(64, '0')).sort() });
  assert.ok(JSON.stringify(full.body).length > 2048, 'a full announcement is bigger than a name claim');
  assert.equal((await announce(server, full.body)).status, 200);
  assert.equal((await announce(server, { ...full.body, padding: 'x'.repeat(9000) })).status, 413);
  assert.equal((await server.instance.fetch(new Request(`${base}/v2/nodes`, { method: 'PUT', body: '{}' }))).status, 415);
  assert.equal((await server.instance.fetch(new Request(`${base}/v2/nodes`, { method: 'DELETE' }))).status, 405);
  const preflight = await server.instance.fetch(new Request(`${base}/v2/nodes`, { method: 'OPTIONS', headers: { 'Access-Control-Request-Method': 'PUT' } }));
  assert.equal(preflight.headers.get('Access-Control-Allow-Methods'), 'GET, PUT');

  // At most 64 live nodes are kept and at most 16 routes are ever listed. When the list is
  // full, the nodes that were there first stay and newcomers wait: a flood of fresh keys cannot
  // push established carriers out, and the newcomer is told it was not retained.
  const incumbent = full;
  for (let i = 0; i < 70; i++) {
    const late = await announce(server, announcement({ routes: [rtcRoute(100 + i)], issuedAt: SECOND + 1 + i }).body);
    assert.equal(late.status, 200);
    assert.equal((await late.json()).retained, i < 63 ? undefined : false, `announcement ${i}`);
  }
  assert.equal(server.db.prepare('SELECT COUNT(*) AS n FROM public_nodes').get().n, 64);
  assert.equal(server.db.prepare('SELECT COUNT(*) AS n FROM public_nodes WHERE endpoint = ?').get(incumbent.endpoint).n, 1, 'the first node is still listed');
  const listed = (await (await server.get('/v1/meta')).json()).publicNodes;
  assert.equal(listed.length, 16);
  assert.equal(listed[0], rtcRoute(162), 'among the retained nodes the newest announcement comes first');
  // Announced routes must be reachable from other networks; the operator's own configuration may name a loopback for development.
  for (const inside of ['127.0.0.1', '10.0.0.4', '172.16.0.4', '192.168.1.4', '169.254.1.4', '100.64.0.4', '[fe80::1]', '[fd00::1]', '[::1]', '[::ffff:192.168.0.1]']) {
    assert.equal((await announce(server, announcement({ routes: [`rtc:${Buffer.alloc(32, 5).toString('base64url')}@${inside}:7844`] }).body)).status, 400, inside);
  }
  assert.equal((await announce(server, announcement({ routes: [`rtc:${Buffer.alloc(32, 5).toString('base64url')}@[2606:4700::6810:84e5]:7844`] }).body)).status, 200);

  // Announcements have their own budget and can be switched off by the operator.
  const limited = start(t, { HOI_REGISTRY_ANNOUNCEMENTS_PER_MINUTE: '1' }, release, () => now);
  assert.equal((await announce(limited, announcement().body)).status, 200);
  assert.equal((await announce(limited, announcement().body)).status, 429);
  assert.equal((await limited.put(proof('still-writable'))).status, 200, 'name claims keep their own budget');
  const off = new RegistryObject({ storage: storage(server.db) }, { HOI_REGISTRY_NODE_ANNOUNCEMENTS: 'off', HOI_PUBLIC_NODES: JSON.stringify([rtcRoute(1)]) }, release, () => now);
  assert.equal((await off.fetch(new Request(`${base}/v2/nodes`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(announcement().body) }))).status, 403);
  assert.deepEqual((await (await off.fetch(new Request(`${base}/v1/meta`))).json()).publicNodes, [rtcRoute(1)], 'earlier announcements are not listed either');
  assert.throws(() => new RegistryObject({ storage: storage(server.db) }, { HOI_REGISTRY_NODE_ANNOUNCEMENTS: 'maybe' }, release), /on or off/);
});

test('a carrier looks a community up by its key and gets the same compatible listing as by name', async (t) => {
  const server = start(t);
  const claim = proof('hiking');
  assert.equal((await server.put(claim)).status, 200);
  const byName = await (await server.get('/v2/communities?name=hiking')).json();
  assert.deepEqual(await (await server.get(`/v2/communities?community=${claim.community}`)).json(), byName);
  assert.equal(byName.indexed, true);
  assert.equal((await server.get(`/v2/communities?community=${'f'.repeat(64)}`)).status, 404);
  for (const query of ['?community=hiking', `?community=${claim.community}&name=hiking`]) assert.equal((await server.get('/v2/communities' + query)).status, 400);
  assert.equal((await server.get(`/v1/communities?community=${claim.community}`)).status, 400, 'v1 has no key lookup');
});

test('Rust and Node share the exact node announcement statement and signature', async (t) => {
  const vector = JSON.parse(readFileSync(new URL('../core/vectors/node-announcement.json', import.meta.url), 'utf8'));
  assert.equal(announcementStatement(vector.registry, vector.endpoint, vector.issuedAt, vector.routes, vector.communities), vector.statement);
  assert.match(vector.statement, /^hoi public node v2\nhttps:\/\/registry\.example\n/);
  // The vector is signed for https://registry.example: a Worker configured with that URL takes
  // it, whatever host the request came in on; one answering to its own host does not.
  const server = start(t, { HOI_REGISTRY_URL: 'https://registry.example/registry' }, release, () => vector.issuedAt * 1000);
  const { seed, statement, ...body } = vector;
  assert.ok(seed && statement);
  const result = await announce(server, body);
  assert.equal(result.status, 200);
  assert.deepEqual((await (await server.get(`/v2/nodes?community=${vector.communities[0]}`)).json()).publicNodes, vector.routes);
  const elsewhere = start(t, {}, release, () => vector.issuedAt * 1000);
  const refused = await announce(elsewhere, body);
  assert.equal(refused.status, 400);
  assert.equal((await refused.json()).error, 'wrong-registry');
});

test('an announcement signed for another registry, or in the old unnamed form, is refused', async (t) => {
  const server = start(t, {}, release, () => SECOND * 1000);
  for (const registry of ['https://other-registry.example', 'https://hoi.example:8443', 'http://hoi.example']) {
    const refused = await announce(server, announcement({ registry }).body);
    assert.equal(refused.status, 400, registry);
    assert.equal((await refused.json()).error, 'wrong-registry');
  }
  const valid = announcement();
  const { registry: _origin, ...unnamed } = valid.body;
  assert.equal((await announce(server, unnamed)).status, 400, 'version 1 bodies name no registry');
  assert.equal((await announce(server, valid.body)).status, 200);
  assert.throws(() => start(t, { HOI_REGISTRY_URL: 'http://registry.example' }), /public https/);
});

const request = (instance) => (url, init) => instance().fetch(new Request(url, init));

test('allowlist mode in the Worker lists only the allowed endpoint keys', async (t) => {
  let now = SECOND * 1000;
  const server = start(t, { HOI_REGISTRY_ANNOUNCERS: ALLOWED_ANNOUNCER.endpoint }, release, () => now);
  await runAllowlistConformance(base, { clock: () => Math.floor(now / 1000), request: request(() => server.instance) });
  assert.throws(() => start(t, { HOI_REGISTRY_ANNOUNCERS: 'not-a-key' }), /endpoint keys/);
});

test('the Worker takes its operator lists from configuration and never accepts private routes', async (t) => {
  let now = SECOND * 1000;
  const db = new DatabaseSync(':memory:');
  t.after(() => db.close());
  // A redeploy with new configuration makes a new object over the same database.
  let instance;
  const deploy = (env = {}) => { instance = new RegistryObject({ storage: storage(db) }, { HOI_REGISTRY_WRITES_PER_MINUTE: '10000', ...env }, release, () => now); };
  deploy();
  await runOperatorConformance(base, {
    hide: async (value) => deploy({ HOI_REGISTRY_WITHHELD: value }),
    unhide: async () => deploy(),
    dropNode: async (endpoint) => deploy({ HOI_REGISTRY_REFUSED_NODES: endpoint }),
    clock: () => Math.floor(now / 1000),
    request: request(() => instance),
  });
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM refused_nodes').get().n, 1);
  deploy();
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM refused_nodes').get().n, 0, 'configuration is the whole list');
  for (const env of [{ HOI_REGISTRY_WITHHELD: 'Not A Name!' }, { HOI_REGISTRY_REFUSED_NODES: 'garden' }]) assert.throws(() => deploy(env), /HOI_REGISTRY_WITHHELD/);
  for (const value of ['on', '1', '']) {
    assert.throws(() => deploy({ HOI_REGISTRY_ALLOW_PRIVATE_ROUTES: value }), /never accepts private or loopback/);
  }
  const loopback = announcement({ routes: [`rtc:${Buffer.alloc(32, 5).toString('base64url')}@127.0.0.1:7844`] });
  deploy();
  assert.equal((await instance.fetch(new Request(`${base}/v2/nodes`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(loopback.body) }))).status, 400);
});

test('the Worker keeps the public log of names, under one key across redeploys', async (t) => {
  const db = new DatabaseSync(':memory:');
  t.after(() => db.close());
  let instance;
  const deploy = (env = {}) => { instance = new RegistryObject({ storage: storage(db) }, { HOI_REGISTRY_WRITES_PER_MINUTE: '10000', ...env }, release); };
  deploy();
  const key = (await (await instance.fetch(new Request(`${base}/v2/log/head`))).json()).logKey;
  await runLogConformance(base, { request: request(() => instance), hide: async (value) => deploy({ HOI_REGISTRY_WITHHELD: value }) });
  deploy();
  assert.equal((await (await instance.fetch(new Request(`${base}/v2/log/head`))).json()).logKey, key);
});
