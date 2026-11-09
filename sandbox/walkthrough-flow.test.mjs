import test from 'node:test';
import assert from 'node:assert/strict';
import { describeFlow } from './walkthrough-flow.mjs';

const topology = { enforced: true, nativeEntry: { configured: true, source: 'Public node link supplied with --node.' }, clients: [
  { id: 'alice', node: 'node-a', communityConnected: true }, { id: 'bob', node: 'node-b', communityConnected: true },
] };
const availability = ['relay-a', 'relay-b'].map(service => ({ actor: 'alice', service, status: 200, apiVersion: 2, path: '/relay/v2/meta' }));
const object = { alias: 'E1', id: 'a'.repeat(64), size: 4096 };
const journey = stage => ({ author: 'alice', stage, objects: [object], stored: [], uploads: [], downloads: [] });

test('private setup distinguishes configured addresses, actual availability and message delivery', () => {
  let flow = describeFlow({ visibility: 'private' });
  assert.ok(flow.discovery.every(item => item.status === 'configured' && !item.available.length));
  assert.ok(flow.edges.filter(edge => edge.kind === 'delivery').every(edge => edge.status === 'pending'));
  flow = describeFlow({ visibility: 'private', availability });
  assert.ok(flow.discovery.every(item => item.status === 'observed'));
  assert.ok(flow.nodes.every(node => node.kind !== 'registry' && node.kind !== 'node'));
  assert.deepEqual(flow.edges.filter(edge => edge.kind === 'delivery').map(edge => [edge.from, edge.to]), [
    ['alice', 'relay-a'], ['alice', 'relay-b'], ['relay-a', 'bob'],
  ], 'replicas are fanout copies, with one selected download source and no fictional relay hop');
});

test('private delivery requires selected HTTP receipts plus matching storage and download bytes', () => {
  const state = journey('store');
  for (const service of ['relay-a', 'relay-b']) {
    state.stored.push({ service, objectId: object.id, identical: true });
    state.uploads.push({ service, objectId: object.id, sha256: object.id, status: 201, receipt: { id: object.id, size: 4096 }, signature: 'not-for-the-view' });
  }
  let flow = describeFlow({ visibility: 'private', journey: state, availability });
  assert.equal(flow.edges.filter(edge => edge.kind === 'delivery' && edge.status === 'observed').length, 2);
  assert.equal(flow.edges.find(edge => edge.stage === 'download').status, 'pending');
  state.uploads[1].receipt.id = 'b'.repeat(64);
  flow = describeFlow({ visibility: 'private', journey: state, availability });
  assert.equal(flow.edges.find(edge => edge.to === 'relay-b' && edge.kind === 'delivery').status, 'pending');
  state.stage = 'download';
  state.downloads.push({ service: 'relay-a', objectAlias: 'E1', objectId: object.id, sha256: object.id, identical: true, size: 4096, status: 200, readProofUsed: true });
  flow = describeFlow({ visibility: 'private', journey: state, availability });
  assert.equal(flow.edges.find(edge => edge.stage === 'download').status, 'observed');
  assert.doesNotMatch(JSON.stringify(flow), /not-for-the-view|signature/);
});

test('public path reverses for Bob and inferred hops require both stores and enforced connections', () => {
  const state = { author: 'bob', stage: 'open', post: { id: 'p' }, stored: [{ service: 'node-b', matchingPost: true }], recovered: { samePost: true, sameText: true, sameTitle: true } };
  let flow = describeFlow({ visibility: 'public', journey: state, topology });
  assert.deepEqual(flow.edges.filter(edge => edge.kind === 'delivery').map(edge => [edge.from, edge.to]), [['bob', 'node-b'], ['node-b', 'node-a'], ['node-a', 'alice']]);
  assert.ok(flow.edges.every(edge => edge.status === 'pending'));
  state.stored.push({ service: 'node-a', matchingPost: true });
  flow = describeFlow({ visibility: 'public', journey: state, topology });
  assert.ok(flow.edges.every(edge => edge.status === 'inferred'));
  assert.ok(flow.edges.every(edge => edge.evidence.some(line => line.includes('Individual native packets were not captured'))));
  flow = describeFlow({ visibility: 'public', journey: state, topology: { ...topology, enforced: false } });
  assert.ok(flow.edges.every(edge => edge.status === 'pending'));
  const setup = describeFlow({ visibility: 'public', topology });
  assert.equal(setup.edges.find(edge => edge.from === 'node-a').status, 'configured', 'a configured native route is not a message receipt');
});

test('failed steps leave uncaptured hops unknown and earlier captured evidence historical', () => {
  const state = journey('store');
  state.failed = true;
  state.stored = [{ service: 'relay-a', objectId: object.id, identical: true }];
  state.uploads = [{ service: 'relay-a', objectId: object.id, sha256: object.id, status: 200, receipt: { id: object.id, size: 4096 } }];
  const flow = describeFlow({ visibility: 'private', journey: state, availability });
  assert.equal(flow.edges.find(edge => edge.id === 'alice-relay-a-store').historical, true);
  assert.equal(flow.edges.find(edge => edge.id === 'alice-relay-b-store').status, 'unknown');
  assert.equal(flow.edges.find(edge => edge.stage === 'download').status, 'unknown');
});
