import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { createCipheriv, createPublicKey, generateKeyPairSync, randomBytes, sign, verify } from 'node:crypto';
import { mkdtempSync, readdirSync, rmdirSync, unlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { RelayObject, fetchRelay } from './worker-runtime.mjs';
import { MAX_OBJECT_BYTES, MAX_TTL_SECONDS, MIN_OBJECT_BYTES, READ_WINDOW_SECONDS, isPaddedSize, objectId, objectStatement, paddedSize, readStatement, removals, verifyRead } from './protocol.mjs';

const ORIGIN = 'https://hoi.example';
const NOW = 1_800_000_000;
const keypair = () => {
  const key = generateKeyPairSync('ed25519');
  return { ...key, id: key.publicKey.export({ type: 'spki', format: 'der' }).subarray(-32).toString('hex') };
};
const signed = (key, statement) => sign(null, Buffer.from(statement), key.privateKey).toString('hex');
const route = (room, id) => `${ORIGIN}/relay/v2/rooms/${room}/objects${id ? '/' + id : ''}`;
const padded = (size = MIN_OBJECT_BYTES) => randomBytes(paddedSize(size));

function envelope(room, bytes, expires = NOW + 3600) {
  const id = objectId(bytes);
  return { bytes, room: room.id, id, headers: {
    'Content-Type': 'application/octet-stream', 'X-Hoi-Expires': String(expires),
    'X-Hoi-Signature': signed(room, objectStatement(room.id, id, expires, bytes.byteLength)),
  } };
}

function reading(room, target, time = NOW, nonce = randomBytes(16).toString('hex'), key = room) {
  return { 'X-Hoi-Read-Time': String(time), 'X-Hoi-Read-Nonce': nonce, 'X-Hoi-Read-Signature': signed(key, readStatement(room.id, target, time, nonce)) };
}

function adapter(db) {
  let alarm = null;
  return {
    get alarmTime() { return alarm; },
    sql: { exec(query, ...args) { return db.prepare(query).all(...args.map(value => value instanceof ArrayBuffer ? new Uint8Array(value) : value)); } },
    transactionSync(callback) {
      db.exec('BEGIN IMMEDIATE');
      try { const value = callback(); db.exec('COMMIT'); return value; }
      catch (error) { db.exec('ROLLBACK'); throw error; }
    },
    async setAlarm(value) { alarm = value; },
    async deleteAlarm() { alarm = null; },
  };
}

function instance(db, env = {}, now = () => NOW * 1000) {
  const storage = adapter(db);
  const relay = new RelayObject({ storage }, { HOI_RELAY_REQUESTS_PER_MINUTE: '10000', HOI_RELAY_WRITES_PER_MINUTE: '10000', ...env }, now);
  const at = () => Math.floor(now() / 1000);
  return { relay, storage,
    get: (room, id, headers = reading(room, `objects/${id}`, at())) => relay.fetch(new Request(route(room.id, id), { headers })),
    list: (room, query = '?after=0', headers = reading(room, `objects${query}`, at())) => relay.fetch(new Request(route(room.id) + query, { headers })),
    put: (claim, headerOverrides = {}) => relay.fetch(new Request(route(claim.room, claim.id), { method: 'PUT', headers: { ...claim.headers, ...headerOverrides }, body: claim.bytes })),
  };
}

function start(t, env, now) {
  const db = new DatabaseSync(':memory:');
  t.after(() => db.close());
  return { db, ...instance(db, env, now) };
}

const count = (db, table = 'objects_v2') => db.prepare(`SELECT COUNT(*) AS count FROM ${table}`).get().count;

test('statements, padding and read checks match the core (hoi-core relay_v2_shared_vector)', () => {
  // The room key derived in the core from the exporter secret [7; 32]; signatures are deterministic.
  const room = 'c7c825f514ccb7c593e05a62df9f4d376bc1f1015580caf4c5cca3f36c486d36';
  const object = '7b6eca5889483df1a0386d9b3027db90c87af5ceba4d94c4041ecf162dcbb25cadb42b8f80a9b1ce6329c5a59ecf60044fc79ee3aee673debb968b75a6d4b50b';
  const read = '525ce23556d6718103e9b49970dd0976c5569bf5951dc8136ada36794c7246a8fa0ce7e70f9c5c152537485744095e0fea4d8f032dda4d843989e756becf0304';
  const key = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(room, 'hex')]), format: 'der', type: 'spki' });
  assert.equal(verify(null, Buffer.from(objectStatement(room, '5'.repeat(64), 1_800_003_600, 8192)), key, Buffer.from(object, 'hex')), true);
  const headers = new Headers({ 'X-Hoi-Read-Time': '1800000000', 'X-Hoi-Read-Nonce': '0f'.repeat(16), 'X-Hoi-Read-Signature': read });
  assert.ok(verifyRead(room, 'objects?after=0', headers, 1_800_000_000));
  assert.equal(verifyRead(room, 'objects?after=1', headers, 1_800_000_000), null);
  assert.deepEqual([1, 4096, 4097, 8191, 8193, 15361, MAX_OBJECT_BYTES].map(paddedSize), [4096, 4096, 4608, 8192, 9216, 16384, MAX_OBJECT_BYTES]);
});

