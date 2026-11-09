import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { mkdtemp, readdir, unlink, rmdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { MAX_OBJECT_BYTES, objectId, objectStatement, readStatement } from '../relay/protocol.mjs';
import { registrationStatement, announcementStatement } from '../registry/protocol.mjs';
import { createLocalRelay, createLocalRegistry, inspectPublicRecord } from './walkthrough-services.mjs';

const keypair = () => {
  const key = generateKeyPairSync('ed25519');
  return { ...key, id: key.publicKey.export({ type: 'spki', format: 'der' }).subarray(-32).toString('hex') };
};
const signed = (key, text) => sign(null, Buffer.from(text), key.privateKey).toString('hex');

async function temporary(t) {
  const directory = await mkdtemp(join(tmpdir(), 'hoi-walkthrough-services-'));
  t.after(async () => {
    // Only the flat files created in this test's fresh directory are removed.
    for (const name of await readdir(directory)) await unlink(join(directory, name));
    await rmdir(directory);
  });
  return directory;
}

async function listen(service) {
  await new Promise((resolve, reject) => {
    service.server.once('error', reject);
    service.server.listen(0, '127.0.0.1', resolve);
  });
  return `http://127.0.0.1:${service.server.address().port}`;
}

function envelope(room, bytes, expires = Math.floor(Date.now() / 1000) + 3600) {
  const id = objectId(bytes);
  return { id, expires, headers: { 'Content-Type': 'application/octet-stream', 'X-Hoi-Expires': String(expires),
    'X-Hoi-Signature': signed(room, objectStatement(room.id, id, expires, bytes.length)) } };
}

test('real relay requests persist opaque objects and the observation only returns bounded ciphertext', async t => {
  const directory = await temporary(t);
  const relay = createLocalRelay({ filename: join(directory, 'relay.db'), origins: ['http://127.0.0.1:7070'] });
  try {
    const url = await listen(relay), room = keypair(), bytes = randomBytes(4096), claim = envelope(room, bytes);
    const path = `/relay/v2/rooms/${room.id}/objects/${claim.id}`;
    assert.equal((await fetch(url + path, { method: 'PUT', headers: claim.headers, body: bytes })).status, 201);
    assert.equal((await fetch(url + path)).status, 401);
    const time = Math.floor(Date.now() / 1000), nonce = randomBytes(16).toString('hex');
    const downloaded = await fetch(url + path, { headers: {
      'X-Hoi-Read-Time': String(time), 'X-Hoi-Read-Nonce': nonce,
      'X-Hoi-Read-Signature': signed(room, readStatement(room.id, `objects/${claim.id}`, time, nonce)),
    } });
    assert.equal(downloaded.status, 200);
    assert.deepEqual(Buffer.from(await downloaded.arrayBuffer()), bytes);
    const snapshot = relay.snapshot({ plaintextCanary: 'A private fixture message.' });
    assert.equal(snapshot.containsCanary, false);
    assert.deepEqual(snapshot.records, [{ id: claim.id, room: room.id, sequence: 1, expires: claim.expires, size: 4096, ciphertextPrefix: bytes.subarray(0, 64).toString('hex') }]);
    assert.equal(snapshot.total, 1);
    assert.equal(snapshot.truncated, false);
    assert.doesNotMatch(JSON.stringify(snapshot), /privateKey|signature|digest|A private fixture message/);
    const forged = { ...claim.headers, 'X-Hoi-Signature': '0'.repeat(128) };
    assert.equal((await fetch(url + path, { method: 'PUT', headers: forged, body: bytes })).status, 400);
    assert.equal(relay.snapshot().total, 1);
  } finally { await relay.close(); }
  await relay.close();
  assert.throws(() => relay.snapshot(), /closed/);
});

test('selected relay inspection binds the room and checks every stored byte', async t => {
  const directory = await temporary(t), filename = join(directory, 'relay.db');
  const relay = createLocalRelay({ filename });
  try {
    const url = await listen(relay), room = keypair(), bytes = randomBytes(4096), claim = envelope(room, bytes);
    assert.deepEqual(relay.inspectObject({ id: claim.id, room: room.id, expectedBody: bytes }), { present: false });
    assert.equal((await fetch(`${url}/relay/v2/rooms/${room.id}/objects/${claim.id}`, { method: 'PUT', headers: claim.headers, body: bytes })).status, 201);
    const observed = relay.inspectObject({ id: claim.id, room: room.id, expectedBody: bytes });
    assert.deepEqual(observed, { present: true, matchesExpected: true, record: {
      id: claim.id, room: room.id, size: bytes.length, sequence: 1, expires: claim.expires,
      sha256: claim.id, hashMatchesId: true, ciphertextPrefix: bytes.subarray(0, 64).toString('hex'),
    } });
    assert.doesNotMatch(JSON.stringify(observed), /privateKey|signature|body/);
    const changedTail = Buffer.from(bytes);
    changedTail[changedTail.length - 1] ^= 1;
    assert.equal(relay.inspectObject({ id: claim.id, room: room.id, expectedBody: changedTail }).matchesExpected, false,
      'matching preview bytes cannot stand in for full-byte equality');
    assert.deepEqual(relay.inspectObject({ id: claim.id, room: 'f'.repeat(64), expectedBody: bytes }), { present: false });
    assert.equal(relay.inspectObject({ id: claim.id, room: room.id }).matchesExpected, undefined);
    assert.throws(() => relay.inspectObject({ id: "' OR 1=1", room: room.id }), /64-character/);
    assert.throws(() => relay.inspectObject({ id: claim.id, room: [] }), /64-character/);
    assert.throws(() => relay.inspectObject({ id: claim.id, room: room.id, expectedBody: bytes.toString('base64url') }), /byte array/);
    assert.throws(() => relay.inspectObject({ id: claim.id, room: room.id, expectedBody: new Uint8Array(MAX_OBJECT_BYTES + 1) }), /bounded/);
    // A changed local database must not be reported as the original, valid object.
    const db = new DatabaseSync(filename);
    try { db.prepare('UPDATE objects_v2 SET body = ? WHERE room = ? AND id = ?').run(changedTail, room.id, claim.id); }
    finally { db.close(); }
    const altered = relay.inspectObject({ id: claim.id, room: room.id, expectedBody: bytes });
    assert.equal(altered.record.sha256, objectId(changedTail));
    assert.equal(altered.record.hashMatchesId, false);
    assert.equal(altered.matchesExpected, false);
    const damaged = new DatabaseSync(filename);
    try {
      damaged.prepare('UPDATE objects_v2 SET size = ? WHERE room = ? AND id = ?').run(bytes.length + 1, room.id, claim.id);
      assert.throws(() => relay.inspectObject({ id: claim.id, room: room.id }), /bounded object size/);
      damaged.prepare('UPDATE objects_v2 SET size = ?, body = zeroblob(?) WHERE room = ? AND id = ?').run(MAX_OBJECT_BYTES + 1, MAX_OBJECT_BYTES + 1, room.id, claim.id);
      assert.throws(() => relay.inspectObject({ id: claim.id, room: room.id }), /bounded object size/);
    } finally { damaged.close(); }
  } finally { await relay.close(); }
  assert.throws(() => relay.inspectObject({ id: 'a'.repeat(64), room: 'b'.repeat(64) }), /closed/);
});

test('relay observations have row and byte limits and report an actual plaintext match', async t => {
  const directory = await temporary(t);
  const relay = createLocalRelay({ filename: join(directory, 'relay.db') });
  try {
    const url = await listen(relay), room = keypair();
    // A signed envelope does not prove encryption. This intentionally bad fixture
    // checks that the observer reports plaintext presence rather than claiming safety.
    const marker = 'readable fixture text';
    let oldest;
    for (let index = 0; index < 33; index++) {
      const bytes = randomBytes(4096);
      if (index === 0) bytes.write(marker, 2048);
      const claim = envelope(room, bytes);
      if (index === 0) oldest = { id: claim.id, room: room.id, expectedBody: bytes };
      assert.equal((await fetch(`${url}/relay/v2/rooms/${room.id}/objects/${claim.id}`, { method: 'PUT', headers: claim.headers, body: bytes })).status, 201);
    }
    const snapshot = relay.snapshot({ plaintextCanary: marker });
    assert.equal(snapshot.total, 33);
    assert.equal(snapshot.records.length, 32);
    assert.equal(snapshot.truncated, true);
    assert.equal(snapshot.containsCanary, true, 'the complete stored data is checked, including rows omitted from preview');
    assert.ok(snapshot.records.every(record => record.ciphertextPrefix.length === 128));
    assert.ok(!snapshot.records.some(record => record.id === oldest.id));
    assert.equal(relay.inspectObject(oldest).matchesExpected, true, 'selected object evidence is independent of the bounded recent-object preview');
    assert.throws(() => relay.snapshot({ plaintextCanary: '' }), /1–16384/);
    assert.throws(() => relay.snapshot({ plaintextCanary: 'x'.repeat(16385) }), /1–16384/);
  } finally { await relay.close(); }
});

test('restored relay objects expire on the alarm and close cancels its timer', async t => {
  const directory = await temporary(t), filename = join(directory, 'relay.db');
  let now = 1_800_000_000_000;
  const relay = createLocalRelay({ filename, now: () => now });
  try {
    const url = await listen(relay), room = keypair(), bytes = randomBytes(4096);
    const claim = envelope(room, bytes, Math.floor(now / 1000) + 1);
    assert.equal((await fetch(`${url}/relay/v2/rooms/${room.id}/objects/${claim.id}`, { method: 'PUT', headers: claim.headers, body: bytes })).status, 201);
  } finally { await relay.close(); }
  now += 1500;
  const restored = createLocalRelay({ filename, now: () => now });
  try {
    // Startup executes the production alarm, even before a server starts listening.
    const db = new DatabaseSync(filename, { readOnly: true });
    try { assert.equal(db.prepare('SELECT COUNT(*) AS count FROM objects_v2').get().count, 0); }
    finally { db.close(); }
  } finally { await restored.close(); }
});

test('local relay rejects remote origins and DNS rebinding hosts', async t => {
  const directory = await temporary(t);
  assert.throws(() => createLocalRelay({ filename: join(directory, 'bad.db'), origins: ['https://example.com'] }), /loopback/);
  assert.throws(() => createLocalRegistry({ filename: join(directory, 'bad.db'), origins: ['http://localhost:7070/'] }), /loopback/);
  const relay = createLocalRelay({ filename: join(directory, 'relay.db') });
  try {
    const url = await listen(relay);
    assert.equal((await fetch(url + '/relay/v2/meta', { headers: { Origin: 'https://example.com' } })).status, 403);
    const status = await new Promise((resolve, reject) => {
      const request = http.get(url + '/relay/v2/meta', { headers: { Host: 'example.com' } }, response => {
        response.resume(); response.on('end', () => resolve(response.statusCode));
      });
      request.on('error', reject);
    });
    assert.equal(status, 403);
  } finally { await relay.close(); }
});

test('the scheduled relay alarm expires the last object without another client request', async t => {
  const directory = await temporary(t), filename = join(directory, 'relay.db');
  let now = 1_800_000_000_980;
  const relay = createLocalRelay({ filename, now: () => now });
  try {
    const url = await listen(relay), room = keypair(), bytes = randomBytes(4096);
    const claim = envelope(room, bytes, Math.floor(now / 1000) + 1);
    assert.equal((await fetch(`${url}/relay/v2/rooms/${room.id}/objects/${claim.id}`, { method: 'PUT', headers: claim.headers, body: bytes })).status, 201);
    now += 100;
    const db = new DatabaseSync(filename, { readOnly: true });
    try {
      const deadline = Date.now() + 2000;
      while (db.prepare('SELECT COUNT(*) AS count FROM objects_v2').get().count && Date.now() < deadline) {
        await new Promise(resolve => setTimeout(resolve, 20));
      }
      assert.equal(db.prepare('SELECT COUNT(*) AS count FROM objects_v2').get().count, 0);
    } finally { db.close(); }
  } finally { await relay.close(); }
});

test('registry observations show real public registrations and announcements without log secrets', async t => {
  const directory = await temporary(t), filename = join(directory, 'registry.db');
  const registry = createLocalRegistry({ filename });
  try {
    const url = await listen(registry), community = keypair(), node = keypair(), name = 'walkthrough';
    const proof = { name, community: community.id, signature: signed(community, registrationStatement(name, community.id)) };
    assert.equal((await fetch(url + '/v1/communities', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(proof) })).status, 200);
    const routes = [`${node.id}@127.0.0.1:7843`], communities = [community.id], issuedAt = Math.floor(Date.now() / 1000);
    const announcement = { registry: url, endpoint: node.id, routes, communities, issuedAt,
      signature: signed(node, announcementStatement(url, node.id, issuedAt, routes, communities)) };
    assert.equal((await fetch(url + '/v2/nodes', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(announcement) })).status, 200);
    const snapshot = registry.snapshot();
    assert.deepEqual(snapshot.records, [
      { kind: 'community', name, community: community.id, protocolVersion: null, indexVersion: null },
      { kind: 'node', endpoint: node.id, routes, communities, issuedAt },
    ]);
    assert.equal(snapshot.total, 2);
    const db = new DatabaseSync(filename, { readOnly: true });
    try {
      const secret = db.prepare('SELECT secret FROM log_secret').get().secret;
      assert.ok(!JSON.stringify(snapshot).includes(secret));
    } finally { db.close(); }
    assert.doesNotMatch(JSON.stringify(snapshot), /signature|log_secret|privateKey/);
  } finally { await registry.close(); }
  await registry.close();
});

