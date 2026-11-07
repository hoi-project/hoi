import test from 'node:test';
import assert from 'node:assert/strict';
import { generateKeyPairSync, sign } from 'node:crypto';
import { mkdtemp, readFile, readdir, unlink, rmdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { createRegistryServer, openRegistry, normalizeName, registrationStatement, publicationStatement, announcementStatement, validatePublicNodes } from './server.mjs';
import { loadRelease } from '../scripts/release-config.mjs';
import { ALLOWED_ANNOUNCER, runAllowlistConformance, runConformance, runLogConformance, runOperatorConformance } from './conformance.mjs';
import { loopbackOrigins, registryOrigin, validateAnnouncers } from './protocol.mjs';
import { runAdmin } from './admin.mjs';

/** A node's announcement to the registry at `registry` (an origin), signed for it. */
function nodeAnnouncement(registry, communities, issuedAt, key = generateKeyPairSync('ed25519'), routes = null) {
  const endpoint = key.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
  const own = routes ?? [`${endpoint}@203.0.113.9:7843`];
  return { registry, endpoint, routes: own, communities, issuedAt, signature: sign(null, Buffer.from(announcementStatement(registry, endpoint, issuedAt, own, communities)), key.privateKey).toString('hex') };
}

function proof(name, key = generateKeyPairSync('ed25519')) {
  const community = key.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
  const canonical = normalizeName(name);
  return { name: canonical, community, signature: sign(null, Buffer.from(registrationStatement(canonical, community)), key.privateKey).toString('hex') };
}

function publication(name, key = generateKeyPairSync('ed25519'), protocolVersion = '0.3', indexVersion = 1) {
  const claim = proof(name, key);
  return { ...claim, protocolVersion, indexVersion, signature: sign(null,
    Buffer.from(publicationStatement(claim.name, claim.community, protocolVersion, indexVersion)), key.privateKey).toString('hex') };
}

const savedProof = (claim) => claim.protocolVersion
  ? { version: 2, signature: claim.signature, protocolVersion: claim.protocolVersion, indexVersion: claim.indexVersion }
  : { version: 1, signature: claim.signature };

async function start(filename = ':memory:', options = {}) {
  const server = createRegistryServer({ filename, requestsPerMinute: 10_000, writesPerMinute: 10_000, ...options });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const url = `http://127.0.0.1:${server.address().port}/v1/communities`;
  return {
    url,
    root: url.replace('/v1/communities', ''),
    v2: url.replace('/v1/', '/v2/'),
    put: (body, headers = {}) => fetch(url, { method: 'PUT', headers: { 'Content-Type': 'application/json', ...headers }, body: JSON.stringify(body) }),
    publish: (body) => fetch(url.replace('/v1/', '/v2/'), { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }),
    close: () => server.listening ? new Promise((resolve, reject) => server.close((e) => e ? reject(e) : resolve())) : Promise.resolve(),
  };
}

test('concurrent reservations across two listeners choose one community and survive a restart', async (t) => {
  const folder = await mkdtemp(join(tmpdir(), 'hoi-registry-'));
  const listeners = [];
  t.after(async () => {
    for (const listener of listeners) await listener.close();
    // Only unlink the flat SQLite files in this test's newly created temporary directory.
    for (const filename of await readdir(folder)) await unlink(join(folder, filename));
    await rmdir(folder);
  });
  const filename = join(folder, 'names.db');
  const a = await start(filename);
  listeners.push(a);
  const b = await start(filename);
  listeners.push(b);
  const claims = Array.from({ length: 16 }, () => proof(' HIKING '));
  const results = await Promise.all(claims.map((claim, i) => (i % 2 ? a : b).put(claim)));
  const winnerIndex = results.findIndex((r) => r.status === 200);
  assert.ok(winnerIndex >= 0);
  assert.equal(results.filter((r) => r.status === 200).length, 1);
  assert.equal(results.filter((r) => r.status === 409).length, 15);
  const winner = claims[winnerIndex];
  const mapping = { name: 'hiking', community: winner.community, proof: savedProof(winner) };
  assert.deepEqual(await (await a.put(winner)).json(), mapping);
  assert.equal((await a.put(winner)).status, 200, 'retry after an uncertain response is idempotent');
  await a.close();
  await b.close();

  const restarted = await start(filename);
  listeners.push(restarted);
  assert.deepEqual(await (await fetch(`${restarted.url}?name=Hiking`)).json(), mapping);
  assert.equal((await restarted.put(proof('hiking'))).status, 409);
  assert.deepEqual(await (await fetch(`${restarted.url}?prefix=hi`)).json(), [mapping]);
  const db = new DatabaseSync(filename);
  assert.deepEqual(db.prepare('PRAGMA table_info(communities)').all().map((column) => column.name), ['name', 'community']);
  assert.deepEqual(db.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").all().map((row) => row.name),
    ['communities', 'community_index', 'community_proofs', 'log_secret', 'name_log', 'public_nodes', 'refused_nodes', 'withheld']);
  assert.deepEqual(db.prepare('PRAGMA table_info(withheld)').all().map((column) => column.name), ['key', 'withheld_at'], 'the operator\'s list holds no note and no notifier');
  assert.deepEqual(db.prepare('PRAGMA table_info(community_index)').all().map((column) => column.name), ['community', 'protocol_version', 'index_version']);
  assert.deepEqual({ ...db.prepare('SELECT * FROM communities').get() }, { name: mapping.name, community: mapping.community });
  assert.deepEqual({ ...db.prepare('SELECT * FROM community_proofs').get() }, { community: winner.community, claim_version: 1, signature: winner.signature });
  db.close();
});

test('a claim is bound to its exact canonical name and community key', async (t) => {
  const server = await start();
  t.after(() => server.close());
  const key = generateKeyPairSync('ed25519');
  const valid = proof('privacy', key);
  for (const bad of [
    { ...valid, name: 'changed' },
    { ...valid, community: proof('privacy').community },
    { ...valid, signature: '0'.repeat(128) },
    { ...valid, signature: valid.signature.slice(2) },
    { ...valid, owner: 'unexpected private data' },
    { ...valid, name: 'prıvacy' },
    { ...valid, name: 'K' },
    { ...valid, community: '01' + '00'.repeat(31), signature: '01' + '00'.repeat(63) },
  ]) assert.equal((await server.put(bad)).status, 400);
  assert.equal((await fetch(`${server.url}?name=privacy`)).status, 404);
  assert.equal((await server.put(valid)).status, 200);
  const secondName = await server.put(proof('another-name', key));
  assert.equal(secondName.status, 409);
  assert.equal((await secondName.json()).error, 'community-registered');
  assert.equal((await fetch(`${server.url}?name=another-name`)).status, 404);
});

test('prefix discovery is bounded, ordered and treats underscore literally', async (t) => {
  const server = await start();
  t.after(() => server.close());
  const names = ['art_x', 'artax', ...Array.from({ length: 53 }, (_, i) => `community-${String(i).padStart(2, '0')}`)];
  for (const name of names) assert.equal((await server.put(proof(name))).status, 200);
  const result = await fetch(`${server.url}?prefix=community-`);
  assert.equal(result.headers.get('cache-control'), 'no-store');
  const list = await result.json();
  assert.equal(list.length, 50);
  assert.equal(list[0].name, 'community-00');
  assert.equal(list[49].name, 'community-49');
  assert.deepEqual((await (await fetch(`${server.url}?prefix=art_`)).json()).map((c) => c.name), ['art_x']);
  for (const query of ['', '?name=x&prefix=x', '?name=x&name=y', '?prefix=%25', '?prefix=%E2%84%AA']) {
    assert.equal((await fetch(server.url + query)).status, 400);
  }
});

test('CORS, request bounds and global rate limits fail without reserving names', async (t) => {
  let now = 0;
  const server = await start(':memory:', { origins: ['https://hoi.example'], writesPerMinute: 1, now: () => now });
  t.after(() => server.close());
  const valid = proof('music');
  assert.equal((await server.put(valid, { Origin: 'https://untrusted.example' })).status, 403);
  const options = await fetch(server.url, { method: 'OPTIONS', headers: { Origin: 'https://hoi.example', 'Access-Control-Request-Method': 'PUT' } });
  assert.equal(options.status, 204);
  assert.equal(options.headers.get('access-control-allow-origin'), 'https://hoi.example');
  const first = await server.put(valid, { Origin: 'https://hoi.example' });
  assert.equal(first.status, 200);
  assert.equal(first.headers.get('access-control-allow-origin'), 'https://hoi.example');
  const limited = await server.put(proof('second'));
  assert.equal(limited.status, 429);
  assert.equal(limited.headers.get('retry-after'), '60');
  now = 60_001;
  assert.equal((await server.put({ ...proof('oversized'), padding: 'x'.repeat(3000) })).status, 413);
  assert.equal((await fetch(`${server.url}?name=oversized`)).status, 404);
  assert.equal((await fetch(server.url, { method: 'DELETE' })).status, 405);
});

test('the configured Tauri origin is explicit and never permits opaque browser origins', async (t) => {
  const server = await start(':memory:', { origins: ['tauri://localhost'] });
  const unconfigured = await start();
  t.after(async () => { await server.close(); await unconfigured.close(); });
  const claim = proof('desktop');
  assert.equal((await unconfigured.put(claim, { Origin: 'tauri://localhost' })).status, 403);
  for (const origin of ['null', 'tauri://other-host', 'tauri://localhost/path']) {
    assert.equal((await server.put(claim, { Origin: origin })).status, 403);
  }
  const reserved = await server.put(claim, { Origin: 'tauri://localhost' });
  assert.equal(reserved.status, 200);
  assert.equal(reserved.headers.get('access-control-allow-origin'), 'tauri://localhost');
  assert.throws(() => createRegistryServer({ origins: ['null'] }));
});

test('the Rust-generated Ed25519 registration vector is accepted without translation', async (t) => {
  const fixture = JSON.parse(await readFile(new URL('../core/vectors/community-registration.json', import.meta.url), 'utf8'));
  assert.equal(registrationStatement(fixture.name, fixture.community), fixture.statement);
  const server = await start();
  t.after(() => server.close());
  const { name, community, signature } = fixture;
  assert.equal((await server.put({ name: 'modified', community, signature })).status, 400);
  const registered = await server.put({ name, community, signature });
  assert.equal(registered.status, 200);
  assert.deepEqual(await registered.json(), { name, community, proof: { version: 1, signature } });
});

test('v2 binds protocol metadata, keeps names reserved across index changes, and reads fresh release notices', async (t) => {
  let release = loadRelease();
  const server = await start(':memory:', { releaseProvider: () => release });
  t.after(() => server.close());
  const key = generateKeyPairSync('ed25519');
  const claim = publication('garden', key);
  for (const bad of [
    { ...claim, protocolVersion: '0.4' }, { ...claim, indexVersion: 2 },
    { ...claim, indexVersion: 0 }, { ...claim, name: 'elsewhere' },
    { ...claim, description: 'Do not store this' }, { ...claim, protocolVersion: '0.3\ninjected' },
  ]) assert.equal((await server.publish(bad)).status, 400);
  const reserved = { name: claim.name, community: claim.community, protocolVersion: '0.3', indexVersion: 1, compatible: true, indexed: true, proof: savedProof(claim) };
  assert.deepEqual(await (await server.publish(claim)).json(), reserved);
  assert.deepEqual(await (await server.publish(claim)).json(), reserved);
  assert.deepEqual(await (await fetch(`${server.v2}?prefix=`)).json(), [reserved]);
  const conflict = await server.publish(publication('garden'));
  assert.equal(conflict.status, 409);
  assert.deepEqual(await conflict.json(), { error: 'name-taken', message: 'This public name belongs to another community.', ...reserved });
  assert.equal((await server.publish(publication('other-name', key))).status, 409);
  assert.equal((await server.publish(publication('garden', key, '0.4', 2))).status, 409, 'a proof cannot mutate a previous index binding');
  release = { ...release, revision: release.revision + 1, protocolVersion: '0.4', supportedProtocols: ['0.4'], indexVersion: 2,
    notice: { title: 'Preview updated', message: 'This preview can lose data.' } };
  assert.deepEqual(await (await fetch(`${server.v2}?prefix=`)).json(), []);
  assert.deepEqual(await (await fetch(`${server.v2}?name=garden`)).json(), { ...reserved, compatible: false, indexed: false });
  assert.equal((await server.publish(publication('garden', undefined, '0.4', 2))).status, 409, 'hiding old discovery never frees its name');
  assert.deepEqual(await (await fetch(`${server.root}/release.json`)).json(), release);
  const metadata = await (await fetch(`${server.root}/v1/meta`)).json();
  assert.deepEqual(metadata.registry, { apiVersion: 2, schemaVersion: 3, indexVersion: 2 });
  assert.equal(metadata.release.notice.title, 'Preview updated');
  assert.deepEqual(await (await fetch(`${server.url}?name=garden`)).json(), { name: claim.name, community: claim.community, proof: savedProof(claim) }, 'v1 includes the retained proof');
});

test('a legacy database migrates without losing reservations or inventing protocol metadata', async (t) => {
  const folder = await mkdtemp(join(tmpdir(), 'hoi-registry-migration-'));
  const filename = join(folder, 'legacy.db');
  const key = generateKeyPairSync('ed25519');
  const legacy = proof('legacy', key);
  const old = new DatabaseSync(filename);
  old.exec('CREATE TABLE communities (name TEXT PRIMARY KEY, community TEXT NOT NULL UNIQUE) STRICT;');
  old.prepare('INSERT INTO communities VALUES (?, ?)').run(legacy.name, legacy.community);
  old.close();
  const server = await start(filename);
  t.after(async () => {
    await server.close();
    for (const child of await readdir(folder)) await unlink(join(folder, child));
    await rmdir(folder);
  });
  assert.deepEqual(await (await fetch(`${server.v2}?name=legacy`)).json(), {
    name: legacy.name, community: legacy.community, protocolVersion: null, indexVersion: null, compatible: false, indexed: false, proof: null,
  });
  assert.deepEqual(await (await fetch(`${server.v2}?prefix=legacy`)).json(), []);
  assert.equal((await server.publish(publication('legacy'))).status, 409);
  assert.equal((await server.publish(publication('legacy', key))).status, 200, 'only a version-bound proof may add metadata to an existing reservation');
  const check = new DatabaseSync(filename);
  assert.equal(check.prepare('PRAGMA user_version').get().user_version, 5);
  assert.deepEqual(check.prepare('SELECT * FROM community_index').all().map((row) => ({ ...row })), [
    { community: legacy.community, protocol_version: '0.3', index_version: 1 },
  ]);
  check.close();
});

test('current directory filtering happens before the page limit and supports a bounded cursor', async (t) => {
  const server = await start();
  t.after(() => server.close());
  for (let i = 0; i < 52; i++) {
    assert.equal((await server.publish(publication(`a-old-${i}`, undefined, '0.2', 1))).status, 200);
    assert.equal((await server.publish(publication(`garden-${String(i).padStart(2, '0')}`))).status, 200);
  }
  const page = await (await fetch(`${server.v2}?prefix=`)).json();
  assert.equal(page.length, 50);
  assert.equal(page[0].name, 'garden-00');
  const last = await (await fetch(`${server.v2}?prefix=&after=${page.at(-1).name}`)).json();
  assert.deepEqual(last.map((row) => row.name), ['garden-50', 'garden-51']);
  for (const query of ['?name=garden&after=x', '?prefix=&after=x&after=y', '?prefix=&after=%25']) {
    assert.equal((await fetch(server.v2 + query)).status, 400);
  }
});

test('only explicit public infrastructure routes and safe app URLs are advertised', async (t) => {
  const route = `rtc:${Buffer.alloc(32, 7).toString('base64url')}@127.0.0.1:9000`;
  const nativeRoute = `${'ab'.repeat(32)}@127.0.0.1:9001`;
  const server = await start(':memory:', { publicNodes: [route, route, nativeRoute], appUrl: 'http://localhost:5173/' });
  t.after(() => server.close());
  const meta = await (await fetch(`${server.root}/v1/meta`)).json();
  assert.deepEqual(meta.publicNodes, [route, nativeRoute]);
  assert.equal(meta.appUrl, 'http://localhost:5173/');
  for (const nodes of [['https://user:secret@example.com'], [route.replace('127.0.0.1', '0.0.0.0')],
    [route.replace('127.0.0.1', '[0:0:0:0:0:0:0:0]')], [route.replace('9000', '65536')], Array(17).fill(route)]) {
    assert.throws(() => validatePublicNodes(nodes));
  }
  assert.throws(() => createRegistryServer({ appUrl: 'https://user:secret@example.com/' }));
  assert.throws(() => createRegistryServer({ appUrl: 'http://example.com/' }));
});

test('Rust and Node share the exact v2 proof format', async (t) => {
  const fixture = JSON.parse(await readFile(new URL('../core/vectors/community-publication.json', import.meta.url), 'utf8'));
  const { name, community, protocolVersion, indexVersion, signature } = fixture;
  assert.equal(publicationStatement(name, community, protocolVersion, indexVersion), fixture.statement);
  const server = await start();
  t.after(() => server.close());
  const result = await server.publish({ name, community, protocolVersion, indexVersion, signature });
  assert.equal(result.status, 200);
  assert.equal((await result.json()).indexed, true);
});

test('v2 competing writes are atomic and a future schema cannot be downgraded', async (t) => {
  const folder = await mkdtemp(join(tmpdir(), 'hoi-registry-atomic-'));
  const filename = join(folder, 'registry.db');
  const a = await start(filename);
  const b = await start(filename);
  t.after(async () => {
    await a.close(); await b.close();
    for (const child of await readdir(folder)) await unlink(join(folder, child));
    await rmdir(folder);
  });
  const claims = Array.from({ length: 12 }, () => publication('shared-name'));
  const results = await Promise.all(claims.map((claim, i) => (i % 2 ? a : b).publish(claim)));
  assert.equal(results.filter((result) => result.status === 200).length, 1);
  assert.equal(results.filter((result) => result.status === 409).length, 11);
  const winner = claims[results.findIndex((result) => result.status === 200)];
  const found = await (await fetch(`${b.v2}?name=shared-name`)).json();
  assert.equal(found.community, winner.community);
  assert.equal(found.indexed, true);
  await a.close(); await b.close();
  const db = new DatabaseSync(filename);
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM communities').get().n, 1);
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM community_index').get().n, 1);
  db.exec('PRAGMA user_version=6');
  db.close();
  assert.throws(() => openRegistry(filename), /newer registry server/);
  const unchanged = new DatabaseSync(filename);
  assert.equal(unchanged.prepare('PRAGMA user_version').get().user_version, 6);
  unchanged.close();
});

test('nodes announce themselves to the standalone registry and are listed per community until they fall silent', async (t) => {
  let now = 1_000_000_000;
  const configured = `rtc:${Buffer.alloc(32, 1).toString('base64url')}@203.0.113.1:7844`;
  const server = await start(':memory:', { publicNodes: [configured], now: () => now });
  t.after(() => server.close());
  const announce = (body) => fetch(`${server.root}/v2/nodes`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  const node = (communities, issuedAt = Math.floor(now / 1000), key = generateKeyPairSync('ed25519')) => nodeAnnouncement(server.root, communities, issuedAt, key);
  const garden = 'a'.repeat(64);
  const carrier = node([garden]);
  assert.equal(carrier.registry, server.root, 'unconfigured, the registry answers to the loopback address it was reached at');
  const saved = await announce(carrier);
  assert.equal(saved.status, 200);
  assert.deepEqual(await saved.json(), carrier.signature ? { endpoint: carrier.endpoint, routes: carrier.routes, communities: [garden], issuedAt: carrier.issuedAt } : null);
  assert.deepEqual((await (await fetch(`${server.root}/v1/meta`)).json()).publicNodes, [configured, ...carrier.routes]);
  assert.deepEqual((await (await fetch(`${server.root}/v2/nodes?community=${garden}`)).json()).publicNodes, [configured, ...carrier.routes], 'the operator\'s routes stay first even for the carried community');
  assert.equal((await fetch(`${server.root}/v2/nodes`)).status, 400);
  assert.equal((await announce({ ...carrier, communities: [] })).status, 400, 'the signature covers the community list');
  assert.equal((await announce(node([garden], Math.floor(now / 1000) - 700))).status, 400, 'stale announcements are refused');
  assert.equal((await fetch(`${server.root}/v2/nodes`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: 'x'.repeat(9000) })).status, 413);
  now += 31 * 60 * 1000;
  assert.deepEqual((await (await fetch(`${server.root}/v1/meta`)).json()).publicNodes, [configured]);

  const off = await start(':memory:', { publicNodes: [configured], nodeAnnouncements: false, now: () => now });
  t.after(() => off.close());
  assert.equal((await fetch(`${off.root}/v2/nodes`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(node([garden])) })).status, 403);
  assert.throws(() => createRegistryServer({ filename: ':memory:', nodeAnnouncements: 'yes' }), /true or false/);
});

