import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { cipherArtifact, describeJourney, newJourney } from './walkthrough-journey.mjs';

test('the ciphertext view segments and hashes the exact captured core output', () => {
  const bytes = randomBytes(4096);
  const object = { id: createHash('sha256').update(bytes).digest('hex'), room: 'a'.repeat(64), size: bytes.length, body: bytes.toString('base64url'), expires: 1 };
  const artifact = cipherArtifact(object);
  assert.equal(artifact.alias, 'E1');
  assert.equal(artifact.from, 'P1');
  assert.equal(artifact.segments.map(part => part.hex).join(''), bytes.toString('hex'));
  assert.deepEqual(artifact.segments.map(part => part.bytes), [24, 4056, 16]);
  assert.throws(() => cipherArtifact({ ...object, id: '0'.repeat(64) }));
  assert.throws(() => cipherArtifact({ ...object, size: 4097 }));
});

test('private progress never presents later receiver plaintext or earlier relay possession', () => {
  const journey = newJourney({ id: 'test', value: { visibility: 'private', author: 'alice', title: 'Plan', text: 'Our private plan.' }, community: { id: 'c', name: 'Circle' } });
  for (const [stage, receiver, relay] of [['draft', 'none', 'none'], ['seal', 'none', 'none'], ['store', 'none', 'ciphertext'], ['download', 'ciphertext', 'ciphertext'], ['open', 'readable', 'ciphertext']]) {
    journey.stage = stage;
    describeJourney(journey);
    assert.equal(journey.permissions.find(item => item.id === 'bob').state, receiver);
    assert.equal(journey.permissions.find(item => item.id === 'relay-a').state, relay);
    assert.equal(journey.stages[0].permissions.find(item => item.id === 'bob').state, 'none');
    assert.ok(journey.stages.filter(item => item.status === 'pending').every(item => item.observations.length === 0 && item.permissions.length === 0));
  }
  assert.ok(journey.permissions.every(item => !item.id.startsWith('registry')));
});

test('public delivery is described as recorded automatic delivery, never encryption or staged download', () => {
  const journey = describeJourney(newJourney({ id: 'test', value: { visibility: 'public', author: 'bob', title: 'Picnic', text: 'Saturday at ten.' }, community: { id: 'c', name: 'Garden' } }));
  assert.equal(journey.mode, 'recorded');
  assert.equal(journey.nextAction, 'Publish and record delivery');
  assert.equal(journey.stages.some(stage => stage.id === 'download'), false);
  assert.match(journey.stages.find(stage => stage.id === 'seal').internals[0], /automatically/);
  assert.equal(journey.objects.length, 0);
  journey.stage = 'open'; journey.complete = true;
  journey.stored = [{ label: 'Public node A', matchingPost: true }];
  assert.match(describeJourney(journey).stages.find(stage => stage.id === 'store').observations[0], /same post ID/);
  assert.doesNotMatch(journey.stages.find(stage => stage.id === 'store').observations[0], /original bytes/);
});

test('failed operations stop further actions and leave current remote visibility unverified', () => {
  const journey = newJourney({ id: 'test', value: { visibility: 'private', author: 'alice', title: 'Plan', text: 'Our private plan.' }, community: { id: 'c', name: 'Circle' } });
  journey.stage = 'seal'; journey.failed = true;
  describeJourney(journey);
  assert.equal(journey.nextAction, null);
  assert.ok(journey.permissions.filter(item => item.id !== 'alice').every(item => item.state === 'unknown'));
  assert.ok(journey.stages.find(item => item.id === 'seal').permissions.filter(item => item.id !== 'alice').every(item => item.state === 'none'), 'historical evidence remains explicitly historical');
});
