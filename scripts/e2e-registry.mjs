// Real compiled browser core -> signed claim -> HTTP registry -> durable core reload.
// Run after pnpm core:wasm. No browser, mock signatures or external service needed.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRegistryServer } from '../registry/server.mjs';
import { initSync, WasmNode } from '../apps/web/src/lib/core/wasm/pkg/hoi_wasm.js';

initSync({ module: readFileSync(new URL('../apps/web/src/lib/core/wasm/pkg/hoi_wasm_bg.wasm', import.meta.url)) });
const call = (node, method, ...args) => JSON.parse(node.call(method, JSON.stringify(args)));
const server = createRegistryServer({ filename: ':memory:' });
await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
const url = `http://127.0.0.1:${server.address().port}/v1/communities`;
const nodes = [];
try {
  const proofs = [];
  const publications = [];
  for (const name of ['Robin', 'Lena']) {
    const node = new WasmNode('web', 'https://hoi.test/app/', [], []);
    nodes.push(node);
    call(node, 'createIdentity', name);
    const community = call(node, 'createCommunity', { name: 'shared-name', description: '', color: 'mint', limits: { powBits: 12 } });
    proofs.push(call(node, 'communityRegistration', community.id));
    publications.push(call(node, 'communityPublication', community.id));
  }
  assert.notEqual(proofs[0].community, proofs[1].community);
  const put = (proof) => fetch(url, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(proof) });
  const claims = await Promise.all(proofs.map(put));
  assert.deepEqual(claims.map((r) => r.status).sort(), [200, 409]);
  const winner = proofs[claims.findIndex((r) => r.status === 200)];
  assert.equal((await put(winner)).status, 200);
  assert.deepEqual(await (await fetch(`${url}?name=shared-name`)).json(), { name: winner.name, community: winner.community, proof: { version: 1, signature: winner.signature } });
  const publish = (proof) => fetch(url.replace('/v1/', '/v2/'), { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(proof) });
  const published = await Promise.all(publications.map(publish));
  assert.deepEqual(published.map(r => r.status).sort(), [200, 409]);
  const listing = await published[claims.findIndex(r => r.status === 200)].json();
  assert.equal(listing.community, winner.community);
  assert.equal(listing.compatible, true);
  assert.equal(listing.indexed, true);
  assert.equal((await publish(publications[claims.findIndex(r => r.status === 200)])).status, 200);

  const records = new Map();
  const blobs = new Map();
  for (const change of nodes[0].take_changes()) {
    if (change.op === 'put') records.set(change.id, change.wire);
    if (change.op === 'delete') records.delete(change.id);
    if (change.op === 'blob') change.value ? blobs.set(change.key, change.value) : blobs.delete(change.key);
    if (change.op === 'wipe') { records.clear(); blobs.clear(); }
  }
  const reopened = new WasmNode('web', 'https://hoi.test/app/', [...records.values()], [...blobs]);
  nodes.push(reopened);
  assert.deepEqual(call(reopened, 'communityRegistration', proofs[0].community), proofs[0]);
  assert.deepEqual(call(reopened, 'communityPublication', proofs[0].community), publications[0]);
  console.log('PASS: real WASM v1/v2 claims, competing keys, versioned HTTP signatures, idempotent retry, and persisted proofs');
} finally {
  for (const node of nodes) node.free();
  await new Promise((resolve) => server.close(resolve));
}
