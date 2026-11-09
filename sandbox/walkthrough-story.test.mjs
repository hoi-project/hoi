import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { buildStory } from './public/walkthrough-story.mjs';
import { describeCryptoJourney } from './walkthrough-crypto-view.mjs';
import { describeFlow } from './walkthrough-flow.mjs';

const original = { title: 'A private plan', text: 'Meet beside the blue door.' };
const object = { alias: 'E1', id: createHash('sha256').update(Buffer.alloc(4096, 0xab)).digest('hex'), size: 4096, hex: 'ab'.repeat(4096), expires: 1800000000 };
const topology = { enforced: true, nativeEntry: { configured: true }, clients: [
  { id: 'alice', node: 'node-a', communityConnected: true }, { id: 'bob', node: 'node-b', communityConnected: true },
] };
const connections = {
  private: { communityId: 'c'.repeat(64), name: 'Our group', sameCommunity: true, sameRoom: true, sameEpoch: true,
    clients: ['alice', 'bob'].map(id => ({ id, active: true, epoch: 1, memberCount: 2, communityId: 'c'.repeat(64) })),
    invitation: { method: 'local-handoff', completed: true }, relays: ['relay-a', 'relay-b'] },
  public: { communityId: 'd'.repeat(64), name: 'Town square', sameCommunity: true,
    clients: [{ id: 'alice', joined: true, node: 'node-a' }, { id: 'bob', joined: true, node: 'node-b' }], nativeLink: { from: 'node-b', to: 'node-a', configured: true } },
};
const discovery = ['alice', 'bob'].map((actor, index) => ({ id: actor + '-discovery', actor, service: 'registry-' + (index ? 'b' : 'a'), path: '/v2/nodes', status: 200, selected: ['node-' + (index ? 'b' : 'a')], available: ['node-a', 'node-b'] }));
const availability = ['relay-a', 'relay-b'].map(service => ({ actor: 'alice', service, path: '/relay/v2/meta', status: 200, apiVersion: 2 }));

function snapshot(stage = 'open', visibility = 'private', author = 'alice') {
  const position = ['draft', 'seal', 'store', 'download', 'open'].indexOf(stage);
  const journey = { id: 'example', author, receiver: author === 'alice' ? 'bob' : 'alice', visibility, stage, complete: stage === 'open', message: original,
    community: { id: 'c'.repeat(64), name: 'Our group' }, objects: [], downloads: [], stored: [], uploads: [] };
  if (position >= 1) {
    journey.post = { alias: 'P1', id: 'e'.repeat(64), ...original, checks: { signature: 'valid' } };
    if (visibility === 'private') journey.objects = [object];
  }
  if (position >= 2) for (const suffix of ['a', 'b']) {
    if (visibility === 'private') {
      journey.stored.push({ service: 'relay-' + suffix, objectId: object.id, identical: true });
      journey.uploads.push({ service: 'relay-' + suffix, objectId: object.id, sha256: object.id, status: 201, receipt: { id: object.id, size: object.size, expires: object.expires } });
    } else journey.stored.push({ service: 'node-' + suffix, matchingPost: true });
  }
  if (position >= 3 && visibility === 'private') journey.downloads.push({ service: 'relay-a', objectAlias: 'E1', objectId: object.id, sha256: object.id, size: object.size, status: 200, identical: true, readProofUsed: true, hex: object.hex });
  if (position === 4) journey.recovered = { ...journey.post, samePost: true, sameTitle: true, sameText: true };
  journey.crypto = describeCryptoJourney(journey);
  journey.flow = describeFlow({ visibility, journey, topology, availability, discovery });
  return { journey, connections, flows: { private: describeFlow({ visibility: 'private', availability }), public: describeFlow({ visibility: 'public', topology, discovery }) } };
}