test('standalone and combined routing use one separate opaque storage binding', async () => {
  const names = [];
  const env = { OPAQUE_RELAY: { idFromName(name) { names.push(name); return 'opaque'; }, get(id) { assert.equal(id, 'opaque'); return { fetch: request => new Response(request.url) }; } } };
  const response = await fetchRelay(new Request(ORIGIN + '/relay/v2/meta'), env);
  assert.equal(await response.text(), ORIGIN + '/relay/v2/meta');
  assert.deepEqual(names, ['opaque-objects']);
  assert.equal((await fetchRelay(new Request(ORIGIN + '/registry/v1/meta'), env)).status, 404);
  const missing = await fetchRelay(new Request(ORIGIN + '/relay/v2/meta'), {});
  assert.equal(missing.status, 503);
  assert.doesNotMatch(await missing.text(), /binding|OPAQUE_RELAY|TypeError/);
});

test('room-signed ciphertext is immutable, readable only with a room signature, and stores no writer or caller identifiers', async (t) => {
  const server = start(t);
  const room = keypair();
  const cipher = createCipheriv('aes-256-gcm', randomBytes(32), randomBytes(12));
  const secret = 'A private message that the storage operator must not receive.';
  const sealed = Buffer.concat([cipher.update(secret), cipher.final(), cipher.getAuthTag()]);
  const bytes = Buffer.concat([sealed, Buffer.alloc(paddedSize(sealed.length) - sealed.length)]);
  const claim = envelope(room, bytes);
  const created = await server.put(claim, { Origin: ORIGIN, 'X-Forwarded-For': '198.51.100.12', Cookie: 'secret=not-stored' });
  assert.equal(created.status, 201);
  const entry = await created.json();
  assert.deepEqual(entry, { id: claim.id, sequence: 1, size: bytes.length, expires: NOW + 3600 });
  assert.equal(created.headers.get('Cache-Control'), 'no-store');
  assert.equal(created.headers.get('Access-Control-Allow-Origin'), ORIGIN);
  assert.equal(created.headers.get('Access-Control-Allow-Credentials'), null);
  assert.equal(server.storage.alarmTime, (NOW + 3600) * 1000);
  assert.equal((await server.put(claim)).status, 200);
  assert.equal((await server.put(envelope(room, bytes, NOW + 7200))).status, 409, 'even the room cannot replace the envelope');
  const downloaded = await server.get(room, claim.id);
  assert.equal(downloaded.status, 200);
  assert.equal(downloaded.headers.get('Content-Type'), 'application/octet-stream');
  for (const name of ['X-Hoi-Signature', 'X-Hoi-Author', 'X-Hoi-Expires', 'X-Hoi-Sequence', 'X-Hoi-Grant-Signature', 'Access-Control-Expose-Headers']) assert.equal(downloaded.headers.get(name), null, name);
  assert.deepEqual(Buffer.from(await downloaded.arrayBuffer()), bytes);
  assert.deepEqual(await (await server.list(room)).json(), { objects: [{ id: claim.id, sequence: 1, size: bytes.length }] }, 'readers get no expiry, signature or writer');
  const stored = server.db.prepare('SELECT * FROM objects_v2').get();
  assert.equal(Buffer.from(stored.body).includes(Buffer.from(secret)), false);
  assert.deepEqual(Object.keys(stored).sort(), ['body', 'expires', 'id', 'room', 'sequence', 'size']);
  assert.doesNotMatch(JSON.stringify(stored), /198\.51\.100\.12|not-stored|private message/);
  const reads = server.db.prepare('SELECT * FROM reads').all();
  assert.equal(reads.length, 2);
  for (const read of reads) assert.deepEqual(Object.keys(read).sort(), ['digest', 'expires']);
  assert.doesNotMatch(JSON.stringify(reads), new RegExp(room.id));
});