test('public record inspection selects one ID, bounds wire previews and never reads identity blobs', async t => {
  const directory = await temporary(t), filename = join(directory, 'hoi.db');
  const id = 'a'.repeat(64), community = 'b'.repeat(64), secret = 'identity-secret-fixture', marker = 'public post fixture';
  assert.deepEqual(inspectPublicRecord({ filename, postId: id }), { present: false });
  const db = new DatabaseSync(filename);
  try {
    db.exec('CREATE TABLE records(id BLOB PRIMARY KEY, community BLOB, expires_at INTEGER NOT NULL, wire BLOB NOT NULL); CREATE TABLE blobs(key TEXT PRIMARY KEY, value BLOB NOT NULL)');
    db.prepare('INSERT INTO blobs VALUES (?, ?)').run('identity', Buffer.from(secret));
    db.prepare('INSERT INTO records VALUES (?, ?, ?, ?)').run(Buffer.from(id, 'hex'), Buffer.from(community, 'hex'), 1_800_003_600, Buffer.from('x'.repeat(1024) + marker));
  } finally { db.close(); }
  const result = inspectPublicRecord({ filename, postId: id, plaintextCanary: marker });
  assert.equal(result.present, true);
  assert.equal(result.containsCanary, true);
  assert.equal(result.record.bytes, 1024 + marker.length);
  assert.equal(result.record.wireHex.length, 1024);
  assert.equal(result.record.wireText.length, 512);
  assert.equal(result.record.community, community);
  assert.ok(!JSON.stringify(result).includes(secret));
  assert.deepEqual(inspectPublicRecord({ filename, postId: 'c'.repeat(64) }), { present: false });
  assert.throws(() => inspectPublicRecord({ filename, postId: "' OR 1=1" }), /64-character/);
});