test('a community can be looked up by key for carriers mirroring the index', async (t) => {
  const server = await start();
  t.after(() => server.close());
  const claim = publication('alps');
  assert.equal((await server.publish(claim)).status, 200);
  assert.deepEqual(await (await fetch(`${server.v2}?community=${claim.community}`)).json(), await (await fetch(`${server.v2}?name=alps`)).json());
  assert.equal((await fetch(`${server.v2}?community=${'0'.repeat(64)}`)).status, 404);
  assert.equal((await fetch(`${server.v2}?community=alps`)).status, 400);
  assert.equal((await fetch(`${server.url}?community=${claim.community}`)).status, 400);
});


test('registry is API-only and never serves website, app or database files', async (t) => {
  const server = await start();
  t.after(() => server.close());
  for (const path of ['/', '/index.html', '/registry.js', '/registry.css', '/assets/tokens.css', '/app/', '/hoi-registry.db', '/relay/v1/meta']) {
    for (const method of ['GET', 'HEAD']) {
      const response = await fetch(server.root + path, { method });
      assert.equal(response.status, 404, `${method} ${path}`);
      assert.match(response.headers.get('content-type'), /application\/json/);
      assert.equal(response.headers.get('set-cookie'), null);
    }
  }
  assert.equal((await fetch(server.root + '/v1/meta')).status, 200);
});