test('listing and fetching require a fresh signature by the room key for that exact request', async (t) => {
  const server = start(t);
  const room = keypair();
  const claim = envelope(room, padded());
  assert.equal((await server.put(claim)).status, 201);
  const stranger = keypair();
  const refused = [
    {}, { 'X-Hoi-Read-Time': String(NOW), 'X-Hoi-Read-Nonce': '0'.repeat(32) },
    reading(room, 'objects?after=0', NOW, undefined, stranger),
    reading(room, 'objects?after=1'), reading(room, `objects/${claim.id}`),
    reading(room, 'objects?after=0', NOW - READ_WINDOW_SECONDS - 1), reading(room, 'objects?after=0', NOW + READ_WINDOW_SECONDS + 1),
    reading(room, 'objects?after=0', NOW, 'A'.repeat(32)), reading(room, 'objects?after=0', NOW, 'ab'),
    { ...reading(room, 'objects?after=0'), 'X-Hoi-Read-Time': '1e9' },
  ];
  for (const headers of refused) {
    const response = await server.list(room, '?after=0', headers);
    assert.equal(response.status, 401);
    assert.doesNotMatch(await response.text(), new RegExp(claim.id));
  }
  for (const headers of [{}, reading(room, 'objects?after=0'), reading(room, `objects/${claim.id}`, NOW, undefined, stranger)]) {
    assert.equal((await server.get(room, claim.id, headers)).status, 401);
  }
  assert.equal((await server.list(stranger, '?after=0', reading(room, 'objects?after=0'))).status, 401, 'a signature for one room opens no other');
  assert.equal((await server.list(room, '?after=0', reading(room, 'objects?after=0', NOW - READ_WINDOW_SECONDS))).status, 200, 'the window is inclusive');
  assert.equal((await server.list(stranger)).status, 200, 'anyone holding a room key may list it, even if it is empty');
  assert.deepEqual(await (await server.list(stranger)).json(), { objects: [] });
});

