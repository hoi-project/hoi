// The registry API, request for request, against any implementation: the Node service, the
// Worker, or `hoi-node --registry`. Everything goes over `fetch`; nothing reaches into a database.
// Names carry a random prefix so a run never collides with an earlier one on the same namespace.
import assert from 'node:assert/strict';
import { createPrivateKey, createPublicKey, generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { announcementStatement, publicationStatement, registrationStatement } from './protocol.mjs';
import { fromHex, logEntryText, logLeaf, toHex, verifiedLogHead, verifyConsistency, verifyInclusion } from './public/log.js';

const keyHex = (key) => key.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
const signed = (key, statement) => sign(null, Buffer.from(statement), key.privateKey).toString('hex');

/** The one node the allowlist run announces as. Start the registry under test allowing
 * `ALLOWED_ANNOUNCER.endpoint` (HOI_REGISTRY_ANNOUNCERS, `--registry-announcer`). */
export const ALLOWED_ANNOUNCER = (() => {
  const privateKey = createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.alloc(32, 0x61)]), format: 'der', type: 'pkcs8' });
  const key = { privateKey, publicKey: createPublicKey(privateKey) };
  return { key, endpoint: keyHex(key) };
})();

/** A node's announcement to the registry at `root`, signed by `key` (a fresh one by default). */
function announcement(root, communities, { clock, key = generateKeyPairSync('ed25519'), routes, issuedAt = clock(), registry = new URL(root).origin } = {}) {
  const endpoint = keyHex(key);
  const own = routes ?? [`${endpoint}@203.0.113.9:7843`];
  return { key, body: { registry, endpoint, routes: own, communities, issuedAt, signature: signed(key, announcementStatement(registry, endpoint, issuedAt, own, communities)) } };
}

/**
 * Runs the conformance suite against the registry at `root` (no trailing slash).
 * - `origins` must be allowed by the registry under test (CORS checks use the first two).
 * - `clock()` is the registry's idea of Unix seconds (announcements must be dated within ten minutes).
 * - `advance(ms)` moves that clock when the implementation lets a test control it; the
 *   half-hour expiry of announcements is only checked when it is given.
 * Returns one reservation it made, so a caller can check the database it ended up in.
 */
