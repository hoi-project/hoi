// Run after building hoi-node and the browser WASM, or inside the walkthrough image.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import { mkdir, writeFile, access } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createWalkthroughNetwork } from './walkthrough-network.mjs';

const output = resolve('sandbox/.state/walkthrough-smoke');
await mkdir(output, { recursive: true });
const network = createWalkthroughNetwork({ stateDir: output, nodeCommand: process.env.HOI_NODE_BIN || process.env.HOI_NODE_COMMAND, onProgress: message => console.log(message) });
try {
  let state = await network.start();
  assert.equal(state.status, 'ready');
  assert.equal(state.services.filter(service => service.kind === 'node').length, 2);
  assert.equal(state.services.filter(service => service.kind === 'registry').length, 2);
  assert.equal(state.services.filter(service => service.kind === 'relay').length, 2);
  const assertChain = state => {
    assert.equal(state.topology.enforced, true);
    assert.deepEqual(state.topology.clients.map(client => [client.id, client.node, client.communityConnected]), [['alice', 'node-a', true], ['bob', 'node-b', true]]);
    assert.ok(state.topology.clients.every(client => client.connections.length && client.connections.every(connection => connection.transport === 'webrtc' && connection.community)));
  };
  assertChain(state);
  assert.equal(state.discovery.length, 2);
  assert.ok(state.discovery.every(lookup => lookup.status === 200 && lookup.available.length === 2 && lookup.selected.length === 1));
  assert.ok(state.flows.private.nodes.every(node => node.kind !== 'registry'));
  assert.equal(state.flows.public.mode, 'public-chain');
  for (const person of state.participants) {
    assert.ok(person.feeds.public.length, `${person.name} reads the public seed`);
    assert.ok(person.feeds.private.length, `${person.name} reads the private seed`);
  }
  for (const visibility of ['public', 'private']) {
    state = await network.publish({ author: 'bob', visibility, title: `Bob's ${visibility} test`, text: `Delivery check for the ${visibility} community.` });
    assert.equal(state.trace.complete, true);
    const kind = visibility === 'public' ? 'node' : 'relay';
    for (const service of state.services.filter(service => service.kind === kind)) {
      assert.equal(state.trace.steps.find(step => step.id === service.id).status, 'observed');
      assert.equal(service.containsCanary, visibility === 'public', 'the raw service store matches its content-visibility claim');
    }
  }
  const message = { author: 'alice', visibility: 'private', title: 'Meet at the greenhouse', text: 'Bring the blue notebook. Our private plan begins at 10:30.' };
  const sequenceError = error => error.code === 'WALKTHROUGH_SEQUENCE';
  const absentFromReceiver = state => {
    const receiver = state.participants.find(person => person.id === state.journey.receiver);
    assert.equal(receiver.feeds.private.some(post => post.id === state.journey.post?.id), false, 'the receiver core has not received this new post');
    assert.equal(state.journey.recovered, undefined, 'no recovered plaintext is projected before Open');
  };
  const advance = async state => network.nextJourney({ journeyId: state.journey.id, stage: state.journey.stage });
  const rejectRepeatedMessage = async value => {
    const postIds = state => state.participants.map(person => person.feeds[value.visibility].map(post => post.id).sort());
    const relayIds = state => state.services.filter(service => service.kind === 'relay').map(service => service.records.map(record => record.id).sort());
    let draft = await network.startJourney(value);
    // Preparing a run flushes older group updates. Compare the rejection with this prepared
    // state so those legitimate setup deliveries are not attributed to the failed post.
    const prepared = draft;
    for (let attempt = 0; attempt < 2; attempt++) {
      await assert.rejects(advance(draft), error => error.code === 'WALKTHROUGH_DUPLICATE');
      draft = await network.snapshot();
      assert.equal(draft.journey.stage, 'draft');
      assert.equal(draft.journey.failed, false, 'a known rejection before acceptance does not require resetting the network');
      assert.equal(draft.journey.recoverable, true);
      assert.match(draft.journey.error, /already sent this message/);
      assert.ok(draft.journey.nextAction, 'the unchanged draft may be tried again without a sequence lock');
      assert.equal(draft.journey.post, undefined);
      assert.equal(draft.journey.objects.length, 0);
      assert.deepEqual(postIds(draft), postIds(prepared), 'duplicate rejection stores no new post');
      assert.deepEqual(relayIds(draft), relayIds(prepared), 'duplicate rejection adds no encrypted relay copy');
    }
    return draft;
  };
  state = await network.startJourney(message);
  assert.equal(state.journey.stage, 'draft');
  assert.equal(state.journey.mode, 'live');
  assert.ok(state.journey.setup.availability.every(probe => probe.status === 200 && probe.apiVersion === 2));
  assert.ok(state.journey.flow.edges.filter(edge => edge.kind === 'delivery').every(edge => edge.status === 'pending'));
  const draftToken = { journeyId: state.journey.id, stage: 'draft' };
  await assert.rejects(network.nextJourney({ ...draftToken, journeyId: 'stale-session' }), sequenceError);
  state = await advance(state);
  assert.equal(state.journey.stage, 'seal');
  absentFromReceiver(state);
  assert.equal(state.journey.stored.length, 0);
  assert.ok(state.journey.objects.length > 0);
  for (const object of state.journey.objects) {
    assert.notEqual(object.id, state.journey.post.id, 'the post and ciphertext have distinct identifiers');
    assert.equal(createHash('sha256').update(Buffer.from(object.hex, 'hex')).digest('hex'), object.id);
    assert.equal(object.segments.map(segment => segment.hex).join(''), object.hex);
    assert.deepEqual([object.segments[0].bytes, object.segments[2].bytes], [24, 16]);
  }
  await assert.rejects(network.nextJourney(draftToken), sequenceError, 'a double click cannot advance another stage');
  await assert.rejects(network.startJourney(message), sequenceError, 'cannot replace an encrypted but undelivered journey');
  await assert.rejects(network.publish({ ...message, title: 'Interleaved message' }), sequenceError);
  state = await advance(state);
  assert.equal(state.journey.stage, 'store');
  absentFromReceiver(state);
  assert.equal(state.journey.stored.length, state.journey.objects.length * 2);
  assert.ok(state.journey.stored.every(copy => copy.identical && copy.sha256 === copy.objectId));
  assert.equal(state.journey.uploads.length, state.journey.objects.length * 2);
  for (const upload of state.journey.uploads) {
    const object = state.journey.objects.find(object => object.id === upload.objectId);
    assert.equal(upload.method, 'PUT');
    assert.ok(upload.status >= 200 && upload.status < 300);
    assert.equal(upload.sha256, object.id);
    assert.deepEqual(upload.receipt, { id: object.id, size: object.size, expires: object.expires });
  }
  assert.doesNotMatch(JSON.stringify(state.journey.uploads), /signature|nonce/);
  assert.equal(state.journey.flow.edges.filter(edge => edge.kind === 'delivery' && edge.status === 'observed').length, 2);
  // Cross the production app's 15-second background interval: explicit controls must still
  // keep the selected message out of the receiver core until Open, even after upload.
  console.log('Checking that the stored private message remains unopened across the background interval');
  await new Promise(done => setTimeout(done, 16_000));
  state = await network.snapshot();
  absentFromReceiver(state);
  assert.equal(state.journey.stage, 'store');
  state = await advance(state);
  assert.equal(state.journey.stage, 'download');
  assertChain(state);
  absentFromReceiver(state);
  assert.equal(state.journey.permissions.find(person => person.id === 'bob').state, 'ciphertext');
  for (const [index, downloaded] of state.journey.downloads.entries()) {
    assert.equal(downloaded.hex, state.journey.objects[index].hex, 'received full bytes match the sender');
    assert.equal(downloaded.sha256, state.journey.objects[index].id);
    assert.equal(downloaded.identical, true);
    assert.equal(downloaded.method, 'GET');
    assert.equal(downloaded.status, 200);
    assert.equal(downloaded.readProofUsed, true);
  }
  assert.equal(state.journey.flow.edges.find(edge => edge.stage === 'download').from, 'relay-a');
  assert.equal(state.journey.flow.edges.find(edge => edge.stage === 'download').status, 'observed');
  state = await advance(state);
  assert.equal(state.journey.stage, 'open');
  assert.equal(state.journey.complete, true);
  assert.equal(state.journey.recovered.samePost, true);
  assert.equal(state.journey.recovered.sameText, true);
  assertChain(state);
  assert.equal(state.journey.recovered.sameTitle, true);
  assert.equal(state.journey.recovered.text, message.text);
  assert.equal(state.journey.stages.find(stage => stage.id === 'download').permissions.find(person => person.id === 'bob').state, 'ciphertext', 'historical download evidence does not acquire later plaintext');
  assert.ok(state.trace.steps.every(step => !step.id.startsWith('registry')), 'discovery registries are not shown as private delivery hops');
  await assert.rejects(advance(state), sequenceError, 'a completed journey cannot run again');
  await rejectRepeatedMessage(message);
  state = await network.startJourney({ ...message, title: 'Updated greenhouse meeting' });
  for (let step = 0; step < 4; step++) state = await advance(state);
  assert.equal(state.journey.complete, true, 'editing the duplicate draft permits a complete new private journey');
  const publicMessage = { author: 'bob', visibility: 'public', title: 'Garden opening', text: 'The greenhouse opens on Saturday at 10:30. Everyone is welcome.' };
  state = await network.startJourney(publicMessage);
  assert.equal(state.journey.mode, 'recorded');
  state = await advance(state);
  assert.equal(state.journey.complete, true);
  assert.equal(state.journey.objects.length, 0);
  assert.equal(state.journey.stored.length, 2);
  assert.ok(state.journey.stored.every(copy => copy.matchingPost && copy.readable && !copy.identical));
  assert.equal(state.journey.recovered.sameText, true);
  assertChain(state);
  assert.deepEqual(state.journey.flow.edges.filter(edge => edge.kind === 'delivery').map(edge => [edge.from, edge.to, edge.status]), [
    ['bob', 'node-b', 'inferred'], ['node-b', 'node-a', 'inferred'], ['node-a', 'alice', 'inferred'],
  ]);
  assert.ok(state.trace.steps.every(step => !step.id.startsWith('registry')), 'registries belong in discovery context, not the message path');
  await rejectRepeatedMessage(publicMessage);
  state = await network.startJourney({ ...publicMessage, title: 'Garden opening invitation' });
  state = await advance(state);
  assert.equal(state.journey.complete, true, 'editing the duplicate draft permits a complete new public journey');
  await writeFile(resolve(output, 'alice.png'), await network.screenshot('alice'));
  await writeFile(resolve(output, 'bob.png'), await network.screenshot('bob'));
  await writeFile(resolve(output, 'alice-public.png'), await network.screenshot('alice', 'public'));
  await writeFile(resolve(output, 'bob-private.png'), await network.screenshot('bob', 'private'));
  // Corrupt only this disposable session's selected relay object, then prove that a failed
  // step cannot silently retry a mutation or continue presenting a confirmed current state.
  state = await network.startJourney({ ...message, author: 'bob', title: 'Integrity check' });
  state = await advance(state);
  state = await advance(state);
  const object = state.journey.objects[0], corrupted = Buffer.from(object.hex, 'hex');
  corrupted[0] ^= 1;
  const relayDb = new DatabaseSync(resolve(output, state.session, 'relay-a.db'));
  try {
    assert.equal(relayDb.prepare('UPDATE objects_v2 SET body = ? WHERE room = ? AND id = ?').run(corrupted, object.room, object.id).changes, 1);
  } finally { relayDb.close(); }
  await assert.rejects(advance(state), 'corrupted bytes must fail before core decryption');
  state = await network.snapshot();
  assert.equal(state.journey.failed, true);
  assert.equal(state.journey.nextAction, null);
  assert.ok(state.journey.permissions.filter(person => person.id !== state.journey.author).every(person => person.state === 'unknown'));
  absentFromReceiver(state);
  await assert.rejects(advance(state), sequenceError);
  await assert.rejects(network.startJourney(message), sequenceError);
  await assert.rejects(network.publish(message), sequenceError);
  const previous = state.session;
  state = await network.reset();
  assert.notEqual(state.session, previous);
  assert.equal(state.journey, null);
  await assert.rejects(access(resolve(output, previous)), 'reset removes the previous owned session');
  assert.equal(state.participants.flatMap(person => person.feeds.public).some(post => post.title === "Bob's public test"), false);
  console.log('PASS: enforced public node chain, real discovery and private PUT/GET evidence, staged encryption/decryption, editable duplicate drafts, corruption handling and reset.');
} finally { await network.close(); }