test('a replayed read signature is refused, also after the relay restarts, and digests expire with the window', async (t) => {
  let time = NOW * 1000;
  const server = start(t, {}, () => time);
  const room = keypair();
  const claim = envelope(room, padded());
  assert.equal((await server.put(claim)).status, 201);
  const listing = reading(room, 'objects?after=0');
  const fetching = reading(room, `objects/${claim.id}`);
  assert.equal((await server.list(room, '?after=0', listing)).status, 200);
  assert.equal((await server.get(room, claim.id, fetching)).status, 200);
  assert.equal((await server.list(room, '?after=0', listing)).status, 401);
  assert.equal((await server.get(room, claim.id, fetching)).status, 401);
  const restarted = instance(server.db, {}, () => time);
  assert.equal((await restarted.list(room, '?after=0', listing)).status, 401, 'replay protection survives an eviction');
  assert.equal((await restarted.get(room, claim.id, fetching)).status, 401);
  assert.equal(restarted.storage.alarmTime, null, 'a fresh adapter has no alarm yet');
  assert.equal((await restarted.list(room)).status, 200);
  assert.equal(restarted.storage.alarmTime, (NOW + READ_WINDOW_SECONDS + 1) * 1000, 'the next read digest expiry is scheduled');
  time = (NOW + READ_WINDOW_SECONDS + 1) * 1000;
  await restarted.relay.alarm();
  assert.equal(count(server.db, 'reads'), 0);
  assert.equal(restarted.storage.alarmTime, (NOW + 3600) * 1000);
  assert.equal((await restarted.list(room, '?after=0', listing)).status, 401, 'an old signature is outside the window');
});

test('fragmented streams are reconstructed exactly and bounded before persistence', async (t) => {
  const server = start(t);
  const room = keypair();
  const claim = envelope(room, padded(8192));
  let offset = 0;
  const fragmented = new ReadableStream({ pull(controller) {
    if (offset === claim.bytes.length) return controller.close();
    controller.enqueue(claim.bytes.subarray(offset, ++offset));
  } });
  const response = await server.relay.fetch(new Request(route(claim.room, claim.id), { method: 'PUT', headers: claim.headers, body: fragmented, duplex: 'half' }));
  assert.equal(response.status, 201);
  assert.deepEqual(Buffer.from(await (await server.get(room, claim.id)).arrayBuffer()), claim.bytes);
  let cancelled = false;
  let chunk = 0;
  const oversized = new ReadableStream({ pull(controller) { controller.enqueue(new Uint8Array(chunk++ ? 1 : MAX_OBJECT_BYTES)); }, cancel() { cancelled = true; } });
  const rejected = await server.relay.fetch(new Request(route(claim.room, claim.id), { method: 'PUT', headers: claim.headers, body: oversized, duplex: 'half' }));
  assert.equal(rejected.status, 413);
  assert.equal(cancelled, true);
  assert.equal(count(server.db), 1);
});

test('only padded sizes are stored: at least 4 KiB, four significant bits, at most 1 MiB', async (t) => {
  const sizes = [];
  for (let size = 1; size <= MAX_OBJECT_BYTES; size += size < 65536 ? 97 : 4099) {
    const bucket = paddedSize(size);
    assert.ok(isPaddedSize(bucket), `${size} → ${bucket}`);
    assert.ok(bucket >= size && bucket <= Math.max(MIN_OBJECT_BYTES, size * 9 / 8), `bounded overhead for ${size}`);
    sizes.push(bucket);
  }
  assert.equal(new Set(sizes).size, 65, 'sixty-five sizes from 4 KiB to 1 MiB');
  assert.deepEqual([4096, 4608, 7680, 8192, 9216, 15360, 16384, 917504, 1048576].map(isPaddedSize), Array(9).fill(true));
  assert.deepEqual([0, 1, 4095, 4097, 4100, 8704, 1048575, 1048577, 1179648, 4096.5, -4096].map(isPaddedSize), Array(11).fill(false));
  assert.equal(paddedSize(MAX_OBJECT_BYTES + 1) > MAX_OBJECT_BYTES, true, 'clients must refuse what cannot be padded');
  const server = start(t);
  const room = keypair();
  for (const size of [1, 64, 4095, 4097, 5000, 8193]) {
    const response = await server.put(envelope(room, randomBytes(size)));
    assert.equal(response.status, 400);
    assert.equal((await response.json()).error, 'unpadded');
  }
  for (const size of [4096, 4608, 9216, MAX_OBJECT_BYTES]) assert.equal((await server.put(envelope(room, randomBytes(size)))).status, 201);
  assert.equal(count(server.db), 4);
});