export async function runConformance(root, { origins = ['https://hoi.example', 'tauri://localhost'], clock = () => Math.floor(Date.now() / 1000), advance } = {}) {
  const t = randomBytes(3).toString('hex');
  const v1 = `${root}/v1/communities`;
  const v2 = `${root}/v2/communities`;
  const json = { 'Content-Type': 'application/json' };
  const put = (url, body, headers = {}) => fetch(url, { method: 'PUT', headers: { ...json, ...headers }, body: typeof body === 'string' ? body : JSON.stringify(body) });
  const release = await (await fetch(`${root}/release.json`)).json();
  const proof = (name, key = generateKeyPairSync('ed25519')) => {
    const community = keyHex(key);
    return { name, community, signature: signed(key, registrationStatement(name, community)) };
  };
  const publication = (name, key = generateKeyPairSync('ed25519'), protocolVersion = release.protocolVersion, indexVersion = release.indexVersion) => {
    const community = keyHex(key);
    return { name, community, protocolVersion, indexVersion, signature: signed(key, publicationStatement(name, community, protocolVersion, indexVersion)) };
  };

  // Metadata and the API-only surface.
  const metaResponse = await fetch(`${root}/v1/meta`);
  assert.equal(metaResponse.status, 200);
  for (const [header, value] of [['cache-control', 'no-store'], ['x-content-type-options', 'nosniff'], ['referrer-policy', 'no-referrer'], ['vary', 'Origin']]) {
    assert.equal(metaResponse.headers.get(header), value, header);
  }
  assert.match(metaResponse.headers.get('content-type'), /^application\/json/);
  const meta = await metaResponse.json();
  assert.deepEqual(meta.registry, { apiVersion: 2, schemaVersion: 3, indexVersion: release.indexVersion });
  assert.deepEqual(meta.release, release);
  assert.ok(Array.isArray(meta.publicNodes) && meta.publicNodes.length <= 16);
  assert.equal(typeof meta.appUrl, 'string');
  assert.equal((await fetch(`${root}/v1/meta?x=1`)).status, 400);
  for (const path of ['/', '/index.html', '/registry.js', '/registry.css', '/assets/tokens.css', '/app/', '/hoi-registry.db', '/relay/v1/meta', '/v1/meta/', '/v3/communities']) {
    for (const method of ['GET', 'HEAD']) {
      const response = await fetch(root + path, { method });
      assert.equal(response.status, 404, `${method} ${path}`);
      assert.match(response.headers.get('content-type'), /application\/json/);
      assert.equal(response.headers.get('set-cookie'), null);
    }
  }
  const deleted = await fetch(v1, { method: 'DELETE' });
  assert.equal(deleted.status, 405);
  assert.equal(deleted.headers.get('allow'), 'GET, PUT, OPTIONS');
  assert.equal((await fetch(`${root}/v1/meta`, { method: 'HEAD' })).status, 405);
  assert.equal((await fetch(`${root}/v1/meta`, { method: 'PUT', headers: json, body: '{}' })).status, 405);

  // v1 claims: bound to the exact canonical name and key, first claim wins, retries are idempotent.
  const hikingKey = generateKeyPairSync('ed25519');
  const hiking = proof(`${t}-hiking`, hikingKey);
  for (const bad of [
    { ...hiking, name: `${t}-changed` },
    { ...hiking, community: proof(`${t}-hiking`).community },
    { ...hiking, signature: '0'.repeat(128) },
    { ...hiking, signature: hiking.signature.slice(2) },
    { ...hiking, owner: 'unexpected private data' },
    { ...hiking, name: 'prıvacy' },
    { ...hiking, name: 'K' },
    { ...hiking, community: '01' + '00'.repeat(31), signature: '01' + '00'.repeat(63) },
  ]) {
    const response = await put(v1, bad);
    assert.equal(response.status, 400, JSON.stringify(bad).slice(0, 80));
    assert.equal((await response.json()).error, 'invalid-proof');
  }
  assert.equal((await fetch(`${v1}?name=${t}-hiking`)).status, 404);
  const mapping = { name: `${t}-hiking`, community: hiking.community, proof: { version: 1, signature: hiking.signature } };
  const reserved = await put(v1, { ...hiking, name: ` ${t.toUpperCase()}-HIKING ` });
  assert.equal(reserved.status, 200, 'names are trimmed and lowercased before the signature is checked');
  assert.deepEqual(await reserved.json(), mapping);
  assert.deepEqual(await (await put(v1, hiking)).json(), mapping, 'retry after an uncertain response is idempotent');
  const taken = await put(v1, proof(`${t}-hiking`));
  assert.equal(taken.status, 409);
  assert.deepEqual(await taken.json(), { error: 'name-taken', message: 'This public name belongs to another community.', ...mapping });
  const secondName = await put(v1, proof(`${t}-another`, hikingKey));
  assert.equal(secondName.status, 409);
  assert.deepEqual(await secondName.json(), { error: 'community-registered', message: 'This community already has a public name.', ...mapping });
  assert.equal((await fetch(`${v1}?name=${t}-another`)).status, 404);
  assert.deepEqual(await (await fetch(`${v1}?name=${t.toUpperCase()}-Hiking`)).json(), mapping, 'lookups are case-insensitive');
  for (const query of ['', '?name=x&prefix=x', '?name=x&name=y', '?prefix=%25', '?prefix=%E2%84%AA', `?community=${hiking.community}`, '?name=']) {
    assert.equal((await fetch(v1 + query)).status, 400, query);
  }

  // Prefix discovery is bounded, ordered and treats underscore literally.
  for (const name of [`${t}art_x`, `${t}artax`, ...Array.from({ length: 53 }, (_, i) => `${t}-c-${String(i).padStart(2, '0')}`)]) {
    assert.equal((await put(v1, proof(name))).status, 200, name);
  }
  const page = await (await fetch(`${v1}?prefix=${t}-c-`)).json();
  assert.equal(page.length, 50);
  assert.equal(page[0].name, `${t}-c-00`);
  assert.equal(page[49].name, `${t}-c-49`);
  assert.ok(page.every((entry) => entry.proof?.version === 1));
  assert.deepEqual((await (await fetch(`${v1}?prefix=${t}art_`)).json()).map((c) => c.name), [`${t}art_x`]);

  // v2 publications bind protocol and index, keep names reserved, never mutate metadata.
  const gardenKey = generateKeyPairSync('ed25519');
  const garden = publication(`${t}-garden`, gardenKey);
  for (const bad of [
    { ...garden, protocolVersion: `${release.protocolVersion}x` }, { ...garden, indexVersion: release.indexVersion + 1 },
    { ...garden, indexVersion: 0 }, { ...garden, name: `${t}-elsewhere` },
    { ...garden, description: 'Do not store this' }, { ...garden, protocolVersion: `${release.protocolVersion}\ninjected` },
  ]) assert.equal((await put(v2, bad)).status, 400);
  const listed = {
    name: `${t}-garden`, community: garden.community, protocolVersion: release.protocolVersion, indexVersion: release.indexVersion, compatible: true, indexed: true,
    proof: { version: 2, signature: garden.signature, protocolVersion: release.protocolVersion, indexVersion: release.indexVersion },
  };
  assert.deepEqual(await (await put(v2, garden)).json(), listed);
  assert.deepEqual(await (await put(v2, garden)).json(), listed);
  assert.deepEqual(await (await fetch(`${v2}?prefix=${t}-garden`)).json(), [listed]);
  const conflict = await put(v2, publication(`${t}-garden`));
  assert.equal(conflict.status, 409);
  assert.deepEqual(await conflict.json(), { error: 'name-taken', message: 'This public name belongs to another community.', ...listed });
  assert.equal((await put(v2, publication(`${t}-other-name`, gardenKey))).status, 409);
  const mutated = await put(v2, publication(`${t}-garden`, gardenKey, `${release.protocolVersion}.1`, release.indexVersion + 1));
  assert.equal(mutated.status, 409, 'a proof cannot mutate a previous index binding');
  assert.equal((await mutated.json()).error, 'index-version-conflict');
  assert.deepEqual(await (await fetch(`${v1}?name=${t}-garden`)).json(), { name: `${t}-garden`, community: garden.community, proof: listed.proof }, 'v1 includes the retained proof');
  assert.deepEqual(await (await fetch(`${v2}?community=${garden.community}`)).json(), listed);
  assert.equal((await fetch(`${v2}?community=${'0'.repeat(64)}`)).status, 404);
  assert.equal((await fetch(`${v2}?community=garden`)).status, 400);
  assert.deepEqual(await (await fetch(`${v2}?name=${t}-hiking`)).json(), { ...mapping, protocolVersion: null, indexVersion: null, compatible: false, indexed: false }, 'a v1 reservation is not current discovery');
  assert.deepEqual(await (await fetch(`${v2}?prefix=${t}-hiking`)).json(), []);
  const old = publication(`${t}-old`, undefined, `${release.protocolVersion}-old`, release.indexVersion);
  const oldListed = await put(v2, old);
  assert.equal(oldListed.status, 200);
  assert.equal((await oldListed.json()).compatible, false, 'another protocol family stays reserved and hidden');
  assert.deepEqual(await (await fetch(`${v2}?prefix=${t}-old`)).json(), []);

  // Filtering happens before the page limit and the cursor is bounded.
  for (let i = 0; i < 52; i++) {
    assert.equal((await put(v2, publication(`${t}g-a-${i}`, undefined, `${release.protocolVersion}-old`, release.indexVersion))).status, 200);
    assert.equal((await put(v2, publication(`${t}g-${String(i).padStart(2, '0')}`))).status, 200);
  }
  const first = await (await fetch(`${v2}?prefix=${t}g-`)).json();
  assert.equal(first.length, 50);
  assert.equal(first[0].name, `${t}g-00`);
  const rest = await (await fetch(`${v2}?prefix=${t}g-&after=${first.at(-1).name}`)).json();
  assert.deepEqual(rest.map((row) => row.name), [`${t}g-50`, `${t}g-51`]);
  for (const query of [`?name=${t}g-00&after=x`, '?prefix=&after=x&after=y', '?prefix=&after=%25', `?community=${garden.community}&name=${t}-garden`]) {
    assert.equal((await fetch(v2 + query)).status, 400, query);
  }

  // The shared vectors are accepted without translation.
  const registration = JSON.parse(await readFile(new URL('../core/vectors/community-registration.json', import.meta.url), 'utf8'));
  assert.equal((await put(v1, { name: 'modified', community: registration.community, signature: registration.signature })).status, 400);
  const vector = await put(v1, { name: registration.name, community: registration.community, signature: registration.signature });
  assert.equal(vector.status, 200);
  assert.equal((await vector.json()).community, registration.community);
  const published = JSON.parse(await readFile(new URL('../core/vectors/community-publication.json', import.meta.url), 'utf8'));
  const { name, community, protocolVersion, indexVersion, signature } = published;
  assert.equal((await put(v2, { name, community, protocolVersion, indexVersion, signature })).status, 200);

  // CORS: configured exact origins only, never the opaque origin; preflight says what is allowed.
  const [browser, desktop] = origins;
  const music = proof(`${t}-music`);
  assert.equal((await put(v1, music, { Origin: 'https://untrusted.example' })).status, 403);
  assert.equal((await put(v1, music, { Origin: 'null' })).status, 403);
  assert.equal((await fetch(`${v1}?name=${t}-music`, { headers: { Origin: 'https://untrusted.example' } })).status, 403);
  assert.equal((await fetch(`${v1}?name=${t}-music`)).status, 404, 'a refused origin reserved nothing');
  const options = await fetch(v1, { method: 'OPTIONS', headers: { Origin: browser, 'Access-Control-Request-Method': 'PUT' } });
  assert.equal(options.status, 204);
  assert.equal(options.headers.get('access-control-allow-origin'), browser);
  assert.equal(options.headers.get('access-control-allow-methods'), 'GET, PUT');
  assert.equal(options.headers.get('access-control-allow-headers'), 'Content-Type');
  const metaOptions = await fetch(`${root}/v1/meta`, { method: 'OPTIONS', headers: { Origin: browser, 'Access-Control-Request-Method': 'PUT' } });
  assert.equal(metaOptions.status, 405);
  assert.equal((await fetch(`${root}/v1/meta`, { method: 'OPTIONS', headers: { Origin: browser, 'Access-Control-Request-Method': 'GET' } })).status, 204);
  const allowed = await put(v1, music, { Origin: browser });
  assert.equal(allowed.status, 200);
  assert.equal(allowed.headers.get('access-control-allow-origin'), browser);
  const fromDesktop = await fetch(`${v1}?name=${t}-music`, { headers: { Origin: desktop } });
  assert.equal(fromDesktop.status, 200);
  assert.equal(fromDesktop.headers.get('access-control-allow-origin'), desktop);
  for (const origin of ['tauri://other-host', 'tauri://localhost/path', `${browser}/`]) {
    assert.equal((await fetch(`${v1}?name=${t}-music`, { headers: { Origin: origin } })).status, 403, origin);
  }
  // As a tracker: any app may read which public nodes carry a community, and only that.
  const tracked = await fetch(`${root}/v2/nodes?community=${'ab'.repeat(32)}`, { headers: { Origin: 'https://another-app.example' } });
  assert.equal(tracked.status, 200);
  assert.equal(tracked.headers.get('access-control-allow-origin'), '*');
  assert.deepEqual((await tracked.json()).publicNodes, []);
  assert.equal((await put(`${root}/v2/nodes`, '{}', { Origin: 'https://another-app.example' })).status, 403, 'announcing still needs an allowed origin, or none');
  const knownTracker = await fetch(`${root}/v2/nodes?community=${'ab'.repeat(32)}`, { headers: { Origin: browser } });
  assert.equal(knownTracker.headers.get('access-control-allow-origin'), browser);

  // Request bounds.
  assert.equal((await put(v1, { ...proof(`${t}-oversized`), padding: 'x'.repeat(3000) })).status, 413);
  assert.equal((await fetch(`${v1}?name=${t}-oversized`)).status, 404);
  assert.equal((await put(v1, proof(`${t}-plain`), { 'Content-Type': 'text/plain' })).status, 415);
  assert.equal((await put(v1, proof(`${t}-charset`), { 'Content-Type': 'application/json; charset=utf-8' })).status, 200, 'a charset parameter is fine');
  const broken = await put(v1, '{');
  assert.equal(broken.status, 400);
  assert.equal((await broken.json()).error, 'invalid-request');
  assert.equal((await put(`${v1}?x=1`, proof(`${t}-query`))).status, 400);
  assert.equal((await put(`${root}/v2/nodes`, 'x'.repeat(9000))).status, 413);
  assert.equal((await put(`${root}/v2/nodes?x=1`, '{}')).status, 400);
  assert.equal((await put(`${root}/v2/nodes`, '{}', { 'Content-Type': 'text/plain' })).status, 415);

  // Node announcements: signed by the transport key for this registry, dated by the registry's
  // clock, public addresses only, listed per community until they fall silent.
  const node = (communities, options = {}) => announcement(root, communities, { clock, ...options });
  const communityA = garden.community;
  const carrier = node([communityA]);
  const saved = await put(`${root}/v2/nodes`, carrier.body);
  assert.equal(saved.status, 200);
  const { signature: _unused, registry: _origin, ...kept } = carrier.body;
  assert.deepEqual(await saved.json(), kept);
  // Signed for another registry, it is refused here: a registry cannot pass on what it was sent.
  const elsewhere = node([communityA], { registry: 'https://other-registry.example' });
  const replayed = await put(`${root}/v2/nodes`, elsewhere.body);
  assert.equal(replayed.status, 400);
  assert.equal((await replayed.json()).error, 'wrong-registry');
  assert.equal((await put(`${root}/v2/nodes`, { ...carrier.body, registry: 'https://other-registry.example' })).status, 400, 'the registry is signed, not just stated');
  // The version 1 statement named no registry; it is refused.
  const v1Key = generateKeyPairSync('ed25519');
  const legacy = { endpoint: keyHex(v1Key), routes: [`${keyHex(v1Key)}@203.0.113.9:7843`], communities: [], issuedAt: clock() };
  legacy.signature = signed(v1Key, `hoi public node v1\n${legacy.endpoint}\n${legacy.issuedAt}\n${legacy.routes.join(',')}\n`);
  const outdated = await put(`${root}/v2/nodes`, legacy);
  assert.equal(outdated.status, 400);
  assert.equal((await outdated.json()).error, 'invalid-announcement');
  assert.ok((await (await fetch(`${root}/v1/meta`)).json()).publicNodes.includes(carrier.body.routes[0]));
  const bystander = node([]);
  assert.equal((await put(`${root}/v2/nodes`, bystander.body)).status, 200);
  const forA = (await (await fetch(`${root}/v2/nodes?community=${communityA}`)).json()).publicNodes;
  assert.ok(forA.indexOf(carrier.body.routes[0]) >= 0 && forA.indexOf(carrier.body.routes[0]) < forA.indexOf(bystander.body.routes[0]), 'the carrier of the wanted community comes before the others');
  assert.equal((await fetch(`${root}/v2/nodes`)).status, 400);
  assert.equal((await fetch(`${root}/v2/nodes?community=hiking`)).status, 400);
  assert.equal((await put(`${root}/v2/nodes`, { ...carrier.body, communities: [] })).status, 400, 'the signature covers the community list');
  assert.equal((await put(`${root}/v2/nodes`, { ...carrier.body, name: "lena's pi" })).status, 400, 'nothing about a person travels in an announcement');
  assert.equal((await put(`${root}/v2/nodes`, node([communityA], { issuedAt: clock() - 700 }).body)).status, 400, 'stale announcements are refused');
  assert.equal((await put(`${root}/v2/nodes`, node([communityA], { issuedAt: clock() + 700 }).body)).status, 400, 'future-dated announcements are refused');
  const privateKey = generateKeyPairSync('ed25519');
  assert.equal((await put(`${root}/v2/nodes`, node([], { key: privateKey, routes: [`${keyHex(privateKey)}@192.168.1.9:7843`] }).body)).status, 400, 'private addresses are refused');
  assert.equal((await put(`${root}/v2/nodes`, node([], { key: privateKey, routes: [`rtc:${Buffer.alloc(32, 9).toString('base64url')}@10.0.0.9:7844`] }).body)).status, 400);
  assert.equal((await put(`${root}/v2/nodes`, node([], { key: privateKey, routes: [`${carrier.body.endpoint}@203.0.113.9:7843`] }).body)).status, 400, 'a native route must name the signing endpoint');
  const replay = node([], { key: carrier.key, issuedAt: carrier.body.issuedAt - 1 });
  assert.equal((await put(`${root}/v2/nodes`, replay.body)).status, 200);
  assert.ok((await (await fetch(`${root}/v2/nodes?community=${communityA}`)).json()).publicNodes.indexOf(carrier.body.routes[0]) < (await (await fetch(`${root}/v2/nodes?community=${communityA}`)).json()).publicNodes.indexOf(bystander.body.routes[0]), 'a replayed older announcement changes nothing');
  const moved = node([], { key: carrier.key, issuedAt: carrier.body.issuedAt + 1, routes: [`rtc:${Buffer.alloc(32, 7).toString('base64url')}@203.0.113.7:7844`] });
  assert.equal((await put(`${root}/v2/nodes`, moved.body)).status, 200);
  const nodes = (await (await fetch(`${root}/v1/meta`)).json()).publicNodes;
  assert.ok(nodes.includes(moved.body.routes[0]) && !nodes.includes(carrier.body.routes[0]), 'a newer announcement replaces the node\'s routes');
  if (advance) {
    advance(31 * 60 * 1000);
    const later = (await (await fetch(`${root}/v1/meta`)).json()).publicNodes;
    assert.ok(!later.includes(moved.body.routes[0]) && !later.includes(bystander.body.routes[0]), 'half an hour of silence drops a node from discovery');
    const fresh = node([communityA], { key: carrier.key });
    assert.equal((await put(`${root}/v2/nodes`, fresh.body)).status, 200);
    assert.ok((await (await fetch(`${root}/v1/meta`)).json()).publicNodes.includes(fresh.body.routes[0]));
  }

  return { name: mapping.name, community: mapping.community, publication: listed };
}