test('the Node service passes the shared API conformance suite that hoi-node --registry also runs', async (t) => {
  let now = 1_700_000_000_000;
  const server = await start(':memory:', { origins: ['https://hoi.example', 'tauri://localhost'], announcementsPerMinute: 10_000, now: () => now });
  t.after(() => server.close());
  const made = await runConformance(server.root, { clock: () => Math.floor(now / 1000), advance: (ms) => { now += ms; } });
  await runLogConformance(server.root);
  assert.deepEqual(await (await fetch(`${server.url}?name=${made.name}`)).json(), { name: made.name, community: made.community, proof: { version: 1, signature: (await (await fetch(`${server.url}?name=${made.name}`)).json()).proof.signature } });
});

test('an announcement names the registry it is for, so no registry can replay it to another', async (t) => {
  const now = 1_700_000_000_000;
  const at = Math.floor(now / 1000);
  const announce = (root, body) => fetch(`${root}/v2/nodes`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  // Configured: only its public origin counts, whatever address the connection came in on.
  const configured = await start(':memory:', { now: () => now, registryUrl: 'https://registry.example/registry' });
  t.after(() => configured.close());
  assert.equal((await announce(configured.root, nodeAnnouncement('https://registry.example', [], at))).status, 200);
  for (const elsewhere of [configured.root, 'https://other-registry.example', 'https://registry.example:8443']) {
    const refused = await announce(configured.root, nodeAnnouncement(elsewhere, [], at));
    assert.equal(refused.status, 400, elsewhere);
    assert.equal((await refused.json()).error, 'wrong-registry');
  }
  // Unconfigured: the loopback address it was reached at, as an IP or as localhost; nothing else.
  const local = await start(':memory:', { now: () => now });
  t.after(() => local.close());
  const port = new URL(local.root).port;
  for (const origin of [`http://127.0.0.1:${port}`, `http://localhost:${port}`]) assert.equal((await announce(local.root, nodeAnnouncement(origin, [], at))).status, 200, origin);
  assert.equal((await announce(local.root, nodeAnnouncement('https://registry.example', [], at))).status, 400);
  assert.deepEqual(loopbackOrigins('127.0.0.1', 7845), ['http://127.0.0.1:7845', 'http://localhost:7845']);
  assert.deepEqual(loopbackOrigins('::1', 7845), ['http://[::1]:7845', 'http://localhost:7845']);
  assert.deepEqual(loopbackOrigins('203.0.113.4', 7845), [], 'reached from outside without a configured URL, it takes no announcements');
  assert.equal(registryOrigin('https://hoi.sh/registry/'), 'https://hoi.sh');
  for (const bad of ['http://registry.example', 'https://user:pw@registry.example', 'https://registry.example/?x=1', 'registry.example']) {
    assert.throws(() => registryOrigin(bad), /public https/, bad);
    assert.throws(() => createRegistryServer({ filename: ':memory:', registryUrl: bad }));
  }
});

test('allowlist mode lists only the endpoint keys its operator allowed and refuses the rest clearly', async (t) => {
  let now = 1_700_000_000_000;
  const server = await start(':memory:', { now: () => now, announcers: [ALLOWED_ANNOUNCER.endpoint] });
  t.after(() => server.close());
  await runAllowlistConformance(server.root, { clock: () => Math.floor(now / 1000) });
  assert.deepEqual([...validateAnnouncers(` ${ALLOWED_ANNOUNCER.endpoint} ,`)], [ALLOWED_ANNOUNCER.endpoint], 'the environment variable is a comma-separated list');
  assert.equal(validateAnnouncers(''), null, 'empty means any node may announce');
  for (const bad of ['not a key', ALLOWED_ANNOUNCER.endpoint.toUpperCase(), '0'.repeat(64)]) assert.throws(() => validateAnnouncers(bad), /endpoint keys/);
  assert.throws(() => createRegistryServer({ filename: ':memory:', announcers: ['nope'] }));
  // Switched on after junk arrived, it hides what announced before at once.
  const folder = await mkdtemp(join(tmpdir(), 'hoi-registry-allowlist-'));
  const filename = join(folder, 'registry.db');
  const listeners = [];
  t.after(async () => {
    for (const listener of listeners) await listener.close();
    for (const child of await readdir(folder)) await unlink(join(folder, child));
    await rmdir(folder);
  });
  const open = await start(filename, { now: () => now });
  listeners.push(open);
  const stranger = nodeAnnouncement(open.root, [], Math.floor(now / 1000));
  assert.equal((await fetch(`${open.root}/v2/nodes`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(stranger) })).status, 200);
  assert.ok((await (await fetch(`${open.root}/v1/meta`)).json()).publicNodes.includes(stranger.routes[0]));
  await open.close();
  const strict = await start(filename, { now: () => now, announcers: [ALLOWED_ANNOUNCER.endpoint] });
  listeners.push(strict);
  assert.deepEqual((await (await fetch(`${strict.root}/v1/meta`)).json()).publicNodes, []);
});