test('version 1 paths are gone, v1 headers are refused, and v1 objects stay unserved until they expire', async (t) => {
  let time = NOW * 1000;
  const db = new DatabaseSync(':memory:');
  t.after(() => db.close());
  db.exec('CREATE TABLE relay_schema (version INTEGER NOT NULL) STRICT; INSERT INTO relay_schema(version) VALUES (1)');
  db.exec(`CREATE TABLE objects (sequence INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT NOT NULL, id TEXT NOT NULL, author TEXT NOT NULL,
    expires INTEGER NOT NULL, size INTEGER NOT NULL CHECK(size > 0), signature TEXT NOT NULL, grant_expires INTEGER, grant_signature TEXT,
    body BLOB NOT NULL, UNIQUE(room, id)) STRICT`);
  const legacy = keypair();
  const insert = db.prepare('INSERT INTO objects(room, id, author, expires, size, signature, body) VALUES (?, ?, ?, ?, ?, ?, ?)');
  insert.run(legacy.id, 'a'.repeat(64), legacy.id, NOW + 10, 3, 'b'.repeat(128), Buffer.from('old'));
  insert.run(legacy.id, 'c'.repeat(64), legacy.id, NOW + 20, 4097, 'b'.repeat(128), randomBytes(4097));
  const server = { db, ...instance(db, { HOI_RELAY_MAX_BYTES: String(12288), HOI_RELAY_MAX_OBJECTS: '3' }, () => time) };
  assert.equal(db.prepare('SELECT version FROM relay_schema').get().version, 2);
  for (const [method, path] of [['GET', '/relay/v1/meta'], ['GET', `/relay/v1/rooms/${legacy.id}/objects`], ['PUT', `/relay/v1/rooms/${legacy.id}/objects/${'a'.repeat(64)}`], ['OPTIONS', '/relay/v1/meta']]) {
    const response = await server.relay.fetch(new Request(ORIGIN + path, { method }));
    assert.equal(response.status, 410, path);
    assert.equal((await response.json()).error, 'protocol-gone');
  }
  assert.deepEqual(await (await server.list(legacy)).json(), { objects: [] }, 'a v1 namespace key lists nothing');
  assert.equal((await server.get(legacy, 'a'.repeat(64))).status, 404);
  const room = keypair();
  const claim = envelope(room, padded(8192));
  for (const headers of [{ 'X-Hoi-Author': room.id }, { 'X-Hoi-Grant-Expires': String(NOW + 10) }, { 'X-Hoi-Grant-Signature': 'b'.repeat(128) }]) {
    assert.equal((await server.put(claim, headers)).status, 400);
  }
  assert.equal((await server.put(claim)).status, 507, 'legacy bytes still count toward the service quota');
  time = (NOW + 21) * 1000;
  await server.relay.alarm();
  assert.equal(count(db, 'objects'), 0, 'v1 objects are deleted at their expiry');
  assert.equal((await server.put(claim)).status, 201);
  const meta = await (await server.relay.fetch(new Request(ORIGIN + '/relay/v2/meta'))).json();
  assert.equal(meta.apiVersion, 2);
  assert.equal(meta.minObjectBytes, MIN_OBJECT_BYTES);
  assert.equal(meta.readWindowSeconds, READ_WINDOW_SECONDS);
});

