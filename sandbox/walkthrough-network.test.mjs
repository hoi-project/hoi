import test from 'node:test';
import assert from 'node:assert/strict';
import { allowedBrowserRequest, relayCanary, validatePublication } from './walkthrough-network.mjs';

test('the walkthrough accepts bounded seeded-persona posts only', () => {
  assert.deepEqual(validatePublication({ author: 'alice', visibility: 'private', title: ' Hello ', text: ' Local text ' }),
    { author: 'alice', visibility: 'private', title: 'Hello', text: 'Local text' });
  const valid = { author: 'bob', visibility: 'public', title: 'Hello', text: 'Local text' };
  for (const patch of [{ author: 'operator' }, { visibility: 'unknown' }, { title: '' }, { title: 'x'.repeat(121) }, { text: 'x'.repeat(4097) }, { text: null }]) {
    assert.throws(() => validatePublication({ ...valid, ...patch }));
  }
});

test('browser traffic is restricted to exactly the allocated loopback services', () => {
  const origins = new Set(['http://127.0.0.1:1234']);
  assert.equal(allowedBrowserRequest('http://127.0.0.1:1234/relay/v2/meta', origins), true);
  for (const url of ['http://127.0.0.1:5678/', 'http://localhost:1234/', 'https://example.com/', 'ws://127.0.0.1:1234/', 'http://user@127.0.0.1:1234/', 'file:///tmp/x', 'invalid']) {
    assert.equal(allowedBrowserRequest(url, origins), false, url);
  }
  assert.equal(allowedBrowserRequest('https://example.com/', new Set(['https://example.com'])), false);
});

test('short message byte coincidences are never presented as relay plaintext evidence', () => {
  for (const text of ['a', 'Hi', 'short message', '界界界界界']) assert.deepEqual(relayCanary(text), {});
  assert.deepEqual(relayCanary('A private message.'), { plaintextCanary: 'A private message.' });
  assert.deepEqual(relayCanary('界界界界界界'), { plaintextCanary: '界界界界界界' });
});
