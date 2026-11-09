import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { describeCryptoJourney } from './walkthrough-crypto-view.mjs';
import { describeJourney, newJourney, cipherArtifact } from './walkthrough-journey.mjs';

function example(stage = 'open') {
  const bytes = Buffer.alloc(4096, 0x5a);
  const id = createHash('sha256').update(bytes).digest('hex');
  const object = cipherArtifact({ id, room: 'b'.repeat(64), body: bytes.toString('base64url'), size: bytes.length, expires: 1 });
  const journey = newJourney({ id: 'example', value: { author: 'alice', visibility: 'private', title: 'Meet me', text: 'Bring the blue notebook.' }, community: { id: 'c', name: 'Circle' } });
  journey.stage = stage;
  journey.complete = stage === 'open';
  journey.post = { alias: 'P1', id: 'a'.repeat(64), title: journey.message.title, text: journey.message.text, checks: { signature: 'valid' } };
  journey.objects = [object];
  journey.downloads = [{ objectAlias: object.alias, objectId: object.id, size: object.size, hex: object.hex }];
  journey.recovered = { ...journey.post, samePost: true, sameTitle: true, sameText: true };
  return journey;
}

function allLayers(steps) {
  const visit = layers => layers.flatMap(item => [item, ...visit(item.children ?? [])]);
  return steps.flatMap(step => visit(step.layers));
}

test('the packet diagram points to exact nonce, ciphertext and tag ranges without duplicating bytes', () => {
  const journey = example();
  const crypto = describeCryptoJourney(journey);
  for (const [direction, id, source] of [['sender', 'outer', journey.objects[0]], ['receiver', 'package', journey.downloads[0]]]) {
    const packet = crypto[direction].find(step => step.id === id).layers[0];
    const ranges = packet.children.map(item => item.byteRange);
    assert.deepEqual(ranges.map(range => [range.offset, range.length]), [[0, 24], [24, 4056], [4080, 16]]);
    assert.ok(ranges.every(range => range.artifactAlias === 'E1' && range.source === direction));
    const reconstructed = ranges.map(range => source.hex.slice(range.offset * 2, (range.offset + range.length) * 2)).join('');
    assert.equal(reconstructed, source.hex);
    assert.equal(createHash('sha256').update(Buffer.from(reconstructed, 'hex')).digest('hex'), packet.fingerprint);
    assert.equal(packet.evidence, 'observed');
  }
  assert.ok(allLayers([...crypto.sender, ...crypto.receiver]).every(layer => !Object.hasOwn(layer, 'hex')));
  assert.match(crypto.copyModel, /complete copy/);
});

test('uncaptured inner operations stay explained after a successful recovery, with no invented MLS or padding length', () => {
  const crypto = describeCryptoJourney(example());
  for (const id of ['mls', 'padding']) assert.equal(crypto.sender.find(step => step.id === id).evidence, 'explained');
  for (const id of ['outer-open', 'unpad', 'mls-open']) assert.equal(crypto.receiver.find(step => step.id === id).evidence, 'explained');
  const padding = crypto.sender.find(step => step.id === 'padding').layers[0];
  assert.equal(padding.bytes, 4056);
  assert.match(padding.sizeBasis, /Derived/);
  assert.equal(padding.children.find(layer => layer.id === 'length').bytes, 4);
  assert.equal(padding.children.find(layer => layer.id === 'mls').bytes, undefined);
  assert.equal(padding.children.find(layer => layer.id === 'zeros').bytes, undefined);
  const openedHeaders = allLayers(crypto.receiver.filter(step => ['outer-open', 'unpad'].includes(step.id))).filter(layer => layer.id === 'mls-header');
  assert.ok(openedHeaders.length > 0);
  assert.ok(openedHeaders.every(layer => layer.label === 'MLS header · group, epoch and message type'));
  assert.equal(crypto.receiver.find(step => step.id === 'readable').layers[0].children[1].value, 'Bring the blue notebook.');
});

test('future outputs are never observed even when a fixture contains later data', () => {
  const stages = ['draft', 'seal', 'store', 'download', 'open'];
  for (const stage of stages) {
    const crypto = describeCryptoJourney(example(stage));
    for (const item of [...crypto.sender, ...crypto.receiver, ...allLayers([...crypto.sender, ...crypto.receiver])]) {
      if (item.evidence === 'observed') assert.ok(stages.indexOf(item.availableAt) <= stages.indexOf(stage), `${stage}: ${item.id}`);
    }
    if (stage !== 'open') {
      assert.ok(crypto.receiver.filter(step => step.id !== 'package').every(step => step.evidence === 'pending' && step.readable === null));
      assert.ok(!allLayers(crypto.receiver).some(layer => Object.hasOwn(layer, 'value')));
    }
  }
});

test('historical download inspection can exclude every recovered plaintext value using availableAt', () => {
  const crypto = describeCryptoJourney(example());
  const order = ['draft', 'seal', 'store', 'download', 'open'];
  const visible = allLayers(crypto.receiver).filter(layer => order.indexOf(layer.availableAt) <= order.indexOf('download'));
  assert.ok(visible.length > 0);
  assert.ok(visible.every(layer => layer.readable === false));
  assert.ok(visible.every(layer => !Object.hasOwn(layer, 'value')));
});

test('an ambiguous failure never claims current receiver readability or verified receipt', () => {
  const journey = example();
  journey.failed = true;
  const crypto = describeCryptoJourney(journey);
  assert.ok(crypto.receiver.every(step => step.evidence === 'unknown' && step.readable === null));
  assert.ok(allLayers(crypto.receiver).every(layer => layer.evidence === 'unknown' && layer.readable === null));
  assert.ok(!allLayers(crypto.receiver).some(layer => Object.hasOwn(layer, 'value')));
});

test('an invalid source hash cannot become an observed encrypted packet', () => {
  const journey = example('seal');
  journey.objects[0].hex = '00'.repeat(4096);
  const crypto = describeCryptoJourney(journey);
  assert.notEqual(crypto.sender.find(step => step.id === 'outer').evidence, 'observed');
  assert.ok(!allLayers(crypto.sender).some(layer => layer.byteRange));
});

test('journey presentation includes the private packet model and excludes it for public delivery', () => {
  assert.equal(describeJourney(example()).crypto.sender.length, 5);
  const publicJourney = example();
  publicJourney.visibility = 'public';
  assert.equal(describeJourney(publicJourney).crypto, null);
});