test('operator removals delete identified rooms and objects at start, block their return and survive restarts', async (t) => {
  const db = new DatabaseSync(':memory:');
  t.after(() => db.close());
  const first = instance(db);
  const room = keypair();
  const other = keypair();
  const kept = envelope(room, padded());
  const target = envelope(room, padded());
  const whole = envelope(other, padded());
  for (const claim of [kept, target, whole]) assert.equal((await first.put(claim)).status, 201);
  db.prepare('INSERT INTO objects(room, id, author, expires, size, signature, body) VALUES (?, ?, ?, ?, ?, ?, ?)').run(room.id, 'e'.repeat(64), room.id, NOW + 10, 3, 'b'.repeat(128), Buffer.from('old'));
  assert.throws(() => instance(db, { HOI_RELAY_PURGE: `${room.id}/not-an-id` }), /Invalid HOI_RELAY_PURGE/);
  assert.throws(() => removals('DROP TABLE objects'), /Invalid HOI_RELAY_PURGE/);
  assert.deepEqual(removals(` ${other.id},\n${room.id}/${target.id} v1 ${other.id}`), [other.id, `${room.id}/${target.id}`, 'v1']);
  const purged = instance(db, { HOI_RELAY_PURGE: `${other.id}\n${room.id}/${target.id},v1` });
  assert.deepEqual(db.prepare('SELECT room, id FROM objects_v2').all().map(row => ({ ...row })), [{ room: room.id, id: kept.id }]);
  assert.equal(count(db, 'objects'), 0);
  assert.equal((await purged.put(target)).status, 410, 'the same object cannot be stored again');
  assert.equal((await purged.put(envelope(other, padded()))).status, 410, 'a removed room accepts nothing');
  assert.equal((await purged.list(other)).status, 410);
  assert.equal((await purged.list(other, '?after=0', {})).status, 401, 'only a room key learns that its room was removed');
  assert.equal((await purged.get(room, target.id)).status, 410);
  assert.equal((await purged.get(room, kept.id)).status, 200);
  assert.deepEqual((await (await purged.list(room)).json()).objects.map(item => item.id), [kept.id]);
  const meta = async (server) => (await (await server.relay.fetch(new Request(ORIGIN + '/relay/v2/meta'))).json()).removed;
  assert.equal(await meta(purged), 2);
  db.prepare('INSERT INTO objects_v2(room, id, expires, size, body) VALUES (?, ?, ?, ?, ?)').run(other.id, whole.id, NOW + 10, whole.bytes.length, whole.bytes);
  const restarted = instance(db);
  assert.equal(count(db), 1, 'restored rows are purged again on start without the setting');
  assert.equal(await meta(restarted), 2);
  assert.equal((await restarted.put(target)).status, 410);
});

test('competing valid envelopes choose one immutable object and quota admission is serialized', async (t) => {
  const server = start(t, { HOI_RELAY_ROOM_OBJECTS: '1' });
  const room = keypair();
  const bytes = padded();
  const claims = Array.from({ length: 8 }, (_, index) => envelope(room, bytes, NOW + 100 + index));
  const results = await Promise.all(claims.map(claim => server.put(claim)));
  assert.equal(results.filter(response => response.status === 201).length, 1);
  assert.equal(results.filter(response => response.status === 409).length, 7);
  const winner = claims[results.findIndex(response => response.status === 201)];
  const original = await (await server.list(room)).json();
  const otherObjects = await Promise.all(Array.from({ length: 8 }, () => server.put(envelope(room, padded()))));
  assert.equal(otherObjects.every(response => response.status === 507), true);
  assert.equal((await server.put(winner)).status, 200);
  assert.deepEqual(await (await server.list(room)).json(), original);
});