test('private story follows membership, protection, whole-copy transfers and local opening', () => {
  const state = snapshot();
  const frames = buildStory(state);
  assert.equal(frames.length, 17);
  assert.deepEqual(frames.map(item => item.id), [
    'private-community', 'private-invitation', 'private-relays', 'message-input', 'protect-signed-post', 'protect-mls', 'protect-padding', 'protect-outer',
    'upload-relay-a', 'upload-relay-b', 'receiver-request', 'receiver-download', 'open-outer-open', 'open-unpad', 'open-mls-open', 'open-validate', 'open-readable',
  ]);
  assert.ok(frames.slice(0, 3).every(item => item.requiredStage === null && item.source.includes('Captured setup replay')));
  assert.deepEqual(frames.filter(item => item.kind === 'transfer').map(item => item.route), [
    { from: 'alice', to: 'relay-a' }, { from: 'alice', to: 'relay-b' }, { from: 'relay-a', to: 'bob' },
  ]);
  const request = frames.find(item => item.id === 'receiver-request');
  assert.deepEqual(request.route, { from: 'bob', to: 'relay-a' });
  assert.equal(request.evidence, 'observed');
  assert.equal(frames.at(-1).artifact, 'recovered');
  assert.ok(frames.at(-1).output.text.includes(original.text));
  assert.match(frames.find(item => item.id === 'protect-mls').source, /intermediate bytes were not captured/);
  assert.match(frames.find(item => item.id === 'upload-relay-a').source, /uploads both copies/);
});

test('private frames gate later evidence and recovered plaintext at each actual stage', () => {
  for (const stage of ['draft', 'seal', 'store', 'download']) {
    const state = snapshot(stage), frames = buildStory(state);
    for (const item of frames.filter(item => item.phase === 'open')) {
      assert.equal(item.evidence, 'pending');
      assert.equal(item.artifact, undefined);
      assert.doesNotMatch(JSON.stringify(item), /Meet beside the blue door/);
    }
    const expected = ['draft', 'seal', 'store', 'download'].indexOf(stage);
    assert.equal(frames.find(item => item.id === 'protect-outer').evidence, expected >= 1 ? 'observed' : 'pending');
    assert.equal(frames.find(item => item.id === 'upload-relay-a').evidence, expected >= 2 ? 'observed' : 'pending');
    assert.equal(frames.find(item => item.id === 'receiver-download').evidence, expected >= 3 ? 'observed' : 'pending');
  }
  const complete = buildStory(snapshot());
  for (const item of complete.filter(item => ['send', 'open'].includes(item.phase) && item.id !== 'open-readable')) {
    assert.doesNotMatch(JSON.stringify(item), /Meet beside the blue door/, 'only final readable frame exposes receiver message text');
  }
});

test('a failed operation never exposes later result artifacts even with stale successful observations', () => {
  const state = snapshot(); state.journey.failed = true;
  const frames = buildStory(state);
  for (const item of frames.filter(item => item.requiredStage && item.requiredStage !== 'draft')) {
    assert.equal(item.evidence, 'unknown');
    assert.equal(item.artifact, undefined);
    assert.doesNotMatch(JSON.stringify(item.layers || []), /"value"|"byteRange"|"fingerprint"/);
  }
  assert.equal(frames.find(item => item.id === 'message-input').artifact, 'message', 'the original author input is still a known local value');
  assert.doesNotMatch(JSON.stringify(frames.at(-1)), /Meet beside the blue door/);
});

test('public story explains separate nodes, observed discovery and inferred message hops', () => {
  const frames = buildStory(snapshot('open', 'public'));
  assert.equal(frames.length, 14);
  assert.equal(frames.find(item => item.id === 'public-community').evidence, 'observed');
  assert.deepEqual(frames.find(item => item.id === 'connect-alice').route, { from: 'alice', to: 'node-a' });
  assert.deepEqual(frames.find(item => item.id === 'connect-bob').route, { from: 'bob', to: 'node-b' });
  assert.equal(frames.find(item => item.id === 'connect-bob').evidence, 'observed');
  assert.match(frames.find(item => item.id === 'connect-carriers').detail, /does not need Alice’s node/);
  assert.ok(frames.filter(item => item.id.startsWith('discover-')).every(item => item.evidence === 'observed'));
  const deliveries = frames.filter(item => item.kind === 'transfer');
  assert.deepEqual(deliveries.map(item => item.route), [{ from: 'alice', to: 'node-a' }, { from: 'node-a', to: 'node-b' }, { from: 'node-b', to: 'bob' }]);
  assert.ok(deliveries.every(item => item.requiredStage === 'open' && item.evidence === 'inferred' && item.source.includes('timing is illustrative')));
  assert.equal(frames.at(-1).artifact, 'recovered');
});