test('accepted multibyte messages remain inspectable in relay and public-node stores', async t => {
  const directory = await temporary(t), filename = join(directory, 'hoi.db');
  const id = 'c'.repeat(64), marker = '你好'.repeat(2000);
  assert.equal(marker.length, 4000);
  assert.equal(Buffer.byteLength(marker), 12000);
  const db = new DatabaseSync(filename);
  try {
    db.exec('CREATE TABLE records(id BLOB PRIMARY KEY, community BLOB, expires_at INTEGER NOT NULL, wire BLOB NOT NULL)');
    db.prepare('INSERT INTO records VALUES (?, ?, ?, ?)').run(Buffer.from(id, 'hex'), Buffer.from('d'.repeat(64), 'hex'), 1_800_003_600, Buffer.from(marker));
  } finally { db.close(); }
  assert.equal(inspectPublicRecord({ filename, postId: id, plaintextCanary: marker }).containsCanary, true);
  const relay = createLocalRelay({ filename: join(directory, 'relay.db') });
  try {
    const url = await listen(relay), room = keypair(), bytes = randomBytes(12288);
    bytes.write(marker);
    const claim = envelope(room, bytes);
    assert.equal((await fetch(`${url}/relay/v2/rooms/${room.id}/objects/${claim.id}`, { method: 'PUT', headers: claim.headers, body: bytes })).status, 201);
    assert.equal(relay.snapshot({ plaintextCanary: marker }).containsCanary, true);
  } finally { await relay.close(); }
});