test('the operator withholds listings and drops nodes from a shell, never over HTTP', async (t) => {
  const folder = await mkdtemp(join(tmpdir(), 'hoi-registry-admin-'));
  const filename = join(folder, 'registry.db');
  let now = 1_700_000_000_000;
  const server = await start(filename, { now: () => now });
  t.after(async () => {
    await server.close();
    for (const child of await readdir(folder)) await unlink(join(folder, child));
    await rmdir(folder);
  });
  const admin = (...args) => runAdmin(['--db', filename, ...args], { env: {}, now: () => Math.floor(now / 1000) });
  await runOperatorConformance(server.root, {
    hide: async (value) => { admin('hide', value); },
    unhide: async (value) => { admin('unhide', value); },
    dropNode: async (endpoint) => { admin('drop-node', endpoint); },
    clock: () => Math.floor(now / 1000),
  });
  const listing = admin('list').join('\n');
  assert.match(listing, /withheld \(0\)/);
  assert.match(listing, /refused nodes \(1\)/);
  assert.match(listing, /announced now \(\d+\)/);
  assert.throws(() => admin('unhide', 'never-hidden'), /not withheld/);
  assert.throws(() => admin('hide', 'Not A Name!'), /public name or a 64-character/);
  assert.throws(() => admin('drop-node', 'abc'), /endpoint key/);
  assert.throws(() => admin('frobnicate'), /Usage/);
  assert.throws(() => admin('hide'), /Usage/);
  assert.throws(() => runAdmin(['--db', join(folder, 'missing.db'), 'list']), /No registry database/);
  const refused = admin('list').find((line) => /^ {2}[0-9a-f]{64} since/.test(line)).trim().split(' ')[0];
  admin('allow-node', refused);
  assert.match(admin('list').join('\n'), /refused nodes \(0\)/);
  // A name withheld before anyone claims it cannot be claimed.
  admin('hide', 'reserved-later');
  const claim = publication('reserved-later');
  const blocked = await server.publish(claim);
  assert.equal(blocked.status, 409);
  assert.equal((await blocked.json()).error, 'withheld');
  assert.equal((await fetch(`${server.v2}?name=reserved-later`)).status, 404);
  // None of this is reachable over HTTP.
  for (const method of ['DELETE', 'POST', 'PATCH']) assert.equal((await fetch(`${server.root}/v2/communities?name=reserved-later`, { method })).status, 405);
  assert.equal((await fetch(`${server.root}/v2/admin`)).status, 404);
});

test('loopback and private announced routes are a test-only option, off by default', async (t) => {
  const now = 1_700_000_000_000;
  const at = Math.floor(now / 1000);
  const announce = (root, body) => fetch(`${root}/v2/nodes`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  const key = generateKeyPairSync('ed25519');
  const local = (root) => nodeAnnouncement(root, [], at, key, [`rtc:${Buffer.alloc(32, 3).toString('base64url')}@127.0.0.1:7844`]);
  const strict = await start(':memory:', { now: () => now });
  const testing = await start(':memory:', { now: () => now, allowPrivateRoutesForTests: true });
  t.after(async () => { await strict.close(); await testing.close(); });
  assert.equal((await announce(strict.root, local(strict.root))).status, 400, 'by default a loopback route is refused');
  assert.equal((await announce(testing.root, local(testing.root))).status, 200);
  assert.ok((await (await fetch(`${testing.root}/v1/meta`)).json()).publicNodes.includes(local(testing.root).routes[0]));
  assert.throws(() => createRegistryServer({ filename: ':memory:', allowPrivateRoutesForTests: 'yes' }), /true or false/);
});