test('bad hash, forged envelope, small-order room, invalid expiry, oversized body and mutation methods are refused', async (t) => {
  const server = start(t);
  const room = keypair();
  const claim = envelope(room, padded());
  const smallOrder = '0100000000000000000000000000000000000000000000000000000000000000';
  for (const changed of [
    { ...claim, id: 'a'.repeat(64) }, { ...claim, bytes: padded() }, { ...claim, room: keypair().id }, { ...claim, room: smallOrder },
    envelope(room, padded(), NOW), envelope(room, padded(), NOW + MAX_TTL_SECONDS + 1),
  ]) assert.equal((await server.put(changed)).status, 400);
  assert.equal((await server.put(claim, { 'X-Hoi-Signature': 'f'.repeat(128) })).status, 400);
  assert.equal((await server.put(claim, { 'X-Hoi-Expires': '1e10' })).status, 400);
  assert.equal((await server.put(claim, { 'Content-Type': 'application/json' })).status, 415);
  assert.equal((await server.put(envelope(room, randomBytes(MAX_OBJECT_BYTES + 1)))).status, 413);
  const oversized = new Request(route(room.id, claim.id), { method: 'PUT', headers: { ...claim.headers, 'Content-Length': String(MAX_OBJECT_BYTES + 1) }, body: claim.bytes });
  assert.equal((await server.relay.fetch(oversized)).status, 413);
  for (const method of ['DELETE', 'POST', 'PATCH']) assert.equal((await server.relay.fetch(new Request(route(room.id, claim.id), { method }))).status, 405);
  assert.equal((await server.relay.fetch(new Request(route(room.id), { method: 'PUT', body: claim.bytes }))).status, 405);
  assert.equal((await server.get(room, claim.id)).status, 404);
  assert.equal(count(server.db), 0);
});

test('global and room quotas are atomic and expiry releases both bytes and object counts', async (t) => {
  let time = NOW * 1000;
  const page = MIN_OBJECT_BYTES;
  const server = start(t, { HOI_RELAY_MAX_BYTES: String(5 * page), HOI_RELAY_ROOM_BYTES: String(3 * page), HOI_RELAY_ROOM_OBJECTS: '2', HOI_RELAY_MAX_OBJECTS: '3' }, () => time);
  const room = keypair();
  const other = keypair();
  const first = envelope(room, randomBytes(2 * page), NOW + 10);
  const second = envelope(room, padded(), NOW + 10);
  assert.equal((await server.put(first)).status, 201);
  assert.equal((await server.put(second)).status, 201);
  assert.equal((await server.put(first)).status, 200, 'identical retries work when quota is full');
  assert.equal((await server.put(envelope(room, padded()))).status, 507);
  assert.equal((await server.put(envelope(other, randomBytes(3 * page)))).status, 507);
  assert.equal((await server.put(envelope(other, randomBytes(2 * page), NOW + 20))).status, 201);
  assert.equal((await server.put(envelope(keypair(), padded()))).status, 507);
  time = (NOW + 11) * 1000;
  await server.relay.alarm();
  assert.equal(server.storage.alarmTime, (NOW + 20) * 1000);
  assert.equal((await server.get(room, first.id)).status, 404);
  assert.deepEqual(await (await server.list(room)).json(), { objects: [] });
  assert.equal((await server.put(envelope(room, padded(), NOW + 100))).status, 201);
  time = (NOW + 101) * 1000;
  await server.relay.alarm();
  assert.equal(server.storage.alarmTime, (NOW + 11 + READ_WINDOW_SECONDS + 1) * 1000, 'only read digests remain');
  time = (NOW + 11 + READ_WINDOW_SECONDS + 1) * 1000;
  await server.relay.alarm();
  assert.equal(server.storage.alarmTime, null);
  assert.equal(count(server.db) + count(server.db, 'reads'), 0);
});

test('pagination has a stable exclusive sequence cursor and rejects ambiguous queries', async (t) => {
  const server = start(t);
  const room = keypair();
  for (let i = 0; i < 66; i++) assert.equal((await server.put(envelope(room, padded()))).status, 201);
  const first = await (await server.list(room)).json();
  assert.equal(first.objects.length, 64);
  assert.equal(first.nextCursor, 64);
  const second = await (await server.list(room, '?after=' + first.nextCursor)).json();
  assert.deepEqual(second.objects.map(item => item.sequence), [65, 66]);
  assert.equal(second.nextCursor, undefined);
  assert.deepEqual(await (await server.list(room, '?after=999')).json(), { objects: [] });
  for (const query of ['?after=-1', '?after=1e3', '?after=1&after=2', '?name=hello']) assert.equal((await server.list(room, query)).status, 400);
  assert.deepEqual(await (await server.list(keypair())).json(), { objects: [] });
});