test('prior connections and Bob introducing a public community are conditional explanations, not captured events', () => {
  const frames = buildStory(snapshot('open', 'public'));
  const [existing, introduced, measured] = frames;
  assert.deepEqual([existing.id, introduced.id, measured.id], ['public-existing-connections', 'public-introduce-community', 'public-community']);
  for (const item of [existing, introduced]) {
    assert.equal(item.evidence, 'explained');
    assert.equal(item.requiredStage, null);
    assert.deepEqual(item.hiddenNodes, ['registry-a', 'registry-b']);
    assert.ok(item.diagramEdges.every(edge => !(edge.from.startsWith('node-') && edge.to.startsWith('node-'))), 'prior relationships do not invent a native node connection');
  }
  assert.match(existing.detail, /one community.*another.*do not mean/);
  assert.equal(existing.route, undefined);
  assert.match(existing.source, /not captured history/);
  assert.deepEqual(introduced.route, { from: 'bob', to: 'node-b' });
  assert.equal(introduced.routeLabel, 'Carry request');
  assert.match(introduced.detail, /signed definition.*policy.*accept/);
  assert.match(introduced.output.text.join(' '), /B learns.*from the request.*If B accepts.*invitation alone does not notify every node.*who invited him/);
  assert.match(introduced.source, /not a request captured.*Private invitations never/);
  assert.equal(measured.evidence, 'observed', 'the actual captured setup stays separate from the possible scenario');
});

test('Bob can author either example while setup accurately keeps Alice as private inviter', () => {
  const privateFrames = buildStory(snapshot('open', 'private', 'bob'));
  assert.deepEqual(privateFrames.find(item => item.id === 'private-invitation').route, { from: 'alice', to: 'bob' });
  assert.deepEqual(privateFrames.find(item => item.id === 'upload-relay-a').route, { from: 'bob', to: 'relay-a' });
  assert.deepEqual(privateFrames.find(item => item.id === 'receiver-download').route, { from: 'relay-a', to: 'alice' });
  const publicFrames = buildStory(snapshot('open', 'public', 'bob'));
  assert.deepEqual(publicFrames.filter(item => item.kind === 'transfer').map(item => item.route), [{ from: 'bob', to: 'node-b' }, { from: 'node-b', to: 'node-a' }, { from: 'node-a', to: 'alice' }]);
});

test('missing connection evidence remains pending and setup snapshots are stable per journey', () => {
  const state = snapshot();
  delete state.connections;
  let frames = buildStory(state);
  assert.ok(frames.slice(0, 3).every(item => item.evidence === 'pending'));
  state.journey.setup = { connection: connections.private };
  state.connections = { private: { sameCommunity: false } };
  frames = buildStory(state);
  assert.ok(frames.slice(0, 3).every(item => item.evidence === 'observed'));
  const initial = buildStory(null, 'public');
  assert.ok(initial.slice(0, 2).every(item => item.evidence === 'explained'), 'possible-context explanations do not require invented observations');
  assert.ok(initial.slice(2).every(item => item.evidence === 'pending'));
  assert.equal(initial.at(-1).artifact, undefined);
});

test('story generation is pure and cannot run or repeat backend operations', () => {
  const state = snapshot();
  const before = JSON.stringify(state);
  assert.deepEqual(buildStory(state), buildStory(state));
  assert.equal(JSON.stringify(state), before);
  assert.ok(buildStory(state).every(item => !Object.values(item).some(value => typeof value === 'function')));
});