/**
 * Allowlist mode: against a registry that allows only `ALLOWED_ANNOUNCER.endpoint`,
 * that node is listed and every other one is refused with 403 `announcer-not-allowed`.
 * `request` replaces `fetch` for an implementation tested without a socket (the Worker).
 */
export async function runAllowlistConformance(root, { clock = () => Math.floor(Date.now() / 1000), request = fetch } = {}) {
  const put = (body) => request(`${root}/v2/nodes`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  const listed = async () => (await (await request(`${root}/v1/meta`)).json()).publicNodes;
  const allowed = announcement(root, [], { clock, key: ALLOWED_ANNOUNCER.key });
  assert.equal((await put(allowed.body)).status, 200);
  assert.ok((await listed()).includes(allowed.body.routes[0]), 'the allowed node is listed');
  const stranger = announcement(root, [], { clock });
  const refused = await put(stranger.body);
  assert.equal(refused.status, 403);
  assert.equal((await refused.json()).error, 'announcer-not-allowed');
  assert.ok(!(await listed()).includes(stranger.body.routes[0]), 'nobody else is');
  // Verification still comes first: an allowed key's bad signature is a bad signature.
  assert.equal((await put({ ...allowed.body, communities: ['a'.repeat(64)] })).status, 400);
}

/**
 * The operator's tools, whatever they are for the implementation: `hide(nameOrKey)` and
 * `unhide(nameOrKey)` withhold a listing, `dropNode(endpoint)` drops a node's announcements and
 * refuses its next ones. A withheld name stays reserved and is served to nobody; a dropped node
 * is listed nowhere. `request` replaces `fetch` as above.
 */
export async function runOperatorConformance(root, { hide, unhide, dropNode, clock = () => Math.floor(Date.now() / 1000), request = fetch }) {
  const t = randomBytes(3).toString('hex');
  const v1 = `${root}/v1/communities`;
  const v2 = `${root}/v2/communities`;
  const put = (url, body) => request(url, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  const status = async (url) => (await request(url)).status;
  const release = await (await request(`${root}/release.json`)).json();
  const publication = (name, key = generateKeyPairSync('ed25519')) => {
    const community = keyHex(key);
    return { name, community, protocolVersion: release.protocolVersion, indexVersion: release.indexVersion,
      signature: signed(key, publicationStatement(name, community, release.protocolVersion, release.indexVersion)) };
  };
  const ownerKey = generateKeyPairSync('ed25519');
  const held = publication(`${t}-held`, ownerKey);
  assert.equal((await put(v2, held)).status, 200);
  assert.equal((await put(v2, publication(`${t}-kept`))).status, 200);
  const carrier = announcement(root, [held.community], { clock });
  assert.equal((await put(`${root}/v2/nodes`, carrier.body)).status, 200);
  const nodesFor = async (community) => (await (await request(`${root}/v2/nodes?community=${community}`)).json()).publicNodes;
  const unknown = 'f'.repeat(64);
  assert.equal((await nodesFor(held.community))[0], carrier.body.routes[0], 'before: its carrier comes first');

  for (const by of [held.name, held.community]) {
    await hide(by);
    for (const url of [`${v2}?name=${held.name}`, `${v2}?community=${held.community}`, `${v1}?name=${held.name}`]) {
      const response = await request(url);
      assert.equal(response.status, 404, `${url} withheld by ${by === held.name ? 'name' : 'key'}`);
      const body = await response.json();
      assert.equal(body.error, 'withheld');
      assert.equal(JSON.stringify(body).includes(held.community), false, 'a withheld answer names no key');
    }
    assert.deepEqual((await (await request(`${v2}?prefix=${t}-`)).json()).map((row) => row.name), [`${t}-kept`], 'not listed');
    assert.deepEqual((await (await request(`${v1}?prefix=${t}-`)).json()).map((row) => row.name), [`${t}-kept`]);
    for (const claim of [held, publication(held.name), publication(`${t}-renamed`, ownerKey)]) {
      const conflict = await put(v2, claim);
      assert.equal(conflict.status, 409, 'still reserved: nobody else gets the name, and the owner cannot list it again');
      const body = await conflict.json();
      assert.equal(body.error, 'withheld');
      assert.equal(JSON.stringify(body).includes(held.community), false);
    }
    assert.deepEqual(await nodesFor(held.community), await nodesFor(unknown), 'nobody is named as its carrier');
    await unhide(by);
    assert.equal(await status(`${v2}?name=${held.name}`), 200, 'listed again');
  }
  assert.equal((await (await request(`${v2}?name=${held.name}`)).json()).community, held.community, 'the reservation never moved');

  await dropNode(carrier.body.endpoint);
  const meta = (await (await request(`${root}/v1/meta`)).json()).publicNodes;
  assert.ok(!meta.includes(carrier.body.routes[0]), 'a dropped node is no longer listed');
  const again = await put(`${root}/v2/nodes`, announcement(root, [held.community], { clock, key: carrier.key }).body);
  assert.equal(again.status, 403);
  assert.equal((await again.json()).error, 'announcer-refused');
  assert.ok(!(await nodesFor(held.community)).includes(carrier.body.routes[0]));
}

/**
 * The public log of names, against any implementation: every retained claim is an
 * entry, the head is signed by the registry's log key, inclusion and consistency proofs check out
 * against the client's own verifier, and the log only grows. `hide` withholds a listing when the
 * implementation lets a test do that: its entry text is then served as null, its leaf still is.
 * `request` replaces `fetch` as above.
 */
export async function runLogConformance(root, { request = fetch, hide } = {}) {
  const t = randomBytes(3).toString('hex');
  const v1 = `${root}/v1/communities`;
  const v2 = `${root}/v2/communities`;
  const put = (url, body) => request(url, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  const get = async (path, headers = {}) => {
    const response = await request(`${root}${path}`, { headers });
    return { status: response.status, body: await response.json(), headers: response.headers };
  };
  const release = await (await request(`${root}/release.json`)).json();
  const head = async () => {
    const { status, body } = await get('/v2/log/head');
    assert.equal(status, 200);
    const verified = await verifiedLogHead(body);
    assert.ok(verified, 'the head is signed by the log key it names');
    return verified;
  };
  const included = async (entry, at) => {
    const leaf = await logLeaf(logEntryText(entry));
    const { status, body } = await get(`/v2/log/proof?leaf=${toHex(leaf)}&size=${at.size}`);
    assert.equal(status, 200, `${entry.name} is in the log`);
    assert.equal(body.size, at.size);
    assert.ok(await verifyInclusion(leaf, body.index, at.size, body.path, fromHex(at.root)), `${entry.name}'s audit path checks out`);
    return body.index;
  };
  const before = await head();

  // A reservation and a publication are each an entry, once, whatever the retries.
  const key = generateKeyPairSync('ed25519');
  const community = keyHex(key);
  const name = `${t}-logged`;
  const reservation = { name, community, signature: signed(key, registrationStatement(name, community)) };
  assert.equal((await put(v1, reservation)).status, 200);
  assert.equal((await put(v1, reservation)).status, 200);
  const publication = { name, community, protocolVersion: release.protocolVersion, indexVersion: release.indexVersion,
    signature: signed(key, publicationStatement(name, community, release.protocolVersion, release.indexVersion)) };
  const listed = await (await put(v2, publication)).json();
  assert.equal((await put(v2, publication)).status, 200);
  const loser = generateKeyPairSync('ed25519');
  assert.equal((await put(v1, { name, community: keyHex(loser), signature: signed(loser, registrationStatement(name, keyHex(loser))) })).status, 409, 'a refused claim is not logged');
  const after = await head();
  assert.equal(after.logKey, before.logKey, 'one log key');
  assert.equal(after.size, before.size + 2, 'the reservation and the publication, nothing else');
  const first = await included({ name, community, proof: { version: 1, signature: reservation.signature } }, after);
  const second = await included(listed, after);
  assert.equal(second, first + 1, 'entries in the order they were retained');
  const unknown = await logLeaf(logEntryText({ name, community: keyHex(loser), proof: { version: 1, signature: '0'.repeat(128) } }));
  assert.equal((await get(`/v2/log/proof?leaf=${toHex(unknown)}&size=${after.size}`)).status, 404);
  assert.equal((await get(`/v2/log/proof?leaf=${toHex(await logLeaf(logEntryText(listed)))}&size=${second}`)).status, 404, 'not in a tree from before it');

  // The new head extends the old one.
  const consistency = await get(`/v2/log/consistency?first=${before.size}&second=${after.size}`);
  assert.equal(consistency.status, 200);
  assert.ok(await verifyConsistency(before.size, after.size, fromHex(before.root), fromHex(after.root), consistency.body.path), 'the log only grew');
  assert.equal(await verifyConsistency(before.size, after.size, fromHex(after.root), fromHex(after.root), consistency.body.path) && before.size > 0, false, 'a different old root does not check out');

  // Entries are served with their leaves; any app may read the log.
  const page = await get(`/v2/log/entries?start=${first}&end=${second + 1}`, { Origin: 'https://auditor.example' });
  assert.equal(page.status, 200);
  assert.equal(page.headers.get('access-control-allow-origin'), '*');
  assert.deepEqual(page.body.entries, [logEntryText({ name, community, proof: { version: 1, signature: reservation.signature } }), logEntryText(listed)]);
  assert.deepEqual(page.body.leaves, [toHex(await logLeaf(page.body.entries[0])), toHex(await logLeaf(page.body.entries[1]))]);
  if (hide) {
    await hide(name);
    const withheld = await get(`/v2/log/entries?start=${first}&end=${second + 1}`);
    assert.deepEqual(withheld.body.entries, [null, null], 'a withheld listing\'s text is not served');
    assert.deepEqual(withheld.body.leaves, page.body.leaves, 'its leaves are, so the tree still checks');
  }

  // Bounds.
  for (const path of ['/v2/log/head?x=1', `/v2/log/entries?start=0&end=${after.size + 1}`, '/v2/log/entries?start=0&end=257', '/v2/log/entries?start=1&end=1',
    '/v2/log/entries?start=-1&end=1', '/v2/log/entries?start=01&end=2', `/v2/log/proof?leaf=${'0'.repeat(64)}&size=0`, `/v2/log/proof?leaf=${'0'.repeat(64)}&size=${after.size + 1}`,
    '/v2/log/proof?leaf=xyz&size=1', `/v2/log/proof?leaf=${'0'.repeat(64)}&leaf=${'1'.repeat(64)}&size=1`, `/v2/log/consistency?first=2&second=1`,
    `/v2/log/consistency?first=1&second=${after.size + 1}`, '/v2/log/consistency?first=1']) {
    assert.equal((await get(path)).status, 400, path);
  }
  assert.equal((await request(`${root}/v2/log/head`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: '{}' })).status, 405);
}