test('ciphertext and sequence survive restarts; unsupported schemas fail closed', async (t) => {
  const folder = mkdtempSync(join(tmpdir(), 'hoi-relay-'));
  const filename = join(folder, 'opaque.db');
  let db = new DatabaseSync(filename);
  t.after(() => { db.close(); for (const file of readdirSync(folder)) unlinkSync(join(folder, file)); rmdirSync(folder); });
  const room = keypair();
  const claim = envelope(room, padded(5000));
  const entry = await (await instance(db).put(claim)).json();
  db.close();
  db = new DatabaseSync(filename);
  const reopened = instance(db);
  assert.deepEqual(await (await reopened.list(room)).json(), { objects: [{ id: entry.id, sequence: entry.sequence, size: entry.size }] });
  assert.deepEqual(Buffer.from(await (await reopened.get(room, claim.id)).arrayBuffer()), claim.bytes);
  assert.equal((await (await reopened.put(envelope(room, padded()))).json()).sequence, entry.sequence + 1);
  db.exec('UPDATE relay_schema SET version = 3');
  assert.throws(() => instance(db), /Unsupported encrypted storage schema/);
  assert.equal(db.prepare('SELECT version FROM relay_schema').get().version, 3);
});

test('CORS, limits, bounded configuration and generic storage failures preserve the trust boundary', async (t) => {
  let now = NOW * 1000;
  const server = start(t, { HOI_RELAY_ORIGINS: 'tauri://localhost', HOI_RELAY_WRITES_PER_MINUTE: '1' }, () => now);
  const claim = envelope(keypair(), padded());
  for (const origin of ['null', 'https://other.example']) assert.equal((await server.put(claim, { Origin: origin })).status, 403);
  const preflight = await server.relay.fetch(new Request(route(claim.room, claim.id), { method: 'OPTIONS', headers: { Origin: 'tauri://localhost', 'Access-Control-Request-Method': 'PUT' } }));
  assert.equal(preflight.status, 204);
  assert.match(preflight.headers.get('Access-Control-Allow-Headers'), /X-Hoi-Read-Signature/);
  assert.doesNotMatch(preflight.headers.get('Access-Control-Allow-Headers'), /Grant|Author/);
  assert.equal(preflight.headers.get('Access-Control-Max-Age'), '600');
  assert.equal((await server.put(claim, { Origin: 'tauri://localhost' })).status, 201);
  const limited = await server.put(claim);
  assert.equal(limited.status, 429);
  assert.equal(limited.headers.get('Retry-After'), '60');
  now += 60_001;
  assert.equal((await server.put(claim)).status, 200);
  const meta = await (await server.relay.fetch(new Request(ORIGIN + '/relay/v2/meta'))).json();
  assert.equal(meta.maxObjectBytes, MAX_OBJECT_BYTES);
  assert.equal(meta.maxTtlSeconds, MAX_TTL_SECONDS);
  assert.throws(() => instance(server.db, { HOI_RELAY_MAX_BYTES: String(512 * 1024 * 1024 + 1) }), /Invalid/);
  assert.throws(() => instance(server.db, { HOI_RELAY_ORIGINS: 'https://example/path' }), /Invalid/);
  server.db.exec("CREATE TRIGGER fail_insert BEFORE INSERT ON objects_v2 BEGIN SELECT RAISE(ABORT, 'private storage path'); END");
  now += 60_001;
  const failed = await server.put(envelope(keypair(), padded()));
  assert.equal(failed.status, 503);
  assert.doesNotMatch(await failed.text(), /private storage|SQLite|trigger|database/);
  assert.equal(count(server.db), 1);
});
